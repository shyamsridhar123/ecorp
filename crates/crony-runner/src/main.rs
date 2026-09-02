mod adapter;
mod deliverable;
mod verifier;
mod workspace;

use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chrono::{DateTime, Utc};
use clap::Parser;
use crony_domain::{DeliverableSpec, VerificationPolicy};
use crony_protocol::{
    ActiveRunClaim, ResolvedSecret, RunnerCapability, RunnerModel, RunnerToServer, ServerToRunner,
};
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Digest;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};
use url::Url;
use uuid::Uuid;

use adapter::{
    AdapterArtifact, AdapterControl, AdapterEvent, AdapterEventSink, AdapterExit, AdapterRegistry,
    AdapterRegistryConfig, AdapterRunRequest, AgentAdapter, CopilotSdkConfig,
};
use workspace::{WorkspaceCleanup, WorkspaceDisposition, WorkspaceLease, WorkspaceManager};

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
    resume_workspace_base_commit: Option<String>,
    verification_policy: VerificationPolicy,
    write_scope: Vec<String>,
    deliverable: Option<DeliverableSpec>,
    secrets: Vec<ResolvedSecret>,
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
    let adapters = Arc::new(AdapterRegistry::new(AdapterRegistryConfig {
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
    }));
    loop {
        let delay = match run_connection(
            args.clone(),
            active_runs.clone(),
            seen_commands.clone(),
            outbound.clone(),
            adapters.clone(),
            workspaces.clone(),
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
        }
    });

    info!(runner_id = %args.runner_id, %connection_epoch, server = %args.server_ws, "runner connected");
    let mut reconnect_delay = Duration::from_secs(2);
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
                    resume_workspace_base_commit: None,
                    verification_policy,
                    write_scope,
                    deliverable,
                    secrets,
                };
                if let Err(error) = validate_assignment_source(&workspaces, &assignment) {
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
                    },
                );
                let task_workspaces = workspaces.clone();
                let runner_id = args.runner_id.clone();
                let task_outbound = outbound.clone();
                let task_runs = active_runs.clone();
                tokio::spawn(async move {
                    if let Err(error) = execute_assignment(
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
                    resume_workspace_base_commit: workspace_base_commit,
                    verification_policy,
                    write_scope,
                    deliverable,
                    secrets,
                };
                if let Err(error) = validate_assignment_source(&workspaces, &assignment) {
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
                    warn!(run_id = %assignment.run_id, "duplicate resume command ignored");
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
                    },
                );
                let task_workspaces = workspaces.clone();
                let runner_id = args.runner_id.clone();
                let task_outbound = outbound.clone();
                let task_runs = active_runs.clone();
                tokio::spawn(async move {
                    if let Err(error) = execute_assignment(
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
            }
            ServerToRunner::ControlMessage {
                run_id,
                actor_id,
                lease_token: _,
                text,
                ..
            } => {
                if let Some(active) = active_runs.get(&run_id) {
                    let _ = active
                        .control
                        .send(AdapterControl::Steer { actor_id, text });
                    info!(%run_id, %actor_id, "accepted fenced control message");
                } else {
                    warn!(%run_id, "control message arrived for inactive run");
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
                let duplicate = seen_commands.insert(command_id, ()).is_some();
                let applied = duplicate
                    || active_runs.get(&run_id).is_some_and(|active| {
                        active
                            .control
                            .send(AdapterControl::CircuitBreaker { stage, reason })
                            .is_ok()
                    });
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
                        "circuit-breaker command had no active provider".to_owned()
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
            AdapterEvent::Artifact(artifact) => match std::fs::read(&artifact.path) {
                Ok(bytes) => {
                    if let Ok(mut artifacts) = self.artifacts.lock() {
                        artifacts.push(artifact.clone());
                    }
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
    let request = AdapterRunRequest {
        run_id: assignment.run_id,
        mission_id: assignment.mission_id,
        task_id: assignment.task_id,
        agent_id: assignment.agent_id,
        mission_title: assignment.mission_title.clone(),
        model: assignment.model.clone(),
        reasoning_effort: assignment.reasoning_effort.clone(),
        workspace: workspace.path.clone(),
        environment: assignment
            .secrets
            .iter()
            .map(|secret| (secret.env_name.clone(), secret.value.clone()))
            .collect(),
    };
    let artifacts = Arc::new(Mutex::new(Vec::<AdapterArtifact>::new()));
    let terminal = Arc::new(Mutex::new(None::<BufferedTerminal>));
    let sink: Arc<dyn AdapterEventSink> = Arc::new(RunnerEventSink {
        outbound: outbound.clone(),
        runner_id: runner_id.clone(),
        assignment: assignment.clone(),
        workspace: workspace.clone(),
        artifacts: artifacts.clone(),
        terminal: terminal.clone(),
    });
    let execution = if let Some(session_id) = resume_session_id {
        adapter.resume(request, &session_id, controls, sink).await
    } else {
        adapter.execute(request, controls, sink).await
    };
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
        let terminal =
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
        match terminal {
            BufferedTerminal::Completed(summary) => {
                send_verification_events(
                    &outbound,
                    &runner_id,
                    &assignment,
                    &workspace,
                    &artifacts,
                    &mut artifact_acks,
                    &summary,
                )
                .await;
            }
            BufferedTerminal::Failed(error) => send_run_event(
                &outbound,
                &runner_id,
                &assignment,
                "run.failed",
                json!({"error": error}),
            ),
            BufferedTerminal::Cancelled(reason) => send_run_event(
                &outbound,
                &runner_id,
                &assignment,
                "run.cancelled",
                json!({"reason": reason}),
            ),
        }
    }
    let cleanup = workspaces.finalize(&workspace).await;
    send_workspace_cleanup_event(&outbound, &runner_id, &assignment, &workspace, cleanup);
    execution?;
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

async fn send_verification_events(
    outbound: &OutboundBus,
    runner_id: &str,
    assignment: &Assignment,
    workspace: &WorkspaceLease,
    artifacts: &Arc<Mutex<Vec<AdapterArtifact>>>,
    artifact_acks: &mut mpsc::UnboundedReceiver<ArtifactAck>,
    completion_summary: &str,
) {
    send_run_event(
        outbound,
        runner_id,
        assignment,
        "run.verification_started",
        json!({
            "check_count": assignment.verification_policy.checks.len(),
        }),
    );
    let artifacts = artifacts
        .lock()
        .map(|artifacts| artifacts.clone())
        .unwrap_or_default();
    let report =
        verifier::verify(&assignment.verification_policy, &workspace.path, &artifacts).await;
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
        return;
    }
    let deliverable_linkage = if let Some(spec) = &assignment.deliverable {
        let exported = match deliverable::export(
            assignment.run_id,
            spec,
            workspace,
            &report,
            &artifacts,
            &assignment.write_scope,
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
                return;
            }
        };
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
            "integration_state": if exported.form == crony_domain::DeliverableForm::ReviewOnlyReport {
                "not_applicable"
            } else {
                "ready_for_review"
            },
        });
        let mut stored = None;
        for _ in 0..6 {
            send_run_event(
                outbound,
                runner_id,
                assignment,
                "run.deliverable_upload",
                upload_payload.clone(),
            );
            let attempt = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let ack = artifact_acks
                        .recv()
                        .await
                        .context("artifact acknowledgment channel closed")?;
                    if ack.artifact_role == "source_deliverable" && ack.sha256 == deliverable_sha256
                    {
                        return Ok::<Uuid, anyhow::Error>(ack.artifact_id);
                    }
                }
            })
            .await;
            match attempt {
                Ok(Ok(artifact_id)) => {
                    stored = Some(artifact_id);
                    break;
                }
                Ok(Err(error)) => {
                    send_run_event(
                        outbound,
                        runner_id,
                        assignment,
                        "run.failed",
                        json!({"error": format!("source deliverable acknowledgment failed: {error:#}")}),
                    );
                    return;
                }
                Err(_) => {}
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
            return;
        }
        Some((exported.verification_sha256, deliverable_sha256))
    } else {
        None
    };
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
}

fn send_workspace_cleanup_event(
    outbound: &OutboundBus,
    runner_id: &str,
    assignment: &Assignment,
    workspace: &WorkspaceLease,
    cleanup: Result<WorkspaceCleanup>,
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
    use chrono::Duration as ChronoDuration;

    use super::*;

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
}
