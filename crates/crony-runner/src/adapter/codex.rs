use std::{
    collections::{BTreeSet, HashSet, VecDeque},
    ffi::OsString,
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, anyhow};
use async_trait::async_trait;
use dashmap::DashMap;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, Command},
    sync::mpsc,
    time::Instant,
};
use uuid::Uuid;

use super::{
    AdapterArtifact, AdapterCapabilities, AdapterControl, AdapterError, AdapterEvent,
    AdapterEventSink, AdapterExit, AdapterRunRequest, AgentAdapter, FeatureSupport, UsageSnapshot,
};

const INITIALIZE_REQUEST_ID: u64 = 1;
const THREAD_REQUEST_ID: u64 = 2;
const TURN_REQUEST_ID: u64 = 3;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(90);
const INTERRUPT_TIMEOUT: Duration = Duration::from_secs(20);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone)]
pub struct CodexAdapter {
    command: PathBuf,
    prefix_args: Arc<Vec<OsString>>,
    available: bool,
    usage: Arc<DashMap<String, UsageSnapshot>>,
}

#[derive(Debug, Clone)]
enum CodexMode {
    Start,
    Resume { session_id: String },
}

#[derive(Debug, Clone, Copy)]
enum TerminationKind {
    Interrupt,
    Stop,
}

#[derive(Debug, Clone)]
struct TerminationRequest {
    kind: TerminationKind,
    reason: String,
    sent: bool,
}

#[derive(Debug)]
enum TerminalOutcome {
    Completed,
    Failed(String),
    Cancelled(String),
}

#[derive(Default)]
struct ParsedRun {
    thread_id: Option<String>,
    turn_id: Option<String>,
    final_message: String,
    usage: UsageSnapshot,
    last_usage_total: Option<u64>,
    streamed_command_items: HashSet<String>,
}

impl CodexAdapter {
    #[cfg(test)]
    pub fn new(command: PathBuf) -> Self {
        Self::with_prefix_args(command, Vec::new())
    }

    pub(crate) fn new_with_prefix(command: PathBuf, prefix_args: Vec<OsString>) -> Self {
        Self::with_prefix_args(command, prefix_args)
    }

    fn with_prefix_args(command: PathBuf, prefix_args: Vec<OsString>) -> Self {
        let available = std::process::Command::new(&command)
            .args(&prefix_args)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        Self {
            command,
            prefix_args: Arc::new(prefix_args),
            available,
            usage: Arc::new(DashMap::new()),
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.command);
        command.args(self.prefix_args.iter());
        command
    }

    async fn run_codex(
        &self,
        request: AdapterRunRequest,
        mut controls: mpsc::UnboundedReceiver<AdapterControl>,
        sink: Arc<dyn AdapterEventSink>,
        mode: CodexMode,
    ) -> Result<AdapterExit, AdapterError> {
        if !self.available {
            return Err(AdapterError::Unsupported {
                feature: "spawn",
                reason: format!(
                    "{} was not found or failed --version",
                    self.command.display()
                ),
            });
        }

        ensure_git_workspace(&request.workspace).await?;
        sink.emit(AdapterEvent::Started {
            workspace: request.workspace.clone(),
        });

        let mut command = self.command();
        command
            .arg("app-server")
            .arg("--listen")
            .arg("stdio://")
            // Do not expose a supervised run to user-configured MCP servers or apps. Managed
            // policies can still add mandatory controls, which Codex reports as notifications.
            .arg("-c")
            .arg("mcp_servers={}")
            .arg("-c")
            .arg("features.apps=false")
            .arg("-c")
            .arg("hooks={}")
            .envs(&request.environment)
            .current_dir(&request.workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .with_context(|| format!("spawn Codex app-server {}", self.command.display()))?;
        let mut stdin = child
            .stdin
            .take()
            .context("Codex app-server stdin missing")?;
        let stdout = child
            .stdout
            .take()
            .context("Codex app-server stdout missing")?;
        let stderr = child
            .stderr
            .take()
            .context("Codex app-server stderr missing")?;

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

        send_rpc(
            &mut stdin,
            json!({
                "method": "initialize",
                "id": INITIALIZE_REQUEST_ID,
                "params": {
                    "clientInfo": {
                        "name": "ecorp-operations",
                        "title": "ECorp Runner",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "capabilities": {
                        "experimentalApi": true
                    }
                }
            }),
        )
        .await?;

        let mut lines = BufReader::new(stdout).lines();
        let mut parsed = ParsedRun::default();
        if let CodexMode::Resume { session_id } = &mode {
            parsed.thread_id = Some(session_id.clone());
        }
        let mut pending_steers = VecDeque::<(uuid::Uuid, String)>::new();
        let mut controls_open = true;
        let mut next_request_id = 10_u64;
        let mut initialized = false;
        let mut thread_requested = false;
        let mut session_emitted = false;
        let mut termination: Option<TerminationRequest> = None;
        let startup_timeout = tokio::time::sleep(STARTUP_TIMEOUT);
        tokio::pin!(startup_timeout);
        let interrupt_timeout = tokio::time::sleep(Duration::from_secs(365 * 24 * 60 * 60));
        tokio::pin!(interrupt_timeout);
        let mut interrupt_timer_active = false;

        let outcome = loop {
            tokio::select! {
                control = controls.recv(), if controls_open => {
                    match control {
                        Some(AdapterControl::Steer { actor_id, text }) => {
                            if termination.is_some() {
                                sink.emit(AdapterEvent::Output {
                                    stream: "control".to_owned(),
                                    text: format!(
                                        "Ignored direction from {actor_id} because termination is already in progress"
                                    ),
                                });
                            } else if let (Some(thread_id), Some(turn_id)) =
                                (parsed.thread_id.as_deref(), parsed.turn_id.as_deref())
                            {
                                send_steer(
                                    &mut stdin,
                                    &mut next_request_id,
                                    thread_id,
                                    turn_id,
                                    actor_id,
                                    &text,
                                )
                                .await?;
                                sink.emit(AdapterEvent::Output {
                                    stream: "control".to_owned(),
                                    text: format!("Forwarded live direction from {actor_id}: {text}"),
                                });
                            } else {
                                pending_steers.push_back((actor_id, text));
                            }
                        }
                        Some(AdapterControl::Interrupt { reason }) => {
                            if termination.is_none() {
                                termination = Some(TerminationRequest {
                                    kind: TerminationKind::Interrupt,
                                    reason,
                                    sent: false,
                                });
                            }
                        }
                        Some(AdapterControl::Stop { reason }) => {
                            if termination.is_none() {
                                termination = Some(TerminationRequest {
                                    kind: TerminationKind::Stop,
                                    reason,
                                    sent: false,
                                });
                            }
                        }
                        Some(AdapterControl::ApprovalDecision {
                            approval_id,
                            approved,
                            note,
                        }) => {
                            let text = if approved {
                                format!(
                                    "Approval {approval_id} was granted. Continue the suspended action. Decision note: {note}"
                                )
                            } else {
                                format!(
                                    "Approval {approval_id} was rejected. Do not perform the action. Decision note: {note}"
                                )
                            };
                            pending_steers.push_back((Uuid::nil(), text));
                        }
                        Some(AdapterControl::CircuitBreaker { stage, reason }) => {
                            if matches!(stage.as_str(), "suspend" | "stop") {
                                if termination.is_none() {
                                    termination = Some(TerminationRequest {
                                        kind: TerminationKind::Stop,
                                        reason: format!(
                                            "Circuit breaker {stage} checkpoint: {reason}"
                                        ),
                                        sent: false,
                                    });
                                }
                            } else {
                                pending_steers.push_back((
                                    Uuid::nil(),
                                    format!("Circuit breaker stage {stage}: {reason}"),
                                ));
                            }
                        }
                        None => controls_open = false,
                    }

                    if let Some(requested) = termination.as_mut()
                        && !requested.sent
                    {
                        if let (Some(thread_id), Some(turn_id)) =
                            (parsed.thread_id.as_deref(), parsed.turn_id.as_deref())
                        {
                            send_interrupt(
                                &mut stdin,
                                &mut next_request_id,
                                thread_id,
                                turn_id,
                            )
                            .await?;
                            requested.sent = true;
                            interrupt_timeout
                                .as_mut()
                                .reset(Instant::now() + INTERRUPT_TIMEOUT);
                            interrupt_timer_active = true;
                        } else if !thread_requested {
                            let reason = terminal_reason(requested);
                            let _ = child.kill().await;
                            break TerminalOutcome::Cancelled(reason);
                        }
                    }
                }
                line = lines.next_line() => {
                    let Some(line) = line? else {
                        let status = child.wait().await?;
                        break match termination.as_ref() {
                            Some(requested) => TerminalOutcome::Cancelled(terminal_reason(requested)),
                            None => TerminalOutcome::Failed(format!(
                                "Codex app-server exited with {status} before a terminal turn event"
                            )),
                        };
                    };
                    let value: Value = match serde_json::from_str(&line) {
                        Ok(value) => value,
                        Err(_) => {
                            sink.emit(AdapterEvent::Output {
                                stream: "stdout".to_owned(),
                                text: line,
                            });
                            continue;
                        }
                    };

                    if value.get("method").is_some() && value.get("id").is_some() {
                        answer_server_request(&mut stdin, &value, sink.clone()).await?;
                        continue;
                    }

                    if let Some(id) = value.get("id").and_then(Value::as_u64) {
                        if let Some(error) = rpc_error(&value) {
                            if id <= TURN_REQUEST_ID {
                                break TerminalOutcome::Failed(error);
                            }
                            sink.emit(AdapterEvent::Output {
                                stream: "stderr".to_owned(),
                                text: error,
                            });
                            continue;
                        }
                        match id {
                            INITIALIZE_REQUEST_ID => {
                                initialized = true;
                                send_rpc(
                                    &mut stdin,
                                    json!({"method": "initialized", "params": {}}),
                                )
                                .await?;
                                let thread_method = match &mode {
                                    CodexMode::Start => "thread/start",
                                    CodexMode::Resume { .. } => "thread/resume",
                                };
                                let mut params = json!({
                                    "cwd": request.workspace,
                                    "approvalPolicy": "never",
                                    "sandbox": "workspace-write"
                                });
                                match &mode {
                                    CodexMode::Start => {
                                        params["ephemeral"] = Value::Bool(false);
                                        params["threadSource"] = Value::String("ecorp-operations".to_owned());
                                    }
                                    CodexMode::Resume { session_id } => {
                                        params["threadId"] = Value::String(session_id.clone());
                                    }
                                }
                                send_rpc(
                                    &mut stdin,
                                    json!({
                                        "method": thread_method,
                                        "id": THREAD_REQUEST_ID,
                                        "params": params
                                    }),
                                )
                                .await?;
                                thread_requested = true;
                            }
                            THREAD_REQUEST_ID => {
                                let thread_id = value
                                    .pointer("/result/thread/id")
                                    .and_then(Value::as_str)
                                    .context("Codex thread response omitted result.thread.id")?
                                    .to_owned();
                                parsed.thread_id = Some(thread_id.clone());
                                if !session_emitted {
                                    sink.emit(AdapterEvent::Session {
                                        session_id: thread_id.clone(),
                                    });
                                    session_emitted = true;
                                }
                                send_turn_start(
                                    &mut stdin,
                                    &request,
                                    &thread_id,
                                )
                                .await?;
                            }
                            TURN_REQUEST_ID => {
                                if let Some(turn_id) = value
                                    .pointer("/result/turn/id")
                                    .and_then(Value::as_str)
                                {
                                    parsed.turn_id = Some(turn_id.to_owned());
                                }
                            }
                            _ => {}
                        }
                    } else if let Some(method) = value.get("method").and_then(Value::as_str) {
                        if method == "thread/started"
                            && let Some(thread_id) = value
                                .pointer("/params/thread/id")
                                .and_then(Value::as_str)
                        {
                            parsed.thread_id = Some(thread_id.to_owned());
                            if !session_emitted {
                                sink.emit(AdapterEvent::Session {
                                    session_id: thread_id.to_owned(),
                                });
                                session_emitted = true;
                            }
                        }
                        if method == "turn/started"
                            && let Some(turn_id) = value
                                .pointer("/params/turn/id")
                                .and_then(Value::as_str)
                        {
                            parsed.turn_id = Some(turn_id.to_owned());
                        }

                        handle_notification(&value, &mut parsed, sink.clone());

                        if method == "turn/completed" {
                            let status = value
                                .pointer("/params/turn/status")
                                .and_then(Value::as_str)
                                .unwrap_or("failed");
                            break match status {
                                "completed" => TerminalOutcome::Completed,
                                "interrupted" => TerminalOutcome::Cancelled(
                                    termination
                                        .as_ref()
                                        .map(terminal_reason)
                                        .unwrap_or_else(|| "Codex interrupted the turn".to_owned()),
                                ),
                                _ => TerminalOutcome::Failed(
                                    value
                                        .pointer("/params/turn/error/message")
                                        .and_then(Value::as_str)
                                        .unwrap_or("Codex turn failed")
                                        .to_owned(),
                                ),
                            };
                        }
                    }

                    if parsed.turn_id.is_some() {
                        while let Some((actor_id, text)) = pending_steers.pop_front() {
                            if termination.is_some() {
                                break;
                            }
                            send_steer(
                                &mut stdin,
                                &mut next_request_id,
                                parsed.thread_id.as_deref().context("Codex thread id missing")?,
                                parsed.turn_id.as_deref().context("Codex turn id missing")?,
                                actor_id,
                                &text,
                            )
                            .await?;
                            sink.emit(AdapterEvent::Output {
                                stream: "control".to_owned(),
                                text: format!("Forwarded live direction from {actor_id}: {text}"),
                            });
                        }
                        if let Some(requested) = termination.as_mut()
                            && !requested.sent
                        {
                            send_interrupt(
                                &mut stdin,
                                &mut next_request_id,
                                parsed.thread_id.as_deref().context("Codex thread id missing")?,
                                parsed.turn_id.as_deref().context("Codex turn id missing")?,
                            )
                            .await?;
                            requested.sent = true;
                            interrupt_timeout
                                .as_mut()
                                .reset(Instant::now() + INTERRUPT_TIMEOUT);
                            interrupt_timer_active = true;
                        }
                    }
                }
                _ = &mut startup_timeout, if parsed.turn_id.is_none() => {
                    break TerminalOutcome::Failed(format!(
                        "Codex app-server did not start a turn within {} seconds \
                         (initialized={initialized}, thread_requested={thread_requested})",
                        STARTUP_TIMEOUT.as_secs()
                    ));
                }
                _ = &mut interrupt_timeout, if interrupt_timer_active => {
                    let reason = termination
                        .as_ref()
                        .map(terminal_reason)
                        .unwrap_or_else(|| "Codex termination timed out".to_owned());
                    let _ = child.kill().await;
                    break TerminalOutcome::Cancelled(format!(
                        "{reason}; Codex did not acknowledge interruption within {} seconds",
                        INTERRUPT_TIMEOUT.as_secs()
                    ));
                }
            }
        };

        drop(stdin);
        if tokio::time::timeout(SHUTDOWN_TIMEOUT, child.wait())
            .await
            .is_err()
        {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        stderr_task.abort();
        let _ = stderr_task.await;

        if let Some(thread_id) = parsed.thread_id.as_deref() {
            self.usage
                .entry(thread_id.to_owned())
                .and_modify(|usage| add_usage(usage, &parsed.usage))
                .or_insert_with(|| parsed.usage.clone());
        }
        let (status, summary) = match &outcome {
            TerminalOutcome::Completed => (
                "completed",
                if parsed.final_message.trim().is_empty() {
                    "Codex completed the repository task.".to_owned()
                } else {
                    parsed.final_message.trim().to_owned()
                },
            ),
            TerminalOutcome::Failed(error) => ("failed", error.clone()),
            TerminalOutcome::Cancelled(reason) => ("cancelled", reason.clone()),
        };
        let artifact = write_evidence(
            &request,
            parsed.thread_id.as_deref(),
            status,
            &summary,
            &parsed.usage,
        )
        .await?;
        sink.emit(AdapterEvent::Artifact(artifact));

        Ok(match outcome {
            TerminalOutcome::Completed => {
                sink.emit(AdapterEvent::Completed { summary });
                AdapterExit::Completed
            }
            TerminalOutcome::Failed(error) => {
                sink.emit(AdapterEvent::Failed { error });
                AdapterExit::Failed
            }
            TerminalOutcome::Cancelled(reason) => {
                sink.emit(AdapterEvent::Cancelled { reason });
                AdapterExit::Cancelled
            }
        })
    }
}

#[async_trait]
impl AgentAdapter for CodexAdapter {
    fn id(&self) -> &'static str {
        "codex"
    }

    fn display_name(&self) -> &'static str {
        "OpenAI Codex app-server"
    }

    fn capabilities(&self) -> AdapterCapabilities {
        let support = |feature: &str| {
            if self.available {
                FeatureSupport::Supported
            } else {
                FeatureSupport::Unsupported {
                    reason: format!(
                        "{feature} unavailable because {} is not installed",
                        self.command.display()
                    ),
                }
            }
        };
        AdapterCapabilities {
            spawn: support("spawn"),
            stream: support("stream"),
            steer: support("steer"),
            interrupt: support("interrupt"),
            stop: support("stop"),
            resume: support("resume"),
            usage: support("usage"),
            artifacts: support("artifacts"),
        }
    }

    async fn execute(
        &self,
        request: AdapterRunRequest,
        controls: mpsc::UnboundedReceiver<AdapterControl>,
        sink: Arc<dyn AdapterEventSink>,
    ) -> Result<AdapterExit, AdapterError> {
        self.run_codex(request, controls, sink, CodexMode::Start)
            .await
    }

    async fn resume(
        &self,
        request: AdapterRunRequest,
        session_id: &str,
        controls: mpsc::UnboundedReceiver<AdapterControl>,
        sink: Arc<dyn AdapterEventSink>,
    ) -> Result<AdapterExit, AdapterError> {
        self.run_codex(
            request,
            controls,
            sink,
            CodexMode::Resume {
                session_id: session_id.to_owned(),
            },
        )
        .await
    }

    async fn collect_usage(&self, session_id: &str) -> Result<UsageSnapshot, AdapterError> {
        self.usage
            .get(session_id)
            .map(|entry| entry.clone())
            .ok_or_else(|| {
                AdapterError::Runtime(anyhow!("no usage recorded for session {session_id}"))
            })
    }
}

async fn send_rpc(stdin: &mut ChildStdin, value: Value) -> Result<(), AdapterError> {
    let mut body = serde_json::to_vec(&value).context("serialize Codex app-server request")?;
    body.push(b'\n');
    stdin.write_all(&body).await?;
    stdin.flush().await?;
    Ok(())
}

async fn send_turn_start(
    stdin: &mut ChildStdin,
    request: &AdapterRunRequest,
    thread_id: &str,
) -> Result<(), AdapterError> {
    let prompt = format!(
        "You are executing a bounded task under ECorp supervision.\n\
         Work only inside the current repository. Do not modify files outside it.\n\
         Complete this mission and verify the resulting files:\n\n{}",
        request.mission_title
    );
    send_rpc(
        stdin,
        json!({
            "method": "turn/start",
            "id": TURN_REQUEST_ID,
            "params": {
                "threadId": thread_id,
                "input": [{"type": "text", "text": prompt}],
                "cwd": request.workspace,
                "approvalPolicy": "never",
                "sandboxPolicy": {
                    "type": "workspaceWrite",
                    "writableRoots": [request.workspace],
                    "networkAccess": false
                }
            }
        }),
    )
    .await
}

async fn send_steer(
    stdin: &mut ChildStdin,
    next_request_id: &mut u64,
    thread_id: &str,
    turn_id: &str,
    actor_id: uuid::Uuid,
    text: &str,
) -> Result<(), AdapterError> {
    let id = *next_request_id;
    *next_request_id = next_request_id.saturating_add(1);
    send_rpc(
        stdin,
        json!({
            "method": "turn/steer",
            "id": id,
            "params": {
                "threadId": thread_id,
                "expectedTurnId": turn_id,
                "input": [{
                    "type": "text",
                    "text": format!("Live direction from human actor {actor_id}: {text}")
                }]
            }
        }),
    )
    .await
}

async fn send_interrupt(
    stdin: &mut ChildStdin,
    next_request_id: &mut u64,
    thread_id: &str,
    turn_id: &str,
) -> Result<(), AdapterError> {
    let id = *next_request_id;
    *next_request_id = next_request_id.saturating_add(1);
    send_rpc(
        stdin,
        json!({
            "method": "turn/interrupt",
            "id": id,
            "params": {
                "threadId": thread_id,
                "turnId": turn_id
            }
        }),
    )
    .await
}

async fn answer_server_request(
    stdin: &mut ChildStdin,
    value: &Value,
    sink: Arc<dyn AdapterEventSink>,
) -> Result<(), AdapterError> {
    let id = value
        .get("id")
        .cloned()
        .context("Codex server request omitted id")?;
    let method = value
        .get("method")
        .and_then(Value::as_str)
        .context("Codex server request omitted method")?;
    let response = match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            json!({"id": id, "result": {"decision": "decline"}})
        }
        "item/tool/requestUserInput" => json!({"id": id, "result": {"answers": {}}}),
        _ => json!({
            "id": id,
            "error": {
                "code": -32000,
                "message": format!(
                    "ECorp does not permit interactive app-server request {method}"
                )
            }
        }),
    };
    sink.emit(AdapterEvent::Output {
        stream: "control".to_owned(),
        text: format!("Denied unsupervised Codex server request: {method}"),
    });
    send_rpc(stdin, response).await
}

fn handle_notification(value: &Value, parsed: &mut ParsedRun, sink: Arc<dyn AdapterEventSink>) {
    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return;
    };
    let params = value.get("params").unwrap_or(&Value::Null);
    match method {
        "turn/started" => sink.emit(AdapterEvent::Status {
            status: "working".to_owned(),
            station: "briefing".to_owned(),
            message: "Codex turn started".to_owned(),
        }),
        "item/started" => {
            if let Some(item) = params.get("item") {
                handle_started_item(item, sink);
            }
        }
        "item/completed" => {
            if let Some(item) = params.get("item") {
                handle_completed_item(item, parsed, sink);
            }
        }
        "item/commandExecution/outputDelta" => {
            if let Some(delta) = params.get("delta").and_then(Value::as_str) {
                if let Some(item_id) = params.get("itemId").and_then(Value::as_str) {
                    parsed.streamed_command_items.insert(item_id.to_owned());
                }
                sink.emit(AdapterEvent::Output {
                    stream: "terminal".to_owned(),
                    text: delta.to_owned(),
                });
            }
        }
        "item/agentMessage/delta" => {
            // App-server emits token-sized deltas. Persisting each one as a domain event creates
            // avoidable database pressure, so ECorp emits the authoritative completed message.
        }
        "thread/tokenUsage/updated" => {
            if let Some(usage) = record_usage(params, parsed) {
                sink.emit(AdapterEvent::Usage(usage));
            }
        }
        "warning" => {
            if let Some(message) = params.get("message").and_then(Value::as_str) {
                sink.emit(AdapterEvent::Output {
                    stream: "stderr".to_owned(),
                    text: message.to_owned(),
                });
            }
        }
        "error" => {
            let message = params
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| params.pointer("/error/message").and_then(Value::as_str))
                .unwrap_or("Codex app-server reported an error");
            sink.emit(AdapterEvent::Output {
                stream: "stderr".to_owned(),
                text: message.to_owned(),
            });
        }
        _ => {}
    }
}

fn handle_started_item(item: &Value, sink: Arc<dyn AdapterEventSink>) {
    match item.get("type").and_then(Value::as_str) {
        Some("commandExecution") => sink.emit(AdapterEvent::Status {
            status: "working".to_owned(),
            station: "terminal".to_owned(),
            message: item
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("Codex command")
                .to_owned(),
        }),
        Some("fileChange") => sink.emit(AdapterEvent::Status {
            status: "working".to_owned(),
            station: "files".to_owned(),
            message: "Codex is changing repository files".to_owned(),
        }),
        Some("agentMessage") => sink.emit(AdapterEvent::Status {
            status: "working".to_owned(),
            station: "reporting".to_owned(),
            message: "Codex is reporting results".to_owned(),
        }),
        _ => {}
    }
}

fn handle_completed_item(item: &Value, parsed: &mut ParsedRun, sink: Arc<dyn AdapterEventSink>) {
    let item_id = item.get("id").and_then(Value::as_str).unwrap_or_default();
    match item.get("type").and_then(Value::as_str) {
        Some("agentMessage") => {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                parsed.final_message = text.to_owned();
                sink.emit(AdapterEvent::Output {
                    stream: "agent".to_owned(),
                    text: text.to_owned(),
                });
            }
        }
        Some("commandExecution") => {
            if !parsed.streamed_command_items.contains(item_id)
                && let Some(output) = item.get("aggregatedOutput").and_then(Value::as_str)
                && !output.is_empty()
            {
                sink.emit(AdapterEvent::Output {
                    stream: "terminal".to_owned(),
                    text: output.to_owned(),
                });
            }
        }
        Some("fileChange") => {
            let changes = item
                .get("changes")
                .and_then(Value::as_array)
                .map(|changes| {
                    changes
                        .iter()
                        .filter_map(|change| {
                            let path = change.get("path")?.as_str()?;
                            let kind = change.get("kind")?.as_str()?;
                            Some(format!("{kind}: {path}"))
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            if !changes.is_empty() {
                sink.emit(AdapterEvent::Output {
                    stream: "files".to_owned(),
                    text: changes,
                });
            }
        }
        _ => {}
    }
}

fn record_usage(params: &Value, parsed: &mut ParsedRun) -> Option<UsageSnapshot> {
    if params.get("turnId").and_then(Value::as_str) != parsed.turn_id.as_deref() {
        return None;
    }
    let total_tokens = params
        .pointer("/tokenUsage/total/totalTokens")
        .and_then(Value::as_u64)?;
    if parsed.last_usage_total == Some(total_tokens) {
        return None;
    }
    // App-server reports `last` for the latest model API call and `total` cumulatively for the
    // thread. Use the monotonic total as the duplicate cursor, but account and emit `last` once.
    let last = params.pointer("/tokenUsage/last").unwrap_or(&Value::Null);
    let usage = UsageSnapshot {
        input_tokens: last.get("inputTokens").and_then(Value::as_u64).unwrap_or(0),
        output_tokens: last
            .get("outputTokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cost_microusd: 0,
    };
    add_usage(&mut parsed.usage, &usage);
    parsed.last_usage_total = Some(total_tokens);
    (usage.input_tokens > 0 || usage.output_tokens > 0 || usage.cost_microusd > 0).then_some(usage)
}

fn rpc_error(value: &Value) -> Option<String> {
    value.get("error").map(|error| {
        error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Codex app-server request failed")
            .to_owned()
    })
}

fn terminal_reason(request: &TerminationRequest) -> String {
    match request.kind {
        TerminationKind::Interrupt => format!("interrupted: {}", request.reason),
        TerminationKind::Stop => request.reason.clone(),
    }
}

fn add_usage(target: &mut UsageSnapshot, addition: &UsageSnapshot) {
    target.input_tokens = target.input_tokens.saturating_add(addition.input_tokens);
    target.output_tokens = target.output_tokens.saturating_add(addition.output_tokens);
    target.cost_microusd = target.cost_microusd.saturating_add(addition.cost_microusd);
}

async fn ensure_git_workspace(workspace: &Path) -> Result<(), AdapterError> {
    tokio::fs::create_dir_all(workspace)
        .await
        .with_context(|| format!("create Codex workspace {}", workspace.display()))?;
    if workspace.join(".git").exists() {
        return Ok(());
    }
    run_git(workspace, &["init", "-b", "main"]).await?;
    let readme = workspace.join("README.md");
    if !readme.exists() {
        tokio::fs::write(&readme, "# ECorp Codex workspace\n").await?;
    }
    run_git(workspace, &["add", "README.md"]).await?;
    run_git(
        workspace,
        &[
            "-c",
            "user.name=ECorp Runner",
            "-c",
            "user.email=crony@example.invalid",
            "commit",
            "-m",
            "chore: initialize Codex workspace",
        ],
    )
    .await?;
    Ok(())
}

async fn run_git(workspace: &Path, args: &[&str]) -> Result<(), AdapterError> {
    git_output(workspace, args).await.map(|_| ())
}

async fn git_output(workspace: &Path, args: &[&str]) -> Result<Vec<u8>, AdapterError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(AdapterError::Runtime(anyhow!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

async fn write_evidence(
    request: &AdapterRunRequest,
    session_id: Option<&str>,
    status: &str,
    summary: &str,
    usage: &UsageSnapshot,
) -> Result<AdapterArtifact, AdapterError> {
    let git_status = git_output(&request.workspace, &["status", "--porcelain=v1"]).await?;
    let git_head = git_output(&request.workspace, &["rev-parse", "HEAD"]).await?;
    let git_diff = git_output(
        &request.workspace,
        &["diff", "--binary", "--no-ext-diff", "HEAD", "--"],
    )
    .await?;
    let (changed_paths, files) = changed_file_fingerprints(&request.workspace).await?;
    let evidence_root = request
        .workspace
        .parent()
        .unwrap_or(&request.workspace)
        .join("evidence");
    tokio::fs::create_dir_all(&evidence_root).await?;
    let path = evidence_root.join(format!("codex-{}.json", request.run_id));
    let body = serde_json::to_vec_pretty(&json!({
        "adapter": "codex-app-server",
        "run_id": request.run_id,
        "mission_id": request.mission_id,
        "task_id": request.task_id,
        "agent_id": request.agent_id,
        "session_id": session_id,
        "status": status,
        "summary": summary,
        "git": {
            "head": String::from_utf8_lossy(&git_head).trim(),
            "status": String::from_utf8_lossy(&git_status),
            "changed_paths": changed_paths,
            "files": files,
            "diff_bytes": git_diff.len(),
            "diff_sha256": hex::encode(Sha256::digest(&git_diff))
        },
        "usage": {
            "input_tokens": usage.input_tokens,
            "output_tokens": usage.output_tokens,
            "cost_microusd": usage.cost_microusd
        }
    }))
    .context("serialize Codex evidence")?;
    tokio::fs::write(&path, &body).await?;
    Ok(AdapterArtifact {
        path,
        sha256: hex::encode(Sha256::digest(&body)),
        bytes: body.len(),
        media_type: "application/json".to_owned(),
    })
}

async fn changed_file_fingerprints(
    workspace: &Path,
) -> Result<(Vec<String>, Vec<Value>), AdapterError> {
    let tracked = git_output(workspace, &["diff", "--name-only", "-z", "HEAD", "--"]).await?;
    let untracked = git_output(
        workspace,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )
    .await?;
    let mut paths = BTreeSet::new();
    for raw in [tracked, untracked] {
        for path in raw.split(|byte| *byte == b'\0') {
            if !path.is_empty() {
                paths.insert(String::from_utf8_lossy(path).to_string());
            }
        }
    }

    let workspace_root = tokio::fs::canonicalize(workspace).await?;
    let mut files = Vec::with_capacity(paths.len());
    for relative in &paths {
        let relative_path = Path::new(relative);
        if relative_path.is_absolute()
            || !relative_path
                .components()
                .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
        {
            files.push(json!({
                "path": relative,
                "kind": "unsafe-path",
            }));
            continue;
        }

        let candidate = workspace.join(relative_path);
        let metadata = match tokio::fs::symlink_metadata(&candidate).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                files.push(json!({
                    "path": relative,
                    "kind": "missing",
                }));
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() {
            files.push(json!({
                "path": relative,
                "kind": "symlink",
            }));
            continue;
        }
        if metadata.is_dir() {
            files.push(json!({
                "path": relative,
                "kind": "directory",
            }));
            continue;
        }

        let canonical = tokio::fs::canonicalize(&candidate).await?;
        if !canonical.starts_with(&workspace_root) {
            files.push(json!({
                "path": relative,
                "kind": "outside-workspace",
            }));
            continue;
        }
        let (bytes, sha256) = hash_file(&canonical).await?;
        files.push(json!({
            "path": relative,
            "kind": "file",
            "bytes": bytes,
            "sha256": sha256,
        }));
    }
    Ok((paths.into_iter().collect(), files))
}

async fn hash_file(path: &Path) -> Result<(u64, String), AdapterError> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut digest = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        bytes = bytes.saturating_add(read as u64);
    }
    Ok((bytes, hex::encode(digest.finalize())))
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
            self.events.lock().expect("recording lock").push(event);
        }
    }

    fn test_adapter() -> CodexAdapter {
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/fake-codex-app-server.mjs");
        CodexAdapter::new_with_prefix(PathBuf::from("node"), vec![script.into_os_string()])
    }

    fn test_request(title: &str) -> AdapterRunRequest {
        let run_id = uuid::Uuid::new_v4();
        AdapterRunRequest {
            run_id,
            mission_id: uuid::Uuid::new_v4(),
            task_id: uuid::Uuid::new_v4(),
            agent_id: uuid::Uuid::new_v4(),
            mission_title: title.to_owned(),
            model: None,
            reasoning_effort: None,
            workspace: std::env::temp_dir()
                .join("crony-codex-adapter-tests")
                .join(run_id.to_string()),
            write_scope: vec!["**".to_owned()],
            environment: std::collections::HashMap::new(),
        }
    }

    fn session_id(sink: &RecordingSink) -> String {
        sink.events
            .lock()
            .expect("recording lock")
            .iter()
            .find_map(|event| match event {
                AdapterEvent::Session { session_id } => Some(session_id.clone()),
                _ => None,
            })
            .expect("session event")
    }

    async fn wait_for_session_id(sink: &RecordingSink) -> String {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(session_id) = sink
                    .events
                    .lock()
                    .expect("recording lock")
                    .iter()
                    .find_map(|event| match event {
                        AdapterEvent::Session { session_id } => Some(session_id.clone()),
                        _ => None,
                    })
                {
                    return session_id;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("session event timeout")
    }

    #[test]
    fn unavailable_codex_reports_explicit_capabilities() {
        let adapter = CodexAdapter::new(PathBuf::from("definitely-missing-codex-command"));
        let capabilities = adapter.capabilities();
        assert!(!capabilities.spawn.supported());
        assert!(capabilities.spawn.reason().is_some());
        assert!(!capabilities.resume.supported());
    }

    #[test]
    fn installed_codex_reports_full_capabilities() {
        let capabilities = test_adapter().capabilities();
        assert!(capabilities.spawn.supported());
        assert!(capabilities.stream.supported());
        assert!(capabilities.steer.supported());
        assert!(capabilities.interrupt.supported());
        assert!(capabilities.stop.supported());
        assert!(capabilities.resume.supported());
        assert!(capabilities.usage.supported());
        assert!(capabilities.artifacts.supported());
    }

    #[tokio::test]
    async fn executes_streams_usage_and_writes_evidence() {
        let adapter = test_adapter();
        let request = test_request("create a base file");
        let workspace = request.workspace.clone();
        let (_control_tx, control_rx) = mpsc::unbounded_channel();
        let sink = Arc::new(RecordingSink::default());
        let exit = adapter
            .execute(request, control_rx, sink.clone())
            .await
            .expect("execute Codex adapter");
        assert_eq!(exit, AdapterExit::Completed);
        assert_eq!(
            tokio::fs::read_to_string(workspace.join("base.txt"))
                .await
                .expect("base file"),
            "base\n"
        );
        let artifact_path = {
            let events = sink.events.lock().expect("recording lock");
            assert!(
                events
                    .iter()
                    .any(|event| matches!(event, AdapterEvent::Session { .. }))
            );
            assert!(
                events
                    .iter()
                    .any(|event| matches!(event, AdapterEvent::Status { .. }))
            );
            assert!(events.iter().any(
                |event| matches!(event, AdapterEvent::Output { stream, .. } if stream == "terminal")
            ));
            assert!(events.iter().any(
                |event| matches!(event, AdapterEvent::Usage(usage) if usage.input_tokens > 0)
            ));
            assert!(
                events
                    .iter()
                    .any(|event| matches!(event, AdapterEvent::Artifact(_)))
            );
            assert!(
                events
                    .iter()
                    .any(|event| matches!(event, AdapterEvent::Completed { .. }))
            );
            events
                .iter()
                .find_map(|event| match event {
                    AdapterEvent::Artifact(artifact) => Some(artifact.path.clone()),
                    _ => None,
                })
                .expect("evidence artifact")
        };
        let evidence: Value = serde_json::from_slice(
            &tokio::fs::read(&artifact_path)
                .await
                .expect("read evidence artifact"),
        )
        .expect("parse evidence artifact");
        assert!(
            evidence
                .pointer("/git/files")
                .and_then(Value::as_array)
                .is_some_and(|files| files.iter().any(|file| {
                    file.get("path").and_then(Value::as_str) == Some("base.txt")
                        && file.get("sha256").and_then(Value::as_str).is_some()
                }))
        );
        let usage = adapter
            .collect_usage(&session_id(&sink))
            .await
            .expect("collect usage");
        assert!(usage.input_tokens > 0);
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn forwards_live_steering_to_the_active_turn() {
        let adapter = test_adapter();
        let request = test_request("[slow] create a base file");
        let workspace = request.workspace.clone();
        let (control_tx, control_rx) = mpsc::unbounded_channel();
        let sink = Arc::new(RecordingSink::default());
        let task_adapter = adapter.clone();
        let task_sink = sink.clone();
        let task =
            tokio::spawn(async move { task_adapter.execute(request, control_rx, task_sink).await });
        tokio::time::sleep(Duration::from_millis(150)).await;
        control_tx
            .send(AdapterControl::Steer {
                actor_id: uuid::Uuid::new_v4(),
                text: "create the steered file".to_owned(),
            })
            .expect("send steer");
        let exit = task.await.expect("join adapter").expect("execute adapter");
        assert_eq!(exit, AdapterExit::Completed);
        assert_eq!(
            tokio::fs::read_to_string(workspace.join("steered.txt"))
                .await
                .expect("steered file"),
            "create the steered file\n"
        );
        assert!(sink.events.lock().expect("recording lock").iter().any(
            |event| matches!(event, AdapterEvent::Output { stream, text }
                if stream == "control" && text.contains("Forwarded live direction"))
        ));
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn interrupts_then_resumes_the_same_session_and_workspace() {
        let adapter = test_adapter();
        let request = test_request("[slow] create a base file");
        let resume_request = request.clone();
        let workspace = request.workspace.clone();
        let (control_tx, control_rx) = mpsc::unbounded_channel();
        let sink = Arc::new(RecordingSink::default());
        let task_adapter = adapter.clone();
        let task_sink = sink.clone();
        let task =
            tokio::spawn(async move { task_adapter.execute(request, control_rx, task_sink).await });
        let provider_session_id = wait_for_session_id(&sink).await;
        control_tx
            .send(AdapterControl::Interrupt {
                reason: "pause for human review".to_owned(),
            })
            .expect("send interrupt");
        let exit = task.await.expect("join adapter").expect("execute adapter");
        assert_eq!(exit, AdapterExit::Cancelled);

        let (_resume_tx, resume_rx) = mpsc::unbounded_channel();
        let resume_sink = Arc::new(RecordingSink::default());
        let resumed = adapter
            .resume(
                AdapterRunRequest {
                    mission_title: "resume the prior task".to_owned(),
                    ..resume_request
                },
                &provider_session_id,
                resume_rx,
                resume_sink.clone(),
            )
            .await
            .expect("resume adapter");
        assert_eq!(resumed, AdapterExit::Completed);
        assert_eq!(
            tokio::fs::read_to_string(workspace.join("resumed.txt"))
                .await
                .expect("resumed file"),
            "resumed\n"
        );
        assert!(
            resume_sink
                .events
                .lock()
                .expect("recording lock")
                .iter()
                .any(|event| matches!(event, AdapterEvent::Session { session_id }
                if session_id == &provider_session_id))
        );
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn stop_is_graceful_and_evidenced() {
        let adapter = test_adapter();
        let request = test_request("[slow] create a base file");
        let workspace = request.workspace.clone();
        let (control_tx, control_rx) = mpsc::unbounded_channel();
        let sink = Arc::new(RecordingSink::default());
        let task_adapter = adapter.clone();
        let task_sink = sink.clone();
        let task =
            tokio::spawn(async move { task_adapter.execute(request, control_rx, task_sink).await });
        tokio::time::sleep(Duration::from_millis(150)).await;
        control_tx
            .send(AdapterControl::Stop {
                reason: "emergency stop".to_owned(),
            })
            .expect("send stop");
        let exit = task.await.expect("join adapter").expect("execute adapter");
        assert_eq!(exit, AdapterExit::Cancelled);
        let events = sink.events.lock().expect("recording lock");
        assert!(events.iter().any(
            |event| matches!(event, AdapterEvent::Cancelled { reason } if reason == "emergency stop")
        ));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, AdapterEvent::Artifact(_)))
        );
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn failed_turn_returns_failed_exit_with_evidence() {
        let adapter = test_adapter();
        let request = test_request("[fail] reject this turn");
        let workspace = request.workspace.clone();
        let (_control_tx, control_rx) = mpsc::unbounded_channel();
        let sink = Arc::new(RecordingSink::default());
        let exit = adapter
            .execute(request, control_rx, sink.clone())
            .await
            .expect("execute adapter");
        assert_eq!(exit, AdapterExit::Failed);
        let events = sink.events.lock().expect("recording lock");
        assert!(
            events
                .iter()
                .any(|event| matches!(event, AdapterEvent::Failed { error } if error.contains("synthetic failure")))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, AdapterEvent::Artifact(_)))
        );
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn usage_notifications_are_deduplicated_and_scoped_to_the_active_turn() {
        let mut parsed = ParsedRun {
            turn_id: Some("turn-1".to_owned()),
            ..ParsedRun::default()
        };
        let notification = json!({
            "turnId": "turn-1",
            "tokenUsage": {
                "total": {"totalTokens": 12},
                "last": {"inputTokens": 10, "outputTokens": 2}
            }
        });
        let first = record_usage(&notification, &mut parsed).expect("first usage");
        assert_eq!(first.input_tokens, 10);
        assert_eq!(first.output_tokens, 2);
        assert!(record_usage(&notification, &mut parsed).is_none());
        let second = record_usage(
            &json!({
                "turnId": "turn-1",
                "tokenUsage": {
                    "total": {"totalTokens": 20},
                    "last": {"inputTokens": 6, "outputTokens": 2}
                }
            }),
            &mut parsed,
        )
        .expect("second usage");
        assert_eq!(second.input_tokens, 6);
        assert_eq!(second.output_tokens, 2);
        assert!(
            record_usage(
                &json!({
                    "turnId": "other",
                    "tokenUsage": {
                        "total": {"totalTokens": 20},
                        "last": {"inputTokens": 5, "outputTokens": 3}
                    }
                }),
                &mut parsed,
            )
            .is_none()
        );
        assert_eq!(parsed.usage.input_tokens, 16);
        assert_eq!(parsed.usage.output_tokens, 4);
    }
}
