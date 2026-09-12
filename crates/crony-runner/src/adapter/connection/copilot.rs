use std::{path::Path, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use crony_domain::{CodingAgent, NativeSignInInstruction, WorkspaceConnectionStatus};
use github_copilot_sdk::{CliProgram, Client, ClientMode, ClientOptions, Transport};
use tokio::{io::AsyncReadExt, process::Command, task::JoinHandle, time::Instant};

use super::{
    AgentReadiness, ProbeError, ProbeResult,
    catalog::{self, LoginTranscript},
    environment::{NativeCommand, ProfileEnvironment},
    process::{OutputLine, SetupProcess, spawn_owned},
};
use crate::adapter::{
    CopilotSdkAdapter,
    copilot::{model_from_sdk, runtime_version_is_supported},
};

struct ClientCleanup(Client);

impl Drop for ClientCleanup {
    fn drop(&mut self) {
        // SDK stop is documented cancel-unsafe; force_stop is its synchronous,
        // idempotent fallback. No sessions exist in an inspection client.
        self.0.force_stop();
    }
}

pub(super) async fn inspect(
    adapter: &CopilotSdkAdapter,
    workspace: &Path,
    deadline: Instant,
) -> ProbeResult<AgentReadiness> {
    let options = adapter.connection_options(workspace).await.map_err(|_| {
        ProbeError::Incompatible(
            "The configured pinned Copilot runtime or account scope is unavailable.",
        )
    })?;
    if Instant::now() >= deadline {
        return Err(ProbeError::Timeout);
    }
    if matches!(&options.transport, Transport::External { .. }) {
        // Only explicit system-account profiles can reach an operator's
        // existing external runtime. This branch owns no local child.
        return inspect_external(options, deadline).await;
    }
    let mut tree = spawn_owned(probe_command(&options)?).await?;
    let streams = {
        let child = tree.child_mut();
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    };
    let (Some(stdin), Some(stdout), Some(stderr)) = streams else {
        let _ = tree.terminate_and_wait().await;
        return Err(ProbeError::Failed(
            "Native Copilot standard streams are unavailable.",
        ));
    };
    let drain = StderrDrain(tokio::spawn(async move {
        let mut limited = stderr.take(1024 * 1024 + 1);
        let bytes = tokio::io::copy(&mut limited, &mut tokio::io::sink())
            .await
            .map_err(ProbeError::io)?;
        if bytes > 1024 * 1024 {
            return Err(ProbeError::Incompatible(
                "Native Copilot output exceeded its bound.",
            ));
        }
        Ok(())
    }));
    // SDK 1.0.11 supports from_streams for custom owned transports. Reuse its
    // RPC implementation with ownership established BEFORE starting the runtime.
    let client = match Client::from_streams(stdout, stdin, workspace.to_path_buf()) {
        Ok(client) => ClientCleanup(client),
        Err(_) => {
            let _ = tree.terminate_and_wait().await;
            return Err(ProbeError::Incompatible(
                "Native Copilot protocol could not be initialized.",
            ));
        }
    };
    let result = tokio::time::timeout_at(deadline, async {
        client.0.verify_protocol_version().await.map_err(|_| {
            ProbeError::Incompatible("Native Copilot protocol does not match the pinned SDK.")
        })?;
        inspect_client(&client.0).await
    })
    .await
    .unwrap_or(Err(ProbeError::Timeout));
    let stopped = tokio::time::timeout(Duration::from_secs(5), client.0.stop()).await;
    tree.terminate_and_wait()
        .await
        .map_err(|_| ProbeError::Failed("Native Copilot process cleanup could not be verified."))?;
    let mut drain = drain;
    match tokio::time::timeout(Duration::from_secs(1), &mut drain.0).await {
        Ok(Ok(Ok(()))) => {}
        _ => {
            return Err(ProbeError::Failed(
                "Native Copilot output cleanup could not be verified.",
            ));
        }
    }
    match stopped {
        Ok(Ok(())) => result,
        _ => Err(ProbeError::Failed(
            "Native Copilot client cleanup could not be verified.",
        )),
    }
}

struct StderrDrain(JoinHandle<ProbeResult<()>>);

impl Drop for StderrDrain {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn inspect_external(
    options: ClientOptions,
    deadline: Instant,
) -> ProbeResult<AgentReadiness> {
    let client = ClientCleanup(
        tokio::time::timeout_at(deadline, Client::start(options))
            .await
            .map_err(|_| ProbeError::Timeout)?
            .map_err(|_| ProbeError::Offline)?,
    );
    let result = tokio::time::timeout_at(deadline, inspect_client(&client.0))
        .await
        .unwrap_or(Err(ProbeError::Timeout));
    match tokio::time::timeout(Duration::from_secs(5), client.0.stop()).await {
        Ok(Ok(())) => result,
        _ => Err(ProbeError::Failed(
            "Native Copilot client cleanup could not be verified.",
        )),
    }
}

pub(super) fn probe_command(options: &ClientOptions) -> ProbeResult<Command> {
    let CliProgram::Path(program) = &options.program else {
        return Err(ProbeError::Incompatible(
            "Native Copilot probes require the selected pinned binary.",
        ));
    };
    if options.mode != ClientMode::Empty {
        return Err(ProbeError::Incompatible(
            "Native Copilot connections require Empty mode.",
        ));
    }
    // Exact SDK 1.0.11 stdio/auth flags. Empty session defaults and session-FS
    // RPC setup remain on execution; this client cannot create/send a session.
    let mut command = Command::new(program);
    command
        .args(&options.prefix_args)
        .args([
            "--server",
            "--stdio",
            "--no-auto-update",
            "--log-level",
            "error",
        ])
        .args(&options.extra_args)
        .current_dir(&options.working_directory)
        .env("COPILOT_DISABLE_KEYTAR", "1");
    if let Some(home) = &options.base_directory {
        command.env("COPILOT_HOME", home);
    }
    if let Some(token) = &options.github_token {
        command
            .env("COPILOT_SDK_AUTH_TOKEN", token)
            .args(["--auth-token-env", "COPILOT_SDK_AUTH_TOKEN"]);
    }
    if !options
        .use_logged_in_user
        .unwrap_or(options.github_token.is_none())
    {
        command.arg("--no-auto-login");
    }
    command.envs(options.env.iter().map(|(key, value)| (key, value)));
    for key in &options.env_remove {
        command.env_remove(key);
    }
    Ok(command)
}

async fn inspect_client(client: &Client) -> ProbeResult<AgentReadiness> {
    let status = client.get_status().await.map_err(|_| ProbeError::Offline)?;
    if !runtime_version_is_supported(&status.version) {
        return Err(ProbeError::Incompatible(
            "Copilot must use the verified SDK 1.0.11 / runtime 1.0.79 pair.",
        ));
    }
    let auth = client
        .get_auth_status()
        .await
        .map_err(|_| ProbeError::Offline)?;
    if !auth.is_authenticated {
        return Ok(AgentReadiness::state(
            WorkspaceConnectionStatus::NeedsSignIn,
            "Copilot needs native sign-in in the selected Empty-mode account scope; machine keychain tokens are not copied.",
        ));
    }
    let models = client
        .list_models()
        .await
        .map_err(|_| ProbeError::Offline)?
        .into_iter()
        .map(model_from_sdk)
        .map(catalog::runner_model)
        .collect::<Vec<_>>();
    catalog::validate_models(&models)?;
    AgentReadiness::ready(models, catalog::github_hint(auth.login.as_deref()))
}

pub(super) async fn sign_in(
    adapter: &CopilotSdkAdapter,
    scope: &ProfileEnvironment,
    workspace: &Path,
    deadline: Instant,
    expires_at: DateTime<Utc>,
    progress: &Arc<dyn Fn(NativeSignInInstruction) + Send + Sync>,
) -> ProbeResult<bool> {
    // AgentProfile already checked this exact configured/bundled binary's
    // native get_status version before allowing the NeedsSignIn transition.
    let (program, prefix) = adapter
        .connection_command()
        .map_err(|_| ProbeError::NotInstalled)?;
    let native = NativeCommand {
        program: Some(program),
        prefix,
    };
    let mut command = native.build(scope, workspace)?;
    command.args(["--no-auto-update", "login", "--device-code"]);
    let mut process = SetupProcess::spawn(command).await?;
    let result = async {
        let mut transcript = LoginTranscript::default();
        let mut emitted = false;
        while let Some(line) = process.next_login(deadline).await? {
            let (OutputLine::Stdout(chunk) | OutputLine::Stderr(chunk)) = line;
            transcript.push(&chunk)?;
            if !emitted
                && let (Some(uri), Some(code)) = (
                    transcript.uri(CodingAgent::GitHubCopilot),
                    transcript.github_code(),
                )
            {
                progress(catalog::sign_in_instruction(
                    CodingAgent::GitHubCopilot,
                    &uri,
                    Some(&code),
                    expires_at,
                    None,
                )?);
                emitted = true;
            }
        }
        process.wait_success(deadline).await
    }
    .await;
    process.finish(result).await
}
