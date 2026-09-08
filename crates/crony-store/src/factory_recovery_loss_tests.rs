//! Native store regressions in SQLx-owned databases with the actual migrations.
//! These metadata fixtures are not provider execution or signed-object evidence.
use super::*;

const CORP: Uuid = Uuid::from_u128(1);
const OWNER: Uuid = Uuid::from_u128(2);
const ROOM: Uuid = Uuid::from_u128(3);
const AGENT: Uuid = Uuid::from_u128(4);
const MISSION: Uuid = Uuid::from_u128(5);
const TASK: Uuid = Uuid::from_u128(6);
const SOURCE: Uuid = Uuid::from_u128(7);
const RUN: Uuid = Uuid::from_u128(8);
const ITEM: Uuid = Uuid::from_u128(9);
const RECOVERY: Uuid = Uuid::from_u128(10);
const COMMAND: Uuid = Uuid::from_u128(11);
const TOKEN: Uuid = Uuid::from_u128(12);
const EPOCH: Uuid = Uuid::from_u128(13);
const RUNNER: &str = "issue183-runner";

async fn fixture(pool: PgPool, mode: &str) -> PgStore {
    sqlx::raw_sql(
        r#"
        INSERT INTO corps(id,slug,name) VALUES
          ('00000000-0000-0000-0000-000000000001','issue183','Recovery loss fixture');
        INSERT INTO actors(id,corp_id,name,kind,role) VALUES
          ('00000000-0000-0000-0000-000000000002','00000000-0000-0000-0000-000000000001','Owner','human','owner'),
          ('00000000-0000-0000-0000-000000000014','00000000-0000-0000-0000-000000000001','Worker','agent','worker');
        INSERT INTO rooms(id,corp_id,name,purpose) VALUES
          ('00000000-0000-0000-0000-000000000003','00000000-0000-0000-0000-000000000001','QA','SQLx-only recovery loss');
        INSERT INTO room_memberships(room_id,actor_id) VALUES
          ('00000000-0000-0000-0000-000000000003','00000000-0000-0000-0000-000000000002');
        INSERT INTO agents(id,corp_id,actor_id,name,role,adapter,status,current_run_id,accent) VALUES
          ('00000000-0000-0000-0000-000000000004','00000000-0000-0000-0000-000000000001',
           '00000000-0000-0000-0000-000000000014','Worker','worker','openai-codex','working',
           '00000000-0000-0000-0000-000000000008','#123456');
        INSERT INTO missions(id,corp_id,room_id,requested_by,title,status,budget_tokens,
                             original_budget_tokens,original_budget_cost_microusd) VALUES
          ('00000000-0000-0000-0000-000000000005','00000000-0000-0000-0000-000000000001',
           '00000000-0000-0000-0000-000000000003','00000000-0000-0000-0000-000000000002',
           'Recovery loss','running',1000000,1000000,5000000);
        "#,
    )
    .execute(&pool)
    .await
    .unwrap();
    let contract = json!({
        "objective":"preserve result.md", "expected_output":"result.md",
        "source_repository":"fixture/source", "source_base_ref":"main",
        "source_base_commit":"a".repeat(40), "acceptance_tests":["result.md exists"],
        "allowed_tools":["filesystem"], "prohibited_actions":["outside worktree"],
        "references":[], "write_scope":["result.md"], "budget_tokens":100000,
        "budget_cost_microusd":1000000, "deadline_at":null, "escalation":"ask owner"
    });
    let policy = json!({
        "checks":[{"type":"file","path":"result.md","min_bytes":1}],
        "manual_gate":null
    });
    sqlx::query(
        "INSERT INTO tasks(id,corp_id,mission_id,title,objective,status,assigned_agent_id,
                           plan_key,contract,verification_policy,attempt_count,max_attempts)
         VALUES($1,$2,$3,'Recovery','preserve result.md','running',$4,'delivery',$5,$6,2,4)",
    )
    .bind(TASK)
    .bind(CORP)
    .bind(MISSION)
    .bind(AGENT)
    .bind(contract)
    .bind(&policy)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO runs(id,corp_id,task_id,agent_id,runner_id,assignment_token,status,
                          workspace_run_id,provider_session_id,workspace_path,workspace_branch,
                          workspace_base_ref,workspace_base_commit,workspace_disposition,
                          workspace_fingerprint,verification_status,source_repository,
                          source_base_ref,source_base_commit,input_tokens,output_tokens,cost_microusd,
                          created_at)
         VALUES($1,$2,$3,$4,$5,$6,'failed',$1,'fixture-session','owned-worktree','crony/fixture',
                'main',$7,'preserved',$8,'failed','fixture/source','main',$7,11,13,17,
                now()-interval '2 minutes')",
    )
    .bind(SOURCE).bind(CORP).bind(TASK).bind(AGENT).bind(RUNNER).bind(Uuid::new_v4())
    .bind("a".repeat(40)).bind("b".repeat(64))
    .execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO runs(id,corp_id,task_id,agent_id,runner_id,assignment_token,status,
                          workspace_run_id,resumed_from_run_id,provider_session_id,execution_mode,
                          source_repository,source_base_ref,source_base_commit,created_at)
         VALUES($1,$2,$3,$4,$5,$6,'starting',$7,$7,$8,$9,
                'fixture/source','main',$10,now()-interval '1 minute')",
    )
    .bind(RUN)
    .bind(CORP)
    .bind(TASK)
    .bind(AGENT)
    .bind(RUNNER)
    .bind(TOKEN)
    .bind(SOURCE)
    .bind((mode == "source_correction").then_some("fixture-session"))
    .bind(if mode == "verifier_only" {
        "verification_only"
    } else {
        "provider"
    })
    .bind("a".repeat(40))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO factory_work_items(id,corp_id,source_kind,source_project_owner,
              source_project_number,source_project_item_id,source_repository_owner,
              source_repository_name,source_issue_number,source_issue_node_id,source_issue_url,
              source_title,source_revision,state,version,claim_owner_id,claim_token,
              lease_expires_at,mission_id,policy)
         VALUES($1,$2,'github_project_issue','fixture',183,'item-183','fixture','source',183,
                'issue-183','https://github.com/fixture/source/issues/183','Loss fixture',
                'revision-1','running',3,$3,$4,now()+interval '1 hour',$5,'{\"fixture\":true}')",
    )
    .bind(ITEM)
    .bind(CORP)
    .bind(OWNER)
    .bind(Uuid::from_u128(15))
    .bind(MISSION)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO factory_verification_recoveries(id,corp_id,factory_work_item_id,
              mission_id,task_id,source_run_id,replacement_run_id,mode,status,authorized_by,
              reason,idempotency_key,observed_source_revision,reviewed_source_snapshot,
              previous_verification_policy,replacement_verification_policy,request)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,'running',$9,'Owned fixture',$10,'revision-1',
                '{\"source_revision\":\"revision-1\",\"issue_number\":183}',$11,$11,$12)",
    )
    .bind(RECOVERY)
    .bind(CORP)
    .bind(ITEM)
    .bind(MISSION)
    .bind(TASK)
    .bind(SOURCE)
    .bind(RUN)
    .bind(mode)
    .bind(OWNER)
    .bind(Uuid::new_v4())
    .bind(&policy)
    .bind(json!({"expected_workspace_fingerprint":"b".repeat(64),"expected_head_commit":null}))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO runner_commands(id,corp_id,runner_id,run_id,command_kind,payload,idempotency_key)
         VALUES($1,$2,$3,$4,'factory_verification_recovery','{}','issue183-command')",
    )
    .bind(COMMAND).bind(CORP).bind(RUNNER).bind(RUN)
    .execute(&pool).await.unwrap();
    let store = PgStore { pool };
    store
        .runner_connected(RunnerConnectInput {
            id: RUNNER.to_owned(),
            corp_id: CORP,
            hostname: "fixture".to_owned(),
            os: "windows".to_owned(),
            capabilities: json!({}),
            connection_epoch: EPOCH,
        })
        .await
        .unwrap();
    store
}

async fn state(store: &PgStore) -> Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object(
          'run',(SELECT to_jsonb(r) FROM runs r WHERE id=$1),
          'source',(SELECT to_jsonb(r) FROM runs r WHERE id=$2),
          'task',(SELECT to_jsonb(t) FROM tasks t WHERE id=$3),
          'mission',(SELECT to_jsonb(m) FROM missions m WHERE id=$4),
          'item',(SELECT to_jsonb(f) FROM factory_work_items f WHERE id=$5),
          'recovery',(SELECT to_jsonb(v) FROM factory_verification_recoveries v WHERE id=$6),
          'agent',(SELECT to_jsonb(a) FROM agents a WHERE id=$7),
          'command',(SELECT to_jsonb(c) FROM runner_commands c WHERE id=$8),
          'events',(SELECT coalesce(jsonb_agg(to_jsonb(e) ORDER BY seq),'[]') FROM events e))",
    )
    .bind(RUN)
    .bind(SOURCE)
    .bind(TASK)
    .bind(MISSION)
    .bind(ITEM)
    .bind(RECOVERY)
    .bind(AGENT)
    .bind(COMMAND)
    .fetch_one(&store.pool)
    .await
    .unwrap()
}

async fn acknowledge_and_lose(store: &PgStore) -> Vec<DomainEvent> {
    assert!(
        store
            .acknowledge_runner_command(COMMAND, RUNNER)
            .await
            .unwrap()
            .is_some()
    );
    store
        .mark_unclaimed_runner_runs_lost(RUNNER, EPOCH, &[])
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue183_acked_source_correction_loss_terminalizes_exact_recovery(pool: PgPool) {
    let store = fixture(pool, "source_correction").await;
    let before = state(&store).await;
    let events = acknowledge_and_lose(&store).await;
    let after = state(&store).await;
    assert_eq!(after["run"]["status"], "lost");
    assert_eq!(after["recovery"]["status"], "failed");
    assert_eq!(after["item"]["state"], "verification_failed");
    assert_eq!(after["item"]["version"], 4);
    assert_eq!(after["task"]["status"], "verification_failed");
    assert_eq!(after["task"]["verification_status"], "failed");
    assert_eq!(after["mission"]["status"], "failed");
    assert_eq!(after["agent"]["status"], "idle");
    assert_eq!(after["agent"]["current_run_id"], Value::Null);
    assert_eq!(after["source"], before["source"]);
    for field in [
        "workspace_path",
        "workspace_fingerprint",
        "workspace_disposition",
        "workspace_run_id",
        "resumed_from_run_id",
        "input_tokens",
        "output_tokens",
        "cost_microusd",
    ] {
        assert_eq!(after["run"][field], before["run"][field], "{field}");
    }
    assert_ne!(after["run"]["workspace_detail"], "dispatch_not_started");
    assert_eq!(
        after["task"]["attempt_count"],
        before["task"]["attempt_count"]
    );
    for field in [
        "budget_tokens",
        "budget_cost_microusd",
        "original_budget_tokens",
        "original_budget_cost_microusd",
    ] {
        assert_eq!(after["mission"][field], before["mission"][field], "{field}");
    }
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, "factory.verification_failed");
    assert_eq!(events[0].payload["cause"], "source_correction_runner_lost");
    assert_eq!(events[1].event_type, "run.lost");
    assert!(events.iter().all(|event| event.room_id == Some(ROOM)));
    assert!(events[0].seq < events[1].seq);
    assert!(
        store
            .mark_unclaimed_runner_runs_lost(RUNNER, EPOCH, &[])
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(state(&store).await, after);
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM factory_verification_recoveries WHERE factory_work_item_id=$1
         AND status IN ('authorized','running')",
    )
    .bind(ITEM)
    .fetch_one(&store.pool)
    .await
    .unwrap();
    assert_eq!(active, 0);
    // Unknown workspace state remains unknown, not an invented recoverable checkpoint.
    assert!(
        store
            .factory_verification_recovery_context(CORP, OWNER, ITEM)
            .await
            .is_err()
    );
}

fn event(kind: &str, payload: Value) -> RunnerEventInput {
    RunnerEventInput {
        event_id: Uuid::new_v4(),
        runner_id: RUNNER.to_owned(),
        corp_id: CORP,
        connection_epoch: EPOCH,
        run_id: RUN,
        agent_id: AGENT,
        assignment_token: TOKEN,
        event_type: kind.to_owned(),
        payload,
    }
}

async fn started(store: &PgStore) {
    store
        .apply_runner_event(event(
            "run.started",
            json!({
                "workspace":"owned-worktree", "workspace_branch":"crony/fixture",
                "workspace_base_ref":"main", "workspace_base_commit":"a".repeat(40)
            }),
        ))
        .await
        .unwrap();
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue183_native_cleanup_restores_exact_source_correction_context(pool: PgPool) {
    let store = fixture(pool, "source_correction").await;
    started(&store).await;
    let before = state(&store).await;
    acknowledge_and_lose(&store).await;
    assert!(
        store
            .factory_verification_recovery_context(CORP, OWNER, ITEM)
            .await
            .is_err()
    );
    assert!(
        store
            .apply_runner_event(event("run.started", json!({})))
            .await
            .is_err()
    );
    let cleanup = event(
        "run.workspace_preserved",
        json!({
            "disposition":"preserved", "detail":"native cleanup finished",
            "workspace_fingerprint":"c".repeat(64)
        }),
    );
    store.apply_runner_event(cleanup.clone()).await.unwrap();
    assert!(
        store
            .apply_runner_event(cleanup)
            .await
            .unwrap()
            .event
            .is_none()
    );
    let context = store
        .factory_verification_recovery_context(CORP, OWNER, ITEM)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(context.source_run_id, RUN);
    assert_eq!(context.workspace_fingerprint, Some("c".repeat(64)));
    assert_eq!(context.remaining_attempts, 2);
    assert_eq!(
        context.work_item.state,
        FactoryWorkItemState::VerificationFailed
    );
    assert!(
        context
            .recoveries
            .iter()
            .all(|recovery| recovery.status == "failed")
    );
    let after = state(&store).await;
    assert_eq!(after["source"], before["source"]);
    assert_eq!(after["run"]["status"], "lost");
    assert_eq!(after["run"]["workspace_fingerprint"], "c".repeat(64));
    assert_eq!(after["source"]["workspace_fingerprint"], "b".repeat(64));
    assert!(
        store
            .apply_runner_event(event(
                "run.workspace_preserved",
                json!({
                    "disposition":"preserved", "workspace_fingerprint":"d".repeat(64)
                })
            ))
            .await
            .is_err()
    );
    assert_eq!(state(&store).await, after);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue183_cleanup_after_missing_started_report_restores_bound_assignment(pool: PgPool) {
    let store = fixture(pool, "source_correction").await;
    let original = state(&store).await;
    acknowledge_and_lose(&store).await;
    store
        .apply_runner_event(event(
            "run.workspace_preserved",
            json!({
                "disposition":"preserved", "detail":"cleanup has not confirmed a fingerprint"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(state(&store).await["run"]["workspace_path"], Value::Null);
    assert!(
        store
            .factory_verification_recovery_context(CORP, OWNER, ITEM)
            .await
            .is_err()
    );
    store.apply_runner_event(event("run.workspace_preserved", json!({
        "disposition":"preserved", "detail":"native cleanup now confirms the exact assignment",
        "workspace_fingerprint":"c".repeat(64),
        "workspace":"untrusted-cleanup-path-that-must-not-be-adopted"
    }))).await.unwrap();
    let context = store
        .factory_verification_recovery_context(CORP, OWNER, ITEM)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(context.source_run_id, RUN);
    assert_eq!(context.workspace_fingerprint, Some("c".repeat(64)));
    let after = state(&store).await;
    assert_eq!(after["source"], original["source"]);
    assert_eq!(after["run"]["status"], "lost");
    for field in [
        "workspace_path",
        "workspace_branch",
        "workspace_base_ref",
        "workspace_base_commit",
    ] {
        assert_eq!(after["run"][field], original["source"][field], "{field}");
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue183_source_correction_grace_expiry_is_atomic_and_epoch_fenced(pool: PgPool) {
    let store = fixture(pool, "source_correction").await;
    store
        .acknowledge_runner_command(COMMAND, RUNNER)
        .await
        .unwrap();
    assert!(
        !store
            .runner_disconnected(RUNNER, EPOCH, 1)
            .await
            .unwrap()
            .is_empty()
    );
    let before = state(&store).await;
    assert!(
        store
            .expire_runner_grace(RUNNER, Uuid::new_v4())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(state(&store).await, before);
    sqlx::query("SELECT pg_sleep(1.1)")
        .execute(&store.pool)
        .await
        .unwrap();
    let events = store.expire_runner_grace(RUNNER, EPOCH).await.unwrap();
    assert_eq!(events.len(), 2);
    let after = state(&store).await;
    assert_eq!(after["run"]["status"], "lost");
    assert_eq!(after["recovery"]["status"], "failed");
    assert_eq!(after["item"]["state"], "verification_failed");
    assert!(
        store
            .expire_runner_grace(RUNNER, EPOCH)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(state(&store).await, after);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue183_pending_command_and_accepted_claim_are_not_lost(pool: PgPool) {
    let store = fixture(pool, "source_correction").await;
    let before = state(&store).await;
    assert!(
        store
            .mark_unclaimed_runner_runs_lost(RUNNER, EPOCH, &[])
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(state(&store).await, before);
    store
        .acknowledge_runner_command(COMMAND, RUNNER)
        .await
        .unwrap();
    let acknowledged = state(&store).await;
    assert!(
        store
            .mark_unclaimed_runner_runs_lost(RUNNER, EPOCH, &[RUN])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .mark_unclaimed_runner_runs_lost(RUNNER, Uuid::new_v4(), &[])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .mark_unclaimed_runner_runs_lost("other-runner", EPOCH, &[])
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(state(&store).await, acknowledged);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue183_verifier_only_loss_keeps_original_cleanup_seal(pool: PgPool) {
    let store = fixture(pool, "verifier_only").await;
    started(&store).await;
    let events = acknowledge_and_lose(&store).await;
    assert_eq!(events[0].payload["cause"], "verifier_runner_lost");
    assert!(
        store
            .apply_runner_event(event(
                "run.workspace_preserved",
                json!({
                    "disposition":"preserved", "workspace_fingerprint":"c".repeat(64)
                })
            ))
            .await
            .is_err()
    );
    store
        .apply_runner_event(event(
            "run.workspace_preserved",
            json!({
                "disposition":"preserved", "workspace_fingerprint":"b".repeat(64)
            }),
        ))
        .await
        .unwrap();
    let context = store
        .factory_verification_recovery_context(CORP, OWNER, ITEM)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(context.source_run_id, RUN);
    assert_eq!(context.workspace_fingerprint, Some("b".repeat(64)));
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue183_ordinary_provider_loss_is_not_a_verification_recovery(pool: PgPool) {
    let store = fixture(pool, "source_correction").await;
    sqlx::query("DELETE FROM factory_verification_recoveries WHERE id=$1")
        .bind(RECOVERY)
        .execute(&store.pool)
        .await
        .unwrap();
    let events = acknowledge_and_lose(&store).await;
    assert_eq!(events.len(), 1);
    let after = state(&store).await;
    assert_eq!(after["run"]["status"], "lost");
    assert_eq!(after["task"]["status"], "blocked");
    assert_eq!(after["item"]["state"], "running");
    store
        .apply_runner_event(event(
            "run.workspace_preserved",
            json!({
                "disposition":"preserved", "workspace_fingerprint":"c".repeat(64)
            }),
        ))
        .await
        .unwrap();
    assert_eq!(state(&store).await["run"]["workspace_path"], Value::Null);
    let mut tx = store.pool.begin().await.unwrap();
    let checkpoint = source_workspace_checkpoint_tx(&mut tx, CORP, RUN)
        .await
        .unwrap();
    assert!(checkpoint.ensure_factory_terminal().is_err());
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue183_foreign_and_replaced_recovery_bindings_are_not_adopted(pool: PgPool) {
    let store = fixture(pool, "source_correction").await;
    started(&store).await;
    acknowledge_and_lose(&store).await;
    store
        .apply_runner_event(event(
            "run.workspace_preserved",
            json!({
                "disposition":"preserved", "workspace_fingerprint":"c".repeat(64)
            }),
        ))
        .await
        .unwrap();
    let original = state(&store).await;
    sqlx::query("UPDATE runs SET source_base_commit=$1 WHERE id=$2")
        .bind("f".repeat(40))
        .bind(RUN)
        .execute(&store.pool)
        .await
        .unwrap();
    assert!(
        store
            .factory_verification_recovery_context(CORP, OWNER, ITEM)
            .await
            .is_err()
    );
    sqlx::query("UPDATE runs SET source_base_commit=$1 WHERE id=$2")
        .bind("a".repeat(40))
        .bind(RUN)
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE factory_verification_recoveries SET status='completed' WHERE id=$1")
        .bind(RECOVERY)
        .execute(&store.pool)
        .await
        .unwrap();
    assert!(
        store
            .factory_verification_recovery_context(CORP, OWNER, ITEM)
            .await
            .is_err()
    );
    sqlx::query("UPDATE factory_verification_recoveries SET status='failed' WHERE id=$1")
        .bind(RECOVERY)
        .execute(&store.pool)
        .await
        .unwrap();
    assert_eq!(state(&store).await["source"], original["source"]);
    let before = state(&store).await;
    let mut foreign = event(
        "run.workspace_preserved",
        json!({
            "disposition":"preserved", "workspace_fingerprint":"c".repeat(64)
        }),
    );
    foreign.assignment_token = Uuid::new_v4();
    assert!(store.apply_runner_event(foreign).await.is_err());
    assert_eq!(state(&store).await, before);
    assert!(
        store
            .factory_verification_recovery_context(Uuid::new_v4(), OWNER, ITEM)
            .await
            .is_err()
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue183_loss_rolls_back_when_factory_transition_cannot_commit(pool: PgPool) {
    let store = fixture(pool, "source_correction").await;
    store
        .acknowledge_runner_command(COMMAND, RUNNER)
        .await
        .unwrap();
    sqlx::raw_sql(
        "CREATE FUNCTION issue183_reject_transition() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'owned issue183 rollback fixture'; END; $$;
         CREATE TRIGGER issue183_reject_transition BEFORE UPDATE ON factory_work_items
         FOR EACH ROW EXECUTE FUNCTION issue183_reject_transition();",
    )
    .execute(&store.pool)
    .await
    .unwrap();
    let before = state(&store).await;
    assert!(
        store
            .mark_unclaimed_runner_runs_lost(RUNNER, EPOCH, &[])
            .await
            .is_err()
    );
    assert_eq!(state(&store).await, before);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue183_terminal_factory_outcomes_are_not_downgraded(pool: PgPool) {
    let store = fixture(pool, "source_correction").await;
    store
        .acknowledge_runner_command(COMMAND, RUNNER)
        .await
        .unwrap();
    for status in ["verified", "published", "cancelled", "failed"] {
        sqlx::query("UPDATE factory_work_items SET state=$1 WHERE id=$2")
            .bind(status)
            .bind(ITEM)
            .execute(&store.pool)
            .await
            .unwrap();
        let before = state(&store).await;
        assert!(
            store
                .mark_unclaimed_runner_runs_lost(RUNNER, EPOCH, &[])
                .await
                .is_err()
        );
        assert_eq!(state(&store).await, before, "{status}");
    }
}

async fn gate_precedes_rows(pool: PgPool, prefix: &str) {
    use std::future::Future;
    use std::task::Poll;
    let store = fixture(pool, "source_correction").await;
    store
        .acknowledge_runner_command(COMMAND, RUNNER)
        .await
        .unwrap();
    let mut blocker = store.pool.begin().await.unwrap();
    lock_factory_keys_tx(&mut blocker, &[format!("{prefix}:{CORP}:{ITEM}")])
        .await
        .unwrap();
    sqlx::query("SELECT id FROM factory_work_items WHERE id=$1 FOR UPDATE")
        .bind(ITEM)
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    let mut loss = Box::pin(store.mark_unclaimed_runner_runs_lost(RUNNER, EPOCH, &[]));
    let pool = store.pool.clone();
    let mut probe = Box::pin(async move {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let waiting: Option<String> = sqlx::query_scalar(
                "SELECT query FROM pg_stat_activity WHERE datname=current_database()
                 AND pid<>pg_backend_pid() AND wait_event_type='Lock'
                 ORDER BY query_start LIMIT 1",
            )
            .fetch_optional(&pool)
            .await?;
            if let Some(query) = waiting {
                if !query.contains("pg_advisory_xact_lock") {
                    return Err(anyhow!("loss took lifecycle rows before its Factory gate"));
                }
                sqlx::query("SELECT id FROM runs WHERE id=$1 FOR UPDATE NOWAIT")
                    .bind(RUN)
                    .fetch_one(&mut *blocker)
                    .await?;
                sqlx::query("SELECT id FROM missions WHERE id=$1 FOR UPDATE NOWAIT")
                    .bind(MISSION)
                    .fetch_one(&mut *blocker)
                    .await?;
                blocker.commit().await?;
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(anyhow!("loss never reached the native Factory gate"));
            }
        }
    });
    let mut finished = None;
    std::future::poll_fn(|context| {
        if finished.is_none()
            && let Poll::Ready(result) = loss.as_mut().poll(context)
        {
            finished = Some(result);
        }
        probe.as_mut().poll(context)
    })
    .await
    .unwrap();
    let events = match finished {
        Some(result) => result,
        None => loss.await,
    }
    .unwrap();
    assert_eq!(events.len(), 2);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue183_item_gate_precedes_loss_rows(pool: PgPool) {
    gate_precedes_rows(pool, "factory:item").await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue183_publication_gate_precedes_loss_rows(pool: PgPool) {
    gate_precedes_rows(pool, "publication:factory").await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue183_next_native_recovery_keeps_the_cleaned_lost_source_binding(pool: PgPool) {
    let store = fixture(pool, "source_correction").await;
    let policy = VerificationPolicy {
        checks: vec![VerifierCheck::File {
            path: "result.md".to_owned(),
            min_bytes: 1,
        }],
        manual_gate: Some(ManualVerificationGate::IndependentReview {
            roles: vec!["member".to_owned()],
            exclude_requester: true,
        }),
    };
    // Valid persisted admission preconditions for this API-level store case.
    // These are established before starting/loss, not by rewriting failed work.
    sqlx::query(
        "UPDATE tasks SET required_adapter='openai-codex',verification_policy=$1 WHERE id=$2",
    )
    .bind(serde_json::to_value(&policy).unwrap())
    .bind(TASK)
    .execute(&store.pool)
    .await
    .unwrap();
    sqlx::query("UPDATE factory_work_items SET policy=$1 WHERE id=$2")
        .bind(json!({
            "schema_version":1, "source_of_truth":"github_project", "auto_merge":false,
            "repository_allowlist":["fixture/source"], "source_base_ref":"main",
            "source_base_commit":"a".repeat(40), "adapter_allowlist":["openai-codex"],
            "strategy_allowlist":["single"], "model":null, "reasoning_effort":null,
            "write_scope":["result.md"], "allowed_tools":["filesystem"],
            "prohibited_actions":["outside worktree"], "secret_ids":[],
            "verification_required":true, "budget_tokens":1000000,
            "budget_cost_microusd":5000000
        }))
        .bind(ITEM)
        .execute(&store.pool)
        .await
        .unwrap();
    let original = state(&store).await;
    // No run.started report: cleanup must recover the persisted assignment.
    acknowledge_and_lose(&store).await;
    store
        .apply_runner_event(event(
            "run.workspace_preserved",
            json!({
                "disposition":"preserved", "detail":"native cleanup finished",
                "workspace_fingerprint":"c".repeat(64)
            }),
        ))
        .await
        .unwrap();
    let lost = state(&store).await;
    let contract_value: Value = sqlx::query_scalar("SELECT contract FROM tasks WHERE id=$1")
        .bind(TASK)
        .fetch_one(&store.pool)
        .await
        .unwrap();
    let revision = store
        .create_mission_contract_revision(CreateMissionContractRevisionInput {
            corp_id: CORP,
            mission_id: MISSION,
            task_id: TASK,
            actor_id: OWNER,
            expected_contract_version: 1,
            next_action: MissionContractRevisionAction::Resume,
            source_run_id: Some(RUN),
            reason: "Continue the confirmed native checkpoint".to_owned(),
            idempotency_key: Uuid::new_v4(),
            description: "Preserve and verify result.md.".to_owned(),
            contract: serde_json::from_value(contract_value).unwrap(),
            verification_policy: policy,
        })
        .await
        .unwrap();
    let request = CreateFactoryVerificationRecoveryInput {
        corp_id: CORP,
        work_item_id: ITEM,
        actor_id: OWNER,
        claim_token: Uuid::from_u128(15),
        expected_factory_version: 4,
        idempotency_key: Uuid::new_v4(),
        source_run_id: RUN,
        mode: FactoryVerificationRecoveryMode::SourceCorrection,
        reason: "Resume the runner-confirmed checkpoint without resetting work".to_owned(),
        observed_source_revision: "revision-1".to_owned(),
        reviewed_source_snapshot: json!({"source_revision":"revision-1","issue_number":183}),
        contract_revision_id: Some(revision.revision.id),
        expected_workspace_fingerprint: "c".repeat(64),
        expected_head_commit: None,
    };
    let outcome = store
        .create_factory_verification_recovery(request.clone())
        .await
        .unwrap();
    let next_run = outcome.recovery.replacement_run_id.unwrap();
    assert_ne!(next_run, RUN);
    assert_eq!(outcome.recovery.source_run_id, RUN);
    assert_eq!(outcome.work_item.state, FactoryWorkItemState::Running);
    let replay = store
        .create_factory_verification_recovery(request)
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.recovery.replacement_run_id, Some(next_run));
    let next: Value = sqlx::query_scalar("SELECT to_jsonb(r) FROM runs r WHERE id=$1")
        .bind(next_run)
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(next["resumed_from_run_id"], RUN.to_string());
    assert_eq!(next["workspace_run_id"], SOURCE.to_string());
    assert_eq!(next["provider_session_id"], "fixture-session");
    assert_eq!(next["source_repository"], "fixture/source");
    assert_eq!(next["source_base_ref"], "main");
    assert_eq!(next["source_base_commit"], "a".repeat(40));
    let commands: Vec<Value> = sqlx::query_scalar(
        "SELECT payload FROM runner_commands WHERE run_id=$1 AND command_kind='factory_verification_recovery'",
    ).bind(next_run).fetch_all(&store.pool).await.unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0]["source_run_id"], RUN.to_string());
    assert_eq!(commands[0]["workspace_run_id"], SOURCE.to_string());
    assert_eq!(
        commands[0]["expected_workspace_fingerprint"],
        "c".repeat(64)
    );
    assert_eq!(commands[0]["provider_session_id"], "fixture-session");
    let after = state(&store).await;
    assert_eq!(after["source"], original["source"]);
    assert_eq!(after["run"], lost["run"]);
    assert_eq!(after["recovery"]["status"], "failed");
    assert_eq!(after["task"]["attempt_count"], 3);
    for field in [
        "budget_tokens",
        "budget_cost_microusd",
        "original_budget_tokens",
        "original_budget_cost_microusd",
    ] {
        assert_eq!(
            after["mission"][field], original["mission"][field],
            "{field}"
        );
    }
}
