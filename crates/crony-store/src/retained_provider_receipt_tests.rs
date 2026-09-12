//! #211 store-only provenance regressions in SQLx-owned disposable databases.
//! Synthetic metadata is not native receipt-byte, signature, object-store, or
//! provider/runtime acceptance. Every migration and store operation is real.
use super::*;
use crony_domain::{
    RETAINED_COPILOT_RECEIPT_FILE, RetainedProviderReceiptGrant, retained_provider_receipt_metadata,
};

const CONNECTION: Uuid = Uuid::from_u128(211);
const SESSION: &str = "00000000-0000-0000-0000-000000000212";
const RECEIPT: &str = "retained_provider_receipt";
const BINDING: &str = "retained_provider_receipt_upload";

fn receipt_profile() -> CheckpointFixtureProfile {
    CheckpointFixtureProfile {
        adapter: "github-copilot",
        provider_session_id: SESSION,
        native_outcome: Some("completed"),
        workspace_connection_id: Some(CONNECTION),
        ..CheckpointFixtureProfile::default()
    }
}

async fn receipt_fixture(pool: PgPool) -> PgStore {
    fixture_with_profile(pool, true, false, None, false, receipt_profile()).await
}

fn grant(command: &PendingRunnerCommand) -> RetainedProviderReceiptGrant {
    let grant: RetainedProviderReceiptGrant =
        serde_json::from_value(command.payload[RECEIPT].clone()).unwrap();
    grant.validate().unwrap();
    grant
}

fn token(command: &PendingRunnerCommand) -> Uuid {
    serde_json::from_value(command.payload["assignment_token"].clone()).unwrap()
}

fn upload(command: &PendingRunnerCommand) -> (RunnerEventInput, StoredArtifact, String) {
    let metadata = retained_provider_receipt_metadata(&grant(command));
    // No native file is read or claimed validated here. These bytes name only a
    // deterministic store-level digest/size reservation.
    let bytes = b"issue211 synthetic artifact metadata fixture";
    let sha256 = hex::encode(Sha256::digest(bytes));
    let input = event(
        command.run_id,
        token(command),
        "run.artifact_upload",
        json!({
            "artifact_role":"provider_evidence",
            "file_name":RETAINED_COPILOT_RECEIPT_FILE,
            "media_type":"application/json",
            "workspace_relative_path":metadata["workspace_relative_path"],
            "retained_provider_receipt":metadata[RECEIPT],
        }),
    );
    let artifact = StoredArtifact {
        id: input.event_id,
        corp_id: CORP,
        task_id: TASK,
        run_id: command.run_id,
        producer_agent_id: AGENT,
        producer_runner_id: RUNNER.to_owned(),
        verifier: "crony-server:artifact-ingest-v1".to_owned(),
        object_key: format!("corps/{CORP}/sha256/{}/{sha256}", &sha256[..2]),
        uri: format!("/api/corps/{CORP}/artifacts/{}", input.event_id),
        sha256,
        media_type: "application/json".to_owned(),
        bytes: bytes.len() as i64,
        artifact_role: "provider_evidence".to_owned(),
        file_name: RETAINED_COPILOT_RECEIPT_FILE.to_owned(),
        metadata,
        provenance_signature: "0".repeat(64),
        retention_until: Utc::now() + Duration::hours(1),
    };
    let staging = format!("staging/corps/{CORP}/{}", input.event_id);
    (input, artifact, staging)
}

async fn stored_payload(store: &PgStore, command: &PendingRunnerCommand) -> Value {
    sqlx::query_scalar("SELECT payload FROM runner_commands WHERE corp_id=$1 AND id=$2")
        .bind(CORP)
        .bind(command.id)
        .fetch_one(&store.pool)
        .await
        .unwrap()
}

async fn set_payload(store: &PgStore, command: &PendingRunnerCommand, payload: &Value) {
    sqlx::query("UPDATE runner_commands SET payload=$3 WHERE corp_id=$1 AND id=$2")
        .bind(CORP)
        .bind(command.id)
        .bind(payload)
        .execute(&store.pool)
        .await
        .unwrap();
}

async fn source_events(store: &PgStore, run_id: Uuid) -> Value {
    sqlx::query_scalar(
        "SELECT coalesce(jsonb_agg(to_jsonb(event) ORDER BY seq),'[]')
         FROM events event WHERE corp_id=$1 AND aggregate_type='run' AND aggregate_id=$2",
    )
    .bind(CORP)
    .bind(run_id)
    .fetch_one(&store.pool)
    .await
    .unwrap()
}

async fn assert_upload_denied(
    store: &PgStore,
    input: RunnerEventInput,
    artifact: StoredArtifact,
    staging: &str,
) {
    let before = state(store).await;
    assert!(
        store
            .retained_provider_receipt_grant_for_upload(&input)
            .await
            .is_err()
    );
    assert!(
        store
            .prepare_artifact_upload(input, artifact, staging)
            .await
            .is_err()
    );
    assert_eq!(state(store).await, before);
}

async fn fail_checkpoint_without_artifact(store: &PgStore, command: &PendingRunnerCommand) {
    for (kind, payload) in [
        (
            "run.started",
            json!({
                "workspace":"fixture-worktree","workspace_branch":"crony/fixture",
                "workspace_base_ref":"main","workspace_base_commit":"a".repeat(40),
                "execution_mode":"verification_only"
            }),
        ),
        ("run.verification_started", json!({})),
        (
            "run.verification_evidence",
            json!({
                "evidence_id":Uuid::new_v4(),"check_index":0,"kind":"file","status":"passed",
                "summary":"Fixture source file passed","payload":{}
            }),
        ),
        (
            "run.verification_evidence",
            json!({
                "evidence_id":Uuid::new_v4(),"check_index":1,"kind":"artifact","status":"failed",
                "summary":"Native verifier found no ready provider artifact","payload":{}
            }),
        ),
        (
            "run.verification_failed",
            json!({"error":"Provider artifact missing"}),
        ),
        (
            "run.workspace_preserved",
            json!({
                "workspace_fingerprint":"b".repeat(64),"head_commit":"a".repeat(40),
                "detail":"Native verifier preserved the unchanged selected source"
            }),
        ),
    ] {
        store
            .apply_runner_event(event(command.run_id, token(command), kind, payload))
            .await
            .unwrap();
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue211_collection_replays_new_receipt_for_latest_failed_checkpoint(pool: PgPool) {
    let store = receipt_fixture(pool).await;
    let first = checkpoint_command(&store).await;
    fail_checkpoint_without_artifact(&store, &first).await;
    let before = state(&store).await;
    let history = source_events(&store, SOURCE).await;
    let selected_history = source_events(&store, first.run_id).await;
    let mut input = request();
    input.source_run_id = first.run_id;
    input.expected_factory_version = before["item"]["version"].as_i64().unwrap();
    let admitted = store
        .create_factory_verification_recovery(input)
        .await
        .unwrap();
    let run_id = admitted.recovery.replacement_run_id.unwrap();
    let command = store
        .pending_runner_commands(RUNNER)
        .await
        .unwrap()
        .into_iter()
        .find(|command| command.run_id == run_id)
        .unwrap();
    let collected = grant(&command);
    assert_eq!(collected.collection_id, command.id);
    assert_eq!(collected.run_id, run_id);
    assert_eq!(collected.source_run_id, first.run_id);
    assert_eq!(collected.checkpoint_run_id, SOURCE);
    assert_eq!(collected.historical_run_id, SOURCE);
    assert_eq!(collected.provider_session_id, SESSION);
    assert_eq!(collected.historical_model.as_deref(), Some("fixture-model"));
    assert_eq!(
        collected.historical_reasoning_effort.as_deref(),
        Some("medium")
    );
    for (run, id) in [
        (first.run_id, collected.source_checkpoint_event_id),
        (SOURCE, collected.historical_checkpoint_event_id),
    ] {
        let latest: Uuid = sqlx::query_scalar(
            "SELECT id FROM events WHERE corp_id=$1 AND aggregate_type='run'
             AND aggregate_id=$2 AND type='run.workspace_preserved' ORDER BY seq DESC LIMIT 1",
        )
        .bind(CORP)
        .bind(run)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(id, latest);
    }
    assert_ne!(
        collected.source_checkpoint_event_id,
        collected.historical_checkpoint_event_id
    );
    for field in [
        "provider_session_id",
        "model",
        "reasoning_effort",
        "provider_artifact",
    ] {
        assert!(command.payload[field].is_null(), "{field}");
    }
    assert_eq!(command.payload["secret_refs"], json!([]));
    assert_dispatch_read(&store, &command, true).await;
    let (input, artifact, staging) = upload(&command);
    let unchanged = state(&store).await;
    assert_eq!(
        store
            .retained_provider_receipt_grant_for_upload(&input)
            .await
            .unwrap(),
        Some(collected.clone())
    );
    assert_eq!(state(&store).await, unchanged);
    let prepared = store
        .prepare_artifact_upload(input.clone(), artifact.clone(), &staging)
        .await
        .unwrap();
    assert_eq!(prepared.status, "staged");
    let mut bound = stored_payload(&store, &command).await;
    assert_eq!(
        bound[BINDING],
        json!({"sha256":artifact.sha256,"bytes":artifact.bytes})
    );
    bound.as_object_mut().unwrap().remove(BINDING);
    assert_eq!(bound, command.payload);
    // The pre-binding command remains exact through the early dispatch-state
    // seam and the collection-specific dispatch authority check.
    assert_eq!(
        store.runner_command_dispatch_state(&command).await.unwrap(),
        RunnerCommandDispatchState::Pending
    );
    assert_dispatch_read(&store, &command, true).await;
    store
        .acknowledge_runner_command(command.id, RUNNER)
        .await
        .unwrap()
        .unwrap();
    assert_dispatch_read(&store, &command, false).await;
    assert_eq!(
        store
            .retained_provider_receipt_grant_for_upload(&input)
            .await
            .unwrap(),
        Some(collected.clone())
    );
    let ready = store
        .finalize_artifact_upload(CORP, artifact.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ready.event_type, "run.artifact");
    assert_eq!(ready.aggregate_id, run_id);
    assert_eq!(
        ready.payload["metadata"],
        retained_provider_receipt_metadata(&collected)
    );
    for flag in [
        "provider_completion_claimed",
        "provider_inference_started",
        "original_server_artifact_acceptance_claimed",
        "historical_digest_attestation_claimed",
    ] {
        assert_eq!(ready.payload["metadata"][RECEIPT][flag], false);
    }
    let replay_before = state(&store).await;
    assert_eq!(
        store
            .retained_provider_receipt_grant_for_upload(&input)
            .await
            .unwrap(),
        Some(collected)
    );
    let same = store
        .prepare_artifact_upload(input, artifact.clone(), &staging)
        .await
        .unwrap();
    assert_eq!(same.status, "ready");
    assert_eq!(same.artifact.id, artifact.id);
    let (new_event, duplicate, new_staging) = upload(&command);
    let deduped = store
        .prepare_artifact_upload(new_event, duplicate, &new_staging)
        .await
        .unwrap();
    assert_eq!(deduped.artifact.id, artifact.id);
    assert!(
        store
            .finalize_artifact_upload(CORP, artifact.id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(state(&store).await, replay_before);
    // Native verification can now reference this new run's stored artifact.
    store
        .apply_runner_event(event(
            run_id,
            token(&command),
            "run.verification_evidence",
            json!({
                "evidence_id":Uuid::new_v4(),"check_index":1,"kind":"artifact","status":"passed",
                "summary":"Newly collected store fixture receipt",
                "payload":{
                    "path":artifact.uri,"sha256":artifact.sha256,
                    "bytes":artifact.bytes,"media_type":artifact.media_type
                }
            }),
        ))
        .await
        .unwrap();
    let after = state(&store).await;
    assert_eq!(after["source"], before["source"]);
    assert_eq!(
        retained_run(&after, first.run_id),
        retained_run(&before, first.run_id)
    );
    assert_eq!(source_events(&store, SOURCE).await, history);
    assert_eq!(source_events(&store, first.run_id).await, selected_history);
    assert_eq!(
        after["task"]["attempt_count"],
        before["task"]["attempt_count"]
    );
    assert_eq!(
        after["task"]["verification_policy"],
        before["task"]["verification_policy"]
    );
    for field in [
        "budget_tokens",
        "original_budget_tokens",
        "budget_cost_microusd",
        "original_budget_cost_microusd",
    ] {
        assert_eq!(after["mission"][field], before["mission"][field]);
    }
    let replacement = retained_run(&after, run_id);
    for field in [
        "budget_tokens_limit",
        "budget_cost_microusd_limit",
        "input_tokens",
        "output_tokens",
        "cost_microusd",
    ] {
        assert_eq!(replacement[field], 0, "{field}");
    }
    assert_eq!(replacement["artifact_id"], json!(artifact.id));
    assert_eq!(after["artifacts"].as_array().unwrap().len(), 1);
    assert_ne!(replacement["status"], "completed");
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue211_collection_does_not_bypass_unjoined_source_signature(pool: PgPool) {
    let store = receipt_fixture(pool).await;
    // Existing authoritative metadata, even without a ready join, is not absence.
    sqlx::query("UPDATE runs SET artifact_signature=$2 WHERE id=$1")
        .bind(SOURCE)
        .bind("synthetic-existing-signature")
        .execute(&store.pool)
        .await
        .unwrap();
    let source = state(&store).await["source"].clone();
    let command = checkpoint_command(&store).await;
    assert!(command.payload[RECEIPT].is_null());
    assert_eq!(state(&store).await["source"], source);
    let bare = event(
        command.run_id,
        token(&command),
        "run.artifact_upload",
        json!({}),
    );
    assert!(
        store
            .retained_provider_receipt_grant_for_upload(&bare)
            .await
            .is_err()
    );
    let mut marked = bare;
    marked.payload[RECEIPT] = json!({});
    assert!(
        store
            .retained_provider_receipt_grant_for_upload(&marked)
            .await
            .is_err()
    );
    assert!(
        !store
            .retained_provider_receipt_dispatch_authorized(&command)
            .await
            .unwrap()
    );
    assert!(
        state(&store).await["artifacts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue211_collection_requires_a_native_completed_receipt_generation(pool: PgPool) {
    let store = fixture_with_profile(
        pool,
        true,
        false,
        None,
        false,
        CheckpointFixtureProfile {
            native_outcome: Some("cancelled"),
            ..receipt_profile()
        },
    )
    .await;
    let command = checkpoint_command(&store).await;
    assert!(command.payload.get(RECEIPT).is_none());
    assert_dispatch_read(&store, &command, true).await;
    let mut input = event(
        command.run_id,
        token(&command),
        "run.artifact_upload",
        json!({}),
    );
    input.payload[RECEIPT] = Value::Null;
    assert!(
        store
            .retained_provider_receipt_grant_for_upload(&input)
            .await
            .is_err()
    );
    assert!(
        !store
            .retained_provider_receipt_dispatch_authorized(&command)
            .await
            .unwrap()
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue211_collection_rejects_missing_malformed_foreign_grants_and_inputs(pool: PgPool) {
    let store = receipt_fixture(pool).await;
    let command = checkpoint_command(&store).await;
    let (input, artifact, staging) = upload(&command);
    let mut candidates = vec![Value::Null, json!({}), json!({"schema_version":99})];
    for field in [
        "collection_id",
        "corp_id",
        "task_id",
        "run_id",
        "source_run_id",
        "historical_run_id",
    ] {
        let mut candidate = command.payload[RECEIPT].clone();
        candidate[field] = json!(Uuid::new_v4());
        candidates.push(candidate);
    }
    for candidate in candidates {
        let mut payload = command.payload.clone();
        payload[RECEIPT] = candidate;
        set_payload(&store, &command, &payload).await;
        assert_upload_denied(&store, input.clone(), artifact.clone(), &staging).await;
        let altered = PendingRunnerCommand {
            payload,
            ..command.clone()
        };
        assert_dispatch_read(&store, &altered, false).await;
    }
    let mut missing = command.payload.clone();
    missing.as_object_mut().unwrap().remove(RECEIPT);
    set_payload(&store, &command, &missing).await;
    assert_upload_denied(&store, input.clone(), artifact.clone(), &staging).await;
    set_payload(&store, &command, &command.payload).await;
    for (field, value) in [
        (RECEIPT, Value::Null),
        ("workspace_relative_path", json!("other.json")),
        ("file_name", json!("other.json")),
        ("artifact_role", json!("source_deliverable")),
    ] {
        let mut altered = input.clone();
        altered.payload[field] = value;
        assert_upload_denied(&store, altered, artifact.clone(), &staging).await;
    }
    let mut wrong_assignment = input.clone();
    wrong_assignment.assignment_token = Uuid::new_v4();
    assert_upload_denied(&store, wrong_assignment, artifact.clone(), &staging).await;
    let mut wrong_epoch = input.clone();
    wrong_epoch.connection_epoch = Uuid::new_v4();
    assert_upload_denied(&store, wrong_epoch, artifact.clone(), &staging).await;
    let mut old_provider = input;
    old_provider.run_id = SOURCE;
    old_provider.assignment_token = TOKEN;
    let mut old_artifact = artifact;
    old_artifact.run_id = SOURCE;
    assert_upload_denied(&store, old_provider, old_artifact, &staging).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue211_collection_binding_survives_abandon_and_rejects_replacement(pool: PgPool) {
    let store = receipt_fixture(pool).await;
    let command = checkpoint_command(&store).await;
    let (input, artifact, staging) = upload(&command);
    store
        .prepare_artifact_upload(input.clone(), artifact.clone(), &staging)
        .await
        .unwrap();
    let first = stored_payload(&store, &command).await;
    assert_eq!(
        store
            .abandon_staged_artifact(CORP, artifact.id)
            .await
            .unwrap(),
        Some(staging.clone())
    );
    assert_eq!(stored_payload(&store, &command).await, first);
    for change_digest in [true, false] {
        let mut altered = artifact.clone();
        if change_digest {
            altered.sha256 = "f".repeat(64);
        } else {
            altered.bytes += 1;
        }
        let before = state(&store).await;
        assert!(
            store
                .prepare_artifact_upload(input.clone(), altered, &staging)
                .await
                .is_err()
        );
        assert_eq!(state(&store).await, before);
    }
    for invalid in [
        Value::Null,
        json!({}),
        json!({"sha256":"invalid","bytes":1}),
        json!({"sha256":artifact.sha256,"bytes":0}),
        json!({"sha256":artifact.sha256,"bytes":artifact.bytes,"extra":true}),
    ] {
        let mut tampered = first.clone();
        tampered[BINDING] = invalid;
        set_payload(&store, &command, &tampered).await;
        assert_upload_denied(&store, input.clone(), artifact.clone(), &staging).await;
        assert_dispatch_read(&store, &command, false).await;
    }
    set_payload(&store, &command, &first).await;
    assert_dispatch_read(&store, &command, true).await;
    let retry = store
        .prepare_artifact_upload(input, artifact, &staging)
        .await
        .unwrap();
    assert_eq!(retry.status, "staged");
    assert_eq!(stored_payload(&store, &command).await, first);
    assert_eq!(first[RECEIPT], command.payload[RECEIPT]);
}

async fn assert_rejected_finalization(store: &PgStore, artifact_id: Uuid) {
    let before = state(store).await;
    assert!(
        store
            .finalize_artifact_upload(CORP, artifact_id)
            .await
            .is_err()
    );
    let after = state(store).await;
    for field in [
        "runs",
        "task",
        "mission",
        "item",
        "agents",
        "events",
        "commands",
        "recoveries",
    ] {
        assert_eq!(after[field], before[field], "{field}");
    }
    let artifact = after["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == json!(artifact_id))
        .unwrap();
    assert_eq!(artifact["status"], "rejected");
    assert!(!artifact["rejection_reason"].as_str().unwrap().is_empty());
    assert!(
        store
            .finalize_artifact_upload(CORP, artifact_id)
            .await
            .is_err()
    );
    assert_eq!(state(store).await, after);
    assert!(
        !store
            .pending_artifact_uploads(100)
            .await
            .unwrap()
            .iter()
            .any(|row| row.artifact.id == artifact_id && row.status == "staged")
    );
}

async fn revoked_finalization(pool: PgPool, authority: &str) {
    let store = receipt_fixture(pool).await;
    let command = checkpoint_command(&store).await;
    let (input, artifact, staging) = upload(&command);
    store
        .prepare_artifact_upload(input.clone(), artifact.clone(), &staging)
        .await
        .unwrap();
    match authority {
        "role" => {
            sqlx::query("UPDATE actors SET role='member' WHERE corp_id=$1 AND id=$2")
                .bind(CORP)
                .bind(OWNER)
                .execute(&store.pool)
                .await
                .unwrap();
        }
        "room" => {
            sqlx::query("DELETE FROM room_memberships WHERE room_id=$1 AND actor_id=$2")
                .bind(ROOM)
                .bind(OWNER)
                .execute(&store.pool)
                .await
                .unwrap();
        }
        "policy" => {
            sqlx::query("UPDATE tasks SET verification_policy=jsonb_set(verification_policy,'{checks,1,min_bytes}','2') WHERE corp_id=$1 AND id=$2")
                .bind(CORP).bind(TASK).execute(&store.pool).await.unwrap();
        }
        "source" => {
            sqlx::query(
                "UPDATE runs SET source_repository='fixture/foreign' WHERE corp_id=$1 AND id=$2",
            )
            .bind(CORP)
            .bind(SOURCE)
            .execute(&store.pool)
            .await
            .unwrap();
        }
        "checkpoint" => {
            sqlx::query(
                "UPDATE events SET payload=payload-'source_checkpoint'
                 WHERE corp_id=$1 AND id=$2",
            )
            .bind(CORP)
            .bind(grant(&command).historical_checkpoint_event_id)
            .execute(&store.pool)
            .await
            .unwrap();
        }
        _ => unreachable!(),
    }
    assert_upload_denied(&store, input, artifact.clone(), &staging).await;
    assert_dispatch_read(&store, &command, false).await;
    assert_rejected_finalization(&store, artifact.id).await;
    if authority == "role" {
        // Restored authorization cannot auto-adopt an upload already denied.
        sqlx::query("UPDATE actors SET role='owner' WHERE corp_id=$1 AND id=$2")
            .bind(CORP)
            .bind(OWNER)
            .execute(&store.pool)
            .await
            .unwrap();
        let before = state(&store).await;
        assert!(
            store
                .finalize_artifact_upload(CORP, artifact.id)
                .await
                .is_err()
        );
        assert_eq!(state(&store).await, before);
    }
}

macro_rules! revocation_case {
    ($name:ident, $authority:literal) => {
        #[sqlx::test(migrations = "../../db/migrations")]
        #[ignore = "requires explicitly owned SQLx maintenance database"]
        async fn $name(pool: PgPool) {
            revoked_finalization(pool, $authority).await;
        }
    };
}

revocation_case!(issue211_collection_rechecks_role_before_finalize, "role");
revocation_case!(issue211_collection_rechecks_room_before_finalize, "room");
revocation_case!(
    issue211_collection_rechecks_policy_before_finalize,
    "policy"
);
revocation_case!(
    issue211_collection_rechecks_source_before_finalize,
    "source"
);
revocation_case!(
    issue211_collection_rechecks_native_checkpoint_before_finalize,
    "checkpoint"
);

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue211_collection_does_not_override_protected_states_or_explicit_stop(pool: PgPool) {
    let store = receipt_fixture(pool).await;
    let command = checkpoint_command(&store).await;
    let (input, artifact, staging) = upload(&command);
    for protected in ["blocked", "verified", "published", "cancelled"] {
        sqlx::query("UPDATE factory_work_items SET state=$3 WHERE corp_id=$1 AND id=$2")
            .bind(CORP)
            .bind(ITEM)
            .bind(protected)
            .execute(&store.pool)
            .await
            .unwrap();
        assert_upload_denied(&store, input.clone(), artifact.clone(), &staging).await;
        assert_dispatch_read(&store, &command, false).await;
    }
    sqlx::query("UPDATE factory_work_items SET state='running' WHERE corp_id=$1 AND id=$2")
        .bind(CORP)
        .bind(ITEM)
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE runs SET workspace_disposition='quarantined' WHERE corp_id=$1 AND id=$2")
        .bind(CORP)
        .bind(SOURCE)
        .execute(&store.pool)
        .await
        .unwrap();
    assert_upload_denied(&store, input.clone(), artifact.clone(), &staging).await;
    sqlx::query("UPDATE runs SET workspace_disposition='preserved' WHERE corp_id=$1 AND id=$2")
        .bind(CORP)
        .bind(SOURCE)
        .execute(&store.pool)
        .await
        .unwrap();
    store
        .prepare_artifact_upload(input.clone(), artifact.clone(), &staging)
        .await
        .unwrap();
    let stop = store
        .request_emergency_stop(CORP, AGENT, OWNER, "Stop the collector explicitly")
        .await
        .unwrap();
    assert_eq!(stop.run_id, command.run_id);
    assert_eq!(stop.event.event_type, "run.stop_requested");
    assert_upload_denied(&store, input, artifact.clone(), &staging).await;
    assert_dispatch_read(&store, &command, false).await;
    assert_rejected_finalization(&store, artifact.id).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue211_collection_reservation_and_finalization_roll_back_sqlx_failures(pool: PgPool) {
    let store = receipt_fixture(pool).await;
    sqlx::raw_sql(
        "CREATE FUNCTION issue211_fault() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'owned issue211 transaction fault'; END $$;
         CREATE TRIGGER issue211_command_fault BEFORE INSERT ON runner_commands
         FOR EACH ROW EXECUTE FUNCTION issue211_fault();",
    )
    .execute(&store.pool)
    .await
    .unwrap();
    rejected_without_changes(&store, request()).await;
    sqlx::raw_sql("DROP TRIGGER issue211_command_fault ON runner_commands")
        .execute(&store.pool)
        .await
        .unwrap();
    let command = checkpoint_command(&store).await;
    let (input, artifact, staging) = upload(&command);
    sqlx::raw_sql(
        "CREATE TRIGGER issue211_artifact_fault BEFORE INSERT ON artifacts
         FOR EACH ROW EXECUTE FUNCTION issue211_fault();",
    )
    .execute(&store.pool)
    .await
    .unwrap();
    let before = state(&store).await;
    let error = store
        .prepare_artifact_upload(input.clone(), artifact.clone(), &staging)
        .await
        .unwrap_err();
    assert!(error.downcast_ref::<sqlx::Error>().is_some());
    assert_eq!(state(&store).await, before);
    assert!(
        stored_payload(&store, &command)
            .await
            .get(BINDING)
            .is_none()
    );
    sqlx::raw_sql("DROP TRIGGER issue211_artifact_fault ON artifacts")
        .execute(&store.pool)
        .await
        .unwrap();
    store
        .prepare_artifact_upload(input, artifact.clone(), &staging)
        .await
        .unwrap();
    sqlx::raw_sql(
        "CREATE TRIGGER issue211_event_fault BEFORE INSERT ON events
         FOR EACH ROW EXECUTE FUNCTION issue211_fault();",
    )
    .execute(&store.pool)
    .await
    .unwrap();
    let before = state(&store).await;
    let error = store
        .finalize_artifact_upload(CORP, artifact.id)
        .await
        .unwrap_err();
    assert!(error.downcast_ref::<sqlx::Error>().is_some());
    assert_eq!(state(&store).await, before);
    sqlx::raw_sql("DROP TRIGGER issue211_event_fault ON events")
        .execute(&store.pool)
        .await
        .unwrap();
    assert!(
        store
            .finalize_artifact_upload(CORP, artifact.id)
            .await
            .unwrap()
            .is_some()
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue211_collection_takes_both_native_gates_before_artifact_rows(pool: PgPool) {
    use std::future::Future;
    use std::task::Poll;

    let store = receipt_fixture(pool).await;
    let command = checkpoint_command(&store).await;
    let (input, artifact, staging) = upload(&command);
    // Typed options refer only to this SQLx-owned fixture DB, never ambient
    // DATABASE_URL. Bound the dedicated waiter, not global database settings.
    let waiter_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect_with(store.pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    sqlx::query("SET statement_timeout='15s'")
        .execute(&waiter_pool)
        .await
        .unwrap();
    let waiter_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&waiter_pool)
        .await
        .unwrap();
    let waiter = PgStore {
        pool: waiter_pool.clone(),
    };
    for reserve in [true, false] {
        for prefix in ["factory:item", "publication:factory"] {
            let mut blocker = store.pool.begin().await.unwrap();
            let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *blocker)
                .await
                .unwrap();
            lock_factory_keys_tx(&mut blocker, &[format!("{prefix}:{CORP}:{ITEM}")])
                .await
                .unwrap();
            let mut operation = Box::pin(async {
                if reserve {
                    waiter
                        .prepare_artifact_upload(input.clone(), artifact.clone(), &staging)
                        .await?;
                } else {
                    waiter.finalize_artifact_upload(CORP, artifact.id).await?;
                }
                Ok::<(), anyhow::Error>(())
            });
            let mut probe = Box::pin(async {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                loop {
                    let waiting: bool = sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM pg_stat_activity activity
                         WHERE activity.datname=current_database() AND activity.pid=$1
                           AND $2=ANY(pg_blocking_pids(activity.pid))
                           AND EXISTS(SELECT 1 FROM pg_locks pending_lock
                             WHERE pending_lock.pid=activity.pid AND pending_lock.locktype='advisory'
                               AND NOT pending_lock.granted))",
                    ).bind(waiter_pid).bind(blocker_pid).fetch_one(&store.pool).await?;
                    if waiting {
                        for table in ["runs", "tasks", "missions"] {
                            // Fixed fixture tables; proves this operation waits
                            // before these rows, not global deadlock freedom.
                            sqlx::query(&format!("SELECT id FROM {table} FOR UPDATE NOWAIT"))
                                .fetch_all(&mut *blocker)
                                .await?;
                        }
                        blocker.commit().await?;
                        return Ok::<(), anyhow::Error>(());
                    }
                    if std::time::Instant::now() >= deadline {
                        return Err(anyhow!(
                            "collector did not wait at its native {prefix} gate"
                        ));
                    }
                }
            });
            let mut finished = None;
            std::future::poll_fn(|cx| {
                if finished.is_none()
                    && let Poll::Ready(result) = operation.as_mut().poll(cx)
                {
                    finished = Some(result);
                }
                probe.as_mut().poll(cx)
            })
            .await
            .unwrap();
            match finished {
                Some(result) => result,
                None => operation.await,
            }
            .unwrap();
        }
    }
    waiter_pool.close().await;
    assert_eq!(state(&store).await["artifacts"][0]["status"], "ready");
}

async fn corrected_then_stopped(
    pool: PgPool,
) -> (PgStore, Uuid, Uuid, CreateFactoryVerificationRecoveryInput) {
    let store = fixture_with_profile(
        pool,
        true,
        false,
        None,
        false,
        CheckpointFixtureProfile {
            mission_tokens: 10_000,
            run_tokens: 5_000,
            used_tokens: 5_300,
            attempt_count: 1,
            expected_stage: "suspend",
            ..receipt_profile()
        },
    )
    .await;
    let failed = checkpoint_command(&store).await;
    fail_checkpoint_without_artifact(&store, &failed).await;
    let before = state(&store).await;
    let mut contract: TaskContract =
        serde_json::from_value(before["task"]["contract"].clone()).unwrap();
    contract
        .prohibited_actions
        .push("Do not replace the preserved application".to_owned());
    let mut revised_policy = before["task"]["verification_policy"].clone();
    revised_policy["checks"][1]["min_bytes"] = json!(2);
    let policy: VerificationPolicy = serde_json::from_value(revised_policy).unwrap();
    let revision = store
        .create_mission_contract_revision(CreateMissionContractRevisionInput {
            corp_id: CORP,
            mission_id: MISSION,
            task_id: TASK,
            actor_id: OWNER,
            expected_contract_version: before["task"]["contract_version"].as_i64().unwrap(),
            next_action: MissionContractRevisionAction::Resume,
            source_run_id: Some(failed.run_id),
            reason: "Explicit correction with a strengthened current artifact policy".to_owned(),
            idempotency_key: Uuid::new_v4(),
            description: "Correct retained source only".to_owned(),
            contract: contract.clone(),
            verification_policy: policy.clone(),
        })
        .await
        .unwrap();
    let mut correction = request();
    correction.source_run_id = failed.run_id;
    correction.expected_factory_version = state(&store).await["item"]["version"].as_i64().unwrap();
    correction.mode = FactoryVerificationRecoveryMode::SourceCorrection;
    correction.contract_revision_id = Some(revision.revision.id);
    let corrected = store
        .create_factory_verification_recovery(correction)
        .await
        .unwrap();
    let correction_id = corrected.recovery.id;
    let FactoryVerificationRecoveryLaunch::SourceCorrection(launch) = corrected.launch else {
        panic!("the historical suspension needs an explicitly budgeted provider correction");
    };
    assert_eq!(launch.provider_session_id, SESSION);
    let allocated = state(&store).await;
    assert_eq!(
        retained_run(&allocated, launch.run_id)["budget_tokens_limit"],
        4_700
    );
    assert_eq!(allocated["task"]["attempt_count"], 2);
    assert_eq!(allocated["source"], before["source"]);
    store.apply_runner_event(event(launch.run_id, launch.assignment_token, "run.started", json!({
        "adapter":"github-copilot","workspace":"fixture-worktree","workspace_branch":"crony/fixture",
        "workspace_base_ref":"main","workspace_base_commit":"a".repeat(40),"execution_mode":"provider"
    }))).await.unwrap();
    store
        .apply_runner_event(event(
            launch.run_id,
            launch.assignment_token,
            "run.usage",
            json!({
                // A 4,700-token allocation stops at 110%; 4,800 only suspends.
                "input_tokens":6_000,"output_tokens":0,"cost_microusd":0
            }),
        ))
        .await
        .unwrap();
    let stopped = store
        .evaluate_circuit_breaker(CORP, launch.run_id)
        .await
        .unwrap();
    assert_eq!(stopped.event.unwrap().payload["stage"], "stop");
    store
        .apply_runner_event(event(
            launch.run_id,
            launch.assignment_token,
            "run.session_terminated",
            json!({
                "adapter":"github-copilot","outcome":"cancelled","provider_process_alive":false
            }),
        ))
        .await
        .unwrap();
    let proof = StoppedSourceCheckpoint {
        schema_version: 1,
        corp_id: CORP,
        mission_id: MISSION,
        task_id: TASK,
        run_id: launch.run_id,
        workspace_run_id: SOURCE,
        agent_id: AGENT,
        runner_id: RUNNER.to_owned(),
        source_repository: "fixture/source".to_owned(),
        source_base_ref: "main".to_owned(),
        source_base_commit: "a".repeat(40),
        workspace_base_commit: "a".repeat(40),
        branch: "crony/fixture".to_owned(),
        head_commit: "c".repeat(40),
        workspace_fingerprint: "d".repeat(64),
        verification_policy_sha256: digest(&policy),
        write_scope_sha256: digest(&contract.write_scope),
        deliverable_policy_sha256: digest(&contract.deliverable),
    };
    store
        .apply_runner_event(event(
            launch.run_id,
            launch.assignment_token,
            "run.workspace_preserved",
            json!({
                "workspace":"fixture-worktree","workspace_branch":"crony/fixture",
                "workspace_base_ref":"main","workspace_base_commit":"a".repeat(40),
                "workspace_fingerprint":"d".repeat(64),"head_commit":"c".repeat(40),
                "source_checkpoint":proof,"detail":"Native corrected provider stopped and preserved"
            }),
        ))
        .await
        .unwrap();
    store
        .apply_runner_event(event(
            launch.run_id,
            launch.assignment_token,
            "run.cancelled",
            json!({
                "reason":"Native current mission budget exhausted"
            }),
        ))
        .await
        .unwrap();
    let after = state(&store).await;
    assert_eq!(after["source"], before["source"]);
    assert_eq!(after["source"]["breaker_stage"], "suspend");
    assert_eq!(retained_run(&after, launch.run_id)["breaker_stage"], "stop");
    let mut collection = request();
    collection.source_run_id = launch.run_id;
    collection.expected_factory_version = after["item"]["version"].as_i64().unwrap();
    collection.expected_workspace_fingerprint = "d".repeat(64);
    collection.expected_head_commit = Some("c".repeat(40));
    (store, launch.run_id, correction_id, collection)
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue211_collection_preserves_authorized_historical_suspension_and_policy(pool: PgPool) {
    let (store, stopped, _, input) = corrected_then_stopped(pool).await;
    let before = state(&store).await;
    let history = source_events(&store, SOURCE).await;
    let stopped_history = source_events(&store, stopped).await;
    let admitted = store
        .create_factory_verification_recovery(input)
        .await
        .unwrap();
    let run_id = admitted.recovery.replacement_run_id.unwrap();
    let command = store
        .pending_runner_commands(RUNNER)
        .await
        .unwrap()
        .into_iter()
        .find(|command| command.run_id == run_id)
        .unwrap();
    let collected = grant(&command);
    assert_eq!(collected.source_run_id, stopped);
    assert_eq!(collected.checkpoint_run_id, stopped);
    assert_eq!(collected.historical_run_id, SOURCE);
    assert_eq!(collected.expected_workspace_fingerprint, "d".repeat(64));
    assert_eq!(collected.expected_head_commit, "c".repeat(40));
    assert_ne!(
        collected.source_checkpoint_event_id,
        collected.historical_checkpoint_event_id
    );
    let old_proof = history
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["id"] == json!(collected.historical_checkpoint_event_id))
        .unwrap();
    assert_ne!(
        old_proof["payload"]["source_checkpoint"]["verification_policy_sha256"],
        json!(digest(
            &serde_json::from_value::<VerificationPolicy>(
                before["task"]["verification_policy"].clone()
            )
            .unwrap()
        ))
    );
    assert_dispatch_read(&store, &command, true).await;
    let (input, artifact, staging) = upload(&command);
    assert_eq!(
        store
            .retained_provider_receipt_grant_for_upload(&input)
            .await
            .unwrap(),
        Some(collected)
    );
    store
        .prepare_artifact_upload(input, artifact.clone(), &staging)
        .await
        .unwrap();
    let ready = store
        .finalize_artifact_upload(CORP, artifact.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ready.aggregate_id, run_id);
    let after = state(&store).await;
    assert_eq!(after["source"], before["source"]);
    assert_eq!(
        retained_run(&after, stopped),
        retained_run(&before, stopped)
    );
    assert_eq!(source_events(&store, SOURCE).await, history);
    assert_eq!(source_events(&store, stopped).await, stopped_history);
    assert_eq!(after["task"]["attempt_count"], 2);
    assert_eq!(
        after["task"]["verification_policy"],
        before["task"]["verification_policy"]
    );
    assert_eq!(retained_run(&after, run_id)["budget_tokens_limit"], 0);
    assert_eq!(retained_run(&after, run_id)["input_tokens"], 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue211_collection_rejects_unrelated_suspension_or_corrupt_historical_authority(
    pool: PgPool,
) {
    let (store, _, correction_id, input) = corrected_then_stopped(pool).await;
    let saved: Value = sqlx::query_scalar(
        "SELECT source_correction_authority FROM factory_verification_recoveries WHERE corp_id=$1 AND id=$2",
    ).bind(CORP).bind(correction_id).fetch_one(&store.pool).await.unwrap();
    let mut corrupt = saved.clone();
    corrupt["previous_policy_sha256"] = json!("f".repeat(64));
    sqlx::query("UPDATE factory_verification_recoveries SET source_correction_authority=$3 WHERE corp_id=$1 AND id=$2")
        .bind(CORP).bind(correction_id).bind(corrupt).execute(&store.pool).await.unwrap();
    rejected_without_changes(&store, input.clone()).await;
    sqlx::query("UPDATE factory_verification_recoveries SET source_correction_authority=NULL WHERE corp_id=$1 AND id=$2")
        .bind(CORP).bind(correction_id).execute(&store.pool).await.unwrap();
    rejected_without_changes(&store, input.clone()).await;
    sqlx::query("UPDATE factory_verification_recoveries SET source_correction_authority=$3 WHERE corp_id=$1 AND id=$2")
        .bind(CORP).bind(correction_id).bind(saved).execute(&store.pool).await.unwrap();
    let predecessor: Uuid = sqlx::query_scalar(
        "SELECT source_run_id FROM factory_verification_recoveries WHERE corp_id=$1 AND id=$2",
    )
    .bind(CORP)
    .bind(correction_id)
    .fetch_one(&store.pool)
    .await
    .unwrap();
    sqlx::query("UPDATE runs SET breaker_stage='suspend' WHERE corp_id=$1 AND id=$2")
        .bind(CORP)
        .bind(predecessor)
        .execute(&store.pool)
        .await
        .unwrap();
    rejected_without_changes(&store, input).await;
    // This is an unrelated checkpoint child, not the exact proven provider
    // suspension. No limits, attempts, usage, or original fixture history reset.
    assert_ne!(predecessor, SOURCE);
    assert_eq!(state(&store).await["source"]["input_tokens"], 5_300);
}
