//! Daemon-owned context fallback payloads.
//!
//! This crate carries BitFun's compression fallback contracts into Cowork's
//! ledger-authoritative runtime. Compressed summaries may omit history, but the
//! root objective observation and at least one task/evidence contract record
//! survive trimming when they exist in the ledger.

use cowork_ledger::{
    ArtifactId, ArtifactKind, BlockerId, BlockerStatus, CheckpointId, CoworkLedger, EntityRef,
    EvidenceId, EvidenceKind, EvidenceResult, EvidenceSourceRef, Metadata, ObjectiveProgress,
    RootScopeObservation, SessionId, SessionKind, SessionStatus, StopReasonId, TaskId, TaskStatus,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextFallbackOptions {
    pub max_tokens: usize,
    #[serde(default)]
    pub todo_snapshot: TodoSnapshot,
    #[serde(default)]
    pub history_hints: Vec<HistoryIndexHint>,
}

impl ContextFallbackOptions {
    pub fn new(max_tokens: usize) -> Self {
        Self {
            max_tokens,
            todo_snapshot: TodoSnapshot::default(),
            history_hints: Vec::new(),
        }
    }

    pub fn with_todo_snapshot(mut self, todo_snapshot: TodoSnapshot) -> Self {
        self.todo_snapshot = todo_snapshot;
        self
    }

    pub fn with_history_hints(mut self, history_hints: Vec<HistoryIndexHint>) -> Self {
        self.history_hints = history_hints;
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoSnapshot {
    #[serde(default)]
    pub items: Vec<TodoItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Metadata::is_empty")]
    pub metadata: Metadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoItem {
    pub content: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryIndexHint {
    pub label: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_id: Option<CheckpointId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_range: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoworkFallbackPayload {
    pub root_observation: RootScopeObservation,
    pub objective_progress: ObjectiveProgress,
    #[serde(default)]
    pub tasks: Vec<TaskSummary>,
    #[serde(default)]
    pub evidence: Vec<EvidenceSummary>,
    #[serde(default)]
    pub checkpoints: Vec<CheckpointSummary>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactSummary>,
    #[serde(default)]
    pub sessions: Vec<SessionSummary>,
    #[serde(default)]
    pub blockers: Vec<BlockerSummary>,
    pub token_accounting: TokenAccounting,
    pub todo_snapshot: TodoSnapshot,
    #[serde(default)]
    pub history_hints: Vec<HistoryIndexHint>,
}

impl CoworkFallbackPayload {
    pub fn from_ledger(ledger: &CoworkLedger, options: ContextFallbackOptions) -> Self {
        let original_counts = PayloadCounts {
            tasks: ledger.tasks().len(),
            evidence: ledger.evidence().len(),
            checkpoints: ledger.checkpoints().len(),
            artifacts: ledger.artifacts().len(),
            sessions: ledger.sessions().len(),
            blockers: ledger.blockers().len(),
        };

        let mut payload = Self {
            root_observation: ledger.root().observation(),
            objective_progress: ledger.objective_progress().clone(),
            tasks: ledger
                .tasks()
                .values()
                .map(TaskSummary::from_ledger_task)
                .collect(),
            evidence: ledger
                .evidence()
                .values()
                .map(EvidenceSummary::from_ledger_evidence)
                .collect(),
            checkpoints: ledger
                .checkpoints()
                .values()
                .map(CheckpointSummary::from_ledger_checkpoint)
                .collect(),
            artifacts: ledger
                .artifacts()
                .values()
                .map(ArtifactSummary::from_ledger_artifact)
                .collect(),
            sessions: ledger
                .sessions()
                .values()
                .map(SessionSummary::from_ledger_session)
                .collect(),
            blockers: ledger
                .blockers()
                .values()
                .map(BlockerSummary::from_ledger_blocker)
                .collect(),
            token_accounting: TokenAccounting {
                max_tokens: options.max_tokens,
                estimated_tokens: 0,
                original_counts,
                trimmed_counts: PayloadCounts::default(),
                was_trimmed: false,
            },
            todo_snapshot: options.todo_snapshot,
            history_hints: options.history_hints,
        };

        payload.apply_budget();
        payload
    }

    fn apply_budget(&mut self) {
        self.token_accounting.estimated_tokens = estimate_tokens(self);
        while self.token_accounting.estimated_tokens > self.token_accounting.max_tokens
            && self.trim_one_detail()
        {
            self.token_accounting.was_trimmed = true;
            self.token_accounting.estimated_tokens = estimate_tokens(self);
        }
    }

    fn trim_one_detail(&mut self) -> bool {
        if self.evidence.len() > 1 {
            self.evidence.pop();
            self.token_accounting.trimmed_counts.evidence += 1;
            return true;
        }
        if self.checkpoints.pop().is_some() {
            self.token_accounting.trimmed_counts.checkpoints += 1;
            return true;
        }
        if self.artifacts.pop().is_some() {
            self.token_accounting.trimmed_counts.artifacts += 1;
            return true;
        }
        if self.sessions.pop().is_some() {
            self.token_accounting.trimmed_counts.sessions += 1;
            return true;
        }
        if self.blockers.pop().is_some() {
            self.token_accounting.trimmed_counts.blockers += 1;
            return true;
        }
        if self.tasks.len() > 1 {
            self.tasks.pop();
            self.token_accounting.trimmed_counts.tasks += 1;
            return true;
        }
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSummary {
    pub id: TaskId,
    pub objective_id: cowork_ledger::RootObjectiveId,
    pub title: String,
    pub status: TaskStatus,
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
}

impl TaskSummary {
    fn from_ledger_task(task: &cowork_ledger::Task) -> Self {
        Self {
            id: task.id.clone(),
            objective_id: task.objective_id.clone(),
            title: task.title.clone(),
            status: task.status,
            dependencies: task.dependencies.clone(),
            blocker_ids: task.blocker_ids.clone(),
            evidence_ids: task.evidence_ids.clone(),
            artifact_ids: task.artifact_ids.clone(),
            checkpoint_ids: task.checkpoint_ids.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceSummary {
    pub id: EvidenceId,
    pub kind: EvidenceKind,
    pub result: EvidenceResult,
    pub summary: String,
    #[serde(default)]
    pub subjects: Vec<EntityRef>,
    #[serde(default)]
    pub source_refs: Vec<EvidenceSourceRef>,
    #[serde(default)]
    pub artifact_ids: Vec<ArtifactId>,
}

impl EvidenceSummary {
    fn from_ledger_evidence(evidence: &cowork_ledger::Evidence) -> Self {
        Self {
            id: evidence.id.clone(),
            kind: evidence.kind,
            result: evidence.result,
            summary: evidence.summary.clone(),
            subjects: evidence.subjects.clone(),
            source_refs: evidence.source_refs.clone(),
            artifact_ids: evidence.artifact_ids.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointSummary {
    pub id: CheckpointId,
    pub objective_id: cowork_ledger::RootObjectiveId,
    pub sequence: u64,
    pub summary: String,
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
}

impl CheckpointSummary {
    fn from_ledger_checkpoint(checkpoint: &cowork_ledger::Checkpoint) -> Self {
        Self {
            id: checkpoint.id.clone(),
            objective_id: checkpoint.objective_id.clone(),
            sequence: checkpoint.sequence,
            summary: checkpoint.summary.clone(),
            session_id: checkpoint.session_id.clone(),
            task_ids: checkpoint.task_ids.clone(),
            evidence_ids: checkpoint.evidence_ids.clone(),
            artifact_ids: checkpoint.artifact_ids.clone(),
            stop_reason_id: checkpoint.stop_reason_id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactSummary {
    pub id: ArtifactId,
    pub kind: ArtifactKind,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    pub produced_by: EntityRef,
    #[serde(default)]
    pub evidence_ids: Vec<EvidenceId>,
}

impl ArtifactSummary {
    fn from_ledger_artifact(artifact: &cowork_ledger::Artifact) -> Self {
        Self {
            id: artifact.id.clone(),
            kind: artifact.kind,
            title: artifact.title.clone(),
            uri: artifact.uri.clone(),
            produced_by: artifact.produced_by.clone(),
            evidence_ids: artifact.evidence_ids.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: SessionId,
    pub kind: SessionKind,
    pub status: SessionStatus,
    pub actor: cowork_ledger::ActorRef,
    #[serde(default)]
    pub checkpoint_ids: Vec<CheckpointId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason_id: Option<StopReasonId>,
}

impl SessionSummary {
    fn from_ledger_session(session: &cowork_ledger::SessionRecord) -> Self {
        Self {
            id: session.id.clone(),
            kind: session.kind,
            status: session.status,
            actor: session.actor.clone(),
            checkpoint_ids: session.checkpoint_ids.clone(),
            stop_reason_id: session.stop_reason_id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockerSummary {
    pub id: BlockerId,
    pub status: BlockerStatus,
    pub summary: String,
    #[serde(default)]
    pub task_ids: Vec<TaskId>,
    #[serde(default)]
    pub required_external_input: Vec<String>,
    #[serde(default)]
    pub evidence_ids: Vec<EvidenceId>,
}

impl BlockerSummary {
    fn from_ledger_blocker(blocker: &cowork_ledger::Blocker) -> Self {
        Self {
            id: blocker.id.clone(),
            status: blocker.status,
            summary: blocker.summary.clone(),
            task_ids: blocker.task_ids.clone(),
            required_external_input: blocker.required_external_input.clone(),
            evidence_ids: blocker.evidence_ids.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenAccounting {
    pub max_tokens: usize,
    pub estimated_tokens: usize,
    pub original_counts: PayloadCounts,
    pub trimmed_counts: PayloadCounts,
    pub was_trimmed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PayloadCounts {
    pub tasks: usize,
    pub evidence: usize,
    pub checkpoints: usize,
    pub artifacts: usize,
    pub sessions: usize,
    pub blockers: usize,
}

fn estimate_tokens<T: Serialize>(value: &T) -> usize {
    serde_json::to_string(value)
        .map(|json| (json.len() / 4).max(1))
        .unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use cowork_ledger::{
        ActorKind, ActorRef, Artifact, Blocker, Checkpoint, ConstraintSeverity, ConstraintSource,
        Evidence, EvidenceSourceRef, LedgerInitialization, LedgerUpdate, RootConstraintId,
        RootConstraintInitialization, RootObjectiveId, SessionKind, SessionRecord, Task,
    };

    fn fixture_payload(max_tokens: usize) -> CoworkFallbackPayload {
        CoworkFallbackPayload::from_ledger(
            &fixture_ledger(),
            ContextFallbackOptions::new(max_tokens)
                .with_todo_snapshot(TodoSnapshot {
                    items: vec![TodoItem {
                        content: "Preserve objective drift fixture".to_string(),
                        status: "in_progress".to_string(),
                    }],
                    summary: Some("Todo survives compaction fallback.".to_string()),
                    metadata: Metadata::new(),
                })
                .with_history_hints(vec![HistoryIndexHint {
                    label: "latest checkpoint".to_string(),
                    source: "session-history://worker-a#index".to_string(),
                    session_id: Some(session_id()),
                    checkpoint_id: Some(checkpoint_id()),
                    line_range: Some("12-24".to_string()),
                }]),
        )
    }

    #[test]
    fn fallback_payload_preserves_ledger_contract_state() {
        let payload = fixture_payload(20_000);

        assert_eq!(
            payload.root_observation.objective_text,
            "Ship durable Cowork scope."
        );
        assert_eq!(payload.root_observation.acceptance_criteria.len(), 2);
        assert_eq!(payload.root_observation.constraints.len(), 1);
        assert_eq!(payload.objective_progress.objective_id, objective_id());

        assert_eq!(payload.tasks.len(), 1);
        assert_eq!(payload.tasks[0].id, task_id());
        assert_eq!(payload.tasks[0].evidence_ids, vec![evidence_id()]);
        assert_eq!(payload.tasks[0].checkpoint_ids, vec![checkpoint_id()]);
        assert_eq!(payload.tasks[0].artifact_ids, vec![artifact_id()]);

        assert_eq!(payload.evidence.len(), 1);
        assert_eq!(payload.evidence[0].id, evidence_id());
        assert_eq!(payload.evidence[0].result, EvidenceResult::Passed);
        assert_eq!(
            payload.evidence[0].subjects,
            vec![EntityRef::Task(task_id())]
        );
        assert!(matches!(
            payload.evidence[0].source_refs[0],
            EvidenceSourceRef::Command { .. }
        ));

        assert_eq!(payload.checkpoints[0].session_id, Some(session_id()));
        assert_eq!(
            payload.artifacts[0].produced_by,
            EntityRef::Session(session_id())
        );
        assert_eq!(payload.sessions[0].status, SessionStatus::Completed);
        assert_eq!(payload.blockers[0].evidence_ids, vec![evidence_id()]);
    }

    #[test]
    fn fallback_payload_preserves_todo_snapshot_and_history_hints() {
        let payload = fixture_payload(20_000);

        assert_eq!(payload.todo_snapshot.items.len(), 1);
        assert_eq!(
            payload.todo_snapshot.items[0].content,
            "Preserve objective drift fixture"
        );
        assert_eq!(
            payload.todo_snapshot.summary.as_deref(),
            Some("Todo survives compaction fallback.")
        );
        assert_eq!(payload.history_hints.len(), 1);
        assert_eq!(payload.history_hints[0].session_id, Some(session_id()));
        assert_eq!(
            payload.history_hints[0].checkpoint_id,
            Some(checkpoint_id())
        );
    }

    #[test]
    fn tiny_budget_keeps_root_task_and_evidence_contract() {
        let payload = fixture_payload(1);

        assert_eq!(
            payload.root_observation.objective_text,
            "Ship durable Cowork scope."
        );
        assert_eq!(payload.tasks.len(), 1);
        assert_eq!(payload.evidence.len(), 1);
        assert_eq!(payload.tasks[0].id, task_id());
        assert_eq!(payload.evidence[0].id, evidence_id());
        assert_eq!(payload.todo_snapshot.items.len(), 1);
        assert_eq!(payload.history_hints.len(), 1);
        assert!(payload.token_accounting.was_trimmed);
        assert!(payload.token_accounting.trimmed_counts.artifacts > 0);
    }

    #[test]
    fn fallback_payload_is_json_serializable() {
        let payload = fixture_payload(20_000);
        let json = serde_json::to_value(&payload).expect("serialize payload");

        assert_eq!(
            json["rootObservation"]["objectiveText"],
            "Ship durable Cowork scope."
        );
        assert_eq!(json["tasks"][0]["id"], "task/context");
        assert_eq!(json["evidence"][0]["id"], "evidence/context");
    }

    fn fixture_ledger() -> CoworkLedger {
        let mut ledger = CoworkLedger::initialize(LedgerInitialization {
            root_objective_id: objective_id(),
            root_objective_text: "Ship durable Cowork scope.".to_string(),
            acceptance_criteria: vec![
                "Root scope survives context compaction.".to_string(),
                "Evidence and checkpoints remain linkable.".to_string(),
            ],
            root_constraints: vec![RootConstraintInitialization {
                id: RootConstraintId::parse("constraint/context-root").unwrap(),
                text: "Context summaries cannot redefine scope.".to_string(),
                severity: ConstraintSeverity::SafetyCritical,
                source: ConstraintSource::Bead,
                metadata: Metadata::new(),
            }],
            initialized_at: timestamp(0),
            initialized_by: actor(),
            metadata: Metadata::new(),
        })
        .expect("initialize ledger");

        ledger
            .apply_update(LedgerUpdate::RegisterTask(Task {
                id: task_id(),
                objective_id: objective_id(),
                title: "Preserve context fallback contract".to_string(),
                description: Some("Carry forward fallback payload semantics.".to_string()),
                status: TaskStatus::InProgress,
                status_updated_at: timestamp(1),
                dependencies: Vec::new(),
                blocker_ids: vec![blocker_id()],
                evidence_ids: vec![evidence_id()],
                artifact_ids: vec![artifact_id()],
                checkpoint_ids: vec![checkpoint_id()],
                metadata: Metadata::new(),
            }))
            .expect("register task");

        ledger
            .apply_update(LedgerUpdate::RecordSession(SessionRecord {
                id: session_id(),
                objective_id: objective_id(),
                kind: SessionKind::Compaction,
                status: SessionStatus::Completed,
                started_at: timestamp(2),
                ended_at: Some(timestamp(8)),
                actor: actor(),
                checkpoint_ids: vec![checkpoint_id()],
                stop_reason_id: None,
                metadata: Metadata::new(),
            }))
            .expect("record session");

        ledger
            .apply_update(LedgerUpdate::RecordArtifact(Artifact {
                id: artifact_id(),
                kind: ArtifactKind::Json,
                title: "Fallback payload".to_string(),
                uri: Some("memory://fallback/context.json".to_string()),
                content_hash: None,
                produced_by: EntityRef::Session(session_id()),
                evidence_ids: Vec::new(),
                metadata: Metadata::new(),
            }))
            .expect("record artifact");

        ledger
            .apply_update(LedgerUpdate::RecordEvidence(Evidence {
                id: evidence_id(),
                kind: EvidenceKind::CompactionSummary,
                summary: "Fallback payload preserved root objective and task evidence.".to_string(),
                collected_at: timestamp(3),
                collected_by: actor(),
                result: EvidenceResult::Passed,
                subjects: vec![EntityRef::Task(task_id())],
                command: None,
                source_refs: vec![EvidenceSourceRef::Command {
                    command: "cargo test -p cowork-context".to_string(),
                }],
                artifact_ids: vec![artifact_id()],
                metadata: Metadata::new(),
            }))
            .expect("record evidence");

        ledger
            .apply_update(LedgerUpdate::RecordCheckpoint(Checkpoint {
                id: checkpoint_id(),
                objective_id: objective_id(),
                sequence: 1,
                summary: "Context fallback contract captured.".to_string(),
                created_at: timestamp(4),
                created_by: actor(),
                session_id: Some(session_id()),
                task_ids: vec![task_id()],
                evidence_ids: vec![evidence_id()],
                artifact_ids: vec![artifact_id()],
                stop_reason_id: None,
                metadata: Metadata::new(),
            }))
            .expect("record checkpoint");

        ledger
            .apply_update(LedgerUpdate::RecordBlocker(Blocker {
                id: blocker_id(),
                summary: "Wait for history retrieval follow-up.".to_string(),
                status: BlockerStatus::Open,
                opened_at: timestamp(5),
                resolved_at: None,
                task_ids: vec![task_id()],
                required_external_input: vec!["Phase 3.2 exact retrieval".to_string()],
                evidence_ids: vec![evidence_id()],
                metadata: Metadata::new(),
            }))
            .expect("record blocker");

        ledger
    }

    fn timestamp(second: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 21, 0, 0, second)
            .single()
            .expect("valid timestamp")
    }

    fn actor() -> ActorRef {
        ActorRef {
            kind: ActorKind::CoworkRuntime,
            id: "cowork-runtime".to_string(),
            display_name: Some("Cowork Runtime".to_string()),
        }
    }

    fn objective_id() -> RootObjectiveId {
        RootObjectiveId::parse("objective/context").unwrap()
    }

    fn task_id() -> TaskId {
        TaskId::parse("task/context").unwrap()
    }

    fn evidence_id() -> EvidenceId {
        EvidenceId::parse("evidence/context").unwrap()
    }

    fn checkpoint_id() -> CheckpointId {
        CheckpointId::parse("checkpoint/context").unwrap()
    }

    fn artifact_id() -> ArtifactId {
        ArtifactId::parse("artifact/context").unwrap()
    }

    fn session_id() -> SessionId {
        SessionId::parse("session/context").unwrap()
    }

    fn blocker_id() -> BlockerId {
        BlockerId::parse("blocker/context").unwrap()
    }
}
