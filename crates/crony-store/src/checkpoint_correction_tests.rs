//! Focused #210 actual-store regressions in SQLx-owned disposable databases.
//! These exercise persisted methods and metadata, not a real provider, physical
//! source verification, signed artifact bytes, browser acceptance or publication.
use super::*;

#[path = "checkpoint_correction_retry_tests.rs"]
mod correction_retry;

const CONNECTION: Uuid = Uuid::from_u128(210);

fn correction_profile() -> CheckpointFixtureProfile {
    CheckpointFixtureProfile {
        mission_tokens: 10_000,
        run_tokens: 5_000,
        used_tokens: 5_300,
        attempt_count: 1,
        expected_stage: "suspend",
        workspace_connection_id: Some(CONNECTION),
        ..CheckpointFixtureProfile::default()
    }
}

async fn failed_checkpoint(store: &PgStore) -> Uuid {
    let attempts_before = state(store).await["task"]["attempt_count"].clone();
    let checkpoint = store
        .create_factory_verification_recovery(request())
        .await
        .expect("native measured suspension must admit checkpoint verification");
    let FactoryVerificationRecoveryLaunch::VerifierOnly(launch) = checkpoint.launch else {
        panic!("checkpoint verification must not start another provider");
    };
    assert_eq!(launch.workspace_connection_id, Some(CONNECTION));
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
            json!({"evidence_id":Uuid::new_v4(),"check_index":0,"kind":"file",
                "status":"passed","summary":"Fixture source-file check passed","payload":{}}),
        ),
    ] {
        store
            .apply_runner_event(event(launch.run_id, launch.assignment_token, kind, payload))
            .await
            .unwrap();
    }
    // The required artifact cannot be claimed as passing when none was stored.
    let before = state(store).await;
    let fabricated = store
        .apply_runner_event(event(
            launch.run_id,
            launch.assignment_token,
            "run.verification_evidence",
            json!({"evidence_id":Uuid::new_v4(),"check_index":1,"kind":"artifact",
                "status":"passed","summary":"Missing artifact cannot pass","payload":{}}),
        ))
        .await;
    assert!(fabricated.is_err());
    assert_eq!(state(store).await, before);
    for (kind, payload) in [
        (
            "run.verification_evidence",
            json!({"evidence_id":Uuid::new_v4(),"check_index":1,"kind":"artifact",
                "status":"failed","summary":"Required provider artifact is missing","payload":{}}),
        ),
        (
            "run.verification_failed",
            json!({"error":"Required provider artifact is missing"}),
        ),
        (
            "run.workspace_preserved",
            json!({"workspace_fingerprint":"b".repeat(64),"head_commit":"a".repeat(40),
                "detail":"Native verifier retained the unchanged source"}),
        ),
    ] {
        store
            .apply_runner_event(event(launch.run_id, launch.assignment_token, kind, payload))
            .await
            .unwrap();
    }
    let failed = state(store).await;
    assert_eq!(failed["item"]["state"], "verification_failed");
    assert_eq!(failed["mission"]["status"], "failed");
    assert_eq!(failed["task"]["verification_status"], "failed");
    assert_eq!(failed["task"]["attempt_count"], attempts_before);
    let child = retained_run(&failed, launch.run_id);
    assert_eq!(child["status"], "failed");
    assert_eq!(child["verification_status"], "failed");
    assert_eq!(child["execution_mode"], "verification_only");
    assert_eq!(child["workspace_disposition"], "preserved");
    assert_eq!(child["budget_tokens_limit"], 0);
    assert_eq!(child["budget_cost_microusd_limit"], 0);
    assert!(child["provider_session_id"].is_null());
    assert!(child["artifact_id"].is_null());
    assert_eq!(failed["recoveries"][0]["status"], "failed");
    launch.run_id
}

async fn correction_request(
    store: &PgStore,
    source_run_id: Uuid,
) -> CreateFactoryVerificationRecoveryInput {
    let before = state(store).await;
    let mut contract: TaskContract =
        serde_json::from_value(before["task"]["contract"].clone()).unwrap();
    contract.objective =
        "Correct result.md and create the genuine missing provider artifact".to_owned();
    let revision = store
        .create_mission_contract_revision(CreateMissionContractRevisionInput {
            corp_id: CORP,
            mission_id: MISSION,
            task_id: TASK,
            actor_id: OWNER,
            expected_contract_version: before["task"]["contract_version"].as_i64().unwrap(),
            next_action: MissionContractRevisionAction::Resume,
            source_run_id: Some(source_run_id),
            reason: "Explicit correction after the native artifact check failed".to_owned(),
            idempotency_key: Uuid::new_v4(),
            description: "Correct the retained application under unchanged source and policy"
                .to_owned(),
            contract,
            verification_policy: serde_json::from_value(
                before["task"]["verification_policy"].clone(),
            )
            .unwrap(),
        })
        .await
        .expect("current narrow resume contract revision must be accepted");
    let after = state(store).await;
    assert_eq!(after["runs"], before["runs"]);
    assert_eq!(
        after["task"]["attempt_count"],
        before["task"]["attempt_count"]
    );
    assert_eq!(
        after["task"]["verification_policy"],
        before["task"]["verification_policy"]
    );
    let mut input = request();
    input.source_run_id = source_run_id;
    input.mode = FactoryVerificationRecoveryMode::SourceCorrection;
    input.expected_factory_version = after["item"]["version"].as_i64().unwrap();
    input.contract_revision_id = Some(revision.revision.id);
    input.reason = "Resume the retained provider to correct the failed checkpoint".to_owned();
    input.observed_source_revision = "revision-2".to_owned();
    input.reviewed_source_snapshot = json!({"source_revision":"revision-2","issue_number":148});
    input
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue210_initial_correction_resumes_after_failed_checkpoint(pool: PgPool) {
    let store = fixture_with_profile(pool, true, false, None, false, correction_profile()).await;
    let origin = state(&store).await;
    assert_eq!(origin["mission"]["budget_tokens"], 10_000);
    assert_eq!(origin["mission"]["original_budget_tokens"], 10_000);
    assert_eq!(origin["source"]["budget_tokens_limit"], 5_000);
    assert_eq!(origin["source"]["input_tokens"], 5_300);
    assert_eq!(origin["source"]["output_tokens"], 0);
    assert_eq!(origin["source"]["breaker_stage"], "suspend");
    assert_eq!(
        origin["source"]["workspace_connection_id"],
        json!(CONNECTION)
    );
    assert_eq!(origin["task"]["attempt_count"], 1);
    assert_eq!(origin["task"]["max_attempts"], 2);

    let failed_run_id = failed_checkpoint(&store).await;
    let input = correction_request(&store, failed_run_id).await;
    let before = state(&store).await;
    assert_eq!(before["source"], origin["source"]);
    let outcome = store
        .create_factory_verification_recovery(input)
        .await
        .expect("issue210: explicit correction must resume the exact measured-suspension origin");
    let FactoryVerificationRecoveryLaunch::SourceCorrection(launch) = outcome.launch else {
        panic!(
            "source correction must allocate a provider continuation, not a zero-provider exemption"
        );
    };
    assert!(!outcome.replayed);
    assert_eq!(launch.source_run_id, failed_run_id);
    assert_eq!(launch.workspace_run_id, SOURCE);
    assert_eq!(launch.provider_session_id, "fixture-session");
    assert_eq!(launch.workspace_connection_id, Some(CONNECTION));
    assert_eq!(launch.runner_id, RUNNER);
    assert_eq!(launch.agent_id, AGENT);
    assert_eq!(launch.source_repository.as_deref(), Some("fixture/source"));
    assert_eq!(launch.source_base_ref.as_deref(), Some("main"));
    assert_eq!(launch.source_base_commit, Some("a".repeat(40)));
    assert_eq!(launch.workspace_base_commit, "a".repeat(40));
    assert_eq!(launch.model.as_deref(), Some("fixture-model"));
    assert_eq!(launch.reasoning_effort.as_deref(), Some("medium"));
    assert!(
        launch
            .task_prompt
            .contains("genuine missing provider artifact")
    );

    let after = state(&store).await;
    assert_eq!(after["source"], origin["source"]);
    assert_eq!(
        retained_run(&after, failed_run_id),
        retained_run(&before, failed_run_id)
    );
    assert_eq!(after["task"]["attempt_count"], 2);
    assert_eq!(after["task"]["max_attempts"], 2);
    assert_eq!(after["task"]["contract"], before["task"]["contract"]);
    assert_eq!(
        after["task"]["verification_policy"],
        before["task"]["verification_policy"]
    );
    assert_eq!(after["item"]["policy"], origin["item"]["policy"]);
    assert_eq!(
        after["item"]["source_revision"],
        origin["item"]["source_revision"]
    );
    for field in [
        "budget_tokens",
        "original_budget_tokens",
        "budget_cost_microusd",
        "original_budget_cost_microusd",
    ] {
        assert_eq!(after["mission"][field], origin["mission"][field], "{field}");
    }
    let resumed = retained_run(&after, launch.run_id);
    assert_eq!(resumed["execution_mode"], "provider");
    assert_eq!(resumed["resumed_from_run_id"], json!(failed_run_id));
    assert_eq!(resumed["workspace_run_id"], json!(SOURCE));
    assert_eq!(resumed["workspace_connection_id"], json!(CONNECTION));
    assert_eq!(resumed["budget_tokens_limit"], 4_700);
    assert_eq!(resumed["budget_cost_microusd_limit"], 1_000_000);
    assert_eq!(resumed["input_tokens"], 0);
    assert_eq!(resumed["output_tokens"], 0);
    assert_eq!(resumed["cost_microusd"], 0);
    assert_eq!(after["runs"].as_array().unwrap().len(), 3);
}

async fn correction_state(store: &PgStore) -> Value {
    let mut snapshot = state(store).await;
    snapshot["issue210_ledger"] = sqlx::query_scalar::<_, Value>(
        "SELECT jsonb_build_object(
          'contract_revisions',(SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY id),'[]')
                                FROM mission_contract_revisions r),
          'budget_revisions',(SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY id),'[]')
                              FROM mission_budget_revisions r),
          'incidents',(SELECT coalesce(jsonb_agg(to_jsonb(i) ORDER BY id),'[]')
                       FROM circuit_breaker_incidents i),
          'budget_policies',(SELECT coalesce(jsonb_agg(to_jsonb(p) ORDER BY corp_id),'[]')
                             FROM corp_budget_policies p),
          'connections',(SELECT coalesce(jsonb_agg(to_jsonb(c) ORDER BY id),'[]')
                         FROM workspace_connections c),
          'runners',(SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY id),'[]') FROM runner_nodes r),
          'queued_messages',(SELECT coalesce(jsonb_agg(to_jsonb(q) ORDER BY id),'[]')
                             FROM queued_messages q))",
    )
    .fetch_one(&store.pool)
    .await
    .unwrap();
    snapshot
}

async fn ready_correction_with_profile(
    pool: PgPool,
    profile: CheckpointFixtureProfile,
) -> (PgStore, CreateFactoryVerificationRecoveryInput) {
    let store = fixture_with_profile(pool, true, false, None, false, profile).await;
    let source_run_id = failed_checkpoint(&store).await;
    let input = correction_request(&store, source_run_id).await;
    (store, input)
}

async fn ready_correction(pool: PgPool) -> (PgStore, CreateFactoryVerificationRecoveryInput) {
    ready_correction_with_profile(pool, correction_profile()).await
}

async fn reject_correction_without_changes(
    store: &PgStore,
    input: CreateFactoryVerificationRecoveryInput,
) {
    let before = correction_state(store).await;
    store
        .create_factory_verification_recovery(input)
        .await
        .expect_err("revoked or unproven source correction must fail closed");
    assert_eq!(
        correction_state(store).await,
        before,
        "rejected correction changed state, authority, accounting or journal"
    );
}

#[derive(Debug, Clone, Copy)]
enum CorrectionFault {
    OriginStop,
    ChildStop,
    OriginExplicitStop,
    ChildExplicitStop,
    UnrelatedSuspension,
    OriginQuarantine,
    ChildQuarantine,
    NoProgress,
    RepeatedTool,
    MissingCheckpoint,
    AlteredCheckpoint,
    MissingTermination,
    LiveTermination,
    MissingBudgetIncident,
    NonBudgetIncident,
    AlteredInheritedAuthority,
    ChangedOriginSource,
    ChangedVerificationPolicy,
    ChangedFactoryPolicy,
    ChangedOriginConnection,
    ChangedChildConnection,
    ConnectionOffline,
    MissingProviderSession,
    ActorDemoted,
    RoomRevoked,
}

async fn inject_correction_fault(store: &PgStore, child_run_id: Uuid, fault: CorrectionFault) {
    // Deliberately corrupt/revoke only this SQLx-owned fixture. These are not
    // repair operations or native transition claims. Never lower/reset limits,
    // consumed usage, or attempts to manufacture a resumable stopped lineage.
    if matches!(
        fault,
        CorrectionFault::OriginExplicitStop | CorrectionFault::ChildExplicitStop
    ) {
        let run_id = if matches!(fault, CorrectionFault::OriginExplicitStop) {
            SOURCE
        } else {
            child_run_id
        };
        let mut tx = store.pool.begin().await.unwrap();
        append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(ROOM),
                correlation_id: Some(MISSION),
                ..NewEvent::new(
                    CORP,
                    Some(OWNER),
                    "run.stop_requested",
                    "run",
                    run_id,
                    format!("issue210-stop-fixture:{}", Uuid::new_v4()),
                    json!({"agent_id":AGENT,"reason":"Explicit stop fence fixture"}),
                )
            },
        )
        .await
        .unwrap()
        .expect("one distinct stop fence");
        tx.commit().await.unwrap();
        return;
    }
    if matches!(fault, CorrectionFault::UnrelatedSuspension) {
        sqlx::query(
            "INSERT INTO runs(id,corp_id,task_id,agent_id,runner_id,assignment_token,status,
               workspace_run_id,provider_session_id,workspace_path,workspace_branch,
               workspace_base_ref,workspace_base_commit,workspace_disposition,workspace_fingerprint,
               source_repository,source_base_ref,source_base_commit,workspace_connection_id,
               breaker_stage,created_at)
             SELECT $1,corp_id,task_id,agent_id,runner_id,$2,'cancelled',workspace_run_id,
               'unrelated-session',workspace_path,workspace_branch,workspace_base_ref,
               workspace_base_commit,'preserved',workspace_fingerprint,source_repository,
               source_base_ref,source_base_commit,workspace_connection_id,'suspend',
               created_at-interval '1 hour' FROM runs WHERE corp_id=$3 AND id=$4",
        )
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(CORP)
        .bind(SOURCE)
        .execute(&store.pool)
        .await
        .unwrap();
        return;
    }
    let (sql, id) = match fault {
        CorrectionFault::OriginStop => (
            "UPDATE runs SET breaker_stage='stop' WHERE corp_id=$1 AND id=$2",
            SOURCE,
        ),
        CorrectionFault::ChildStop => (
            "UPDATE runs SET breaker_stage='stop' WHERE corp_id=$1 AND id=$2",
            child_run_id,
        ),
        CorrectionFault::OriginQuarantine => (
            "UPDATE runs SET workspace_disposition='quarantined' WHERE corp_id=$1 AND id=$2",
            SOURCE,
        ),
        CorrectionFault::ChildQuarantine => (
            "UPDATE runs SET workspace_disposition='quarantined' WHERE corp_id=$1 AND id=$2",
            child_run_id,
        ),
        CorrectionFault::NoProgress => (
            "UPDATE runs SET no_progress_events=8 WHERE corp_id=$1 AND id=$2",
            SOURCE,
        ),
        CorrectionFault::RepeatedTool => (
            "UPDATE runs SET repeated_tool_count=5 WHERE corp_id=$1 AND id=$2",
            SOURCE,
        ),
        CorrectionFault::MissingCheckpoint => (
            "UPDATE events SET payload=payload-'source_checkpoint'
             WHERE corp_id=$1 AND aggregate_id=$2 AND type='run.workspace_preserved'",
            SOURCE,
        ),
        CorrectionFault::AlteredCheckpoint => (
            "UPDATE events SET payload=jsonb_set(payload,'{source_checkpoint,workspace_fingerprint}',
               to_jsonb(repeat('c',64)),false)
             WHERE corp_id=$1 AND aggregate_id=$2 AND type='run.workspace_preserved'",
            SOURCE,
        ),
        CorrectionFault::MissingTermination => (
            "DELETE FROM events WHERE corp_id=$1 AND aggregate_id=$2 AND type='run.session_terminated'",
            SOURCE,
        ),
        CorrectionFault::LiveTermination => (
            "UPDATE events SET payload=jsonb_set(payload,'{provider_process_alive}','true',false)
             WHERE corp_id=$1 AND aggregate_id=$2 AND type='run.session_terminated'",
            SOURCE,
        ),
        CorrectionFault::MissingBudgetIncident => (
            "DELETE FROM circuit_breaker_incidents WHERE corp_id=$1 AND run_id=$2",
            SOURCE,
        ),
        CorrectionFault::NonBudgetIncident => (
            "UPDATE circuit_breaker_incidents SET input=jsonb_set(input,'{metric}','\"no_progress\"')
             WHERE corp_id=$1 AND run_id=$2",
            SOURCE,
        ),
        CorrectionFault::AlteredInheritedAuthority => (
            "UPDATE factory_verification_recoveries
             SET checkpoint_authority=jsonb_set(checkpoint_authority,
               '{checkpoint,workspace_fingerprint}',to_jsonb(repeat('c',64)),false)
             WHERE corp_id=$1 AND replacement_run_id=$2",
            child_run_id,
        ),
        CorrectionFault::ChangedOriginSource => (
            "UPDATE runs SET source_base_commit=repeat('c',40) WHERE corp_id=$1 AND id=$2",
            SOURCE,
        ),
        CorrectionFault::ChangedVerificationPolicy => (
            "UPDATE tasks SET verification_policy=jsonb_set(
               verification_policy,'{checks,1,min_bytes}','2',false)
             WHERE corp_id=$1 AND id=$2",
            TASK,
        ),
        CorrectionFault::ChangedFactoryPolicy => (
            "UPDATE factory_work_items SET policy=jsonb_set(policy,'{write_scope}','[\"other.md\"]')
             WHERE corp_id=$1 AND id=$2",
            ITEM,
        ),
        CorrectionFault::ChangedOriginConnection => (
            "UPDATE runs SET workspace_connection_id=NULL WHERE corp_id=$1 AND id=$2",
            SOURCE,
        ),
        CorrectionFault::ChangedChildConnection => (
            "UPDATE runs SET workspace_connection_id=NULL WHERE corp_id=$1 AND id=$2",
            child_run_id,
        ),
        CorrectionFault::ConnectionOffline => (
            "UPDATE workspace_connections SET status='offline' WHERE corp_id=$1 AND id=$2",
            CONNECTION,
        ),
        CorrectionFault::MissingProviderSession => (
            "UPDATE runs SET provider_session_id=NULL WHERE corp_id=$1 AND id=$2",
            SOURCE,
        ),
        CorrectionFault::ActorDemoted => (
            "UPDATE actors SET role='member' WHERE corp_id=$1 AND id=$2",
            OWNER,
        ),
        CorrectionFault::RoomRevoked => (
            "DELETE FROM room_memberships membership USING rooms room
             WHERE room.id=membership.room_id AND room.corp_id=$1 AND membership.actor_id=$2",
            OWNER,
        ),
        CorrectionFault::OriginExplicitStop
        | CorrectionFault::ChildExplicitStop
        | CorrectionFault::UnrelatedSuspension => unreachable!(),
    };
    let affected = sqlx::query(sql)
        .bind(CORP)
        .bind(id)
        .execute(&store.pool)
        .await
        .unwrap()
        .rows_affected();
    assert!(
        affected > 0,
        "fault {fault:?} did not reach its fixture row"
    );
}

macro_rules! admission_reject_case {
    ($name:ident, $fault:ident) => {
        #[sqlx::test(migrations = "../../db/migrations")]
        #[ignore = "requires explicitly owned SQLx maintenance database"]
        async fn $name(pool: PgPool) {
            let (store, input) = ready_correction(pool).await;
            inject_correction_fault(&store, input.source_run_id, CorrectionFault::$fault).await;
            reject_correction_without_changes(&store, input).await;
        }
    };
}

admission_reject_case!(issue210_correction_rejects_origin_stop, OriginStop);
admission_reject_case!(issue210_correction_rejects_child_stop, ChildStop);
admission_reject_case!(
    issue210_correction_rejects_origin_explicit_stop,
    OriginExplicitStop
);
admission_reject_case!(
    issue210_correction_rejects_child_explicit_stop,
    ChildExplicitStop
);
admission_reject_case!(
    issue210_correction_rejects_unrelated_suspension,
    UnrelatedSuspension
);
admission_reject_case!(
    issue210_correction_rejects_origin_quarantine,
    OriginQuarantine
);
admission_reject_case!(
    issue210_correction_rejects_child_quarantine,
    ChildQuarantine
);
admission_reject_case!(issue210_correction_rejects_no_progress, NoProgress);
admission_reject_case!(issue210_correction_rejects_repeated_tool, RepeatedTool);
admission_reject_case!(
    issue210_correction_rejects_missing_native_checkpoint,
    MissingCheckpoint
);
admission_reject_case!(
    issue210_correction_rejects_altered_native_checkpoint,
    AlteredCheckpoint
);
admission_reject_case!(
    issue210_correction_rejects_missing_native_termination,
    MissingTermination
);
admission_reject_case!(
    issue210_correction_rejects_live_native_termination,
    LiveTermination
);
admission_reject_case!(
    issue210_correction_rejects_missing_budget_incident,
    MissingBudgetIncident
);
admission_reject_case!(
    issue210_correction_rejects_non_budget_incident,
    NonBudgetIncident
);
admission_reject_case!(
    issue210_correction_rejects_altered_inherited_authority,
    AlteredInheritedAuthority
);
admission_reject_case!(
    issue210_correction_rejects_changed_origin_source,
    ChangedOriginSource
);
admission_reject_case!(
    issue210_correction_rejects_changed_verification_policy,
    ChangedVerificationPolicy
);
admission_reject_case!(
    issue210_correction_rejects_changed_factory_policy,
    ChangedFactoryPolicy
);
admission_reject_case!(
    issue210_correction_rejects_changed_origin_connection,
    ChangedOriginConnection
);
admission_reject_case!(
    issue210_correction_rejects_changed_child_connection,
    ChangedChildConnection
);
admission_reject_case!(
    issue210_correction_rejects_offline_connection,
    ConnectionOffline
);
admission_reject_case!(
    issue210_correction_rejects_missing_provider_session,
    MissingProviderSession
);
admission_reject_case!(issue210_correction_rejects_actor_demotion, ActorDemoted);
admission_reject_case!(issue210_correction_rejects_room_revocation, RoomRevoked);

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue210_correction_requires_explicit_current_revision(pool: PgPool) {
    let (store, input) = ready_correction(pool).await;
    let mut missing = input.clone();
    missing.contract_revision_id = None;
    reject_correction_without_changes(&store, missing).await;
    let mut foreign = input.clone();
    foreign.contract_revision_id = Some(Uuid::new_v4());
    reject_correction_without_changes(&store, foreign).await;
    let latest = correction_request(&store, input.source_run_id).await;
    assert_ne!(latest.contract_revision_id, input.contract_revision_id);
    reject_correction_without_changes(&store, input).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue210_other_modes_cannot_resume_measured_provider_origin(pool: PgPool) {
    let (store, input) = ready_correction(pool).await;
    // An ordinary verifier-only recovery must not receive the new provider
    // exception. Checkpoint verification remains its separate zero-model path.
    let mut verifier_only = input.clone();
    verifier_only.mode = FactoryVerificationRecoveryMode::VerifierOnly;
    reject_correction_without_changes(&store, verifier_only).await;
    let mut provider_origin = input;
    provider_origin.source_run_id = SOURCE;
    reject_correction_without_changes(&store, provider_origin).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue210_correction_exact_replay_and_changed_request_are_mutation_free(pool: PgPool) {
    let (store, input) = ready_correction(pool).await;
    let created = store
        .create_factory_verification_recovery(input.clone())
        .await
        .unwrap();
    let FactoryVerificationRecoveryLaunch::SourceCorrection(first) = created.launch else {
        panic!("provider continuation required");
    };
    let before = correction_state(&store).await;
    for _ in 0..2 {
        let replay = store
            .create_factory_verification_recovery(input.clone())
            .await
            .unwrap();
        assert!(replay.replayed);
        assert!(replay.events.is_empty());
        assert_eq!(replay.recovery.id, created.recovery.id);
        let FactoryVerificationRecoveryLaunch::SourceCorrection(launch) = replay.launch else {
            panic!("replay changed recovery execution mode");
        };
        assert_eq!(launch.run_id, first.run_id);
        assert_eq!(launch.assignment_token, first.assignment_token);
        assert_eq!(launch.provider_session_id, first.provider_session_id);
        assert_eq!(
            launch.workspace_connection_id,
            first.workspace_connection_id
        );
        assert_eq!(correction_state(&store).await, before);
    }
    let mut changed = input.clone();
    changed.reason.push_str(" changed");
    reject_correction_without_changes(&store, changed).await;
    let mut changed_mode = input;
    changed_mode.mode = FactoryVerificationRecoveryMode::VerifierOnly;
    reject_correction_without_changes(&store, changed_mode).await;
    assert_eq!(correction_state(&store).await, before);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue210_correction_command_failure_rolls_back_authority_and_accounting(pool: PgPool) {
    let (store, input) = ready_correction(pool).await;
    sqlx::raw_sql(
        "CREATE FUNCTION issue210_reject_correction_command() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN
           IF NEW.command_kind='factory_verification_recovery'
              AND NEW.payload->>'mode'='source_correction' THEN
             RAISE EXCEPTION 'issue210 owned SQLx correction-command fault';
           END IF;
           RETURN NEW;
         END $$;
         CREATE TRIGGER issue210_correction_command_fault BEFORE INSERT ON runner_commands
           FOR EACH ROW EXECUTE FUNCTION issue210_reject_correction_command();",
    )
    .execute(&store.pool)
    .await
    .unwrap();
    let before = correction_state(&store).await;
    let error = store
        .create_factory_verification_recovery(input)
        .await
        .expect_err("injected durable command failure must roll back the whole correction");
    assert!(
        format!("{error:#}").contains("issue210 owned SQLx correction-command fault"),
        "did not reach the intended rollback boundary: {error:#}"
    );
    assert_eq!(correction_state(&store).await, before);
}

fn rolling_limits() -> CheckpointRollingLimits {
    CheckpointRollingLimits {
        actor_tokens: 4_000_000,
        actor_cost_microusd: 10_000_000,
        corp_tokens: 20_000_000,
        corp_cost_microusd: 100_000_000,
    }
}

macro_rules! exhausted_authority_case {
    ($name:ident, $profile:expr) => {
        #[sqlx::test(migrations = "../../db/migrations")]
        #[ignore = "requires explicitly owned SQLx maintenance database"]
        async fn $name(pool: PgPool) {
            let (store, input) = ready_correction_with_profile(pool, $profile).await;
            reject_correction_without_changes(&store, input).await;
        }
    };
}

exhausted_authority_case!(
    issue210_correction_rejects_exhausted_mission_tokens,
    CheckpointFixtureProfile {
        mission_tokens: 5_300,
        ..correction_profile()
    }
);
exhausted_authority_case!(
    issue210_correction_rejects_exhausted_mission_cost,
    CheckpointFixtureProfile {
        mission_cost_microusd: 1_000_000,
        used_cost_microusd: 1_000_000,
        ..correction_profile()
    }
);
exhausted_authority_case!(
    issue210_correction_rejects_exhausted_provider_attempts,
    CheckpointFixtureProfile {
        attempt_count: 2,
        ..correction_profile()
    }
);
exhausted_authority_case!(
    issue210_correction_rejects_exhausted_actor_tokens,
    CheckpointFixtureProfile {
        rolling_limits: Some(CheckpointRollingLimits {
            actor_tokens: 5_300,
            ..rolling_limits()
        }),
        ..correction_profile()
    }
);
exhausted_authority_case!(
    issue210_correction_rejects_exhausted_corp_tokens,
    CheckpointFixtureProfile {
        rolling_limits: Some(CheckpointRollingLimits {
            corp_tokens: 5_300,
            ..rolling_limits()
        }),
        ..correction_profile()
    }
);
exhausted_authority_case!(
    issue210_correction_rejects_exhausted_actor_cost,
    CheckpointFixtureProfile {
        used_cost_microusd: 100_000,
        rolling_limits: Some(CheckpointRollingLimits {
            actor_cost_microusd: 100_000,
            ..rolling_limits()
        }),
        ..correction_profile()
    }
);
exhausted_authority_case!(
    issue210_correction_rejects_exhausted_corp_cost,
    CheckpointFixtureProfile {
        used_cost_microusd: 100_000,
        rolling_limits: Some(CheckpointRollingLimits {
            corp_cost_microusd: 100_000,
            ..rolling_limits()
        }),
        ..correction_profile()
    }
);

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue210_correction_clamps_to_remaining_rolling_tokens_and_cost(pool: PgPool) {
    let profile = CheckpointFixtureProfile {
        used_cost_microusd: 100_000,
        rolling_limits: Some(CheckpointRollingLimits {
            actor_tokens: 6_000,
            corp_tokens: 6_500,
            actor_cost_microusd: 250_000,
            corp_cost_microusd: 200_000,
        }),
        ..correction_profile()
    };
    let (store, input) = ready_correction_with_profile(pool, profile).await;
    let before = correction_state(&store).await;
    let created = store
        .create_factory_verification_recovery(input)
        .await
        .unwrap();
    let FactoryVerificationRecoveryLaunch::SourceCorrection(launch) = created.launch else {
        panic!("remaining rolling authority must fund a provider continuation");
    };
    let after = correction_state(&store).await;
    let run = retained_run(&after, launch.run_id);
    assert_eq!(run["budget_tokens_limit"], 700);
    assert_eq!(run["budget_cost_microusd_limit"], 100_000);
    assert_eq!(after["source"], before["source"]);
    assert_eq!(
        after["issue210_ledger"]["budget_policies"],
        before["issue210_ledger"]["budget_policies"]
    );
    assert_eq!(
        after["issue210_ledger"]["incidents"],
        before["issue210_ledger"]["incidents"]
    );
    assert_eq!(after["task"]["attempt_count"], 2);
}

async fn admitted_correction(
    pool: PgPool,
) -> (
    PgStore,
    CreateFactoryVerificationRecoveryInput,
    PendingRunnerCommand,
) {
    let (store, input) = ready_correction(pool).await;
    let created = store
        .create_factory_verification_recovery(input.clone())
        .await
        .unwrap();
    let FactoryVerificationRecoveryLaunch::SourceCorrection(launch) = created.launch else {
        panic!("provider continuation required");
    };
    let command = store
        .pending_runner_commands(RUNNER)
        .await
        .unwrap()
        .into_iter()
        .find(|command| command.run_id == launch.run_id)
        .expect("native durable source-correction command");
    assert_eq!(command.command_kind, "factory_verification_recovery");
    assert_eq!(command.payload["mode"], "source_correction");
    (store, input, command)
}

async fn assert_correction_dispatch(
    store: &PgStore,
    command: &PendingRunnerCommand,
    expected: bool,
) {
    let before = correction_state(store).await;
    assert_eq!(
        store
            .source_correction_command_authorized(command)
            .await
            .unwrap(),
        expected,
        "unexpected current-authority dispatch result"
    );
    assert_eq!(correction_state(store).await, before);
}

macro_rules! dispatch_reject_case {
    ($name:ident, $fault:ident) => {
        #[sqlx::test(migrations = "../../db/migrations")]
        #[ignore = "requires explicitly owned SQLx maintenance database"]
        async fn $name(pool: PgPool) {
            let (store, input, command) = admitted_correction(pool).await;
            assert_correction_dispatch(&store, &command, true).await;
            inject_correction_fault(&store, input.source_run_id, CorrectionFault::$fault).await;
            assert_correction_dispatch(&store, &command, false).await;
        }
    };
}

dispatch_reject_case!(issue210_dispatch_rechecks_origin_stop, OriginStop);
dispatch_reject_case!(issue210_dispatch_rechecks_child_stop, ChildStop);
dispatch_reject_case!(
    issue210_dispatch_rechecks_origin_explicit_stop,
    OriginExplicitStop
);
dispatch_reject_case!(
    issue210_dispatch_rechecks_child_explicit_stop,
    ChildExplicitStop
);
dispatch_reject_case!(
    issue210_dispatch_rechecks_unrelated_suspension,
    UnrelatedSuspension
);
dispatch_reject_case!(
    issue210_dispatch_rechecks_origin_quarantine,
    OriginQuarantine
);
dispatch_reject_case!(issue210_dispatch_rechecks_child_quarantine, ChildQuarantine);
dispatch_reject_case!(
    issue210_dispatch_rechecks_missing_checkpoint,
    MissingCheckpoint
);
dispatch_reject_case!(
    issue210_dispatch_rechecks_altered_checkpoint,
    AlteredCheckpoint
);
dispatch_reject_case!(
    issue210_dispatch_rechecks_missing_termination,
    MissingTermination
);
dispatch_reject_case!(
    issue210_dispatch_rechecks_missing_budget_incident,
    MissingBudgetIncident
);
dispatch_reject_case!(
    issue210_dispatch_rechecks_altered_inherited_authority,
    AlteredInheritedAuthority
);
dispatch_reject_case!(
    issue210_dispatch_rechecks_origin_source,
    ChangedOriginSource
);
dispatch_reject_case!(
    issue210_dispatch_rechecks_verification_policy,
    ChangedVerificationPolicy
);
dispatch_reject_case!(
    issue210_dispatch_rechecks_factory_policy,
    ChangedFactoryPolicy
);
dispatch_reject_case!(
    issue210_dispatch_rechecks_origin_connection,
    ChangedOriginConnection
);
dispatch_reject_case!(
    issue210_dispatch_rechecks_child_connection,
    ChangedChildConnection
);
dispatch_reject_case!(
    issue210_dispatch_rechecks_offline_connection,
    ConnectionOffline
);
dispatch_reject_case!(issue210_dispatch_rechecks_actor_demotion, ActorDemoted);
dispatch_reject_case!(issue210_dispatch_rechecks_room_revocation, RoomRevoked);

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue210_dispatch_binds_exact_native_command_and_rejects_other_modes(pool: PgPool) {
    let (store, _, command) = admitted_correction(pool).await;
    assert_correction_dispatch(&store, &command, true).await;
    let mut foreign_id = command.clone();
    foreign_id.id = Uuid::new_v4();
    let mut foreign_corp = command.clone();
    foreign_corp.corp_id = Uuid::new_v4();
    let mut foreign_runner = command.clone();
    foreign_runner.runner_id = "unrelated-runner".to_owned();
    let mut foreign_run = command.clone();
    foreign_run.run_id = SOURCE;
    let mut foreign_kind = command.clone();
    foreign_kind.command_kind = "unrelated-command".to_owned();
    for altered in [
        foreign_id,
        foreign_corp,
        foreign_runner,
        foreign_run,
        foreign_kind,
    ] {
        assert_correction_dispatch(&store, &altered, false).await;
    }
    for (field, value) in [
        ("mode", json!("verifier_only")),
        ("mode", json!("checkpoint_verification")),
        ("assignment_token", json!(Uuid::new_v4())),
        ("source_run_id", json!(SOURCE)),
        ("workspace_run_id", json!(Uuid::new_v4())),
        ("workspace_connection_id", Value::Null),
        ("provider_session_id", json!("another-session")),
        ("source_base_commit", json!("c".repeat(40))),
        ("expected_head_commit", json!("c".repeat(40))),
        ("expected_workspace_fingerprint", json!("c".repeat(64))),
        ("verification_policy", json!({"checks":[]})),
        ("write_scope", json!(["other.md"])),
    ] {
        let mut altered = command.clone();
        altered.payload[field] = value;
        assert_correction_dispatch(&store, &altered, false).await;
    }
    assert_correction_dispatch(&store, &command, true).await;
    assert_eq!(command.payload["provider_session_id"], "fixture-session");
    assert_eq!(command.payload["workspace_run_id"], json!(SOURCE));
    assert_eq!(
        command.payload["workspace_connection_id"],
        json!(CONNECTION)
    );
    assert!(command.payload["provider_artifact"].is_null());
    let before = correction_state(&store).await;
    assert_eq!(before["task"]["attempt_count"], 2);
    assert_eq!(before["task"]["max_attempts"], 2);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue210_dispatch_rechecks_native_explicit_stop_of_provider_child(pool: PgPool) {
    let (store, _, command) = admitted_correction(pool).await;
    assert_correction_dispatch(&store, &command, true).await;
    let stopped = store
        .request_emergency_stop(
            CORP,
            AGENT,
            OWNER,
            "Stop this provider continuation explicitly",
        )
        .await
        .unwrap();
    assert_eq!(stopped.run_id, command.run_id);
    assert_correction_dispatch(&store, &command, false).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue210_dispatch_rejects_settled_native_command(pool: PgPool) {
    let (store, _, command) = admitted_correction(pool).await;
    assert_correction_dispatch(&store, &command, true).await;
    store
        .acknowledge_runner_command(command.id, RUNNER)
        .await
        .unwrap();
    assert_correction_dispatch(&store, &command, false).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue210_dispatch_rejects_failed_recovery_without_another_attempt(pool: PgPool) {
    let (store, _, command) = admitted_correction(pool).await;
    assert_correction_dispatch(&store, &command, true).await;
    let token = Uuid::parse_str(command.payload["assignment_token"].as_str().unwrap()).unwrap();
    store
        .apply_runner_event(event(
            command.run_id,
            token,
            "run.failed",
            json!({"error":"Synthetic native pre-start correction failure"}),
        ))
        .await
        .unwrap();
    assert_correction_dispatch(&store, &command, false).await;
    let snapshot = correction_state(&store).await;
    assert_eq!(snapshot["task"]["attempt_count"], 2);
    assert_eq!(snapshot["item"]["state"], "verification_failed");
    assert_eq!(retained_run(&snapshot, command.run_id)["status"], "failed");
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue210_correction_provider_usage_consumes_allocation_without_zero_exemption(
    pool: PgPool,
) {
    let (store, _, command) = admitted_correction(pool).await;
    assert_correction_dispatch(&store, &command, true).await;
    let before = correction_state(&store).await;
    let token = Uuid::parse_str(command.payload["assignment_token"].as_str().unwrap()).unwrap();
    store
        .apply_runner_event(event(
            command.run_id,
            token,
            "run.started",
            json!({"workspace":"fixture-worktree","workspace_branch":"crony/fixture",
                "workspace_base_ref":"main","workspace_base_commit":"a".repeat(40),
                "provider_session_id":"fixture-session","execution_mode":"provider"}),
        ))
        .await
        .unwrap();
    store
        .apply_runner_event(event(
            command.run_id,
            token,
            "run.usage",
            json!({"input_tokens":4_700,"output_tokens":0,"cost_microusd":0}),
        ))
        .await
        .expect("a coding-agent continuation must accept and account for model usage");
    let breaker = store
        .evaluate_circuit_breaker(CORP, command.run_id)
        .await
        .unwrap();
    assert_eq!(breaker.event.unwrap().payload["stage"], "suspend");
    assert_correction_dispatch(&store, &command, false).await;
    let after = correction_state(&store).await;
    assert_eq!(after["source"], before["source"]);
    let run = retained_run(&after, command.run_id);
    assert_eq!(run["budget_tokens_limit"], 4_700);
    assert_eq!(run["input_tokens"], 4_700);
    assert_eq!(run["breaker_stage"], "suspend");
    assert_eq!(after["task"]["attempt_count"], 2);
    assert_eq!(after["mission"]["budget_tokens"], 10_000);
    assert_eq!(after["mission"]["original_budget_tokens"], 10_000);
}

macro_rules! replay_revocation_case {
    ($name:ident, $fault:ident) => {
        #[sqlx::test(migrations = "../../db/migrations")]
        #[ignore = "requires explicitly owned SQLx maintenance database"]
        async fn $name(pool: PgPool) {
            let (store, input, command) = admitted_correction(pool).await;
            assert_correction_dispatch(&store, &command, true).await;
            inject_correction_fault(&store, input.source_run_id, CorrectionFault::$fault).await;
            reject_correction_without_changes(&store, input).await;
            assert_correction_dispatch(&store, &command, false).await;
        }
    };
}

replay_revocation_case!(issue210_correction_replay_rechecks_actor_role, ActorDemoted);
replay_revocation_case!(
    issue210_correction_replay_rechecks_room_membership,
    RoomRevoked
);

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue210_dispatch_and_replay_reject_missing_correction_grant(pool: PgPool) {
    let (store, input, command) = admitted_correction(pool).await;
    assert_correction_dispatch(&store, &command, true).await;
    let before = correction_state(&store).await;
    let correction = before["recoveries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|recovery| recovery["replacement_run_id"] == json!(command.run_id))
        .unwrap();
    assert!(correction["source_correction_authority"].is_object());
    assert!(correction["checkpoint_authority"].is_null());
    sqlx::query(
        "UPDATE factory_verification_recoveries SET source_correction_authority=NULL
         WHERE corp_id=$1 AND replacement_run_id=$2",
    )
    .bind(CORP)
    .bind(command.run_id)
    .execute(&store.pool)
    .await
    .unwrap();
    assert_correction_dispatch(&store, &command, false).await;
    reject_correction_without_changes(&store, input).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue210_dispatch_and_replay_reject_damaged_correction_grant(pool: PgPool) {
    let (store, input, command) = admitted_correction(pool).await;
    assert_correction_dispatch(&store, &command, true).await;
    sqlx::query(
        "UPDATE factory_verification_recoveries SET source_correction_authority=jsonb_set(
           source_correction_authority,'{provider_session_id}','\"unrelated-session\"',false)
         WHERE corp_id=$1 AND replacement_run_id=$2",
    )
    .bind(CORP)
    .bind(command.run_id)
    .execute(&store.pool)
    .await
    .unwrap();
    assert_correction_dispatch(&store, &command, false).await;
    reject_correction_without_changes(&store, input).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue210_ordinary_source_correction_retains_healthy_provider_path(pool: PgPool) {
    let store = fixture_with_profile(
        pool,
        true,
        false,
        None,
        false,
        CheckpointFixtureProfile {
            used_tokens: 1_000,
            expected_stage: "healthy",
            verification_failure: true,
            ..correction_profile()
        },
    )
    .await;
    let original = correction_state(&store).await;
    assert_eq!(original["source"]["breaker_stage"], "healthy");
    assert_eq!(original["source"]["status"], "failed");
    assert_eq!(original["source"]["verification_status"], "failed");
    assert_eq!(original["source"]["execution_mode"], "provider");
    assert!(original["recoveries"].as_array().unwrap().is_empty());
    assert!(
        original["issue210_ledger"]["incidents"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let context = store
        .factory_verification_recovery_context(CORP, OWNER, ITEM)
        .await
        .unwrap()
        .unwrap();
    assert!(!context.checkpoint_verification);
    let mut input = correction_request(&store, SOURCE).await;
    input.expected_head_commit = context.expected_head_commit;
    let created = store
        .create_factory_verification_recovery(input.clone())
        .await
        .expect("ordinary source correction must not require a budget-checkpoint grant");
    let FactoryVerificationRecoveryLaunch::SourceCorrection(launch) = created.launch else {
        panic!("ordinary correction must resume a provider");
    };
    assert_eq!(launch.source_run_id, SOURCE);
    assert_eq!(launch.workspace_run_id, SOURCE);
    assert_eq!(launch.workspace_connection_id, Some(CONNECTION));
    assert_eq!(launch.provider_session_id, "fixture-session");
    let command = store
        .pending_runner_commands(RUNNER)
        .await
        .unwrap()
        .into_iter()
        .find(|command| command.run_id == launch.run_id)
        .unwrap();
    assert_correction_dispatch(&store, &command, true).await;
    let after = correction_state(&store).await;
    assert_eq!(after["source"], original["source"]);
    assert_eq!(after["recoveries"].as_array().unwrap().len(), 1);
    assert!(after["recoveries"][0]["checkpoint_authority"].is_null());
    assert!(after["recoveries"][0]["source_correction_authority"].is_null());
    assert_eq!(after["task"]["attempt_count"], 2);
    assert_eq!(
        retained_run(&after, launch.run_id)["budget_tokens_limit"],
        5_000
    );
    let replay = store
        .create_factory_verification_recovery(input)
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(correction_state(&store).await, after);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue210_dispatch_rejects_malformed_correction_grant(pool: PgPool) {
    let (store, _, command) = admitted_correction(pool).await;
    assert_correction_dispatch(&store, &command, true).await;
    sqlx::query(
        "UPDATE factory_verification_recoveries SET source_correction_authority='{}'
         WHERE corp_id=$1 AND replacement_run_id=$2",
    )
    .bind(CORP)
    .bind(command.run_id)
    .execute(&store.pool)
    .await
    .unwrap();
    assert_correction_dispatch(&store, &command, false).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue210_dispatch_retains_allocated_attempt_while_database_fences_excess(pool: PgPool) {
    let (store, _, command) = admitted_correction(pool).await;
    assert_correction_dispatch(&store, &command, true).await;
    let before = correction_state(&store).await;
    let denied =
        sqlx::query("UPDATE tasks SET attempt_count=attempt_count+1 WHERE corp_id=$1 AND id=$2")
            .bind(CORP)
            .bind(TASK)
            .execute(&store.pool)
            .await
            .expect_err("the native database must reject attempts beyond its ceiling");
    assert_eq!(
        denied
            .as_database_error()
            .and_then(|error| error.constraint()),
        Some("tasks_execution_limits_check"),
    );
    assert_eq!(correction_state(&store).await, before);
    assert_eq!(before["task"]["attempt_count"], 2);
    assert_eq!(before["task"]["max_attempts"], 2);
    // The already-authorized last attempt is valid; an impossible counter must
    // not be manufactured to test a state that the database cannot represent.
    assert_correction_dispatch(&store, &command, true).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue210_dispatch_rejects_superseded_generation(pool: PgPool) {
    let (store, _, command) = admitted_correction(pool).await;
    assert_correction_dispatch(&store, &command, true).await;
    // Adversarial later metadata, not a claim that a provider bypassed attempts.
    // The old command must fail closed even if another generation appears.
    sqlx::query(
        "INSERT INTO runs(id,corp_id,task_id,agent_id,runner_id,assignment_token,status,
           workspace_run_id,resumed_from_run_id,execution_mode,source_repository,
           source_base_ref,source_base_commit,workspace_connection_id,
           budget_tokens_limit,budget_cost_microusd_limit,created_at)
         SELECT $1,corp_id,task_id,agent_id,runner_id,$2,'failed',workspace_run_id,id,
           'verification_only',source_repository,source_base_ref,source_base_commit,
           workspace_connection_id,0,0,created_at+interval '1 second'
         FROM runs WHERE corp_id=$3 AND id=$4",
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(CORP)
    .bind(command.run_id)
    .execute(&store.pool)
    .await
    .unwrap();
    assert_correction_dispatch(&store, &command, false).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue210_dispatch_rechecks_rolling_spend_after_authorization(pool: PgPool) {
    let (store, input) = ready_correction_with_profile(
        pool,
        CheckpointFixtureProfile {
            rolling_limits: Some(CheckpointRollingLimits {
                actor_tokens: 6_000,
                ..rolling_limits()
            }),
            ..correction_profile()
        },
    )
    .await;
    let created = store
        .create_factory_verification_recovery(input)
        .await
        .unwrap();
    let FactoryVerificationRecoveryLaunch::SourceCorrection(launch) = created.launch else {
        panic!("provider continuation required");
    };
    let command = store
        .pending_runner_commands(RUNNER)
        .await
        .unwrap()
        .into_iter()
        .find(|command| command.run_id == launch.run_id)
        .unwrap();
    assert_correction_dispatch(&store, &command, true).await;
    let before = correction_state(&store).await;
    assert_eq!(
        retained_run(&before, launch.run_id)["budget_tokens_limit"],
        700
    );

    // New, unrelated paid usage is a separate synthetic history record. It
    // consumes rolling allowance without changing this mission's immutable
    // limits, source usage, provider attempts, or already allocated run ceiling.
    let other_mission = Uuid::new_v4();
    let other_task = Uuid::new_v4();
    let other_run = Uuid::new_v4();
    let mut tx = store.pool.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO missions(id,corp_id,room_id,requested_by,title,status,budget_tokens,
           original_budget_tokens,original_budget_cost_microusd)
         VALUES($1,$2,$3,$4,'Separate rolling-spend fixture','failed',10000,10000,5000000)",
    )
    .bind(other_mission)
    .bind(CORP)
    .bind(ROOM)
    .bind(OWNER)
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO tasks(id,corp_id,mission_id,title,objective,status,assigned_agent_id,
           required_adapter,contract,verification_policy,attempt_count,max_attempts,plan_key)
         SELECT $1,corp_id,$2,'Rolling spend','Fixture paid usage','failed',assigned_agent_id,
           required_adapter,contract,verification_policy,1,1,'rolling-spend'
         FROM tasks WHERE corp_id=$3 AND id=$4",
    )
    .bind(other_task)
    .bind(other_mission)
    .bind(CORP)
    .bind(TASK)
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO runs(id,corp_id,task_id,agent_id,runner_id,assignment_token,status,
           workspace_run_id,input_tokens,budget_tokens_limit)
         VALUES($1,$2,$3,$4,$5,$6,'failed',$1,700,1000)",
    )
    .bind(other_run)
    .bind(CORP)
    .bind(other_task)
    .bind(AGENT)
    .bind(RUNNER)
    .bind(Uuid::new_v4())
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_correction_dispatch(&store, &command, false).await;
    let after = correction_state(&store).await;
    assert_eq!(after["source"], before["source"]);
    assert_eq!(after["mission"], before["mission"]);
    assert_eq!(after["task"], before["task"]);
    assert_eq!(
        retained_run(&after, launch.run_id),
        retained_run(&before, launch.run_id)
    );
    assert_eq!(
        after["issue210_ledger"]["budget_policies"],
        before["issue210_ledger"]["budget_policies"]
    );
}

async fn correction_wait_observed(
    pool: &PgPool,
    waiter_pid: i32,
    blocker_pid: i32,
    require_advisory: bool,
) -> Result<bool> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let observed: bool = sqlx::query_scalar(
            "SELECT EXISTS(
               SELECT 1 FROM pg_stat_activity activity
               WHERE activity.datname=current_database() AND activity.pid=$1
                 AND activity.wait_event_type='Lock'
                 AND $2=ANY(pg_blocking_pids(activity.pid))
                 AND (NOT $3::boolean OR EXISTS(
                   SELECT 1 FROM pg_locks waiting_lock
                   WHERE waiting_lock.pid=activity.pid AND NOT waiting_lock.granted
                     AND waiting_lock.locktype='advisory')))",
        )
        .bind(waiter_pid)
        .bind(blocker_pid)
        .bind(require_advisory)
        .fetch_one(pool)
        .await?;
        if observed || std::time::Instant::now() >= deadline {
            return Ok(observed);
        }
    }
}

async fn correction_single_connection_pool(pool: &PgPool) -> PgPool {
    // Reuse only this SQLx-owned test database's typed options. Never consult
    // ambient DATABASE_URL or print a connection string/credential.
    let owned = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    // Bound only these dedicated fixture sessions, including a failing lock
    // regression. This does not alter server settings or another test's pool.
    sqlx::query("SET statement_timeout = '15s'")
        .execute(&owned)
        .await
        .unwrap();
    owned
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue210_dispatch_serializes_native_factory_block_while_connection_waits(pool: PgPool) {
    use std::future::Future;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::Poll;

    let (store, _, command) = admitted_correction(pool).await;
    assert_correction_dispatch(&store, &command, true).await;
    let before = correction_state(&store).await;
    let dispatch_pool = correction_single_connection_pool(&store.pool).await;
    let block_pool = correction_single_connection_pool(&store.pool).await;
    let dispatch_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&dispatch_pool)
        .await
        .unwrap();
    let block_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&block_pool)
        .await
        .unwrap();
    let dispatch_store = PgStore {
        pool: dispatch_pool.clone(),
    };
    let block_store = PgStore {
        pool: block_pool.clone(),
    };
    let mut connection_hold = store.pool.begin().await.unwrap();
    let connection_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *connection_hold)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM workspace_connections WHERE corp_id=$1 AND id=$2 FOR UPDATE")
        .bind(CORP)
        .bind(CONNECTION)
        .fetch_one(&mut *connection_hold)
        .await
        .unwrap();

    let mut dispatch = Box::pin(dispatch_store.source_correction_command_authorized(&command));
    let mut native_block = Box::pin(block_store.transition_factory_work_item(
        TransitionFactoryWorkItemInput {
            corp_id: CORP,
            work_item_id: ITEM,
            actor_id: OWNER,
            claim_token: CLAIM,
            expected_version: before["item"]["version"].as_i64().unwrap(),
            idempotency_key: format!("issue210-connection-wait-block:{}", Uuid::new_v4()),
            state: FactoryWorkItemState::Blocked,
            failure_detail: Some("Native Factory block during dispatch connection wait".to_owned()),
        },
    ));
    let begin_block = AtomicBool::new(false);
    let mut probe = Box::pin(async {
        let dispatch_wait =
            correction_wait_observed(&store.pool, dispatch_pid, connection_pid, false).await;
        begin_block.store(true, Ordering::Release);
        let block_wait = correction_wait_observed(&store.pool, block_pid, dispatch_pid, true).await;
        let held_item_state: String =
            sqlx::query_scalar("SELECT state FROM factory_work_items WHERE corp_id=$1 AND id=$2")
                .bind(CORP)
                .bind(ITEM)
                .fetch_one(&store.pool)
                .await
                .unwrap();
        // Always release the row before asserting observations or collecting
        // operation errors, so a regression does not strand our fixture wait.
        connection_hold.rollback().await.unwrap();
        (dispatch_wait, block_wait, held_item_state)
    });
    let mut dispatch_finished = None;
    let mut block_finished = None;
    let observed = std::future::poll_fn(|context| {
        if dispatch_finished.is_none()
            && let Poll::Ready(result) = dispatch.as_mut().poll(context)
        {
            dispatch_finished = Some(result);
        }
        if begin_block.load(Ordering::Acquire)
            && block_finished.is_none()
            && let Poll::Ready(result) = native_block.as_mut().poll(context)
        {
            block_finished = Some(result);
        }
        probe.as_mut().poll(context)
    })
    .await;
    drop(probe);
    let dispatch_result = match dispatch_finished {
        Some(result) => result,
        None => dispatch.await,
    };
    let block_result = match block_finished {
        Some(result) => result,
        None => native_block.await,
    };
    dispatch_pool.close().await;
    block_pool.close().await;

    assert!(
        observed.0.unwrap(),
        "dispatch never waited on our connection row"
    );
    assert!(
        observed.1.unwrap(),
        "native Factory block did not wait on the dispatch transaction's advisory gate"
    );
    assert_eq!(
        observed.2, "running",
        "Factory block committed during the connection wait"
    );
    assert!(
        dispatch_result.unwrap(),
        "the serialized pre-block dispatch lost valid authority"
    );
    assert_eq!(
        block_result.unwrap().work_item.state,
        FactoryWorkItemState::Blocked
    );
    let after = correction_state(&store).await;
    assert_eq!(after["item"]["state"], "blocked");
    for field in [
        "source",
        "runs",
        "task",
        "mission",
        "commands",
        "issue210_ledger",
    ] {
        assert_eq!(after[field], before[field], "{field}");
    }
    assert_correction_dispatch(&store, &command, false).await;
    // This proves only the observed connection-wait/native-Factory ordering in
    // this disposable database, not global deadlock freedom or runtime safety.
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue210_publication_guard_rejects_missing_correction_grant_without_panic(pool: PgPool) {
    let (store, _, command) = admitted_correction(pool).await;
    let context = store
        .factory_verification_recovery_context(CORP, OWNER, ITEM)
        .await
        .unwrap()
        .unwrap();
    let before = correction_state(&store).await;
    // Exercise just the publication provenance guard. An admitted run is enough
    // for this helper; this is not a verified export or publication acceptance.
    let mut tx = store.pool.begin().await.unwrap();
    checkpoint_correction::publication_authority_tx(&mut tx, &context.work_item, command.run_id)
        .await
        .expect("the exact retained correction grant must validate at this provenance guard");
    tx.rollback().await.unwrap();
    assert_eq!(correction_state(&store).await, before);
    sqlx::query(
        "UPDATE factory_verification_recoveries SET source_correction_authority=NULL
         WHERE corp_id=$1 AND replacement_run_id=$2",
    )
    .bind(CORP)
    .bind(command.run_id)
    .execute(&store.pool)
    .await
    .unwrap();
    let missing = correction_state(&store).await;
    let mut tx = store.pool.begin().await.unwrap();
    let denied = checkpoint_correction::publication_authority_tx(
        &mut tx,
        &context.work_item,
        command.run_id,
    )
    .await;
    tx.rollback().await.unwrap();
    denied.expect_err("SQL NULL provenance must return normal denial rather than panic");
    assert_eq!(correction_state(&store).await, missing);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue210_revised_narrowing_context_distinguishes_correction_from_checkpoint(pool: PgPool) {
    let store = fixture_with_profile(pool, true, false, None, false, correction_profile()).await;
    let failed_run_id = failed_checkpoint(&store).await;
    let before = correction_state(&store).await;
    let initial_context = store
        .factory_verification_recovery_context(CORP, OWNER, ITEM)
        .await
        .unwrap()
        .unwrap();
    assert!(initial_context.checkpoint_verification);
    assert!(initial_context.checkpoint_verification_available);
    let mut contract: TaskContract =
        serde_json::from_value(before["task"]["contract"].clone()).unwrap();
    contract
        .prohibited_actions
        .push("Do not replace the preserved application".to_owned());
    let mut policy = before["task"]["verification_policy"].clone();
    policy["checks"][1]["min_bytes"] = json!(2);
    let policy: VerificationPolicy = serde_json::from_value(policy).unwrap();
    let revision = store
        .create_mission_contract_revision(CreateMissionContractRevisionInput {
            corp_id: CORP,
            mission_id: MISSION,
            task_id: TASK,
            actor_id: OWNER,
            expected_contract_version: before["task"]["contract_version"].as_i64().unwrap(),
            next_action: MissionContractRevisionAction::Resume,
            source_run_id: Some(failed_run_id),
            reason: "Strengthen the artifact floor while preserving all original authority"
                .to_owned(),
            idempotency_key: Uuid::new_v4(),
            description: "Correct only the retained source under the strengthened evidence check"
                .to_owned(),
            contract,
            verification_policy: policy.clone(),
        })
        .await
        .unwrap();
    assert_ne!(
        revision.revision.previous_verification_policy,
        revision.revision.replacement_verification_policy
    );
    let revised = correction_state(&store).await;
    assert_eq!(revised["source"], before["source"]);
    assert_eq!(revised["runs"], before["runs"]);
    assert_eq!(revised["task"]["attempt_count"], 1);
    let context = store
        .factory_verification_recovery_context(CORP, OWNER, ITEM)
        .await
        .expect("current narrow Resume revision must retain readable historical checkpoint proof")
        .unwrap();
    assert!(
        context.checkpoint_verification,
        "checkpoint-family hint must remain true"
    );
    assert!(
        !context.checkpoint_verification_available,
        "a provider-free retry cannot adopt the changed policy"
    );
    assert!(context.checkpoint_source_correction);
    assert_eq!(context.source_run_id, failed_run_id);
    assert_eq!(context.remaining_attempts, 1);
    assert_eq!(context.remaining_mission_tokens, 4_700);
    assert_eq!(context.workspace_fingerprint, Some("b".repeat(64)));
    assert_eq!(context.expected_head_commit, Some("a".repeat(40)));
    assert_eq!(correction_state(&store).await, revised);

    let mut checkpoint_only = request();
    checkpoint_only.source_run_id = failed_run_id;
    checkpoint_only.expected_factory_version = context.work_item.version;
    reject_correction_without_changes(&store, checkpoint_only).await;
    let mut correction = request();
    correction.source_run_id = failed_run_id;
    correction.expected_factory_version = context.work_item.version;
    correction.mode = FactoryVerificationRecoveryMode::SourceCorrection;
    correction.contract_revision_id = Some(revision.revision.id);
    correction.reason =
        "Explicit provider correction under the current narrowed revision".to_owned();
    correction.observed_source_revision = "revision-2".to_owned();
    correction.reviewed_source_snapshot =
        json!({"source_revision":"revision-2","issue_number":148});
    let outcome = store
        .create_factory_verification_recovery(correction)
        .await
        .expect("explicit source correction must still use its current authorized revision");
    let FactoryVerificationRecoveryLaunch::SourceCorrection(launch) = outcome.launch else {
        panic!("a revised checkpoint requires provider correction, not a zero-provider retry");
    };
    assert_eq!(launch.verification_policy, policy);
    assert_eq!(launch.provider_session_id, "fixture-session");
    assert_eq!(launch.workspace_connection_id, Some(CONNECTION));
    let after = correction_state(&store).await;
    assert_eq!(after["source"], before["source"]);
    assert_eq!(
        retained_run(&after, launch.run_id)["budget_tokens_limit"],
        4_700
    );
    assert_eq!(after["task"]["attempt_count"], 2);
    let active_context = store
        .factory_verification_recovery_context(CORP, OWNER, ITEM)
        .await
        .expect("an active correction must retain its exact revised checkpoint context")
        .unwrap();
    assert_eq!(active_context.work_item.id, ITEM);
    assert_eq!(active_context.source_run_id, failed_run_id);
    assert!(active_context.checkpoint_verification);
    assert!(!active_context.checkpoint_verification_available);
    assert!(
        !active_context.checkpoint_source_correction,
        "the active provider must not advertise another correction"
    );
    let active_recovery = active_context
        .recoveries
        .iter()
        .find(|recovery| recovery.id == outcome.recovery.id)
        .expect("context must retain the exact authorized recovery");
    assert_eq!(active_recovery.source_run_id, failed_run_id);
    assert_eq!(active_recovery.replacement_run_id, Some(launch.run_id));
    assert_eq!(
        active_recovery.mode,
        FactoryVerificationRecoveryMode::SourceCorrection
    );
    assert_eq!(correction_state(&store).await, after);
}
