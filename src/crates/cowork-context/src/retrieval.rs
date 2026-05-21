use cowork_ledger::{
    Artifact, Blocker, Checkpoint, CoworkLedger, EntityRef, Evidence, SessionRecord, Task,
};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRetrievalRequest {
    pub target: HistoryRetrievalTarget,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_sequence_range: Option<HistorySequenceRange>,
    pub include_linked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryRetrievalTarget {
    All,
    Task,
    Evidence,
    Checkpoint,
    Artifact,
    Session,
    Blocker,
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
    #[serde(default)]
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    #[serde(default)]
    pub checkpoints: Vec<Checkpoint>,
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    #[serde(default)]
    pub sessions: Vec<SessionRecord>,
    #[serde(default)]
    pub blockers: Vec<Blocker>,
    pub result_count: usize,
}

impl HistoryRetrievalResult {
    pub fn from_ledger(
        ledger: &CoworkLedger,
        request: HistoryRetrievalRequest,
    ) -> Result<Self, HistoryRetrievalError> {
        request.validate()?;

        let mut result = Self {
            request: request.clone(),
            tasks: Vec::new(),
            evidence: Vec::new(),
            checkpoints: Vec::new(),
            artifacts: Vec::new(),
            sessions: Vec::new(),
            blockers: Vec::new(),
            result_count: 0,
        };

        collect_direct_matches(ledger, &request, &mut result);
        if request.include_linked {
            collect_linked_records(ledger, &mut result);
        }
        result.result_count = result_record_count(&result);

        if result.result_count == 0 {
            return Err(HistoryRetrievalError::NotFound {
                target: request.target,
                selector: selector_label(&request),
            });
        }

        Ok(result)
    }
}

impl HistoryRetrievalRequest {
    fn validate(&self) -> Result<(), HistoryRetrievalError> {
        if matches!(self.query.as_deref(), Some(query) if query.trim().is_empty()) {
            return Err(HistoryRetrievalError::EmptyQuery);
        }
        if let Some(range) = self.checkpoint_sequence_range {
            if range.start > range.end {
                return Err(HistoryRetrievalError::InvalidRange {
                    start: range.start,
                    end: range.end,
                });
            }
        }
        Ok(())
    }

    fn normalized_query(&self) -> Option<String> {
        self.query
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryRetrievalError {
    EmptyQuery,
    InvalidRange {
        start: u64,
        end: u64,
    },
    NotFound {
        target: HistoryRetrievalTarget,
        selector: String,
    },
}

impl HistoryRetrievalError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::EmptyQuery => "empty_history_query",
            Self::InvalidRange { .. } => "invalid_history_range",
            Self::NotFound { .. } => "history_record_not_found",
        }
    }
}

impl fmt::Display for HistoryRetrievalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyQuery => {
                formatter.write_str("Cowork history retrieval query cannot be empty")
            }
            Self::InvalidRange { start, end } => write!(
                formatter,
                "invalid Cowork history checkpoint sequence range {start}..{end}"
            ),
            Self::NotFound { target, selector } => {
                write!(
                    formatter,
                    "Cowork history {target:?} record not found for {selector}"
                )
            }
        }
    }
}

impl std::error::Error for HistoryRetrievalError {}

fn collect_direct_matches(
    ledger: &CoworkLedger,
    request: &HistoryRetrievalRequest,
    result: &mut HistoryRetrievalResult,
) {
    if matches!(
        request.target,
        HistoryRetrievalTarget::All | HistoryRetrievalTarget::Task
    ) {
        result.tasks.extend(
            ledger
                .tasks()
                .values()
                .filter(|task| task_matches(task, request))
                .cloned(),
        );
    }

    if matches!(
        request.target,
        HistoryRetrievalTarget::All | HistoryRetrievalTarget::Evidence
    ) {
        result.evidence.extend(
            ledger
                .evidence()
                .values()
                .filter(|evidence| evidence_matches(evidence, request))
                .cloned(),
        );
    }

    if matches!(
        request.target,
        HistoryRetrievalTarget::All | HistoryRetrievalTarget::Checkpoint
    ) {
        result.checkpoints.extend(
            ledger
                .checkpoints()
                .values()
                .filter(|checkpoint| checkpoint_matches(checkpoint, request))
                .cloned(),
        );
    }

    if matches!(
        request.target,
        HistoryRetrievalTarget::All | HistoryRetrievalTarget::Artifact
    ) {
        result.artifacts.extend(
            ledger
                .artifacts()
                .values()
                .filter(|artifact| artifact_matches(artifact, request))
                .cloned(),
        );
    }

    if matches!(
        request.target,
        HistoryRetrievalTarget::All | HistoryRetrievalTarget::Session
    ) {
        result.sessions.extend(
            ledger
                .sessions()
                .values()
                .filter(|session| session_matches(session, request))
                .cloned(),
        );
    }

    if matches!(
        request.target,
        HistoryRetrievalTarget::All | HistoryRetrievalTarget::Blocker
    ) {
        result.blockers.extend(
            ledger
                .blockers()
                .values()
                .filter(|blocker| blocker_matches(blocker, request))
                .cloned(),
        );
    }
}

fn collect_linked_records(ledger: &CoworkLedger, result: &mut HistoryRetrievalResult) {
    loop {
        let before = result_record_count(result);
        collect_linked_once(ledger, result);
        if result_record_count(result) == before {
            break;
        }
    }
}

fn collect_linked_once(ledger: &CoworkLedger, result: &mut HistoryRetrievalResult) {
    let task_ids = result
        .tasks
        .iter()
        .map(|task| task.id.clone())
        .chain(
            result
                .evidence
                .iter()
                .flat_map(|evidence| evidence.subjects.iter())
                .filter_map(|subject| match subject {
                    EntityRef::Task(id) => Some(id.clone()),
                    _ => None,
                }),
        )
        .chain(
            result
                .checkpoints
                .iter()
                .flat_map(|checkpoint| checkpoint.task_ids.iter().cloned()),
        )
        .chain(
            result
                .blockers
                .iter()
                .flat_map(|blocker| blocker.task_ids.iter().cloned()),
        )
        .collect::<Vec<_>>();
    for task_id in task_ids {
        if let Some(task) = ledger.tasks().get(&task_id) {
            push_unique(&mut result.tasks, task.clone());
        }
    }

    let evidence_ids = result
        .evidence
        .iter()
        .map(|evidence| evidence.id.clone())
        .chain(
            result
                .tasks
                .iter()
                .flat_map(|task| task.evidence_ids.iter().cloned()),
        )
        .chain(
            result
                .checkpoints
                .iter()
                .flat_map(|checkpoint| checkpoint.evidence_ids.iter().cloned()),
        )
        .chain(
            result
                .artifacts
                .iter()
                .flat_map(|artifact| artifact.evidence_ids.iter().cloned()),
        )
        .chain(
            result
                .blockers
                .iter()
                .flat_map(|blocker| blocker.evidence_ids.iter().cloned()),
        )
        .collect::<Vec<_>>();
    for evidence_id in evidence_ids {
        if let Some(evidence) = ledger.evidence().get(&evidence_id) {
            push_unique(&mut result.evidence, evidence.clone());
        }
    }

    let checkpoint_ids = result
        .checkpoints
        .iter()
        .map(|checkpoint| checkpoint.id.clone())
        .chain(
            result
                .tasks
                .iter()
                .flat_map(|task| task.checkpoint_ids.iter().cloned()),
        )
        .chain(
            result
                .sessions
                .iter()
                .flat_map(|session| session.checkpoint_ids.iter().cloned()),
        )
        .collect::<Vec<_>>();
    for checkpoint_id in checkpoint_ids {
        if let Some(checkpoint) = ledger.checkpoints().get(&checkpoint_id) {
            push_unique(&mut result.checkpoints, checkpoint.clone());
        }
    }

    let artifact_ids = result
        .artifacts
        .iter()
        .map(|artifact| artifact.id.clone())
        .chain(
            result
                .tasks
                .iter()
                .flat_map(|task| task.artifact_ids.iter().cloned()),
        )
        .chain(
            result
                .evidence
                .iter()
                .flat_map(|evidence| evidence.artifact_ids.iter().cloned()),
        )
        .chain(
            result
                .checkpoints
                .iter()
                .flat_map(|checkpoint| checkpoint.artifact_ids.iter().cloned()),
        )
        .collect::<Vec<_>>();
    for artifact_id in artifact_ids {
        if let Some(artifact) = ledger.artifacts().get(&artifact_id) {
            push_unique(&mut result.artifacts, artifact.clone());
        }
    }

    let session_ids = result
        .sessions
        .iter()
        .map(|session| session.id.clone())
        .chain(
            result
                .checkpoints
                .iter()
                .filter_map(|checkpoint| checkpoint.session_id.clone()),
        )
        .chain(
            result
                .artifacts
                .iter()
                .filter_map(|artifact| match &artifact.produced_by {
                    EntityRef::Session(id) => Some(id.clone()),
                    _ => None,
                }),
        )
        .collect::<Vec<_>>();
    for session_id in session_ids {
        if let Some(session) = ledger.sessions().get(&session_id) {
            push_unique(&mut result.sessions, session.clone());
        }
    }

    let blocker_ids = result
        .blockers
        .iter()
        .map(|blocker| blocker.id.clone())
        .chain(
            result
                .tasks
                .iter()
                .flat_map(|task| task.blocker_ids.iter().cloned()),
        )
        .collect::<Vec<_>>();
    for blocker_id in blocker_ids {
        if let Some(blocker) = ledger.blockers().get(&blocker_id) {
            push_unique(&mut result.blockers, blocker.clone());
        }
    }
}

fn push_unique<T: PartialEq>(target: &mut Vec<T>, item: T) {
    if !target.contains(&item) {
        target.push(item);
    }
}

fn task_matches(task: &Task, request: &HistoryRetrievalRequest) -> bool {
    id_matches(task.id.as_str(), request)
        && query_matches(
            [
                task.id.as_str(),
                task.title.as_str(),
                task.description.as_deref().unwrap_or_default(),
            ],
            request,
        )
        && request.checkpoint_sequence_range.is_none()
}

fn evidence_matches(evidence: &Evidence, request: &HistoryRetrievalRequest) -> bool {
    id_matches(evidence.id.as_str(), request)
        && query_matches([evidence.id.as_str(), evidence.summary.as_str()], request)
        && request.checkpoint_sequence_range.is_none()
}

fn checkpoint_matches(checkpoint: &Checkpoint, request: &HistoryRetrievalRequest) -> bool {
    id_matches(checkpoint.id.as_str(), request)
        && query_matches(
            [checkpoint.id.as_str(), checkpoint.summary.as_str()],
            request,
        )
        && sequence_matches(checkpoint.sequence, request)
}

fn artifact_matches(artifact: &Artifact, request: &HistoryRetrievalRequest) -> bool {
    id_matches(artifact.id.as_str(), request)
        && query_matches(
            [
                artifact.id.as_str(),
                artifact.title.as_str(),
                artifact.uri.as_deref().unwrap_or_default(),
            ],
            request,
        )
        && request.checkpoint_sequence_range.is_none()
}

fn session_matches(session: &SessionRecord, request: &HistoryRetrievalRequest) -> bool {
    id_matches(session.id.as_str(), request)
        && query_matches(
            [
                session.id.as_str(),
                &format!("{:?}", session.kind),
                &format!("{:?}", session.status),
            ],
            request,
        )
        && request.checkpoint_sequence_range.is_none()
}

fn blocker_matches(blocker: &Blocker, request: &HistoryRetrievalRequest) -> bool {
    id_matches(blocker.id.as_str(), request)
        && query_matches(
            [
                blocker.id.as_str(),
                blocker.summary.as_str(),
                &blocker.required_external_input.join(" "),
            ],
            request,
        )
        && request.checkpoint_sequence_range.is_none()
}

fn id_matches(record_id: &str, request: &HistoryRetrievalRequest) -> bool {
    request.id.as_deref().map_or(true, |id| id == record_id)
}

fn query_matches<'a>(
    values: impl IntoIterator<Item = &'a str>,
    request: &HistoryRetrievalRequest,
) -> bool {
    request.normalized_query().as_ref().map_or(true, |query| {
        values.into_iter().any(|value| contains_query(value, query))
    })
}

fn sequence_matches(sequence: u64, request: &HistoryRetrievalRequest) -> bool {
    request.checkpoint_sequence_range.map_or(true, |range| {
        sequence >= range.start && sequence <= range.end
    })
}

fn contains_query(value: &str, query: &str) -> bool {
    value.to_ascii_lowercase().contains(query)
}

fn result_record_count(result: &HistoryRetrievalResult) -> usize {
    result.tasks.len()
        + result.evidence.len()
        + result.checkpoints.len()
        + result.artifacts.len()
        + result.sessions.len()
        + result.blockers.len()
}

fn selector_label(request: &HistoryRetrievalRequest) -> String {
    if let Some(id) = &request.id {
        return format!("id '{id}'");
    }
    if let Some(query) = &request.query {
        return format!("query '{}'", query.trim().to_ascii_lowercase());
    }
    if let Some(range) = request.checkpoint_sequence_range {
        return format!("checkpoint sequence {}..{}", range.start, range.end);
    }
    "all records".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use cowork_ledger::{
        ActorKind, ActorRef, ArtifactId, ArtifactKind, BlockerId, BlockerStatus, CheckpointId,
        ConstraintSeverity, ConstraintSource, EvidenceId, EvidenceKind, EvidenceResult,
        EvidenceSourceRef, LedgerInitialization, LedgerUpdate, Metadata, RootConstraintId,
        RootConstraintInitialization, RootObjectiveId, SessionId, SessionKind, SessionStatus,
        TaskId, TaskStatus,
    };

    #[test]
    fn retrieves_exact_task_with_linked_evidence_artifact_checkpoint_session_and_blocker() {
        let ledger = fixture_ledger();
        let result = HistoryRetrievalResult::from_ledger(
            &ledger,
            HistoryRetrievalRequest {
                target: HistoryRetrievalTarget::Task,
                id: Some("task/context".to_string()),
                query: None,
                checkpoint_sequence_range: None,
                include_linked: true,
            },
        )
        .expect("retrieve task");

        assert_eq!(result.tasks[0].id.as_str(), "task/context");
        assert_eq!(result.evidence[0].id.as_str(), "evidence/context");
        assert_eq!(result.artifacts[0].id.as_str(), "artifact/context");
        assert_eq!(result.checkpoints[0].id.as_str(), "checkpoint/context");
        assert_eq!(result.sessions[0].id.as_str(), "session/context");
        assert_eq!(result.blockers[0].id.as_str(), "blocker/context");
    }

    #[test]
    fn retrieves_checkpoint_range_and_query_from_ledger_history() {
        let ledger = fixture_ledger();
        let range = HistoryRetrievalResult::from_ledger(
            &ledger,
            HistoryRetrievalRequest {
                target: HistoryRetrievalTarget::Checkpoint,
                id: None,
                query: None,
                checkpoint_sequence_range: Some(HistorySequenceRange { start: 1, end: 1 }),
                include_linked: true,
            },
        )
        .expect("retrieve checkpoint range");
        assert_eq!(range.checkpoints[0].sequence, 1);
        assert_eq!(range.tasks[0].id.as_str(), "task/context");

        let query = HistoryRetrievalResult::from_ledger(
            &ledger,
            HistoryRetrievalRequest {
                target: HistoryRetrievalTarget::All,
                id: None,
                query: Some("fallback".to_string()),
                checkpoint_sequence_range: None,
                include_linked: true,
            },
        )
        .expect("retrieve query");
        assert!(query
            .tasks
            .iter()
            .any(|task| task.id.as_str() == "task/context"));
        assert!(query
            .evidence
            .iter()
            .any(|evidence| evidence.id.as_str() == "evidence/context"));
    }

    #[test]
    fn missing_empty_and_invalid_requests_have_stable_error_codes() {
        let ledger = fixture_ledger();
        let missing = HistoryRetrievalResult::from_ledger(
            &ledger,
            HistoryRetrievalRequest {
                target: HistoryRetrievalTarget::Evidence,
                id: Some("evidence/missing".to_string()),
                query: None,
                checkpoint_sequence_range: None,
                include_linked: true,
            },
        )
        .unwrap_err();
        assert_eq!(missing.code(), "history_record_not_found");

        let empty = HistoryRetrievalResult::from_ledger(
            &ledger,
            HistoryRetrievalRequest {
                target: HistoryRetrievalTarget::All,
                id: None,
                query: Some("  ".to_string()),
                checkpoint_sequence_range: None,
                include_linked: true,
            },
        )
        .unwrap_err();
        assert_eq!(empty.code(), "empty_history_query");

        let range = HistoryRetrievalResult::from_ledger(
            &ledger,
            HistoryRetrievalRequest {
                target: HistoryRetrievalTarget::Checkpoint,
                id: None,
                query: None,
                checkpoint_sequence_range: Some(HistorySequenceRange { start: 2, end: 1 }),
                include_linked: true,
            },
        )
        .unwrap_err();
        assert_eq!(range.code(), "invalid_history_range");
    }

    fn fixture_ledger() -> CoworkLedger {
        let mut ledger = CoworkLedger::initialize(LedgerInitialization {
            root_objective_id: objective_id(),
            root_objective_text: "Ship durable Cowork scope.".to_string(),
            acceptance_criteria: vec!["Exact retrieval survives compaction.".to_string()],
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
                description: Some("Carry forward exact fallback evidence.".to_string()),
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
            id: "agent/context".to_string(),
            display_name: Some("Context worker".to_string()),
        }
    }

    fn objective_id() -> cowork_ledger::RootObjectiveId {
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
