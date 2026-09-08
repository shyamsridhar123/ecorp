//! Actual-store metadata tests in SQLx-owned disposable databases, not provider
//! execution, physical source verification, or signed-object acceptance.
use super::*;
use crony_domain::StoppedSourceCheckpoint;

const CORP: Uuid = Uuid::from_u128(1);
const MISSION: Uuid = Uuid::from_u128(2);
const TASK: Uuid = Uuid::from_u128(3);
const SOURCE: Uuid = Uuid::from_u128(4);
const AGENT: Uuid = Uuid::from_u128(5);
const ROOM: Uuid = Uuid::from_u128(6);
const ITEM: Uuid = Uuid::from_u128(7);
const TOKEN: Uuid = Uuid::from_u128(8);
const OWNER: Uuid = Uuid::from_u128(9);
const REVIEWER: Uuid = Uuid::from_u128(11);
const CLAIM: Uuid = Uuid::from_u128(16);
const RUNNER: &str = "issue148-checkpoint-runner";

fn event(run_id: Uuid, token: Uuid, kind: &str, payload: Value) -> RunnerEventInput {
    RunnerEventInput {
        event_id: Uuid::new_v4(),
        runner_id: RUNNER.to_owned(),
        corp_id: CORP,
        connection_epoch: Uuid::from_u128(20),
        run_id,
        agent_id: AGENT,
        assignment_token: token,
        event_type: kind.to_owned(),
        payload,
    }
}

fn digest(value: &impl serde::Serialize) -> String {
    hex::encode(Sha256::digest(serde_json::to_vec(value).unwrap()))
}

fn gate() -> Value {
    json!({"type":"independent_review","roles":["owner","member"],"exclude_requester":true})
}

async fn fixture(pool: PgPool, needs_artifact: bool) -> PgStore {
    sqlx::raw_sql(
        r#"
        INSERT INTO corps(id,slug,name) VALUES
          ('00000000-0000-0000-0000-000000000001','checkpoint148','Checkpoint fixture');
        INSERT INTO actors(id,corp_id,name,kind,role) VALUES
          ('00000000-0000-0000-0000-000000000009','00000000-0000-0000-0000-000000000001','Owner','human','owner'),
          ('00000000-0000-0000-0000-00000000000a','00000000-0000-0000-0000-000000000001','Worker','agent','worker'),
          ('00000000-0000-0000-0000-00000000000b','00000000-0000-0000-0000-000000000001','Reviewer','human','member');
        INSERT INTO rooms(id,corp_id,name,purpose) VALUES
          ('00000000-0000-0000-0000-000000000006','00000000-0000-0000-0000-000000000001','Fixture','Store-only checkpoint acceptance');
        INSERT INTO room_memberships(room_id,actor_id) VALUES
          ('00000000-0000-0000-0000-000000000006','00000000-0000-0000-0000-000000000009'),
          ('00000000-0000-0000-0000-000000000006','00000000-0000-0000-0000-00000000000b');
        INSERT INTO agents(id,corp_id,actor_id,name,role,adapter,status,current_run_id,accent) VALUES
          ('00000000-0000-0000-0000-000000000005','00000000-0000-0000-0000-000000000001',
           '00000000-0000-0000-0000-00000000000a','Worker','worker','codex','working',
           '00000000-0000-0000-0000-000000000004','#123456');
        INSERT INTO missions(id,corp_id,room_id,requested_by,title,status,budget_tokens,
                             original_budget_tokens,original_budget_cost_microusd) VALUES
          ('00000000-0000-0000-0000-000000000002','00000000-0000-0000-0000-000000000001',
           '00000000-0000-0000-0000-000000000006','00000000-0000-0000-0000-000000000009',
           'Preserve completed source','running',5000,5000,5000000);
        "#,
    ).execute(&pool).await.unwrap();
    let contract: TaskContract = serde_json::from_value(json!({
        "objective":"verify result.md", "expected_output":"result.md",
        "source_repository":"fixture/source", "source_base_ref":"main",
        "source_base_commit":"a".repeat(40), "acceptance_tests":["result.md is present"],
        "allowed_tools":["filesystem"], "prohibited_actions":["no external effects"],
        "references":[], "write_scope":["result.md"], "budget_tokens":5000,
        "budget_cost_microusd":1000000, "deadline_at":null, "escalation":"ask owner",
        "model":"fixture-model", "reasoning_effort":"medium"
    }))
    .unwrap();
    let mut checks = vec![json!({"type":"file","path":"result.md","min_bytes":1})];
    if needs_artifact {
        checks.push(json!({"type":"artifact","min_bytes":1}));
    }
    let policy: VerificationPolicy =
        serde_json::from_value(json!({"checks":checks,"manual_gate":gate()})).unwrap();
    sqlx::query(
        "INSERT INTO tasks(id,corp_id,mission_id,title,objective,status,assigned_agent_id,
          required_adapter,plan_key,contract,verification_policy,attempt_count,max_attempts)
         VALUES($1,$2,$3,'Deliver','verify result.md','running',$4,'codex','deliver',$5,$6,2,2)",
    )
    .bind(TASK)
    .bind(CORP)
    .bind(MISSION)
    .bind(AGENT)
    .bind(serde_json::to_value(&contract).unwrap())
    .bind(serde_json::to_value(&policy).unwrap())
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO runs(id,corp_id,task_id,agent_id,runner_id,assignment_token,status,
           workspace_run_id,provider_session_id,workspace_path,workspace_branch,
           workspace_base_ref,workspace_base_commit,workspace_disposition,
           source_repository,source_base_ref,source_base_commit,model,reasoning_effort)
         VALUES($1,$2,$3,$4,$5,$6,'running',$1,'fixture-session','fixture-worktree',
           'crony/fixture','main',$7,'active','fixture/source','main',$7,'fixture-model','medium')",
    )
    .bind(SOURCE)
    .bind(CORP)
    .bind(TASK)
    .bind(AGENT)
    .bind(RUNNER)
    .bind(TOKEN)
    .bind("a".repeat(40))
    .execute(&pool)
    .await
    .unwrap();
    let factory_policy = json!({
        "source_base_ref":"main", "source_base_commit":"a".repeat(40),
        "repository_allowlist":["fixture/source"], "write_scope":["result.md"],
        "allowed_tools":["filesystem"], "prohibited_actions":["no external effects"]
    });
    sqlx::query(
        "INSERT INTO factory_work_items(id,corp_id,source_kind,source_project_owner,
          source_project_number,source_project_item_id,source_repository_owner,source_repository_name,
          source_issue_number,source_issue_node_id,source_issue_url,source_title,source_revision,
          state,claim_owner_id,claim_token,lease_expires_at,mission_id,policy)
         VALUES($1,$2,'github_project_issue','fixture',148,'item148','fixture','source',148,
          'issue148','https://github.com/fixture/source/issues/148','Checkpoint','revision-1',
          'running',$3,$4,now()+interval '1 hour',$5,$6)",
    ).bind(ITEM).bind(CORP).bind(OWNER).bind(CLAIM).bind(MISSION).bind(factory_policy)
        .execute(&pool).await.unwrap();
    let store = PgStore { pool };
    store
        .apply_runner_event(event(
            SOURCE,
            TOKEN,
            "run.usage",
            json!({"input_tokens":6000,"output_tokens":0,"cost_microusd":0}),
        ))
        .await
        .unwrap();
    let breaker = store.evaluate_circuit_breaker(CORP, SOURCE).await.unwrap();
    assert_eq!(breaker.event.unwrap().payload["stage"], "stop");
    store
        .apply_runner_event(event(
            SOURCE,
            TOKEN,
            "run.session_terminated",
            json!({"adapter":"codex","outcome":"cancelled","provider_process_alive":false}),
        ))
        .await
        .unwrap();
    let proof = StoppedSourceCheckpoint {
        schema_version: 1,
        corp_id: CORP,
        mission_id: MISSION,
        task_id: TASK,
        run_id: SOURCE,
        workspace_run_id: SOURCE,
        agent_id: AGENT,
        runner_id: RUNNER.to_owned(),
        source_repository: "fixture/source".to_owned(),
        source_base_ref: "main".to_owned(),
        source_base_commit: "a".repeat(40),
        workspace_base_commit: "a".repeat(40),
        branch: "crony/fixture".to_owned(),
        head_commit: "a".repeat(40),
        workspace_fingerprint: "b".repeat(64),
        verification_policy_sha256: digest(&policy),
        write_scope_sha256: digest(&contract.write_scope),
        deliverable_policy_sha256: digest(&contract.deliverable),
    };
    store
        .apply_runner_event(event(
            SOURCE,
            TOKEN,
            "run.workspace_preserved",
            json!({
                "workspace":"fixture-worktree","workspace_branch":"crony/fixture",
                "workspace_base_ref":"main","workspace_base_commit":"a".repeat(40),
                "head_commit":"a".repeat(40),"workspace_fingerprint":"b".repeat(64),
                "source_checkpoint":proof,"detail":"Native stopped checkpoint fixture"
            }),
        ))
        .await
        .unwrap();
    store
        .apply_runner_event(event(
            SOURCE,
            TOKEN,
            "run.cancelled",
            json!({"reason":"native budget stop"}),
        ))
        .await
        .unwrap();
    store
}

fn request() -> CreateFactoryVerificationRecoveryInput {
    CreateFactoryVerificationRecoveryInput {
        corp_id: CORP,
        work_item_id: ITEM,
        actor_id: OWNER,
        claim_token: CLAIM,
        expected_factory_version: 1,
        idempotency_key: Uuid::new_v4(),
        source_run_id: SOURCE,
        mode: FactoryVerificationRecoveryMode::CheckpointVerification,
        reason: "Verify the saved source without a model".to_owned(),
        observed_source_revision: "revision-1".to_owned(),
        reviewed_source_snapshot: json!({"source_revision":"revision-1","issue_number":148}),
        contract_revision_id: None,
        expected_workspace_fingerprint: "b".repeat(64),
        expected_head_commit: Some("a".repeat(40)),
    }
}

async fn state(store: &PgStore) -> Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object(
          'source',(SELECT to_jsonb(r) FROM runs r WHERE id=$1),
          'runs',(SELECT jsonb_agg(to_jsonb(r) ORDER BY id) FROM runs r),
          'task',(SELECT to_jsonb(t) FROM tasks t WHERE id=$2),
          'mission',(SELECT to_jsonb(m) FROM missions m WHERE id=$3),
          'item',(SELECT to_jsonb(f) FROM factory_work_items f WHERE id=$4),
          'agents',(SELECT jsonb_agg(to_jsonb(a) ORDER BY id) FROM agents a),
          'recoveries',(SELECT coalesce(jsonb_agg(to_jsonb(v) ORDER BY id),'[]') FROM factory_verification_recoveries v),
          'commands',(SELECT coalesce(jsonb_agg(to_jsonb(c) ORDER BY id),'[]') FROM runner_commands c),
          'events',(SELECT coalesce(jsonb_agg(to_jsonb(e) ORDER BY seq),'[]') FROM events e))",
    ).bind(SOURCE).bind(TASK).bind(MISSION).bind(ITEM)
        .fetch_one(&store.pool).await.unwrap()
}

async fn rejected_without_changes(store: &PgStore, input: CreateFactoryVerificationRecoveryInput) {
    let before = state(store).await;
    assert!(
        store
            .create_factory_verification_recovery(input)
            .await
            .is_err()
    );
    assert_eq!(state(store).await, before);
}

async fn verify(store: &PgStore, run: Uuid, token: Uuid) {
    for (kind, payload) in [
        (
            "run.started",
            json!({"workspace":"fixture-worktree","workspace_branch":"crony/fixture",
            "workspace_base_ref":"main","workspace_base_commit":"a".repeat(40),
            "execution_mode":"verification_only"}),
        ),
        ("run.verification_started", json!({})),
        (
            "run.verification_evidence",
            json!({"evidence_id":Uuid::new_v4(),"check_index":0,
            "kind":"file","status":"passed","summary":"Fixture file verified","payload":{}}),
        ),
        (
            "run.verification_passed",
            json!({"summary":"Fixture checks passed"}),
        ),
        (
            "run.verification_waiting",
            json!({"gate":gate(),"gate_type":"independent_review"}),
        ),
    ] {
        store
            .apply_runner_event(event(run, token, kind, payload))
            .await
            .unwrap();
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_reaches_independent_review_without_more_provider_budget(pool: PgPool) {
    let store = fixture(pool, false).await;
    let before = state(&store).await;
    let context = store
        .factory_verification_recovery_context(CORP, OWNER, ITEM)
        .await
        .unwrap()
        .unwrap();
    assert!(context.checkpoint_verification);
    assert_eq!(context.remaining_attempts, 0);
    assert_eq!(context.remaining_mission_tokens, 0);
    assert_eq!(context.expected_head_commit, Some("a".repeat(40)));
    let input = request();
    let result = store
        .create_factory_verification_recovery(input.clone())
        .await
        .unwrap();
    let FactoryVerificationRecoveryLaunch::VerifierOnly(launch) = result.launch else {
        panic!("checkpoint recovery started a provider");
    };
    let after = state(&store).await;
    assert_eq!(after["source"], before["source"]);
    assert_eq!(after["task"]["attempt_count"], 2);
    assert_eq!(after["task"]["contract"], before["task"]["contract"]);
    assert_eq!(
        after["task"]["verification_policy"],
        before["task"]["verification_policy"]
    );
    for key in [
        "budget_tokens",
        "budget_cost_microusd",
        "original_budget_tokens",
        "original_budget_cost_microusd",
    ] {
        assert_eq!(after["mission"][key], before["mission"][key]);
    }
    let run = after["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == json!(launch.run_id))
        .unwrap();
    for field in [
        "input_tokens",
        "output_tokens",
        "cost_microusd",
        "budget_tokens_limit",
        "budget_cost_microusd_limit",
    ] {
        assert_eq!(run[field], 0, "{field}");
    }
    for field in ["provider_session_id", "model", "reasoning_effort"] {
        assert!(run[field].is_null());
    }
    assert_eq!(run["resumed_from_run_id"], json!(SOURCE));
    assert_eq!(run["workspace_run_id"], json!(SOURCE));
    assert!(
        store
            .create_factory_verification_recovery(input)
            .await
            .unwrap()
            .replayed
    );
    assert_eq!(state(&store).await, after);
    assert!(
        store
            .apply_runner_event(event(
                launch.run_id,
                launch.assignment_token,
                "run.usage",
                json!({"input_tokens":1,"output_tokens":0,"cost_microusd":0})
            ))
            .await
            .is_err()
    );
    assert_eq!(state(&store).await, after);
    verify(&store, launch.run_id, launch.assignment_token).await;
    assert!(
        store
            .decide_verification(
                CORP,
                launch.run_id,
                OWNER,
                true,
                "self",
                Some(Uuid::new_v4())
            )
            .await
            .is_err()
    );
    store
        .decide_verification(
            CORP,
            launch.run_id,
            REVIEWER,
            true,
            "Reviewed exact result",
            Some(Uuid::new_v4()),
        )
        .await
        .unwrap();
    let complete = state(&store).await;
    assert_eq!(complete["source"], before["source"]);
    assert_eq!(complete["task"]["attempt_count"], 2);
    assert_eq!(complete["mission"]["status"], "completed");
    assert_eq!(complete["item"]["state"], "verified");
    assert_eq!(complete["recoveries"][0]["status"], "completed");
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_requires_native_termination_and_exact_policy(pool: PgPool) {
    let store = fixture(pool, false).await;
    let event_id: Uuid =
        sqlx::query_scalar("SELECT id FROM events WHERE type='run.workspace_preserved'")
            .fetch_one(&store.pool)
            .await
            .unwrap();
    let original: Value = sqlx::query_scalar("SELECT payload FROM events WHERE id=$1")
        .bind(event_id)
        .fetch_one(&store.pool)
        .await
        .unwrap();
    for (key, value) in [
        ("schema_version", json!(2)),
        ("corp_id", json!(Uuid::new_v4())),
        ("run_id", json!(Uuid::new_v4())),
        ("workspace_run_id", json!(Uuid::new_v4())),
        ("source_repository", json!("other/repo")),
        ("head_commit", json!("c".repeat(40))),
        ("verification_policy_sha256", json!("c".repeat(64))),
        ("write_scope_sha256", json!("c".repeat(64))),
        ("deliverable_policy_sha256", json!("c".repeat(64))),
    ] {
        let mut altered = original.clone();
        altered["source_checkpoint"][key] = value;
        sqlx::query("UPDATE events SET payload=$1 WHERE id=$2")
            .bind(altered)
            .bind(event_id)
            .execute(&store.pool)
            .await
            .unwrap();
        rejected_without_changes(&store, request()).await;
    }
    sqlx::query("UPDATE events SET payload=$1 WHERE id=$2")
        .bind(original)
        .bind(event_id)
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM events WHERE type='run.session_terminated'")
        .execute(&store.pool)
        .await
        .unwrap();
    rejected_without_changes(&store, request()).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_never_reopens_provider_or_changes_reviewed_source(pool: PgPool) {
    let store = fixture(pool, false).await;
    for mode in [
        FactoryVerificationRecoveryMode::VerifierOnly,
        FactoryVerificationRecoveryMode::SourceCorrection,
    ] {
        let mut input = request();
        input.mode = mode;
        rejected_without_changes(&store, input).await;
    }
    let mut revision = request();
    revision.contract_revision_id = Some(Uuid::new_v4());
    rejected_without_changes(&store, revision).await;
    let mut issue = request();
    issue.observed_source_revision = "revision-2".to_owned();
    issue.reviewed_source_snapshot["source_revision"] = json!("revision-2");
    rejected_without_changes(&store, issue).await;
    let mut head = request();
    head.expected_head_commit = None;
    rejected_without_changes(&store, head).await;
    let mut actor = request();
    actor.actor_id = REVIEWER;
    rejected_without_changes(&store, actor).await;
    sqlx::query("DELETE FROM room_memberships WHERE room_id=$1 AND actor_id=$2")
        .bind(ROOM)
        .bind(OWNER)
        .execute(&store.pool)
        .await
        .unwrap();
    rejected_without_changes(&store, request()).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_preserves_unrelated_blocks_quarantine_and_loop_stops(pool: PgPool) {
    let store = fixture(pool, false).await;
    sqlx::query("UPDATE factory_work_items SET state='blocked',failure_detail='unrelated policy' WHERE id=$1")
        .bind(ITEM).execute(&store.pool).await.unwrap();
    rejected_without_changes(&store, request()).await;
    sqlx::query("UPDATE factory_work_items SET state='running',failure_detail=NULL WHERE id=$1")
        .bind(ITEM)
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE runs SET workspace_disposition='quarantined' WHERE id=$1")
        .bind(SOURCE)
        .execute(&store.pool)
        .await
        .unwrap();
    rejected_without_changes(&store, request()).await;
    sqlx::query(
        "UPDATE runs SET workspace_disposition='preserved',no_progress_events=8 WHERE id=$1",
    )
    .bind(SOURCE)
    .execute(&store.pool)
    .await
    .unwrap();
    rejected_without_changes(&store, request()).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_missing_provider_artifact_cannot_pass_its_required_check(
    pool: PgPool,
) {
    let store = fixture(pool, true).await;
    let result = store
        .create_factory_verification_recovery(request())
        .await
        .unwrap();
    let FactoryVerificationRecoveryLaunch::VerifierOnly(launch) = result.launch else {
        panic!("provider launch")
    };
    for (index, kind) in [(0, "file"), (1, "artifact")] {
        let before = state(&store).await;
        let outcome = store
            .apply_runner_event(event(
                launch.run_id,
                launch.assignment_token,
                "run.verification_evidence",
                json!({"evidence_id":Uuid::new_v4(),"check_index":index,"kind":kind,
                "status":"passed","summary":"Untrusted fixture claim","payload":{}}),
            ))
            .await;
        if kind == "artifact" {
            assert!(outcome.is_err());
            assert_eq!(state(&store).await, before);
        } else {
            outcome.unwrap();
        }
    }
    let before = state(&store).await;
    assert!(
        store
            .apply_runner_event(event(
                launch.run_id,
                launch.assignment_token,
                "run.verification_passed",
                json!({"summary":"Must not accept missing evidence"})
            ))
            .await
            .is_err()
    );
    assert_eq!(state(&store).await, before);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_retry_stays_provider_free_and_old_source_is_fenced(pool: PgPool) {
    let store = fixture(pool, false).await;
    let first = store
        .create_factory_verification_recovery(request())
        .await
        .unwrap();
    let FactoryVerificationRecoveryLaunch::VerifierOnly(launch) = first.launch else {
        panic!("provider launch")
    };
    store
        .apply_runner_event(event(
            launch.run_id,
            launch.assignment_token,
            "run.started",
            json!({"workspace":"fixture-worktree","workspace_branch":"crony/fixture",
          "workspace_base_ref":"main","workspace_base_commit":"a".repeat(40),
          "execution_mode":"verification_only"}),
        ))
        .await
        .unwrap();
    store
        .apply_runner_event(event(
            launch.run_id,
            launch.assignment_token,
            "run.failed",
            json!({"error":"fixture verifier unavailable"}),
        ))
        .await
        .unwrap();
    store
        .apply_runner_event(event(
            launch.run_id,
            launch.assignment_token,
            "run.workspace_preserved",
            json!({"workspace_fingerprint":"b".repeat(64),"head_commit":"a".repeat(40),
          "detail":"Native verifier preserved original source"}),
        ))
        .await
        .unwrap();
    let prior = state(&store).await;
    let version = prior["item"]["version"].as_i64().unwrap();
    let mut stale = request();
    stale.expected_factory_version = version;
    rejected_without_changes(&store, stale).await;
    let mut retry = request();
    retry.expected_factory_version = version;
    retry.source_run_id = launch.run_id;
    let second = store
        .create_factory_verification_recovery(retry)
        .await
        .unwrap();
    assert!(matches!(
        second.launch,
        FactoryVerificationRecoveryLaunch::VerifierOnly(_)
    ));
    let after = state(&store).await;
    assert_eq!(after["source"], prior["source"]);
    assert_eq!(after["task"]["attempt_count"], 2);
    assert_eq!(after["recoveries"].as_array().unwrap().len(), 2);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_command_failure_rolls_back_the_entire_authorization(pool: PgPool) {
    let store = fixture(pool, false).await;
    sqlx::raw_sql(
        r#"
        CREATE FUNCTION reject_checkpoint_command() RETURNS trigger LANGUAGE plpgsql AS $$
        BEGIN
          IF NEW.command_kind='factory_verification_recovery' THEN
            RAISE EXCEPTION 'owned SQLx command-insertion fault';
          END IF;
          RETURN NEW;
        END $$;
        CREATE TRIGGER checkpoint_command_fault BEFORE INSERT ON runner_commands
          FOR EACH ROW EXECUTE FUNCTION reject_checkpoint_command();
        "#,
    )
    .execute(&store.pool)
    .await
    .unwrap();
    rejected_without_changes(&store, request()).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_zero_limits_are_not_a_generic_verifier_budget_bypass(pool: PgPool) {
    let store = fixture(pool, false).await;
    let run = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO runs(id,corp_id,task_id,agent_id,runner_id,assignment_token,status,
           workspace_run_id,execution_mode,budget_tokens_limit,budget_cost_microusd_limit)
         VALUES($1,$2,$3,$4,$5,$6,'verifying',$1,'verification_only',0,0)",
    )
    .bind(run)
    .bind(CORP)
    .bind(TASK)
    .bind(AGENT)
    .bind(RUNNER)
    .bind(Uuid::new_v4())
    .execute(&store.pool)
    .await
    .unwrap();
    let mut tx = store.pool.begin().await.unwrap();
    assert!(
        !budget_checkpoint::zero_provider_allocation_tx(&mut tx, CORP, run)
            .await
            .unwrap()
    );
    assert!(hard_breaker_reached_tx(&mut tx, CORP, run).await.unwrap());
    assert!(
        ensure_run_not_hard_blocked_tx(&mut tx, CORP, run, "healthy", "unbound verifier")
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_preserves_another_active_agent_assignment(pool: PgPool) {
    let store = fixture(pool, false).await;
    let other = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO runs(id,corp_id,task_id,agent_id,runner_id,assignment_token,status,
           workspace_run_id) VALUES($1,$2,$3,$4,$5,$6,'verifying',$1)",
    )
    .bind(other)
    .bind(CORP)
    .bind(TASK)
    .bind(AGENT)
    .bind(RUNNER)
    .bind(Uuid::new_v4())
    .execute(&store.pool)
    .await
    .unwrap();
    sqlx::query("UPDATE agents SET current_run_id=$1,status='reviewing' WHERE id=$2")
        .bind(other)
        .bind(AGENT)
        .execute(&store.pool)
        .await
        .unwrap();
    rejected_without_changes(&store, request()).await;
}
