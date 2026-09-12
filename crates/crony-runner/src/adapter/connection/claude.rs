use std::{
    path::Path,
    sync::{Arc, atomic::Ordering},
};

use chrono::{DateTime, Utc};
use crony_domain::{
    CodingAgent, NativeSignInInputKind, NativeSignInInstruction, WorkspaceConnectionStatus,
};
use serde_json::{Value, json};
use tokio::time::Instant;
use uuid::Uuid;

use super::{
    AgentReadiness, NativeCodeReceiver, ProbeError, ProbeResult,
    catalog::{self, AccountState, LoginTranscript},
    environment::{NativeCommand, ProfileEnvironment},
    process::{OutputLine, SetupProcess},
};

async fn account(
    native: &NativeCommand,
    scope: &ProfileEnvironment,
    workspace: &Path,
    deadline: Instant,
) -> ProbeResult<AccountState> {
    let mut command = native.build(scope, workspace)?;
    command.args(["auth", "status", "--json"]);
    let mut process = SetupProcess::spawn(command).await?;
    let result = async {
        let mut stdout = String::new();
        while let Some(line) = process.next(deadline).await? {
            if let OutputLine::Stdout(line) = line {
                if stdout.len() + line.len() > 64 * 1024 {
                    return Err(ProbeError::Incompatible(
                        "Claude account response exceeded its bound.",
                    ));
                }
                stdout.push_str(&line);
                stdout.push('\n');
            }
        }
        let successful_exit = process.wait_success(deadline).await?;
        let status: Value = serde_json::from_str(&stdout).map_err(|_| {
            ProbeError::Incompatible("Claude auth status did not return native JSON.")
        })?;
        catalog::claude_account(&status, successful_exit)
    }
    .await;
    process.finish(result).await
}

pub(super) async fn inspect(
    native: &NativeCommand,
    scope: &ProfileEnvironment,
    workspace: &Path,
    deadline: Instant,
) -> ProbeResult<AgentReadiness> {
    let login = match account(native, scope, workspace, deadline).await? {
        AccountState::SignedOut => {
            return Ok(AgentReadiness::state(
                WorkspaceConnectionStatus::NeedsSignIn,
                "Claude Code needs native sign-in for this connection's account scope.",
            ));
        }
        AccountState::SignedIn(login) => login,
    };
    if Instant::now() >= deadline {
        return Err(ProbeError::Timeout);
    }
    let mut command = native.build(scope, workspace)?;
    crate::adapter::external::append_claude_control_args(&mut command);
    let mut process = SetupProcess::spawn(command).await?;
    let result = async {
        let request_id = format!("ecorp-connection-{}", Uuid::new_v4());
        process
            .send_before(
                &json!({
                    "type":"control_request",
                    "request_id":request_id,
                    "request":{"subtype":"initialize","hooks":null}
                }),
                deadline,
            )
            .await?;
        let initialization = loop {
            let frame = process.next_json(deadline).await?;
            if frame.get("type").and_then(Value::as_str) == Some("control_response") {
                break initialization_response(&frame, &request_id)?;
            }
            if frame.get("type").and_then(Value::as_str) != Some("system") {
                return Err(ProbeError::Incompatible(
                    "Unexpected Claude frame before initialization.",
                ));
            }
        };
        let models = catalog::claude_models(&initialization)?;
        let account_hint =
            login.or_else(|| catalog::email_hint(initialization.pointer("/account/email")));
        // Deliberately no user frame after initialize: catalog/account only.
        AgentReadiness::ready(models, account_hint)
    }
    .await;
    process.finish(result).await
}

pub(super) fn initialization_response(frame: &Value, request_id: &str) -> ProbeResult<Value> {
    if frame.get("type").and_then(Value::as_str) != Some("control_response")
        || frame.pointer("/response/subtype").and_then(Value::as_str) != Some("success")
        || frame
            .pointer("/response/request_id")
            .and_then(Value::as_str)
            != Some(request_id)
    {
        return Err(ProbeError::Incompatible(
            "Claude initialization did not match its native request.",
        ));
    }
    frame
        .pointer("/response/response")
        .filter(|payload| payload.is_object())
        .cloned()
        .ok_or(ProbeError::Incompatible(
            "Claude omitted native initialization metadata.",
        ))
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn sign_in(
    native: &NativeCommand,
    scope: &ProfileEnvironment,
    workspace: &Path,
    deadline: Instant,
    expires_at: DateTime<Utc>,
    progress: &Arc<dyn Fn(NativeSignInInstruction) + Send + Sync>,
    input: &mut NativeCodeReceiver,
) -> ProbeResult<bool> {
    let mut command = native.build(scope, workspace)?;
    command.args(["auth", "login"]);
    let mut process = SetupProcess::spawn(command).await?;
    let result = login_dialogue(&mut process, deadline, expires_at, progress, input).await;
    input.clear_prompt();
    process.finish(result).await
}

async fn login_dialogue(
    process: &mut SetupProcess,
    deadline: Instant,
    expires_at: DateTime<Utc>,
    progress: &Arc<dyn Fn(NativeSignInInstruction) + Send + Sync>,
    input: &mut NativeCodeReceiver,
) -> ProbeResult<bool> {
    let mut transcript = LoginTranscript::default();
    let mut uri: Option<String> = None;
    let mut input_requested = false;
    let mut code_sent = false;
    let mut native_prompt_id = None;
    loop {
        let output = tokio::select! {
            _ = tokio::time::sleep_until(deadline) => return Err(ProbeError::Timeout),
            response = input.receiver.recv(), if input_requested && !code_sent => {
                input.clear_prompt();
                let response = response.ok_or(ProbeError::Failed("Native sign-in input was cancelled."))?;
                if Some(response.input_id) != native_prompt_id {
                    return Err(ProbeError::Failed("Native authorization-code response did not match the pending prompt."));
                }
                if Instant::now() >= deadline {
                    return Err(ProbeError::Timeout);
                }
                process.send_authorization_code(response.authorization_code, deadline).await?;
                code_sent = true;
                if let Some(uri) = &uri {
                    progress(catalog::sign_in_instruction(
                        CodingAgent::ClaudeCode, uri, None, expires_at, None,
                    )?);
                }
                continue;
            }
            output = process.next_login(deadline) => output?,
        };
        match output {
            Some(OutputLine::Stdout(chunk) | OutputLine::Stderr(chunk)) => {
                transcript.push(&chunk)?;
                if uri.is_none()
                    && let Some(native_uri) = transcript.uri(CodingAgent::ClaudeCode)
                {
                    progress(catalog::sign_in_instruction(
                        CodingAgent::ClaudeCode,
                        &native_uri,
                        None,
                        expires_at,
                        None,
                    )?);
                    uri = Some(native_uri);
                }
                if !input_requested
                    && transcript.claude_code_prompt()
                    && let Some(uri) = &uri
                {
                    // A code queued before the native prompt is not authorized
                    // input. The manager also fences actor, operation and expiry.
                    if input.receiver.try_recv().is_ok() {
                        return Err(ProbeError::Failed(
                            "Unexpected native input before the authorization-code prompt.",
                        ));
                    }
                    input_requested = true;
                    let id = Uuid::new_v4();
                    native_prompt_id = Some(id);
                    *input
                        .prompt_id
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) = Some(id);
                    input.waiting.store(true, Ordering::Release);
                    let mut instruction = catalog::sign_in_instruction(
                        CodingAgent::ClaudeCode,
                        uri,
                        None,
                        expires_at,
                        Some(NativeSignInInputKind::AuthorizationCode),
                    )?;
                    instruction.input_id = Some(id);
                    progress(instruction);
                }
            }
            None => return process.wait_success(deadline).await,
        }
    }
}
