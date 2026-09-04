use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, anyhow};
use async_trait::async_trait;
use dashmap::DashMap;
use github_copilot_sdk::{
    CliProgram, Client, ClientMode, ClientOptions, DeliveryMode, InfiniteSessionConfig, LogLevel,
    MessageOptions, ResumeSessionConfig, SessionConfig, SessionId, SystemMessageConfig, ToolSet,
    Transport,
    handler::{PermissionHandler, PermissionResult},
    rpc::UserSettingsSetRequest,
    types::{
        DisableBypassPermissionsMode, ManagedSettings, ManagedSettingsPermissions,
        PermissionRequestData, RequestId, SessionFsConfig, SessionFsConventions,
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::{
    process::Command,
    sync::{OnceCell, mpsc, oneshot},
};
use uuid::Uuid;

use super::{
    AdapterArtifact, AdapterCapabilities, AdapterControl, AdapterError, AdapterEvent,
    AdapterEventSink, AdapterExit, AdapterModel, AdapterRunRequest, AgentAdapter, FeatureSupport,
    UsageSnapshot,
    copilot_fs::ContainedSessionFs,
    permission::{path_is_inside, path_is_inside_workspace},
};

#[derive(Debug, Clone)]
pub struct CopilotSdkConfig {
    pub cli_path: Option<PathBuf>,
    pub cli_prefix_args: Vec<OsString>,
    pub external_host: Option<String>,
    pub external_port: Option<u16>,
    pub github_token_file: Option<PathBuf>,
    pub connection_token_file: Option<PathBuf>,
    pub base_directory: PathBuf,
    pub use_logged_in_user: bool,
    pub log_level: String,
    pub fixture: bool,
}

#[derive(Clone)]
pub struct CopilotSdkAdapter {
    config: CopilotSdkConfig,
    models: Arc<OnceCell<Vec<AdapterModel>>>,
}

#[derive(Debug)]
struct PermissionDecision {
    approved: bool,
    note: String,
}

struct CronyPermissionHandler {
    workspace: PathBuf,
    state_directory: PathBuf,
    sink: Arc<dyn AdapterEventSink>,
    pending: Arc<DashMap<Uuid, oneshot::Sender<PermissionDecision>>>,
}

#[async_trait]
impl PermissionHandler for CronyPermissionHandler {
    async fn handle(
        &self,
        _session_id: SessionId,
        request_id: RequestId,
        data: PermissionRequestData,
    ) -> PermissionResult {
        let request = data.extra.get("permissionRequest").unwrap_or(&data.extra);
        let managed_approval_required = data.managed_approval_required == Some(true);
        let automatically_safe =
            permission_is_automatically_safe(request, &self.workspace, &self.state_directory);
        if !managed_approval_required && automatically_safe {
            return PermissionResult::approve_once();
        }

        let approval_id = Uuid::new_v4();
        let (tx, rx) = oneshot::channel();
        self.pending.insert(approval_id, tx);
        self.sink.emit(AdapterEvent::ApprovalRequested {
            approval_id,
            action_key: format!(
                "copilot:{}:{}",
                request_id,
                request
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
            ),
            action: permission_summary(request),
            risk: permission_risk(request),
            rationale: permission_rationale(request),
            required_roles: vec![
                "owner".to_owned(),
                "admin".to_owned(),
                "manager".to_owned(),
                "member".to_owned(),
            ],
            expires_in_seconds: 600,
        });
        match tokio::time::timeout(Duration::from_secs(600), rx).await {
            Ok(Ok(decision)) if decision.approved => PermissionResult::approve_once(),
            Ok(Ok(decision)) => PermissionResult::reject(Some(decision.note)),
            _ => PermissionResult::reject(Some("ECorp approval expired".to_owned())),
        }
    }
}

impl CopilotSdkAdapter {
    pub fn new(config: CopilotSdkConfig) -> Self {
        Self {
            config,
            models: Arc::new(OnceCell::new()),
        }
    }

    fn installed(&self) -> bool {
        self.config.fixture
            || self.config.external_host.is_some()
            || self
                .config
                .cli_path
                .as_ref()
                .is_some_and(|path| path.is_file())
            || github_copilot_sdk::HAS_BUNDLED_CLI
    }

    async fn client_options(&self, workspace: &Path) -> Result<ClientOptions, AdapterError> {
        let state_directory = self.state_directory(workspace);
        let conventions = if cfg!(windows) {
            SessionFsConventions::Windows
        } else {
            SessionFsConventions::Posix
        };
        let mut options = ClientOptions::new()
            .with_cwd(workspace)
            .with_base_directory(&state_directory)
            .with_mode(ClientMode::Empty)
            .with_session_fs(SessionFsConfig::new(
                workspace.to_string_lossy(),
                state_directory.to_string_lossy(),
                conventions,
            ))
            .with_env_remove(copilot_sensitive_environment_names())
            .with_log_level(parse_log_level(&self.config.log_level)?);

        if let Some(host) = &self.config.external_host {
            let port = self
                .config
                .external_port
                .context("Copilot external runtime omitted its port")?;
            options = options.with_transport(Transport::External {
                host: host.clone(),
                port,
                connection_token: read_optional_secret(
                    self.config.connection_token_file.as_deref(),
                )
                .await?,
            });
        } else {
            options =
                options.with_extra_args(["--experimental", "--sandbox", "--disallow-temp-dir"]);
            if let Some(path) = &self.config.cli_path {
                options = options
                    .with_program(CliProgram::Path(path.clone()))
                    .with_prefix_args(self.config.cli_prefix_args.clone());
            }
            if let Some(token) =
                read_optional_secret(self.config.github_token_file.as_deref()).await?
            {
                options = options.with_github_token(token);
            } else {
                options = options.with_use_logged_in_user(self.config.use_logged_in_user);
            }
        }
        Ok(options)
    }

    fn state_directory(&self, workspace: &Path) -> PathBuf {
        let digest = hex::encode(Sha256::digest(workspace.to_string_lossy().as_bytes()));
        self.config.base_directory.join(&digest[..24])
    }

    async fn sdk_models_uncached(&self) -> Result<Vec<AdapterModel>, AdapterError> {
        let catalog_workspace = self.config.base_directory.join("catalog-workspace");
        tokio::fs::create_dir_all(&catalog_workspace).await?;
        let client = Client::start(self.client_options(&catalog_workspace).await?)
            .await
            .map_err(sdk_error)?;
        let models = client
            .list_models()
            .await
            .map_err(sdk_error)?
            .into_iter()
            .map(model_from_sdk)
            .collect();
        let _ = client.stop().await;
        Ok(models)
    }

    async fn discovered_models(&self) -> Result<Vec<AdapterModel>, AdapterError> {
        if self.config.fixture {
            return Ok(fixture_models());
        }
        Ok(self
            .models
            .get_or_try_init(|| self.sdk_models_uncached())
            .await?
            .clone())
    }

    async fn sdk_run(
        &self,
        request: AdapterRunRequest,
        resume_session_id: Option<&str>,
        mut controls: mpsc::UnboundedReceiver<AdapterControl>,
        sink: Arc<dyn AdapterEventSink>,
    ) -> Result<AdapterExit, AdapterError> {
        tokio::fs::create_dir_all(&request.workspace).await?;
        let state_directory = self.state_directory(&request.workspace);
        tokio::fs::create_dir_all(&state_directory).await?;
        let client = Client::start(self.client_options(&request.workspace).await?)
            .await
            .map_err(sdk_error)?;
        configure_native_sandbox(&client, &request.workspace).await?;
        let pending = Arc::new(DashMap::new());
        let session_fs = Arc::new(ContainedSessionFs::new(
            request.workspace.clone(),
            state_directory.clone(),
        ));
        let permission_handler = Arc::new(CronyPermissionHandler {
            workspace: request.workspace.clone(),
            state_directory,
            sink: sink.clone(),
            pending: pending.clone(),
        });
        let session = if let Some(session_id) = resume_session_id {
            let mut config = ResumeSessionConfig::new(SessionId::from(session_id));
            apply_resume_config(&mut config, &request);
            client
                .resume_session(
                    config
                        .with_permission_handler(permission_handler)
                        .with_session_fs_provider(session_fs),
                )
                .await
                .map_err(sdk_error)?
        } else {
            let mut config = SessionConfig::default();
            apply_session_config(&mut config, &request);
            client
                .create_session(
                    config
                        .with_permission_handler(permission_handler)
                        .with_session_fs_provider(session_fs),
                )
                .await
                .map_err(sdk_error)?
        };
        let session_id = session.id().to_string();
        sink.emit(AdapterEvent::Session {
            session_id: session_id.clone(),
        });
        sink.emit(AdapterEvent::Started {
            workspace: request.workspace.clone(),
        });

        let mut events = session.subscribe();
        session
            .send(request.mission_title.clone())
            .await
            .map_err(sdk_error)?;
        let mut streamed_messages = HashSet::<String>::new();
        let mut tool_signatures = HashMap::<String, String>::new();
        let mut assistant_messages = 0_u64;
        let mut tool_calls = 0_u64;
        let mut cancelled = None;
        let mut failed = None;
        let mut controls_open = true;

        loop {
            tokio::select! {
                control = controls.recv(), if controls_open => match control {
                    Some(AdapterControl::Steer { actor_id, text }) => {
                        session
                            .send(
                                MessageOptions::new(format!("Direction from {actor_id}: {text}"))
                                    .with_mode(DeliveryMode::Immediate),
                            )
                            .await
                            .map_err(sdk_error)?;
                    }
                    Some(AdapterControl::Interrupt { reason })
                    | Some(AdapterControl::Stop { reason }) => {
                        reject_pending(&pending, &reason);
                        cancelled = Some(reason);
                        session.abort().await.map_err(sdk_error)?;
                    }
                    Some(AdapterControl::ApprovalDecision {
                        approval_id,
                        approved,
                        note,
                    }) => {
                        if let Some((_, sender)) = pending.remove(&approval_id) {
                            let _ = sender.send(PermissionDecision { approved, note });
                        }
                    }
                    Some(AdapterControl::CircuitBreaker { stage, reason }) => {
                        if matches!(stage.as_str(), "suspend" | "stop") {
                            let reason =
                                format!("Circuit breaker {stage} checkpoint: {reason}");
                            reject_pending(&pending, &reason);
                            cancelled = Some(reason);
                            session.abort().await.map_err(sdk_error)?;
                        } else {
                            session
                                .send(
                                    MessageOptions::new(format!(
                                        "ECorp circuit breaker {stage}: {reason}"
                                    ))
                                    .with_mode(DeliveryMode::Immediate),
                                )
                                .await
                                .map_err(sdk_error)?;
                        }
                    }
                    None => controls_open = false,
                },
                event = events.recv() => {
                    let event = match event {
                        Ok(event) => event,
                        Err(error) => {
                            failed = Some(format!("Copilot event stream closed: {error}"));
                            break;
                        }
                    };
                    match event.event_type.as_str() {
                        "assistant.turn_start" => sink.emit(AdapterEvent::Status {
                            status: "working".to_owned(),
                            station: "copilot".to_owned(),
                            message: format!(
                                "GitHub Copilot turn using {}",
                                event.data.get("model").and_then(Value::as_str)
                                    .or(request.model.as_deref())
                                    .unwrap_or("default model")
                            ),
                        }),
                        "assistant.message_delta" => {
                            if let Some(message_id) = event.data.get("messageId").and_then(Value::as_str) {
                                streamed_messages.insert(message_id.to_owned());
                            }
                            if let Some(text) = event.data.get("deltaContent").and_then(Value::as_str) {
                                sink.emit(AdapterEvent::Output {
                                    stream: "assistant".to_owned(),
                                    text: text.to_owned(),
                                });
                            }
                        }
                        "assistant.message" => {
                            assistant_messages += 1;
                            let message_id = event.data.get("messageId").and_then(Value::as_str);
                            if !message_id.is_some_and(|id| streamed_messages.contains(id))
                                && let Some(text) = event.data.get("content").and_then(Value::as_str)
                            {
                                sink.emit(AdapterEvent::Output {
                                    stream: "assistant".to_owned(),
                                    text: text.to_owned(),
                                });
                            }
                        }
                        "assistant.reasoning_delta" => {
                            if let Some(text) = event.data.get("deltaContent").and_then(Value::as_str) {
                                sink.emit(AdapterEvent::Output {
                                    stream: "reasoning".to_owned(),
                                    text: text.to_owned(),
                                });
                            }
                        }
                        "assistant.usage" => sink.emit(AdapterEvent::Usage(UsageSnapshot {
                            input_tokens: event.data.get("inputTokens").and_then(Value::as_u64).unwrap_or_default(),
                            output_tokens: event.data.get("outputTokens").and_then(Value::as_u64).unwrap_or_default(),
                            cost_microusd: 0,
                        })),
                        "tool.execution_start" => {
                            tool_calls += 1;
                            let tool_call_id = event.data.get("toolCallId").and_then(Value::as_str).unwrap_or("unknown");
                            let tool_name = event.data.get("toolName").and_then(Value::as_str).unwrap_or("copilot-tool");
                            tool_signatures.insert(
                                tool_call_id.to_owned(),
                                tool_activity_signature(tool_name, event.data.get("arguments")),
                            );
                            sink.emit(AdapterEvent::Status {
                                status: "working".to_owned(),
                                station: "tool".to_owned(),
                                message: format!("GitHub Copilot is running {tool_name}"),
                            });
                        }
                        "tool.execution_complete" => {
                            let tool_call_id = event.data.get("toolCallId").and_then(Value::as_str).unwrap_or("unknown");
                            sink.emit(AdapterEvent::ToolActivity {
                                signature: tool_signatures.remove(tool_call_id).unwrap_or_else(|| "copilot-tool:unknown".to_owned()),
                                progressed: event.data.get("success").and_then(Value::as_bool).unwrap_or(false),
                                human_conversation: false,
                            });
                        }
                        "session.error" => {
                            failed = Some(
                                event.data.get("message").and_then(Value::as_str)
                                    .unwrap_or("GitHub Copilot session failed")
                                    .to_owned()
                            );
                            break;
                        }
                        "session.idle" => {
                            if event.data.get("aborted").and_then(Value::as_bool) == Some(true)
                                && cancelled.is_none()
                            {
                                cancelled = Some("GitHub Copilot session was aborted".to_owned());
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }

        pending.clear();
        let _ = session.disconnect().await;
        let _ = client.stop().await;

        if let Some(reason) = cancelled {
            sink.emit(AdapterEvent::Cancelled { reason });
            return Ok(AdapterExit::Cancelled);
        }
        if let Some(error) = failed {
            sink.emit(AdapterEvent::Failed { error });
            return Ok(AdapterExit::Failed);
        }
        let artifact = write_evidence(
            &request,
            &session_id,
            assistant_messages,
            tool_calls,
            self.discovered_models().await?.len(),
        )
        .await?;
        sink.emit(AdapterEvent::Artifact(artifact));
        sink.emit(AdapterEvent::Completed {
            summary: format!(
                "GitHub Copilot completed with {}.",
                request.model.as_deref().unwrap_or("its default model")
            ),
        });
        Ok(AdapterExit::Completed)
    }

    async fn fixture_run(
        &self,
        request: AdapterRunRequest,
        resume_session_id: Option<&str>,
        sink: Arc<dyn AdapterEventSink>,
    ) -> Result<AdapterExit, AdapterError> {
        tokio::fs::create_dir_all(&request.workspace).await?;
        let session_id = resume_session_id
            .map(str::to_owned)
            .unwrap_or_else(|| format!("copilot-fixture-{}", request.run_id));
        sink.emit(AdapterEvent::Session {
            session_id: session_id.clone(),
        });
        sink.emit(AdapterEvent::Started {
            workspace: request.workspace.clone(),
        });
        sink.emit(AdapterEvent::Status {
            status: "working".to_owned(),
            station: "copilot".to_owned(),
            message: format!(
                "GitHub Copilot fixture using {}",
                request.model.as_deref().unwrap_or("default model")
            ),
        });
        tokio::fs::write(
            request.workspace.join("copilot-result.md"),
            format!("# GitHub Copilot result\n\n{}\n", request.mission_title),
        )
        .await?;
        sink.emit(AdapterEvent::Usage(UsageSnapshot {
            input_tokens: 321,
            output_tokens: 123,
            cost_microusd: 0,
        }));
        let artifact = write_evidence(&request, &session_id, 1, 1, fixture_models().len()).await?;
        sink.emit(AdapterEvent::Artifact(artifact));
        sink.emit(AdapterEvent::Completed {
            summary: "GitHub Copilot fixture completed with normalized evidence.".to_owned(),
        });
        Ok(AdapterExit::Completed)
    }
}

#[async_trait]
impl AgentAdapter for CopilotSdkAdapter {
    fn id(&self) -> &'static str {
        "github-copilot"
    }

    fn display_name(&self) -> &'static str {
        "GitHub Copilot SDK"
    }

    fn capabilities(&self) -> AdapterCapabilities {
        let installed = if self.installed() {
            FeatureSupport::Supported
        } else {
            FeatureSupport::Unsupported {
                reason:
                    "no bundled Copilot CLI, explicit CLI path, or external Copilot runtime is configured"
                        .to_owned(),
            }
        };
        AdapterCapabilities {
            spawn: installed.clone(),
            stream: installed.clone(),
            steer: installed.clone(),
            interrupt: installed.clone(),
            stop: installed.clone(),
            resume: installed.clone(),
            usage: installed.clone(),
            artifacts: installed,
        }
    }

    async fn list_models(&self) -> Result<Vec<AdapterModel>, AdapterError> {
        self.discovered_models().await
    }

    async fn execute(
        &self,
        request: AdapterRunRequest,
        controls: mpsc::UnboundedReceiver<AdapterControl>,
        sink: Arc<dyn AdapterEventSink>,
    ) -> Result<AdapterExit, AdapterError> {
        if self.config.fixture {
            self.fixture_run(request, None, sink).await
        } else {
            self.sdk_run(request, None, controls, sink).await
        }
    }

    async fn resume(
        &self,
        request: AdapterRunRequest,
        session_id: &str,
        controls: mpsc::UnboundedReceiver<AdapterControl>,
        sink: Arc<dyn AdapterEventSink>,
    ) -> Result<AdapterExit, AdapterError> {
        if self.config.fixture {
            self.fixture_run(request, Some(session_id), sink).await
        } else {
            self.sdk_run(request, Some(session_id), controls, sink)
                .await
        }
    }
}

fn apply_session_config(config: &mut SessionConfig, request: &AdapterRunRequest) {
    config.model = request.model.clone();
    config.reasoning_effort = request.reasoning_effort.clone();
    config.streaming = Some(true);
    config.working_directory = Some(request.workspace.clone());
    config.available_tools = Some(
        ToolSet::new()
            .add_builtin("*")
            .expect("valid Copilot builtin wildcard")
            .into(),
    );
    config.excluded_tools = Some(vec![
        "builtin:web_fetch".to_owned(),
        "builtin:web_search".to_owned(),
    ]);
    config.system_message = Some(
        SystemMessageConfig::new()
            .with_mode("append")
            .with_content(format!(
                "You are supervised by ECorp. Your assigned worktree is {}. Use relative paths rooted there or that exact absolute path; do not use /workspace and never invent a home-directory path. Work only inside that worktree. Use Copilot's built-in read, create, and edit tools for file operations; reserve shell commands for build and test programs. Network access and sandbox bypass require a durable human approval. Produce concrete repository changes and verification evidence.",
                request.workspace.display()
            )),
    );
    // ECorp owns the worktree lifecycle. Copilot infinite-session workspaces copy
    // files under COPILOT_HOME, which would move writes outside the assigned tree.
    config.infinite_sessions = Some(InfiniteSessionConfig::new().with_enabled(false));
    config.enable_config_discovery = Some(false);
    config.enable_session_store = Some(true);
    config.managed_settings = Some(copilot_native_managed_settings());
}

async fn configure_native_sandbox(client: &Client, workspace: &Path) -> Result<(), AdapterError> {
    let denied_paths = copilot_sandbox_denied_paths(workspace);
    let result = client
        .rpc()
        .user()
        .settings()
        .set(UserSettingsSetRequest {
            settings: json!({
                "sandbox": {
                    "enabled": true,
                    "addCurrentWorkingDirectory": true,
                    "allowDevToolAccess": true,
                    "allowBypass": false,
                    "auth": {
                        "gh": false,
                        "git": false
                    },
                    "sandboxMcpServers": true,
                    "sandboxLspServers": true,
                    "userPolicy": {
                        "filesystem": {
                            "clearPolicyOnExit": true,
                            "deniedPaths": denied_paths,
                            "readonlyPaths": [],
                            "readwritePaths": [workspace.to_string_lossy()]
                        },
                        "network": {
                            "allowLocalNetwork": false,
                            "allowOutbound": false
                        }
                    }
                }
            }),
        })
        .await
        .map_err(sdk_error)?;
    if result
        .shadowed_keys
        .iter()
        .any(|key| key.eq_ignore_ascii_case("sandbox"))
    {
        return Err(AdapterError::Runtime(anyhow!(
            "GitHub Copilot sandbox settings are shadowed by legacy configuration"
        )));
    }
    client
        .rpc()
        .user()
        .settings()
        .reload()
        .await
        .map_err(sdk_error)?;
    let settings = client
        .rpc()
        .user()
        .settings()
        .get()
        .await
        .map_err(sdk_error)?;
    let sandbox = settings
        .settings
        .get("sandbox")
        .map(|metadata| &metadata.value)
        .context("GitHub Copilot omitted the required sandbox setting")
        .map_err(AdapterError::Runtime)?;
    if sandbox.pointer("/enabled").and_then(Value::as_bool) != Some(true)
        || sandbox.pointer("/allowBypass").and_then(Value::as_bool) != Some(false)
        || sandbox
            .pointer("/userPolicy/network/allowOutbound")
            .and_then(Value::as_bool)
            != Some(false)
        || sandbox
            .pointer("/userPolicy/network/allowLocalNetwork")
            .and_then(Value::as_bool)
            != Some(false)
    {
        return Err(AdapterError::Runtime(anyhow!(
            "GitHub Copilot did not retain the required native sandbox restrictions"
        )));
    }
    Ok(())
}

fn copilot_sandbox_denied_paths(workspace: &Path) -> Vec<String> {
    let Some(home) = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
    else {
        return Vec::new();
    };
    let mut denied = vec![home.to_string_lossy().into_owned()];
    let candidates = [
        ".ssh",
        ".aws",
        ".azure",
        ".kube",
        ".docker",
        ".gnupg",
        ".mcp-auth",
        ".copilot",
        ".claude",
        ".npmrc",
        ".git-credentials",
        ".cargo/credentials",
        ".cargo/credentials.toml",
        ".config/gh",
        ".codex/auth.json",
        "AppData/Roaming/GitHub CLI/hosts.yml",
    ];
    denied.extend(
        candidates
            .into_iter()
            .map(|relative| {
                relative
                    .split('/')
                    .fold(home.clone(), |path, component| path.join(component))
            })
            .filter(|path| path != workspace)
            .map(|path| path.to_string_lossy().into_owned()),
    );
    denied
}

fn apply_resume_config(config: &mut ResumeSessionConfig, request: &AdapterRunRequest) {
    config.model = request.model.clone();
    config.reasoning_effort = request.reasoning_effort.clone();
    config.streaming = Some(true);
    config.working_directory = Some(request.workspace.clone());
    config.available_tools = Some(
        ToolSet::new()
            .add_builtin("*")
            .expect("valid Copilot builtin wildcard")
            .into(),
    );
    config.excluded_tools = Some(vec![
        "builtin:web_fetch".to_owned(),
        "builtin:web_search".to_owned(),
    ]);
    config.system_message = Some(
        SystemMessageConfig::new()
            .with_mode("append")
            .with_content(format!(
                "You are supervised by ECorp. Your assigned worktree is {}. Use relative paths rooted there or that exact absolute path; do not use /workspace and never invent a home-directory path. Continue only inside that worktree. Use Copilot's built-in read, create, and edit tools for file operations; reserve shell commands for build and test programs. Network access and sandbox bypass require a durable human approval.",
                request.workspace.display()
            )),
    );
    config.infinite_sessions = Some(InfiniteSessionConfig::new().with_enabled(false));
    config.enable_config_discovery = Some(false);
    config.enable_session_store = Some(true);
    config.managed_settings = Some(copilot_native_managed_settings());
}

fn copilot_native_managed_settings() -> ManagedSettings {
    ManagedSettings::default().with_permissions(
        ManagedSettingsPermissions::default()
            .with_disable_bypass_permissions_mode(DisableBypassPermissionsMode::Disable)
            .with_allow(vec!["read".to_owned(), "write".to_owned()])
            .with_ask(vec!["shell".to_owned()]),
    )
}

fn copilot_sensitive_environment_names() -> [&'static str; 20] {
    [
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GITHUB_PAT",
        "AZURE_CLIENT_SECRET",
        "AZURE_CLIENT_CERTIFICATE_PATH",
        "AZURE_TENANT_ID",
        "AZURE_SUBSCRIPTION_ID",
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "AWS_SESSION_TOKEN",
        "GOOGLE_APPLICATION_CREDENTIALS",
        "GOOGLE_API_KEY",
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "NPM_TOKEN",
        "NODE_AUTH_TOKEN",
        "DOCKER_AUTH_CONFIG",
        "KUBECONFIG",
        "SSH_AUTH_SOCK",
        "GIT_ASKPASS",
    ]
}

fn model_from_sdk(model: github_copilot_sdk::Model) -> AdapterModel {
    let value = serde_json::to_value(&model).unwrap_or_else(|_| json!({}));
    AdapterModel {
        id: model.id,
        name: model.name,
        policy_state: value
            .pointer("/policy/state")
            .and_then(Value::as_str)
            .map(str::to_owned),
        policy_terms: value
            .pointer("/policy/terms")
            .and_then(Value::as_str)
            .map(str::to_owned),
        supports_vision: value
            .pointer("/capabilities/supports/vision")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        supports_reasoning_effort: value
            .pointer("/capabilities/supports/reasoningEffort")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        max_prompt_tokens: value
            .pointer("/capabilities/limits/max_prompt_tokens")
            .or_else(|| value.pointer("/capabilities/limits/maxPromptTokens"))
            .and_then(Value::as_u64),
        max_context_window_tokens: value
            .pointer("/capabilities/limits/max_context_window_tokens")
            .or_else(|| value.pointer("/capabilities/limits/maxContextWindowTokens"))
            .and_then(Value::as_u64),
        supported_reasoning_efforts: model.supported_reasoning_efforts.unwrap_or_default(),
        default_reasoning_effort: value
            .get("defaultReasoningEffort")
            .and_then(Value::as_str)
            .map(str::to_owned),
        billing_multiplier: model.billing.and_then(|billing| billing.multiplier),
    }
}

fn tool_activity_signature(tool_name: &str, arguments: Option<&Value>) -> String {
    let mut canonical_arguments = arguments.cloned().unwrap_or(Value::Null);
    canonicalize_json(&mut canonical_arguments);
    let encoded = serde_json::to_vec(&canonical_arguments).unwrap_or_default();
    let digest = hex::encode(Sha256::digest(encoded));
    format!("{tool_name}:{}", &digest[..16])
}

fn canonicalize_json(value: &mut Value) {
    match value {
        Value::Array(values) => {
            for value in values {
                canonicalize_json(value);
            }
        }
        Value::Object(values) => {
            let mut entries = std::mem::take(values).into_iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            for (key, mut value) in entries {
                canonicalize_json(&mut value);
                values.insert(key, value);
            }
        }
        _ => {}
    }
}

fn fixture_models() -> Vec<AdapterModel> {
    vec![
        AdapterModel {
            id: "copilot-test-fast".to_owned(),
            name: "Copilot Test Fast".to_owned(),
            policy_state: Some("enabled".to_owned()),
            policy_terms: None,
            supports_vision: false,
            supports_reasoning_effort: false,
            max_prompt_tokens: None,
            max_context_window_tokens: Some(64_000),
            supported_reasoning_efforts: Vec::new(),
            default_reasoning_effort: None,
            billing_multiplier: Some(1.0),
        },
        AdapterModel {
            id: "copilot-test-reasoning".to_owned(),
            name: "Copilot Test Reasoning".to_owned(),
            policy_state: Some("enabled".to_owned()),
            policy_terms: None,
            supports_vision: true,
            supports_reasoning_effort: true,
            max_prompt_tokens: Some(120_000),
            max_context_window_tokens: Some(128_000),
            supported_reasoning_efforts: vec![
                "low".to_owned(),
                "medium".to_owned(),
                "high".to_owned(),
            ],
            default_reasoning_effort: Some("medium".to_owned()),
            billing_multiplier: Some(1.5),
        },
        AdapterModel {
            id: "copilot-test-disabled".to_owned(),
            name: "Copilot Test Disabled".to_owned(),
            policy_state: Some("disabled".to_owned()),
            policy_terms: Some("fixture policy".to_owned()),
            supports_vision: false,
            supports_reasoning_effort: false,
            max_prompt_tokens: None,
            max_context_window_tokens: Some(32_000),
            supported_reasoning_efforts: Vec::new(),
            default_reasoning_effort: None,
            billing_multiplier: None,
        },
    ]
}

async fn write_evidence(
    request: &AdapterRunRequest,
    session_id: &str,
    assistant_messages: u64,
    tool_calls: u64,
    model_count: usize,
) -> Result<AdapterArtifact, AdapterError> {
    let status = Command::new("git")
        .args(["status", "--porcelain=v1", "--untracked-files=all"])
        .current_dir(&request.workspace)
        .output()
        .await?;
    let diff = Command::new("git")
        .args(["diff", "--binary", "HEAD"])
        .current_dir(&request.workspace)
        .output()
        .await?;
    let changed_paths = if status.status.success() {
        String::from_utf8_lossy(&status.stdout)
            .lines()
            .filter_map(|line| line.get(3..))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let evidence = serde_json::to_vec_pretty(&json!({
        "provider": "github-copilot",
        "sdk_version": "1.0.11",
        "session_id": session_id,
        "model": request.model.as_deref().unwrap_or("copilot-default"),
        "reasoning_effort": request.reasoning_effort,
        "discovered_model_count": model_count,
        "assistant_messages": assistant_messages,
        "tool_calls": tool_calls,
        "changed_paths": changed_paths,
        "diff_sha256": diff.status.success().then(|| hex::encode(Sha256::digest(&diff.stdout))),
    }))
    .context("serialize GitHub Copilot evidence")?;
    let path = request.workspace.join("copilot-evidence.json");
    tokio::fs::write(&path, &evidence).await?;
    Ok(AdapterArtifact {
        path,
        sha256: hex::encode(Sha256::digest(&evidence)),
        bytes: evidence.len(),
        media_type: "application/json".to_owned(),
    })
}

fn permission_is_automatically_safe(
    request: &Value,
    workspace: &Path,
    state_directory: &Path,
) -> bool {
    if request
        .get("managedApprovalRequired")
        .and_then(Value::as_bool)
        == Some(true)
        || request.get("requestSandboxBypass").and_then(Value::as_bool) == Some(true)
    {
        return false;
    }
    match request.get("kind").and_then(Value::as_str) {
        Some("read") => request
            .get("path")
            .and_then(Value::as_str)
            .is_some_and(|candidate| {
                path_is_inside_workspace(workspace, candidate)
                    || path_is_inside(state_directory, candidate)
            }),
        Some("write") => request
            .get("fileName")
            .and_then(Value::as_str)
            .is_some_and(|candidate| path_is_inside_workspace(workspace, candidate)),
        Some("shell") => {
            let commands_read_only = request
                .get("commands")
                .and_then(Value::as_array)
                .is_some_and(|commands| {
                    !commands.is_empty()
                        && commands.iter().all(|command| {
                            command.get("readOnly").and_then(Value::as_bool) == Some(true)
                        })
                });
            let command_is_known_pathless_read_only =
                shell_command_is_known_pathless_read_only(request);
            let command_is_scoped_read_only =
                shell_command_is_scoped_read_only(request, workspace, state_directory);
            let possible_paths = request
                .get("possiblePaths")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let paths_are_read_scoped = !possible_paths.is_empty()
                && possible_paths.iter().all(|candidate| {
                    candidate.as_str().is_some_and(|candidate| {
                        path_is_inside_workspace(workspace, candidate)
                            || path_is_inside(state_directory, candidate)
                    })
                });
            request
                .get("possibleUrls")
                .and_then(Value::as_array)
                .is_none_or(Vec::is_empty)
                && !request
                    .get("hasWriteFileRedirection")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                && (command_is_known_pathless_read_only
                    || command_is_scoped_read_only
                    || (commands_read_only && paths_are_read_scoped))
        }
        _ => false,
    }
}

fn shell_command_is_known_pathless_read_only(request: &Value) -> bool {
    let Some(command) = request.get("fullCommandText").and_then(Value::as_str) else {
        return false;
    };
    let command = command.trim().to_ascii_lowercase();
    if command.is_empty()
        || command.contains('\n')
        || command.contains('`')
        || command.contains('&')
        || command.contains('>')
        || command.contains('<')
        || command.contains("||")
        || command.contains("$(")
        || command.contains("${")
        || command.contains("$env:")
        || command.contains("::")
    {
        return false;
    }
    if pathless_read_only_segment(&command) {
        return true;
    }
    request
        .get("commandSegments")
        .and_then(Value::as_array)
        .is_some_and(|segments| {
            !segments.is_empty()
                && segments.iter().all(|segment| {
                    segment
                        .get("fullCommandText")
                        .and_then(Value::as_str)
                        .is_some_and(|segment| {
                            pathless_read_only_segment(&segment.trim().to_ascii_lowercase())
                        })
                })
        })
}

fn pathless_read_only_segment(command: &str) -> bool {
    let exact = [
        "pwd",
        "get-location",
        "(get-location).path",
        "$pwd.path",
        "ls",
        "ls -a",
        "ls -la",
        "dir",
        "get-childitem",
        "get-childitem -force",
    ];
    if exact.contains(&command) {
        return true;
    }
    if pathless_directory_listing(command) {
        return true;
    }
    if [
        "git status",
        "git diff",
        "git rev-parse",
        "git log",
        "git show",
    ]
    .iter()
    .any(|prefix| command == *prefix || command.starts_with(&format!("{prefix} ")))
    {
        return !["--ext-diff", "--no-index", "--output", "--textconv", "-c "]
            .iter()
            .any(|forbidden| command.contains(forbidden));
    }
    command
        .strip_prefix("select-object ")
        .is_some_and(simple_property_list)
}

fn pathless_directory_listing(command: &str) -> bool {
    let mut parts = command.split_ascii_whitespace();
    let Some(program) = parts.next() else {
        return false;
    };
    match program {
        "get-childitem" => parts.all(|argument| {
            matches!(
                argument,
                "-force" | "-name" | "-file" | "-directory" | "-hidden"
            )
        }),
        "ls" => parts.all(|argument| matches!(argument, "-a" | "-l" | "-al" | "-la" | "-1")),
        "dir" => parts.next().is_none(),
        _ => false,
    }
}

fn shell_command_is_scoped_read_only(
    request: &Value,
    workspace: &Path,
    state_directory: &Path,
) -> bool {
    let Some(command) = request.get("fullCommandText").and_then(Value::as_str) else {
        return false;
    };
    let lower = command.to_ascii_lowercase();
    if command.is_empty()
        || command.contains('\n')
        || command.contains('`')
        || command.contains('&')
        || command.contains('>')
        || command.contains('<')
        || command.contains("$(")
        || command.contains("${")
        || lower.contains("$env:")
        || lower.contains("[environment]")
        || lower.contains("::")
    {
        return false;
    }

    let Some(segments) = request.get("commandSegments").and_then(Value::as_array) else {
        return false;
    };
    if segments.is_empty()
        || !segments.iter().all(|segment| {
            let identifier = segment
                .get("identifier")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            let full = segment
                .get("fullCommandText")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            matches!(
                identifier.as_str(),
                "get-location"
                    | "get-childitem"
                    | "join-path"
                    | "test-path"
                    | "get-content"
                    | "write-output"
                    | "select-object"
            ) && (identifier != "select-object"
                || full
                    .strip_prefix("select-object ")
                    .is_some_and(simple_property_list))
        })
    {
        return false;
    }

    let assigned_variable = assigned_join_path_variable(command);
    let references = variable_references(command);
    if command.contains('=') && assigned_variable.is_none() {
        return false;
    }
    match assigned_variable {
        Some(variable)
            if references
                .iter()
                .any(|candidate| !candidate.eq_ignore_ascii_case(&variable)) =>
        {
            return false;
        }
        None if !references.is_empty() => return false,
        _ => {}
    }

    let path_literals = quoted_path_literals(command);
    !path_literals.is_empty()
        && path_literals.iter().all(|candidate| {
            path_is_inside_workspace(workspace, candidate)
                || path_is_inside(state_directory, candidate)
        })
}

fn assigned_join_path_variable(command: &str) -> Option<String> {
    let (assignment, expression) = command.split_once('=')?;
    if expression.contains('=')
        || !expression
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("join-path ")
    {
        return None;
    }
    let variable = assignment.trim().strip_prefix('$')?;
    (!variable.is_empty()
        && variable
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_'))
    .then(|| variable.to_owned())
}

fn variable_references(command: &str) -> Vec<String> {
    let mut references = Vec::new();
    let bytes = command.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'$' {
            index += 1;
            continue;
        }
        let start = index + 1;
        let mut end = start;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            end += 1;
        }
        if end > start {
            references.push(command[start..end].to_owned());
        }
        index = end.max(index + 1);
    }
    references
}

fn quoted_path_literals(command: &str) -> Vec<String> {
    let mut literals = Vec::new();
    let mut quote = None;
    let mut start = 0;
    for (index, character) in command.char_indices() {
        match quote {
            None if matches!(character, '\'' | '"') => {
                quote = Some(character);
                start = index + character.len_utf8();
            }
            Some(expected) if character == expected => {
                let literal = &command[start..index];
                if looks_like_path(literal) {
                    literals.push(literal.to_owned());
                }
                quote = None;
            }
            _ => {}
        }
    }
    literals
}

fn looks_like_path(value: &str) -> bool {
    value.contains('/')
        || value.contains('\\')
        || value.starts_with('.')
        || value
            .rsplit_once('.')
            .is_some_and(|(_, extension)| !extension.is_empty() && extension.len() <= 12)
}

fn simple_property_list(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, ' ' | '\t' | ',' | '-' | '_' | '.' | '*')
        })
}

fn permission_summary(request: &Value) -> String {
    match request.get("kind").and_then(Value::as_str) {
        Some("read") => format!(
            "Read {}",
            request
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("a path")
        ),
        Some("write") => format!(
            "Write {}",
            request
                .get("fileName")
                .and_then(Value::as_str)
                .unwrap_or("a file")
        ),
        Some("shell") => format!(
            "Run: {}",
            summarize_for_approval(
                request
                    .get("fullCommandText")
                    .and_then(Value::as_str)
                    .unwrap_or("an unspecified shell command")
            )
        ),
        Some("url") => format!(
            "Access {}",
            request
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("a URL")
        ),
        Some("mcp") => format!(
            "Call {}/{}",
            request
                .get("serverName")
                .and_then(Value::as_str)
                .unwrap_or("MCP"),
            request
                .get("toolName")
                .and_then(Value::as_str)
                .unwrap_or("tool")
        ),
        Some(kind) => format!("Authorize GitHub Copilot {kind} operation"),
        None => "Authorize a GitHub Copilot operation".to_owned(),
    }
}

fn summarize_for_approval(value: &str) -> String {
    const LIMIT: usize = 500;
    let mut summary = value.chars().take(LIMIT).collect::<String>();
    if value.chars().count() > LIMIT {
        summary.push('…');
    }
    summary
}

fn permission_risk(request: &Value) -> String {
    if request.get("requestSandboxBypass").and_then(Value::as_bool) == Some(true) {
        "critical".to_owned()
    } else {
        match request.get("kind").and_then(Value::as_str) {
            Some("read") => "low",
            Some("write") => "medium",
            Some("shell") | Some("url") => "high",
            _ => "high",
        }
        .to_owned()
    }
}

fn permission_rationale(request: &Value) -> String {
    [
        "intention",
        "requestSandboxBypassReason",
        "toolDescription",
        "description",
    ]
    .iter()
    .find_map(|field| request.get(*field).and_then(Value::as_str))
    .unwrap_or(
        "GitHub Copilot requested a capability outside ECorp’s automatically approved worktree boundary.",
    )
    .to_owned()
}

async fn read_optional_secret(path: Option<&Path>) -> Result<Option<String>, AdapterError> {
    let Some(path) = path else {
        return Ok(None);
    };
    let value = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("read Copilot credential file {}", path.display()))?;
    let value = value.trim();
    if value.is_empty() {
        return Err(AdapterError::Runtime(anyhow!(
            "Copilot credential file {} is empty",
            path.display()
        )));
    }
    Ok(Some(value.to_owned()))
}

fn parse_log_level(value: &str) -> Result<LogLevel, AdapterError> {
    match value {
        "none" => Ok(LogLevel::None),
        "error" => Ok(LogLevel::Error),
        "warning" => Ok(LogLevel::Warning),
        "info" => Ok(LogLevel::Info),
        "debug" => Ok(LogLevel::Debug),
        "all" => Ok(LogLevel::All),
        other => Err(AdapterError::Runtime(anyhow!(
            "invalid Copilot log level {other}"
        ))),
    }
}

fn sdk_error(error: github_copilot_sdk::Error) -> AdapterError {
    AdapterError::Runtime(anyhow!(error.to_string()))
}

fn reject_pending(pending: &DashMap<Uuid, oneshot::Sender<PermissionDecision>>, reason: &str) {
    let ids = pending.iter().map(|entry| *entry.key()).collect::<Vec<_>>();
    for id in ids {
        if let Some((_, sender)) = pending.remove(&id) {
            let _ = sender.send(PermissionDecision {
                approved: false,
                note: reason.to_owned(),
            });
        }
    }
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

    #[test]
    fn native_managed_permissions_never_pre_authorize_shell() {
        let managed = copilot_native_managed_settings();
        let permissions = managed.permissions.expect("managed permissions");
        assert_eq!(
            permissions.disable_bypass_permissions_mode,
            Some(DisableBypassPermissionsMode::Disable)
        );
        let allow = permissions.allow.expect("managed allow rules");
        assert!(allow.contains(&"read".to_owned()));
        assert!(allow.contains(&"write".to_owned()));
        assert!(
            allow.iter().all(|rule| !rule.starts_with("shell")),
            "managed shell allows bypass the worktree-scoped permission handler"
        );
        let ask = permissions.ask.expect("managed ask rules");
        assert_eq!(ask, vec!["shell".to_owned()]);
    }

    #[test]
    fn copilot_cli_does_not_inherit_common_credential_environment() {
        let names = copilot_sensitive_environment_names();
        assert!(names.contains(&"GH_TOKEN"));
        assert!(names.contains(&"OPENAI_API_KEY"));
        assert!(names.contains(&"AWS_SECRET_ACCESS_KEY"));
        assert!(names.contains(&"AZURE_CLIENT_SECRET"));
        assert!(
            !names.contains(&"COPILOT_SDK_AUTH_TOKEN"),
            "the SDK must retain its scoped authentication channel"
        );
    }

    #[test]
    fn sandbox_denies_the_user_profile_and_sensitive_stores() {
        let workspace = std::env::temp_dir()
            .join("crony-copilot-sandbox")
            .join("worktree");
        let denied = copilot_sandbox_denied_paths(&workspace);
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .expect("test home");
        assert!(denied.contains(&home.to_string_lossy().into_owned()));
        assert!(denied.iter().any(|path| path.ends_with(".ssh")));
        assert!(denied.iter().any(|path| path.ends_with(".npmrc")));
    }

    #[test]
    fn fixture_exposes_every_model_and_policy_state() {
        let models = fixture_models();
        assert_eq!(models.len(), 3);
        assert!(models.iter().any(|model| model.supports_reasoning_effort));
        assert!(
            models
                .iter()
                .any(|model| model.policy_state.as_deref() == Some("disabled"))
        );
    }

    #[test]
    fn tool_activity_signature_tracks_arguments_without_exposing_them() {
        let first = tool_activity_signature(
            "view",
            Some(&json!({
                "path": "src/alpha.ts",
                "options": {"line": 10, "context": 3}
            })),
        );
        let reordered = tool_activity_signature(
            "view",
            Some(&json!({
                "options": {"context": 3, "line": 10},
                "path": "src/alpha.ts"
            })),
        );
        let second = tool_activity_signature(
            "view",
            Some(&json!({
                "path": "src/beta.ts",
                "options": {"line": 10, "context": 3}
            })),
        );
        assert_eq!(first, reordered);
        assert_ne!(first, second);
        assert!(first.starts_with("view:"));
        assert!(!first.contains("alpha"));
        assert!(!first.contains("src/"));
    }

    #[test]
    fn permission_policy_is_scoped_to_the_worktree() {
        let root = std::env::temp_dir()
            .join("crony-copilot-permission-tests")
            .join(Uuid::new_v4().to_string());
        let workspace = root.join("workspace");
        let state_directory = root.join("copilot-state");
        std::fs::create_dir_all(workspace.join("src")).expect("create workspace");
        std::fs::create_dir_all(state_directory.join("session-state/session"))
            .expect("create state directory");
        let workspace_file = workspace.join("src/lib.rs");
        let other_file = root.join("other/secret.txt");
        let state_file = state_directory.join("session-state/session/file.txt");
        assert!(permission_is_automatically_safe(
            &json!({"kind":"write","fileName":workspace_file}),
            &workspace,
            &state_directory,
        ));
        assert!(permission_is_automatically_safe(
            &json!({"kind":"read","path":r"C:\workspace\src\lib.rs"}),
            &workspace,
            &state_directory,
        ));
        assert!(permission_is_automatically_safe(
            &json!({"kind":"write","fileName":"/workspace/src/generated.rs"}),
            &workspace,
            &state_directory,
        ));
        assert!(!permission_is_automatically_safe(
            &json!({"kind":"read","path":r"C:\workspace\..\other\secret.txt"}),
            &workspace,
            &state_directory,
        ));
        assert!(permission_is_automatically_safe(
            &json!({
                "kind":"shell",
                "fullCommandText":"Get-Location",
                "commands":[{"identifier":"powershell","readOnly":false}],
                "possiblePaths":[],
                "possibleUrls":[]
            }),
            &workspace,
            &state_directory,
        ));
        assert!(permission_is_automatically_safe(
            &json!({
                "kind":"shell",
                "fullCommandText":"pwd; Get-ChildItem -Force -Name | Select-Object -First 200",
                "commands":[{"identifier":"powershell","readOnly":false}],
                "commandSegments":[
                    {"identifier":"pwd","fullCommandText":"pwd"},
                    {"identifier":"Get-ChildItem","fullCommandText":"Get-ChildItem -Force -Name"},
                    {"identifier":"Select-Object","fullCommandText":"Select-Object -First 200"}
                ],
                "possiblePaths":[],
                "possibleUrls":[]
            }),
            &workspace,
            &state_directory,
        ));
        assert!(permission_is_automatically_safe(
            &json!({
                "kind":"shell",
                "fullCommandText":"Get-Location; Get-ChildItem -Force | Select-Object Name,Length,LastWriteTime",
                "commands":[{"identifier":"powershell","readOnly":false}],
                "commandSegments":[
                    {"identifier":"Get-Location","fullCommandText":"Get-Location"},
                    {"identifier":"Get-ChildItem","fullCommandText":"Get-ChildItem -Force"},
                    {"identifier":"Select-Object","fullCommandText":"Select-Object Name,Length,LastWriteTime"}
                ],
                "possiblePaths":[],
                "possibleUrls":[]
            }),
            &workspace,
            &state_directory,
        ));
        assert!(!permission_is_automatically_safe(
            &json!({
                "kind":"shell",
                "fullCommandText":"Get-Location; Remove-Item -Recurse .",
                "commands":[{"identifier":"powershell","readOnly":false}],
                "possiblePaths":[],
                "possibleUrls":[]
            }),
            &workspace,
            &state_directory,
        ));
        assert!(!permission_is_automatically_safe(
            &json!({"kind":"write","fileName":other_file}),
            &workspace,
            &state_directory,
        ));
        assert!(!permission_is_automatically_safe(
            &json!({"kind":"url","url":"https://example.com"}),
            &workspace,
            &state_directory,
        ));
        assert!(!permission_is_automatically_safe(
            &json!({"kind":"write","fileName":workspace.join("../other/secret.txt")}),
            &workspace,
            &state_directory,
        ));
        assert!(permission_is_automatically_safe(
            &json!({
                "kind":"shell",
                "commands":[{"identifier":"git","readOnly":true}],
                "possiblePaths":[workspace_file],
                "possibleUrls":[]
            }),
            &workspace,
            &state_directory,
        ));
        assert!(!permission_is_automatically_safe(
            &json!({
                "kind":"shell",
                "commands":[{"identifier":"python","readOnly":false}],
                "possiblePaths":[],
                "possibleUrls":[]
            }),
            &workspace,
            &state_directory,
        ));
        assert!(!permission_is_automatically_safe(
            &json!({
                "kind":"shell",
                "fullCommandText":"node -e \"require('fs').writeFileSync('C:\\\\outside.txt','x')\"",
                "commands":[{"identifier":"node","readOnly":false}],
                "possiblePaths":[],
                "possibleUrls":[]
            }),
            &workspace,
            &state_directory,
        ));
        assert!(!permission_is_automatically_safe(
            &json!({
                "kind":"shell",
                "fullCommandText":"Invoke-WebRequest https://example.com",
                "commands":[{"identifier":"Invoke-WebRequest","readOnly":false}],
                "possiblePaths":[],
                "possibleUrls":["https://example.com"]
            }),
            &workspace,
            &state_directory,
        ));
        assert!(!permission_is_automatically_safe(
            &json!({
                "kind":"shell",
                "fullCommandText":"Write-Output $env:GITHUB_TOKEN",
                "commands":[{"identifier":"Write-Output","readOnly":false}],
                "possiblePaths":[],
                "possibleUrls":[]
            }),
            &workspace,
            &state_directory,
        ));
        assert!(!permission_is_automatically_safe(
            &json!({
                "kind":"shell",
                "fullCommandText":"Get-Content README.md | Set-Content copy.md",
                "commands":[
                    {"identifier":"Get-Content","readOnly":true},
                    {"identifier":"Set-Content","readOnly":false}
                ],
                "possiblePaths":[workspace.join("README.md"), workspace.join("copy.md")],
                "possibleUrls":[],
                "hasWriteFileRedirection":false
            }),
            &workspace,
            &state_directory,
        ));
        assert!(permission_is_automatically_safe(
            &json!({
                "kind":"read",
                "path":state_file
            }),
            &workspace,
            &state_directory,
        ));
        assert!(!permission_is_automatically_safe(
            &json!({
                "kind":"write",
                "fileName":state_directory.join("config.json")
            }),
            &workspace,
            &state_directory,
        ));
        assert!(!permission_is_automatically_safe(
            &json!({
                "kind":"shell",
                "fullCommandText":format!("Get-Content '{}'", other_file.display()),
                "commands":[{"identifier":"Get-Content","readOnly":true}],
                "possiblePaths":[other_file],
                "possibleUrls":[]
            }),
            &workspace,
            &state_directory,
        ));
        let inspect_command = format!(
            "$p = Join-Path -Path '{}' -ChildPath 'copilot-live-proof.txt'; if (Test-Path $p) {{ Get-Content -Raw $p }} else {{ Write-Output '__MISSING__' }}",
            workspace.display()
        );
        assert!(permission_is_automatically_safe(
            &json!({
                "kind":"shell",
                "fullCommandText":inspect_command,
                "commands":[{"identifier":"powershell","readOnly":false}],
                "commandSegments":[
                    {"identifier":"Join-Path","fullCommandText":format!("Join-Path -Path '{}' -ChildPath 'copilot-live-proof.txt'", workspace.display())},
                    {"identifier":"Test-Path","fullCommandText":"Test-Path $p"},
                    {"identifier":"Get-Content","fullCommandText":"Get-Content -Raw $p"},
                    {"identifier":"Write-Output","fullCommandText":"Write-Output '__MISSING__'"}
                ],
                "possiblePaths":[],
                "possibleUrls":[],
                "hasWriteFileRedirection":false
            }),
            &workspace,
            &state_directory,
        ));
        let escaped_command = format!(
            "$p = Join-Path -Path '{}' -ChildPath '../other/secret.txt'; if (Test-Path $p) {{ Get-Content -Raw $p }}",
            workspace.display()
        );
        assert!(!permission_is_automatically_safe(
            &json!({
                "kind":"shell",
                "fullCommandText":escaped_command,
                "commands":[{"identifier":"powershell","readOnly":false}],
                "commandSegments":[
                    {"identifier":"Join-Path","fullCommandText":format!("Join-Path -Path '{}' -ChildPath '../other/secret.txt'", workspace.display())},
                    {"identifier":"Test-Path","fullCommandText":"Test-Path $p"},
                    {"identifier":"Get-Content","fullCommandText":"Get-Content -Raw $p"}
                ],
                "possiblePaths":[],
                "possibleUrls":[],
                "hasWriteFileRedirection":false
            }),
            &workspace,
            &state_directory,
        ));
        std::fs::remove_dir_all(root).expect("remove permission test root");
    }

    #[tokio::test]
    async fn fixture_runs_selected_model_and_emits_evidence() {
        let run_id = Uuid::new_v4();
        let workspace = std::env::temp_dir()
            .join("crony-copilot-adapter-tests")
            .join(run_id.to_string());
        let adapter = CopilotSdkAdapter::new(CopilotSdkConfig {
            cli_path: None,
            cli_prefix_args: Vec::new(),
            external_host: None,
            external_port: None,
            github_token_file: None,
            connection_token_file: None,
            base_directory: workspace.join("state"),
            use_logged_in_user: false,
            log_level: "warning".to_owned(),
            fixture: true,
        });
        let request = AdapterRunRequest {
            run_id,
            mission_id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            agent_id: Uuid::new_v4(),
            mission_title: "fixture Copilot task".to_owned(),
            model: Some("copilot-test-reasoning".to_owned()),
            reasoning_effort: Some("high".to_owned()),
            workspace: workspace.clone(),
            environment: HashMap::new(),
        };
        let (_control_tx, control_rx) = mpsc::unbounded_channel();
        let sink = Arc::new(RecordingSink::default());
        let exit = adapter
            .execute(request, control_rx, sink.clone())
            .await
            .expect("fixture execute");
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
        drop(events);
        assert!(workspace.join("copilot-evidence.json").is_file());
        let _ = std::fs::remove_dir_all(workspace);
    }
}
