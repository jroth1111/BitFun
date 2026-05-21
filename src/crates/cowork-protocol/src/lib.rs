//! Cowork local daemon protocol DTOs.
//!
//! This crate defines the JSON-serializable boundary between GUI/CLI clients
//! and the Cowork daemon. It is deliberately DTO-only: no runtime, network,
//! application, platform, or BitFun core dependencies belong here.
//!
//! Versioning policy:
//! - `major` changes are breaking wire-contract changes.
//! - `minor` changes are backward-compatible additions within the same major
//!   version.
//! - `patch` changes clarify behavior without changing the wire shape.
//! - A daemon accepts peers with the same major version whose minor version is
//!   not newer than the daemon's current minor version.
//!
//! Authority invariant:
//! GUI and CLI clients are request/render surfaces only. Objective, task,
//! evidence, checkpoint, artifact, browser, provider, subagent, and context
//! state is authoritative only when emitted by the daemon. Client-originated
//! payloads must be treated as requests until the daemon records them in a
//! daemon-authored snapshot or event.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

pub const PROTOCOL_NAME: &str = "cowork.daemon";
pub const JSON_RPC_VERSION: &str = "2.0";
pub const PROTOCOL_VERSION_MAJOR: u16 = 1;
pub const PROTOCOL_VERSION_MINOR: u16 = 2;
pub const PROTOCOL_VERSION_PATCH: u16 = 0;

pub type Metadata = BTreeMap<String, serde_json::Value>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl ProtocolVersion {
    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    pub const fn current() -> Self {
        Self::new(
            PROTOCOL_VERSION_MAJOR,
            PROTOCOL_VERSION_MINOR,
            PROTOCOL_VERSION_PATCH,
        )
    }

    pub fn compatibility_with(self, peer: ProtocolVersion) -> VersionCompatibility {
        if peer.major != self.major {
            return VersionCompatibility::MajorMismatch {
                supported_major: self.major,
                peer_major: peer.major,
            };
        }

        if peer.minor > self.minor {
            return VersionCompatibility::PeerNewerMinor {
                supported: self,
                peer,
            };
        }

        VersionCompatibility::Compatible
    }

    pub fn is_compatible_with(self, peer: ProtocolVersion) -> bool {
        matches!(
            self.compatibility_with(peer),
            VersionCompatibility::Compatible
        )
    }

    pub fn ensure_compatible_with(self, peer: ProtocolVersion) -> Result<(), ProtocolError> {
        match self.compatibility_with(peer) {
            VersionCompatibility::Compatible => Ok(()),
            compatibility => Err(ProtocolError::UnsupportedVersion {
                requested: peer,
                supported: self,
                compatibility,
            }),
        }
    }
}

impl Default for ProtocolVersion {
    fn default() -> Self {
        Self::current()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum VersionCompatibility {
    Compatible,
    MajorMismatch {
        supported_major: u16,
        peer_major: u16,
    },
    PeerNewerMinor {
        supported: ProtocolVersion,
        peer: ProtocolVersion,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolErrorCode {
    UnsupportedVersion,
    InvalidRequest,
    PermissionDenied,
    NotFound,
    Conflict,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "code")]
pub enum ProtocolError {
    #[error("unsupported protocol version {requested:?}; supported {supported:?}")]
    UnsupportedVersion {
        requested: ProtocolVersion,
        supported: ProtocolVersion,
        compatibility: VersionCompatibility,
    },
    #[error("invalid request: {message}")]
    InvalidRequest { message: String },
    #[error("permission denied: {message}")]
    PermissionDenied { message: String },
    #[error("not found: {resource}")]
    NotFound { resource: String },
    #[error("conflict: {message}")]
    Conflict { message: String },
    #[error("internal protocol error: {message}")]
    Internal { message: String },
}

impl ProtocolError {
    pub fn code(&self) -> ProtocolErrorCode {
        match self {
            Self::UnsupportedVersion { .. } => ProtocolErrorCode::UnsupportedVersion,
            Self::InvalidRequest { .. } => ProtocolErrorCode::InvalidRequest,
            Self::PermissionDenied { .. } => ProtocolErrorCode::PermissionDenied,
            Self::NotFound { .. } => ProtocolErrorCode::NotFound,
            Self::Conflict { .. } => ProtocolErrorCode::Conflict,
            Self::Internal { .. } => ProtocolErrorCode::Internal,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolEnvelope<T> {
    pub jsonrpc: String,
    pub id: String,
    pub protocol: String,
    pub protocol_version: ProtocolVersion,
    pub authority: AuthorityInvariant,
    #[serde(flatten)]
    pub message: ProtocolMessage<T>,
}

impl<T> ProtocolEnvelope<T> {
    pub fn request(id: impl Into<String>, method: impl Into<String>, params: T) -> Self {
        Self {
            jsonrpc: JSON_RPC_VERSION.to_string(),
            id: id.into(),
            protocol: PROTOCOL_NAME.to_string(),
            protocol_version: ProtocolVersion::current(),
            authority: AuthorityInvariant::daemon_authoritative(),
            message: ProtocolMessage::Request {
                method: method.into(),
                params,
            },
        }
    }

    pub fn response(id: impl Into<String>, result: T) -> Self {
        Self {
            jsonrpc: JSON_RPC_VERSION.to_string(),
            id: id.into(),
            protocol: PROTOCOL_NAME.to_string(),
            protocol_version: ProtocolVersion::current(),
            authority: AuthorityInvariant::daemon_authoritative(),
            message: ProtocolMessage::Response { result },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ProtocolMessage<T> {
    Request { method: String, params: T },
    Response { result: T },
    Error { error: ProtocolError },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonAuthority {
    Daemon,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientAuthority {
    RequestAndRenderOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorityInvariant {
    pub objective_authority: DaemonAuthority,
    pub task_authority: DaemonAuthority,
    pub evidence_authority: DaemonAuthority,
    pub checkpoint_authority: DaemonAuthority,
    pub artifact_authority: DaemonAuthority,
    pub context_authority: DaemonAuthority,
    pub client_authority: ClientAuthority,
}

impl AuthorityInvariant {
    pub const fn daemon_authoritative() -> Self {
        Self {
            objective_authority: DaemonAuthority::Daemon,
            task_authority: DaemonAuthority::Daemon,
            evidence_authority: DaemonAuthority::Daemon,
            checkpoint_authority: DaemonAuthority::Daemon,
            artifact_authority: DaemonAuthority::Daemon,
            context_authority: DaemonAuthority::Daemon,
            client_authority: ClientAuthority::RequestAndRenderOnly,
        }
    }
}

impl Default for AuthorityInvariant {
    fn default() -> Self {
        Self::daemon_authoritative()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoworkSnapshot {
    pub protocol_version: ProtocolVersion,
    pub authority: AuthorityInvariant,
    pub objective: Objective,
    #[serde(default)]
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    #[serde(default)]
    pub checkpoints: Vec<Checkpoint>,
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    #[serde(default)]
    pub subagent_registry: SubagentRegistry,
    #[serde(default)]
    pub subagents: Vec<Subagent>,
    #[serde(default)]
    pub providers: Vec<Provider>,
    pub browser: BrowserState,
    pub run_status: RunStatus,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRetrievalRequest {
    pub target: HistoryRetrievalTarget,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_range: Option<HistorySequenceRange>,
    pub include_linked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryRetrievalTarget {
    All,
    Objective,
    Task,
    Evidence,
    Checkpoint,
    Artifact,
    Subagent,
    Provider,
    Browser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySequenceRange {
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRetrievalResult {
    pub request: HistoryRetrievalRequest,
    pub objective: Option<Objective>,
    #[serde(default)]
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    #[serde(default)]
    pub checkpoints: Vec<Checkpoint>,
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    #[serde(default)]
    pub subagents: Vec<Subagent>,
    #[serde(default)]
    pub providers: Vec<Provider>,
    pub browser: Option<BrowserState>,
    pub result_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Objective {
    pub id: String,
    pub instruction: String,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    pub status: ObjectiveStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveStatus {
    Draft,
    Active,
    Paused,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub objective_id: String,
    pub title: String,
    pub status: TaskStatus,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
    #[serde(default)]
    pub artifact_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assigned_subagent_id: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    NotStarted,
    InProgress,
    Blocked,
    ImplementedUnverified,
    Verified,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub id: String,
    pub kind: EvidenceKind,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_id: Option<String>,
    #[serde(default)]
    pub artifact_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Command,
    Test,
    ManualRuntime,
    Inspection,
    ArtifactValidation,
    UserApproval,
    SubagentReport,
    BrowserAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Checkpoint {
    pub id: String,
    pub objective_id: String,
    pub sequence: u64,
    pub summary: String,
    #[serde(default)]
    pub task_ids: Vec<String>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
    #[serde(default)]
    pub artifact_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub id: String,
    pub kind: ArtifactKind,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
    pub provenance: ArtifactProvenance,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
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
    Diff,
    Json,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactProvenance {
    pub objective_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subagent {
    pub id: String,
    pub name: String,
    pub role: String,
    pub status: SubagentStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_evidence_id: Option<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub denied_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentRegistry {
    #[serde(default)]
    pub definitions: Vec<SubagentDefinition>,
}

impl SubagentRegistry {
    pub fn validate(&self) -> Result<(), SubagentRegistryError> {
        let mut ids = BTreeMap::<&str, ()>::new();
        for definition in &self.definitions {
            definition.validate()?;
            if ids.insert(definition.id.as_str(), ()).is_some() {
                return Err(SubagentRegistryError::DuplicateDefinitionId {
                    id: definition.id.clone(),
                });
            }
        }

        for definition in &self.definitions {
            if let Some(parent_id) = &definition.parent_subagent_id {
                if parent_id == &definition.id {
                    return Err(SubagentRegistryError::InvalidParentSubagent {
                        id: definition.id.clone(),
                        parent_id: parent_id.clone(),
                        reason: "definition cannot name itself as parent".to_string(),
                    });
                }
                if !ids.contains_key(parent_id.as_str()) {
                    return Err(SubagentRegistryError::InvalidParentSubagent {
                        id: definition.id.clone(),
                        parent_id: parent_id.clone(),
                        reason: "parent subagent definition is not registered".to_string(),
                    });
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentDefinition {
    pub id: String,
    pub name: String,
    pub role: String,
    pub prompt: String,
    pub model: String,
    pub budget: SubagentBudget,
    pub workspace: SubagentWorkspace,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_subagent_id: Option<String>,
    pub lifecycle_state: SubagentLifecycleState,
    #[serde(default)]
    pub tool_allowlist: Vec<String>,
    #[serde(default)]
    pub tool_denylist: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

impl SubagentDefinition {
    pub fn validate(&self) -> Result<(), SubagentRegistryError> {
        require_non_empty("id", &self.id)?;
        require_non_empty("name", &self.name)?;
        require_non_empty("role", &self.role)?;
        require_non_empty("prompt", &self.prompt)?;
        require_clean_token("model", &self.model)?;
        self.budget.validate()?;
        self.workspace.validate()?;
        if let Some(parent_task_id) = &self.parent_task_id {
            require_non_empty("parentTaskId", parent_task_id)?;
        }
        validate_tool_set("toolAllowlist", &self.tool_allowlist)?;
        validate_tool_set("toolDenylist", &self.tool_denylist)?;
        for tool in &self.tool_allowlist {
            if self.tool_denylist.iter().any(|denied| denied == tool) {
                return Err(SubagentRegistryError::ToolPolicyConflict { tool: tool.clone() });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentLifecycleState {
    Defined,
    Scheduled,
    Running,
    WaitingForReport,
    Reported,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentBudget {
    pub max_turns: u32,
    pub max_tool_calls: u32,
    pub max_output_tokens: u32,
}

impl SubagentBudget {
    pub const fn new(max_turns: u32, max_tool_calls: u32, max_output_tokens: u32) -> Self {
        Self {
            max_turns,
            max_tool_calls,
            max_output_tokens,
        }
    }

    fn validate(&self) -> Result<(), SubagentRegistryError> {
        if self.max_turns == 0 || self.max_tool_calls == 0 || self.max_output_tokens == 0 {
            return Err(SubagentRegistryError::InvalidBudget {
                reason: "budget limits must be positive".to_string(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentWorkspace {
    pub root_id: String,
    pub uri: String,
}

impl SubagentWorkspace {
    pub fn new(root_id: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            root_id: root_id.into(),
            uri: uri.into(),
        }
    }

    fn validate(&self) -> Result<(), SubagentRegistryError> {
        require_non_empty("workspace.rootId", &self.root_id)?;
        require_non_empty("workspace.uri", &self.uri)?;
        if !self.uri.starts_with("workspace://") {
            return Err(SubagentRegistryError::InvalidWorkspace {
                uri: self.uri.clone(),
                reason: "workspace uri must start with workspace://".to_string(),
            });
        }
        if self.uri.contains("..") {
            return Err(SubagentRegistryError::InvalidWorkspace {
                uri: self.uri.clone(),
                reason: "workspace uri must not contain traversal segments".to_string(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SubagentRegistryError {
    #[error("subagent definition field '{field}' cannot be empty")]
    EmptyField { field: &'static str },
    #[error("subagent definition field '{field}' contains an invalid token '{value}'")]
    InvalidToken { field: &'static str, value: String },
    #[error("duplicate subagent definition id '{id}'")]
    DuplicateDefinitionId { id: String },
    #[error("invalid subagent workspace '{uri}': {reason}")]
    InvalidWorkspace { uri: String, reason: String },
    #[error("invalid subagent budget: {reason}")]
    InvalidBudget { reason: String },
    #[error("invalid parent subagent for '{id}' -> '{parent_id}': {reason}")]
    InvalidParentSubagent {
        id: String,
        parent_id: String,
        reason: String,
    },
    #[error("invalid subagent tool policy entry in '{field}': '{tool}'")]
    InvalidToolPolicyEntry { field: &'static str, tool: String },
    #[error("subagent tool '{tool}' appears in both allowlist and denylist")]
    ToolPolicyConflict { tool: String },
}

fn require_non_empty(field: &'static str, value: &str) -> Result<(), SubagentRegistryError> {
    if value.trim().is_empty() {
        return Err(SubagentRegistryError::EmptyField { field });
    }
    Ok(())
}

fn require_clean_token(field: &'static str, value: &str) -> Result<(), SubagentRegistryError> {
    require_non_empty(field, value)?;
    if value.chars().any(char::is_control) {
        return Err(SubagentRegistryError::InvalidToken {
            field,
            value: value.to_string(),
        });
    }
    Ok(())
}

fn validate_tool_set(field: &'static str, tools: &[String]) -> Result<(), SubagentRegistryError> {
    for tool in tools {
        require_non_empty(field, tool)?;
        if tool.chars().any(char::is_whitespace) || tool.chars().any(char::is_control) {
            return Err(SubagentRegistryError::InvalidToolPolicyEntry {
                field,
                tool: tool.clone(),
            });
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    Idle,
    Running,
    WaitingForReport,
    Reported,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    pub id: String,
    pub kind: ProviderKind,
    pub label: String,
    pub status: ProviderStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_model: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<ProviderCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Byok,
    OpenAiCompatible,
    SubscriptionCli,
    LocalModel,
    Connector,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStatus {
    Available,
    NeedsAuth,
    RateLimited,
    Disabled,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCapability {
    Text,
    Vision,
    ToolUse,
    FileInput,
    LongContext,
    Embeddings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserState {
    #[serde(default)]
    pub sessions: Vec<BrowserSession>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_session_id: Option<String>,
    pub status: BrowserStatus,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSession {
    pub id: String,
    pub adapter: BrowserAdapter,
    pub status: BrowserStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_tab_id: Option<String>,
    #[serde(default)]
    pub tabs: Vec<BrowserTab>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAdapter {
    ManagedProfileCdp,
    ExtensionBridge,
    ExternalCdp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserStatus {
    Unavailable,
    Available,
    NeedsUserAuth,
    WaitingForUser,
    Running,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTab {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Idle,
    Running,
    Paused,
    Blocked,
    Completed,
    Cancelled,
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_and_snapshot_json_roundtrip() {
        let snapshot = fixture_snapshot();
        let envelope = ProtocolEnvelope::response("snapshot-1", snapshot);

        let json = serde_json::to_string(&envelope).expect("serialize envelope");
        let decoded: ProtocolEnvelope<CoworkSnapshot> =
            serde_json::from_str(&json).expect("deserialize envelope");

        assert_eq!(decoded, envelope);
        assert_eq!(decoded.jsonrpc, JSON_RPC_VERSION);
        assert_eq!(decoded.protocol, PROTOCOL_NAME);
    }

    #[test]
    fn version_compatibility_policy() {
        let current = ProtocolVersion::current();
        let older_patch = ProtocolVersion::new(current.major, current.minor, 0);
        let next_minor = ProtocolVersion::new(current.major, current.minor + 1, 0);
        let next_major = ProtocolVersion::new(current.major + 1, 0, 0);

        assert!(current.is_compatible_with(older_patch));
        assert_eq!(
            current.compatibility_with(next_minor),
            VersionCompatibility::PeerNewerMinor {
                supported: current,
                peer: next_minor,
            }
        );
        assert_eq!(
            current.compatibility_with(next_major),
            VersionCompatibility::MajorMismatch {
                supported_major: current.major,
                peer_major: next_major.major,
            }
        );

        let error = current.ensure_compatible_with(next_major).unwrap_err();
        assert_eq!(error.code(), ProtocolErrorCode::UnsupportedVersion);
    }

    #[test]
    fn snapshot_includes_all_required_domain_sections() {
        let snapshot = fixture_snapshot();

        assert_eq!(
            snapshot.authority.client_authority,
            ClientAuthority::RequestAndRenderOnly
        );
        assert_eq!(
            snapshot.authority.context_authority,
            DaemonAuthority::Daemon
        );
        assert!(!snapshot.objective.id.is_empty());
        assert_eq!(snapshot.tasks.len(), 1);
        assert_eq!(snapshot.evidence.len(), 1);
        assert_eq!(snapshot.checkpoints.len(), 1);
        assert_eq!(snapshot.artifacts.len(), 1);
        assert_eq!(snapshot.subagents.len(), 1);
        assert_eq!(snapshot.providers.len(), 1);
        assert_eq!(snapshot.browser.sessions.len(), 1);
        assert_eq!(snapshot.run_status, RunStatus::Running);
    }

    #[test]
    fn history_retrieval_request_and_result_are_protocol_serializable() {
        let snapshot = fixture_snapshot();
        let request = HistoryRetrievalRequest {
            target: HistoryRetrievalTarget::Task,
            id: Some("task-1".to_string()),
            query: None,
            sequence_range: None,
            include_linked: true,
        };
        let result = HistoryRetrievalResult {
            request: request.clone(),
            objective: Some(snapshot.objective.clone()),
            tasks: snapshot.tasks.clone(),
            evidence: snapshot.evidence.clone(),
            checkpoints: snapshot.checkpoints.clone(),
            artifacts: snapshot.artifacts.clone(),
            subagents: Vec::new(),
            providers: Vec::new(),
            browser: None,
            result_count: 4,
        };

        let envelope = ProtocolEnvelope::response("history-1", result.clone());
        let json = serde_json::to_string(&envelope).expect("serialize history envelope");
        let decoded: ProtocolEnvelope<HistoryRetrievalResult> =
            serde_json::from_str(&json).expect("deserialize history envelope");

        assert_eq!(decoded.message, ProtocolMessage::Response { result });
        assert!(json.contains("\"target\":\"task\""));
        assert!(json.contains("\"includeLinked\":true"));
    }

    fn fixture_snapshot() -> CoworkSnapshot {
        let objective_id = "objective-1".to_string();
        let task_id = "task-1".to_string();
        let evidence_id = "evidence-1".to_string();
        let artifact_id = "artifact-1".to_string();
        let checkpoint_id = "checkpoint-1".to_string();
        let subagent_id = "subagent-1".to_string();

        CoworkSnapshot {
            protocol_version: ProtocolVersion::current(),
            authority: AuthorityInvariant::daemon_authoritative(),
            objective: Objective {
                id: objective_id.clone(),
                instruction: "Produce a source-backed brief.".to_string(),
                constraints: vec!["Preserve daemon authority.".to_string()],
                acceptance_criteria: vec!["Evidence is linked to tasks.".to_string()],
                status: ObjectiveStatus::Active,
                created_at: Some("2026-05-21T00:00:00Z".to_string()),
                metadata: Metadata::new(),
            },
            tasks: vec![Task {
                id: task_id.clone(),
                objective_id: objective_id.clone(),
                title: "Collect evidence".to_string(),
                status: TaskStatus::InProgress,
                dependencies: Vec::new(),
                evidence_ids: vec![evidence_id.clone()],
                artifact_ids: vec![artifact_id.clone()],
                assigned_subagent_id: Some(subagent_id.clone()),
                metadata: Metadata::new(),
            }],
            evidence: vec![Evidence {
                id: evidence_id.clone(),
                kind: EvidenceKind::Inspection,
                summary: "Protocol DTOs include authority fields.".to_string(),
                task_id: Some(task_id.clone()),
                checkpoint_id: Some(checkpoint_id.clone()),
                artifact_ids: vec![artifact_id.clone()],
                created_at: Some("2026-05-21T00:01:00Z".to_string()),
                metadata: Metadata::new(),
            }],
            checkpoints: vec![Checkpoint {
                id: checkpoint_id,
                objective_id: objective_id.clone(),
                sequence: 1,
                summary: "Initial daemon protocol snapshot.".to_string(),
                task_ids: vec![task_id.clone()],
                evidence_ids: vec![evidence_id.clone()],
                artifact_ids: vec![artifact_id.clone()],
                created_at: Some("2026-05-21T00:02:00Z".to_string()),
                metadata: Metadata::new(),
            }],
            artifacts: vec![Artifact {
                id: artifact_id,
                kind: ArtifactKind::Markdown,
                title: "Brief".to_string(),
                uri: Some("file:///tmp/brief.md".to_string()),
                evidence_ids: vec![evidence_id],
                provenance: ArtifactProvenance {
                    objective_id,
                    task_id: Some(task_id.clone()),
                    evidence_id: Some("evidence-1".to_string()),
                },
                metadata: Metadata::new(),
            }],
            subagent_registry: fixture_subagent_registry(&subagent_id, &task_id),
            subagents: vec![Subagent {
                id: subagent_id,
                name: "researcher".to_string(),
                role: "Collect source material".to_string(),
                status: SubagentStatus::Running,
                parent_task_id: Some(task_id.clone()),
                report_evidence_id: None,
                allowed_tools: vec!["browser".to_string()],
                denied_tools: Vec::new(),
                metadata: Metadata::new(),
            }],
            providers: vec![Provider {
                id: "provider-1".to_string(),
                kind: ProviderKind::OpenAiCompatible,
                label: "local gateway".to_string(),
                status: ProviderStatus::Available,
                selected_model: Some("cowork-model".to_string()),
                capabilities: vec![ProviderCapability::Text, ProviderCapability::ToolUse],
                credential_ref: Some("keychain://cowork/provider-1".to_string()),
                metadata: Metadata::new(),
            }],
            browser: BrowserState {
                sessions: vec![BrowserSession {
                    id: "browser-1".to_string(),
                    adapter: BrowserAdapter::ManagedProfileCdp,
                    status: BrowserStatus::Running,
                    profile_id: Some("profile-1".to_string()),
                    active_tab_id: Some("tab-1".to_string()),
                    tabs: vec![BrowserTab {
                        id: "tab-1".to_string(),
                        title: Some("Example".to_string()),
                        url: Some("https://example.com".to_string()),
                        task_id: Some(task_id),
                    }],
                }],
                active_session_id: Some("browser-1".to_string()),
                status: BrowserStatus::Running,
                metadata: Metadata::new(),
            },
            run_status: RunStatus::Running,
            metadata: Metadata::new(),
        }
    }

    fn fixture_subagent_registry(subagent_id: &str, task_id: &str) -> SubagentRegistry {
        SubagentRegistry {
            definitions: vec![SubagentDefinition {
                id: subagent_id.to_string(),
                name: "researcher".to_string(),
                role: "Collect source material".to_string(),
                prompt: "Find source-backed evidence and report with citations.".to_string(),
                model: "cowork-model".to_string(),
                budget: SubagentBudget::new(8, 24, 32_000),
                workspace: SubagentWorkspace::new("project", "workspace://project"),
                parent_task_id: Some(task_id.to_string()),
                parent_subagent_id: None,
                lifecycle_state: SubagentLifecycleState::Running,
                tool_allowlist: vec!["browser".to_string(), "session_history.read".to_string()],
                tool_denylist: vec!["shell.exec".to_string()],
                metadata: Metadata::new(),
            }],
        }
    }

    #[test]
    fn subagent_registry_validates_definition_graph() {
        let registry = SubagentRegistry {
            definitions: vec![
                SubagentDefinition {
                    id: "subagent-parent".to_string(),
                    name: "planner".to_string(),
                    role: "Plan worker execution".to_string(),
                    prompt: "Plan the delegated work.".to_string(),
                    model: "cowork-model".to_string(),
                    budget: SubagentBudget::new(4, 8, 8_000),
                    workspace: SubagentWorkspace::new("project", "workspace://project"),
                    parent_task_id: Some("task-1".to_string()),
                    parent_subagent_id: None,
                    lifecycle_state: SubagentLifecycleState::Defined,
                    tool_allowlist: vec!["session_history.read".to_string()],
                    tool_denylist: vec!["shell.exec".to_string()],
                    metadata: Metadata::new(),
                },
                SubagentDefinition {
                    id: "subagent-child".to_string(),
                    name: "implementer".to_string(),
                    role: "Implement scoped changes".to_string(),
                    prompt: "Implement only the assigned write scope.".to_string(),
                    model: "cowork-model".to_string(),
                    budget: SubagentBudget::new(8, 24, 32_000),
                    workspace: SubagentWorkspace::new("project", "workspace://project/src"),
                    parent_task_id: Some("task-1".to_string()),
                    parent_subagent_id: Some("subagent-parent".to_string()),
                    lifecycle_state: SubagentLifecycleState::Scheduled,
                    tool_allowlist: vec![
                        "workspace.read".to_string(),
                        "workspace.write".to_string(),
                    ],
                    tool_denylist: vec!["git.push".to_string()],
                    metadata: Metadata::new(),
                },
            ],
        };

        registry.validate().expect("valid subagent registry");
    }

    #[test]
    fn subagent_registry_rejects_invalid_model_workspace_tools_budget_and_parents() {
        let mut invalid = fixture_subagent_registry("subagent-1", "task-1");
        invalid.definitions[0].model = "\n".to_string();
        assert!(matches!(
            invalid.validate(),
            Err(SubagentRegistryError::EmptyField { field: "model" })
        ));

        let mut invalid = fixture_subagent_registry("subagent-1", "task-1");
        invalid.definitions[0].workspace.uri = "file:///tmp/project".to_string();
        assert!(matches!(
            invalid.validate(),
            Err(SubagentRegistryError::InvalidWorkspace { .. })
        ));

        let mut invalid = fixture_subagent_registry("subagent-1", "task-1");
        invalid.definitions[0]
            .tool_denylist
            .push("browser".to_string());
        assert!(matches!(
            invalid.validate(),
            Err(SubagentRegistryError::ToolPolicyConflict { tool }) if tool == "browser"
        ));

        let mut invalid = fixture_subagent_registry("subagent-1", "task-1");
        invalid.definitions[0].budget.max_turns = 0;
        assert!(matches!(
            invalid.validate(),
            Err(SubagentRegistryError::InvalidBudget { .. })
        ));

        let mut invalid = fixture_subagent_registry("subagent-1", "task-1");
        invalid.definitions.push(invalid.definitions[0].clone());
        assert!(matches!(
            invalid.validate(),
            Err(SubagentRegistryError::DuplicateDefinitionId { .. })
        ));

        let mut invalid = fixture_subagent_registry("subagent-1", "task-1");
        invalid.definitions[0].parent_subagent_id = Some("missing-parent".to_string());
        assert!(matches!(
            invalid.validate(),
            Err(SubagentRegistryError::InvalidParentSubagent { .. })
        ));
    }
}
