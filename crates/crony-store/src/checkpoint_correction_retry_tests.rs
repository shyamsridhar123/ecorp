//! #221 baseline: real store methods in an explicitly owned disposable SQLx DB.
//! Runner reports are synthetic metadata, not provider execution, physical source
//! verification, signed-artifact acceptance, or an application/runtime proof.
use super::*;

fn assert_retry_preserves_history(before: &Value, after: &Value) {
    // Live task/mission/agent/Factory projections may advance. Existing run,
    // receipt, revision, and spend history must not be rewritten to enable it.
    for key in [
        "runs",
        "recoveries",
        "commands",
        "artifacts",
        "deliverables",
        "verification_evidence",
        "verification_requests",
        "actors",
        "memberships",
        "operations",
    ] {
        let current = after[key].as_array().unwrap();
        for row in before[key].as_array().unwrap() {
            assert!(
                current.contains(row),
                "retry rewrote or removed an existing {key} row"
            );
        }
    }
    assert_eq!(after["source"], before["source"]);
    assert_eq!(after["issue210_ledger"], before["issue210_ledger"]);
    let previous_events = before["events"].as_array().unwrap();
    let current_events = after["events"].as_array().unwrap();
    assert!(current_events.len() >= previous_events.len());
    assert_eq!(
        &current_events[..previous_events.len()],
        previous_events.as_slice()
    );
    for field in [
        "budget_tokens",
        "original_budget_tokens",
        "budget_cost_microusd",
        "original_budget_cost_microusd",
    ] {
        assert_eq!(after["mission"][field], before["mission"][field], "{field}");
    }
    for field in ["contract", "verification_policy", "max_attempts"] {
        assert_eq!(after["task"][field], before["task"][field], "{field}");
    }
    for field in ["policy", "source_revision", "mission_id"] {
        assert_eq!(after["item"][field], before["item"][field], "{field}");
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue221_retry_after_failed_checkpoint_correction(pool: PgPool) {
    // Authorize both corrections before the original provider runs. Never
    // increase/reset attempts or budgets after a failure to manufacture a retry.
    let profile = CheckpointFixtureProfile {
        max_attempts: 3,
        ..correction_profile()
    };
    let store = fixture_with_profile(pool, true, false, None, false, profile).await;
    let original = correction_state(&store).await;
    assert_eq!(original["task"]["attempt_count"], 1);
    assert_eq!(original["task"]["max_attempts"], 3);
    assert_eq!(original["mission"]["budget_tokens"], 10_000);
    assert_eq!(original["mission"]["original_budget_tokens"], 10_000);
    assert_eq!(original["source"]["input_tokens"], 5_300);
    assert_eq!(original["source"]["breaker_stage"], "suspend");

    let verifier = failed_checkpoint(&store).await;
    let first_input = correction_request(&store, verifier).await;
    let first_outcome = store
        .create_factory_verification_recovery(first_input.clone())
        .await
        .expect("baseline must first admit the existing native checkpoint correction");
    let FactoryVerificationRecoveryLaunch::SourceCorrection(first) = first_outcome.launch else {
        panic!("first correction must be an ordinary provider continuation");
    };
    let first_allocated = correction_state(&store).await;
    assert_eq!(first_allocated["task"]["attempt_count"], 2);
    assert_eq!(first_allocated["task"]["max_attempts"], 3);
    assert_eq!(first_allocated["source"], original["source"]);
    assert_eq!(
        retained_run(&first_allocated, first.run_id)["budget_tokens_limit"],
        4_700
    );
    let first_command = store
        .pending_runner_commands(RUNNER)
        .await
        .unwrap()
        .into_iter()
        .find(|command| command.run_id == first.run_id)
        .expect("first correction must have its native durable runner command");
    assert_correction_dispatch(&store, &first_command, true).await;
    store
        .acknowledge_runner_command(first_command.id, RUNNER)
        .await
        .unwrap();

    // This is a started provider followed by failed verification, not a
    // dispatch_not_started failure that latest_workspace_source_id may skip.
    let latest_fingerprint = "c".repeat(64);
    for (kind, payload) in [
        (
            "run.started",
            json!({
                "adapter":"codex","execution_mode":"provider",
                "provider_session_id":"fixture-session",
                "workspace":"fixture-worktree","workspace_branch":"crony/fixture",
                "workspace_base_ref":"main","workspace_base_commit":"a".repeat(40)
            }),
        ),
        (
            "run.usage",
            json!({"input_tokens":100,"output_tokens":0,"cost_microusd":1_000}),
        ),
        (
            "run.session_terminated",
            json!({"adapter":"codex","outcome":"completed","provider_process_alive":false}),
        ),
        ("run.verification_started", json!({})),
        (
            "run.verification_evidence",
            json!({
                "evidence_id":Uuid::new_v4(),"check_index":0,"kind":"file",
                "status":"passed","summary":"Fixture source-file check passed","payload":{}
            }),
        ),
        (
            "run.verification_evidence",
            json!({
                "evidence_id":Uuid::new_v4(),"check_index":1,"kind":"artifact",
                "status":"failed","summary":"Correction still lacks its required artifact","payload":{}
            }),
        ),
        (
            "run.verification_failed",
            json!({"error":"Correction still lacks its required artifact"}),
        ),
        (
            "run.workspace_preserved",
            json!({
                "workspace_fingerprint":latest_fingerprint,"head_commit":"a".repeat(40),
                "detail":"Native failed provider correction retained its current source"
            }),
        ),
    ] {
        store
            .apply_runner_event(event(first.run_id, first.assignment_token, kind, payload))
            .await
            .unwrap_or_else(|error| {
                panic!("native first-correction event {kind} failed: {error:#}")
            });
    }
    let failed = correction_state(&store).await;
    let failed_provider = retained_run(&failed, first.run_id);
    assert_eq!(failed_provider["execution_mode"], "provider");
    assert_eq!(failed_provider["status"], "failed");
    assert_eq!(failed_provider["verification_status"], "failed");
    assert_eq!(failed_provider["workspace_disposition"], "preserved");
    assert_eq!(failed_provider["workspace_fingerprint"], latest_fingerprint);
    assert_eq!(failed_provider["input_tokens"], 100);
    assert_eq!(failed_provider["cost_microusd"], 1_000);
    assert_eq!(failed_provider["breaker_stage"], "healthy");
    assert_eq!(failed["item"]["state"], "verification_failed");
    assert_eq!(failed["task"]["attempt_count"], 2);
    assert_eq!(failed["task"]["max_attempts"], 3);
    assert_eq!(failed["source"], original["source"]);

    // A new public revision must select the latest failed provider, not its old
    // checkpoint verifier. The helper creates a fresh native idempotency key.
    let mut second_input = correction_request(&store, first.run_id).await;
    let context = store
        .factory_verification_recovery_context(CORP, OWNER, ITEM)
        .await
        .unwrap()
        .expect("latest failed provider must remain inspectable through native context");
    assert_eq!(context.source_run_id, first.run_id);
    assert_ne!(context.source_run_id, verifier);
    assert_eq!(context.remaining_attempts, 1);
    assert_eq!(context.remaining_mission_tokens, 4_600);
    assert_eq!(context.remaining_mission_cost_microusd, 4_999_000);
    assert_eq!(
        context.workspace_fingerprint.as_deref(),
        Some(latest_fingerprint.as_str())
    );
    // A failed provider without a source deliverable has no exported head.
    // Do not carry correction_request's verifier-only Some(head) into this call.
    assert!(context.expected_head_commit.is_none());
    second_input.expected_factory_version = context.work_item.version;
    second_input.expected_workspace_fingerprint = context.workspace_fingerprint.unwrap();
    second_input.expected_head_commit = context.expected_head_commit;
    assert_ne!(second_input.idempotency_key, first_input.idempotency_key);
    assert_ne!(
        second_input.contract_revision_id,
        first_input.contract_revision_id
    );
    let before_retry = correction_state(&store).await;
    let revision = before_retry["issue210_ledger"]["contract_revisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == json!(second_input.contract_revision_id.unwrap()))
        .expect("second correction must use the new persisted public revision");
    assert_eq!(revision["source_run_id"], json!(first.run_id));
    assert_eq!(
        revision["version"],
        before_retry["task"]["contract_version"]
    );
    assert_eq!(revision["next_action"], "resume");
    assert_eq!(before_retry["runs"], failed["runs"]);
    assert_eq!(before_retry["source"], original["source"]);

    let result = store
        .create_factory_verification_recovery(second_input)
        .await;
    if result.is_err() {
        assert_eq!(
            correction_state(&store).await,
            before_retry,
            "rejected second correction must not consume attempts, budget, or history"
        );
    }
    // Intended RED on 0113080: the strict origin gate accepts only the old
    // failed checkpoint verifier, not the newly revised failed provider source.
    let second_outcome =
        result.expect("issue221: next correction must admit the latest failed provider correction");
    let FactoryVerificationRecoveryLaunch::SourceCorrection(second) = second_outcome.launch else {
        panic!("second correction must consume ordinary provider allocation");
    };
    assert!(!second_outcome.replayed);
    assert_eq!(second.source_run_id, first.run_id);
    assert_eq!(second.workspace_run_id, SOURCE);
    assert_eq!(second.provider_session_id, first.provider_session_id);
    assert_eq!(
        second.workspace_connection_id,
        first.workspace_connection_id
    );
    assert_eq!(second.runner_id, first.runner_id);
    assert_eq!(second.agent_id, first.agent_id);
    assert_eq!(second.source_repository, first.source_repository);
    assert_eq!(second.source_base_ref, first.source_base_ref);
    assert_eq!(second.source_base_commit, first.source_base_commit);
    assert_eq!(second.workspace_base_commit, first.workspace_base_commit);
    assert_eq!(second.model, first.model);
    assert_eq!(second.reasoning_effort, first.reasoning_effort);
    let after_retry = correction_state(&store).await;
    assert_eq!(after_retry["task"]["attempt_count"], 3);
    assert_eq!(after_retry["task"]["max_attempts"], 3);
    assert_eq!(after_retry["runs"].as_array().unwrap().len(), 4);
    for key in ["runs", "recoveries", "commands"] {
        assert_eq!(
            after_retry[key].as_array().unwrap().len(),
            before_retry[key].as_array().unwrap().len() + 1,
            "second correction must add exactly one {key} row"
        );
    }
    let next = retained_run(&after_retry, second.run_id);
    assert_eq!(next["execution_mode"], "provider");
    assert_eq!(next["resumed_from_run_id"], json!(first.run_id));
    assert_eq!(next["workspace_run_id"], json!(SOURCE));
    assert_eq!(next["workspace_connection_id"], json!(CONNECTION));
    assert_eq!(next["budget_tokens_limit"], 4_600);
    assert_eq!(next["budget_cost_microusd_limit"], 1_000_000);
    assert_eq!(next["input_tokens"], 0);
    assert_eq!(next["output_tokens"], 0);
    assert_eq!(next["cost_microusd"], 0);
    assert_retry_preserves_history(&before_retry, &after_retry);
    let second_command = store
        .pending_runner_commands(RUNNER)
        .await
        .unwrap()
        .into_iter()
        .find(|command| command.run_id == second.run_id)
        .expect("second correction must have its native durable runner command");
    assert_eq!(second_command.command_kind, "factory_verification_recovery");
    assert_eq!(second_command.payload["mode"], "source_correction");
    assert_correction_dispatch(&store, &second_command, true).await;
    assert_retry_preserves_history(&before_retry, &correction_state(&store).await);
}

// #221 focused matrices. Three attempts are valid stored policy, established
// before the original provider's usage. The current public planner strategies
// expose only one/two attempts: these fixtures are NOT public-runtime proof of
// a three-attempt mission. No test resets counters, spend, or run/task lifecycle
// state to manufacture a retry.
fn retry_profile() -> CheckpointFixtureProfile {
    CheckpointFixtureProfile {
        max_attempts: 3,
        ..correction_profile()
    }
}

struct RetryRun {
    recovery_id: Uuid,
    launch: ResumeLaunchRecord,
    command: PendingRunnerCommand,
}

struct RetryFixture {
    store: PgStore,
    verifier: Uuid,
    first_input: CreateFactoryVerificationRecoveryInput,
    first: RetryRun,
}

#[derive(Clone, Copy)]
enum RetryFailure {
    Verification,
    Provider,
}

async fn retry_admit(store: &PgStore, input: &CreateFactoryVerificationRecoveryInput) -> RetryRun {
    let before = correction_state(store).await;
    let outcome = store
        .create_factory_verification_recovery(input.clone())
        .await
        .expect("current native SourceCorrection must admit one provider allocation");
    assert!(!outcome.replayed);
    let FactoryVerificationRecoveryLaunch::SourceCorrection(launch) = outcome.launch else {
        panic!("retry must not substitute a zero-provider verifier");
    };
    let command = store
        .pending_runner_commands(RUNNER)
        .await
        .unwrap()
        .into_iter()
        .find(|command| command.run_id == launch.run_id)
        .expect("one native durable correction command");
    assert_correction_dispatch(store, &command, true).await;
    let after = correction_state(store).await;
    assert_eq!(
        after["task"]["attempt_count"].as_i64().unwrap(),
        before["task"]["attempt_count"].as_i64().unwrap() + 1
    );
    assert_eq!(after["task"]["max_attempts"], 3);
    for key in ["runs", "recoveries", "commands"] {
        assert_eq!(
            after[key].as_array().unwrap().len(),
            before[key].as_array().unwrap().len() + 1,
            "admission must append exactly one {key}"
        );
    }
    assert_retry_preserves_history(&before, &after);
    RetryRun {
        recovery_id: outcome.recovery.id,
        launch,
        command,
    }
}

async fn retry_allocated_first(pool: PgPool, profile: CheckpointFixtureProfile) -> RetryFixture {
    retry_allocated_first_with_verifier_seal(pool, profile, false).await
}

async fn retry_allocated_first_with_verifier_seal(
    pool: PgPool,
    profile: CheckpointFixtureProfile,
    omit_verifier_seal_head: bool,
) -> RetryFixture {
    assert_eq!(profile.max_attempts, 3);
    assert_eq!(profile.attempt_count, 1);
    let store = fixture_with_profile(pool, true, false, None, false, profile).await;
    let original = correction_state(&store).await;
    assert_eq!(original["task"]["max_attempts"], 3);
    assert_eq!(original["source"]["input_tokens"], 5_300);
    assert_eq!(original["mission"]["original_budget_tokens"], 10_000);
    let verifier = failed_checkpoint(&store).await;
    if omit_verifier_seal_head {
        // Shape only this setup event, before any correction authorization.
        // Native verifier teardown reports its checked fingerprint without a
        // top-level head_commit; retain the existing helper's headed default.
        let mut expected = correction_state(&store).await;
        let request = retry_recovery(&expected, verifier)["request"].clone();
        let seal = expected["events"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|row| {
                row["aggregate_id"] == json!(verifier) && row["type"] == "run.workspace_preserved"
            })
            .unwrap();
        assert_eq!(
            seal["payload"]["workspace_fingerprint"],
            request["expected_workspace_fingerprint"]
        );
        assert_eq!(
            seal["payload"]
                .as_object_mut()
                .unwrap()
                .remove("head_commit"),
            Some(request["expected_head_commit"].clone())
        );
        assert_eq!(request["expected_head_commit"], "a".repeat(40));
        let changed = sqlx::query(
            "UPDATE events SET payload=$3 WHERE corp_id=$1 AND id=$2
             AND aggregate_id=$4 AND aggregate_type='run' AND type='run.workspace_preserved'",
        )
        .bind(CORP)
        .bind(Uuid::parse_str(seal["id"].as_str().unwrap()).unwrap())
        .bind(seal["payload"].clone())
        .bind(verifier)
        .execute(&store.pool)
        .await
        .unwrap();
        assert_eq!(changed.rows_affected(), 1);
        assert_eq!(correction_state(&store).await, expected);
    }
    let first_input = correction_request(&store, verifier).await;
    let first = retry_admit(&store, &first_input).await;
    assert_eq!(first.launch.source_run_id, verifier);
    assert_eq!(first.launch.workspace_run_id, SOURCE);
    assert_eq!(correction_state(&store).await["source"], original["source"]);
    RetryFixture {
        store,
        verifier,
        first_input,
        first,
    }
}

async fn retry_fail_provider(
    store: &PgStore,
    run: &RetryRun,
    failure: RetryFailure,
    input_tokens: i64,
    cost_microusd: i64,
    fingerprint: &str,
) {
    let before = correction_state(store).await;
    assert_correction_dispatch(store, &run.command, true).await;
    store
        .acknowledge_runner_command(run.command.id, RUNNER)
        .await
        .unwrap();
    let mut reports = vec![
        (
            "run.started",
            json!({
                "adapter":"codex","execution_mode":"provider",
                "provider_session_id":"fixture-session",
                "workspace":"fixture-worktree","workspace_branch":"crony/fixture",
                "workspace_base_ref":"main","workspace_base_commit":"a".repeat(40)
            }),
        ),
        (
            "run.usage",
            json!({"input_tokens":input_tokens,"output_tokens":0,"cost_microusd":cost_microusd}),
        ),
        (
            "run.session_terminated",
            json!({
                "adapter":"codex","provider_process_alive":false,
                "outcome":if matches!(failure, RetryFailure::Verification) {
                    "completed"
                } else {
                    "failed"
                }
            }),
        ),
    ];
    match failure {
        RetryFailure::Verification => reports.extend([
            ("run.verification_started", json!({})),
            (
                "run.verification_evidence",
                json!({"evidence_id":Uuid::new_v4(),"check_index":0,"kind":"file",
                    "status":"passed","summary":"Native fixture source check passed","payload":{}}),
            ),
            (
                "run.verification_evidence",
                json!({"evidence_id":Uuid::new_v4(),"check_index":1,"kind":"artifact",
                    "status":"failed","summary":"Required correction artifact is missing","payload":{}}),
            ),
            (
                "run.verification_failed",
                json!({"error":"Required correction artifact is missing"}),
            ),
        ]),
        // A native recovery failure governs the replacement itself. Do not
        // fabricate verification events or UPDATE status columns to select it.
        RetryFailure::Provider => reports.push((
            "run.failed",
            json!({"error":"Synthetic started provider failure before verification"}),
        )),
    }
    reports.push((
        "run.workspace_preserved",
        json!({"workspace_fingerprint":fingerprint,"head_commit":"a".repeat(40),
            "detail":"Native failed correction retained its current source"}),
    ));
    for (kind, payload) in reports {
        store
            .apply_runner_event(event(
                run.launch.run_id,
                run.launch.assignment_token,
                kind,
                payload,
            ))
            .await
            .unwrap_or_else(|error| panic!("native retry report {kind} failed: {error:#}"));
    }
    let after = correction_state(store).await;
    let failed = retained_run(&after, run.launch.run_id);
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["verification_status"], "failed");
    assert_eq!(failed["execution_mode"], "provider");
    assert_eq!(failed["workspace_disposition"], "preserved");
    assert_eq!(failed["workspace_fingerprint"], fingerprint);
    assert_ne!(failed["workspace_detail"], "dispatch_not_started");
    assert_eq!(failed["input_tokens"], input_tokens);
    assert_eq!(failed["cost_microusd"], cost_microusd);
    assert_eq!(
        after["task"]["attempt_count"],
        before["task"]["attempt_count"]
    );
    assert_eq!(after["task"]["max_attempts"], 3);
    assert_eq!(after["source"], before["source"]);
    assert_eq!(after["item"]["state"], "verification_failed");
    assert_correction_dispatch(store, &run.command, false).await;
}

async fn retry_failed_fixture(
    pool: PgPool,
    profile: CheckpointFixtureProfile,
    failure: RetryFailure,
    input_tokens: i64,
    cost_microusd: i64,
) -> RetryFixture {
    let fixture = retry_allocated_first(pool, profile).await;
    retry_fail_provider(
        &fixture.store,
        &fixture.first,
        failure,
        input_tokens,
        cost_microusd,
        &"c".repeat(64),
    )
    .await;
    fixture
}

async fn retry_ready(pool: PgPool) -> (RetryFixture, CreateFactoryVerificationRecoveryInput) {
    let fixture = retry_failed_fixture(
        pool,
        retry_profile(),
        RetryFailure::Verification,
        100,
        1_000,
    )
    .await;
    let input = retry_revise(&fixture.store, fixture.first.launch.run_id).await;
    (fixture, input)
}

async fn retry_context(store: &PgStore) -> FactoryVerificationRecoveryContext {
    let before = correction_state(store).await;
    let context = store
        .factory_verification_recovery_context(CORP, OWNER, ITEM)
        .await
        .expect("native current context must be readable")
        .expect("current owner must see the governed recovery");
    assert_eq!(correction_state(store).await, before);
    context
}

fn assert_retry_provider_context(
    context: &FactoryVerificationRecoveryContext,
    source_run_id: Uuid,
    fingerprint: &str,
    remaining_attempts: i32,
) {
    assert_eq!(context.source_run_id, source_run_id);
    assert!(
        context.checkpoint_verification,
        "retain checkpoint-family history"
    );
    assert!(
        !context.checkpoint_verification_available,
        "family history cannot authorize the old checkpoint against changed provider bytes"
    );
    assert_eq!(context.workspace_fingerprint.as_deref(), Some(fingerprint));
    assert!(
        context.expected_head_commit.is_none(),
        "no exported provider HEAD"
    );
    assert_eq!(context.remaining_attempts, remaining_attempts);
}

async fn retry_revise(
    store: &PgStore,
    source_run_id: Uuid,
) -> CreateFactoryVerificationRecoveryInput {
    let before = correction_state(store).await;
    let mut input = correction_request(store, source_run_id).await;
    let context = retry_context(store).await;
    assert_eq!(context.source_run_id, source_run_id);
    input.expected_factory_version = context.work_item.version;
    input.expected_workspace_fingerprint = context.workspace_fingerprint.unwrap();
    input.expected_head_commit = context.expected_head_commit;
    let after = correction_state(store).await;
    for key in ["runs", "recoveries", "commands", "source"] {
        assert_eq!(after[key], before[key], "revision changed existing {key}");
    }
    for old in before["issue210_ledger"]["contract_revisions"]
        .as_array()
        .unwrap()
    {
        assert!(
            after["issue210_ledger"]["contract_revisions"]
                .as_array()
                .unwrap()
                .contains(old),
            "current revision rewrote an earlier revision"
        );
    }
    assert_eq!(
        after["task"]["attempt_count"],
        before["task"]["attempt_count"]
    );
    assert_eq!(after["task"]["max_attempts"], 3);
    input
}

fn retry_recovery(snapshot: &Value, replacement_run_id: Uuid) -> &Value {
    snapshot["recoveries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["replacement_run_id"] == json!(replacement_run_id))
        .expect("exact native recovery row")
}

fn retry_event<'a>(snapshot: &'a Value, aggregate_id: Uuid, kind: &str) -> &'a Value {
    snapshot["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["aggregate_id"] == json!(aggregate_id) && row["type"] == kind)
        .expect("exact native journal row")
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue221_retry_context_and_current_replay_keep_two_correction_chain(pool: PgPool) {
    let fixture = retry_failed_fixture(
        pool,
        retry_profile(),
        RetryFailure::Verification,
        100,
        1_000,
    )
    .await;
    let store = &fixture.store;
    let before_revision = retry_context(store).await;
    assert_retry_provider_context(
        &before_revision,
        fixture.first.launch.run_id,
        &"c".repeat(64),
        1,
    );
    assert_eq!(before_revision.remaining_mission_tokens, 4_600);
    let input = retry_revise(store, fixture.first.launch.run_id).await;
    let revised_context = retry_context(store).await;
    assert_retry_provider_context(
        &revised_context,
        fixture.first.launch.run_id,
        &"c".repeat(64),
        1,
    );
    assert!(revised_context.checkpoint_source_correction);

    // Only a fresh admission uses this optimistic Factory fence. The stored
    // recovery replay contract intentionally does not compare this version.
    let mut stale_version = input.clone();
    stale_version.expected_factory_version -= 1;
    reject_correction_without_changes(store, stale_version).await;
    reject_correction_without_changes(store, fixture.first_input.clone()).await;
    let second = retry_admit(store, &input).await;
    let allocated = correction_state(store).await;
    let prior_grant =
        &retry_recovery(&allocated, fixture.first.launch.run_id)["source_correction_authority"];
    let current_grant =
        &retry_recovery(&allocated, second.launch.run_id)["source_correction_authority"];
    assert_eq!(prior_grant["schema_version"], 1);
    assert_eq!(current_grant["schema_version"], 1);
    assert_eq!(current_grant["checkpoint"], prior_grant["checkpoint"]);
    assert_eq!(
        prior_grant["prefix_run_ids"],
        json!([SOURCE, fixture.verifier])
    );
    assert_eq!(
        current_grant["prefix_run_ids"],
        json!([SOURCE, fixture.verifier, fixture.first.launch.run_id])
    );
    assert_eq!(
        current_grant["contract_revision_id"],
        json!(input.contract_revision_id)
    );
    assert_eq!(allocated["task"]["attempt_count"], 3);
    let active_context = retry_context(store).await;
    assert_retry_provider_context(
        &active_context,
        fixture.first.launch.run_id,
        &"c".repeat(64),
        0,
    );
    assert!(!active_context.checkpoint_source_correction);
    for _ in 0..2 {
        let replay = store
            .create_factory_verification_recovery(input.clone())
            .await
            .expect("current second correction replays without another attempt");
        assert!(replay.replayed);
        assert!(replay.events.is_empty());
        assert_eq!(replay.recovery.id, second.recovery_id);
        let FactoryVerificationRecoveryLaunch::SourceCorrection(launch) = replay.launch else {
            panic!("replay changed provider mode");
        };
        assert_eq!(launch.run_id, second.launch.run_id);
        assert_eq!(launch.assignment_token, second.launch.assignment_token);
        assert_eq!(launch.source_run_id, fixture.first.launch.run_id);
        assert_eq!(
            launch.provider_session_id,
            fixture.first.launch.provider_session_id
        );
        assert_eq!(launch.workspace_run_id, SOURCE);
        assert_eq!(launch.workspace_connection_id, Some(CONNECTION));
        assert_eq!(correction_state(store).await, allocated);
    }
    reject_correction_without_changes(store, fixture.first_input).await;
    assert_correction_dispatch(store, &second.command, true).await;
    assert_eq!(correction_state(store).await, allocated);
}

// These bounded negative fixtures change one explicitly named authority column
// and restore only its exact old value between variants. They do not repair a
// product DB, change run/task lifecycle statuses, or mutate limits/usage/attempt counters.
struct RetryColumnFault {
    label: &'static str,
    sql: &'static str,
    id: Uuid,
    original: Value,
    changed: Option<Value>,
}

fn retry_column_fault(
    label: &'static str,
    sql: &'static str,
    id: Uuid,
    original: &Value,
    changed: Option<Value>,
) -> RetryColumnFault {
    RetryColumnFault {
        label,
        sql,
        id,
        original: original.clone(),
        changed,
    }
}

fn retry_json_fault(
    label: &'static str,
    sql: &'static str,
    id: Uuid,
    original: &Value,
    field: &str,
    value: Value,
) -> RetryColumnFault {
    let mut changed = original.clone();
    changed[field] = value;
    retry_column_fault(label, sql, id, original, Some(changed))
}

async fn retry_fault_denied(
    store: &PgStore,
    input: Option<&CreateFactoryVerificationRecoveryInput>,
    command: Option<&PendingRunnerCommand>,
    fault: &RetryColumnFault,
) {
    assert!(input.is_some() || command.is_some());
    let before = correction_state(store).await;
    let changed = sqlx::query(fault.sql)
        .bind(CORP)
        .bind(fault.id)
        .bind(fault.changed.clone())
        .execute(&store.pool)
        .await
        .unwrap();
    assert_eq!(
        changed.rows_affected(),
        1,
        "fault did not reach {}",
        fault.label
    );
    assert_ne!(
        correction_state(store).await,
        before,
        "no-op {}",
        fault.label
    );
    if let Some(input) = input {
        reject_correction_without_changes(store, input.clone()).await;
    }
    if let Some(command) = command {
        assert_correction_dispatch(store, command, false).await;
    }
    let restored = sqlx::query(fault.sql)
        .bind(CORP)
        .bind(fault.id)
        .bind(Some(fault.original.clone()))
        .execute(&store.pool)
        .await
        .unwrap();
    assert_eq!(restored.rows_affected(), 1);
    assert_eq!(
        correction_state(store).await,
        before,
        "restore scope: {}",
        fault.label
    );
}

const RETRY_GRANT_UPDATE: &str =
    "UPDATE factory_verification_recoveries SET source_correction_authority=$3
     WHERE corp_id=$1 AND replacement_run_id=$2";

fn retry_grant_faults(snapshot: &Value, run_id: Uuid) -> Vec<RetryColumnFault> {
    let original = &retry_recovery(snapshot, run_id)["source_correction_authority"];
    assert_eq!(original["schema_version"], 1);
    vec![
        retry_column_fault("SQL NULL grant", RETRY_GRANT_UPDATE, run_id, original, None),
        retry_column_fault(
            "empty grant",
            RETRY_GRANT_UPDATE,
            run_id,
            original,
            Some(json!({})),
        ),
        retry_json_fault(
            "missing session",
            RETRY_GRANT_UPDATE,
            run_id,
            original,
            "provider_session_id",
            Value::Null,
        ),
        retry_json_fault(
            "changed session",
            RETRY_GRANT_UPDATE,
            run_id,
            original,
            "provider_session_id",
            json!("unrelated-session"),
        ),
        retry_json_fault(
            "shortened prefix",
            RETRY_GRANT_UPDATE,
            run_id,
            original,
            "prefix_run_ids",
            json!([SOURCE]),
        ),
        retry_json_fault(
            "changed revision",
            RETRY_GRANT_UPDATE,
            run_id,
            original,
            "contract_revision_id",
            json!(Uuid::new_v4()),
        ),
    ]
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue221_retry_rejects_missing_and_damaged_prior_and_current_grants(pool: PgPool) {
    let (fixture, input) = retry_ready(pool).await;
    let store = &fixture.store;
    for fault in retry_grant_faults(&correction_state(store).await, fixture.first.launch.run_id) {
        retry_fault_denied(store, Some(&input), None, &fault).await;
    }
    let second = retry_admit(store, &input).await;
    for run_id in [fixture.first.launch.run_id, second.launch.run_id] {
        let before = correction_state(store).await;
        for fault in retry_grant_faults(&before, run_id) {
            retry_fault_denied(store, Some(&input), Some(&second.command), &fault).await;
        }
        // The actual migration rejects JSON null/non-object and schema changes.
        // Do not disable its constraint just to reach the Rust decoder.
        for forbidden in [Value::Null, json!([]), json!({"schema_version":2})] {
            let error = sqlx::query(RETRY_GRANT_UPDATE)
                .bind(CORP)
                .bind(run_id)
                .bind(forbidden)
                .execute(&store.pool)
                .await
                .expect_err("migration must fence invalid grant envelopes");
            assert!(
                error
                    .to_string()
                    .contains("factory_source_correction_authority_check"),
                "unexpected fixture error: {error}"
            );
            assert_eq!(correction_state(store).await, before);
        }
    }
    assert_correction_dispatch(store, &second.command, true).await;
}

async fn retry_denied_at(
    store: &PgStore,
    input: CreateFactoryVerificationRecoveryInput,
    expected_error: &str,
) {
    let before = correction_state(store).await;
    let error = store
        .create_factory_verification_recovery(input)
        .await
        .expect_err("the existing native recovery gate must reject this admission");
    assert!(
        format!("{error:#}").contains(expected_error),
        "did not reach {expected_error}: {error:#}"
    );
    assert_eq!(correction_state(store).await, before);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue221_retry_native_provider_failure_stays_governed_and_bounded(pool: PgPool) {
    let fixture =
        retry_failed_fixture(pool, retry_profile(), RetryFailure::Provider, 100, 1_000).await;
    let store = &fixture.store;
    let failed = correction_state(store).await;
    let first_id = fixture.first.launch.run_id;
    assert!(
        !failed["events"].as_array().unwrap().iter().any(|row| {
            row["aggregate_id"] == json!(first_id)
                && row["type"]
                    .as_str()
                    .is_some_and(|kind| kind.starts_with("run.verification_"))
        }),
        "ordinary failure must reach native recovery terminalization without verifier reports"
    );
    assert_eq!(
        retry_event(&failed, first_id, "run.failed")["payload"]["error"],
        "Synthetic started provider failure before verification"
    );
    let context = retry_context(store).await;
    assert_retry_provider_context(&context, first_id, &"c".repeat(64), 1);

    // Generic resume is not a second retry engine for this Factory lineage.
    let error = store
        .create_resume_run(CORP, first_id, OWNER)
        .await
        .expect_err("a native failed recovery remains governed");
    assert!(format!("{error:#}").contains("governed verification recovery"));
    assert_eq!(correction_state(store).await, failed);

    let input = retry_revise(store, first_id).await;
    let second = retry_admit(store, &input).await;
    assert_eq!(second.launch.source_run_id, first_id);
    assert_eq!(second.launch.workspace_run_id, SOURCE);
    assert_eq!(
        second.launch.provider_session_id,
        fixture.first.launch.provider_session_id
    );
    assert_eq!(second.launch.workspace_connection_id, Some(CONNECTION));
    let admitted = correction_state(store).await;
    assert_eq!(admitted["task"]["attempt_count"], 3);
    assert_eq!(
        retained_run(&admitted, second.launch.run_id)["budget_tokens_limit"],
        4_600
    );

    // Two corrections form the valid stored-policy chain. A third correction
    // would require attempt four, which this fixture never grants or writes.
    retry_fail_provider(
        store,
        &second,
        RetryFailure::Provider,
        50,
        500,
        &"d".repeat(64),
    )
    .await;
    let context = retry_context(store).await;
    assert_retry_provider_context(&context, second.launch.run_id, &"d".repeat(64), 0);
    assert!(!context.checkpoint_source_correction);
    assert_eq!(context.remaining_mission_tokens, 4_550);
    let exhausted = retry_revise(store, second.launch.run_id).await;
    retry_denied_at(store, exhausted, "exhausted its attempt limit").await;
    let after = correction_state(store).await;
    assert_eq!(after["task"]["attempt_count"], 3);
    assert_eq!(after["task"]["max_attempts"], 3);
    assert_eq!(after["runs"].as_array().unwrap().len(), 4);
    assert_eq!(after["source"], failed["source"]);
    assert_eq!(
        retained_run(&after, first_id),
        retained_run(&failed, first_id)
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue221_retry_native_predispatch_failure_keeps_latest_physical_source(pool: PgPool) {
    let fixture = retry_allocated_first(pool, retry_profile()).await;
    let store = &fixture.store;
    let before_failure = correction_state(store).await;
    // No start, workspace, usage, or fabricated verifier report. The public
    // event handler itself records dispatch_not_started and retires the command.
    store
        .apply_runner_event(event(
            fixture.first.launch.run_id,
            fixture.first.launch.assignment_token,
            "run.failed",
            json!({"error":"Synthetic native correction rejection before provider start"}),
        ))
        .await
        .unwrap();
    let failed = correction_state(store).await;
    let unused = retained_run(&failed, fixture.first.launch.run_id);
    assert_eq!(unused["status"], "failed");
    assert_eq!(unused["workspace_detail"], "dispatch_not_started");
    assert!(unused["workspace_path"].is_null());
    assert_eq!(unused["input_tokens"], 0);
    assert_eq!(unused["cost_microusd"], 0);
    assert_eq!(failed["task"]["attempt_count"], 2);
    assert_eq!(failed["source"], before_failure["source"]);
    assert_correction_dispatch(store, &fixture.first.command, false).await;

    let context = retry_context(store).await;
    assert_eq!(context.source_run_id, fixture.verifier);
    assert_ne!(context.source_run_id, fixture.first.launch.run_id);
    assert_eq!(context.workspace_fingerprint, Some("b".repeat(64)));
    assert_eq!(context.expected_head_commit, Some("a".repeat(40)));
    assert_eq!(context.remaining_attempts, 1);
    let input = retry_revise(store, fixture.verifier).await;
    assert_ne!(input.idempotency_key, fixture.first_input.idempotency_key);
    assert_ne!(
        input.contract_revision_id,
        fixture.first_input.contract_revision_id
    );

    // Skipping the run as a physical source cannot skip the failed allocation's
    // immutable revision/grant bridge or revive its superseded original key.
    let snapshot = correction_state(store).await;
    let grant =
        &retry_recovery(&snapshot, fixture.first.launch.run_id)["source_correction_authority"];
    let missing = retry_column_fault(
        "pre-dispatch history still requires its grant",
        RETRY_GRANT_UPDATE,
        fixture.first.launch.run_id,
        grant,
        None,
    );
    retry_fault_denied(store, Some(&input), None, &missing).await;
    reject_correction_without_changes(store, fixture.first_input.clone()).await;
    let second = retry_admit(store, &input).await;
    assert_eq!(second.launch.source_run_id, fixture.verifier);
    assert_eq!(second.launch.workspace_run_id, SOURCE);
    assert_eq!(
        second.launch.provider_session_id,
        fixture.first.launch.provider_session_id
    );
    let after = correction_state(store).await;
    assert_eq!(after["task"]["attempt_count"], 3);
    assert_eq!(after["task"]["max_attempts"], 3);
    assert_eq!(
        retained_run(&after, second.launch.run_id)["budget_tokens_limit"],
        4_700
    );
    assert_eq!(retained_run(&after, fixture.first.launch.run_id), unused);
    assert_eq!(
        retry_recovery(&after, fixture.first.launch.run_id),
        retry_recovery(&snapshot, fixture.first.launch.run_id)
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue221_retry_command_insert_failure_rolls_back_the_full_allocation(pool: PgPool) {
    let (fixture, input) = retry_ready(pool).await;
    let store = &fixture.store;
    // The throw is scoped to the second native command and proves that its
    // run, grant and attempt allocation were reached inside the transaction.
    sqlx::raw_sql(&format!(
        "CREATE FUNCTION issue221_reject_retry_command() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN
           IF NEW.command_kind='factory_verification_recovery'
              AND NEW.payload->>'mode'='source_correction'
              AND NEW.payload->>'source_run_id'='{source}' THEN
             IF NOT EXISTS (
               SELECT 1 FROM runs run
               JOIN tasks task ON task.id=run.task_id AND task.corp_id=run.corp_id
               JOIN factory_verification_recoveries recovery
                 ON recovery.replacement_run_id=run.id AND recovery.corp_id=run.corp_id
               WHERE run.id=NEW.run_id AND run.corp_id=NEW.corp_id
                 AND run.resumed_from_run_id='{source}'::uuid
                 AND run.budget_tokens_limit=4600 AND run.budget_cost_microusd_limit=1000000
                 AND task.attempt_count=3 AND task.max_attempts=3
                 AND recovery.source_correction_authority->>'schema_version'='1'
             ) THEN
               RAISE EXCEPTION 'issue221 retry trigger reached before allocation';
             END IF;
             RAISE EXCEPTION 'issue221 owned post-allocation retry command fault';
           END IF;
           RETURN NEW;
         END $$;
         CREATE TRIGGER issue221_retry_command_fault BEFORE INSERT ON runner_commands
           FOR EACH ROW EXECUTE FUNCTION issue221_reject_retry_command();",
        source = fixture.first.launch.run_id,
    ))
    .execute(&store.pool)
    .await
    .unwrap();
    let before = correction_state(store).await;
    retry_denied_at(
        store,
        input.clone(),
        "issue221 owned post-allocation retry command fault",
    )
    .await;
    assert_eq!(before["task"]["attempt_count"], 2);
    assert_eq!(correction_state(store).await, before);
    sqlx::raw_sql(
        "DROP TRIGGER issue221_retry_command_fault ON runner_commands;
         DROP FUNCTION issue221_reject_retry_command();",
    )
    .execute(&store.pool)
    .await
    .unwrap();
    let second = retry_admit(store, &input).await;
    assert_eq!(second.launch.source_run_id, fixture.first.launch.run_id);
    assert_retry_preserves_history(&before, &correction_state(store).await);
}

fn retry_revision(snapshot: &Value, id: Uuid) -> &Value {
    snapshot["issue210_ledger"]["contract_revisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|revision| revision["id"] == json!(id))
        .expect("exact immutable contract revision")
}

fn retry_history_faults(
    fixture: &RetryFixture,
    input: &CreateFactoryVerificationRecoveryInput,
    snapshot: &Value,
) -> Vec<RetryColumnFault> {
    let run_id = fixture.first.launch.run_id;
    let run = retained_run(snapshot, run_id);
    let recovery = retry_recovery(snapshot, run_id);
    let authorized = retry_event(
        snapshot,
        fixture.first.recovery_id,
        "factory.verification_recovery_authorized",
    );
    let checkpoint = retry_event(snapshot, SOURCE, "run.workspace_preserved");
    let terminated = retry_event(snapshot, SOURCE, "run.session_terminated");
    let prior_id = fixture.first_input.contract_revision_id.unwrap();
    let current_id = input.contract_revision_id.unwrap();
    let prior = retry_revision(snapshot, prior_id);
    let current = retry_revision(snapshot, current_id);
    let recovery_request =
        "UPDATE factory_verification_recoveries SET request=$3 WHERE corp_id=$1 AND id=$2";
    let event_payload = "UPDATE events SET payload=$3 WHERE corp_id=$1 AND id=$2";
    let revision_source =
        "UPDATE mission_contract_revisions SET source_run_id=($3::jsonb#>>'{}')::uuid
         WHERE corp_id=$1 AND id=$2";
    let mut damaged_checkpoint = checkpoint["payload"].clone();
    damaged_checkpoint["source_checkpoint"]["workspace_fingerprint"] = json!("e".repeat(64));
    vec![
        retry_column_fault(
            "native parent",
            "UPDATE runs SET resumed_from_run_id=($3::jsonb#>>'{}')::uuid WHERE corp_id=$1 AND id=$2",
            run_id,
            &run["resumed_from_run_id"],
            Some(json!(SOURCE)),
        ),
        retry_column_fault(
            "recovery parent",
            "UPDATE factory_verification_recoveries SET source_run_id=($3::jsonb#>>'{}')::uuid WHERE corp_id=$1 AND id=$2",
            fixture.first.recovery_id,
            &recovery["source_run_id"],
            Some(json!(SOURCE)),
        ),
        retry_column_fault(
            "historical author",
            "UPDATE factory_verification_recoveries SET authorized_by=($3::jsonb#>>'{}')::uuid WHERE corp_id=$1 AND id=$2",
            fixture.first.recovery_id,
            &recovery["authorized_by"],
            Some(json!(REVIEWER)),
        ),
        retry_json_fault(
            "historical request parent",
            recovery_request,
            fixture.first.recovery_id,
            &recovery["request"],
            "source_run_id",
            json!(SOURCE),
        ),
        retry_json_fault(
            "historical request checkpoint",
            recovery_request,
            fixture.first.recovery_id,
            &recovery["request"],
            "expected_workspace_fingerprint",
            json!("e".repeat(64)),
        ),
        retry_json_fault(
            "historical command assignment",
            "UPDATE runner_commands SET payload=$3 WHERE corp_id=$1 AND id=$2",
            fixture.first.command.id,
            &fixture.first.command.payload,
            "assignment_token",
            json!(Uuid::new_v4()),
        ),
        retry_json_fault(
            "historical authorization event",
            event_payload,
            Uuid::parse_str(authorized["id"].as_str().unwrap()).unwrap(),
            &authorized["payload"],
            "replacement_run_id",
            json!(SOURCE),
        ),
        retry_column_fault(
            "native checkpoint journal",
            event_payload,
            Uuid::parse_str(checkpoint["id"].as_str().unwrap()).unwrap(),
            &checkpoint["payload"],
            Some(damaged_checkpoint),
        ),
        retry_json_fault(
            "native termination journal",
            event_payload,
            Uuid::parse_str(terminated["id"].as_str().unwrap()).unwrap(),
            &terminated["payload"],
            "provider_process_alive",
            json!(true),
        ),
        // Five independently corrupted revision bindings: no revision is
        // rewritten to make a successful retry; each negative restores only
        // that column before the next case.
        retry_column_fault(
            "prior revision source",
            revision_source,
            prior_id,
            &prior["source_run_id"],
            Some(json!(SOURCE)),
        ),
        retry_column_fault(
            "current revision source",
            revision_source,
            current_id,
            &current["source_run_id"],
            Some(json!(fixture.verifier)),
        ),
        retry_column_fault(
            "current revision author",
            "UPDATE mission_contract_revisions SET revised_by=($3::jsonb#>>'{}')::uuid WHERE corp_id=$1 AND id=$2",
            current_id,
            &current["revised_by"],
            Some(json!(REVIEWER)),
        ),
        retry_json_fault(
            "revision ledger bridge",
            "UPDATE mission_contract_revisions SET previous_contract=$3 WHERE corp_id=$1 AND id=$2",
            current_id,
            &current["previous_contract"],
            "objective",
            json!("Unrelated historical contract"),
        ),
        retry_json_fault(
            "current revision projection",
            "UPDATE mission_contract_revisions SET replacement_contract=$3 WHERE corp_id=$1 AND id=$2",
            current_id,
            &current["replacement_contract"],
            "objective",
            json!("Unapproved replacement contract"),
        ),
    ]
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue221_retry_rechecks_native_history_and_revision_ledger_bridge(pool: PgPool) {
    let (fixture, input) = retry_ready(pool).await;
    let store = &fixture.store;
    let before = correction_state(store).await;
    let faults = retry_history_faults(&fixture, &input, &before);
    for fault in &faults {
        retry_fault_denied(store, Some(&input), None, fault).await;
    }
    assert_eq!(correction_state(store).await, before);
    let second = retry_admit(store, &input).await;
    // The same prior edge remains authority for the pending command and the
    // current idempotent readback, not merely for the initial admission.
    for fault in &faults {
        retry_fault_denied(store, Some(&input), Some(&second.command), fault).await;
    }
    assert_correction_dispatch(store, &second.command, true).await;
    assert_retry_preserves_history(&before, &correction_state(store).await);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue221_retry_fences_current_scope_request_and_pending_command(pool: PgPool) {
    let (fixture, input) = retry_ready(pool).await;
    let store = &fixture.store;
    let mut foreign_corp = input.clone();
    foreign_corp.corp_id = Uuid::new_v4();
    let mut foreign_item = input.clone();
    foreign_item.work_item_id = Uuid::new_v4();
    let mut member = input.clone();
    member.actor_id = REVIEWER;
    let mut old_source = input.clone();
    old_source.source_run_id = fixture.verifier;
    old_source.expected_workspace_fingerprint = "b".repeat(64);
    old_source.expected_head_commit = Some("a".repeat(40));
    let mut old_revision = input.clone();
    old_revision.contract_revision_id = fixture.first_input.contract_revision_id;
    let mut no_revision = input.clone();
    no_revision.contract_revision_id = None;
    let mut old_fingerprint = input.clone();
    old_fingerprint.expected_workspace_fingerprint = "b".repeat(64);
    let mut old_head = input.clone();
    old_head.expected_head_commit = Some("a".repeat(40));
    for rejected in [
        foreign_corp,
        foreign_item,
        member,
        old_source,
        old_revision,
        no_revision,
        old_fingerprint,
        old_head,
    ] {
        reject_correction_without_changes(store, rejected).await;
    }

    let second = retry_admit(store, &input).await;
    let mut changed_reason = input.clone();
    changed_reason.reason.push_str(" different authorization");
    let mut changed_revision = input.clone();
    changed_revision.contract_revision_id = fixture.first_input.contract_revision_id;
    let mut changed_source = input.clone();
    changed_source.source_run_id = fixture.verifier;
    let mut changed_head = input.clone();
    changed_head.expected_head_commit = Some("a".repeat(40));
    let mut changed_fingerprint = input.clone();
    changed_fingerprint.expected_workspace_fingerprint = "b".repeat(64);
    let mut changed_review = input.clone();
    changed_review.observed_source_revision = "different-reviewed-source".to_owned();
    changed_review.reviewed_source_snapshot["source_revision"] =
        json!(changed_review.observed_source_revision);
    let mut verifier_mode = input.clone();
    verifier_mode.mode = FactoryVerificationRecoveryMode::VerifierOnly;
    let mut checkpoint_mode = input.clone();
    checkpoint_mode.mode = FactoryVerificationRecoveryMode::CheckpointVerification;
    for rejected in [
        changed_reason,
        changed_revision,
        changed_source,
        changed_head,
        changed_fingerprint,
        changed_review,
        verifier_mode,
        checkpoint_mode,
    ] {
        reject_correction_without_changes(store, rejected).await;
    }

    let mut foreign_command = second.command.clone();
    foreign_command.corp_id = Uuid::new_v4();
    assert_correction_dispatch(store, &foreign_command, false).await;
    let mut old_run = second.command.clone();
    old_run.run_id = fixture.first.launch.run_id;
    assert_correction_dispatch(store, &old_run, false).await;
    for (field, value) in [
        ("mode", json!("verifier_only")),
        ("assignment_token", json!(Uuid::new_v4())),
        ("source_run_id", json!(fixture.verifier)),
        ("provider_session_id", json!("different-session")),
        ("workspace_connection_id", Value::Null),
        ("expected_workspace_fingerprint", json!("b".repeat(64))),
        ("expected_head_commit", json!("a".repeat(40))),
        ("verification_policy", json!({"checks":[]})),
        ("write_scope", json!(["unapproved.md"])),
    ] {
        let mut forged = second.command.clone();
        forged.payload[field] = value;
        assert_correction_dispatch(store, &forged, false).await;
        let persisted = retry_column_fault(
            "persisted current command binding",
            "UPDATE runner_commands SET payload=$3 WHERE corp_id=$1 AND id=$2",
            second.command.id,
            &second.command.payload,
            Some(forged.payload.clone()),
        );
        // Passing the matching tampered payload must also fail native binding;
        // rejection cannot be only equality against the old pending payload.
        // A read-only recovery receipt is not itself command dispatch authority.
        retry_fault_denied(store, None, Some(&forged), &persisted).await;
    }
    assert_correction_dispatch(store, &second.command, true).await;
}

fn retry_authority_faults(snapshot: &Value, source_run_id: Uuid) -> Vec<RetryColumnFault> {
    let owner = snapshot["actors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|actor| actor["id"] == json!(OWNER))
        .unwrap();
    let connection = snapshot["issue210_ledger"]["connections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|connection| connection["id"] == json!(CONNECTION))
        .unwrap();
    let mut faults = vec![
        retry_column_fault(
            "current author role",
            "UPDATE actors SET role=$3::jsonb#>>'{}' WHERE corp_id=$1 AND id=$2",
            OWNER,
            &owner["role"],
            Some(json!("member")),
        ),
        retry_column_fault(
            "current saved connection offline",
            "UPDATE workspace_connections SET status=$3::jsonb#>>'{}' WHERE corp_id=$1 AND id=$2",
            CONNECTION,
            &connection["status"],
            Some(json!("offline")),
        ),
    ];
    for run_id in [SOURCE, source_run_id] {
        let run = retained_run(snapshot, run_id);
        for (label, sql, field, value) in [
            (
                "source identity",
                "UPDATE runs SET source_base_commit=$3::jsonb#>>'{}' WHERE corp_id=$1 AND id=$2",
                "source_base_commit",
                json!("e".repeat(40)),
            ),
            (
                "provider session",
                "UPDATE runs SET provider_session_id=$3::jsonb#>>'{}' WHERE corp_id=$1 AND id=$2",
                "provider_session_id",
                Value::Null,
            ),
            (
                "saved connection binding",
                "UPDATE runs SET workspace_connection_id=($3::jsonb#>>'{}')::uuid WHERE corp_id=$1 AND id=$2",
                "workspace_connection_id",
                Value::Null,
            ),
            (
                "protected stop",
                "UPDATE runs SET breaker_stage=$3::jsonb#>>'{}' WHERE corp_id=$1 AND id=$2",
                "breaker_stage",
                json!("stop"),
            ),
            (
                "protected quarantine",
                "UPDATE runs SET workspace_disposition=$3::jsonb#>>'{}' WHERE corp_id=$1 AND id=$2",
                "workspace_disposition",
                json!("quarantined"),
            ),
        ] {
            faults.push(retry_column_fault(
                label,
                sql,
                run_id,
                &run[field],
                Some(value),
            ));
        }
    }
    // The original measured suspension is the sole exception. A secondary
    // suspension, without that origin's native proof, must not inherit it.
    faults.push(retry_column_fault(
        "unrelated secondary suspension",
        "UPDATE runs SET breaker_stage=$3::jsonb#>>'{}' WHERE corp_id=$1 AND id=$2",
        source_run_id,
        &retained_run(snapshot, source_run_id)["breaker_stage"],
        Some(json!("suspend")),
    ));
    faults
}

async fn retry_room_revocation(
    store: &PgStore,
    input: &CreateFactoryVerificationRecoveryInput,
    command: Option<&PendingRunnerCommand>,
) {
    let before = correction_state(store).await;
    let membership = before["memberships"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["room_id"] == json!(ROOM) && row["actor_id"] == json!(OWNER))
        .unwrap()
        .clone();
    let deleted = sqlx::query(
        "DELETE FROM room_memberships membership USING rooms room
         WHERE room.id=membership.room_id AND room.corp_id=$1
           AND membership.actor_id=$2 AND membership.room_id=$3",
    )
    .bind(CORP)
    .bind(OWNER)
    .bind(ROOM)
    .execute(&store.pool)
    .await
    .unwrap();
    assert_eq!(deleted.rows_affected(), 1);
    let revoked = correction_state(store).await;
    reject_correction_without_changes(store, input.clone()).await;
    if let Some(command) = command {
        assert_correction_dispatch(store, command, false).await;
    }
    assert!(
        store
            .factory_verification_recovery_context(CORP, OWNER, ITEM)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(correction_state(store).await, revoked);
    // Restore only the exact intentionally removed membership, including its
    // original joined_at/role, not an actor, mission, run or budget projection.
    let restored = sqlx::query(
        "INSERT INTO room_memberships
         SELECT saved.* FROM jsonb_populate_record(NULL::room_memberships,$1::jsonb) saved",
    )
    .bind(membership)
    .execute(&store.pool)
    .await
    .unwrap();
    assert_eq!(restored.rows_affected(), 1);
    assert_eq!(correction_state(store).await, before);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue221_retry_rechecks_current_authority_and_protected_lineage(pool: PgPool) {
    let (fixture, input) = retry_ready(pool).await;
    let store = &fixture.store;
    let faults =
        retry_authority_faults(&correction_state(store).await, fixture.first.launch.run_id);
    for fault in &faults {
        retry_fault_denied(store, Some(&input), None, fault).await;
    }
    retry_room_revocation(store, &input, None).await;
    let second = retry_admit(store, &input).await;
    for fault in &faults {
        // Current role and room gate replay too. An unchanged read-only receipt
        // is not required to become a fresh connection readiness check.
        let replay = (fault.id == OWNER).then_some(&input);
        retry_fault_denied(store, replay, Some(&second.command), fault).await;
    }
    retry_room_revocation(store, &input, Some(&second.command)).await;
    let before_stop = correction_state(store).await;
    let quarantine = retry_column_fault(
        "active replacement quarantine",
        "UPDATE runs SET workspace_disposition=$3::jsonb#>>'{}' WHERE corp_id=$1 AND id=$2",
        second.launch.run_id,
        &retained_run(&before_stop, second.launch.run_id)["workspace_disposition"],
        Some(json!("quarantined")),
    );
    retry_fault_denied(store, None, Some(&second.command), &quarantine).await;
    assert_correction_dispatch(store, &second.command, true).await;
    let stopped = store
        .request_emergency_stop(CORP, AGENT, OWNER, "Explicitly stop the current retry")
        .await
        .unwrap();
    assert_eq!(stopped.run_id, second.launch.run_id);
    assert_correction_dispatch(store, &second.command, false).await;
    let after_stop = correction_state(store).await;
    assert_eq!(after_stop["source"], before_stop["source"]);
    assert_eq!(
        retained_run(&after_stop, fixture.first.launch.run_id),
        retained_run(&before_stop, fixture.first.launch.run_id)
    );
    assert_eq!(after_stop["task"]["attempt_count"], 3);
    assert_eq!(after_stop["task"]["max_attempts"], 3);
}

async fn retry_exhausted_spend(pool: PgPool, cost: bool) {
    let profile = if cost {
        CheckpointFixtureProfile {
            mission_cost_microusd: 1_000_000,
            ..retry_profile()
        }
    } else {
        retry_profile()
    };
    let fixture = retry_failed_fixture(
        pool,
        profile,
        RetryFailure::Provider,
        if cost { 100 } else { 4_700 },
        if cost { 1_000_000 } else { 1_000 },
    )
    .await;
    let store = &fixture.store;
    let context = retry_context(store).await;
    assert_retry_provider_context(&context, fixture.first.launch.run_id, &"c".repeat(64), 1);
    assert!(!context.checkpoint_source_correction);
    if cost {
        assert_eq!(context.remaining_mission_cost_microusd, 0);
        assert_eq!(context.remaining_mission_tokens, 4_600);
    } else {
        assert_eq!(context.remaining_mission_tokens, 0);
        assert_eq!(context.remaining_mission_cost_microusd, 4_999_000);
    }
    let input = retry_revise(store, fixture.first.launch.run_id).await;
    retry_denied_at(store, input, "no remaining mission budget").await;
    let after = correction_state(store).await;
    assert_eq!(
        after["task"]["attempt_count"], 2,
        "an attempt still remains"
    );
    assert_eq!(after["task"]["max_attempts"], 3);
    assert_eq!(after["runs"].as_array().unwrap().len(), 3);
    assert_eq!(after["source"]["input_tokens"], 5_300);
    assert_eq!(after["source"]["cost_microusd"], 0);
    assert_eq!(after["mission"]["budget_tokens"], 10_000);
    assert_eq!(after["mission"]["original_budget_tokens"], 10_000);
    assert_eq!(
        after["mission"]["budget_cost_microusd"],
        profile.mission_cost_microusd
    );
    assert_eq!(
        after["mission"]["original_budget_cost_microusd"],
        profile.mission_cost_microusd
    );
    assert!(
        after["issue210_ledger"]["budget_revisions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let used = retained_run(&after, fixture.first.launch.run_id);
    assert_eq!(used["input_tokens"], if cost { 100 } else { 4_700 });
    assert_eq!(used["cost_microusd"], if cost { 1_000_000 } else { 1_000 });
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue221_retry_does_not_reset_exhausted_token_spend(pool: PgPool) {
    retry_exhausted_spend(pool, false).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue221_retry_does_not_reset_exhausted_cost_spend(pool: PgPool) {
    retry_exhausted_spend(pool, true).await;
}

async fn legacy_verifier_seal_failed_correction(pool: PgPool) -> RetryFixture {
    let fixture = retry_allocated_first_with_verifier_seal(pool, retry_profile(), true).await;
    retry_fail_provider(
        &fixture.store,
        &fixture.first,
        RetryFailure::Verification,
        100,
        1_000,
        &"c".repeat(64),
    )
    .await;
    fixture
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue224_legacy_verifier_seal_allows_context_revision_retry_and_exact_replay(
    pool: PgPool,
) {
    let fixture = legacy_verifier_seal_failed_correction(pool).await;
    let store = &fixture.store;
    let failed = correction_state(store).await;
    let seal = retry_event(&failed, fixture.verifier, "run.workspace_preserved");
    assert!(seal["payload"].get("head_commit").is_none());
    assert_eq!(seal["payload"]["workspace_fingerprint"], "b".repeat(64));
    assert_eq!(failed["task"]["attempt_count"], 2);

    let mut tx = store.pool.begin().await.unwrap();
    let checkpoint =
        source_workspace_checkpoint_with_lock_tx(&mut tx, CORP, fixture.verifier, false)
            .await
            .unwrap();
    checkpoint.ensure_preserved().unwrap();
    assert_eq!(checkpoint.execution_mode, "verification_only");
    assert_eq!(
        checkpoint.expected_verifier_fingerprint,
        Some("b".repeat(64))
    );
    assert_eq!(checkpoint.expected_head_commit, Some("a".repeat(40)));
    tx.rollback().await.unwrap();
    assert_eq!(correction_state(store).await, failed);

    let before_revision = retry_context(store).await;
    assert_retry_provider_context(
        &before_revision,
        fixture.first.launch.run_id,
        &"c".repeat(64),
        1,
    );
    assert!(before_revision.checkpoint_source_correction);
    assert_eq!(before_revision.remaining_mission_tokens, 4_600);

    // Availability describes this family, not a substitute for a new
    // source-bound Resume revision. Neither absence nor the old revision admits.
    let mut unrevised = fixture.first_input.clone();
    unrevised.idempotency_key = Uuid::new_v4();
    unrevised.source_run_id = fixture.first.launch.run_id;
    unrevised.expected_factory_version = before_revision.work_item.version;
    unrevised.expected_workspace_fingerprint =
        before_revision.workspace_fingerprint.clone().unwrap();
    unrevised.expected_head_commit = before_revision.expected_head_commit.clone();
    for revision_id in [None, fixture.first_input.contract_revision_id] {
        unrevised.contract_revision_id = revision_id;
        reject_correction_without_changes(store, unrevised.clone()).await;
    }
    assert_eq!(correction_state(store).await, failed);

    let input = retry_revise(store, fixture.first.launch.run_id).await;
    let revised = retry_context(store).await;
    assert_retry_provider_context(&revised, fixture.first.launch.run_id, &"c".repeat(64), 1);
    assert!(revised.checkpoint_source_correction);
    assert_eq!(revised.remaining_mission_tokens, 4_600);
    let before_admission = correction_state(store).await;
    let second = retry_admit(store, &input).await;
    let allocated = correction_state(store).await;
    assert_eq!(allocated["task"]["attempt_count"], 3);
    assert_retry_preserves_history(&before_admission, &allocated);
    assert_eq!(second.launch.source_run_id, fixture.first.launch.run_id);
    assert_eq!(second.launch.workspace_run_id, SOURCE);
    assert_eq!(second.launch.workspace_connection_id, Some(CONNECTION));
    assert_eq!(second.launch.provider_session_id, "fixture-session");
    assert_eq!(
        retained_run(&allocated, second.launch.run_id)["budget_tokens_limit"],
        4_600
    );

    let replay = store
        .create_factory_verification_recovery(input)
        .await
        .expect("the exact current correction must replay with the legacy verifier seal");
    assert!(replay.replayed);
    assert!(replay.events.is_empty());
    assert_eq!(replay.recovery.id, second.recovery_id);
    let FactoryVerificationRecoveryLaunch::SourceCorrection(launch) = replay.launch else {
        panic!("replay must retain provider correction mode");
    };
    assert_eq!(launch.run_id, second.launch.run_id);
    assert_eq!(launch.assignment_token, second.launch.assignment_token);
    assert_eq!(launch.source_run_id, fixture.first.launch.run_id);
    assert_eq!(launch.workspace_run_id, SOURCE);
    assert_eq!(
        launch.provider_session_id,
        second.launch.provider_session_id
    );
    assert_correction_dispatch(store, &second.command, true).await;
    assert_eq!(correction_state(store).await, allocated);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue224_legacy_verifier_seal_rejects_contradictory_head_without_mutation(pool: PgPool) {
    let fixture = legacy_verifier_seal_failed_correction(pool).await;
    let store = &fixture.store;
    let input = retry_revise(store, fixture.first.launch.run_id).await;
    let snapshot = correction_state(store).await;
    let seal = retry_event(&snapshot, fixture.verifier, "run.workspace_preserved");
    let fault = retry_json_fault(
        "contradictory explicit verifier seal head",
        "UPDATE events SET payload=$3 WHERE corp_id=$1 AND id=$2",
        Uuid::parse_str(seal["id"].as_str().unwrap()).unwrap(),
        &seal["payload"],
        "head_commit",
        json!("e".repeat(40)),
    );
    retry_fault_denied(store, Some(&input), None, &fault).await;
    let second = retry_admit(store, &input).await;
    retry_fault_denied(store, Some(&input), Some(&second.command), &fault).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue224_legacy_verifier_seal_requires_matching_fingerprint_and_verifier_grant(
    pool: PgPool,
) {
    let fixture = legacy_verifier_seal_failed_correction(pool).await;
    let store = &fixture.store;
    let input = retry_revise(store, fixture.first.launch.run_id).await;
    let snapshot = correction_state(store).await;
    let seal = retry_event(&snapshot, fixture.verifier, "run.workspace_preserved");
    let seal_id = Uuid::parse_str(seal["id"].as_str().unwrap()).unwrap();
    let mut missing_fingerprint = seal["payload"].clone();
    assert_eq!(
        missing_fingerprint
            .as_object_mut()
            .unwrap()
            .remove("workspace_fingerprint"),
        Some(json!("b".repeat(64)))
    );
    let verifier = retry_recovery(&snapshot, fixture.verifier);
    let verifier_id = Uuid::parse_str(verifier["id"].as_str().unwrap()).unwrap();
    let mut missing_authorized_fingerprint = verifier["request"].clone();
    assert_eq!(
        missing_authorized_fingerprint
            .as_object_mut()
            .unwrap()
            .remove("expected_workspace_fingerprint"),
        Some(json!("b".repeat(64)))
    );
    let faults = [
        retry_column_fault(
            "missing verifier seal fingerprint",
            "UPDATE events SET payload=$3 WHERE corp_id=$1 AND id=$2",
            seal_id,
            &seal["payload"],
            Some(missing_fingerprint),
        ),
        retry_json_fault(
            "changed verifier seal fingerprint",
            "UPDATE events SET payload=$3 WHERE corp_id=$1 AND id=$2",
            seal_id,
            &seal["payload"],
            "workspace_fingerprint",
            json!("e".repeat(64)),
        ),
        retry_column_fault(
            "missing verifier authorization fingerprint",
            "UPDATE factory_verification_recoveries SET request=$3 WHERE corp_id=$1 AND id=$2",
            verifier_id,
            &verifier["request"],
            Some(missing_authorized_fingerprint),
        ),
        retry_json_fault(
            "broken verifier checkpoint grant",
            "UPDATE factory_verification_recoveries SET checkpoint_authority=$3 WHERE corp_id=$1 AND id=$2",
            verifier_id,
            &verifier["checkpoint_authority"],
            "checkpoint_event_id",
            json!(Uuid::new_v4()),
        ),
    ];
    for fault in &faults {
        retry_fault_denied(store, Some(&input), None, fault).await;
    }
}
