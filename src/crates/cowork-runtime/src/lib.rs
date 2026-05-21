//! Autonomous objective loop primitives for Cowork.
//!
//! The runtime is intentionally ledger-backed. Model messages, tool output, and
//! retries become evidence and typed stop reasons before a task can close.

use chrono::Utc;
use cowork_ledger::{
    ActorRef, CoworkLedger, EntityRef, Evidence, EvidenceId, EvidenceKind, EvidenceResult,
    LedgerError, LedgerUpdate, Metadata, ObjectiveProgressUpdate, ObjectiveStatus, StopReasonId,
    StopReasonKind, StopReasonRecord, Task, TaskId, TaskStatus, TaskStatusUpdate,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

pub const RUNTIME_VERSION: &str = "cowork.runtime.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeStopKind {
    Verified,
    NoReadyTask,
    Cancelled,
    MaxTurnsReached,
    BudgetExceeded,
    Failed,
    ModelMessageWithoutEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStop {
    pub kind: RuntimeStopKind,
    pub turns: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyntheticRunReport {
    pub stop: RuntimeStop,
    pub selected_task_id: Option<String>,
    pub tool_calls: u32,
    pub evidence_records: usize,
    pub stop_reasons: usize,
    pub repaired_after_failure: bool,
    pub final_task_status: Option<TaskStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeOptions {
    pub max_turns: u32,
    pub budget_limit: u32,
    pub max_repair_attempts: u32,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            max_turns: 16,
            budget_limit: 16,
            max_repair_attempts: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRequest {
    pub task_id: TaskId,
    pub attempt: u32,
    pub tool_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub success: bool,
    pub evidence_summary: Option<String>,
    pub evidence_result: EvidenceResult,
    pub model_message: Option<String>,
}

impl ToolResult {
    pub fn passed(summary: impl Into<String>) -> Self {
        Self {
            success: true,
            evidence_summary: Some(summary.into()),
            evidence_result: EvidenceResult::Passed,
            model_message: None,
        }
    }

    pub fn failed(summary: impl Into<String>) -> Self {
        Self {
            success: false,
            evidence_summary: Some(summary.into()),
            evidence_result: EvidenceResult::Failed,
            model_message: None,
        }
    }

    pub fn model_message(message: impl Into<String>) -> Self {
        Self {
            success: true,
            evidence_summary: None,
            evidence_result: EvidenceResult::Informational,
            model_message: Some(message.into()),
        }
    }
}

pub trait ObjectiveDriver {
    fn execute(&mut self, request: &ToolRequest, task: &Task) -> ToolResult;
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Ledger(#[from] LedgerError),
}

pub struct CoworkRuntime<D> {
    ledger: CoworkLedger,
    driver: D,
    options: RuntimeOptions,
    actor: ActorRef,
    turns: u32,
    tool_calls: u32,
    repaired_after_failure: bool,
    selected_task_id: Option<TaskId>,
    attempts: BTreeMap<TaskId, u32>,
    cancel_requested: bool,
}

impl<D> CoworkRuntime<D>
where
    D: ObjectiveDriver,
{
    pub fn new(ledger: CoworkLedger, driver: D, options: RuntimeOptions, actor: ActorRef) -> Self {
        Self {
            ledger,
            driver,
            options,
            actor,
            turns: 0,
            tool_calls: 0,
            repaired_after_failure: false,
            selected_task_id: None,
            attempts: BTreeMap::new(),
            cancel_requested: false,
        }
    }

    pub fn request_cancel(&mut self) {
        self.cancel_requested = true;
    }

    pub fn next_task(&self) -> Option<&Task> {
        self.ledger.tasks().values().find(|task| {
            matches!(
                task.status,
                TaskStatus::NotStarted | TaskStatus::InProgress | TaskStatus::ImplementedUnverified
            )
        })
    }

    pub fn run(mut self) -> Result<(CoworkLedger, SyntheticRunReport), RuntimeError> {
        loop {
            if self.cancel_requested {
                let stop = self.stop_next_task(
                    RuntimeStopKind::Cancelled,
                    TaskStatus::Cancelled,
                    StopReasonKind::Cancelled,
                    "Runtime cancellation requested.",
                )?;
                return Ok(self.finish(stop));
            }

            if self.options.max_turns > 0 && self.turns >= self.options.max_turns {
                let stop = self.stop_objective(
                    RuntimeStopKind::MaxTurnsReached,
                    ObjectiveStatus::Blocked,
                    StopReasonKind::EnvironmentDegraded,
                    "Runtime stopped after reaching the max-turn guard.",
                )?;
                return Ok(self.finish(stop));
            }

            if self.options.budget_limit > 0 && self.tool_calls >= self.options.budget_limit {
                let stop = self.stop_objective(
                    RuntimeStopKind::BudgetExceeded,
                    ObjectiveStatus::Blocked,
                    StopReasonKind::EnvironmentDegraded,
                    "Runtime stopped after reaching the tool budget guard.",
                )?;
                return Ok(self.finish(stop));
            }

            let Some(task) = self.next_task().cloned() else {
                let all_verified = self
                    .ledger
                    .tasks()
                    .values()
                    .all(|task| task.status == TaskStatus::Verified);
                if all_verified {
                    let stop = self.stop_objective(
                        RuntimeStopKind::Verified,
                        ObjectiveStatus::Verified,
                        StopReasonKind::Verified,
                        "All scheduled tasks are verified.",
                    )?;
                    return Ok(self.finish(stop));
                }

                let stop = RuntimeStop {
                    kind: RuntimeStopKind::NoReadyTask,
                    turns: self.turns,
                    task_id: None,
                    stop_reason_id: None,
                };
                return Ok(self.finish(stop));
            };

            self.selected_task_id.get_or_insert_with(|| task.id.clone());
            self.ensure_in_progress(&task)?;

            let attempt = *self.attempts.entry(task.id.clone()).or_insert(0);
            self.turns += 1;
            self.tool_calls += 1;

            let request = ToolRequest {
                task_id: task.id.clone(),
                attempt,
                tool_name: "synthetic-objective-tool".to_string(),
            };
            let result = self.driver.execute(&request, &task);

            if result.success
                && result.evidence_result == EvidenceResult::Passed
                && result.evidence_summary.is_some()
            {
                let evidence_id = self.record_tool_evidence(&task.id, attempt, result)?;
                self.verify_task(&task, evidence_id)?;
                continue;
            }

            if result.success && result.evidence_summary.is_none() && result.model_message.is_some()
            {
                let stop = self.stop_task(
                    &task.id,
                    RuntimeStopKind::ModelMessageWithoutEvidence,
                    TaskStatus::Failed,
                    StopReasonKind::Failed,
                    "Model message did not include passed evidence, so the task cannot close.",
                )?;
                return Ok(self.finish(stop));
            }

            self.record_tool_evidence(&task.id, attempt, result)?;
            if attempt < self.options.max_repair_attempts {
                self.repaired_after_failure = true;
                self.attempts.insert(task.id.clone(), attempt + 1);
                continue;
            }

            let stop = self.stop_task(
                &task.id,
                RuntimeStopKind::Failed,
                TaskStatus::Failed,
                StopReasonKind::Failed,
                "Runtime failed after exhausting repair attempts.",
            )?;
            return Ok(self.finish(stop));
        }
    }

    fn finish(self, stop: RuntimeStop) -> (CoworkLedger, SyntheticRunReport) {
        let selected_task_id = self.selected_task_id.as_ref().map(ToString::to_string);
        let final_task_status = self
            .selected_task_id
            .as_ref()
            .and_then(|task_id| self.ledger.tasks().get(task_id))
            .map(|task| task.status);
        let report = SyntheticRunReport {
            stop,
            selected_task_id,
            tool_calls: self.tool_calls,
            evidence_records: self.ledger.evidence().len(),
            stop_reasons: self.ledger.stop_reasons().len(),
            repaired_after_failure: self.repaired_after_failure,
            final_task_status,
        };
        (self.ledger, report)
    }

    fn ensure_in_progress(&mut self, task: &Task) -> Result<(), LedgerError> {
        if task.status != TaskStatus::NotStarted {
            return Ok(());
        }

        self.ledger
            .apply_update(LedgerUpdate::UpdateTaskStatus(TaskStatusUpdate {
                task_id: task.id.clone(),
                status: TaskStatus::InProgress,
                updated_at: Utc::now(),
                updated_by: self.actor.clone(),
                evidence_ids: Vec::new(),
                blocker_ids: task.blocker_ids.clone(),
                stop_reason_id: None,
                verification_waiver_id: None,
                metadata: runtime_metadata("selected"),
            }))?;
        Ok(())
    }

    fn record_tool_evidence(
        &mut self,
        task_id: &TaskId,
        attempt: u32,
        result: ToolResult,
    ) -> Result<EvidenceId, LedgerError> {
        let evidence_id = EvidenceId::parse(format!(
            "evidence/runtime-turn-{}-attempt-{}",
            self.turns, attempt
        ))?;
        let summary = result
            .evidence_summary
            .unwrap_or_else(|| "Tool returned no evidence.".to_string());
        self.ledger
            .apply_update(LedgerUpdate::RecordEvidence(Evidence {
                id: evidence_id.clone(),
                kind: if result.success {
                    EvidenceKind::Test
                } else {
                    EvidenceKind::FailedAttempt
                },
                summary,
                collected_at: Utc::now(),
                collected_by: self.actor.clone(),
                result: result.evidence_result,
                subjects: vec![EntityRef::Task(task_id.clone())],
                command: None,
                source_refs: Vec::new(),
                artifact_ids: Vec::new(),
                metadata: runtime_metadata("tool_result"),
            }))?;
        Ok(evidence_id)
    }

    fn verify_task(&mut self, task: &Task, evidence_id: EvidenceId) -> Result<(), LedgerError> {
        let stop_reason_id = self.record_task_stop_reason(
            &task.id,
            StopReasonKind::Verified,
            "Task has passed evidence from the objective loop.",
        )?;
        self.ledger
            .apply_update(LedgerUpdate::UpdateTaskStatus(TaskStatusUpdate {
                task_id: task.id.clone(),
                status: TaskStatus::Verified,
                updated_at: Utc::now(),
                updated_by: self.actor.clone(),
                evidence_ids: vec![evidence_id],
                blocker_ids: task.blocker_ids.clone(),
                stop_reason_id: Some(stop_reason_id),
                verification_waiver_id: None,
                metadata: runtime_metadata("verified"),
            }))?;
        Ok(())
    }

    fn stop_next_task(
        &mut self,
        kind: RuntimeStopKind,
        status: TaskStatus,
        stop_kind: StopReasonKind,
        summary: &str,
    ) -> Result<RuntimeStop, LedgerError> {
        let Some(task) = self.next_task().cloned() else {
            return Ok(RuntimeStop {
                kind,
                turns: self.turns,
                task_id: None,
                stop_reason_id: None,
            });
        };
        self.stop_task(&task.id, kind, status, stop_kind, summary)
    }

    fn stop_task(
        &mut self,
        task_id: &TaskId,
        kind: RuntimeStopKind,
        status: TaskStatus,
        stop_kind: StopReasonKind,
        summary: &str,
    ) -> Result<RuntimeStop, LedgerError> {
        let task = self
            .ledger
            .tasks()
            .get(task_id)
            .expect("runtime selected an existing task")
            .clone();
        let stop_reason_id = self.record_task_stop_reason(task_id, stop_kind, summary)?;
        self.ledger
            .apply_update(LedgerUpdate::UpdateTaskStatus(TaskStatusUpdate {
                task_id: task_id.clone(),
                status,
                updated_at: Utc::now(),
                updated_by: self.actor.clone(),
                evidence_ids: Vec::new(),
                blocker_ids: task.blocker_ids,
                stop_reason_id: Some(stop_reason_id.clone()),
                verification_waiver_id: None,
                metadata: runtime_metadata("stopped"),
            }))?;
        Ok(RuntimeStop {
            kind,
            turns: self.turns,
            task_id: Some(task_id.to_string()),
            stop_reason_id: Some(stop_reason_id.to_string()),
        })
    }

    fn stop_objective(
        &mut self,
        kind: RuntimeStopKind,
        status: ObjectiveStatus,
        stop_kind: StopReasonKind,
        summary: &str,
    ) -> Result<RuntimeStop, LedgerError> {
        let objective_id = self.ledger.root().objective().id().clone();
        let stop_reason_id = StopReasonId::parse(format!("stop/runtime-objective-{}", self.turns))?;
        self.ledger
            .apply_update(LedgerUpdate::RecordStopReason(StopReasonRecord {
                id: stop_reason_id.clone(),
                kind: stop_kind,
                summary: summary.to_string(),
                recorded_at: Utc::now(),
                recorded_by: self.actor.clone(),
                subjects: vec![EntityRef::Objective(objective_id.clone())],
                metadata: runtime_metadata("objective_stop"),
            }))?;
        self.ledger
            .apply_update(LedgerUpdate::UpdateObjectiveProgress(
                ObjectiveProgressUpdate {
                    objective_id,
                    status,
                    updated_at: Utc::now(),
                    stop_reason_id: Some(stop_reason_id.clone()),
                },
            ))?;
        Ok(RuntimeStop {
            kind,
            turns: self.turns,
            task_id: None,
            stop_reason_id: Some(stop_reason_id.to_string()),
        })
    }

    fn record_task_stop_reason(
        &mut self,
        task_id: &TaskId,
        kind: StopReasonKind,
        summary: &str,
    ) -> Result<StopReasonId, LedgerError> {
        let stop_reason_id = StopReasonId::parse(format!(
            "stop/runtime-task-{}-{}",
            self.turns,
            kind_slug(kind)
        ))?;
        self.ledger
            .apply_update(LedgerUpdate::RecordStopReason(StopReasonRecord {
                id: stop_reason_id.clone(),
                kind,
                summary: summary.to_string(),
                recorded_at: Utc::now(),
                recorded_by: self.actor.clone(),
                subjects: vec![EntityRef::Task(task_id.clone())],
                metadata: runtime_metadata("task_stop"),
            }))?;
        Ok(stop_reason_id)
    }
}

pub fn runtime_metadata(event: &str) -> Metadata {
    let mut metadata = Metadata::new();
    metadata.insert(
        "runtimeVersion".to_string(),
        serde_json::Value::String(RUNTIME_VERSION.to_string()),
    );
    metadata.insert(
        "runtimeEvent".to_string(),
        serde_json::Value::String(event.to_string()),
    );
    metadata
}

fn kind_slug(kind: StopReasonKind) -> &'static str {
    match kind {
        StopReasonKind::Verified => "verified",
        StopReasonKind::UserPaused => "user-paused",
        StopReasonKind::SessionCapacityReached => "session-capacity",
        StopReasonKind::BlockedExternalInput => "blocked-external-input",
        StopReasonKind::EnvironmentDegraded => "environment-degraded",
        StopReasonKind::Descoped => "descoped",
        StopReasonKind::Cancelled => "cancelled",
        StopReasonKind::Failed => "failed",
    }
}

pub fn run_deterministic_synthetic_loop(
    ledger: CoworkLedger,
    actor: ActorRef,
) -> Result<(CoworkLedger, SyntheticRunReport), RuntimeError> {
    let runtime = CoworkRuntime::new(
        ledger,
        SequenceDriver::new(vec![
            ToolResult::failed("initial tool failure"),
            ToolResult::passed("repair attempt passed acceptance probes"),
        ]),
        RuntimeOptions::default(),
        actor,
    );
    runtime.run()
}

#[derive(Debug, Clone)]
pub struct SequenceDriver {
    results: Vec<ToolResult>,
    index: usize,
}

impl SequenceDriver {
    pub fn new(results: Vec<ToolResult>) -> Self {
        Self { results, index: 0 }
    }
}

impl ObjectiveDriver for SequenceDriver {
    fn execute(&mut self, _request: &ToolRequest, _task: &Task) -> ToolResult {
        let result = self
            .results
            .get(self.index)
            .cloned()
            .unwrap_or_else(|| ToolResult::failed("scripted driver exhausted"));
        self.index += 1;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cowork_ledger::{
        ActorKind, ConstraintSeverity, ConstraintSource, LedgerInitialization, RootConstraintId,
        RootConstraintInitialization, RootObjectiveId,
    };

    fn actor() -> ActorRef {
        ActorRef {
            kind: ActorKind::Worker,
            id: "runtime-test".to_string(),
            display_name: Some("runtime test".to_string()),
        }
    }

    fn objective_id() -> RootObjectiveId {
        RootObjectiveId::parse("objective/runtime-test").expect("objective id")
    }

    fn task_id() -> TaskId {
        TaskId::parse("task/runtime-test").expect("task id")
    }

    fn ledger_with_task(status: TaskStatus) -> CoworkLedger {
        let mut ledger = CoworkLedger::initialize(LedgerInitialization {
            root_objective_id: objective_id(),
            root_objective_text: "Prove the autonomous objective loop.".to_string(),
            acceptance_criteria: vec!["A model message without evidence cannot close.".to_string()],
            root_constraints: vec![RootConstraintInitialization {
                id: RootConstraintId::parse("constraint/runtime-test").expect("constraint id"),
                text: "Evidence is required before verified completion.".to_string(),
                severity: ConstraintSeverity::Required,
                source: ConstraintSource::RuntimePolicy,
                metadata: Metadata::new(),
            }],
            initialized_at: Utc::now(),
            initialized_by: actor(),
            metadata: Metadata::new(),
        })
        .expect("initialize ledger");

        ledger
            .apply_update(LedgerUpdate::RegisterTask(Task {
                id: task_id(),
                objective_id: objective_id(),
                title: "Run synthetic objective task".to_string(),
                description: None,
                status,
                status_updated_at: Utc::now(),
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

    fn run_with(results: Vec<ToolResult>) -> (CoworkLedger, SyntheticRunReport) {
        CoworkRuntime::new(
            ledger_with_task(TaskStatus::NotStarted),
            SequenceDriver::new(results),
            RuntimeOptions::default(),
            actor(),
        )
        .run()
        .expect("runtime run")
    }

    #[test]
    fn selects_next_task_records_tool_evidence_and_verifies_with_typed_stop() {
        let (ledger, report) = run_with(vec![ToolResult::passed("tool call passed")]);

        assert_eq!(
            report.selected_task_id.as_deref(),
            Some("task/runtime-test")
        );
        assert_eq!(report.tool_calls, 1);
        assert_eq!(report.evidence_records, 1);
        assert_eq!(report.final_task_status, Some(TaskStatus::Verified));
        assert_eq!(
            ledger.tasks().get(&task_id()).unwrap().status,
            TaskStatus::Verified
        );
        assert_eq!(ledger.task_transitions().len(), 2);
        assert!(ledger.stop_reasons().values().any(|reason| {
            reason.kind == StopReasonKind::Verified
                && reason.subjects == vec![EntityRef::Task(task_id())]
        }));
    }

    #[test]
    fn failed_tool_result_repairs_then_verifies() {
        let (ledger, report) =
            run_deterministic_synthetic_loop(ledger_with_task(TaskStatus::NotStarted), actor())
                .expect("synthetic loop");

        assert!(report.repaired_after_failure);
        assert_eq!(report.tool_calls, 2);
        assert_eq!(report.evidence_records, 2);
        assert_eq!(report.final_task_status, Some(TaskStatus::Verified));
        let task = ledger.tasks().get(&task_id()).unwrap();
        assert_eq!(task.status, TaskStatus::Verified);
        assert_eq!(task.evidence_ids.len(), 2);
    }

    #[test]
    fn max_turn_guard_returns_typed_objective_stop() {
        let runtime = CoworkRuntime::new(
            ledger_with_task(TaskStatus::NotStarted),
            SequenceDriver::new(vec![ToolResult::failed("keeps failing")]),
            RuntimeOptions {
                max_turns: 1,
                budget_limit: 10,
                max_repair_attempts: 3,
            },
            actor(),
        );

        let (ledger, report) = runtime.run().expect("runtime run");

        assert_eq!(report.stop.kind, RuntimeStopKind::MaxTurnsReached);
        assert_eq!(ledger.objective_progress().status, ObjectiveStatus::Blocked);
        assert!(ledger.stop_reasons().values().any(|reason| {
            reason.kind == StopReasonKind::EnvironmentDegraded
                && reason.subjects == vec![EntityRef::Objective(objective_id())]
        }));
    }

    #[test]
    fn budget_guard_returns_typed_objective_stop() {
        let runtime = CoworkRuntime::new(
            ledger_with_task(TaskStatus::NotStarted),
            SequenceDriver::new(vec![ToolResult::failed("keeps failing")]),
            RuntimeOptions {
                max_turns: 10,
                budget_limit: 1,
                max_repair_attempts: 3,
            },
            actor(),
        );

        let (_ledger, report) = runtime.run().expect("runtime run");

        assert_eq!(report.stop.kind, RuntimeStopKind::BudgetExceeded);
        assert_eq!(report.tool_calls, 1);
    }

    #[test]
    fn cancellation_updates_task_with_cancelled_stop_reason() {
        let mut runtime = CoworkRuntime::new(
            ledger_with_task(TaskStatus::NotStarted),
            SequenceDriver::new(vec![ToolResult::passed("should not run")]),
            RuntimeOptions::default(),
            actor(),
        );
        runtime.request_cancel();

        let (ledger, report) = runtime.run().expect("runtime run");

        assert_eq!(report.stop.kind, RuntimeStopKind::Cancelled);
        assert_eq!(report.tool_calls, 0);
        assert_eq!(
            ledger.tasks().get(&task_id()).unwrap().status,
            TaskStatus::Cancelled
        );
        assert!(ledger.stop_reasons().values().any(|reason| {
            reason.kind == StopReasonKind::Cancelled
                && reason.subjects == vec![EntityRef::Task(task_id())]
        }));
    }

    #[test]
    fn model_message_without_evidence_cannot_close_task() {
        let (ledger, report) = run_with(vec![ToolResult::model_message("done")]);

        assert_eq!(
            report.stop.kind,
            RuntimeStopKind::ModelMessageWithoutEvidence
        );
        assert_eq!(report.evidence_records, 0);
        assert_eq!(report.final_task_status, Some(TaskStatus::Failed));
        assert_ne!(
            ledger.tasks().get(&task_id()).unwrap().status,
            TaskStatus::Verified
        );
    }
}
