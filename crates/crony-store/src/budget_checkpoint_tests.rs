//! Actual-store metadata tests in SQLx-owned disposable databases, not provider
//! execution, physical source verification, or signed-object acceptance.
use super::*;
use crony_domain::{DeliverableForm, DeliverableSpec, StoppedSourceCheckpoint};

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
    fixture_with_stop_request(pool, needs_artifact, false).await
}

async fn fixture_with_stop_request(
    pool: PgPool,
    needs_artifact: bool,
    explicit_stop: bool,
) -> PgStore {
    fixture_with_deliverable(pool, needs_artifact, explicit_stop, None).await
}

async fn fixture_with_deliverable(
    pool: PgPool,
    needs_artifact: bool,
    explicit_stop: bool,
    deliverable: Option<DeliverableSpec>,
) -> PgStore {
    fixture_with_publication_policy(pool, needs_artifact, explicit_stop, deliverable, false).await
}

async fn fixture_with_publication_policy(
    pool: PgPool,
    needs_artifact: bool,
    explicit_stop: bool,
    deliverable: Option<DeliverableSpec>,
    publication_authorized: bool,
) -> PgStore {
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
    let mut contract: TaskContract = serde_json::from_value(json!({
        "objective":"verify result.md", "expected_output":"result.md",
        "source_repository":"fixture/source", "source_base_ref":"main",
        "source_base_commit":"a".repeat(40), "acceptance_tests":["result.md is present"],
        "allowed_tools":["filesystem"], "prohibited_actions":["no external effects"],
        "references":[], "write_scope":["result.md"], "budget_tokens":5000,
        "budget_cost_microusd":1000000, "deadline_at":null, "escalation":"ask owner",
        "model":"fixture-model", "reasoning_effort":"medium"
    }))
    .unwrap();
    contract.deliverable = deliverable;
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
    let mut factory_policy = json!({
        "source_base_ref":"main", "source_base_commit":"a".repeat(40),
        "repository_allowlist":["fixture/source"], "write_scope":["result.md"],
        "allowed_tools":["filesystem"], "prohibited_actions":["no external effects"]
    });
    if let Some(deliverable) = &contract.deliverable {
        factory_policy["deliverable_form"] = json!(deliverable.form.as_str());
    }
    if publication_authorized {
        // Present before the original run/checkpoint; never widen a stopped case.
        factory_policy["auto_merge"] = json!(false);
        factory_policy["publication"] = json!({
            "allowed":true,"repository_allowlist":["fixture/source"],
            "base_ref":"main","branch_prefix":"ecorp/","status_before":"In Progress",
            "review_status":"In Review","auto_merge":false,"merge":false,"deploy":false
        });
    }
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
    if explicit_stop {
        let stopped = store
            .request_emergency_stop(CORP, AGENT, OWNER, "Stop this assignment explicitly")
            .await
            .unwrap();
        assert_eq!(stopped.run_id, SOURCE);
        assert_eq!(stopped.event.event_type, "run.stop_requested");
    }
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
          'tasks',(SELECT jsonb_agg(to_jsonb(t) ORDER BY id) FROM tasks t),
          'missions',(SELECT jsonb_agg(to_jsonb(m) ORDER BY id) FROM missions m),
          'item',(SELECT to_jsonb(f) FROM factory_work_items f WHERE id=$4),
          'agents',(SELECT jsonb_agg(to_jsonb(a) ORDER BY id) FROM agents a),
          'actors',(SELECT jsonb_agg(to_jsonb(a) ORDER BY id) FROM actors a),
          'memberships',(SELECT coalesce(jsonb_agg(to_jsonb(m) ORDER BY room_id,actor_id),'[]') FROM room_memberships m),
          'recoveries',(SELECT coalesce(jsonb_agg(to_jsonb(v) ORDER BY id),'[]') FROM factory_verification_recoveries v),
          'commands',(SELECT coalesce(jsonb_agg(to_jsonb(c) ORDER BY id),'[]') FROM runner_commands c),
          'artifacts',(SELECT coalesce(jsonb_agg(to_jsonb(a) ORDER BY id),'[]') FROM artifacts a),
          'deliverables',(SELECT coalesce(jsonb_agg(to_jsonb(d) ORDER BY id),'[]') FROM source_deliverables d),
          'verification_evidence',(SELECT coalesce(jsonb_agg(to_jsonb(v) ORDER BY id),'[]') FROM verification_evidence v),
          'verification_requests',(SELECT coalesce(jsonb_agg(to_jsonb(v) ORDER BY run_id),'[]') FROM verification_requests v),
          'operations',(SELECT coalesce(jsonb_agg(to_jsonb(o) ORDER BY corp_id,idempotency_key),'[]') FROM factory_operations o),
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

async fn checkpoint_command(store: &PgStore) -> PendingRunnerCommand {
    let recovery = store
        .create_factory_verification_recovery(request())
        .await
        .unwrap();
    let replacement_run_id = recovery
        .recovery
        .replacement_run_id
        .expect("native checkpoint replacement run");
    store
        .pending_runner_commands(RUNNER)
        .await
        .unwrap()
        .into_iter()
        .find(|command| {
            command.command_kind == "factory_verification_recovery"
                && command.run_id == replacement_run_id
        })
        .expect("native checkpoint recovery command")
}

async fn assert_dispatch_read(store: &PgStore, command: &PendingRunnerCommand, expected: bool) {
    let before = state(store).await;
    let payload = command.payload.clone();
    assert_eq!(
        store
            .verification_recovery_dispatch_authorized(command)
            .await
            .unwrap(),
        expected
    );
    assert_eq!(command.payload, payload);
    assert_eq!(state(store).await, before);
}

async fn assert_explicit_stop_rejected(
    store: &PgStore,
    input: CreateFactoryVerificationRecoveryInput,
) {
    let before = state(store).await;
    let error = store
        .create_factory_verification_recovery(input)
        .await
        .expect_err("explicit stop must reject checkpoint authorization");
    assert!(
        error.to_string().contains("explicit stop request"),
        "wrong rejection: {error:#}"
    );
    assert_eq!(state(store).await, before);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_dispatch_rechecks_artifact_free_author_role(pool: PgPool) {
    let store = fixture(pool, false).await;
    let command = checkpoint_command(&store).await;
    assert!(command.payload["provider_artifact"].is_null());
    for role in ["owner", "admin", "manager", "member", "guest"] {
        sqlx::query("UPDATE actors SET role=$1 WHERE id=$2 AND corp_id=$3")
            .bind(role)
            .bind(OWNER)
            .bind(CORP)
            .execute(&store.pool)
            .await
            .unwrap();
        assert_dispatch_read(
            &store,
            &command,
            matches!(role, "owner" | "admin" | "manager"),
        )
        .await;
    }
    sqlx::query("UPDATE actors SET role='owner',kind='service' WHERE id=$1 AND corp_id=$2")
        .bind(OWNER)
        .bind(CORP)
        .execute(&store.pool)
        .await
        .unwrap();
    assert_dispatch_read(&store, &command, false).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_dispatch_rechecks_artifact_free_room_membership(pool: PgPool) {
    let store = fixture(pool, false).await;
    let command = checkpoint_command(&store).await;
    assert_dispatch_read(&store, &command, true).await;
    sqlx::query("DELETE FROM room_memberships WHERE room_id=$1 AND actor_id=$2")
        .bind(ROOM)
        .bind(OWNER)
        .execute(&store.pool)
        .await
        .unwrap();
    assert_dispatch_read(&store, &command, false).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_dispatch_rejects_foreign_and_stale_commands(pool: PgPool) {
    let store = fixture(pool, false).await;
    let command = checkpoint_command(&store).await;
    let mut foreign = command.clone();
    foreign.id = Uuid::new_v4();
    assert_dispatch_read(&store, &foreign, false).await;
    let mut foreign = command.clone();
    foreign.corp_id = Uuid::new_v4();
    foreign.payload["corp_id"] = json!(foreign.corp_id);
    assert_dispatch_read(&store, &foreign, false).await;
    let mut foreign = command.clone();
    foreign.run_id = Uuid::new_v4();
    foreign.payload["run_id"] = json!(foreign.run_id);
    assert_dispatch_read(&store, &foreign, false).await;
    let mut foreign = command.clone();
    foreign.runner_id = "foreign-runner".to_owned();
    assert_dispatch_read(&store, &foreign, false).await;
    let mut foreign = command.clone();
    foreign.command_kind = "circuit_breaker".to_owned();
    assert_dispatch_read(&store, &foreign, false).await;

    for field in [
        "source_run_id",
        "workspace_run_id",
        "agent_id",
        "assignment_token",
        "task_id",
        "mission_id",
        "room_id",
    ] {
        let mut altered = command.clone();
        altered.payload[field] = json!(Uuid::new_v4());
        assert_dispatch_read(&store, &altered, false).await;
    }
    let mut altered = command.clone();
    altered.payload["mode"] = json!("source_correction");
    assert_dispatch_read(&store, &altered, false).await;

    let mut replacement = command.payload.clone();
    replacement["prompt"] = json!("Changed durable metadata after the pending read");
    sqlx::query("UPDATE runner_commands SET payload=$1 WHERE id=$2")
        .bind(replacement)
        .bind(command.id)
        .execute(&store.pool)
        .await
        .unwrap();
    assert_dispatch_read(&store, &command, false).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_dispatch_rejects_settled_and_inactive_recoveries(pool: PgPool) {
    let store = fixture(pool, false).await;
    let command = checkpoint_command(&store).await;
    for status in ["completed", "failed", "cancelled", "lost"] {
        sqlx::query("UPDATE runs SET status=$1 WHERE id=$2")
            .bind(status)
            .bind(command.run_id)
            .execute(&store.pool)
            .await
            .unwrap();
        assert_dispatch_read(&store, &command, false).await;
    }
    sqlx::query("UPDATE runs SET status='starting' WHERE id=$1")
        .bind(command.run_id)
        .execute(&store.pool)
        .await
        .unwrap();
    for status in ["completed", "failed"] {
        sqlx::query(
            "UPDATE factory_verification_recoveries SET status=$1 WHERE replacement_run_id=$2",
        )
        .bind(status)
        .bind(command.run_id)
        .execute(&store.pool)
        .await
        .unwrap();
        assert_dispatch_read(&store, &command, false).await;
    }
    sqlx::query(
        "UPDATE factory_verification_recoveries SET status='running' WHERE replacement_run_id=$1",
    )
    .bind(command.run_id)
    .execute(&store.pool)
    .await
    .unwrap();
    assert_dispatch_read(&store, &command, true).await;
    store
        .acknowledge_runner_command(command.id, RUNNER)
        .await
        .unwrap()
        .expect("native command acknowledgment");
    assert_dispatch_read(&store, &command, false).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_dispatch_binds_current_command_source_and_policy(pool: PgPool) {
    let store = fixture(pool, false).await;
    let command = checkpoint_command(&store).await;
    for (field, value) in [
        ("assignment_token", json!(Uuid::new_v4())),
        ("source_run_id", json!(Uuid::new_v4())),
        ("workspace_run_id", json!(Uuid::new_v4())),
        ("room_id", json!(Uuid::new_v4())),
        ("source_repository", json!("foreign/source")),
        ("source_base_ref", json!("foreign-ref")),
        ("source_base_commit", json!("c".repeat(40))),
        ("expected_workspace_fingerprint", json!("c".repeat(64))),
        ("expected_head_commit", json!("c".repeat(40))),
        ("verification_policy", json!({"checks":[]})),
        ("write_scope", json!(["foreign/**"])),
        ("mode", json!("verifier_only")),
        ("secret_refs", json!([{"fixture":"not a provider grant"}])),
    ] {
        // Supply the changed stored payload too: denial must come from native
        // assignment/recovery/source binding, not just stale-input comparison.
        let mut altered = command.clone();
        altered.payload[field] = value;
        sqlx::query("UPDATE runner_commands SET payload=$1 WHERE id=$2")
            .bind(&altered.payload)
            .bind(command.id)
            .execute(&store.pool)
            .await
            .unwrap();
        assert_dispatch_read(&store, &altered, false).await;
    }
    sqlx::query("UPDATE runner_commands SET payload=$1 WHERE id=$2")
        .bind(&command.payload)
        .bind(command.id)
        .execute(&store.pool)
        .await
        .unwrap();
    assert_dispatch_read(&store, &command, true).await;
    sqlx::query("UPDATE runs SET source_base_commit=$1 WHERE id=$2")
        .bind("c".repeat(40))
        .bind(SOURCE)
        .execute(&store.pool)
        .await
        .unwrap();
    assert_dispatch_read(&store, &command, false).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_budget_incident_cannot_override_native_explicit_stop(pool: PgPool) {
    let store = fixture_with_stop_request(pool, false, true).await;
    assert_explicit_stop_rejected(&store, request()).await;
    let before = state(&store).await;
    let error = store
        .factory_verification_recovery_context(CORP, OWNER, ITEM)
        .await
        .expect_err("explicitly stopped checkpoint cannot be offered for recovery");
    assert!(error.to_string().contains("explicit stop request"));
    assert_eq!(state(&store).await, before);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_explicit_stop_fences_dispatch_budget_exemption_and_retry(
    pool: PgPool,
) {
    let store = fixture(pool, false).await;
    let command = checkpoint_command(&store).await;
    let token = Uuid::parse_str(command.payload["assignment_token"].as_str().unwrap()).unwrap();
    assert_dispatch_read(&store, &command, true).await;
    store
        .apply_runner_event(event(
            command.run_id,
            token,
            "run.started",
            json!({"workspace":"fixture-worktree","workspace_branch":"crony/fixture",
                "workspace_base_ref":"main","workspace_base_commit":"a".repeat(40),
                "execution_mode":"verification_only"}),
        ))
        .await
        .unwrap();
    let stopped = store
        .request_emergency_stop(CORP, AGENT, OWNER, "Stop this verifier explicitly")
        .await
        .unwrap();
    assert_eq!(stopped.run_id, command.run_id);
    assert_dispatch_read(&store, &command, false).await;
    let before = state(&store).await;
    let mut tx = store.pool.begin().await.unwrap();
    assert!(
        !budget_checkpoint::zero_provider_allocation_tx(&mut tx, CORP, command.run_id)
            .await
            .unwrap()
    );
    assert!(
        hard_breaker_reached_tx(&mut tx, CORP, command.run_id)
            .await
            .unwrap()
    );
    tx.rollback().await.unwrap();
    assert_eq!(state(&store).await, before);
    store
        .apply_runner_event(event(
            command.run_id,
            token,
            "run.cancelled",
            json!({"reason":"Explicit operator stop"}),
        ))
        .await
        .unwrap();
    store
        .apply_runner_event(event(
            command.run_id,
            token,
            "run.workspace_preserved",
            json!({"workspace_fingerprint":"b".repeat(64),"head_commit":"a".repeat(40),
                "detail":"Native verifier preserved original source"}),
        ))
        .await
        .unwrap();
    let mut retry = request();
    retry.source_run_id = command.run_id;
    retry.expected_factory_version = state(&store).await["item"]["version"].as_i64().unwrap();
    assert_explicit_stop_rejected(&store, retry).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_unrelated_stop_events_do_not_revoke_budget_recovery(pool: PgPool) {
    let store = fixture(pool, false).await;
    let other_corp = Uuid::new_v4();
    sqlx::query("INSERT INTO corps(id,slug,name) VALUES($1,'foreign-stop','Foreign stop fixture')")
        .bind(other_corp)
        .execute(&store.pool)
        .await
        .unwrap();
    let mut tx = store.pool.begin().await.unwrap();
    for (corp_id, aggregate_type, run_id) in [
        (CORP, "run", Uuid::new_v4()),
        (other_corp, "run", SOURCE),
        (CORP, "task", SOURCE),
    ] {
        append_event_tx(
            &mut tx,
            NewEvent::new(
                corp_id,
                None,
                "run.stop_requested",
                aggregate_type,
                run_id,
                Uuid::new_v4().to_string(),
                json!({"reason":"Unrelated stop metadata fixture"}),
            ),
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    let command = checkpoint_command(&store).await;
    assert_dispatch_read(&store, &command, true).await;
    let mut tx = store.pool.begin().await.unwrap();
    assert!(
        budget_checkpoint::zero_provider_allocation_tx(&mut tx, CORP, command.run_id)
            .await
            .unwrap()
    );
    tx.rollback().await.unwrap();
}

fn retained_run(snapshot: &Value, run_id: Uuid) -> &Value {
    snapshot["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|run| run["id"] == json!(run_id))
        .expect("retained verifier run")
}

fn retention_event(command: &PendingRunnerCommand) -> RunnerEventInput {
    event(
        command.run_id,
        Uuid::parse_str(command.payload["assignment_token"].as_str().unwrap()).unwrap(),
        "run.workspace_preserved",
        json!({
            "workspace":"fixture-worktree","workspace_branch":"crony/fixture",
            "workspace_base_ref":"main","workspace_base_commit":"a".repeat(40),
            "workspace_fingerprint":"b".repeat(64),"workspace_quarantined":false,
            "detail":"Native verifier retained its checked source",
        }),
    )
}

async fn waiting_export_fixture(pool: PgPool) -> (PgStore, PendingRunnerCommand, Uuid) {
    waiting_export_fixture_with_publication(pool, false).await
}

async fn waiting_export_fixture_with_publication(
    pool: PgPool,
    publication_authorized: bool,
) -> (PgStore, PendingRunnerCommand, Uuid) {
    let store = fixture_with_publication_policy(
        pool,
        false,
        false,
        Some(DeliverableSpec {
            form: DeliverableForm::CommitBranch,
            commit_after_verification: true,
            paths: vec!["result.md".to_owned()],
        }),
        publication_authorized,
    )
    .await;
    sqlx::query(
        "INSERT INTO runner_nodes(id,corp_id,hostname,os,connection_epoch,status)
         VALUES($1,$2,'sqlx-retention-fixture','fixture',$3,'connected')",
    )
    .bind(RUNNER)
    .bind(CORP)
    .bind(Uuid::from_u128(20))
    .execute(&store.pool)
    .await
    .unwrap();
    let command = checkpoint_command(&store).await;
    let token = retention_event(&command).assignment_token;
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
    ] {
        store
            .apply_runner_event(event(command.run_id, token, kind, payload))
            .await
            .unwrap();
    }
    // Exercise native store reservation/finalization and verification linkage.
    // These are synthetic metadata, not real Git bundle, signature, or object-store acceptance.
    let content = br#"{"fixture":"SQLx-only source bundle metadata"}"#;
    let sha256 = hex::encode(Sha256::digest(content));
    let upload = event(command.run_id, token, "run.deliverable_upload", json!({}));
    let artifact_id = upload.event_id;
    let artifact = StoredArtifact {
        id: artifact_id,
        corp_id: CORP,
        task_id: TASK,
        run_id: command.run_id,
        producer_agent_id: AGENT,
        producer_runner_id: RUNNER.to_owned(),
        verifier: "crony-server:artifact-ingest-v1".to_owned(),
        object_key: format!("corps/{CORP}/sha256/{}/{sha256}", &sha256[..2]),
        uri: format!("/api/corps/{CORP}/artifacts/{artifact_id}"),
        sha256: sha256.clone(),
        media_type: "application/vnd.ecorp.deliverable+json".to_owned(),
        bytes: i64::try_from(content.len()).unwrap(),
        artifact_role: "source_deliverable".to_owned(),
        file_name: "ecorp-commit-branch.json".to_owned(),
        metadata: json!({
            "form":"commit_branch","verification_sha256":"c".repeat(64),
            "base_commit":"a".repeat(40),"head_commit":"d".repeat(40),
            "branch":"crony/fixture","integration_state":"ready_for_review",
            "git_bundle_sha256":hex::encode(Sha256::digest(b"SQLx bundle metadata")),
            "publication_ready":true,
        }),
        provenance_signature: "0".repeat(64),
        retention_until: Utc::now() + chrono::Duration::hours(1),
    };
    store
        .prepare_artifact_upload(
            upload,
            artifact,
            &format!("staging/corps/{CORP}/{artifact_id}"),
        )
        .await
        .unwrap();
    store
        .finalize_artifact_upload(CORP, artifact_id)
        .await
        .unwrap();
    for (kind, payload) in [
        (
            "run.verification_passed",
            json!({"summary":"Fixture checks passed",
            "verification_sha256":"c".repeat(64),"deliverable_sha256":sha256}),
        ),
        (
            "run.verification_waiting",
            json!({"gate":gate(),"gate_type":"independent_review"}),
        ),
    ] {
        store
            .apply_runner_event(event(command.run_id, token, kind, payload))
            .await
            .unwrap();
    }
    store
        .acknowledge_runner_command(command.id, RUNNER)
        .await
        .unwrap();
    let snapshot = state(&store).await;
    let run = retained_run(&snapshot, command.run_id);
    assert_eq!(run["status"], "waiting_for_approval");
    assert_eq!(run["verification_status"], "waiting_for_approval");
    assert_eq!(run["workspace_disposition"], "active");
    assert!(run["workspace_fingerprint"].is_null());
    (store, command, artifact_id)
}

async fn retention_request(
    store: &PgStore,
    command: &PendingRunnerCommand,
) -> CheckpointFactoryWorkspaceInput {
    CheckpointFactoryWorkspaceInput {
        corp_id: CORP,
        work_item_id: ITEM,
        actor_id: OWNER,
        claim_token: CLAIM,
        expected_version: state(store).await["item"]["version"].as_i64().unwrap(),
        idempotency_key: Uuid::new_v4().to_string(),
        source_run_id: command.run_id,
        expected_head_commit: "d".repeat(40),
    }
}

async fn reject_retention_event(store: &PgStore, input: RunnerEventInput) {
    let before = state(store).await;
    assert!(store.apply_runner_event(input).await.is_err());
    assert_eq!(state(store).await, before);
}

async fn reject_retention_request(store: &PgStore, input: CheckpointFactoryWorkspaceInput) {
    let before = state(store).await;
    assert!(store.checkpoint_factory_workspace(input).await.is_err());
    assert_eq!(state(store).await, before);
}

async fn assert_exported_checkpoint_retry(pool: PgPool, terminal_event: &str) {
    let (store, command, _) = waiting_export_fixture(pool).await;
    let mut preserved = retention_event(&command);
    preserved.payload["head_commit"] = json!("d".repeat(40));
    store
        .apply_runner_event(event(
            command.run_id,
            preserved.assignment_token,
            terminal_event,
            json!({"error":"Native verifier interrupted after source export"}),
        ))
        .await
        .unwrap();
    store.apply_runner_event(preserved).await.unwrap();
    let before = state(&store).await;
    let context = store
        .factory_verification_recovery_context(CORP, OWNER, ITEM)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(context.source_run_id, command.run_id);
    assert!(context.checkpoint_verification);
    assert_eq!(context.expected_head_commit, Some("d".repeat(40)));
    assert_eq!(context.workspace_fingerprint, Some("b".repeat(64)));
    assert_eq!(
        state(&store).await,
        before,
        "preview must not mutate history"
    );

    let mut input = request();
    input.source_run_id = context.source_run_id;
    input.expected_factory_version = context.work_item.version;
    input.expected_head_commit = context.expected_head_commit;
    input.expected_workspace_fingerprint = context.workspace_fingerprint.unwrap();
    let mut stale = input.clone();
    stale.expected_head_commit = Some("a".repeat(40));
    rejected_without_changes(&store, stale).await;
    let recovery = store
        .create_factory_verification_recovery(input.clone())
        .await
        .unwrap();
    let FactoryVerificationRecoveryLaunch::VerifierOnly(launch) = recovery.launch else {
        panic!("checkpoint retry must not launch a provider")
    };
    assert_eq!(launch.expected_head_commit, Some("d".repeat(40)));
    assert_eq!(launch.source_run_id, command.run_id);
    let after = state(&store).await;
    for key in [
        "source",
        "artifacts",
        "deliverables",
        "verification_evidence",
    ] {
        assert_eq!(after[key], before[key], "{key}");
    }
    assert_eq!(
        retained_run(&after, command.run_id),
        retained_run(&before, command.run_id),
        "failed verifier history must remain immutable"
    );
    assert_eq!(
        after["task"]["attempt_count"],
        before["task"]["attempt_count"]
    );
    let next = retained_run(&after, launch.run_id);
    assert_eq!(next["workspace_run_id"], json!(SOURCE));
    assert_eq!(next["resumed_from_run_id"], json!(command.run_id));
    assert_eq!(next["input_tokens"], 0);
    assert_eq!(next["output_tokens"], 0);
    assert_eq!(next["cost_microusd"], 0);
    assert!(next["provider_session_id"].is_null());
    let next_recovery = after["recoveries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["replacement_run_id"] == json!(launch.run_id))
        .unwrap();
    assert_eq!(
        next_recovery["request"]["expected_head_commit"],
        "d".repeat(40)
    );
    assert_eq!(
        next_recovery["checkpoint_authority"]["checkpoint"]["head_commit"],
        "a".repeat(40),
        "original provider head remains separate admission authority"
    );
    let mut tx = store.pool.begin().await.unwrap();
    assert!(
        budget_checkpoint::zero_provider_allocation_tx(&mut tx, CORP, launch.run_id)
            .await
            .unwrap(),
        "the exported-head retry must remain a valid provider-free allocation"
    );
    tx.rollback().await.unwrap();
    let next_command = store
        .pending_runner_commands(RUNNER)
        .await
        .unwrap()
        .into_iter()
        .find(|pending| pending.run_id == launch.run_id)
        .unwrap();
    assert_eq!(next_command.payload["expected_head_commit"], "d".repeat(40));
    assert_dispatch_read(&store, &next_command, true).await;
    let replay = store
        .create_factory_verification_recovery(input)
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(state(&store).await, after);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_exported_retry_after_failure_preserves_head(pool: PgPool) {
    assert_exported_checkpoint_retry(pool, "run.failed").await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_exported_retry_after_cancellation_preserves_head(pool: PgPool) {
    assert_exported_checkpoint_retry(pool, "run.cancelled").await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_retention_binds_export_head_by_mode_without_rewriting_origin(
    pool: PgPool,
) {
    let (store, command, _) = waiting_export_fixture(pool).await;
    let before = state(&store).await;
    store
        .apply_runner_event(retention_event(&command))
        .await
        .unwrap();
    let after = state(&store).await;
    for key in [
        "source",
        "task",
        "mission",
        "recoveries",
        "artifacts",
        "deliverables",
    ] {
        assert_eq!(after[key], before[key], "{key}");
    }
    let run = retained_run(&after, command.run_id);
    assert_eq!(run["status"], "waiting_for_approval");
    assert_eq!(run["workspace_disposition"], "preserved");
    assert_eq!(run["workspace_fingerprint"], "b".repeat(64));
    assert_eq!(
        after["recoveries"][0]["request"]["expected_head_commit"],
        "a".repeat(40)
    );
    assert_eq!(
        after["recoveries"][0]["checkpoint_authority"]["checkpoint"]["head_commit"],
        "a".repeat(40)
    );
    assert_eq!(after["deliverables"][0]["head_commit"], "d".repeat(40));
    // Ordinary-mode metadata variant, not a claim that ordinary admission can waive the budget stop.
    sqlx::query(
        "UPDATE factory_verification_recoveries SET mode='verifier_only',checkpoint_authority=NULL,
         request=jsonb_set(request,'{mode}','\"verifier_only\"') WHERE replacement_run_id=$1",
    )
    .bind(command.run_id)
    .execute(&store.pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE runner_commands SET payload=jsonb_set(payload,'{mode}','\"verifier_only\"')
         WHERE id=$1",
    )
    .bind(command.id)
    .execute(&store.pool)
    .await
    .unwrap();
    reject_retention_event(&store, retention_event(&command)).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_retention_rejects_unbound_export(pool: PgPool) {
    let (store, command, artifact_id) = waiting_export_fixture(pool).await;
    let mut wrong_report = retention_event(&command);
    wrong_report.payload["head_commit"] = json!("e".repeat(40));
    reject_retention_event(&store, wrong_report).await;
    let finalized_at: Option<chrono::DateTime<Utc>> =
        sqlx::query_scalar("SELECT finalized_at FROM artifacts WHERE id=$1")
            .bind(artifact_id)
            .fetch_one(&store.pool)
            .await
            .unwrap();
    sqlx::query("UPDATE artifacts SET status='staged',finalized_at=NULL WHERE id=$1")
        .bind(artifact_id)
        .execute(&store.pool)
        .await
        .unwrap();
    reject_retention_event(&store, retention_event(&command)).await;
    sqlx::query("UPDATE artifacts SET status='ready',finalized_at=$2 WHERE id=$1")
        .bind(artifact_id)
        .bind(finalized_at)
        .execute(&store.pool)
        .await
        .unwrap();
    // Corrupt one binding at a time only inside this disposable metadata fixture.
    for (corrupt, restore) in [
        (
            "UPDATE artifacts SET artifact_role='provider_evidence' WHERE id=$1",
            "UPDATE artifacts SET artifact_role='source_deliverable' WHERE id=$1",
        ),
        (
            "UPDATE artifacts SET run_id='00000000-0000-0000-0000-000000000004' WHERE id=$1",
            "UPDATE artifacts a SET run_id=d.run_id FROM source_deliverables d WHERE d.artifact_id=a.id AND a.id=$1",
        ),
        (
            "UPDATE artifacts SET sha256=repeat('e',64) WHERE id=$1",
            "UPDATE artifacts a SET sha256=r.deliverable_sha256 FROM runs r WHERE a.run_id=r.id AND a.id=$1",
        ),
        (
            "UPDATE source_deliverables SET base_commit=repeat('e',40) WHERE artifact_id=$1",
            "UPDATE source_deliverables d SET base_commit=r.workspace_base_commit FROM runs r WHERE d.run_id=r.id AND d.artifact_id=$1",
        ),
        (
            "UPDATE source_deliverables SET verification_sha256=repeat('e',64) WHERE artifact_id=$1",
            "UPDATE source_deliverables d SET verification_sha256=r.verification_sha256 FROM runs r WHERE d.run_id=r.id AND d.artifact_id=$1",
        ),
        (
            "UPDATE source_deliverables SET head_commit=repeat('e',40) WHERE artifact_id=$1",
            "UPDATE source_deliverables d SET head_commit=a.metadata->>'head_commit' FROM artifacts a WHERE d.artifact_id=a.id AND a.id=$1",
        ),
    ] {
        sqlx::query(corrupt)
            .bind(artifact_id)
            .execute(&store.pool)
            .await
            .unwrap();
        reject_retention_event(&store, retention_event(&command)).await;
        sqlx::query(restore)
            .bind(artifact_id)
            .execute(&store.pool)
            .await
            .unwrap();
    }
    // Removing the ready artifact also cascades its deliverable row. Already
    // verified export digests must not fall back to A for a fingerprint-only report.
    sqlx::query("DELETE FROM artifacts WHERE id=$1")
        .bind(artifact_id)
        .execute(&store.pool)
        .await
        .unwrap();
    reject_retention_event(&store, retention_event(&command)).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_retention_reattests_waiting_run_with_one_replayable_command(
    pool: PgPool,
) {
    let (store, command, _) = waiting_export_fixture(pool).await;
    let input = retention_request(&store, &command).await;
    let before = state(&store).await;
    let outcome = store
        .checkpoint_factory_workspace(input.clone())
        .await
        .unwrap();
    assert!(!outcome.replayed);
    assert_eq!(outcome.source_run_id, command.run_id);
    assert!(
        outcome.workspace_fingerprint.is_none(),
        "request is not a runner attestation"
    );
    let command_id = outcome.command_id.expect("native checkpoint command");
    let after = state(&store).await;
    for key in [
        "source",
        "runs",
        "tasks",
        "missions",
        "task",
        "mission",
        "agents",
        "recoveries",
        "artifacts",
        "deliverables",
        "verification_evidence",
        "verification_requests",
    ] {
        assert_eq!(after[key], before[key], "{key}");
    }
    assert_eq!(
        after["commands"].as_array().unwrap().len(),
        before["commands"].as_array().unwrap().len() + 1
    );
    let pending = store.pending_runner_commands(RUNNER).await.unwrap();
    let checkpoint = pending.iter().find(|entry| entry.id == command_id).unwrap();
    assert_eq!(checkpoint.command_kind, "factory_workspace_checkpoint");
    assert_eq!(checkpoint.run_id, command.run_id);
    assert_eq!(checkpoint.payload["workspace_run_id"], json!(SOURCE));
    assert_eq!(checkpoint.payload["expected_head_commit"], "d".repeat(40));
    assert_eq!(checkpoint.payload["source_base_commit"], "a".repeat(40));
    assert_eq!(
        checkpoint.payload["assignment_token"],
        command.payload["assignment_token"]
    );
    let replay = store.checkpoint_factory_workspace(input).await.unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.command_id, Some(command_id));
    assert_eq!(state(&store).await, after);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_retention_reattest_rechecks_role_room_and_claim(pool: PgPool) {
    let (store, command, _) = waiting_export_fixture(pool).await;
    let input = retention_request(&store, &command).await;
    store
        .checkpoint_factory_workspace(input.clone())
        .await
        .unwrap();
    sqlx::query("UPDATE actors SET role='member' WHERE id=$1")
        .bind(OWNER)
        .execute(&store.pool)
        .await
        .unwrap();
    reject_retention_request(&store, input.clone()).await;
    sqlx::query("UPDATE actors SET role='owner' WHERE id=$1")
        .bind(OWNER)
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM room_memberships WHERE room_id=$1 AND actor_id=$2")
        .bind(ROOM)
        .bind(OWNER)
        .execute(&store.pool)
        .await
        .unwrap();
    reject_retention_request(&store, input.clone()).await;
    sqlx::query("INSERT INTO room_memberships(room_id,actor_id) VALUES($1,$2)")
        .bind(ROOM)
        .bind(OWNER)
        .execute(&store.pool)
        .await
        .unwrap();
    let mut stale = input.clone();
    stale.claim_token = Uuid::new_v4();
    reject_retention_request(&store, stale).await;
    let mut stale = input.clone();
    stale.expected_version += 1;
    reject_retention_request(&store, stale).await;
    assert!(
        store
            .checkpoint_factory_workspace(input)
            .await
            .unwrap()
            .replayed
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_retention_reattest_rejects_stop_quarantine_and_active_lineage(
    pool: PgPool,
) {
    let (store, command, _) = waiting_export_fixture(pool).await;
    let input = retention_request(&store, &command).await;
    let other = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO runs(id,corp_id,task_id,agent_id,runner_id,assignment_token,status,workspace_run_id)
         VALUES($1,$2,$3,$4,$5,$6,'running',$7)",
    ).bind(other).bind(CORP).bind(TASK).bind(AGENT).bind(RUNNER).bind(Uuid::new_v4())
        .bind(SOURCE).execute(&store.pool).await.unwrap();
    reject_retention_request(&store, input.clone()).await;
    // Restore only injected fixture state between independent negative cases.
    sqlx::query("DELETE FROM runs WHERE id=$1")
        .bind(other)
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE runs SET workspace_disposition='quarantined' WHERE id=$1")
        .bind(command.run_id)
        .execute(&store.pool)
        .await
        .unwrap();
    reject_retention_request(&store, input.clone()).await;
    sqlx::query("UPDATE runs SET workspace_disposition='active' WHERE id=$1")
        .bind(command.run_id)
        .execute(&store.pool)
        .await
        .unwrap();
    let stopped = store
        .request_emergency_stop(CORP, AGENT, OWNER, "Stop retained verification")
        .await
        .unwrap();
    assert_eq!(stopped.run_id, command.run_id);
    reject_retention_request(&store, input).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_retention_unclaimed_reconnect_preserves_only_bound_review(
    pool: PgPool,
) {
    let (store, command, artifact_id) = waiting_export_fixture(pool).await;
    let before = state(&store).await;
    let mut tx = store.pool.begin().await.unwrap();
    let events = mark_runner_runs_lost_tx(
        &mut tx,
        RUNNER,
        &[],
        None,
        "fixture reconnect without a provider process claim",
    )
    .await
    .unwrap();
    assert!(
        events.is_empty(),
        "evidence-complete review must remain waiting"
    );
    tx.commit().await.unwrap();
    assert_eq!(state(&store).await, before);

    for corrupt in [
        "UPDATE artifacts SET run_id='00000000-0000-0000-0000-000000000004' WHERE id=$1",
        "DELETE FROM artifacts WHERE id=$1",
    ] {
        // The private native method shares this test transaction. Rollback keeps
        // negative metadata variants isolated without resetting any lifecycle.
        let mut tx = store.pool.begin().await.unwrap();
        sqlx::query(corrupt)
            .bind(artifact_id)
            .execute(&mut *tx)
            .await
            .unwrap();
        let events = mark_runner_runs_lost_tx(
            &mut tx,
            RUNNER,
            &[],
            None,
            "fixture reconnect with unbound review evidence",
        )
        .await
        .unwrap();
        assert!(
            !events.is_empty(),
            "unbound review must take the normal loss path"
        );
        let status: String = sqlx::query_scalar("SELECT status FROM runs WHERE id=$1")
            .bind(command.run_id)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        assert_eq!(status, "lost");
        tx.rollback().await.unwrap();
        assert_eq!(state(&store).await, before);
    }
}

// Publication tests reuse native checkpoint, staged/finalized artifact metadata,
// independent review and publisher credentials. No publisher process, Git effect,
// object download, signature validation or real Git-bundle bytes are claimed.
async fn publication_fixture(
    pool: PgPool,
) -> (
    PgStore,
    PendingRunnerCommand,
    Uuid,
    StartPullRequestPublicationInput,
) {
    let (store, command, artifact_id) = waiting_export_fixture_with_publication(pool, true).await;
    store
        .apply_runner_event(retention_event(&command))
        .await
        .unwrap();
    store
        .decide_verification(
            CORP,
            command.run_id,
            REVIEWER,
            true,
            "Independent SQLx metadata review",
            Some(Uuid::new_v4()),
        )
        .await
        .unwrap();
    let publisher_hash = digest(&"issue148 SQLx-only publisher credential");
    store
        .create_publication_publisher_credential(
            CORP,
            OWNER,
            "issue148-store-publisher",
            &publisher_hash,
            Utc::now() + Duration::hours(1),
        )
        .await
        .unwrap();
    let deliverable_id =
        sqlx::query_scalar("SELECT id FROM source_deliverables WHERE artifact_id=$1")
            .bind(artifact_id)
            .fetch_one(&store.pool)
            .await
            .unwrap();
    let snapshot = state(&store).await;
    assert_eq!(snapshot["item"]["state"], "verified");
    assert_eq!(snapshot["recoveries"][0]["status"], "completed");
    assert_eq!(
        retained_run(&snapshot, command.run_id)["status"],
        "completed"
    );
    let input = StartPullRequestPublicationInput {
        corp_id: CORP,
        work_item_id: ITEM,
        actor_id: OWNER,
        actor_role: "owner".to_owned(),
        source_deliverable_id: deliverable_id,
        target_repository: "fixture/source".to_owned(),
        base_ref: "main".to_owned(),
        branch: "ecorp/issue148-checkpoint".to_owned(),
        title: "Reviewed checkpoint metadata".to_owned(),
        body: "Closes https://github.com/fixture/source/issues/148\nSQLx metadata only.".to_owned(),
        authorization_id: Uuid::new_v4(),
        authorization_reason: "Publish this exact reviewed export".to_owned(),
        effect_key: Uuid::new_v4().to_string(),
        idempotency_key: Uuid::new_v4().to_string(),
        publisher_id: "issue148-store-publisher".to_owned(),
        publisher_credential_hash: publisher_hash,
        lease_seconds: 300,
    };
    (store, command, artifact_id, input)
}

fn publication_renewal(
    input: &StartPullRequestPublicationInput,
    started: &PullRequestPublicationOutcome,
) -> RenewPullRequestPublicationInput {
    RenewPullRequestPublicationInput {
        corp_id: CORP,
        publication_id: started.publication.id,
        actor_id: input.actor_id,
        publisher_id: input.publisher_id.clone(),
        publisher_credential_hash: input.publisher_credential_hash.clone(),
        publisher_token: started.publisher_token.unwrap(),
        expected_version: started.publication.version,
        idempotency_key: Uuid::new_v4().to_string(),
        lease_seconds: 300,
    }
}

async fn publication_state(store: &PgStore) -> Value {
    let mut snapshot = state(store).await;
    snapshot["publication"] = sqlx::query_scalar(
        "SELECT jsonb_build_object(
          'rows',(SELECT coalesce(jsonb_agg(to_jsonb(p) ORDER BY id),'[]') FROM pull_request_publications p),
          'attempts',(SELECT coalesce(jsonb_agg(to_jsonb(a) ORDER BY id),'[]') FROM pull_request_publication_attempts a),
          'operations',(SELECT coalesce(jsonb_agg(to_jsonb(o) ORDER BY idempotency_key),'[]') FROM pull_request_publication_operations o),
          'credentials',(SELECT coalesce(jsonb_agg(to_jsonb(c) ORDER BY id),'[]') FROM publication_publisher_credentials c))",
    ).fetch_one(&store.pool).await.unwrap();
    snapshot
}

fn assert_publication_preserves_models(before: &Value, after: &Value) {
    for key in [
        "source",
        "runs",
        "tasks",
        "missions",
        "agents",
        "recoveries",
        "commands",
        "artifacts",
        "deliverables",
        "verification_evidence",
        "verification_requests",
    ] {
        assert_eq!(after[key], before[key], "publication must not change {key}");
    }
    let prefix = before["events"].as_array().unwrap();
    let events = after["events"].as_array().unwrap();
    assert_eq!(&events[..prefix.len()], prefix.as_slice());
    assert!(
        events[prefix.len()..]
            .iter()
            .all(|event| !event["type"].as_str().unwrap().starts_with("run."))
    );
    assert_eq!(after["source"]["input_tokens"], 6000);
    assert_eq!(after["source"]["breaker_stage"], "stop");
    assert_eq!(after["task"]["attempt_count"], 2);
    assert_eq!(after["task"]["max_attempts"], 2);
}

async fn reject_publication(
    store: &PgStore,
    input: &StartPullRequestPublicationInput,
    renewal: Option<&RenewPullRequestPublicationInput>,
) {
    let before = publication_state(store).await;
    if let Some(renewal) = renewal {
        assert!(
            store
                .renew_pull_request_publication(renewal.clone())
                .await
                .is_err()
        );
        assert_eq!(
            publication_state(store).await,
            before,
            "failed renewal must roll back"
        );
        // A rejected, current-version native checkpoint metadata request.
        // No Git runs here and no branch effect is claimed to have happened.
        assert!(
            store
                .record_pull_request_publication_checkpoint(
                    RecordPullRequestPublicationCheckpointInput {
                        corp_id: CORP,
                        publication_id: renewal.publication_id,
                        actor_id: renewal.actor_id,
                        publisher_id: renewal.publisher_id.clone(),
                        publisher_credential_hash: renewal.publisher_credential_hash.clone(),
                        publisher_token: renewal.publisher_token,
                        expected_version: before["publication"]["rows"][0]["version"]
                            .as_i64()
                            .unwrap(),
                        idempotency_key: Uuid::new_v4().to_string(),
                        checkpoint: PullRequestPublicationCheckpointInput::BranchPushed {
                            commit_sha: "d".repeat(40),
                        },
                    }
                )
                .await
                .is_err()
        );
    } else {
        assert!(
            store
                .start_pull_request_publication(input.clone())
                .await
                .is_err()
        );
    }
    assert_eq!(
        publication_state(store).await,
        before,
        "rejected publication must roll back"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_publication_start_renew_replay_preserves_provenance_and_spend(
    pool: PgPool,
) {
    let (store, command, _, input) = publication_fixture(pool).await;
    let before = state(&store).await;
    let authority: budget_checkpoint::Authority =
        serde_json::from_value(before["recoveries"][0]["checkpoint_authority"].clone()).unwrap();
    let authority_sha256 = digest(&authority);
    let source_event = |kind: &str| {
        before["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|event| event["aggregate_id"] == json!(SOURCE) && event["type"] == kind)
            .unwrap()["id"]
            .clone()
    };
    assert_eq!(
        json!(authority.checkpoint_event_id),
        source_event("run.workspace_preserved")
    );
    assert_eq!(
        json!(authority.termination_event_id),
        source_event("run.session_terminated")
    );
    let started = store
        .start_pull_request_publication(input.clone())
        .await
        .unwrap();
    assert!(!started.replayed && !started.busy);
    assert_eq!(started.publication.run_id, command.run_id);
    assert_eq!(started.publication.provenance["schema_version"], 3);
    assert_eq!(
        started.publication.provenance["checkpoint"],
        json!({
            "schema_version":1,"recovery_id":before["recoveries"][0]["id"],
            "origin_run_id":SOURCE,"workspace_run_id":SOURCE,
            "checkpoint_event_id":authority.checkpoint_event_id,"termination_event_id":authority.termination_event_id,
            "workspace_fingerprint":"b".repeat(64),"source_base_commit":"a".repeat(40),
            "original_head_commit":"a".repeat(40),"verified_head_commit":"d".repeat(40),
            "authority_sha256":authority_sha256,"execution_mode":"verification_only",
        })
    );
    let replayed = store
        .start_pull_request_publication(input.clone())
        .await
        .unwrap();
    assert!(replayed.replayed && !replayed.busy);
    assert_eq!(replayed.publication.id, started.publication.id);
    assert_eq!(replayed.publisher_token, started.publisher_token);
    let renewal = publication_renewal(&input, &started);
    let renewed = store
        .renew_pull_request_publication(renewal.clone())
        .await
        .unwrap();
    let renewed_again = store.renew_pull_request_publication(renewal).await.unwrap();
    assert!(renewed_again.replayed);
    assert_eq!(
        renewed.publication.provenance,
        started.publication.provenance
    );
    assert_eq!(
        renewed_again.publication.version,
        renewed.publication.version
    );
    assert_eq!(renewed_again.publication.attempt_count, 1); // Publisher, not model attempts.
    let after = publication_state(&store).await;
    assert_publication_preserves_models(&before, &after);
    assert_eq!(after["runs"].as_array().unwrap().len(), 2);
    assert_eq!(after["publication"]["rows"].as_array().unwrap().len(), 1);
    assert_eq!(
        after["publication"]["attempts"].as_array().unwrap().len(),
        1
    );
    assert!(
        !renewed.publication.auto_merge_enabled
            && !renewed.publication.merge_authorized
            && !renewed.publication.deployment_authorized
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_publication_healthy_history_ignores_only_retrospective_model_budget(
    pool: PgPool,
) {
    let (store, command, _, input) = publication_fixture(pool).await;
    let historical_task = Uuid::from_u128(60);
    let historical_run = Uuid::from_u128(61);
    // Historical metadata: another completed task, not an extra checkpoint
    // descendant. Its own spend is healthy; shared mission/actor/Corp spend is not.
    sqlx::query(
        "INSERT INTO tasks SELECT (jsonb_populate_record(NULL::tasks,to_jsonb(t) ||
          jsonb_build_object('id',$1::uuid,'plan_key','historical','attempt_count',1,
            'created_at',now()-interval '1 hour','updated_at',now()-interval '1 hour'))).*
         FROM tasks t WHERE id=$2",
    )
    .bind(historical_task)
    .bind(TASK)
    .execute(&store.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO runs SELECT (jsonb_populate_record(NULL::runs,to_jsonb(r) ||
          jsonb_build_object('id',$1::uuid,'task_id',$2::uuid,'assignment_token',$3::uuid,
            'workspace_run_id',$1::uuid,'resumed_from_run_id',NULL,'execution_mode','provider',
            'provider_session_id','historical-session','model','fixture-model','input_tokens',1000,
            'budget_tokens_limit',100000,'budget_cost_microusd_limit',1000000,
            'workspace_path','historical-worktree','workspace_branch','crony/historical',
            'deliverable_sha256',NULL,'created_at',now()-interval '1 hour',
            'updated_at',now()-interval '1 hour'))).* FROM runs r WHERE id=$4",
    )
    .bind(historical_run)
    .bind(historical_task)
    .bind(Uuid::new_v4())
    .bind(command.run_id)
    .execute(&store.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO verification_evidence SELECT (jsonb_populate_record(NULL::verification_evidence,
          to_jsonb(e) || jsonb_build_object('id',$1::uuid,'run_id',$2::uuid,'task_id',$3::uuid))).*
         FROM verification_evidence e WHERE run_id=$4 AND check_index=0",
    ).bind(Uuid::new_v4()).bind(historical_run).bind(historical_task).bind(command.run_id)
        .execute(&store.pool).await.unwrap();
    sqlx::query(
        "INSERT INTO verification_requests SELECT (jsonb_populate_record(NULL::verification_requests,
          to_jsonb(v) || jsonb_build_object('run_id',$1::uuid,'task_id',$2::uuid,'decision_key',$3::uuid))).*
         FROM verification_requests v WHERE run_id=$4",
    ).bind(historical_run).bind(historical_task).bind(Uuid::new_v4()).bind(command.run_id)
        .execute(&store.pool).await.unwrap();
    sqlx::query("INSERT INTO corp_budget_policies(corp_id,actor_tokens_per_24h,corp_tokens_per_24h) VALUES($1,7000,7000)")
        .bind(CORP).execute(&store.pool).await.unwrap();
    let before = state(&store).await;
    assert_eq!(
        retained_run(&before, historical_run)["breaker_stage"],
        "healthy"
    );
    assert_eq!(
        before["runs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|run| run["input_tokens"].as_i64().unwrap())
            .sum::<i64>(),
        7000
    );
    assert_eq!(before["mission"]["budget_tokens"], 5000);
    let started = store
        .start_pull_request_publication(input.clone())
        .await
        .unwrap();
    let renewal = publication_renewal(&input, &started);
    store
        .renew_pull_request_publication(renewal.clone())
        .await
        .unwrap();
    assert_publication_preserves_models(&before, &state(&store).await);
    for (field, denied, restored) in [
        ("breaker_stage", json!("stop"), json!("healthy")),
        ("breaker_stage", json!("suspend"), json!("healthy")),
        ("repeated_tool_count", json!(5), json!(0)),
        ("no_progress_events", json!(8), json!(0)),
    ] {
        let update = format!(
            "UPDATE runs SET {field}=(jsonb_populate_record(NULL::runs,jsonb_build_object('{field}',$2::jsonb))).{field} WHERE id=$1"
        );
        sqlx::query(&update)
            .bind(historical_run)
            .bind(denied)
            .execute(&store.pool)
            .await
            .unwrap();
        reject_publication(&store, &input, Some(&renewal)).await;
        sqlx::query(&update)
            .bind(historical_run)
            .bind(restored)
            .execute(&store.pool)
            .await
            .unwrap();
    }
    assert!(
        store
            .renew_pull_request_publication(renewal.clone())
            .await
            .unwrap()
            .replayed
    );
    let mut tx = store.pool.begin().await.unwrap();
    append_event_tx(
        &mut tx,
        NewEvent::new(
            CORP,
            Some(OWNER),
            "run.stop_requested",
            "run",
            historical_run,
            Uuid::new_v4().to_string(),
            json!({"reason":"Explicit stop in another task of this mission"}),
        ),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    reject_publication(&store, &input, Some(&renewal)).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_publication_requires_checkpoint_mode_and_native_origin_events(
    pool: PgPool,
) {
    let (store, command, _, input) = publication_fixture(pool).await;
    let before = state(&store).await;
    let recovery = &before["recoveries"][0];
    // The same zero-limit verifier cannot borrow a checkpoint exemption when
    // its persisted recovery is ordinary verifier_only instead.
    sqlx::query(
        "UPDATE factory_verification_recoveries SET mode='verifier_only',checkpoint_authority=NULL,
         request=jsonb_set(request,'{mode}','\"verifier_only\"') WHERE replacement_run_id=$1",
    )
    .bind(command.run_id)
    .execute(&store.pool)
    .await
    .unwrap();
    reject_publication(&store, &input, None).await;
    sqlx::query(
        "UPDATE factory_verification_recoveries SET mode='checkpoint_verification',
         checkpoint_authority=$2,request=$3 WHERE replacement_run_id=$1",
    )
    .bind(command.run_id)
    .bind(&recovery["checkpoint_authority"])
    .bind(&recovery["request"])
    .execute(&store.pool)
    .await
    .unwrap();
    for (key, kind) in [
        ("termination_event_id", "run.session_terminated"),
        ("checkpoint_event_id", "run.workspace_preserved"),
    ] {
        let id = Uuid::parse_str(recovery["checkpoint_authority"][key].as_str().unwrap()).unwrap();
        sqlx::query("UPDATE events SET type='fixture.withdrawn' WHERE id=$1")
            .bind(id)
            .execute(&store.pool)
            .await
            .unwrap();
        reject_publication(&store, &input, None).await;
        sqlx::query("UPDATE events SET type=$2 WHERE id=$1")
            .bind(id)
            .bind(kind)
            .execute(&store.pool)
            .await
            .unwrap();
    }
    let started = store.start_pull_request_publication(input).await.unwrap();
    assert_eq!(started.publication.run_id, command.run_id);
    assert_publication_preserves_models(&before, &state(&store).await);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_publication_rechecks_source_policy_checkpoint_and_quarantine(
    pool: PgPool,
) {
    let (store, command, _, input) = publication_fixture(pool).await;
    let started = store
        .start_pull_request_publication(input.clone())
        .await
        .unwrap();
    let renewal = publication_renewal(&input, &started);
    store
        .renew_pull_request_publication(renewal.clone())
        .await
        .unwrap();
    for (changed, restored) in [
        (
            "UPDATE runs SET source_base_commit=repeat('e',40) WHERE id=$1",
            "UPDATE runs SET source_base_commit=repeat('a',40) WHERE id=$1",
        ),
        (
            "UPDATE tasks SET verification_policy=jsonb_set(verification_policy,'{checks,0,min_bytes}','2') WHERE id=(SELECT task_id FROM runs WHERE id=$1)",
            "UPDATE tasks SET verification_policy=jsonb_set(verification_policy,'{checks,0,min_bytes}','1') WHERE id=(SELECT task_id FROM runs WHERE id=$1)",
        ),
        (
            "UPDATE events SET payload=jsonb_set(payload,'{source_checkpoint,head_commit}',to_jsonb(repeat('e',40))) WHERE type='run.workspace_preserved' AND aggregate_id=(SELECT workspace_run_id FROM runs WHERE id=$1)",
            "UPDATE events SET payload=jsonb_set(payload,'{source_checkpoint,head_commit}',to_jsonb(repeat('a',40))) WHERE type='run.workspace_preserved' AND aggregate_id=(SELECT workspace_run_id FROM runs WHERE id=$1)",
        ),
        (
            "UPDATE runs SET workspace_disposition='quarantined' WHERE id=(SELECT workspace_run_id FROM runs WHERE id=$1)",
            "UPDATE runs SET workspace_disposition='preserved' WHERE id=(SELECT workspace_run_id FROM runs WHERE id=$1)",
        ),
        (
            "UPDATE runs SET workspace_fingerprint=repeat('e',64) WHERE id=$1",
            "UPDATE runs SET workspace_fingerprint=repeat('b',64) WHERE id=$1",
        ),
    ] {
        sqlx::query(changed)
            .bind(command.run_id)
            .execute(&store.pool)
            .await
            .unwrap();
        reject_publication(&store, &input, Some(&renewal)).await;
        sqlx::query(restored)
            .bind(command.run_id)
            .execute(&store.pool)
            .await
            .unwrap();
    }
    // A valid live checkpoint cannot excuse absent or altered persisted
    // publication provenance on renewal or a native publication checkpoint.
    for changed in [
        "UPDATE pull_request_publications SET provenance=provenance-'checkpoint' WHERE id=$1",
        "UPDATE pull_request_publications SET provenance=jsonb_set(provenance,'{checkpoint,authority_sha256}',to_jsonb(repeat('e',64))) WHERE id=$1",
    ] {
        sqlx::query(changed)
            .bind(started.publication.id)
            .execute(&store.pool)
            .await
            .unwrap();
        reject_publication(&store, &input, Some(&renewal)).await;
        sqlx::query("UPDATE pull_request_publications SET provenance=$2 WHERE id=$1")
            .bind(started.publication.id)
            .bind(&started.publication.provenance)
            .execute(&store.pool)
            .await
            .unwrap();
    }
    assert!(
        store
            .renew_pull_request_publication(renewal)
            .await
            .unwrap()
            .replayed
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_publication_requires_ready_unexpired_exact_artifact_metadata(
    pool: PgPool,
) {
    let (store, _, artifact_id, input) = publication_fixture(pool).await;
    let original = state(&store).await;
    let artifact = original["artifacts"][0].clone();
    let deliverable = original["deliverables"][0].clone();
    // Delete before publication (whose FKs intentionally restrict deletion).
    // Restore only these exact synthetic metadata rows for subsequent cases.
    sqlx::query("DELETE FROM artifacts WHERE id=$1")
        .bind(artifact_id)
        .execute(&store.pool)
        .await
        .unwrap();
    reject_publication(&store, &input, None).await;
    sqlx::query("INSERT INTO artifacts SELECT * FROM jsonb_populate_record(NULL::artifacts,$1)")
        .bind(&artifact)
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO source_deliverables SELECT * FROM jsonb_populate_record(NULL::source_deliverables,$1)")
        .bind(&deliverable).execute(&store.pool).await.unwrap();
    let started = store
        .start_pull_request_publication(input.clone())
        .await
        .unwrap();
    let renewal = publication_renewal(&input, &started);
    store
        .renew_pull_request_publication(renewal.clone())
        .await
        .unwrap();
    for changed in [
        "UPDATE artifacts SET status='staged',finalized_at=NULL WHERE id=$1",
        "UPDATE artifacts SET artifact_role='provider_evidence' WHERE id=$1",
        "UPDATE artifacts SET run_id='00000000-0000-0000-0000-000000000004' WHERE id=$1",
        "UPDATE artifacts SET sha256=repeat('e',64) WHERE id=$1",
        "UPDATE artifacts SET created_at=now()-interval '2 hours',retention_until=now()-interval '1 hour' WHERE id=$1",
    ] {
        sqlx::query(changed)
            .bind(artifact_id)
            .execute(&store.pool)
            .await
            .unwrap();
        reject_publication(&store, &input, Some(&renewal)).await;
        sqlx::query(
            "UPDATE artifacts a SET status=p.status,finalized_at=p.finalized_at,artifact_role=p.artifact_role,
             run_id=p.run_id,sha256=p.sha256,created_at=p.created_at,retention_until=p.retention_until
             FROM jsonb_populate_record(NULL::artifacts,$2) p WHERE a.id=$1",
        ).bind(artifact_id).bind(&artifact).execute(&store.pool).await.unwrap();
    }
    assert!(
        store
            .renew_pull_request_publication(renewal)
            .await
            .unwrap()
            .replayed
    );
    assert_publication_preserves_models(&original, &state(&store).await);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_publication_rechecks_actor_room_and_publisher_credentials(
    pool: PgPool,
) {
    let (store, _, _, input) = publication_fixture(pool).await;
    let started = store
        .start_pull_request_publication(input.clone())
        .await
        .unwrap();
    let renewal = publication_renewal(&input, &started);
    store
        .renew_pull_request_publication(renewal.clone())
        .await
        .unwrap();
    let before = publication_state(&store).await;
    for (changed, restored) in [
        (
            "UPDATE actors SET role='member' WHERE id=$1",
            "UPDATE actors SET role='owner' WHERE id=$1",
        ),
        (
            "DELETE FROM room_memberships WHERE actor_id=$1 AND room_id='00000000-0000-0000-0000-000000000006'",
            "INSERT INTO room_memberships(actor_id,room_id) VALUES($1,'00000000-0000-0000-0000-000000000006')",
        ),
    ] {
        sqlx::query(changed)
            .bind(OWNER)
            .execute(&store.pool)
            .await
            .unwrap();
        reject_publication(&store, &input, None).await; // Also fences exact start replay.
        reject_publication(&store, &input, Some(&renewal)).await;
        sqlx::query(restored)
            .bind(OWNER)
            .execute(&store.pool)
            .await
            .unwrap();
    }
    for mismatched_publisher in [false, true] {
        let mut bad_start = input.clone();
        let mut bad_renewal = renewal.clone();
        if mismatched_publisher {
            bad_start.publisher_id = "another-publisher".to_owned();
            bad_renewal.publisher_id = bad_start.publisher_id.clone();
        } else {
            bad_start.publisher_credential_hash = "f".repeat(64);
            bad_renewal.publisher_credential_hash = bad_start.publisher_credential_hash.clone();
        }
        reject_publication(&store, &bad_start, None).await;
        reject_publication(&store, &bad_start, Some(&bad_renewal)).await;
    }
    let credential = &before["publication"]["credentials"][0];
    let credential_id = Uuid::parse_str(credential["id"].as_str().unwrap()).unwrap();
    for changed in [
        "UPDATE publication_publisher_credentials SET created_at=now()-interval '2 hours',expires_at=now()-interval '1 hour' WHERE id=$1",
        "UPDATE publication_publisher_credentials SET revoked_at=now() WHERE id=$1",
    ] {
        sqlx::query(changed)
            .bind(credential_id)
            .execute(&store.pool)
            .await
            .unwrap();
        reject_publication(&store, &input, None).await;
        reject_publication(&store, &input, Some(&renewal)).await;
        sqlx::query(
            "UPDATE publication_publisher_credentials c SET created_at=p.created_at,expires_at=p.expires_at,
             revoked_at=p.revoked_at FROM jsonb_populate_record(NULL::publication_publisher_credentials,$2) p WHERE c.id=$1",
        ).bind(credential_id).bind(credential).execute(&store.pool).await.unwrap();
    }
    assert!(
        store
            .start_pull_request_publication(input)
            .await
            .unwrap()
            .replayed
    );
    assert!(
        store
            .renew_pull_request_publication(renewal)
            .await
            .unwrap()
            .replayed
    );
    assert_publication_preserves_models(&before, &state(&store).await);
}

async fn assert_publication_failure_after_revocation(pool: PgPool, revoked: &str) {
    let (store, _, artifact_id, input) = publication_fixture(pool).await;
    let started = store
        .start_pull_request_publication(input.clone())
        .await
        .unwrap();
    let (statement, target) = match revoked {
        "role" => ("UPDATE actors SET role='member' WHERE id=$1", OWNER),
        "room" => ("DELETE FROM room_memberships WHERE actor_id=$1", OWNER),
        "artifact" => (
            "UPDATE artifacts SET created_at=now()-interval '2 hours',
             retention_until=now()-interval '1 hour' WHERE id=$1",
            artifact_id,
        ),
        _ => panic!("unknown fixture revocation"),
    };
    sqlx::query(statement)
        .bind(target)
        .execute(&store.pool)
        .await
        .unwrap();
    let renewal = publication_renewal(&input, &started);
    reject_publication(&store, &input, Some(&renewal)).await;
    let before = publication_state(&store).await;
    let failure = RecordPullRequestPublicationCheckpointInput {
        corp_id: CORP,
        publication_id: started.publication.id,
        actor_id: OWNER,
        publisher_id: input.publisher_id.clone(),
        publisher_credential_hash: input.publisher_credential_hash.clone(),
        publisher_token: started.publisher_token.unwrap(),
        expected_version: started.publication.version,
        idempotency_key: Uuid::new_v4().to_string(),
        checkpoint: PullRequestPublicationCheckpointInput::Failed {
            failure_detail: "Publication stopped because its authority changed".to_owned(),
        },
    };
    // Failure reporting remains owned; it is not an unauthenticated cleanup API.
    for mismatch in ["actor", "token", "credential", "version"] {
        let mut invalid = failure.clone();
        match mismatch {
            "actor" => invalid.actor_id = REVIEWER,
            "token" => invalid.publisher_token = Uuid::new_v4(),
            "credential" => invalid.publisher_credential_hash = "f".repeat(64),
            "version" => invalid.expected_version += 1,
            _ => unreachable!(),
        }
        assert!(
            store
                .record_pull_request_publication_checkpoint(invalid)
                .await
                .is_err()
        );
        assert_eq!(publication_state(&store).await, before);
    }
    let recorded = store
        .record_pull_request_publication_checkpoint(failure.clone())
        .await
        .unwrap();
    assert!(recorded.publisher_token.is_none());
    assert_eq!(
        recorded.publication.failure_detail.as_deref(),
        Some("Publication stopped because its authority changed")
    );
    let after = publication_state(&store).await;
    assert_eq!(after["publication"]["attempts"][0]["state"], "failed");
    assert!(!after["publication"]["attempts"][0]["finished_at"].is_null());
    assert!(after["publication"]["rows"][0]["publisher_lease_expires_at"].is_null());
    assert_eq!(after["item"], before["item"]);
    assert_publication_preserves_models(&before, &after);
    assert_eq!(
        after["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["type"] == "factory.publication_failed")
            .count(),
        1
    );
    let replayed = store
        .record_pull_request_publication_checkpoint(failure)
        .await
        .unwrap();
    assert!(replayed.replayed);
    let replay = publication_state(&store).await;
    for field in ["rows", "attempts", "operations"] {
        assert_eq!(replay["publication"][field], after["publication"][field]);
    }
    assert_eq!(replay["events"], after["events"]);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_publication_review_failure_after_role_change(pool: PgPool) {
    assert_publication_failure_after_revocation(pool, "role").await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_publication_review_failure_after_room_revocation(pool: PgPool) {
    assert_publication_failure_after_revocation(pool, "room").await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_publication_review_failure_after_artifact_expiry(pool: PgPool) {
    assert_publication_failure_after_revocation(pool, "artifact").await;
}

async fn assert_legacy_checkpoint_publication_upgrade(pool: PgPool, renewal_replay: bool) {
    let (store, _, _, input) = publication_fixture(pool).await;
    let started = store
        .start_pull_request_publication(input.clone())
        .await
        .unwrap();
    let renewal = publication_renewal(&input, &started);
    if renewal_replay {
        store
            .renew_pull_request_publication(renewal.clone())
            .await
            .unwrap();
    }
    // Disposable metadata representing an in-flight older deployment. This is
    // not a claim that an old binary or real remote Git effects ran in this test.
    let mut legacy = started.publication.provenance.clone();
    legacy.as_object_mut().unwrap().remove("checkpoint");
    legacy["schema_version"] = json!(if renewal_replay { 2 } else { 1 });
    if !renewal_replay {
        let issue = legacy["source_issue"].as_object_mut().unwrap();
        issue.remove("claimed_revision");
        issue.remove("recovery_id");
    }
    sqlx::query("UPDATE pull_request_publications SET provenance=$2 WHERE id=$1")
        .bind(started.publication.id)
        .bind(legacy)
        .execute(&store.pool)
        .await
        .unwrap();
    let before = publication_state(&store).await;
    let result = if renewal_replay {
        store.renew_pull_request_publication(renewal).await.unwrap()
    } else {
        store
            .record_pull_request_publication_checkpoint(
                RecordPullRequestPublicationCheckpointInput {
                    corp_id: CORP,
                    publication_id: started.publication.id,
                    actor_id: OWNER,
                    publisher_id: input.publisher_id.clone(),
                    publisher_credential_hash: input.publisher_credential_hash.clone(),
                    publisher_token: started.publisher_token.unwrap(),
                    expected_version: started.publication.version,
                    idempotency_key: Uuid::new_v4().to_string(),
                    checkpoint: PullRequestPublicationCheckpointInput::BranchPushed {
                        commit_sha: "d".repeat(40),
                    },
                },
            )
            .await
            .unwrap()
    };
    let after = publication_state(&store).await;
    assert_eq!(result.replayed, renewal_replay);
    assert_eq!(result.publication.id, started.publication.id);
    assert_eq!(result.publication.attempt_count, 1);
    assert_eq!(result.publication.provenance["schema_version"], 3);
    assert_eq!(
        result.publication.provenance["checkpoint"],
        started.publication.provenance["checkpoint"]
    );
    assert_eq!(
        result.publication.provenance["source_issue"],
        started.publication.provenance["source_issue"]
    );
    assert_eq!(
        json!(result.publication.provenance),
        after["publication"]["rows"][0]["provenance"],
        "the response must return the upgraded, not pre-validation, record"
    );
    assert_publication_preserves_models(&before, &after);
    if renewal_replay {
        assert_eq!(after["events"], before["events"]);
        assert_eq!(
            after["publication"]["operations"],
            before["publication"]["operations"]
        );
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_publication_review_legacy_upgrade_on_renewal_replay(pool: PgPool) {
    assert_legacy_checkpoint_publication_upgrade(pool, true).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue148_checkpoint_publication_review_legacy_upgrade_on_checkpoint(pool: PgPool) {
    assert_legacy_checkpoint_publication_upgrade(pool, false).await;
}
