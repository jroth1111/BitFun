use clap::{Args, Subcommand};
use cowork_protocol::{
    Artifact, ArtifactKind, ArtifactProvenance, AuthorityInvariant, BrowserAdapter, BrowserSession,
    BrowserState, BrowserStatus, BrowserTab, Checkpoint, CoworkSnapshot, Evidence, EvidenceKind,
    Metadata, Objective, ObjectiveStatus, ProtocolEnvelope, ProtocolError, ProtocolErrorCode,
    ProtocolVersion, Provider, ProviderCapability, ProviderKind, ProviderStatus, RunStatus,
    Subagent, SubagentStatus, Task, TaskStatus,
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
    /// Export the deterministic CoworkSnapshot smoke state as JSON or Markdown
    Export(DaemonExportOptions),
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

#[derive(Debug, Clone, Args, PartialEq, Eq)]
pub struct DaemonExportOptions {
    #[command(flatten)]
    pub smoke: DaemonSmokeOptions,

    /// Export format. Supported values: json, markdown.
    #[arg(long, default_value = "json")]
    pub format: String,
}

impl Default for DaemonExportOptions {
    fn default() -> Self {
        Self {
            smoke: DaemonSmokeOptions::default(),
            format: DaemonExportFormat::Json.as_str().to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DaemonExportFormat {
    Json,
    Markdown,
}

impl DaemonExportFormat {
    fn parse(input: &str) -> Result<Self, CoworkDaemonSmokeError> {
        match input {
            "json" => Ok(Self::Json),
            "markdown" | "md" => Ok(Self::Markdown),
            format => Err(CoworkDaemonSmokeError::UnsupportedExportFormat {
                format: format.to_string(),
            }),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Markdown => "markdown",
        }
    }
}

pub fn run_action(action: DaemonAction) -> Result<String, CoworkDaemonSmokeError> {
    match action {
        DaemonAction::Start(options) => to_pretty_json(&start_output(options)?),
        DaemonAction::Status(options) => to_pretty_json(&status_output(options)?),
        DaemonAction::Export(options) => export_output(options),
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

fn export_output(options: DaemonExportOptions) -> Result<String, CoworkDaemonSmokeError> {
    let format = DaemonExportFormat::parse(&options.format)?;
    let snapshot = export_snapshot(options.smoke)?;

    match format {
        DaemonExportFormat::Json => to_pretty_json(&snapshot),
        DaemonExportFormat::Markdown => Ok(render_snapshot_markdown(&snapshot)),
    }
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

fn render_snapshot_markdown(snapshot: &CoworkSnapshot) -> String {
    let mut markdown = String::new();

    markdown.push_str("# Cowork Run Ledger Export\n\n");
    markdown.push_str("## Run\n\n");
    markdown.push_str(&format!(
        "- Status: `{}`\n- Protocol: `{}.{}.{}`\n- Authority: daemon-authored\n\n",
        json_label(&snapshot.run_status),
        snapshot.protocol_version.major,
        snapshot.protocol_version.minor,
        snapshot.protocol_version.patch
    ));

    markdown.push_str("## Objective\n\n");
    markdown.push_str(&format!(
        "- ID: `{}`\n- Status: `{}`\n- Instruction: {}\n",
        escape_inline(&snapshot.objective.id),
        json_label(&snapshot.objective.status),
        escape_inline(&snapshot.objective.instruction)
    ));
    if !snapshot.objective.constraints.is_empty() {
        markdown.push_str("- Constraints:\n");
        for constraint in &snapshot.objective.constraints {
            markdown.push_str(&format!("  - {}\n", escape_inline(constraint)));
        }
    }
    if !snapshot.objective.acceptance_criteria.is_empty() {
        markdown.push_str("- Acceptance criteria:\n");
        for criterion in &snapshot.objective.acceptance_criteria {
            markdown.push_str(&format!("  - {}\n", escape_inline(criterion)));
        }
    }
    markdown.push('\n');

    render_tasks(&mut markdown, &snapshot.tasks);
    render_evidence(&mut markdown, &snapshot.evidence);
    render_checkpoints(&mut markdown, &snapshot.checkpoints);
    render_artifacts(&mut markdown, &snapshot.artifacts);
    render_browser(&mut markdown, &snapshot.browser);
    render_providers(&mut markdown, &snapshot.providers);
    render_subagents(&mut markdown, &snapshot.subagents);

    markdown
}

fn render_tasks(markdown: &mut String, tasks: &[Task]) {
    markdown.push_str("## Tasks\n\n");
    if tasks.is_empty() {
        markdown.push_str("_No tasks recorded._\n\n");
        return;
    }

    markdown.push_str("| ID | Status | Title | Evidence | Artifacts | Subagent |\n");
    markdown.push_str("| --- | --- | --- | --- | --- | --- |\n");
    for task in tasks {
        markdown.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            escape_cell(&task.id),
            escape_cell(json_label(&task.status)),
            escape_cell(&task.title),
            escape_cell(join_or_none(&task.evidence_ids)),
            escape_cell(join_or_none(&task.artifact_ids)),
            escape_cell(task.assigned_subagent_id.as_deref().unwrap_or("none"))
        ));
    }
    markdown.push('\n');
}

fn render_evidence(markdown: &mut String, evidence: &[Evidence]) {
    markdown.push_str("## Evidence\n\n");
    if evidence.is_empty() {
        markdown.push_str("_No evidence recorded._\n\n");
        return;
    }

    markdown.push_str("| ID | Kind | Task | Checkpoint | Summary |\n");
    markdown.push_str("| --- | --- | --- | --- | --- |\n");
    for item in evidence {
        markdown.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            escape_cell(&item.id),
            escape_cell(json_label(&item.kind)),
            escape_cell(item.task_id.as_deref().unwrap_or("none")),
            escape_cell(item.checkpoint_id.as_deref().unwrap_or("none")),
            escape_cell(&item.summary)
        ));
    }
    markdown.push('\n');
}

fn render_checkpoints(markdown: &mut String, checkpoints: &[Checkpoint]) {
    markdown.push_str("## Checkpoints\n\n");
    if checkpoints.is_empty() {
        markdown.push_str("_No checkpoints recorded._\n\n");
        return;
    }

    markdown.push_str("| Sequence | ID | Summary | Tasks | Evidence | Artifacts |\n");
    markdown.push_str("| --- | --- | --- | --- | --- | --- |\n");
    for checkpoint in checkpoints {
        markdown.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            checkpoint.sequence,
            escape_cell(&checkpoint.id),
            escape_cell(&checkpoint.summary),
            escape_cell(join_or_none(&checkpoint.task_ids)),
            escape_cell(join_or_none(&checkpoint.evidence_ids)),
            escape_cell(join_or_none(&checkpoint.artifact_ids))
        ));
    }
    markdown.push('\n');
}

fn render_artifacts(markdown: &mut String, artifacts: &[Artifact]) {
    markdown.push_str("## Artifacts\n\n");
    if artifacts.is_empty() {
        markdown.push_str("_No artifacts recorded._\n\n");
        return;
    }

    markdown.push_str("| ID | Kind | Title | URI | Evidence |\n");
    markdown.push_str("| --- | --- | --- | --- | --- |\n");
    for artifact in artifacts {
        markdown.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            escape_cell(&artifact.id),
            escape_cell(json_label(&artifact.kind)),
            escape_cell(&artifact.title),
            escape_cell(artifact.uri.as_deref().unwrap_or("none")),
            escape_cell(join_or_none(&artifact.evidence_ids))
        ));
    }
    markdown.push('\n');
}

fn render_browser(markdown: &mut String, browser: &BrowserState) {
    markdown.push_str("## Browser\n\n");
    markdown.push_str(&format!(
        "- Status: `{}`\n- Active session: `{}`\n\n",
        json_label(&browser.status),
        escape_inline(browser.active_session_id.as_deref().unwrap_or("none"))
    ));

    if browser.sessions.is_empty() {
        markdown.push_str("_No browser sessions recorded._\n\n");
        return;
    }

    markdown.push_str("| Session | Adapter | Status | Active tab | Tabs |\n");
    markdown.push_str("| --- | --- | --- | --- | --- |\n");
    for session in &browser.sessions {
        let tabs = session
            .tabs
            .iter()
            .map(|tab| {
                format!(
                    "{} ({})",
                    tab.id,
                    tab.url
                        .as_deref()
                        .unwrap_or(tab.title.as_deref().unwrap_or("untitled"))
                )
            })
            .collect::<Vec<_>>();
        markdown.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            escape_cell(&session.id),
            escape_cell(json_label(&session.adapter)),
            escape_cell(json_label(&session.status)),
            escape_cell(session.active_tab_id.as_deref().unwrap_or("none")),
            escape_cell(join_or_none(&tabs))
        ));
    }
    markdown.push('\n');
}

fn render_providers(markdown: &mut String, providers: &[Provider]) {
    markdown.push_str("## Providers\n\n");
    if providers.is_empty() {
        markdown.push_str("_No providers recorded._\n\n");
        return;
    }

    markdown.push_str("| ID | Kind | Label | Status | Model | Capabilities |\n");
    markdown.push_str("| --- | --- | --- | --- | --- | --- |\n");
    for provider in providers {
        let capabilities = provider
            .capabilities
            .iter()
            .map(json_label)
            .collect::<Vec<_>>();
        markdown.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            escape_cell(&provider.id),
            escape_cell(json_label(&provider.kind)),
            escape_cell(&provider.label),
            escape_cell(json_label(&provider.status)),
            escape_cell(provider.selected_model.as_deref().unwrap_or("none")),
            escape_cell(join_or_none(&capabilities))
        ));
    }
    markdown.push('\n');
}

fn render_subagents(markdown: &mut String, subagents: &[Subagent]) {
    markdown.push_str("## Subagents\n\n");
    if subagents.is_empty() {
        markdown.push_str("_No subagents recorded._\n\n");
        return;
    }

    markdown.push_str("| ID | Name | Role | Status | Parent task | Report evidence |\n");
    markdown.push_str("| --- | --- | --- | --- | --- | --- |\n");
    for subagent in subagents {
        markdown.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            escape_cell(&subagent.id),
            escape_cell(&subagent.name),
            escape_cell(&subagent.role),
            escape_cell(json_label(&subagent.status)),
            escape_cell(subagent.parent_task_id.as_deref().unwrap_or("none")),
            escape_cell(subagent.report_evidence_id.as_deref().unwrap_or("none"))
        ));
    }
    markdown.push('\n');
}

fn json_label<T: Serialize>(value: T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(label)) => label,
        Ok(other) => other.to_string(),
        Err(_) => "unknown".to_string(),
    }
}

fn join_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "none".to_string()
    } else {
        values.join(", ")
    }
}

fn escape_inline(value: &str) -> String {
    value.replace('\n', " ")
}

fn escape_cell(value: impl AsRef<str>) -> String {
    value
        .as_ref()
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('\n', "<br>")
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
        let subagent_id = "cowork-smoke-worker".to_string();
        let provider_id = "cowork-smoke-provider".to_string();
        let browser_session_id = "cowork-smoke-browser".to_string();
        let browser_tab_id = "cowork-smoke-tab".to_string();

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
                assigned_subagent_id: Some(subagent_id.clone()),
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
                evidence_ids: vec![evidence_id.clone()],
                provenance: ArtifactProvenance {
                    objective_id,
                    task_id: Some(task_id.clone()),
                    evidence_id: Some("cowork-smoke-evidence".to_string()),
                },
                metadata: self.metadata(),
            }],
            subagents: vec![Subagent {
                id: subagent_id.clone(),
                name: "smoke-worker".to_string(),
                role: "Exercise the deterministic daemon export boundary.".to_string(),
                status: SubagentStatus::Running,
                parent_task_id: Some(task_id.clone()),
                report_evidence_id: Some(evidence_id.clone()),
                allowed_tools: vec!["daemon.status".to_string(), "daemon.export".to_string()],
                denied_tools: Vec::new(),
                metadata: self.metadata(),
            }],
            providers: vec![Provider {
                id: provider_id,
                kind: ProviderKind::OpenAiCompatible,
                label: "smoke provider".to_string(),
                status: ProviderStatus::Available,
                selected_model: Some("cowork-smoke-model".to_string()),
                capabilities: vec![ProviderCapability::Text, ProviderCapability::ToolUse],
                credential_ref: None,
                metadata: self.metadata(),
            }],
            browser: BrowserState {
                sessions: vec![BrowserSession {
                    id: browser_session_id.clone(),
                    adapter: BrowserAdapter::ManagedProfileCdp,
                    status: BrowserStatus::Running,
                    profile_id: Some("cowork-smoke-profile".to_string()),
                    active_tab_id: Some(browser_tab_id.clone()),
                    tabs: vec![BrowserTab {
                        id: browser_tab_id,
                        title: Some("Cowork smoke".to_string()),
                        url: Some("https://example.test/cowork-smoke".to_string()),
                        task_id: Some(task_id),
                    }],
                }],
                active_session_id: Some(browser_session_id),
                status: BrowserStatus::Running,
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
    UnsupportedExportFormat { format: String },
    NotStarted,
    Serialization { message: String },
}

impl CoworkDaemonSmokeError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedEndpoint { .. } => "unsupported_endpoint",
            Self::UnsupportedTarget { .. } => "unsupported_target",
            Self::UnsupportedExportFormat { .. } => "unsupported_export_format",
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
            Self::UnsupportedExportFormat { format } => ProtocolError::InvalidRequest {
                message: format!(
                    "unsupported Cowork daemon export format '{format}'; supported formats are 'json' and 'markdown'"
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
            Self::UnsupportedExportFormat { format } => {
                write!(
                    formatter,
                    "unsupported Cowork daemon export format '{format}'"
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
        assert_eq!(connection.snapshot.subagents.len(), 1);
        assert_eq!(connection.snapshot.providers.len(), 1);
        assert_eq!(connection.snapshot.browser.sessions.len(), 1);
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
            run_action(DaemonAction::Export(DaemonExportOptions::default())).expect("export json");
        let export_value: serde_json::Value =
            serde_json::from_str(&export_json).expect("parse export json");
        assert_eq!(export_value["runStatus"], "running");
        assert_eq!(export_value["protocolVersion"]["major"], 1);
        assert_eq!(export_value["subagents"][0]["id"], "cowork-smoke-worker");
        assert_eq!(export_value["providers"][0]["id"], "cowork-smoke-provider");
        assert_eq!(
            export_value["browser"]["sessions"][0]["id"],
            "cowork-smoke-browser"
        );
    }

    #[test]
    fn export_json_format_is_explicit_and_default_stable() {
        let default_json =
            run_action(DaemonAction::Export(DaemonExportOptions::default())).expect("export json");
        let explicit_json = run_action(DaemonAction::Export(DaemonExportOptions {
            smoke: DaemonSmokeOptions::default(),
            format: "json".to_string(),
        }))
        .expect("explicit export json");

        let default_value: serde_json::Value =
            serde_json::from_str(&default_json).expect("parse default json");
        let explicit_value: serde_json::Value =
            serde_json::from_str(&explicit_json).expect("parse explicit json");
        assert_eq!(default_value, explicit_value);
    }

    #[test]
    fn export_markdown_includes_all_ledger_sections() {
        let markdown = run_action(DaemonAction::Export(DaemonExportOptions {
            smoke: DaemonSmokeOptions::default(),
            format: "markdown".to_string(),
        }))
        .expect("export markdown");

        for heading in [
            "# Cowork Run Ledger Export",
            "## Run",
            "## Objective",
            "## Tasks",
            "## Evidence",
            "## Checkpoints",
            "## Artifacts",
            "## Browser",
            "## Providers",
            "## Subagents",
        ] {
            assert!(markdown.contains(heading), "missing heading {heading}");
        }

        assert!(markdown.contains("cowork-smoke-objective"));
        assert!(markdown.contains("cowork-smoke-worker"));
        assert!(markdown.contains("cowork-smoke-provider"));
        assert!(markdown.contains("cowork-smoke-browser"));
    }

    #[test]
    fn export_restarts_to_same_snapshot() {
        let request = SmokeDaemonRequest::default();
        let mut first_controller = SmokeDaemonController::default();
        first_controller
            .start(request.clone())
            .expect("first start");
        let first = first_controller
            .connect(request.clone())
            .expect("first connect")
            .snapshot;

        let mut second_controller = SmokeDaemonController::default();
        second_controller
            .start(request.clone())
            .expect("second start");
        let second = second_controller
            .connect(request)
            .expect("second connect")
            .snapshot;

        assert_eq!(first, second);
        assert_eq!(
            to_pretty_json(&first).expect("first json"),
            to_pretty_json(&second).expect("second json")
        );
    }

    #[test]
    fn unsupported_export_format_has_stable_error_code() {
        let error = run_action(DaemonAction::Export(DaemonExportOptions {
            smoke: DaemonSmokeOptions::default(),
            format: "yaml".to_string(),
        }))
        .unwrap_err();

        assert!(matches!(
            error,
            CoworkDaemonSmokeError::UnsupportedExportFormat { .. }
        ));
        assert_eq!(error.code(), "unsupported_export_format");
        assert_eq!(
            error.protocol_error().code(),
            ProtocolErrorCode::InvalidRequest
        );
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
