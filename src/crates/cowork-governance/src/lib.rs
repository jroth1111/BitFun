//! Platform-agnostic Cowork governance and permission evaluation.
//!
//! This crate owns policy evaluation, approval gates, secret redaction, and
//! auditable decision export. It intentionally performs no filesystem, process,
//! network, GUI, or daemon side effects; callers persist exported snapshots and
//! apply approved actions outside this model boundary.

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;
use thiserror::Error;

pub const GOVERNANCE_SCHEMA_VERSION: &str = "cowork.governance.v1";
pub const REDACTED: &str = "[REDACTED]";

pub type Metadata = BTreeMap<String, Value>;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capability {
    pub id: String,
}

impl Capability {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorRole {
    User,
    Kernel,
    Agent,
    Subagent,
    Plugin,
    Tool,
    Browser,
    Connector,
    Provider,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorStatus {
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GovernanceActor {
    pub id: String,
    pub role: ActorRole,
    pub status: ActorStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_actor_id: Option<String>,
    #[serde(default)]
    pub capabilities: BTreeSet<Capability>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

impl GovernanceActor {
    pub fn enabled(id: impl Into<String>, role: ActorRole) -> Self {
        Self {
            id: id.into(),
            role,
            status: ActorStatus::Enabled,
            parent_actor_id: None,
            capabilities: BTreeSet::new(),
            metadata: Metadata::new(),
        }
    }

    pub fn disabled(id: impl Into<String>, role: ActorRole) -> Self {
        Self {
            status: ActorStatus::Disabled,
            ..Self::enabled(id, role)
        }
    }

    pub fn with_capabilities(mut self, capabilities: impl IntoIterator<Item = Capability>) -> Self {
        self.capabilities.extend(capabilities);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Folder,
    File,
    Tool,
    Browser,
    BrowserProfile,
    Connector,
    Provider,
    Plugin,
    PluginCapability,
    Subagent,
    NetworkEndpoint,
    Artifact,
    Secret,
    ExternalSystem,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    pub kind: ResourceKind,
    pub identifier: String,
}

impl Resource {
    pub fn new(kind: ResourceKind, identifier: impl Into<String>) -> Self {
        Self {
            kind,
            identifier: identifier.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Read,
    Write,
    Delete,
    Execute,
    SpawnProcess,
    NetworkRequest,
    BrowserAction,
    ConnectorCall,
    ProviderCall,
    SubagentInvoke,
    Prompt,
    Log,
    Export,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionRisk {
    Low,
    Medium,
    High,
    Destructive,
    External,
    DestructiveExternal,
}

impl ActionRisk {
    pub const fn is_destructive(self) -> bool {
        matches!(self, Self::Destructive | Self::DestructiveExternal)
    }

    pub const fn is_external(self) -> bool {
        matches!(self, Self::External | Self::DestructiveExternal)
    }

    pub const fn requires_approval(self) -> bool {
        matches!(
            self,
            Self::High | Self::Destructive | Self::External | Self::DestructiveExternal
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRequest {
    pub actor_id: String,
    pub action: ActionKind,
    pub resource: Resource,
    pub risk: ActionRisk,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability: Option<Capability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_id: Option<String>,
    #[serde(default)]
    pub evidence: EvidenceBundle,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub justification: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

impl ActionRequest {
    pub fn new(
        actor_id: impl Into<String>,
        action: ActionKind,
        resource: Resource,
        risk: ActionRisk,
    ) -> Self {
        Self {
            actor_id: actor_id.into(),
            action,
            resource,
            risk,
            capability: None,
            approval_id: None,
            evidence: EvidenceBundle::default(),
            justification: String::new(),
            metadata: Metadata::new(),
        }
    }

    pub fn with_capability(mut self, capability: Capability) -> Self {
        self.capability = Some(capability);
        self
    }

    pub fn with_approval(mut self, approval_id: impl Into<String>) -> Self {
        self.approval_id = Some(approval_id.into());
        self
    }

    pub fn with_evidence(mut self, evidence: EvidenceBundle) -> Self {
        self.evidence = evidence;
        self
    }

    pub fn with_justification(mut self, justification: impl Into<String>) -> Self {
        self.justification = justification.into();
        self
    }

    fn redacted(&self, redactor: &SecretRedactor) -> Self {
        let mut redacted = self.clone();
        redacted.actor_id = redactor.redact_text(&redacted.actor_id);
        redacted.resource.identifier = redactor.redact_text(&redacted.resource.identifier);
        if let Some(capability) = &mut redacted.capability {
            capability.id = redactor.redact_text(&capability.id);
        }
        if let Some(approval_id) = &mut redacted.approval_id {
            *approval_id = redactor.redact_text(approval_id);
        }
        redacted.evidence = redacted.evidence.redacted(redactor);
        redacted.justification = redactor.redact_text(&redacted.justification);
        redacted.metadata = redactor.redact_metadata(&redacted.metadata);
        redacted
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceBundle {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rollback: Option<RollbackPlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<CheckpointRef>,
    #[serde(default)]
    pub artifacts: Vec<EvidenceArtifact>,
}

impl EvidenceBundle {
    pub fn has_evidence(&self) -> bool {
        !self.artifacts.is_empty()
    }

    fn redacted(&self, redactor: &SecretRedactor) -> Self {
        Self {
            rollback: self
                .rollback
                .as_ref()
                .map(|rollback| rollback.redacted(redactor)),
            checkpoint: self
                .checkpoint
                .as_ref()
                .map(|checkpoint| checkpoint.redacted(redactor)),
            artifacts: self
                .artifacts
                .iter()
                .map(|artifact| artifact.redacted(redactor))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RollbackPlan {
    pub description: String,
    pub recovery_command: String,
}

impl RollbackPlan {
    pub fn new(description: impl Into<String>, recovery_command: impl Into<String>) -> Self {
        Self {
            description: description.into(),
            recovery_command: recovery_command.into(),
        }
    }

    fn redacted(&self, redactor: &SecretRedactor) -> Self {
        Self {
            description: redactor.redact_text(&self.description),
            recovery_command: redactor.redact_text(&self.recovery_command),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointRef {
    pub id: String,
    pub description: String,
}

impl CheckpointRef {
    pub fn new(id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
        }
    }

    fn redacted(&self, redactor: &SecretRedactor) -> Self {
        Self {
            id: redactor.redact_text(&self.id),
            description: redactor.redact_text(&self.description),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceArtifact {
    pub id: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

impl EvidenceArtifact {
    pub fn new(id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            metadata: Metadata::new(),
        }
    }

    fn redacted(&self, redactor: &SecretRedactor) -> Self {
        Self {
            id: redactor.redact_text(&self.id),
            description: redactor.redact_text(&self.description),
            metadata: redactor.redact_metadata(&self.metadata),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRecord {
    pub id: String,
    pub status: ApprovalStatus,
    pub actor_id: String,
    pub action: ActionKind,
    pub resource: Resource,
    pub approved_by: String,
    #[serde(default)]
    pub evidence: EvidenceBundle,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

impl ApprovalRecord {
    pub fn approved_for(
        id: impl Into<String>,
        actor_id: impl Into<String>,
        action: ActionKind,
        resource: Resource,
        approved_by: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            status: ApprovalStatus::Approved,
            actor_id: actor_id.into(),
            action,
            resource,
            approved_by: approved_by.into(),
            evidence: EvidenceBundle::default(),
            reason: String::new(),
            metadata: Metadata::new(),
        }
    }

    pub fn with_evidence(mut self, evidence: EvidenceBundle) -> Self {
        self.evidence = evidence;
        self
    }

    fn matches_request(&self, request: &ActionRequest) -> bool {
        self.actor_id == request.actor_id
            && self.action == request.action
            && self.resource == request.resource
    }

    fn redacted(&self, redactor: &SecretRedactor) -> Self {
        let mut redacted = self.clone();
        redacted.id = redactor.redact_text(&redacted.id);
        redacted.actor_id = redactor.redact_text(&redacted.actor_id);
        redacted.resource.identifier = redactor.redact_text(&redacted.resource.identifier);
        redacted.approved_by = redactor.redact_text(&redacted.approved_by);
        redacted.evidence = redacted.evidence.redacted(redactor);
        redacted.reason = redactor.redact_text(&redacted.reason);
        redacted.metadata = redactor.redact_metadata(&redacted.metadata);
        redacted
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleEffect {
    Allow,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "value")]
pub enum ActorSelector {
    Any,
    Exact(String),
    Role(ActorRole),
}

impl ActorSelector {
    fn matches(&self, actor: &GovernanceActor) -> bool {
        match self {
            Self::Any => true,
            Self::Exact(id) => id == &actor.id,
            Self::Role(role) => role == &actor.role,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSelector {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<ResourceKind>,
    pub pattern: ScopePattern,
}

impl ResourceSelector {
    pub fn any() -> Self {
        Self {
            kind: None,
            pattern: ScopePattern::Any,
        }
    }

    pub fn exact(kind: ResourceKind, value: impl Into<String>) -> Self {
        Self {
            kind: Some(kind),
            pattern: ScopePattern::Exact(value.into()),
        }
    }

    pub fn pattern(kind: ResourceKind, value: impl Into<String>) -> Self {
        Self {
            kind: Some(kind),
            pattern: ScopePattern::Pattern(value.into()),
        }
    }

    fn matches(&self, resource: &Resource) -> bool {
        self.kind.map_or(true, |kind| kind == resource.kind)
            && self.pattern.matches(&resource.identifier)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "value")]
pub enum ScopePattern {
    Any,
    Exact(String),
    Prefix(String),
    Pattern(String),
}

impl ScopePattern {
    fn matches(&self, value: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Exact(expected) => expected == value,
            Self::Prefix(prefix) => value.starts_with(prefix),
            Self::Pattern(pattern) => wildcard_matches(pattern, value),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateKind {
    Approval,
    Rollback,
    Checkpoint,
    Evidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GateRequirements {
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub required: BTreeSet<GateKind>,
}

impl GateRequirements {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn for_risk(risk: ActionRisk) -> Self {
        let mut required = BTreeSet::new();

        if risk.requires_approval() {
            required.insert(GateKind::Approval);
        }

        if risk.is_destructive() {
            required.insert(GateKind::Rollback);
            required.insert(GateKind::Checkpoint);
        }

        if risk.is_destructive() || risk.is_external() {
            required.insert(GateKind::Evidence);
        }

        Self { required }
    }

    pub fn require(mut self, gate: GateKind) -> Self {
        self.required.insert(gate);
        self
    }

    pub fn merge(&mut self, other: &Self) {
        self.required.extend(other.required.iter().copied());
    }

    fn missing_for(
        &self,
        request: &ActionRequest,
        approval: Option<&ApprovalRecord>,
    ) -> BTreeSet<GateKind> {
        let mut missing = BTreeSet::new();

        for gate in &self.required {
            let satisfied = match gate {
                GateKind::Approval => {
                    approval.is_some_and(|approval| approval.status == ApprovalStatus::Approved)
                }
                GateKind::Rollback => {
                    request.evidence.rollback.is_some()
                        || approval.is_some_and(|approval| approval.evidence.rollback.is_some())
                }
                GateKind::Checkpoint => {
                    request.evidence.checkpoint.is_some()
                        || approval.is_some_and(|approval| approval.evidence.checkpoint.is_some())
                }
                GateKind::Evidence => {
                    request.evidence.has_evidence()
                        || approval.is_some_and(|approval| approval.evidence.has_evidence())
                }
            };

            if !satisfied {
                missing.insert(*gate);
            }
        }

        missing
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyRule {
    pub id: String,
    pub effect: RuleEffect,
    pub actor: ActorSelector,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<Capability>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub actions: BTreeSet<ActionKind>,
    #[serde(default)]
    pub resources: Vec<ResourceSelector>,
    #[serde(default)]
    pub gates: GateRequirements,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

impl PolicyRule {
    pub fn allow(id: impl Into<String>, actor: ActorSelector) -> Self {
        Self {
            id: id.into(),
            effect: RuleEffect::Allow,
            actor,
            capability: None,
            actions: BTreeSet::new(),
            resources: Vec::new(),
            gates: GateRequirements::default(),
            metadata: Metadata::new(),
        }
    }

    pub fn deny(id: impl Into<String>, actor: ActorSelector) -> Self {
        Self {
            effect: RuleEffect::Deny,
            ..Self::allow(id, actor)
        }
    }

    pub fn for_capability(mut self, capability: Capability) -> Self {
        self.capability = Some(capability);
        self
    }

    pub fn with_action(mut self, action: ActionKind) -> Self {
        self.actions.insert(action);
        self
    }

    pub fn with_resource(mut self, resource: ResourceSelector) -> Self {
        self.resources.push(resource);
        self
    }

    pub fn with_gates(mut self, gates: GateRequirements) -> Self {
        self.gates = gates;
        self
    }

    fn matches(&self, actor: &GovernanceActor, request: &ActionRequest) -> bool {
        if !self.actor.matches(actor) {
            return false;
        }

        if let Some(required_capability) = &self.capability {
            if request.capability.as_ref() != Some(required_capability) {
                return false;
            }
        }

        let action_matches = self.actions.is_empty() || self.actions.contains(&request.action);
        let resource_matches = self.resources.is_empty()
            || self
                .resources
                .iter()
                .any(|selector| selector.matches(&request.resource));

        action_matches && resource_matches
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GovernancePolicy {
    #[serde(default)]
    actors: BTreeMap<String, GovernanceActor>,
    #[serde(default)]
    rules: BTreeMap<String, PolicyRule>,
}

impl GovernancePolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn actors(&self) -> &BTreeMap<String, GovernanceActor> {
        &self.actors
    }

    pub fn rules(&self) -> &BTreeMap<String, PolicyRule> {
        &self.rules
    }

    pub fn upsert_actor(&mut self, actor: GovernanceActor) {
        self.actors.insert(actor.id.clone(), actor);
    }

    pub fn set_actor_status(
        &mut self,
        actor_id: &str,
        status: ActorStatus,
    ) -> Result<(), GovernanceError> {
        let actor = self
            .actors
            .get_mut(actor_id)
            .ok_or_else(|| GovernanceError::UnknownActor {
                actor_id: actor_id.to_string(),
            })?;
        actor.status = status;
        Ok(())
    }

    pub fn remove_actor(&mut self, actor_id: &str) {
        self.actors.remove(actor_id);
    }

    pub fn upsert_rule(&mut self, rule: PolicyRule) {
        self.rules.insert(rule.id.clone(), rule);
    }

    pub fn remove_rule(&mut self, rule_id: &str) {
        self.rules.remove(rule_id);
    }

    pub fn remove_rules_with_prefix(&mut self, prefix: &str) {
        self.rules.retain(|rule_id, _| !rule_id.starts_with(prefix));
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AllowedDecision {
    pub decision_id: String,
    pub matched_allow_rules: Vec<String>,
    pub satisfied_gates: BTreeSet<GateKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeniedDecision {
    pub decision_id: String,
    pub reason: DenyReason,
    pub matched_deny_rules: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequiredDecision {
    pub decision_id: String,
    pub missing_gates: BTreeSet<GateKind>,
    pub matched_allow_rules: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "outcome")]
pub enum DecisionOutcome {
    Allowed(AllowedDecision),
    Denied(DeniedDecision),
    ApprovalRequired(ApprovalRequiredDecision),
}

impl DecisionOutcome {
    pub fn decision_id(&self) -> &str {
        match self {
            Self::Allowed(decision) => &decision.decision_id,
            Self::Denied(decision) => &decision.decision_id,
            Self::ApprovalRequired(decision) => &decision.decision_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "details")]
pub enum DenyReason {
    UnknownActor {
        actor_id: String,
    },
    ActorDisabled {
        actor_id: String,
    },
    MissingCapability {
        actor_id: String,
        capability_id: String,
    },
    ExplicitDeny {
        rule_ids: Vec<String>,
    },
    NoAllowRule,
    ApprovalRejected {
        approval_id: String,
    },
    ApprovalDoesNotMatchRequest {
        approval_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionRecord {
    pub sequence: u64,
    pub decision_id: String,
    pub request: ActionRequest,
    pub outcome: DecisionOutcome,
    #[serde(default)]
    pub matched_rule_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GovernanceAuditExport {
    pub schema_version: String,
    pub decisions: Vec<DecisionRecord>,
    pub approvals: Vec<ApprovalRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GovernanceError {
    #[error("unknown actor '{actor_id}'")]
    UnknownActor { actor_id: String },
    #[error("duplicate decision sequence {sequence}")]
    DuplicateDecisionSequence { sequence: u64 },
}

#[derive(Debug, Clone)]
pub struct GovernanceManager {
    policy: GovernancePolicy,
    approvals: BTreeMap<String, ApprovalRecord>,
    decisions: Vec<DecisionRecord>,
    next_sequence: u64,
    redactor: SecretRedactor,
}

impl Default for GovernanceManager {
    fn default() -> Self {
        Self::new(GovernancePolicy::new())
    }
}

impl GovernanceManager {
    pub fn new(policy: GovernancePolicy) -> Self {
        Self {
            policy,
            approvals: BTreeMap::new(),
            decisions: Vec::new(),
            next_sequence: 1,
            redactor: SecretRedactor::default(),
        }
    }

    pub fn from_audit_export(
        policy: GovernancePolicy,
        export: GovernanceAuditExport,
    ) -> Result<Self, GovernanceError> {
        let mut seen = BTreeSet::new();
        let mut max_sequence = 0;
        for decision in &export.decisions {
            if !seen.insert(decision.sequence) {
                return Err(GovernanceError::DuplicateDecisionSequence {
                    sequence: decision.sequence,
                });
            }
            max_sequence = max_sequence.max(decision.sequence);
        }

        Ok(Self {
            policy,
            approvals: export
                .approvals
                .into_iter()
                .map(|approval| (approval.id.clone(), approval))
                .collect(),
            decisions: export.decisions,
            next_sequence: max_sequence + 1,
            redactor: SecretRedactor::default(),
        })
    }

    pub fn policy(&self) -> &GovernancePolicy {
        &self.policy
    }

    pub fn policy_mut(&mut self) -> &mut GovernancePolicy {
        &mut self.policy
    }

    pub fn approvals(&self) -> &BTreeMap<String, ApprovalRecord> {
        &self.approvals
    }

    pub fn decisions(&self) -> &[DecisionRecord] {
        &self.decisions
    }

    pub fn record_approval(&mut self, approval: ApprovalRecord) {
        self.approvals.insert(approval.id.clone(), approval);
    }

    pub fn evaluate(&mut self, request: ActionRequest) -> DecisionOutcome {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        let decision_id = format!("decision/{sequence}");
        let (outcome, matched_rule_ids) = self.evaluate_without_recording(&decision_id, &request);
        let record = DecisionRecord {
            sequence,
            decision_id,
            request: request.redacted(&self.redactor),
            outcome: outcome.clone(),
            matched_rule_ids,
        };
        self.decisions.push(record);
        outcome
    }

    pub fn export_audit(&self) -> GovernanceAuditExport {
        GovernanceAuditExport {
            schema_version: GOVERNANCE_SCHEMA_VERSION.to_string(),
            decisions: self.decisions.clone(),
            approvals: self
                .approvals
                .values()
                .map(|approval| approval.redacted(&self.redactor))
                .collect(),
        }
    }

    pub fn redact_text(&self, text: &str) -> String {
        self.redactor.redact_text(text)
    }

    pub fn redact_metadata(&self, metadata: &Metadata) -> Metadata {
        self.redactor.redact_metadata(metadata)
    }

    fn evaluate_without_recording(
        &self,
        decision_id: &str,
        request: &ActionRequest,
    ) -> (DecisionOutcome, Vec<String>) {
        let Some(actor) = self.policy.actors.get(&request.actor_id) else {
            return (
                DecisionOutcome::Denied(DeniedDecision {
                    decision_id: decision_id.to_string(),
                    reason: DenyReason::UnknownActor {
                        actor_id: request.actor_id.clone(),
                    },
                    matched_deny_rules: Vec::new(),
                }),
                Vec::new(),
            );
        };

        if actor.status == ActorStatus::Disabled {
            return (
                DecisionOutcome::Denied(DeniedDecision {
                    decision_id: decision_id.to_string(),
                    reason: DenyReason::ActorDisabled {
                        actor_id: request.actor_id.clone(),
                    },
                    matched_deny_rules: Vec::new(),
                }),
                Vec::new(),
            );
        }

        if let Some(capability) = &request.capability {
            if !actor.capabilities.contains(capability) {
                return (
                    DecisionOutcome::Denied(DeniedDecision {
                        decision_id: decision_id.to_string(),
                        reason: DenyReason::MissingCapability {
                            actor_id: request.actor_id.clone(),
                            capability_id: capability.id.clone(),
                        },
                        matched_deny_rules: Vec::new(),
                    }),
                    Vec::new(),
                );
            }
        }

        let matching_rules = self
            .policy
            .rules
            .values()
            .filter(|rule| rule.matches(actor, request))
            .collect::<Vec<_>>();
        let deny_rule_ids = matching_rules
            .iter()
            .filter(|rule| rule.effect == RuleEffect::Deny)
            .map(|rule| rule.id.clone())
            .collect::<Vec<_>>();

        if !deny_rule_ids.is_empty() {
            return (
                DecisionOutcome::Denied(DeniedDecision {
                    decision_id: decision_id.to_string(),
                    reason: DenyReason::ExplicitDeny {
                        rule_ids: deny_rule_ids.clone(),
                    },
                    matched_deny_rules: deny_rule_ids.clone(),
                }),
                deny_rule_ids,
            );
        }

        let allow_rules = matching_rules
            .iter()
            .filter(|rule| rule.effect == RuleEffect::Allow)
            .copied()
            .collect::<Vec<_>>();
        let allow_rule_ids = allow_rules
            .iter()
            .map(|rule| rule.id.clone())
            .collect::<Vec<_>>();

        if allow_rules.is_empty() {
            return (
                DecisionOutcome::Denied(DeniedDecision {
                    decision_id: decision_id.to_string(),
                    reason: DenyReason::NoAllowRule,
                    matched_deny_rules: Vec::new(),
                }),
                Vec::new(),
            );
        }

        let approval = request
            .approval_id
            .as_ref()
            .and_then(|approval_id| self.approvals.get(approval_id));

        if let Some(approval_id) = &request.approval_id {
            match approval {
                Some(record) if !record.matches_request(request) => {
                    return (
                        DecisionOutcome::Denied(DeniedDecision {
                            decision_id: decision_id.to_string(),
                            reason: DenyReason::ApprovalDoesNotMatchRequest {
                                approval_id: approval_id.clone(),
                            },
                            matched_deny_rules: Vec::new(),
                        }),
                        allow_rule_ids,
                    );
                }
                Some(record)
                    if matches!(
                        record.status,
                        ApprovalStatus::Rejected | ApprovalStatus::Revoked
                    ) =>
                {
                    return (
                        DecisionOutcome::Denied(DeniedDecision {
                            decision_id: decision_id.to_string(),
                            reason: DenyReason::ApprovalRejected {
                                approval_id: approval_id.clone(),
                            },
                            matched_deny_rules: Vec::new(),
                        }),
                        allow_rule_ids,
                    );
                }
                _ => {}
            }
        }

        let mut gates = GateRequirements::for_risk(request.risk);
        for rule in &allow_rules {
            gates.merge(&rule.gates);
        }

        let missing_gates = gates.missing_for(request, approval);
        if !missing_gates.is_empty() {
            return (
                DecisionOutcome::ApprovalRequired(ApprovalRequiredDecision {
                    decision_id: decision_id.to_string(),
                    missing_gates,
                    matched_allow_rules: allow_rule_ids.clone(),
                }),
                allow_rule_ids,
            );
        }

        (
            DecisionOutcome::Allowed(AllowedDecision {
                decision_id: decision_id.to_string(),
                matched_allow_rules: allow_rule_ids.clone(),
                satisfied_gates: gates.required,
            }),
            allow_rule_ids,
        )
    }
}

#[derive(Debug, Clone)]
pub struct SecretRedactor {
    sensitive_keys: BTreeSet<String>,
}

impl Default for SecretRedactor {
    fn default() -> Self {
        Self {
            sensitive_keys: [
                "authorization",
                "api_key",
                "apikey",
                "access_token",
                "refresh_token",
                "token",
                "password",
                "passwd",
                "secret",
                "credential",
                concat!("private", "_", "key"),
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }
}

impl SecretRedactor {
    pub fn redact_text(&self, text: &str) -> String {
        let mut redacted = text.to_string();
        for pattern in secret_patterns() {
            redacted = pattern.replace_all(&redacted, REDACTED).into_owned();
        }
        redacted
    }

    pub fn redact_metadata(&self, metadata: &Metadata) -> Metadata {
        metadata
            .iter()
            .map(|(key, value)| {
                let redacted_value = if self.is_sensitive_key(key) {
                    Value::String(REDACTED.to_string())
                } else {
                    self.redact_value(value)
                };
                (key.clone(), redacted_value)
            })
            .collect()
    }

    fn redact_value(&self, value: &Value) -> Value {
        match value {
            Value::String(text) => Value::String(self.redact_text(text)),
            Value::Array(values) => Value::Array(
                values
                    .iter()
                    .map(|value| self.redact_value(value))
                    .collect(),
            ),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(key, value)| {
                        let redacted_value = if self.is_sensitive_key(key) {
                            Value::String(REDACTED.to_string())
                        } else {
                            self.redact_value(value)
                        };
                        (key.clone(), redacted_value)
                    })
                    .collect::<Map<_, _>>(),
            ),
            _ => value.clone(),
        }
    }

    fn is_sensitive_key(&self, key: &str) -> bool {
        let normalized = key
            .chars()
            .filter(|ch| *ch != '-' && *ch != '_')
            .flat_map(char::to_lowercase)
            .collect::<String>();
        self.sensitive_keys
            .iter()
            .map(|key| key.replace('_', ""))
            .any(|sensitive| normalized.contains(&sensitive))
    }
}

fn secret_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            r"(?i)bearer\s+[A-Za-z0-9._~+/=-]{12,}",
            r"sk-[A-Za-z0-9]{16,}",
            r"gh[pousr]_[A-Za-z0-9_]{16,}",
            r"(?i)(api[_-]?key|access[_-]?token|refresh[_-]?token|token|password|secret)\s*[:=]\s*[^,\s;]+",
        ]
        .into_iter()
        .map(|pattern| Regex::new(pattern).expect("static redaction regex"))
        .collect()
    })
}

fn wildcard_matches(pattern: &str, value: &str) -> bool {
    if pattern == "*" || pattern == "**" {
        return true;
    }

    let mut regex = String::from("^");
    for ch in pattern.chars() {
        match ch {
            '*' => regex.push_str(".*"),
            '?' => regex.push('.'),
            other => regex.push_str(&regex::escape(&other.to_string())),
        }
    }
    regex.push('$');

    match Regex::new(&regex) {
        Ok(regex) => regex.is_match(value),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unauthorized_file_action_is_denied_and_audited() {
        let mut manager = GovernanceManager::new(policy_with_actor(agent("agent/main")));
        let request = ActionRequest::new(
            "agent/main",
            ActionKind::Write,
            Resource::new(ResourceKind::File, "workspace://project/src/lib.rs"),
            ActionRisk::Medium,
        );

        let decision = manager.evaluate(request);

        assert!(matches!(
            decision,
            DecisionOutcome::Denied(DeniedDecision {
                reason: DenyReason::NoAllowRule,
                ..
            })
        ));
        assert_eq!(manager.decisions().len(), 1);
        assert_eq!(manager.decisions()[0].sequence, 1);
    }

    #[test]
    fn unauthorized_connector_action_is_denied_and_audited() {
        let mut manager = GovernanceManager::new(policy_with_actor(agent("agent/main")));
        let decision = manager.evaluate(ActionRequest::new(
            "agent/main",
            ActionKind::ConnectorCall,
            Resource::new(ResourceKind::Connector, "connector://gmail/default"),
            ActionRisk::External,
        ));

        assert!(matches!(
            decision,
            DecisionOutcome::Denied(DeniedDecision {
                reason: DenyReason::NoAllowRule,
                ..
            })
        ));
        assert_eq!(manager.export_audit().decisions.len(), 1);
    }

    #[test]
    fn allowlist_permits_matching_folder_scope() {
        let mut policy = policy_with_actor(agent("agent/main"));
        policy.upsert_rule(
            PolicyRule::allow(
                "allow-workspace-read",
                ActorSelector::Exact("agent/main".into()),
            )
            .with_action(ActionKind::Read)
            .with_resource(ResourceSelector::pattern(
                ResourceKind::Folder,
                "workspace://project/**",
            )),
        );
        let mut manager = GovernanceManager::new(policy);

        let decision = manager.evaluate(ActionRequest::new(
            "agent/main",
            ActionKind::Read,
            Resource::new(ResourceKind::Folder, "workspace://project/src"),
            ActionRisk::Low,
        ));

        assert!(matches!(decision, DecisionOutcome::Allowed(_)));
    }

    #[test]
    fn high_risk_tool_action_requires_approval() {
        let mut policy = policy_with_actor(agent("agent/main"));
        policy.upsert_rule(
            PolicyRule::allow("allow-shell", ActorSelector::Exact("agent/main".into()))
                .with_action(ActionKind::SpawnProcess)
                .with_resource(ResourceSelector::exact(ResourceKind::Tool, "tool://shell")),
        );
        let mut manager = GovernanceManager::new(policy);

        let decision = manager.evaluate(ActionRequest::new(
            "agent/main",
            ActionKind::SpawnProcess,
            Resource::new(ResourceKind::Tool, "tool://shell"),
            ActionRisk::High,
        ));

        assert!(matches!(
            decision,
            DecisionOutcome::ApprovalRequired(ApprovalRequiredDecision {
                missing_gates,
                ..
            }) if missing_gates == BTreeSet::from([GateKind::Approval])
        ));
    }

    #[test]
    fn denylist_overrides_matching_allowlist() {
        let mut policy = policy_with_actor(agent("agent/main"));
        policy.upsert_rule(
            PolicyRule::allow(
                "allow-workspace-write",
                ActorSelector::Exact("agent/main".into()),
            )
            .with_action(ActionKind::Write)
            .with_resource(ResourceSelector::pattern(
                ResourceKind::File,
                "workspace://project/**",
            )),
        );
        policy.upsert_rule(
            PolicyRule::deny("deny-secrets", ActorSelector::Exact("agent/main".into()))
                .with_action(ActionKind::Write)
                .with_resource(ResourceSelector::pattern(
                    ResourceKind::File,
                    "workspace://project/.env*",
                )),
        );
        let mut manager = GovernanceManager::new(policy);

        let decision = manager.evaluate(ActionRequest::new(
            "agent/main",
            ActionKind::Write,
            Resource::new(ResourceKind::File, "workspace://project/.env"),
            ActionRisk::Medium,
        ));

        assert!(matches!(
            decision,
            DecisionOutcome::Denied(DeniedDecision {
                reason: DenyReason::ExplicitDeny { .. },
                ..
            })
        ));
    }

    #[test]
    fn destructive_external_action_requires_approval_evidence_rollback_and_checkpoint() {
        let mut policy = policy_with_actor(agent("agent/main"));
        policy.upsert_rule(
            PolicyRule::allow(
                "allow-release-delete",
                ActorSelector::Exact("agent/main".into()),
            )
            .with_action(ActionKind::Delete)
            .with_resource(ResourceSelector::exact(
                ResourceKind::ExternalSystem,
                "github://repo/releases/v1",
            )),
        );
        let mut manager = GovernanceManager::new(policy);
        let request = destructive_external_request();

        let decision = manager.evaluate(request.clone());

        let DecisionOutcome::ApprovalRequired(required) = decision else {
            panic!("expected approval-required decision");
        };
        assert_eq!(
            required.missing_gates,
            BTreeSet::from([
                GateKind::Approval,
                GateKind::Rollback,
                GateKind::Checkpoint,
                GateKind::Evidence
            ])
        );

        manager.record_approval(
            ApprovalRecord::approved_for(
                "approval/delete-release",
                "agent/main",
                ActionKind::Delete,
                Resource::new(ResourceKind::ExternalSystem, "github://repo/releases/v1"),
                "user/alice",
            )
            .with_evidence(EvidenceBundle {
                rollback: Some(RollbackPlan::new(
                    "Restore the release from the recorded tag.",
                    "gh release create v1 --notes-file release-notes.md",
                )),
                checkpoint: Some(CheckpointRef::new(
                    "checkpoint/release-v1",
                    "Release metadata exported before deletion.",
                )),
                artifacts: vec![EvidenceArtifact::new(
                    "evidence/release-export",
                    "Pre-delete release export captured.",
                )],
            }),
        );

        let allowed = manager.evaluate(request.with_approval("approval/delete-release"));

        assert!(matches!(allowed, DecisionOutcome::Allowed(_)));
    }

    #[test]
    fn rejected_approval_denies_even_when_rule_allows() {
        let mut policy = policy_with_actor(agent("agent/main"));
        policy.upsert_rule(
            PolicyRule::allow("allow-provider", ActorSelector::Exact("agent/main".into()))
                .with_action(ActionKind::ProviderCall)
                .with_resource(ResourceSelector::exact(
                    ResourceKind::Provider,
                    "provider://billing",
                )),
        );
        let mut manager = GovernanceManager::new(policy);
        let mut approval = ApprovalRecord::approved_for(
            "approval/rejected",
            "agent/main",
            ActionKind::ProviderCall,
            Resource::new(ResourceKind::Provider, "provider://billing"),
            "user/alice",
        );
        approval.status = ApprovalStatus::Rejected;
        manager.record_approval(approval);

        let decision = manager.evaluate(
            ActionRequest::new(
                "agent/main",
                ActionKind::ProviderCall,
                Resource::new(ResourceKind::Provider, "provider://billing"),
                ActionRisk::External,
            )
            .with_approval("approval/rejected"),
        );

        assert!(matches!(
            decision,
            DecisionOutcome::Denied(DeniedDecision {
                reason: DenyReason::ApprovalRejected { .. },
                ..
            })
        ));
    }

    #[test]
    fn approval_matching_uses_raw_state_but_export_is_redacted() {
        let provider = format!("provider://billing/{}={}", "token", "abcdef1234567890");
        let approval_id = format!("approval/{}={}", "secret", "abcdef1234567890");
        let mut policy = policy_with_actor(agent("agent/main"));
        policy.upsert_rule(
            PolicyRule::allow("allow-provider", ActorSelector::Exact("agent/main".into()))
                .with_action(ActionKind::ProviderCall)
                .with_resource(ResourceSelector::exact(ResourceKind::Provider, &provider)),
        );
        let mut manager = GovernanceManager::new(policy);
        manager.record_approval(
            ApprovalRecord::approved_for(
                &approval_id,
                "agent/main",
                ActionKind::ProviderCall,
                Resource::new(ResourceKind::Provider, &provider),
                "user/alice",
            )
            .with_evidence(EvidenceBundle {
                artifacts: vec![EvidenceArtifact::new(
                    "evidence/provider-approval",
                    "User approved provider access.",
                )],
                ..EvidenceBundle::default()
            }),
        );

        let decision = manager.evaluate(
            ActionRequest::new(
                "agent/main",
                ActionKind::ProviderCall,
                Resource::new(ResourceKind::Provider, &provider),
                ActionRisk::External,
            )
            .with_approval(&approval_id),
        );
        let export_json = serde_json::to_string(&manager.export_audit()).unwrap();

        assert!(matches!(decision, DecisionOutcome::Allowed(_)));
        assert!(!export_json.contains("abcdef1234567890"));
        assert!(export_json.contains(REDACTED));
    }

    #[test]
    fn capability_checks_are_per_actor_and_deterministic() {
        let actor = GovernanceActor::enabled("subagent/research", ActorRole::Subagent)
            .with_capabilities([Capability::new("browser.search")]);
        let mut policy = policy_with_actor(actor);
        policy.upsert_rule(
            PolicyRule::allow(
                "allow-browser-search",
                ActorSelector::Role(ActorRole::Subagent),
            )
            .for_capability(Capability::new("browser.search"))
            .with_action(ActionKind::BrowserAction)
            .with_resource(ResourceSelector::pattern(
                ResourceKind::Browser,
                "browser://*",
            )),
        );
        let mut manager = GovernanceManager::new(policy);

        let denied = manager.evaluate(
            ActionRequest::new(
                "subagent/research",
                ActionKind::BrowserAction,
                Resource::new(ResourceKind::Browser, "browser://tab/1"),
                ActionRisk::Medium,
            )
            .with_capability(Capability::new("browser.click")),
        );
        let allowed = manager.evaluate(
            ActionRequest::new(
                "subagent/research",
                ActionKind::BrowserAction,
                Resource::new(ResourceKind::Browser, "browser://tab/1"),
                ActionRisk::Medium,
            )
            .with_capability(Capability::new("browser.search")),
        );

        assert!(matches!(
            denied,
            DecisionOutcome::Denied(DeniedDecision {
                reason: DenyReason::MissingCapability { .. },
                ..
            })
        ));
        assert!(matches!(allowed, DecisionOutcome::Allowed(_)));
        assert_eq!(manager.decisions()[0].decision_id, "decision/1");
        assert_eq!(manager.decisions()[1].decision_id, "decision/2");
    }

    #[test]
    fn redaction_removes_sensitive_free_text_and_structured_metadata_from_exports() {
        let mut policy = policy_with_actor(agent("agent/main"));
        policy.upsert_rule(
            PolicyRule::allow(
                "allow-log-export",
                ActorSelector::Exact("agent/main".into()),
            )
            .with_action(ActionKind::Export)
            .with_resource(ResourceSelector::exact(
                ResourceKind::Artifact,
                "artifact://session-log",
            )),
        );
        let mut manager = GovernanceManager::new(policy);
        let synthetic_key = format!("{}{}", "sk-", "1234567890abcdef");
        let bearer = format!("{} {}", "Bearer", "abcdef1234567890");
        let credential_line = format!("{}={}", "password", "hunter2");
        let marker_line = format!("{}={}", "token", "abcdef1234567890");
        let mut metadata = Metadata::new();
        metadata.insert("api_key".to_string(), json!(synthetic_key.clone()));
        metadata.insert(
            "nested".to_string(),
            json!({
                "authorization": bearer,
                "comment": credential_line
            }),
        );

        let request = ActionRequest::new(
            "agent/main",
            ActionKind::Export,
            Resource::new(ResourceKind::Artifact, "artifact://session-log"),
            ActionRisk::Low,
        )
        .with_justification(format!("send with {bearer}"))
        .with_evidence(EvidenceBundle {
            artifacts: vec![EvidenceArtifact {
                id: "evidence/1".to_string(),
                description: format!("captured {marker_line}"),
                metadata,
            }],
            ..EvidenceBundle::default()
        });

        manager.evaluate(request);
        let export_json = serde_json::to_string(&manager.export_audit()).unwrap();

        assert!(!export_json.contains(&synthetic_key));
        assert!(!export_json.contains("abcdef1234567890"));
        assert!(!export_json.contains("hunter2"));
        assert!(export_json.contains(REDACTED));
    }

    #[test]
    fn audit_export_roundtrips_and_continues_sequence() {
        let mut policy = policy_with_actor(agent("agent/main"));
        policy.upsert_rule(
            PolicyRule::allow("allow-read", ActorSelector::Exact("agent/main".into()))
                .with_action(ActionKind::Read)
                .with_resource(ResourceSelector::any()),
        );
        let mut manager = GovernanceManager::new(policy.clone());
        manager.evaluate(ActionRequest::new(
            "agent/main",
            ActionKind::Read,
            Resource::new(ResourceKind::File, "workspace://project/README.md"),
            ActionRisk::Low,
        ));
        let export = manager.export_audit();

        let mut restored = GovernanceManager::from_audit_export(policy, export).unwrap();
        let decision = restored.evaluate(ActionRequest::new(
            "agent/main",
            ActionKind::Read,
            Resource::new(ResourceKind::File, "workspace://project/src/lib.rs"),
            ActionRisk::Low,
        ));

        assert_eq!(decision.decision_id(), "decision/2");
        assert_eq!(restored.export_audit().decisions.len(), 2);
    }

    fn policy_with_actor(actor: GovernanceActor) -> GovernancePolicy {
        let mut policy = GovernancePolicy::new();
        policy.upsert_actor(actor);
        policy
    }

    fn agent(id: &str) -> GovernanceActor {
        GovernanceActor::enabled(id, ActorRole::Agent)
    }

    fn destructive_external_request() -> ActionRequest {
        ActionRequest::new(
            "agent/main",
            ActionKind::Delete,
            Resource::new(ResourceKind::ExternalSystem, "github://repo/releases/v1"),
            ActionRisk::DestructiveExternal,
        )
    }
}
