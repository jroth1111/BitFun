//! Kernel-owned plugin and extension manifest registry for Cowork.
//!
//! This crate owns DTOs and in-memory registry rules for capability packs. It
//! intentionally does not execute plugins, load processes, touch the network,
//! or integrate with GUI/CLI/daemon runtime code.

use cowork_governance::{
    ActionKind, ActionRequest, ActionRisk, ActorRole, ActorSelector, ActorStatus, ApprovalRecord,
    DecisionOutcome, EvidenceBundle, GateKind, GateRequirements, GovernanceActor,
    GovernanceAuditExport, GovernanceManager, PolicyRule, Resource, ResourceKind, ResourceSelector,
    ScopePattern,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

pub type Metadata = BTreeMap<String, serde_json::Value>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub version_evidence: VersionEvidence,
    pub source: SourceMetadata,
    #[serde(default)]
    pub capabilities: Vec<CapabilityManifest>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

impl PluginManifest {
    pub fn validate(&self) -> Result<(), ManifestValidationError> {
        require_id("plugin.id", &self.id)?;
        require_text("plugin.name", &self.name)?;
        require_text("plugin.version", &self.version)?;
        self.version_evidence.validate()?;
        self.source.validate()?;

        if self.capabilities.is_empty() {
            return Err(ManifestValidationError::MissingCapabilities {
                plugin_id: self.id.clone(),
            });
        }

        let mut capability_ids = BTreeSet::new();
        for capability in &self.capabilities {
            capability.validate(&self.id)?;
            if !capability_ids.insert(capability.id.clone()) {
                return Err(ManifestValidationError::DuplicateCapability {
                    plugin_id: self.id.clone(),
                    capability_id: capability.id.clone(),
                });
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionEvidence {
    pub manifest_schema: String,
    pub manifest_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub evidence_links: Vec<EvidenceLink>,
}

impl VersionEvidence {
    pub fn validate(&self) -> Result<(), ManifestValidationError> {
        require_text("versionEvidence.manifestSchema", &self.manifest_schema)?;
        require_text("versionEvidence.manifestVersion", &self.manifest_version)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceLink {
    pub label: String,
    pub uri: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceMetadata {
    pub kind: SourceKind,
    pub origin: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    pub legal: LegalMetadata,
    #[serde(default)]
    pub imported_components: Vec<ImportedComponent>,
}

impl SourceMetadata {
    pub fn validate(&self) -> Result<(), ManifestValidationError> {
        require_text("source.origin", &self.origin)?;

        if matches!(self.kind, SourceKind::Imported | SourceKind::Mixed) {
            self.legal.require_links("source.legal")?;
        }

        for component in &self.imported_components {
            component.validate()?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Native,
    Imported,
    Mixed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegalMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license_expression: Option<String>,
    #[serde(default)]
    pub notice_links: Vec<LegalLink>,
    #[serde(default)]
    pub source_links: Vec<LegalLink>,
}

impl LegalMetadata {
    pub fn require_links(&self, field: &'static str) -> Result<(), ManifestValidationError> {
        if self.notice_links.is_empty() && self.source_links.is_empty() {
            return Err(ManifestValidationError::MissingLegalMetadata {
                field,
                reason: "imported code/assets require notice or source legal links".to_string(),
            });
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegalLink {
    pub label: String,
    pub uri: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedComponent {
    pub id: String,
    pub kind: ImportedComponentKind,
    pub source_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default)]
    pub legal_links: Vec<LegalLink>,
}

impl ImportedComponent {
    pub fn validate(&self) -> Result<(), ManifestValidationError> {
        require_id("importedComponent.id", &self.id)?;
        require_text("importedComponent.sourceUri", &self.source_uri)?;

        if self.legal_links.is_empty() {
            return Err(ManifestValidationError::MissingLegalMetadata {
                field: "importedComponent.legalLinks",
                reason: format!("imported component '{}' must link legal metadata", self.id),
            });
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportedComponentKind {
    Code,
    Asset,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityManifest {
    pub id: String,
    pub kind: CapabilityKind,
    pub display_name: String,
    pub version: String,
    #[serde(default)]
    pub permissions: BTreeSet<Permission>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

impl CapabilityManifest {
    pub fn validate(&self, plugin_id: &str) -> Result<(), ManifestValidationError> {
        require_id("capability.id", &self.id)?;
        require_text("capability.displayName", &self.display_name)?;
        require_text("capability.version", &self.version)?;

        if self.id.starts_with("kernel.") || self.id.starts_with("cowork.kernel.") {
            return Err(ManifestValidationError::ReservedCapabilityId {
                plugin_id: plugin_id.to_string(),
                capability_id: self.id.clone(),
            });
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    Skill,
    Tool,
    McpServer,
    Connector,
    BrowserDriver,
    ProviderAdapter,
    ResearchProvider,
    SubagentPack,
    ArtifactPack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKind {
    WorkspaceRead,
    WorkspaceWrite,
    ProcessSpawn,
    Network,
    BrowserProfile,
    ProviderCredential,
    ConnectorAccount,
    ArtifactWrite,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Permission {
    pub kind: PermissionKind,
    pub scope: PermissionScope,
}

impl Permission {
    pub fn new(kind: PermissionKind, scope: impl Into<String>) -> Self {
        Self {
            kind,
            scope: PermissionScope::Pattern(scope.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum PermissionScope {
    Any,
    Pattern(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginDirectory {
    pub root: String,
    pub manifest_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assets_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_dir: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginState {
    Disabled,
    Enabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredPlugin {
    pub manifest: PluginManifest,
    pub directory: PluginDirectory,
    pub state: PluginState,
    #[serde(default)]
    granted_permissions: BTreeSet<Permission>,
}

impl RegisteredPlugin {
    pub fn granted_permissions(&self) -> &BTreeSet<Permission> {
        &self.granted_permissions
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredCapability {
    pub plugin_id: String,
    pub capability: CapabilityManifest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionPermit {
    pub plugin_id: String,
    pub capability_id: String,
    pub capability_kind: CapabilityKind,
    pub granted_permissions: BTreeSet<Permission>,
    #[serde(default)]
    pub governance_decision_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreExecutionContext {
    #[serde(default)]
    pub permission_approvals: BTreeMap<Permission, String>,
    #[serde(default)]
    pub evidence: EvidenceBundle,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub justification: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistrySnapshot {
    pub plugins: Vec<RegisteredPlugin>,
    pub enabled_capabilities: Vec<RegisteredCapability>,
}

#[derive(Debug, Default, Clone)]
pub struct PluginRegistry {
    plugins: BTreeMap<String, RegisteredPlugin>,
    enabled_capabilities: BTreeMap<String, String>,
    governance: GovernanceManager,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_manifest(
        &mut self,
        manifest: PluginManifest,
        directory: PluginDirectory,
    ) -> Result<(), RegistryError> {
        manifest
            .validate()
            .map_err(RegistryError::InvalidManifest)?;

        if self.plugins.contains_key(&manifest.id) {
            return Err(RegistryError::PluginAlreadyRegistered {
                plugin_id: manifest.id,
            });
        }

        let plugin_id = manifest.id.clone();
        let actor = plugin_actor(&manifest, ActorStatus::Disabled);
        self.plugins.insert(
            plugin_id,
            RegisteredPlugin {
                manifest,
                directory,
                state: PluginState::Disabled,
                granted_permissions: BTreeSet::new(),
            },
        );
        self.governance.policy_mut().upsert_actor(actor);

        Ok(())
    }

    pub fn enable_plugin(&mut self, plugin_id: &str) -> Result<(), RegistryError> {
        let (capability_ids, actor) = {
            let plugin = self.plugin(plugin_id)?;
            (
                plugin
                    .manifest
                    .capabilities
                    .iter()
                    .map(|capability| capability.id.clone())
                    .collect::<Vec<_>>(),
                plugin_actor(&plugin.manifest, ActorStatus::Enabled),
            )
        };

        for capability_id in &capability_ids {
            if let Some(existing_plugin_id) = self.enabled_capabilities.get(capability_id) {
                if existing_plugin_id != plugin_id {
                    return Err(RegistryError::CapabilityConflict {
                        capability_id: capability_id.clone(),
                        existing_plugin_id: existing_plugin_id.clone(),
                        requested_plugin_id: plugin_id.to_string(),
                    });
                }
            }
        }

        {
            let plugin = self.plugin_mut(plugin_id)?;
            plugin.state = PluginState::Enabled;
        }

        for capability_id in capability_ids {
            self.enabled_capabilities
                .insert(capability_id.clone(), plugin_id.to_string());
            self.governance
                .policy_mut()
                .upsert_rule(plugin_execution_rule(plugin_id, &capability_id));
        }
        self.governance.policy_mut().upsert_actor(actor);

        Ok(())
    }

    pub fn disable_plugin(&mut self, plugin_id: &str) -> Result<(), RegistryError> {
        {
            let plugin = self.plugin_mut(plugin_id)?;
            plugin.state = PluginState::Disabled;
        }
        self.enabled_capabilities
            .retain(|_, owner_plugin_id| owner_plugin_id != plugin_id);
        self.governance
            .policy_mut()
            .set_actor_status(plugin_id, ActorStatus::Disabled)
            .map_err(|error| RegistryError::GovernancePolicy(error.to_string()))?;
        Ok(())
    }

    pub fn unload_plugin(&mut self, plugin_id: &str) -> Result<RegisteredPlugin, RegistryError> {
        let removed =
            self.plugins
                .remove(plugin_id)
                .ok_or_else(|| RegistryError::PluginNotFound {
                    plugin_id: plugin_id.to_string(),
                })?;
        self.enabled_capabilities
            .retain(|_, owner_plugin_id| owner_plugin_id != plugin_id);
        self.governance.policy_mut().remove_actor(plugin_id);
        self.governance
            .policy_mut()
            .remove_rules_with_prefix(&plugin_rule_prefix(plugin_id));
        Ok(removed)
    }

    pub fn grant_permission(
        &mut self,
        plugin_id: &str,
        capability_id: &str,
        permission: Permission,
    ) -> Result<(), RegistryError> {
        let capability = self.capability_for_plugin(plugin_id, capability_id)?;
        if !capability.permissions.contains(&permission) {
            return Err(RegistryError::PermissionNotDeclared {
                plugin_id: plugin_id.to_string(),
                capability_id: capability_id.to_string(),
                permission,
            });
        }

        self.plugin_mut(plugin_id)?
            .granted_permissions
            .insert(permission.clone());
        self.governance
            .policy_mut()
            .upsert_rule(plugin_permission_rule(
                plugin_id,
                capability_id,
                &permission,
            ));
        Ok(())
    }

    pub fn revoke_permission(
        &mut self,
        plugin_id: &str,
        permission: &Permission,
    ) -> Result<(), RegistryError> {
        self.plugin_mut(plugin_id)?
            .granted_permissions
            .remove(permission);
        self.governance
            .policy_mut()
            .remove_rules_with_prefix(&plugin_permission_rule_prefix(plugin_id, permission));
        Ok(())
    }

    pub fn check_pre_execution(
        &mut self,
        plugin_id: &str,
        capability_id: &str,
    ) -> Result<ExecutionPermit, RegistryError> {
        self.check_pre_execution_with_context(
            plugin_id,
            capability_id,
            PreExecutionContext::default(),
        )
    }

    pub fn check_pre_execution_with_context(
        &mut self,
        plugin_id: &str,
        capability_id: &str,
        context: PreExecutionContext,
    ) -> Result<ExecutionPermit, RegistryError> {
        let plugin = self.plugin(plugin_id)?;
        let capability = plugin
            .manifest
            .capabilities
            .iter()
            .find(|capability| capability.id == capability_id)
            .cloned()
            .ok_or_else(|| RegistryError::CapabilityNotFound {
                plugin_id: plugin_id.to_string(),
                capability_id: capability_id.to_string(),
            })?;
        let mut governance_decision_ids = Vec::new();

        self.require_governance_allowed(
            plugin_id,
            capability_id,
            governance_plugin_execution_request(plugin_id, capability_id, &context),
            &mut governance_decision_ids,
        )?;

        match self.enabled_capabilities.get(capability_id) {
            Some(owner_plugin_id) if owner_plugin_id == plugin_id => {}
            Some(owner_plugin_id) => {
                return Err(RegistryError::CapabilityOwnedByDifferentPlugin {
                    capability_id: capability_id.to_string(),
                    owner_plugin_id: owner_plugin_id.clone(),
                    requested_plugin_id: plugin_id.to_string(),
                });
            }
            None => {
                return Err(RegistryError::CapabilityNotEnabled {
                    plugin_id: plugin_id.to_string(),
                    capability_id: capability_id.to_string(),
                });
            }
        }

        for permission in &capability.permissions {
            self.require_governance_allowed(
                plugin_id,
                capability_id,
                governance_permission_request(plugin_id, capability_id, permission, &context),
                &mut governance_decision_ids,
            )?;
        }

        Ok(ExecutionPermit {
            plugin_id: plugin_id.to_string(),
            capability_id: capability_id.to_string(),
            capability_kind: capability.kind,
            granted_permissions: capability.permissions.clone(),
            governance_decision_ids,
        })
    }

    pub fn record_governance_approval(&mut self, approval: ApprovalRecord) {
        self.governance.record_approval(approval);
    }

    pub fn governance_audit_export(&self) -> GovernanceAuditExport {
        self.governance.export_audit()
    }

    pub fn is_enabled(&self, plugin_id: &str) -> bool {
        self.plugins
            .get(plugin_id)
            .is_some_and(|plugin| plugin.state == PluginState::Enabled)
    }

    pub fn plugin(&self, plugin_id: &str) -> Result<&RegisteredPlugin, RegistryError> {
        self.plugins
            .get(plugin_id)
            .ok_or_else(|| RegistryError::PluginNotFound {
                plugin_id: plugin_id.to_string(),
            })
    }

    pub fn capability_owner(&self, capability_id: &str) -> Option<&str> {
        self.enabled_capabilities
            .get(capability_id)
            .map(String::as_str)
    }

    pub fn enabled_capability(
        &self,
        capability_id: &str,
    ) -> Result<RegisteredCapability, RegistryError> {
        let plugin_id = self
            .enabled_capabilities
            .get(capability_id)
            .ok_or_else(|| RegistryError::CapabilityNotEnabled {
                plugin_id: "<unknown>".to_string(),
                capability_id: capability_id.to_string(),
            })?;
        let capability = self.capability_for_plugin(plugin_id, capability_id)?;
        Ok(RegisteredCapability {
            plugin_id: plugin_id.clone(),
            capability,
        })
    }

    pub fn snapshot(&self) -> RegistrySnapshot {
        let plugins = self.plugins.values().cloned().collect::<Vec<_>>();
        let enabled_capabilities = self
            .enabled_capabilities
            .keys()
            .filter_map(|capability_id| self.enabled_capability(capability_id).ok())
            .collect::<Vec<_>>();

        RegistrySnapshot {
            plugins,
            enabled_capabilities,
        }
    }

    fn require_governance_allowed(
        &mut self,
        plugin_id: &str,
        capability_id: &str,
        request: ActionRequest,
        decision_ids: &mut Vec<String>,
    ) -> Result<(), RegistryError> {
        let decision = self.governance.evaluate(request);
        let decision_id = decision.decision_id().to_string();
        decision_ids.push(decision_id.clone());

        match decision {
            DecisionOutcome::Allowed(_) => Ok(()),
            DecisionOutcome::Denied(denied) => Err(RegistryError::GovernanceDenied {
                plugin_id: plugin_id.to_string(),
                capability_id: capability_id.to_string(),
                decision_id,
                reason: format!("{:?}", denied.reason),
            }),
            DecisionOutcome::ApprovalRequired(required) => {
                Err(RegistryError::GovernanceApprovalRequired {
                    plugin_id: plugin_id.to_string(),
                    capability_id: capability_id.to_string(),
                    decision_id,
                    missing_gates: required.missing_gates,
                })
            }
        }
    }

    fn plugin_mut(&mut self, plugin_id: &str) -> Result<&mut RegisteredPlugin, RegistryError> {
        self.plugins
            .get_mut(plugin_id)
            .ok_or_else(|| RegistryError::PluginNotFound {
                plugin_id: plugin_id.to_string(),
            })
    }

    fn capability_for_plugin(
        &self,
        plugin_id: &str,
        capability_id: &str,
    ) -> Result<CapabilityManifest, RegistryError> {
        self.plugin(plugin_id)?
            .manifest
            .capabilities
            .iter()
            .find(|capability| capability.id == capability_id)
            .cloned()
            .ok_or_else(|| RegistryError::CapabilityNotFound {
                plugin_id: plugin_id.to_string(),
                capability_id: capability_id.to_string(),
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ManifestValidationError {
    #[error("required field '{field}' is missing or empty")]
    MissingRequiredField { field: &'static str },
    #[error("plugin '{plugin_id}' declares no capabilities")]
    MissingCapabilities { plugin_id: String },
    #[error("plugin '{plugin_id}' has duplicate capability '{capability_id}'")]
    DuplicateCapability {
        plugin_id: String,
        capability_id: String,
    },
    #[error("plugin '{plugin_id}' declares reserved capability id '{capability_id}'")]
    ReservedCapabilityId {
        plugin_id: String,
        capability_id: String,
    },
    #[error("missing legal metadata at '{field}': {reason}")]
    MissingLegalMetadata { field: &'static str, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RegistryError {
    #[error("{0}")]
    InvalidManifest(ManifestValidationError),
    #[error("plugin already registered: {plugin_id}")]
    PluginAlreadyRegistered { plugin_id: String },
    #[error("plugin not found: {plugin_id}")]
    PluginNotFound { plugin_id: String },
    #[error("plugin is disabled: {plugin_id}")]
    PluginDisabled { plugin_id: String },
    #[error("capability not found: {plugin_id}/{capability_id}")]
    CapabilityNotFound {
        plugin_id: String,
        capability_id: String,
    },
    #[error("capability not enabled: {plugin_id}/{capability_id}")]
    CapabilityNotEnabled {
        plugin_id: String,
        capability_id: String,
    },
    #[error("capability '{capability_id}' already owned by '{existing_plugin_id}'")]
    CapabilityConflict {
        capability_id: String,
        existing_plugin_id: String,
        requested_plugin_id: String,
    },
    #[error(
        "capability '{capability_id}' is owned by '{owner_plugin_id}', not '{requested_plugin_id}'"
    )]
    CapabilityOwnedByDifferentPlugin {
        capability_id: String,
        owner_plugin_id: String,
        requested_plugin_id: String,
    },
    #[error("permission denied for {plugin_id}/{capability_id}: {permission:?}")]
    PermissionDenied {
        plugin_id: String,
        capability_id: String,
        permission: Permission,
    },
    #[error("permission not declared for {plugin_id}/{capability_id}: {permission:?}")]
    PermissionNotDeclared {
        plugin_id: String,
        capability_id: String,
        permission: Permission,
    },
    #[error("governance denied {plugin_id}/{capability_id} at {decision_id}: {reason}")]
    GovernanceDenied {
        plugin_id: String,
        capability_id: String,
        decision_id: String,
        reason: String,
    },
    #[error(
        "governance requires approval for {plugin_id}/{capability_id} at {decision_id}: {missing_gates:?}"
    )]
    GovernanceApprovalRequired {
        plugin_id: String,
        capability_id: String,
        decision_id: String,
        missing_gates: BTreeSet<GateKind>,
    },
    #[error("governance policy update failed: {0}")]
    GovernancePolicy(String),
}

fn require_id(field: &'static str, value: &str) -> Result<(), ManifestValidationError> {
    require_text(field, value)?;
    if value.chars().any(char::is_whitespace) {
        return Err(ManifestValidationError::MissingRequiredField { field });
    }
    Ok(())
}

fn require_text(field: &'static str, value: &str) -> Result<(), ManifestValidationError> {
    if value.trim().is_empty() {
        return Err(ManifestValidationError::MissingRequiredField { field });
    }
    Ok(())
}

fn plugin_actor(manifest: &PluginManifest, status: ActorStatus) -> GovernanceActor {
    GovernanceActor {
        id: manifest.id.clone(),
        role: ActorRole::Plugin,
        status,
        parent_actor_id: None,
        capabilities: manifest
            .capabilities
            .iter()
            .map(|capability| governance_plugin_capability(&capability.id))
            .collect(),
        metadata: Metadata::new(),
    }
}

fn governance_plugin_capability(capability_id: &str) -> cowork_governance::Capability {
    cowork_governance::Capability::new(format!("plugin:{capability_id}"))
}

fn plugin_rule_prefix(plugin_id: &str) -> String {
    format!("plugin/{plugin_id}/")
}

fn plugin_execution_rule_id(plugin_id: &str, capability_id: &str) -> String {
    format!("{}execute/{capability_id}", plugin_rule_prefix(plugin_id))
}

fn plugin_permission_rule_prefix(plugin_id: &str, permission: &Permission) -> String {
    let (action, resource_kind, _, _) = permission_governance_shape(permission);
    format!(
        "{}permission/{:?}/{:?}/{}/",
        plugin_rule_prefix(plugin_id),
        action,
        resource_kind,
        permission_scope_value(permission)
    )
}

fn plugin_permission_rule_id(
    plugin_id: &str,
    capability_id: &str,
    permission: &Permission,
) -> String {
    format!(
        "{}{capability_id}",
        plugin_permission_rule_prefix(plugin_id, permission)
    )
}

fn plugin_execution_rule(plugin_id: &str, capability_id: &str) -> PolicyRule {
    PolicyRule::allow(
        plugin_execution_rule_id(plugin_id, capability_id),
        ActorSelector::Exact(plugin_id.to_string()),
    )
    .for_capability(governance_plugin_capability(capability_id))
    .with_action(ActionKind::Execute)
    .with_resource(ResourceSelector::exact(
        ResourceKind::PluginCapability,
        plugin_capability_resource_id(plugin_id, capability_id),
    ))
}

fn plugin_permission_rule(
    plugin_id: &str,
    capability_id: &str,
    permission: &Permission,
) -> PolicyRule {
    let (action, _, risk, selector) = permission_governance_shape(permission);
    PolicyRule::allow(
        plugin_permission_rule_id(plugin_id, capability_id, permission),
        ActorSelector::Exact(plugin_id.to_string()),
    )
    .for_capability(governance_plugin_capability(capability_id))
    .with_action(action)
    .with_resource(selector)
    .with_gates(GateRequirements::for_risk(risk))
}

fn governance_plugin_execution_request(
    plugin_id: &str,
    capability_id: &str,
    context: &PreExecutionContext,
) -> ActionRequest {
    request_with_context(
        ActionRequest::new(
            plugin_id,
            ActionKind::Execute,
            Resource::new(
                ResourceKind::PluginCapability,
                plugin_capability_resource_id(plugin_id, capability_id),
            ),
            ActionRisk::Low,
        )
        .with_capability(governance_plugin_capability(capability_id)),
        None,
        context,
    )
}

fn governance_permission_request(
    plugin_id: &str,
    capability_id: &str,
    permission: &Permission,
    context: &PreExecutionContext,
) -> ActionRequest {
    let (action, resource_kind, risk, _) = permission_governance_shape(permission);
    request_with_context(
        ActionRequest::new(
            plugin_id,
            action,
            Resource::new(resource_kind, permission_scope_value(permission)),
            risk,
        )
        .with_capability(governance_plugin_capability(capability_id)),
        context.permission_approvals.get(permission).cloned(),
        context,
    )
}

fn request_with_context(
    mut request: ActionRequest,
    approval_id: Option<String>,
    context: &PreExecutionContext,
) -> ActionRequest {
    if let Some(approval_id) = approval_id {
        request = request.with_approval(approval_id);
    }
    request.metadata = context.metadata.clone();
    request.justification = context.justification.clone();
    request.with_evidence(context.evidence.clone())
}

fn permission_governance_shape(
    permission: &Permission,
) -> (ActionKind, ResourceKind, ActionRisk, ResourceSelector) {
    let (action, resource_kind, risk) = match permission.kind {
        PermissionKind::WorkspaceRead => (ActionKind::Read, ResourceKind::Folder, ActionRisk::Low),
        PermissionKind::WorkspaceWrite => {
            (ActionKind::Write, ResourceKind::Folder, ActionRisk::Medium)
        }
        PermissionKind::ProcessSpawn => (
            ActionKind::SpawnProcess,
            ResourceKind::Tool,
            ActionRisk::High,
        ),
        PermissionKind::Network => (
            ActionKind::NetworkRequest,
            ResourceKind::NetworkEndpoint,
            ActionRisk::External,
        ),
        PermissionKind::BrowserProfile => (
            ActionKind::BrowserAction,
            ResourceKind::BrowserProfile,
            ActionRisk::Medium,
        ),
        PermissionKind::ProviderCredential => (
            ActionKind::ProviderCall,
            ResourceKind::Provider,
            ActionRisk::External,
        ),
        PermissionKind::ConnectorAccount => (
            ActionKind::ConnectorCall,
            ResourceKind::Connector,
            ActionRisk::External,
        ),
        PermissionKind::ArtifactWrite => (
            ActionKind::Write,
            ResourceKind::Artifact,
            ActionRisk::Medium,
        ),
    };

    let selector = match &permission.scope {
        PermissionScope::Any => ResourceSelector {
            kind: Some(resource_kind),
            pattern: ScopePattern::Any,
        },
        PermissionScope::Pattern(pattern) => {
            ResourceSelector::pattern(resource_kind, pattern.as_str())
        }
    };

    (action, resource_kind, risk, selector)
}

fn permission_scope_value(permission: &Permission) -> String {
    match &permission.scope {
        PermissionScope::Any => "*".to_string(),
        PermissionScope::Pattern(pattern) => pattern.clone(),
    }
}

fn plugin_capability_resource_id(plugin_id: &str, capability_id: &str) -> String {
    format!("plugin://{plugin_id}/{capability_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_manifest_rejection_is_typed() {
        let mut manifest = fixture_manifest();
        manifest.capabilities.clear();

        let error = manifest.validate().unwrap_err();

        assert_eq!(
            error,
            ManifestValidationError::MissingCapabilities {
                plugin_id: "cowork.example".to_string(),
            }
        );
    }

    #[test]
    fn legal_metadata_is_required_for_imported_code_and_assets() {
        let mut manifest = fixture_manifest();
        manifest.source.imported_components = vec![ImportedComponent {
            id: "openwork-browser-adapter".to_string(),
            kind: ImportedComponentKind::Code,
            source_uri: "https://example.invalid/openwork/browser".to_string(),
            license: Some("MIT".to_string()),
            legal_links: Vec::new(),
        }];

        let error = manifest.validate().unwrap_err();

        assert!(matches!(
            error,
            ManifestValidationError::MissingLegalMetadata {
                field: "importedComponent.legalLinks",
                ..
            }
        ));
    }

    #[test]
    fn pre_execution_denies_missing_permission_before_execution() {
        let permission = Permission::new(PermissionKind::Network, "https://api.example.invalid/*");
        let mut registry =
            registry_with_plugin(fixture_manifest_with_permission(permission.clone()));
        registry
            .enable_plugin("cowork.example")
            .expect("enable plugin");

        let error = registry
            .check_pre_execution("cowork.example", "cowork.example.web-search")
            .unwrap_err();

        assert!(matches!(
            error,
            RegistryError::GovernanceDenied {
                plugin_id,
                capability_id,
                reason,
                ..
            } if plugin_id == "cowork.example"
                && capability_id == "cowork.example.web-search"
                && reason.contains("NoAllowRule")
        ));
        assert_eq!(
            registry.governance_audit_export().decisions.len(),
            2,
            "execution and permission checks are both audited by governance"
        );
        assert_eq!(
            registry.governance_audit_export().decisions[0].decision_id,
            "decision/1"
        );
    }

    #[test]
    fn enable_disable_behavior_updates_capability_availability() {
        let permission = Permission::new(PermissionKind::WorkspaceRead, "workspace://project/**");
        let mut registry =
            registry_with_plugin(fixture_manifest_with_permission(permission.clone()));

        assert!(!registry.is_enabled("cowork.example"));
        assert_eq!(registry.capability_owner("cowork.example.web-search"), None);

        registry
            .enable_plugin("cowork.example")
            .expect("enable plugin");
        assert!(registry.is_enabled("cowork.example"));
        assert_eq!(
            registry.capability_owner("cowork.example.web-search"),
            Some("cowork.example")
        );

        registry
            .grant_permission("cowork.example", "cowork.example.web-search", permission)
            .expect("grant declared permission");
        registry
            .check_pre_execution("cowork.example", "cowork.example.web-search")
            .expect("permit after enabling and granting");

        registry
            .disable_plugin("cowork.example")
            .expect("disable plugin");
        assert!(!registry.is_enabled("cowork.example"));
        assert_eq!(registry.capability_owner("cowork.example.web-search"), None);
        assert!(matches!(
            registry.check_pre_execution("cowork.example", "cowork.example.web-search"),
            Err(RegistryError::GovernanceDenied { reason, .. }) if reason.contains("ActorDisabled")
        ));
    }

    #[test]
    fn external_plugin_permission_requires_approval_and_evidence() {
        let permission = Permission::new(PermissionKind::Network, "https://api.example.invalid/*");
        let mut registry =
            registry_with_plugin(fixture_manifest_with_permission(permission.clone()));
        registry
            .enable_plugin("cowork.example")
            .expect("enable plugin");
        registry
            .grant_permission(
                "cowork.example",
                "cowork.example.web-search",
                permission.clone(),
            )
            .expect("grant declared permission");

        let error = registry
            .check_pre_execution("cowork.example", "cowork.example.web-search")
            .unwrap_err();

        assert!(matches!(
            error,
            RegistryError::GovernanceApprovalRequired { missing_gates, .. }
                if missing_gates == BTreeSet::from([GateKind::Approval, GateKind::Evidence])
        ));

        registry.record_governance_approval(
            ApprovalRecord::approved_for(
                "approval/network-search",
                "cowork.example",
                ActionKind::NetworkRequest,
                Resource::new(
                    ResourceKind::NetworkEndpoint,
                    "https://api.example.invalid/*",
                ),
                "user/alice",
            )
            .with_evidence(EvidenceBundle {
                artifacts: vec![cowork_governance::EvidenceArtifact::new(
                    "evidence/network-approval",
                    "User approved this connector request.",
                )],
                ..EvidenceBundle::default()
            }),
        );

        let permit = registry
            .check_pre_execution_with_context(
                "cowork.example",
                "cowork.example.web-search",
                PreExecutionContext {
                    permission_approvals: BTreeMap::from([(
                        permission,
                        "approval/network-search".to_string(),
                    )]),
                    evidence: EvidenceBundle::default(),
                    justification: "approved search call".to_string(),
                    metadata: Metadata::new(),
                },
            )
            .expect("approved external permission should pass");

        assert_eq!(permit.governance_decision_ids.len(), 2);
    }

    #[test]
    fn governance_audit_redacts_plugin_pre_execution_metadata() {
        let permission = Permission::new(PermissionKind::WorkspaceRead, "workspace://project/**");
        let mut registry =
            registry_with_plugin(fixture_manifest_with_permission(permission.clone()));
        registry
            .enable_plugin("cowork.example")
            .expect("enable plugin");
        registry
            .grant_permission("cowork.example", "cowork.example.web-search", permission)
            .expect("grant declared permission");
        let synthetic_credential = format!("{}{}", "ghp_", "abcdefghijklmnopqrstuvwxyz");
        let synthetic_assignment = format!("{}={}", "token", "abcdef1234567890");
        let mut metadata = Metadata::new();
        metadata.insert(
            "access_token".to_string(),
            serde_json::json!(synthetic_credential),
        );

        registry
            .check_pre_execution_with_context(
                "cowork.example",
                "cowork.example.web-search",
                PreExecutionContext {
                    permission_approvals: BTreeMap::new(),
                    evidence: EvidenceBundle::default(),
                    justification: synthetic_assignment,
                    metadata,
                },
            )
            .expect("workspace read should pass");

        let export = serde_json::to_string(&registry.governance_audit_export()).unwrap();
        assert!(!export.contains("abcdefghijklmnopqrstuvwxyz"));
        assert!(!export.contains("abcdef1234567890"));
        assert!(export.contains(cowork_governance::REDACTED));
    }

    #[test]
    fn unload_cleans_registered_capability_state() {
        let mut registry = registry_with_plugin(fixture_manifest());

        registry
            .enable_plugin("cowork.example")
            .expect("enable plugin");
        assert_eq!(
            registry.snapshot().enabled_capabilities.len(),
            1,
            "fixture should expose one enabled capability"
        );

        let removed = registry
            .unload_plugin("cowork.example")
            .expect("unload plugin");

        assert_eq!(removed.manifest.id, "cowork.example");
        assert!(matches!(
            registry.plugin("cowork.example"),
            Err(RegistryError::PluginNotFound { .. })
        ));
        assert_eq!(registry.capability_owner("cowork.example.web-search"), None);
        assert!(registry.snapshot().enabled_capabilities.is_empty());
    }

    fn registry_with_plugin(manifest: PluginManifest) -> PluginRegistry {
        let mut registry = PluginRegistry::new();
        registry
            .register_manifest(manifest, fixture_directory())
            .expect("register manifest");
        registry
    }

    fn fixture_manifest() -> PluginManifest {
        fixture_manifest_with_permission_set(BTreeSet::new())
    }

    fn fixture_manifest_with_permission(permission: Permission) -> PluginManifest {
        fixture_manifest_with_permission_set(BTreeSet::from([permission]))
    }

    fn fixture_manifest_with_permission_set(permissions: BTreeSet<Permission>) -> PluginManifest {
        PluginManifest {
            id: "cowork.example".to_string(),
            name: "Example capability pack".to_string(),
            description: "Test manifest for registry behavior.".to_string(),
            version: "1.0.0".to_string(),
            version_evidence: VersionEvidence {
                manifest_schema: "cowork.plugin.v1".to_string(),
                manifest_version: "1.0.0".to_string(),
                source_revision: Some("93e0d0ce1bcb61ecb334d178a60f74040d07f381".to_string()),
                content_hash: Some("sha256:example".to_string()),
                evidence_links: vec![EvidenceLink {
                    label: "build plan".to_string(),
                    uri: "https://example.invalid/plan".to_string(),
                }],
            },
            source: SourceMetadata {
                kind: SourceKind::Native,
                origin: "cowork-kernel".to_string(),
                homepage: None,
                repository: Some("https://example.invalid/cowork".to_string()),
                license: Some("MIT".to_string()),
                legal: LegalMetadata {
                    license_expression: Some("MIT".to_string()),
                    notice_links: Vec::new(),
                    source_links: Vec::new(),
                },
                imported_components: Vec::new(),
            },
            capabilities: vec![CapabilityManifest {
                id: "cowork.example.web-search".to_string(),
                kind: CapabilityKind::ResearchProvider,
                display_name: "Example Web Search".to_string(),
                version: "1.0.0".to_string(),
                permissions,
                metadata: Metadata::new(),
            }],
            metadata: Metadata::new(),
        }
    }

    fn fixture_directory() -> PluginDirectory {
        PluginDirectory {
            root: "plugins/cowork.example".to_string(),
            manifest_path: "plugins/cowork.example/cowork-plugin.json".to_string(),
            assets_dir: Some("plugins/cowork.example/assets".to_string()),
            runtime_dir: None,
        }
    }
}
