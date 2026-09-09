mod adapter;
mod connections;
mod deliverable;
mod source_checkpoint;
mod verifier;
mod workspace;

use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chrono::{DateTime, Utc};
use clap::Parser;
use crony_domain::{DeliverableSpec, VerificationPolicy};
use crony_protocol::{
    ActiveRunClaim, MAX_VERIFICATION_ARTIFACT_BYTES, ResolvedSecret, RunnerCapability, RunnerModel,
    RunnerToServer, ServerToRunner, VerificationArtifactReference,
};
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Digest;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, watch},
};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};
use url::Url;
use uuid::Uuid;

use adapter::{
    AdapterArtifact, AdapterControl, AdapterEvent, AdapterEventSink, AdapterExit, AdapterRegistry,
    AdapterRegistryConfig, AdapterRunRequest, AgentAdapter, CopilotSdkConfig,
};
use workspace::{
    VerificationSnapshot, WorkspaceCleanup, WorkspaceDisposition, WorkspaceLease, WorkspaceManager,
};

#[derive(Debug, Parser, Clone)]
#[command(name = "crony-runner")]
struct Args {
    #[arg(
        long,
        env = "CRONY_SERVER_WS",
        default_value = "ws://127.0.0.1:8791/ws/runner"
    )]
    server_ws: String,

    #[arg(long, env = "CRONY_RUNNER_ID", default_value = "runner-local")]
    runner_id: String,

    #[arg(long, env = "CRONY_CORP_ID")]
    corp_id: Uuid,

    #[arg(
        long,
        env = "CRONY_RUNNER_CREDENTIAL_FILE",
        default_value = "./output/runner/credential.json"
    )]
    credential_file: PathBuf,

    #[arg(long, env = "CRONY_RUNNER_ENROLLMENT_TOKEN_FILE")]
    enrollment_token_file: Option<PathBuf>,

    #[arg(
        long,
        env = "CRONY_RUNNER_WORKSPACE",
        default_value = "./output/runner"
    )]
    workspace: PathBuf,

    #[arg(long, env = "CRONY_SOURCE_REPOSITORY", default_value = ".")]
    source_repository: PathBuf,

    #[arg(long, env = "CRONY_SOURCE_BASE_REF", default_value = "HEAD")]
    source_base_ref: String,

    #[arg(
        long,
        env = "CRONY_FAKE_AGENT_SCRIPT",
        default_value = "./scripts/fake-agent.mjs"
    )]
    fake_agent_script: PathBuf,

    #[arg(long, env = "CRONY_CODEX_COMMAND")]
    codex_command: Option<PathBuf>,

    #[arg(
        long = "codex-command-arg",
        env = "CRONY_CODEX_COMMAND_ARGS",
        value_delimiter = ';'
    )]
    codex_command_args: Vec<std::ffi::OsString>,

    #[arg(long, env = "CRONY_CLAUDE_COMMAND")]
    claude_command: Option<PathBuf>,

    #[arg(
        long = "claude-command-arg",
        env = "CRONY_CLAUDE_COMMAND_ARGS",
        value_delimiter = ';'
    )]
    claude_command_args: Vec<std::ffi::OsString>,

    #[arg(long, env = "CRONY_OPENCODE_COMMAND")]
    opencode_command: Option<PathBuf>,

    #[arg(
        long = "opencode-command-arg",
        env = "CRONY_OPENCODE_COMMAND_ARGS",
        value_delimiter = ';'
    )]
    opencode_command_args: Vec<std::ffi::OsString>,

    #[arg(long, env = "CRONY_COPILOT_RUNTIME_URL")]
    copilot_runtime_url: Option<String>,

    #[arg(long, env = "CRONY_COPILOT_CLI_PATH")]
    copilot_cli_path: Option<PathBuf>,

    #[arg(
        long = "copilot-cli-prefix-arg",
        env = "CRONY_COPILOT_CLI_PREFIX_ARGS",
        value_delimiter = ';'
    )]
    copilot_cli_prefix_args: Vec<std::ffi::OsString>,

    #[arg(long, env = "CRONY_COPILOT_GITHUB_TOKEN_FILE")]
    copilot_github_token_file: Option<PathBuf>,

    #[arg(long, env = "CRONY_COPILOT_CONNECTION_TOKEN_FILE")]
    copilot_connection_token_file: Option<PathBuf>,

    #[arg(long, env = "CRONY_COPILOT_HOME")]
    copilot_home: Option<PathBuf>,

    #[arg(long, env = "CRONY_COPILOT_USE_LOGGED_IN_USER", default_value_t = true)]
    copilot_use_logged_in_user: bool,

    #[arg(long, env = "CRONY_COPILOT_LOG_LEVEL", default_value = "warning")]
    copilot_log_level: String,

    #[arg(long, env = "CRONY_COPILOT_FIXTURE", default_value_t = false)]
    copilot_fixture: bool,

    #[arg(long, env = "CRONY_CONNECTIONS_DIRECTORY")]
    connections_directory: Option<PathBuf>,

    #[arg(
        long = "repository-root",
        env = "CRONY_REPOSITORY_ROOTS",
        value_delimiter = ';'
    )]
    repository_roots: Vec<PathBuf>,

    #[arg(long, env = "CRONY_GITHUB_COMMAND", default_value = "gh")]
    github_command: PathBuf,
}

#[derive(Debug, Clone)]
struct Assignment {
    corp_id: Uuid,
    connection_epoch: Uuid,
    room_id: Uuid,
    mission_id: Uuid,
    task_id: Uuid,
    run_id: Uuid,
    workspace_run_id: Uuid,
    agent_id: Uuid,
    assignment_token: Uuid,
    adapter: String,
    mission_title: String,
    model: Option<String>,
    reasoning_effort: Option<String>,
    source_repository: Option<String>,
    source_base_ref: Option<String>,
    source_base_commit: Option<String>,
    workspace_connection_id: Option<Uuid>,
    resume_workspace_base_commit: Option<String>,
    verification_policy: VerificationPolicy,
    write_scope: Vec<String>,
    deliverable: Option<DeliverableSpec>,
    secrets: Vec<ResolvedSecret>,
    expected_workspace_fingerprint: Option<String>,
    expected_head_commit: Option<String>,
    provider_artifact: Option<VerificationArtifactReference>,
    checkpoint_verification: bool,
    hard_boundary_checkpoint: Arc<HardBoundaryControl>,
}

impl Assignment {
    fn hard_boundary_requested(&self) -> bool {
        self.hard_boundary_checkpoint.requested()
    }
}

#[derive(Debug)]
struct HardBoundaryControl {
    phase: AtomicU8,
    cancellation: watch::Sender<bool>,
}

impl Default for HardBoundaryControl {
    fn default() -> Self {
        Self {
            phase: AtomicU8::new(Self::OPEN),
            cancellation: watch::channel(false).0,
        }
    }
}

impl HardBoundaryControl {
    const OPEN: u8 = 0;
    const REQUESTED: u8 = 1;
    const FINALIZING: u8 = 2;

    fn request(&self) -> bool {
        match self.phase.compare_exchange(
            Self::OPEN,
            Self::REQUESTED,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) | Err(Self::REQUESTED) => {
                self.cancellation.send_replace(true);
                true
            }
            Err(_) => false,
        }
    }

    fn requested(&self) -> bool {
        self.phase.load(Ordering::Acquire) == Self::REQUESTED
    }

    fn begin_finalization(&self) -> bool {
        self.phase
            .compare_exchange(
                Self::OPEN,
                Self::FINALIZING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct CredentialFile {
    runner_id: String,
    corp_id: Uuid,
    credential: String,
    expires_at: String,
}

#[derive(Clone)]
struct ActiveRunControl {
    assignment_token: Uuid,
    control: mpsc::UnboundedSender<AdapterControl>,
    artifact_ack: mpsc::UnboundedSender<ArtifactAck>,
    hard_boundary_checkpoint: Arc<HardBoundaryControl>,
}

impl ActiveRunControl {
    fn apply_circuit_breaker(&self, stage: String, reason: String) -> bool {
        let hard = matches!(stage.as_str(), "suspend" | "stop");
        if hard {
            // Remember native stop authority before handing it to the adapter.
            // This is retention intent, never authority to resume or complete.
            if !self.hard_boundary_checkpoint.request() {
                // Finalization won the boundary. Never acknowledge retention
                // after cleanup has committed to its native removal decision.
                return false;
            }
        }
        let delivered = self
            .control
            .send(AdapterControl::CircuitBreaker { stage, reason })
            .is_ok();
        // The assignment can still own cancellable verification after the
        // provider's control receiver closes. The hard directive applies there too.
        delivered || hard
    }
}

#[derive(Debug)]
struct ArtifactAck {
    artifact_id: Uuid,
    artifact_role: String,
    sha256: String,
}

struct AssignmentChannels {
    controls: mpsc::UnboundedReceiver<AdapterControl>,
    artifact_acks: mpsc::UnboundedReceiver<ArtifactAck>,
}

type ActiveRuns = Arc<DashMap<Uuid, ActiveRunControl>>;

fn apply_circuit_breaker_command(
    seen_commands: &DashMap<Uuid, ()>,
    active_runs: &ActiveRuns,
    command_id: Uuid,
    run_id: Uuid,
    stage: String,
    reason: String,
) -> (bool, bool) {
    let duplicate = seen_commands.contains_key(&command_id);
    let applied = duplicate
        || active_runs
            .get(&run_id)
            .is_some_and(|active| active.apply_circuit_breaker(stage, reason));
    if applied {
        seen_commands.insert(command_id, ());
    }
    (applied, duplicate)
}

fn default_codex_command() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from("codex.exe")
    } else {
        PathBuf::from("codex")
    }
}

fn default_provider_command(name: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(format!("{name}.exe"))
    } else {
        PathBuf::from(name)
    }
}

fn connections_directory(args: &Args) -> Result<PathBuf> {
    if let Some(directory) = &args.connections_directory {
        return Ok(directory.clone());
    }
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
    }
    .context("configure CRONY_CONNECTIONS_DIRECTORY outside the source repositories")?;
    let namespace = hex::encode(sha2::Sha256::digest(
        format!("{}|{}|{}", args.server_ws, args.corp_id, args.runner_id).as_bytes(),
    ));
    Ok(base
        .join("ECorp")
        .join("connections")
        .join(&namespace[..24]))
}

fn copilot_config(args: &Args) -> Result<CopilotSdkConfig> {
    if args.copilot_runtime_url.is_some() && args.copilot_cli_path.is_some() {
        return Err(anyhow!(
            "configure either CRONY_COPILOT_RUNTIME_URL or CRONY_COPILOT_CLI_PATH, not both"
        ));
    }
    let (external_host, external_port) = if let Some(value) = &args.copilot_runtime_url {
        let url = Url::parse(value).context("parse CRONY_COPILOT_RUNTIME_URL")?;
        let host = url
            .host_str()
            .context("Copilot runtime URL omitted a host")?
            .to_owned();
        let port = url
            .port()
            .context("Copilot runtime URL must include an explicit port")?;
        (Some(host), Some(port))
    } else {
        (None, None)
    };
    Ok(CopilotSdkConfig {
        cli_path: args.copilot_cli_path.clone(),
        cli_prefix_args: args.copilot_cli_prefix_args.clone(),
        external_host,
        external_port,
        github_token_file: args.copilot_github_token_file.clone(),
        connection_token_file: args.copilot_connection_token_file.clone(),
        base_directory: args
            .copilot_home
            .clone()
            .unwrap_or_else(|| args.workspace.join("copilot-home")),
        use_logged_in_user: args.copilot_use_logged_in_user,
        log_level: args.copilot_log_level.clone(),
        fixture: args.copilot_fixture,
    })
}

#[derive(Clone, Default)]
struct OutboundBus {
    state: Arc<Mutex<OutboundState>>,
}

#[derive(Default)]
struct OutboundState {
    connection: Option<mpsc::UnboundedSender<RunnerToServer>>,
    connection_epoch: Option<Uuid>,
    pending: VecDeque<RunnerToServer>,
}

impl OutboundBus {
    fn send_live(&self, mut message: RunnerToServer) -> bool {
        let Ok(state) = self.state.lock() else {
            return false;
        };
        let (Some(connection), Some(epoch)) = (&state.connection, state.connection_epoch) else {
            return false;
        };
        bind_connection_epoch(&mut message, epoch);
        connection.send(message).is_ok()
    }

    fn attach(&self, connection: mpsc::UnboundedSender<RunnerToServer>, connection_epoch: Uuid) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.connection = Some(connection.clone());
        state.connection_epoch = Some(connection_epoch);
        while let Some(mut message) = state.pending.pop_front() {
            bind_connection_epoch(&mut message, connection_epoch);
            if connection.send(message.clone()).is_err() {
                state.pending.push_front(message);
                state.connection = None;
                state.connection_epoch = None;
                break;
            }
        }
    }

    fn detach(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.connection = None;
            state.connection_epoch = None;
        }
    }

    fn send(&self, mut message: RunnerToServer) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if let Some(connection_epoch) = state.connection_epoch {
            bind_connection_epoch(&mut message, connection_epoch);
        }
        let delivered = state
            .connection
            .as_ref()
            .is_some_and(|connection| connection.send(message.clone()).is_ok());
        if delivered {
            return;
        }
        state.connection = None;
        if state.pending.len() == 2_000 {
            warn!(
                buffered_events = state.pending.len(),
                "runner transport is offline; preserving the complete outbound event buffer"
            );
        }
        state.pending.push_back(message);
    }
}

fn bind_connection_epoch(message: &mut RunnerToServer, connection_epoch: Uuid) {
    match message {
        RunnerToServer::RunEvent {
            connection_epoch: event_epoch,
            ..
        }
        | RunnerToServer::CommandAck {
            connection_epoch: event_epoch,
            ..
        }
        | RunnerToServer::WorkspaceSetupReport {
            connection_epoch: event_epoch,
            ..
        }
        | RunnerToServer::WorkspaceSignInAck {
            connection_epoch: event_epoch,
            ..
        }
        | RunnerToServer::CapabilitiesUpdated {
            connection_epoch: event_epoch,
            ..
        } => *event_epoch = connection_epoch,
        _ => {}
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "crony_runner=info".into()),
        )
        .init();
    let args = Args::parse();
    let active_runs: ActiveRuns = Arc::new(DashMap::new());
    let seen_commands = Arc::new(DashMap::<Uuid, ()>::new());
    let outbound = OutboundBus::default();
    let workspaces = Arc::new(
        WorkspaceManager::initialize(
            args.workspace.clone(),
            args.source_repository.clone(),
            args.source_base_ref.clone(),
        )
        .await?,
    );
    let codex_command = args
        .codex_command
        .clone()
        .unwrap_or_else(default_codex_command);
    let copilot_config = copilot_config(&args)?;
    let adapter_config = AdapterRegistryConfig {
        fake_agent_script: args.fake_agent_script.clone(),
        codex_command,
        codex_prefix_args: args.codex_command_args.clone(),
        claude_command: args
            .claude_command
            .clone()
            .unwrap_or_else(|| default_provider_command("claude")),
        claude_prefix_args: args.claude_command_args.clone(),
        opencode_command: args
            .opencode_command
            .clone()
            .unwrap_or_else(|| default_provider_command("opencode")),
        opencode_prefix_args: args.opencode_command_args.clone(),
        copilot: copilot_config,
    };
    let adapters = Arc::new(AdapterRegistry::new(adapter_config.clone()));
    let connection_root = connections_directory(&args)?;
    let connection_manager = match connections::ConnectionManager::new(
        connections::ConnectionManagerConfig {
            root: connection_root,
            corp_id: args.corp_id,
            runner_id: args.runner_id.clone(),
            default_workspaces: workspaces.clone(),
            default_adapters: adapters.clone(),
            adapter_config,
            github_command: args.github_command.clone(),
            allowed_local_roots: if args.repository_roots.is_empty() {
                vec![args.source_repository.clone()]
            } else {
                args.repository_roots.clone()
            },
        },
    )
    .await
    {
        Ok(manager) => Some(Arc::new(manager)),
        Err(error) => {
            warn!(%error, "new project setup needs operator-private storage; existing runner work remains available");
            None
        }
    };
    loop {
        let delay = match run_connection(
            args.clone(),
            active_runs.clone(),
            seen_commands.clone(),
            outbound.clone(),
            adapters.clone(),
            workspaces.clone(),
            connection_manager.clone(),
        )
        .await
        {
            Ok(delay) => delay,
            Err(error) => {
                warn!(%error, "runner connection ended; retrying");
                Duration::from_secs(2)
            }
        };
        tokio::time::sleep(delay).await;
    }
}

async fn run_connection(
    args: Args,
    active_runs: ActiveRuns,
    seen_commands: Arc<DashMap<Uuid, ()>>,
    outbound: OutboundBus,
    adapters: Arc<AdapterRegistry>,
    workspaces: Arc<WorkspaceManager>,
    connection_manager: Option<Arc<connections::ConnectionManager>>,
) -> Result<Duration> {
    let credential = load_runner_credential(&args).await?;
    let (socket, _) = connect_async(&args.server_ws)
        .await
        .with_context(|| format!("connect to {}", args.server_ws))?;
    let (mut socket_tx, mut socket_rx) = socket.split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<RunnerToServer>();
    let connection_epoch = Uuid::new_v4();

    let writer = tokio::spawn(async move {
        while let Some(message) = out_rx.recv().await {
            let text = match serde_json::to_string(&message) {
                Ok(text) => text,
                Err(error) => {
                    error!(%error, "failed to serialize runner message");
                    continue;
                }
            };
            if socket_tx.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    let hostname = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown-host".to_owned());
    let claims = active_run_claims(&active_runs);
    let mut capabilities = Vec::new();
    for adapter in adapters.all() {
        let adapter_capabilities = adapter.capabilities();
        let mut available = adapter_capabilities.spawn.supported();
        let mut detail = format!(
            "{}; {}",
            adapter.display_name(),
            adapter_capabilities.summary()
        );
        let models = if available {
            match adapter.list_models().await {
                Ok(models) => models
                    .into_iter()
                    .map(|model| RunnerModel {
                        id: model.id,
                        name: model.name,
                        policy_state: model.policy_state,
                        policy_terms: model.policy_terms,
                        supports_vision: model.supports_vision,
                        supports_reasoning_effort: model.supports_reasoning_effort,
                        max_prompt_tokens: model.max_prompt_tokens,
                        max_context_window_tokens: model.max_context_window_tokens,
                        supported_reasoning_efforts: model.supported_reasoning_efforts,
                        default_reasoning_effort: model.default_reasoning_effort,
                        billing_multiplier: model.billing_multiplier,
                    })
                    .collect(),
                Err(error) => {
                    available = false;
                    detail.push_str(&format!("; model discovery failed: {error}"));
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        capabilities.push(RunnerCapability {
            workspace_connection_id: None,
            name: adapter.id().to_owned(),
            available,
            detail: Some(detail),
            models,
            source_repository: None,
            source_base_ref: None,
            source_base_commit: None,
        });
    }
    capabilities.push(RunnerCapability {
        workspace_connection_id: None,
        name: "workspace-isolation".to_owned(),
        available: true,
        detail: Some(format!(
            "root={}; repository={}; remote={}; base={}; commit={}",
            workspaces.root().display(),
            workspaces.repository().display(),
            workspaces.repository_identity().unwrap_or("unidentified"),
            workspaces.base_ref(),
            workspaces.base_commit()
        )),
        models: Vec::new(),
        source_repository: workspaces.repository_identity().map(str::to_owned),
        source_base_ref: Some(workspaces.base_ref().to_owned()),
        source_base_commit: Some(workspaces.base_commit().to_owned()),
    });
    capabilities.push(RunnerCapability {
        workspace_connection_id: None,
        name: "durable-control-v1".to_owned(),
        available: true,
        detail: Some(
            "control messages use command ids, runner deduplication, and explicit acknowledgment"
                .to_owned(),
        ),
        models: Vec::new(),
        source_repository: None,
        source_base_ref: None,
        source_base_commit: None,
    });
    capabilities.push(RunnerCapability {
        workspace_connection_id: None,
        name: "verification-artifact-transfer-v1".to_owned(),
        available: true,
        detail: Some(
            "bounded, digest-checked provider evidence isolated from source and check snapshots"
                .to_owned(),
        ),
        models: Vec::new(),
        source_repository: None,
        source_base_ref: None,
        source_base_commit: None,
    });
    capabilities.push(RunnerCapability {
        workspace_connection_id: None,
        name: "checkpoint-verification-v1".to_owned(),
        available: true,
        detail: Some(
            "exact stopped-source admission and runner-owned verification commit without a provider"
                .to_owned(),
        ),
        models: Vec::new(),
        source_repository: None,
        source_base_ref: None,
        source_base_commit: None,
    });
    capabilities.push(RunnerCapability {
        workspace_connection_id: None,
        name: "secret-delivery".to_owned(),
        available: true,
        detail: Some(
            "task-scoped, expiring broker grants; environment injection is reduced assurance"
                .to_owned(),
        ),
        models: Vec::new(),
        source_repository: None,
        source_base_ref: None,
        source_base_commit: None,
    });
    capabilities.push(RunnerCapability {
        workspace_connection_id: None,
        name: "workspace-setup-v1".to_owned(),
        available: connection_manager.is_some(),
        detail: Some(if connection_manager.is_some() {
            "Native repository and coding-agent connections with runner-private state".to_owned()
        } else {
            "Configure an operator-private connections directory outside source repositories"
                .to_owned()
        }),
        models: vec![],
        source_repository: None,
        source_base_ref: None,
        source_base_commit: None,
    });
    let base_capabilities = Arc::new(capabilities.clone());
    if let Some(manager) = &connection_manager {
        capabilities.extend(manager.capabilities());
    }
    out_tx
        .send(RunnerToServer::Register {
            runner_id: args.runner_id.clone(),
            corp_id: args.corp_id,
            credential,
            connection_epoch,
            hostname,
            os: std::env::consts::OS.to_owned(),
            capabilities,
            active_runs: claims,
        })
        .map_err(|_| anyhow!("runner writer stopped before registration"))?;
    outbound.attach(out_tx.clone(), connection_epoch);

    let heartbeat_tx = out_tx.clone();
    let heartbeat_runner_id = args.runner_id.clone();
    let heartbeat_runs = active_runs.clone();
    let heartbeat_connections = connection_manager.clone();
    let heartbeat_outbound = outbound.clone();
    let heartbeat_corp = args.corp_id;
    let heartbeat = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(10));
        loop {
            tick.tick().await;
            if heartbeat_tx
                .send(RunnerToServer::Heartbeat {
                    runner_id: heartbeat_runner_id.clone(),
                    connection_epoch,
                    active_runs: active_run_claims(&heartbeat_runs),
                })
                .is_err()
            {
                break;
            }
            if let Some(manager) = &heartbeat_connections {
                for (operation_id, report) in manager.pending_reports() {
                    heartbeat_outbound.send_live(RunnerToServer::WorkspaceSetupReport {
                        runner_id: heartbeat_runner_id.clone(),
                        corp_id: heartbeat_corp,
                        connection_epoch,
                        operation_id,
                        report,
                    });
                }
            }
        }
    });

    info!(runner_id = %args.runner_id, %connection_epoch, server = %args.server_ws, "runner connected");
    let mut reconnect_delay = Duration::from_secs(2);
    let mut registration_accepted = false;
    'read: while let Some(message) = socket_rx.next().await {
        let text = match message {
            Ok(Message::Text(text)) => text,
            Ok(Message::Close(_)) => break,
            Ok(_) => continue,
            Err(error) => {
                warn!(%error, "runner socket read failed");
                break;
            }
        };
        let command: ServerToRunner =
            serde_json::from_str(text.as_str()).context("decode server command")?;
        match command {
            ServerToRunner::WorkspaceSetup { command } => {
                if !registration_accepted
                    || command.corp_id != args.corp_id
                    || command.runner_id != args.runner_id
                {
                    warn!("rejected setup outside this registered runner");
                    continue;
                }
                let Some(manager) = connection_manager.clone() else {
                    continue;
                };
                let output = outbound.clone();
                let runner_id = args.runner_id.clone();
                let corp_id = args.corp_id;
                tokio::spawn(async move {
                    let operation_id = command.operation_id;
                    let connection_id = command.action.connection_id();
                    let progress_output = output.clone();
                    let progress_runner = runner_id.clone();
                    let progress: connections::SetupProgress = Arc::new(move |report| {
                        progress_output.send_live(RunnerToServer::WorkspaceSetupReport {
                            runner_id: progress_runner.clone(),
                            corp_id,
                            connection_epoch,
                            operation_id,
                            report,
                        });
                    });
                    let report = match manager.execute(command, progress).await {
                        Ok(report) => report,
                        Err(error) => {
                            warn!(%error, %operation_id, "native setup could not finish");
                            crony_domain::WorkspaceSetupReport {
                                status: crony_domain::WorkspaceSetupStatus::Failed,
                                detail: "The native connection check could not finish. Review this machine's setup and try again.".to_owned(),
                                connection_status: connection_id.map(|_|crony_domain::WorkspaceConnectionStatus::Failed),
                                source: None, models: vec![], account_login: None, sign_in: None,
                                repositories: vec![],
                            }
                        }
                    };
                    output.send_live(RunnerToServer::WorkspaceSetupReport {
                        runner_id,
                        corp_id,
                        connection_epoch,
                        operation_id,
                        report,
                    });
                });
            }
            ServerToRunner::WorkspaceSetupAck {
                operation_id,
                accepted,
            } => {
                if !registration_accepted {
                    continue;
                }
                if let Some(manager) = &connection_manager {
                    if let Err(error) = manager.acknowledge(operation_id, accepted).await {
                        warn!(%error, %operation_id, "setup acknowledgment could not be retained");
                        continue;
                    }
                    let mut capabilities = (*base_capabilities).clone();
                    capabilities.extend(manager.capabilities());
                    outbound.send_live(RunnerToServer::CapabilitiesUpdated {
                        runner_id: args.runner_id.clone(),
                        corp_id: args.corp_id,
                        connection_epoch,
                        capabilities,
                    });
                }
            }
            ServerToRunner::WorkspaceSignInInput {
                operation_id,
                request_id,
                response,
            } => {
                if !registration_accepted {
                    continue;
                }
                let applied = if let Some(manager) = &connection_manager {
                    manager.submit_sign_in(operation_id, response).await.is_ok()
                } else {
                    false
                };
                outbound.send_live(RunnerToServer::WorkspaceSignInAck {
                    runner_id: args.runner_id.clone(),
                    corp_id: args.corp_id,
                    connection_epoch,
                    operation_id,
                    request_id,
                    applied,
                });
            }
            ServerToRunner::Registered {
                runner_id,
                credential,
                expires_at,
            } => {
                persist_runner_credential(
                    &args,
                    CredentialFile {
                        runner_id: runner_id.clone(),
                        corp_id: args.corp_id,
                        credential,
                        expires_at: expires_at.clone(),
                    },
                )
                .await?;
                registration_accepted = runner_id == args.runner_id;
                if let Some(enrollment_file) = &args.enrollment_token_file {
                    let _ = tokio::fs::remove_file(enrollment_file).await;
                }
                info!(%runner_id, %expires_at, "runner registration accepted");
            }
            ServerToRunner::RegistrationRejected { reason } => {
                return Err(anyhow!("runner registration rejected: {reason}"));
            }
            ServerToRunner::ArtifactStored {
                run_id,
                artifact_id,
                artifact_role,
                sha256,
            } => {
                if let Some(active) = active_runs.get(&run_id) {
                    let _ = active.artifact_ack.send(ArtifactAck {
                        artifact_id,
                        artifact_role,
                        sha256,
                    });
                }
            }
            ServerToRunner::StartRun {
                workspace_connection_id,
                corp_id,
                room_id,
                mission_id,
                task_id,
                run_id,
                agent_id,
                assignment_token,
                adapter,
                mission_title,
                model,
                reasoning_effort,
                source_repository,
                source_base_ref,
                source_base_commit,
                verification_policy,
                write_scope,
                deliverable,
                secrets,
            } => {
                let assignment = Assignment {
                    corp_id,
                    connection_epoch,
                    room_id,
                    mission_id,
                    task_id,
                    run_id,
                    workspace_run_id: run_id,
                    agent_id,
                    assignment_token,
                    adapter,
                    mission_title,
                    model,
                    reasoning_effort,
                    source_repository,
                    source_base_ref,
                    source_base_commit,
                    workspace_connection_id,
                    resume_workspace_base_commit: None,
                    verification_policy,
                    write_scope,
                    deliverable,
                    secrets,
                    expected_workspace_fingerprint: None,
                    expected_head_commit: None,
                    provider_artifact: None,
                    checkpoint_verification: false,
                    hard_boundary_checkpoint: Arc::default(),
                };
                if assignment.workspace_connection_id.is_none()
                    && let Err(error) = validate_assignment_source(&workspaces, &assignment)
                {
                    send_run_event(
                        &outbound,
                        &args.runner_id,
                        &assignment,
                        "run.failed",
                        json!({"error": error.to_string()}),
                    );
                    continue;
                }
                let secret_ttl = match secret_expiry_delay(&assignment.secrets) {
                    Ok(ttl) => ttl,
                    Err(error) => {
                        send_run_event(
                            &outbound,
                            &args.runner_id,
                            &assignment,
                            "run.failed",
                            json!({"error": error.to_string()}),
                        );
                        continue;
                    }
                };
                if active_runs.contains_key(&assignment.run_id) {
                    warn!(run_id = %assignment.run_id, "duplicate start command ignored");
                    continue;
                }
                let Some(adapter) = adapters.get(&assignment.adapter) else {
                    send_run_event(
                        &outbound,
                        &args.runner_id,
                        &assignment,
                        "run.failed",
                        json!({
                            "error": format!("adapter {} is not installed", assignment.adapter)
                        }),
                    );
                    continue;
                };
                let (control_tx, control_rx) = mpsc::unbounded_channel::<AdapterControl>();
                let (artifact_ack_tx, artifact_ack_rx) = mpsc::unbounded_channel::<ArtifactAck>();
                schedule_secret_expiry(secret_ttl, control_tx.clone());
                active_runs.insert(
                    assignment.run_id,
                    ActiveRunControl {
                        assignment_token,
                        control: control_tx,
                        artifact_ack: artifact_ack_tx,
                        hard_boundary_checkpoint: assignment.hard_boundary_checkpoint.clone(),
                    },
                );
                let task_workspaces = workspaces.clone();
                let runner_id = args.runner_id.clone();
                let task_outbound = outbound.clone();
                let task_runs = active_runs.clone();
                let task_connections = connection_manager.clone();
                tokio::spawn(async move {
                    if let Err(error) = execute_connected_assignment(
                        task_workspaces,
                        runner_id.clone(),
                        assignment.clone(),
                        adapter,
                        task_outbound.clone(),
                        AssignmentChannels {
                            controls: control_rx,
                            artifact_acks: artifact_ack_rx,
                        },
                        None,
                        task_connections,
                    )
                    .await
                    {
                        error!(%error, run_id = %assignment.run_id, "run execution failed");
                        send_run_event(
                            &task_outbound,
                            &runner_id,
                            &assignment,
                            "run.failed",
                            json!({"error": error.to_string()}),
                        );
                    }
                    task_runs.remove(&assignment.run_id);
                });
            }
            ServerToRunner::ResumeRun {
                workspace_connection_id,
                command_id,
                corp_id,
                room_id,
                mission_id,
                task_id,
                run_id,
                workspace_run_id,
                agent_id,
                assignment_token,
                adapter,
                provider_session_id,
                prompt,
                model,
                reasoning_effort,
                source_repository,
                source_base_ref,
                source_base_commit,
                workspace_base_commit,
                expected_workspace_fingerprint,
                expected_head_commit,
                verification_policy,
                write_scope,
                deliverable,
                secrets,
            } => {
                if command_id.is_some_and(|id| seen_commands.insert(id, ()).is_some()) {
                    send_command_ack(
                        &outbound,
                        &args.runner_id,
                        connection_epoch,
                        command_id,
                        true,
                        "factory recovery resume command was already applied",
                    );
                    continue;
                }
                let assignment = Assignment {
                    corp_id,
                    connection_epoch,
                    room_id,
                    mission_id,
                    task_id,
                    run_id,
                    workspace_run_id,
                    agent_id,
                    assignment_token,
                    adapter,
                    mission_title: prompt,
                    model,
                    reasoning_effort,
                    source_repository,
                    source_base_ref,
                    source_base_commit,
                    workspace_connection_id,
                    resume_workspace_base_commit: workspace_base_commit,
                    verification_policy,
                    write_scope,
                    deliverable,
                    secrets,
                    expected_workspace_fingerprint,
                    expected_head_commit,
                    provider_artifact: None,
                    checkpoint_verification: false,
                    hard_boundary_checkpoint: Arc::default(),
                };
                if assignment.workspace_connection_id.is_none()
                    && let Err(error) = validate_assignment_source(&workspaces, &assignment)
                {
                    if let Some(command_id) = command_id {
                        seen_commands.remove(&command_id);
                    }
                    send_run_event(
                        &outbound,
                        &args.runner_id,
                        &assignment,
                        "run.failed",
                        json!({"error": error.to_string()}),
                    );
                    send_command_ack(
                        &outbound,
                        &args.runner_id,
                        connection_epoch,
                        command_id,
                        false,
                        "factory recovery resume source validation failed",
                    );
                    continue;
                }
                let secret_ttl = match secret_expiry_delay(&assignment.secrets) {
                    Ok(ttl) => ttl,
                    Err(error) => {
                        if let Some(command_id) = command_id {
                            seen_commands.remove(&command_id);
                        }
                        send_run_event(
                            &outbound,
                            &args.runner_id,
                            &assignment,
                            "run.failed",
                            json!({"error": error.to_string()}),
                        );
                        send_command_ack(
                            &outbound,
                            &args.runner_id,
                            connection_epoch,
                            command_id,
                            false,
                            "factory recovery resume secret validation failed",
                        );
                        continue;
                    }
                };
                if active_runs.contains_key(&assignment.run_id) {
                    warn!(run_id = %assignment.run_id, "duplicate resume command ignored");
                    send_command_ack(
                        &outbound,
                        &args.runner_id,
                        connection_epoch,
                        command_id,
                        true,
                        "factory recovery resume run was already active",
                    );
                    continue;
                }
                let Some(adapter) = adapters.get(&assignment.adapter) else {
                    if let Some(command_id) = command_id {
                        seen_commands.remove(&command_id);
                    }
                    send_run_event(
                        &outbound,
                        &args.runner_id,
                        &assignment,
                        "run.failed",
                        json!({
                            "error": format!("adapter {} is not installed", assignment.adapter)
                        }),
                    );
                    send_command_ack(
                        &outbound,
                        &args.runner_id,
                        connection_epoch,
                        command_id,
                        false,
                        "factory recovery adapter is unavailable",
                    );
                    continue;
                };
                let (control_tx, control_rx) = mpsc::unbounded_channel::<AdapterControl>();
                let (artifact_ack_tx, artifact_ack_rx) = mpsc::unbounded_channel::<ArtifactAck>();
                schedule_secret_expiry(secret_ttl, control_tx.clone());
                active_runs.insert(
                    assignment.run_id,
                    ActiveRunControl {
                        assignment_token,
                        control: control_tx,
                        artifact_ack: artifact_ack_tx,
                        hard_boundary_checkpoint: assignment.hard_boundary_checkpoint.clone(),
                    },
                );
                let task_workspaces = workspaces.clone();
                let runner_id = args.runner_id.clone();
                let task_outbound = outbound.clone();
                let task_runs = active_runs.clone();
                let task_connections = connection_manager.clone();
                tokio::spawn(async move {
                    if let Err(error) = execute_connected_assignment(
                        task_workspaces,
                        runner_id.clone(),
                        assignment.clone(),
                        adapter,
                        task_outbound.clone(),
                        AssignmentChannels {
                            controls: control_rx,
                            artifact_acks: artifact_ack_rx,
                        },
                        Some(provider_session_id),
                        task_connections,
                    )
                    .await
                    {
                        error!(%error, run_id = %assignment.run_id, "resumed run execution failed");
                        send_run_event(
                            &task_outbound,
                            &runner_id,
                            &assignment,
                            "run.failed",
                            json!({"error": error.to_string()}),
                        );
                    }
                    task_runs.remove(&assignment.run_id);
                });
                send_command_ack(
                    &outbound,
                    &args.runner_id,
                    connection_epoch,
                    command_id,
                    true,
                    "factory recovery resume command accepted",
                );
            }
            ServerToRunner::VerifyRun {
                workspace_connection_id,
                command_id,
                corp_id,
                room_id,
                mission_id,
                task_id,
                run_id,
                workspace_run_id,
                agent_id,
                assignment_token,
                source_repository,
                source_base_ref,
                source_base_commit,
                workspace_base_commit,
                expected_workspace_fingerprint,
                expected_head_commit,
                checkpoint_verification,
                verification_policy,
                write_scope,
                deliverable,
                provider_artifact,
            } => {
                if seen_commands.insert(command_id, ()).is_some() {
                    send_command_ack(
                        &outbound,
                        &args.runner_id,
                        connection_epoch,
                        Some(command_id),
                        true,
                        "verifier-only recovery command was already applied",
                    );
                    continue;
                }
                let assignment = Assignment {
                    corp_id,
                    connection_epoch,
                    room_id,
                    mission_id,
                    task_id,
                    run_id,
                    workspace_run_id,
                    agent_id,
                    assignment_token,
                    adapter: "verification-only".to_owned(),
                    mission_title: "Verifier-only factory recovery".to_owned(),
                    model: None,
                    reasoning_effort: None,
                    source_repository,
                    source_base_ref,
                    source_base_commit,
                    workspace_connection_id,
                    resume_workspace_base_commit: Some(workspace_base_commit),
                    verification_policy,
                    write_scope,
                    deliverable,
                    secrets: Vec::new(),
                    expected_workspace_fingerprint: Some(expected_workspace_fingerprint),
                    expected_head_commit,
                    provider_artifact,
                    checkpoint_verification,
                    hard_boundary_checkpoint: Arc::default(),
                };
                if assignment.workspace_connection_id.is_none()
                    && let Err(error) = validate_assignment_source(&workspaces, &assignment)
                {
                    seen_commands.remove(&command_id);
                    send_run_event(
                        &outbound,
                        &args.runner_id,
                        &assignment,
                        "run.failed",
                        json!({"error": error.to_string()}),
                    );
                    send_command_ack(
                        &outbound,
                        &args.runner_id,
                        connection_epoch,
                        Some(command_id),
                        false,
                        "verifier-only recovery source validation failed",
                    );
                    continue;
                }
                if active_runs.contains_key(&assignment.run_id) {
                    warn!(run_id = %assignment.run_id, "duplicate verification command ignored");
                    send_command_ack(
                        &outbound,
                        &args.runner_id,
                        connection_epoch,
                        Some(command_id),
                        true,
                        "verifier-only recovery run was already active",
                    );
                    continue;
                }
                let (control_tx, control_rx) = mpsc::unbounded_channel::<AdapterControl>();
                let (artifact_ack_tx, artifact_ack_rx) = mpsc::unbounded_channel::<ArtifactAck>();
                active_runs.insert(
                    assignment.run_id,
                    ActiveRunControl {
                        assignment_token,
                        control: control_tx,
                        artifact_ack: artifact_ack_tx,
                        hard_boundary_checkpoint: assignment.hard_boundary_checkpoint.clone(),
                    },
                );
                let task_workspaces = workspaces.clone();
                let runner_id = args.runner_id.clone();
                let task_outbound = outbound.clone();
                let task_runs = active_runs.clone();
                let task_connections = connection_manager.clone();
                tokio::spawn(async move {
                    if let Err(error) = execute_connected_verification_assignment(
                        task_workspaces,
                        runner_id.clone(),
                        assignment.clone(),
                        task_outbound.clone(),
                        AssignmentChannels {
                            controls: control_rx,
                            artifact_acks: artifact_ack_rx,
                        },
                        task_connections,
                    )
                    .await
                    {
                        error!(%error, run_id = %assignment.run_id, "verification-only run failed");
                        send_run_event(
                            &task_outbound,
                            &runner_id,
                            &assignment,
                            "run.failed",
                            json!({"error": error.to_string()}),
                        );
                    }
                    task_runs.remove(&assignment.run_id);
                });
                send_command_ack(
                    &outbound,
                    &args.runner_id,
                    connection_epoch,
                    Some(command_id),
                    true,
                    "verifier-only recovery command accepted",
                );
            }
            ServerToRunner::CheckpointWorkspace {
                workspace_connection_id,
                command_id,
                corp_id,
                room_id,
                mission_id,
                task_id,
                run_id,
                workspace_run_id,
                agent_id,
                assignment_token,
                source_repository,
                source_base_ref,
                source_base_commit,
                workspace_base_commit,
                expected_head_commit,
            } => {
                if seen_commands.insert(command_id, ()).is_some() {
                    send_command_ack(
                        &outbound,
                        &args.runner_id,
                        connection_epoch,
                        Some(command_id),
                        true,
                        "factory workspace checkpoint command was already applied",
                    );
                    continue;
                }
                let assignment = Assignment {
                    corp_id,
                    connection_epoch,
                    room_id,
                    mission_id,
                    task_id,
                    run_id,
                    workspace_run_id,
                    agent_id,
                    assignment_token,
                    adapter: "workspace-checkpoint".to_owned(),
                    mission_title: "Factory workspace checkpoint".to_owned(),
                    model: None,
                    reasoning_effort: None,
                    source_repository,
                    source_base_ref,
                    source_base_commit,
                    workspace_connection_id,
                    resume_workspace_base_commit: Some(workspace_base_commit),
                    verification_policy: VerificationPolicy {
                        checks: Vec::new(),
                        manual_gate: None,
                    },
                    write_scope: Vec::new(),
                    deliverable: None,
                    secrets: Vec::new(),
                    expected_workspace_fingerprint: None,
                    expected_head_commit: Some(expected_head_commit),
                    provider_artifact: None,
                    checkpoint_verification: false,
                    hard_boundary_checkpoint: Arc::default(),
                };
                match checkpoint_connected_workspace(
                    workspaces.clone(),
                    args.runner_id.clone(),
                    assignment,
                    outbound.clone(),
                    connection_manager.clone(),
                )
                .await
                {
                    Ok(()) => send_command_ack(
                        &outbound,
                        &args.runner_id,
                        connection_epoch,
                        Some(command_id),
                        true,
                        "factory workspace checkpoint recorded",
                    ),
                    Err(error) => {
                        seen_commands.remove(&command_id);
                        warn!(%error, run_id = %run_id, "factory workspace checkpoint failed");
                        send_command_ack(
                            &outbound,
                            &args.runner_id,
                            connection_epoch,
                            Some(command_id),
                            false,
                            "factory workspace checkpoint failed closed",
                        );
                    }
                }
            }
            ServerToRunner::ControlMessage {
                command_id,
                message_id,
                run_id,
                actor_id,
                lease_token: _,
                text,
                ..
            } => {
                let Some(command_id) = command_id else {
                    if let Some(active) = active_runs.get(&run_id) {
                        let _ = active
                            .control
                            .send(AdapterControl::Steer { actor_id, text });
                        info!(%run_id, %actor_id, "accepted legacy fenced control message");
                    } else {
                        warn!(%run_id, "legacy control message arrived for inactive run");
                    }
                    continue;
                };
                let message_id = message_id.unwrap_or(command_id);
                let duplicate = seen_commands.contains_key(&command_id);
                let applied = if duplicate {
                    true
                } else {
                    let applied = active_runs.get(&run_id).is_some_and(|active| {
                        active
                            .control
                            .send(AdapterControl::Steer {
                                actor_id,
                                text: text.clone(),
                            })
                            .is_ok()
                    });
                    if applied {
                        seen_commands.insert(command_id, ());
                    }
                    applied
                };
                outbound.send(RunnerToServer::CommandAck {
                    runner_id: args.runner_id.clone(),
                    connection_epoch,
                    command_id,
                    applied,
                    detail: if duplicate {
                        format!("control message {message_id} was already applied")
                    } else if applied {
                        format!("control message {message_id} was applied")
                    } else {
                        format!("control message {message_id} has no active run")
                    },
                });
                if applied {
                    info!(%run_id, %actor_id, %message_id, duplicate, "accepted durable control message");
                } else {
                    warn!(%run_id, %message_id, "durable control message arrived for inactive run");
                }
            }
            ServerToRunner::StopRun { run_id, reason, .. } => {
                if let Some(active) = active_runs.get(&run_id) {
                    let _ = active.control.send(AdapterControl::Stop { reason });
                } else {
                    warn!(%run_id, "stop command arrived for inactive run");
                }
            }
            ServerToRunner::InterruptRun { run_id, reason } => {
                if let Some(active) = active_runs.get(&run_id) {
                    let _ = active.control.send(AdapterControl::Interrupt { reason });
                } else {
                    warn!(%run_id, "interrupt command arrived for inactive run");
                }
            }
            ServerToRunner::ApprovalDecision {
                command_id,
                run_id,
                approval_id,
                approved,
                note,
            } => {
                let duplicate = seen_commands.insert(command_id, ()).is_some();
                let applied = duplicate
                    || active_runs.get(&run_id).is_some_and(|active| {
                        active
                            .control
                            .send(AdapterControl::ApprovalDecision {
                                approval_id,
                                approved,
                                note,
                            })
                            .is_ok()
                    });
                outbound.send(RunnerToServer::CommandAck {
                    runner_id: args.runner_id.clone(),
                    connection_epoch,
                    command_id,
                    applied,
                    detail: if duplicate {
                        "approval decision was already applied".to_owned()
                    } else if applied {
                        "approval decision delivered to the active provider".to_owned()
                    } else {
                        "approval decision had no active provider".to_owned()
                    },
                });
            }
            ServerToRunner::CircuitBreaker {
                command_id,
                run_id,
                stage,
                reason,
            } => {
                let (applied, duplicate) = apply_circuit_breaker_command(
                    &seen_commands,
                    &active_runs,
                    command_id,
                    run_id,
                    stage,
                    reason,
                );
                outbound.send(RunnerToServer::CommandAck {
                    runner_id: args.runner_id.clone(),
                    connection_epoch,
                    command_id,
                    applied,
                    detail: if duplicate {
                        "circuit-breaker command was already applied".to_owned()
                    } else if applied {
                        "circuit-breaker command delivered to the active provider".to_owned()
                    } else {
                        "circuit-breaker command had no active provider or cancellable verification"
                            .to_owned()
                    },
                });
            }
            ServerToRunner::Disconnect {
                reason,
                reconnect_delay_ms,
            } => {
                reconnect_delay = Duration::from_millis(reconnect_delay_ms);
                info!(%reason, reconnect_delay_ms, "server requested runner reconnect");
                break 'read;
            }
        }
    }

    outbound.detach();
    heartbeat.abort();
    writer.abort();
    Ok(reconnect_delay)
}

async fn load_runner_credential(args: &Args) -> Result<String> {
    if let Ok(contents) = tokio::fs::read_to_string(&args.credential_file).await {
        let credential: CredentialFile =
            serde_json::from_str(&contents).context("decode runner credential file")?;
        if credential.runner_id != args.runner_id || credential.corp_id != args.corp_id {
            return Err(anyhow!(
                "runner credential file is scoped to a different runner or Corp"
            ));
        }
        if credential.credential.trim().is_empty() {
            return Err(anyhow!(
                "runner credential file contains an empty credential"
            ));
        }
        return Ok(credential.credential);
    }
    let enrollment_file = args
        .enrollment_token_file
        .as_ref()
        .context("runner is not enrolled; provide CRONY_RUNNER_ENROLLMENT_TOKEN_FILE")?;
    let token = tokio::fs::read_to_string(enrollment_file)
        .await
        .with_context(|| {
            format!(
                "read runner enrollment token from {}",
                enrollment_file.display()
            )
        })?;
    let token = token.trim();
    if token.is_empty() {
        return Err(anyhow!("runner enrollment token file is empty"));
    }
    Ok(token.to_owned())
}

async fn persist_runner_credential(args: &Args, credential: CredentialFile) -> Result<()> {
    if let Some(parent) = args.credential_file.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create credential directory {}", parent.display()))?;
    }
    let temporary = args.credential_file.with_extension("json.tmp");
    let bytes = serde_json::to_vec(&credential).context("encode runner credential")?;
    tokio::fs::write(&temporary, bytes)
        .await
        .with_context(|| format!("write temporary credential {}", temporary.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))
            .await
            .with_context(|| format!("restrict credential permissions {}", temporary.display()))?;
    }
    tokio::fs::rename(&temporary, &args.credential_file)
        .await
        .with_context(|| {
            format!(
                "replace runner credential file {}",
                args.credential_file.display()
            )
        })?;
    Ok(())
}

struct RunnerEventSink {
    outbound: OutboundBus,
    runner_id: String,
    assignment: Assignment,
    workspace: WorkspaceLease,
    artifacts: Arc<Mutex<Vec<AdapterArtifact>>>,
    terminal: Arc<Mutex<Option<BufferedTerminal>>>,
    teardown_uncertain: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
enum BufferedTerminal {
    Completed(String),
    Failed(String),
    Cancelled(String),
}

impl AdapterEventSink for RunnerEventSink {
    fn emit(&self, event: AdapterEvent) {
        let (event_type, payload) = match event {
            AdapterEvent::Session { session_id } => {
                ("run.session", json!({"session_id": session_id}))
            }
            AdapterEvent::Started { workspace } => (
                "run.started",
                json!({
                    "room_id": self.assignment.room_id,
                    "mission_id": self.assignment.mission_id,
                    "task_id": self.assignment.task_id,
                    "workspace": workspace,
                    "workspace_branch": self.workspace.branch,
                    "workspace_base_ref": self.workspace.base_ref,
                    "workspace_base_commit": self.workspace.base_commit,
                    "adapter": self.assignment.adapter,
                }),
            ),
            AdapterEvent::Status {
                status,
                station,
                message,
            } => (
                "run.status",
                json!({
                    "status": status,
                    "station": station,
                    "message": message,
                }),
            ),
            AdapterEvent::Output { stream, text } => (
                "run.output",
                json!({
                    "stream": stream,
                    "text": text,
                }),
            ),
            AdapterEvent::Artifact(_) if self.assignment.hard_boundary_requested() => {
                // Native adapters may finish their local transcript on interruption.
                // Keep those bytes in the retained worktree, but do not turn a known
                // hard stop into a rejected upload and premature run.failed event.
                return;
            }
            AdapterEvent::Artifact(artifact) => match std::fs::read(&artifact.path) {
                Ok(bytes) => {
                    if let Ok(mut artifacts) = self.artifacts.lock() {
                        artifacts.push(artifact.clone());
                    }
                    let workspace_relative_path = artifact
                        .path
                        .strip_prefix(&self.workspace.path)
                        .ok()
                        .map(|path| path.to_string_lossy().replace('\\', "/"));
                    (
                        "run.artifact_upload",
                        json!({
                            "sha256": artifact.sha256,
                            "bytes": artifact.bytes,
                            "media_type": artifact.media_type,
                            "artifact_role": "provider_evidence",
                            "file_name": artifact.path.file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or("artifact.bin"),
                            "workspace_relative_path": workspace_relative_path,
                            "content_base64": BASE64.encode(bytes),
                        }),
                    )
                }
                Err(error) => (
                    "run.failed",
                    json!({
                        "error": format!("artifact upload preparation failed: {error}"),
                    }),
                ),
            },
            AdapterEvent::Usage(usage) => (
                "run.usage",
                json!({
                    "input_tokens": usage.input_tokens,
                    "output_tokens": usage.output_tokens,
                    "cost_microusd": usage.cost_microusd,
                }),
            ),
            AdapterEvent::ApprovalRequested {
                approval_id,
                action_key,
                action,
                risk,
                rationale,
                required_roles,
                expires_in_seconds,
            } => (
                "run.approval_requested",
                json!({
                    "approval_id": approval_id,
                    "action_key": action_key,
                    "action": action,
                    "risk": risk,
                    "rationale": rationale,
                    "required_roles": required_roles,
                    "expires_in_seconds": expires_in_seconds,
                }),
            ),
            AdapterEvent::ToolActivity {
                signature,
                progressed,
                human_conversation,
            } => (
                "run.tool_activity",
                json!({
                    "signature": signature,
                    "progressed": progressed,
                    "human_conversation": human_conversation,
                }),
            ),
            AdapterEvent::TeardownUncertain { detail } => {
                self.teardown_uncertain.store(true, Ordering::Release);
                send_run_event(
                    &self.outbound,
                    &self.runner_id,
                    &self.assignment,
                    "run.teardown_uncertain",
                    json!({
                        "adapter": self.assignment.adapter,
                        "provider_process_state": "unverified",
                        "detail": detail,
                    }),
                );
                send_teardown_workspace_preserved(
                    &self.outbound,
                    &self.runner_id,
                    &self.assignment,
                    &self.workspace,
                    &detail,
                    None,
                    false,
                );
                return;
            }
            AdapterEvent::Completed { summary } => {
                if let Ok(mut terminal) = self.terminal.lock() {
                    *terminal = Some(BufferedTerminal::Completed(summary));
                }
                return;
            }
            AdapterEvent::Failed { error } => {
                if let Ok(mut terminal) = self.terminal.lock() {
                    *terminal = Some(BufferedTerminal::Failed(error));
                }
                return;
            }
            AdapterEvent::Cancelled { reason } => {
                if let Ok(mut terminal) = self.terminal.lock() {
                    *terminal = Some(BufferedTerminal::Cancelled(reason));
                }
                return;
            }
        };
        send_run_event(
            &self.outbound,
            &self.runner_id,
            &self.assignment,
            event_type,
            payload,
        );
    }
}

fn secret_expiry_delay(secrets: &[ResolvedSecret]) -> Result<Option<Duration>> {
    let now = Utc::now();
    let mut earliest = None;
    for secret in secrets {
        let expires_at = DateTime::parse_from_rfc3339(&secret.expires_at)
            .with_context(|| format!("secret grant {} has an invalid expiry", secret.grant_id))?
            .with_timezone(&Utc);
        if expires_at <= now {
            return Err(anyhow!(
                "secret grant {} expired before provider start",
                secret.grant_id
            ));
        }
        earliest =
            Some(earliest.map_or(expires_at, |current: DateTime<Utc>| current.min(expires_at)));
    }
    earliest
        .map(|expires_at| {
            (expires_at - now)
                .to_std()
                .context("secret grant expiry interval is invalid")
        })
        .transpose()
}

fn schedule_secret_expiry(ttl: Option<Duration>, control: mpsc::UnboundedSender<AdapterControl>) {
    let Some(ttl) = ttl else {
        return;
    };
    tokio::spawn(async move {
        tokio::time::sleep(ttl).await;
        let _ = control.send(AdapterControl::Stop {
            reason: "task-scoped secret grant expired; provider stopped".to_owned(),
        });
    });
}

fn connection_source(assignment: &Assignment) -> Result<crony_domain::WorkspaceSourceIdentity> {
    Ok(crony_domain::WorkspaceSourceIdentity {
        repository: assignment
            .source_repository
            .clone()
            .context("saved connection omitted its repository")?,
        repository_id: None,
        base_ref: assignment
            .source_base_ref
            .clone()
            .context("saved connection omitted its source ref")?,
        base_commit: assignment
            .source_base_commit
            .clone()
            .context("saved connection omitted its pinned commit")?,
    })
}

#[allow(clippy::too_many_arguments)]
async fn execute_connected_assignment(
    workspaces: Arc<WorkspaceManager>,
    runner_id: String,
    assignment: Assignment,
    adapter: Arc<dyn AgentAdapter>,
    outbound: OutboundBus,
    channels: AssignmentChannels,
    resume_session_id: Option<String>,
    connections: Option<Arc<connections::ConnectionManager>>,
) -> Result<()> {
    let (workspaces, adapter) = if let Some(id) = assignment.workspace_connection_id {
        let manager =
            connections.context("this runner cannot open the saved execution connection")?;
        let runtime = manager
            .resolve(id, &connection_source(&assignment)?, &assignment.adapter)
            .await?;
        (runtime.workspaces, runtime.adapter)
    } else {
        (workspaces, adapter)
    };
    execute_assignment(
        workspaces,
        runner_id,
        assignment,
        adapter,
        outbound,
        channels,
        resume_session_id,
    )
    .await
}

async fn execute_connected_verification_assignment(
    workspaces: Arc<WorkspaceManager>,
    runner_id: String,
    assignment: Assignment,
    outbound: OutboundBus,
    channels: AssignmentChannels,
    connections: Option<Arc<connections::ConnectionManager>>,
) -> Result<()> {
    let workspaces = if let Some(id) = assignment.workspace_connection_id {
        connections
            .context("this runner cannot open the saved workspace")?
            .resolve_workspace(id, &connection_source(&assignment)?)
            .await?
    } else {
        workspaces
    };
    execute_verification_assignment(workspaces, runner_id, assignment, outbound, channels).await
}

async fn checkpoint_connected_workspace(
    workspaces: Arc<WorkspaceManager>,
    runner_id: String,
    assignment: Assignment,
    outbound: OutboundBus,
    connections: Option<Arc<connections::ConnectionManager>>,
) -> Result<()> {
    let workspaces = if let Some(id) = assignment.workspace_connection_id {
        connections
            .context("this runner cannot open the saved workspace")?
            .resolve_workspace(id, &connection_source(&assignment)?)
            .await?
    } else {
        workspaces
    };
    checkpoint_preserved_workspace(workspaces, runner_id, assignment, outbound).await
}

async fn execute_assignment(
    workspaces: Arc<WorkspaceManager>,
    runner_id: String,
    assignment: Assignment,
    adapter: Arc<dyn AgentAdapter>,
    outbound: OutboundBus,
    channels: AssignmentChannels,
    resume_session_id: Option<String>,
) -> Result<()> {
    let AssignmentChannels {
        controls,
        mut artifact_acks,
    } = channels;
    validate_assignment_source(&workspaces, &assignment)?;
    let workspace = workspaces
        .prepare(
            assignment.task_id,
            assignment.workspace_run_id,
            assignment.source_base_commit.as_deref(),
            assignment.resume_workspace_base_commit.as_deref(),
        )
        .await
        .context("prepare isolated task worktree")?;
    if let Err(error) =
        verify_prepared_recovery_workspace(&workspaces, &workspace, &assignment).await
    {
        send_run_event(
            &outbound,
            &runner_id,
            &assignment,
            "run.failed",
            json!({"error": format!("{error:#}")}),
        );
        send_teardown_workspace_preserved(
            &outbound,
            &runner_id,
            &assignment,
            &workspace,
            &format!("quarantined before provider startup: {error:#}"),
            None,
            true,
        );
        return Ok(());
    }
    let request = AdapterRunRequest {
        run_id: assignment.run_id,
        mission_id: assignment.mission_id,
        task_id: assignment.task_id,
        agent_id: assignment.agent_id,
        mission_title: assignment.mission_title.clone(),
        model: assignment.model.clone(),
        reasoning_effort: assignment.reasoning_effort.clone(),
        workspace: workspace.path.clone(),
        write_scope: assignment.write_scope.clone(),
        environment: assignment
            .secrets
            .iter()
            .map(|secret| (secret.env_name.clone(), secret.value.clone()))
            .collect(),
    };
    let artifacts = Arc::new(Mutex::new(Vec::<AdapterArtifact>::new()));
    let terminal = Arc::new(Mutex::new(None::<BufferedTerminal>));
    let teardown_uncertain = Arc::new(AtomicBool::new(false));
    let sink: Arc<dyn AdapterEventSink> = Arc::new(RunnerEventSink {
        outbound: outbound.clone(),
        runner_id: runner_id.clone(),
        assignment: assignment.clone(),
        workspace: workspace.clone(),
        artifacts: artifacts.clone(),
        terminal: terminal.clone(),
        teardown_uncertain: teardown_uncertain.clone(),
    });
    let execution = if let Some(session_id) = resume_session_id {
        adapter.resume(request, &session_id, controls, sink).await
    } else {
        adapter.execute(request, controls, sink).await
    };
    let mut preserve_workspace =
        execution.is_err() || assignment.resume_workspace_base_commit.is_some();
    let mut workspace_quarantined = false;
    let mut checkpoint_reported = false;
    let mut verification_started = false;
    let provider_outcome = match &execution {
        Ok(AdapterExit::Completed) => "completed",
        Ok(AdapterExit::Failed) => "failed",
        Ok(AdapterExit::Cancelled) => "cancelled",
        Err(_) => "runtime_error",
    };
    send_run_event(
        &outbound,
        &runner_id,
        &assignment,
        "run.session_terminated",
        json!({
            "adapter": &assignment.adapter,
            "outcome": provider_outcome,
            "provider_process_alive": false,
            "message": "Provider session stopped; no idle agent process remains.",
        }),
    );
    if execution.is_ok() {
        let buffered = terminal.lock().ok().and_then(|mut value| value.take());
        let mut terminal =
            buffered.unwrap_or_else(|| match execution.as_ref().expect("checked above") {
                AdapterExit::Completed => {
                    BufferedTerminal::Completed("Agent completed without a summary.".to_owned())
                }
                AdapterExit::Failed => {
                    BufferedTerminal::Failed("Agent failed without an error message.".to_owned())
                }
                AdapterExit::Cancelled => {
                    BufferedTerminal::Cancelled("Agent cancelled without a reason.".to_owned())
                }
            });
        if assignment.hard_boundary_requested() {
            source_checkpoint::report(
                &outbound,
                &runner_id,
                &assignment,
                &workspace,
                &workspaces,
                !teardown_uncertain.load(Ordering::Acquire),
                false,
            )
            .await;
            checkpoint_reported = true;
            preserve_workspace = true;
            if matches!(terminal, BufferedTerminal::Completed(_)) {
                // A delivered hard stop wins over a late provider completion.
                // Verification requires a separately authorized recovery.
                terminal = BufferedTerminal::Cancelled(
                    "Hard circuit-breaker boundary reached; source retained for explicit recovery."
                        .to_owned(),
                );
            }
        }
        match terminal {
            BufferedTerminal::Completed(summary) => {
                verification_started = true;
                let mut cancellation = assignment.hard_boundary_checkpoint.cancellation.subscribe();
                let verification = send_verification_events(
                    &outbound,
                    &runner_id,
                    &assignment,
                    &workspace,
                    &workspaces,
                    None,
                    None,
                    None,
                    &artifacts,
                    None,
                    &mut artifact_acks,
                    &summary,
                    None,
                    None,
                    None,
                    Some(&mut cancellation),
                )
                .await;
                preserve_workspace |= verification != VerificationRunOutcome::Finished;
                workspace_quarantined |= verification == VerificationRunOutcome::IntegrityFailed;
                if verification == VerificationRunOutcome::Cancelled {
                    send_run_event(
                        &outbound,
                        &runner_id,
                        &assignment,
                        "run.cancelled",
                        json!({"reason": "Hard circuit breaker cancelled verification; source retained without a pre-verification checkpoint."}),
                    );
                }
            }
            BufferedTerminal::Failed(error) => {
                preserve_workspace = true;
                send_run_event(
                    &outbound,
                    &runner_id,
                    &assignment,
                    "run.failed",
                    json!({"error": error}),
                );
            }
            BufferedTerminal::Cancelled(reason) => {
                send_run_event(
                    &outbound,
                    &runner_id,
                    &assignment,
                    "run.cancelled",
                    json!({"reason": reason}),
                );
            }
        }
    }
    if !checkpoint_reported && !assignment.hard_boundary_checkpoint.begin_finalization() {
        // Atomically choose retention or ordinary finalization before either
        // path awaits. A subsequently delivered hard directive cannot race removal.
        source_checkpoint::report(
            &outbound,
            &runner_id,
            &assignment,
            &workspace,
            &workspaces,
            execution.is_ok()
                && !teardown_uncertain.load(Ordering::Acquire)
                && !verification_started,
            workspace_quarantined,
        )
        .await;
        checkpoint_reported = true;
    }
    if checkpoint_reported {
        // No finalize, second seal, or source deletion after a hard-boundary report.
        execution?;
        return Ok(());
    }
    if teardown_uncertain.load(Ordering::Acquire) || preserve_workspace {
        let fingerprint = workspaces.fingerprint(&workspace).await.ok();
        send_teardown_workspace_preserved(
            &outbound,
            &runner_id,
            &assignment,
            &workspace,
            "provider recovery, failure, or teardown requires the exact worktree to be retained",
            fingerprint.as_deref(),
            workspace_quarantined,
        );
    } else {
        let cleanup = workspaces.finalize(&workspace).await;
        let fingerprint = match &cleanup {
            Ok(cleanup) if cleanup.disposition == WorkspaceDisposition::Preserved => {
                workspaces.fingerprint(&workspace).await.ok()
            }
            Err(_) => workspaces.fingerprint(&workspace).await.ok(),
            _ => None,
        };
        send_workspace_cleanup_event(
            &outbound,
            &runner_id,
            &assignment,
            &workspace,
            cleanup,
            fingerprint.as_deref(),
        );
    }
    execution?;
    Ok(())
}

async fn verify_prepared_recovery_workspace(
    workspaces: &WorkspaceManager,
    workspace: &WorkspaceLease,
    assignment: &Assignment,
) -> Result<()> {
    verify_preserved_workspace_checkpoint(
        workspaces,
        workspace,
        assignment.expected_workspace_fingerprint.as_deref(),
        assignment.expected_head_commit.as_deref(),
    )
    .await
}

async fn verify_preserved_workspace_checkpoint(
    workspaces: &WorkspaceManager,
    workspace: &WorkspaceLease,
    expected_fingerprint: Option<&str>,
    expected_head: Option<&str>,
) -> Result<()> {
    if let Some(expected) = expected_fingerprint {
        let actual = workspaces.fingerprint(workspace).await?;
        if actual != expected {
            return Err(anyhow!(
                "recovery workspace fingerprint mismatch: expected {expected}, found {actual}"
            ));
        }
    }
    if let Some(expected) = expected_head {
        let actual = workspaces.head_commit(workspace).await?;
        if actual != expected {
            return Err(anyhow!(
                "recovery workspace head mismatch: expected {expected}, found {actual}"
            ));
        }
    }
    Ok(())
}

async fn execute_verification_assignment(
    workspaces: Arc<WorkspaceManager>,
    runner_id: String,
    assignment: Assignment,
    outbound: OutboundBus,
    channels: AssignmentChannels,
) -> Result<()> {
    let AssignmentChannels {
        mut controls,
        mut artifact_acks,
    } = channels;
    validate_assignment_source(&workspaces, &assignment)?;
    let workspace = workspaces
        .prepare(
            assignment.task_id,
            assignment.workspace_run_id,
            assignment.source_base_commit.as_deref(),
            assignment.resume_workspace_base_commit.as_deref(),
        )
        .await
        .context("prepare preserved verifier-only worktree")?;
    send_run_event(
        &outbound,
        &runner_id,
        &assignment,
        "run.started",
        json!({
            "workspace": workspace.path,
            "workspace_branch": workspace.branch,
            "workspace_base_ref": workspace.base_ref,
            "workspace_base_commit": workspace.base_commit,
            "execution_mode": "verification_only",
        }),
    );
    // Own every snapshot before the next fallible operation. All prepared-workspace exits below
    // explicitly clean these snapshots and preserve the source; Drop is only a last-resort retry.
    let mut verification_baseline = None;
    let mut active_check_snapshot = None;
    let mut artifact_snapshot = None;
    let mut checkpoint_admitted = false;
    let mut workspace_quarantined = false;
    let mut cleanup_head_commit = assignment.expected_head_commit.clone();
    let result = async {
        let expected_fingerprint = assignment
            .expected_workspace_fingerprint
            .as_deref()
            .context("verifier-only assignment omitted workspace fingerprint")?;
        if let Err(error) =
            verify_prepared_recovery_workspace(&workspaces, &workspace, &assignment).await
        {
            workspace_quarantined = true;
            return Err(error);
        }
        verification_baseline =
            Some(workspace::verification_snapshot(&workspace.path, assignment.run_id).await?);
        let baseline = verification_baseline
            .as_mut()
            .context("verifier-only baseline was not created")?;
        let baseline_fingerprint = match workspace::fingerprint_path(baseline.path()).await {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                workspace_quarantined = true;
                return Err(error);
            }
        };
        if baseline_fingerprint != expected_fingerprint {
            workspace_quarantined = true;
            return Err(anyhow!(
                "verifier-only baseline fingerprint mismatch: expected {expected_fingerprint}, found {baseline_fingerprint}"
            ));
        }
        checkpoint_admitted = true;
        let (artifacts, source_artifacts) = match assignment.provider_artifact.as_ref() {
            Some(reference) => {
                artifact_snapshot =
                    Some(workspace::empty_verification_snapshot(assignment.run_id).await?);
                let prepared = verifier_only_artifact(
                    &workspace,
                    artifact_snapshot
                        .as_ref()
                        .context("artifact snapshot was not created")?,
                    reference,
                )
                .await?;
                (
                    vec![prepared.artifact],
                    prepared.source_artifact.into_iter().collect::<Vec<_>>(),
                )
            }
            None => (Vec::new(), Vec::new()),
        };
        let artifacts = Arc::new(Mutex::new(artifacts));
        let (cancellation_tx, mut cancellation_rx) = watch::channel(false);
        let mut interruption = None;
        let verification = send_verification_events(
            &outbound,
            &runner_id,
            &assignment,
            &workspace,
            &workspaces,
            Some(baseline),
            Some(&mut active_check_snapshot),
            Some(&mut artifact_snapshot),
            &artifacts,
            Some(&source_artifacts),
            &mut artifact_acks,
            "Verifier-only recovery completed without starting a provider.",
            Some(expected_fingerprint),
            if assignment.checkpoint_verification {
                None
            } else {
                assignment.expected_head_commit.as_deref()
            },
            Some(&mut cleanup_head_commit),
            Some(&mut cancellation_rx),
        );
        tokio::pin!(verification);
        let outcome = loop {
            tokio::select! {
                biased;
                control = controls.recv(), if interruption.is_none() => {
                    match control {
                        Some(AdapterControl::Interrupt { reason })
                        | Some(AdapterControl::Stop { reason }) => {
                            interruption = Some(Ok(reason));
                            let _ = cancellation_tx.send(true);
                        }
                        Some(AdapterControl::CircuitBreaker { stage, reason })
                            if matches!(stage.as_str(), "suspend" | "stop") =>
                        {
                            interruption =
                                Some(Ok(format!("circuit breaker {stage}: {reason}")));
                            let _ = cancellation_tx.send(true);
                        }
                        Some(_) => {}
                        None => {
                            interruption = Some(Err(
                                "verifier-only control channel closed during verification"
                                    .to_owned(),
                            ));
                            let _ = cancellation_tx.send(true);
                        }
                    }
                }
                outcome = &mut verification => break outcome,
            }
        };
        Ok((outcome, interruption))
    }
    .await;

    let mut failures = Vec::new();
    let finished = matches!(&result, Ok((VerificationRunOutcome::Finished, _)));
    if !finished {
        if let Some(baseline) = verification_baseline.as_mut()
            && let Err(error) = cleanup_verification_snapshots(
                &mut active_check_snapshot,
                baseline,
                &mut artifact_snapshot,
            )
            .await
        {
            failures.push(format!("verifier-only snapshot cleanup failed: {error:#}"));
        }
        if let Err(error) = verify_preserved_workspace_checkpoint(
            &workspaces,
            &workspace,
            assignment.expected_workspace_fingerprint.as_deref(),
            cleanup_head_commit.as_deref(),
        )
        .await
        {
            workspace_quarantined = true;
            failures.push(format!("verifier-only source is quarantined: {error:#}"));
        }
    }
    // Finished means cleanup and the last checkpoint check preceded the synchronous success
    // event batch. Never perform another fallible integrity check after accepted completion.
    workspace_quarantined |= matches!(&result, Ok((VerificationRunOutcome::IntegrityFailed, _)));
    // Artifact/check failures may retain the exact admitted checkpoint for a later retry.
    // Integrity rejection is sticky: even if the source changes back, never mint a new checkpoint.
    let trusted_fingerprint =
        if checkpoint_admitted && failures.is_empty() && !workspace_quarantined {
            assignment.expected_workspace_fingerprint.as_deref()
        } else {
            None
        };
    match result {
        Err(error) => failures.push(format!("verifier-only recovery failed: {error:#}")),
        Ok((VerificationRunOutcome::Cancelled, interruption)) if failures.is_empty() => {
            match interruption.unwrap_or_else(|| {
                Err("verifier-only execution cancelled without a control reason".to_owned())
            }) {
                Ok(reason) => send_run_event(
                    &outbound,
                    &runner_id,
                    &assignment,
                    "run.cancelled",
                    json!({"reason": reason}),
                ),
                Err(error) => send_run_event(
                    &outbound,
                    &runner_id,
                    &assignment,
                    "run.failed",
                    json!({"error": error}),
                ),
            }
        }
        Ok(_) => {}
    }
    if !failures.is_empty() {
        send_run_event(
            &outbound,
            &runner_id,
            &assignment,
            "run.failed",
            json!({"error": failures.join("; ")}),
        );
    }
    send_teardown_workspace_preserved(
        &outbound,
        &runner_id,
        &assignment,
        &workspace,
        if workspace_quarantined {
            "verifier-only source workspace retained in quarantine; no new fingerprint admitted"
        } else if trusted_fingerprint.is_some() {
            "verifier-only recovery retained the exact authorized source workspace"
        } else {
            "verifier-only source workspace retained without an admitted checkpoint"
        },
        trusted_fingerprint,
        workspace_quarantined,
    );
    Ok(())
}

async fn checkpoint_preserved_workspace(
    workspaces: Arc<WorkspaceManager>,
    runner_id: String,
    assignment: Assignment,
    outbound: OutboundBus,
) -> Result<()> {
    validate_assignment_source(&workspaces, &assignment)?;
    let workspace = workspaces
        .prepare(
            assignment.task_id,
            assignment.workspace_run_id,
            assignment.source_base_commit.as_deref(),
            assignment.resume_workspace_base_commit.as_deref(),
        )
        .await
        .context("prepare preserved workspace checkpoint")?;
    let checkpoint = async {
        let expected_head = assignment
            .expected_head_commit
            .as_deref()
            .context("workspace checkpoint omitted expected head commit")?;
        let actual_head = workspaces.head_commit(&workspace).await?;
        if actual_head != expected_head {
            return Err(anyhow!(
                "workspace checkpoint head mismatch: expected {expected_head}, found {actual_head}"
            ));
        }
        Ok((workspaces.fingerprint(&workspace).await?, actual_head))
    }
    .await;
    let (fingerprint, actual_head) = match checkpoint {
        Ok(checkpoint) => checkpoint,
        Err(error) => {
            send_run_event(
                &outbound,
                &runner_id,
                &assignment,
                "run.failed",
                json!({"error": format!("{error:#}")}),
            );
            send_teardown_workspace_preserved(
                &outbound,
                &runner_id,
                &assignment,
                &workspace,
                &format!("workspace checkpoint quarantined: {error:#}"),
                None,
                true,
            );
            return Err(error);
        }
    };
    send_run_event(
        &outbound,
        &runner_id,
        &assignment,
        "run.workspace_preserved",
        json!({
            "workspace": workspace.path,
            "workspace_branch": workspace.branch,
            "workspace_base_ref": workspace.base_ref,
            "workspace_base_commit": workspace.base_commit,
            "detail": "preserved legacy workspace fingerprint checkpointed for governed recovery",
            "dirty": Value::Null,
            "commits_ahead": Value::Null,
            "branch_deleted": false,
            "workspace_fingerprint": fingerprint,
            "workspace_quarantined": false,
            "head_commit": actual_head,
        }),
    );
    Ok(())
}

struct PreparedVerificationArtifact {
    artifact: AdapterArtifact,
    source_artifact: Option<AdapterArtifact>,
}

async fn verifier_only_artifact(
    workspace: &WorkspaceLease,
    artifact_snapshot: &VerificationSnapshot,
    reference: &VerificationArtifactReference,
) -> Result<PreparedVerificationArtifact> {
    let (bytes, source_path) = if reference.data_base64.is_some() {
        let bytes = decode_verification_artifact(reference)?;
        // Preserve source-export exclusions only for a matching contained original file.
        // A missing/external provider file must not prevent using its signed transferred bytes.
        let source_path = read_contained_verifier_artifact(workspace, reference, false)
            .await
            .ok()
            .map(|(path, _)| path);
        (bytes, source_path)
    } else {
        let (path, bytes) = read_contained_verifier_artifact(workspace, reference, true).await?;
        (bytes, Some(path))
    };
    let artifact_root = tokio::fs::canonicalize(artifact_snapshot.path())
        .await
        .context("canonicalize private verifier artifact snapshot")?;
    let source_root = tokio::fs::canonicalize(&workspace.path)
        .await
        .context("canonicalize preserved verifier source")?;
    if artifact_root.starts_with(&source_root) || source_root.starts_with(&artifact_root) {
        return Err(anyhow!(
            "verifier artifact snapshot overlaps preserved source"
        ));
    }
    // This snapshot starts empty and is never copied into the sealed source baseline or a check.
    // reference.path is read-only legacy metadata, never a destination for transferred bytes.
    let path = artifact_root.join(format!("provider-evidence-{}.bin", Uuid::new_v4().simple()));
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .await
        .context("create private verifier artifact")?;
    file.write_all(&bytes)
        .await
        .context("stage transferred verifier artifact")?;
    file.flush()
        .await
        .context("finish staging transferred verifier artifact")?;
    Ok(PreparedVerificationArtifact {
        artifact: AdapterArtifact {
            path,
            sha256: reference.sha256.clone(),
            bytes: reference.bytes,
            media_type: reference.media_type.clone(),
        },
        source_artifact: source_path.map(|path| AdapterArtifact {
            path,
            sha256: reference.sha256.clone(),
            bytes: reference.bytes,
            media_type: reference.media_type.clone(),
        }),
    })
}

async fn read_contained_verifier_artifact(
    workspace: &WorkspaceLease,
    reference: &VerificationArtifactReference,
    search_legacy_name: bool,
) -> Result<(PathBuf, Vec<u8>)> {
    if reference.bytes > MAX_VERIFICATION_ARTIFACT_BYTES {
        return Err(anyhow!("legacy verifier artifact exceeds the byte limit"));
    }
    let requested = PathBuf::from(&reference.path);
    let allow_legacy_search =
        search_legacy_name && !requested.is_absolute() && requested.components().count() == 1;
    let candidate = if requested.is_absolute() {
        requested.clone()
    } else {
        workspace.path.join(&requested)
    };
    let root = tokio::fs::canonicalize(&workspace.path)
        .await
        .context("canonicalize verifier-only workspace")?;
    let path = match tokio::fs::canonicalize(&candidate).await {
        Ok(path) => path,
        Err(error) if allow_legacy_search => workspace::find_file_by_digest(
            &root,
            &reference.path,
            &reference.sha256,
            reference.bytes,
        )
        .await
        .with_context(|| {
            format!(
                "locate legacy verifier-only artifact after {} could not be resolved: {error}",
                candidate.display()
            )
        })?,
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "canonicalize verifier-only artifact {}",
                    candidate.display()
                )
            });
        }
    };
    if !path.starts_with(&root) {
        return Err(anyhow!(
            "verifier-only artifact escapes the preserved workspace"
        ));
    }
    let metadata = tokio::fs::metadata(&path)
        .await
        .context("inspect verifier-only artifact")?;
    if !metadata.is_file() || metadata.len() as usize != reference.bytes {
        return Err(anyhow!(
            "verifier-only artifact metadata does not match the preserved source run"
        ));
    }
    let file = tokio::fs::File::open(&path)
        .await
        .context("open contained legacy verifier artifact")?;
    let mut bytes = Vec::new();
    file.take(MAX_VERIFICATION_ARTIFACT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .await
        .context("read bounded legacy verifier artifact")?;
    validate_verification_artifact_bytes(reference, &bytes)?;
    Ok((path, bytes))
}

fn decode_verification_artifact(reference: &VerificationArtifactReference) -> Result<Vec<u8>> {
    let encoded = reference
        .data_base64
        .as_deref()
        .context("transferred verifier artifact omitted inline data")?;
    if reference.bytes > MAX_VERIFICATION_ARTIFACT_BYTES
        || encoded.len() > MAX_VERIFICATION_ARTIFACT_BYTES.div_ceil(3) * 4
    {
        return Err(anyhow!(
            "transferred verifier artifact exceeds the byte limit"
        ));
    }
    let bytes = BASE64
        .decode(encoded)
        .context("decode transferred verifier artifact")?;
    validate_verification_artifact_bytes(reference, &bytes)?;
    Ok(bytes)
}

fn validate_verification_artifact_bytes(
    reference: &VerificationArtifactReference,
    bytes: &[u8],
) -> Result<()> {
    if bytes.len() > MAX_VERIFICATION_ARTIFACT_BYTES || bytes.len() != reference.bytes {
        return Err(anyhow!("transferred verifier artifact byte count mismatch"));
    }
    if hex::encode(sha2::Sha256::digest(bytes)) != reference.sha256 {
        return Err(anyhow!("transferred verifier artifact SHA-256 mismatch"));
    }
    Ok(())
}

fn validate_assignment_source(
    workspaces: &WorkspaceManager,
    assignment: &Assignment,
) -> Result<()> {
    workspaces
        .verify_source_identity(
            assignment.source_repository.as_deref(),
            assignment.source_base_ref.as_deref(),
            assignment.source_base_commit.as_deref(),
        )
        .context("reject source assignment")
}

enum IsolatedVerificationOutcome {
    Report(verifier::VerificationReport),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerificationRunOutcome {
    Finished,
    Failed,
    IntegrityFailed,
    Cancelled,
}

async fn verify_isolated_recovery_checks(
    policy: &VerificationPolicy,
    run_id: Uuid,
    baseline: &mut VerificationSnapshot,
    active_check_snapshot: &mut Option<VerificationSnapshot>,
    artifact_snapshot: &mut Option<VerificationSnapshot>,
    artifacts: &[AdapterArtifact],
    cancellation: &mut watch::Receiver<bool>,
) -> Result<IsolatedVerificationOutcome> {
    let mut checks = Vec::with_capacity(policy.checks.len());
    for (index, check) in policy.checks.iter().enumerate() {
        let result = if matches!(
            check,
            crony_domain::VerifierCheck::Command { .. } | crony_domain::VerifierCheck::Test { .. }
        ) {
            *active_check_snapshot =
                Some(workspace::verification_snapshot(baseline.path(), run_id).await?);
            let check_workspace = active_check_snapshot
                .as_ref()
                .context("verifier check snapshot was not created")?
                .path()
                .to_owned();
            let result = verifier::run_check_cancellable(
                index as i32,
                check,
                &check_workspace,
                artifacts,
                cancellation,
            )
            .await;
            if matches!(result, verifier::CancellableCheckResult::Cancelled) {
                cleanup_verification_snapshots(active_check_snapshot, baseline, artifact_snapshot)
                    .await
                    .context("clean cancelled verifier snapshots")?;
                return Ok(IsolatedVerificationOutcome::Cancelled);
            }
            active_check_snapshot
                .as_mut()
                .context("verifier check snapshot disappeared before cleanup")?
                .cleanup()
                .await
                .with_context(|| format!("clean verifier snapshot after check {index}"))?;
            *active_check_snapshot = None;
            match result {
                verifier::CancellableCheckResult::Completed(result) => result,
                verifier::CancellableCheckResult::Cancelled => unreachable!(),
            }
        } else {
            match verifier::run_check_cancellable(
                index as i32,
                check,
                baseline.path(),
                artifacts,
                cancellation,
            )
            .await
            {
                verifier::CancellableCheckResult::Completed(result) => result,
                verifier::CancellableCheckResult::Cancelled => {
                    cleanup_verification_snapshots(
                        active_check_snapshot,
                        baseline,
                        artifact_snapshot,
                    )
                    .await
                    .context("clean cancelled verifier snapshots")?;
                    return Ok(IsolatedVerificationOutcome::Cancelled);
                }
            }
        };
        checks.push(result);
    }
    cleanup_verification_snapshots(active_check_snapshot, baseline, artifact_snapshot)
        .await
        .context("clean sealed verifier baseline and private evidence")?;
    let failed = checks.iter().filter(|check| !check.passed).count();
    Ok(IsolatedVerificationOutcome::Report(
        verifier::VerificationReport {
            passed: failed == 0,
            summary: if failed == 0 {
                format!("all {} verifier checks passed", checks.len())
            } else {
                format!("{failed} of {} verifier checks failed", checks.len())
            },
            checks,
            manual_gate: policy.manual_gate.clone(),
        },
    ))
}

async fn cleanup_verification_snapshots(
    active_check_snapshot: &mut Option<VerificationSnapshot>,
    baseline: &mut VerificationSnapshot,
    artifact_snapshot: &mut Option<VerificationSnapshot>,
) -> Result<()> {
    let mut failures = Vec::new();
    if let Some(snapshot) = active_check_snapshot.as_mut() {
        match snapshot.cleanup().await {
            Ok(()) => *active_check_snapshot = None,
            Err(error) => failures.push(format!("active check snapshot: {error:#}")),
        }
    }
    if let Err(error) = baseline.cleanup().await {
        failures.push(format!("sealed verifier baseline: {error:#}"));
    }
    if let Some(snapshot) = artifact_snapshot.as_mut() {
        match snapshot.cleanup().await {
            Ok(()) => *artifact_snapshot = None,
            Err(error) => failures.push(format!("private verifier artifacts: {error:#}")),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(failures.join("; ")))
    }
}

#[allow(clippy::too_many_arguments)]
async fn send_verification_events(
    outbound: &OutboundBus,
    runner_id: &str,
    assignment: &Assignment,
    workspace: &WorkspaceLease,
    workspaces: &WorkspaceManager,
    verification_baseline: Option<&mut VerificationSnapshot>,
    active_check_snapshot: Option<&mut Option<VerificationSnapshot>>,
    artifact_snapshot: Option<&mut Option<VerificationSnapshot>>,
    artifacts: &Arc<Mutex<Vec<AdapterArtifact>>>,
    source_artifacts: Option<&[AdapterArtifact]>,
    artifact_acks: &mut mpsc::UnboundedReceiver<ArtifactAck>,
    completion_summary: &str,
    expected_workspace_fingerprint: Option<&str>,
    preserve_head_commit: Option<&str>,
    mut cleanup_head_commit: Option<&mut Option<String>>,
    mut cancellation: Option<&mut watch::Receiver<bool>>,
) -> VerificationRunOutcome {
    send_run_event(
        outbound,
        runner_id,
        assignment,
        "run.verification_started",
        json!({
            "check_count": assignment.verification_policy.checks.len(),
        }),
    );
    // Admission still binds the original checkpoint HEAD. Only the exporter may
    // replace it with the exact runner-owned verification commit after checks.
    let mut completion_head_commit = preserve_head_commit
        .or(expected_workspace_fingerprint.and(assignment.expected_head_commit.as_deref()))
        .map(str::to_owned);
    if expected_workspace_fingerprint.is_some() && completion_head_commit.is_none() {
        match workspaces.head_commit(workspace).await {
            Ok(head) => completion_head_commit = Some(head),
            Err(error) => {
                send_run_event(
                    outbound,
                    runner_id,
                    assignment,
                    "run.failed",
                    json!({"error": format!("verifier-only checkpoint head could not be read: {error:#}")}),
                );
                return VerificationRunOutcome::IntegrityFailed;
            }
        }
    }
    let artifacts = artifacts
        .lock()
        .map(|artifacts| artifacts.clone())
        .unwrap_or_default();
    let mut report = match (
        verification_baseline,
        active_check_snapshot,
        artifact_snapshot,
    ) {
        (Some(baseline), Some(active_check_snapshot), Some(artifact_snapshot)) => {
            match verify_isolated_recovery_checks(
                &assignment.verification_policy,
                assignment.run_id,
                baseline,
                active_check_snapshot,
                artifact_snapshot,
                &artifacts,
                cancellation
                    .as_deref_mut()
                    .expect("isolated verification requires cancellation state"),
            )
            .await
            {
                Ok(IsolatedVerificationOutcome::Report(report)) => report,
                Ok(IsolatedVerificationOutcome::Cancelled) => {
                    return VerificationRunOutcome::Cancelled;
                }
                Err(error) => {
                    let cleanup = cleanup_verification_snapshots(
                        active_check_snapshot,
                        baseline,
                        artifact_snapshot,
                    )
                    .await;
                    send_run_event(
                        outbound,
                        runner_id,
                        assignment,
                        "run.failed",
                        json!({
                            "error": format!(
                                "verifier-only isolated verification failed: {error:#}{}",
                                cleanup.err().map_or_else(String::new, |cleanup_error| {
                                    format!("; snapshot cleanup also failed: {cleanup_error:#}")
                                }),
                            ),
                        }),
                    );
                    return VerificationRunOutcome::Failed;
                }
            }
        }
        (None, None, None) => {
            if let Some(cancellation) = cancellation.as_deref_mut() {
                match verifier::verify_cancellable(
                    &assignment.verification_policy,
                    &workspace.path,
                    &artifacts,
                    cancellation,
                )
                .await
                {
                    Some(report) => report,
                    None => return VerificationRunOutcome::Cancelled,
                }
            } else {
                verifier::verify(&assignment.verification_policy, &workspace.path, &artifacts).await
            }
        }
        _ => {
            send_run_event(
                outbound,
                runner_id,
                assignment,
                "run.failed",
                json!({"error": "verifier snapshot state was incomplete"}),
            );
            return VerificationRunOutcome::Failed;
        }
    };
    if cancellation
        .as_ref()
        .is_some_and(|receiver| *receiver.borrow())
    {
        return VerificationRunOutcome::Cancelled;
    }
    let mut integrity_failed = false;
    if let Err(error) = verify_preserved_workspace_checkpoint(
        workspaces,
        workspace,
        expected_workspace_fingerprint,
        completion_head_commit.as_deref(),
    )
    .await
    {
        integrity_failed = true;
        if let Some(first) = report.checks.first_mut() {
            first.passed = false;
            first.summary = format!("{error:#}");
            first.payload = json!({"error": format!("{error:#}")});
        }
        report.passed = false;
        report.summary = "verifier-only workspace integrity check failed".to_owned();
    }
    for check in &report.checks {
        send_run_event(
            outbound,
            runner_id,
            assignment,
            "run.verification_evidence",
            json!({
                "evidence_id": Uuid::new_v4(),
                "check_index": check.check_index,
                "kind": check.kind,
                "status": if check.passed { "passed" } else { "failed" },
                "summary": check.summary,
                "payload": check.payload,
            }),
        );
    }
    if !report.passed {
        send_run_event(
            outbound,
            runner_id,
            assignment,
            "run.verification_failed",
            json!({
                "error": report.summary,
                "completion_summary": completion_summary,
            }),
        );
        return if integrity_failed {
            VerificationRunOutcome::IntegrityFailed
        } else {
            VerificationRunOutcome::Failed
        };
    }
    if cancellation
        .as_ref()
        .is_some_and(|receiver| *receiver.borrow())
    {
        return VerificationRunOutcome::Cancelled;
    }
    let deliverable_linkage = if let Some(spec) = &assignment.deliverable {
        let exported = match deliverable::export(
            assignment.run_id,
            spec,
            workspace,
            &report,
            source_artifacts.unwrap_or(&artifacts),
            &assignment.write_scope,
            preserve_head_commit,
        )
        .await
        {
            Ok(exported) => exported,
            Err(error) => {
                send_run_event(
                    outbound,
                    runner_id,
                    assignment,
                    "run.failed",
                    json!({
                        "error": format!("verified deliverable export failed: {error:#}"),
                    }),
                );
                return VerificationRunOutcome::Failed;
            }
        };
        if expected_workspace_fingerprint.is_some()
            && preserve_head_commit.is_none()
            && let Some(head) = exported.head_commit.as_ref()
        {
            // A first authorized commit may be created by export. Bind that exact produced head
            // for the upload wait, without ever replacing an explicitly preserved head.
            completion_head_commit = Some(head.clone());
            // ACK failure or cancellation must retain this exact runner-owned head,
            // not mistake it for source tampering. Never read/adopt a cleanup-time HEAD.
            if let Some(cleanup_guard) = cleanup_head_commit.as_mut() {
                **cleanup_guard = completion_head_commit.clone();
            }
        }
        if cancellation
            .as_ref()
            .is_some_and(|receiver| *receiver.borrow())
        {
            return VerificationRunOutcome::Cancelled;
        }
        if let Err(error) = verify_preserved_workspace_checkpoint(
            workspaces,
            workspace,
            expected_workspace_fingerprint,
            completion_head_commit.as_deref(),
        )
        .await
        {
            send_run_event(
                outbound,
                runner_id,
                assignment,
                "run.failed",
                json!({"error": format!("verifier-only checkpoint changed during deliverable export: {error:#}")}),
            );
            return VerificationRunOutcome::IntegrityFailed;
        }
        let deliverable_sha256 = hex::encode(sha2::Sha256::digest(&exported.bytes));
        let upload_payload = json!({
            "sha256": deliverable_sha256,
            "bytes": exported.bytes.len(),
            "media_type": exported.media_type,
            "content_base64": BASE64.encode(&exported.bytes),
            "file_name": exported.file_name,
            "artifact_role": "source_deliverable",
            "form": exported.form.as_str(),
            "verification_sha256": exported.verification_sha256,
            "base_commit": exported.base_commit,
            "head_commit": exported.head_commit,
            "branch": exported.branch,
            "git_bundle_sha256": exported.git_bundle_sha256,
            "publication_ready": exported.publication_ready,
            "integration_state": if exported.form == crony_domain::DeliverableForm::ReviewOnlyReport {
                "not_applicable"
            } else {
                "ready_for_review"
            },
        });
        let mut stored = None;
        for _ in 0..6 {
            if cancellation
                .as_ref()
                .is_some_and(|receiver| *receiver.borrow())
            {
                return VerificationRunOutcome::Cancelled;
            }
            send_run_event(
                outbound,
                runner_id,
                assignment,
                "run.deliverable_upload",
                upload_payload.clone(),
            );
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            loop {
                let attempt = if let Some(cancellation) = cancellation.as_deref_mut() {
                    tokio::select! {
                        biased;
                        () = verifier::wait_for_verifier_cancellation(cancellation) => {
                            return VerificationRunOutcome::Cancelled;
                        }
                        attempt = tokio::time::timeout_at(deadline, artifact_acks.recv()) => attempt,
                    }
                } else {
                    tokio::time::timeout_at(deadline, artifact_acks.recv()).await
                };
                // Every ACK (including an unrelated one) is an asynchronous boundary, not an
                // integrity receipt. Keep this check outside the upload-wait timeout itself.
                if let Err(error) = verify_preserved_workspace_checkpoint(
                    workspaces,
                    workspace,
                    expected_workspace_fingerprint,
                    completion_head_commit.as_deref(),
                )
                .await
                {
                    send_run_event(
                        outbound,
                        runner_id,
                        assignment,
                        "run.failed",
                        json!({"error": format!("verifier-only checkpoint changed while awaiting deliverable acknowledgment: {error:#}")}),
                    );
                    return VerificationRunOutcome::IntegrityFailed;
                }
                match attempt {
                    Ok(Some(ack))
                        if ack.artifact_role == "source_deliverable"
                            && ack.sha256 == deliverable_sha256 =>
                    {
                        stored = Some(ack.artifact_id);
                        break;
                    }
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        send_run_event(
                            outbound,
                            runner_id,
                            assignment,
                            "run.failed",
                            json!({"error": "source deliverable acknowledgment failed: artifact acknowledgment channel closed"}),
                        );
                        return VerificationRunOutcome::Failed;
                    }
                    Err(_) => break,
                }
            }
            if stored.is_some() {
                break;
            }
        }
        if stored.is_none() {
            send_run_event(
                outbound,
                runner_id,
                assignment,
                "run.failed",
                json!({"error": "source deliverable storage acknowledgment timed out"}),
            );
            return VerificationRunOutcome::Failed;
        }
        Some((exported.verification_sha256, deliverable_sha256))
    } else {
        None
    };
    // This is the last awaited operation before the synchronous success-event batch.
    // In particular, an uploaded artifact must never make a drifted workspace acceptable.
    if let Err(error) = verify_preserved_workspace_checkpoint(
        workspaces,
        workspace,
        expected_workspace_fingerprint,
        completion_head_commit.as_deref(),
    )
    .await
    {
        send_run_event(
            outbound,
            runner_id,
            assignment,
            "run.failed",
            json!({"error": format!("verifier-only checkpoint changed before verification acceptance: {error:#}")}),
        );
        return VerificationRunOutcome::IntegrityFailed;
    }
    // Controls can arrive during the final checkpoint read.
    if cancellation
        .as_ref()
        .is_some_and(|receiver| *receiver.borrow())
    {
        return VerificationRunOutcome::Cancelled;
    }
    let verification_payload = match deliverable_linkage {
        Some((verification_sha256, deliverable_sha256)) => json!({
            "summary": report.summary,
            "verification_sha256": verification_sha256,
            "deliverable_sha256": deliverable_sha256,
        }),
        None => json!({"summary": report.summary}),
    };
    send_run_event(
        outbound,
        runner_id,
        assignment,
        "run.verification_passed",
        verification_payload,
    );
    if let Some(gate) = report.manual_gate {
        send_run_event(
            outbound,
            runner_id,
            assignment,
            "run.verification_waiting",
            json!({
                "gate_type": gate.kind(),
                "gate": gate,
                "verification_summary": report.summary,
                "completion_summary": completion_summary,
            }),
        );
    } else {
        send_run_event(
            outbound,
            runner_id,
            assignment,
            "run.completed",
            json!({"summary": completion_summary}),
        );
    }
    VerificationRunOutcome::Finished
}

fn send_workspace_cleanup_event(
    outbound: &OutboundBus,
    runner_id: &str,
    assignment: &Assignment,
    workspace: &WorkspaceLease,
    cleanup: Result<WorkspaceCleanup>,
    workspace_fingerprint: Option<&str>,
) {
    let (event_type, cleanup) = match cleanup {
        Ok(cleanup) => {
            let event_type = match cleanup.disposition {
                WorkspaceDisposition::Removed => "run.workspace_removed",
                WorkspaceDisposition::Preserved => "run.workspace_preserved",
            };
            (event_type, cleanup)
        }

        Err(error) => (
            "run.workspace_preserved",
            WorkspaceCleanup {
                disposition: WorkspaceDisposition::Preserved,
                detail: format!("cleanup verification failed; worktree preserved: {error:#}"),
                dirty: None,
                commits_ahead: None,
                branch_deleted: false,
            },
        ),
    };
    send_run_event(
        outbound,
        runner_id,
        assignment,
        event_type,
        json!({
            "workspace": workspace.path,
            "workspace_branch": workspace.branch,
            "workspace_base_ref": workspace.base_ref,
            "workspace_base_commit": workspace.base_commit,
            "detail": cleanup.detail,
            "dirty": cleanup.dirty,
            "commits_ahead": cleanup.commits_ahead,
            "branch_deleted": cleanup.branch_deleted,
            "workspace_fingerprint": workspace_fingerprint,
        }),
    );
}

fn send_teardown_workspace_preserved(
    outbound: &OutboundBus,
    runner_id: &str,
    assignment: &Assignment,
    workspace: &WorkspaceLease,
    detail: &str,
    workspace_fingerprint: Option<&str>,
    workspace_quarantined: bool,
) {
    let workspace_fingerprint = if workspace_quarantined {
        None
    } else {
        workspace_fingerprint
    };
    send_run_event(
        outbound,
        runner_id,
        assignment,
        "run.workspace_preserved",
        json!({
            "workspace": workspace.path,
            "workspace_branch": workspace.branch,
            "workspace_base_ref": workspace.base_ref,
            "workspace_base_commit": workspace.base_commit,
            "detail": detail,
            "dirty": Value::Null,
            "commits_ahead": Value::Null,
            "branch_deleted": false,
            "workspace_fingerprint": workspace_fingerprint,
            "workspace_quarantined": workspace_quarantined,
        }),
    );
}

fn send_run_event(
    outbound: &OutboundBus,
    runner_id: &str,
    assignment: &Assignment,
    event_type: &str,
    payload: Value,
) {
    let message = RunnerToServer::RunEvent {
        event_id: Uuid::new_v4(),
        runner_id: runner_id.to_owned(),
        corp_id: assignment.corp_id,
        connection_epoch: assignment.connection_epoch,
        run_id: assignment.run_id,
        agent_id: assignment.agent_id,
        assignment_token: assignment.assignment_token,
        event_type: event_type.to_owned(),
        payload,
    };
    outbound.send(message.clone());
    if event_type == "run.started" && assignment.mission_title.contains("[duplicate-event]") {
        outbound.send(message);
    }
}

fn send_command_ack(
    outbound: &OutboundBus,
    runner_id: &str,
    connection_epoch: Uuid,
    command_id: Option<Uuid>,
    applied: bool,
    detail: &str,
) {
    let Some(command_id) = command_id else {
        return;
    };
    outbound.send(RunnerToServer::CommandAck {
        runner_id: runner_id.to_owned(),
        connection_epoch,
        command_id,
        applied,
        detail: detail.to_owned(),
    });
}

fn active_run_claims(active_runs: &ActiveRuns) -> Vec<ActiveRunClaim> {
    let mut claims = active_runs
        .iter()
        .map(|entry| ActiveRunClaim {
            run_id: *entry.key(),
            assignment_token: entry.assignment_token,
        })
        .collect::<Vec<_>>();
    claims.sort_by_key(|claim| claim.run_id);
    claims
}

#[cfg(test)]
mod tests {
    use std::{path::Path, process::Command as StdCommand};

    use async_trait::async_trait;
    use chrono::Duration as ChronoDuration;
    use tokio::sync::Notify;

    use super::*;

    struct LatchedTeardownAdapter {
        uncertain: Arc<Notify>,
        verified: Arc<Notify>,
    }

    #[async_trait]
    impl AgentAdapter for LatchedTeardownAdapter {
        fn id(&self) -> &'static str {
            "latched-teardown"
        }

        fn display_name(&self) -> &'static str {
            "Latched teardown test adapter"
        }

        fn capabilities(&self) -> adapter::AdapterCapabilities {
            let supported = adapter::FeatureSupport::Supported;
            adapter::AdapterCapabilities {
                spawn: supported.clone(),
                stream: supported.clone(),
                steer: supported.clone(),
                interrupt: supported.clone(),
                stop: supported.clone(),
                resume: supported.clone(),
                usage: supported.clone(),
                artifacts: supported,
            }
        }

        async fn execute(
            &self,
            request: AdapterRunRequest,
            _controls: mpsc::UnboundedReceiver<AdapterControl>,
            sink: Arc<dyn AdapterEventSink>,
        ) -> Result<AdapterExit, adapter::AdapterError> {
            sink.emit(AdapterEvent::Started {
                workspace: request.workspace,
            });
            sink.emit(AdapterEvent::TeardownUncertain {
                detail: "injected latched provider-scope query failure".to_owned(),
            });
            self.uncertain.notify_one();
            self.verified.notified().await;
            Err(adapter::AdapterError::Runtime(anyhow!(
                "verified teardown runtime failure"
            )))
        }
    }

    fn git(cwd: &Path, args: &[&str]) -> String {
        let output = StdCommand::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("run git fixture command");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    fn teardown_fixture() -> (PathBuf, PathBuf, PathBuf) {
        let root = std::env::temp_dir()
            .join("crony-runner-teardown-tests")
            .join(Uuid::new_v4().to_string());
        let repository = root.join("source");
        let managed = root.join("managed");
        std::fs::create_dir_all(&repository).expect("create source repository");
        git(&repository, &["init", "-b", "main"]);
        std::fs::write(repository.join("sentinel.txt"), b"preserve exact bytes\n")
            .expect("write sentinel");
        git(&repository, &["add", "sentinel.txt"]);
        git(
            &repository,
            &[
                "-c",
                "user.name=ECorp Test",
                "-c",
                "user.email=crony@example.invalid",
                "commit",
                "-m",
                "init",
            ],
        );
        (root, repository, managed)
    }

    fn secret(expires_at: DateTime<Utc>) -> ResolvedSecret {
        ResolvedSecret {
            grant_id: Uuid::new_v4(),
            secret_id: Uuid::new_v4(),
            env_name: "CRONY_TEST_SECRET".to_owned(),
            value: "redacted-test-value".to_owned(),
            tool: "test".to_owned(),
            resource: "test://secret".to_owned(),
            expires_at: expires_at.to_rfc3339(),
            assurance: "environment_reduced_assurance".to_owned(),
        }
    }

    #[test]
    fn expired_secrets_fail_before_provider_start() {
        assert!(secret_expiry_delay(&[secret(Utc::now() - ChronoDuration::seconds(1))]).is_err());
    }

    #[test]
    fn active_secrets_schedule_a_bounded_provider_lifetime() {
        let ttl = secret_expiry_delay(&[secret(Utc::now() + ChronoDuration::seconds(60))])
            .expect("valid secret")
            .expect("secret expiry");
        assert!(ttl <= Duration::from_secs(60));
        assert!(ttl > Duration::from_secs(50));
    }

    fn verification_artifact_reference(path: &str, bytes: &[u8]) -> VerificationArtifactReference {
        VerificationArtifactReference {
            path: path.to_owned(),
            sha256: hex::encode(sha2::Sha256::digest(bytes)),
            bytes: bytes.len(),
            media_type: "application/json".to_owned(),
            data_base64: Some(BASE64.encode(bytes)),
        }
    }

    fn verification_source_fixture() -> (PathBuf, WorkspaceLease) {
        let root = std::env::temp_dir()
            .join("crony-verification-artifact-tests")
            .join(Uuid::new_v4().to_string());
        let source = root.join("source");
        std::fs::create_dir_all(&source).expect("create owned source fixture");
        std::fs::write(source.join("sentinel.txt"), b"exact original source\n")
            .expect("write source sentinel");
        (
            root,
            WorkspaceLease {
                path: source,
                branch: "crony/test-verifier-artifact".to_owned(),
                base_ref: "HEAD".to_owned(),
                base_commit: "0".repeat(40),
            },
        )
    }

    fn verification_assignment(workspace: &WorkspaceLease, run_id: Uuid) -> Assignment {
        Assignment {
            workspace_connection_id: None,
            corp_id: Uuid::new_v4(),
            connection_epoch: Uuid::new_v4(),
            room_id: Uuid::new_v4(),
            mission_id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            run_id,
            workspace_run_id: Uuid::new_v4(),
            agent_id: Uuid::new_v4(),
            assignment_token: Uuid::new_v4(),
            adapter: "verification-only".to_owned(),
            mission_title: "Verifier artifact transfer regression".to_owned(),
            model: None,
            reasoning_effort: None,
            source_repository: None,
            source_base_ref: None,
            source_base_commit: None,
            resume_workspace_base_commit: Some(workspace.base_commit.clone()),
            verification_policy: VerificationPolicy {
                checks: vec![crony_domain::VerifierCheck::Artifact { min_bytes: 1 }],
                manual_gate: None,
            },
            write_scope: Vec::new(),
            deliverable: None,
            secrets: Vec::new(),
            expected_workspace_fingerprint: None,
            expected_head_commit: None,
            provider_artifact: None,
            checkpoint_verification: false,
            hard_boundary_checkpoint: Arc::default(),
        }
    }

    async fn prepared_verification_fixture()
    -> (PathBuf, Arc<WorkspaceManager>, WorkspaceLease, Assignment) {
        let (root, repository, managed) = teardown_fixture();
        let workspaces = Arc::new(
            WorkspaceManager::initialize(managed, repository, "HEAD".to_owned())
                .await
                .expect("initialize owned recovery fixture"),
        );
        let task_id = Uuid::new_v4();
        let workspace_run_id = Uuid::new_v4();
        let workspace = workspaces
            .prepare(task_id, workspace_run_id, None, None)
            .await
            .expect("prepare source workspace");
        let mut assignment = verification_assignment(&workspace, Uuid::new_v4());
        assignment.task_id = task_id;
        assignment.workspace_run_id = workspace_run_id;
        assignment.expected_workspace_fingerprint = Some(
            workspaces
                .fingerprint(&workspace)
                .await
                .expect("source fingerprint"),
        );
        assignment.expected_head_commit = Some(
            workspaces
                .head_commit(&workspace)
                .await
                .expect("source head"),
        );
        (root, workspaces, workspace, assignment)
    }

    fn recorded_run_events(outbound: &OutboundBus) -> Vec<(String, Value)> {
        let (connection, mut received) = mpsc::unbounded_channel();
        outbound.attach(connection, Uuid::new_v4());
        let mut events = Vec::new();
        while let Ok(message) = received.try_recv() {
            if let RunnerToServer::RunEvent {
                event_type,
                payload,
                ..
            } = message
            {
                events.push((event_type, payload));
            }
        }
        events
    }

    fn assert_no_verification_snapshots(run_id: Uuid) {
        let parent = std::env::temp_dir().join("ecorp-verification-snapshots");
        let entries = match std::fs::read_dir(parent) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => panic!("inspect snapshot cleanup: {error}"),
        };
        let prefix = format!("{}-", run_id.simple());
        for entry in entries {
            let entry = entry.expect("snapshot entry");
            assert!(
                !entry.file_name().to_string_lossy().starts_with(&prefix),
                "snapshot was not explicitly cleaned: {}",
                entry.path().display()
            );
        }
    }

    #[test]
    fn verifier_artifact_transfer_decodes_inline_bytes() {
        let bytes = br#"{"provider":"codex","evidence":"ready"}"#;
        let reference = verification_artifact_reference("not-a-destination.json", bytes);
        assert_eq!(decode_verification_artifact(&reference).unwrap(), bytes);
    }

    #[test]
    fn verifier_artifact_transfer_rejects_tampered_hash() {
        let mut reference = verification_artifact_reference("ignored.json", b"durable bytes");
        reference.sha256 = "0".repeat(64);
        assert!(
            decode_verification_artifact(&reference)
                .unwrap_err()
                .to_string()
                .contains("SHA-256 mismatch")
        );
    }

    #[test]
    fn verifier_artifact_transfer_bounds_encoded_decoded_and_declared_size() {
        let mut reference = verification_artifact_reference("ignored.json", b"abc");
        reference.bytes = 4;
        assert!(
            decode_verification_artifact(&reference)
                .unwrap_err()
                .to_string()
                .contains("byte count mismatch")
        );
        reference.bytes = MAX_VERIFICATION_ARTIFACT_BYTES + 1;
        reference.data_base64 = Some("!!!!".to_owned());
        assert!(
            decode_verification_artifact(&reference)
                .unwrap_err()
                .to_string()
                .contains("byte limit")
        );
        reference.bytes = 0;
        reference.data_base64 =
            Some("!".repeat(MAX_VERIFICATION_ARTIFACT_BYTES.div_ceil(3) * 4 + 1));
        assert!(
            decode_verification_artifact(&reference)
                .unwrap_err()
                .to_string()
                .contains("byte limit"),
            "encoded input must be bounded before base64 decoding"
        );
        reference.bytes = MAX_VERIFICATION_ARTIFACT_BYTES;
        reference.data_base64 = Some(BASE64.encode(vec![0; MAX_VERIFICATION_ARTIFACT_BYTES + 1]));
        assert!(
            decode_verification_artifact(&reference)
                .unwrap_err()
                .to_string()
                .contains("byte count mismatch"),
            "base64 padding can admit more decoded bytes than the limit"
        );
        reference.bytes = 3;
        reference.data_base64 = Some("!!!!".to_owned());
        assert!(
            decode_verification_artifact(&reference)
                .unwrap_err()
                .to_string()
                .contains("decode transferred")
        );
    }

    #[tokio::test]
    async fn verifier_artifact_transfer_keeps_exact_source_and_baseline() {
        let (root, workspace) = verification_source_fixture();
        let run_id = Uuid::new_v4();
        let fingerprint = workspace::fingerprint_path(&workspace.path).await.unwrap();
        let mut baseline = workspace::verification_snapshot(&workspace.path, run_id)
            .await
            .unwrap();
        let baseline_path = baseline.path().to_owned();
        let mut artifact_snapshot = Some(
            workspace::empty_verification_snapshot(run_id)
                .await
                .unwrap(),
        );
        let artifact_root = artifact_snapshot.as_ref().unwrap().path().to_owned();
        assert_eq!(std::fs::read_dir(&artifact_root).unwrap().count(), 0);
        let bytes = br#"{"provider":"codex","ready":true}"#;
        let reference = verification_artifact_reference("missing-provider.json", bytes);
        let mut rejected = reference.clone();
        rejected.sha256 = "0".repeat(64);
        assert!(
            verifier_only_artifact(&workspace, artifact_snapshot.as_ref().unwrap(), &rejected)
                .await
                .is_err()
        );
        rejected = reference.clone();
        rejected.bytes += 1;
        assert!(
            verifier_only_artifact(&workspace, artifact_snapshot.as_ref().unwrap(), &rejected)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read_dir(&artifact_root).unwrap().count(), 0);
        let prepared =
            verifier_only_artifact(&workspace, artifact_snapshot.as_ref().unwrap(), &reference)
                .await
                .unwrap();
        assert!(prepared.source_artifact.is_none());
        assert_eq!(
            prepared.artifact.path.parent().unwrap(),
            std::fs::canonicalize(&artifact_root).unwrap()
        );
        assert_ne!(
            prepared.artifact.path.file_name().unwrap(),
            std::ffi::OsStr::new("missing-provider.json")
        );
        assert_eq!(std::fs::read(&prepared.artifact.path).unwrap(), bytes);
        let forbidden_destination = root.join("never-write-inline-here.json");
        let absolute_reference =
            verification_artifact_reference(forbidden_destination.to_str().unwrap(), bytes);
        verifier_only_artifact(
            &workspace,
            artifact_snapshot.as_ref().unwrap(),
            &absolute_reference,
        )
        .await
        .unwrap();
        assert!(!forbidden_destination.exists());
        assert!(!workspace.path.join(&reference.path).exists());
        assert!(!baseline_path.join(&reference.path).exists());
        assert_eq!(
            workspace::fingerprint_path(&baseline_path).await.unwrap(),
            fingerprint
        );
        assert_eq!(
            workspace::fingerprint_path(&workspace.path).await.unwrap(),
            fingerprint
        );
        let policy = VerificationPolicy {
            checks: vec![
                crony_domain::VerifierCheck::Artifact { min_bytes: 1 },
                crony_domain::VerifierCheck::Test {
                    program: "node".to_owned(),
                    args: vec![
                        "-e".to_owned(),
                        "require('node:assert/strict').deepEqual(require('node:fs').readdirSync('.').sort(), ['sentinel.txt'])".to_owned(),
                    ],
                    timeout_ms: 5_000,
                },
                crony_domain::VerifierCheck::File {
                    path: reference.path,
                    min_bytes: 1,
                },
            ],
            manual_gate: None,
        };
        let (_cancel_tx, mut cancellation) = watch::channel(false);
        let mut active = None;
        let outcome = verify_isolated_recovery_checks(
            &policy,
            run_id,
            &mut baseline,
            &mut active,
            &mut artifact_snapshot,
            &[prepared.artifact],
            &mut cancellation,
        )
        .await
        .unwrap();
        let IsolatedVerificationOutcome::Report(report) = outcome else {
            panic!("expected verification report");
        };
        assert!(!report.passed);
        assert!(report.checks[0].passed, "transferred artifact must verify");
        assert!(report.checks[1].passed, "check directory must remain exact");
        assert!(
            !report.checks[2].passed,
            "missing source file must not be manufactured"
        );
        assert!(active.is_none());
        assert!(artifact_snapshot.is_none());
        assert!(!baseline_path.exists());
        assert!(!artifact_root.exists());
        assert_no_verification_snapshots(run_id);
        assert_eq!(
            workspace::fingerprint_path(&workspace.path).await.unwrap(),
            fingerprint
        );
        std::fs::remove_dir_all(root).expect("remove owned source fixture");
    }

    #[tokio::test]
    async fn verifier_artifact_transfer_stages_contained_legacy_bytes_privately() {
        let (root, workspace) = verification_source_fixture();
        let evidence = workspace.path.join("evidence");
        std::fs::create_dir(&evidence).unwrap();
        let bytes = br#"{"legacy":"provider evidence"}"#;
        std::fs::write(evidence.join("provider.json"), bytes).unwrap();
        let run_id = Uuid::new_v4();
        let fingerprint = workspace::fingerprint_path(&workspace.path).await.unwrap();
        let mut baseline = workspace::verification_snapshot(&workspace.path, run_id)
            .await
            .unwrap();
        let baseline_path = baseline.path().to_owned();
        let mut artifact_snapshot = Some(
            workspace::empty_verification_snapshot(run_id)
                .await
                .unwrap(),
        );
        let mut reference = verification_artifact_reference("provider.json", bytes);
        reference.data_base64 = None;
        let prepared =
            verifier_only_artifact(&workspace, artifact_snapshot.as_ref().unwrap(), &reference)
                .await
                .unwrap();
        assert_eq!(std::fs::read(&prepared.artifact.path).unwrap(), bytes);
        assert_eq!(
            prepared.source_artifact.unwrap().path,
            std::fs::canonicalize(evidence.join("provider.json")).unwrap(),
            "source export must still exclude the original provider evidence"
        );
        let inline = verification_artifact_reference("evidence/provider.json", bytes);
        assert!(
            verifier_only_artifact(&workspace, artifact_snapshot.as_ref().unwrap(), &inline)
                .await
                .unwrap()
                .source_artifact
                .is_some()
        );
        let outside = root.join("outside.json");
        std::fs::write(&outside, bytes).unwrap();
        reference.path = outside.to_string_lossy().into_owned();
        let error =
            verifier_only_artifact(&workspace, artifact_snapshot.as_ref().unwrap(), &reference)
                .await
                .err()
                .expect("legacy evidence outside the source must be rejected");
        assert!(
            error
                .to_string()
                .contains("escapes the preserved workspace")
        );
        reference.path = "evidence/provider.json".to_owned();
        reference.sha256 = "0".repeat(64);
        assert!(
            verifier_only_artifact(&workspace, artifact_snapshot.as_ref().unwrap(), &reference)
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("SHA-256 mismatch")
        );
        assert_eq!(
            workspace::fingerprint_path(&workspace.path).await.unwrap(),
            fingerprint
        );
        assert_eq!(
            workspace::fingerprint_path(&baseline_path).await.unwrap(),
            fingerprint
        );
        cleanup_verification_snapshots(&mut None, &mut baseline, &mut artifact_snapshot)
            .await
            .unwrap();
        assert_no_verification_snapshots(run_id);
        std::fs::remove_dir_all(root).expect("remove owned legacy fixture");
    }

    #[tokio::test]
    async fn verifier_artifact_transfer_early_error_preserves_prepared_workspace() {
        let (root, workspaces, workspace, mut assignment) = prepared_verification_fixture().await;
        let fingerprint = assignment.expected_workspace_fingerprint.clone().unwrap();
        let mut reference =
            verification_artifact_reference("codex-outside-provider.json", b"server-held evidence");
        reference.data_base64 = None;
        assignment.provider_artifact = Some(reference);
        let outbound = OutboundBus::default();
        let (_control_tx, controls) = mpsc::unbounded_channel();
        let (_ack_tx, artifact_acks) = mpsc::unbounded_channel();
        execute_verification_assignment(
            workspaces.clone(),
            "runner-test".to_owned(),
            assignment.clone(),
            outbound.clone(),
            AssignmentChannels {
                controls,
                artifact_acks,
            },
        )
        .await
        .expect("prepared failures must emit their terminal and workspace outcomes");
        let events = recorded_run_events(&outbound);
        assert_eq!(
            events
                .iter()
                .map(|(kind, _)| kind.as_str())
                .collect::<Vec<_>>(),
            ["run.started", "run.failed", "run.workspace_preserved"]
        );
        assert_eq!(events[2].1["workspace"], json!(workspace.path));
        assert_eq!(events[2].1["workspace_fingerprint"], fingerprint);
        assert_eq!(events[2].1["workspace_quarantined"], false);
        assert_eq!(events[2].1["branch_deleted"], false);
        assert!(
            workspace.path.is_dir(),
            "a clean preserved source must never be finalized"
        );
        assert!(git(&workspace.path, &["status", "--porcelain"]).is_empty());
        assert_eq!(
            workspaces.fingerprint(&workspace).await.unwrap(),
            fingerprint
        );
        assert_no_verification_snapshots(assignment.run_id);
        std::fs::remove_dir_all(root).expect("remove owned managed fixture");
    }

    #[tokio::test]
    async fn verifier_artifact_transfer_inline_assignment_preserves_completed_source() {
        let (root, workspaces, workspace, mut assignment) = prepared_verification_fixture().await;
        let fingerprint = assignment.expected_workspace_fingerprint.clone().unwrap();
        assignment.provider_artifact = Some(verification_artifact_reference(
            "codex-evidence-outside-worktree.json",
            br#"{"provider":"codex","artifact":"ready in object storage"}"#,
        ));
        let outbound = OutboundBus::default();
        let (_control_tx, controls) = mpsc::unbounded_channel();
        let (_ack_tx, artifact_acks) = mpsc::unbounded_channel();
        execute_verification_assignment(
            workspaces.clone(),
            "runner-test".to_owned(),
            assignment.clone(),
            outbound.clone(),
            AssignmentChannels {
                controls,
                artifact_acks,
            },
        )
        .await
        .unwrap();
        let events = recorded_run_events(&outbound);
        assert_eq!(
            events
                .iter()
                .map(|(kind, _)| kind.as_str())
                .collect::<Vec<_>>(),
            [
                "run.started",
                "run.verification_started",
                "run.verification_evidence",
                "run.verification_passed",
                "run.completed",
                "run.workspace_preserved",
            ]
        );
        assert_eq!(
            events.last().unwrap().1["workspace_fingerprint"],
            fingerprint
        );
        assert_eq!(events.last().unwrap().1["workspace_quarantined"], false);
        assert!(workspace.path.is_dir());
        assert!(git(&workspace.path, &["status", "--porcelain"]).is_empty());
        assert_eq!(
            workspaces.fingerprint(&workspace).await.unwrap(),
            fingerprint
        );
        assert_no_verification_snapshots(assignment.run_id);
        std::fs::remove_dir_all(root).expect("remove owned successful recovery fixture");
    }

    #[tokio::test]
    async fn verifier_artifact_transfer_cancellation_during_upload_cannot_accept_completion() {
        let (root, workspaces, workspace, mut assignment) = prepared_verification_fixture().await;
        let fingerprint = assignment.expected_workspace_fingerprint.clone().unwrap();
        assignment.provider_artifact = Some(verification_artifact_reference(
            "provider-outside-worktree.json",
            br#"{"evidence":"verified before cancellation"}"#,
        ));
        assignment.deliverable = Some(DeliverableSpec {
            form: crony_domain::DeliverableForm::ReviewOnlyReport,
            commit_after_verification: false,
            paths: Vec::new(),
        });
        assignment.write_scope = vec!["sentinel.txt".to_owned()];
        let outbound = OutboundBus::default();
        let (connection, mut received) = mpsc::unbounded_channel();
        outbound.attach(connection, assignment.connection_epoch);
        let (control_tx, controls) = mpsc::unbounded_channel();
        let (ack_tx, artifact_acks) = mpsc::unbounded_channel();
        let task_assignment = assignment.clone();
        let task_workspaces = workspaces.clone();
        let task = tokio::spawn(async move {
            execute_verification_assignment(
                task_workspaces,
                "runner-test".to_owned(),
                task_assignment,
                outbound,
                AssignmentChannels {
                    controls,
                    artifact_acks,
                },
            )
            .await
        });
        let mut event_types = Vec::new();
        let upload_sha = tokio::time::timeout(Duration::from_secs(30), async {
            while let Some(message) = received.recv().await {
                if let RunnerToServer::RunEvent {
                    event_type,
                    payload,
                    ..
                } = message
                {
                    event_types.push(event_type.clone());
                    if event_type == "run.failed" {
                        panic!(
                            "verification failed before the upload checkpoint: {}",
                            payload["error"]
                                .as_str()
                                .unwrap_or("missing failure detail")
                        );
                    }
                    if event_type == "run.deliverable_upload" {
                        return payload["sha256"].as_str().unwrap().to_owned();
                    }
                }
            }
            panic!("verification ended before the upload checkpoint");
        })
        .await
        .expect("verifier reached its upload checkpoint");
        assert_no_verification_snapshots(assignment.run_id);
        control_tx
            .send(AdapterControl::Interrupt {
                reason: "cancel while awaiting durable upload acknowledgment".to_owned(),
            })
            .expect("request cancellation");
        ack_tx
            .send(ArtifactAck {
                artifact_id: Uuid::new_v4(),
                artifact_role: "source_deliverable".to_owned(),
                sha256: upload_sha,
            })
            .expect("release upload acknowledgment wait");
        tokio::time::timeout(Duration::from_secs(15), task)
            .await
            .expect("cancelled verification settles")
            .expect("join verification")
            .expect("cancellation was handled");
        let mut retained = None;
        while let Ok(message) = received.try_recv() {
            if let RunnerToServer::RunEvent {
                event_type,
                payload,
                ..
            } = message
            {
                if event_type == "run.workspace_preserved" {
                    retained = Some(payload);
                }
                event_types.push(event_type);
            }
        }
        assert!(event_types.iter().any(|kind| kind == "run.cancelled"));
        assert!(
            event_types
                .iter()
                .any(|kind| kind == "run.workspace_preserved")
        );
        assert!(!event_types.iter().any(|kind| matches!(
            kind.as_str(),
            "run.verification_passed" | "run.verification_waiting" | "run.completed"
        )));
        let retained = retained.expect("cancelled run retained its workspace");
        assert_eq!(retained["workspace_quarantined"], false);
        assert_eq!(retained["workspace_fingerprint"], fingerprint);
        assert_no_verification_snapshots(assignment.run_id);
        assert_eq!(
            workspaces.fingerprint(&workspace).await.unwrap(),
            fingerprint
        );
        let resolved = std::fs::canonicalize(&root).unwrap();
        let temporary = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        assert!(resolved.starts_with(temporary));
        std::fs::remove_dir_all(root).expect("remove owned upload-cancellation fixture");
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum HeldUploadChange {
        None,
        Source,
        Head,
        AckClosed,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum CheckpointAck {
        Immediate,
        Closed,
        Cancelled,
    }

    async fn checkpoint_commit_case(
        checkpoint_mode: bool,
        change_head: bool,
        acknowledgment: CheckpointAck,
    ) {
        let (root, workspaces, workspace, mut assignment) = prepared_verification_fixture().await;
        let canonical_root = std::fs::canonicalize(&root).unwrap();
        let temporary = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        assert!(canonical_root != temporary && canonical_root.starts_with(&temporary));
        std::fs::write(workspace.path.join("checkpoint.txt"), b"completed source\n").unwrap();
        let original_head = assignment.expected_head_commit.clone().unwrap();
        let fingerprint = workspaces.fingerprint(&workspace).await.unwrap();
        assignment.expected_workspace_fingerprint = Some(fingerprint.clone());
        assignment.checkpoint_verification = checkpoint_mode;
        assignment.verification_policy = VerificationPolicy {
            checks: vec![crony_domain::VerifierCheck::File {
                path: "checkpoint.txt".to_owned(),
                min_bytes: 1,
            }],
            manual_gate: None,
        };
        assignment.write_scope = vec!["checkpoint.txt".to_owned()];
        assignment.deliverable = Some(DeliverableSpec {
            form: crony_domain::DeliverableForm::CommitBranch,
            commit_after_verification: true,
            paths: vec!["checkpoint.txt".to_owned()],
        });
        let outbound = OutboundBus::default();
        let (connection, mut received) = mpsc::unbounded_channel();
        outbound.attach(connection, assignment.connection_epoch);
        let (control_tx, controls) = mpsc::unbounded_channel();
        let (ack_tx, artifact_acks) = mpsc::unbounded_channel();
        let mut ack_tx = Some(ack_tx);
        let task_assignment = assignment.clone();
        let task_workspaces = workspaces.clone();
        let execution = tokio::spawn(async move {
            execute_verification_assignment(
                task_workspaces,
                "runner-test".to_owned(),
                task_assignment,
                outbound,
                AssignmentChannels {
                    controls,
                    artifact_acks,
                },
            )
            .await
        });
        let mut uploaded = false;
        let mut final_head = None;
        let mut events = Vec::new();
        tokio::time::timeout(Duration::from_secs(45), async {
            while let Some(message) = received.recv().await {
                if let RunnerToServer::RunEvent {
                    event_type,
                    payload,
                    ..
                } = message
                {
                    if event_type == "run.deliverable_upload" {
                        assert!(
                            checkpoint_mode,
                            "ordinary verifier must preserve its existing commit"
                        );
                        assert!(!uploaded, "one native upload with an immediate ACK");
                        uploaded = true;
                        let bytes = BASE64
                            .decode(payload["content_base64"].as_str().unwrap())
                            .unwrap();
                        let document: Value = serde_json::from_slice(&bytes).unwrap();
                        assert_eq!(document["base_commit"], workspace.base_commit);
                        assert_eq!(document["changes"][0]["path"], "checkpoint.txt");
                        assert_eq!(
                            BASE64
                                .decode(document["changes"][0]["content_base64"].as_str().unwrap())
                                .unwrap(),
                            b"completed source\n"
                        );
                        let bundle = BASE64
                            .decode(document["git_bundle_base64"].as_str().unwrap())
                            .unwrap();
                        assert!(!bundle.is_empty());
                        assert_eq!(
                            hex::encode(sha2::Sha256::digest(&bundle)),
                            document["git_bundle_sha256"]
                        );
                        assert_ne!(payload["head_commit"], original_head);
                        assert_eq!(payload["publication_ready"], true);
                        final_head = Some(payload["head_commit"].as_str().unwrap().to_owned());
                        if change_head {
                            git(
                                &workspace.path,
                                &[
                                    "-c",
                                    "user.name=ECorp Test",
                                    "-c",
                                    "user.email=test@example.invalid",
                                    "commit",
                                    "--allow-empty",
                                    "--no-gpg-sign",
                                    "-m",
                                    "unauthorized head drift",
                                ],
                            );
                        }
                        if acknowledgment == CheckpointAck::Cancelled {
                            control_tx
                                .send(AdapterControl::Interrupt {
                                    reason: "cancel after the runner-owned verification commit"
                                        .to_owned(),
                                })
                                .unwrap();
                        }
                        if acknowledgment == CheckpointAck::Closed {
                            drop(ack_tx.take());
                        } else {
                            ack_tx
                                .as_ref()
                                .unwrap()
                                .send(ArtifactAck {
                                    artifact_id: Uuid::new_v4(),
                                    artifact_role: "source_deliverable".to_owned(),
                                    sha256: payload["sha256"].as_str().unwrap().to_owned(),
                                })
                                .unwrap();
                        }
                    }
                    let finished = event_type == "run.workspace_preserved";
                    events.push((event_type, payload));
                    if finished {
                        break;
                    }
                }
            }
        })
        .await
        .expect("native verifier and upload ACK settle");
        execution.await.unwrap().unwrap();
        let completed = events.iter().any(|(kind, _)| kind == "run.completed");
        assert_eq!(
            completed,
            checkpoint_mode && !change_head && acknowledgment == CheckpointAck::Immediate,
            "{events:?}"
        );
        assert_eq!(uploaded, checkpoint_mode, "{events:?}");
        let retained = events
            .iter()
            .find(|(kind, _)| kind == "run.workspace_preserved")
            .unwrap();
        assert_eq!(
            retained.1["workspace_quarantined"], change_head,
            "{events:?}"
        );
        assert!(!events.iter().any(|(kind, _)| matches!(
            kind.as_str(),
            "run.session" | "run.session_terminated" | "run.output" | "run.usage"
        )));
        assert_eq!(
            workspaces.fingerprint(&workspace).await.unwrap(),
            fingerprint
        );
        assert_eq!(
            std::fs::read(workspace.path.join("checkpoint.txt")).unwrap(),
            b"completed source\n"
        );
        if checkpoint_mode && !change_head {
            assert_eq!(
                workspaces.head_commit(&workspace).await.unwrap(),
                final_head.unwrap()
            );
            assert_eq!(
                git(&workspace.path, &["rev-parse", "HEAD^"]),
                workspace.base_commit
            );
            if acknowledgment == CheckpointAck::Cancelled {
                assert!(
                    events.iter().any(|(kind, _)| kind == "run.cancelled"),
                    "{events:?}"
                );
                assert!(
                    !events.iter().any(|(kind, _)| kind == "run.failed"),
                    "{events:?}"
                );
            } else if acknowledgment == CheckpointAck::Closed {
                assert!(
                    events.iter().any(|(kind, _)| kind == "run.failed"),
                    "{events:?}"
                );
            }
        } else {
            let error = events
                .iter()
                .find(|(kind, _)| kind == "run.failed")
                .unwrap();
            assert!(
                error.1["error"].as_str().unwrap().contains(if change_head {
                    "head mismatch"
                } else {
                    "tree changed from preserved head"
                }),
                "{error:?}"
            );
        }
        assert_no_verification_snapshots(assignment.run_id);
        std::fs::remove_dir_all(canonical_root).expect("remove exact owned checkpoint fixture");
    }

    #[tokio::test]
    async fn checkpoint_verifier_exports_uncommitted_source_without_a_provider() {
        checkpoint_commit_case(true, false, CheckpointAck::Immediate).await;
    }

    #[tokio::test]
    async fn ordinary_verifier_still_rejects_an_uncommitted_replacement_tree() {
        checkpoint_commit_case(false, false, CheckpointAck::Immediate).await;
    }

    #[tokio::test]
    async fn checkpoint_verifier_rejects_head_drift_after_its_owned_commit() {
        checkpoint_commit_case(true, true, CheckpointAck::Immediate).await;
    }

    #[tokio::test]
    async fn checkpoint_verifier_retains_own_commit_after_ack_failure() {
        checkpoint_commit_case(true, false, CheckpointAck::Closed).await;
    }

    #[tokio::test]
    async fn checkpoint_verifier_cancellation_does_not_quarantine_own_commit() {
        checkpoint_commit_case(true, false, CheckpointAck::Cancelled).await;
    }

    #[tokio::test]
    async fn checkpoint_verifier_cancellation_still_quarantines_unexpected_head() {
        checkpoint_commit_case(true, true, CheckpointAck::Cancelled).await;
    }

    async fn held_upload_checkpoint_case(change: HeldUploadChange, manual_gate: bool) {
        let (root, workspaces, workspace, mut assignment) = prepared_verification_fixture().await;
        let resolved = std::fs::canonicalize(&root).unwrap();
        let temporary = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        assert!(resolved != temporary && resolved.starts_with(&temporary));
        assert!(
            std::fs::canonicalize(&workspace.path)
                .unwrap()
                .starts_with(&resolved)
        );
        let fingerprint = assignment.expected_workspace_fingerprint.clone().unwrap();
        let head = assignment.expected_head_commit.clone().unwrap();
        assignment.provider_artifact = Some(verification_artifact_reference(
            "provider-outside-worktree.json",
            b"verified provider evidence",
        ));
        assignment.deliverable = Some(DeliverableSpec {
            form: crony_domain::DeliverableForm::ReviewOnlyReport,
            // Keep a verification-linked head in the uploaded metadata, as in the late-upload
            // quarantine/checkpoint regression, without creating a new commit on this clean tree.
            commit_after_verification: true,
            paths: Vec::new(),
        });
        assignment.write_scope = vec!["sentinel.txt".to_owned()];
        assignment.verification_policy.manual_gate =
            manual_gate.then(|| crony_domain::ManualVerificationGate::HumanApproval {
                roles: vec!["owner".to_owned()],
            });
        let outbound = OutboundBus::default();
        let (connection, mut received) = mpsc::unbounded_channel();
        outbound.attach(connection, assignment.connection_epoch);
        let (_control_tx, controls) = mpsc::unbounded_channel();
        let (ack_tx, artifact_acks) = mpsc::unbounded_channel();
        let task_assignment = assignment.clone();
        let task_workspaces = workspaces.clone();
        let task = tokio::spawn(async move {
            execute_verification_assignment(
                task_workspaces,
                "runner-test".to_owned(),
                task_assignment,
                outbound,
                AssignmentChannels {
                    controls,
                    artifact_acks,
                },
            )
            .await
        });
        let mut events = Vec::new();
        let upload_sha = tokio::time::timeout(Duration::from_secs(30), async {
            while let Some(message) = received.recv().await {
                if let RunnerToServer::RunEvent {
                    event_type,
                    payload,
                    ..
                } = message
                {
                    assert_ne!(event_type, "run.failed", "failed before upload: {payload}");
                    events.push((event_type.clone(), payload.clone()));
                    if event_type == "run.deliverable_upload" {
                        assert_eq!(payload["head_commit"], head);
                        return payload["sha256"].as_str().unwrap().to_owned();
                    }
                }
            }
            panic!("verification ended before its upload checkpoint");
        })
        .await
        .expect("verifier reached held upload ACK");
        assert_no_verification_snapshots(assignment.run_id);
        match change {
            HeldUploadChange::Source => {
                std::fs::write(
                    workspace.path.join("sentinel.txt"),
                    b"source drift while durable upload ACK was held\n",
                )
                .unwrap();
            }
            HeldUploadChange::Head => {
                git(
                    &workspace.path,
                    &[
                        "-c",
                        "user.name=ECorp Test",
                        "-c",
                        "user.email=ecorp@example.invalid",
                        "commit",
                        "--allow-empty",
                        "--no-gpg-sign",
                        "-m",
                        "head drift during held ACK",
                    ],
                );
            }
            HeldUploadChange::None | HeldUploadChange::AckClosed => {}
        }
        if change == HeldUploadChange::AckClosed {
            drop(ack_tx);
        } else {
            ack_tx
                .send(ArtifactAck {
                    artifact_id: Uuid::new_v4(),
                    artifact_role: "source_deliverable".to_owned(),
                    sha256: upload_sha,
                })
                .expect("release held upload ACK");
        }
        tokio::time::timeout(Duration::from_secs(15), task)
            .await
            .expect("held upload verification settles")
            .expect("join verification")
            .expect("verification terminal outcome was handled");
        while let Ok(message) = received.try_recv() {
            if let RunnerToServer::RunEvent {
                event_type,
                payload,
                ..
            } = message
            {
                events.push((event_type, payload));
            }
        }
        let retained = events.last().expect("retained outcome");
        assert_eq!(retained.0, "run.workspace_preserved");
        let quarantined = matches!(change, HeldUploadChange::Source | HeldUploadChange::Head);
        assert_eq!(retained.1["workspace_quarantined"], quarantined);
        assert_eq!(retained.1["branch_deleted"], false);
        if quarantined {
            assert_eq!(retained.1["workspace_fingerprint"], Value::Null);
        } else {
            assert_eq!(retained.1["workspace_fingerprint"], fingerprint);
        }
        let successes = events
            .iter()
            .filter(|(kind, _)| {
                matches!(
                    kind.as_str(),
                    "run.verification_passed" | "run.verification_waiting" | "run.completed"
                )
            })
            .map(|(kind, _)| kind.as_str())
            .collect::<Vec<_>>();
        if change == HeldUploadChange::None {
            assert_eq!(
                successes,
                vec![
                    "run.verification_passed",
                    if manual_gate {
                        "run.verification_waiting"
                    } else {
                        "run.completed"
                    },
                ]
            );
            assert!(!events.iter().any(|(kind, _)| kind == "run.failed"));
        } else {
            assert!(
                successes.is_empty(),
                "no accepted event may precede drift failure: {events:?}"
            );
            assert!(events.iter().any(|(kind, _)| kind == "run.failed"));
        }
        if change == HeldUploadChange::Source {
            assert_ne!(
                workspaces.fingerprint(&workspace).await.unwrap(),
                fingerprint
            );
            assert_eq!(workspaces.head_commit(&workspace).await.unwrap(), head);
        } else {
            assert_eq!(
                workspaces.fingerprint(&workspace).await.unwrap(),
                fingerprint
            );
            if change == HeldUploadChange::Head {
                assert_ne!(workspaces.head_commit(&workspace).await.unwrap(), head);
            } else {
                assert_eq!(workspaces.head_commit(&workspace).await.unwrap(), head);
            }
        }
        assert_no_verification_snapshots(assignment.run_id);
        std::fs::remove_dir_all(&resolved).expect("remove owned held-upload fixture");
    }

    #[tokio::test]
    async fn verifier_artifact_transfer_held_upload_source_drift_is_quarantined() {
        held_upload_checkpoint_case(HeldUploadChange::Source, false).await;
    }

    #[tokio::test]
    async fn verifier_artifact_transfer_held_upload_source_drift_with_manual_gate_is_quarantined() {
        held_upload_checkpoint_case(HeldUploadChange::Source, true).await;
    }

    #[tokio::test]
    async fn verifier_artifact_transfer_held_upload_head_drift_is_quarantined() {
        held_upload_checkpoint_case(HeldUploadChange::Head, false).await;
    }

    #[tokio::test]
    async fn verifier_artifact_transfer_held_upload_head_drift_with_manual_gate_is_quarantined() {
        held_upload_checkpoint_case(HeldUploadChange::Head, true).await;
    }

    #[tokio::test]
    async fn verifier_artifact_transfer_held_upload_unchanged_checkpoint_is_trusted() {
        for manual_gate in [false, true] {
            held_upload_checkpoint_case(HeldUploadChange::None, manual_gate).await;
        }
    }

    #[tokio::test]
    async fn verifier_artifact_transfer_held_upload_artifact_error_keeps_matching_checkpoint_trusted()
     {
        held_upload_checkpoint_case(HeldUploadChange::AckClosed, false).await;
    }

    #[test]
    fn verifier_artifact_transfer_quarantine_flag_is_explicit_not_text_derived() {
        let workspace = WorkspaceLease {
            path: PathBuf::from("unused-workspace"),
            branch: "unused-branch".to_owned(),
            base_ref: "HEAD".to_owned(),
            base_commit: "a".repeat(40),
        };
        let assignment = verification_assignment(&workspace, Uuid::new_v4());
        let outbound = OutboundBus::default();
        let fingerprint = "b".repeat(64);
        send_teardown_workspace_preserved(
            &outbound,
            "runner-test",
            &assignment,
            &workspace,
            "display text mentions quarantine but grants no authority",
            Some(&fingerprint),
            false,
        );
        send_teardown_workspace_preserved(
            &outbound,
            "runner-test",
            &assignment,
            &workspace,
            "display text does not classify this outcome",
            Some(&fingerprint),
            true,
        );
        let events = recorded_run_events(&outbound);
        assert_eq!(events[0].1["workspace_quarantined"], false);
        assert_eq!(events[0].1["workspace_fingerprint"], fingerprint);
        assert_eq!(events[1].1["workspace_quarantined"], true);
        assert_eq!(events[1].1["workspace_fingerprint"], Value::Null);
    }

    #[tokio::test]
    async fn verifier_artifact_transfer_fingerprint_mismatch_is_quarantined() {
        let (root, workspaces, workspace, assignment) = prepared_verification_fixture().await;
        std::fs::write(
            workspace.path.join("sentinel.txt"),
            b"unreviewed source change\n",
        )
        .unwrap();
        let changed = workspaces.fingerprint(&workspace).await.unwrap();
        assert_ne!(
            Some(&changed),
            assignment.expected_workspace_fingerprint.as_ref()
        );
        let outbound = OutboundBus::default();
        let (_control_tx, controls) = mpsc::unbounded_channel();
        let (_ack_tx, artifact_acks) = mpsc::unbounded_channel();
        execute_verification_assignment(
            workspaces.clone(),
            "runner-test".to_owned(),
            assignment.clone(),
            outbound.clone(),
            AssignmentChannels {
                controls,
                artifact_acks,
            },
        )
        .await
        .unwrap();
        let events = recorded_run_events(&outbound);
        assert_eq!(
            events
                .iter()
                .map(|(kind, _)| kind.as_str())
                .collect::<Vec<_>>(),
            ["run.started", "run.failed", "run.workspace_preserved"]
        );
        assert!(
            events[1].1["error"]
                .as_str()
                .unwrap()
                .contains("fingerprint mismatch")
        );
        assert_eq!(events[2].1["workspace_fingerprint"], Value::Null);
        assert_eq!(events[2].1["workspace_quarantined"], true);
        assert!(
            events[2].1["detail"]
                .as_str()
                .unwrap()
                .contains("quarantine")
        );
        assert_eq!(workspaces.fingerprint(&workspace).await.unwrap(), changed);
        assert_no_verification_snapshots(assignment.run_id);
        std::fs::remove_dir_all(root).expect("remove owned quarantine fixture");
    }

    #[tokio::test]
    async fn verifier_artifact_transfer_cancellation_cleans_all_snapshots() {
        let (root, workspace) = verification_source_fixture();
        let run_id = Uuid::new_v4();
        let fingerprint = workspace::fingerprint_path(&workspace.path).await.unwrap();
        let mut baseline = workspace::verification_snapshot(&workspace.path, run_id)
            .await
            .unwrap();
        let mut artifact_snapshot = Some(
            workspace::empty_verification_snapshot(run_id)
                .await
                .unwrap(),
        );
        let reference = verification_artifact_reference("absent.json", b"ready evidence");
        let prepared =
            verifier_only_artifact(&workspace, artifact_snapshot.as_ref().unwrap(), &reference)
                .await
                .unwrap();
        let policy = VerificationPolicy {
            checks: vec![crony_domain::VerifierCheck::Command {
                program: "node".to_owned(),
                args: vec![
                    "-e".to_owned(),
                    "throw Error('cancelled check must not execute')".to_owned(),
                ],
                timeout_ms: 5_000,
            }],
            manual_gate: None,
        };
        let (_cancel_tx, mut cancellation) = watch::channel(true);
        let mut active = None;
        assert!(matches!(
            verify_isolated_recovery_checks(
                &policy,
                run_id,
                &mut baseline,
                &mut active,
                &mut artifact_snapshot,
                &[prepared.artifact],
                &mut cancellation,
            )
            .await
            .unwrap(),
            IsolatedVerificationOutcome::Cancelled
        ));
        assert!(active.is_none());
        assert!(artifact_snapshot.is_none());
        assert_no_verification_snapshots(run_id);
        assert_eq!(
            workspace::fingerprint_path(&workspace.path).await.unwrap(),
            fingerprint
        );
        std::fs::remove_dir_all(root).expect("remove owned cancellation fixture");
    }

    #[tokio::test]
    async fn verifier_artifact_transfer_cleanup_error_blocks_acceptance() {
        let (root, workspaces, workspace, mut assignment) = prepared_verification_fixture().await;
        let run_id = assignment.run_id;
        assignment.verification_policy.checks = vec![crony_domain::VerifierCheck::File {
            path: "sentinel.txt".to_owned(),
            min_bytes: 1,
        }];
        let fingerprint = workspace::fingerprint_path(&workspace.path).await.unwrap();
        let mut baseline = workspace::verification_snapshot(&workspace.path, run_id)
            .await
            .unwrap();
        let mut artifact_snapshot = Some(
            workspace::empty_verification_snapshot(run_id)
                .await
                .unwrap(),
        );
        let artifact_root = artifact_snapshot.as_ref().unwrap().path().to_owned();
        // A file at the owned directory path makes remove_dir_all fail on every supported OS.
        std::fs::remove_dir(&artifact_root).unwrap();
        std::fs::write(&artifact_root, b"injected cleanup failure").unwrap();
        let mut active = None;
        let (_cancel_tx, mut cancellation) = watch::channel(false);
        let (_ack_tx, mut artifact_acks) = mpsc::unbounded_channel();
        let outbound = OutboundBus::default();
        let artifacts = Arc::new(Mutex::new(Vec::new()));
        let outcome = send_verification_events(
            &outbound,
            "runner-test",
            &assignment,
            &workspace,
            &workspaces,
            Some(&mut baseline),
            Some(&mut active),
            Some(&mut artifact_snapshot),
            &artifacts,
            None,
            &mut artifact_acks,
            "cleanup fixture",
            Some(&fingerprint),
            None,
            None,
            Some(&mut cancellation),
        )
        .await;
        assert_eq!(outcome, VerificationRunOutcome::Failed);
        let events = recorded_run_events(&outbound);
        assert_eq!(
            events
                .iter()
                .map(|(kind, _)| kind.as_str())
                .collect::<Vec<_>>(),
            ["run.verification_started", "run.failed"]
        );
        assert!(events[1].1["error"].as_str().unwrap().contains("cleanup"));
        std::fs::remove_file(&artifact_root).unwrap();
        std::fs::create_dir(&artifact_root).unwrap();
        cleanup_verification_snapshots(&mut active, &mut baseline, &mut artifact_snapshot)
            .await
            .unwrap();
        assert_no_verification_snapshots(run_id);
        assert_eq!(
            workspace::fingerprint_path(&workspace.path).await.unwrap(),
            fingerprint
        );
        std::fs::remove_dir_all(root).expect("remove owned cleanup fixture");
    }

    #[tokio::test]
    async fn verifier_artifact_transfer_provider_admission_failure_preserves_workspace() {
        let (root, workspaces, workspace, mut assignment) = prepared_verification_fixture().await;
        assignment.expected_workspace_fingerprint = Some("0".repeat(64));
        let adapter: Arc<dyn AgentAdapter> = Arc::new(LatchedTeardownAdapter {
            uncertain: Arc::new(Notify::new()),
            verified: Arc::new(Notify::new()),
        });
        let outbound = OutboundBus::default();
        let (_control_tx, controls) = mpsc::unbounded_channel();
        let (_ack_tx, artifact_acks) = mpsc::unbounded_channel();
        tokio::time::timeout(
            Duration::from_secs(10),
            execute_assignment(
                workspaces,
                "runner-test".to_owned(),
                assignment,
                adapter,
                outbound.clone(),
                AssignmentChannels {
                    controls,
                    artifact_acks,
                },
                Some("must-not-resume".to_owned()),
            ),
        )
        .await
        .expect("admission must fail before the latched adapter can run")
        .unwrap();
        let events = recorded_run_events(&outbound);
        assert_eq!(
            events
                .iter()
                .map(|(kind, _)| kind.as_str())
                .collect::<Vec<_>>(),
            ["run.failed", "run.workspace_preserved"]
        );
        assert_eq!(events[1].1["workspace_fingerprint"], Value::Null);
        assert_eq!(events[1].1["workspace_quarantined"], true);
        assert!(workspace.path.is_dir());
        assert!(git(&workspace.path, &["status", "--porcelain"]).is_empty());
        std::fs::remove_dir_all(root).expect("remove owned provider-admission fixture");
    }

    #[test]
    fn buffered_run_events_rebind_epoch_without_changing_assignment_fence() {
        let old_epoch = Uuid::new_v4();
        let new_epoch = Uuid::new_v4();
        let assignment_token = Uuid::new_v4();
        let mut message = RunnerToServer::RunEvent {
            event_id: Uuid::new_v4(),
            runner_id: "runner-test".to_owned(),
            corp_id: Uuid::new_v4(),
            connection_epoch: old_epoch,
            run_id: Uuid::new_v4(),
            agent_id: Uuid::new_v4(),
            assignment_token,
            event_type: "run.output".to_owned(),
            payload: json!({"text": "buffered"}),
        };
        bind_connection_epoch(&mut message, new_epoch);
        let RunnerToServer::RunEvent {
            connection_epoch,
            assignment_token: rebound_token,
            ..
        } = message
        else {
            panic!("expected run event");
        };
        assert_eq!(connection_epoch, new_epoch);
        assert_eq!(rebound_token, assignment_token);
    }

    #[test]
    fn teardown_fail_closed_preserves_workspace_without_false_terminal_claim() {
        let assignment = Assignment {
            workspace_connection_id: None,
            corp_id: Uuid::new_v4(),
            connection_epoch: Uuid::new_v4(),
            room_id: Uuid::new_v4(),
            mission_id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            run_id: Uuid::new_v4(),
            workspace_run_id: Uuid::new_v4(),
            agent_id: Uuid::new_v4(),
            assignment_token: Uuid::new_v4(),
            adapter: "claude-code".to_owned(),
            mission_title: "teardown failure".to_owned(),
            model: None,
            reasoning_effort: None,
            source_repository: None,
            source_base_ref: None,
            source_base_commit: None,
            resume_workspace_base_commit: None,
            verification_policy: VerificationPolicy {
                checks: Vec::new(),
                manual_gate: None,
            },
            write_scope: Vec::new(),
            deliverable: None,
            secrets: Vec::new(),
            expected_workspace_fingerprint: None,
            expected_head_commit: None,
            provider_artifact: None,
            checkpoint_verification: false,
            hard_boundary_checkpoint: Arc::default(),
        };
        let workspace = WorkspaceLease {
            path: PathBuf::from("worktrees/exact-run"),
            branch: "crony/task/exact-run".to_owned(),
            base_ref: "ecorp/base".to_owned(),
            base_commit: "0123456789abcdef".to_owned(),
        };
        let outbound = OutboundBus::default();
        let teardown_uncertain = Arc::new(AtomicBool::new(false));
        let terminal = Arc::new(Mutex::new(None));
        let sink = RunnerEventSink {
            outbound: outbound.clone(),
            runner_id: "runner-test".to_owned(),
            assignment,
            workspace: workspace.clone(),
            artifacts: Arc::new(Mutex::new(Vec::new())),
            terminal: terminal.clone(),
            teardown_uncertain: teardown_uncertain.clone(),
        };

        sink.emit(AdapterEvent::TeardownUncertain {
            detail: "injected provider-scope query failure".to_owned(),
        });
        sink.emit(AdapterEvent::Cancelled {
            reason: "stop requested".to_owned(),
        });

        let (connection, mut received) = mpsc::unbounded_channel();
        outbound.attach(connection, Uuid::new_v4());
        let messages = [received.try_recv().unwrap(), received.try_recv().unwrap()];
        assert!(received.try_recv().is_err());
        assert!(teardown_uncertain.load(Ordering::Acquire));
        assert!(matches!(
            terminal.lock().expect("terminal lock").as_ref(),
            Some(BufferedTerminal::Cancelled(reason)) if reason == "stop requested"
        ));

        let mut event_types = Vec::new();
        for message in messages {
            let RunnerToServer::RunEvent {
                event_type,
                payload,
                ..
            } = message
            else {
                panic!("expected run event");
            };
            assert_ne!(event_type, "run.session_terminated");
            assert_ne!(
                payload.get("provider_process_alive"),
                Some(&Value::Bool(false))
            );
            if event_type == "run.workspace_preserved" {
                assert_eq!(payload["workspace"], json!(workspace.path));
                assert_eq!(payload["dirty"], Value::Null);
                assert_eq!(payload["branch_deleted"], false);
            }
            event_types.push(event_type);
        }
        assert_eq!(
            event_types,
            ["run.teardown_uncertain", "run.workspace_preserved"]
        );

        let (replacement, mut replayed) = mpsc::unbounded_channel();
        outbound.attach(replacement, Uuid::new_v4());
        assert!(replayed.try_recv().is_err());
    }

    #[tokio::test]
    async fn teardown_fail_closed_execute_assignment_preserves_exact_workspace_until_verified() {
        let (root, repository, managed) = teardown_fixture();
        let workspaces =
            WorkspaceManager::initialize(managed, repository.clone(), "HEAD".to_owned())
                .await
                .expect("initialize workspace manager");
        let task_id = Uuid::new_v4();
        let workspace_run_id = Uuid::new_v4();
        let expected_workspace = workspaces
            .root()
            .join("worktrees")
            .join(task_id.simple().to_string())
            .join(workspace_run_id.simple().to_string());
        let expected_branch = format!(
            "crony/task-{}/run-{}",
            task_id.simple(),
            workspace_run_id.simple()
        );
        let base_commit = workspaces.base_commit().to_owned();
        let assignment = Assignment {
            workspace_connection_id: None,
            corp_id: Uuid::new_v4(),
            connection_epoch: Uuid::new_v4(),
            room_id: Uuid::new_v4(),
            mission_id: Uuid::new_v4(),
            task_id,
            run_id: Uuid::new_v4(),
            workspace_run_id,
            agent_id: Uuid::new_v4(),
            assignment_token: Uuid::new_v4(),
            adapter: "latched-teardown".to_owned(),
            mission_title: "latched teardown failure".to_owned(),
            model: None,
            reasoning_effort: None,
            source_repository: None,
            source_base_ref: None,
            source_base_commit: None,
            resume_workspace_base_commit: None,
            verification_policy: VerificationPolicy {
                checks: Vec::new(),
                manual_gate: None,
            },
            write_scope: Vec::new(),
            deliverable: None,
            secrets: Vec::new(),
            expected_workspace_fingerprint: None,
            expected_head_commit: None,
            provider_artifact: None,
            checkpoint_verification: false,
            hard_boundary_checkpoint: Arc::default(),
        };
        let uncertain = Arc::new(Notify::new());
        let verified = Arc::new(Notify::new());
        let adapter: Arc<dyn AgentAdapter> = Arc::new(LatchedTeardownAdapter {
            uncertain: uncertain.clone(),
            verified: verified.clone(),
        });
        let outbound = OutboundBus::default();
        let (connection, mut received) = mpsc::unbounded_channel();
        outbound.attach(connection, Uuid::new_v4());
        let (_control_tx, controls) = mpsc::unbounded_channel();
        let (_artifact_ack_tx, artifact_acks) = mpsc::unbounded_channel();
        let task_assignment = assignment.clone();
        let task_outbound = outbound.clone();
        let execution = tokio::spawn(async move {
            let result = execute_assignment(
                Arc::new(workspaces),
                "runner-test".to_owned(),
                task_assignment.clone(),
                adapter,
                task_outbound.clone(),
                AssignmentChannels {
                    controls,
                    artifact_acks,
                },
                None,
            )
            .await;
            if let Err(error) = &result {
                send_run_event(
                    &task_outbound,
                    "runner-test",
                    &task_assignment,
                    "run.failed",
                    json!({"error": error.to_string()}),
                );
            }
            result
        });

        tokio::time::timeout(Duration::from_secs(10), uncertain.notified())
            .await
            .expect("teardown uncertainty was not observed");
        let mut before_verification = Vec::new();
        for _ in 0..3 {
            before_verification.push(
                tokio::time::timeout(Duration::from_secs(5), received.recv())
                    .await
                    .expect("runner event deadline")
                    .expect("runner event channel closed"),
            );
        }
        for message in &before_verification {
            let RunnerToServer::RunEvent { event_type, .. } = message else {
                panic!("expected run event");
            };
            assert!(!matches!(
                event_type.as_str(),
                "run.session_terminated" | "run.completed" | "run.failed" | "run.cancelled"
            ));
        }
        assert!(!execution.is_finished());
        let sentinel_before =
            std::fs::read(expected_workspace.join("sentinel.txt")).expect("read sentinel");
        assert!(!sentinel_before.is_empty());
        assert_eq!(
            git(&expected_workspace, &["rev-parse", "HEAD"]),
            base_commit
        );
        assert_eq!(
            git(&expected_workspace, &["branch", "--show-current"]),
            expected_branch
        );
        assert!(git(&expected_workspace, &["status", "--porcelain"]).is_empty());
        assert_eq!(
            std::fs::canonicalize(&expected_workspace).expect("resolve managed worktree"),
            std::fs::canonicalize(PathBuf::from(git(
                &expected_workspace,
                &["rev-parse", "--show-toplevel"],
            )))
            .expect("resolve reported worktree")
        );

        verified.notify_one();
        let error = tokio::time::timeout(Duration::from_secs(10), execution)
            .await
            .expect("execute_assignment completion deadline")
            .expect("join execute_assignment")
            .expect_err("adapter runtime failure");
        assert!(
            error
                .to_string()
                .contains("verified teardown runtime failure")
        );

        let mut after_verification = Vec::new();
        for _ in 0..3 {
            after_verification.push(
                tokio::time::timeout(Duration::from_secs(5), received.recv())
                    .await
                    .expect("terminal runner event deadline")
                    .expect("runner event channel closed"),
            );
        }
        let event_types = after_verification
            .iter()
            .map(|message| match message {
                RunnerToServer::RunEvent { event_type, .. } => event_type.as_str(),
                _ => panic!("expected run event"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            event_types,
            [
                "run.session_terminated",
                "run.workspace_preserved",
                "run.failed"
            ]
        );
        let RunnerToServer::RunEvent { payload, .. } = &after_verification[0] else {
            panic!("expected terminal run event");
        };
        assert_eq!(
            payload.get("provider_process_alive"),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            std::fs::read(expected_workspace.join("sentinel.txt")).expect("read sentinel"),
            sentinel_before
        );
        assert_eq!(
            git(&expected_workspace, &["rev-parse", "HEAD"]),
            base_commit
        );
        assert_eq!(
            git(&expected_workspace, &["branch", "--show-current"]),
            expected_branch
        );
        assert!(git(&expected_workspace, &["status", "--porcelain"]).is_empty());
        assert_eq!(
            git(&repository, &["rev-parse", &expected_branch]),
            base_commit
        );

        git(
            &repository,
            &[
                "worktree",
                "remove",
                "--force",
                expected_workspace.to_str().expect("workspace path"),
            ],
        );
        git(&repository, &["branch", "-D", &expected_branch]);
        let _ = std::fs::remove_dir_all(root);
    }
}
