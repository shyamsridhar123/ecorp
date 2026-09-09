use std::{collections::HashSet, path::Path, sync::Arc};

use chrono::{DateTime, Utc};
use crony_domain::{CodingAgent, NativeSignInInstruction, WorkspaceConnectionStatus};
use serde_json::{Value, json};
use tokio::time::Instant;

use super::{
    AgentReadiness, ProbeError, ProbeResult,
    catalog::{self, AccountState},
    environment::{NativeCommand, ProfileEnvironment},
    process::SetupProcess,
};

async fn spawn(
    native: &NativeCommand,
    scope: &ProfileEnvironment,
    workspace: &Path,
) -> ProbeResult<SetupProcess> {
    let mut command = native.build(scope, workspace)?;
    crate::adapter::codex::append_app_server_args(&mut command, Some(scope));
    SetupProcess::spawn(command).await
}

async fn initialize(process: &mut SetupProcess, deadline: Instant) -> ProbeResult<()> {
    process
        .rpc(
            "initialize",
            1,
            json!({
                "clientInfo": {
                    "name": "ecorp-connections",
                    "title": "ECorp Native Connections",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {"experimentalApi":true}
            }),
            deadline,
        )
        .await?;
    process
        .send_before(&json!({"method":"initialized"}), deadline)
        .await
}

pub(super) async fn inspect(
    native: &NativeCommand,
    scope: &ProfileEnvironment,
    workspace: &Path,
    deadline: Instant,
) -> ProbeResult<AgentReadiness> {
    let mut process = spawn(native, scope, workspace).await?;
    let result = async {
        initialize(&mut process, deadline).await?;
        let account = process
            .rpc("account/read", 2, json!({"refreshToken":false}), deadline)
            .await?;
        let login = match catalog::codex_account(&account)? {
            AccountState::SignedOut => {
                return Ok(AgentReadiness::state(
                    WorkspaceConnectionStatus::NeedsSignIn,
                    "Codex needs native sign-in for this connection's account scope.",
                ));
            }
            AccountState::SignedIn(login) => login,
        };
        let mut models = Vec::new();
        let mut cursor: Option<String> = None;
        let mut cursors = HashSet::new();
        for page in 0..16_u64 {
            let result = process
                .rpc(
                    "model/list",
                    10 + page,
                    json!({"limit":100,"cursor":cursor,"includeHidden":false}),
                    deadline,
                )
                .await?;
            models.extend(catalog::codex_models(&result)?);
            catalog::validate_models(&models)?;
            cursor = match result.get("nextCursor") {
                None | Some(Value::Null) => {
                    return AgentReadiness::ready(models, login);
                }
                Some(Value::String(cursor)) if !cursor.is_empty() && cursor.len() <= 1024 => {
                    if !cursors.insert(cursor.clone()) {
                        return Err(ProbeError::Incompatible(
                            "Codex repeated its model catalog cursor.",
                        ));
                    }
                    Some(cursor.clone())
                }
                _ => {
                    return Err(ProbeError::Incompatible(
                        "Codex returned an invalid model cursor.",
                    ));
                }
            };
        }
        Err(ProbeError::Incompatible(
            "Codex model pagination exceeded its bound.",
        ))
    }
    .await;
    process.finish(result).await
}

pub(super) async fn sign_in(
    native: &NativeCommand,
    scope: &ProfileEnvironment,
    workspace: &Path,
    deadline: Instant,
    expires_at: DateTime<Utc>,
    progress: &Arc<dyn Fn(NativeSignInInstruction) + Send + Sync>,
) -> ProbeResult<bool> {
    let mut process = spawn(native, scope, workspace).await?;
    let mut login_id = None;
    let result = async {
        initialize(&mut process, deadline).await?;
        let account = process
            .rpc("account/read", 2, json!({"refreshToken":false}), deadline)
            .await?;
        if matches!(catalog::codex_account(&account)?, AccountState::SignedIn(_)) {
            return Ok(true);
        }
        let login = process
            .rpc(
                "account/login/start",
                3,
                json!({"type":"chatgptDeviceCode"}),
                deadline,
            )
            .await?;
        let id = login
            .get("loginId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 256)
            .ok_or(ProbeError::Incompatible(
                "Codex omitted its native login correlation.",
            ))?;
        login_id = Some(id.to_owned());
        let uri = login.get("verificationUrl").and_then(Value::as_str).ok_or(
            ProbeError::Incompatible("Codex omitted its native verification URL."),
        )?;
        let code =
            login
                .get("userCode")
                .and_then(Value::as_str)
                .ok_or(ProbeError::Incompatible(
                    "Codex omitted its native device code.",
                ))?;
        progress(catalog::sign_in_instruction(
            CodingAgent::Codex,
            uri,
            Some(code),
            expires_at,
            None,
        )?);
        loop {
            let frame = process.next_json(deadline).await?;
            if frame.get("method").and_then(Value::as_str) == Some("account/login/completed")
                && frame.pointer("/params/loginId").and_then(Value::as_str) == Some(id)
            {
                return frame
                    .pointer("/params/success")
                    .and_then(Value::as_bool)
                    .ok_or(ProbeError::Incompatible(
                        "Codex omitted its native login result.",
                    ));
            }
            if frame.get("id").is_some() {
                return Err(ProbeError::Incompatible(
                    "Unexpected native request during Codex login.",
                ));
            }
        }
    }
    .await;
    if !matches!(result, Ok(true))
        && let Some(id) = login_id
    {
        // Cancel only THIS native login attempt, never account/logout.
        let _ = process
            .send(&json!({
                "id":4, "method":"account/login/cancel", "params":{"loginId":id}
            }))
            .await;
    }
    process.finish(result).await
}
