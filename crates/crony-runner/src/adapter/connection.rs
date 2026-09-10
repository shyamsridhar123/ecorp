//! Native account setup is a runner-private control operation, never an agent
//! turn. The manager owns actor/Corp/source fencing, protected directory ACLs,
//! durable operation adoption, and acknowledgement before dispatch.
mod catalog;
mod claude;
mod codex;
mod copilot;
mod environment;
mod process;
#[cfg(test)]
mod tests;

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Result, bail};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use crony_domain::{
    CodingAgent, NativeSignInInstruction, NativeSignInResponse, RunnerModel,
    WorkspaceConnectionStatus,
};
use tokio::sync::{Mutex, mpsc};
use uuid::Uuid;

use super::{
    AdapterCapabilities, AdapterControl, AdapterError, AdapterEventSink, AdapterExit, AdapterModel,
    AdapterRegistryConfig, AdapterRunRequest, AgentAdapter, CodexAdapter, CopilotSdkAdapter,
    ExternalCliAdapter, ExternalFlavor, UsageSnapshot,
};
use environment::NativeCommand;
pub(crate) use environment::ProfileEnvironment;

const INSPECT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_SIGN_IN_DURATION: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone)]
pub(crate) struct AgentReadiness {
    pub status: WorkspaceConnectionStatus,
    pub detail: String,
    pub models: Vec<RunnerModel>,
    pub account_login: Option<String>,
}

impl AgentReadiness {
    fn state(status: WorkspaceConnectionStatus, detail: impl Into<String>) -> Self {
        Self {
            status,
            detail: detail.into(),
            models: Vec::new(),
            account_login: None,
        }
    }

    fn ready(mut models: Vec<RunnerModel>, account_login: Option<String>) -> ProbeResult<Self> {
        // A native catalog can include policy-disabled metadata. Only advertise
        // selectable entries: absent/unconfigured policy is not a denial, but
        // disabled or unrecognized native policy must never establish Ready.
        models.retain(|model| {
            matches!(
                model.policy_state.as_deref(),
                None | Some("enabled" | "unconfigured")
            )
        });
        if models.is_empty() {
            return Err(ProbeError::Incompatible(
                "Native authentication is present, but no usable model catalog was returned.",
            ));
        }
        Ok(Self {
            status: WorkspaceConnectionStatus::Ready,
            detail: "Native account and model catalog checked; inference has not been tested."
                .to_owned(),
            models,
            account_login,
        })
    }
}

/// One transient, operation-owned input channel. Only a native Claude
/// authorization-code prompt may arm it; nothing is persisted or sent to a model.
pub(crate) struct NativeCodeReceiver {
    pub receiver: mpsc::Receiver<NativeSignInResponse>,
    pub waiting: Arc<AtomicBool>,
    pub prompt_id: Arc<std::sync::Mutex<Option<Uuid>>>,
}

impl NativeCodeReceiver {
    fn clear_prompt(&self) {
        self.waiting.store(false, Ordering::Release);
        *self
            .prompt_id
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
    }
}

impl Drop for NativeCodeReceiver {
    fn drop(&mut self) {
        self.clear_prompt();
    }
}

#[derive(Debug, thiserror::Error)]
enum ProbeError {
    #[error("Native agent is not installed at its configured location.")]
    NotInstalled,
    #[error("{0}")]
    Incompatible(&'static str),
    #[error("Native account service is unavailable; no readiness was established.")]
    Offline,
    #[error("Native operation exceeded its deadline; no readiness was established.")]
    Timeout,
    #[error("{0}")]
    Failed(&'static str),
}

type ProbeResult<T> = std::result::Result<T, ProbeError>;

impl ProbeError {
    fn io(error: std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::NotFound => Self::NotInstalled,
            std::io::ErrorKind::Unsupported => Self::Incompatible(
                "This platform does not provide ECorp's required native process ownership.",
            ),
            _ => Self::Offline,
        }
    }

    fn readiness(self) -> AgentReadiness {
        let status = match self {
            Self::NotInstalled => WorkspaceConnectionStatus::NotInstalled,
            Self::Incompatible(_) => WorkspaceConnectionStatus::Incompatible,
            Self::Offline | Self::Timeout => WorkspaceConnectionStatus::Offline,
            Self::Failed(_) => WorkspaceConnectionStatus::Failed,
        };
        AgentReadiness::state(status, self.to_string())
    }
}

#[derive(Clone)]
enum Backend {
    Codex(NativeCommand, Arc<CodexAdapter>),
    Claude(NativeCommand, Arc<ExternalCliAdapter>),
    Copilot(Arc<CopilotSdkAdapter>),
}

#[derive(Clone)]
pub(crate) struct AgentProfile {
    scope: ProfileEnvironment,
    workspace: PathBuf,
    backend: Backend,
    operation: Arc<Mutex<()>>,
}

impl AgentProfile {
    /// Synchronous and side-effect free: no process, version probe, auth read,
    /// directory creation, SDK extraction, model request, or credential mutation.
    /// `home` is a manager-protected root outside ALL source and task worktrees.
    /// Its `native` subdirectory holds native auth; `session-fs` is separate.
    pub(crate) fn new(
        config: AdapterRegistryConfig,
        agent: CodingAgent,
        home: PathBuf,
        workspace: PathBuf,
        use_system_installation: bool,
    ) -> Result<Self> {
        let scope = ProfileEnvironment::new(agent, home, use_system_installation)?;
        scope.validate_workspace(&workspace)?;
        let backend = match agent {
            CodingAgent::Codex => {
                let native =
                    NativeCommand::configured(config.codex_command, config.codex_prefix_args)?;
                let adapter = CodexAdapter::for_connection(
                    native.program.clone(),
                    native.prefix.clone(),
                    scope.clone(),
                );
                Backend::Codex(native, Arc::new(adapter))
            }
            CodingAgent::ClaudeCode => {
                let native =
                    NativeCommand::configured(config.claude_command, config.claude_prefix_args)?;
                let adapter = ExternalCliAdapter::for_connection(
                    ExternalFlavor::ClaudeCode,
                    native.program.clone(),
                    native.prefix.clone(),
                    scope.clone(),
                );
                Backend::Claude(native, Arc::new(adapter))
            }
            CodingAgent::GitHubCopilot => {
                if config.copilot.fixture {
                    bail!("a deterministic Copilot fixture cannot establish a native connection");
                }
                if !use_system_installation && config.copilot.external_host.is_some() {
                    bail!(
                        "a personal connection requires a local, separately scoped Copilot runtime"
                    );
                }
                if let Some(path) = &config.copilot.cli_path
                    && !path.is_absolute()
                {
                    bail!(
                        "Copilot requires an explicit absolute runtime path or the pinned SDK bundle"
                    );
                }
                if config.copilot.cli_path.is_none() && !config.copilot.cli_prefix_args.is_empty() {
                    bail!("Copilot prefix arguments require an explicitly configured runtime");
                }
                Backend::Copilot(Arc::new(CopilotSdkAdapter::for_connection(
                    config.copilot,
                    scope.clone(),
                )))
            }
        };
        Ok(Self {
            scope,
            workspace,
            backend,
            operation: Arc::new(Mutex::new(())),
        })
    }

    pub(crate) fn adapter(&self) -> Arc<dyn AgentAdapter> {
        Arc::new(ProfileAdapter(self.clone()))
    }

    fn inner_adapter(&self) -> Arc<dyn AgentAdapter> {
        match &self.backend {
            Backend::Codex(_, adapter) => adapter.clone(),
            Backend::Claude(_, adapter) => adapter.clone(),
            Backend::Copilot(adapter) => adapter.clone(),
        }
    }

    pub(crate) async fn inspect(&self) -> Result<AgentReadiness> {
        self.inspect_before(Utc::now() + chrono::Duration::seconds(30))
            .await
    }

    pub(crate) async fn inspect_before(&self, expires_at: DateTime<Utc>) -> Result<AgentReadiness> {
        let remaining = (expires_at - Utc::now())
            .to_std()
            .map_err(|_| anyhow::anyhow!("native inspection operation has expired"))?;
        if remaining.is_zero() {
            bail!("native inspection operation has expired");
        }
        let deadline = tokio::time::Instant::now() + remaining.min(INSPECT_TIMEOUT);
        let Ok(_operation) = self.operation.try_lock() else {
            return Ok(AgentReadiness::state(
                WorkspaceConnectionStatus::Connecting,
                "Another native account operation is still active.",
            ));
        };
        self.scope.prepare(&self.workspace).await?;
        let result = self.inspect_native(deadline).await;
        // Each inspector awaits cleanup even after its response deadline.
        if Utc::now() >= expires_at {
            return Ok(ProbeError::Timeout.readiness());
        }
        Ok(result.unwrap_or_else(ProbeError::readiness))
    }

    async fn inspect_native(&self, deadline: tokio::time::Instant) -> ProbeResult<AgentReadiness> {
        if tokio::time::Instant::now() >= deadline {
            return Err(ProbeError::Timeout);
        }
        match &self.backend {
            Backend::Codex(command, _) => {
                codex::inspect(command, &self.scope, &self.workspace, deadline).await
            }
            Backend::Claude(command, _) => {
                claude::inspect(command, &self.scope, &self.workspace, deadline).await
            }
            Backend::Copilot(adapter) => copilot::inspect(adapter, &self.workspace, deadline).await,
        }
    }

    pub(crate) async fn sign_in(
        &self,
        expires_at: DateTime<Utc>,
        progress: Arc<dyn Fn(NativeSignInInstruction) + Send + Sync>,
        mut input: NativeCodeReceiver,
    ) -> Result<AgentReadiness> {
        input.clear_prompt();
        if self.scope.system {
            bail!(
                "system-installation connections are read-only; personal sign-in requires an isolated profile"
            );
        }
        let _operation = self
            .operation
            .try_lock()
            .map_err(|_| anyhow::anyhow!("another native account operation is still active"))?;
        let remaining = (expires_at - Utc::now())
            .to_std()
            .map_err(|_| anyhow::anyhow!("native sign-in operation has expired"))?;
        if remaining.is_zero() {
            bail!("native sign-in operation has expired");
        }
        let duration = remaining.min(MAX_SIGN_IN_DURATION);
        let deadline = tokio::time::Instant::now() + duration;
        let instruction_expiry = expires_at.min(
            Utc::now()
                + chrono::Duration::from_std(duration)
                    .map_err(|_| anyhow::anyhow!("invalid native sign-in expiry"))?,
        );
        self.scope.prepare(&self.workspace).await?;
        // Adopt an already-authenticated profile rather than restart login.
        // An authenticated but offline/incompatible catalog is NOT a reason to
        // replace its account or launch another authentication flow.
        let before = self
            .inspect_native(deadline.min(tokio::time::Instant::now() + INSPECT_TIMEOUT))
            .await
            .unwrap_or_else(ProbeError::readiness);
        if before.status != WorkspaceConnectionStatus::NeedsSignIn {
            return Ok(before);
        }
        let result = match &self.backend {
            Backend::Codex(command, _) => {
                codex::sign_in(
                    command,
                    &self.scope,
                    &self.workspace,
                    deadline,
                    instruction_expiry,
                    &progress,
                )
                .await
            }
            Backend::Claude(command, _) => {
                claude::sign_in(
                    command,
                    &self.scope,
                    &self.workspace,
                    deadline,
                    instruction_expiry,
                    &progress,
                    &mut input,
                )
                .await
            }
            Backend::Copilot(adapter) => {
                copilot::sign_in(
                    adapter,
                    &self.scope,
                    &self.workspace,
                    deadline,
                    instruction_expiry,
                    &progress,
                )
                .await
            }
        };
        input.clear_prompt();
        match result {
            Ok(true) => {
                // A URL, code, or successful CLI exit is not proof of auth or
                // inference. A new native status/catalog check establishes Ready.
                let readiness = self
                    .inspect_native(deadline.min(tokio::time::Instant::now() + INSPECT_TIMEOUT))
                    .await
                    .unwrap_or_else(ProbeError::readiness);
                if tokio::time::Instant::now() >= deadline {
                    Ok(ProbeError::Timeout.readiness())
                } else {
                    Ok(readiness)
                }
            }
            Ok(false) | Err(ProbeError::Timeout) => Ok(AgentReadiness::state(
                WorkspaceConnectionStatus::NeedsSignIn,
                "Native sign-in did not complete before the operation ended.",
            )),
            Err(error) => Ok(error.readiness()),
        }
    }
}

struct ProfileAdapter(AgentProfile);

#[async_trait]
impl AgentAdapter for ProfileAdapter {
    fn id(&self) -> &'static str {
        self.0.scope.agent.as_str()
    }

    fn display_name(&self) -> &'static str {
        self.0.scope.agent.label()
    }

    fn capabilities(&self) -> AdapterCapabilities {
        self.0.inner_adapter().capabilities()
    }

    async fn list_models(&self) -> std::result::Result<Vec<AdapterModel>, AdapterError> {
        let readiness = self.0.inspect().await?;
        if readiness.status != WorkspaceConnectionStatus::Ready {
            return Err(AdapterError::Runtime(anyhow::anyhow!(readiness.detail)));
        }
        Ok(readiness
            .models
            .into_iter()
            .map(catalog::adapter_model)
            .collect())
    }

    async fn execute(
        &self,
        request: AdapterRunRequest,
        controls: mpsc::UnboundedReceiver<AdapterControl>,
        sink: Arc<dyn AdapterEventSink>,
    ) -> std::result::Result<AdapterExit, AdapterError> {
        self.0.scope.prepare(&request.workspace).await?;
        self.0.scope.validate_request(&request.environment)?;
        self.0
            .inner_adapter()
            .execute(request, controls, sink)
            .await
    }

    async fn resume(
        &self,
        request: AdapterRunRequest,
        session_id: &str,
        controls: mpsc::UnboundedReceiver<AdapterControl>,
        sink: Arc<dyn AdapterEventSink>,
    ) -> std::result::Result<AdapterExit, AdapterError> {
        self.0.scope.prepare(&request.workspace).await?;
        self.0.scope.validate_request(&request.environment)?;
        self.0
            .inner_adapter()
            .resume(request, session_id, controls, sink)
            .await
    }

    async fn collect_usage(
        &self,
        session_id: &str,
    ) -> std::result::Result<UsageSnapshot, AdapterError> {
        self.0.inner_adapter().collect_usage(session_id).await
    }
}
