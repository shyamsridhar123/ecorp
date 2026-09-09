//! Issue 199 regressions against the actual migrations in SQLx-owned databases.
//! The parent must opt in through its approved isolated maintenance loader.
//!
//! Target missions use native bootstrap/create/claim/materialize store methods.
//! No mission is launched and no runner, provider, browser, account, or checkout
//! is used. Direct SQL supplies identity/membership fixtures, deterministic
//! historical fillers, one deliberately corrupt link, and independent durable
//! state assertions. Private-data canaries are synthetic, not credentials.

use super::{
    ClaimFactoryWorkItemInput, DemoIds, FactorySourceInput, MaterializeFactoryMissionInput,
    MissionPlanIds, PgStore, TransitionFactoryWorkItemInput,
};
use anyhow::{Context, Result};
use crony_domain::{
    FactoryWorkItem, FactoryWorkItemState, MissionContext, MissionOrigin, PlannedTask,
    TaskContract, TaskGraphPlan, VerificationPolicy, VerifierCheck,
};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use uuid::Uuid;

const TITLE: &str = "Issue 199 mission context fixture";
const SOURCE_REPOSITORY: &str = "fixture/issue199-source";
const SOURCE_ISSUE_NUMBER: i64 = 199;
const SOURCE_ISSUE_URL: &str = "https://github.com/fixture/issue199-source/issues/199";
const PRIVATE_POLICY: &str = "issue199-private-policy-canary";
const PRIVATE_NATIVE_DIAGNOSTIC: &str = "issue199-private-native-diagnostic";
const PRIVATE_CHECKOUT: &str = "/fixture/issue199-private-checkout";
const PRIVATE_REQUEST: &str = "issue199-private-materialization-request";

struct Fixture {
    store: PgStore,
    ids: DemoIds,
    direct: MissionPlanIds,
    factory_ids: MissionPlanIds,
    factory: FactoryWorkItem,
    claim_token: Uuid,
}

fn plan(agent_id: Uuid) -> TaskGraphPlan {
    TaskGraphPlan {
        strategy: "single".to_owned(),
        max_nodes: 1,
        max_depth: 0,
        budget_tokens: 1_000,
        budget_cost_microusd: 1_000_000,
        staffing: Vec::new(),
        tasks: vec![PlannedTask {
            key: "deliver".to_owned(),
            title: "Prepare context fixture".to_owned(),
            assigned_agent_id: agent_id,
            required_adapter: "fake-process".to_owned(),
            depends_on: Vec::new(),
            depth: 0,
            max_attempts: 1,
            contract: TaskContract {
                workspace_connection_id: None,
                objective: "Write result.md".to_owned(),
                expected_output: "result.md".to_owned(),
                source_repository: Some(SOURCE_REPOSITORY.to_owned()),
                source_base_ref: Some("main".to_owned()),
                source_base_commit: Some("a".repeat(40)),
                acceptance_tests: vec!["result.md exists".to_owned()],
                allowed_tools: vec!["filesystem".to_owned()],
                prohibited_actions: vec!["merge requires separate authorization".to_owned()],
                references: Vec::new(),
                write_scope: vec!["result.md".to_owned()],
                budget_tokens: 1_000,
                budget_cost_microusd: 1_000_000,
                deadline_at: None,
                escalation: "ask the operator".to_owned(),
                secret_refs: Vec::new(),
                model: None,
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

fn factory_policy() -> Value {
    json!({
        "schema_version": 1,
        "source_of_truth": "github_project",
        "auto_merge": false,
        "repository_allowlist": [SOURCE_REPOSITORY],
        "source_base_ref": "main",
        "source_base_commit": "a".repeat(40),
        "adapter_allowlist": ["fake-process"],
        "strategy_allowlist": ["single"],
        "model": null,
        "reasoning_effort": null,
        "write_scope": ["result.md"],
        "allowed_tools": ["filesystem"],
        "prohibited_actions": ["merge requires separate authorization"],
        "secret_ids": [],
        "verification_required": true,
        "budget_tokens": 1_000,
        "budget_cost_microusd": 1_000_000,
        "fixture_private_policy": PRIVATE_POLICY,
        "native_diagnostics": {
            "detail": PRIVATE_NATIVE_DIAGNOSTIC,
            "workspace_path": PRIVATE_CHECKOUT
        }
    })
}

async fn fixture(pool: PgPool) -> Result<Fixture> {
    let store = PgStore { pool };
    let (ids, _) = store.bootstrap_demo().await?;
    let plan = plan(ids.worker_agent_id);
    let description = format!(
        "Source issue: {SOURCE_ISSUE_URL}; {PRIVATE_NATIVE_DIAGNOSTIC}: {PRIVATE_CHECKOUT}"
    );

    // Deliberately identical title, description, and source tuple: none of those
    // strings can distinguish the direct mission from the Factory-created one.
    let (direct, _) = store
        .create_mission(ids.corp_id, ids.alice_actor_id, TITLE, &description, &plan)
        .await?;
    let claim = store
        .claim_factory_work_item(ClaimFactoryWorkItemInput {
            corp_id: ids.corp_id,
            actor_id: ids.alice_actor_id,
            source: FactorySourceInput {
                project_owner: "FiXtUrE".to_owned(),
                project_number: 199,
                project_item_id: "issue199-project-item".to_owned(),
                repository_owner: "FiXtUrE".to_owned(),
                repository_name: "Issue199-Source".to_owned(),
                issue_number: SOURCE_ISSUE_NUMBER,
                issue_node_id: "issue199-issue-node".to_owned(),
                issue_url: SOURCE_ISSUE_URL.to_owned(),
                title: TITLE.to_owned(),
                revision: "issue199-source-revision".to_owned(),
            },
            idempotency_key: "issue199:claim".to_owned(),
            lease_seconds: 300,
            policy: factory_policy(),
        })
        .await?;
    assert!(claim.work_item.mission_id.is_none());
    let claim_token = claim
        .claim_token
        .context("native claim must return its token")?;
    let materialized = store
        .materialize_factory_mission(
            MaterializeFactoryMissionInput {
                corp_id: ids.corp_id,
                work_item_id: claim.work_item.id,
                actor_id: ids.alice_actor_id,
                claim_token,
                expected_version: claim.work_item.version,
                idempotency_key: "issue199:materialize".to_owned(),
                title: TITLE.to_owned(),
                description: description.clone(),
                request: json!({
                    "title": TITLE,
                    "description": description,
                    "adapter": "fake-process",
                    "strategy": "single",
                    "fixture_private_request": PRIVATE_REQUEST
                }),
            },
            &plan,
        )
        .await?;
    assert!(!materialized.replayed);
    assert_eq!(
        materialized.work_item.mission_id,
        Some(materialized.ids.mission_id)
    );
    assert_ne!(direct.mission_id, materialized.ids.mission_id);

    let counts: (i64, i64, i64) = sqlx::query_as(
        "SELECT
           (SELECT count(*) FROM missions WHERE corp_id=$1),
           (SELECT count(*) FROM tasks WHERE corp_id=$1),
           (SELECT count(*) FROM runs WHERE corp_id=$1)",
    )
    .bind(ids.corp_id)
    .fetch_one(&store.pool)
    .await?;
    assert_eq!(
        counts,
        (2, 2, 0),
        "native creation must not launch either mission"
    );

    Ok(Fixture {
        store,
        ids,
        direct,
        factory_ids: materialized.ids,
        factory: materialized.work_item,
        claim_token,
    })
}

// Unlike the bounded client snapshot, this captures complete rows (including
// timestamps and private fixture canaries) in the disposable test database.
// Comparison detects updates, inserts, deletions, and unexpected execution,
// approval, message, or journal effects from successful and denied reads alike.
async fn persisted_state(store: &PgStore) -> Result<Value> {
    sqlx::query_scalar(
        "SELECT jsonb_build_object(
          'corps',(SELECT jsonb_agg(to_jsonb(c) ORDER BY id) FROM corps c),
          'actors',(SELECT jsonb_agg(to_jsonb(a) ORDER BY id) FROM actors a),
          'rooms',(SELECT jsonb_agg(to_jsonb(r) ORDER BY id) FROM rooms r),
          'memberships',(SELECT jsonb_agg(to_jsonb(m) ORDER BY room_id,actor_id) FROM room_memberships m),
          'agents',(SELECT jsonb_agg(to_jsonb(a) ORDER BY id) FROM agents a),
          'missions',(SELECT jsonb_agg(to_jsonb(m) ORDER BY id) FROM missions m),
          'tasks',(SELECT jsonb_agg(to_jsonb(t) ORDER BY id) FROM tasks t),
          'dependencies',(SELECT jsonb_agg(to_jsonb(d) ORDER BY task_id,depends_on_task_id) FROM task_dependencies d),
          'runs',(SELECT jsonb_agg(to_jsonb(r) ORDER BY id) FROM runs r),
          'factory',(SELECT jsonb_agg(to_jsonb(f) ORDER BY id) FROM factory_work_items f),
          'factory_operations',(SELECT jsonb_agg(to_jsonb(o) ORDER BY corp_id,idempotency_key) FROM factory_operations o),
          'recoveries',(SELECT jsonb_agg(to_jsonb(r) ORDER BY id) FROM factory_verification_recoveries r),
          'publications',(SELECT jsonb_agg(to_jsonb(p) ORDER BY id) FROM pull_request_publications p),
          'approvals',(SELECT jsonb_agg(to_jsonb(a) ORDER BY id) FROM action_approvals a),
          'queued_messages',(SELECT jsonb_agg(to_jsonb(q) ORDER BY id) FROM queued_messages q),
          'room_messages',(SELECT jsonb_agg(to_jsonb(m) ORDER BY id) FROM room_messages m),
          'commands',(SELECT jsonb_agg(to_jsonb(c) ORDER BY id) FROM runner_commands c),
          'verification_requests',(SELECT jsonb_agg(to_jsonb(v) ORDER BY run_id) FROM verification_requests v),
          'control_leases',(SELECT jsonb_agg(to_jsonb(l) ORDER BY agent_id) FROM control_leases l),
          'events',(SELECT jsonb_agg(to_jsonb(e) ORDER BY seq) FROM events e))",
    )
    .fetch_one(&store.pool)
    .await
    .context("capture complete fixture persistence")
}

async fn read_unchanged(
    f: &Fixture,
    corp_id: Uuid,
    actor_id: Uuid,
    mission_id: Uuid,
) -> Result<Option<MissionContext>> {
    let before = persisted_state(&f.store).await?;
    let result = f.store.mission_context(corp_id, actor_id, mission_id).await;
    let after = persisted_state(&f.store).await?;
    assert_eq!(after, before, "mission_context must be persistence-neutral");
    result
}

fn factory_origin(f: &Fixture) -> MissionOrigin {
    MissionOrigin::Factory {
        work_item_id: f.factory.id,
        source_repository: SOURCE_REPOSITORY.to_owned(),
        source_issue_number: SOURCE_ISSUE_NUMBER,
        source_issue_url: SOURCE_ISSUE_URL.to_owned(),
    }
}

fn assert_exact_context(
    f: &Fixture,
    context: &MissionContext,
    actor_id: Uuid,
    mission_id: Uuid,
    origin: MissionOrigin,
) -> Result<()> {
    assert_eq!(context.corp_id, f.ids.corp_id);
    assert_eq!(context.actor_id, actor_id);
    assert_eq!(context.mission_id, mission_id);
    assert_eq!(context.room_id, f.ids.room_id);
    assert_eq!(context.origin, origin);
    let origin_json = match origin {
        MissionOrigin::Direct => json!({"kind": "direct"}),
        MissionOrigin::Factory {
            work_item_id,
            source_repository,
            source_issue_number,
            source_issue_url,
        } => json!({
            "kind": "factory",
            "work_item_id": work_item_id,
            "source_repository": source_repository,
            "source_issue_number": source_issue_number,
            "source_issue_url": source_issue_url
        }),
    };
    assert_eq!(
        serde_json::to_value(context)?,
        json!({
            "corp_id": f.ids.corp_id,
            "actor_id": actor_id,
            "mission_id": mission_id,
            "room_id": f.ids.room_id,
            "origin": origin_json
        }),
        "the public DTO must contain only the agreed context fields"
    );
    Ok(())
}

async fn assert_both_contexts(f: &Fixture, actor_id: Uuid) -> Result<()> {
    for (mission_id, origin) in [
        (f.direct.mission_id, MissionOrigin::Direct),
        (f.factory_ids.mission_id, factory_origin(f)),
    ] {
        let context = read_unchanged(f, f.ids.corp_id, actor_id, mission_id)
            .await?
            .context("authorized operator must receive exact mission context")?;
        assert_exact_context(f, &context, actor_id, mission_id, origin)?;
    }
    Ok(())
}

async fn assert_neither_context(f: &Fixture, corp_id: Uuid, actor_id: Uuid) -> Result<()> {
    for mission_id in [f.direct.mission_id, f.factory_ids.mission_id] {
        assert!(
            read_unchanged(f, corp_id, actor_id, mission_id)
                .await?
                .is_none(),
            "denied visibility must be None, never a synthetic Direct context"
        );
    }
    Ok(())
}

async fn join_room(store: &PgStore, room_id: Uuid, actor_id: Uuid) -> Result<()> {
    sqlx::query(
        "INSERT INTO room_memberships(room_id,actor_id,role) VALUES($1,$2,'member')
         ON CONFLICT(room_id,actor_id) DO NOTHING",
    )
    .bind(room_id)
    .bind(actor_id)
    .execute(&store.pool)
    .await?;
    Ok(())
}

async fn add_actor(
    store: &PgStore,
    corp_id: Uuid,
    kind: &str,
    role: &str,
    room_id: Option<Uuid>,
) -> Result<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO actors(id,corp_id,name,kind,role) VALUES($1,$2,$3,$4,$5)")
        .bind(id)
        .bind(corp_id)
        .bind(format!("issue199-{kind}-{role}"))
        .bind(kind)
        .bind(role)
        .execute(&store.pool)
        .await?;
    if let Some(room_id) = room_id {
        join_room(store, room_id, id).await?;
    }
    Ok(id)
}

async fn add_corp(store: &PgStore) -> Result<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO corps(id,slug,name) VALUES($1,$2,'Issue 199 other Corp')")
        .bind(id)
        .bind(format!("issue199-{id}"))
        .execute(&store.pool)
        .await?;
    Ok(id)
}

async fn add_room(store: &PgStore, corp_id: Uuid, name: &str) -> Result<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO rooms(id,corp_id,name,purpose) VALUES($1,$2,$3,'Issue 199 scope fixture')",
    )
    .bind(id)
    .bind(corp_id)
    .bind(name)
    .execute(&store.pool)
    .await?;
    Ok(id)
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires the approved isolated SQLx loader with filter issue199_"]
async fn issue199_direct_context_uses_native_creation_and_exact_fields(pool: PgPool) -> Result<()> {
    let f = fixture(pool).await?;
    let linkage_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM factory_work_items WHERE mission_id=$1")
            .bind(f.direct.mission_id)
            .fetch_one(&f.store.pool)
            .await?;
    assert_eq!(linkage_count, 0);
    let context = read_unchanged(&f, f.ids.corp_id, f.ids.alice_actor_id, f.direct.mission_id)
        .await?
        .context("a newly saved direct mission has an authoritative origin")?;
    assert_exact_context(
        &f,
        &context,
        f.ids.alice_actor_id,
        f.direct.mission_id,
        MissionOrigin::Direct,
    )
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires the approved isolated SQLx loader with filter issue199_"]
async fn issue199_factory_context_uses_native_materialization_and_exact_fields(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    assert_eq!(f.factory.state, FactoryWorkItemState::MissionCreated);
    let linked_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM events
         WHERE corp_id=$1 AND type='factory.mission_linked'
           AND aggregate_id=$2 AND correlation_id=$3",
    )
    .bind(f.ids.corp_id)
    .bind(f.factory.id)
    .bind(f.factory_ids.mission_id)
    .fetch_one(&f.store.pool)
    .await?;
    assert_eq!(linked_events, 1);
    let context = read_unchanged(
        &f,
        f.ids.corp_id,
        f.ids.alice_actor_id,
        f.factory_ids.mission_id,
    )
    .await?
    .context("held Factory mission must be readable before any run or publication")?;
    assert_exact_context(
        &f,
        &context,
        f.ids.alice_actor_id,
        f.factory_ids.mission_id,
        factory_origin(&f),
    )
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires the approved isolated SQLx loader with filter issue199_"]
async fn issue199_current_human_operator_roles_read_both_origins(pool: PgPool) -> Result<()> {
    let f = fixture(pool).await?;
    let admin = add_actor(
        &f.store,
        f.ids.corp_id,
        "human",
        "admin",
        Some(f.ids.room_id),
    )
    .await?;
    let manager = add_actor(
        &f.store,
        f.ids.corp_id,
        "human",
        "manager",
        Some(f.ids.room_id),
    )
    .await?;
    for actor_id in [f.ids.alice_actor_id, admin, manager, f.ids.bob_actor_id] {
        // In particular, actor_id is the viewer, not the mission requester.
        assert_both_contexts(&f, actor_id).await?;
    }
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires the approved isolated SQLx loader with filter issue199_"]
async fn issue199_factory_origin_survives_501_newer_snapshot_items(pool: PgPool) -> Result<()> {
    let f = fixture(pool).await?;
    // These bounded, unmaterialized history rows are SQL-only window fillers.
    // Derive every timestamp from the target, avoiding sleeps and clock ties.
    let inserted = sqlx::query(
        "INSERT INTO factory_work_items(
           id,corp_id,source_kind,source_project_owner,source_project_number,
           source_project_item_id,source_repository_owner,source_repository_name,
           source_issue_number,source_issue_node_id,source_issue_url,source_title,
           source_revision,state,version,claim_owner_id,claim_token,lease_expires_at,
           policy,created_at,updated_at)
         SELECT md5('issue199-newer-item-' || newer.i::text)::uuid,
           original.corp_id,original.source_kind,original.source_project_owner,
           original.source_project_number,'issue199-newer-project-item-' || newer.i::text,
           original.source_repository_owner,original.source_repository_name,
           original.source_issue_number+newer.i,'issue199-newer-node-' || newer.i::text,
           format('https://github.com/%s/%s/issues/%s',original.source_repository_owner,
                  original.source_repository_name,original.source_issue_number+newer.i),
           'Newer issue199 fixture ' || newer.i::text,'issue199-newer-revision-' || newer.i::text,
           'claimed',1,original.claim_owner_id,
           md5('issue199-newer-token-' || newer.i::text)::uuid,original.lease_expires_at,
           original.policy,original.created_at+newer.i*interval '1 microsecond',
           original.created_at+newer.i*interval '1 microsecond'
         FROM factory_work_items original CROSS JOIN generate_series(1,501) AS newer(i)
         WHERE original.id=$1 AND original.corp_id=$2",
    )
    .bind(f.factory.id)
    .bind(f.ids.corp_id)
    .execute(&f.store.pool)
    .await?;
    assert_eq!(inserted.rows_affected(), 501);
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM factory_work_items WHERE corp_id=$1")
        .bind(f.ids.corp_id)
        .fetch_one(&f.store.pool)
        .await?;
    assert_eq!(total, 502);

    let snapshot = f
        .store
        .snapshot(f.ids.corp_id, f.ids.alice_actor_id)
        .await?;
    assert_eq!(snapshot.factory_work_items.len(), 500);
    assert!(
        snapshot
            .factory_work_items
            .iter()
            .all(|item| item.id != f.factory.id),
        "fixture must actually displace the native target from the bounded projection"
    );
    assert!(
        snapshot
            .missions
            .iter()
            .any(|mission| mission.id == f.factory_ids.mission_id)
    );
    // A complete read still distinguishes the missing historical link from the
    // genuinely unlinked direct mission; neither depends on the snapshot slice.
    assert_both_contexts(&f, f.ids.alice_actor_id).await
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires the approved isolated SQLx loader with filter issue199_"]
async fn issue199_direct_and_factory_context_deny_guests_and_spectators(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    join_room(&f.store, f.ids.room_id, f.ids.eve_actor_id).await?;
    let spectator = add_actor(
        &f.store,
        f.ids.corp_id,
        "human",
        "spectator",
        Some(f.ids.room_id),
    )
    .await?;
    for actor_id in [f.ids.eve_actor_id, spectator] {
        let snapshot = f.store.snapshot(f.ids.corp_id, actor_id).await?;
        assert!(snapshot.factory_work_items.is_empty());
        for mission_id in [f.direct.mission_id, f.factory_ids.mission_id] {
            assert!(
                snapshot
                    .missions
                    .iter()
                    .any(|mission| mission.id == mission_id),
                "role denial is exercised with actual mission-room visibility"
            );
        }
        // Preserve #185: a role-filtered empty Factory projection proves neither
        // Direct nor Factory, even for a real direct mission visible in the room.
        assert_neither_context(&f, f.ids.corp_id, actor_id).await?;
    }
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires the approved isolated SQLx loader with filter issue199_"]
async fn issue199_context_denies_unknown_and_nonhuman_actors(pool: PgPool) -> Result<()> {
    let f = fixture(pool).await?;
    assert_neither_context(&f, f.ids.corp_id, Uuid::new_v4()).await?;
    for kind in ["agent", "service"] {
        // An otherwise allowed role and membership cannot override actor kind.
        let actor_id =
            add_actor(&f.store, f.ids.corp_id, kind, "owner", Some(f.ids.room_id)).await?;
        assert_neither_context(&f, f.ids.corp_id, actor_id).await?;
    }
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires the approved isolated SQLx loader with filter issue199_"]
async fn issue199_context_is_scoped_to_corp_and_mission_room(pool: PgPool) -> Result<()> {
    let f = fixture(pool).await?;
    let other_corp = add_corp(&f.store).await?;
    let foreign_room = add_room(&f.store, other_corp, "Other Corp room").await?;
    let foreign_owner =
        add_actor(&f.store, other_corp, "human", "owner", Some(foreign_room)).await?;
    // Fixture-only cross-Corp membership demonstrates that membership alone
    // cannot substitute for the viewer belonging to the requested Corp.
    join_room(&f.store, f.ids.room_id, foreign_owner).await?;
    let other_room = add_room(&f.store, f.ids.corp_id, "Other room").await?;
    let other_room_owner =
        add_actor(&f.store, f.ids.corp_id, "human", "owner", Some(other_room)).await?;
    let unassigned_admin = add_actor(&f.store, f.ids.corp_id, "human", "admin", None).await?;

    for (corp_id, actor_id) in [
        (f.ids.corp_id, foreign_owner),
        (other_corp, foreign_owner),
        (other_corp, f.ids.alice_actor_id),
        (f.ids.corp_id, other_room_owner),
        (f.ids.corp_id, unassigned_admin),
    ] {
        assert_neither_context(&f, corp_id, actor_id).await?;
    }
    assert_both_contexts(&f, f.ids.alice_actor_id).await
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires the approved isolated SQLx loader with filter issue199_"]
async fn issue199_cross_corp_factory_link_is_none_not_direct(pool: PgPool) -> Result<()> {
    let f = fixture(pool).await?;
    assert_both_contexts(&f, f.ids.alice_actor_id).await?;
    let other_corp = add_corp(&f.store).await?;

    // Deliberate integrity anomaly, never a supported creation/link operation.
    // The id-only FK permits it; filtering the row out of a LEFT JOIN must not
    // convert a known Factory mission into a fabricated Direct result.
    let updated =
        sqlx::query("UPDATE factory_work_items SET corp_id=$1 WHERE id=$2 AND corp_id=$3")
            .bind(other_corp)
            .bind(f.factory.id)
            .bind(f.ids.corp_id)
            .execute(&f.store.pool)
            .await?;
    assert_eq!(updated.rows_affected(), 1);
    assert!(
        read_unchanged(
            &f,
            f.ids.corp_id,
            f.ids.alice_actor_id,
            f.factory_ids.mission_id,
        )
        .await?
        .is_none(),
        "an anomalous cross-Corp link must fail closed, not become Direct"
    );
    let retained: (Uuid, Uuid) =
        sqlx::query_as("SELECT corp_id,mission_id FROM factory_work_items WHERE id=$1")
            .bind(f.factory.id)
            .fetch_one(&f.store.pool)
            .await?;
    assert_eq!(retained, (other_corp, f.factory_ids.mission_id));

    let direct = read_unchanged(&f, f.ids.corp_id, f.ids.alice_actor_id, f.direct.mission_id)
        .await?
        .context("an unrelated direct mission must remain readable")?;
    assert_exact_context(
        &f,
        &direct,
        f.ids.alice_actor_id,
        f.direct.mission_id,
        MissionOrigin::Direct,
    )
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires the approved isolated SQLx loader with filter issue199_"]
async fn issue199_context_rechecks_membership_removal(pool: PgPool) -> Result<()> {
    let f = fixture(pool).await?;
    assert_both_contexts(&f, f.ids.bob_actor_id).await?;
    let removed = sqlx::query("DELETE FROM room_memberships WHERE room_id=$1 AND actor_id=$2")
        .bind(f.ids.room_id)
        .bind(f.ids.bob_actor_id)
        .execute(&f.store.pool)
        .await?;
    assert_eq!(removed.rows_affected(), 1);
    assert_neither_context(&f, f.ids.corp_id, f.ids.bob_actor_id).await?;
    join_room(&f.store, f.ids.room_id, f.ids.bob_actor_id).await?;
    assert_both_contexts(&f, f.ids.bob_actor_id).await
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires the approved isolated SQLx loader with filter issue199_"]
async fn issue199_context_rechecks_role_removal(pool: PgPool) -> Result<()> {
    let f = fixture(pool).await?;
    assert_both_contexts(&f, f.ids.bob_actor_id).await?;
    for role in ["guest", "spectator"] {
        let updated = sqlx::query("UPDATE actors SET role=$1 WHERE id=$2 AND corp_id=$3")
            .bind(role)
            .bind(f.ids.bob_actor_id)
            .bind(f.ids.corp_id)
            .execute(&f.store.pool)
            .await?;
        assert_eq!(updated.rows_affected(), 1);
        assert_neither_context(&f, f.ids.corp_id, f.ids.bob_actor_id).await?;
    }
    sqlx::query("UPDATE actors SET role='member' WHERE id=$1 AND corp_id=$2")
        .bind(f.ids.bob_actor_id)
        .bind(f.ids.corp_id)
        .execute(&f.store.pool)
        .await?;
    assert_both_contexts(&f, f.ids.bob_actor_id).await
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires the approved isolated SQLx loader with filter issue199_"]
async fn issue199_factory_context_excludes_private_metadata(pool: PgPool) -> Result<()> {
    let f = fixture(pool).await?;
    let diagnostic = format!("{PRIVATE_NATIVE_DIAGNOSTIC}: {PRIVATE_CHECKOUT}");
    let blocked = f
        .store
        .transition_factory_work_item(TransitionFactoryWorkItemInput {
            corp_id: f.ids.corp_id,
            work_item_id: f.factory.id,
            actor_id: f.ids.alice_actor_id,
            claim_token: f.claim_token,
            expected_version: f.factory.version,
            idempotency_key: "issue199:private-diagnostics".to_owned(),
            state: FactoryWorkItemState::Blocked,
            failure_detail: Some(diagnostic.clone()),
        })
        .await?;
    assert_eq!(blocked.work_item.mission_id, Some(f.factory_ids.mission_id));

    let retained = sqlx::query(
        "SELECT claim_token,policy,failure_detail FROM factory_work_items WHERE id=$1 AND corp_id=$2",
    )
    .bind(f.factory.id)
    .bind(f.ids.corp_id)
    .fetch_one(&f.store.pool)
    .await?;
    assert_eq!(retained.get::<Uuid, _>("claim_token"), f.claim_token);
    let retained_policy: Value = retained.get("policy");
    assert_eq!(retained_policy["fixture_private_policy"], PRIVATE_POLICY);
    assert_eq!(
        retained_policy["native_diagnostics"]["detail"],
        PRIVATE_NATIVE_DIAGNOSTIC
    );
    assert_eq!(retained.get::<String, _>("failure_detail"), diagnostic);

    let claim_token = f.claim_token.to_string();
    for (mission_id, origin) in [
        (f.direct.mission_id, MissionOrigin::Direct),
        (f.factory_ids.mission_id, factory_origin(&f)),
    ] {
        let context = read_unchanged(&f, f.ids.corp_id, f.ids.bob_actor_id, mission_id)
            .await?
            .context("an authorized collaborator can inspect origin without private metadata")?;
        assert_exact_context(&f, &context, f.ids.bob_actor_id, mission_id, origin)?;
        let serialized = serde_json::to_string(&context)?;
        for forbidden in [
            claim_token.as_str(),
            PRIVATE_POLICY,
            PRIVATE_NATIVE_DIAGNOSTIC,
            PRIVATE_CHECKOUT,
            PRIVATE_REQUEST,
            "\"claim_token\"",
            "\"claim_owner_id\"",
            "\"policy\"",
            "\"native_diagnostics\"",
            "\"failure_detail\"",
            "\"request\"",
            "\"workspace_path\"",
            "\"provider_session_id\"",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "mission context leaked private fixture metadata: {forbidden}"
            );
        }
    }
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires the approved isolated SQLx loader with filter issue199_"]
async fn issue199_context_returns_none_for_absent_mission(pool: PgPool) -> Result<()> {
    let f = fixture(pool).await?;
    for mission_id in [Uuid::new_v4(), Uuid::nil(), f.factory.id] {
        assert!(
            read_unchanged(&f, f.ids.corp_id, f.ids.alice_actor_id, mission_id)
                .await?
                .is_none(),
            "unknown mission IDs and Factory item IDs must not fabricate Direct"
        );
    }
    Ok(())
}
