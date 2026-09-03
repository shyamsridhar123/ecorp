use std::{
    collections::{HashMap, VecDeque},
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, anyhow};
use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, Command},
    sync::mpsc,
};
use uuid::Uuid;

use super::{
    AdapterArtifact, AdapterCapabilities, AdapterControl, AdapterError, AdapterEvent,
    AdapterEventSink, AdapterExit, AdapterRunRequest, AgentAdapter, FeatureSupport, UsageSnapshot,
    permission::{bounded_text, path_is_inside_workspace},
    process_tree::OwnedProcessTree,
};

const MAX_PROTOCOL_LINE_BYTES: usize = 64 * 1024;
const APPROVAL_EXPIRY_SECONDS: u64 = 600;
const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(5);
const PROTOCOL_WRITE_TIMEOUT: Duration = Duration::from_millis(500);
const TERMINAL_PERMISSION_TIMEOUT: Duration = Duration::from_secs(1);
const STDIN_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(500);
const ROOT_POLL_INTERVAL: Duration = Duration::from_millis(25);
const STREAM_DRAIN_TIMEOUT: Duration = Duration::from_secs(1);
const TEARDOWN_RETRY_INTERVAL: Duration = Duration::from_millis(250);
const AVAILABILITY_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const AVAILABILITY_ROOT_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq)]
struct ClaudePermissionRequest {
    request_id: String,
    tool_use_id: String,
    tool_name: String,
    input: Value,
    blocked_path: Option<String>,
    decision_reason: Option<String>,
    title: Option<String>,
    display_name: Option<String>,
    description: Option<String>,
}

#[derive(Debug)]
struct PendingClaudePermission {
    request: ClaudePermissionRequest,
}

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
    platform_supported: bool,
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
        Self {
            flavor,
            command,
            prefix_args,
            platform_supported: cfg!(windows),
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.command);
        command.args(&self.prefix_args);
        command
    }

    fn append_run_args(
        &self,
        command: &mut Command,
        mission_title: &str,
        resume_session_id: Option<&str>,
    ) {
        match self.flavor {
            ExternalFlavor::ClaudeCode => {
                command
                    .arg("--safe-mode")
                    .arg("--no-chrome")
                    .arg("--disable-slash-commands")
                    .arg("--strict-mcp-config")
                    .arg("--mcp-config")
                    .arg(r#"{"mcpServers":{}}"#)
                    .arg("--print")
                    .arg("--verbose")
                    .arg("--input-format")
                    .arg("stream-json")
                    .arg("--output-format")
                    .arg("stream-json")
                    .arg("--permission-mode")
                    .arg("manual")
                    .arg("--permission-prompt-tool")
                    .arg("stdio");
                if let Some(session_id) = resume_session_id {
                    command.arg(format!("--resume={session_id}"));
                }
            }

            async fn probe_available(&self) -> Result<(), AdapterError> {
                if !self.platform_supported {
                    return Err(AdapterError::Runtime(anyhow!(unix_unsupported_reason())));
                }
                let mut command = self.command();
                command
                    .arg("--version")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                let tree = OwnedProcessTree::spawn(&mut command)
                    .with_context(|| format!("spawn {} availability probe", self.display_name()))?;
                let mut guardian = tokio::spawn(async move {
                    let mut tree = tree;
                    let root_deadline = tokio::time::sleep(AVAILABILITY_ROOT_TIMEOUT);
                    tokio::pin!(root_deadline);
                    let mut root_tick = tokio::time::interval(ROOT_POLL_INTERVAL);
                    root_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                    loop {
                        tokio::select! {
                            _ = &mut root_deadline => break,
                            _ = root_tick.tick() => {
                                if tree.try_wait_root()?.is_some() {
                                    break;
                                }
                            }
                        }
                    }
                    loop {
                        match tree.terminate_and_wait().await {
                            Ok(status) => return Ok::<_, std::io::Error>(status),
                            Err(_) => tokio::time::sleep(TEARDOWN_RETRY_INTERVAL).await,
                        }
                    }
                });
                match tokio::time::timeout(AVAILABILITY_PROBE_TIMEOUT, &mut guardian).await {
                    Ok(Ok(Ok(status))) if status.success() => Ok(()),
                    Ok(Ok(Ok(status))) => Err(AdapterError::Runtime(anyhow!(
                        "{} availability probe exited with {status}",
                        self.display_name()
                    ))),
                    Ok(Ok(Err(error))) => Err(AdapterError::Runtime(anyhow!(
                        "{} availability probe failed: {error}",
                        self.display_name()
                    ))),
                    Ok(Err(error)) => Err(AdapterError::Runtime(anyhow!(
                        "{} availability guardian failed: {error}",
                        self.display_name()
                    ))),
                    Err(_) => Err(AdapterError::Runtime(anyhow!(
                        "{} availability probe exceeded its deadline; owned cleanup continues",
                        self.display_name()
                    ))),
                }
            }
            ExternalFlavor::OpenCode => {
                command.arg("run").arg("--pure").arg("--format").arg("json");
                if let Some(session_id) = resume_session_id {
                    command.arg("--session").arg(session_id);
                }
                command.arg(mission_title);
            }
        }
    }

    async fn run(
        &self,
        request: AdapterRunRequest,
        resume_session_id: Option<&str>,
        mut controls: mpsc::UnboundedReceiver<AdapterControl>,
        sink: Arc<dyn AdapterEventSink>,
    ) -> Result<AdapterExit, AdapterError> {
        if !self.platform_supported {
            return Err(AdapterError::Runtime(anyhow!(
                "{}",
                unix_unsupported_reason()
            )));
        }
        tokio::fs::create_dir_all(&request.workspace).await?;
        sink.emit(AdapterEvent::Started {
            workspace: request.workspace.clone(),
        });

        let mut command = self.command();
        self.append_run_args(&mut command, &request.mission_title, resume_session_id);
        command
            .current_dir(&request.workspace)
            .envs(&request.environment)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut tree = OwnedProcessTree::spawn(&mut command)
            .with_context(|| format!("spawn {}", self.display_name()))?;
        #[cfg(test)]
        if request
            .mission_title
            .contains("[teardown:verification-failure]")
        {
            tree.force_verification_failures(1);
        }
        let streams = {
            let child = tree.child_mut();
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        };
        let (Some(mut stdin), Some(stdout), Some(stderr)) = streams else {
            teardown_until_verified(&mut tree, &mut controls, &sink).await;
            sink.emit(AdapterEvent::Failed {
                error: "provider standard stream missing".to_owned(),
            });
            return Ok(AdapterExit::Failed);
        };
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
        let mut lines = BufReader::new(stdout).lines();
        let mut session_id = resume_session_id.map(str::to_owned);
        let mut cancelled = None;
        let mut failed = None;
        let mut pending = HashMap::<Uuid, PendingClaudePermission>::new();
        let mut pending_by_request = HashMap::<String, Uuid>::new();
        let mut deferred_controls = VecDeque::new();
        if matches!(self.flavor, ExternalFlavor::ClaudeCode) {
            let initialization = initialize_claude(
                &mut stdin,
                &mut lines,
                &mut raw_output,
                &request.mission_title,
            );
            tokio::pin!(initialization);
            let initialize_deadline = tokio::time::sleep(INITIALIZE_TIMEOUT);
            tokio::pin!(initialize_deadline);
            let mut root_tick = tokio::time::interval(ROOT_POLL_INTERVAL);
            root_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    result = &mut initialization => {
                        if let Err(error) = result {
                            failed = Some(format!("initialize Claude stream protocol: {error}"));
                        }
                        break;
                    }
                    _ = &mut initialize_deadline => {
                        failed = Some(format!(
                            "Claude initialization exceeded the {:?} deadline",
                            INITIALIZE_TIMEOUT
                        ));
                        break;
                    }
                    _ = root_tick.tick() => {
                        match tree.try_wait_root() {
                            Ok(Some(status)) => {
                                failed = Some(format!(
                                    "{} exited during initialization with {status}",
                                    self.display_name()
                                ));
                                break;
                            }
                            Ok(None) => {}
                            Err(error) => {
                                failed = Some(format!("observe provider root during initialization: {error}"));
                                break;
                            }
                        }
                    }
                    control = controls.recv() => match control {
                        Some(AdapterControl::Stop { reason })
                        | Some(AdapterControl::Interrupt { reason }) => {
                            cancelled = Some(reason);
                            break;
                        }
                        Some(AdapterControl::CircuitBreaker { stage, reason })
                            if matches!(stage.as_str(), "suspend" | "stop") =>
                        {
                            cancelled = Some(format!(
                                "Circuit breaker {stage} checkpoint: {reason}"
                            ));
                            break;
                        }
                        Some(control) => deferred_controls.push_back(control),
                        None => {}
                    }
                }
            }
        }

        if failed.is_none() && cancelled.is_none() {
        let mut root_tick = tokio::time::interval(ROOT_POLL_INTERVAL);
        root_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        'provider: loop {
            tokio::select! {
                control = async {
                    if let Some(control) = deferred_controls.pop_front() {
                        Some(control)
                    } else {
                        controls.recv().await
                    }
                } => match control {
                    Some(AdapterControl::Stop { reason })
                    | Some(AdapterControl::Interrupt { reason }) => {
                        if let Err(error) = deny_pending_permissions(
                            &mut stdin,
                            &mut pending,
                            &mut pending_by_request,
                            &reason,
                        ).await {
                            failed = Some(format!(
                                "failed to deliver Claude permission denial: {error}"
                            ));
                        }
                        if failed.is_none() {
                            cancelled = Some(reason);
                        }
                        break;
                    }
                    Some(AdapterControl::Steer { actor_id, text }) => {
                        let frame = if matches!(self.flavor, ExternalFlavor::ClaudeCode) {
                            json!({
                                "type": "user",
                                "message": {
                                    "role": "user",
                                    "content": format!("Direction from {actor_id}: {text}"),
                                },
                            })
                        } else {
                            json!({
                                "type": "user",
                                "actor_id": actor_id,
                                "text": text,
                            })
                        };
                        let _ = send_protocol_frame(&mut stdin, &frame).await;
                    }
                    Some(AdapterControl::ApprovalDecision { approval_id, approved, note }) => {
                        if let Some(pending_request) = pending.remove(&approval_id) {
                            pending_by_request.remove(&pending_request.request.request_id);
                            if let Err(error) = send_claude_permission_response(
                                &mut stdin,
                                &pending_request.request,
                                approved,
                                &note,
                            ).await {
                                failed = Some(format!(
                                    "failed to deliver Claude permission response: {error}"
                                ));
                                break;
                            }
                        }
                    }
                    Some(AdapterControl::CircuitBreaker { stage, reason }) => {
                        if matches!(stage.as_str(), "suspend" | "stop") {
                            if let Err(error) = deny_pending_permissions(
                                &mut stdin,
                                &mut pending,
                                &mut pending_by_request,
                                &reason,
                            ).await {
                                failed = Some(format!(
                                    "failed to deliver Claude permission denial: {error}"
                                ));
                            }
                            if failed.is_none() {
                                cancelled = Some(format!(
                                    "Circuit breaker {stage} checkpoint: {reason}"
                                ));
                            }
                            break;
                        }
                        let frame = if matches!(self.flavor, ExternalFlavor::ClaudeCode) {
                            json!({
                                "type": "user",
                                "message": {
                                    "role": "user",
                                    "content": format!("ECorp circuit breaker {stage}: {reason}"),
                                },
                            })
                        } else {
                            json!({
                                "type": "policy",
                                "stage": stage,
                                "reason": reason,
                            })
                        };
                        if let Err(error) = send_protocol_frame(&mut stdin, &frame).await {
                            failed = Some(format!(
                                "failed to deliver Claude circuit-breaker frame: {error}"
                            ));
                            break;
                        }
                    }
                    None => {}
                },
                _ = root_tick.tick() => {
                    match tree.try_wait_root() {
                        Ok(Some(status)) => {
                            if !status.success() {
                                failed = Some(format!(
                                    "{} root exited with {status}",
                                    self.display_name()
                                ));
                            }
                            break 'provider;
                        }
                        Ok(None) => {}
                        Err(error) => {
                            failed = Some(format!("observe provider root process: {error}"));
                            break 'provider;
                        }
                    }
                }
                line = lines.next_line() => {
                    let line = match line {
                        Ok(Some(line)) => line,
                        Ok(None) => break,
                        Err(error) => {
                            failed = Some(format!("read provider protocol stream: {error}"));
                            break;
                        }
                    };
                    if line.len() > MAX_PROTOCOL_LINE_BYTES {
                        failed = Some("provider emitted an oversized stream-JSON frame".to_owned());
                        if let Err(error) = deny_pending_permissions(
                            &mut stdin,
                            &mut pending,
                            &mut pending_by_request,
                            "Provider protocol frame exceeded the ECorp bound",
                        ).await {
                            failed = Some(format!(
                                "failed to deliver Claude permission denial: {error}"
                            ));
                        }
                        break;
                    }
                    raw_output.extend_from_slice(line.as_bytes());
                    raw_output.push(b'\n');
                    if let Ok(value) = serde_json::from_str::<Value>(&line) {
                        if matches!(self.flavor, ExternalFlavor::ClaudeCode)
                            && value.get("type").and_then(Value::as_str)
                                == Some("control_cancel_request")
                        {
                            let Some(provider_request_id) =
                                value.get("request_id").and_then(Value::as_str)
                            else {
                                failed = Some(
                                    "Claude emitted a malformed control cancellation".to_owned(),
                                );
                                if let Err(error) = deny_pending_permissions(
                                    &mut stdin,
                                    &mut pending,
                                    &mut pending_by_request,
                                    "Malformed Claude control cancellation",
                                ).await {
                                    failed = Some(format!(
                                        "failed to deliver Claude permission denial: {error}"
                                    ));
                                }
                                break;
                            };
                            let Some(approval_id) =
                                pending_by_request.remove(provider_request_id)
                            else {
                                failed = Some(
                                    "Claude cancelled an unknown control request".to_owned(),
                                );
                                if let Err(error) = deny_pending_permissions(
                                    &mut stdin,
                                    &mut pending,
                                    &mut pending_by_request,
                                    "Claude cancelled an unknown control request",
                                ).await {
                                    failed = Some(format!(
                                        "failed to deliver Claude permission denial: {error}"
                                    ));
                                }
                                break;
                            };
                            if pending.remove(&approval_id).is_none() {
                                failed = Some(
                                    "Claude cancellation did not match a pending approval".to_owned(),
                                );
                                break;
                            }
                            continue;
                        }
                        if matches!(self.flavor, ExternalFlavor::ClaudeCode)
                            && value.get("type").and_then(Value::as_str)
                                == Some("control_request")
                        {
                            match parse_claude_permission_request(&value) {
                                Ok(provider_request) => {
                                    if let Some(existing_id) =
                                        pending_by_request.get(&provider_request.request_id)
                                    {
                                        let duplicate_matches = pending
                                            .get(existing_id)
                                            .is_some_and(|existing| {
                                                existing.request == provider_request
                                            });
                                        if duplicate_matches {
                                            continue;
                                        }
                                        failed = Some(
                                            "Claude reused a pending control request ID".to_owned(),
                                        );
                                        if let Err(error) = deny_pending_permissions(
                                            &mut stdin,
                                            &mut pending,
                                            &mut pending_by_request,
                                            "Claude reused a pending control request ID",
                                        ).await {
                                            failed = Some(format!(
                                                "failed to deliver Claude permission denial: {error}"
                                            ));
                                        }
                                        break;
                                    }
                                    if claude_permission_is_automatically_safe(
                                        &provider_request,
                                        &request.workspace,
                                    ) {
                                        if let Err(error) = send_claude_permission_response(
                                            &mut stdin,
                                            &provider_request,
                                            true,
                                            "",
                                        ).await {
                                            failed = Some(format!(
                                                "failed to deliver Claude permission response: {error}"
                                            ));
                                            break;
                                        }
                                        continue;
                                    }
                                    let action =
                                        match claude_permission_action(&provider_request) {
                                            Ok(action) => action,
                                            Err(error) => {
                                                failed = Some(error.to_string());
                                                if let Err(error) = deny_pending_permissions(
                                                    &mut stdin,
                                                    &mut pending,
                                                    &mut pending_by_request,
                                                    "Claude permission context exceeded the durable approval bound",
                                                )
                                                .await {
                                                    failed = Some(format!(
                                                        "failed to deliver Claude permission denial: {error}"
                                                    ));
                                                }
                                                break;
                                            }
                                        };
                                    let approval_id = Uuid::new_v4();
                                    sink.emit(AdapterEvent::ApprovalRequested {
                                        approval_id,
                                        action_key: format!(
                                            "claude-code:{}",
                                            provider_request.request_id
                                        ),
                                        action,
                                        risk: claude_permission_risk(&provider_request).to_owned(),
                                        rationale: bounded_text(
                                            provider_request
                                                .decision_reason
                                                .as_deref()
                                                .or(provider_request.description.as_deref())
                                                .unwrap_or(
                                                    "Claude Code requested a tool capability outside the automatically allowed worktree-local read/write boundary.",
                                                ),
                                            2_000,
                                        ),
                                        required_roles: vec![
                                            "owner".to_owned(),
                                            "admin".to_owned(),
                                            "manager".to_owned(),
                                            "member".to_owned(),
                                        ],
                                        expires_in_seconds: APPROVAL_EXPIRY_SECONDS,
                                    });
                                    pending_by_request.insert(
                                        provider_request.request_id.clone(),
                                        approval_id,
                                    );
                                    pending.insert(
                                        approval_id,
                                        PendingClaudePermission {
                                            request: provider_request,
                                        },
                                    );
                                }
                                Err(error) => {
                                    failed = Some(format!(
                                        "Claude permission protocol rejected: {error}"
                                    ));
                                    if let Err(delivery_error) = deny_pending_permissions(
                                        &mut stdin,
                                        &mut pending,
                                        &mut pending_by_request,
                                        "Malformed or unsupported Claude permission request",
                                    ).await {
                                        failed = Some(format!(
                                            "failed to deliver Claude permission denial: {delivery_error}"
                                        ));
                                    }
                                    break;
                                }
                            }
                            continue;
                        }
                        if matches!(self.flavor, ExternalFlavor::ClaudeCode)
                            && value.get("type").and_then(Value::as_str) == Some("result")
                        {
                            if !pending.is_empty() {
                                failed = Some(
                                    "Claude returned a result while a tool permission was still pending"
                                        .to_owned(),
                                );
                                if let Err(error) = deny_pending_permissions(
                                    &mut stdin,
                                    &mut pending,
                                    &mut pending_by_request,
                                    "Claude ended the turn before its permission request was decided",
                                )
                                .await
                                {
                                    failed = Some(format!(
                                        "failed to deliver Claude permission denial: {error}"
                                    ));
                                }
                                break;
                            }
                            if let Err(error) = shutdown_stdin(&mut stdin).await {
                                failed = Some(format!(
                                    "close Claude stream input after result: {error}"
                                ));
                                break;
                            }
                            break;
                        }
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
                        if matches!(self.flavor, ExternalFlavor::ClaudeCode)
                            && line.contains("control_request")
                        {
                            failed = Some(
                                "Claude emitted malformed stream-JSON control data".to_owned(),
                            );
                            if let Err(error) = deny_pending_permissions(
                                &mut stdin,
                                &mut pending,
                                &mut pending_by_request,
                                "Malformed Claude stream-JSON control data",
                            ).await {
                                failed = Some(format!(
                                    "failed to deliver Claude permission denial: {error}"
                                ));
                            }
                            break;
                        }
                        sink.emit(AdapterEvent::Output {
                            stream: "provider".to_owned(),
                            text: line,
                        });
                    }
                }
            }
            }
        }
        if let Err(error) = deny_pending_permissions(
            &mut stdin,
            &mut pending,
            &mut pending_by_request,
            "Claude provider session ended before an approval decision",
        )
        .await
        {
            failed.get_or_insert_with(|| {
                format!("failed to deliver Claude permission denial: {error}")
            });
        }
        if let Err(error) = shutdown_stdin(&mut stdin).await {
            failed.get_or_insert_with(|| format!("close provider protocol input: {error}"));
        }
        drop(stdin);
        let status = teardown_until_verified(&mut tree, &mut controls, &sink).await;
        drain_stream_task(stderr_task).await;
        if let Some(reason) = cancelled {
            sink.emit(AdapterEvent::Cancelled { reason });
            return Ok(AdapterExit::Cancelled);
        }
        if let Some(error) = failed {
            sink.emit(AdapterEvent::Failed { error });
            return Ok(AdapterExit::Failed);
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
        let available = if self.platform_supported {
            FeatureSupport::Supported
        } else {
            FeatureSupport::Unsupported {
                reason: unix_unsupported_reason().to_owned(),
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

    async fn list_models(&self) -> Result<Vec<super::AdapterModel>, AdapterError> {
        self.probe_available().await?;
        Ok(Vec::new())
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

async fn send_protocol_frame(stdin: &mut ChildStdin, frame: &Value) -> std::io::Result<()> {
    let encoded = serde_json::to_vec(frame).map_err(std::io::Error::other)?;
    tokio::time::timeout(PROTOCOL_WRITE_TIMEOUT, async {
        stdin.write_all(&encoded).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await
    })
    .await
    .map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "provider protocol write exceeded its deadline",
        )
    })?
}

async fn initialize_claude(
    stdin: &mut ChildStdin,
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    raw_output: &mut Vec<u8>,
    mission_title: &str,
) -> Result<(), AdapterError> {
    let request_id = format!("ecorp-initialize-{}", Uuid::new_v4());
    send_protocol_frame(
        stdin,
        &json!({
            "type": "control_request",
            "request_id": request_id,
            "request": {
                "subtype": "initialize",
                "hooks": null,
            },
        }),
    )
    .await
    .context("send Claude initialize request")?;
    let line = lines
        .next_line()
        .await?
        .ok_or_else(|| AdapterError::Runtime(anyhow!("Claude omitted initialize response")))?;
    if line.len() > MAX_PROTOCOL_LINE_BYTES {
        return Err(AdapterError::Runtime(anyhow!(
            "Claude initialize response exceeded the protocol bound"
        )));
    }
    raw_output.extend_from_slice(line.as_bytes());
    raw_output.push(b'\n');
    let frame: Value = serde_json::from_str(&line)
        .context("Claude initialize response was not valid stream JSON")?;
    let response = frame
        .get("response")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            AdapterError::Runtime(anyhow!("Claude initialize response was malformed"))
        })?;
    if frame.get("type").and_then(Value::as_str) != Some("control_response")
        || response.get("subtype").and_then(Value::as_str) != Some("success")
        || response.get("request_id").and_then(Value::as_str) != Some(request_id.as_str())
        || !response
            .get("response")
            .is_some_and(|value| value.is_object())
    {
        return Err(AdapterError::Runtime(anyhow!(
            "Claude initialize response did not match the request"
        )));
    }
    send_protocol_frame(
        stdin,
        &json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": mission_title,
            },
        }),
    )
    .await
    .context("send Claude mission frame")?;
    Ok(())
}

async fn send_claude_permission_response(
    stdin: &mut ChildStdin,
    request: &ClaudePermissionRequest,
    approved: bool,
    note: &str,
) -> std::io::Result<()> {
    let decision = if approved {
        json!({
            "behavior": "allow",
            "updatedInput": request.input,
        })
    } else {
        json!({
            "behavior": "deny",
            "message": bounded_text(note, 500),
        })
    };
    send_protocol_frame(
        stdin,
        &json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": request.request_id,
                "response": decision,
            },
        }),
    )
    .await
}

async fn deny_pending_permissions(
    stdin: &mut ChildStdin,
    pending: &mut HashMap<Uuid, PendingClaudePermission>,
    pending_by_request: &mut HashMap<String, Uuid>,
    reason: &str,
) -> std::io::Result<()> {
    let pending_requests = pending
        .drain()
        .map(|(_, pending)| pending.request)
        .collect::<Vec<_>>();
    pending_by_request.clear();
    tokio::time::timeout(TERMINAL_PERMISSION_TIMEOUT, async {
        let mut first_error = None;
        for request in pending_requests {
            if let Err(error) =
                send_claude_permission_response(stdin, &request, false, reason).await
            {
                first_error.get_or_insert(error);
            }
        }
        if let Some(error) = first_error {
            Err(error)
        } else {
            Ok(())
        }
    })
    .await
    .map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "terminal permission denial exceeded its deadline",
        )
    })?
}

async fn shutdown_stdin(stdin: &mut ChildStdin) -> std::io::Result<()> {
    tokio::time::timeout(STDIN_SHUTDOWN_TIMEOUT, stdin.shutdown())
        .await
        .map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "provider stdin shutdown exceeded its deadline",
            )
        })?
}

async fn teardown_until_verified(
    tree: &mut OwnedProcessTree,
    controls: &mut mpsc::UnboundedReceiver<AdapterControl>,
    sink: &Arc<dyn AdapterEventSink>,
) -> std::process::ExitStatus {
    let mut uncertainty_reported = false;
    let mut controls_open = true;
    loop {
        match tree.terminate_and_wait().await {
            Ok(status) => return status,
            Err(error) => {
                if !uncertainty_reported {
                    uncertainty_reported = true;
                    sink.emit(AdapterEvent::TeardownUncertain {
                        detail: format!(
                            "provider process-scope termination is unverified; cleanup supervision retained: {error}"
                        ),
                    });
                }
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(TEARDOWN_RETRY_INTERVAL) => {}
            control = controls.recv(), if controls_open => {
                if control.is_none() {
                    controls_open = false;
                }
            }
        }
    }
}

async fn drain_stream_task(mut task: tokio::task::JoinHandle<()>) {
    if tokio::time::timeout(STREAM_DRAIN_TIMEOUT, &mut task)
        .await
        .is_err()
    {
        task.abort();
        let _ = task.await;
    }
}

fn unix_unsupported_reason() -> &'static str {
    "external CLI providers require a non-escapable process boundary; Unix setsid/setpgid process groups cannot prove complete descendant termination"
}

fn parse_claude_permission_request(frame: &Value) -> Result<ClaudePermissionRequest, &'static str> {
    let request_id = required_bounded_string(frame, "request_id", 64)?;
    let request = frame
        .get("request")
        .and_then(Value::as_object)
        .ok_or("control request omitted its request object")?;
    if request.get("subtype").and_then(Value::as_str) != Some("can_use_tool") {
        return Err("unsupported control request subtype");
    }
    let tool_use_id = required_bounded_string(&Value::Object(request.clone()), "tool_use_id", 128)?;
    let tool_name = required_bounded_string(&Value::Object(request.clone()), "tool_name", 128)?;
    let input = request
        .get("input")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or("can_use_tool request omitted structured input")?;

    let parsed = ClaudePermissionRequest {
        request_id,
        tool_use_id,
        tool_name,
        input,
        blocked_path: optional_string(request, frame, &["blocked_path", "blockedPath"], 1_000)?,
        decision_reason: optional_string(
            request,
            frame,
            &["decision_reason", "decisionReason"],
            2_000,
        )?,
        title: optional_string(request, frame, &["title"], 500)?,
        display_name: optional_string(request, frame, &["display_name", "displayName"], 500)?,
        description: optional_string(request, frame, &["description"], 2_000)?,
    };
    if parsed
        .blocked_path
        .as_deref()
        .is_some_and(|value| value.is_empty())
    {
        return Err("blocked path was empty");
    }
    Ok(parsed)
}

fn required_bounded_string(
    value: &Value,
    field: &str,
    limit: usize,
) -> Result<String, &'static str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| {
            !value.is_empty()
                && value.chars().count() <= limit
                && !value.chars().any(char::is_control)
        })
        .map(str::to_owned)
        .ok_or("required protocol identifier was missing or invalid")
}

fn optional_string(
    request: &serde_json::Map<String, Value>,
    frame: &Value,
    fields: &[&str],
    limit: usize,
) -> Result<Option<String>, &'static str> {
    let value = fields
        .iter()
        .find_map(|field| request.get(*field).or_else(|| frame.get(*field)));
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .filter(|value| value.chars().count() <= limit)
        .map(str::to_owned)
        .map(Some)
        .ok_or("permission metadata was not a bounded string")
}

fn claude_permission_is_automatically_safe(
    request: &ClaudePermissionRequest,
    workspace: &Path,
) -> bool {
    if request.blocked_path.is_some() {
        return false;
    }
    let path_field = match request.tool_name.as_str() {
        "Read" | "Write" | "Edit" | "MultiEdit" => "file_path",
        "NotebookEdit" => "notebook_path",
        "Glob" | "Grep" => "path",
        _ => return false,
    };
    request
        .input
        .get(path_field)
        .and_then(Value::as_str)
        .is_some_and(|candidate| path_is_inside_workspace(workspace, candidate))
}

fn claude_permission_action(request: &ClaudePermissionRequest) -> Result<String, AdapterError> {
    let encoded_input = serde_json::to_vec(&request.input)
        .context("serialize Claude permission input for durable summary")?;
    let input_fields = request
        .input
        .as_object()
        .into_iter()
        .flat_map(|input| input.iter())
        .take(16)
        .map(|(name, value)| {
            let encoded = serde_json::to_vec(value).unwrap_or_default();
            json!({
                "name": bounded_text(name, 64),
                "type": json_value_kind(value),
                "sha256": hex::encode(Sha256::digest(&encoded)),
                "bytes": encoded.len(),
            })
        })
        .collect::<Vec<_>>();
    let input_field_count = request.input.as_object().map_or(0, |input| input.len());
    let path = claude_permission_path(request).map(|value| bounded_text(value, 500));
    let action = json!({
        "provider": "claude-code",
        "request_id": request.request_id,
        "tool_use_id": request.tool_use_id,
        "tool_name": request.tool_name,
        "path": path,
        "blocked_path": request.blocked_path.as_deref().map(|value| bounded_text(value, 500)),
        "input_summary": {
            "sha256": hex::encode(Sha256::digest(&encoded_input)),
            "bytes": encoded_input.len(),
            "fields": input_fields,
            "field_count": input_field_count,
            "fields_truncated": input_field_count > 16,
        },
        "decision_reason": request.decision_reason,
        "title": request.title,
        "display_name": request.display_name,
        "description": request.description,
    })
    .to_string();
    if action.is_empty() || action.len() > 2_000 {
        return Err(AdapterError::Runtime(anyhow!(
            "Claude permission context exceeded the durable approval bound"
        )));
    }
    Ok(action)
}

fn claude_permission_path(request: &ClaudePermissionRequest) -> Option<&str> {
    let path_field = match request.tool_name.as_str() {
        "Read" | "Write" | "Edit" | "MultiEdit" => "file_path",
        "NotebookEdit" => "notebook_path",
        "Glob" | "Grep" => "path",
        _ => return None,
    };
    request.input.get(path_field).and_then(Value::as_str)
}

fn json_value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn claude_permission_risk(request: &ClaudePermissionRequest) -> &'static str {
    if request.blocked_path.is_some() {
        "critical"
    } else {
        match request.tool_name.as_str() {
            "Bash" | "WebFetch" | "WebSearch" => "high",
            "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => "medium",
            "Read" | "Glob" | "Grep" => "low",
            _ => "high",
        }
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
    use std::{sync::Mutex, time::Duration};

    use serde::Deserialize;

    use super::*;
    use crate::adapter::process_tree::process_is_alive;

    #[derive(Default)]
    struct RecordingSink {
        events: Mutex<Vec<AdapterEvent>>,
    }

    impl AdapterEventSink for RecordingSink {
        fn emit(&self, event: AdapterEvent) {
            self.events.lock().expect("event lock").push(event);
        }
    }

    #[derive(Debug, Deserialize)]
    struct FixturePids {
        parent: u32,
        grandchild: u32,
    }

    #[derive(Clone, Copy)]
    enum TreeControl {
        Interrupt,
        Stop,
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

    #[test]
    fn claude_runs_without_user_plugins_mcp_or_browser_integrations() {
        let adapter = adapter(ExternalFlavor::ClaudeCode);
        let mut command = adapter.command();
        adapter.append_run_args(&mut command, "bounded mission", None);
        let args = command
            .as_std()
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        for required in [
            "--safe-mode",
            "--no-chrome",
            "--disable-slash-commands",
            "--strict-mcp-config",
            "--mcp-config",
            r#"{"mcpServers":{}}"#,
            "--input-format",
            "--output-format",
            "--permission-mode",
            "manual",
            "--permission-prompt-tool",
            "stdio",
        ] {
            assert!(
                args.iter().any(|argument| argument == required),
                "Claude command omitted isolation argument {required}: {args:?}"
            );
        }
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--input-format", "stream-json"])
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--output-format", "stream-json"])
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--permission-prompt-tool", "stdio"])
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--permission-mode", "manual"])
        );
        assert!(
            !args.iter().any(|argument| {
                matches!(
                    argument.as_str(),
                    "--dangerously-skip-permissions" | "bypassPermissions" | "acceptEdits"
                )
            }),
            "Claude command must not bypass provider permission checks: {args:?}"
        );
        assert!(!args.iter().any(|argument| argument == "bounded mission"));
    }

    #[test]
    fn claude_resume_session_id_cannot_be_parsed_as_a_flag() {
        let adapter = adapter(ExternalFlavor::ClaudeCode);
        let mut command = adapter.command();
        adapter.append_run_args(&mut command, "bounded mission", Some("-dash-leading"));
        let args = command
            .as_std()
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(
            args.iter()
                .any(|argument| argument == "--resume=-dash-leading")
        );
        assert!(!args.iter().any(|argument| argument == "-dash-leading"));
    }

    #[test]
    fn opencode_runs_without_external_plugins() {
        let adapter = adapter(ExternalFlavor::OpenCode);
        let mut command = adapter.command();
        adapter.append_run_args(&mut command, "bounded mission", None);
        let args = command
            .as_std()
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert!(
            args.iter().any(|argument| argument == "--pure"),
            "OpenCode command must disable external plugins: {args:?}"
        );
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

    async fn wait_for_tree_pids(workspace: &Path) -> FixturePids {
        let pid_file = workspace.join("external-tree-pids.json");
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(bytes) = tokio::fs::read(&pid_file).await
                    && let Ok(pids) = serde_json::from_slice(&bytes)
                {
                    return pids;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("external tree PID file timeout")
    }

    async fn wait_until_dead(pid: u32) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while process_is_alive(pid) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("external fixture process remained alive");
    }

    async fn assert_control_removes_tree(
        mission_title: &str,
        control: TreeControl,
        descendant_exits_first: bool,
    ) {
        let adapter = adapter(ExternalFlavor::ClaudeCode);
        let mut request = request(adapter.id());
        request.mission_title = mission_title.to_owned();
        let workspace = request.workspace.clone();
        let (control_tx, control_rx) = mpsc::unbounded_channel();
        let sink = Arc::new(RecordingSink::default());
        let task_adapter = adapter.clone();
        let task_sink = sink.clone();
        let task =
            tokio::spawn(async move { task_adapter.execute(request, control_rx, task_sink).await });
        let pids = wait_for_tree_pids(&workspace).await;

        assert!(process_is_alive(pids.parent));
        if descendant_exits_first {
            wait_until_dead(pids.grandchild).await;
        } else {
            assert!(process_is_alive(pids.grandchild));
        }

        let reason = match control {
            TreeControl::Interrupt => "process-tree interrupt",
            TreeControl::Stop => "process-tree stop",
        };
        let command = match control {
            TreeControl::Interrupt => AdapterControl::Interrupt {
                reason: reason.to_owned(),
            },
            TreeControl::Stop => AdapterControl::Stop {
                reason: reason.to_owned(),
            },
        };
        control_tx.send(command).expect("send tree control");
        let exit = tokio::time::timeout(Duration::from_secs(8), task)
            .await
            .expect("external adapter termination timeout")
            .expect("join external adapter")
            .expect("execute external adapter");

        assert_eq!(exit, AdapterExit::Cancelled);
        wait_until_dead(pids.parent).await;
        wait_until_dead(pids.grandchild).await;
        assert!(sink.events.lock().expect("event lock").iter().any(
            |event| matches!(event, AdapterEvent::Cancelled { reason: actual } if actual == reason)
        ));
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn external_interrupt_removes_stubborn_process_tree() {
        assert_control_removes_tree(
            "[process-tree:stubborn] interrupt fixture",
            TreeControl::Interrupt,
            false,
        )
        .await;
    }

    #[tokio::test]
    async fn external_stop_removes_process_tree_after_grandchild_already_exited() {
        assert_control_removes_tree(
            "[process-tree:exited-descendant] stop fixture",
            TreeControl::Stop,
            true,
        )
        .await;
    }

    async fn wait_for_approval(sink: &RecordingSink) -> (Uuid, String, String) {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(approval) =
                    sink.events
                        .lock()
                        .expect("event lock")
                        .iter()
                        .find_map(|event| match event {
                            AdapterEvent::ApprovalRequested {
                                approval_id,
                                action_key,
                                action,
                                ..
                            } => Some((*approval_id, action_key.clone(), action.clone())),
                            _ => None,
                        })
                {
                    return approval;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("approval request timeout")
    }

    #[test]
    fn claude_permission_frame_preserves_provider_context() {
        let frame = json!({
            "type": "control_request",
            "request_id": "request-7",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "Bash",
                "input": {"command": "cargo test", "timeout": 120000},
                "tool_use_id": "tool-9",
                "blocked_path": "C:\\blocked",
                "decision_reason": "policy",
                "title": "Run tests",
                "display_name": "Shell",
                "description": "Execute a command"
            }
        });
        let parsed = parse_claude_permission_request(&frame).expect("parse request");
        assert_eq!(parsed.request_id, "request-7");
        assert_eq!(parsed.tool_use_id, "tool-9");
        assert_eq!(parsed.tool_name, "Bash");
        assert_eq!(
            parsed.input,
            json!({"command": "cargo test", "timeout": 120000})
        );
        assert_eq!(parsed.blocked_path.as_deref(), Some("C:\\blocked"));
        assert_eq!(parsed.decision_reason.as_deref(), Some("policy"));
        assert_eq!(parsed.title.as_deref(), Some("Run tests"));
        assert_eq!(parsed.display_name.as_deref(), Some("Shell"));
        assert_eq!(parsed.description.as_deref(), Some("Execute a command"));
        assert!(
            parse_claude_permission_request(&json!({
                "type": "control_request",
                "request_id": "request-8",
                "request": {"subtype": "unknown"}
            }))
            .is_err()
        );
    }

    #[tokio::test]
    async fn claude_permission_bridge_suspends_and_applies_one_matching_approval() {
        let adapter = adapter(ExternalFlavor::ClaudeCode);
        let mut request = request(adapter.id());
        request.mission_title = "[permission:approve] governed tool".to_owned();
        let workspace = request.workspace.clone();
        let (control_tx, control_rx) = mpsc::unbounded_channel();
        let sink = Arc::new(RecordingSink::default());
        let task_adapter = adapter.clone();
        let task_sink = sink.clone();
        let task =
            tokio::spawn(async move { task_adapter.execute(request, control_rx, task_sink).await });

        let (approval_id, action_key, action) = wait_for_approval(&sink).await;
        assert_eq!(action_key, "claude-code:claude-request-001");
        let context: Value = serde_json::from_str(&action).expect("action context");
        assert_eq!(context["tool_name"], "Bash");
        assert_eq!(context["tool_use_id"], "claude-tool-use-001");
        assert_eq!(
            context["title"],
            Value::String("Protocol-faithful fake request".to_owned())
        );
        assert!(context.get("input").is_none());
        assert!(context["input_summary"]["sha256"].is_string());
        assert!(context["input_summary"]["fields"].is_array());
        assert!(!action.contains("sk-secret-must-not-be-durable"));
        assert!(!action.contains("ECORP_SECRET_TOKEN"));
        control_tx
            .send(AdapterControl::ApprovalDecision {
                approval_id: Uuid::new_v4(),
                approved: true,
                note: "unknown approval must be ignored".to_owned(),
            })
            .expect("send unknown decision");
        control_tx
            .send(AdapterControl::ApprovalDecision {
                approval_id,
                approved: true,
                note: "approved once".to_owned(),
            })
            .expect("send approval");
        control_tx
            .send(AdapterControl::ApprovalDecision {
                approval_id,
                approved: true,
                note: "duplicate must be ignored".to_owned(),
            })
            .expect("send duplicate");

        let exit = task.await.expect("join adapter").expect("execute adapter");
        assert_eq!(exit, AdapterExit::Completed);
        assert_eq!(
            std::fs::read_to_string(workspace.join("claude-permission-artifact.txt"))
                .expect("approved artifact"),
            "durably approved exactly once\n"
        );
        assert_eq!(
            sink.events
                .lock()
                .expect("event lock")
                .iter()
                .filter(|event| matches!(event, AdapterEvent::ApprovalRequested { .. }))
                .count(),
            1
        );
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn claude_permission_bridge_denies_rejection_and_expiry() {
        for note in ["operator rejected", "ECorp approval expired"] {
            let adapter = adapter(ExternalFlavor::ClaudeCode);
            let mut request = request(adapter.id());
            request.mission_title = "[permission:reject] governed tool".to_owned();
            let workspace = request.workspace.clone();
            let (control_tx, control_rx) = mpsc::unbounded_channel();
            let sink = Arc::new(RecordingSink::default());
            let task_adapter = adapter.clone();
            let task_sink = sink.clone();
            let task =
                tokio::spawn(
                    async move { task_adapter.execute(request, control_rx, task_sink).await },
                );
            let (approval_id, _, _) = wait_for_approval(&sink).await;
            control_tx
                .send(AdapterControl::ApprovalDecision {
                    approval_id,
                    approved: false,
                    note: note.to_owned(),
                })
                .expect("send denial");
            let exit = task.await.expect("join adapter").expect("execute adapter");
            assert_eq!(exit, AdapterExit::Completed);
            assert!(!workspace.join("claude-permission-artifact.txt").exists());
            let _ = std::fs::remove_dir_all(workspace);
        }
    }

    #[tokio::test]
    async fn claude_permission_bridge_auto_allows_only_contained_worktree_path() {
        let adapter = adapter(ExternalFlavor::ClaudeCode);
        let mut request = request(adapter.id());
        request.mission_title = "[permission:safe] local write".to_owned();
        let workspace = request.workspace.clone();
        let (_control_tx, control_rx) = mpsc::unbounded_channel();
        let sink = Arc::new(RecordingSink::default());
        let exit = adapter
            .execute(request, control_rx, sink.clone())
            .await
            .expect("execute adapter");
        assert_eq!(exit, AdapterExit::Completed);
        assert_eq!(
            std::fs::read_to_string(workspace.join("claude-permission-artifact.txt"))
                .expect("safe artifact"),
            "safe worktree write\n"
        );
        assert!(
            !sink
                .events
                .lock()
                .expect("event lock")
                .iter()
                .any(|event| matches!(event, AdapterEvent::ApprovalRequested { .. }))
        );
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn claude_permission_response_write_failure_fails_provider() {
        let adapter = adapter(ExternalFlavor::ClaudeCode);
        let mut request = request(adapter.id());
        request.mission_title = "[permission:response-write-failure] governed tool".to_owned();
        let workspace = request.workspace.clone();
        let (_control_tx, control_rx) = mpsc::unbounded_channel();
        let sink = Arc::new(RecordingSink::default());
        let exit = adapter
            .execute(request, control_rx, sink.clone())
            .await
            .expect("execute adapter");
        assert_eq!(exit, AdapterExit::Failed);
        assert!(sink.events.lock().expect("event lock").iter().any(|event| {
            matches!(
                event,
                AdapterEvent::Failed { error }
                    if error.contains("failed to deliver Claude permission denial")
            )
        }));
        assert!(!workspace.join("claude-permission-artifact.txt").exists());
        let _ = std::fs::remove_dir_all(workspace);
    }
}
