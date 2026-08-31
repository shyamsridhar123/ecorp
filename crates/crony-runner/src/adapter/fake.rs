use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, anyhow};
use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::mpsc,
};
use uuid::Uuid;

use super::{
    AdapterArtifact, AdapterCapabilities, AdapterControl, AdapterError, AdapterEvent,
    AdapterEventSink, AdapterExit, AdapterRunRequest, AgentAdapter, FeatureSupport,
};

#[derive(Clone)]
pub struct FakeProcessAdapter {
    script: PathBuf,
}

impl FakeProcessAdapter {
    pub fn new(script: PathBuf) -> Self {
        Self { script }
    }
}

#[async_trait]
impl AgentAdapter for FakeProcessAdapter {
    fn id(&self) -> &'static str {
        "fake-process"
    }

    fn display_name(&self) -> &'static str {
        "Deterministic process"
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            spawn: FeatureSupport::Supported,
            stream: FeatureSupport::Supported,
            steer: FeatureSupport::Supported,
            interrupt: FeatureSupport::Unsupported {
                reason: "the deterministic process has no distinct interrupt state".to_owned(),
            },
            stop: FeatureSupport::Supported,
            resume: FeatureSupport::Unsupported {
                reason: "the deterministic process does not persist provider sessions".to_owned(),
            },
            usage: FeatureSupport::Unsupported {
                reason: "the deterministic process does not consume model tokens".to_owned(),
            },
            artifacts: FeatureSupport::Supported,
        }
    }

    async fn execute(
        &self,
        request: AdapterRunRequest,
        mut controls: mpsc::UnboundedReceiver<AdapterControl>,
        sink: Arc<dyn AdapterEventSink>,
    ) -> Result<AdapterExit, AdapterError> {
        tokio::fs::create_dir_all(&request.workspace)
            .await
            .with_context(|| format!("create run directory {}", request.workspace.display()))?;
        sink.emit(AdapterEvent::Started {
            workspace: request.workspace.clone(),
        });

        let mut child = Command::new("node")
            .arg(&self.script)
            .arg("--run-id")
            .arg(request.run_id.to_string())
            .arg("--workdir")
            .arg(&request.workspace)
            .arg("--mission")
            .arg(&request.mission_title)
            .env("CRONY_MISSION_ID", request.mission_id.to_string())
            .env("CRONY_TASK_ID", request.task_id.to_string())
            .env("CRONY_AGENT_ID", request.agent_id.to_string())
            .envs(&request.environment)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("spawn fake agent script {}", self.script.display()))?;

        let stdin = child.stdin.take().context("fake agent stdin missing")?;
        let stdout = child.stdout.take().context("fake agent stdout missing")?;
        let stderr = child.stderr.take().context("fake agent stderr missing")?;
        let (input_tx, mut input_rx) = mpsc::unbounded_channel::<String>();

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

        let stderr_sink = sink.clone();
        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                stderr_sink.emit(AdapterEvent::Output {
                    stream: "stderr".to_owned(),
                    text: line,
                });
            }
        });

        let mut terminal_event = false;
        let mut cancelled = false;
        let mut failed = false;
        let mut lines = BufReader::new(stdout).lines();
        loop {
            tokio::select! {
                control = controls.recv() => {
                    match control {
                        Some(AdapterControl::Steer { actor_id, text }) => {
                            let _ = input_tx.send(json!({"actor_id": actor_id, "text": text}).to_string());
                        }
                        Some(AdapterControl::Interrupt { reason }) => {
                            sink.emit(AdapterEvent::Output {
                                stream: "control".to_owned(),
                                text: format!("Interrupt unsupported by fake-process: {reason}"),
                            });
                        }
                        Some(AdapterControl::Stop { reason }) => {
                            child.kill().await.context("kill stopped agent process")?;
                            sink.emit(AdapterEvent::Cancelled { reason });
                            terminal_event = true;
                            cancelled = true;
                            break;
                        }
                        Some(AdapterControl::ApprovalDecision { approval_id, approved, note }) => {
                            let _ = input_tx.send(json!({
                                "type": "approval_decision",
                                "approval_id": approval_id,
                                "approved": approved,
                                "note": note,
                            }).to_string());
                        }
                        Some(AdapterControl::CircuitBreaker { stage, reason }) => {
                            if matches!(stage.as_str(), "suspend" | "stop") {
                                let reason =
                                    format!("Circuit breaker {stage} checkpoint: {reason}");
                                child.kill().await.context("kill breaker-stopped agent process")?;
                                sink.emit(AdapterEvent::Cancelled { reason });
                                terminal_event = true;
                                cancelled = true;
                                break;
                            }
                            let _ = input_tx.send(json!({
                                "type": "circuit_breaker",
                                "stage": stage,
                                "reason": reason,
                            }).to_string());
                        }
                        None => {}
                    }
                }
                line = lines.next_line() => {
                    let Some(line) = line? else {
                        break;
                    };
                    let event: Value = match serde_json::from_str(&line) {
                        Ok(event) => event,
                        Err(_) => {
                            sink.emit(AdapterEvent::Output {
                                stream: "stdout".to_owned(),
                                text: line,
                            });
                            continue;
                        }
                    };
                    let kind = event
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("output");
                    match kind {
                        "status" => sink.emit(AdapterEvent::Status {
                            status: event
                                .get("status")
                                .and_then(Value::as_str)
                                .unwrap_or("working")
                                .to_owned(),
                            station: event
                                .get("station")
                                .and_then(Value::as_str)
                                .unwrap_or("terminal")
                                .to_owned(),
                            message: event
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_owned(),
                        }),
                        "artifact" => {
                            let relative = event
                                .get("path")
                                .and_then(Value::as_str)
                                .context("artifact event missing path")?;
                            let artifact_path = request.workspace.join(relative);
                            let bytes = tokio::fs::read(&artifact_path)
                                .await
                                .with_context(|| format!("read artifact {}", artifact_path.display()))?;
                            sink.emit(AdapterEvent::Artifact(AdapterArtifact {
                                path: artifact_path,
                                sha256: hex::encode(Sha256::digest(&bytes)),
                                bytes: bytes.len(),
                                media_type: event
                                    .get("media_type")
                                    .and_then(Value::as_str)
                                    .unwrap_or("text/markdown")
                                    .to_owned(),
                            }));
                        }
                        "usage" => sink.emit(AdapterEvent::Usage(super::UsageSnapshot {
                            input_tokens: event
                                .get("input_tokens")
                                .and_then(Value::as_u64)
                                .unwrap_or_default(),
                            output_tokens: event
                                .get("output_tokens")
                                .and_then(Value::as_u64)
                                .unwrap_or_default(),
                            cost_microusd: event
                                .get("cost_microusd")
                                .and_then(Value::as_u64)
                                .unwrap_or_default(),
                        })),
                        "approval_requested" => {
                            let approval_id = event
                                .get("approval_id")
                                .and_then(Value::as_str)
                                .context("approval event omitted approval_id")
                                .and_then(|value| {
                                    Uuid::parse_str(value).context("approval id is invalid")
                                })?;
                            sink.emit(AdapterEvent::ApprovalRequested {
                                approval_id,
                                action_key: event
                                    .get("action_key")
                                    .and_then(Value::as_str)
                                    .context("approval event omitted action_key")?
                                    .to_owned(),
                                action: event
                                    .get("action")
                                    .and_then(Value::as_str)
                                    .context("approval event omitted action")?
                                    .to_owned(),
                                risk: event
                                    .get("risk")
                                    .and_then(Value::as_str)
                                    .context("approval event omitted risk")?
                                    .to_owned(),
                                rationale: event
                                    .get("rationale")
                                    .and_then(Value::as_str)
                                    .context("approval event omitted rationale")?
                                    .to_owned(),
                                required_roles: event
                                    .get("required_roles")
                                    .and_then(Value::as_array)
                                    .context("approval event omitted required_roles")?
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .map(str::to_owned)
                                    .collect(),
                                expires_in_seconds: event
                                    .get("expires_in_seconds")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(300),
                            });
                        }
                        "tool_activity" => sink.emit(AdapterEvent::ToolActivity {
                            signature: event
                                .get("signature")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown")
                                .to_owned(),
                            progressed: event
                                .get("progressed")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                            human_conversation: event
                                .get("human_conversation")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        }),
                        "completed" => {
                            terminal_event = true;
                            sink.emit(AdapterEvent::Completed {
                                summary: event
                                    .get("summary")
                                    .and_then(Value::as_str)
                                    .unwrap_or("Mission artifact completed")
                                    .to_owned(),
                            });
                        }
                        "failed" => {
                            terminal_event = true;
                            failed = true;
                            sink.emit(AdapterEvent::Failed {
                                error: event
                                    .get("error")
                                    .and_then(Value::as_str)
                                    .unwrap_or("Fake agent reported failure")
                                    .to_owned(),
                            });
                        }
                        "cancelled" => {
                            terminal_event = true;
                            cancelled = true;
                            sink.emit(AdapterEvent::Cancelled {
                                reason: event
                                    .get("reason")
                                    .and_then(Value::as_str)
                                    .unwrap_or("Fake agent cancelled")
                                    .to_owned(),
                            });
                        }
                        _ => sink.emit(AdapterEvent::Output {
                            stream: event
                                .get("stream")
                                .and_then(Value::as_str)
                                .unwrap_or("stdout")
                                .to_owned(),
                            text: event
                                .get("text")
                                .and_then(Value::as_str)
                                .unwrap_or(&line)
                                .to_owned(),
                        }),
                    }
                }
            }
        }

        let status = child.wait().await?;
        input_writer.abort();
        let _ = stderr_task.await;
        if cancelled {
            return Ok(AdapterExit::Cancelled);
        }
        if !status.success() && !terminal_event {
            return Err(AdapterError::Runtime(anyhow!(
                "fake agent exited with {status}"
            )));
        }
        if !terminal_event {
            return Err(AdapterError::Runtime(anyhow!(
                "fake agent exited without a terminal event"
            )));
        }
        if failed {
            Ok(AdapterExit::Failed)
        } else {
            Ok(AdapterExit::Completed)
        }
    }
}
