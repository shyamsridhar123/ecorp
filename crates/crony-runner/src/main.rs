use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use crony_protocol::{RunnerCapability, RunnerToServer, ServerToRunner};
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

type ActiveInputs = Arc<DashMap<Uuid, mpsc::UnboundedSender<String>>>;

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

    loop {
        if let Err(error) = run_connection(args.clone()).await {
            warn!(%error, "runner connection ended; retrying");
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn run_connection(args: Args) -> Result<()> {
    let (socket, _) = connect_async(&args.server_ws)
        .await
        .with_context(|| format!("connect to {}", args.server_ws))?;
    let (mut socket_tx, mut socket_rx) = socket.split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<RunnerToServer>();
    let active_inputs: ActiveInputs = Arc::new(DashMap::new());

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
    out_tx
        .send(RunnerToServer::Register {
            runner_id: args.runner_id.clone(),
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
        })
        .map_err(|_| anyhow!("runner writer stopped before registration"))?;

    let heartbeat_tx = out_tx.clone();
    let heartbeat_runner_id = args.runner_id.clone();
    let heartbeat = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(10));
        loop {
            tick.tick().await;
            if heartbeat_tx
                .send(RunnerToServer::Heartbeat {
                    runner_id: heartbeat_runner_id.clone(),
                })
                .is_err()
            {
                break;
            }
        }
    });

    info!(runner_id = %args.runner_id, server = %args.server_ws, "runner connected");
    while let Some(message) = socket_rx.next().await {
        let text = match message {
            Ok(Message::Text(text)) => text,
            Ok(Message::Close(_)) => break,
            Ok(_) => continue,
            Err(error) => return Err(error.into()),
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
                let task_args = args.clone();
                let task_tx = out_tx.clone();
                let task_inputs = active_inputs.clone();
                tokio::spawn(async move {
                    let runner_id = task_args.runner_id.clone();
                    if let Err(error) = execute_assignment(
                        task_args,
                        assignment.clone(),
                        task_tx.clone(),
                        task_inputs,
                    )
                    .await
                    {
                        error!(%error, run_id = %assignment.run_id, "run execution failed");
                        send_run_event(
                            &task_tx,
                            &runner_id,
                            &assignment,
                            "run.failed",
                            json!({"error": error.to_string()}),
                        );
                    }
                });
            }
            ServerToRunner::ControlMessage {
                run_id,
                actor_id,
                text,
                ..
            } => {
                if let Some(input) = active_inputs.get(&run_id) {
                    let _ = input.send(json!({"actor_id": actor_id, "text": text}).to_string());
                } else {
                    warn!(%run_id, "control message arrived for inactive run");
                }
            }
            ServerToRunner::StopRun { run_id, reason, .. } => {
                warn!(%run_id, %reason, "stop command is not implemented in the first slice");
            }
        }
    }

    heartbeat.abort();
    writer.abort();
    Ok(())
}

async fn execute_assignment(
    args: Args,
    assignment: Assignment,
    out_tx: mpsc::UnboundedSender<RunnerToServer>,
    active_inputs: ActiveInputs,
) -> Result<()> {
    let run_dir = args.workspace.join(assignment.run_id.to_string());
    tokio::fs::create_dir_all(&run_dir)
        .await
        .with_context(|| format!("create run directory {}", run_dir.display()))?;

    send_run_event(
        &out_tx,
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
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<String>();
    active_inputs.insert(assignment.run_id, input_tx);

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
    let stderr_tx = out_tx.clone();
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            send_run_event(
                &stderr_tx,
                &stderr_runner_id,
                &stderr_assignment,
                "run.output",
                json!({"stream": "stderr", "text": line}),
            );
        }
    });

    let mut completed = false;
    let mut lines = BufReader::new(stdout).lines();
    while let Some(line) = lines.next_line().await? {
        let event: Value = match serde_json::from_str(&line) {
            Ok(event) => event,
            Err(_) => {
                send_run_event(
                    &out_tx,
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
                    &out_tx,
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
                    &out_tx,
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
                completed = true;
                send_run_event(
                    &out_tx,
                    &args.runner_id,
                    &assignment,
                    "run.completed",
                    json!({
                        "summary": event.get("summary").and_then(Value::as_str).unwrap_or("Mission artifact completed")
                    }),
                );
            }
            "failed" => {
                completed = true;
                send_run_event(
                    &out_tx,
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
                    &out_tx,
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

    let status = child.wait().await?;
    active_inputs.remove(&assignment.run_id);
    input_writer.abort();
    let _ = stderr_task.await;
    if !status.success() && !completed {
        return Err(anyhow!("fake agent exited with {status}"));
    }
    if !completed {
        return Err(anyhow!("fake agent exited without a terminal event"));
    }
    Ok(())
}

fn send_run_event(
    out_tx: &mpsc::UnboundedSender<RunnerToServer>,
    runner_id: &str,
    assignment: &Assignment,
    event_type: &str,
    payload: Value,
) {
    let _ = out_tx.send(RunnerToServer::RunEvent {
        event_id: Uuid::new_v4(),
        runner_id: runner_id.to_owned(),
        corp_id: assignment.corp_id,
        run_id: assignment.run_id,
        agent_id: assignment.agent_id,
        event_type: event_type.to_owned(),
        payload,
    });
}
