mod adapter;
mod verifier;
mod workspace;

use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use crony_domain::VerificationPolicy;
use crony_protocol::{
    ActiveRunClaim, ResolvedSecret, RunnerCapability, RunnerToServer, ServerToRunner,
};
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};
use uuid::Uuid;

use adapter::{
    AdapterArtifact, AdapterControl, AdapterEvent, AdapterEventSink, AdapterExit, AdapterRegistry,
    AdapterRunRequest, AgentAdapter,
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
}

#[derive(Debug, Clone)]
struct Assignment {
    corp_id: Uuid,
    room_id: Uuid,
    mission_id: Uuid,
    task_id: Uuid,
    run_id: Uuid,
    workspace_run_id: Uuid,
    agent_id: Uuid,
    adapter: String,
    mission_title: String,
    verification_policy: VerificationPolicy,
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
}

type ActiveRuns = Arc<DashMap<Uuid, ActiveRunControl>>;

fn default_codex_command() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from("codex.exe")
    } else {
        PathBuf::from("codex")
    }
}

#[derive(Clone, Default)]
struct OutboundBus {
    state: Arc<Mutex<OutboundState>>,
}

#[derive(Default)]
struct OutboundState {
    connection: Option<mpsc::UnboundedSender<RunnerToServer>>,
    pending: VecDeque<RunnerToServer>,
}

impl OutboundBus {
    fn attach(&self, connection: mpsc::UnboundedSender<RunnerToServer>) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.connection = Some(connection.clone());
        while let Some(message) = state.pending.pop_front() {
            if connection.send(message.clone()).is_err() {
                state.pending.push_front(message);
                state.connection = None;
                break;
            }
        }
    }

    fn detach(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.connection = None;
        }
    }

    fn send(&self, message: RunnerToServer) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let delivered = state
            .connection
            .as_ref()
            .is_some_and(|connection| connection.send(message.clone()).is_ok());
        if delivered {
            return;
        }
        state.connection = None;
        if state.pending.len() >= 2_000 {
            state.pending.pop_front();
        }
        state.pending.push_back(message);
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
    let adapters = Arc::new(AdapterRegistry::new(
        args.fake_agent_script.clone(),
        codex_command,
    ));
    loop {
        let delay = match run_connection(
            args.clone(),
            active_runs.clone(),
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
    let mut capabilities = adapters
        .all()
        .into_iter()
        .map(|adapter| RunnerCapability {
            name: adapter.id().to_owned(),
            available: adapter.capabilities().spawn.supported(),
            detail: Some(format!(
                "{}; {}",
                adapter.display_name(),
                adapter.capabilities().summary()
            )),
        })
        .collect::<Vec<_>>();
    capabilities.push(RunnerCapability {
        name: "workspace-isolation".to_owned(),
        available: true,
        detail: Some(format!(
            "root={}; repository={}; base={}",
            workspaces.root().display(),
            workspaces.repository().display(),
            workspaces.base_ref()
        )),
    });
    capabilities.push(RunnerCapability {
        name: "secret-delivery".to_owned(),
        available: true,
        detail: Some(
            "task-scoped, expiring broker grants; environment injection is reduced assurance"
                .to_owned(),
        ),
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
    outbound.attach(out_tx.clone());

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
                verification_policy,
                secrets,
            } => {
                let assignment = Assignment {
                    corp_id,
                    room_id,
                    mission_id,
                    task_id,
                    run_id,
                    workspace_run_id: run_id,
                    agent_id,
                    adapter,
                    mission_title,
                    verification_policy,
                    secrets,
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
                active_runs.insert(
                    assignment.run_id,
                    ActiveRunControl {
                        assignment_token,
                        control: control_tx,
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
                        control_rx,
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
                verification_policy,
                secrets,
            } => {
                let assignment = Assignment {
                    corp_id,
                    room_id,
                    mission_id,
                    task_id,
                    run_id,
                    workspace_run_id,
                    agent_id,
                    adapter,
                    mission_title: prompt,
                    verification_policy,
                    secrets,
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
                active_runs.insert(
                    assignment.run_id,
                    ActiveRunControl {
                        assignment_token,
                        control: control_tx,
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
                        control_rx,
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
            AdapterEvent::Artifact(artifact) => {
                if let Ok(mut artifacts) = self.artifacts.lock() {
                    artifacts.push(artifact.clone());
                }
                (
                    "run.artifact",
                    json!({
                        "path": artifact.path,
                        "sha256": artifact.sha256,
                        "bytes": artifact.bytes,
                        "media_type": artifact.media_type,
                    }),
                )
            }
            AdapterEvent::Usage(usage) => (
                "run.usage",
                json!({
                    "input_tokens": usage.input_tokens,
                    "output_tokens": usage.output_tokens,
                    "cost_microusd": usage.cost_microusd,
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

async fn execute_assignment(
    workspaces: Arc<WorkspaceManager>,
    runner_id: String,
    assignment: Assignment,
    adapter: Arc<dyn AgentAdapter>,
    outbound: OutboundBus,
    controls: mpsc::UnboundedReceiver<AdapterControl>,
    resume_session_id: Option<String>,
) -> Result<()> {
    let workspace = workspaces
        .prepare(assignment.task_id, assignment.workspace_run_id)
        .await
        .context("prepare isolated task worktree")?;
    let request = AdapterRunRequest {
        run_id: assignment.run_id,
        mission_id: assignment.mission_id,
        task_id: assignment.task_id,
        agent_id: assignment.agent_id,
        mission_title: assignment.mission_title.clone(),
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

async fn send_verification_events(
    outbound: &OutboundBus,
    runner_id: &str,
    assignment: &Assignment,
    workspace: &WorkspaceLease,
    artifacts: &Arc<Mutex<Vec<AdapterArtifact>>>,
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
    send_run_event(
        outbound,
        runner_id,
        assignment,
        "run.verification_passed",
        json!({"summary": report.summary}),
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
        run_id: assignment.run_id,
        agent_id: assignment.agent_id,
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
