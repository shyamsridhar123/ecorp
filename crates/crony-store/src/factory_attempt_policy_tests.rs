//! Prospective Factory attempt-policy boundaries in SQLx-owned fixture databases.
//! Child of factory_connections: reuse its private fixture and ledger unchanged.
//! These cases do not launch a run/provider or prove public server/runner acceptance.
use super::*;

fn prospective_plan(f: &Fixture, connection_id: Uuid, attempts: i32) -> TaskGraphPlan {
    let mut plan = factory_plan(f, Some(connection_id));
    // This is proposed graph construction, before any mission/task exists.
    // Keep the original fixture's task and mission budgets exactly as supplied.
    for task in &mut plan.tasks {
        task.max_attempts = attempts;
    }
    plan
}

fn assert_no_graph(state: &(Value, Value)) {
    for table in ["missions", "tasks", "runs"] {
        assert_eq!(state.0[table], json!([]), "unexpected persisted {table}");
    }
}

fn assert_task_attempts(state: &(Value, Value), outcome: &FactoryMissionOutcome, attempts: i32) {
    let tasks = state.0["tasks"].as_array().expect("task ledger");
    assert_eq!(tasks.len(), outcome.ids.task_ids.len());
    assert!(!tasks.is_empty());
    for task_id in &outcome.ids.task_ids {
        let task = tasks
            .iter()
            .find(|task| task["id"] == json!(task_id))
            .expect("materialized task in the independent ledger");
        assert_eq!(task["mission_id"], json!(outcome.ids.mission_id));
        assert_eq!(task["attempt_count"], json!(0));
        assert_eq!(task["max_attempts"], json!(attempts));
    }
    assert_eq!(
        state.0["runs"],
        json!([]),
        "materialization must not dispatch"
    );
}

fn saved_operation<'a>(state: &'a (Value, Value), key: &str) -> &'a Value {
    state.1["operations"]
        .as_array()
        .expect("Factory operation ledger")
        .iter()
        .find(|operation| operation["idempotency_key"].as_str() == Some(key))
        .expect("the exact native operation was recorded")
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue224_explicit_attempt_policy_preflight_materialization_and_replay(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let ready = f.ready("issue224-explicit-three").await?;
    let connection_id = connection(&ready).id;
    let mut approved_policy = policy(Some(connection_id));
    approved_policy["max_task_attempts"] = json!(3);
    let plan = prospective_plan(&f, connection_id, 3);
    let mut proposed = preflight(&f, approved_policy.clone());
    proposed.request["max_task_attempts"] = json!(3);
    let before = ledger(&f).await?;
    assert_no_graph(&before);
    let checked = f.store.preflight_factory_mission(proposed, &plan).await?;
    assert_eq!(ledger(&f).await?, before, "preflight is mutation-free");
    assert!(checked.tasks.iter().all(|task| task.max_attempts == 3));
    assert_eq!(checked.budget_tokens, plan.budget_tokens);
    assert_eq!(checked.budget_cost_microusd, plan.budget_cost_microusd);
    assert_eq!(
        checked.tasks[0].contract.budget_tokens,
        plan.tasks[0].contract.budget_tokens
    );
    assert_eq!(
        checked.tasks[0].contract.budget_cost_microusd,
        plan.tasks[0].contract.budget_cost_microusd
    );

    let claim = f
        .store
        .claim_factory_work_item(claim_input(&f, approved_policy, "issue224-explicit"))
        .await?;
    assert_eq!(claim.work_item.policy["max_task_attempts"], json!(3));
    let claimed = ledger(&f).await?;
    assert_no_graph(&claimed);
    let selected = f
        .store
        .factory_planning_source(f.ids.corp_id, claim.work_item.id, f.ids.alice_actor_id)
        .await?;
    assert_eq!(selected.max_task_attempts, Some(3));
    assert_eq!(selected.workspace_connection_id, Some(connection_id));
    assert_eq!(selected.repository, source().repository);
    assert_eq!(selected.base_ref, source().base_ref);
    assert_eq!(selected.base_commit, source().base_commit);
    assert_eq!(
        ledger(&f).await?,
        claimed,
        "planning-source lookup is read-only"
    );

    let mut input = materialize(&f, &claim, "issue224-explicit");
    input.request["max_task_attempts"] = json!(3);
    let outcome = f
        .store
        .materialize_factory_mission(input.clone(), &checked)
        .await?;
    assert_eq!(
        outcome.work_item.state,
        FactoryWorkItemState::MissionCreated
    );
    assert_eq!(outcome.work_item.policy, claim.work_item.policy);
    let saved = ledger(&f).await?;
    assert_task_attempts(&saved, &outcome, 3);
    let item = saved.1["items"]
        .as_array()
        .expect("Factory item ledger")
        .iter()
        .find(|item| item["id"] == json!(outcome.work_item.id))
        .expect("persisted claimed item");
    assert_eq!(item["policy"]["max_task_attempts"], json!(3));
    let operation = saved_operation(&saved, &input.idempotency_key);
    assert_eq!(operation["operation"], json!("materialize"));
    assert_eq!(operation["request"], input.request);
    assert_materialization_replays(&f, &input, &checked, &outcome).await?;
    assert_eq!(ledger(&f).await?, saved);
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue224_attempt_policy_rejections_preserve_preflight_and_claimed_ledgers(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let ready = f.ready("issue224-denials").await?;
    let connection_id = connection(&ready).id;
    let mut approved_policy = policy(Some(connection_id));
    approved_policy["max_task_attempts"] = json!(3);
    let plan = prospective_plan(&f, connection_id, 3);
    let invalid_values = [
        json!(0),
        json!(-1),
        json!(4),
        json!(3.0),
        json!("3"),
        json!(false),
        json!([]),
        json!({}),
    ];
    let initial = ledger(&f).await?;
    assert_no_graph(&initial);
    for (index, value) in invalid_values.iter().enumerate() {
        let mut invalid_policy = approved_policy.clone();
        invalid_policy["max_task_attempts"] = value.clone();
        let mut proposed = preflight(&f, invalid_policy.clone());
        proposed.request["max_task_attempts"] = json!(3);
        denied(
            f.store.preflight_factory_mission(proposed, &plan).await,
            "max_task_attempts",
        );
        assert_eq!(ledger(&f).await?, initial);
        denied(
            f.store
                .claim_factory_work_item(claim_input(
                    &f,
                    invalid_policy,
                    &format!("issue224-invalid-policy-{index}"),
                ))
                .await,
            "max_task_attempts",
        );
        assert_eq!(
            ledger(&f).await?,
            initial,
            "invalid policies cannot create a claim or graph"
        );
    }

    let claim = f
        .store
        .claim_factory_work_item(claim_input(&f, approved_policy.clone(), "issue224-denied"))
        .await?;
    let claimed = ledger(&f).await?;
    assert_no_graph(&claimed);
    // A null request is a legacy omission, not agreement with an explicit grant.
    for requested in [None, Some(Value::Null), Some(json!(2))]
        .into_iter()
        .chain(invalid_values.iter().cloned().map(Some))
    {
        let mut proposed = preflight(&f, approved_policy.clone());
        let mut input = materialize(&f, &claim, "issue224-denied");
        if let Some(value) = requested {
            proposed.request["max_task_attempts"] = value.clone();
            input.request["max_task_attempts"] = value;
        }
        denied(
            f.store.preflight_factory_mission(proposed, &plan).await,
            "max_task_attempts",
        );
        assert_eq!(ledger(&f).await?, claimed);
        denied(
            f.store.materialize_factory_mission(input, &plan).await,
            "max_task_attempts",
        );
        assert_eq!(
            ledger(&f).await?,
            claimed,
            "rejection must not persist any graph or run"
        );
    }
    for attempts in [0, 2, 4] {
        let inconsistent = prospective_plan(&f, connection_id, attempts);
        let mut proposed = preflight(&f, approved_policy.clone());
        proposed.request["max_task_attempts"] = json!(3);
        let mut input = materialize(&f, &claim, "issue224-denied-plan");
        input.request["max_task_attempts"] = json!(3);
        denied(
            f.store
                .preflight_factory_mission(proposed, &inconsistent)
                .await,
            "task-attempt allowance",
        );
        assert_eq!(ledger(&f).await?, claimed);
        denied(
            f.store
                .materialize_factory_mission(input, &inconsistent)
                .await,
            "task-attempt allowance",
        );
        assert_eq!(ledger(&f).await?, claimed);
    }

    let legacy_policy = policy(Some(connection_id));
    let legacy_plan = factory_plan(&f, Some(connection_id));
    let mut unclaimed_option = preflight(&f, legacy_policy.clone());
    unclaimed_option.request["max_task_attempts"] = json!(3);
    denied(
        f.store
            .preflight_factory_mission(unclaimed_option, &legacy_plan)
            .await,
        "max_task_attempts",
    );
    assert_eq!(ledger(&f).await?, claimed);
    let legacy_claim = f
        .store
        .claim_factory_work_item(claim_input(&f, legacy_policy, "issue224-omitted-policy"))
        .await?;
    let legacy_claimed = ledger(&f).await?;
    assert_no_graph(&legacy_claimed);
    let mut input = materialize(&f, &legacy_claim, "issue224-injected-option");
    input.request["max_task_attempts"] = json!(3);
    denied(
        f.store
            .materialize_factory_mission(input, &legacy_plan)
            .await,
        "max_task_attempts",
    );
    let after = ledger(&f).await?;
    assert_eq!(after, legacy_claimed);
    assert_no_graph(&after);
    Ok(())
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue224_legacy_attempt_omission_replays_without_retrospective_override(
    pool: PgPool,
) -> Result<()> {
    let f = fixture(pool).await?;
    let ready = f.ready("issue224-legacy").await?;
    let connection_id = connection(&ready).id;
    let legacy_policy = policy(Some(connection_id));
    let plan = factory_plan(&f, Some(connection_id));
    let original_attempts = plan.tasks[0].max_attempts;
    assert_eq!(
        original_attempts, 1,
        "retain the original private fixture plan"
    );
    let proposed = preflight(&f, legacy_policy.clone());
    assert!(legacy_policy.get("max_task_attempts").is_none());
    assert!(proposed.request.get("max_task_attempts").is_none());
    let before = ledger(&f).await?;
    assert_no_graph(&before);
    let checked = f.store.preflight_factory_mission(proposed, &plan).await?;
    assert_eq!(checked.tasks[0].max_attempts, original_attempts);
    assert_eq!(ledger(&f).await?, before);

    let claim_request = claim_input(&f, legacy_policy.clone(), "issue224-legacy");
    let claim = f
        .store
        .claim_factory_work_item(claim_request.clone())
        .await?;
    assert!(claim.work_item.policy.get("max_task_attempts").is_none());
    for (key, value) in legacy_policy.as_object().expect("original policy") {
        assert_eq!(claim.work_item.policy.get(key), Some(value));
    }
    let claimed = ledger(&f).await?;
    let selected = f
        .store
        .factory_planning_source(f.ids.corp_id, claim.work_item.id, f.ids.alice_actor_id)
        .await?;
    assert_eq!(selected.max_task_attempts, None);
    assert_eq!(ledger(&f).await?, claimed);
    let input = materialize(&f, &claim, "issue224-legacy");
    let original_request = input.request.clone();
    assert!(original_request.get("max_task_attempts").is_none());
    let outcome = f
        .store
        .materialize_factory_mission(input.clone(), &checked)
        .await?;
    assert_eq!(outcome.work_item.policy, claim.work_item.policy);
    let saved = ledger(&f).await?;
    assert_task_attempts(&saved, &outcome, original_attempts);
    let operation = saved_operation(&saved, &input.idempotency_key);
    assert_eq!(operation["request"], original_request);
    assert!(operation["request"].get("max_task_attempts").is_none());
    assert_materialization_replays(&f, &input, &checked, &outcome).await?;
    let claim_replay = f.store.claim_factory_work_item(claim_request).await?;
    assert!(claim_replay.replayed);
    assert_eq!(claim_replay.work_item.id, claim.work_item.id);
    assert_eq!(claim_replay.work_item.policy, claim.work_item.policy);
    assert_eq!(ledger(&f).await?, saved);

    // Only the submitted operation is changed. Never UPDATE a persisted counter,
    // allowance, policy or budget, and never construct a replacement graph here.
    let mut changed = input.clone();
    changed.request["max_task_attempts"] = json!(3);
    assert_eq!(changed.idempotency_key, input.idempotency_key);
    assert_materialization_replays_denied(&f, &changed, &checked, "idempotency").await?;
    let after = ledger(&f).await?;
    assert_eq!(
        after, saved,
        "rejected override cannot alter counters or any recorded history"
    );
    assert_task_attempts(&after, &outcome, original_attempts);
    Ok(())
}
