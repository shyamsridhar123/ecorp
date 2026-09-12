//! Actual handlers + real migrations; the only runner is an in-memory capability
//! fixture. No native account, provider process or application database is used.
use super::*;
use anyhow::Result;
use crony_domain::{
    CodingAgent, GitHubAccountSource, RunnerModel, WorkspaceConnectionConfiguration,
    WorkspaceConnectionStatus, WorkspaceRepositorySetup, WorkspaceSetupAction,
    WorkspaceSetupReport, WorkspaceSetupStatus, WorkspaceSourceIdentity,
};
use crony_store::{CreateWorkspaceConnectionInput, DemoIds, WorkspaceSetupInput};
use serde_json::Value;
use sqlx::{ConnectOptions, PgPool};

struct Fixture {
    state: AppState,
    ids: DemoIds,
    connection_id: Uuid,
    runner_id: String,
    epoch: Uuid,
    artifact_root: PathBuf,
    _commands: mpsc::UnboundedReceiver<ServerToRunner>,
}

fn source() -> WorkspaceSourceIdentity {
    WorkspaceSourceIdentity {
        repository: "fixture/project".to_owned(),
        repository_id: Some("issue204-repository".to_owned()),
        base_ref: "main".to_owned(),
        base_commit: "a".repeat(40),
    }
}

fn configuration() -> WorkspaceConnectionConfiguration {
    WorkspaceConnectionConfiguration {
        repository: WorkspaceRepositorySetup::GitHub {
            repository: source().repository,
            repository_id: source().repository_id,
            base_ref: source().base_ref,
            account: GitHubAccountSource::Personal,
        },
        agent: CodingAgent::Codex,
        use_system_installation: false,
        use_machine_account: Some(false),
    }
}

fn model() -> RunnerModel {
    RunnerModel {
        id: "issue204-handler-model".to_owned(),
        name: "Synthetic handler model".to_owned(),
        policy_state: Some("enabled".to_owned()),
        policy_terms: None,
        supports_vision: false,
        supports_reasoning_effort: false,
        max_prompt_tokens: None,
        max_context_window_tokens: Some(4096),
        supported_reasoning_efforts: vec![],
        default_reasoning_effort: None,
        billing_multiplier: None,
    }
}

fn report() -> WorkspaceSetupReport {
    WorkspaceSetupReport {
        status: WorkspaceSetupStatus::Succeeded,
        detail: "Synthetic checked connection; no provider was invoked".to_owned(),
        connection_status: Some(WorkspaceConnectionStatus::Ready),
        source: Some(source()),
        models: vec![model()],
        account_login: None,
        sign_in: None,
        repositories: vec![],
    }
}

fn capability(name: &str, connection_id: Option<Uuid>) -> RunnerCapability {
    RunnerCapability {
        name: name.to_owned(),
        available: true,
        detail: None,
        models: if name == "codex" {
            vec![model()]
        } else {
            vec![]
        },
        source_repository: connection_id.map(|_| source().repository),
        source_base_ref: connection_id.map(|_| source().base_ref),
        source_base_commit: connection_id.map(|_| source().base_commit),
        workspace_connection_id: connection_id,
    }
}

impl Fixture {
    async fn new(pool: PgPool) -> Result<Self> {
        // This URL comes only from the SQLx-provided disposable pool, never the
        // ambient environment. It is not logged, serialized or passed to a child.
        let database = pool.connect_options().to_url_lossy();
        let store = PgStore::connect(database.as_str()).await?;
        let (ids, _) = store.bootstrap_demo().await?;
        let runner_id = format!("issue204-handler-{}", Uuid::new_v4());
        let epoch = Uuid::new_v4();
        store
            .runner_connected(RunnerConnectInput {
                id: runner_id.clone(),
                corp_id: ids.corp_id,
                hostname: "synthetic-only".to_owned(),
                os: "windows".to_owned(),
                capabilities: json!([capability("workspace-setup-v1", None)]),
                connection_epoch: epoch,
            })
            .await?;
        let created = store
            .create_workspace_connection(CreateWorkspaceConnectionInput {
                corp_id: ids.corp_id,
                room_id: ids.room_id,
                actor_id: ids.alice_actor_id,
                runner_id: runner_id.clone(),
                label: "Handler connection fixture".to_owned(),
                configuration: configuration(),
                idempotency_key: "issue204-handler-connect".to_owned(),
            })
            .await?;
        let ready = store
            .apply_workspace_setup_report(
                ids.corp_id,
                &runner_id,
                epoch,
                created.operation.id,
                report(),
            )
            .await?;
        let connection_id = ready.connection.context("saved fixture connection")?.id;
        let artifact_root =
            std::env::temp_dir().join(format!("ecorp-issue204-handlers-{}", Uuid::new_v4()));
        let artifacts = ArtifactStore::initialize(
            "local",
            artifact_root.clone(),
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            1024,
            false,
        )?;
        let (event_tx, _) = broadcast::channel(64);
        let state = AppState {
            store,
            event_tx,
            runners: Arc::new(DashMap::new()),
            strategies: StrategyRegistry::new(),
            runner_grace_secs: 10,
            runner_credential_ttl_secs: 300,
            publication_publisher_credential_ttl_secs: 300,
            auth: AuthService::initialize(ServerMode::Development, None, false).await?,
            secret_cipher: SecretCipher::initialize(ServerMode::Development, None)?,
            artifacts,
            artifact_retention_days: 1,
            workspace_sign_in: Arc::new(DashMap::new()),
        };
        let (tx, commands) = mpsc::unbounded_channel();
        // There is deliberately no unbound coding capability. Forgetting the
        // policy connection in either handler reproduces the actual API defect.
        state.runners.insert(
            runner_id.clone(),
            RunnerConnection {
                corp_id: ids.corp_id,
                connection_epoch: epoch,
                dispatch_ready: true,
                tx,
                capabilities: vec![
                    capability("workspace-isolation", Some(connection_id)),
                    capability("codex", Some(connection_id)),
                ],
            },
        );
        Ok(Self {
            state,
            ids,
            connection_id,
            runner_id,
            epoch,
            artifact_root,
            _commands: commands,
        })
    }

    fn preflight(&self) -> PreflightFactoryMissionRequest {
        serde_json::from_value(json!({
            "actor_id": self.ids.alice_actor_id,
            "source_repository_owner": "fixture",
            "source_repository_name": "project",
            "title": "Issue 204 handler connection",
            "description": "Synthetic handler acceptance, no provider execution",
            "preferred_adapter": "codex",
            "preferred_model": model().id,
            "strategy": "single",
            "budget_tokens": 1000,
            "budget_cost_microusd": 1000000,
            "contract": {
                "objective": "Verify the exact Factory connection",
                "expected_output": "result.md",
                "allowed_tools": ["filesystem"],
                "prohibited_actions": ["No external effects"],
                "write_scope": ["result.md"],
            },
            "policy": {
                "schema_version": 1,
                "source_of_truth": "github_project",
                "auto_merge": false,
                "workspace_connection_id": self.connection_id,
                "repository_allowlist": [source().repository],
                "source_base_ref": source().base_ref,
                "source_base_commit": source().base_commit,
                "adapter_allowlist": ["codex"],
                "strategy_allowlist": ["single"],
                "model": model().id,
                "reasoning_effort": null,
                "write_scope": ["result.md"],
                "allowed_tools": ["filesystem"],
                "prohibited_actions": ["No external effects"],
                "secret_ids": [],
                "verification_required": true,
                "budget_tokens": 1000,
                "budget_cost_microusd": 1000000,
            },
        }))
        .unwrap()
    }

    async fn claimed_request(&self) -> Result<MaterializeFactoryMissionRequest> {
        let request = self.preflight();
        let outcome = self
            .state
            .store
            .claim_factory_work_item(ClaimFactoryWorkItemInput {
                corp_id: self.ids.corp_id,
                actor_id: self.ids.alice_actor_id,
                source: FactorySourceInput {
                    project_owner: "fixture".to_owned(),
                    project_number: 3,
                    project_item_id: "issue204-handler-item".to_owned(),
                    repository_owner: "fixture".to_owned(),
                    repository_name: "project".to_owned(),
                    issue_number: 204,
                    issue_node_id: "issue204-handler-issue".to_owned(),
                    issue_url: "https://github.com/fixture/project/issues/204".to_owned(),
                    title: request.title.clone(),
                    revision: "2026-09-09T00:00:00Z".to_owned(),
                },
                idempotency_key: "issue204-handler-claim".to_owned(),
                lease_seconds: 300,
                policy: request.policy.clone(),
            })
            .await?;
        let mut value = serde_json::to_value(request)?;
        value["claim_token"] = json!(outcome.claim_token.context("claim token")?);
        value["expected_version"] = json!(outcome.work_item.version);
        value["idempotency_key"] = json!("issue204-handler-materialize");
        Ok(serde_json::from_value(value)?)
    }

    async fn item(&self) -> Result<crony_domain::FactoryWorkItem> {
        self.state
            .store
            .snapshot(self.ids.corp_id, self.ids.alice_actor_id)
            .await?
            .factory_work_items
            .into_iter()
            .next()
            .context("fixture work item")
    }

    async fn snapshot(&self) -> Result<Value> {
        Ok(serde_json::to_value(
            self.state
                .store
                .snapshot(self.ids.corp_id, self.ids.alice_actor_id)
                .await?,
        )?)
    }

    async fn change_connection(&self, source_changed: bool) -> Result<()> {
        let operation = self
            .state
            .store
            .begin_workspace_setup(WorkspaceSetupInput {
                corp_id: self.ids.corp_id,
                room_id: self.ids.room_id,
                actor_id: self.ids.alice_actor_id,
                runner_id: self.runner_id.clone(),
                action: WorkspaceSetupAction::Test {
                    connection_id: self.connection_id,
                    configuration: configuration(),
                },
                idempotency_key: "issue204-handler-change".to_owned(),
            })
            .await?;
        let mut changed = report();
        if source_changed {
            changed.source.as_mut().unwrap().base_commit = "b".repeat(40);
        } else {
            changed.status = WorkspaceSetupStatus::Failed;
            changed.connection_status = Some(WorkspaceConnectionStatus::Failed);
            changed.source = None;
            changed.models.clear();
        }
        self.state
            .store
            .apply_workspace_setup_report(
                self.ids.corp_id,
                &self.runner_id,
                self.epoch,
                operation.operation.id,
                changed,
            )
            .await?;
        Ok(())
    }

    async fn finish(self) {
        self.state.store.pool().close().await;
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // The handlers under test never write artifact bytes. Remove only our
        // empty random directory; unexpected contents are preserved, not recursed.
        let _ = std::fs::remove_dir(&self.artifact_root);
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue204_handler_bound_only_preflight_and_materialization(pool: PgPool) -> Result<()> {
    let f = Fixture::new(pool).await?;
    let before = f.snapshot().await?;
    let checked = preflight_factory_mission(
        State(f.state.clone()),
        Extension(Principal::Development),
        Path(f.ids.corp_id),
        Json(f.preflight()),
    )
    .await
    .map_err(|error| anyhow::anyhow!(error.message))?;
    assert!(checked.valid);
    assert_eq!(checked.task_count, 1);
    assert_eq!(f.snapshot().await?, before);
    let request = f.claimed_request().await?;
    let item = f.item().await?;
    let result = materialize_factory_mission(
        State(f.state.clone()),
        Extension(Principal::Development),
        Path((f.ids.corp_id, item.id)),
        Json(request),
    )
    .await
    .map_err(|error| anyhow::anyhow!(error.message))?;
    let snapshot = f
        .state
        .store
        .snapshot(f.ids.corp_id, f.ids.alice_actor_id)
        .await?;
    let tasks: Vec<_> = snapshot
        .tasks
        .iter()
        .filter(|task| task.mission_id == result.mission_id)
        .collect();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].contract.workspace_connection_id,
        Some(f.connection_id)
    );
    assert!(
        snapshot.runs.is_empty(),
        "materialization must not launch work"
    );
    f.finish().await;
    Ok(())
}

async fn assert_admission_conflict_releases_claim(
    pool: PgPool,
    source_changed: bool,
) -> Result<()> {
    let f = Fixture::new(pool).await?;
    let request = f.claimed_request().await?;
    let original = f.item().await?;
    f.change_connection(source_changed).await?;
    let error = materialize_factory_mission(
        State(f.state.clone()),
        Extension(Principal::Development),
        Path((f.ids.corp_id, original.id)),
        Json(request.clone()),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status, StatusCode::CONFLICT);
    assert!(
        error.message.contains(if source_changed {
            "saved source changed"
        } else {
            "saved connection needs attention"
        }),
        "{}",
        error.message
    );
    let blocked = f.item().await?;
    assert_eq!(blocked.state, crony_domain::FactoryWorkItemState::Blocked);
    assert_eq!(blocked.policy, original.policy);
    assert!(blocked.mission_id.is_none());
    assert!(blocked.lease_expires_at <= Utc::now());
    let after = f.snapshot().await?;
    assert!(after["missions"].as_array().unwrap().is_empty());
    assert!(after["tasks"].as_array().unwrap().is_empty());
    assert!(after["runs"].as_array().unwrap().is_empty());
    assert!(
        materialize_factory_mission(
            State(f.state.clone()),
            Extension(Principal::Development),
            Path((f.ids.corp_id, original.id)),
            Json(request),
        )
        .await
        .is_err()
    );
    assert_eq!(f.snapshot().await?, after);
    f.finish().await;
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue204_handler_post_claim_readiness_conflict_releases_exact_claim(
    pool: PgPool,
) -> Result<()> {
    assert_admission_conflict_releases_claim(pool, false).await
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue204_handler_post_claim_source_conflict_releases_exact_claim(
    pool: PgPool,
) -> Result<()> {
    assert_admission_conflict_releases_claim(pool, true).await
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue204_handler_stale_claim_conflicts_do_not_release_current_claim(
    pool: PgPool,
) -> Result<()> {
    let f = Fixture::new(pool).await?;
    let original = f.claimed_request().await?;
    let item = f.item().await?;
    let before = f.snapshot().await?;
    for changed_token in [false, true] {
        let mut request = original.clone();
        if changed_token {
            request.claim_token = Uuid::new_v4();
        } else {
            request.expected_version += 1;
        }
        let error = materialize_factory_mission(
            State(f.state.clone()),
            Extension(Principal::Development),
            Path((f.ids.corp_id, item.id)),
            Json(request),
        )
        .await
        .unwrap_err();
        assert_eq!(error.status, StatusCode::CONFLICT);
        assert_eq!(f.snapshot().await?, before);
    }
    f.finish().await;
    Ok(())
}
