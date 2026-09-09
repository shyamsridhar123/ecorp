//! Actual-store regressions using only the pool supplied by SQLx's disposable
//! database harness. The parent must opt in through its owned maintenance loader.
//! Runner registration, sign-in reports and launches below are metadata only:
//! no account, native executable, provider, filesystem checkout or service is used.
//!
//! Bootstrap, connection/setup, runner and mission/run transitions use PgStore.
//! Direct SQL supplies extra tenant/membership fixtures, deterministic expiry,
//! late transaction faults and independent persisted-state assertions only.

use super::{
    CreateWorkspaceConnectionInput, DemoIds, PgStore, RunnerConnectInput, RunnerRecord,
    WorkspaceSetupInput, WorkspaceSetupMutation,
};
use anyhow::{Context, Result};
use chrono::Duration;
use crony_domain::{
    CodingAgent, DomainEvent, GitHubAccountSource, GitHubRepositoryChoice, NativeSignInInputKind,
    NativeSignInInstruction, PlannedTask, RunnerModel, TaskContract, TaskGraphPlan,
    VerificationPolicy, VerifierCheck, WorkspaceConnection, WorkspaceConnectionConfiguration,
    WorkspaceConnectionStatus, WorkspaceRepositorySetup, WorkspaceSetupAction,
    WorkspaceSetupOperation, WorkspaceSetupReport, WorkspaceSetupStatus, WorkspaceSourceIdentity,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

const PRIVATE_DIRECTORY: &str = "/issue198-private-path/project";
const PRIVATE_LOGIN: &str = "issue198-private-account";
const PRIVATE_CODE: &str = "ISSUE198-DEVICE-CODE";
const PRIVATE_URI: &str = "https://claude.ai/oauth/authorize?state=issue198-private-oauth-state";
const PRIVATE_REPOSITORY: &str = "fixture/issue198-private-repository";

struct Fixture {
    store: PgStore,
    ids: DemoIds,
    runner: RunnerRecord,
}

fn source() -> WorkspaceSourceIdentity {
    WorkspaceSourceIdentity {
        repository: "fixture/project".to_owned(),
        repository_id: Some("issue198-repository-id".to_owned()),
        base_ref: "main".to_owned(),
        base_commit: "a".repeat(40),
    }
}

fn configuration() -> WorkspaceConnectionConfiguration {
    let source = source();
    WorkspaceConnectionConfiguration {
        repository: WorkspaceRepositorySetup::GitHub {
            repository: source.repository,
            repository_id: source.repository_id,
            base_ref: source.base_ref,
            account: GitHubAccountSource::Personal,
        },
        agent: CodingAgent::Codex,
        use_system_installation: false,
        use_machine_account: None,
    }
}

fn model() -> RunnerModel {
    RunnerModel {
        id: "issue198-fixture-model".to_owned(),
        name: "Fixture model".to_owned(),
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

fn report(status: WorkspaceSetupStatus) -> WorkspaceSetupReport {
    WorkspaceSetupReport {
        status,
        detail: "Synthetic native check result".to_owned(),
        connection_status: None,
        source: None,
        models: vec![],
        account_login: None,
        sign_in: None,
        repositories: vec![],
    }
}

fn ready_report() -> WorkspaceSetupReport {
    WorkspaceSetupReport {
        connection_status: Some(WorkspaceConnectionStatus::Ready),
        source: Some(source()),
        models: vec![model()],
        ..report(WorkspaceSetupStatus::Succeeded)
    }
}

fn sign_in_report(operation: &WorkspaceSetupOperation, step: Uuid) -> WorkspaceSetupReport {
    WorkspaceSetupReport {
        detail: "Complete native sign-in on the selected machine".to_owned(),
        account_login: Some(PRIVATE_LOGIN.to_owned()),
        sign_in: Some(NativeSignInInstruction {
            provider: "claude-code".to_owned(),
            verification_uri: PRIVATE_URI.to_owned(),
            user_code: Some(PRIVATE_CODE.to_owned()),
            expires_at: operation.expires_at - Duration::minutes(1),
            input_kind: Some(NativeSignInInputKind::AuthorizationCode),
            input_id: Some(step),
        }),
        ..report(WorkspaceSetupStatus::NeedsSignIn)
    }
}

fn test_action(connection_id: Uuid) -> WorkspaceSetupAction {
    WorkspaceSetupAction::Test {
        connection_id,
        configuration: configuration(),
    }
}

async fn connect_runner(
    store: &PgStore,
    corp_id: Uuid,
    runner_id: &str,
    epoch: Uuid,
) -> Result<RunnerRecord> {
    store
        .runner_connected(RunnerConnectInput {
            id: runner_id.to_owned(),
            corp_id,
            hostname: "issue198-disposable-fixture".to_owned(),
            os: "linux".to_owned(),
            capabilities: json!([
                {"name": "workspace-setup-v1", "available": true},
                {"name": "codex", "available": true, "source": source(), "models": [model()]}
            ]),
            connection_epoch: epoch,
        })
        .await
}

async fn fixture(pool: PgPool) -> Result<Fixture> {
    let store = PgStore { pool };
    let (ids, _) = store.bootstrap_demo().await?;
    let runner = connect_runner(
        &store,
        ids.corp_id,
        &format!("issue198-{}", Uuid::new_v4()),
        Uuid::new_v4(),
    )
    .await?;
    Ok(Fixture { store, ids, runner })
}

impl Fixture {
    fn create_input(&self, key: &str) -> CreateWorkspaceConnectionInput {
        CreateWorkspaceConnectionInput {
            corp_id: self.ids.corp_id,
            room_id: self.ids.room_id,
            actor_id: self.ids.alice_actor_id,
            runner_id: self.runner.id.clone(),
            label: "Fixture connection".to_owned(),
            configuration: configuration(),
            idempotency_key: format!("issue198-{key}"),
        }
    }

    fn setup_input(&self, key: &str, action: WorkspaceSetupAction) -> WorkspaceSetupInput {
        WorkspaceSetupInput {
            corp_id: self.ids.corp_id,
            room_id: self.ids.room_id,
            actor_id: self.ids.alice_actor_id,
            runner_id: self.runner.id.clone(),
            action,
            idempotency_key: format!("issue198-{key}"),
        }
    }

    async fn apply(
        &self,
        operation_id: Uuid,
        report: WorkspaceSetupReport,
    ) -> Result<WorkspaceSetupMutation> {
        self.store
            .apply_workspace_setup_report(
                self.ids.corp_id,
                &self.runner.id,
                self.runner.connection_epoch,
                operation_id,
                report,
            )
            .await
    }

    async fn ready(&self, key: &str) -> Result<WorkspaceSetupMutation> {
        let created = self
            .store
            .create_workspace_connection(self.create_input(key))
            .await?;
        self.apply(created.operation.id, ready_report()).await
    }

    async fn pending(&self) -> Result<Vec<crony_domain::WorkspaceSetupCommand>> {
        self.store
            .pending_workspace_setup(
                self.ids.corp_id,
                &self.runner.id,
                self.runner.connection_epoch,
            )
            .await
    }
}

fn connection(mutation: &WorkspaceSetupMutation) -> &WorkspaceConnection {
    mutation.connection.as_ref().expect("linked connection")
}

fn denied<T>(result: Result<T>, expected: &str) {
    let error = match result {
        Ok(_) => panic!("expected denial containing {expected:?}"),
        Err(error) => error,
    };
    let message = format!("{error:#}");
    assert!(message.contains(expected), "unexpected denial: {message}");
}

fn assert_redacted(value: &impl serde::Serialize) {
    let serialized = serde_json::to_string(value).expect("serialize synthetic DTO");
    for canary in [
        "issue198-private-path",
        PRIVATE_LOGIN,
        PRIVATE_CODE,
        "issue198-private-oauth-state",
        PRIVATE_REPOSITORY,
    ] {
        assert!(!serialized.contains(canary), "shared DTO leaked {canary}");
    }
}

fn assert_refresh(mutation: &WorkspaceSetupMutation) {
    assert_eq!(mutation.events.len(), 1);
    let event = &mutation.events[0];
    assert_eq!(event.event_type, "workspace.connection_updated");
    assert_eq!(event.corp_id, mutation.operation.corp_id);
    assert_eq!(event.room_id, Some(mutation.operation.room_id));
    assert_eq!(event.actor_id, Some(mutation.operation.actor_id));
    assert_eq!(event.aggregate_id, connection(mutation).id);
    assert_eq!(event.visibility, "room");
    assert_eq!(
        event.payload,
        json!({
            "connection_id": connection(mutation).id,
            "operation_id": mutation.operation.id,
            "status": mutation.operation.status.as_str()
        })
    );
    assert_redacted(event);
}

async fn events(f: &Fixture) -> Result<Vec<DomainEvent>> {
    f.store
        .events_after(f.ids.corp_id, f.ids.alice_actor_id, 0, 1000)
        .await
}

// Compare actual persisted rows, including private request/epoch fields and
// timestamps. Sequence allocation itself is intentionally not transactional.
async fn persisted_state(f: &Fixture) -> Result<Value> {
    Ok(sqlx::query_scalar(
        r#"
        SELECT jsonb_build_object(
          'connections', (SELECT coalesce(jsonb_agg(to_jsonb(c) ORDER BY c.id), '[]'::jsonb)
                          FROM workspace_connections c WHERE c.corp_id=$1),
          'operations', (SELECT coalesce(jsonb_agg(to_jsonb(o) ORDER BY o.id), '[]'::jsonb)
                         FROM workspace_setup_operations o WHERE o.corp_id=$1),
          'preferences', (SELECT coalesce(jsonb_agg(to_jsonb(p) ORDER BY p.room_id,p.actor_id), '[]'::jsonb)
                          FROM workspace_connection_preferences p WHERE p.corp_id=$1),
          'missions', (SELECT coalesce(jsonb_agg(to_jsonb(m) ORDER BY m.id), '[]'::jsonb)
                       FROM missions m WHERE m.corp_id=$1),
          'tasks', (SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t.id), '[]'::jsonb)
                    FROM tasks t WHERE t.corp_id=$1),
          'runs', (SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY r.id), '[]'::jsonb)
                   FROM runs r WHERE r.corp_id=$1),
          'agents', (SELECT coalesce(jsonb_agg(to_jsonb(a) ORDER BY a.id), '[]'::jsonb)
                     FROM agents a WHERE a.corp_id=$1),
          'events', (SELECT coalesce(jsonb_agg(to_jsonb(e) ORDER BY e.seq), '[]'::jsonb)
                     FROM events e WHERE e.corp_id=$1)
        )
        "#,
    )
    .bind(f.ids.corp_id)
    .fetch_one(&f.store.pool)
    .await?)
}

async fn add_room(store: &PgStore, corp_id: Uuid, members: &[Uuid]) -> Result<Uuid> {
    // No public room/membership creation entry point is available in this store.
    let room_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO rooms(id,corp_id,name,purpose) VALUES($1,$2,'Issue 198 room','Isolation fixture')",
    )
    .bind(room_id)
    .bind(corp_id)
    .execute(&store.pool)
    .await?;
    for actor_id in members {
        sqlx::query("INSERT INTO room_memberships(room_id,actor_id) VALUES($1,$2)")
            .bind(room_id)
            .bind(actor_id)
            .execute(&store.pool)
            .await?;
    }
    Ok(room_id)
}

async fn foreign_scope(store: &PgStore) -> Result<(Uuid, Uuid, Uuid, RunnerRecord)> {
    let corp_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();
    sqlx::query("INSERT INTO corps(id,slug,name) VALUES($1,$2,'Issue 198 foreign Corp')")
        .bind(corp_id)
        .bind(format!("issue198-{corp_id}"))
        .execute(&store.pool)
        .await?;
    sqlx::query(
        "INSERT INTO actors(id,corp_id,name,kind,role) VALUES($1,$2,'Foreign owner','human','owner')",
    )
    .bind(actor_id)
    .bind(corp_id)
    .execute(&store.pool)
    .await?;
    let room_id = add_room(store, corp_id, &[actor_id]).await?;
    let runner = connect_runner(
        store,
        corp_id,
        &format!("issue198-foreign-{}", Uuid::new_v4()),
        Uuid::new_v4(),
    )
    .await?;
    Ok((corp_id, room_id, actor_id, runner))
}

async fn expire_operation(f: &Fixture, operation_id: Uuid) -> Result<()> {
    // Shift both timestamps to preserve the migration's expires_at > created_at
    // invariant; no sleep, real clock wait or terminal-state fabrication.
    sqlx::query(
        "UPDATE workspace_setup_operations
         SET created_at=now()-interval '20 minutes',expires_at=now()-interval '5 minutes'
         WHERE corp_id=$1 AND id=$2",
    )
    .bind(f.ids.corp_id)
    .bind(operation_id)
    .execute(&f.store.pool)
    .await?;
    Ok(())
}

fn plan(f: &Fixture, connection_id: Uuid) -> TaskGraphPlan {
    let source = source();
    TaskGraphPlan {
        strategy: "single".to_owned(),
        max_nodes: 2,
        max_depth: 1,
        budget_tokens: 1000,
        budget_cost_microusd: 1_000_000,
        staffing: vec![],
        tasks: vec![PlannedTask {
            key: "fixture".to_owned(),
            title: "Store-only connection binding".to_owned(),
            assigned_agent_id: f.ids.codex_agent_id,
            required_adapter: "codex".to_owned(),
            depends_on: vec![],
            depth: 0,
            max_attempts: 1,
            contract: TaskContract {
                objective: "Persist a bound launch without executing it".to_owned(),
                expected_output: "result.md".to_owned(),
                source_repository: Some(source.repository),
                source_base_ref: Some(source.base_ref),
                source_base_commit: Some(source.base_commit),
                workspace_connection_id: Some(connection_id),
                acceptance_tests: vec!["Connection binding is retained".to_owned()],
                allowed_tools: vec!["filesystem".to_owned()],
                prohibited_actions: vec!["No external effects".to_owned()],
                references: vec![],
                write_scope: vec!["result.md".to_owned()],
                budget_tokens: 1000,
                budget_cost_microusd: 1_000_000,
                deadline_at: None,
                escalation: "Ask the fixture owner".to_owned(),
                secret_refs: vec![],
                model: Some(model().id),
                reasoning_effort: None,
                deliverable: None,
            },
            verification_policy: VerificationPolicy {
                checks: vec![VerifierCheck::File {
                    path: "result.md".to_owned(),
                    min_bytes: 1,
                }],
                manual_gate: None,
            },
        }],
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue198_create_replay_normalizes_and_rejects_changed_requests(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let mut request = f.create_input("create");
    request.label = format!("  {}  ", request.label);
    request.runner_id = format!(" {} ", request.runner_id);
    request.idempotency_key = format!(" {} ", request.idempotency_key);
    if let WorkspaceRepositorySetup::GitHub {
        repository,
        base_ref,
        ..
    } = &mut request.configuration.repository
    {
        *repository = "https://github.com/Fixture/Project.git".to_owned();
        *base_ref = " main ".to_owned();
    }
    let created = f.store.create_workspace_connection(request.clone()).await?;
    assert!(!created.replayed);
    assert_eq!(created.operation.status, WorkspaceSetupStatus::Queued);
    assert_eq!(
        connection(&created).status,
        WorkspaceConnectionStatus::Connecting
    );
    assert_eq!(connection(&created).version, 1);
    assert_eq!(connection(&created).label, "Fixture connection");
    assert_refresh(&created);
    let (_, saved) = f
        .store
        .workspace_connection_settings(f.ids.corp_id, f.ids.alice_actor_id, connection(&created).id)
        .await?;
    assert_eq!(saved, configuration());
    let before = persisted_state(&f).await?;

    for replay_input in [request, f.create_input("create")] {
        let replay = f.store.create_workspace_connection(replay_input).await?;
        assert!(replay.replayed);
        assert!(replay.events.is_empty());
        assert_eq!(replay.operation.id, created.operation.id);
        assert_eq!(connection(&replay).id, connection(&created).id);
        assert_eq!(persisted_state(&f).await?, before);
    }
    let mut changed_label = f.create_input("create");
    changed_label.label = "A different request".to_owned();
    let mut changed_agent = f.create_input("create");
    changed_agent.configuration.agent = CodingAgent::ClaudeCode;
    let mut changed_source = f.create_input("create");
    if let WorkspaceRepositorySetup::GitHub { base_ref, .. } =
        &mut changed_source.configuration.repository
    {
        *base_ref = "release".to_owned();
    }
    for changed in [changed_label, changed_agent, changed_source] {
        denied(
            f.store.create_workspace_connection(changed).await,
            "different settings",
        );
        assert_eq!(persisted_state(&f).await?, before);
    }
    let pending = f.pending().await?;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].operation_id, created.operation.id);
    assert_eq!(pending[0].expected_connection_version, Some(1));
    assert_eq!(pending[0].connection_owner_id, Some(f.ids.alice_actor_id));
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue198_connections_enforce_actor_room_and_corp_isolation(pool: PgPool) -> Result<()> {
    let f = fixture(pool).await?;
    let alice = f
        .store
        .create_workspace_connection(f.create_input("same-key"))
        .await?;
    let mut bob_input = f.create_input("same-key");
    bob_input.actor_id = f.ids.bob_actor_id;
    let bob = f.store.create_workspace_connection(bob_input).await?;
    assert_ne!(bob.operation.id, alice.operation.id);
    assert_ne!(connection(&bob).id, connection(&alice).id);
    assert!(!bob.replayed);

    let private_room = add_room(&f.store, f.ids.corp_id, &[f.ids.alice_actor_id]).await?;
    let mut private_input = f.create_input("other-room");
    private_input.room_id = private_room;
    let private = f.store.create_workspace_connection(private_input).await?;
    let (foreign_corp, foreign_room, foreign_actor, foreign_runner) =
        foreign_scope(&f.store).await?;
    let mut foreign_input = f.create_input("same-key");
    foreign_input.corp_id = foreign_corp;
    foreign_input.room_id = foreign_room;
    foreign_input.actor_id = foreign_actor;
    foreign_input.runner_id = foreign_runner.id.clone();
    let foreign = f.store.create_workspace_connection(foreign_input).await?;
    let before = persisted_state(&f).await?;

    let bob_view = f
        .store
        .workspace_connections(f.ids.corp_id, f.ids.room_id, f.ids.bob_actor_id)
        .await?;
    assert_eq!(bob_view.connections.len(), 2);
    assert_eq!(bob_view.operations.len(), 1);
    assert_eq!(bob_view.operations[0].id, bob.operation.id);
    for (corp_id, actor_id, operation_id) in [
        (f.ids.corp_id, f.ids.bob_actor_id, alice.operation.id),
        (f.ids.corp_id, f.ids.alice_actor_id, bob.operation.id),
        (foreign_corp, foreign_actor, alice.operation.id),
        (f.ids.corp_id, f.ids.alice_actor_id, foreign.operation.id),
    ] {
        assert!(
            f.store
                .workspace_setup_operation(corp_id, actor_id, operation_id)
                .await?
                .is_none()
        );
    }
    for (corp_id, room_id, actor_id) in [
        (f.ids.corp_id, private_room, f.ids.bob_actor_id),
        (f.ids.corp_id, f.ids.room_id, foreign_actor),
        (foreign_corp, foreign_room, f.ids.alice_actor_id),
    ] {
        denied(
            f.store
                .workspace_connections(corp_id, room_id, actor_id)
                .await,
            "forbidden",
        );
    }
    denied(
        f.store
            .workspace_connection_settings(
                f.ids.corp_id,
                f.ids.bob_actor_id,
                connection(&private).id,
            )
            .await,
        "forbidden",
    );
    denied(
        f.store
            .select_workspace_connection(
                f.ids.corp_id,
                f.ids.room_id,
                f.ids.alice_actor_id,
                connection(&private).id,
            )
            .await,
        "another room",
    );
    denied(
        f.store
            .select_workspace_connection(
                foreign_corp,
                foreign_room,
                foreign_actor,
                connection(&alice).id,
            )
            .await,
        "not found",
    );
    let mut wrong_room = f.setup_input("wrong-room", test_action(connection(&private).id));
    wrong_room.room_id = f.ids.room_id;
    denied(
        f.store.begin_workspace_setup(wrong_room).await,
        "another room or machine",
    );
    let visible = f
        .store
        .visible_workspace_connections(f.ids.corp_id, f.ids.bob_actor_id)
        .await?;
    assert_eq!(visible.len(), 2);
    assert!(visible.contains(&connection(&alice).id));
    assert!(!visible.contains(&connection(&private).id));
    assert!(!visible.contains(&connection(&foreign).id));
    assert!(
        f.store
            .visible_workspace_connections(foreign_corp, f.ids.alice_actor_id)
            .await?
            .is_empty()
    );
    let bindings = f
        .store
        .workspace_runtime_bindings(
            f.ids.corp_id,
            &f.runner.id,
            &[connection(&alice).id, connection(&foreign).id],
        )
        .await?;
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].id, connection(&alice).id);
    assert!(
        f.store
            .workspace_runtime_bindings(
                f.ids.corp_id,
                &foreign_runner.id,
                &[connection(&alice).id],
            )
            .await?
            .is_empty()
    );
    for viewer in [f.ids.bob_actor_id, f.ids.eve_actor_id, foreign_actor] {
        let replay = f.store.events_after(f.ids.corp_id, viewer, 0, 1000).await?;
        assert!(
            replay
                .iter()
                .all(|event| event.aggregate_id != connection(&private).id)
        );
        if viewer != f.ids.bob_actor_id {
            assert!(
                replay
                    .iter()
                    .all(|event| event.event_type != "workspace.connection_updated")
            );
        }
    }
    assert_eq!(persisted_state(&f).await?, before);
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue198_native_setup_requires_current_human_and_machine_authority(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    sqlx::query("INSERT INTO room_memberships(room_id,actor_id) VALUES($1,$2)")
        .bind(f.ids.room_id)
        .bind(f.ids.eve_actor_id)
        .execute(&f.store.pool)
        .await?;
    let agent_actor: Uuid = sqlx::query_scalar("SELECT actor_id FROM agents WHERE id=$1")
        .bind(f.ids.codex_agent_id)
        .fetch_one(&f.store.pool)
        .await?;
    let before = persisted_state(&f).await?;
    for actor in [f.ids.eve_actor_id, agent_actor] {
        let mut input = f.create_input("not-human-operator");
        input.actor_id = actor;
        denied(
            f.store.create_workspace_connection(input).await,
            "human room operator",
        );
    }
    let mut local = configuration();
    local.repository = WorkspaceRepositorySetup::Local {
        directory: PRIVATE_DIRECTORY.to_owned(),
        base_ref: "main".to_owned(),
    };
    let mut machine = configuration();
    if let WorkspaceRepositorySetup::GitHub { account, .. } = &mut machine.repository {
        *account = GitHubAccountSource::Machine;
    }
    let mut system = configuration();
    system.use_system_installation = true;
    for config in [local, machine, system] {
        let mut input = f.create_input("machine-authority");
        input.actor_id = f.ids.bob_actor_id;
        input.configuration = config;
        denied(
            f.store.create_workspace_connection(input).await,
            "owner or admin",
        );
    }
    let mut inspect = f.setup_input(
        "machine-account",
        WorkspaceSetupAction::InspectGitHub {
            account: GitHubAccountSource::Machine,
        },
    );
    inspect.actor_id = f.ids.bob_actor_id;
    denied(
        f.store.begin_workspace_setup(inspect).await,
        "owner or admin",
    );
    assert_eq!(persisted_state(&f).await?, before);

    let mut input = f.create_input("personal");
    input.actor_id = f.ids.bob_actor_id;
    let personal = f.store.create_workspace_connection(input).await?;
    assert_eq!(personal.operation.actor_id, f.ids.bob_actor_id);
    // Membership is rechecked at durable dispatch, not only when queued.
    sqlx::query("DELETE FROM room_memberships WHERE room_id=$1 AND actor_id=$2")
        .bind(f.ids.room_id)
        .bind(f.ids.bob_actor_id)
        .execute(&f.store.pool)
        .await?;
    assert!(f.pending().await?.is_empty());
    let view = f
        .store
        .workspace_connections(f.ids.corp_id, f.ids.room_id, f.ids.alice_actor_id)
        .await?;
    assert_eq!(view.connections.len(), 1);
    assert_eq!(
        view.connections[0].status,
        WorkspaceConnectionStatus::Failed
    );
    denied(
        f.store
            .workspace_setup_operation(f.ids.corp_id, f.ids.bob_actor_id, personal.operation.id)
            .await,
        "forbidden",
    );
    let failed = persisted_state(&f).await?;
    denied(
        f.apply(personal.operation.id, ready_report()).await,
        "different terminal result",
    );
    assert_eq!(persisted_state(&f).await?, failed);
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue198_sign_in_and_catalogue_are_private_but_connection_events_are_redacted(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let mut input = f.create_input("private-sign-in");
    input.configuration.agent = CodingAgent::ClaudeCode;
    input.configuration.repository = WorkspaceRepositorySetup::Local {
        directory: PRIVATE_DIRECTORY.to_owned(),
        base_ref: "main".to_owned(),
    };
    let created = f.store.create_workspace_connection(input).await?;
    assert_refresh(&created);
    let private_report = sign_in_report(&created.operation, Uuid::new_v4());
    let waiting = f
        .apply(created.operation.id, private_report.clone())
        .await?;
    assert_refresh(&waiting);
    let discovery = f
        .store
        .begin_workspace_setup(f.setup_input(
            "catalogue",
            WorkspaceSetupAction::ListGitHubRepositories {
                account: GitHubAccountSource::Personal,
            },
        ))
        .await?;
    assert!(discovery.connection.is_none());
    assert!(discovery.events.is_empty());
    let mut catalogue = report(WorkspaceSetupStatus::Succeeded);
    catalogue.account_login = Some(PRIVATE_LOGIN.to_owned());
    catalogue.repositories.push(GitHubRepositoryChoice {
        id: "issue198-private-catalogue-id".to_owned(),
        repository: PRIVATE_REPOSITORY.to_owned(),
        default_branch: "main".to_owned(),
        private: true,
        can_push: false,
    });
    let discovered = f.apply(discovery.operation.id, catalogue.clone()).await?;
    assert!(discovered.connection.is_none());
    assert!(discovered.events.is_empty());

    for (operation_id, expected_report) in [
        (created.operation.id, private_report),
        (discovery.operation.id, catalogue),
    ] {
        let own = f
            .store
            .workspace_setup_operation(f.ids.corp_id, f.ids.alice_actor_id, operation_id)
            .await?
            .context("owner's private operation")?;
        assert_eq!(serde_json::to_value(own.report)?, json!(expected_report));
        assert!(
            f.store
                .workspace_setup_operation(f.ids.corp_id, f.ids.bob_actor_id, operation_id)
                .await?
                .is_none()
        );
    }
    let owner = f
        .store
        .workspace_connections(f.ids.corp_id, f.ids.room_id, f.ids.alice_actor_id)
        .await?;
    assert_eq!(owner.operations.len(), 2);
    assert_redacted(&owner.connections);
    let other = f
        .store
        .workspace_connections(f.ids.corp_id, f.ids.room_id, f.ids.bob_actor_id)
        .await?;
    assert!(other.operations.is_empty());
    assert_eq!(other.connections.len(), 1);
    assert_eq!(
        other.connections[0].status,
        WorkspaceConnectionStatus::NeedsSignIn
    );
    assert_redacted(&other);
    assert_redacted(
        &f.store
            .events_after(f.ids.corp_id, f.ids.bob_actor_id, 0, 1000)
            .await?,
    );
    let pending = f.pending().await?;
    assert_eq!(pending.len(), 1);
    assert!(
        serde_json::to_string(&pending[0].action)?.contains(PRIVATE_DIRECTORY),
        "the private native command, unlike the public DTO, retains its selected path"
    );
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue198_shared_connection_does_not_echo_private_native_diagnostics(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let mut input = f.create_input("diagnostic-redaction");
    input.configuration.agent = CodingAgent::ClaudeCode;
    input.configuration.repository = WorkspaceRepositorySetup::Local {
        directory: PRIVATE_DIRECTORY.to_owned(),
        base_ref: "main".to_owned(),
    };
    let created = f.store.create_workspace_connection(input).await?;
    let mut private_report = sign_in_report(&created.operation, Uuid::new_v4());
    private_report.detail = format!(
        "Native diagnostic: {PRIVATE_DIRECTORY}; account {PRIVATE_LOGIN}; \
         code {PRIVATE_CODE}; URL {PRIVATE_URI}"
    );
    let applied = f
        .apply(created.operation.id, private_report.clone())
        .await?;
    assert_refresh(&applied);
    let own = f
        .store
        .workspace_setup_operation(f.ids.corp_id, f.ids.alice_actor_id, created.operation.id)
        .await?
        .context("private diagnostic operation")?;
    assert_eq!(
        own.report.context("private report")?.detail,
        private_report.detail
    );
    let shared = f
        .store
        .workspace_connections(f.ids.corp_id, f.ids.room_id, f.ids.bob_actor_id)
        .await?;
    assert!(shared.operations.is_empty());
    // Regression: copying report.detail verbatim into the shared connection leaks
    // actor-private diagnostics despite the correctly redacted event envelope.
    assert_redacted(&shared);
    assert_redacted(connection(&applied));
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue198_checks_use_saved_configuration_and_sign_in_stays_owner_bound(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let ready = f.ready("saved-settings").await?;
    let id = connection(&ready).id;
    let mut sign_in = f.setup_input(
        "foreign-sign-in",
        WorkspaceSetupAction::SignInAgent {
            connection_id: id,
            configuration: configuration(),
        },
    );
    sign_in.actor_id = f.ids.bob_actor_id;
    let before = persisted_state(&f).await?;
    denied(
        f.store.begin_workspace_setup(sign_in).await,
        "only the connection owner",
    );
    assert_eq!(persisted_state(&f).await?, before);

    let mut supplied = configuration();
    supplied.agent = CodingAgent::ClaudeCode;
    supplied.repository = WorkspaceRepositorySetup::Local {
        directory: PRIVATE_DIRECTORY.to_owned(),
        base_ref: "release".to_owned(),
    };
    let mut input = f.setup_input(
        "saved-check",
        WorkspaceSetupAction::Test {
            connection_id: id,
            configuration: supplied,
        },
    );
    input.actor_id = f.ids.bob_actor_id;
    let check = f.store.begin_workspace_setup(input.clone()).await?;
    assert!(!check.replayed);
    assert_eq!(connection(&check).version, connection(&ready).version + 1);
    let pending = f.pending().await?;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].actor_id, f.ids.bob_actor_id);
    assert_eq!(pending[0].connection_owner_id, Some(f.ids.alice_actor_id));
    assert_eq!(pending[0].action, test_action(id));
    let queued = persisted_state(&f).await?;
    let replay = f.store.begin_workspace_setup(input.clone()).await?;
    assert!(replay.replayed);
    assert_eq!(replay.operation.id, check.operation.id);
    assert!(replay.events.is_empty());
    assert_eq!(persisted_state(&f).await?, queued);

    input.action = WorkspaceSetupAction::InspectGitHub {
        account: GitHubAccountSource::Personal,
    };
    denied(
        f.store.begin_workspace_setup(input).await,
        "different settings",
    );
    assert_eq!(persisted_state(&f).await?, queued);
    f.apply(check.operation.id, ready_report()).await?;
    let own_sign_in = f
        .store
        .begin_workspace_setup(f.setup_input(
            "owner-sign-in",
            WorkspaceSetupAction::SignInAgent {
                connection_id: id,
                configuration: configuration(),
            },
        ))
        .await?;
    assert_eq!(own_sign_in.operation.actor_id, f.ids.alice_actor_id);
    assert_eq!(own_sign_in.operation.kind, "sign_in_agent");
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue198_wrong_source_reports_leave_connection_operation_and_events_unchanged(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    for advertised in [false, true] {
        let mut input = f.create_input(if advertised { "advertised" } else { "github" });
        if advertised {
            input.configuration.repository =
                WorkspaceRepositorySetup::Advertised { source: source() };
        }
        let created = f.store.create_workspace_connection(input).await?;
        let before = persisted_state(&f).await?;
        // GitHub resolves the selected ref during setup. Advertised sources also
        // pin the exact commit, so only that variant rejects a different commit.
        let source_fields = if advertised { 4 } else { 3 };
        for field in 0..source_fields {
            let mut wrong = ready_report();
            let wrong_source = wrong.source.as_mut().context("fixture source")?;
            match field {
                0 => wrong_source.repository = "fixture/unselected".to_owned(),
                1 => wrong_source.repository_id = Some("issue198-wrong-identity".to_owned()),
                2 => wrong_source.base_ref = "release".to_owned(),
                _ => wrong_source.base_commit = "b".repeat(40),
            }
            denied(f.apply(created.operation.id, wrong).await, "setup result");
            assert_eq!(persisted_state(&f).await?, before);
        }
        let mut incomplete = ready_report();
        incomplete.source = None;
        denied(
            f.apply(created.operation.id, incomplete).await,
            "completed native source and agent checks",
        );
        assert_eq!(persisted_state(&f).await?, before);
        let accepted = f.apply(created.operation.id, ready_report()).await?;
        assert_eq!(connection(&accepted).source, Some(source()));
        assert_eq!(connection(&accepted).models, vec![model()]);
        assert_eq!(
            connection(&accepted).status,
            WorkspaceConnectionStatus::Ready
        );
        assert!(connection(&accepted).last_checked_at.is_some());
    }
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue198_setup_reports_and_delivery_reject_wrong_runner_and_stale_epoch(
    pool: PgPool,
) -> Result<()> {
    let mut f = fixture(pool).await?;
    let created = f
        .store
        .create_workspace_connection(f.create_input("runner-fences"))
        .await?;
    let other = connect_runner(
        &f.store,
        f.ids.corp_id,
        &format!("issue198-other-{}", Uuid::new_v4()),
        Uuid::new_v4(),
    )
    .await?;
    let before = persisted_state(&f).await?;
    assert!(
        f.store
            .pending_workspace_setup(f.ids.corp_id, &other.id, other.connection_epoch)
            .await?
            .is_empty()
    );
    denied(
        f.store
            .apply_workspace_setup_report(
                f.ids.corp_id,
                &other.id,
                other.connection_epoch,
                created.operation.id,
                ready_report(),
            )
            .await,
        "does not belong to this runner",
    );
    denied(
        f.store
            .apply_workspace_setup_report(
                Uuid::new_v4(),
                &f.runner.id,
                f.runner.connection_epoch,
                created.operation.id,
                ready_report(),
            )
            .await,
        "stale or foreign setup runner",
    );
    let old_epoch = f.runner.connection_epoch;
    let original_commands = f.pending().await?;
    assert_eq!(original_commands.len(), 1);
    let reconnected = connect_runner(&f.store, f.ids.corp_id, &f.runner.id, Uuid::new_v4()).await?;
    f.runner = reconnected;
    assert!(
        f.store
            .pending_workspace_setup(f.ids.corp_id, &f.runner.id, old_epoch)
            .await?
            .is_empty()
    );
    denied(
        f.store
            .apply_workspace_setup_report(
                f.ids.corp_id,
                &f.runner.id,
                old_epoch,
                created.operation.id,
                ready_report(),
            )
            .await,
        "stale or foreign setup runner",
    );
    assert_eq!(persisted_state(&f).await?, before);
    let commands = f.pending().await?;
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].operation_id, original_commands[0].operation_id);
    assert_eq!(commands[0].action, original_commands[0].action);
    let accepted = f.apply(created.operation.id, ready_report()).await?;
    assert_eq!(accepted.operation.status, WorkspaceSetupStatus::Succeeded);
    let stored_epoch: Option<Uuid> = sqlx::query_scalar(
        "SELECT dispatch_epoch FROM workspace_setup_operations WHERE corp_id=$1 AND id=$2",
    )
    .bind(f.ids.corp_id)
    .bind(created.operation.id)
    .fetch_one(&f.store.pool)
    .await?;
    assert_eq!(stored_epoch, Some(f.runner.connection_epoch));
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue198_duplicate_reports_do_not_duplicate_events_or_replace_terminal_results(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let created = f
        .store
        .create_workspace_connection(f.create_input("duplicate-reports"))
        .await?;
    let progress = report(WorkspaceSetupStatus::Running);
    let running = f.apply(created.operation.id, progress.clone()).await?;
    assert_refresh(&running);
    let journal = serde_json::to_value(events(&f).await?)?;
    let duplicate_progress = f.apply(created.operation.id, progress.clone()).await?;
    assert!(duplicate_progress.events.is_empty());
    assert_eq!(connection(&duplicate_progress).version, 1);
    assert_eq!(serde_json::to_value(events(&f).await?)?, journal);

    let terminal = ready_report();
    let completed = f.apply(created.operation.id, terminal.clone()).await?;
    assert_refresh(&completed);
    let before = persisted_state(&f).await?;
    let replay = f.apply(created.operation.id, terminal.clone()).await?;
    assert!(replay.replayed);
    assert!(replay.events.is_empty());
    assert_eq!(replay.operation.id, completed.operation.id);
    assert_eq!(persisted_state(&f).await?, before);
    let mut changed_terminal = terminal;
    changed_terminal.detail = "A different terminal answer".to_owned();
    for late in [
        changed_terminal,
        progress,
        report(WorkspaceSetupStatus::Failed),
        report(WorkspaceSetupStatus::NeedsSignIn),
    ] {
        denied(
            f.apply(created.operation.id, late).await,
            "different terminal result",
        );
        assert_eq!(persisted_state(&f).await?, before);
    }
    for prohibited in [
        WorkspaceSetupStatus::Queued,
        WorkspaceSetupStatus::Cancelled,
    ] {
        denied(
            f.apply(created.operation.id, report(prohibited)).await,
            "runner cannot assert",
        );
        assert_eq!(persisted_state(&f).await?, before);
    }
    assert!(f.pending().await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue198_offline_selection_is_saved_per_actor_without_admitting_a_plan(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let ready = f.ready("offline-preference").await?;
    let id = connection(&ready).id;
    f.store
        .runner_disconnected(&f.runner.id, f.runner.connection_epoch, 60)
        .await?;
    f.store
        .select_workspace_connection(f.ids.corp_id, f.ids.room_id, f.ids.alice_actor_id, id)
        .await?;
    // A new store handle demonstrates database persistence, not client memory.
    let reader = PgStore {
        pool: f.store.pool.clone(),
    };
    let saved = reader
        .workspace_connections(f.ids.corp_id, f.ids.room_id, f.ids.alice_actor_id)
        .await?;
    assert_eq!(saved.selected_connection_id, Some(id));
    assert_eq!(saved.connections[0].id, id);
    assert!(!saved.connections[0].runner_connected);
    assert_eq!(saved.connections[0].source, Some(source()));
    let bob = reader
        .workspace_connections(f.ids.corp_id, f.ids.room_id, f.ids.bob_actor_id)
        .await?;
    assert_eq!(bob.selected_connection_id, None);
    let before = persisted_state(&f).await?;
    denied(
        reader
            .validate_mission_creation(
                f.ids.corp_id,
                f.ids.alice_actor_id,
                "Offline plan",
                "Metadata only",
                &plan(&f, id),
            )
            .await,
        "not ready",
    );
    assert_eq!(persisted_state(&f).await?, before);
    connect_runner(&f.store, f.ids.corp_id, &f.runner.id, Uuid::new_v4()).await?;
    let reconnected = reader
        .workspace_connections(f.ids.corp_id, f.ids.room_id, f.ids.alice_actor_id)
        .await?;
    assert_eq!(reconnected.selected_connection_id, Some(id));
    assert!(reconnected.connections[0].runner_connected);
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue198_stale_success_and_expired_checks_cannot_revive_a_newer_failed_connection(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let original = f.ready("stale-check").await?;
    let id = connection(&original).id;
    let newer = f
        .store
        .begin_workspace_setup(f.setup_input("newer-check", test_action(id)))
        .await?;
    let failed = f
        .apply(newer.operation.id, report(WorkspaceSetupStatus::Failed))
        .await?;
    assert_eq!(
        connection(&failed).version,
        connection(&original).version + 1
    );
    assert_eq!(
        connection(&failed).status,
        WorkspaceConnectionStatus::Failed
    );
    let before = persisted_state(&f).await?;
    let stale_replay = f.apply(original.operation.id, ready_report()).await?;
    assert!(stale_replay.replayed);
    assert!(stale_replay.events.is_empty());
    assert_eq!(
        connection(&stale_replay).version,
        connection(&failed).version
    );
    assert_eq!(
        connection(&stale_replay).status,
        WorkspaceConnectionStatus::Failed
    );
    assert_eq!(persisted_state(&f).await?, before);

    let expiring = f
        .store
        .begin_workspace_setup(f.setup_input("expiring-check", test_action(id)))
        .await?;
    expire_operation(&f, expiring.operation.id).await?;
    // Native admission expires the old operation and creates the new generation.
    let newest = f
        .store
        .begin_workspace_setup(f.setup_input("newest-check", test_action(id)))
        .await?;
    assert_eq!(
        connection(&newest).version,
        connection(&expiring).version + 1
    );
    let newest_failed = f
        .apply(newest.operation.id, report(WorkspaceSetupStatus::Failed))
        .await?;
    let before = persisted_state(&f).await?;
    denied(
        f.apply(expiring.operation.id, ready_report()).await,
        "different terminal result",
    );
    let old = f
        .store
        .workspace_setup_operation(f.ids.corp_id, f.ids.alice_actor_id, expiring.operation.id)
        .await?
        .context("expired operation")?;
    assert_eq!(old.status, WorkspaceSetupStatus::Failed);
    assert!(
        old.report
            .context("expiry report")?
            .detail
            .contains("expired")
    );
    assert!(f.pending().await?.is_empty());
    let view = f
        .store
        .workspace_connections(f.ids.corp_id, f.ids.room_id, f.ids.alice_actor_id)
        .await?;
    assert_eq!(
        view.connections[0].status,
        WorkspaceConnectionStatus::Failed
    );
    assert_eq!(
        view.connections[0].version,
        connection(&newest_failed).version
    );
    assert_eq!(persisted_state(&f).await?, before);
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue198_native_sign_in_input_is_actor_step_and_terminal_fenced(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let mut input = f.create_input("native-input");
    input.configuration.agent = CodingAgent::ClaudeCode;
    let created = f.store.create_workspace_connection(input).await?;
    let first_step = Uuid::new_v4();
    f.apply(
        created.operation.id,
        sign_in_report(&created.operation, first_step),
    )
    .await?;
    let authorized = f
        .store
        .workspace_sign_in_operation(
            f.ids.corp_id,
            f.ids.alice_actor_id,
            created.operation.id,
            first_step,
        )
        .await?;
    assert_eq!(authorized.id, created.operation.id);
    let before = persisted_state(&f).await?;
    for (corp_id, actor_id, operation_id) in [
        (f.ids.corp_id, f.ids.bob_actor_id, created.operation.id),
        (Uuid::new_v4(), f.ids.alice_actor_id, created.operation.id),
        (f.ids.corp_id, f.ids.alice_actor_id, Uuid::new_v4()),
    ] {
        denied(
            f.store
                .workspace_sign_in_operation(corp_id, actor_id, operation_id, first_step)
                .await,
            "not found",
        );
    }
    denied(
        f.store
            .workspace_sign_in_operation(
                f.ids.corp_id,
                f.ids.alice_actor_id,
                created.operation.id,
                Uuid::new_v4(),
            )
            .await,
        "changed or expired",
    );
    assert_eq!(persisted_state(&f).await?, before);

    let next_step = Uuid::new_v4();
    f.apply(
        created.operation.id,
        sign_in_report(&created.operation, next_step),
    )
    .await?;
    denied(
        f.store
            .workspace_sign_in_operation(
                f.ids.corp_id,
                f.ids.alice_actor_id,
                created.operation.id,
                first_step,
            )
            .await,
        "changed or expired",
    );
    f.store
        .workspace_sign_in_operation(
            f.ids.corp_id,
            f.ids.alice_actor_id,
            created.operation.id,
            next_step,
        )
        .await?;
    f.apply(created.operation.id, ready_report()).await?;
    let terminal = persisted_state(&f).await?;
    denied(
        f.store
            .workspace_sign_in_operation(
                f.ids.corp_id,
                f.ids.alice_actor_id,
                created.operation.id,
                next_step,
            )
            .await,
        "not waiting",
    );
    assert_eq!(persisted_state(&f).await?, terminal);
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue198_native_sign_in_input_obeys_instruction_and_operation_expiry(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let mut input = f.create_input("native-input-expiry");
    input.configuration.agent = CodingAgent::ClaudeCode;
    let created = f.store.create_workspace_connection(input).await?;
    let step = Uuid::new_v4();
    let valid = sign_in_report(&created.operation, step);
    let mut overlong = valid.clone();
    overlong.sign_in.as_mut().context("instruction")?.expires_at =
        created.operation.expires_at + Duration::seconds(1);
    let before = persisted_state(&f).await?;
    denied(
        f.apply(created.operation.id, overlong).await,
        "outside this operation's lifetime",
    );
    assert_eq!(persisted_state(&f).await?, before);
    f.apply(created.operation.id, valid.clone()).await?;
    // Deterministic passage of the instruction deadline, without sleeping.
    sqlx::query(
        "UPDATE workspace_setup_operations
         SET report=jsonb_set(report,'{sign_in,expires_at}',to_jsonb(now()-interval '1 second'))
         WHERE corp_id=$1 AND id=$2",
    )
    .bind(f.ids.corp_id)
    .bind(created.operation.id)
    .execute(&f.store.pool)
    .await?;
    let expired_instruction = persisted_state(&f).await?;
    denied(
        f.store
            .workspace_sign_in_operation(
                f.ids.corp_id,
                f.ids.alice_actor_id,
                created.operation.id,
                step,
            )
            .await,
        "changed or expired",
    );
    assert_eq!(persisted_state(&f).await?, expired_instruction);
    f.apply(created.operation.id, valid).await?;
    expire_operation(&f, created.operation.id).await?;
    let expired_operation = persisted_state(&f).await?;
    denied(
        f.store
            .workspace_sign_in_operation(
                f.ids.corp_id,
                f.ids.alice_actor_id,
                created.operation.id,
                step,
            )
            .await,
        "changed or expired",
    );
    assert_eq!(persisted_state(&f).await?, expired_operation);
    assert!(f.pending().await?.is_empty());
    let expired = f
        .store
        .workspace_setup_operation(f.ids.corp_id, f.ids.alice_actor_id, created.operation.id)
        .await?
        .context("expired private operation")?;
    assert_eq!(expired.status, WorkspaceSetupStatus::Failed);
    assert!(expired.report.context("expiry report")?.sign_in.is_none());
    Ok(())
}

async fn reject_workspace_events(store: &PgStore) -> Result<()> {
    // A late database fault proves writes and the shared journal commit together.
    // NOT VALID leaves prior fixture events intact while rejecting new inserts.
    sqlx::query(
        "ALTER TABLE events ADD CONSTRAINT issue198_fail_workspace_event
         CHECK (type <> 'workspace.connection_updated') NOT VALID",
    )
    .execute(&store.pool)
    .await?;
    Ok(())
}

async fn allow_workspace_events(store: &PgStore) -> Result<()> {
    sqlx::query("ALTER TABLE events DROP CONSTRAINT issue198_fail_workspace_event")
        .execute(&store.pool)
        .await?;
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue198_create_check_and_report_roll_back_on_late_event_failure(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let before = persisted_state(&f).await?;
    reject_workspace_events(&f.store).await?;
    denied(
        f.store
            .create_workspace_connection(f.create_input("create-rollback"))
            .await,
        "issue198_fail_workspace_event",
    );
    assert_eq!(persisted_state(&f).await?, before);
    allow_workspace_events(&f.store).await?;
    let created = f
        .store
        .create_workspace_connection(f.create_input("create-rollback"))
        .await?;
    assert!(!created.replayed);
    assert_eq!(connection(&created).version, 1);
    assert_refresh(&created);

    let before = persisted_state(&f).await?;
    reject_workspace_events(&f.store).await?;
    denied(
        f.apply(created.operation.id, ready_report()).await,
        "issue198_fail_workspace_event",
    );
    assert_eq!(persisted_state(&f).await?, before);
    allow_workspace_events(&f.store).await?;
    let ready = f.apply(created.operation.id, ready_report()).await?;
    assert!(!ready.replayed);
    assert_eq!(connection(&ready).status, WorkspaceConnectionStatus::Ready);
    assert_refresh(&ready);

    let before = persisted_state(&f).await?;
    let input = f.setup_input("check-rollback", test_action(connection(&ready).id));
    reject_workspace_events(&f.store).await?;
    denied(
        f.store.begin_workspace_setup(input.clone()).await,
        "issue198_fail_workspace_event",
    );
    assert_eq!(persisted_state(&f).await?, before);
    allow_workspace_events(&f.store).await?;
    let check = f.store.begin_workspace_setup(input).await?;
    assert!(!check.replayed);
    assert_eq!(connection(&check).version, connection(&ready).version + 1);
    assert_refresh(&check);
    let pending = f.pending().await?;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].operation_id, check.operation.id);
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue198_native_plan_and_run_retain_connection_room_source_and_machine(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let first_room = f.ready("first-room").await?;
    let selected_room = add_room(&f.store, f.ids.corp_id, &[f.ids.alice_actor_id]).await?;
    let mut input = f.create_input("selected-room");
    input.room_id = selected_room;
    let created = f.store.create_workspace_connection(input).await?;
    let selected = f.apply(created.operation.id, ready_report()).await?;
    let selected_id = connection(&selected).id;
    let bound = plan(&f, selected_id);
    let before = persisted_state(&f).await?;

    for field in 0..4 {
        let mut invalid = bound.clone();
        match field {
            0 => {
                invalid.tasks[0].contract.source_repository = Some("fixture/unselected".to_owned());
            }
            1 => invalid.tasks[0].contract.source_base_ref = Some("release".to_owned()),
            2 => invalid.tasks[0].contract.source_base_commit = Some("b".repeat(40)),
            _ => invalid.tasks[0].required_adapter = "claude-code".to_owned(),
        }
        denied(
            f.store
                .create_mission(
                    f.ids.corp_id,
                    f.ids.alice_actor_id,
                    "Invalid bound plan",
                    "Metadata only",
                    &invalid,
                )
                .await,
            "adapter or source",
        );
        assert_eq!(persisted_state(&f).await?, before);
    }
    let mut mixed = bound.clone();
    mixed.budget_tokens *= 2;
    mixed.budget_cost_microusd *= 2;
    let mut unbound = mixed.tasks[0].clone();
    unbound.key = "unbound".to_owned();
    unbound.contract.workspace_connection_id = None;
    mixed.tasks.push(unbound);
    for reverse in [false, true] {
        if reverse {
            mixed.tasks.reverse();
        }
        assert!(
            f.store
                .create_mission(
                    f.ids.corp_id,
                    f.ids.alice_actor_id,
                    "Mixed execution environments",
                    "Metadata only",
                    &mixed,
                )
                .await
                .is_err(),
            "bound and unbound tasks must be rejected in either order"
        );
        assert_eq!(persisted_state(&f).await?, before);
    }
    let mut cross_room = bound.clone();
    cross_room.budget_tokens *= 2;
    cross_room.budget_cost_microusd *= 2;
    let mut other_task = cross_room.tasks[0].clone();
    other_task.key = "other-room".to_owned();
    other_task.contract.workspace_connection_id = Some(connection(&first_room).id);
    cross_room.tasks.push(other_task);
    denied(
        f.store
            .create_mission(
                f.ids.corp_id,
                f.ids.alice_actor_id,
                "Cross-room plan",
                "Metadata only",
                &cross_room,
            )
            .await,
        "unrelated project rooms",
    );
    denied(
        f.store
            .create_mission(
                f.ids.corp_id,
                f.ids.bob_actor_id,
                "Foreign room plan",
                "Metadata only",
                &bound,
            )
            .await,
        "forbidden",
    );
    assert_eq!(persisted_state(&f).await?, before);

    let (mission, _) = f
        .store
        .create_mission(
            f.ids.corp_id,
            f.ids.alice_actor_id,
            "Native connected plan",
            "Persist metadata only; never dispatch its returned command",
            &bound,
        )
        .await?;
    let stored_room: Uuid =
        sqlx::query_scalar("SELECT room_id FROM missions WHERE corp_id=$1 AND id=$2")
            .bind(f.ids.corp_id)
            .bind(mission.mission_id)
            .fetch_one(&f.store.pool)
            .await?;
    assert_eq!(stored_room, selected_room);
    assert_ne!(stored_room, f.ids.room_id);
    assert_eq!(mission.task_ids.len(), 1);
    let schedulable = f
        .store
        .schedulable_tasks(f.ids.corp_id, mission.mission_id, true)
        .await?;
    assert_eq!(schedulable.len(), 1);
    assert_eq!(schedulable[0].workspace_connection_id, Some(selected_id));
    assert_eq!(schedulable[0].required_adapter, "codex");
    assert_eq!(
        schedulable[0].required_source_repository,
        Some(source().repository)
    );
    assert_eq!(
        schedulable[0].required_source_base_ref,
        Some(source().base_ref)
    );
    assert_eq!(
        schedulable[0].required_source_base_commit,
        Some(source().base_commit)
    );

    let other_runner = connect_runner(
        &f.store,
        f.ids.corp_id,
        &format!("issue198-wrong-launch-{}", Uuid::new_v4()),
        Uuid::new_v4(),
    )
    .await?;
    let before = persisted_state(&f).await?;
    denied(
        f.store
            .create_task_run(
                f.ids.corp_id,
                mission.mission_id,
                mission.task_ids[0],
                Some(f.ids.alice_actor_id),
                &other_runner.id,
            )
            .await,
        "selected connection",
    );
    assert_eq!(persisted_state(&f).await?, before);
    let (launch, event) = f
        .store
        .create_task_run(
            f.ids.corp_id,
            mission.mission_id,
            mission.task_ids[0],
            Some(f.ids.alice_actor_id),
            &f.runner.id,
        )
        .await?;
    assert_eq!(launch.room_id, selected_room);
    assert_eq!(launch.workspace_connection_id, Some(selected_id));
    assert_eq!(launch.source_repository, Some(source().repository));
    assert_eq!(launch.source_base_ref, Some(source().base_ref));
    assert_eq!(launch.source_base_commit, Some(source().base_commit));
    assert_eq!(event.room_id, Some(selected_room));
    let stored_binding: Option<Uuid> =
        sqlx::query_scalar("SELECT workspace_connection_id FROM runs WHERE corp_id=$1 AND id=$2")
            .bind(f.ids.corp_id)
            .bind(launch.run_id)
            .fetch_one(&f.store.pool)
            .await?;
    assert_eq!(stored_binding, Some(selected_id));
    let snapshot = f
        .store
        .snapshot(f.ids.corp_id, f.ids.alice_actor_id)
        .await?;
    let run = snapshot
        .runs
        .iter()
        .find(|run| run.id == launch.run_id)
        .context("persisted bound run")?;
    assert_eq!(run.workspace_connection_id, Some(selected_id));
    assert_eq!(run.runner_id, f.runner.id);

    let before = persisted_state(&f).await?;
    let mut sign_in = f.setup_input(
        "active-run-sign-in",
        WorkspaceSetupAction::SignInAgent {
            connection_id: selected_id,
            configuration: configuration(),
        },
    );
    sign_in.room_id = selected_room;
    denied(
        f.store.begin_workspace_setup(sign_in).await,
        "wait for current work",
    );
    assert_eq!(persisted_state(&f).await?, before);
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue198_run_admission_rechecks_failed_connection_and_changed_source_atomically(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let ready = f.ready("run-admission").await?;
    let id = connection(&ready).id;
    let (mission, _) = f
        .store
        .create_mission(
            f.ids.corp_id,
            f.ids.alice_actor_id,
            "Held connection-bound plan",
            "Metadata only",
            &plan(&f, id),
        )
        .await?;
    let check = f
        .store
        .begin_workspace_setup(f.setup_input("fail-before-run", test_action(id)))
        .await?;
    f.apply(check.operation.id, report(WorkspaceSetupStatus::Failed))
        .await?;
    let before = persisted_state(&f).await?;
    denied(
        f.store
            .create_task_run(
                f.ids.corp_id,
                mission.mission_id,
                mission.task_ids[0],
                Some(f.ids.alice_actor_id),
                &f.runner.id,
            )
            .await,
        "needs attention",
    );
    assert_eq!(persisted_state(&f).await?, before);
    let recheck = f
        .store
        .begin_workspace_setup(f.setup_input("changed-revision", test_action(id)))
        .await?;
    let mut changed = ready_report();
    changed
        .source
        .as_mut()
        .context("fixture source")?
        .base_commit = "b".repeat(40);
    let updated = f.apply(recheck.operation.id, changed).await?;
    assert_eq!(
        connection(&updated).status,
        WorkspaceConnectionStatus::Ready
    );
    let before = persisted_state(&f).await?;
    denied(
        f.store
            .create_task_run(
                f.ids.corp_id,
                mission.mission_id,
                mission.task_ids[0],
                Some(f.ids.alice_actor_id),
                &f.runner.id,
            )
            .await,
        "source revision changed",
    );
    assert_eq!(persisted_state(&f).await?, before);
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue198_saved_ready_connection_cannot_start_a_run_after_runner_disconnect(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let ready = f.ready("offline-run").await?;
    let id = connection(&ready).id;
    let (mission, _) = f
        .store
        .create_mission(
            f.ids.corp_id,
            f.ids.alice_actor_id,
            "Plan saved before disconnect",
            "Metadata only",
            &plan(&f, id),
        )
        .await?;
    f.store
        .runner_disconnected(&f.runner.id, f.runner.connection_epoch, 60)
        .await?;
    f.store
        .select_workspace_connection(f.ids.corp_id, f.ids.room_id, f.ids.alice_actor_id, id)
        .await?;
    let offline = f
        .store
        .workspace_connections(f.ids.corp_id, f.ids.room_id, f.ids.alice_actor_id)
        .await?;
    assert_eq!(offline.selected_connection_id, Some(id));
    assert!(!offline.connections[0].runner_connected);
    let before = persisted_state(&f).await?;
    let attempted = f
        .store
        .create_task_run(
            f.ids.corp_id,
            mission.mission_id,
            mission.task_ids[0],
            Some(f.ids.alice_actor_id),
            &f.runner.id,
        )
        .await;
    // Regression: checking saved status alone misses disconnection after planning.
    assert!(
        attempted.is_err(),
        "a saved ready status and preference must not authorize an offline runner"
    );
    assert_eq!(persisted_state(&f).await?, before);
    Ok(())
}
