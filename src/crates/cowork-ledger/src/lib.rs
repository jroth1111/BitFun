//! Cowork-owned durable ledger domain model.
//!
//! This crate is intentionally a domain/model boundary, not a protocol DTO
//! crate. GUI, CLI, context summaries, and compaction outputs can request
//! changes, but root objective and root constraint authority lives here and is
//! initialized exactly once.
//!
//! Relationship model:
//! - `RootScope` owns the immutable root objective, acceptance criteria, and
//!   root constraints for a Cowork run.
//! - `Task`, `Blocker`, `Evidence`, `Checkpoint`, `Artifact`, `SessionRecord`,
//!   `StopReasonRecord`, `Waiver`, and `ProvenanceLink` records use typed stable
//!   IDs and `EntityRef` edges to name relationships.
//! - `CoworkLedger::apply_update` is the only mutation entry point. It validates
//!   referenced entities before appending relationship edges, and it rejects any
//!   compaction summary whose observed root scope differs from the initialized
//!   root.

use chrono::{DateTime, Utc};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

pub const LEDGER_SCHEMA_VERSION: &str = "cowork.ledger.v1";

pub type Metadata = BTreeMap<String, serde_json::Value>;

macro_rules! define_id {
    ($name:ident, $kind:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, LedgerError> {
                let value = value.into();
                validate_id($kind, &value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl FromStr for $name {
            type Err = LedgerError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = LedgerError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = LedgerError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                struct IdVisitor;

                impl<'de> Visitor<'de> for IdVisitor {
                    type Value = $name;

                    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                        write!(formatter, "a valid {} string", $kind)
                    }

                    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
                    where
                        E: de::Error,
                    {
                        $name::parse(value).map_err(E::custom)
                    }

                    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
                    where
                        E: de::Error,
                    {
                        $name::parse(value).map_err(E::custom)
                    }
                }

                deserializer.deserialize_string(IdVisitor)
            }
        }
    };
}

define_id!(RootObjectiveId, "root objective id");
define_id!(RootConstraintId, "root constraint id");
define_id!(TaskId, "task id");
define_id!(BlockerId, "blocker id");
define_id!(EvidenceId, "evidence id");
define_id!(CheckpointId, "checkpoint id");
define_id!(ArtifactId, "artifact id");
define_id!(SessionId, "session id");
define_id!(StopReasonId, "stop reason id");
define_id!(WaiverId, "waiver id");
define_id!(ProvenanceLinkId, "provenance link id");

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LedgerError {
    #[error("invalid {kind} '{value}': {reason}")]
    InvalidId {
        kind: &'static str,
        value: String,
        reason: &'static str,
    },
    #[error("required text field '{field}' is empty")]
    EmptyText { field: &'static str },
    #[error("duplicate {entity} id '{id}'")]
    DuplicateId { entity: &'static str, id: String },
    #[error("unknown reference from {from:?} to {to:?}")]
    UnknownReference { from: EntityRef, to: EntityRef },
    #[error("task '{task_id}' does not belong to root objective '{objective_id}'")]
    TaskObjectiveMismatch {
        task_id: TaskId,
        objective_id: RootObjectiveId,
    },
    #[error(
        "root scope mutation rejected for {target:?}: current '{current}', attempted '{attempted}'"
    )]
    ImmutableRootMutationRejected {
        target: RootMutationTarget,
        current: String,
        attempted: String,
    },
    #[error("missing root scope observation in compaction summary update")]
    MissingRootObservation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootMutationTarget {
    ObjectiveText,
    AcceptanceCriteria,
    RootConstraints,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAuthority {
    CoworkLedger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalAuthority {
    RequestOnly,
    NonAuthoritativeContext,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorityInvariant {
    pub objective_authority: RuntimeAuthority,
    pub constraint_authority: RuntimeAuthority,
    pub task_authority: RuntimeAuthority,
    pub blocker_authority: RuntimeAuthority,
    pub evidence_authority: RuntimeAuthority,
    pub checkpoint_authority: RuntimeAuthority,
    pub artifact_authority: RuntimeAuthority,
    pub session_authority: RuntimeAuthority,
    pub waiver_authority: RuntimeAuthority,
    pub provenance_authority: RuntimeAuthority,
    pub client_authority: ExternalAuthority,
    pub compaction_authority: ExternalAuthority,
}

impl AuthorityInvariant {
    pub const fn cowork_authoritative() -> Self {
        Self {
            objective_authority: RuntimeAuthority::CoworkLedger,
            constraint_authority: RuntimeAuthority::CoworkLedger,
            task_authority: RuntimeAuthority::CoworkLedger,
            blocker_authority: RuntimeAuthority::CoworkLedger,
            evidence_authority: RuntimeAuthority::CoworkLedger,
            checkpoint_authority: RuntimeAuthority::CoworkLedger,
            artifact_authority: RuntimeAuthority::CoworkLedger,
            session_authority: RuntimeAuthority::CoworkLedger,
            waiver_authority: RuntimeAuthority::CoworkLedger,
            provenance_authority: RuntimeAuthority::CoworkLedger,
            client_authority: ExternalAuthority::RequestOnly,
            compaction_authority: ExternalAuthority::NonAuthoritativeContext,
        }
    }
}

impl Default for AuthorityInvariant {
    fn default() -> Self {
        Self::cowork_authoritative()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerInitialization {
    pub root_objective_id: RootObjectiveId,
    pub root_objective_text: String,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    pub root_constraints: Vec<RootConstraintInitialization>,
    pub initialized_at: DateTime<Utc>,
    pub initialized_by: ActorRef,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootConstraintInitialization {
    pub id: RootConstraintId,
    pub text: String,
    pub severity: ConstraintSeverity,
    pub source: ConstraintSource,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootScope {
    objective: RootObjective,
    constraints: BTreeMap<RootConstraintId, RootConstraint>,
    initialized_at: DateTime<Utc>,
    initialized_by: ActorRef,
}

impl RootScope {
    pub fn objective(&self) -> &RootObjective {
        &self.objective
    }

    pub fn constraints(&self) -> &BTreeMap<RootConstraintId, RootConstraint> {
        &self.constraints
    }

    pub fn initialized_at(&self) -> DateTime<Utc> {
        self.initialized_at
    }

    pub fn initialized_by(&self) -> &ActorRef {
        &self.initialized_by
    }

    pub fn observation(&self) -> RootScopeObservation {
        RootScopeObservation {
            objective_id: self.objective.id.clone(),
            objective_text: self.objective.text.clone(),
            acceptance_criteria: self.objective.acceptance_criteria.clone(),
            constraints: self
                .constraints
                .values()
                .map(RootConstraintObservation::from)
                .collect(),
        }
    }

    fn verify_observation(&self, observed: &RootScopeObservation) -> Result<(), LedgerError> {
        if observed.objective_id != self.objective.id {
            return Err(LedgerError::ImmutableRootMutationRejected {
                target: RootMutationTarget::ObjectiveText,
                current: self.objective.id.to_string(),
                attempted: observed.objective_id.to_string(),
            });
        }

        if observed.objective_text != self.objective.text {
            return Err(LedgerError::ImmutableRootMutationRejected {
                target: RootMutationTarget::ObjectiveText,
                current: self.objective.text.clone(),
                attempted: observed.objective_text.clone(),
            });
        }

        if observed.acceptance_criteria != self.objective.acceptance_criteria {
            return Err(LedgerError::ImmutableRootMutationRejected {
                target: RootMutationTarget::AcceptanceCriteria,
                current: format!("{:?}", self.objective.acceptance_criteria),
                attempted: format!("{:?}", observed.acceptance_criteria),
            });
        }

        let current_constraints = self
            .constraints
            .values()
            .map(RootConstraintObservation::from)
            .map(|constraint| (constraint.id.clone(), constraint))
            .collect::<BTreeMap<_, _>>();
        let observed_constraints = observed
            .constraints
            .iter()
            .cloned()
            .map(|constraint| (constraint.id.clone(), constraint))
            .collect::<BTreeMap<_, _>>();

        if observed_constraints.len() != observed.constraints.len()
            || observed_constraints != current_constraints
        {
            return Err(LedgerError::ImmutableRootMutationRejected {
                target: RootMutationTarget::RootConstraints,
                current: format!("{current_constraints:?}"),
                attempted: format!("{observed_constraints:?}"),
            });
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootObjective {
    id: RootObjectiveId,
    text: String,
    acceptance_criteria: Vec<String>,
    metadata: Metadata,
}

impl RootObjective {
    pub fn id(&self) -> &RootObjectiveId {
        &self.id
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn acceptance_criteria(&self) -> &[String] {
        &self.acceptance_criteria
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootConstraint {
    id: RootConstraintId,
    text: String,
    severity: ConstraintSeverity,
    source: ConstraintSource,
    metadata: Metadata,
}

impl RootConstraint {
    pub fn id(&self) -> &RootConstraintId {
        &self.id
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn severity(&self) -> ConstraintSeverity {
        self.severity
    }

    pub fn source(&self) -> ConstraintSource {
        self.source
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootScopeObservation {
    pub objective_id: RootObjectiveId,
    pub objective_text: String,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    pub constraints: Vec<RootConstraintObservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootConstraintObservation {
    pub id: RootConstraintId,
    pub text: String,
    pub severity: ConstraintSeverity,
    pub source: ConstraintSource,
}

impl From<&RootConstraint> for RootConstraintObservation {
    fn from(constraint: &RootConstraint) -> Self {
        Self {
            id: constraint.id.clone(),
            text: constraint.text.clone(),
            severity: constraint.severity,
            source: constraint.source,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintSeverity {
    Required,
    SafetyCritical,
    Advisory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintSource {
    User,
    Bead,
    RepoPolicy,
    RuntimePolicy,
}

/// Authoritative schema aggregate for a Cowork run.
///
/// Fields are private so callers cannot rewrite the root objective or
/// relationship maps directly. New state must enter through `apply_update`,
/// which preserves root-scope immutability and validates referenced IDs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoworkLedger {
    schema_version: String,
    authority: AuthorityInvariant,
    root: RootScope,
    objective_progress: ObjectiveProgress,
    tasks: BTreeMap<TaskId, Task>,
    blockers: BTreeMap<BlockerId, Blocker>,
    evidence: BTreeMap<EvidenceId, Evidence>,
    checkpoints: BTreeMap<CheckpointId, Checkpoint>,
    artifacts: BTreeMap<ArtifactId, Artifact>,
    sessions: BTreeMap<SessionId, SessionRecord>,
    stop_reasons: BTreeMap<StopReasonId, StopReasonRecord>,
    waivers: BTreeMap<WaiverId, Waiver>,
    provenance_links: BTreeMap<ProvenanceLinkId, ProvenanceLink>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    metadata: Metadata,
}

impl CoworkLedger {
    pub fn initialize(initialization: LedgerInitialization) -> Result<Self, LedgerError> {
        require_text("rootObjective.text", &initialization.root_objective_text)?;

        let mut criteria = Vec::with_capacity(initialization.acceptance_criteria.len());
        for criterion in initialization.acceptance_criteria {
            require_text("rootObjective.acceptanceCriteria", &criterion)?;
            criteria.push(criterion);
        }

        let mut constraints = BTreeMap::new();
        for constraint in initialization.root_constraints {
            require_text("rootConstraint.text", &constraint.text)?;
            let id = constraint.id;
            if constraints
                .insert(
                    id.clone(),
                    RootConstraint {
                        id: id.clone(),
                        text: constraint.text,
                        severity: constraint.severity,
                        source: constraint.source,
                        metadata: constraint.metadata,
                    },
                )
                .is_some()
            {
                return Err(LedgerError::DuplicateId {
                    entity: "root constraint",
                    id: id.to_string(),
                });
            }
        }

        Ok(Self {
            schema_version: LEDGER_SCHEMA_VERSION.to_string(),
            authority: AuthorityInvariant::cowork_authoritative(),
            root: RootScope {
                objective: RootObjective {
                    id: initialization.root_objective_id.clone(),
                    text: initialization.root_objective_text,
                    acceptance_criteria: criteria,
                    metadata: initialization.metadata.clone(),
                },
                constraints,
                initialized_at: initialization.initialized_at,
                initialized_by: initialization.initialized_by,
            },
            objective_progress: ObjectiveProgress {
                objective_id: initialization.root_objective_id,
                status: ObjectiveStatus::Active,
                updated_at: initialization.initialized_at,
                stop_reason_id: None,
                metadata: Metadata::new(),
            },
            tasks: BTreeMap::new(),
            blockers: BTreeMap::new(),
            evidence: BTreeMap::new(),
            checkpoints: BTreeMap::new(),
            artifacts: BTreeMap::new(),
            sessions: BTreeMap::new(),
            stop_reasons: BTreeMap::new(),
            waivers: BTreeMap::new(),
            provenance_links: BTreeMap::new(),
            metadata: initialization.metadata,
        })
    }

    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    pub fn authority(&self) -> &AuthorityInvariant {
        &self.authority
    }

    pub fn root(&self) -> &RootScope {
        &self.root
    }

    pub fn objective_progress(&self) -> &ObjectiveProgress {
        &self.objective_progress
    }

    pub fn tasks(&self) -> &BTreeMap<TaskId, Task> {
        &self.tasks
    }

    pub fn blockers(&self) -> &BTreeMap<BlockerId, Blocker> {
        &self.blockers
    }

    pub fn evidence(&self) -> &BTreeMap<EvidenceId, Evidence> {
        &self.evidence
    }

    pub fn checkpoints(&self) -> &BTreeMap<CheckpointId, Checkpoint> {
        &self.checkpoints
    }

    pub fn artifacts(&self) -> &BTreeMap<ArtifactId, Artifact> {
        &self.artifacts
    }

    pub fn sessions(&self) -> &BTreeMap<SessionId, SessionRecord> {
        &self.sessions
    }

    pub fn stop_reasons(&self) -> &BTreeMap<StopReasonId, StopReasonRecord> {
        &self.stop_reasons
    }

    pub fn waivers(&self) -> &BTreeMap<WaiverId, Waiver> {
        &self.waivers
    }

    pub fn provenance_links(&self) -> &BTreeMap<ProvenanceLinkId, ProvenanceLink> {
        &self.provenance_links
    }

    pub fn apply_update(&mut self, update: LedgerUpdate) -> Result<ApplyReport, LedgerError> {
        match update {
            LedgerUpdate::RegisterTask(task) => {
                self.record_task(task)?;
                Ok(ApplyReport::applied("register_task"))
            }
            LedgerUpdate::UpdateTaskStatus(update) => {
                self.update_task_status(update)?;
                Ok(ApplyReport::applied("update_task_status"))
            }
            LedgerUpdate::UpdateObjectiveProgress(update) => {
                self.update_objective_progress(update)?;
                Ok(ApplyReport::applied("update_objective_progress"))
            }
            LedgerUpdate::RecordBlocker(blocker) => {
                self.record_blocker(blocker)?;
                Ok(ApplyReport::applied("record_blocker"))
            }
            LedgerUpdate::RecordEvidence(evidence) => {
                self.record_evidence(evidence)?;
                Ok(ApplyReport::applied("record_evidence"))
            }
            LedgerUpdate::RecordCheckpoint(checkpoint) => {
                self.record_checkpoint(checkpoint)?;
                Ok(ApplyReport::applied("record_checkpoint"))
            }
            LedgerUpdate::RecordArtifact(artifact) => {
                self.record_artifact(artifact)?;
                Ok(ApplyReport::applied("record_artifact"))
            }
            LedgerUpdate::RecordSession(session) => {
                self.record_session(session)?;
                Ok(ApplyReport::applied("record_session"))
            }
            LedgerUpdate::RecordStopReason(stop_reason) => {
                self.record_stop_reason(stop_reason)?;
                Ok(ApplyReport::applied("record_stop_reason"))
            }
            LedgerUpdate::RecordWaiver(waiver) => {
                self.record_waiver(waiver)?;
                Ok(ApplyReport::applied("record_waiver"))
            }
            LedgerUpdate::RecordProvenanceLink(link) => {
                self.record_provenance_link(link)?;
                Ok(ApplyReport::applied("record_provenance_link"))
            }
            LedgerUpdate::AttemptRootObjectiveMutation { attempted_text } => {
                Err(LedgerError::ImmutableRootMutationRejected {
                    target: RootMutationTarget::ObjectiveText,
                    current: self.root.objective.text.clone(),
                    attempted: attempted_text,
                })
            }
            LedgerUpdate::AttemptRootConstraintMutation {
                attempted_constraints,
            } => Err(LedgerError::ImmutableRootMutationRejected {
                target: RootMutationTarget::RootConstraints,
                current: format!("{:?}", self.root.observation().constraints),
                attempted: format!("{attempted_constraints:?}"),
            }),
            LedgerUpdate::ApplyCompactionSummary(summary) => self.apply_compaction_summary(summary),
        }
    }

    fn apply_compaction_summary(
        &mut self,
        summary: CompactionSummaryUpdate,
    ) -> Result<ApplyReport, LedgerError> {
        self.root.verify_observation(&summary.observed_root)?;

        let mut applied = ApplyReport::applied("apply_compaction_summary");

        if let Some(progress) = summary.objective_progress {
            self.update_objective_progress(progress)?;
            applied.effects.push("objective_progress".to_string());
        }

        for task in summary.tasks {
            self.record_task(task)?;
            applied.effects.push("task".to_string());
        }

        for blocker in summary.blockers {
            self.record_blocker(blocker)?;
            applied.effects.push("blocker".to_string());
        }

        for evidence in summary.evidence {
            self.record_evidence(evidence)?;
            applied.effects.push("evidence".to_string());
        }

        for artifact in summary.artifacts {
            self.record_artifact(artifact)?;
            applied.effects.push("artifact".to_string());
        }

        for checkpoint in summary.checkpoints {
            self.record_checkpoint(checkpoint)?;
            applied.effects.push("checkpoint".to_string());
        }

        for status_update in summary.task_status_updates {
            self.update_task_status(status_update)?;
            applied.effects.push("task_status".to_string());
        }

        for stop_reason in summary.stop_reasons {
            self.record_stop_reason(stop_reason)?;
            applied.effects.push("stop_reason".to_string());
        }

        for waiver in summary.waivers {
            self.record_waiver(waiver)?;
            applied.effects.push("waiver".to_string());
        }

        for link in summary.provenance_links {
            self.record_provenance_link(link)?;
            applied.effects.push("provenance_link".to_string());
        }

        Ok(applied)
    }

    fn record_task(&mut self, task: Task) -> Result<(), LedgerError> {
        require_text("task.title", &task.title)?;
        if task.objective_id != self.root.objective.id {
            return Err(LedgerError::TaskObjectiveMismatch {
                task_id: task.id,
                objective_id: task.objective_id,
            });
        }

        if self.tasks.contains_key(&task.id) {
            return Err(LedgerError::DuplicateId {
                entity: "task",
                id: task.id.to_string(),
            });
        }

        self.tasks.insert(task.id.clone(), task);
        Ok(())
    }

    fn update_task_status(&mut self, update: TaskStatusUpdate) -> Result<(), LedgerError> {
        if let Some(evidence_id) = &update.evidence_id {
            self.require_entity_exists(
                &update.subject_ref(),
                &EntityRef::Evidence(evidence_id.clone()),
            )?;
        }

        let Some(task) = self.tasks.get_mut(&update.task_id) else {
            return Err(LedgerError::UnknownReference {
                from: update.subject_ref(),
                to: EntityRef::Task(update.task_id),
            });
        };

        task.status = update.status;
        task.status_updated_at = update.updated_at;
        if let Some(evidence_id) = update.evidence_id {
            push_unique(&mut task.evidence_ids, evidence_id);
        }
        Ok(())
    }

    fn update_objective_progress(
        &mut self,
        update: ObjectiveProgressUpdate,
    ) -> Result<(), LedgerError> {
        if update.objective_id != self.root.objective.id {
            return Err(LedgerError::UnknownReference {
                from: EntityRef::Objective(update.objective_id.clone()),
                to: EntityRef::Objective(self.root.objective.id.clone()),
            });
        }

        if let Some(stop_reason_id) = &update.stop_reason_id {
            self.require_entity_exists(
                &EntityRef::Objective(update.objective_id.clone()),
                &EntityRef::StopReason(stop_reason_id.clone()),
            )?;
        }

        self.objective_progress.status = update.status;
        self.objective_progress.updated_at = update.updated_at;
        self.objective_progress.stop_reason_id = update.stop_reason_id;
        Ok(())
    }

    fn record_blocker(&mut self, blocker: Blocker) -> Result<(), LedgerError> {
        require_text("blocker.summary", &blocker.summary)?;
        for task_id in &blocker.task_ids {
            self.require_entity_exists(
                &EntityRef::Blocker(blocker.id.clone()),
                &EntityRef::Task(task_id.clone()),
            )?;
        }

        if self.blockers.contains_key(&blocker.id) {
            return Err(LedgerError::DuplicateId {
                entity: "blocker",
                id: blocker.id.to_string(),
            });
        }

        for task_id in &blocker.task_ids {
            if let Some(task) = self.tasks.get_mut(task_id) {
                push_unique(&mut task.blocker_ids, blocker.id.clone());
            }
        }

        self.blockers.insert(blocker.id.clone(), blocker);
        Ok(())
    }

    fn record_evidence(&mut self, evidence: Evidence) -> Result<(), LedgerError> {
        require_text("evidence.summary", &evidence.summary)?;
        for subject in &evidence.subjects {
            self.require_entity_exists(&EntityRef::Evidence(evidence.id.clone()), subject)?;
        }

        if self.evidence.contains_key(&evidence.id) {
            return Err(LedgerError::DuplicateId {
                entity: "evidence",
                id: evidence.id.to_string(),
            });
        }

        for subject in &evidence.subjects {
            if let EntityRef::Task(task_id) = subject {
                if let Some(task) = self.tasks.get_mut(task_id) {
                    push_unique(&mut task.evidence_ids, evidence.id.clone());
                }
            }
        }

        self.evidence.insert(evidence.id.clone(), evidence);
        Ok(())
    }

    fn record_checkpoint(&mut self, checkpoint: Checkpoint) -> Result<(), LedgerError> {
        require_text("checkpoint.summary", &checkpoint.summary)?;
        if checkpoint.objective_id != self.root.objective.id {
            return Err(LedgerError::UnknownReference {
                from: EntityRef::Checkpoint(checkpoint.id),
                to: EntityRef::Objective(checkpoint.objective_id),
            });
        }

        for task_id in &checkpoint.task_ids {
            self.require_entity_exists(
                &EntityRef::Checkpoint(checkpoint.id.clone()),
                &EntityRef::Task(task_id.clone()),
            )?;
        }

        for evidence_id in &checkpoint.evidence_ids {
            self.require_entity_exists(
                &EntityRef::Checkpoint(checkpoint.id.clone()),
                &EntityRef::Evidence(evidence_id.clone()),
            )?;
        }

        if self.checkpoints.contains_key(&checkpoint.id) {
            return Err(LedgerError::DuplicateId {
                entity: "checkpoint",
                id: checkpoint.id.to_string(),
            });
        }

        self.checkpoints.insert(checkpoint.id.clone(), checkpoint);
        Ok(())
    }

    fn record_artifact(&mut self, artifact: Artifact) -> Result<(), LedgerError> {
        require_text("artifact.title", &artifact.title)?;
        for evidence_id in &artifact.evidence_ids {
            self.require_entity_exists(
                &EntityRef::Artifact(artifact.id.clone()),
                &EntityRef::Evidence(evidence_id.clone()),
            )?;
        }

        self.require_entity_exists(
            &EntityRef::Artifact(artifact.id.clone()),
            &artifact.produced_by,
        )?;

        if self.artifacts.contains_key(&artifact.id) {
            return Err(LedgerError::DuplicateId {
                entity: "artifact",
                id: artifact.id.to_string(),
            });
        }

        self.artifacts.insert(artifact.id.clone(), artifact);
        Ok(())
    }

    fn record_session(&mut self, session: SessionRecord) -> Result<(), LedgerError> {
        if session.objective_id != self.root.objective.id {
            return Err(LedgerError::UnknownReference {
                from: EntityRef::Session(session.id),
                to: EntityRef::Objective(session.objective_id),
            });
        }

        if self.sessions.contains_key(&session.id) {
            return Err(LedgerError::DuplicateId {
                entity: "session",
                id: session.id.to_string(),
            });
        }

        self.sessions.insert(session.id.clone(), session);
        Ok(())
    }

    fn record_stop_reason(&mut self, stop_reason: StopReasonRecord) -> Result<(), LedgerError> {
        require_text("stopReason.summary", &stop_reason.summary)?;
        for subject in &stop_reason.subjects {
            self.require_entity_exists(&EntityRef::StopReason(stop_reason.id.clone()), subject)?;
        }

        if self.stop_reasons.contains_key(&stop_reason.id) {
            return Err(LedgerError::DuplicateId {
                entity: "stop reason",
                id: stop_reason.id.to_string(),
            });
        }

        self.stop_reasons
            .insert(stop_reason.id.clone(), stop_reason);
        Ok(())
    }

    fn record_waiver(&mut self, waiver: Waiver) -> Result<(), LedgerError> {
        require_text("waiver.reason", &waiver.reason)?;
        self.require_entity_exists(&EntityRef::Waiver(waiver.id.clone()), &waiver.waived_entity)?;

        if let Some(evidence_id) = &waiver.evidence_id {
            self.require_entity_exists(
                &EntityRef::Waiver(waiver.id.clone()),
                &EntityRef::Evidence(evidence_id.clone()),
            )?;
        }

        if self.waivers.contains_key(&waiver.id) {
            return Err(LedgerError::DuplicateId {
                entity: "waiver",
                id: waiver.id.to_string(),
            });
        }

        self.waivers.insert(waiver.id.clone(), waiver);
        Ok(())
    }

    fn record_provenance_link(&mut self, link: ProvenanceLink) -> Result<(), LedgerError> {
        self.require_entity_exists(&EntityRef::ProvenanceLink(link.id.clone()), &link.from)?;
        self.require_entity_exists(&EntityRef::ProvenanceLink(link.id.clone()), &link.to)?;

        if self.provenance_links.contains_key(&link.id) {
            return Err(LedgerError::DuplicateId {
                entity: "provenance link",
                id: link.id.to_string(),
            });
        }

        self.provenance_links.insert(link.id.clone(), link);
        Ok(())
    }

    fn require_entity_exists(&self, from: &EntityRef, to: &EntityRef) -> Result<(), LedgerError> {
        if self.contains_entity(to) {
            Ok(())
        } else {
            Err(LedgerError::UnknownReference {
                from: from.clone(),
                to: to.clone(),
            })
        }
    }

    fn contains_entity(&self, entity: &EntityRef) -> bool {
        match entity {
            EntityRef::Objective(id) => &self.root.objective.id == id,
            EntityRef::RootConstraint(id) => self.root.constraints.contains_key(id),
            EntityRef::Task(id) => self.tasks.contains_key(id),
            EntityRef::Blocker(id) => self.blockers.contains_key(id),
            EntityRef::Evidence(id) => self.evidence.contains_key(id),
            EntityRef::Checkpoint(id) => self.checkpoints.contains_key(id),
            EntityRef::Artifact(id) => self.artifacts.contains_key(id),
            EntityRef::Session(id) => self.sessions.contains_key(id),
            EntityRef::StopReason(id) => self.stop_reasons.contains_key(id),
            EntityRef::Waiver(id) => self.waivers.contains_key(id),
            EntityRef::ProvenanceLink(id) => self.provenance_links.contains_key(id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyReport {
    pub accepted: bool,
    pub update_kind: String,
    #[serde(default)]
    pub effects: Vec<String>,
}

impl ApplyReport {
    fn applied(update_kind: impl Into<String>) -> Self {
        Self {
            accepted: true,
            update_kind: update_kind.into(),
            effects: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum LedgerUpdate {
    RegisterTask(Task),
    UpdateTaskStatus(TaskStatusUpdate),
    UpdateObjectiveProgress(ObjectiveProgressUpdate),
    RecordBlocker(Blocker),
    RecordEvidence(Evidence),
    RecordCheckpoint(Checkpoint),
    RecordArtifact(Artifact),
    RecordSession(SessionRecord),
    RecordStopReason(StopReasonRecord),
    RecordWaiver(Waiver),
    RecordProvenanceLink(ProvenanceLink),
    AttemptRootObjectiveMutation {
        attempted_text: String,
    },
    AttemptRootConstraintMutation {
        attempted_constraints: Vec<RootConstraintObservation>,
    },
    ApplyCompactionSummary(CompactionSummaryUpdate),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionSummaryUpdate {
    pub observed_root: RootScopeObservation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub objective_progress: Option<ObjectiveProgressUpdate>,
    #[serde(default)]
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub task_status_updates: Vec<TaskStatusUpdate>,
    #[serde(default)]
    pub blockers: Vec<Blocker>,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    #[serde(default)]
    pub checkpoints: Vec<Checkpoint>,
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    #[serde(default)]
    pub stop_reasons: Vec<StopReasonRecord>,
    #[serde(default)]
    pub waivers: Vec<Waiver>,
    #[serde(default)]
    pub provenance_links: Vec<ProvenanceLink>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectiveProgress {
    pub objective_id: RootObjectiveId,
    pub status: ObjectiveStatus,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason_id: Option<StopReasonId>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectiveProgressUpdate {
    pub objective_id: RootObjectiveId,
    pub status: ObjectiveStatus,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason_id: Option<StopReasonId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveStatus {
    Active,
    Paused,
    Blocked,
    Verified,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: TaskId,
    pub objective_id: RootObjectiveId,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub status: TaskStatus,
    pub status_updated_at: DateTime<Utc>,
    #[serde(default)]
    pub dependencies: Vec<TaskId>,
    #[serde(default)]
    pub blocker_ids: Vec<BlockerId>,
    #[serde(default)]
    pub evidence_ids: Vec<EvidenceId>,
    #[serde(default)]
    pub artifact_ids: Vec<ArtifactId>,
    #[serde(default)]
    pub checkpoint_ids: Vec<CheckpointId>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatusUpdate {
    pub task_id: TaskId,
    pub status: TaskStatus,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_id: Option<EvidenceId>,
}

impl TaskStatusUpdate {
    fn subject_ref(&self) -> EntityRef {
        EntityRef::Task(self.task_id.clone())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    NotStarted,
    InProgress,
    Blocked,
    ImplementedUnverified,
    Verified,
    Descoped,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Blocker {
    pub id: BlockerId,
    pub summary: String,
    pub status: BlockerStatus,
    pub opened_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub task_ids: Vec<TaskId>,
    #[serde(default)]
    pub required_external_input: Vec<String>,
    #[serde(default)]
    pub evidence_ids: Vec<EvidenceId>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockerStatus {
    Open,
    Resolved,
    Waived,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub id: EvidenceId,
    pub kind: EvidenceKind,
    pub summary: String,
    pub collected_at: DateTime<Utc>,
    pub collected_by: ActorRef,
    pub result: EvidenceResult,
    #[serde(default)]
    pub subjects: Vec<EntityRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<CommandEvidence>,
    #[serde(default)]
    pub artifact_ids: Vec<ArtifactId>,
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
    UserInput,
    SubagentReport,
    CompactionSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceResult {
    Passed,
    Failed,
    Blocked,
    Informational,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandEvidence {
    pub command_line: String,
    pub cwd: String,
    pub exit_code: i32,
    pub started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout_excerpt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr_excerpt: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub environment: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Checkpoint {
    pub id: CheckpointId,
    pub objective_id: RootObjectiveId,
    pub sequence: u64,
    pub summary: String,
    pub created_at: DateTime<Utc>,
    pub created_by: ActorRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(default)]
    pub task_ids: Vec<TaskId>,
    #[serde(default)]
    pub evidence_ids: Vec<EvidenceId>,
    #[serde(default)]
    pub artifact_ids: Vec<ArtifactId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason_id: Option<StopReasonId>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub id: ArtifactId,
    pub kind: ArtifactKind,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    pub produced_by: EntityRef,
    #[serde(default)]
    pub evidence_ids: Vec<EvidenceId>,
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
    Log,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    pub id: SessionId,
    pub objective_id: RootObjectiveId,
    pub kind: SessionKind,
    pub status: SessionStatus,
    pub started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    pub actor: ActorRef,
    #[serde(default)]
    pub checkpoint_ids: Vec<CheckpointId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason_id: Option<StopReasonId>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Main,
    Worker,
    Subagent,
    Compaction,
    Audit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Running,
    Paused,
    Blocked,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StopReasonRecord {
    pub id: StopReasonId,
    pub kind: StopReasonKind,
    pub summary: String,
    pub recorded_at: DateTime<Utc>,
    pub recorded_by: ActorRef,
    #[serde(default)]
    pub subjects: Vec<EntityRef>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReasonKind {
    Verified,
    UserPaused,
    SessionCapacityReached,
    BlockedExternalInput,
    EnvironmentDegraded,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Waiver {
    pub id: WaiverId,
    pub waived_entity: EntityRef,
    pub reason: String,
    pub granted_by: ActorRef,
    pub granted_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_id: Option<EvidenceId>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvenanceLink {
    pub id: ProvenanceLinkId,
    pub kind: ProvenanceLinkKind,
    pub from: EntityRef,
    pub to: EntityRef,
    pub created_at: DateTime<Utc>,
    pub created_by: ActorRef,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceLinkKind {
    DefinesScope,
    DecomposesInto,
    DependsOn,
    Blocks,
    Resolves,
    Supports,
    Contradicts,
    Produced,
    Checkpointed,
    ObservedInSession,
    Waives,
    Supersedes,
    DerivedFrom,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum EntityRef {
    Objective(RootObjectiveId),
    RootConstraint(RootConstraintId),
    Task(TaskId),
    Blocker(BlockerId),
    Evidence(EvidenceId),
    Checkpoint(CheckpointId),
    Artifact(ArtifactId),
    Session(SessionId),
    StopReason(StopReasonId),
    Waiver(WaiverId),
    ProvenanceLink(ProvenanceLinkId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorRef {
    pub kind: ActorKind,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    User,
    CoworkRuntime,
    Worker,
    Subagent,
    System,
}

fn validate_id(kind: &'static str, value: &str) -> Result<(), LedgerError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(LedgerError::InvalidId {
            kind,
            value: value.to_string(),
            reason: "must not be empty",
        });
    }

    if trimmed != value {
        return Err(LedgerError::InvalidId {
            kind,
            value: value.to_string(),
            reason: "must not have leading or trailing whitespace",
        });
    }

    if value.len() > 128 {
        return Err(LedgerError::InvalidId {
            kind,
            value: value.to_string(),
            reason: "must be at most 128 bytes",
        });
    }

    if !value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
    }) {
        return Err(LedgerError::InvalidId {
            kind,
            value: value.to_string(),
            reason: "must contain only ASCII alphanumeric, '-', '_', '.', ':', or '/'",
        });
    }

    Ok(())
}

fn require_text(field: &'static str, value: &str) -> Result<(), LedgerError> {
    if value.trim().is_empty() {
        Err(LedgerError::EmptyText { field })
    } else {
        Ok(())
    }
}

fn push_unique<T>(values: &mut Vec<T>, value: T)
where
    T: PartialEq,
{
    if !values.contains(&value) {
        values.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initializes_authoritative_root_scope_with_validated_ids() {
        let ledger = fixture_ledger();

        assert_eq!(ledger.schema_version(), LEDGER_SCHEMA_VERSION);
        assert_eq!(
            ledger.authority().compaction_authority,
            ExternalAuthority::NonAuthoritativeContext
        );
        assert_eq!(
            ledger.root().objective().text(),
            "Ship durable Cowork scope."
        );
        assert_eq!(ledger.root().constraints().len(), 2);

        let json = serde_json::to_string(&ledger).expect("serialize ledger");
        let decoded: CoworkLedger = serde_json::from_str(&json).expect("deserialize ledger");
        assert_eq!(decoded.root().observation(), ledger.root().observation());

        let invalid = TaskId::parse("bad id with spaces").expect_err("reject invalid task id");
        assert!(matches!(invalid, LedgerError::InvalidId { .. }));
    }

    #[test]
    fn all_core_entity_ids_are_validated_stable_strings() {
        macro_rules! assert_id_roundtrip {
            ($ty:ty, $value:literal) => {{
                let id = <$ty>::parse($value).expect("parse stable id");
                let json = serde_json::to_string(&id).expect("serialize id");
                assert_eq!(json, format!("\"{}\"", $value));

                let decoded: $ty = serde_json::from_str(&json).expect("deserialize id");
                assert_eq!(decoded.as_str(), $value);
            }};
        }

        assert_id_roundtrip!(RootObjectiveId, "objective/root");
        assert_id_roundtrip!(RootConstraintId, "constraint/root");
        assert_id_roundtrip!(TaskId, "task/1");
        assert_id_roundtrip!(BlockerId, "blocker/1");
        assert_id_roundtrip!(EvidenceId, "evidence/1");
        assert_id_roundtrip!(CheckpointId, "checkpoint/1");
        assert_id_roundtrip!(ArtifactId, "artifact/1");
        assert_id_roundtrip!(SessionId, "session/1");
        assert_id_roundtrip!(StopReasonId, "stop/1");
        assert_id_roundtrip!(WaiverId, "waiver/1");
        assert_id_roundtrip!(ProvenanceLinkId, "link/1");

        assert!(matches!(
            RootObjectiveId::parse(" objective/root"),
            Err(LedgerError::InvalidId { .. })
        ));
    }

    #[test]
    fn direct_root_mutation_updates_are_rejected() {
        let mut ledger = fixture_ledger();

        let error = ledger
            .apply_update(LedgerUpdate::AttemptRootObjectiveMutation {
                attempted_text: "Replace the root objective from a summary.".to_string(),
            })
            .expect_err("root objective text cannot change");

        assert!(matches!(
            error,
            LedgerError::ImmutableRootMutationRejected {
                target: RootMutationTarget::ObjectiveText,
                ..
            }
        ));
        assert_eq!(
            ledger.root().objective().text(),
            "Ship durable Cowork scope."
        );
    }

    #[test]
    fn compaction_summary_cannot_redefine_root_objective_or_constraints() {
        let mut ledger = fixture_ledger_with_task();
        let original_root = ledger.root().observation();

        let mut changed_objective = original_root.clone();
        changed_objective.objective_text =
            "A compressed summary tries to narrow scope.".to_string();

        let error = ledger
            .apply_update(LedgerUpdate::ApplyCompactionSummary(
                allowed_summary_update(changed_objective),
            ))
            .expect_err("compaction root objective rewrite must fail");

        assert!(matches!(
            error,
            LedgerError::ImmutableRootMutationRejected {
                target: RootMutationTarget::ObjectiveText,
                ..
            }
        ));
        assert!(ledger.evidence().is_empty());

        let mut changed_constraints = original_root.clone();
        changed_constraints.constraints.pop();

        let error = ledger
            .apply_update(LedgerUpdate::ApplyCompactionSummary(
                allowed_summary_update(changed_constraints),
            ))
            .expect_err("compaction root constraint rewrite must fail");

        assert!(matches!(
            error,
            LedgerError::ImmutableRootMutationRejected {
                target: RootMutationTarget::RootConstraints,
                ..
            }
        ));
        assert!(ledger.checkpoints().is_empty());
    }

    #[test]
    fn compaction_summary_can_record_status_evidence_checkpoint_and_links() {
        let mut ledger = fixture_ledger_with_task();
        let mut observed_root = ledger.root().observation();
        observed_root.constraints.reverse();

        let report = ledger
            .apply_update(LedgerUpdate::ApplyCompactionSummary(
                allowed_summary_update(observed_root),
            ))
            .expect("matching root observation allows non-root updates");

        assert!(report.accepted);
        assert_eq!(
            ledger.tasks().get(&task_id()).expect("task exists").status,
            TaskStatus::Verified
        );
        assert!(ledger.evidence().contains_key(&evidence_id()));
        assert!(ledger.checkpoints().contains_key(&checkpoint_id()));
        assert!(ledger.provenance_links().contains_key(&link_id()));
    }

    #[test]
    fn relation_links_require_existing_entities() {
        let mut ledger = fixture_ledger();
        let error = ledger
            .apply_update(LedgerUpdate::RecordProvenanceLink(ProvenanceLink {
                id: link_id(),
                kind: ProvenanceLinkKind::Supports,
                from: EntityRef::Evidence(evidence_id()),
                to: EntityRef::Task(task_id()),
                created_at: timestamp(10),
                created_by: worker(),
                metadata: Metadata::new(),
            }))
            .expect_err("link to missing entities fails");

        assert!(matches!(error, LedgerError::UnknownReference { .. }));
    }

    fn fixture_ledger_with_task() -> CoworkLedger {
        let mut ledger = fixture_ledger();
        ledger
            .apply_update(LedgerUpdate::RegisterTask(Task {
                id: task_id(),
                objective_id: objective_id(),
                title: "Implement ledger crate".to_string(),
                description: Some("Model durable Cowork scope and proof records.".to_string()),
                status: TaskStatus::InProgress,
                status_updated_at: timestamp(1),
                dependencies: Vec::new(),
                blocker_ids: Vec::new(),
                evidence_ids: Vec::new(),
                artifact_ids: Vec::new(),
                checkpoint_ids: Vec::new(),
                metadata: Metadata::new(),
            }))
            .expect("register task");
        ledger
    }

    fn allowed_summary_update(observed_root: RootScopeObservation) -> CompactionSummaryUpdate {
        CompactionSummaryUpdate {
            observed_root,
            objective_progress: Some(ObjectiveProgressUpdate {
                objective_id: objective_id(),
                status: ObjectiveStatus::Verified,
                updated_at: timestamp(5),
                stop_reason_id: None,
            }),
            tasks: Vec::new(),
            task_status_updates: vec![TaskStatusUpdate {
                task_id: task_id(),
                status: TaskStatus::Verified,
                updated_at: timestamp(6),
                evidence_id: Some(evidence_id()),
            }],
            blockers: Vec::new(),
            evidence: vec![Evidence {
                id: evidence_id(),
                kind: EvidenceKind::CompactionSummary,
                summary: "Summary preserved root scope and recorded verification.".to_string(),
                collected_at: timestamp(4),
                collected_by: worker(),
                result: EvidenceResult::Passed,
                subjects: vec![EntityRef::Task(task_id())],
                command: None,
                artifact_ids: Vec::new(),
                metadata: Metadata::new(),
            }],
            checkpoints: vec![Checkpoint {
                id: checkpoint_id(),
                objective_id: objective_id(),
                sequence: 1,
                summary: "Ledger model boundary verified after compaction.".to_string(),
                created_at: timestamp(7),
                created_by: worker(),
                session_id: Some(session_id()),
                task_ids: vec![task_id()],
                evidence_ids: vec![evidence_id()],
                artifact_ids: Vec::new(),
                stop_reason_id: None,
                metadata: Metadata::new(),
            }],
            artifacts: Vec::new(),
            stop_reasons: Vec::new(),
            waivers: Vec::new(),
            provenance_links: vec![ProvenanceLink {
                id: link_id(),
                kind: ProvenanceLinkKind::Supports,
                from: EntityRef::Evidence(evidence_id()),
                to: EntityRef::Task(task_id()),
                created_at: timestamp(8),
                created_by: worker(),
                metadata: Metadata::new(),
            }],
        }
    }

    fn fixture_ledger() -> CoworkLedger {
        CoworkLedger::initialize(LedgerInitialization {
            root_objective_id: objective_id(),
            root_objective_text: "Ship durable Cowork scope.".to_string(),
            acceptance_criteria: vec![
                "Root scope survives context compaction.".to_string(),
                "Evidence and checkpoints remain linkable.".to_string(),
            ],
            root_constraints: vec![
                RootConstraintInitialization {
                    id: RootConstraintId::parse("constraint/root-immutability").unwrap(),
                    text: "Root objective text cannot change after initialization.".to_string(),
                    severity: ConstraintSeverity::SafetyCritical,
                    source: ConstraintSource::Bead,
                    metadata: Metadata::new(),
                },
                RootConstraintInitialization {
                    id: RootConstraintId::parse("constraint/runtime-authority").unwrap(),
                    text: "Cowork ledger is authoritative over summaries.".to_string(),
                    severity: ConstraintSeverity::Required,
                    source: ConstraintSource::RuntimePolicy,
                    metadata: Metadata::new(),
                },
            ],
            initialized_at: timestamp(0),
            initialized_by: worker(),
            metadata: Metadata::new(),
        })
        .expect("initialize ledger")
    }

    fn objective_id() -> RootObjectiveId {
        RootObjectiveId::parse("objective/root").unwrap()
    }

    fn task_id() -> TaskId {
        TaskId::parse("task/ledger").unwrap()
    }

    fn evidence_id() -> EvidenceId {
        EvidenceId::parse("evidence/compaction").unwrap()
    }

    fn checkpoint_id() -> CheckpointId {
        CheckpointId::parse("checkpoint/1").unwrap()
    }

    fn session_id() -> SessionId {
        SessionId::parse("session/worker-a").unwrap()
    }

    fn link_id() -> ProvenanceLinkId {
        ProvenanceLinkId::parse("link/evidence-task").unwrap()
    }

    fn worker() -> ActorRef {
        ActorRef {
            kind: ActorKind::Worker,
            id: "worker-a".to_string(),
            display_name: Some("Worker Variant A".to_string()),
        }
    }

    fn timestamp(second: u32) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(&format!("2026-05-21T00:00:{second:02}Z"))
            .unwrap()
            .with_timezone(&Utc)
    }
}
