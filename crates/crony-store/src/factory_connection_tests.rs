//! Factory/connection integration against real migrations in SQLx-owned databases.
//! Reuses the connection fixture; no native account, provider, service or app DB.
use super::*;
use crate::{
    ClaimFactoryWorkItemInput, FactoryMissionOutcome, FactorySourceInput, FactoryWorkItemOutcome,
    MaterializeFactoryMissionInput, PreflightFactoryMissionInput,
};
use crony_domain::{FactoryWorkItemState, ManualVerificationGate};

fn factory_plan(f: &Fixture, connection_id: Option<Uuid>) -> TaskGraphPlan {
    let mut result = plan(f, connection_id.unwrap_or_else(Uuid::new_v4));
    result.tasks[0].contract.workspace_connection_id = connection_id;
    result.tasks[0].verification_policy.manual_gate =
        Some(ManualVerificationGate::IndependentReview {
            roles: vec!["member".to_owned()],
            exclude_requester: true,
        });
    result
}

fn policy(connection_id: Option<Uuid>) -> Value {
    let source = source();
    let mut value = json!({
        "schema_version": 1,
        "source_of_truth": "github_project",
        "auto_merge": false,
        "repository_allowlist": [source.repository],
        "source_base_ref": source.base_ref,
        "source_base_commit": source.base_commit,
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
        "budget_cost_microusd": 1000000
    });
    if let Some(id) = connection_id {
        value["workspace_connection_id"] = json!(id);
    }
    value
}

fn preflight(f: &Fixture, policy: Value) -> PreflightFactoryMissionInput {
    PreflightFactoryMissionInput {
        corp_id: f.ids.corp_id,
        actor_id: f.ids.alice_actor_id,
        source_repository_owner: "fixture".to_owned(),
        source_repository_name: "project".to_owned(),
        policy,
        title: "Issue 204 native Factory binding".to_owned(),
        description: "Actual-store metadata, not provider execution".to_owned(),
        request: json!({"scope": "issue204", "title": "Factory connection"}),
    }
}

fn claim_input(f: &Fixture, policy: Value, key: &str) -> ClaimFactoryWorkItemInput {
    ClaimFactoryWorkItemInput {
        corp_id: f.ids.corp_id,
        actor_id: f.ids.alice_actor_id,
        source: FactorySourceInput {
            project_owner: "fixture".to_owned(),
            project_number: 3,
            project_item_id: format!("issue204-{key}"),
            repository_owner: "fixture".to_owned(),
            repository_name: "project".to_owned(),
            issue_number: 204,
            issue_node_id: format!("issue204-node-{key}"),
            issue_url: "https://github.com/fixture/project/issues/204".to_owned(),
            title: "Factory connection fixture".to_owned(),
            revision: "2026-09-09T00:00:00Z".to_owned(),
        },
        idempotency_key: format!("issue204-claim-{key}"),
        lease_seconds: 300,
        policy,
    }
}

fn materialize(
    f: &Fixture,
    claim: &FactoryWorkItemOutcome,
    key: &str,
) -> MaterializeFactoryMissionInput {
    MaterializeFactoryMissionInput {
        corp_id: f.ids.corp_id,
        actor_id: f.ids.alice_actor_id,
        work_item_id: claim.work_item.id,
        claim_token: claim.claim_token.expect("native fixture claim"),
        expected_version: claim.work_item.version,
        idempotency_key: format!("issue204-materialize-{key}"),
        title: "Issue 204 native Factory binding".to_owned(),
        description: "Actual-store metadata, not provider execution".to_owned(),
        request: json!({"scope": "issue204", "title": "Factory connection"}),
    }
}

async fn ledger(f: &Fixture) -> Result<(Value, Value)> {
    let factory = sqlx::query_scalar(
        "SELECT jsonb_build_object(
           'items',(SELECT coalesce(jsonb_agg(to_jsonb(w) ORDER BY w.id),'[]'::jsonb)
                    FROM factory_work_items w WHERE w.corp_id=$1),
           'operations',(SELECT coalesce(jsonb_agg(to_jsonb(o) ORDER BY o.idempotency_key),'[]'::jsonb)
                         FROM factory_operations o WHERE o.corp_id=$1))",
    )
    .bind(f.ids.corp_id)
    .fetch_one(&f.store.pool)
    .await?;
    Ok((persisted_state(f).await?, factory))
}

async fn assert_materialization_replays(
    f: &Fixture,
    input: &MaterializeFactoryMissionInput,
    plan: &TaskGraphPlan,
    original: &FactoryMissionOutcome,
) -> Result<()> {
    let before = ledger(f).await?;
    for full_materialization in [false, true] {
        let replay = if full_materialization {
            f.store
                .materialize_factory_mission(input.clone(), plan)
                .await?
        } else {
            f.store
                .replay_factory_materialization(input)
                .await?
                .context("the original materialization must still be replayable")?
        };
        assert!(replay.replayed);
        assert!(replay.events.is_empty());
        assert_eq!(replay.ids.mission_id, original.ids.mission_id);
        assert_eq!(replay.ids.task_ids, original.ids.task_ids);
        assert_eq!(replay.strategy, original.strategy);
        assert_eq!(replay.work_item.id, original.work_item.id);
        assert_eq!(replay.work_item.policy, original.work_item.policy);
        assert_eq!(ledger(f).await?, before);
    }
    Ok(())
}

async fn assert_materialization_replays_denied(
    f: &Fixture,
    input: &MaterializeFactoryMissionInput,
    plan: &TaskGraphPlan,
    expected: &str,
) -> Result<()> {
    let before = ledger(f).await?;
    denied(
        f.store.replay_factory_materialization(input).await,
        expected,
    );
    assert_eq!(ledger(f).await?, before);
    denied(
        f.store
            .materialize_factory_mission(input.clone(), plan)
            .await,
        expected,
    );
    assert_eq!(ledger(f).await?, before);
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue204_factory_claim_rejects_unknown_foreign_and_other_room_connections(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let ready = f.ready("claim-authorized").await?;
    let authorized_id = connection(&ready).id;
    let (foreign_corp, foreign_room, foreign_actor, foreign_runner) =
        foreign_scope(&f.store).await?;
    let mut foreign_input = f.create_input("claim-foreign");
    foreign_input.corp_id = foreign_corp;
    foreign_input.room_id = foreign_room;
    foreign_input.actor_id = foreign_actor;
    foreign_input.runner_id = foreign_runner.id;
    let foreign = f.store.create_workspace_connection(foreign_input).await?;
    let foreign_id = connection(&foreign).id;
    let (foreign_before, _) = f
        .store
        .workspace_connection_settings(foreign_corp, foreign_actor, foreign_id)
        .await?;
    let foreign_before = serde_json::to_value(foreign_before)?;
    let private_room = add_room(&f.store, f.ids.corp_id, &[f.ids.bob_actor_id]).await?;
    let mut private_input = f.create_input("claim-other-room");
    private_input.room_id = private_room;
    private_input.actor_id = f.ids.bob_actor_id;
    let private = f.store.create_workspace_connection(private_input).await?;
    let before = ledger(&f).await?;

    for (key, id, expected) in [
        (
            "claim-unknown",
            Uuid::new_v4(),
            "connection was not found in this Corp",
        ),
        (
            "claim-foreign",
            foreign_id,
            "connection was not found in this Corp",
        ),
        (
            "claim-other-room",
            connection(&private).id,
            "not a member of this room",
        ),
    ] {
        denied(
            f.store
                .claim_factory_work_item(claim_input(&f, policy(Some(id)), key))
                .await,
            expected,
        );
        assert_eq!(ledger(&f).await?, before);
    }
    let (foreign_after, _) = f
        .store
        .workspace_connection_settings(foreign_corp, foreign_actor, foreign_id)
        .await?;
    assert_eq!(serde_json::to_value(foreign_after)?, foreign_before);

    // A denied request must not reserve its item or idempotency key.
    let input = claim_input(&f, policy(Some(authorized_id)), "claim-unknown");
    let accepted = f.store.claim_factory_work_item(input.clone()).await?;
    assert_eq!(accepted.work_item.state, FactoryWorkItemState::Claimed);
    assert_eq!(
        accepted.work_item.policy["workspace_connection_id"],
        json!(authorized_id)
    );
    let accepted_state = ledger(&f).await?;
    let replay = f.store.claim_factory_work_item(input).await?;
    assert!(replay.replayed);
    assert_eq!(replay.work_item.id, accepted.work_item.id);
    assert_eq!(ledger(&f).await?, accepted_state);
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue204_factory_claim_replay_and_reclaim_recheck_room_authority(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let ready = f.ready("claim-revocation").await?;
    let id = connection(&ready).id;
    let input = claim_input(&f, policy(Some(id)), "claim-revocation");
    let original = f.store.claim_factory_work_item(input.clone()).await?;
    add_room(&f.store, f.ids.corp_id, &[f.ids.alice_actor_id]).await?;
    sqlx::query("DELETE FROM room_memberships WHERE room_id=$1 AND actor_id=$2")
        .bind(f.ids.room_id)
        .bind(f.ids.alice_actor_id)
        .execute(&f.store.pool)
        .await?;
    let before = ledger(&f).await?;
    denied(
        f.store.claim_factory_work_item(input.clone()).await,
        "not a member of this room",
    );
    assert_eq!(ledger(&f).await?, before);
    let mut active = input.clone();
    active.idempotency_key.push_str("-active-owner");
    denied(
        f.store.claim_factory_work_item(active).await,
        "not a member of this room",
    );
    assert_eq!(ledger(&f).await?, before);

    sqlx::query(
        "UPDATE factory_work_items SET lease_expires_at=now()-interval '1 second'
         WHERE corp_id=$1 AND id=$2",
    )
    .bind(f.ids.corp_id)
    .bind(original.work_item.id)
    .execute(&f.store.pool)
    .await?;
    let expired = ledger(&f).await?;
    let mut reclaim = input;
    reclaim.idempotency_key.push_str("-expired-reclaim");
    denied(
        f.store.claim_factory_work_item(reclaim.clone()).await,
        "not a member of this room",
    );
    assert_eq!(ledger(&f).await?, expired);

    sqlx::query("INSERT INTO room_memberships(room_id,actor_id) VALUES($1,$2)")
        .bind(f.ids.room_id)
        .bind(f.ids.alice_actor_id)
        .execute(&f.store.pool)
        .await?;
    let accepted = f.store.claim_factory_work_item(reclaim).await?;
    assert!(!accepted.replayed);
    assert_eq!(accepted.work_item.id, original.work_item.id);
    assert_eq!(accepted.work_item.policy, original.work_item.policy);
    assert_eq!(accepted.work_item.version, original.work_item.version + 1);
    assert_ne!(accepted.claim_token, original.claim_token);
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue204_both_materialization_replays_reject_revoked_mission_room(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let ready = f.ready("replay-room").await?;
    let mut cases = Vec::new();
    for (id, key) in [
        (Some(connection(&ready).id), "bound-room"),
        (None, "legacy-room"),
    ] {
        let claim = f
            .store
            .claim_factory_work_item(claim_input(&f, policy(id), key))
            .await?;
        let plan = factory_plan(&f, id);
        let input = materialize(&f, &claim, key);
        let original = f
            .store
            .materialize_factory_mission(input.clone(), &plan)
            .await?;
        cases.push((input, plan, original));
    }
    // Membership in another room must not authorize this historical mission.
    add_room(&f.store, f.ids.corp_id, &[f.ids.alice_actor_id]).await?;
    sqlx::query("DELETE FROM room_memberships WHERE room_id=$1 AND actor_id=$2")
        .bind(f.ids.room_id)
        .bind(f.ids.alice_actor_id)
        .execute(&f.store.pool)
        .await?;
    for (input, plan, _) in &cases {
        assert_materialization_replays_denied(&f, input, plan, "not a member of this room").await?;
    }
    sqlx::query("INSERT INTO room_memberships(room_id,actor_id) VALUES($1,$2)")
        .bind(f.ids.room_id)
        .bind(f.ids.alice_actor_id)
        .execute(&f.store.pool)
        .await?;
    for (input, plan, original) in &cases {
        assert_materialization_replays(&f, input, plan, original).await?;
    }
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue204_both_materialization_replays_require_current_human_operator(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let ready = f.ready("replay-human").await?;
    let id = connection(&ready).id;
    let claim_input = claim_input(&f, policy(Some(id)), "replay-human");
    let claim = f.store.claim_factory_work_item(claim_input.clone()).await?;
    let plan = factory_plan(&f, Some(id));
    let input = materialize(&f, &claim, "replay-human");
    let original = f
        .store
        .materialize_factory_mission(input.clone(), &plan)
        .await?;
    for (kind, role, expected) in [
        ("agent", "owner", "human room operator"),
        ("human", "spectator", "actor cannot dispatch missions"),
    ] {
        sqlx::query("UPDATE actors SET kind=$3,role=$4 WHERE corp_id=$1 AND id=$2")
            .bind(f.ids.corp_id)
            .bind(f.ids.alice_actor_id)
            .bind(kind)
            .bind(role)
            .execute(&f.store.pool)
            .await?;
        assert_materialization_replays_denied(&f, &input, &plan, expected).await?;
        let before = ledger(&f).await?;
        denied(
            f.store.claim_factory_work_item(claim_input.clone()).await,
            expected,
        );
        assert_eq!(ledger(&f).await?, before);
    }
    sqlx::query("UPDATE actors SET kind='human',role='owner' WHERE corp_id=$1 AND id=$2")
        .bind(f.ids.corp_id)
        .bind(f.ids.alice_actor_id)
        .execute(&f.store.pool)
        .await?;
    assert_materialization_replays(&f, &input, &plan, &original).await?;
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue204_authorized_claim_and_both_materialization_replays_survive_offline_connection(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let ready = f.ready("replay-offline").await?;
    let id = connection(&ready).id;
    let mut cases = Vec::new();
    for (binding, key) in [(Some(id), "bound-offline"), (None, "legacy-offline")] {
        let claim_input = claim_input(&f, policy(binding), key);
        let claim = f.store.claim_factory_work_item(claim_input.clone()).await?;
        let plan = factory_plan(&f, binding);
        let input = materialize(&f, &claim, key);
        let original = f
            .store
            .materialize_factory_mission(input.clone(), &plan)
            .await?;
        cases.push((claim_input, input, plan, original));
    }
    let check = f
        .store
        .begin_workspace_setup(f.setup_input("replay-provider-failed", test_action(id)))
        .await?;
    f.apply(
        check.operation.id,
        WorkspaceSetupReport {
            connection_status: Some(WorkspaceConnectionStatus::Failed),
            ..report(WorkspaceSetupStatus::Failed)
        },
    )
    .await?;
    f.store
        .runner_disconnected(&f.runner.id, f.runner.connection_epoch, 60)
        .await?;
    let (offline, _) = f
        .store
        .workspace_connection_settings(f.ids.corp_id, f.ids.alice_actor_id, id)
        .await?;
    assert_eq!(offline.status, WorkspaceConnectionStatus::Failed);
    assert!(!offline.runner_connected);
    let before = ledger(&f).await?;
    for (claim_input, input, plan, original) in &cases {
        assert_materialization_replays(&f, input, plan, original).await?;
        let replay = f.store.claim_factory_work_item(claim_input.clone()).await?;
        assert!(replay.replayed);
        assert!(replay.event.is_none());
        assert_eq!(replay.work_item.id, original.work_item.id);
        assert_eq!(replay.work_item.policy, original.work_item.policy);
        assert_eq!(ledger(&f).await?, before);
    }
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue204_preflight_materialization_and_run_retain_claimed_connection(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let ready = f.ready("factory-positive").await?;
    let id = connection(&ready).id;
    let plan = factory_plan(&f, Some(id));
    let before = ledger(&f).await?;
    let checked = f
        .store
        .preflight_factory_mission(preflight(&f, policy(Some(id))), &plan)
        .await?;
    assert_eq!(ledger(&f).await?, before);
    assert_eq!(checked.tasks[0].contract.workspace_connection_id, Some(id));
    let claim = f
        .store
        .claim_factory_work_item(claim_input(&f, policy(Some(id)), "positive"))
        .await?;
    let source_tuple = f
        .store
        .factory_staffing_source(f.ids.corp_id, claim.work_item.id, f.ids.alice_actor_id)
        .await?;
    assert_eq!(
        source_tuple,
        (
            source().repository,
            source().base_ref,
            source().base_commit,
            Some(id)
        )
    );
    let outcome = f
        .store
        .materialize_factory_mission(materialize(&f, &claim, "positive"), &checked)
        .await?;
    assert_eq!(
        outcome.work_item.state,
        FactoryWorkItemState::MissionCreated
    );
    assert_eq!(
        outcome.work_item.policy["workspace_connection_id"],
        json!(id)
    );
    let candidates = f
        .store
        .schedulable_tasks(f.ids.corp_id, outcome.ids.mission_id, true)
        .await?;
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].workspace_connection_id, Some(id));
    let (run, _) = f
        .store
        .create_task_run(
            f.ids.corp_id,
            outcome.ids.mission_id,
            outcome.ids.task_ids[0],
            Some(f.ids.alice_actor_id),
            &f.runner.id,
        )
        .await?;
    assert_eq!(run.workspace_connection_id, Some(id));
    assert_eq!(run.source_repository, Some(source().repository));
    assert_eq!(run.source_base_commit, Some(source().base_commit));
    let saved: Option<Uuid> =
        sqlx::query_scalar("SELECT workspace_connection_id FROM runs WHERE corp_id=$1 AND id=$2")
            .bind(f.ids.corp_id)
            .bind(run.run_id)
            .fetch_one(&f.store.pool)
            .await?;
    assert_eq!(saved, Some(id));
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue204_factory_uses_connection_room_not_oldest_room(pool: PgPool) -> Result<()> {
    let f = fixture(pool).await?;
    let room = add_room(&f.store, f.ids.corp_id, &[f.ids.alice_actor_id]).await?;
    let mut input = f.create_input("factory-other-room");
    input.room_id = room;
    let created = f.store.create_workspace_connection(input).await?;
    let ready = f.apply(created.operation.id, ready_report()).await?;
    let id = connection(&ready).id;
    let plan = factory_plan(&f, Some(id));
    f.store
        .preflight_factory_mission(preflight(&f, policy(Some(id))), &plan)
        .await?;
    let claim = f
        .store
        .claim_factory_work_item(claim_input(&f, policy(Some(id)), "other-room"))
        .await?;
    let outcome = f
        .store
        .materialize_factory_mission(materialize(&f, &claim, "other-room"), &plan)
        .await?;
    let actual: Uuid =
        sqlx::query_scalar("SELECT room_id FROM missions WHERE corp_id=$1 AND id=$2")
            .bind(f.ids.corp_id)
            .bind(outcome.ids.mission_id)
            .fetch_one(&f.store.pool)
            .await?;
    assert_eq!(actual, room);
    assert_ne!(actual, f.ids.room_id);
    assert!(
        outcome
            .events
            .iter()
            .all(|event| event.room_id == Some(room))
    );
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue204_factory_binding_cannot_be_removed_substituted_or_injected(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let ready = f.ready("factory-mismatch").await?;
    let id = connection(&ready).id;
    let claim = f
        .store
        .claim_factory_work_item(claim_input(&f, policy(Some(id)), "mismatch"))
        .await?;
    let before = ledger(&f).await?;
    for proposed_id in [None, Some(Uuid::new_v4())] {
        let plan = factory_plan(&f, proposed_id);
        denied(
            f.store
                .preflight_factory_mission(preflight(&f, policy(Some(id))), &plan)
                .await,
            "claimed workspace connection",
        );
        denied(
            f.store
                .materialize_factory_mission(materialize(&f, &claim, "mismatch"), &plan)
                .await,
            "claimed workspace connection",
        );
        assert_eq!(ledger(&f).await?, before);
    }
    denied(
        f.store
            .preflight_factory_mission(preflight(&f, policy(None)), &factory_plan(&f, Some(id)))
            .await,
        "claimed workspace connection",
    );
    assert_eq!(ledger(&f).await?, before);
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue204_factory_preflight_rechecks_current_actor_corp_and_room(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let ready = f.ready("factory-authority").await?;
    let id = connection(&ready).id;
    let plan = factory_plan(&f, Some(id));
    let (foreign_corp, _, foreign_actor, _) = foreign_scope(&f.store).await?;
    let before = ledger(&f).await?;
    for (corp, actor) in [
        (f.ids.corp_id, f.ids.eve_actor_id),
        (f.ids.corp_id, Uuid::new_v4()),
        (foreign_corp, foreign_actor),
    ] {
        let mut request = preflight(&f, policy(Some(id)));
        request.corp_id = corp;
        request.actor_id = actor;
        assert!(
            f.store
                .preflight_factory_mission(request, &plan)
                .await
                .is_err()
        );
        assert_eq!(ledger(&f).await?, before);
    }
    let mut bob = preflight(&f, policy(Some(id)));
    bob.actor_id = f.ids.bob_actor_id;
    f.store
        .preflight_factory_mission(bob.clone(), &plan)
        .await?;
    sqlx::query("DELETE FROM room_memberships WHERE room_id=$1 AND actor_id=$2")
        .bind(f.ids.room_id)
        .bind(f.ids.bob_actor_id)
        .execute(&f.store.pool)
        .await?;
    let revoked = ledger(&f).await?;
    assert!(f.store.preflight_factory_mission(bob, &plan).await.is_err());
    assert_eq!(ledger(&f).await?, revoked);
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue204_factory_materialization_rechecks_connection_readiness(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let ready = f.ready("factory-readiness").await?;
    let id = connection(&ready).id;
    let claim = f
        .store
        .claim_factory_work_item(claim_input(&f, policy(Some(id)), "readiness"))
        .await?;
    let check = f
        .store
        .begin_workspace_setup(f.setup_input("factory-failed-check", test_action(id)))
        .await?;
    f.apply(
        check.operation.id,
        WorkspaceSetupReport {
            connection_status: Some(WorkspaceConnectionStatus::Failed),
            ..report(WorkspaceSetupStatus::Failed)
        },
    )
    .await?;
    let before = ledger(&f).await?;
    let plan = factory_plan(&f, Some(id));
    denied(
        f.store
            .preflight_factory_mission(preflight(&f, policy(Some(id))), &plan)
            .await,
        "not ready",
    );
    denied(
        f.store
            .materialize_factory_mission(materialize(&f, &claim, "readiness"), &plan)
            .await,
        "not ready",
    );
    assert_eq!(ledger(&f).await?, before);
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue204_factory_materialization_rejects_source_changed_after_claim(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let ready = f.ready("factory-source").await?;
    let id = connection(&ready).id;
    let claim = f
        .store
        .claim_factory_work_item(claim_input(&f, policy(Some(id)), "source"))
        .await?;
    let check = f
        .store
        .begin_workspace_setup(f.setup_input("factory-source-refresh", test_action(id)))
        .await?;
    let mut report = ready_report();
    report
        .source
        .as_mut()
        .context("fixture source")?
        .base_commit = "b".repeat(40);
    f.apply(check.operation.id, report).await?;
    let before = ledger(&f).await?;
    denied(
        f.store
            .materialize_factory_mission(
                materialize(&f, &claim, "source"),
                &factory_plan(&f, Some(id)),
            )
            .await,
        "does not match the saved connection",
    );
    assert_eq!(ledger(&f).await?, before);
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue204_factory_claim_and_materialization_replay_preserve_original_binding(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let ready = f.ready("factory-replay").await?;
    let id = connection(&ready).id;
    let request = claim_input(&f, policy(Some(id)), "replay");
    let claim = f.store.claim_factory_work_item(request.clone()).await?;
    let plan = factory_plan(&f, Some(id));
    let input = materialize(&f, &claim, "replay");
    let outcome = f
        .store
        .materialize_factory_mission(input.clone(), &plan)
        .await?;
    let before = ledger(&f).await?;
    let replay = f
        .store
        .materialize_factory_mission(input.clone(), &plan)
        .await?;
    assert!(replay.replayed);
    assert_eq!(replay.ids.mission_id, outcome.ids.mission_id);
    assert_eq!(
        replay.work_item.policy["workspace_connection_id"],
        json!(id)
    );
    assert_eq!(ledger(&f).await?, before);
    let claim_replay = f.store.claim_factory_work_item(request.clone()).await?;
    assert!(claim_replay.replayed);
    assert_eq!(claim_replay.work_item.id, claim.work_item.id);
    assert_eq!(ledger(&f).await?, before);
    let mut replaced = request;
    replaced.policy["workspace_connection_id"] = json!(Uuid::new_v4());
    assert!(f.store.claim_factory_work_item(replaced).await.is_err());
    let mut changed_request = input;
    changed_request.request["workspace_connection_id"] = json!(Uuid::new_v4());
    assert!(
        f.store
            .materialize_factory_mission(changed_request, &plan)
            .await
            .is_err()
    );
    assert_eq!(ledger(&f).await?, before);
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue204_factory_late_failure_rolls_back_connection_bound_graph(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let ready = f.ready("factory-rollback").await?;
    let id = connection(&ready).id;
    let claim = f
        .store
        .claim_factory_work_item(claim_input(&f, policy(Some(id)), "rollback"))
        .await?;
    // A failure after native graph creation proves the whole transaction rolls back.
    sqlx::raw_sql(
        "CREATE FUNCTION issue204_reject_link() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'issue204 synthetic late link failure'; END $$;
         CREATE TRIGGER issue204_reject_link BEFORE UPDATE ON factory_work_items
         FOR EACH ROW WHEN (NEW.mission_id IS NOT NULL) EXECUTE FUNCTION issue204_reject_link();",
    )
    .execute(&f.store.pool)
    .await?;
    let before = ledger(&f).await?;
    denied(
        f.store
            .materialize_factory_mission(
                materialize(&f, &claim, "rollback"),
                &factory_plan(&f, Some(id)),
            )
            .await,
        "issue204 synthetic late link failure",
    );
    assert_eq!(ledger(&f).await?, before);
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue204_legacy_unbound_factory_materialization_remains_compatible(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let plan = factory_plan(&f, None);
    f.store
        .preflight_factory_mission(preflight(&f, policy(None)), &plan)
        .await?;
    let claim = f
        .store
        .claim_factory_work_item(claim_input(&f, policy(None), "legacy"))
        .await?;
    assert!(
        claim
            .work_item
            .policy
            .get("workspace_connection_id")
            .is_none()
    );
    let outcome = f
        .store
        .materialize_factory_mission(materialize(&f, &claim, "legacy"), &plan)
        .await?;
    let candidates = f
        .store
        .schedulable_tasks(f.ids.corp_id, outcome.ids.mission_id, true)
        .await?;
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].workspace_connection_id, None);
    Ok(())
}

#[path = "factory_attempt_policy_tests.rs"]
mod attempt_policy;
