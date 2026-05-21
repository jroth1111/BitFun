use crate::retrieval::{
    HistoryRetrievalError, HistoryRetrievalRequest, HistoryRetrievalResult, HistoryRetrievalTarget,
};
use crate::{ContextFallbackOptions, CoworkFallbackPayload};
use chrono::{Duration, TimeZone, Utc};
use cowork_ledger::{
    ActorKind, ActorRef, Artifact, ArtifactId, ArtifactKind, Blocker, BlockerId, BlockerStatus,
    Checkpoint, CheckpointId, ConstraintSeverity, ConstraintSource, CoworkLedger, EntityRef,
    Evidence, EvidenceId, EvidenceKind, EvidenceResult, EvidenceSourceRef, LedgerError,
    LedgerInitialization, LedgerUpdate, Metadata, ObjectiveProgressUpdate, ObjectiveStatus,
    RootConstraintId, RootConstraintInitialization, RootObjectiveId, SessionId, SessionKind,
    SessionRecord, SessionStatus, StopReasonId, StopReasonKind, StopReasonRecord, Task, TaskId,
    TaskStatus, TaskStatusUpdate,
};
use serde::{Deserialize, Serialize};
use std::fmt;

const DEFAULT_STEPS: usize = 50;
const DEFAULT_APPROX_TOKEN_PRESSURE: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LongHorizonHarnessConfig {
    pub steps: usize,
    pub approx_token_pressure: usize,
    #[serde(default)]
    pub compaction_budgets: Vec<usize>,
}

impl Default for LongHorizonHarnessConfig {
    fn default() -> Self {
        Self {
            steps: DEFAULT_STEPS,
            approx_token_pressure: DEFAULT_APPROX_TOKEN_PRESSURE,
            compaction_budgets: vec![120_000, 24_000, 6_000, 1_500],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LongHorizonHarnessReport {
    pub steps: usize,
    pub failed_tool_attempts: usize,
    pub blockers: usize,
    pub compaction_passes: usize,
    pub restart_resume_passes: usize,
    pub serialized_ledger_bytes: usize,
    pub final_task_id: TaskId,
    pub final_evidence_id: EvidenceId,
    pub final_artifact_id: ArtifactId,
    pub final_artifact_uri: String,
    pub final_checkpoint_id: CheckpointId,
    pub final_blocker_id: BlockerId,
    pub final_stop_reason_id: StopReasonId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriftRegressionFixtureReport {
    pub compaction_turns: usize,
    pub root_rewrite_rejections: usize,
    pub constraint_drop_rejections: usize,
    pub evidence_drop_rejections: usize,
    pub false_completion_rejections: usize,
    pub blocker_omission_rejections: usize,
}

pub fn run_long_horizon_harness(
    config: LongHorizonHarnessConfig,
) -> Result<LongHorizonHarnessReport, LongHorizonHarnessError> {
    config.validate()?;
    let ledger = build_long_horizon_ledger(&config)?;
    verify_repeated_compaction(&ledger, &config)?;
    verify_exact_retrieval_guards(&ledger)?;
    verify_resume_after_restart(&ledger, &config)
}

pub fn run_drift_regression_fixtures(
    config: LongHorizonHarnessConfig,
) -> Result<DriftRegressionFixtureReport, LongHorizonHarnessError> {
    config.validate()?;
    let ledger = build_long_horizon_ledger(&config)?;
    let turns = config.steps + 1;
    let mut report = DriftRegressionFixtureReport {
        compaction_turns: turns,
        root_rewrite_rejections: 0,
        constraint_drop_rejections: 0,
        evidence_drop_rejections: 0,
        false_completion_rejections: 0,
        blocker_omission_rejections: 0,
    };

    for turn in 0..turns {
        let budget = config.compaction_budgets[turn % config.compaction_budgets.len()];
        let compacted = CoworkFallbackPayload::from_ledger(
            &ledger,
            ContextFallbackOptions::new(budget).with_history_hints(Vec::new()),
        );
        validate_payload_against_long_horizon_ledger(&ledger, &compacted)?;

        let untrimmed = CoworkFallbackPayload::from_ledger(
            &ledger,
            ContextFallbackOptions::new(usize::MAX / 4).with_history_hints(Vec::new()),
        );

        let mut root_rewrite = untrimmed.clone();
        root_rewrite.root_observation.objective_text =
            "Compaction narrowed the objective after the fact.".to_string();
        expect_fixture_rejection(
            &ledger,
            &root_rewrite,
            "root objective rewrite fixture",
            &mut report.root_rewrite_rejections,
        )?;

        let mut constraint_drop = untrimmed.clone();
        constraint_drop.root_observation.constraints.clear();
        expect_fixture_rejection(
            &ledger,
            &constraint_drop,
            "root constraint drop fixture",
            &mut report.constraint_drop_rejections,
        )?;

        let mut evidence_drop = untrimmed.clone();
        evidence_drop.evidence.clear();
        expect_fixture_rejection(
            &ledger,
            &evidence_drop,
            "evidence drop fixture",
            &mut report.evidence_drop_rejections,
        )?;

        let mut false_completion = untrimmed.clone();
        let failed_task_id = task_id(first_failed_attempt_step(config.steps)?)?;
        let failed_task = false_completion
            .tasks
            .iter_mut()
            .find(|task| task.id == failed_task_id)
            .ok_or_else(|| {
                LongHorizonHarnessError::Invariant(
                    "false-completion fixture missing failed task".to_string(),
                )
            })?;
        failed_task.status = TaskStatus::Verified;
        expect_fixture_rejection(
            &ledger,
            &false_completion,
            "false completion fixture",
            &mut report.false_completion_rejections,
        )?;

        let mut blocker_omission = untrimmed;
        blocker_omission.blockers.clear();
        for task in &mut blocker_omission.tasks {
            task.blocker_ids.clear();
        }
        expect_fixture_rejection(
            &ledger,
            &blocker_omission,
            "blocker omission fixture",
            &mut report.blocker_omission_rejections,
        )?;
    }

    Ok(report)
}

#[derive(Debug)]
pub enum LongHorizonHarnessError {
    InvalidConfig(String),
    Ledger(LedgerError),
    Serialization(serde_json::Error),
    Retrieval(HistoryRetrievalError),
    Invariant(String),
}

impl fmt::Display for LongHorizonHarnessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(message) => write!(formatter, "invalid harness config: {message}"),
            Self::Ledger(error) => write!(formatter, "ledger error: {error}"),
            Self::Serialization(error) => write!(formatter, "serialization error: {error}"),
            Self::Retrieval(error) => write!(formatter, "retrieval error: {error}"),
            Self::Invariant(message) => write!(formatter, "harness invariant failed: {message}"),
        }
    }
}

impl std::error::Error for LongHorizonHarnessError {}

impl From<LedgerError> for LongHorizonHarnessError {
    fn from(error: LedgerError) -> Self {
        Self::Ledger(error)
    }
}

impl From<serde_json::Error> for LongHorizonHarnessError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}

impl From<HistoryRetrievalError> for LongHorizonHarnessError {
    fn from(error: HistoryRetrievalError) -> Self {
        Self::Retrieval(error)
    }
}

impl LongHorizonHarnessConfig {
    fn validate(&self) -> Result<(), LongHorizonHarnessError> {
        if self.steps < DEFAULT_STEPS {
            return Err(LongHorizonHarnessError::InvalidConfig(format!(
                "steps must be at least {DEFAULT_STEPS}"
            )));
        }
        if self.approx_token_pressure < DEFAULT_APPROX_TOKEN_PRESSURE {
            return Err(LongHorizonHarnessError::InvalidConfig(format!(
                "approx_token_pressure must be at least {DEFAULT_APPROX_TOKEN_PRESSURE}"
            )));
        }
        if self.compaction_budgets.is_empty() {
            return Err(LongHorizonHarnessError::InvalidConfig(
                "at least one compaction budget is required".to_string(),
            ));
        }
        Ok(())
    }
}

fn build_long_horizon_ledger(
    config: &LongHorizonHarnessConfig,
) -> Result<CoworkLedger, LongHorizonHarnessError> {
    let objective_id = objective_id();
    let mut ledger = CoworkLedger::initialize(LedgerInitialization {
        root_objective_id: objective_id.clone(),
        root_objective_text: "Prove Cowork long-horizon recovery.".to_string(),
        acceptance_criteria: vec![
            "Objective and task state survive repeated compaction.".to_string(),
            "Restart/resume retrieves exact prior evidence and artifacts.".to_string(),
        ],
        root_constraints: vec![RootConstraintInitialization {
            id: RootConstraintId::parse("constraint/long-horizon-root")?,
            text: "Compaction cannot redefine root scope.".to_string(),
            severity: ConstraintSeverity::SafetyCritical,
            source: ConstraintSource::Bead,
            metadata: Metadata::new(),
        }],
        initialized_at: timestamp(0),
        initialized_by: actor("orchestrator", ActorKind::CoworkRuntime),
        metadata: Metadata::new(),
    })?;

    let main_session_id = SessionId::parse("session/long-horizon-main")?;
    ledger.apply_update(LedgerUpdate::RecordSession(SessionRecord {
        id: main_session_id.clone(),
        objective_id: objective_id.clone(),
        kind: SessionKind::Main,
        status: SessionStatus::Running,
        started_at: timestamp(1),
        ended_at: None,
        actor: actor("orchestrator", ActorKind::CoworkRuntime),
        checkpoint_ids: Vec::new(),
        stop_reason_id: None,
        metadata: Metadata::new(),
    }))?;

    let pressure_chars_per_step = (config.approx_token_pressure * 4 / config.steps).max(64);
    for index in 0..config.steps {
        let step = index + 1;
        let current_task_id = task_id(step)?;
        let previous_task_id = if step == 1 {
            None
        } else {
            Some(task_id(step - 1)?)
        };
        let evidence_id = evidence_id(step)?;
        let artifact_id = artifact_id(step)?;
        let checkpoint_id = checkpoint_id(step)?;
        let blocker_id = blocker_id(step)?;

        ledger.apply_update(LedgerUpdate::RegisterTask(Task {
            id: current_task_id.clone(),
            objective_id: objective_id.clone(),
            title: format!("Nested long-horizon step {step:02}"),
            description: Some(format!(
                "Synthetic nested step {step:02} under sustained compaction pressure."
            )),
            status: TaskStatus::InProgress,
            status_updated_at: timestamp(step as i64 * 10),
            dependencies: previous_task_id.into_iter().collect(),
            blocker_ids: if step % 10 == 0 {
                vec![blocker_id.clone()]
            } else {
                Vec::new()
            },
            evidence_ids: vec![evidence_id.clone()],
            artifact_ids: vec![artifact_id.clone()],
            checkpoint_ids: vec![checkpoint_id.clone()],
            metadata: Metadata::new(),
        }))?;

        ledger.apply_update(LedgerUpdate::RecordArtifact(Artifact {
            id: artifact_id.clone(),
            kind: ArtifactKind::Json,
            title: format!("Long-horizon step {step:02} artifact"),
            uri: Some(artifact_uri(step)),
            content_hash: Some(format!("sha256-long-horizon-{step:02}")),
            produced_by: EntityRef::Session(main_session_id.clone()),
            evidence_ids: Vec::new(),
            metadata: Metadata::new(),
        }))?;

        let failed_attempt = step % 7 == 0;
        ledger.apply_update(LedgerUpdate::RecordEvidence(Evidence {
            id: evidence_id.clone(),
            kind: if failed_attempt {
                EvidenceKind::FailedAttempt
            } else {
                EvidenceKind::Command
            },
            summary: pressure_summary(step, pressure_chars_per_step, failed_attempt),
            collected_at: timestamp(step as i64 * 10 + 1),
            collected_by: actor(&format!("worker-{step:02}"), ActorKind::Worker),
            result: if failed_attempt {
                EvidenceResult::Failed
            } else {
                EvidenceResult::Passed
            },
            subjects: vec![EntityRef::Task(current_task_id.clone())],
            command: None,
            source_refs: vec![if failed_attempt {
                EvidenceSourceRef::FailedAttempt {
                    action: format!("synthetic-tool-step-{step:02}"),
                }
            } else {
                EvidenceSourceRef::Command {
                    command: format!("cowork-harness step {step:02}"),
                }
            }],
            artifact_ids: vec![artifact_id.clone()],
            metadata: Metadata::new(),
        }))?;

        if step % 10 == 0 {
            ledger.apply_update(LedgerUpdate::RecordBlocker(Blocker {
                id: blocker_id,
                summary: format!("Synthetic blocker for nested step {step:02}"),
                status: BlockerStatus::Open,
                opened_at: timestamp(step as i64 * 10 + 2),
                resolved_at: None,
                task_ids: vec![current_task_id.clone()],
                required_external_input: vec![format!("operator decision for step {step:02}")],
                evidence_ids: vec![evidence_id.clone()],
                metadata: Metadata::new(),
            }))?;
        }

        ledger.apply_update(LedgerUpdate::RecordCheckpoint(Checkpoint {
            id: checkpoint_id,
            objective_id: objective_id.clone(),
            sequence: step as u64,
            summary: format!("Checkpoint after long-horizon step {step:02}"),
            created_at: timestamp(step as i64 * 10 + 3),
            created_by: actor("orchestrator", ActorKind::CoworkRuntime),
            session_id: Some(main_session_id.clone()),
            task_ids: vec![current_task_id.clone()],
            evidence_ids: vec![evidence_id.clone()],
            artifact_ids: vec![artifact_id],
            stop_reason_id: None,
            metadata: Metadata::new(),
        }))?;

        if !failed_attempt {
            ledger.apply_update(LedgerUpdate::UpdateTaskStatus(TaskStatusUpdate {
                task_id: current_task_id,
                status: TaskStatus::ImplementedUnverified,
                updated_at: timestamp(step as i64 * 10 + 4),
                updated_by: actor("orchestrator", ActorKind::CoworkRuntime),
                evidence_ids: vec![evidence_id],
                blocker_ids: Vec::new(),
                stop_reason_id: None,
                verification_waiver_id: None,
                metadata: Metadata::new(),
            }))?;
        }
    }

    let final_task_id = task_id(config.steps)?;
    let final_evidence_id = evidence_id(config.steps)?;
    let final_stop_reason_id = StopReasonId::parse("stop/long-horizon-verified")?;
    ledger.apply_update(LedgerUpdate::RecordStopReason(StopReasonRecord {
        id: final_stop_reason_id.clone(),
        kind: StopReasonKind::Verified,
        summary: "Long-horizon harness reached verified final stop condition.".to_string(),
        recorded_at: timestamp(900),
        recorded_by: actor("orchestrator", ActorKind::CoworkRuntime),
        subjects: vec![
            EntityRef::Task(final_task_id.clone()),
            EntityRef::Objective(objective_id.clone()),
        ],
        metadata: Metadata::new(),
    }))?;
    ledger.apply_update(LedgerUpdate::UpdateTaskStatus(TaskStatusUpdate {
        task_id: final_task_id,
        status: TaskStatus::Verified,
        updated_at: timestamp(901),
        updated_by: actor("orchestrator", ActorKind::CoworkRuntime),
        evidence_ids: vec![final_evidence_id],
        blocker_ids: Vec::new(),
        stop_reason_id: Some(final_stop_reason_id.clone()),
        verification_waiver_id: None,
        metadata: Metadata::new(),
    }))?;
    ledger.apply_update(LedgerUpdate::UpdateObjectiveProgress(
        ObjectiveProgressUpdate {
            objective_id,
            status: ObjectiveStatus::Verified,
            updated_at: timestamp(902),
            stop_reason_id: Some(final_stop_reason_id),
        },
    ))?;

    Ok(ledger)
}

fn verify_repeated_compaction(
    ledger: &CoworkLedger,
    config: &LongHorizonHarnessConfig,
) -> Result<(), LongHorizonHarnessError> {
    for budget in &config.compaction_budgets {
        let payload = CoworkFallbackPayload::from_ledger(
            ledger,
            ContextFallbackOptions::new(*budget).with_history_hints(Vec::new()),
        );
        assert_root_and_final_state(ledger, &payload, config.steps)?;
    }
    Ok(())
}

fn verify_resume_after_restart(
    ledger: &CoworkLedger,
    config: &LongHorizonHarnessConfig,
) -> Result<LongHorizonHarnessReport, LongHorizonHarnessError> {
    let serialized = serde_json::to_string(ledger)?;
    let resumed: CoworkLedger = serde_json::from_str(&serialized)?;
    if resumed.root().observation() != ledger.root().observation() {
        return Err(LongHorizonHarnessError::Invariant(
            "root observation changed across restart/resume".to_string(),
        ));
    }
    let payload = CoworkFallbackPayload::from_ledger(
        &resumed,
        ContextFallbackOptions::new(*config.compaction_budgets.last().expect("validated budget")),
    );
    assert_root_and_final_state(&resumed, &payload, config.steps)?;
    assert_task_statuses_survive_restart(ledger, &resumed, config.steps)?;
    assert_failed_attempts_and_blockers_survive_restart(&resumed, config.steps)?;

    let final_task_id = task_id(config.steps)?;
    let final_evidence_id = evidence_id(config.steps)?;
    let final_artifact_id = artifact_id(config.steps)?;
    let final_checkpoint_id = checkpoint_id(config.steps)?;
    let final_blocker_id = blocker_id(config.steps)?;
    let final_stop_reason_id = StopReasonId::parse("stop/long-horizon-verified")?;
    let final_result = retrieve_exact(
        &resumed,
        HistoryRetrievalTarget::Task,
        final_task_id.as_str(),
    )?;
    let final_artifact = final_result
        .artifacts
        .iter()
        .find(|artifact| artifact.id == final_artifact_id)
        .ok_or_else(|| {
            LongHorizonHarnessError::Invariant(
                "final linked artifact missing after restart/resume".to_string(),
            )
        })?;
    let final_artifact_uri = final_artifact.uri.clone().ok_or_else(|| {
        LongHorizonHarnessError::Invariant("final artifact URI missing".to_string())
    })?;

    Ok(LongHorizonHarnessReport {
        steps: config.steps,
        failed_tool_attempts: failed_tool_attempt_count(&resumed),
        blockers: resumed.blockers().len(),
        compaction_passes: config.compaction_budgets.len(),
        restart_resume_passes: 1,
        serialized_ledger_bytes: serialized.len(),
        final_task_id,
        final_evidence_id,
        final_artifact_id,
        final_artifact_uri,
        final_checkpoint_id,
        final_blocker_id,
        final_stop_reason_id,
    })
}

fn verify_exact_retrieval_guards(ledger: &CoworkLedger) -> Result<(), LongHorizonHarnessError> {
    let error = HistoryRetrievalResult::from_ledger(
        ledger,
        HistoryRetrievalRequest {
            target: HistoryRetrievalTarget::Evidence,
            id: Some("evidence/long-horizon-missing".to_string()),
            query: None,
            checkpoint_sequence_range: None,
            include_linked: true,
        },
    )
    .expect_err("missing evidence must fail");
    if error.code() != "history_record_not_found" {
        return Err(LongHorizonHarnessError::Invariant(format!(
            "unexpected retrieval error code {}",
            error.code()
        )));
    }
    Ok(())
}

fn assert_root_and_final_state(
    ledger: &CoworkLedger,
    payload: &CoworkFallbackPayload,
    steps: usize,
) -> Result<(), LongHorizonHarnessError> {
    if payload.root_observation != ledger.root().observation() {
        return Err(LongHorizonHarnessError::Invariant(
            "payload root observation changed".to_string(),
        ));
    }
    if payload.objective_progress.status != ObjectiveStatus::Verified {
        return Err(LongHorizonHarnessError::Invariant(
            "verified final stop condition did not survive payload".to_string(),
        ));
    }
    if payload.objective_progress.stop_reason_id.as_ref()
        != Some(&StopReasonId::parse("stop/long-horizon-verified")?)
    {
        return Err(LongHorizonHarnessError::Invariant(
            "final objective stop reason did not survive payload".to_string(),
        ));
    }

    let final_task_id = task_id(steps)?;
    let final_evidence_id = evidence_id(steps)?;
    let final_artifact_id = artifact_id(steps)?;
    let final_checkpoint_id = checkpoint_id(steps)?;
    let final_blocker_id = blocker_id(steps)?;

    let final_task = retrieve_exact(ledger, HistoryRetrievalTarget::Task, final_task_id.as_str())?;
    if final_task.tasks.first().map(|task| task.status) != Some(TaskStatus::Verified) {
        return Err(LongHorizonHarnessError::Invariant(
            "final task status was not verified".to_string(),
        ));
    }
    if !final_task
        .evidence
        .iter()
        .any(|evidence| evidence.id == final_evidence_id)
    {
        return Err(LongHorizonHarnessError::Invariant(
            "final evidence link missing".to_string(),
        ));
    }
    if !final_task
        .artifacts
        .iter()
        .any(|artifact| artifact.id == final_artifact_id && artifact.uri.as_deref().is_some())
    {
        return Err(LongHorizonHarnessError::Invariant(
            "final artifact path missing".to_string(),
        ));
    }
    if !final_task
        .checkpoints
        .iter()
        .any(|checkpoint| checkpoint.id == final_checkpoint_id)
    {
        return Err(LongHorizonHarnessError::Invariant(
            "final checkpoint link missing".to_string(),
        ));
    }
    if !final_task
        .blockers
        .iter()
        .any(|blocker| blocker.id == final_blocker_id)
    {
        return Err(LongHorizonHarnessError::Invariant(
            "final blocker link missing".to_string(),
        ));
    }

    let checkpoint_range = HistoryRetrievalResult::from_ledger(
        ledger,
        HistoryRetrievalRequest {
            target: HistoryRetrievalTarget::Checkpoint,
            id: None,
            query: None,
            checkpoint_sequence_range: Some(crate::retrieval::HistorySequenceRange {
                start: steps as u64,
                end: steps as u64,
            }),
            include_linked: true,
        },
    )?;
    if checkpoint_range.checkpoints.len() != 1 {
        return Err(LongHorizonHarnessError::Invariant(
            "checkpoint range did not return exactly one final checkpoint".to_string(),
        ));
    }
    let failed_attempt_step = first_failed_attempt_step(steps)?;
    let failed_attempt_result = retrieve_exact(
        ledger,
        HistoryRetrievalTarget::Evidence,
        evidence_id(failed_attempt_step)?.as_str(),
    )?;
    let failed_attempt = failed_attempt_result
        .evidence
        .iter()
        .find(|evidence| evidence.id == evidence_id(failed_attempt_step).expect("valid id"))
        .ok_or_else(|| {
            LongHorizonHarnessError::Invariant(
                "failed tool evidence missing from exact retrieval".to_string(),
            )
        })?;
    if failed_attempt.kind != EvidenceKind::FailedAttempt
        || failed_attempt.result != EvidenceResult::Failed
    {
        return Err(LongHorizonHarnessError::Invariant(
            "failed tool evidence did not preserve failed-attempt semantics".to_string(),
        ));
    }
    if failed_attempt_result.tasks.is_empty() || failed_attempt_result.artifacts.is_empty() {
        return Err(LongHorizonHarnessError::Invariant(
            "failed tool evidence lost linked task or artifact".to_string(),
        ));
    }
    Ok(())
}

fn assert_task_statuses_survive_restart(
    original: &CoworkLedger,
    resumed: &CoworkLedger,
    steps: usize,
) -> Result<(), LongHorizonHarnessError> {
    if resumed.tasks().len() != steps {
        return Err(LongHorizonHarnessError::Invariant(format!(
            "expected {steps} resumed tasks, found {}",
            resumed.tasks().len()
        )));
    }

    for step in 1..=steps {
        let id = task_id(step)?;
        let original_status = original
            .tasks()
            .get(&id)
            .ok_or_else(|| {
                LongHorizonHarnessError::Invariant(format!("original task {id} missing"))
            })?
            .status;
        let result = retrieve_exact(resumed, HistoryRetrievalTarget::Task, id.as_str())?;
        let resumed_status = result
            .tasks
            .first()
            .ok_or_else(|| {
                LongHorizonHarnessError::Invariant(format!("resumed task {id} missing"))
            })?
            .status;
        if resumed_status != original_status {
            return Err(LongHorizonHarnessError::Invariant(format!(
                "task {id} status changed from {original_status:?} to {resumed_status:?}"
            )));
        }
    }

    Ok(())
}

fn assert_failed_attempts_and_blockers_survive_restart(
    resumed: &CoworkLedger,
    steps: usize,
) -> Result<(), LongHorizonHarnessError> {
    let expected_failed_attempts = steps / 7;
    let expected_blockers = steps / 10;

    if failed_tool_attempt_count(resumed) != expected_failed_attempts {
        return Err(LongHorizonHarnessError::Invariant(format!(
            "expected {expected_failed_attempts} failed tool attempts after restart/resume, found {}",
            failed_tool_attempt_count(resumed)
        )));
    }
    if resumed.blockers().len() != expected_blockers {
        return Err(LongHorizonHarnessError::Invariant(format!(
            "expected {expected_blockers} blockers after restart/resume, found {}",
            resumed.blockers().len()
        )));
    }

    for step in (10..=steps).step_by(10) {
        let id = blocker_id(step)?;
        let result = retrieve_exact(resumed, HistoryRetrievalTarget::Blocker, id.as_str())?;
        if result.blockers.first().map(|blocker| blocker.id.clone()) != Some(id) {
            return Err(LongHorizonHarnessError::Invariant(format!(
                "blocker for step {step:02} missing after restart/resume"
            )));
        }
        if result.tasks.is_empty() || result.evidence.is_empty() {
            return Err(LongHorizonHarnessError::Invariant(format!(
                "blocker for step {step:02} lost linked task or evidence after restart/resume"
            )));
        }
    }

    Ok(())
}

fn validate_payload_against_long_horizon_ledger(
    ledger: &CoworkLedger,
    payload: &CoworkFallbackPayload,
) -> Result<(), LongHorizonHarnessError> {
    if payload.root_observation != ledger.root().observation() {
        return Err(LongHorizonHarnessError::Invariant(
            "payload root observation drifted from immutable ledger root".to_string(),
        ));
    }
    if payload.objective_progress != *ledger.objective_progress() {
        return Err(LongHorizonHarnessError::Invariant(
            "payload objective progress drifted from ledger progress".to_string(),
        ));
    }
    if payload.token_accounting.original_counts.tasks != ledger.tasks().len()
        || payload.token_accounting.original_counts.evidence != ledger.evidence().len()
        || payload.token_accounting.original_counts.blockers != ledger.blockers().len()
    {
        return Err(LongHorizonHarnessError::Invariant(
            "payload original counts lost ledger records".to_string(),
        ));
    }

    if payload.evidence.is_empty() {
        return Err(LongHorizonHarnessError::Invariant(
            "payload dropped every evidence record".to_string(),
        ));
    }
    if payload.token_accounting.trimmed_counts.evidence == 0
        && payload.evidence.len() != ledger.evidence().len()
    {
        return Err(LongHorizonHarnessError::Invariant(
            "untrimmed payload lost evidence records".to_string(),
        ));
    }
    if payload.token_accounting.trimmed_counts.blockers == 0
        && payload.blockers.len() != ledger.blockers().len()
    {
        return Err(LongHorizonHarnessError::Invariant(
            "untrimmed payload lost blocker records".to_string(),
        ));
    }

    for task in &payload.tasks {
        let ledger_task = ledger.tasks().get(&task.id).ok_or_else(|| {
            LongHorizonHarnessError::Invariant(format!("payload invented task {}", task.id))
        })?;
        if task.status != ledger_task.status {
            return Err(LongHorizonHarnessError::Invariant(format!(
                "payload task {} status drifted from {:?} to {:?}",
                task.id, ledger_task.status, task.status
            )));
        }
        if task.evidence_ids != ledger_task.evidence_ids
            || task.artifact_ids != ledger_task.artifact_ids
            || task.checkpoint_ids != ledger_task.checkpoint_ids
            || task.blocker_ids != ledger_task.blocker_ids
        {
            return Err(LongHorizonHarnessError::Invariant(format!(
                "payload task {} lost ledger links",
                task.id
            )));
        }
    }

    Ok(())
}

fn expect_fixture_rejection(
    ledger: &CoworkLedger,
    payload: &CoworkFallbackPayload,
    fixture_name: &str,
    counter: &mut usize,
) -> Result<(), LongHorizonHarnessError> {
    match validate_payload_against_long_horizon_ledger(ledger, payload) {
        Ok(()) => Err(LongHorizonHarnessError::Invariant(format!(
            "{fixture_name} was accepted"
        ))),
        Err(LongHorizonHarnessError::Invariant(_)) => {
            *counter += 1;
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn failed_tool_attempt_count(ledger: &CoworkLedger) -> usize {
    ledger
        .evidence()
        .values()
        .filter(|evidence| {
            evidence.kind == EvidenceKind::FailedAttempt
                && evidence.result == EvidenceResult::Failed
        })
        .count()
}

fn first_failed_attempt_step(steps: usize) -> Result<usize, LongHorizonHarnessError> {
    (1..=steps).find(|step| step % 7 == 0).ok_or_else(|| {
        LongHorizonHarnessError::Invariant(
            "harness config did not include a failed tool attempt".to_string(),
        )
    })
}

fn retrieve_exact(
    ledger: &CoworkLedger,
    target: HistoryRetrievalTarget,
    id: &str,
) -> Result<HistoryRetrievalResult, LongHorizonHarnessError> {
    Ok(HistoryRetrievalResult::from_ledger(
        ledger,
        HistoryRetrievalRequest {
            target,
            id: Some(id.to_string()),
            query: None,
            checkpoint_sequence_range: None,
            include_linked: true,
        },
    )?)
}

fn pressure_summary(step: usize, pressure_chars: usize, failed_attempt: bool) -> String {
    let outcome = if failed_attempt {
        "failed tool attempt retained for diagnosis"
    } else {
        "successful evidence retained for completion"
    };
    format!(
        "Step {step:02} {outcome}. {}",
        "context-pressure ".repeat(pressure_chars / "context-pressure ".len())
    )
}

fn timestamp(offset_seconds: i64) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 21, 1, 0, 0)
        .single()
        .expect("valid timestamp")
        + Duration::seconds(offset_seconds)
}

fn actor(id: &str, kind: ActorKind) -> ActorRef {
    ActorRef {
        kind,
        id: id.to_string(),
        display_name: Some(id.to_string()),
    }
}

fn objective_id() -> RootObjectiveId {
    RootObjectiveId::parse("objective/long-horizon").expect("valid objective id")
}

fn task_id(step: usize) -> Result<TaskId, LedgerError> {
    TaskId::parse(format!("task/long-horizon/{step:02}"))
}

fn evidence_id(step: usize) -> Result<EvidenceId, LedgerError> {
    EvidenceId::parse(format!("evidence/long-horizon/{step:02}"))
}

fn checkpoint_id(step: usize) -> Result<CheckpointId, LedgerError> {
    CheckpointId::parse(format!("checkpoint/long-horizon/{step:02}"))
}

fn artifact_id(step: usize) -> Result<ArtifactId, LedgerError> {
    ArtifactId::parse(format!("artifact/long-horizon/{step:02}"))
}

fn blocker_id(step: usize) -> Result<BlockerId, LedgerError> {
    BlockerId::parse(format!("blocker/long-horizon/{step:02}"))
}

fn artifact_uri(step: usize) -> String {
    format!("memory://long-horizon/artifacts/step-{step:02}.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_horizon_harness_survives_compaction_and_restart_resume() {
        let report = run_long_horizon_harness(LongHorizonHarnessConfig::default())
            .expect("long-horizon harness");

        assert_eq!(report.steps, DEFAULT_STEPS);
        assert_eq!(report.failed_tool_attempts, 7);
        assert_eq!(report.blockers, 5);
        assert_eq!(report.compaction_passes, 4);
        assert_eq!(report.restart_resume_passes, 1);
        assert_eq!(report.final_task_id.as_str(), "task/long-horizon/50");
        assert_eq!(
            report.final_evidence_id.as_str(),
            "evidence/long-horizon/50"
        );
        assert_eq!(
            report.final_checkpoint_id.as_str(),
            "checkpoint/long-horizon/50"
        );
        assert_eq!(report.final_blocker_id.as_str(), "blocker/long-horizon/50");
        assert_eq!(
            report.final_artifact_uri,
            "memory://long-horizon/artifacts/step-50.json"
        );
        assert!(report.serialized_ledger_bytes > DEFAULT_APPROX_TOKEN_PRESSURE * 4);
    }

    #[test]
    fn long_horizon_harness_rejects_underpowered_config() {
        let error = run_long_horizon_harness(LongHorizonHarnessConfig {
            steps: 49,
            approx_token_pressure: DEFAULT_APPROX_TOKEN_PRESSURE,
            compaction_budgets: vec![1_500],
        })
        .unwrap_err();

        assert!(matches!(error, LongHorizonHarnessError::InvalidConfig(_)));
    }

    #[test]
    fn drift_regression_fixtures_reject_broken_compaction_outputs() {
        let report = run_drift_regression_fixtures(LongHorizonHarnessConfig::default())
            .expect("drift fixtures");

        assert_eq!(report.compaction_turns, DEFAULT_STEPS + 1);
        assert_eq!(report.root_rewrite_rejections, DEFAULT_STEPS + 1);
        assert_eq!(report.constraint_drop_rejections, DEFAULT_STEPS + 1);
        assert_eq!(report.evidence_drop_rejections, DEFAULT_STEPS + 1);
        assert_eq!(report.false_completion_rejections, DEFAULT_STEPS + 1);
        assert_eq!(report.blocker_omission_rejections, DEFAULT_STEPS + 1);
    }
}
