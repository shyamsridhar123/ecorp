use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use crony_protocol::{ActiveRunClaim, RunnerCapability, RunnerToServer, ServerToRunner};
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::mpsc,
};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};
use uuid::Uuid;

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

    #[arg(
        long,
        env = "CRONY_RUNNER_WORKSPACE",
        default_value = "./output/runner"
    )]
    workspace: PathBuf,

    #[arg(
        long,
        env = "CRONY_FAKE_AGENT_SCRIPT",
        default_value = "./scripts/fake-agent.mjs"
    )]
    fake_agent_script: PathBuf,
}

#[derive(Debug, Clone)]
struct Assignment {
    corp_id: Uuid,
    room_id: Uuid,
    mission_id: Uuid,
    task_id: Uuid,
    run_id: Uuid,
    agent_id: Uuid,
    mission_title: String,
}

#[derive(Clone)]
struct ActiveRunControl {
    assignment_token: Uuid,
    input: mpsc::UnboundedSender<String>,
    stop: mpsc::UnboundedSender<String>,
}

type ActiveRuns = Arc<DashMap<Uuid, ActiveRunControl>>;

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
    tokio::fs::create_dir_all(&args.workspace)
        .await
        .with_context(|| format!("create runner workspace {}", args.workspace.display()))?;

    let active_runs: ActiveRuns = Arc::new(DashMap::new());
    let outbound = OutboundBus::default();
    loop {
        let delay = match run_connection(args.clone(), active_runs.clone(), outbound.clone()).await
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
) -> Result<Duration> {
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
    out_tx
        .send(RunnerToServer::Register {
            runner_id: args.runner_id.clone(),
            connection_epoch,
            hostname,
            os: std::env::consts::OS.to_owned(),
            capabilities: vec![
                RunnerCapability {
                    name: "fake-process".to_owned(),
                    available: true,
                    detail: Some("deterministic child-process adapter".to_owned()),
                },
                RunnerCapability {
                    name: "workspace-isolation".to_owned(),
                    available: true,
                    detail: Some(args.workspace.display().to_string()),
                },
            ],
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
            ServerToRunner::Registered { runner_id } => {
                info!(%runner_id, "runner registration accepted");
            }
            ServerToRunner::StartRun {
                corp_id,
                room_id,
                mission_id,
                task_id,
                run_id,
                agent_id,
                assignment_token,
                mission_title,
            } => {
                let assignment = Assignment {
                    corp_id,
                    room_id,
                    mission_id,
                    task_id,
                    run_id,
                    agent_id,
                    mission_title,
                };
                if active_runs.contains_key(&assignment.run_id) {
                    warn!(run_id = %assignment.run_id, "duplicate start command ignored");
                    continue;
                }
                let (input_tx, input_rx) = mpsc::unbounded_channel::<String>();
                let (stop_tx, stop_rx) = mpsc::unbounded_channel::<String>();
                active_runs.insert(
                    assignment.run_id,
                    ActiveRunControl {
                        assignment_token,
                        input: input_tx,
                        stop: stop_tx,
                    },
                );
                let task_args = args.clone();
                let task_outbound = outbound.clone();
                let task_runs = active_runs.clone();
                tokio::spawn(async move {
                    let runner_id = task_args.runner_id.clone();
                    if let Err(error) = execute_assignment(
                        task_args,
                        assignment.clone(),
                        task_outbound.clone(),
                        input_rx,
                        stop_rx,
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
            ServerToRunner::ControlMessage {
                run_id,
                actor_id,
                lease_token: _,
                text,
                ..
            } => {
                if let Some(active) = active_runs.get(&run_id) {
                    let _ = active.input.send(
                        json!({
                            "actor_id": actor_id,
                            "text": text
                        })
                        .to_string(),
                    );
                    info!(%run_id, %actor_id, "accepted fenced control message");
                } else {
                    warn!(%run_id, "control message arrived for inactive run");
                }
            }
            ServerToRunner::StopRun { run_id, reason, .. } => {
                if let Some(active) = active_runs.get(&run_id) {
                    let _ = active.stop.send(reason);
                } else {
                    warn!(%run_id, "stop command arrived for inactive run");
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

async fn execute_assignment(
    args: Args,
    assignment: Assignment,
    outbound: OutboundBus,
    mut input_rx: mpsc::UnboundedReceiver<String>,
    mut stop_rx: mpsc::UnboundedReceiver<String>,
) -> Result<()> {
    let run_dir = args.workspace.join(assignment.run_id.to_string());
    tokio::fs::create_dir_all(&run_dir)
        .await
        .with_context(|| format!("create run directory {}", run_dir.display()))?;

    send_run_event(
        &outbound,
        &args.runner_id,
        &assignment,
        "run.started",
        json!({
            "room_id": assignment.room_id,
            "mission_id": assignment.mission_id,
            "task_id": assignment.task_id,
            "workspace": run_dir,
        }),
    );

    let mut child = Command::new("node")
        .arg(&args.fake_agent_script)
        .arg("--run-id")
        .arg(assignment.run_id.to_string())
        .arg("--workdir")
        .arg(&run_dir)
        .arg("--mission")
        .arg(&assignment.mission_title)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| {
            format!(
                "spawn fake agent script {}",
                args.fake_agent_script.display()
            )
        })?;

    let stdin = child.stdin.take().context("fake agent stdin missing")?;
    let stdout = child.stdout.take().context("fake agent stdout missing")?;
    let stderr = child.stderr.take().context("fake agent stderr missing")?;

    let input_writer = tokio::spawn(async move {
        let mut stdin = stdin;
        while let Some(line) = input_rx.recv().await {
            if stdin.write_all(line.as_bytes()).await.is_err()
                || stdin.write_all(b"\n").await.is_err()
            {
                break;
            }
        }
    });

    let stderr_assignment = assignment.clone();
    let stderr_runner_id = args.runner_id.clone();
    let stderr_outbound = outbound.clone();
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            send_run_event(
                &stderr_outbound,
                &stderr_runner_id,
                &stderr_assignment,
                "run.output",
                json!({"stream": "stderr", "text": line}),
            );
        }
    });

    let mut terminal_event = false;
    let mut cancelled = false;
    let mut lines = BufReader::new(stdout).lines();
    loop {
        tokio::select! {
            reason = stop_rx.recv() => {
                if let Some(reason) = reason {
                    child.kill().await.context("kill stopped agent process")?;
                    send_run_event(
                        &outbound,
                        &args.runner_id,
                        &assignment,
                        "run.cancelled",
                        json!({"reason": reason}),
                    );
                    terminal_event = true;
                    cancelled = true;
                    break;
                }
            }
            line = lines.next_line() => {
                let Some(line) = line? else {
                    break;
                };
                let event: Value = match serde_json::from_str(&line) {
                    Ok(event) => event,
                    Err(_) => {
                        send_run_event(
                            &outbound,
                            &args.runner_id,
                            &assignment,
                            "run.output",
                            json!({"stream": "stdout", "text": line}),
                        );
                        continue;
                    }
                };
                let kind = event
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("output");
                match kind {
                    "status" => {
                        send_run_event(
                            &outbound,
                            &args.runner_id,
                            &assignment,
                            "run.status",
                            json!({
                                "status": event.get("status").and_then(Value::as_str).unwrap_or("working"),
                                "station": event.get("station").and_then(Value::as_str).unwrap_or("terminal"),
                                "message": event.get("message").and_then(Value::as_str).unwrap_or("")
                            }),
                        );
                    }
                    "artifact" => {
                        let relative = event
                            .get("path")
                            .and_then(Value::as_str)
                            .context("artifact event missing path")?;
                        let artifact_path = run_dir.join(relative);
                        let bytes = tokio::fs::read(&artifact_path)
                            .await
                            .with_context(|| format!("read artifact {}", artifact_path.display()))?;
                        let sha256 = hex::encode(Sha256::digest(&bytes));
                        send_run_event(
                            &outbound,
                            &args.runner_id,
                            &assignment,
                            "run.artifact",
                            json!({
                                "path": artifact_path,
                                "sha256": sha256,
                                "bytes": bytes.len(),
                                "media_type": event.get("media_type").and_then(Value::as_str).unwrap_or("text/markdown")
                            }),
                        );
                    }
                    "completed" => {
                        terminal_event = true;
                        send_run_event(
                            &outbound,
                            &args.runner_id,
                            &assignment,
                            "run.completed",
                            json!({
                                "summary": event.get("summary").and_then(Value::as_str).unwrap_or("Mission artifact completed")
                            }),
                        );
                    }
                    "failed" => {
                        terminal_event = true;
                        send_run_event(
                            &outbound,
                            &args.runner_id,
                            &assignment,
                            "run.failed",
                            json!({
                                "error": event.get("error").and_then(Value::as_str).unwrap_or("Fake agent reported failure")
                            }),
                        );
                    }
                    _ => {
                        send_run_event(
                            &outbound,
                            &args.runner_id,
                            &assignment,
                            "run.output",
                            json!({
                                "stream": event.get("stream").and_then(Value::as_str).unwrap_or("stdout"),
                                "text": event.get("text").and_then(Value::as_str).unwrap_or(&line)
                            }),
                        );
                    }
                }
            }
        }
    }

    let status = child.wait().await?;
    input_writer.abort();
    let _ = stderr_task.await;
    if cancelled {
        return Ok(());
    }
    if !status.success() && !terminal_event {
        return Err(anyhow!("fake agent exited with {status}"));
    }
    if !terminal_event {
        return Err(anyhow!("fake agent exited without a terminal event"));
    }
    Ok(())
}

fn send_run_event(
    outbound: &OutboundBus,
    runner_id: &str,
    assignment: &Assignment,
    event_type: &str,
    payload: Value,
) {
    outbound.send(RunnerToServer::RunEvent {
        event_id: Uuid::new_v4(),
        runner_id: runner_id.to_owned(),
        corp_id: assignment.corp_id,
        run_id: assignment.run_id,
        agent_id: assignment.agent_id,
        event_type: event_type.to_owned(),
        payload,
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
