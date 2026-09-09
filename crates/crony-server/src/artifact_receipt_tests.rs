//! In-memory signed-byte tests; no database, native CLI, or provider calls.

use super::*;
use object_store::memory::InMemory;

fn store() -> ArtifactStore {
    ArtifactStore {
        store: Arc::new(InMemory::new()),
        signing_key: Arc::new(vec![0x4c; 32]),
        max_bytes: 16_777_216,
    }
}

fn identity() -> ArtifactIdentity<'static> {
    ArtifactIdentity {
        id: Uuid::from_u128(20),
        corp_id: Uuid::from_u128(1),
        task_id: Uuid::from_u128(2),
        run_id: Uuid::from_u128(3),
        agent_id: Uuid::from_u128(4),
        runner_id: "receipt-test",
    }
}

fn grant() -> RetainedProviderReceiptGrant {
    RetainedProviderReceiptGrant {
        schema_version: 1,
        collection_id: Uuid::from_u128(10),
        corp_id: identity().corp_id,
        task_id: identity().task_id,
        run_id: identity().run_id,
        workspace_run_id: Uuid::from_u128(5),
        source_run_id: Uuid::from_u128(6),
        checkpoint_run_id: Uuid::from_u128(6),
        provider_session_id: Uuid::from_u128(7).to_string(),
        historical_run_id: Uuid::from_u128(5),
        historical_termination_event_id: Uuid::from_u128(8),
        historical_checkpoint_event_id: Uuid::from_u128(9),
        source_checkpoint_event_id: Uuid::from_u128(11),
        expected_workspace_fingerprint: "a".repeat(64),
        expected_head_commit: "b".repeat(40),
        historical_model: Some("test-model".to_owned()),
        historical_reasoning_effort: None,
    }
}

fn receipt() -> Vec<u8> {
    serde_json::to_vec_pretty(&json!({
        "provider": "github-copilot",
        "sdk_version": "1.0.11",
        "session_id": grant().provider_session_id,
        "model": "test-model",
        "reasoning_effort": null,
        "discovered_model_count": 1,
        "assistant_messages": 2,
        "tool_calls": 3,
        "changed_paths": ["application/index.html"],
        "diff_sha256": "c".repeat(64),
    }))
    .unwrap()
}

fn payload(bytes: &[u8]) -> Value {
    let mut payload = retained_provider_receipt_metadata(&grant());
    let object = payload.as_object_mut().unwrap();
    object.extend(
        json!({
            "artifact_role": "provider_evidence",
            "file_name": RETAINED_COPILOT_RECEIPT_FILE,
            "media_type": "application/json",
            "sha256": hex::encode(Sha256::digest(bytes)),
            "bytes": bytes.len(),
            "content_base64": BASE64.encode(bytes),
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    payload
}

fn prepare(
    store: &ArtifactStore,
    identity: ArtifactIdentity<'_>,
    payload: &Value,
) -> Result<StagedArtifact> {
    store.prepare_retained_provider_receipt_staging(
        identity,
        payload,
        Utc::now() + chrono::Duration::days(1),
        &grant(),
    )
}

#[tokio::test]
async fn issue211_retained_artifact_signs_current_collection_and_preserves_exact_bytes() {
    let store = store();
    let bytes = receipt();
    let staged = prepare(&store, identity(), &payload(&bytes)).unwrap();
    assert_eq!(staged.bytes.as_ref(), bytes);
    assert_eq!(staged.artifact.run_id, grant().run_id);
    assert_ne!(staged.artifact.run_id, grant().historical_run_id);
    assert_eq!(
        staged.artifact.metadata,
        retained_provider_receipt_metadata(&grant())
    );
    assert!(store.staged_keys().await.unwrap().is_empty());

    // These writes represent a successful store reservation; metadata is signed
    // and the actual object bytes are checked independently of the SQLx lane.
    store.write_staged(&staged).await.unwrap();
    store
        .finalize_staged(&staged.artifact, &staged.staging_key)
        .await
        .unwrap();
    assert_eq!(
        store
            .read_verified(&staged.artifact)
            .await
            .unwrap()
            .as_ref(),
        bytes
    );
    store.discard_staged(&staged.staging_key).await.unwrap();
    assert!(store.staged_keys().await.unwrap().is_empty());
}

#[tokio::test]
async fn issue211_retained_artifact_cannot_enter_ordinary_or_missing_grant_path() {
    let store = store();
    let mut payload = payload(&receipt());
    assert!(
        store
            .prepare_staging(identity(), &payload, Utc::now())
            .is_err()
    );
    payload["retained_provider_receipt"] = Value::Null;
    assert!(
        store
            .prepare_staging(identity(), &payload, Utc::now())
            .is_err()
    );
    assert!(prepare(&store, identity(), &payload).is_err());
    payload
        .as_object_mut()
        .unwrap()
        .remove("retained_provider_receipt");
    assert!(prepare(&store, identity(), &payload).is_err());
    assert!(store.staged_keys().await.unwrap().is_empty());
}

#[test]
fn issue211_retained_artifact_rejects_foreign_new_run_task_or_corp() {
    let store = store();
    let payload = payload(&receipt());
    for candidate in [
        ArtifactIdentity {
            corp_id: Uuid::new_v4(),
            ..identity()
        },
        ArtifactIdentity {
            task_id: Uuid::new_v4(),
            ..identity()
        },
        ArtifactIdentity {
            run_id: grant().historical_run_id,
            ..identity()
        },
    ] {
        assert!(prepare(&store, candidate, &payload).is_err());
    }
}

#[test]
fn issue211_retained_artifact_rejects_relabelled_provenance_and_false_completion() {
    let store = store();
    for (pointer, value) in [
        (
            "/retained_provider_receipt/provider_completion_claimed",
            json!(true),
        ),
        (
            "/retained_provider_receipt/original_server_artifact_acceptance_claimed",
            json!(true),
        ),
        (
            "/retained_provider_receipt/historical_digest_attestation_claimed",
            json!(true),
        ),
        (
            "/retained_provider_receipt/provider_inference_started",
            json!(true),
        ),
        (
            "/retained_provider_receipt/receipt_generation",
            json!("current"),
        ),
        (
            "/retained_provider_receipt/grant/collection_id",
            json!(Uuid::new_v4()),
        ),
        ("/workspace_relative_path", json!("other.json")),
        ("/file_name", json!("other.json")),
        ("/artifact_role", json!("source_deliverable")),
        ("/media_type", json!("text/plain")),
    ] {
        let mut candidate = payload(&receipt());
        *candidate.pointer_mut(pointer).unwrap() = value;
        assert!(
            prepare(&store, identity(), &candidate).is_err(),
            "{pointer}"
        );
    }
    let mut missing_role = payload(&receipt());
    missing_role
        .as_object_mut()
        .unwrap()
        .remove("artifact_role");
    assert!(prepare(&store, identity(), &missing_role).is_err());
}

#[test]
fn issue211_retained_artifact_checks_native_content_not_only_its_declared_digest() {
    let store = store();
    let mut value: Value = serde_json::from_slice(&receipt()).unwrap();
    for field in ["session_id", "model", "sdk_version"] {
        let original = value[field].clone();
        value[field] = json!("foreign");
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(
            prepare(&store, identity(), &payload(&bytes)).is_err(),
            "{field}"
        );
        value[field] = original;
    }
    value["provider_completed"] = json!(true);
    assert!(
        prepare(
            &store,
            identity(),
            &payload(&serde_json::to_vec(&value).unwrap())
        )
        .is_err()
    );
}

#[tokio::test]
async fn issue211_retained_artifact_signature_binds_historical_and_current_identity() {
    let store = store();
    let staged = prepare(&store, identity(), &payload(&receipt())).unwrap();
    store.write_staged(&staged).await.unwrap();
    store
        .finalize_staged(&staged.artifact, &staged.staging_key)
        .await
        .unwrap();
    for field in [
        "historical_run_id",
        "source_checkpoint_event_id",
        "collection_id",
    ] {
        let mut altered = staged.artifact.clone();
        altered.metadata["retained_provider_receipt"]["grant"][field] = json!(Uuid::new_v4());
        assert!(store.read_verified(&altered).await.is_err(), "{field}");
    }
    let mut altered = staged.artifact.clone();
    altered.run_id = grant().historical_run_id;
    assert!(store.read_verified(&altered).await.is_err());
    store.discard_staged(&staged.staging_key).await.unwrap();
}
