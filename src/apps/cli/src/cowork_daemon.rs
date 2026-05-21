use clap::{Args, Subcommand};
use cowork_protocol::{
    Artifact, ArtifactKind, ArtifactProvenance, AuthorityInvariant, BrowserState, BrowserStatus,
    Checkpoint, CoworkSnapshot, Evidence, EvidenceKind, Metadata, Objective, ObjectiveStatus,
    ProtocolEnvelope, ProtocolError, ProtocolErrorCode, ProtocolVersion, RunStatus, Task,
    TaskStatus,
};
use serde::Serialize;
use std::error::Error;
use std::fmt;

const SUPPORTED_ENDPOINT: &str = "in-process";
const SUPPORTED_TARGET: &str = "local-smoke";
const START_RESPONSE_ID: &str = "cowork-daemon-start";

#[derive(Debug, Clone, Subcommand)]
pub enum DaemonAction {
    /// Start and connect to the deterministic in-process smoke daemon
    Start(DaemonSmokeOptions),
    /// Report run status from the deterministic smoke daemon
    Status(DaemonSmokeOptions),
    /// Export the deterministic CoworkSnapshot smoke state
    Export(DaemonSmokeOptions),
}

#[derive(Debug, Clone, Args, PartialEq, Eq)]
pub struct DaemonSmokeOptions {
    /// Smoke daemon endpoint. The local smoke boundary supports only in-process.
    #[arg(long, default_value = SUPPORTED_ENDPOINT)]
    pub endpoint: String,

    /// Smoke daemon target. The local smoke boundary supports only local-smoke.
    #[arg(long, default_value = SUPPORTED_TARGET)]
    pub target: String,
}

impl Default for DaemonSmokeOptions {
    fn default() -> Self {
        Self {
            endpoint: SUPPORTED_ENDPOINT.to_string(),
            target: SUPPORTED_TARGET.to_string(),
        }
    }
}

pub fn run_action(action: DaemonAction) -> Result<String, CoworkDaemonSmokeError> {
    match action {
        DaemonAction::Start(options) => to_pretty_json(&start_output(options)?),
        DaemonAction::Status(options) => to_pretty_json(&status_output(options)?),
        DaemonAction::Export(options) => to_pretty_json(&export_snapshot(options)?),
    }
}

pub fn render_error(error: &CoworkDaemonSmokeError) -> String {
    serde_json::to_string_pretty(&error.report()).unwrap_or_else(|_| {
        format!(
            "{{\"code\":\"{}\",\"message\":\"{}\"}}",
            error.code(),
            error
        )
    })
}

fn start_output(options: DaemonSmokeOptions) -> Result<DaemonStartOutput, CoworkDaemonSmokeError> {
    let request = SmokeDaemonRequest::new(options.endpoint, options.target)?;
    let mut controller = SmokeDaemonController::default();
    controller.start(request.clone())?;
    let connection = controller.connect(request)?;

    Ok(DaemonStartOutput {
        endpoint: connection.endpoint.to_string(),
        target: connection.target.to_string(),
        connected: true,
        state: ProtocolEnvelope::response(START_RESPONSE_ID, connection.snapshot),
    })
}

fn status_output(
    options: DaemonSmokeOptions,
) -> Result<DaemonStatusOutput, CoworkDaemonSmokeError> {
    let request = SmokeDaemonRequest::new(options.endpoint, options.target)?;
    let snapshot = start_and_connect(request.clone())?;

    Ok(DaemonStatusOutput {
        endpoint: request.endpoint.as_str().to_string(),
        target: request.target.as_str().to_string(),
        connected: true,
        protocol_version: snapshot.protocol_version,
        run_status: snapshot.run_status,
        objective_id: snapshot.objective.id,
    })
}

fn export_snapshot(options: DaemonSmokeOptions) -> Result<CoworkSnapshot, CoworkDaemonSmokeError> {
    let request = SmokeDaemonRequest::new(options.endpoint, options.target)?;
    start_and_connect(request)
}

fn start_and_connect(
    request: SmokeDaemonRequest,
) -> Result<CoworkSnapshot, CoworkDaemonSmokeError> {
    let mut controller = SmokeDaemonController::default();
    controller.start(request.clone())?;
    Ok(controller.connect(request)?.snapshot)
}

fn to_pretty_json<T: Serialize>(value: &T) -> Result<String, CoworkDaemonSmokeError> {
    serde_json::to_string_pretty(value).map_err(CoworkDaemonSmokeError::serialization)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmokeDaemonRequest {
    endpoint: SmokeEndpoint,
    target: SmokeTarget,
}

impl SmokeDaemonRequest {
    pub fn new(
        endpoint: impl Into<String>,
        target: impl Into<String>,
    ) -> Result<Self, CoworkDaemonSmokeError> {
        Ok(Self {
            endpoint: SmokeEndpoint::parse(endpoint.into())?,
            target: SmokeTarget::parse(target.into())?,
        })
    }
}

impl Default for SmokeDaemonRequest {
    fn default() -> Self {
        Self {
            endpoint: SmokeEndpoint::InProcess,
            target: SmokeTarget::LocalSmoke,
        }
    }
}

#[derive(Debug, Default)]
pub struct SmokeDaemonController {
    daemon: Option<LocalSmokeDaemon>,
}

impl SmokeDaemonController {
    pub fn start(
        &mut self,
        request: SmokeDaemonRequest,
    ) -> Result<CoworkSnapshot, CoworkDaemonSmokeError> {
        let daemon = LocalSmokeDaemon::start(request.endpoint, request.target);
        let snapshot = daemon.snapshot();
        self.daemon = Some(daemon);
        Ok(snapshot)
    }

    pub fn connect(
        &self,
        request: SmokeDaemonRequest,
    ) -> Result<SmokeDaemonConnection, CoworkDaemonSmokeError> {
        let daemon = self
            .daemon
            .as_ref()
            .ok_or(CoworkDaemonSmokeError::NotStarted)?;

        if daemon.endpoint != request.endpoint {
            return Err(CoworkDaemonSmokeError::UnsupportedEndpoint {
                endpoint: request.endpoint.as_str().to_string(),
            });
        }

        if daemon.target != request.target {
            return Err(CoworkDaemonSmokeError::UnsupportedTarget {
                target: request.target.as_str().to_string(),
            });
        }

        Ok(SmokeDaemonConnection {
            endpoint: daemon.endpoint,
            target: daemon.target,
            snapshot: daemon.snapshot(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmokeDaemonConnection {
    endpoint: SmokeEndpoint,
    target: SmokeTarget,
    pub snapshot: CoworkSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SmokeEndpoint {
    InProcess,
}

impl SmokeEndpoint {
    fn parse(input: String) -> Result<Self, CoworkDaemonSmokeError> {
        if input == SUPPORTED_ENDPOINT {
            Ok(Self::InProcess)
        } else {
            Err(CoworkDaemonSmokeError::UnsupportedEndpoint { endpoint: input })
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::InProcess => SUPPORTED_ENDPOINT,
        }
    }
}

impl fmt::Display for SmokeEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SmokeTarget {
    LocalSmoke,
}

impl SmokeTarget {
    fn parse(input: String) -> Result<Self, CoworkDaemonSmokeError> {
        if input == SUPPORTED_TARGET {
            Ok(Self::LocalSmoke)
        } else {
            Err(CoworkDaemonSmokeError::UnsupportedTarget { target: input })
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::LocalSmoke => SUPPORTED_TARGET,
        }
    }
}

impl fmt::Display for SmokeTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalSmokeDaemon {
    endpoint: SmokeEndpoint,
    target: SmokeTarget,
}

impl LocalSmokeDaemon {
    fn start(endpoint: SmokeEndpoint, target: SmokeTarget) -> Self {
        Self { endpoint, target }
    }

    fn snapshot(&self) -> CoworkSnapshot {
        let objective_id = "cowork-smoke-objective".to_string();
        let task_id = "cowork-smoke-connect".to_string();
        let evidence_id = "cowork-smoke-evidence".to_string();
        let checkpoint_id = "cowork-smoke-checkpoint".to_string();
        let artifact_id = "cowork-smoke-snapshot".to_string();

        CoworkSnapshot {
            protocol_version: ProtocolVersion::current(),
            authority: AuthorityInvariant::daemon_authoritative(),
            objective: Objective {
                id: objective_id.clone(),
                instruction: "Smoke-test the Cowork daemon protocol boundary.".to_string(),
                constraints: vec![
                    "Use the deterministic in-process CLI smoke daemon.".to_string(),
                    "Do not start a persistent service or background process.".to_string(),
                ],
                acceptance_criteria: vec![
                    "CLI clients can start and connect to the smoke daemon.".to_string(),
                    "Status and export return daemon-authored protocol state.".to_string(),
                ],
                status: ObjectiveStatus::Active,
                created_at: Some("2026-05-21T00:00:00Z".to_string()),
                metadata: self.metadata(),
            },
            tasks: vec![Task {
                id: task_id.clone(),
                objective_id: objective_id.clone(),
                title: "Establish smoke daemon connection".to_string(),
                status: TaskStatus::InProgress,
                dependencies: Vec::new(),
                evidence_ids: vec![evidence_id.clone()],
                artifact_ids: vec![artifact_id.clone()],
                assigned_subagent_id: None,
                metadata: self.metadata(),
            }],
            evidence: vec![Evidence {
                id: evidence_id.clone(),
                kind: EvidenceKind::ManualRuntime,
                summary: "In-process smoke daemon started and returned a protocol snapshot."
                    .to_string(),
                task_id: Some(task_id.clone()),
                checkpoint_id: Some(checkpoint_id.clone()),
                artifact_ids: vec![artifact_id.clone()],
                created_at: Some("2026-05-21T00:01:00Z".to_string()),
                metadata: self.metadata(),
            }],
            checkpoints: vec![Checkpoint {
                id: checkpoint_id,
                objective_id: objective_id.clone(),
                sequence: 1,
                summary: "Smoke daemon protocol boundary is reachable.".to_string(),
                task_ids: vec![task_id.clone()],
                evidence_ids: vec![evidence_id.clone()],
                artifact_ids: vec![artifact_id.clone()],
                created_at: Some("2026-05-21T00:02:00Z".to_string()),
                metadata: self.metadata(),
            }],
            artifacts: vec![Artifact {
                id: artifact_id,
                kind: ArtifactKind::Json,
                title: "Cowork smoke daemon snapshot".to_string(),
                uri: None,
                evidence_ids: vec![evidence_id],
                provenance: ArtifactProvenance {
                    objective_id,
                    task_id: Some(task_id),
                    evidence_id: Some("cowork-smoke-evidence".to_string()),
                },
                metadata: self.metadata(),
            }],
            subagents: Vec::new(),
            providers: Vec::new(),
            browser: BrowserState {
                sessions: Vec::new(),
                active_session_id: None,
                status: BrowserStatus::Unavailable,
                metadata: self.metadata(),
            },
            run_status: RunStatus::Running,
            metadata: self.metadata(),
        }
    }

    fn metadata(&self) -> Metadata {
        let mut metadata = Metadata::new();
        metadata.insert(
            "endpoint".to_string(),
            serde_json::Value::String(self.endpoint.as_str().to_string()),
        );
        metadata.insert(
            "target".to_string(),
            serde_json::Value::String(self.target.as_str().to_string()),
        );
        metadata.insert(
            "mode".to_string(),
            serde_json::Value::String("cli_smoke".to_string()),
        );
        metadata
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoworkDaemonSmokeError {
    UnsupportedEndpoint { endpoint: String },
    UnsupportedTarget { target: String },
    NotStarted,
    Serialization { message: String },
}

impl CoworkDaemonSmokeError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedEndpoint { .. } => "unsupported_endpoint",
            Self::UnsupportedTarget { .. } => "unsupported_target",
            Self::NotStarted => "daemon_not_started",
            Self::Serialization { .. } => "serialization_failed",
        }
    }

    pub fn protocol_error(&self) -> ProtocolError {
        match self {
            Self::UnsupportedEndpoint { endpoint } => ProtocolError::InvalidRequest {
                message: format!(
                    "unsupported Cowork smoke daemon endpoint '{endpoint}'; supported endpoint is '{SUPPORTED_ENDPOINT}'"
                ),
            },
            Self::UnsupportedTarget { target } => ProtocolError::InvalidRequest {
                message: format!(
                    "unsupported Cowork smoke daemon target '{target}'; supported target is '{SUPPORTED_TARGET}'"
                ),
            },
            Self::NotStarted => ProtocolError::Conflict {
                message: "Cowork smoke daemon has not been started".to_string(),
            },
            Self::Serialization { message } => ProtocolError::Internal {
                message: message.clone(),
            },
        }
    }

    fn serialization(error: serde_json::Error) -> Self {
        Self::Serialization {
            message: error.to_string(),
        }
    }

    fn report(&self) -> DaemonErrorReport {
        let protocol_error = self.protocol_error();
        DaemonErrorReport {
            code: self.code(),
            protocol_error_code: protocol_error.code(),
            message: self.to_string(),
            protocol_error,
        }
    }
}

impl fmt::Display for CoworkDaemonSmokeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedEndpoint { endpoint } => write!(
                formatter,
                "unsupported Cowork smoke daemon endpoint '{endpoint}'"
            ),
            Self::UnsupportedTarget { target } => {
                write!(
                    formatter,
                    "unsupported Cowork smoke daemon target '{target}'"
                )
            }
            Self::NotStarted => formatter.write_str("Cowork smoke daemon has not been started"),
            Self::Serialization { message } => {
                write!(
                    formatter,
                    "failed to serialize Cowork smoke daemon output: {message}"
                )
            }
        }
    }
}

impl Error for CoworkDaemonSmokeError {}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DaemonErrorReport {
    code: &'static str,
    protocol_error_code: ProtocolErrorCode,
    message: String,
    protocol_error: ProtocolError,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DaemonStartOutput {
    endpoint: String,
    target: String,
    connected: bool,
    state: ProtocolEnvelope<CoworkSnapshot>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DaemonStatusOutput {
    endpoint: String,
    target: String,
    connected: bool,
    protocol_version: ProtocolVersion,
    run_status: RunStatus,
    objective_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_then_connect_returns_protocol_snapshot() {
        let request = SmokeDaemonRequest::default();
        let mut controller = SmokeDaemonController::default();

        let started = controller.start(request.clone()).expect("start daemon");
        assert_eq!(started.run_status, RunStatus::Running);
        assert_eq!(started.protocol_version, ProtocolVersion::current());

        let connection = controller.connect(request).expect("connect daemon");
        assert_eq!(connection.snapshot.run_status, RunStatus::Running);
        assert_eq!(
            connection.snapshot.authority,
            AuthorityInvariant::daemon_authoritative()
        );
        assert_eq!(connection.snapshot.objective.id, "cowork-smoke-objective");
    }

    #[test]
    fn status_and_export_outputs_are_snapshot_based() {
        let status = status_output(DaemonSmokeOptions::default()).expect("status output");
        assert!(status.connected);
        assert_eq!(status.endpoint, SUPPORTED_ENDPOINT);
        assert_eq!(status.target, SUPPORTED_TARGET);
        assert_eq!(status.run_status, RunStatus::Running);
        assert_eq!(status.protocol_version, ProtocolVersion::current());

        let snapshot = export_snapshot(DaemonSmokeOptions::default()).expect("export snapshot");
        assert_eq!(snapshot.run_status, status.run_status);
        assert_eq!(snapshot.objective.id, status.objective_id);
    }

    #[test]
    fn run_action_serializes_start_status_and_export() {
        let start_json =
            run_action(DaemonAction::Start(DaemonSmokeOptions::default())).expect("start json");
        let start_value: serde_json::Value =
            serde_json::from_str(&start_json).expect("parse start json");
        assert_eq!(start_value["connected"], true);
        assert_eq!(start_value["state"]["kind"], "response");
        assert_eq!(
            start_value["state"]["result"]["runStatus"],
            serde_json::Value::String("running".to_string())
        );

        let status_json =
            run_action(DaemonAction::Status(DaemonSmokeOptions::default())).expect("status json");
        let status_value: serde_json::Value =
            serde_json::from_str(&status_json).expect("parse status json");
        assert_eq!(status_value["runStatus"], "running");

        let export_json =
            run_action(DaemonAction::Export(DaemonSmokeOptions::default())).expect("export json");
        let export_value: serde_json::Value =
            serde_json::from_str(&export_json).expect("parse export json");
        assert_eq!(export_value["runStatus"], "running");
        assert_eq!(export_value["protocolVersion"]["major"], 1);
    }

    #[test]
    fn unsupported_endpoint_and_target_have_stable_error_codes() {
        let endpoint_error =
            SmokeDaemonRequest::new("tcp://127.0.0.1:4040", SUPPORTED_TARGET).unwrap_err();
        assert!(matches!(
            endpoint_error,
            CoworkDaemonSmokeError::UnsupportedEndpoint { .. }
        ));
        assert_eq!(endpoint_error.code(), "unsupported_endpoint");
        assert_eq!(
            endpoint_error.protocol_error().code(),
            ProtocolErrorCode::InvalidRequest
        );

        let target_error = SmokeDaemonRequest::new(SUPPORTED_ENDPOINT, "production").unwrap_err();
        assert!(matches!(
            target_error,
            CoworkDaemonSmokeError::UnsupportedTarget { .. }
        ));
        assert_eq!(target_error.code(), "unsupported_target");
        assert_eq!(
            target_error.protocol_error().code(),
            ProtocolErrorCode::InvalidRequest
        );
    }
}
