use std::{ffi::OsString, path::PathBuf, process::Stdio, sync::Arc};

use anyhow::{Context, anyhow};
use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::mpsc,
};

use super::{
    AdapterArtifact, AdapterCapabilities, AdapterControl, AdapterError, AdapterEvent,
    AdapterEventSink, AdapterExit, AdapterRunRequest, AgentAdapter, FeatureSupport, UsageSnapshot,
};

#[derive(Debug, Clone, Copy)]
pub enum ExternalFlavor {
    ClaudeCode,
    OpenCode,
}

impl ExternalFlavor {
    fn id(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::OpenCode => "opencode",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Anthropic Claude Code",
            Self::OpenCode => "OpenCode",
        }
    }
}

#[derive(Clone)]
pub struct ExternalCliAdapter {
    flavor: ExternalFlavor,
    command: PathBuf,
    prefix_args: Vec<OsString>,
    available: bool,
}

impl ExternalCliAdapter {
    pub fn new_with_prefix(
        flavor: ExternalFlavor,
        command: PathBuf,
        prefix_args: Vec<OsString>,
    ) -> Self {
        Self::with_prefix_args(flavor, command, prefix_args)
    }

    fn with_prefix_args(
        flavor: ExternalFlavor,
        command: PathBuf,
        prefix_args: Vec<OsString>,
    ) -> Self {
        let available = std::process::Command::new(&command)
            .args(&prefix_args)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        Self {
            flavor,
            command,
            prefix_args,
            available,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.command);
        command.args(&self.prefix_args);
        command
    }

    async fn run(
        &self,
        request: AdapterRunRequest,
        resume_session_id: Option<&str>,
        mut controls: mpsc::UnboundedReceiver<AdapterControl>,
        sink: Arc<dyn AdapterEventSink>,
    ) -> Result<AdapterExit, AdapterError> {
        if !self.available {
            return Err(AdapterError::Runtime(anyhow!(
                "{} executable is unavailable: {}",
                self.display_name(),
                self.command.display()
            )));
        }
        tokio::fs::create_dir_all(&request.workspace).await?;
        sink.emit(AdapterEvent::Started {
            workspace: request.workspace.clone(),
        });

        let mut command = self.command();
        match self.flavor {
            ExternalFlavor::ClaudeCode => {
                command
                    .arg("--print")
                    .arg("--verbose")
                    .arg("--output-format")
                    .arg("stream-json")
                    .arg("--permission-mode")
                    .arg("acceptEdits");
                if let Some(session_id) = resume_session_id {
                    command.arg("--resume").arg(session_id);
                }
                command.arg(&request.mission_title);
            }
            ExternalFlavor::OpenCode => {
                command.arg("run").arg("--format").arg("json");
                if let Some(session_id) = resume_session_id {
                    command.arg("--session").arg(session_id);
                }
                command.arg(&request.mission_title);
            }
        }
        command
            .current_dir(&request.workspace)
            .envs(&request.environment)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .with_context(|| format!("spawn {}", self.display_name()))?;
        let stdin = child.stdin.take().context("provider stdin missing")?;
        let stdout = child.stdout.take().context("provider stdout missing")?;
        let stderr = child.stderr.take().context("provider stderr missing")?;
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

        let mut raw_output = Vec::new();
        let mut session_id = resume_session_id.map(str::to_owned);
        let mut cancelled = None;
        let mut lines = BufReader::new(stdout).lines();
        loop {
            tokio::select! {
                control = controls.recv() => match control {
                    Some(AdapterControl::Stop { reason })
                    | Some(AdapterControl::Interrupt { reason }) => {
                        child.kill().await.context("stop provider process")?;
                        cancelled = Some(reason);
                        break;
                    }
                    Some(AdapterControl::Steer { actor_id, text }) => {
                        let _ = input_tx.send(json!({
                            "type": "user",
                            "actor_id": actor_id,
                            "text": text,
                        }).to_string());
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
                        if stage == "stop" {
                            child.kill().await.context("stop provider at circuit breaker")?;
                            cancelled = Some(reason);
                            break;
                        }
                        let _ = input_tx.send(json!({
                            "type": "policy",
                            "stage": stage,
                            "reason": reason,
                        }).to_string());
                    }
                    None => {}
                },
                line = lines.next_line() => {
                    let Some(line) = line? else { break };
                    raw_output.extend_from_slice(line.as_bytes());
                    raw_output.push(b'\n');
                    if let Ok(value) = serde_json::from_str::<Value>(&line) {
                        if session_id.is_none() {
                            session_id = find_string(&value, &["session_id", "sessionId", "sessionID"]);
                            if let Some(id) = &session_id {
                                sink.emit(AdapterEvent::Session { session_id: id.clone() });
                            }
                        }
                        if let Some(usage) = value.get("usage") {
                            sink.emit(AdapterEvent::Usage(UsageSnapshot {
                                input_tokens: find_u64(usage, &["input_tokens", "inputTokens"]).unwrap_or_default(),
                                output_tokens: find_u64(usage, &["output_tokens", "outputTokens"]).unwrap_or_default(),
                                cost_microusd: find_u64(usage, &["cost_microusd", "costMicrousd"]).unwrap_or_default(),
                            }));
                        }
                        if let Some(text) = find_string(&value, &["text", "content", "message"]) {
                            sink.emit(AdapterEvent::Output {
                                stream: "provider".to_owned(),
                                text,
                            });
                        }
                    } else {
                        sink.emit(AdapterEvent::Output {
                            stream: "provider".to_owned(),
                            text: line,
                        });
                    }
                }
            }
        }
        let status = child.wait().await?;
        input_writer.abort();
        stderr_task.abort();
        if let Some(reason) = cancelled {
            sink.emit(AdapterEvent::Cancelled { reason });
            return Ok(AdapterExit::Cancelled);
        }
        if !status.success() {
            sink.emit(AdapterEvent::Failed {
                error: format!("{} exited with {status}", self.display_name()),
            });
            return Ok(AdapterExit::Failed);
        }
        let session_id = session_id.unwrap_or_else(|| format!("{}-{}", self.id(), request.run_id));
        sink.emit(AdapterEvent::Session {
            session_id: session_id.clone(),
        });
        let evidence_path = request
            .workspace
            .join(format!("{}-evidence.json", self.id()));
        let evidence = serde_json::to_vec_pretty(&json!({
            "provider": self.id(),
            "session_id": session_id,
            "stdout_sha256": hex::encode(Sha256::digest(&raw_output)),
            "stdout_bytes": raw_output.len(),
            "exit_success": true,
        }))
        .context("serialize provider evidence")?;
        tokio::fs::write(&evidence_path, &evidence).await?;
        sink.emit(AdapterEvent::Artifact(AdapterArtifact {
            path: evidence_path,
            sha256: hex::encode(Sha256::digest(&evidence)),
            bytes: evidence.len(),
            media_type: "application/json".to_owned(),
        }));
        sink.emit(AdapterEvent::Completed {
            summary: format!(
                "{} completed with normalized evidence.",
                self.display_name()
            ),
        });
        Ok(AdapterExit::Completed)
    }
}

#[async_trait]
impl AgentAdapter for ExternalCliAdapter {
    fn id(&self) -> &'static str {
        self.flavor.id()
    }

    fn display_name(&self) -> &'static str {
        self.flavor.display_name()
    }

    fn capabilities(&self) -> AdapterCapabilities {
        let available = if self.available {
            FeatureSupport::Supported
        } else {
            FeatureSupport::Unsupported {
                reason: format!("{} is not installed", self.command.display()),
            }
        };
        AdapterCapabilities {
            spawn: available.clone(),
            stream: available.clone(),
            steer: FeatureSupport::Unsupported {
                reason: "batch CLI mode does not guarantee live steering".to_owned(),
            },
            interrupt: available.clone(),
            stop: available.clone(),
            resume: available.clone(),
            usage: available.clone(),
            artifacts: available,
        }
    }

    async fn execute(
        &self,
        request: AdapterRunRequest,
        controls: mpsc::UnboundedReceiver<AdapterControl>,
        sink: Arc<dyn AdapterEventSink>,
    ) -> Result<AdapterExit, AdapterError> {
        self.run(request, None, controls, sink).await
    }

    async fn resume(
        &self,
        request: AdapterRunRequest,
        session_id: &str,
        controls: mpsc::UnboundedReceiver<AdapterControl>,
        sink: Arc<dyn AdapterEventSink>,
    ) -> Result<AdapterExit, AdapterError> {
        self.run(request, Some(session_id), controls, sink).await
    }
}

fn find_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(|value| match value {
            Value::String(value) => Some(value.clone()),
            Value::Object(value) => value.get("text").and_then(Value::as_str).map(str::to_owned),
            _ => None,
        })
}

fn find_u64(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(Value::as_u64)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct RecordingSink {
        events: Mutex<Vec<AdapterEvent>>,
    }

    impl AdapterEventSink for RecordingSink {
        fn emit(&self, event: AdapterEvent) {
            self.events.lock().expect("event lock").push(event);
        }
    }

    fn adapter(flavor: ExternalFlavor) -> ExternalCliAdapter {
        let script =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/fake-external-agent.mjs");
        ExternalCliAdapter::new_with_prefix(
            flavor,
            PathBuf::from("node"),
            vec![
                script.into_os_string(),
                OsString::from("--provider"),
                OsString::from(flavor.id()),
            ],
        )
    }

    fn request(provider: &str) -> AdapterRunRequest {
        let run_id = uuid::Uuid::new_v4();
        AdapterRunRequest {
            run_id,
            mission_id: uuid::Uuid::new_v4(),
            task_id: uuid::Uuid::new_v4(),
            agent_id: uuid::Uuid::new_v4(),
            mission_title: format!("common {provider} sample"),
            model: None,
            reasoning_effort: None,
            workspace: std::env::temp_dir()
                .join("crony-external-adapter-tests")
                .join(run_id.to_string()),
            environment: std::collections::HashMap::new(),
        }
    }

    #[tokio::test]
    async fn claude_and_opencode_produce_equivalent_evidence() {
        for flavor in [ExternalFlavor::ClaudeCode, ExternalFlavor::OpenCode] {
            let adapter = adapter(flavor);
            assert!(adapter.capabilities().spawn.supported());
            let request = request(adapter.id());
            let workspace = request.workspace.clone();
            let (_tx, rx) = mpsc::unbounded_channel();
            let sink = Arc::new(RecordingSink::default());
            let exit = adapter
                .execute(request, rx, sink.clone())
                .await
                .expect("provider execution");
            assert_eq!(exit, AdapterExit::Completed);
            let events = sink.events.lock().expect("event lock");
            assert!(
                events
                    .iter()
                    .any(|event| matches!(event, AdapterEvent::Session { .. }))
            );
            assert!(
                events
                    .iter()
                    .any(|event| matches!(event, AdapterEvent::Usage(_)))
            );
            assert!(
                events
                    .iter()
                    .any(|event| matches!(event, AdapterEvent::Artifact(_)))
            );
            assert!(
                workspace
                    .join(format!("{}-evidence.json", adapter.id()))
                    .is_file()
            );
            drop(events);
            let _ = std::fs::remove_dir_all(workspace);
        }
    }
}
