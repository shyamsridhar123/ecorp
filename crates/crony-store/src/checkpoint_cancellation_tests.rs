//! New #206 cases reuse the native stopped-source fixture but never execute the
//! older ignored test functions. All SQL belongs to SQLx disposable databases.
use super::*;
use crate::ReconcileCheckpointCancellationInput;

async fn controller_cancel(store: &PgStore, native_key: bool, detail: Option<&str>) -> Uuid {
    let current = state(store).await;
    let version = current["item"]["version"].as_i64().unwrap();
    let key = if native_key {
        format!("github-project:fixture:148:item148:revision-1:transition:cancelled:{version}")
    } else {
        "explicit-operator-cancellation".to_owned()
    };
    let result = store
        .transition_factory_work_item(TransitionFactoryWorkItemInput {
            corp_id: CORP,
            work_item_id: ITEM,
            actor_id: OWNER,
            claim_token: CLAIM,
            expected_version: version,
            idempotency_key: key,
            state: FactoryWorkItemState::Cancelled,
            failure_detail: detail.map(str::to_owned),
        })
        .await
        .unwrap();
    result.event.unwrap().id
}

async fn reconciliation(store: &PgStore, event_id: Uuid) -> ReconcileCheckpointCancellationInput {
    let snapshot = state(store).await;
    ReconcileCheckpointCancellationInput {
        corp_id: CORP,
        work_item_id: ITEM,
        actor_id: OWNER,
        source_run_id: SOURCE,
        expected_factory_version: snapshot["item"]["version"].as_i64().unwrap(),
        cancellation_event_id: event_id,
        expected_workspace_fingerprint: "b".repeat(64),
        expected_head_commit: "a".repeat(40),
        observed_source_revision: "revision-1".to_owned(),
        idempotency_key: "issue206-reconcile".to_owned(),
        reason: "Verify the retained source; do not run another model".to_owned(),
    }
}

async fn rejected(store: &PgStore, input: ReconcileCheckpointCancellationInput) {
    let before = state(store).await;
    assert!(
        store
            .reconcile_checkpoint_cancellation(input)
            .await
            .is_err()
    );
    assert_eq!(state(store).await, before);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue206_reconcile_native_projection_then_verify_same_source(pool: PgPool) {
    let store = fixture(pool, false).await;
    let event_id = controller_cancel(&store, true, None).await;
    let original = state(&store).await;
    let context = store
        .factory_verification_recovery_context(CORP, OWNER, ITEM)
        .await
        .unwrap()
        .unwrap();
    assert!(context.checkpoint_verification);
    assert_eq!(context.checkpoint_cancellation_event_id, Some(event_id));
    assert_eq!(
        state(&store).await,
        original,
        "context must remain read-only"
    );
    let input = reconciliation(&store, event_id).await;
    let outcome = store
        .reconcile_checkpoint_cancellation(input.clone())
        .await
        .unwrap();
    assert_eq!(outcome.work_item.state, FactoryWorkItemState::Running);
    assert_eq!(
        outcome.work_item.version,
        input.expected_factory_version + 1
    );
    assert_eq!(outcome.claim_token, None);
    let event = outcome.event.unwrap();
    assert_eq!(
        event.event_type,
        "factory.checkpoint_cancellation_reconciled"
    );
    assert_eq!(event.room_id, Some(ROOM));
    assert_eq!(event.causation_id, Some(event_id));
    let after = state(&store).await;
    for key in [
        "source",
        "runs",
        "task",
        "tasks",
        "mission",
        "missions",
        "agents",
        "actors",
        "memberships",
        "commands",
        "recoveries",
        "artifacts",
        "deliverables",
        "verification_evidence",
        "verification_requests",
    ] {
        assert_eq!(after[key], original[key], "reconciliation changed {key}");
    }
    let replay = store
        .reconcile_checkpoint_cancellation(input.clone())
        .await
        .unwrap();
    assert!(replay.replayed);
    assert!(replay.event.is_none());
    assert_eq!(state(&store).await, after);
    let mut changed = input;
    changed.reason = "a different request".to_owned();
    rejected(&store, changed).await;

    let mut verify_request = request();
    verify_request.expected_factory_version = outcome.work_item.version;
    let recovery = store
        .create_factory_verification_recovery(verify_request)
        .await
        .unwrap();
    assert_eq!(recovery.recovery.source_run_id, SOURCE);
    assert_eq!(
        recovery.recovery.mode,
        FactoryVerificationRecoveryMode::CheckpointVerification
    );
    assert!(matches!(
        recovery.launch,
        FactoryVerificationRecoveryLaunch::VerifierOnly(_)
    ));
    assert_eq!(state(&store).await["source"], original["source"]);
    assert_eq!(
        state(&store).await["mission"]["original_budget_tokens"],
        original["mission"]["original_budget_tokens"]
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue206_explicit_operator_cancellation_is_not_reconciled(pool: PgPool) {
    let store = fixture(pool, false).await;
    let event_id = controller_cancel(&store, false, None).await;
    let context = store
        .factory_verification_recovery_context(CORP, OWNER, ITEM)
        .await
        .unwrap()
        .unwrap();
    assert!(context.checkpoint_cancellation_event_id.is_none());
    rejected(&store, reconciliation(&store, event_id).await).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue206_policy_cancellation_detail_remains_protected(pool: PgPool) {
    let store = fixture(pool, false).await;
    let event_id = controller_cancel(
        &store,
        true,
        Some("Operator withdrew publication authority"),
    )
    .await;
    rejected(&store, reconciliation(&store, event_id).await).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue206_native_explicit_stop_cannot_be_overridden(pool: PgPool) {
    let store = fixture_with_stop_request(pool, false, true).await;
    let event_id = controller_cancel(&store, true, None).await;
    rejected(&store, reconciliation(&store, event_id).await).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue206_reconciliation_binds_version_event_revision_and_checkpoint(pool: PgPool) {
    let store = fixture(pool, false).await;
    let event_id = controller_cancel(&store, true, None).await;
    let input = reconciliation(&store, event_id).await;
    let mut cases = vec![];
    let mut changed = input.clone();
    changed.expected_factory_version += 1;
    cases.push(changed);
    let mut changed = input.clone();
    changed.cancellation_event_id = Uuid::new_v4();
    cases.push(changed);
    let mut changed = input.clone();
    changed.observed_source_revision = "changed".to_owned();
    cases.push(changed);
    let mut changed = input.clone();
    changed.expected_workspace_fingerprint = "c".repeat(64);
    cases.push(changed);
    let mut changed = input.clone();
    changed.expected_head_commit = "c".repeat(40);
    cases.push(changed);
    let mut changed = input.clone();
    changed.source_run_id = Uuid::new_v4();
    cases.push(changed);
    let mut changed = input.clone();
    changed.work_item_id = Uuid::new_v4();
    cases.push(changed);
    let mut changed = input.clone();
    changed.corp_id = Uuid::new_v4();
    cases.push(changed);
    let mut changed = input;
    changed.reason.clear();
    cases.push(changed);
    for input in cases {
        rejected(&store, input).await;
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue206_reconciliation_and_replay_require_current_recovery_authority(pool: PgPool) {
    let store = fixture(pool, false).await;
    let event_id = controller_cancel(&store, true, None).await;
    let input = reconciliation(&store, event_id).await;
    let mut member = input.clone();
    member.actor_id = REVIEWER;
    rejected(&store, member).await;
    let mut unknown = input.clone();
    unknown.actor_id = Uuid::new_v4();
    rejected(&store, unknown).await;
    store
        .reconcile_checkpoint_cancellation(input.clone())
        .await
        .unwrap();
    sqlx::query("DELETE FROM room_memberships WHERE room_id=$1 AND actor_id=$2")
        .bind(ROOM)
        .bind(OWNER)
        .execute(&store.pool)
        .await
        .unwrap();
    rejected(&store, input).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue206_missing_termination_or_native_budget_proof_never_reconciles(pool: PgPool) {
    let store = fixture(pool, false).await;
    let event_id = controller_cancel(&store, true, None).await;
    sqlx::query("UPDATE events SET type='fixture.removed-termination' WHERE aggregate_id=$1 AND type='run.session_terminated'")
        .bind(SOURCE).execute(&store.pool).await.unwrap();
    rejected(&store, reconciliation(&store, event_id).await).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue206_newer_factory_event_rejects_stale_projection_proof(pool: PgPool) {
    let store = fixture(pool, false).await;
    let event_id = controller_cancel(&store, true, None).await;
    let mut tx = store.pool.begin().await.unwrap();
    append_event_tx(
        &mut tx,
        NewEvent::new(
            CORP,
            Some(OWNER),
            "factory.policy_changed",
            "factory_work_item",
            ITEM,
            "issue206-unrelated-policy-event",
            json!({"reason":"different current policy"}),
        ),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    rejected(&store, reconciliation(&store, event_id).await).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue206_late_journal_failure_rolls_back_reconciliation(pool: PgPool) {
    let store = fixture(pool, false).await;
    let event_id = controller_cancel(&store, true, None).await;
    sqlx::raw_sql(
        "CREATE FUNCTION issue206_fail_journal() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN IF NEW.type='factory.checkpoint_cancellation_reconciled' THEN
           RAISE EXCEPTION 'issue206 fixture journal failure'; END IF; RETURN NEW; END $$;
         CREATE TRIGGER issue206_fail_journal BEFORE INSERT ON events
         FOR EACH ROW EXECUTE FUNCTION issue206_fail_journal();",
    )
    .execute(&store.pool)
    .await
    .unwrap();
    rejected(&store, reconciliation(&store, event_id).await).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue206_reconciliation_serializes_actor_demotion(pool: PgPool) {
    use std::future::Future;
    use std::task::Poll;

    let store = fixture(pool, false).await;
    let event_id = controller_cancel(&store, true, None).await;
    let input = reconciliation(&store, event_id).await;
    let mut blocker = store.pool.begin().await.unwrap();
    let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM runs WHERE corp_id=$1 AND id=$2 FOR UPDATE")
        .bind(CORP)
        .bind(SOURCE)
        .fetch_one(&mut *blocker)
        .await
        .unwrap();

    let mut reconcile = Box::pin(store.reconcile_checkpoint_cancellation(input.clone()));
    let probe_pool = store.pool.clone();
    let mut probe = Box::pin(async move {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            // Observe the actual source-row wait, after actor authorization.
            // A sleep or merely observing an unfinished future is not proof.
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(
                   SELECT 1 FROM pg_stat_activity activity
                   WHERE activity.datname=current_database()
                     AND activity.wait_event_type='Lock'
                     AND $1=ANY(pg_blocking_pids(activity.pid))
                     AND activity.query LIKE '%FROM runs run%'
                     AND activity.query LIKE '%FOR UPDATE OF run%')",
            )
            .bind(blocker_pid)
            .fetch_one(&probe_pool)
            .await?;
            if waiting {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err(anyhow::anyhow!(
                    "reconciliation never reached the held source-row lock"
                ));
            }
        }

        let mut demotion = probe_pool.begin().await?;
        // Updating a non-key role column takes NO KEY UPDATE. NOWAIT proves
        // incompatibility with the retained actor SHARE lock without a timeout.
        let role_lock = sqlx::query(
            "SELECT id FROM actors WHERE corp_id=$1 AND id=$2 FOR NO KEY UPDATE NOWAIT",
        )
        .bind(CORP)
        .bind(OWNER)
        .fetch_one(&mut *demotion)
        .await;
        demotion.rollback().await?;
        match role_lock {
            Err(sqlx::Error::Database(error)) if error.code().as_deref() == Some("55P03") => {}
            Err(error) => return Err(error.into()),
            Ok(_) => {
                return Err(anyhow::anyhow!(
                    "actor demotion was not fenced while reconciliation waited on its source"
                ));
            }
        }
        blocker.rollback().await?;
        Ok::<(), anyhow::Error>(())
    });
    let mut finished = None;
    std::future::poll_fn(|context| {
        if finished.is_none()
            && let Poll::Ready(result) = reconcile.as_mut().poll(context)
        {
            finished = Some(result);
        }
        probe.as_mut().poll(context)
    })
    .await
    .unwrap();
    let outcome = match finished {
        Some(result) => result,
        None => reconcile.await,
    }
    .unwrap();
    assert_eq!(outcome.work_item.state, FactoryWorkItemState::Running);
    assert_eq!(
        outcome.work_item.version,
        input.expected_factory_version + 1
    );
    assert_eq!(outcome.claim_token, None);

    // Commit releases the fence: the same lock and an actual demotion now work.
    let mut demotion = store.pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM actors WHERE corp_id=$1 AND id=$2 FOR NO KEY UPDATE NOWAIT")
        .bind(CORP)
        .bind(OWNER)
        .fetch_one(&mut *demotion)
        .await
        .unwrap();
    let updated = sqlx::query("UPDATE actors SET role='member' WHERE corp_id=$1 AND id=$2")
        .bind(CORP)
        .bind(OWNER)
        .execute(&mut *demotion)
        .await
        .unwrap();
    assert_eq!(updated.rows_affected(), 1);
    demotion.commit().await.unwrap();

    let before_replay = state(&store).await;
    let error = store
        .reconcile_checkpoint_cancellation(input)
        .await
        .expect_err("a demoted actor must not replay reconciliation");
    assert!(
        error
            .to_string()
            .contains("requires owner, admin, or manager authority"),
        "{error:#}"
    );
    assert_eq!(state(&store).await, before_replay);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue206_quarantined_predecessor_rejects_valid_latest_source(pool: PgPool) {
    let store = fixture(pool, false).await;
    let event_id = controller_cancel(&store, true, None).await;
    let input = reconciliation(&store, event_id).await;
    let original_source = state(&store).await["source"].clone();
    let predecessor_id = Uuid::new_v4();
    let inserted = sqlx::query(
        "INSERT INTO runs(
           id,corp_id,task_id,agent_id,runner_id,assignment_token,status,execution_mode,
           breaker_stage,workspace_run_id,workspace_path,workspace_branch,workspace_base_ref,
           workspace_base_commit,workspace_disposition,workspace_fingerprint,
           source_repository,source_base_ref,source_base_commit,created_at,updated_at)
         SELECT $1,corp_id,task_id,agent_id,runner_id,$2,'cancelled',execution_mode,
           'suspend',workspace_run_id,workspace_path,workspace_branch,workspace_base_ref,
           workspace_base_commit,'preserved',workspace_fingerprint,
           source_repository,source_base_ref,source_base_commit,
           created_at-interval '1 second',created_at-interval '1 second'
         FROM runs WHERE corp_id=$3 AND id=$4",
    )
    .bind(predecessor_id)
    .bind(Uuid::new_v4())
    .bind(CORP)
    .bind(SOURCE)
    .execute(&store.pool)
    .await
    .unwrap();
    assert_eq!(inserted.rows_affected(), 1);

    // Establish that this older same-workspace row is otherwise admissible.
    let mut tx = store.pool.begin().await.unwrap();
    let lineage = workspace_lineage_tx(&mut tx, CORP, SOURCE).await.unwrap();
    assert_eq!(lineage.len(), 2);
    assert_eq!(latest_workspace_source_id(&lineage).unwrap(), SOURCE);
    let authority = crate::budget_checkpoint::source_authority_tx(&mut tx, CORP, ITEM, SOURCE)
        .await
        .unwrap();
    crate::budget_checkpoint::validate_lineage_tx(&mut tx, CORP, ITEM, SOURCE, &authority)
        .await
        .unwrap();
    tx.rollback().await.unwrap();

    let updated = sqlx::query(
        "UPDATE runs SET workspace_disposition='quarantined' WHERE corp_id=$1 AND id=$2",
    )
    .bind(CORP)
    .bind(predecessor_id)
    .execute(&store.pool)
    .await
    .unwrap();
    assert_eq!(updated.rows_affected(), 1);
    let before = state(&store).await;
    assert_eq!(before["source"], original_source);

    // Quarantine only the predecessor: neither latest selection nor the complete
    // native stopped-source proof may be the reason reconciliation is rejected.
    let mut tx = store.pool.begin().await.unwrap();
    let lineage = workspace_lineage_tx(&mut tx, CORP, SOURCE).await.unwrap();
    assert_eq!(latest_workspace_source_id(&lineage).unwrap(), SOURCE);
    let current_authority =
        crate::budget_checkpoint::source_authority_tx(&mut tx, CORP, ITEM, SOURCE)
            .await
            .unwrap();
    assert_eq!(
        serde_json::to_value(&current_authority).unwrap(),
        serde_json::to_value(&authority).unwrap()
    );
    tx.rollback().await.unwrap();

    let error = store
        .reconcile_checkpoint_cancellation(input)
        .await
        .expect_err("quarantined predecessor must prevent reconciliation");
    assert_eq!(
        error.to_string(),
        "checkpoint lineage contains active, changed, quarantined or stopped work"
    );
    assert_eq!(state(&store).await, before);
}
