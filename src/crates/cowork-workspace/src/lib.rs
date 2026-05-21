//! Platform-agnostic local workspace authority for Cowork.
//!
//! This crate owns the local filesystem boundary for authorized Cowork
//! workspaces. It validates workspace roots before IO, prevents traversal and
//! symlink escape, and returns ledger-ready evidence and artifact provenance
//! records without coupling to any adapter or persistence crate.

use chrono::{DateTime, Utc};
use image::GenericImageView;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

pub const WORKSPACE_SCHEMA_VERSION: &str = "cowork.workspace.v1";

pub type Metadata = BTreeMap<String, Value>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRoot {
    pub id: String,
    pub path: PathBuf,
}

impl WorkspaceRoot {
    pub fn new(id: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            id: id.into(),
            path: path.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizedRoot {
    pub id: String,
    pub canonical_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePath {
    pub root_id: String,
    pub relative_path: PathBuf,
}

impl WorkspacePath {
    pub fn new(root_id: impl Into<String>, relative_path: impl Into<PathBuf>) -> Self {
        Self {
            root_id: root_id.into(),
            relative_path: relative_path.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedWorkspacePath {
    pub root_id: String,
    pub relative_path: PathBuf,
    pub absolute_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct AuthorizedWorkspace {
    roots: BTreeMap<String, AuthorizedRoot>,
}

impl AuthorizedWorkspace {
    pub fn new(roots: impl IntoIterator<Item = WorkspaceRoot>) -> Result<Self, WorkspaceError> {
        let mut authorized_roots = BTreeMap::new();

        for root in roots {
            if root.id.trim().is_empty() {
                return Err(WorkspaceError::EmptyRootId);
            }

            if authorized_roots.contains_key(&root.id) {
                return Err(WorkspaceError::DuplicateRootId { root_id: root.id });
            }

            let canonical_path = dunce::canonicalize(&root.path).map_err(|error| {
                WorkspaceError::RootUnavailable {
                    root_id: root.id.clone(),
                    path: path_to_string(&root.path),
                    message: error.to_string(),
                }
            })?;

            let metadata =
                fs::metadata(&canonical_path).map_err(|error| WorkspaceError::RootUnavailable {
                    root_id: root.id.clone(),
                    path: path_to_string(&canonical_path),
                    message: error.to_string(),
                })?;

            if !metadata.is_dir() {
                return Err(WorkspaceError::RootUnavailable {
                    root_id: root.id.clone(),
                    path: path_to_string(&canonical_path),
                    message: "authorized workspace root is not a directory".to_string(),
                });
            }

            authorized_roots.insert(
                root.id.clone(),
                AuthorizedRoot {
                    id: root.id,
                    canonical_path,
                },
            );
        }

        Ok(Self {
            roots: authorized_roots,
        })
    }

    pub fn roots(&self) -> &BTreeMap<String, AuthorizedRoot> {
        &self.roots
    }

    pub fn read_file(&self, request: ReadFileRequest) -> Result<ReadFileResult, WorkspaceError> {
        let started_at = Utc::now();
        let resolved = self.resolve_existing(&request.path)?;
        let bytes = fs::read(&resolved.absolute_path)
            .map_err(|error| WorkspaceError::io("read", &resolved.absolute_path, error))?;
        let digest = sha256_digest(&bytes);

        Ok(ReadFileResult {
            bytes,
            evidence: FileOperationEvidence::new(
                &request.actor,
                &request.operation_id,
                FileOperationKind::Read,
                started_at,
                Some(resolved),
                None,
                FileOperationOutcome::succeeded(
                    "file read completed",
                    Some(digest.byte_len),
                    Some(digest.sha256.clone()),
                ),
                Vec::new(),
            ),
        })
    }

    pub fn write_file(
        &self,
        request: WriteFileRequest,
    ) -> Result<FileOperationResult, WorkspaceError> {
        let started_at = Utc::now();
        let target = self.resolve_write_target(&request.path)?;
        let digest = sha256_digest(&request.bytes);
        let temp_path = target
            .parent_path
            .join(format!(".cowork-workspace-{}.tmp", Uuid::new_v4()));

        let write_result = (|| -> Result<(), WorkspaceError> {
            let mut temp_file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
                .map_err(|error| WorkspaceError::io("create temporary file", &temp_path, error))?;
            temp_file
                .write_all(&request.bytes)
                .map_err(|error| WorkspaceError::io("write temporary file", &temp_path, error))?;
            temp_file
                .sync_all()
                .map_err(|error| WorkspaceError::io("sync temporary file", &temp_path, error))?;
            drop(temp_file);

            fs::rename(&temp_path, &target.resolved.absolute_path).map_err(|error| {
                WorkspaceError::io(
                    "replace authorized file",
                    &target.resolved.absolute_path,
                    error,
                )
            })?;

            Ok(())
        })();

        if write_result.is_err() {
            let _ = fs::remove_file(&temp_path);
        }

        write_result?;

        let evidence_id = evidence_id(&request.operation_id);
        let artifacts = request
            .artifact
            .into_iter()
            .map(|artifact| {
                ArtifactManifest::from_declaration(
                    artifact,
                    &target.resolved,
                    &request.actor,
                    &request.operation_id,
                    &evidence_id,
                    &digest,
                    started_at,
                )
            })
            .collect::<Vec<_>>();

        Ok(FileOperationResult {
            evidence: FileOperationEvidence {
                id: evidence_id,
                operation_id: request.operation_id,
                actor: request.actor,
                kind: FileOperationKind::Write,
                recorded_at: started_at,
                source: None,
                destination: Some(target.resolved),
                result: FileOperationOutcome::succeeded(
                    "file write completed",
                    Some(digest.byte_len),
                    Some(digest.sha256),
                ),
                artifacts,
                metadata: Metadata::new(),
            },
        })
    }

    pub fn move_file(
        &self,
        request: MoveFileRequest,
    ) -> Result<FileOperationResult, WorkspaceError> {
        self.move_like(request, FileOperationKind::Move)
    }

    pub fn rename_file(
        &self,
        request: MoveFileRequest,
    ) -> Result<FileOperationResult, WorkspaceError> {
        self.move_like(request, FileOperationKind::Rename)
    }

    pub fn organize_file(
        &self,
        request: MoveFileRequest,
    ) -> Result<FileOperationResult, WorkspaceError> {
        self.move_like(request, FileOperationKind::Organize)
    }

    pub fn inspect_metadata(
        &self,
        request: MetadataRequest,
    ) -> Result<MetadataResult, WorkspaceError> {
        let started_at = Utc::now();
        let resolved = self.resolve_existing(&request.path)?;
        let metadata = fs::metadata(&resolved.absolute_path)
            .map_err(|error| WorkspaceError::io("metadata", &resolved.absolute_path, error))?;

        let file_type = if metadata.is_file() {
            WorkspaceFileType::File
        } else if metadata.is_dir() {
            WorkspaceFileType::Directory
        } else {
            WorkspaceFileType::Other
        };

        let digest = if metadata.is_file() {
            let bytes = fs::read(&resolved.absolute_path).map_err(|error| {
                WorkspaceError::io(
                    "read file for metadata hash",
                    &resolved.absolute_path,
                    error,
                )
            })?;
            Some(sha256_digest(&bytes))
        } else {
            None
        };

        let workspace_metadata = WorkspaceMetadata {
            file_type,
            byte_len: metadata.len(),
            readonly: metadata.permissions().readonly(),
            sha256: digest.as_ref().map(|digest| digest.sha256.clone()),
        };

        Ok(MetadataResult {
            metadata: workspace_metadata,
            evidence: FileOperationEvidence::new(
                &request.actor,
                &request.operation_id,
                FileOperationKind::Metadata,
                started_at,
                Some(resolved),
                None,
                FileOperationOutcome::succeeded(
                    "metadata inspection completed",
                    Some(metadata.len()),
                    digest.map(|digest| digest.sha256),
                ),
                Vec::new(),
            ),
        })
    }

    pub fn parse_artifact(
        &self,
        request: ParseArtifactRequest,
    ) -> Result<ParseArtifactResult, WorkspaceError> {
        let started_at = Utc::now();
        let resolved = self.resolve_existing(&request.path)?;

        if let Some(failure) = unsupported_parser_failure(&request.path, request.format) {
            return Err(WorkspaceError::Parser(failure));
        }

        let bytes = fs::read(&resolved.absolute_path).map_err(|error| {
            WorkspaceError::io("read parser input", &resolved.absolute_path, error)
        })?;
        let digest = sha256_digest(&bytes);

        let parsed = match request.format {
            ParseFormat::Json => {
                let value = serde_json::from_slice(&bytes).map_err(|error| {
                    WorkspaceError::Parser(ParserFailure::invalid_content(
                        request.path.clone(),
                        request.format,
                        error.to_string(),
                    ))
                })?;
                ParsedArtifact::Json { value }
            }
            ParseFormat::Csv => {
                let content = String::from_utf8(bytes.clone()).map_err(|error| {
                    WorkspaceError::Parser(ParserFailure::invalid_content(
                        request.path.clone(),
                        request.format,
                        error.to_string(),
                    ))
                })?;
                ParsedArtifact::Csv(parse_csv_document(
                    &content,
                    request.path.clone(),
                    request.format,
                )?)
            }
            ParseFormat::ImageMetadata => {
                let image = image::load_from_memory(&bytes).map_err(|error| {
                    WorkspaceError::Parser(ParserFailure::invalid_content(
                        request.path.clone(),
                        request.format,
                        error.to_string(),
                    ))
                })?;
                let (width, height) = image.dimensions();
                ParsedArtifact::ImageMetadata(ImageMetadata {
                    width,
                    height,
                    color_type: format!("{:?}", image.color()),
                })
            }
            ParseFormat::Pdf
            | ParseFormat::Docx
            | ParseFormat::Xlsx
            | ParseFormat::Screenshot
            | ParseFormat::Ocr => unreachable!("unsupported formats returned above"),
        };

        Ok(ParseArtifactResult {
            parsed,
            evidence: FileOperationEvidence::new(
                &request.actor,
                &request.operation_id,
                FileOperationKind::Parse,
                started_at,
                Some(resolved),
                None,
                FileOperationOutcome::succeeded(
                    "artifact parse completed",
                    Some(digest.byte_len),
                    Some(digest.sha256),
                ),
                Vec::new(),
            ),
        })
    }

    fn move_like(
        &self,
        request: MoveFileRequest,
        kind: FileOperationKind,
    ) -> Result<FileOperationResult, WorkspaceError> {
        let started_at = Utc::now();
        let source = self.resolve_mutation_source(&request.source, kind)?;
        let destination = self.resolve_write_target(&request.destination)?;

        if source.absolute_path == destination.resolved.absolute_path {
            return Err(WorkspaceError::SourceDestinationSame {
                path: path_to_string(&source.absolute_path),
            });
        }

        if destination.resolved.absolute_path.exists() && !request.overwrite {
            return Err(WorkspaceError::DestinationExists {
                path: path_to_string(&destination.resolved.absolute_path),
            });
        }

        let digest = if fs::metadata(&source.absolute_path)
            .map_err(|error| {
                WorkspaceError::io("metadata source before move", &source.absolute_path, error)
            })?
            .is_file()
        {
            let bytes = fs::read(&source.absolute_path).map_err(|error| {
                WorkspaceError::io("read source before move", &source.absolute_path, error)
            })?;
            Some(sha256_digest(&bytes))
        } else {
            None
        };

        fs::rename(&source.absolute_path, &destination.resolved.absolute_path).map_err(
            |error| {
                WorkspaceError::io(
                    "move authorized path",
                    &destination.resolved.absolute_path,
                    error,
                )
            },
        )?;

        Ok(FileOperationResult {
            evidence: FileOperationEvidence::new(
                &request.actor,
                &request.operation_id,
                kind,
                started_at,
                Some(source),
                Some(destination.resolved),
                FileOperationOutcome::succeeded(
                    "workspace path moved",
                    digest.as_ref().map(|digest| digest.byte_len),
                    digest.map(|digest| digest.sha256),
                ),
                Vec::new(),
            ),
        })
    }

    fn resolve_existing(
        &self,
        path: &WorkspacePath,
    ) -> Result<ResolvedWorkspacePath, WorkspaceError> {
        let root = self.require_root(&path.root_id)?;
        let relative_path = normalize_relative_path(&path.root_id, &path.relative_path)?;
        let candidate = root.canonical_path.join(&relative_path);
        let resolved = dunce::canonicalize(&candidate)
            .map_err(|error| WorkspaceError::io("canonicalize existing path", &candidate, error))?;
        ensure_under_root(root, &path.relative_path, &resolved)?;

        Ok(ResolvedWorkspacePath {
            root_id: root.id.clone(),
            relative_path,
            absolute_path: resolved,
        })
    }

    fn resolve_mutation_source(
        &self,
        path: &WorkspacePath,
        operation: FileOperationKind,
    ) -> Result<ResolvedWorkspacePath, WorkspaceError> {
        let root = self.require_root(&path.root_id)?;
        let relative_path = normalize_relative_path(&path.root_id, &path.relative_path)?;
        let candidate = root.canonical_path.join(&relative_path);

        if fs::symlink_metadata(&candidate)
            .map_err(|error| WorkspaceError::io("inspect mutation source", &candidate, error))?
            .file_type()
            .is_symlink()
        {
            return Err(WorkspaceError::SymlinkMutationDenied {
                operation,
                root_id: path.root_id.clone(),
                path: path_to_string(&path.relative_path),
            });
        }

        let resolved = dunce::canonicalize(&candidate).map_err(|error| {
            WorkspaceError::io("canonicalize mutation source", &candidate, error)
        })?;
        ensure_under_root(root, &path.relative_path, &resolved)?;

        Ok(ResolvedWorkspacePath {
            root_id: root.id.clone(),
            relative_path,
            absolute_path: resolved,
        })
    }

    fn resolve_write_target(&self, path: &WorkspacePath) -> Result<WriteTarget, WorkspaceError> {
        let root = self.require_root(&path.root_id)?;
        let relative_path = normalize_relative_path(&path.root_id, &path.relative_path)?;

        if relative_path.as_os_str().is_empty() {
            return Err(WorkspaceError::RootTargetDenied {
                root_id: path.root_id.clone(),
            });
        }

        let candidate = root.canonical_path.join(&relative_path);
        let mut authorized_target_path = None;

        match fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(WorkspaceError::SymlinkWriteDenied {
                    root_id: path.root_id.clone(),
                    path: path_to_string(&path.relative_path),
                });
            }
            Ok(_) => {
                let resolved = dunce::canonicalize(&candidate).map_err(|error| {
                    WorkspaceError::io("canonicalize write target", &candidate, error)
                })?;
                ensure_under_root(root, &path.relative_path, &resolved)?;
                authorized_target_path = Some(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(WorkspaceError::io(
                    "inspect write target",
                    &candidate,
                    error,
                ));
            }
        }

        let parent = candidate
            .parent()
            .ok_or_else(|| WorkspaceError::ParentUnavailable {
                root_id: path.root_id.clone(),
                path: path_to_string(&path.relative_path),
                message: "target has no parent directory".to_string(),
            })?;
        let canonical_parent =
            authorize_parent_for_write(root, &path.root_id, &relative_path, parent)?;
        let target_file_name =
            relative_path
                .file_name()
                .ok_or_else(|| WorkspaceError::RootTargetDenied {
                    root_id: path.root_id.clone(),
                })?;
        let absolute_path =
            authorized_target_path.unwrap_or_else(|| canonical_parent.join(target_file_name));

        Ok(WriteTarget {
            parent_path: canonical_parent,
            resolved: ResolvedWorkspacePath {
                root_id: root.id.clone(),
                relative_path,
                absolute_path,
            },
        })
    }

    fn require_root(&self, root_id: &str) -> Result<&AuthorizedRoot, WorkspaceError> {
        self.roots
            .get(root_id)
            .ok_or_else(|| WorkspaceError::UnknownRoot {
                root_id: root_id.to_string(),
            })
    }
}

#[derive(Debug, Clone)]
struct WriteTarget {
    parent_path: PathBuf,
    resolved: ResolvedWorkspacePath,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadFileRequest {
    pub actor: String,
    pub operation_id: String,
    pub path: WorkspacePath,
}

impl ReadFileRequest {
    pub fn new(
        actor: impl Into<String>,
        operation_id: impl Into<String>,
        path: WorkspacePath,
    ) -> Self {
        Self {
            actor: actor.into(),
            operation_id: operation_id.into(),
            path,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteFileRequest {
    pub actor: String,
    pub operation_id: String,
    pub path: WorkspacePath,
    pub bytes: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<ArtifactDeclaration>,
}

impl WriteFileRequest {
    pub fn new(
        actor: impl Into<String>,
        operation_id: impl Into<String>,
        path: WorkspacePath,
        bytes: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            actor: actor.into(),
            operation_id: operation_id.into(),
            path,
            bytes: bytes.into(),
            artifact: None,
        }
    }

    pub fn with_artifact(mut self, artifact: ArtifactDeclaration) -> Self {
        self.artifact = Some(artifact);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveFileRequest {
    pub actor: String,
    pub operation_id: String,
    pub source: WorkspacePath,
    pub destination: WorkspacePath,
    pub overwrite: bool,
}

impl MoveFileRequest {
    pub fn new(
        actor: impl Into<String>,
        operation_id: impl Into<String>,
        source: WorkspacePath,
        destination: WorkspacePath,
    ) -> Self {
        Self {
            actor: actor.into(),
            operation_id: operation_id.into(),
            source,
            destination,
            overwrite: false,
        }
    }

    pub fn with_overwrite(mut self, overwrite: bool) -> Self {
        self.overwrite = overwrite;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataRequest {
    pub actor: String,
    pub operation_id: String,
    pub path: WorkspacePath,
}

impl MetadataRequest {
    pub fn new(
        actor: impl Into<String>,
        operation_id: impl Into<String>,
        path: WorkspacePath,
    ) -> Self {
        Self {
            actor: actor.into(),
            operation_id: operation_id.into(),
            path,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseFormat {
    Json,
    Csv,
    ImageMetadata,
    Pdf,
    Docx,
    Xlsx,
    Screenshot,
    Ocr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParseArtifactRequest {
    pub actor: String,
    pub operation_id: String,
    pub path: WorkspacePath,
    pub format: ParseFormat,
}

impl ParseArtifactRequest {
    pub fn new(
        actor: impl Into<String>,
        operation_id: impl Into<String>,
        path: WorkspacePath,
        format: ParseFormat,
    ) -> Self {
        Self {
            actor: actor.into(),
            operation_id: operation_id.into(),
            path,
            format,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadFileResult {
    pub bytes: Vec<u8>,
    pub evidence: FileOperationEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileOperationResult {
    pub evidence: FileOperationEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataResult {
    pub metadata: WorkspaceMetadata,
    pub evidence: FileOperationEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParseArtifactResult {
    pub parsed: ParsedArtifact,
    pub evidence: FileOperationEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileOperationEvidence {
    pub id: String,
    pub operation_id: String,
    pub actor: String,
    pub kind: FileOperationKind,
    pub recorded_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<ResolvedWorkspacePath>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<ResolvedWorkspacePath>,
    pub result: FileOperationOutcome,
    #[serde(default)]
    pub artifacts: Vec<ArtifactManifest>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

impl FileOperationEvidence {
    fn new(
        actor: &str,
        operation_id: &str,
        kind: FileOperationKind,
        recorded_at: DateTime<Utc>,
        source: Option<ResolvedWorkspacePath>,
        destination: Option<ResolvedWorkspacePath>,
        result: FileOperationOutcome,
        artifacts: Vec<ArtifactManifest>,
    ) -> Self {
        Self {
            id: evidence_id(operation_id),
            operation_id: operation_id.to_string(),
            actor: actor.to_string(),
            kind,
            recorded_at,
            source,
            destination,
            result,
            artifacts,
            metadata: Metadata::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileOperationKind {
    Read,
    Write,
    Move,
    Rename,
    Organize,
    Metadata,
    Parse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileOperationStatus {
    Succeeded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileOperationOutcome {
    pub status: FileOperationStatus,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub byte_len: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

impl FileOperationOutcome {
    fn succeeded(
        summary: impl Into<String>,
        byte_len: Option<u64>,
        sha256: Option<String>,
    ) -> Self {
        Self {
            status: FileOperationStatus::Succeeded,
            summary: summary.into(),
            byte_len,
            sha256,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactDeclaration {
    pub id: String,
    pub kind: ArtifactKind,
    pub title: String,
    pub provenance: ArtifactProvenance,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

impl ArtifactDeclaration {
    pub fn generated(
        id: impl Into<String>,
        kind: ArtifactKind,
        title: impl Into<String>,
        generator: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            title: title.into(),
            provenance: ArtifactProvenance::Generated {
                generator: generator.into(),
                input_sources: Vec::new(),
                parameters: Metadata::new(),
            },
            metadata: Metadata::new(),
        }
    }

    pub fn imported(
        id: impl Into<String>,
        kind: ArtifactKind,
        title: impl Into<String>,
        source_uri: impl Into<String>,
        imported_by: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            title: title.into(),
            provenance: ArtifactProvenance::Imported {
                source_uri: source_uri.into(),
                imported_by: imported_by.into(),
                source_hash: None,
                retrieved_at: None,
                license: None,
            },
            metadata: Metadata::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactManifest {
    pub schema_version: String,
    pub id: String,
    pub kind: ArtifactKind,
    pub title: String,
    pub path: WorkspacePath,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    pub operation_id: String,
    pub evidence_id: String,
    pub byte_len: u64,
    pub content_hash: String,
    pub provenance: ArtifactProvenance,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

impl ArtifactManifest {
    fn from_declaration(
        declaration: ArtifactDeclaration,
        path: &ResolvedWorkspacePath,
        actor: &str,
        operation_id: &str,
        evidence_id: &str,
        digest: &ContentDigest,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            schema_version: WORKSPACE_SCHEMA_VERSION.to_string(),
            id: declaration.id,
            kind: declaration.kind,
            title: declaration.title,
            path: WorkspacePath {
                root_id: path.root_id.clone(),
                relative_path: path.relative_path.clone(),
            },
            created_at,
            created_by: actor.to_string(),
            operation_id: operation_id.to_string(),
            evidence_id: evidence_id.to_string(),
            byte_len: digest.byte_len,
            content_hash: digest.sha256.clone(),
            provenance: declaration.provenance,
            metadata: declaration.metadata,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Markdown,
    Html,
    Pdf,
    Docx,
    Xlsx,
    Pptx,
    Image,
    Screenshot,
    Csv,
    Json,
    Text,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "origin")]
pub enum ArtifactProvenance {
    Generated {
        generator: String,
        #[serde(default)]
        input_sources: Vec<ProvenanceSource>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        parameters: Metadata,
    },
    Imported {
        source_uri: String,
        imported_by: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_hash: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        retrieved_at: Option<DateTime<Utc>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        license: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvenanceSource {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceMetadata {
    pub file_type: WorkspaceFileType,
    pub byte_len: u64,
    pub readonly: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceFileType {
    File,
    Directory,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ParsedArtifact {
    Json { value: Value },
    Csv(CsvDocument),
    ImageMetadata(ImageMetadata),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvDocument {
    pub headers: Vec<String>,
    pub rows: Vec<CsvRow>,
    pub records: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvRow {
    pub fields: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageMetadata {
    pub width: u32,
    pub height: u32,
    pub color_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserFailure {
    pub kind: ParserFailureKind,
    pub input: WorkspacePath,
    pub format: ParseFormat,
    pub retryability: ParserRetryability,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_capability: Option<String>,
    pub reason: String,
}

impl ParserFailure {
    fn invalid_content(
        input: WorkspacePath,
        format: ParseFormat,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            kind: ParserFailureKind::InvalidContent,
            input,
            format,
            retryability: ParserRetryability::InputChangeRequired,
            missing_capability: None,
            reason: reason.into(),
        }
    }

    pub fn is_resumable(&self) -> bool {
        matches!(
            self.retryability,
            ParserRetryability::ResumableAfterCapabilityAdded
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParserFailureKind {
    InvalidContent,
    MissingCapability,
    UnsupportedFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParserRetryability {
    InputChangeRequired,
    ResumableAfterCapabilityAdded,
    NotRetryable,
}

#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "code")]
pub enum WorkspaceError {
    #[error("authorized workspace root id is empty")]
    EmptyRootId,
    #[error("duplicate authorized workspace root id '{root_id}'")]
    DuplicateRootId { root_id: String },
    #[error("authorized workspace root '{root_id}' at '{path}' is unavailable: {message}")]
    RootUnavailable {
        root_id: String,
        path: String,
        message: String,
    },
    #[error("unknown authorized workspace root '{root_id}'")]
    UnknownRoot { root_id: String },
    #[error("path traversal denied for root '{root_id}': {path}")]
    PathTraversal { root_id: String, path: String },
    #[error("path escaped authorized root '{root_id}': requested '{path}', resolved '{resolved}'")]
    PathEscape {
        root_id: String,
        path: String,
        resolved: String,
        root: String,
    },
    #[error("workspace root '{root_id}' cannot be used as a file operation target")]
    RootTargetDenied { root_id: String },
    #[error("parent path for root '{root_id}' target '{path}' is unavailable: {message}")]
    ParentUnavailable {
        root_id: String,
        path: String,
        message: String,
    },
    #[error("write through symlink denied for root '{root_id}': {path}")]
    SymlinkWriteDenied { root_id: String, path: String },
    #[error("{operation:?} through symlink denied for root '{root_id}': {path}")]
    SymlinkMutationDenied {
        operation: FileOperationKind,
        root_id: String,
        path: String,
    },
    #[error("destination already exists: {path}")]
    DestinationExists { path: String },
    #[error("source and destination are the same path: {path}")]
    SourceDestinationSame { path: String },
    #[error("{operation} failed for '{path}': {message}")]
    Io {
        operation: String,
        path: String,
        message: String,
    },
    #[error("parser failure: {0:?}")]
    Parser(ParserFailure),
}

impl WorkspaceError {
    fn io(operation: impl Into<String>, path: &Path, error: std::io::Error) -> Self {
        Self::Io {
            operation: operation.into(),
            path: path_to_string(path),
            message: error.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ContentDigest {
    byte_len: u64,
    sha256: String,
}

fn evidence_id(operation_id: &str) -> String {
    format!("evidence/{}", operation_id)
}

fn unsupported_parser_failure(input: &WorkspacePath, format: ParseFormat) -> Option<ParserFailure> {
    let capability = match format {
        ParseFormat::Json | ParseFormat::Csv | ParseFormat::ImageMetadata => return None,
        ParseFormat::Pdf => "cowork.parser.pdf",
        ParseFormat::Docx => "cowork.parser.docx",
        ParseFormat::Xlsx => "cowork.parser.xlsx",
        ParseFormat::Screenshot => "cowork.parser.screenshot",
        ParseFormat::Ocr => "cowork.parser.ocr",
    };

    Some(ParserFailure {
        kind: ParserFailureKind::MissingCapability,
        input: input.clone(),
        format,
        retryability: ParserRetryability::ResumableAfterCapabilityAdded,
        missing_capability: Some(capability.to_string()),
        reason: format!(
            "{format:?} extraction is not available in cowork-workspace without capability {capability}"
        ),
    })
}

fn normalize_relative_path(root_id: &str, path: &Path) -> Result<PathBuf, WorkspaceError> {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(WorkspaceError::PathTraversal {
                    root_id: root_id.to_string(),
                    path: path_to_string(path),
                });
            }
        }
    }

    Ok(normalized)
}

fn ensure_under_root(
    root: &AuthorizedRoot,
    requested: &Path,
    resolved: &Path,
) -> Result<(), WorkspaceError> {
    if resolved == root.canonical_path || resolved.starts_with(&root.canonical_path) {
        return Ok(());
    }

    Err(WorkspaceError::PathEscape {
        root_id: root.id.clone(),
        path: path_to_string(requested),
        resolved: path_to_string(resolved),
        root: path_to_string(&root.canonical_path),
    })
}

fn authorize_parent_for_write(
    root: &AuthorizedRoot,
    root_id: &str,
    relative_path: &Path,
    parent: &Path,
) -> Result<PathBuf, WorkspaceError> {
    let relative_parent = relative_path.parent().unwrap_or_else(|| Path::new(""));
    let mut current = root.canonical_path.clone();
    let mut seen_relative = PathBuf::new();

    for component in relative_parent.components() {
        let Component::Normal(part) = component else {
            continue;
        };
        seen_relative.push(part);
        current.push(part);

        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let target = dunce::canonicalize(&current).map_err(|error| {
                    WorkspaceError::ParentUnavailable {
                        root_id: root_id.to_string(),
                        path: path_to_string(&seen_relative),
                        message: error.to_string(),
                    }
                })?;
                ensure_under_root(root, &seen_relative, &target)?;
                current = target;
            }
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                return Err(WorkspaceError::ParentUnavailable {
                    root_id: root_id.to_string(),
                    path: path_to_string(&seen_relative),
                    message: "parent path component is not a directory".to_string(),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(WorkspaceError::ParentUnavailable {
                    root_id: root_id.to_string(),
                    path: path_to_string(&seen_relative),
                    message: error.to_string(),
                });
            }
        }
    }

    fs::create_dir_all(parent).map_err(|error| WorkspaceError::ParentUnavailable {
        root_id: root_id.to_string(),
        path: path_to_string(parent),
        message: error.to_string(),
    })?;

    let canonical_parent =
        dunce::canonicalize(parent).map_err(|error| WorkspaceError::ParentUnavailable {
            root_id: root_id.to_string(),
            path: path_to_string(parent),
            message: error.to_string(),
        })?;
    ensure_under_root(root, relative_path, &canonical_parent)?;
    Ok(canonical_parent)
}

fn sha256_digest(bytes: &[u8]) -> ContentDigest {
    let digest = Sha256::digest(bytes);
    ContentDigest {
        byte_len: bytes.len() as u64,
        sha256: format!("sha256:{}", lowercase_hex(&digest)),
    }
}

fn lowercase_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn parse_csv_document(
    content: &str,
    input: WorkspacePath,
    format: ParseFormat,
) -> Result<CsvDocument, WorkspaceError> {
    let mut records = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut chars = content.chars().peekable();
    let mut in_quotes = false;

    while let Some(ch) = chars.next() {
        if in_quotes {
            match ch {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => in_quotes = false,
                _ => field.push(ch),
            }
            continue;
        }

        match ch {
            '"' if field.is_empty() => in_quotes = true,
            ',' => {
                row.push(std::mem::take(&mut field));
            }
            '\n' => {
                row.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut row));
            }
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                row.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut row));
            }
            _ => field.push(ch),
        }
    }

    if in_quotes {
        return Err(WorkspaceError::Parser(ParserFailure::invalid_content(
            input,
            format,
            "unterminated quoted CSV field",
        )));
    }

    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        records.push(row);
    }

    if records.is_empty() {
        return Err(WorkspaceError::Parser(ParserFailure::invalid_content(
            input,
            format,
            "CSV input is empty",
        )));
    }

    let headers = records.remove(0);
    let rows = records
        .iter()
        .map(|record| {
            let mut fields = BTreeMap::new();
            for (index, header) in headers.iter().enumerate() {
                fields.insert(
                    header.clone(),
                    record.get(index).cloned().unwrap_or_default(),
                );
            }
            CsvRow { fields }
        })
        .collect();

    Ok(CsvDocument {
        headers,
        rows,
        records,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn in_root_operations_succeed_and_emit_evidence() {
        let temp = TestDir::new("ops");
        let workspace = workspace_for(&temp);

        let write = workspace
            .write_file(
                WriteFileRequest::new(
                    "agent/main",
                    "op/write",
                    WorkspacePath::new("main", "docs/input.txt"),
                    b"hello workspace".to_vec(),
                )
                .with_artifact(ArtifactDeclaration::generated(
                    "artifact/generated-note",
                    ArtifactKind::Text,
                    "Generated note",
                    "cowork-test-generator",
                )),
            )
            .expect("write inside root");

        assert_eq!(write.evidence.actor, "agent/main");
        assert_eq!(write.evidence.kind, FileOperationKind::Write);
        assert_eq!(write.evidence.result.byte_len, Some(15));
        assert!(write
            .evidence
            .result
            .sha256
            .as_ref()
            .expect("write hash")
            .starts_with("sha256:"));
        assert_eq!(write.evidence.artifacts.len(), 1);
        assert!(matches!(
            write.evidence.artifacts[0].provenance,
            ArtifactProvenance::Generated { .. }
        ));

        let read = workspace
            .read_file(ReadFileRequest::new(
                "agent/main",
                "op/read",
                WorkspacePath::new("main", "docs/input.txt"),
            ))
            .expect("read inside root");
        assert_eq!(read.bytes, b"hello workspace");
        assert_eq!(read.evidence.kind, FileOperationKind::Read);

        let moved = workspace
            .move_file(MoveFileRequest::new(
                "agent/main",
                "op/move",
                WorkspacePath::new("main", "docs/input.txt"),
                WorkspacePath::new("main", "docs/moved.txt"),
            ))
            .expect("move inside root");
        assert_eq!(moved.evidence.kind, FileOperationKind::Move);
        assert!(moved.evidence.source.is_some());
        assert!(moved.evidence.destination.is_some());
        assert_eq!(moved.evidence.result.byte_len, Some(15));

        let renamed = workspace
            .rename_file(MoveFileRequest::new(
                "agent/main",
                "op/rename",
                WorkspacePath::new("main", "docs/moved.txt"),
                WorkspacePath::new("main", "docs/renamed.txt"),
            ))
            .expect("rename inside root");
        assert_eq!(renamed.evidence.kind, FileOperationKind::Rename);
        assert!(temp.path.join("docs/renamed.txt").exists());

        let metadata = workspace
            .inspect_metadata(MetadataRequest::new(
                "agent/main",
                "op/metadata",
                WorkspacePath::new("main", "docs/renamed.txt"),
            ))
            .expect("metadata inside root");
        assert_eq!(metadata.metadata.file_type, WorkspaceFileType::File);
        assert_eq!(metadata.metadata.byte_len, 15);
        assert!(metadata.metadata.sha256.is_some());
        assert_eq!(metadata.evidence.kind, FileOperationKind::Metadata);
    }

    #[test]
    fn outside_traversal_and_symlink_escape_are_denied_without_writing() {
        let temp = TestDir::new("deny");
        let outside = TestDir::new("outside");
        let workspace = workspace_for(&temp);

        let traversal = workspace.write_file(WriteFileRequest::new(
            "agent/main",
            "op/traversal",
            WorkspacePath::new("main", "../outside/escape.txt"),
            b"escape".to_vec(),
        ));
        assert!(matches!(
            traversal,
            Err(WorkspaceError::PathTraversal { .. })
        ));
        assert!(!outside.path.join("escape.txt").exists());

        let outside_file = outside.path.join("outside.txt");
        fs::write(&outside_file, b"original").expect("write outside file");
        let link = temp.path.join("link.txt");
        create_file_symlink(&outside_file, &link).expect("create symlink");

        let symlink_write = workspace.write_file(WriteFileRequest::new(
            "agent/main",
            "op/symlink-write",
            WorkspacePath::new("main", "link.txt"),
            b"changed".to_vec(),
        ));
        assert!(matches!(
            symlink_write,
            Err(WorkspaceError::SymlinkWriteDenied { .. })
        ));
        assert_eq!(
            fs::read(&outside_file).expect("outside file unchanged"),
            b"original"
        );

        let symlink_read = workspace.read_file(ReadFileRequest::new(
            "agent/main",
            "op/symlink-read",
            WorkspacePath::new("main", "link.txt"),
        ));
        assert!(matches!(
            symlink_read,
            Err(WorkspaceError::PathEscape { .. })
        ));
    }

    #[test]
    fn artifact_manifests_capture_generated_and_imported_provenance() {
        let temp = TestDir::new("manifests");
        let workspace = workspace_for(&temp);

        let generated = workspace
            .write_file(
                WriteFileRequest::new(
                    "agent/main",
                    "op/generated",
                    WorkspacePath::new("main", "generated.json"),
                    br#"{"source":"generated"}"#.to_vec(),
                )
                .with_artifact(ArtifactDeclaration::generated(
                    "artifact/generated-json",
                    ArtifactKind::Json,
                    "Generated JSON",
                    "cowork-generator",
                )),
            )
            .expect("write generated artifact");
        let generated_manifest = &generated.evidence.artifacts[0];
        assert_eq!(
            generated_manifest.schema_version,
            WORKSPACE_SCHEMA_VERSION.to_string()
        );
        assert_eq!(generated_manifest.created_by, "agent/main");
        assert!(matches!(
            generated_manifest.provenance,
            ArtifactProvenance::Generated { .. }
        ));

        let imported = workspace
            .write_file(
                WriteFileRequest::new(
                    "agent/main",
                    "op/imported",
                    WorkspacePath::new("main", "imported.csv"),
                    b"name\nAda\n".to_vec(),
                )
                .with_artifact(ArtifactDeclaration::imported(
                    "artifact/imported-csv",
                    ArtifactKind::Csv,
                    "Imported CSV",
                    "file:///incoming/imported.csv",
                    "agent/main",
                )),
            )
            .expect("write imported artifact");
        let imported_manifest = &imported.evidence.artifacts[0];
        assert!(matches!(
            imported_manifest.provenance,
            ArtifactProvenance::Imported { .. }
        ));
        assert!(imported_manifest.content_hash.starts_with("sha256:"));
        assert_eq!(
            imported_manifest.path.relative_path,
            PathBuf::from("imported.csv")
        );
    }

    #[test]
    fn json_and_csv_parsing_success_returns_structured_output() {
        let temp = TestDir::new("parse");
        let workspace = workspace_for(&temp);

        fs::write(temp.path.join("data.json"), br#"{"ok":true,"count":2}"#)
            .expect("write json fixture");
        fs::write(
            temp.path.join("data.csv"),
            "name,notes\nAda,\"math, computing\"\nGrace,compiler\n",
        )
        .expect("write csv fixture");

        let json = workspace
            .parse_artifact(ParseArtifactRequest::new(
                "agent/main",
                "op/parse-json",
                WorkspacePath::new("main", "data.json"),
                ParseFormat::Json,
            ))
            .expect("parse json");
        match json.parsed {
            ParsedArtifact::Json { value } => {
                assert_eq!(value["ok"], Value::Bool(true));
                assert_eq!(value["count"], Value::from(2));
            }
            other => panic!("unexpected JSON parse output: {other:?}"),
        }
        assert_eq!(json.evidence.kind, FileOperationKind::Parse);

        let csv = workspace
            .parse_artifact(ParseArtifactRequest::new(
                "agent/main",
                "op/parse-csv",
                WorkspacePath::new("main", "data.csv"),
                ParseFormat::Csv,
            ))
            .expect("parse csv");
        match csv.parsed {
            ParsedArtifact::Csv(document) => {
                assert_eq!(document.headers, vec!["name", "notes"]);
                assert_eq!(document.rows.len(), 2);
                assert_eq!(
                    document.rows[0].fields.get("notes").expect("notes field"),
                    "math, computing"
                );
            }
            other => panic!("unexpected CSV parse output: {other:?}"),
        }
    }

    #[test]
    fn unsupported_parsers_return_typed_resumable_failures() {
        let temp = TestDir::new("unsupported");
        let workspace = workspace_for(&temp);

        for (file_name, format) in [
            ("report.pdf", ParseFormat::Pdf),
            ("report.docx", ParseFormat::Docx),
            ("sheet.xlsx", ParseFormat::Xlsx),
            ("screen.png", ParseFormat::Screenshot),
            ("scan.png", ParseFormat::Ocr),
        ] {
            fs::write(temp.path.join(file_name), b"placeholder").expect("write parser fixture");
            let result = workspace.parse_artifact(ParseArtifactRequest::new(
                "agent/main",
                format!("op/unsupported-{format:?}"),
                WorkspacePath::new("main", file_name),
                format,
            ));

            let failure = match result {
                Err(WorkspaceError::Parser(failure)) => failure,
                other => panic!("expected parser failure, got {other:?}"),
            };

            assert_eq!(failure.kind, ParserFailureKind::MissingCapability);
            assert_eq!(
                failure.retryability,
                ParserRetryability::ResumableAfterCapabilityAdded
            );
            assert!(failure.is_resumable());
            assert!(failure.missing_capability.is_some());
            assert_eq!(failure.input.relative_path, PathBuf::from(file_name));
        }
    }

    fn workspace_for(temp: &TestDir) -> AuthorizedWorkspace {
        AuthorizedWorkspace::new([WorkspaceRoot::new("main", temp.path.clone())])
            .expect("authorized workspace")
    }

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(name: &str) -> Self {
            let path = env::temp_dir().join(format!("cowork-workspace-{name}-{}", Uuid::new_v4()));
            fs::create_dir_all(&path).expect("create temp directory");
            Self { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[cfg(unix)]
    fn create_file_symlink(source: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(source, link)
    }

    #[cfg(windows)]
    fn create_file_symlink(source: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(source, link)
    }
}
