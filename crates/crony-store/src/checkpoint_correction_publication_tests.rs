//! #216 baseline in a SQLx-owned disposable database.
//! Every store transition is real; receipt/export bytes, native-format events,
//! signatures and review are explicitly synthetic fixture metadata. This is not
//! provider execution, physical source verification, Git or runtime acceptance.
use super::*;
use crony_domain::{
    RETAINED_COPILOT_RECEIPT_FILE, RetainedProviderReceiptGrant, retained_provider_receipt_metadata,
};

const CONNECTION: Uuid = Uuid::from_u128(216);
const SESSION: &str = "00000000-0000-0000-0000-000000000217";

struct PublicationFixture {
    store: PgStore,
    input: StartPullRequestPublicationInput,
    failed_run_id: Uuid,
    corrected_run_id: Uuid,
    verifier_run_id: Uuid,
    correction_recovery_id: Uuid,
    checkpoint_recovery_id: Uuid,
    checkpoint_event_id: Uuid,
    termination_event_id: Uuid,
    historical_termination_event_id: Uuid,
    publisher_credential_id: Uuid,
}

fn fixture_artifact(
    input: &RunnerEventInput,
    bytes: &[u8],
    role: &str,
    file_name: &str,
    media_type: &str,
    metadata: Value,
) -> StoredArtifact {
    let sha256 = hex::encode(Sha256::digest(bytes));
    StoredArtifact {
        id: input.event_id,
        corp_id: CORP,
        task_id: TASK,
        run_id: input.run_id,
        producer_agent_id: AGENT,
        producer_runner_id: RUNNER.to_owned(),
        verifier: "crony-server:artifact-ingest-v1".to_owned(),
        object_key: format!("corps/{CORP}/sha256/{}/{sha256}", &sha256[..2]),
        uri: format!("/api/corps/{CORP}/artifacts/{}", input.event_id),
        sha256,
        media_type: media_type.to_owned(),
        bytes: i64::try_from(bytes.len()).unwrap(),
        artifact_role: role.to_owned(),
        file_name: file_name.to_owned(),
        metadata,
        provenance_signature: "0".repeat(64),
        retention_until: Utc::now() + Duration::hours(1),
    }
}

fn run_journal(snapshot: &Value, run_id: Uuid) -> Vec<Value> {
    snapshot["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["aggregate_type"] == "run" && event["aggregate_id"] == json!(run_id))
        .cloned()
        .collect()
}

async fn corrected_publication_fixture(pool: PgPool) -> PublicationFixture {
    // Publication and export authority exist before the original source runs.
    // Do not widen a stopped contract, reset usage, or replace the mission.
    let store = fixture_with_profile(
        pool,
        true,
        false,
        Some(DeliverableSpec {
            form: DeliverableForm::CommitBranch,
            commit_after_verification: true,
            paths: vec!["result.md".to_owned()],
        }),
        true,
        CheckpointFixtureProfile {
            mission_tokens: 10_000,
            run_tokens: 5_000,
            used_tokens: 5_300,
            attempt_count: 1,
            expected_stage: "suspend",
            workspace_connection_id: Some(CONNECTION),
            adapter: "github-copilot",
            provider_session_id: SESSION,
            native_outcome: Some("completed"),
            ..CheckpointFixtureProfile::default()
        },
    )
    .await;
    let original = state(&store).await;
    assert_eq!(original["source"]["breaker_stage"], "suspend");
    assert_eq!(original["source"]["input_tokens"], 5_300);
    assert!(original["source"]["artifact_id"].is_null());

    // The first source-only verifier fails its genuinely missing artifact check.
    let failed = checkpoint_command(&store).await;
    let failed_token =
        Uuid::parse_str(failed.payload["assignment_token"].as_str().unwrap()).unwrap();
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
                "evidence_id":Uuid::new_v4(),"check_index":0,"kind":"file",
                "status":"passed","summary":"SQLx fixture source-file metadata","payload":{}
            }),
        ),
        (
            "run.verification_evidence",
            json!({
                "evidence_id":Uuid::new_v4(),"check_index":1,"kind":"artifact",
                "status":"failed","summary":"No ready provider artifact exists","payload":{}
            }),
        ),
        (
            "run.verification_failed",
            json!({"error":"Required provider artifact is missing"}),
        ),
        (
            "run.workspace_preserved",
            json!({
                "workspace_fingerprint":"b".repeat(64),"head_commit":"a".repeat(40),
                "detail":"SQLx failed-verifier preservation metadata"
            }),
        ),
    ] {
        store
            .apply_runner_event(event(failed.run_id, failed_token, kind, payload))
            .await
            .unwrap();
    }
    let before_revision = state(&store).await;
    assert_eq!(before_revision["item"]["state"], "verification_failed");
    assert_eq!(before_revision["task"]["attempt_count"], 1);
    assert_eq!(before_revision["source"], original["source"]);

    // Match the existing #211 corrected-then-stopped seam, including its
    // narrowed policy. The historical receipt is not a new completion claim.
    let mut contract: TaskContract =
        serde_json::from_value(before_revision["task"]["contract"].clone()).unwrap();
    contract
        .prohibited_actions
        .push("Do not replace the preserved application".to_owned());
    let mut revised_policy = before_revision["task"]["verification_policy"].clone();
    revised_policy["checks"][1]["min_bytes"] = json!(2);
    let policy: VerificationPolicy = serde_json::from_value(revised_policy).unwrap();
    let revision = store
        .create_mission_contract_revision(CreateMissionContractRevisionInput {
            corp_id: CORP,
            mission_id: MISSION,
            task_id: TASK,
            actor_id: OWNER,
            expected_contract_version: before_revision["task"]["contract_version"]
                .as_i64()
                .unwrap(),
            next_action: MissionContractRevisionAction::Resume,
            source_run_id: Some(failed.run_id),
            reason: "Explicit fixture correction after a failed checkpoint".to_owned(),
            idempotency_key: Uuid::new_v4(),
            description: "Correct the same retained source under narrowed authority".to_owned(),
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
    correction.reason = "Explicitly authorize the remaining budgeted correction attempt".to_owned();
    let corrected = store
        .create_factory_verification_recovery(correction)
        .await
        .unwrap();
    let correction_recovery_id = corrected.recovery.id;
    let FactoryVerificationRecoveryLaunch::SourceCorrection(correction_launch) = corrected.launch
    else {
        panic!("explicit correction must retain the original provider session");
    };
    let corrected_run_id = correction_launch.run_id;
    assert_eq!(correction_launch.provider_session_id, SESSION);
    let allocated = state(&store).await;
    assert_eq!(
        retained_run(&allocated, corrected_run_id)["budget_tokens_limit"],
        4_700
    );
    assert_eq!(allocated["task"]["attempt_count"], 2);
    assert_eq!(allocated["source"], original["source"]);
    store
        .apply_runner_event(event(
            corrected_run_id,
            correction_launch.assignment_token,
            "run.started",
            json!({
                "adapter":"github-copilot","workspace":"fixture-worktree",
                "workspace_branch":"crony/fixture","workspace_base_ref":"main",
                "workspace_base_commit":"a".repeat(40),"execution_mode":"provider"
            }),
        ))
        .await
        .unwrap();
    store
        .apply_runner_event(event(
            corrected_run_id,
            correction_launch.assignment_token,
            "run.usage",
            json!({"input_tokens":6_000,"output_tokens":0,"cost_microusd":0}),
        ))
        .await
        .unwrap();
    let stopped = store
        .evaluate_circuit_breaker(CORP, corrected_run_id)
        .await
        .unwrap();
    assert_eq!(stopped.event.unwrap().payload["stage"], "stop");
    let terminated = event(
        corrected_run_id,
        correction_launch.assignment_token,
        "run.session_terminated",
        json!({
            "adapter":"github-copilot","outcome":"cancelled","provider_process_alive":false
        }),
    );
    let termination_event_id = terminated.event_id;
    store.apply_runner_event(terminated).await.unwrap();
    let proof = StoppedSourceCheckpoint {
        schema_version: 1,
        corp_id: CORP,
        mission_id: MISSION,
        task_id: TASK,
        run_id: corrected_run_id,
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
    let preserved = event(
        corrected_run_id,
        correction_launch.assignment_token,
        "run.workspace_preserved",
        json!({
            "workspace":"fixture-worktree","workspace_branch":"crony/fixture",
            "workspace_base_ref":"main","workspace_base_commit":"a".repeat(40),
            "workspace_fingerprint":"d".repeat(64),"head_commit":"c".repeat(40),
            "source_checkpoint":proof,"detail":"SQLx corrected stopped-source metadata"
        }),
    );
    let checkpoint_event_id = preserved.event_id;
    store.apply_runner_event(preserved).await.unwrap();
    store
        .apply_runner_event(event(
            corrected_run_id,
            correction_launch.assignment_token,
            "run.cancelled",
            json!({"reason":"Native current mission budget exhausted"}),
        ))
        .await
        .unwrap();
    let stopped_state = state(&store).await;
    assert_eq!(stopped_state["source"], original["source"]);
    assert_eq!(stopped_state["task"]["attempt_count"], 2);
    assert_eq!(stopped_state["task"]["max_attempts"], 2);
    assert_eq!(
        retained_run(&stopped_state, corrected_run_id)["breaker_stage"],
        "stop"
    );
    assert_eq!(
        retained_run(&stopped_state, corrected_run_id)["input_tokens"],
        6_000
    );

    // This second checkpoint verifies the correction's source without allocating
    // another model attempt or treating the cancelled provider as completed.
    let mut collection = request();
    collection.source_run_id = corrected_run_id;
    collection.expected_factory_version = stopped_state["item"]["version"].as_i64().unwrap();
    collection.expected_workspace_fingerprint = "d".repeat(64);
    collection.expected_head_commit = Some("c".repeat(40));
    let collected = store
        .create_factory_verification_recovery(collection)
        .await
        .unwrap();
    let checkpoint_recovery_id = collected.recovery.id;
    let FactoryVerificationRecoveryLaunch::VerifierOnly(verifier) = collected.launch else {
        panic!("retained receipt collection must not start another provider");
    };
    let command = store
        .pending_runner_commands(RUNNER)
        .await
        .unwrap()
        .into_iter()
        .find(|command| command.run_id == verifier.run_id)
        .unwrap();
    let grant: RetainedProviderReceiptGrant =
        serde_json::from_value(command.payload["retained_provider_receipt"].clone()).unwrap();
    grant.validate().unwrap();
    assert_eq!(grant.collection_id, command.id);
    assert_eq!(grant.run_id, verifier.run_id);
    assert_eq!(grant.source_run_id, corrected_run_id);
    assert_eq!(grant.checkpoint_run_id, corrected_run_id);
    assert_eq!(grant.workspace_run_id, SOURCE);
    assert_eq!(grant.historical_run_id, SOURCE);
    assert_eq!(grant.source_checkpoint_event_id, checkpoint_event_id);
    assert_ne!(
        grant.historical_checkpoint_event_id,
        grant.source_checkpoint_event_id
    );
    assert_eq!(grant.provider_session_id, SESSION);
    for field in [
        "model",
        "reasoning_effort",
        "provider_session_id",
        "provider_artifact",
    ] {
        assert!(command.payload[field].is_null(), "{field}");
    }
    assert_eq!(command.payload["secret_refs"], json!([]));
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
                "summary":"SQLx current source-file check metadata","payload":{}
            }),
        ),
    ] {
        store
            .apply_runner_event(event(
                verifier.run_id,
                verifier.assignment_token,
                kind,
                payload,
            ))
            .await
            .unwrap();
    }
    let receipt_metadata = retained_provider_receipt_metadata(&grant);
    let receipt_input = event(
        verifier.run_id,
        verifier.assignment_token,
        "run.artifact_upload",
        json!({
            "artifact_role":"provider_evidence","file_name":RETAINED_COPILOT_RECEIPT_FILE,
            "media_type":"application/json",
            "workspace_relative_path":receipt_metadata["workspace_relative_path"],
            "retained_provider_receipt":receipt_metadata["retained_provider_receipt"]
        }),
    );
    // Deliberately store-only bytes. The server's native JSON/signature boundary
    // is not invoked or claimed tested by this actual-store publication baseline.
    let receipt = fixture_artifact(
        &receipt_input,
        b"issue216 synthetic retained receipt metadata",
        "provider_evidence",
        RETAINED_COPILOT_RECEIPT_FILE,
        "application/json",
        receipt_metadata,
    );
    assert_eq!(
        store
            .retained_provider_receipt_grant_for_upload(&receipt_input)
            .await
            .unwrap(),
        Some(grant.clone())
    );
    store
        .prepare_artifact_upload(
            receipt_input,
            receipt.clone(),
            &format!("staging/corps/{CORP}/{}", receipt.id),
        )
        .await
        .unwrap();
    let receipt_ready = store
        .finalize_artifact_upload(CORP, receipt.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt_ready.event_type, "run.artifact");
    assert_eq!(receipt_ready.aggregate_id, verifier.run_id);
    for flag in [
        "provider_completion_claimed",
        "provider_inference_started",
        "original_server_artifact_acceptance_claimed",
        "historical_digest_attestation_claimed",
    ] {
        assert_eq!(
            receipt_ready.payload["metadata"]["retained_provider_receipt"][flag], false,
            "{flag}"
        );
    }
    store
        .apply_runner_event(event(
            verifier.run_id,
            verifier.assignment_token,
            "run.verification_evidence",
            json!({
                "evidence_id":Uuid::new_v4(),"check_index":1,"kind":"artifact","status":"passed",
                "summary":"Current verifier references its newly ready fixture receipt",
                "payload":{
                    "path":receipt.uri,"sha256":receipt.sha256,
                    "bytes":receipt.bytes,"media_type":receipt.media_type
                }
            }),
        ))
        .await
        .unwrap();

    // Produce the existing native-store commit-branch export shape, not a Git
    // push or fabricated runtime bundle. Its verified HEAD differs from the
    // corrected provider's preserved HEAD, while the immutable base stays fixed.
    let export_input = event(
        verifier.run_id,
        verifier.assignment_token,
        "run.deliverable_upload",
        json!({}),
    );
    let export = fixture_artifact(
        &export_input,
        br#"{"fixture":"issue216 SQLx-only commit-branch metadata"}"#,
        "source_deliverable",
        "ecorp-commit-branch.json",
        "application/vnd.ecorp.deliverable+json",
        json!({
            "form":"commit_branch","verification_sha256":"e".repeat(64),
            "base_commit":"a".repeat(40),"head_commit":"f".repeat(40),
            "branch":"crony/fixture","integration_state":"ready_for_review",
            "git_bundle_sha256":hex::encode(Sha256::digest(b"issue216 SQLx-only bundle metadata")),
            "publication_ready":true
        }),
    );
    store
        .prepare_artifact_upload(
            export_input,
            export.clone(),
            &format!("staging/corps/{CORP}/{}", export.id),
        )
        .await
        .unwrap();
    store
        .finalize_artifact_upload(CORP, export.id)
        .await
        .unwrap()
        .unwrap();
    for (kind, payload) in [
        (
            "run.verification_passed",
            json!({
                "summary":"All current SQLx fixture checks passed",
                "verification_sha256":"e".repeat(64),"deliverable_sha256":export.sha256
            }),
        ),
        (
            "run.verification_waiting",
            json!({"gate":gate(),"gate_type":"independent_review"}),
        ),
        (
            "run.workspace_preserved",
            json!({
                "workspace":"fixture-worktree","workspace_branch":"crony/fixture",
                "workspace_base_ref":"main","workspace_base_commit":"a".repeat(40),
                "workspace_fingerprint":"d".repeat(64),"head_commit":"f".repeat(40),
                "workspace_quarantined":false,
                "detail":"SQLx verifier retained the checked export"
            }),
        ),
    ] {
        store
            .apply_runner_event(event(
                verifier.run_id,
                verifier.assignment_token,
                kind,
                payload,
            ))
            .await
            .unwrap();
    }
    store
        .acknowledge_runner_command(command.id, RUNNER)
        .await
        .unwrap();
    store
        .decide_verification(
            CORP,
            verifier.run_id,
            REVIEWER,
            true,
            "Independent SQLx fixture review of the current export",
            Some(Uuid::new_v4()),
        )
        .await
        .unwrap();
    let publisher_hash = digest(&"issue216 SQLx-only publisher credential");
    let publisher = store
        .create_publication_publisher_credential(
            CORP,
            OWNER,
            "issue216-store-publisher",
            &publisher_hash,
            Utc::now() + Duration::hours(1),
        )
        .await
        .unwrap();
    let deliverable_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM source_deliverables WHERE corp_id=$1 AND artifact_id=$2",
    )
    .bind(CORP)
    .bind(export.id)
    .fetch_one(&store.pool)
    .await
    .unwrap();
    let before = publication_state(&store).await;
    assert_eq!(before["item"]["state"], "verified");
    assert_eq!(before["mission"]["status"], "completed");
    assert_eq!(before["task"]["status"], "completed");
    assert_eq!(before["task"]["attempt_count"], 2);
    assert_eq!(before["task"]["max_attempts"], 2);
    assert_eq!(before["runs"].as_array().unwrap().len(), 4);
    assert_eq!(before["source"], original["source"]);
    assert_eq!(
        retained_run(&before, corrected_run_id),
        retained_run(&stopped_state, corrected_run_id)
    );
    assert_eq!(run_journal(&before, SOURCE), run_journal(&original, SOURCE));
    assert_eq!(
        run_journal(&before, corrected_run_id),
        run_journal(&stopped_state, corrected_run_id)
    );
    let checked = retained_run(&before, verifier.run_id);
    assert_eq!(checked["status"], "completed");
    assert_eq!(checked["execution_mode"], "verification_only");
    assert_eq!(checked["workspace_disposition"], "preserved");
    assert!(checked["model"].is_null());
    assert!(checked["provider_session_id"].is_null());
    for field in [
        "budget_tokens_limit",
        "budget_cost_microusd_limit",
        "input_tokens",
        "output_tokens",
        "cost_microusd",
    ] {
        assert_eq!(checked[field], 0, "{field}");
    }
    assert_eq!(checked["artifact_id"], json!(receipt.id));
    assert!(
        before["recoveries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|recovery| {
                recovery["replacement_run_id"] == json!(verifier.run_id)
                    && recovery["status"] == "completed"
            })
    );

    let input = StartPullRequestPublicationInput {
        corp_id: CORP,
        work_item_id: ITEM,
        actor_id: OWNER,
        actor_role: "owner".to_owned(),
        source_deliverable_id: deliverable_id,
        target_repository: "fixture/source".to_owned(),
        base_ref: "main".to_owned(),
        branch: "ecorp/issue216-checkpoint".to_owned(),
        title: "Reviewed corrected-checkpoint fixture".to_owned(),
        body: "Closes https://github.com/fixture/source/issues/148\nSQLx metadata only.".to_owned(),
        authorization_id: Uuid::new_v4(),
        authorization_reason: "Publish only this independently reviewed retained export".to_owned(),
        effect_key: Uuid::new_v4().to_string(),
        idempotency_key: Uuid::new_v4().to_string(),
        publisher_id: "issue216-store-publisher".to_owned(),
        publisher_credential_hash: publisher_hash,
        lease_seconds: 300,
    };
    PublicationFixture {
        store,
        input,
        failed_run_id: failed.run_id,
        corrected_run_id,
        verifier_run_id: verifier.run_id,
        correction_recovery_id,
        checkpoint_recovery_id,
        checkpoint_event_id,
        termination_event_id,
        historical_termination_event_id: grant.historical_termination_event_id,
        publisher_credential_id: publisher.credential_id,
    }
}

fn assert_checkpoint_provenance(
    fixture: &PublicationFixture,
    started: &PullRequestPublicationOutcome,
) {
    assert_eq!(started.publication.run_id, fixture.verifier_run_id);
    assert_eq!(
        started.publication.provenance["checkpoint"]["origin_run_id"],
        json!(fixture.corrected_run_id)
    );
    assert_eq!(
        started.publication.provenance["checkpoint"]["workspace_run_id"],
        json!(SOURCE)
    );
    assert_eq!(
        started.publication.provenance["checkpoint"]["checkpoint_event_id"],
        json!(fixture.checkpoint_event_id)
    );
    assert_eq!(
        started.publication.provenance["checkpoint"]["termination_event_id"],
        json!(fixture.termination_event_id)
    );
    assert_eq!(
        started.publication.provenance["checkpoint"]["original_head_commit"],
        "c".repeat(40)
    );
    assert_eq!(
        started.publication.provenance["checkpoint"]["verified_head_commit"],
        "f".repeat(40)
    );
    assert!(
        !started.publication.auto_merge_enabled
            && !started.publication.merge_authorized
            && !started.publication.deployment_authorized
    );
}

fn assert_model_history(before: &Value, after: &Value) {
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
        assert_eq!(after[key], before[key], "publication changed {key}");
    }
    let prefix = before["events"].as_array().unwrap();
    let events = after["events"].as_array().unwrap();
    assert_eq!(&events[..prefix.len()], prefix.as_slice());
    assert!(
        events[prefix.len()..]
            .iter()
            .all(|event| !event["type"].as_str().unwrap().starts_with("run."))
    );
    assert_eq!(after["source"]["breaker_stage"], "suspend");
    assert_eq!(after["source"]["input_tokens"], 5_300);
    assert_eq!(after["task"]["attempt_count"], 2);
    assert_eq!(after["task"]["max_attempts"], 2);
}

fn assert_replay_preserves_state(
    before: &Value,
    after: &Value,
    input: &StartPullRequestPublicationInput,
) {
    // Successful credential revalidation audits last_used_at even on replay.
    // Exempt only that exact credential's monotonic timestamp, not other rows,
    // fields or events. Rejected operations still use exact rollback equality.
    let matching = before["publication"]["credentials"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .filter(|(_, credential)| {
            credential["corp_id"] == json!(input.corp_id)
                && credential["publisher_id"].as_str() == Some(input.publisher_id.as_str())
                && credential["credential_hash"].as_str()
                    == Some(input.publisher_credential_hash.as_str())
        })
        .collect::<Vec<_>>();
    assert_eq!(matching.len(), 1, "exact replay credential must exist");
    let (index, previous) = matching[0];
    let current = &after["publication"]["credentials"][index];
    assert_eq!(current["id"], previous["id"]);
    let previous_used = chrono::DateTime::parse_from_rfc3339(
        previous["last_used_at"]
            .as_str()
            .expect("previous successful authorization has an audit timestamp"),
    )
    .expect("previous audit timestamp is valid");
    let current_used = chrono::DateTime::parse_from_rfc3339(
        current["last_used_at"]
            .as_str()
            .expect("replay authorization retains an audit timestamp"),
    )
    .expect("replay audit timestamp is valid");
    assert!(
        current_used >= previous_used,
        "publisher credential audit time must not regress"
    );
    let mut expected = before.clone();
    expected["publication"]["credentials"][index]["last_used_at"] = current["last_used_at"].clone();
    assert_eq!(
        after, &expected,
        "replay must preserve every field except the exact credential audit timestamp"
    );
}

async fn start_publication(fixture: &PublicationFixture) -> PullRequestPublicationOutcome {
    let before = publication_state(&fixture.store).await;
    let started = fixture
        .store
        .start_pull_request_publication(fixture.input.clone())
        .await
        .expect("valid reviewed corrected-checkpoint publication must start");
    assert!(!started.replayed && !started.busy);
    assert!(started.publisher_token.is_some());
    assert_checkpoint_provenance(fixture, &started);
    assert_model_history(&before, &publication_state(&fixture.store).await);
    started
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue216_corrected_checkpoint_publication_start_preserves_history(pool: PgPool) {
    let fixture = corrected_publication_fixture(pool).await;
    let before = publication_state(&fixture.store).await;
    let result = fixture
        .store
        .start_pull_request_publication(fixture.input.clone())
        .await;
    if let Err(error) = &result {
        assert_eq!(
            publication_state(&fixture.store).await,
            before,
            "rejected publication must leave every persisted fixture row/event unchanged"
        );
        assert!(
            error
                .to_string()
                .contains("source correction lineage has another stop, suspension or quarantine"),
            "unexpected baseline failure before the known #216 admission guard: {error:#}"
        );
    }
    // This positive expectation is the intended RED on unchanged product code.
    let started = result.expect(
        "the completed checkpoint verifier must publish its reviewed export past only the exact proven historical suspension and authorized correction stop",
    );
    assert!(!started.replayed && !started.busy);
    assert!(started.publisher_token.is_some());
    assert_checkpoint_provenance(&fixture, &started);
    let after = publication_state(&fixture.store).await;
    assert_model_history(&before, &after);
    assert_eq!(after["publication"]["rows"].as_array().unwrap().len(), 1);
    assert_eq!(
        after["publication"]["attempts"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        after["publication"]["operations"].as_array().unwrap().len(),
        1
    );
}

fn branch_checkpoint(
    input: &StartPullRequestPublicationInput,
    active: &PullRequestPublicationOutcome,
    key: String,
) -> RecordPullRequestPublicationCheckpointInput {
    RecordPullRequestPublicationCheckpointInput {
        corp_id: CORP,
        publication_id: active.publication.id,
        actor_id: input.actor_id,
        publisher_id: input.publisher_id.clone(),
        publisher_credential_hash: input.publisher_credential_hash.clone(),
        publisher_token: active.publisher_token.unwrap(),
        expected_version: active.publication.version,
        idempotency_key: key,
        checkpoint: PullRequestPublicationCheckpointInput::BranchPushed {
            commit_sha: "f".repeat(40),
        },
    }
}

fn pull_request_checkpoint(
    input: &StartPullRequestPublicationInput,
    active: &PullRequestPublicationOutcome,
    key: String,
) -> RecordPullRequestPublicationCheckpointInput {
    RecordPullRequestPublicationCheckpointInput {
        checkpoint: PullRequestPublicationCheckpointInput::PullRequestCreated {
            number: 216,
            node_id: "PR_issue216_sqlx_fixture".to_owned(),
            url: "https://github.com/fixture/source/pull/216".to_owned(),
            state: "open".to_owned(),
            draft: true,
            title: input.title.clone(),
            body: input.body.clone(),
            head_ref: input.branch.clone(),
            base_ref: input.base_ref.clone(),
            head_sha: "f".repeat(40),
            head_repository_owner: "fixture".to_owned(),
            is_cross_repository: false,
            auto_merge_enabled: false,
        },
        ..branch_checkpoint(input, active, key)
    }
}

async fn assert_controls_denied(
    fixture: &PublicationFixture,
    input: &StartPullRequestPublicationInput,
    active: Option<&PullRequestPublicationOutcome>,
    label: &str,
) {
    let before = publication_state(&fixture.store).await;
    assert!(
        fixture
            .store
            .start_pull_request_publication(input.clone())
            .await
            .is_err(),
        "{label}: start, including exact active-start replay, must revalidate"
    );
    assert_eq!(
        publication_state(&fixture.store).await,
        before,
        "{label}: start rollback"
    );
    if let Some(active) = active {
        assert!(
            fixture
                .store
                .renew_pull_request_publication(publication_renewal(input, active))
                .await
                .is_err(),
            "{label}: a new renewal must revalidate"
        );
        assert_eq!(
            publication_state(&fixture.store).await,
            before,
            "{label}: renewal rollback"
        );
        // A fresh key and the correct verified SHA ensure this would advance a
        // healthy publication, rather than testing a no-effect historical replay.
        assert!(
            fixture
                .store
                .record_pull_request_publication_checkpoint(branch_checkpoint(
                    input,
                    active,
                    Uuid::new_v4().to_string(),
                ))
                .await
                .is_err(),
            "{label}: a forward checkpoint must revalidate"
        );
        assert_eq!(
            publication_state(&fixture.store).await,
            before,
            "{label}: checkpoint rollback"
        );
    }
}

struct MetadataMutation {
    label: &'static str,
    table: &'static str,
    column: &'static str,
    id: Uuid,
    value: Value,
}

impl MetadataMutation {
    fn new(
        label: &'static str,
        table: &'static str,
        column: &'static str,
        id: Uuid,
        value: Value,
    ) -> Self {
        Self {
            label,
            table,
            column,
            id,
            value,
        }
    }
}

async fn metadata_column(store: &PgStore, table: &str, column: &str, id: Uuid) -> Value {
    // Identifiers come only from the closed test-local case lists below.
    assert!(matches!(
        table,
        "runs" | "events" | "factory_verification_recoveries" | "actors" | "runner_commands"
    ));
    assert!(
        column
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    );
    sqlx::query_scalar(&format!(
        "SELECT to_jsonb(target)->$3::text FROM {table} target WHERE corp_id=$1 AND id=$2"
    ))
    .bind(CORP)
    .bind(id)
    .bind(column)
    .fetch_one(&store.pool)
    .await
    .unwrap()
}

async fn set_metadata_column(store: &PgStore, change: &MetadataMutation, value: &Value) {
    let table = change.table;
    let column = change.column;
    assert!(matches!(
        table,
        "runs" | "events" | "factory_verification_recoveries" | "actors" | "runner_commands"
    ));
    assert!(
        column
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    );
    let result = sqlx::query(&format!(
        "UPDATE {table} SET {column} =
         (jsonb_populate_record(NULL::{table},jsonb_build_object($3::text,$4::jsonb))).{column}
         WHERE corp_id=$1 AND id=$2"
    ))
    .bind(CORP)
    .bind(change.id)
    .bind(column)
    .bind(value)
    .execute(&store.pool)
    .await
    .unwrap();
    assert_eq!(result.rows_affected(), 1, "{}", change.label);
}

async fn deny_metadata_change(
    fixture: &PublicationFixture,
    change: &MetadataMutation,
    active: Option<&PullRequestPublicationOutcome>,
) {
    let before = publication_state(&fixture.store).await;
    let original = metadata_column(&fixture.store, change.table, change.column, change.id).await;
    // Restore only deliberately injected fixture corruption. This is not a
    // native repair path; budgets, consumed usage and attempts are never reset.
    set_metadata_column(&fixture.store, change, &change.value).await;
    assert_controls_denied(fixture, &fixture.input, active, change.label).await;
    set_metadata_column(&fixture.store, change, &original).await;
    assert_eq!(
        publication_state(&fixture.store).await,
        before,
        "{}: restore injected corruption only",
        change.label
    );
}

async fn exercise_metadata_changes(fixture: &PublicationFixture, cases: &[MetadataMutation]) {
    for change in cases {
        deny_metadata_change(fixture, change, None).await;
    }
    let started = start_publication(fixture).await;
    for change in cases {
        deny_metadata_change(fixture, change, Some(&started)).await;
    }
    // Positive controls prevent permanently broken fixtures from making the
    // denial matrix vacuously pass.
    let before = publication_state(&fixture.store).await;
    let renewed = fixture
        .store
        .renew_pull_request_publication(publication_renewal(&fixture.input, &started))
        .await
        .unwrap();
    let advanced = fixture
        .store
        .record_pull_request_publication_checkpoint(branch_checkpoint(
            &fixture.input,
            &renewed,
            Uuid::new_v4().to_string(),
        ))
        .await
        .unwrap();
    assert!(!advanced.replayed);
    assert_checkpoint_provenance(fixture, &advanced);
    assert_model_history(&before, &publication_state(&fixture.store).await);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue216_public_start_renew_forward_and_exact_replay_preserve_history(pool: PgPool) {
    let fixture = corrected_publication_fixture(pool).await;
    let before = publication_state(&fixture.store).await;
    let started = start_publication(&fixture).await;
    let after_start = publication_state(&fixture.store).await;
    let replay = fixture
        .store
        .start_pull_request_publication(fixture.input.clone())
        .await
        .unwrap();
    assert!(replay.replayed && !replay.busy);
    assert_eq!(replay.publication.id, started.publication.id);
    assert_eq!(replay.publication.version, started.publication.version);
    assert_eq!(replay.publisher_token, started.publisher_token);
    assert_replay_preserves_state(
        &after_start,
        &publication_state(&fixture.store).await,
        &fixture.input,
    );

    let renewal = publication_renewal(&fixture.input, &started);
    let renewed = fixture
        .store
        .renew_pull_request_publication(renewal.clone())
        .await
        .unwrap();
    let after_renew = publication_state(&fixture.store).await;
    let replay = fixture
        .store
        .renew_pull_request_publication(renewal)
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.publication.version, renewed.publication.version);
    assert_eq!(
        replay.publication.provenance,
        renewed.publication.provenance
    );
    assert_replay_preserves_state(
        &after_renew,
        &publication_state(&fixture.store).await,
        &fixture.input,
    );

    let branch_key = Uuid::new_v4().to_string();
    let branch = fixture
        .store
        .record_pull_request_publication_checkpoint(branch_checkpoint(
            &fixture.input,
            &renewed,
            branch_key.clone(),
        ))
        .await
        .unwrap();
    assert!(!branch.replayed);
    assert!(branch.publication.branch_pushed_at.is_some());
    let after_branch = publication_state(&fixture.store).await;
    let replay = fixture
        .store
        .record_pull_request_publication_checkpoint(branch_checkpoint(
            &fixture.input,
            &renewed,
            branch_key,
        ))
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.publication.version, branch.publication.version);
    assert_replay_preserves_state(
        &after_branch,
        &publication_state(&fixture.store).await,
        &fixture.input,
    );

    // Synthetic review-only PR metadata through the public store checkpoint:
    // no native Git or GitHub operation is executed by this test.
    let pr_key = Uuid::new_v4().to_string();
    let pr = fixture
        .store
        .record_pull_request_publication_checkpoint(pull_request_checkpoint(
            &fixture.input,
            &branch,
            pr_key.clone(),
        ))
        .await
        .unwrap();
    let before_pr_replay = publication_state(&fixture.store).await;
    let replay = fixture
        .store
        .record_pull_request_publication_checkpoint(pull_request_checkpoint(
            &fixture.input,
            &branch,
            pr_key,
        ))
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.publication.id, pr.publication.id);
    assert_eq!(replay.publication.version, pr.publication.version);
    let after_pr = publication_state(&fixture.store).await;
    assert_replay_preserves_state(&before_pr_replay, &after_pr, &fixture.input);
    assert_checkpoint_provenance(&fixture, &pr);
    assert_eq!(pr.publication.attempt_count, 1);

    let mut changed_start = fixture.input.clone();
    changed_start.title.push_str(" changed under the same key");
    assert!(
        fixture
            .store
            .start_pull_request_publication(changed_start)
            .await
            .is_err()
    );
    assert_eq!(publication_state(&fixture.store).await, after_pr);
    for wrong_token in [false, true] {
        let mut bad = publication_renewal(&fixture.input, &pr);
        if wrong_token {
            bad.publisher_token = Uuid::new_v4();
        } else {
            bad.expected_version = started.publication.version;
        }
        assert!(
            fixture
                .store
                .renew_pull_request_publication(bad)
                .await
                .is_err()
        );
        assert_eq!(publication_state(&fixture.store).await, after_pr);
    }
    assert_model_history(&before, &after_pr);
    assert_eq!(after_pr["publication"]["rows"].as_array().unwrap().len(), 1);
    assert_eq!(
        after_pr["publication"]["attempts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue216_historical_correction_grant_revalidates_all_public_controls(pool: PgPool) {
    let fixture = corrected_publication_fixture(pool).await;
    let authority = metadata_column(
        &fixture.store,
        "factory_verification_recoveries",
        "source_correction_authority",
        fixture.correction_recovery_id,
    )
    .await;
    assert!(authority.is_object());
    let mut bad_policy = authority.clone();
    bad_policy["previous_policy_sha256"] = json!("9".repeat(64));
    let mut bad_session = authority;
    bad_session["provider_session_id"] = json!("another-fixture-session");
    let cases = [
        ("missing historical grant", Value::Null),
        ("malformed historical grant", json!({})),
        ("changed historical policy proof", bad_policy),
        ("changed historical provider session", bad_session),
    ]
    .into_iter()
    .map(|(label, value)| {
        MetadataMutation::new(
            label,
            "factory_verification_recoveries",
            "source_correction_authority",
            fixture.correction_recovery_id,
            value,
        )
    })
    .collect::<Vec<_>>();
    exercise_metadata_changes(&fixture, &cases).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue216_erased_checkpoint_markers_cannot_hide_native_suspension(pool: PgPool) {
    let fixture = corrected_publication_fixture(pool).await;
    let predecessor = sqlx::query(
        "SELECT id,mode,checkpoint_authority,request FROM factory_verification_recoveries
         WHERE corp_id=$1 AND replacement_run_id=$2 AND mode='checkpoint_verification'",
    )
    .bind(CORP)
    .bind(fixture.failed_run_id)
    .fetch_one(&fixture.store.pool)
    .await
    .unwrap();
    let predecessor_id: Uuid = predecessor.get("id");
    let original_mode: String = predecessor.get("mode");
    let original_authority: Value = predecessor.get("checkpoint_authority");
    let original_request: Value = predecessor.get("request");
    assert_ne!(predecessor_id, fixture.checkpoint_recovery_id);
    let command = sqlx::query(
        "SELECT id,payload FROM runner_commands
         WHERE corp_id=$1 AND run_id=$2 AND command_kind='factory_verification_recovery'
           AND idempotency_key=$3",
    )
    .bind(CORP)
    .bind(fixture.failed_run_id)
    .bind(format!("factory-verification-recovery:{predecessor_id}"))
    .fetch_one(&fixture.store.pool)
    .await
    .unwrap();
    let mut request = original_request.clone();
    request["mode"] = json!("verifier_only");
    let mut payload: Value = command.get("payload");
    payload["mode"] = json!("verifier_only");
    payload
        .as_object_mut()
        .unwrap()
        .remove("retained_provider_receipt");
    payload
        .as_object_mut()
        .unwrap()
        .remove("retained_provider_receipt_upload");
    let changes = [
        MetadataMutation::new(
            "erased correction grant",
            "factory_verification_recoveries",
            "source_correction_authority",
            fixture.correction_recovery_id,
            Value::Null,
        ),
        MetadataMutation::new(
            "downgraded predecessor command",
            "runner_commands",
            "payload",
            command.get("id"),
            payload,
        ),
    ];
    let mut original = Vec::new();
    for change in &changes {
        original
            .push(metadata_column(&fixture.store, change.table, change.column, change.id).await);
    }
    // Both initial allocation and active publication revalidation must deny this
    // coherent-looking family downgrade. Only fixture corruption is restored;
    // the real native SUSPEND, latest checkpoint, usage and attempts never change.
    let mut active = None;
    for _ in 0..2 {
        let before = publication_state(&fixture.store).await;
        for change in &changes {
            set_metadata_column(&fixture.store, change, &change.value).await;
        }
        // The schema couples checkpoint mode to non-NULL authority. Change the
        // prior record and its request coherently in one statement, never by
        // dropping the constraint or exposing an impossible intermediate row.
        let downgraded = sqlx::query(
            "UPDATE factory_verification_recoveries
             SET mode='verifier_only',checkpoint_authority=NULL,request=$3
             WHERE corp_id=$1 AND id=$2",
        )
        .bind(CORP)
        .bind(predecessor_id)
        .bind(&request)
        .execute(&fixture.store.pool)
        .await
        .unwrap();
        assert_eq!(downgraded.rows_affected(), 1);
        let null_grants: bool = sqlx::query_scalar(
            "SELECT correction.source_correction_authority IS NULL
                    AND predecessor.checkpoint_authority IS NULL
             FROM factory_verification_recoveries correction
             JOIN factory_verification_recoveries predecessor
               ON predecessor.replacement_run_id=correction.source_run_id
              AND predecessor.corp_id=correction.corp_id
             WHERE correction.corp_id=$1 AND correction.id=$2 AND predecessor.id=$3",
        )
        .bind(CORP)
        .bind(fixture.correction_recovery_id)
        .bind(predecessor_id)
        .fetch_one(&fixture.store.pool)
        .await
        .unwrap();
        assert!(
            null_grants,
            "the downgrade must use SQL NULL, not a JSON-null marker"
        );
        let damaged = publication_state(&fixture.store).await;
        assert_eq!(damaged["source"], before["source"]);
        assert_eq!(damaged["source"]["breaker_stage"], "suspend");
        assert_eq!(run_journal(&damaged, SOURCE), run_journal(&before, SOURCE));
        let error = fixture
            .store
            .start_pull_request_publication(fixture.input.clone())
            .await
            .expect_err("historical native suspension still requires its exact grant");
        assert!(
            error.to_string().contains(
                "publication correction predecessor provenance is missing or inconsistent"
            ),
            "the historical-suspension classifier must deny the erased family witnesses: {error:#}"
        );
        assert_eq!(publication_state(&fixture.store).await, damaged);
        assert_controls_denied(
            &fixture,
            &fixture.input,
            active.as_ref(),
            "erased checkpoint family",
        )
        .await;
        let restored = sqlx::query(
            "UPDATE factory_verification_recoveries
             SET mode=$3,checkpoint_authority=$4,request=$5
             WHERE corp_id=$1 AND id=$2",
        )
        .bind(CORP)
        .bind(predecessor_id)
        .bind(&original_mode)
        .bind(&original_authority)
        .bind(&original_request)
        .execute(&fixture.store.pool)
        .await
        .unwrap();
        assert_eq!(restored.rows_affected(), 1);
        for (change, value) in changes.iter().zip(&original).rev() {
            set_metadata_column(&fixture.store, change, value).await;
        }
        assert_eq!(publication_state(&fixture.store).await, before);
        if active.is_none() {
            active = Some(start_publication(&fixture).await);
        }
    }
    let started = active.unwrap();
    let renewed = fixture
        .store
        .renew_pull_request_publication(publication_renewal(&fixture.input, &started))
        .await
        .unwrap();
    let advanced = fixture
        .store
        .record_pull_request_publication_checkpoint(branch_checkpoint(
            &fixture.input,
            &renewed,
            Uuid::new_v4().to_string(),
        ))
        .await
        .unwrap();
    assert!(!advanced.replayed);
    assert_checkpoint_provenance(&fixture, &advanced);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue216_current_checkpoint_proof_revalidates_all_public_controls(pool: PgPool) {
    let fixture = corrected_publication_fixture(pool).await;
    // SQL NULL is an impossible checkpoint-mode state under the real migration.
    // Keep that missing-proof negative at the schema boundary and require full
    // rollback; do not drop the constraint to manufacture a runtime fixture.
    let before_missing = publication_state(&fixture.store).await;
    let error = sqlx::query(
        "UPDATE factory_verification_recoveries SET checkpoint_authority=NULL
         WHERE corp_id=$1 AND id=$2",
    )
    .bind(CORP)
    .bind(fixture.checkpoint_recovery_id)
    .execute(&fixture.store.pool)
    .await
    .expect_err("schema must reject a missing current checkpoint grant");
    let database_error = error
        .as_database_error()
        .expect("Postgres constraint error");
    assert_eq!(database_error.code().as_deref(), Some("23514"));
    assert_eq!(
        database_error.constraint(),
        Some("factory_checkpoint_authority_check")
    );
    assert_eq!(publication_state(&fixture.store).await, before_missing);
    let authority = metadata_column(
        &fixture.store,
        "factory_verification_recoveries",
        "checkpoint_authority",
        fixture.checkpoint_recovery_id,
    )
    .await;
    let mut wrong_termination = authority.clone();
    wrong_termination["termination_event_id"] = json!(fixture.historical_termination_event_id);
    let mut wrong_fingerprint = authority;
    wrong_fingerprint["checkpoint"]["workspace_fingerprint"] = json!("9".repeat(64));
    let native = metadata_column(
        &fixture.store,
        "events",
        "payload",
        fixture.checkpoint_event_id,
    )
    .await;
    let mut missing_native = native.clone();
    missing_native
        .as_object_mut()
        .unwrap()
        .remove("source_checkpoint");
    let mut wrong_head = native;
    wrong_head["source_checkpoint"]["head_commit"] = json!("9".repeat(40));
    let mut live_provider = metadata_column(
        &fixture.store,
        "events",
        "payload",
        fixture.termination_event_id,
    )
    .await;
    live_provider["provider_process_alive"] = json!(true);
    let cases = vec![
        MetadataMutation::new(
            "historical termination substituted for current stop",
            "factory_verification_recoveries",
            "checkpoint_authority",
            fixture.checkpoint_recovery_id,
            wrong_termination,
        ),
        MetadataMutation::new(
            "current checkpoint fingerprint changed",
            "factory_verification_recoveries",
            "checkpoint_authority",
            fixture.checkpoint_recovery_id,
            wrong_fingerprint,
        ),
        MetadataMutation::new(
            "native checkpoint proof missing",
            "events",
            "payload",
            fixture.checkpoint_event_id,
            missing_native,
        ),
        MetadataMutation::new(
            "native checkpoint head changed",
            "events",
            "payload",
            fixture.checkpoint_event_id,
            wrong_head,
        ),
        MetadataMutation::new(
            "current provider termination not stopped",
            "events",
            "payload",
            fixture.termination_event_id,
            live_provider,
        ),
    ];
    exercise_metadata_changes(&fixture, &cases).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue216_quarantine_of_exact_origin_correction_or_verifier_is_never_exempt(pool: PgPool) {
    let fixture = corrected_publication_fixture(pool).await;
    let cases = [
        ("exact suspended origin quarantined", SOURCE),
        (
            "exact authorized stopped correction quarantined",
            fixture.corrected_run_id,
        ),
        (
            "selected completed verifier quarantined",
            fixture.verifier_run_id,
        ),
    ]
    .into_iter()
    .map(|(label, id)| {
        MetadataMutation::new(
            label,
            "runs",
            "workspace_disposition",
            id,
            json!("quarantined"),
        )
    })
    .collect::<Vec<_>>();
    // The SOURCE suspension exception must not exempt quarantine in the same OR.
    exercise_metadata_changes(&fixture, &cases).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue216_unrelated_checkpoint_child_stop_or_suspension_is_not_exempt(pool: PgPool) {
    let fixture = corrected_publication_fixture(pool).await;
    assert_ne!(fixture.failed_run_id, SOURCE);
    assert_ne!(fixture.failed_run_id, fixture.corrected_run_id);
    let cases = [
        MetadataMutation::new(
            "different checkpoint child stopped",
            "runs",
            "breaker_stage",
            fixture.failed_run_id,
            json!("stop"),
        ),
        MetadataMutation::new(
            "different checkpoint child suspended",
            "runs",
            "breaker_stage",
            fixture.failed_run_id,
            json!("suspend"),
        ),
    ];
    exercise_metadata_changes(&fixture, &cases).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue216_native_loop_counters_are_not_a_checkpoint_budget_exemption(pool: PgPool) {
    let fixture = corrected_publication_fixture(pool).await;
    let mut cases = Vec::new();
    for id in [SOURCE, fixture.corrected_run_id] {
        // Exact persisted native counters/thresholds, not invented audit metrics.
        cases.push(MetadataMutation::new(
            "native no-progress limit",
            "runs",
            "no_progress_events",
            id,
            json!(8),
        ));
        cases.push(MetadataMutation::new(
            "native repeated-tool limit",
            "runs",
            "repeated_tool_count",
            id,
            json!(5),
        ));
    }
    exercise_metadata_changes(&fixture, &cases).await;
}

async fn foreign_parent_fixture(fixture: &PublicationFixture) -> Uuid {
    // A real, separate tenant graph in the disposable database. It is never
    // dispatched or accepted as a provider result. Global run-id FKs alone do
    // not prove the Corp/task/agent/runner/workspace binding of a parent edge.
    let corp = Uuid::new_v4();
    let owner = Uuid::new_v4();
    let worker = Uuid::new_v4();
    let room = Uuid::new_v4();
    let agent = Uuid::new_v4();
    let mission = Uuid::new_v4();
    let task = Uuid::new_v4();
    let run = Uuid::new_v4();
    let runner = format!("issue216-foreign-{corp}");
    sqlx::query("INSERT INTO corps(id,slug,name) VALUES($1,$2,'Foreign SQLx fixture')")
        .bind(corp)
        .bind(format!("issue216-foreign-{corp}"))
        .execute(&fixture.store.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO actors(id,corp_id,name,kind,role) VALUES
         ($1,$3,'Foreign owner','human','owner'),($2,$3,'Foreign worker','agent','worker')",
    )
    .bind(owner)
    .bind(worker)
    .bind(corp)
    .execute(&fixture.store.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO rooms(id,corp_id,name,purpose) VALUES($1,$2,'Foreign','SQLx fixture only')",
    )
    .bind(room)
    .bind(corp)
    .execute(&fixture.store.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO agents(id,corp_id,actor_id,name,role,adapter,status,accent)
         VALUES($1,$2,$3,'Foreign worker','worker','github-copilot','idle','#123456')",
    )
    .bind(agent)
    .bind(corp)
    .bind(worker)
    .execute(&fixture.store.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO runner_nodes(id,corp_id,hostname,os,connection_epoch,status)
         VALUES($1,$2,'foreign-fixture','test',$3,'offline')",
    )
    .bind(&runner)
    .bind(corp)
    .bind(Uuid::new_v4())
    .execute(&fixture.store.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO missions(id,corp_id,room_id,requested_by,title,status,budget_tokens,
             original_budget_tokens,budget_cost_microusd,original_budget_cost_microusd)
         VALUES($1,$2,$3,$4,'Foreign fixture','failed',5000,5000,1000000,1000000)",
    )
    .bind(mission)
    .bind(corp)
    .bind(room)
    .bind(owner)
    .execute(&fixture.store.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO tasks(id,corp_id,mission_id,title,objective,status,assigned_agent_id,
             required_adapter,plan_key,contract,verification_policy,attempt_count,max_attempts)
         SELECT $1,$2,$3,'Foreign fixture','Not a publication source','failed',$4,
             required_adapter,'foreign',contract,verification_policy,1,2
         FROM tasks WHERE corp_id=$5 AND id=$6",
    )
    .bind(task)
    .bind(corp)
    .bind(mission)
    .bind(agent)
    .bind(CORP)
    .bind(TASK)
    .execute(&fixture.store.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO runs(id,corp_id,task_id,agent_id,runner_id,assignment_token,status,
             workspace_run_id,workspace_path,workspace_branch,workspace_base_ref,workspace_base_commit,
             workspace_disposition,source_repository,source_base_ref,source_base_commit,execution_mode,
             budget_tokens_limit,budget_cost_microusd_limit)
         VALUES($1,$2,$3,$4,$5,$6,'failed',$1,'foreign-worktree','crony/foreign','main',$7,
             'preserved','foreign/source','main',$7,'verification_only',0,0)",
    ).bind(run).bind(corp).bind(task).bind(agent).bind(runner).bind(Uuid::new_v4())
        .bind("a".repeat(40)).execute(&fixture.store.pool).await.unwrap();
    run
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue216_broken_or_foreign_correction_parent_and_source_bindings_are_denied(pool: PgPool) {
    let fixture = corrected_publication_fixture(pool).await;
    let foreign = foreign_parent_fixture(&fixture).await;
    let cases = [
        MetadataMutation::new(
            "missing corrected parent",
            "runs",
            "resumed_from_run_id",
            fixture.corrected_run_id,
            Value::Null,
        ),
        MetadataMutation::new(
            "corrected parent skips failed checkpoint",
            "runs",
            "resumed_from_run_id",
            fixture.corrected_run_id,
            json!(SOURCE),
        ),
        MetadataMutation::new(
            "foreign corrected parent",
            "runs",
            "resumed_from_run_id",
            fixture.corrected_run_id,
            json!(foreign),
        ),
        MetadataMutation::new(
            "foreign corrected workspace",
            "runs",
            "workspace_run_id",
            fixture.corrected_run_id,
            json!(foreign),
        ),
        MetadataMutation::new(
            "foreign correction authorization source",
            "factory_verification_recoveries",
            "source_run_id",
            fixture.correction_recovery_id,
            json!(foreign),
        ),
        MetadataMutation::new(
            "foreign current checkpoint source",
            "factory_verification_recoveries",
            "source_run_id",
            fixture.checkpoint_recovery_id,
            json!(foreign),
        ),
        MetadataMutation::new(
            "corrected source commit changed",
            "runs",
            "source_base_commit",
            fixture.corrected_run_id,
            json!("9".repeat(40)),
        ),
    ];
    exercise_metadata_changes(&fixture, &cases).await;
}

async fn explicit_stop_case(pool: PgPool, original: bool) {
    let fixture = corrected_publication_fixture(pool).await;
    let started = start_publication(&fixture).await;
    let target = if original {
        SOURCE
    } else {
        fixture.corrected_run_id
    };
    let before_stop = publication_state(&fixture.store).await;
    // Exact event type, aggregate, room/correlation and payload used by native
    // request_emergency_stop. Append a fixture control fence to an already
    // stopped producer; do not manufacture another active provider to stop it.
    let mut tx = fixture.store.pool.begin().await.unwrap();
    let stop = append_event_tx(
        &mut tx,
        NewEvent {
            room_id: Some(ROOM),
            correlation_id: Some(MISSION),
            ..NewEvent::new(
                CORP,
                Some(OWNER),
                "run.stop_requested",
                "run",
                target,
                format!("run-stop:{target}:{}", Uuid::new_v4()),
                json!({"agent_id":AGENT,"reason":"Explicit SQLx owner stop fence"}),
            )
        },
    )
    .await
    .unwrap()
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(stop.aggregate_id, target);
    assert_eq!(stop.event_type, "run.stop_requested");
    let after_stop = publication_state(&fixture.store).await;
    assert_eq!(after_stop["runs"], before_stop["runs"]);
    assert_eq!(after_stop["tasks"], before_stop["tasks"]);
    assert_eq!(after_stop["missions"], before_stop["missions"]);
    assert_controls_denied(
        &fixture,
        &fixture.input,
        Some(&started),
        "native explicit stop",
    )
    .await;
    assert_eq!(publication_state(&fixture.store).await, after_stop);
    // The stop event is intentionally retained, not removed to reopen work.
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue216_explicit_stop_of_exact_original_suspension_blocks_publication(pool: PgPool) {
    explicit_stop_case(pool, true).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue216_explicit_stop_of_authorized_corrected_source_blocks_publication(pool: PgPool) {
    explicit_stop_case(pool, false).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue216_current_role_room_and_publisher_authority_are_revalidated(pool: PgPool) {
    let fixture = corrected_publication_fixture(pool).await;
    let demoted = MetadataMutation::new(
        "current publisher actor demoted",
        "actors",
        "role",
        OWNER,
        json!("member"),
    );
    deny_metadata_change(&fixture, &demoted, None).await;
    let started = start_publication(&fixture).await;
    deny_metadata_change(&fixture, &demoted, Some(&started)).await;

    let before_room = publication_state(&fixture.store).await;
    let membership: Value = sqlx::query_scalar(
        "SELECT to_jsonb(member) FROM room_memberships member WHERE room_id=$1 AND actor_id=$2",
    )
    .bind(ROOM)
    .bind(OWNER)
    .fetch_one(&fixture.store.pool)
    .await
    .unwrap();
    sqlx::query("DELETE FROM room_memberships WHERE room_id=$1 AND actor_id=$2")
        .bind(ROOM)
        .bind(OWNER)
        .execute(&fixture.store.pool)
        .await
        .unwrap();
    assert_controls_denied(
        &fixture,
        &fixture.input,
        Some(&started),
        "mission-room membership revoked",
    )
    .await;
    // Restore the exact deliberately removed fixture membership, including its
    // original timestamp, rather than inventing a new approval/role history.
    sqlx::query("INSERT INTO room_memberships SELECT * FROM jsonb_populate_record(NULL::room_memberships,$1)")
        .bind(membership).execute(&fixture.store.pool).await.unwrap();
    assert_eq!(publication_state(&fixture.store).await, before_room);

    for wrong_identity in [false, true] {
        let mut input = fixture.input.clone();
        if wrong_identity {
            input.publisher_id = "another-store-publisher".to_owned();
        } else {
            input.publisher_credential_hash = "9".repeat(64);
        }
        assert_controls_denied(
            &fixture,
            &input,
            Some(&started),
            "publisher credential/identity mismatch",
        )
        .await;
    }
    let before_revocation = publication_state(&fixture.store).await;
    fixture
        .store
        .revoke_publication_publisher_credential(
            CORP,
            OWNER,
            fixture.publisher_credential_id,
            "Explicit SQLx publisher revocation",
        )
        .await
        .unwrap();
    assert_controls_denied(
        &fixture,
        &fixture.input,
        Some(&started),
        "publisher revoked through native store API",
    )
    .await;
    assert_model_history(&before_revocation, &publication_state(&fixture.store).await);
}

async fn install_allocation_fault(store: &PgStore, operation: &str) {
    assert!(matches!(operation, "start" | "renew"));
    sqlx::raw_sql(&format!(
        r#"
        CREATE FUNCTION issue216_after_allocation_fault() RETURNS trigger LANGUAGE plpgsql AS $$
        BEGIN
            IF NEW.operation = TG_ARGV[0] THEN
                IF NOT EXISTS (
                    SELECT 1 FROM pull_request_publications publication
                    JOIN pull_request_publication_attempts attempt
                      ON attempt.publication_id=publication.id AND attempt.corp_id=publication.corp_id
                    WHERE publication.id=NEW.publication_id AND publication.corp_id=NEW.corp_id
                      AND publication.version=NEW.resulting_version
                      AND publication.publisher_token=NEW.publisher_token
                ) THEN
                    RAISE EXCEPTION 'issue216 fault did not observe allocated publication control';
                END IF;
                RAISE EXCEPTION 'issue216 post-allocation fault: %', NEW.operation;
            END IF;
            RETURN NEW;
        END $$;
        CREATE TRIGGER issue216_after_allocation_fault
        BEFORE INSERT ON pull_request_publication_operations FOR EACH ROW
        EXECUTE FUNCTION issue216_after_allocation_fault('{operation}');
        "#
    )).execute(&store.pool).await.unwrap();
}

async fn remove_allocation_fault(store: &PgStore) {
    sqlx::raw_sql(
        "DROP TRIGGER issue216_after_allocation_fault ON pull_request_publication_operations;
         DROP FUNCTION issue216_after_allocation_fault();",
    )
    .execute(&store.pool)
    .await
    .unwrap();
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue216_start_and_renew_post_allocation_failures_roll_back_every_row(pool: PgPool) {
    let fixture = corrected_publication_fixture(pool).await;
    install_allocation_fault(&fixture.store, "start").await;
    let before_start = publication_state(&fixture.store).await;
    let error = fixture
        .store
        .start_pull_request_publication(fixture.input.clone())
        .await
        .expect_err("injected post-allocation start failure");
    assert!(
        format!("{error:#}").contains("issue216 post-allocation fault: start"),
        "{error:#}"
    );
    assert_eq!(publication_state(&fixture.store).await, before_start);
    remove_allocation_fault(&fixture.store).await;
    // Same request/key succeeds only if the failed transaction left no partial
    // publication, attempt, operation, token, Factory transition or journal row.
    let started = start_publication(&fixture).await;
    assert_eq!(started.publication.attempt_count, 1);

    install_allocation_fault(&fixture.store, "renew").await;
    let renewal = publication_renewal(&fixture.input, &started);
    let before_renew = publication_state(&fixture.store).await;
    let error = fixture
        .store
        .renew_pull_request_publication(renewal.clone())
        .await
        .expect_err("injected post-update renewal failure");
    assert!(
        format!("{error:#}").contains("issue216 post-allocation fault: renew"),
        "{error:#}"
    );
    assert_eq!(publication_state(&fixture.store).await, before_renew);
    remove_allocation_fault(&fixture.store).await;
    let renewed = fixture
        .store
        .renew_pull_request_publication(renewal)
        .await
        .unwrap();
    assert!(!renewed.replayed);
    assert_eq!(renewed.publication.version, started.publication.version + 1);
    assert_eq!(renewed.publication.attempt_count, 1);
    let advanced = fixture
        .store
        .record_pull_request_publication_checkpoint(branch_checkpoint(
            &fixture.input,
            &renewed,
            Uuid::new_v4().to_string(),
        ))
        .await
        .unwrap();
    assert!(!advanced.replayed);
    assert_checkpoint_provenance(&fixture, &advanced);
    assert_model_history(&before_start, &publication_state(&fixture.store).await);
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue216_ordinary_checkpoint_publication_start_renew_replay_preserves_history(
    pool: PgPool,
) {
    // Reuse the unchanged parent's ordinary checkpoint fixture: no source
    // correction and no retained-receipt collection are introduced for control.
    let (store, command, _, input) = publication_fixture(pool).await;
    let before = publication_state(&store).await;
    assert_eq!(before["runs"].as_array().unwrap().len(), 2);
    assert_eq!(before["recoveries"].as_array().unwrap().len(), 1);
    assert_eq!(before["recoveries"][0]["mode"], "checkpoint_verification");
    assert!(before["recoveries"][0]["source_correction_authority"].is_null());

    let started = store
        .start_pull_request_publication(input.clone())
        .await
        .expect("ordinary reviewed checkpoint publication must still start");
    assert!(!started.replayed && !started.busy);
    assert!(started.publisher_token.is_some());
    assert_eq!(started.publication.run_id, command.run_id);
    assert_eq!(
        started.publication.provenance["checkpoint"]["origin_run_id"],
        json!(SOURCE)
    );
    assert_eq!(
        started.publication.provenance["checkpoint"]["workspace_run_id"],
        json!(SOURCE)
    );
    let after_start = publication_state(&store).await;
    let replay = store
        .start_pull_request_publication(input.clone())
        .await
        .unwrap();
    assert!(replay.replayed && !replay.busy);
    assert_eq!(replay.publication.id, started.publication.id);
    assert_eq!(replay.publication.version, started.publication.version);
    assert_eq!(replay.publisher_token, started.publisher_token);
    assert_replay_preserves_state(&after_start, &publication_state(&store).await, &input);

    let renewal = publication_renewal(&input, &started);
    let renewed = store
        .renew_pull_request_publication(renewal.clone())
        .await
        .expect("ordinary checkpoint authority must still renew");
    assert!(!renewed.replayed);
    assert_eq!(
        renewed.publication.provenance,
        started.publication.provenance
    );
    let after_renew = publication_state(&store).await;
    let replay = store.renew_pull_request_publication(renewal).await.unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.publication.id, renewed.publication.id);
    assert_eq!(replay.publication.version, renewed.publication.version);
    assert_eq!(replay.publisher_token, renewed.publisher_token);
    assert_eq!(replay.publication.attempt_count, 1);
    assert_replay_preserves_state(&after_renew, &publication_state(&store).await, &input);

    // This parent assertion is appropriate here: the ordinary fixture retains
    // its original 6,000-token STOP, unlike the corrected fixture's suspension.
    assert_publication_preserves_models(&before, &after_renew);
    assert_eq!(
        after_renew["publication"]["rows"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        after_renew["publication"]["attempts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        !renewed.publication.auto_merge_enabled
            && !renewed.publication.merge_authorized
            && !renewed.publication.deployment_authorized
    );
}

async fn apply_fixture_events(
    store: &PgStore,
    run_id: Uuid,
    assignment_token: Uuid,
    events: Vec<(&'static str, Value)>,
) {
    for (kind, payload) in events {
        store
            .apply_runner_event(event(run_id, assignment_token, kind, payload))
            .await
            .unwrap();
    }
}

async fn ready_fixture_artifact(
    store: &PgStore,
    input: RunnerEventInput,
    artifact: &StoredArtifact,
) {
    store
        .prepare_artifact_upload(
            input,
            artifact.clone(),
            &format!("staging/corps/{CORP}/{}", artifact.id),
        )
        .await
        .unwrap();
    store
        .finalize_artifact_upload(CORP, artifact.id)
        .await
        .unwrap()
        .unwrap();
}

#[sqlx::test(migrations = "../../db/migrations")]
#[ignore = "requires explicitly owned SQLx maintenance database"]
async fn issue216_ordinary_source_only_predecessor_correction_stop_checkpoint_publication_succeeds(
    pool: PgPool,
) {
    // Healthy provider execution, followed by failed ordinary verification.
    // Publication/export authority is present from the outset, not patched in
    // after the correction stops. All data is owned SQLx fixture metadata.
    let store = fixture_with_profile(
        pool,
        true,
        false,
        Some(DeliverableSpec {
            form: DeliverableForm::CommitBranch,
            commit_after_verification: true,
            paths: vec!["result.md".to_owned()],
        }),
        true,
        CheckpointFixtureProfile {
            mission_tokens: 10_000,
            run_tokens: 5_000,
            used_tokens: 1_000,
            attempt_count: 1,
            max_attempts: 3,
            expected_stage: "healthy",
            verification_failure: true,
            workspace_connection_id: Some(CONNECTION),
            adapter: "github-copilot",
            provider_session_id: SESSION,
            native_outcome: Some("completed"),
            ..CheckpointFixtureProfile::default()
        },
    )
    .await;
    let original = state(&store).await;
    assert_eq!(original["source"]["breaker_stage"], "healthy");
    assert_eq!(original["source"]["execution_mode"], "provider");
    assert_eq!(original["source"]["input_tokens"], 1_000);
    assert_eq!(original["task"]["attempt_count"], 1);
    assert_eq!(original["task"]["max_attempts"], 3);
    assert!(original["recoveries"].as_array().unwrap().is_empty());
    let context = store
        .factory_verification_recovery_context(CORP, OWNER, ITEM)
        .await
        .unwrap()
        .unwrap();
    assert!(!context.checkpoint_verification);

    // SOURCE_ONLY uses the public ordinary VerifierOnly / "verifier_only" mode.
    // Crucially, create it through admission rather than relabelling a checkpoint
    // recovery or editing its persisted authority/command provenance.
    let mut source_only = request();
    source_only.mode = FactoryVerificationRecoveryMode::VerifierOnly;
    source_only.expected_factory_version = context.work_item.version;
    source_only.expected_workspace_fingerprint = context.workspace_fingerprint.unwrap();
    source_only.expected_head_commit = context.expected_head_commit;
    source_only.reason =
        "Ordinary source-only fixture verification without another provider".to_owned();
    let ordinary = store
        .create_factory_verification_recovery(source_only)
        .await
        .expect("healthy failed provider must admit ordinary source-only verification");
    let ordinary_recovery_id = ordinary.recovery.id;
    let FactoryVerificationRecoveryLaunch::VerifierOnly(ordinary_verifier) = ordinary.launch else {
        panic!("ordinary source-only verification must not launch a provider");
    };
    let ordinary_command = store
        .pending_runner_commands(RUNNER)
        .await
        .unwrap()
        .into_iter()
        .find(|command| command.run_id == ordinary_verifier.run_id)
        .unwrap();
    assert_eq!(ordinary_command.payload["mode"], "verifier_only");
    assert_eq!(ordinary_command.payload["source_run_id"], json!(SOURCE));
    assert!(ordinary_command.payload["retained_provider_receipt"].is_null());
    assert!(
        metadata_column(
            &store,
            "factory_verification_recoveries",
            "checkpoint_authority",
            ordinary_recovery_id
        )
        .await
        .is_null()
    );
    apply_fixture_events(&store, ordinary_verifier.run_id, ordinary_verifier.assignment_token, vec![
        ("run.started", json!({
            "workspace":"fixture-worktree","workspace_branch":"crony/fixture",
            "workspace_base_ref":"main","workspace_base_commit":"a".repeat(40),
            "execution_mode":"verification_only"
        })),
        ("run.verification_started", json!({})),
        ("run.verification_evidence", json!({
            "evidence_id":Uuid::new_v4(),"check_index":0,"kind":"file","status":"passed",
            "summary":"Ordinary source-only fixture file check","payload":{}
        })),
        ("run.verification_evidence", json!({
            "evidence_id":Uuid::new_v4(),"check_index":1,"kind":"artifact","status":"failed",
            "summary":"Ordinary source-only fixture still lacks provider artifact","payload":{}
        })),
        ("run.verification_failed", json!({"error":"Required provider artifact remains missing"})),
        ("run.workspace_preserved", json!({
            "workspace_fingerprint":"b".repeat(64),"head_commit":"a".repeat(40),
            "detail":"Ordinary source-only verifier preserved its failed source"
        })),
    ]).await;
    store
        .acknowledge_runner_command(ordinary_command.id, RUNNER)
        .await
        .unwrap();
    let ordinary_failed = state(&store).await;
    assert_eq!(
        retained_run(&ordinary_failed, ordinary_verifier.run_id)["execution_mode"],
        "verification_only"
    );
    assert_eq!(
        retained_run(&ordinary_failed, ordinary_verifier.run_id)["verification_status"],
        "failed"
    );
    assert_eq!(
        retained_run(&ordinary_failed, ordinary_verifier.run_id)["breaker_stage"],
        "healthy"
    );
    assert_eq!(ordinary_failed["source"], original["source"]);
    assert_eq!(ordinary_failed["task"]["attempt_count"], 2);
    assert_eq!(ordinary_failed["task"]["max_attempts"], 3);

    let mut contract: TaskContract =
        serde_json::from_value(ordinary_failed["task"]["contract"].clone()).unwrap();
    contract.objective =
        "Correct result.md after ordinary source-only verification failed".to_owned();
    let policy: VerificationPolicy =
        serde_json::from_value(ordinary_failed["task"]["verification_policy"].clone()).unwrap();
    let revision = store
        .create_mission_contract_revision(CreateMissionContractRevisionInput {
            corp_id: CORP,
            mission_id: MISSION,
            task_id: TASK,
            actor_id: OWNER,
            expected_contract_version: ordinary_failed["task"]["contract_version"]
                .as_i64()
                .unwrap(),
            next_action: MissionContractRevisionAction::Resume,
            source_run_id: Some(ordinary_verifier.run_id),
            reason: "Explicit ordinary correction of the same retained source".to_owned(),
            idempotency_key: Uuid::new_v4(),
            description: "Correct source without checkpoint-correction authority".to_owned(),
            contract: contract.clone(),
            verification_policy: policy.clone(),
        })
        .await
        .unwrap();
    let context = store
        .factory_verification_recovery_context(CORP, OWNER, ITEM)
        .await
        .unwrap()
        .unwrap();
    assert!(!context.checkpoint_verification);
    let mut correction = request();
    correction.mode = FactoryVerificationRecoveryMode::SourceCorrection;
    correction.source_run_id = ordinary_verifier.run_id;
    correction.contract_revision_id = Some(revision.revision.id);
    correction.expected_factory_version = context.work_item.version;
    correction.expected_workspace_fingerprint = context.workspace_fingerprint.unwrap();
    correction.expected_head_commit = context.expected_head_commit;
    correction.reason = "Ordinary provider correction after source-only verification".to_owned();
    let corrected = store
        .create_factory_verification_recovery(correction)
        .await
        .expect("ordinary verifier predecessor must not require a #210 checkpoint grant");
    let correction_recovery_id = corrected.recovery.id;
    let FactoryVerificationRecoveryLaunch::SourceCorrection(provider) = corrected.launch else {
        panic!("ordinary correction must reuse the original provider session");
    };
    assert_eq!(provider.source_run_id, ordinary_verifier.run_id);
    assert_eq!(provider.provider_session_id, SESSION);
    assert!(
        metadata_column(
            &store,
            "factory_verification_recoveries",
            "source_correction_authority",
            correction_recovery_id
        )
        .await
        .is_null()
    );
    let correction_command = store
        .pending_runner_commands(RUNNER)
        .await
        .unwrap()
        .into_iter()
        .find(|command| command.run_id == provider.run_id)
        .unwrap();
    assert_eq!(correction_command.payload["mode"], "source_correction");
    assert_eq!(
        correction_command.payload["source_run_id"],
        json!(ordinary_verifier.run_id)
    );
    store.apply_runner_event(event(provider.run_id, provider.assignment_token, "run.started", json!({
        "adapter":"github-copilot","workspace":"fixture-worktree","workspace_branch":"crony/fixture",
        "workspace_base_ref":"main","workspace_base_commit":"a".repeat(40),"execution_mode":"provider"
    }))).await.unwrap();
    store
        .acknowledge_runner_command(correction_command.id, RUNNER)
        .await
        .unwrap();
    let allocated = state(&store).await;
    assert_eq!(
        retained_run(&allocated, provider.run_id)["budget_tokens_limit"],
        5_000
    );
    assert_eq!(allocated["task"]["attempt_count"], 3);
    assert_eq!(allocated["task"]["max_attempts"], 3);

    // The ordinary provider stores its own artifact while active, before STOP.
    // The final verifier will reuse this accepted store reference; no historical
    // retained-receipt grant is synthesized to make this control eligible.
    let receipt_input = event(
        provider.run_id,
        provider.assignment_token,
        "run.artifact_upload",
        json!({"artifact_role":"provider_evidence","file_name":RETAINED_COPILOT_RECEIPT_FILE,
            "media_type":"application/json","workspace_relative_path":RETAINED_COPILOT_RECEIPT_FILE}),
    );
    let receipt = fixture_artifact(
        &receipt_input,
        br#"{"fixture":"SQLx-only ordinary provider receipt metadata"}"#,
        "provider_evidence",
        RETAINED_COPILOT_RECEIPT_FILE,
        "application/json",
        json!({"workspace_relative_path":RETAINED_COPILOT_RECEIPT_FILE}),
    );
    ready_fixture_artifact(&store, receipt_input, &receipt).await;
    store
        .apply_runner_event(event(
            provider.run_id,
            provider.assignment_token,
            "run.usage",
            json!({"input_tokens":6_000,"output_tokens":0,"cost_microusd":0}),
        ))
        .await
        .unwrap();
    let stopped = store
        .evaluate_circuit_breaker(CORP, provider.run_id)
        .await
        .unwrap();
    assert_eq!(stopped.event.unwrap().payload["stage"], "stop");
    let proof = StoppedSourceCheckpoint {
        schema_version: 1,
        corp_id: CORP,
        mission_id: MISSION,
        task_id: TASK,
        run_id: provider.run_id,
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
    apply_fixture_events(&store, provider.run_id, provider.assignment_token, vec![
        ("run.session_terminated", json!({
            "adapter":"github-copilot","outcome":"cancelled","provider_process_alive":false
        })),
        ("run.workspace_preserved", json!({
            "workspace":"fixture-worktree","workspace_branch":"crony/fixture",
            "workspace_base_ref":"main","workspace_base_commit":"a".repeat(40),
            "workspace_fingerprint":"d".repeat(64),"head_commit":"c".repeat(40),
            "source_checkpoint":proof,"detail":"Ordinary correction reached a native budget stop"
        })),
        ("run.cancelled", json!({"reason":"Native correction exhausted its run allocation"})),
    ]).await;
    let stopped_state = state(&store).await;
    assert_eq!(stopped_state["source"], original["source"]);
    assert_eq!(stopped_state["task"]["attempt_count"], 3);
    assert_eq!(stopped_state["task"]["max_attempts"], 3);
    assert_eq!(
        retained_run(&stopped_state, provider.run_id)["breaker_stage"],
        "stop"
    );
    assert_eq!(
        retained_run(&stopped_state, provider.run_id)["input_tokens"],
        6_000
    );
    assert!(
        metadata_column(
            &store,
            "factory_verification_recoveries",
            "source_correction_authority",
            correction_recovery_id
        )
        .await
        .is_null()
    );

    let mut checkpoint = request();
    checkpoint.source_run_id = provider.run_id;
    checkpoint.expected_factory_version = stopped_state["item"]["version"].as_i64().unwrap();
    checkpoint.expected_workspace_fingerprint = "d".repeat(64);
    checkpoint.expected_head_commit = Some("c".repeat(40));
    let collected = store
        .create_factory_verification_recovery(checkpoint)
        .await
        .unwrap();
    let FactoryVerificationRecoveryLaunch::VerifierOnly(verifier) = collected.launch else {
        panic!("native stopped correction must be verified without another provider");
    };
    let command = store
        .pending_runner_commands(RUNNER)
        .await
        .unwrap()
        .into_iter()
        .find(|command| command.run_id == verifier.run_id)
        .unwrap();
    assert_eq!(command.payload["mode"], "checkpoint_verification");
    assert!(command.payload["retained_provider_receipt"].is_null());
    assert_eq!(
        command.payload["provider_artifact"]["sha256"],
        json!(receipt.sha256)
    );
    apply_fixture_events(&store, verifier.run_id, verifier.assignment_token, vec![
        ("run.started", json!({
            "workspace":"fixture-worktree","workspace_branch":"crony/fixture",
            "workspace_base_ref":"main","workspace_base_commit":"a".repeat(40),
            "execution_mode":"verification_only"
        })),
        ("run.verification_started", json!({})),
        ("run.verification_evidence", json!({
            "evidence_id":Uuid::new_v4(),"check_index":0,"kind":"file","status":"passed",
            "summary":"SQLx current source-file metadata","payload":{}
        })),
        ("run.verification_evidence", json!({
            "evidence_id":Uuid::new_v4(),"check_index":1,"kind":"artifact","status":"passed",
            "summary":"Reuse the ordinary provider's accepted fixture artifact",
            "payload":{"path":receipt.uri,"sha256":receipt.sha256,"bytes":receipt.bytes,
                "media_type":receipt.media_type}
        })),
    ]).await;
    let export_input = event(
        verifier.run_id,
        verifier.assignment_token,
        "run.deliverable_upload",
        json!({}),
    );
    let export = fixture_artifact(
        &export_input,
        br#"{"fixture":"SQLx-only ordinary-correction checkpoint export"}"#,
        "source_deliverable",
        "ecorp-commit-branch.json",
        "application/vnd.ecorp.deliverable+json",
        json!({
            "form":"commit_branch","verification_sha256":"e".repeat(64),
            "base_commit":"a".repeat(40),"head_commit":"f".repeat(40),
            "branch":"crony/fixture","integration_state":"ready_for_review",
            "git_bundle_sha256":hex::encode(Sha256::digest(b"ordinary source-only SQLx bundle")),
            "publication_ready":true
        }),
    );
    ready_fixture_artifact(&store, export_input, &export).await;
    apply_fixture_events(&store, verifier.run_id, verifier.assignment_token, vec![
        ("run.verification_passed", json!({
            "summary":"Current ordinary-correction fixture checks passed",
            "verification_sha256":"e".repeat(64),"deliverable_sha256":export.sha256
        })),
        ("run.verification_waiting", json!({"gate":gate(),"gate_type":"independent_review"})),
        ("run.workspace_preserved", json!({
            "workspace":"fixture-worktree","workspace_branch":"crony/fixture",
            "workspace_base_ref":"main","workspace_base_commit":"a".repeat(40),
            "workspace_fingerprint":"d".repeat(64),"head_commit":"f".repeat(40),
            "workspace_quarantined":false,"detail":"Checked ordinary-correction fixture export retained"
        })),
    ]).await;
    store
        .acknowledge_runner_command(command.id, RUNNER)
        .await
        .unwrap();
    store
        .decide_verification(
            CORP,
            verifier.run_id,
            REVIEWER,
            true,
            "Independent SQLx review of ordinary-correction checkpoint",
            Some(Uuid::new_v4()),
        )
        .await
        .unwrap();
    let publisher_hash = digest(&"issue216 ordinary-source-only SQLx publisher credential");
    store
        .create_publication_publisher_credential(
            CORP,
            OWNER,
            "issue216-source-only-publisher",
            &publisher_hash,
            Utc::now() + Duration::hours(1),
        )
        .await
        .unwrap();
    let deliverable_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM source_deliverables WHERE corp_id=$1 AND artifact_id=$2",
    )
    .bind(CORP)
    .bind(export.id)
    .fetch_one(&store.pool)
    .await
    .unwrap();
    let before = publication_state(&store).await;
    assert_eq!(before["item"]["state"], "verified");
    assert_eq!(before["mission"]["status"], "completed");
    assert_eq!(before["task"]["attempt_count"], 3);
    assert_eq!(before["task"]["max_attempts"], 3);
    assert_eq!(before["runs"].as_array().unwrap().len(), 4);
    assert_eq!(before["source"], original["source"]);
    assert_eq!(
        retained_run(&before, provider.run_id),
        retained_run(&stopped_state, provider.run_id)
    );
    let checked = retained_run(&before, verifier.run_id);
    assert_eq!(checked["status"], "completed");
    assert_eq!(checked["execution_mode"], "verification_only");
    assert!(checked["model"].is_null() && checked["provider_session_id"].is_null());
    for field in [
        "input_tokens",
        "output_tokens",
        "cost_microusd",
        "budget_tokens_limit",
        "budget_cost_microusd_limit",
    ] {
        assert_eq!(checked[field], 0, "{field}");
    }
    let input = StartPullRequestPublicationInput {
        corp_id: CORP,
        work_item_id: ITEM,
        actor_id: OWNER,
        actor_role: "owner".to_owned(),
        source_deliverable_id: deliverable_id,
        target_repository: "fixture/source".to_owned(),
        base_ref: "main".to_owned(),
        branch: "ecorp/issue216-source-only-control".to_owned(),
        title: "Reviewed ordinary source-only correction fixture".to_owned(),
        body: "Closes https://github.com/fixture/source/issues/148\nSQLx metadata only.".to_owned(),
        authorization_id: Uuid::new_v4(),
        authorization_reason: "Publish only the reviewed ordinary-correction checkpoint".to_owned(),
        effect_key: Uuid::new_v4().to_string(),
        idempotency_key: Uuid::new_v4().to_string(),
        publisher_id: "issue216-source-only-publisher".to_owned(),
        publisher_credential_hash: publisher_hash,
        lease_seconds: 300,
    };
    let started = store
        .start_pull_request_publication(input.clone())
        .await
        .expect("ordinary source-only verifier ancestry must not require a fabricated #210 grant");
    assert_eq!(started.publication.run_id, verifier.run_id);
    assert_eq!(
        started.publication.provenance["checkpoint"]["origin_run_id"],
        json!(provider.run_id)
    );
    let renewed = store
        .renew_pull_request_publication(publication_renewal(&input, &started))
        .await
        .unwrap();
    let advanced = store
        .record_pull_request_publication_checkpoint(branch_checkpoint(
            &input,
            &renewed,
            Uuid::new_v4().to_string(),
        ))
        .await
        .unwrap();
    assert!(!advanced.replayed);
    assert_eq!(advanced.publication.attempt_count, 1);
    assert!(
        !advanced.publication.auto_merge_enabled
            && !advanced.publication.merge_authorized
            && !advanced.publication.deployment_authorized
    );
    let after = publication_state(&store).await;
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
        assert_eq!(
            after[key], before[key],
            "ordinary-control publication changed {key}"
        );
    }
    let prefix = before["events"].as_array().unwrap();
    let events = after["events"].as_array().unwrap();
    assert_eq!(&events[..prefix.len()], prefix.as_slice());
    assert!(
        events[prefix.len()..]
            .iter()
            .all(|event| !event["type"].as_str().unwrap().starts_with("run."))
    );
    assert_eq!(after["source"]["breaker_stage"], "healthy");
    assert_eq!(after["source"]["input_tokens"], 1_000);
    assert_eq!(after["task"]["attempt_count"], 3);
    assert_eq!(after["task"]["max_attempts"], 3);
    assert!(
        metadata_column(
            &store,
            "factory_verification_recoveries",
            "source_correction_authority",
            correction_recovery_id
        )
        .await
        .is_null()
    );
}
