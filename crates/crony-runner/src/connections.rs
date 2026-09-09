//! Runner-owned native setup, durable side-effect adoption and accepted source pins.
//! The server owns user authorization/epochs. This boundary never accepts commands,
//! tokens or arbitrary environment variables from setup payloads.
#[path = "connection_setup/github.rs"]
mod github;
#[path = "connection_setup/process.rs"]
mod process;
#[path = "connection_setup/storage.rs"]
mod storage;

use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use crony_domain::{
    CodingAgent, GitHubAccountSource, NativeSignInInputKind, NativeSignInInstruction,
    NativeSignInResponse, RunnerModel, WorkspaceConnectionConfiguration, WorkspaceConnectionStatus,
    WorkspaceRepositorySetup, WorkspaceSetupAction, WorkspaceSetupCommand, WorkspaceSetupReport,
    WorkspaceSetupStatus, WorkspaceSourceIdentity,
};
use crony_protocol::RunnerCapability;
use dashmap::DashMap;
use futures_util::FutureExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex as AsyncMutex, mpsc, watch};
use uuid::Uuid;

use crate::{
    adapter::{
        AdapterRegistry, AdapterRegistryConfig, AgentAdapter,
        connection::{AgentProfile, AgentReadiness, NativeCodeReceiver},
    },
    workspace::WorkspaceManager,
};
use github::{NativeGitHub, valid_commit, validate_ref};
use storage::{PrivateRoot, canonical_directory, reject_links, reject_user_path};

const REGISTRY_FILE: &str = "registry.json";
const REGISTRY_SCHEMA: u32 = 1;
const MAX_CONNECTIONS: usize = 128;
const MAX_OPERATIONS: usize = 2048;
const MAX_SNAPSHOTS: usize = 128;

pub type SetupProgress = Arc<dyn Fn(WorkspaceSetupReport) + Send + Sync>;

pub struct ConnectionManagerConfig {
    pub root: PathBuf,
    pub corp_id: Uuid,
    pub runner_id: String,
    pub default_workspaces: Arc<WorkspaceManager>,
    pub default_adapters: Arc<AdapterRegistry>,
    pub adapter_config: AdapterRegistryConfig,
    pub github_command: PathBuf,
    /// Operator-owned configuration, never a setup request's claimed filesystem scope.
    pub allowed_local_roots: Vec<PathBuf>,
}

#[derive(Clone)]
pub struct ConnectionRuntime {
    pub workspaces: Arc<WorkspaceManager>,
    pub adapter: Arc<dyn AgentAdapter>,
}

#[derive(Clone)]
pub struct ConnectionManager {
    inner: Arc<Inner>,
}

struct Inner {
    root: PrivateRoot,
    corp_id: Uuid,
    runner_id: String,
    default_workspaces: Arc<WorkspaceManager>,
    default_adapters: Arc<AdapterRegistry>,
    adapter_config: AdapterRegistryConfig,
    github_command: PathBuf,
    allowed_local_roots: Vec<PathBuf>,
    state: Mutex<Registry>,
    flights: Mutex<HashMap<Uuid, Arc<Flight>>>,
    gates: DashMap<Uuid, Arc<AsyncMutex<()>>>,
    workspaces: AsyncMutex<HashMap<Uuid, Arc<WorkspaceManager>>>,
    runtimes: AsyncMutex<HashMap<String, ConnectionRuntime>>,
}

struct Flight {
    done: watch::Sender<Option<WorkspaceSetupReport>>,
    recent: Mutex<Option<WorkspaceSetupReport>>,
    observer: Mutex<Option<SetupProgress>>,
    code_sender: mpsc::Sender<NativeSignInResponse>,
    code_waiting: Arc<AtomicBool>,
    code_prompt_id: Arc<Mutex<Option<Uuid>>>,
    submitted_codes: Mutex<HashMap<Uuid, [u8; 32]>>,
    claude_sign_in: bool,
    expires_at: DateTime<Utc>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    schema_version: u32,
    corp_id: Uuid,
    runner_id: String,
    github_accounts: BTreeMap<Uuid, String>,
    connections: BTreeMap<Uuid, SavedConnection>,
    operations: BTreeMap<Uuid, SavedOperation>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedConnection {
    owner_id: Uuid,
    room_id: Uuid,
    version: i64,
    configuration: WorkspaceConnectionConfiguration,
    verified_repository_id: Option<String>,
    current_operation: Uuid,
    active_snapshot: Option<String>,
    source_snapshot: Option<String>,
    snapshots: BTreeMap<String, Snapshot>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    source: WorkspaceSourceIdentity,
    models: Vec<RunnerModel>,
    agent: CodingAgent,
    use_system_installation: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    use_machine_account: Option<bool>,
    advertised: bool,
    source_accepted: bool,
    accepted: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedOperation {
    command: WorkspaceSetupCommand,
    /// Set before invoking native login. Restart/replay may inspect/adopt it, never relogin.
    login_started: bool,
    report: Option<WorkspaceSetupReport>,
    candidate: Option<String>,
    acknowledged: Option<bool>,
}

struct PreparedSource {
    source: WorkspaceSourceIdentity,
    workspaces: Arc<WorkspaceManager>,
    advertised: bool,
}

struct SetupResult {
    report: WorkspaceSetupReport,
    snapshot: Option<Snapshot>,
}

impl ConnectionManager {
    pub async fn new(config: ConnectionManagerConfig) -> Result<Self> {
        if config.corp_id.is_nil() || config.runner_id.trim().is_empty() {
            return Err(anyhow!(
                "connection manager requires a Corp and enrolled runner identity"
            ));
        }
        let source_checkout = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .context("runner source checkout is unavailable")?;
        let root = PrivateRoot::open(
            config.root,
            &[
                source_checkout.to_owned(),
                config.default_workspaces.repository().to_owned(),
            ],
        )
        .await?;
        let mut allowed_local_roots = Vec::new();
        let roots = if config.allowed_local_roots.is_empty() {
            vec![config.default_workspaces.repository().to_owned()]
        } else {
            config.allowed_local_roots
        };
        for path in roots {
            if !path.is_absolute() {
                return Err(anyhow!("operator repository roots must be absolute"));
            }
            let path =
                canonical_directory(&path).context("operator repository root is unavailable")?;
            if !allowed_local_roots.contains(&path) {
                allowed_local_roots.push(path);
            }
        }
        let state = root.read::<Registry>(REGISTRY_FILE)?.unwrap_or(Registry {
            schema_version: REGISTRY_SCHEMA,
            corp_id: config.corp_id,
            runner_id: config.runner_id.clone(),
            github_accounts: BTreeMap::new(),
            connections: BTreeMap::new(),
            operations: BTreeMap::new(),
        });
        validate_registry(&state, config.corp_id, &config.runner_id)?;
        root.write(REGISTRY_FILE, &state)?;
        Ok(Self {
            inner: Arc::new(Inner {
                root,
                corp_id: config.corp_id,
                runner_id: config.runner_id,
                default_workspaces: config.default_workspaces,
                default_adapters: config.default_adapters,
                adapter_config: config.adapter_config,
                github_command: config.github_command,
                allowed_local_roots,
                state: Mutex::new(state),
                flights: Mutex::new(HashMap::new()),
                gates: DashMap::new(),
                workspaces: AsyncMutex::new(HashMap::new()),
                runtimes: AsyncMutex::new(HashMap::new()),
            }),
        })
    }

    /// Disconnecting an observer does not terminate setup. Exact duplicate commands
    /// adopt the same flight/receipt; a changed payload under its UUID is rejected.
    pub async fn execute(
        &self,
        command: WorkspaceSetupCommand,
        progress: SetupProgress,
    ) -> Result<WorkspaceSetupReport> {
        self.validate_command(&command)?;
        let (flight, receiver, start) = {
            let mut flights = self
                .inner
                .flights
                .lock()
                .map_err(|_| anyhow!("setup registry unavailable"))?;
            {
                let state = self
                    .inner
                    .state
                    .lock()
                    .map_err(|_| anyhow!("setup registry unavailable"))?;
                if let Some(existing) = state.operations.get(&command.operation_id) {
                    require_same_command(&existing.command, &command)?;
                    if let Some(report) = &existing.report {
                        return Ok(report.clone());
                    }
                }
                if let Some(id) = command.action.connection_id()
                    && let Some(connection) = state.connections.get(&id)
                    && connection.current_operation != command.operation_id
                    && flights.contains_key(&connection.current_operation)
                {
                    return Err(anyhow!(
                        "another native setup operation is still active for this connection"
                    ));
                }
            }
            if let Some(flight) = flights.get(&command.operation_id) {
                let receiver = flight.done.subscribe();
                (flight.clone(), receiver, None)
            } else {
                if command.expires_at <= Utc::now() {
                    return Err(anyhow!("native setup operation expired"));
                }
                self.update(|state| begin_operation(state, &command))?;
                let (done, receiver) = watch::channel(None);
                let (code_sender, code_receiver) = mpsc::channel(1);
                let flight = Arc::new(Flight {
                    done,
                    recent: Mutex::new(None),
                    observer: Mutex::new(Some(progress.clone())),
                    code_sender,
                    code_waiting: Arc::new(AtomicBool::new(false)),
                    code_prompt_id: Arc::new(Mutex::new(None)),
                    submitted_codes: Mutex::new(HashMap::new()),
                    claude_sign_in: matches!(&command.action,
                        WorkspaceSetupAction::SignInAgent { configuration, .. }
                        if configuration.agent == CodingAgent::ClaudeCode
                            && !configuration.use_machine_account.unwrap_or(configuration.use_system_installation)),
                    expires_at: command.expires_at,
                });
                flights.insert(command.operation_id, flight.clone());
                (flight, receiver, Some(code_receiver))
            }
        };
        if let Ok(mut observer) = flight.observer.lock() {
            *observer = Some(progress.clone());
        }
        if let Some(recent) = flight.recent.lock().ok().and_then(|value| value.clone()) {
            progress(recent);
        }
        if let Some(code_receiver) = start {
            let manager = self.clone();
            let flight = flight.clone();
            let update_flight = flight.clone();
            let callback: SetupProgress = Arc::new(move |report| {
                if let Ok(mut recent) = update_flight.recent.lock() {
                    *recent = Some(report.clone());
                }
                let observer = update_flight
                    .observer
                    .lock()
                    .ok()
                    .and_then(|value| value.clone());
                if let Some(observer) = observer {
                    observer(report);
                }
            });
            tokio::spawn(async move {
                let operation_id = command.operation_id;
                let input = NativeCodeReceiver {
                    receiver: code_receiver,
                    waiting: flight.code_waiting.clone(),
                    prompt_id: flight.code_prompt_id.clone(),
                };
                let result =
                    std::panic::AssertUnwindSafe(manager.perform(&command, callback, input))
                        .catch_unwind()
                        .await;
                let result = match result {
                    Ok(Ok(result)) => result,
                    _ => SetupResult {
                        report: report(
                            WorkspaceSetupStatus::Failed,
                            Some(WorkspaceConnectionStatus::Failed),
                            "Native setup could not complete safely; no new dispatch authority was granted.",
                        ),
                        snapshot: None,
                    },
                };
                flight.code_waiting.store(false, Ordering::Release);
                let mut final_report = manager.finish(&command, result).unwrap_or_else(|_| {
                    report(WorkspaceSetupStatus::Failed, Some(WorkspaceConnectionStatus::Failed),
                        "Native setup receipt could not be persisted; no new dispatch authority was granted.")
                });
                // Sign-in instructions and submitted codes are never journaled.
                final_report.sign_in = None;
                let _ = flight.done.send(Some(final_report));
                if let Ok(mut flights) = manager.inner.flights.lock() {
                    flights.remove(&operation_id);
                }
            });
        }
        let mut receiver = receiver;
        loop {
            if let Some(report) = receiver.borrow().clone() {
                return Ok(report);
            }
            receiver
                .changed()
                .await
                .context("native setup observer disconnected")?;
        }
    }

    /// Only a current Claude native-login prompt may receive this transient code.
    pub async fn submit_sign_in(
        &self,
        operation_id: Uuid,
        response: NativeSignInResponse,
    ) -> Result<()> {
        if response.input_id.is_nil()
            || response.authorization_code.is_empty()
            || response.authorization_code.len() > 4096
            || response.authorization_code.chars().any(char::is_control)
        {
            return Err(anyhow!("invalid native authorization code"));
        }
        let flight = self
            .inner
            .flights
            .lock()
            .map_err(|_| anyhow!("setup registry unavailable"))?
            .get(&operation_id)
            .cloned()
            .context("no matching native sign-in is waiting")?;
        {
            let state = self
                .inner
                .state
                .lock()
                .map_err(|_| anyhow!("setup registry unavailable"))?;
            let operation = state
                .operations
                .get(&operation_id)
                .context("native sign-in operation is missing")?;
            let id = operation
                .command
                .action
                .connection_id()
                .context("native sign-in has no connection")?;
            let connection = state
                .connections
                .get(&id)
                .context("native sign-in connection is missing")?;
            if operation.report.is_some()
                || Some(connection.version) != operation.command.expected_connection_version
                || connection.current_operation != operation_id
            {
                return Err(anyhow!("native sign-in operation is no longer current"));
            }
        }
        submit_code(&flight, response)
    }

    /// Only the server's accepted Ready result can publish a new dispatchable pin.
    pub async fn acknowledge(&self, operation_id: Uuid, accepted: bool) -> Result<()> {
        self.update(|state| acknowledge(state, operation_id, accepted))
    }

    /// Replayed on reconnect even when the server committed a result but its
    /// acknowledgement was lost. Sign-in instructions are not persisted here.
    pub fn pending_reports(&self) -> Vec<(Uuid, WorkspaceSetupReport)> {
        let Ok(state) = self.inner.state.lock() else {
            return Vec::new();
        };
        state
            .operations
            .iter()
            .filter_map(|(id, operation)| {
                if operation.acknowledged.is_some() {
                    return None;
                }
                operation
                    .report
                    .as_ref()
                    .map(|report| (*id, report.clone()))
            })
            .collect()
    }

    pub fn capabilities(&self) -> Vec<RunnerCapability> {
        let Ok(state) = self.inner.state.lock() else {
            return Vec::new();
        };
        let mut capabilities = Vec::new();
        for (id, connection) in &state.connections {
            if let Some(snapshot) = connection
                .source_snapshot
                .as_ref()
                .and_then(|key| connection.snapshots.get(key))
                && snapshot.source_accepted
            {
                capabilities.push(snapshot_capability(*id, snapshot, false));
            }
            if let Some(snapshot) = connection
                .active_snapshot
                .as_ref()
                .and_then(|key| connection.snapshots.get(key))
                && snapshot.accepted
            {
                capabilities.push(snapshot_capability(*id, snapshot, true));
            }
        }
        capabilities
    }

    pub async fn resolve(
        &self,
        connection_id: Uuid,
        expected_source: &WorkspaceSourceIdentity,
        adapter: &str,
    ) -> Result<ConnectionRuntime> {
        validate_source(expected_source)?;
        let (owner, snapshot) = {
            let state = self
                .inner
                .state
                .lock()
                .map_err(|_| anyhow!("setup registry unavailable"))?;
            let connection = state
                .connections
                .get(&connection_id)
                .context("saved connection is not known on this runner")?;
            if !connection
                .active_snapshot
                .as_ref()
                .and_then(|key| connection.snapshots.get(key))
                .is_some_and(|snapshot| snapshot.accepted && snapshot.agent.as_str() == adapter)
            {
                return Err(anyhow!(
                    "saved connection has no currently accepted provider readiness"
                ));
            }
            let snapshot = connection
                .snapshots
                .values()
                .find(|snapshot| {
                    snapshot.accepted
                        && snapshot.agent.as_str() == adapter
                        && source_matches(&snapshot.source, expected_source)
                })
                .cloned()
                .context("source/agent has no server-accepted connection snapshot")?;
            (connection.owner_id, snapshot)
        };
        let key = format!("{connection_id}:{}", snapshot_key(&snapshot)?);
        let mut runtimes = self.inner.runtimes.lock().await;
        if let Some(runtime) = runtimes.get(&key) {
            return Ok(runtime.clone());
        }
        let workspaces = self
            .workspace_snapshot(connection_id, owner, &snapshot)
            .await?;
        let profile = self.agent_profile(
            connection_id,
            owner,
            snapshot.agent,
            snapshot.use_system_installation,
            snapshot.use_machine_account,
        )?;
        let runtime = ConnectionRuntime {
            workspaces,
            adapter: profile.adapter(),
        };
        runtimes.insert(key, runtime.clone());
        Ok(runtime)
    }

    /// Source-only recovery never performs an agent auth/catalog check or starts
    /// a provider. A later failed login cannot destroy an accepted worktree pin.
    pub async fn resolve_workspace(
        &self,
        connection_id: Uuid,
        expected_source: &WorkspaceSourceIdentity,
    ) -> Result<Arc<WorkspaceManager>> {
        validate_source(expected_source)?;
        let (owner, snapshot) = {
            let state = self
                .inner
                .state
                .lock()
                .map_err(|_| anyhow!("setup registry unavailable"))?;
            let connection = state
                .connections
                .get(&connection_id)
                .context("saved connection is not known on this runner")?;
            let snapshot = connection
                .snapshots
                .values()
                .find(|snapshot| {
                    snapshot.source_accepted && source_matches(&snapshot.source, expected_source)
                })
                .cloned()
                .context("source has no server-accepted workspace snapshot")?;
            (connection.owner_id, snapshot)
        };
        self.workspace_snapshot(connection_id, owner, &snapshot)
            .await
    }

    fn validate_command(&self, command: &WorkspaceSetupCommand) -> Result<()> {
        if command.operation_id.is_nil()
            || command.actor_id.is_nil()
            || command.room_id.is_nil()
            || command.corp_id != self.inner.corp_id
            || command.runner_id != self.inner.runner_id
        {
            return Err(anyhow!(
                "native setup command does not belong to this runner and Corp"
            ));
        }
        if command.action.connection_id().is_some()
            && (command.connection_owner_id.is_none_or(|id| id.is_nil())
                || command
                    .expected_connection_version
                    .is_none_or(|version| version <= 0))
        {
            return Err(anyhow!(
                "connection setup omitted its owner or current version"
            ));
        }
        if let Some(configuration) = configuration(&command.action) {
            match &configuration.repository {
                WorkspaceRepositorySetup::GitHub {
                    repository,
                    repository_id,
                    base_ref,
                    ..
                } => {
                    github::validate_repository(repository)?;
                    validate_ref(base_ref)?;
                    if repository_id
                        .as_ref()
                        .is_some_and(|id| id.is_empty() || id.len() > 200)
                    {
                        return Err(anyhow!("invalid immutable GitHub repository identity"));
                    }
                }
                WorkspaceRepositorySetup::Local {
                    directory,
                    base_ref,
                } => {
                    reject_user_path(Path::new(directory))?;
                    validate_ref(base_ref)?;
                }
                WorkspaceRepositorySetup::Advertised { source } => validate_source(source)?,
            }
        }
        Ok(())
    }

    fn update<T>(&self, change: impl FnOnce(&mut Registry) -> Result<T>) -> Result<T> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| anyhow!("setup registry unavailable"))?;
        let mut next = state.clone();
        let result = change(&mut next)?;
        self.inner.root.write(REGISTRY_FILE, &next)?;
        *state = next;
        Ok(result)
    }

    fn finish(
        &self,
        command: &WorkspaceSetupCommand,
        result: SetupResult,
    ) -> Result<WorkspaceSetupReport> {
        self.update(|state| {
            let mut candidate = None;
            let mut final_report = result.report;
            if let Some(snapshot) = result.snapshot {
                let id = command.action.connection_id().context("setup snapshot omitted connection")?;
                let connection = state.connections.get_mut(&id).context("setup connection disappeared")?;
                if Some(connection.version) != command.expected_connection_version
                    || connection.current_operation != command.operation_id
                {
                    final_report = report(WorkspaceSetupStatus::Failed, Some(WorkspaceConnectionStatus::Failed),
                        "This setup operation was superseded; no new dispatch authority was granted.");
                } else {
                    let key = snapshot_key(&snapshot)?;
                    let mut snapshot = snapshot;
                    if let Some(previous) = connection.snapshots.get(&key) {
                        snapshot.accepted = previous.accepted;
                        snapshot.source_accepted = previous.source_accepted;
                    } else if connection.snapshots.len() >= MAX_SNAPSHOTS {
                        return Err(anyhow!("connection snapshot retention bound reached"));
                    }
                    connection.snapshots.insert(key.clone(), snapshot);
                    candidate = Some(key);
                }
            }
            let operation = state.operations.get_mut(&command.operation_id).context("setup operation disappeared")?;
            final_report.sign_in = None;
            operation.report = Some(final_report.clone());
            operation.candidate = candidate;
            Ok(final_report)
        })
    }

    async fn perform(
        &self,
        command: &WorkspaceSetupCommand,
        progress: SetupProgress,
        input: NativeCodeReceiver,
    ) -> Result<SetupResult> {
        let lane = command.action.connection_id().unwrap_or(command.actor_id);
        let gate = self
            .inner
            .gates
            .entry(lane)
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();
        let _guard = gate.lock().await;
        if command.expires_at <= Utc::now() {
            return Err(anyhow!("setup operation expired"));
        }
        if let Some(id) = command.action.connection_id() {
            let state = self
                .inner
                .state
                .lock()
                .map_err(|_| anyhow!("setup registry unavailable"))?;
            let current = state
                .connections
                .get(&id)
                .context("setup connection is missing")?;
            if current.current_operation != command.operation_id
                || Some(current.version) != command.expected_connection_version
            {
                return Err(anyhow!("native setup operation is no longer current"));
            }
        }
        progress(report(
            WorkspaceSetupStatus::Running,
            Some(WorkspaceConnectionStatus::Connecting),
            "Checking the selected native account and source on this runner.",
        ));
        match &command.action {
            WorkspaceSetupAction::InspectGitHub { account }
            | WorkspaceSetupAction::ListGitHubRepositories { account } => {
                let github = self.github(*account, command.actor_id)?;
                let Some(identity) = github.inspect(command.expires_at).await? else {
                    return Ok(SetupResult {
                        report: needs_sign_in("Sign in to the selected native GitHub account."),
                        snapshot: None,
                    });
                };
                self.bind_github_identity(*account, command.actor_id, &identity.login)?;
                let mut result = report(
                    WorkspaceSetupStatus::Succeeded,
                    None,
                    identity.storage.detail(),
                );
                result.account_login = Some(identity.login);
                if matches!(
                    command.action,
                    WorkspaceSetupAction::ListGitHubRepositories { .. }
                ) {
                    result.repositories = github.list(command.expires_at).await?;
                    result.detail = format!(
                        "Up to 100 recently updated accessible repositories. {}",
                        identity.storage.detail()
                    );
                }
                Ok(SetupResult {
                    report: result,
                    snapshot: None,
                })
            }
            WorkspaceSetupAction::SignInGitHub => {
                let github = self.github(GitHubAccountSource::Personal, command.actor_id)?;
                let login = if self.claim_login(command.operation_id)? {
                    let notify = sign_in_progress(progress);
                    github.sign_in(command.expires_at, notify).await?
                } else {
                    // A restarted command may adopt completed login, never start it again.
                    github.inspect(command.expires_at).await?
                };
                let mut result = if login.is_some() {
                    report(
                        WorkspaceSetupStatus::Succeeded,
                        None,
                        "Native personal GitHub account checked.",
                    )
                } else {
                    needs_sign_in(
                        "Native sign-in was not confirmed. Start a new sign-in operation to retry.",
                    )
                };
                if let Some(identity) = login {
                    self.bind_github_identity(
                        GitHubAccountSource::Personal,
                        command.actor_id,
                        &identity.login,
                    )?;
                    result.detail = identity.storage.detail().to_owned();
                    result.account_login = Some(identity.login);
                }
                Ok(SetupResult {
                    report: result,
                    snapshot: None,
                })
            }
            WorkspaceSetupAction::Connect {
                connection_id,
                configuration,
            }
            | WorkspaceSetupAction::Test {
                connection_id,
                configuration,
            }
            | WorkspaceSetupAction::SignInAgent {
                connection_id,
                configuration,
            } => {
                let owner = command
                    .connection_owner_id
                    .context("connection owner is missing")?;
                let prepared = self
                    .prepare_source(*connection_id, owner, configuration, command.expires_at)
                    .await?;
                let Some(prepared) = prepared else {
                    return Ok(SetupResult {
                        report: needs_sign_in(
                            "Sign in to the repository's selected native GitHub account first.",
                        ),
                        snapshot: None,
                    });
                };
                let profile = self.agent_profile(
                    *connection_id,
                    owner,
                    configuration.agent,
                    configuration.use_system_installation,
                    configuration.use_machine_account,
                )?;
                let readiness = if matches!(
                    command.action,
                    WorkspaceSetupAction::SignInAgent { .. }
                ) {
                    if command.actor_id != owner {
                        return Err(anyhow!("only the personal connection owner may sign in"));
                    }
                    if configuration
                        .use_machine_account
                        .unwrap_or(configuration.use_system_installation)
                    {
                        return Ok(SetupResult {
                            report: report(
                                WorkspaceSetupStatus::Failed,
                                None,
                                "System native accounts are read-only here. Choose a personal profile to sign in.",
                            ),
                            snapshot: None,
                        });
                    }
                    if self.claim_login(command.operation_id)? {
                        profile
                            .sign_in(command.expires_at, sign_in_progress(progress), input)
                            .await?
                    } else {
                        profile.inspect_before(command.expires_at).await?
                    }
                } else {
                    profile.inspect_before(command.expires_at).await?
                };
                let mut result = readiness_report(readiness);
                result.source = Some(prepared.source.clone());
                self.inner
                    .workspaces
                    .lock()
                    .await
                    .entry(*connection_id)
                    .or_insert(prepared.workspaces);
                Ok(SetupResult {
                    snapshot: Some(Snapshot {
                        source: prepared.source,
                        models: result.models.clone(),
                        agent: configuration.agent,
                        use_system_installation: configuration.use_system_installation,
                        use_machine_account: configuration.use_machine_account,
                        advertised: prepared.advertised,
                        source_accepted: false,
                        accepted: false,
                    }),
                    report: result,
                })
            }
        }
    }

    fn claim_login(&self, operation_id: Uuid) -> Result<bool> {
        self.update(|state| {
            let operation = state
                .operations
                .get_mut(&operation_id)
                .context("setup operation disappeared")?;
            let first = !operation.login_started;
            operation.login_started = true;
            Ok(first)
        })
    }

    fn github(&self, account: GitHubAccountSource, owner: Uuid) -> Result<NativeGitHub> {
        let relative = match account {
            GitHubAccountSource::Machine => PathBuf::from("github-machine"),
            GitHubAccountSource::Personal => PathBuf::from("accounts")
                .join(owner.simple().to_string())
                .join("github"),
        };
        let cwd = self.inner.root.directory(&relative)?;
        let home = (account == GitHubAccountSource::Personal)
            .then(|| self.inner.root.directory(&relative.join("native")))
            .transpose()?;
        let expected_login = if account == GitHubAccountSource::Personal {
            self.inner
                .state
                .lock()
                .map_err(|_| anyhow!("setup registry unavailable"))?
                .github_accounts
                .get(&owner)
                .cloned()
        } else {
            None
        };
        Ok(NativeGitHub::new(
            self.inner.github_command.clone(),
            cwd,
            home,
            expected_login,
        ))
    }

    fn bind_github_identity(
        &self,
        account: GitHubAccountSource,
        owner: Uuid,
        login: &str,
    ) -> Result<()> {
        if account == GitHubAccountSource::Machine {
            return Ok(());
        }
        self.update(|state| {
            if state
                .github_accounts
                .get(&owner)
                .is_some_and(|expected| !expected.eq_ignore_ascii_case(login))
            {
                return Err(anyhow!(
                    "native GitHub identity changed from its bound actor profile"
                ));
            }
            state.github_accounts.insert(owner, login.to_owned());
            Ok(())
        })
    }

    fn agent_profile(
        &self,
        connection: Uuid,
        owner: Uuid,
        agent: CodingAgent,
        system: bool,
        machine_account: Option<bool>,
    ) -> Result<AgentProfile> {
        if self.inner.default_adapters.get(agent.as_str()).is_none() {
            return Err(anyhow!("native agent is not configured on this runner"));
        }
        let home = self.inner.root.directory(
            &PathBuf::from("accounts")
                .join(owner.simple().to_string())
                .join("agents")
                .join(connection.simple().to_string()),
        )?;
        let catalog = self.inner.root.directory(
            &PathBuf::from("catalog")
                .join(owner.simple().to_string())
                .join(connection.simple().to_string()),
        )?;
        let mut config = self.inner.adapter_config.clone();
        if system {
            match agent {
                CodingAgent::Codex => {
                    (config.codex_command, config.codex_prefix_args) = system_agent_command(agent);
                }
                CodingAgent::ClaudeCode => {
                    (config.claude_command, config.claude_prefix_args) =
                        system_agent_command(agent);
                }
                CodingAgent::GitHubCopilot => {}
            }
        }
        AgentProfile::new(
            config,
            agent,
            home,
            catalog,
            machine_account.unwrap_or(system),
        )
    }

    fn managed_paths(&self, connection: Uuid, owner: Uuid) -> Result<(PathBuf, PathBuf)> {
        let owner_part = owner.simple().to_string();
        let connection_part = connection.simple().to_string();
        let old_repository = PathBuf::from("repositories")
            .join(&owner_part)
            .join(&connection_part);
        let old_workspaces = PathBuf::from("workspaces")
            .join(&owner_part)
            .join(&connection_part);
        // UUIDs are already unique inside this server/Corp/runner-private root.
        // Repeating the owner UUID made ordinary Windows worktree metadata exceed
        // Git's path buffer. Keep old owned paths exactly where they are; use the
        // compact layout only for new connections, never move retained work.
        let legacy = self.inner.root.path().join(&old_repository).exists()
            || self.inner.root.path().join(&old_workspaces).exists();
        let (repository_path, workspace_path) = if legacy {
            (old_repository, old_workspaces)
        } else {
            (
                PathBuf::from("r").join(&connection_part),
                if self
                    .inner
                    .root
                    .path()
                    .join("w")
                    .join(&connection_part)
                    .exists()
                {
                    PathBuf::from("w").join(&connection_part)
                } else {
                    // Task/run UUIDs already uniquely name each native worktree.
                    // Sharing this parent does not share repository ownership:
                    // WorkspaceManager validates the exact Git common directory.
                    PathBuf::from("w")
                },
            )
        };
        let parent = self
            .inner
            .root
            .directory(repository_path.parent().expect("managed repository parent"))?;
        let repository = parent.join(&connection_part);
        reject_links(&repository)?;
        let workspaces = self.inner.root.directory(&workspace_path)?;
        Ok((repository, workspaces))
    }

    async fn prepare_source(
        &self,
        connection: Uuid,
        owner: Uuid,
        configuration: &WorkspaceConnectionConfiguration,
        expires: DateTime<Utc>,
    ) -> Result<Option<PreparedSource>> {
        match &configuration.repository {
            WorkspaceRepositorySetup::Advertised { source } => {
                if source.repository_id.is_some() {
                    return Err(anyhow!(
                        "advertised source has no native immutable GitHub-ID proof"
                    ));
                }
                self.inner.default_workspaces.verify_source_identity(
                    Some(&source.repository),
                    Some(&source.base_ref),
                    Some(&source.base_commit),
                )?;
                let workspaces = Arc::new(
                    self.inner
                        .default_workspaces
                        .pinned(&source.base_ref, &source.base_commit)
                        .await?,
                );
                Ok(Some(PreparedSource {
                    source: source.clone(),
                    workspaces,
                    advertised: true,
                }))
            }
            WorkspaceRepositorySetup::GitHub {
                repository,
                repository_id,
                base_ref,
                account,
            } => {
                let github = self.github(*account, owner)?;
                let Some(identity) = github.inspect(expires).await? else {
                    return Ok(None);
                };
                self.bind_github_identity(*account, owner, &identity.login)?;
                let pinned_id = self
                    .inner
                    .state
                    .lock()
                    .map_err(|_| anyhow!("setup registry unavailable"))?
                    .connections
                    .get(&connection)
                    .context("setup connection is missing")?
                    .verified_repository_id
                    .clone()
                    .or_else(|| repository_id.clone());
                let remote = github
                    .repository(repository, pinned_id.as_deref(), base_ref, expires)
                    .await?;
                self.update(|state| {
                    let saved = state
                        .connections
                        .get_mut(&connection)
                        .context("setup connection is missing")?;
                    if saved.owner_id != owner
                        || saved
                            .verified_repository_id
                            .as_ref()
                            .is_some_and(|id| id != &remote.choice.id)
                    {
                        return Err(anyhow!("native repository identity changed"));
                    }
                    saved.verified_repository_id = Some(remote.choice.id.clone());
                    Ok(())
                })?;
                let (directory, workspace_root) = self.managed_paths(connection, owner)?;
                if !directory.exists() || std::fs::read_dir(&directory)?.next().is_none() {
                    github
                        .clone_repository(&remote.choice.repository, &directory, expires)
                        .await?;
                }
                reject_links(&directory)?;
                verify_github_origin(&github, &directory, &remote.source.repository, expires)
                    .await?;
                ensure_commit(
                    &github,
                    &directory,
                    &remote.source.base_commit,
                    "origin",
                    expires,
                )
                .await?;
                let manager = self
                    .managed_workspace(
                        connection,
                        directory,
                        workspace_root,
                        &remote.source.base_ref,
                        &remote.source.base_commit,
                    )
                    .await?;
                manager.verify_source_identity(
                    Some(&remote.source.repository),
                    Some(&remote.source.base_ref),
                    Some(&remote.source.base_commit),
                )?;
                Ok(Some(PreparedSource {
                    source: remote.source,
                    workspaces: manager,
                    advertised: false,
                }))
            }
            WorkspaceRepositorySetup::Local {
                directory,
                base_ref,
            } => {
                reject_user_path(Path::new(directory))?;
                let original = canonical_directory(Path::new(directory))?;
                if original.starts_with(self.inner.root.path())
                    || original.starts_with(self.inner.default_workspaces.root())
                {
                    return Err(anyhow!(
                        "private connection or execution state cannot be selected as a local source"
                    ));
                }
                if !self
                    .inner
                    .allowed_local_roots
                    .iter()
                    .any(|root| original.starts_with(root))
                {
                    return Err(anyhow!(
                        "local repository is outside operator-authorized roots"
                    ));
                }
                let git = self.github(GitHubAccountSource::Machine, owner)?;
                let top =
                    git_text(&git, &original, &["rev-parse", "--show-toplevel"], expires).await?;
                if canonical_directory(Path::new(top.trim()))? != original {
                    return Err(anyhow!(
                        "local selection must identify its exact repository root"
                    ));
                }
                let common = git_text(
                    &git,
                    &original,
                    &["rev-parse", "--path-format=absolute", "--git-common-dir"],
                    expires,
                )
                .await?;
                let common = canonical_directory(Path::new(common.trim()))?;
                if original != self.inner.default_workspaces.repository()
                    && !self
                        .inner
                        .allowed_local_roots
                        .iter()
                        .any(|root| common.starts_with(root))
                {
                    return Err(anyhow!(
                        "local repository metadata is outside operator-authorized roots"
                    ));
                }
                let commit = git_text(
                    &git,
                    &original,
                    &["rev-parse", "--verify", &format!("{base_ref}^{{commit}}")],
                    expires,
                )
                .await?;
                let commit = commit.trim().to_ascii_lowercase();
                if !valid_commit(&commit) {
                    return Err(anyhow!("local source did not resolve to a full commit"));
                }
                let (directory, workspace_root) = self.managed_paths(connection, owner)?;
                if !directory.exists() || std::fs::read_dir(&directory)?.next().is_none() {
                    let mut args = vec![
                        OsString::from("clone"),
                        OsString::from("--no-checkout"),
                        OsString::from("--no-hardlinks"),
                        OsString::from("--no-local"),
                    ];
                    #[cfg(windows)]
                    args.push(OsString::from("--config=core.longpaths=true"));
                    args.extend([
                        original.as_os_str().to_owned(),
                        directory.as_os_str().to_owned(),
                    ]);
                    if !git
                        .git(self.inner.root.path(), &args, expires)
                        .await?
                        .success
                    {
                        return Err(anyhow!("native local source clone did not complete"));
                    }
                }
                reject_links(&directory)?;
                let origin =
                    git_text(&git, &directory, &["remote", "get-url", "origin"], expires).await?;
                if canonical_directory(Path::new(origin.trim()))? != original {
                    return Err(anyhow!(
                        "managed local repository belongs to another source"
                    ));
                }
                ensure_commit(&git, &directory, &commit, "origin", expires).await?;
                let manager = self
                    .managed_workspace(connection, directory, workspace_root, base_ref, &commit)
                    .await?;
                let source = WorkspaceSourceIdentity {
                    repository: manager
                        .repository_identity()
                        .context("native local repository identity is unavailable")?
                        .to_owned(),
                    repository_id: None,
                    base_ref: base_ref.clone(),
                    base_commit: commit,
                };
                Ok(Some(PreparedSource {
                    source,
                    workspaces: manager,
                    advertised: false,
                }))
            }
        }
    }

    async fn managed_workspace(
        &self,
        id: Uuid,
        repository: PathBuf,
        root: PathBuf,
        base_ref: &str,
        commit: &str,
    ) -> Result<Arc<WorkspaceManager>> {
        let mut workspaces = self.inner.workspaces.lock().await;
        let base = match workspaces.get(&id) {
            Some(manager) => manager.clone(),
            None => {
                let manager = Arc::new(
                    WorkspaceManager::initialize_pinned(
                        root,
                        repository,
                        base_ref.to_owned(),
                        commit.to_owned(),
                    )
                    .await?,
                );
                workspaces.insert(id, manager.clone());
                manager
            }
        };
        Ok(Arc::new(base.pinned(base_ref, commit).await?))
    }

    async fn workspace_snapshot(
        &self,
        id: Uuid,
        owner: Uuid,
        snapshot: &Snapshot,
    ) -> Result<Arc<WorkspaceManager>> {
        if snapshot.advertised {
            if self
                .inner
                .default_workspaces
                .repository_identity()
                .is_none_or(|identity| !identity.eq_ignore_ascii_case(&snapshot.source.repository))
            {
                return Err(anyhow!("advertised source identity changed on this runner"));
            }
            return Ok(Arc::new(
                self.inner
                    .default_workspaces
                    .pinned(&snapshot.source.base_ref, &snapshot.source.base_commit)
                    .await?,
            ));
        }
        let (repository, root) = self.managed_paths(id, owner)?;
        if !repository.is_dir() {
            return Err(anyhow!(
                "accepted source checkout is missing; refusing to recreate a preserved run"
            ));
        }
        let manager = self
            .managed_workspace(
                id,
                repository,
                root,
                &snapshot.source.base_ref,
                &snapshot.source.base_commit,
            )
            .await?;
        manager.verify_source_identity(
            Some(&snapshot.source.repository),
            Some(&snapshot.source.base_ref),
            Some(&snapshot.source.base_commit),
        )?;
        Ok(manager)
    }
}

fn system_agent_command(agent: CodingAgent) -> (PathBuf, Vec<OsString>) {
    let command = match agent {
        CodingAgent::Codex => crate::default_codex_command(),
        CodingAgent::ClaudeCode => crate::default_provider_command("claude"),
        CodingAgent::GitHubCopilot => crate::default_provider_command("copilot"),
    };
    (command, Vec::new())
}

fn report(
    status: WorkspaceSetupStatus,
    connection_status: Option<WorkspaceConnectionStatus>,
    detail: &str,
) -> WorkspaceSetupReport {
    WorkspaceSetupReport {
        status,
        detail: detail.to_owned(),
        connection_status,
        source: None,
        models: Vec::new(),
        account_login: None,
        sign_in: None,
        repositories: Vec::new(),
    }
}

fn submit_code(flight: &Flight, response: NativeSignInResponse) -> Result<()> {
    if !flight.claude_sign_in || flight.expires_at <= Utc::now() {
        return Err(anyhow!("no matching native sign-in is waiting"));
    }
    let digest: [u8; 32] = Sha256::digest(response.authorization_code.as_bytes()).into();
    let mut submitted = flight
        .submitted_codes
        .lock()
        .map_err(|_| anyhow!("native sign-in input is unavailable"))?;
    if let Some(previous) = submitted.get(&response.input_id) {
        return if *previous == digest {
            Ok(())
        } else {
            Err(anyhow!(
                "native input ID was reused with a different response"
            ))
        };
    }
    if submitted.len() >= 32 {
        return Err(anyhow!("native sign-in prompt limit reached"));
    }
    let prompt = flight
        .code_prompt_id
        .lock()
        .map_err(|_| anyhow!("native sign-in input is unavailable"))?;
    if *prompt != Some(response.input_id)
        || flight
            .code_waiting
            .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
    {
        return Err(anyhow!("no matching native sign-in is waiting"));
    }
    let id = response.input_id;
    flight
        .code_sender
        .try_send(response)
        .map_err(|_| anyhow!("native sign-in input is no longer available"))?;
    submitted.insert(id, digest);
    Ok(())
}

fn needs_sign_in(detail: &str) -> WorkspaceSetupReport {
    report(
        WorkspaceSetupStatus::Succeeded,
        Some(WorkspaceConnectionStatus::NeedsSignIn),
        detail,
    )
}

fn snapshot_capability(id: Uuid, snapshot: &Snapshot, provider: bool) -> RunnerCapability {
    RunnerCapability {
        name: if provider { snapshot.agent.as_str() } else { "workspace-isolation" }.to_owned(),
        available: true,
        detail: Some(if provider {
            "Server-accepted native account/catalog and immutable source pin; inference not tested."
        } else {
            "Server-accepted retained source/workspace pin; model sign-in is not required for recovery."
        }.to_owned()),
        models: if provider { snapshot.models.clone() } else { Vec::new() },
        source_repository: Some(snapshot.source.repository.clone()),
        source_base_ref: Some(snapshot.source.base_ref.clone()),
        source_base_commit: Some(snapshot.source.base_commit.clone()),
        workspace_connection_id: Some(id),
    }
}

fn readiness_report(readiness: AgentReadiness) -> WorkspaceSetupReport {
    let status = match readiness.status {
        WorkspaceConnectionStatus::Ready => WorkspaceSetupStatus::Succeeded,
        WorkspaceConnectionStatus::NeedsSignIn => WorkspaceSetupStatus::Succeeded,
        _ => WorkspaceSetupStatus::Failed,
    };
    let mut report = report(status, Some(readiness.status), &readiness.detail);
    report.models = readiness.models;
    report.account_login = readiness.account_login;
    report
}

fn sign_in_progress(progress: SetupProgress) -> Arc<dyn Fn(NativeSignInInstruction) + Send + Sync> {
    Arc::new(move |instruction| {
        let mut report = needs_sign_in(
            if instruction.input_kind == Some(NativeSignInInputKind::AuthorizationCode) {
                "Paste the one-time authorization code returned by the native Claude sign-in page."
            } else {
                "Complete the native sign-in in your browser; credentials stay on the selected runner."
            },
        );
        report.status = WorkspaceSetupStatus::NeedsSignIn;
        report.sign_in = Some(instruction);
        progress(report);
    })
}

fn configuration(action: &WorkspaceSetupAction) -> Option<&WorkspaceConnectionConfiguration> {
    match action {
        WorkspaceSetupAction::Connect { configuration, .. }
        | WorkspaceSetupAction::Test { configuration, .. }
        | WorkspaceSetupAction::SignInAgent { configuration, .. } => Some(configuration),
        _ => None,
    }
}

fn require_same_command(left: &WorkspaceSetupCommand, right: &WorkspaceSetupCommand) -> Result<()> {
    if serde_json::to_value(left)? != serde_json::to_value(right)? {
        return Err(anyhow!(
            "setup operation UUID was reused with different authority or configuration"
        ));
    }
    Ok(())
}

fn immutable_configuration_matches(
    left: &WorkspaceConnectionConfiguration,
    right: &WorkspaceConnectionConfiguration,
) -> bool {
    if left.agent != right.agent
        || left.use_system_installation != right.use_system_installation
        || left.use_machine_account != right.use_machine_account
    {
        return false;
    }
    match (&left.repository, &right.repository) {
        (
            WorkspaceRepositorySetup::GitHub {
                repository: left,
                account: la,
                repository_id: li,
                ..
            },
            WorkspaceRepositorySetup::GitHub {
                repository: right,
                account: ra,
                repository_id: ri,
                ..
            },
        ) => left.eq_ignore_ascii_case(right) && la == ra && li == ri,
        (
            WorkspaceRepositorySetup::Local {
                directory: left, ..
            },
            WorkspaceRepositorySetup::Local {
                directory: right, ..
            },
        ) => left == right,
        (
            WorkspaceRepositorySetup::Advertised { source: left },
            WorkspaceRepositorySetup::Advertised { source: right },
        ) => {
            left.repository.eq_ignore_ascii_case(&right.repository)
                && left.repository_id == right.repository_id
        }
        _ => false,
    }
}

fn begin_operation(state: &mut Registry, command: &WorkspaceSetupCommand) -> Result<()> {
    if let Some(existing) = state.operations.get(&command.operation_id) {
        return require_same_command(&existing.command, command);
    }
    if state.operations.len() >= MAX_OPERATIONS {
        return Err(anyhow!("native setup receipt retention bound reached"));
    }
    if let Some(id) = command.action.connection_id() {
        let owner = command
            .connection_owner_id
            .context("connection owner is missing")?;
        let version = command
            .expected_connection_version
            .context("connection version is missing")?;
        let configuration =
            configuration(&command.action).context("connection configuration is missing")?;
        if let Some(connection) = state.connections.get_mut(&id) {
            if connection.owner_id != owner
                || connection.room_id != command.room_id
                || version < connection.version
                || !immutable_configuration_matches(&connection.configuration, configuration)
                || (version == connection.version && connection.configuration != *configuration)
            {
                return Err(anyhow!(
                    "saved connection owner, version or native profile scope changed"
                ));
            }
            connection.version = version;
            connection.configuration = configuration.clone();
            connection.current_operation = command.operation_id;
            connection.active_snapshot = None;
        } else {
            if state.connections.len() >= MAX_CONNECTIONS {
                return Err(anyhow!("saved connection retention bound reached"));
            }
            state.connections.insert(
                id,
                SavedConnection {
                    owner_id: owner,
                    room_id: command.room_id,
                    version,
                    current_operation: command.operation_id,
                    verified_repository_id: match &configuration.repository {
                        WorkspaceRepositorySetup::GitHub { repository_id, .. } => {
                            repository_id.clone()
                        }
                        _ => None,
                    },
                    configuration: configuration.clone(),
                    active_snapshot: None,
                    source_snapshot: None,
                    snapshots: BTreeMap::new(),
                },
            );
        }
    }
    state.operations.insert(
        command.operation_id,
        SavedOperation {
            command: command.clone(),
            login_started: false,
            report: None,
            candidate: None,
            acknowledged: None,
        },
    );
    Ok(())
}

fn acknowledge(state: &mut Registry, operation_id: Uuid, accepted: bool) -> Result<()> {
    let operation = state
        .operations
        .get(&operation_id)
        .context("unknown setup acknowledgement")?
        .clone();
    if let Some(previous) = operation.acknowledged {
        if previous != accepted {
            return Err(anyhow!("setup acknowledgement decision changed"));
        }
        return Ok(());
    }
    let report = operation
        .report
        .as_ref()
        .context("setup has no completed native receipt")?;
    if !report.status.terminal() || report.sign_in.is_some() {
        return Err(anyhow!(
            "setup acknowledgement requires a terminal native receipt"
        ));
    }
    if accepted && let Some(key) = operation.candidate.as_ref() {
        let id = operation
            .command
            .action
            .connection_id()
            .context("ready result has no connection")?;
        let connection = state
            .connections
            .get_mut(&id)
            .context("ready connection is missing")?;
        let current = Some(connection.version) == operation.command.expected_connection_version
            && connection.current_operation == operation_id;
        let snapshot = connection
            .snapshots
            .get_mut(key)
            .context("ready source snapshot is missing")?;
        if report.source.as_ref() != Some(&snapshot.source) || report.models != snapshot.models {
            return Err(anyhow!(
                "ready acknowledgement does not match its native receipt"
            ));
        }
        snapshot.source_accepted = true;
        let provider_ready = report.status == WorkspaceSetupStatus::Succeeded
            && report.connection_status == Some(WorkspaceConnectionStatus::Ready);
        if provider_ready {
            snapshot.accepted = true;
        }
        // A late ACK may restore the historical source the server already
        // accepted, but must never replace a newer connection/profile selection.
        if current {
            connection.source_snapshot = Some(key.clone());
            connection.active_snapshot = provider_ready.then(|| key.clone());
        }
    } else if accepted && report.connection_status == Some(WorkspaceConnectionStatus::Ready) {
        return Err(anyhow!("ready result has no native source snapshot"));
    }
    state
        .operations
        .get_mut(&operation_id)
        .unwrap()
        .acknowledged = Some(accepted);
    Ok(())
}

fn validate_registry(state: &Registry, corp: Uuid, runner: &str) -> Result<()> {
    if state.schema_version != REGISTRY_SCHEMA
        || state.corp_id != corp
        || state.runner_id != runner
        || state.connections.len() > MAX_CONNECTIONS
        || state.operations.len() > MAX_OPERATIONS
    {
        return Err(anyhow!(
            "connection registry belongs to another scope or unsupported schema"
        ));
    }
    for (id, connection) in &state.connections {
        if id.is_nil()
            || connection.owner_id.is_nil()
            || connection.room_id.is_nil()
            || connection.version <= 0
            || connection.snapshots.len() > MAX_SNAPSHOTS
        {
            return Err(anyhow!("invalid saved connection authority"));
        }
        for (key, snapshot) in &connection.snapshots {
            validate_source(&snapshot.source)?;
            if snapshot_key(snapshot)? != *key
                || snapshot.agent != connection.configuration.agent
                || snapshot.use_system_installation
                    != connection.configuration.use_system_installation
                || snapshot.use_machine_account != connection.configuration.use_machine_account
            {
                return Err(anyhow!("saved source snapshot changed scope"));
            }
            if matches!(
                connection.configuration.repository,
                WorkspaceRepositorySetup::GitHub { .. }
            ) && snapshot.source.repository_id != connection.verified_repository_id
            {
                return Err(anyhow!(
                    "saved source snapshot changed native repository identity"
                ));
            }
            if snapshot.accepted
                && !state.operations.values().any(|operation| {
                    operation.command.action.connection_id() == Some(*id)
                        && operation.acknowledged == Some(true)
                        && operation.candidate.as_ref() == Some(key)
                        && operation.report.as_ref().is_some_and(|report| {
                            report.status == WorkspaceSetupStatus::Succeeded
                                && report.connection_status
                                    == Some(WorkspaceConnectionStatus::Ready)
                                && report.source.as_ref() == Some(&snapshot.source)
                        })
                })
            {
                return Err(anyhow!(
                    "accepted source snapshot has no server-accepted receipt"
                ));
            }
            if snapshot.source_accepted
                && !state.operations.values().any(|operation| {
                    operation.command.action.connection_id() == Some(*id)
                        && operation.acknowledged == Some(true)
                        && operation.candidate.as_ref() == Some(key)
                        && operation.report.as_ref().is_some_and(|report| {
                            report.status.terminal()
                                && report.source.as_ref() == Some(&snapshot.source)
                        })
                })
            {
                return Err(anyhow!("retained source has no accepted native receipt"));
            }
        }
        if let Some(key) = &connection.active_snapshot
            && !connection
                .snapshots
                .get(key)
                .is_some_and(|snapshot| snapshot.accepted)
        {
            return Err(anyhow!("active source snapshot has no accepted authority"));
        }
        if let Some(key) = &connection.source_snapshot
            && !connection
                .snapshots
                .get(key)
                .is_some_and(|snapshot| snapshot.source_accepted)
        {
            return Err(anyhow!(
                "retained source selection has no accepted authority"
            ));
        }
    }
    for (id, operation) in &state.operations {
        if *id != operation.command.operation_id
            || operation.command.corp_id != corp
            || operation.command.runner_id != runner
            || operation
                .report
                .as_ref()
                .is_some_and(|report| report.sign_in.is_some() || !report.status.terminal())
        {
            return Err(anyhow!("invalid or non-private setup receipt"));
        }
    }
    Ok(())
}

fn snapshot_key(snapshot: &Snapshot) -> Result<String> {
    let base = (
        &snapshot.source,
        snapshot.agent,
        snapshot.use_system_installation,
        snapshot.advertised,
        &snapshot.models,
    );
    let encoded = if let Some(machine_account) = snapshot.use_machine_account {
        serde_json::to_vec(&(base, machine_account))?
    } else {
        // Preserve the keys of already retained initial-schema snapshots.
        serde_json::to_vec(&base)?
    };
    Ok(hex::encode(Sha256::digest(encoded)))
}

fn validate_source(source: &WorkspaceSourceIdentity) -> Result<()> {
    if source.repository.is_empty()
        || source.repository.len() > 250
        || source.repository.chars().any(char::is_control)
        || !valid_commit(&source.base_commit)
        || source
            .repository_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 200)
    {
        return Err(anyhow!("source identity is incomplete or invalid"));
    }
    validate_ref(&source.base_ref)
}

fn source_matches(stored: &WorkspaceSourceIdentity, expected: &WorkspaceSourceIdentity) -> bool {
    stored.repository.eq_ignore_ascii_case(&expected.repository)
        && stored.base_ref == expected.base_ref
        && stored
            .base_commit
            .eq_ignore_ascii_case(&expected.base_commit)
        && expected
            .repository_id
            .as_ref()
            .is_none_or(|id| stored.repository_id.as_ref() == Some(id))
}

async fn git_text(
    git: &NativeGitHub,
    cwd: &Path,
    args: &[&str],
    expires: DateTime<Utc>,
) -> Result<String> {
    let args = args.iter().map(OsString::from).collect::<Vec<_>>();
    let output = git.git(cwd, &args, expires).await?;
    if !output.success {
        return Err(anyhow!("native source inspection failed"));
    }
    String::from_utf8(output.stdout).context("native source response is not UTF-8")
}

async fn ensure_commit(
    git: &NativeGitHub,
    cwd: &Path,
    commit: &str,
    remote: &str,
    expires: DateTime<Utc>,
) -> Result<()> {
    if !valid_commit(commit) {
        return Err(anyhow!("invalid immutable source commit"));
    }
    let expression = format!("{commit}^{{commit}}");
    let args = ["cat-file", "-e", &expression].map(OsString::from);
    if !git.git(cwd, &args, expires).await?.success {
        let args = [
            "fetch",
            "--no-tags",
            "--no-write-fetch-head",
            remote,
            commit,
        ]
        .map(OsString::from);
        if !git.git(cwd, &args, expires).await?.success {
            return Err(anyhow!(
                "exact source commit is unavailable; existing checkout was preserved"
            ));
        }
    }
    let resolved = git_text(git, cwd, &["rev-parse", "--verify", &expression], expires).await?;
    if !resolved.trim().eq_ignore_ascii_case(commit) {
        return Err(anyhow!("source commit did not resolve exactly"));
    }
    Ok(())
}

async fn verify_github_origin(
    git: &NativeGitHub,
    cwd: &Path,
    repository: &str,
    expires: DateTime<Utc>,
) -> Result<()> {
    let origin = git_text(git, cwd, &["remote", "get-url", "origin"], expires).await?;
    let origin = origin.trim();
    let path = if let Some(path) = origin.strip_prefix("git@github.com:") {
        path
    } else {
        let url = url::Url::parse(origin).context("managed origin is not a GitHub URL")?;
        if url.scheme() != "https"
            || url.host_str() != Some("github.com")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(anyhow!(
                "managed origin is outside the selected GitHub identity"
            ));
        }
        if !url
            .path()
            .trim_matches('/')
            .trim_end_matches(".git")
            .eq_ignore_ascii_case(repository)
        {
            return Err(anyhow!("managed checkout belongs to another repository"));
        }
        return Ok(());
    };
    if !path
        .trim_end_matches(".git")
        .eq_ignore_ascii_case(repository)
    {
        return Err(anyhow!("managed checkout belongs to another repository"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "connection_setup/tests.rs"]
mod tests;

#[cfg(all(test, windows))]
#[path = "connection_setup/integration_tests.rs"]
mod integration_tests;
