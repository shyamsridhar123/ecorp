use crony_domain::{
    MAX_RETAINED_PROVIDER_RECEIPT_BYTES, RETAINED_COPILOT_RECEIPT_FILE,
    retained_provider_receipt_metadata,
};

use super::*;

struct ReceiptFixture {
    root: PathBuf,
    workspaces: Arc<WorkspaceManager>,
    workspace: WorkspaceLease,
    assignment: Assignment,
    bytes: Vec<u8>,
}

impl ReceiptFixture {
    async fn new() -> Self {
        let (root, workspaces, workspace, mut assignment) =
            super::tests::prepared_verification_fixture().await;
        let session_id = Uuid::new_v4().to_string();
        let mut bytes = serde_json::to_vec_pretty(&json!({
            "provider":"github-copilot",
            "sdk_version":"1.0.11",
            "session_id":session_id,
            "model":"fixture-model",
            "reasoning_effort":"high",
            "discovered_model_count":3,
            "assistant_messages":2,
            "tool_calls":4,
            "changed_paths":["current.txt","receipt-check.test.mjs"],
            "diff_sha256":hex::encode(sha2::Sha256::digest(b"owned fixture diff")),
        }))
        .unwrap();
        bytes.extend_from_slice(b"\n");
        std::fs::write(workspace.path.join(RETAINED_COPILOT_RECEIPT_FILE), &bytes).unwrap();
        std::fs::write(workspace.path.join("current.txt"), b"current sealed work\n").unwrap();
        std::fs::write(
            workspace.path.join("receipt-check.test.mjs"),
            b"import test from 'node:test';\nimport assert from 'node:assert/strict';\nimport {readFileSync} from 'node:fs';\ntest('current file',()=>assert.equal(readFileSync('current.txt','utf8'),'current sealed work\\n'));\n",
        )
        .unwrap();
        assignment.checkpoint_verification = true;
        assignment.verification_command_id = Some(Uuid::new_v4());
        assignment.verification_policy = VerificationPolicy {
            checks: vec![
                crony_domain::VerifierCheck::Artifact { min_bytes: 1 },
                crony_domain::VerifierCheck::File {
                    path: "current.txt".to_owned(),
                    min_bytes: 1,
                },
                crony_domain::VerifierCheck::Test {
                    program: "node".to_owned(),
                    args: vec!["--test".to_owned(), "receipt-check.test.mjs".to_owned()],
                    timeout_ms: 10_000,
                },
            ],
            manual_gate: None,
        };
        assignment.write_scope = vec!["**".to_owned()];
        assignment.retained_provider_receipt = Some(RetainedProviderReceiptGrant {
            schema_version: 1,
            collection_id: assignment.verification_command_id.unwrap(),
            corp_id: assignment.corp_id,
            task_id: assignment.task_id,
            run_id: assignment.run_id,
            workspace_run_id: assignment.workspace_run_id,
            source_run_id: Uuid::new_v4(),
            checkpoint_run_id: Uuid::new_v4(),
            provider_session_id: session_id,
            historical_run_id: Uuid::new_v4(),
            historical_termination_event_id: Uuid::new_v4(),
            historical_checkpoint_event_id: Uuid::new_v4(),
            source_checkpoint_event_id: Uuid::new_v4(),
            expected_workspace_fingerprint: "a".repeat(64),
            expected_head_commit: "b".repeat(40),
            historical_model: Some("fixture-model".to_owned()),
            historical_reasoning_effort: Some("high".to_owned()),
        });
        let mut fixture = Self {
            root,
            workspaces,
            workspace,
            assignment,
            bytes,
        };
        fixture.reseal().await;
        fixture
    }

    async fn reseal(&mut self) {
        let fingerprint = self.workspaces.fingerprint(&self.workspace).await.unwrap();
        let head = self.workspaces.head_commit(&self.workspace).await.unwrap();
        self.assignment.expected_workspace_fingerprint = Some(fingerprint.clone());
        self.assignment.expected_head_commit = Some(head.clone());
        if let Some(grant) = &mut self.assignment.retained_provider_receipt {
            grant.expected_workspace_fingerprint = fingerprint;
            grant.expected_head_commit = head;
        }
    }

    fn cleanup(self) {
        super::tests::assert_no_verification_snapshots(self.assignment.run_id);
        let root = std::fs::canonicalize(&self.root).unwrap();
        let temp = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        assert!(root.starts_with(&temp) && root != temp);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UploadBehavior {
    Acknowledge,
    ForeignThenAcknowledge,
    CloseAck,
    Withhold,
    Stop,
    Interrupt,
    CloseControl,
    CloseBeforeRead,
    ChangeSource,
    ReplaceReceipt,
}

async fn run_receipt_fixture(
    fixture: &ReceiptFixture,
    behavior: UploadBehavior,
) -> Vec<(String, Value)> {
    let outbound = OutboundBus::default();
    let (connection, mut received) = mpsc::unbounded_channel();
    outbound.attach(connection, fixture.assignment.connection_epoch);
    let (control_tx, controls) = mpsc::unbounded_channel();
    let (ack_tx, artifact_acks) = mpsc::unbounded_channel();
    let mut control_tx = Some(control_tx);
    let mut ack_tx = Some(ack_tx);
    if behavior == UploadBehavior::CloseBeforeRead {
        drop(control_tx.take());
    }
    let assignment = fixture.assignment.clone();
    let workspaces = fixture.workspaces.clone();
    let task = tokio::spawn(async move {
        execute_verification_assignment(
            workspaces,
            "owned-receipt-test".to_owned(),
            assignment,
            outbound,
            AssignmentChannels {
                controls,
                artifact_acks,
            },
        )
        .await
    });
    let run_id = fixture.assignment.run_id;
    let mut events = Vec::new();
    tokio::time::timeout(Duration::from_secs(60), async {
        while let Some(message) = received.recv().await {
            if let RunnerToServer::RunEvent { run_id: observed_run, event_type, payload, .. } = message {
                assert_eq!(observed_run, run_id);
                if event_type == "run.artifact_upload" {
                    assert_eq!(payload["artifact_role"], "provider_evidence");
                    assert_eq!(payload["file_name"], RETAINED_COPILOT_RECEIPT_FILE);
                    assert_eq!(payload["workspace_relative_path"], RETAINED_COPILOT_RECEIPT_FILE);
                    assert_eq!(payload["media_type"], "application/json");
                    let bytes = BASE64.decode(payload["content_base64"].as_str().unwrap()).unwrap();
                    assert_eq!(bytes, fixture.bytes, "collection must not rewrite native bytes");
                    assert_eq!(payload["bytes"], bytes.len());
                    assert_eq!(payload["sha256"], hex::encode(sha2::Sha256::digest(&bytes)));
                    let expected = retained_provider_receipt_metadata(
                        fixture.assignment.retained_provider_receipt.as_ref().unwrap(),
                    );
                    assert_eq!(payload["retained_provider_receipt"], expected["retained_provider_receipt"]);
                    assert_eq!(payload.as_object().unwrap().len(), 8, "no arbitrary metadata");
                    let sha256 = payload["sha256"].as_str().unwrap().to_owned();
                    let mut acknowledge = matches!(
                        behavior,
                        UploadBehavior::Acknowledge | UploadBehavior::ForeignThenAcknowledge
                            | UploadBehavior::ChangeSource | UploadBehavior::ReplaceReceipt
                    );
                    match behavior {
                        UploadBehavior::ForeignThenAcknowledge => {
                            for ack in [
                                ArtifactAck { run_id: Uuid::new_v4(), artifact_id: Uuid::new_v4(),
                                    artifact_role: "provider_evidence".to_owned(), sha256: sha256.clone() },
                                ArtifactAck { run_id, artifact_id: Uuid::new_v4(),
                                    artifact_role: "source_deliverable".to_owned(), sha256: sha256.clone() },
                                ArtifactAck { run_id, artifact_id: Uuid::new_v4(),
                                    artifact_role: "provider_evidence".to_owned(), sha256: "0".repeat(64) },
                            ] {
                                ack_tx.as_ref().unwrap().send(ack).unwrap();
                            }
                            // The current check pipeline must remain blocked until a matching ACK.
                            if let Ok(Some(RunnerToServer::RunEvent { event_type, payload, .. })) =
                                tokio::time::timeout(Duration::from_millis(75), received.recv()).await
                            {
                                assert!(!event_type.starts_with("run.verification"));
                                assert_ne!(event_type, "run.completed");
                                events.push((event_type, payload));
                            }
                        }
                        UploadBehavior::CloseAck => drop(ack_tx.take()),
                        UploadBehavior::Stop | UploadBehavior::Interrupt => {
                            let reason = "cancel retained receipt collection".to_owned();
                            let control = if behavior == UploadBehavior::Stop {
                                AdapterControl::Stop { reason }
                            } else {
                                AdapterControl::Interrupt { reason }
                            };
                            control_tx.as_ref().unwrap().send(control).unwrap();
                        }
                        UploadBehavior::CloseControl => drop(control_tx.take()),
                        UploadBehavior::ChangeSource => {
                            std::fs::write(fixture.workspace.path.join("current.txt"), b"changed during ACK wait\n").unwrap();
                        }
                        UploadBehavior::ReplaceReceipt => {
                            let receipt = fixture.workspace.path.join(RETAINED_COPILOT_RECEIPT_FILE);
                            let prior = fixture.root.join("replaced-receipt.json");
                            match std::fs::rename(&receipt, &prior) {
                                Ok(()) => {
                                    std::fs::write(receipt, &fixture.bytes).unwrap();
                                    assert_eq!(
                                        fixture.workspaces.fingerprint(&fixture.workspace).await.unwrap(),
                                        fixture.assignment.expected_workspace_fingerprint.as_ref().unwrap().as_str(),
                                        "identical-byte replacement must not be detected only by content hash"
                                    );
                                }
                                Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                                    // A platform that refuses replacement while the handle is held
                                    // has already fenced it. Cancel instead of forcing that boundary.
                                    acknowledge = false;
                                    control_tx.as_ref().unwrap().send(AdapterControl::Stop {
                                        reason: "receipt replacement was fenced by the filesystem".to_owned(),
                                    }).unwrap();
                                }
                                Err(error) => panic!("replace owned receipt: {error}"),
                            }
                        }
                        UploadBehavior::Acknowledge | UploadBehavior::Withhold
                        | UploadBehavior::CloseBeforeRead => {}
                    }
                    if acknowledge {
                        ack_tx.as_ref().unwrap().send(ArtifactAck {
                            run_id,
                            artifact_id: Uuid::new_v4(),
                            artifact_role: "provider_evidence".to_owned(),
                            sha256,
                        }).unwrap();
                    }
                } else if event_type == "run.deliverable_upload" {
                    let bytes = BASE64.decode(payload["content_base64"].as_str().unwrap()).unwrap();
                    let exported: Value = serde_json::from_slice(&bytes).unwrap();
                    let changes = exported["changes"].as_array().unwrap();
                    assert!(changes.iter().any(|change| change["path"] == "current.txt"));
                    assert!(!changes.iter().any(|change| change["path"] == RETAINED_COPILOT_RECEIPT_FILE));
                    ack_tx.as_ref().unwrap().send(ArtifactAck {
                        run_id,
                        artifact_id: Uuid::new_v4(),
                        artifact_role: "source_deliverable".to_owned(),
                        sha256: payload["sha256"].as_str().unwrap().to_owned(),
                    }).unwrap();
                }
                let finished = event_type == "run.workspace_preserved";
                events.push((event_type, payload));
                if finished { break; }
            }
        }
    }).await.expect("bounded retained-receipt verifier settles");
    task.await.unwrap().unwrap();
    assert!(
        !events.iter().any(|(kind, _)| matches!(
            kind.as_str(),
            "run.session" | "run.session_terminated" | "run.output" | "run.usage"
        )),
        "collection cannot start or resume a provider: {events:?}"
    );
    events
}

#[tokio::test]
async fn issue211_retained_receipt_upload_is_exact_and_all_current_checks_and_manual_gate_run() {
    let mut fixture = ReceiptFixture::new().await;
    fixture.assignment.verification_policy.manual_gate =
        Some(crony_domain::ManualVerificationGate::IndependentReview {
            roles: vec!["owner".to_owned()],
            exclude_requester: true,
        });
    let original = fixture
        .assignment
        .expected_workspace_fingerprint
        .clone()
        .unwrap();
    let events = run_receipt_fixture(&fixture, UploadBehavior::ForeignThenAcknowledge).await;
    let checks = events
        .iter()
        .filter(|(kind, _)| kind == "run.verification_evidence")
        .map(|(_, payload)| {
            (
                payload["kind"].as_str().unwrap(),
                payload["status"].as_str().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        checks,
        vec![
            ("artifact", "passed"),
            ("file", "passed"),
            ("test", "passed")
        ]
    );
    assert!(events.iter().any(|(kind, payload)| {
        kind == "run.verification_waiting"
            && payload
                .to_string()
                .contains("Historical native provider receipt collected now")
    }));
    assert!(!events.iter().any(|(kind, _)| kind == "run.completed"));
    assert_eq!(
        fixture
            .workspaces
            .fingerprint(&fixture.workspace)
            .await
            .unwrap(),
        original
    );
    assert_eq!(
        std::fs::read(fixture.workspace.path.join(RETAINED_COPILOT_RECEIPT_FILE)).unwrap(),
        fixture.bytes
    );
    fixture.cleanup();
}

#[tokio::test]
async fn issue211_retained_receipt_is_excluded_from_export_without_changing_source_bytes() {
    let mut fixture = ReceiptFixture::new().await;
    fixture.assignment.deliverable = Some(DeliverableSpec {
        form: crony_domain::DeliverableForm::CommitBranch,
        commit_after_verification: true,
        paths: Vec::new(),
    });
    let original = fixture
        .assignment
        .expected_workspace_fingerprint
        .clone()
        .unwrap();
    let events = run_receipt_fixture(&fixture, UploadBehavior::Acknowledge).await;
    assert!(
        events
            .iter()
            .any(|(kind, _)| kind == "run.deliverable_upload")
    );
    assert!(events.iter().any(|(kind, payload)| {
        kind == "run.completed"
            && payload
                .to_string()
                .contains("Historical native provider receipt collected now")
    }));
    assert_eq!(
        fixture
            .workspaces
            .fingerprint(&fixture.workspace)
            .await
            .unwrap(),
        original
    );
    assert_eq!(
        std::fs::read(fixture.workspace.path.join(RETAINED_COPILOT_RECEIPT_FILE)).unwrap(),
        fixture.bytes
    );
    fixture.cleanup();
}

#[tokio::test]
async fn issue211_retained_receipt_grant_fences_command_mode_run_and_checkpoint_scope() {
    let fixture = ReceiptFixture::new().await;
    let valid = &fixture.assignment;
    retained_provider_receipt::validate_assignment(valid).unwrap();
    let mut variants = Vec::new();
    for field in [
        "collection",
        "corp",
        "task",
        "run",
        "workspace",
        "fingerprint",
        "head",
    ] {
        let mut changed = valid.clone();
        let grant = changed.retained_provider_receipt.as_mut().unwrap();
        match field {
            "collection" => grant.collection_id = Uuid::new_v4(),
            "corp" => grant.corp_id = Uuid::new_v4(),
            "task" => grant.task_id = Uuid::new_v4(),
            "run" => grant.run_id = Uuid::new_v4(),
            "workspace" => grant.workspace_run_id = Uuid::new_v4(),
            "fingerprint" => grant.expected_workspace_fingerprint = "0".repeat(64),
            "head" => grant.expected_head_commit = "0".repeat(40),
            _ => unreachable!(),
        }
        variants.push(changed);
    }
    let mut wrong_mode = valid.clone();
    wrong_mode.checkpoint_verification = false;
    variants.push(wrong_mode);
    let mut provider = valid.clone();
    provider.adapter = "github-copilot".to_owned();
    variants.push(provider);
    let mut with_reference = valid.clone();
    with_reference.provider_artifact = Some(VerificationArtifactReference {
        path: RETAINED_COPILOT_RECEIPT_FILE.to_owned(),
        sha256: hex::encode(sha2::Sha256::digest(&fixture.bytes)),
        bytes: fixture.bytes.len(),
        media_type: "application/json".to_owned(),
        data_base64: Some(BASE64.encode(&fixture.bytes)),
    });
    variants.push(with_reference);
    for changed in variants {
        assert!(retained_provider_receipt::validate_assignment(&changed).is_err());
    }
    fixture.cleanup();
}

#[tokio::test]
async fn issue211_retained_receipt_rejects_wrong_historical_session_model_and_current_checkpoint() {
    for variant in ["session", "model", "fingerprint", "head"] {
        let mut fixture = ReceiptFixture::new().await;
        let grant = fixture
            .assignment
            .retained_provider_receipt
            .as_mut()
            .unwrap();
        match variant {
            "session" => grant.provider_session_id = Uuid::new_v4().to_string(),
            "model" => grant.historical_model = Some("other-fixture-model".to_owned()),
            "fingerprint" => {
                grant.expected_workspace_fingerprint = "0".repeat(64);
                fixture.assignment.expected_workspace_fingerprint = Some("0".repeat(64));
            }
            "head" => {
                grant.expected_head_commit = "0".repeat(40);
                fixture.assignment.expected_head_commit = Some("0".repeat(40));
            }
            _ => unreachable!(),
        }
        let events = run_receipt_fixture(&fixture, UploadBehavior::Acknowledge).await;
        assert!(
            events.iter().any(|(kind, _)| kind == "run.failed"),
            "{variant}"
        );
        assert!(
            !events.iter().any(|(kind, _)| matches!(
                kind.as_str(),
                "run.artifact_upload" | "run.verification_evidence" | "run.completed"
            )),
            "{variant}"
        );
        assert_eq!(
            std::fs::read(fixture.workspace.path.join(RETAINED_COPILOT_RECEIPT_FILE)).unwrap(),
            fixture.bytes
        );
        fixture.cleanup();
    }
}

#[tokio::test]
async fn issue211_retained_receipt_missing_malformed_and_oversized_files_never_upload() {
    for variant in ["missing", "malformed", "duplicate", "oversized", "hardlink"] {
        let mut fixture = ReceiptFixture::new().await;
        let receipt = fixture.workspace.path.join(RETAINED_COPILOT_RECEIPT_FILE);
        match variant {
            "missing" => std::fs::remove_file(&receipt).unwrap(),
            "malformed" => std::fs::write(&receipt, b"{malformed").unwrap(),
            "duplicate" => {
                let mut bytes = br#"{"provider":"github-copilot","#.to_vec();
                bytes.extend_from_slice(&fixture.bytes[1..]);
                std::fs::write(&receipt, bytes).unwrap();
            }
            "oversized" => {
                let file = std::fs::File::create(&receipt).unwrap();
                file.set_len(MAX_RETAINED_PROVIDER_RECEIPT_BYTES as u64 + 1)
                    .unwrap();
            }
            "hardlink" => {
                std::fs::hard_link(&receipt, fixture.root.join("receipt-alias.json")).unwrap()
            }
            _ => unreachable!(),
        }
        fixture.reseal().await;
        let before = std::fs::read(&receipt).ok();
        let events = run_receipt_fixture(&fixture, UploadBehavior::Acknowledge).await;
        assert!(
            events.iter().any(|(kind, _)| kind == "run.failed"),
            "{variant}"
        );
        assert!(
            !events.iter().any(|(kind, _)| matches!(
                kind.as_str(),
                "run.artifact_upload" | "run.verification_evidence" | "run.completed"
            )),
            "{variant}"
        );
        assert_eq!(std::fs::read(&receipt).ok(), before);
        fixture.cleanup();
    }
}

#[tokio::test]
async fn issue211_retained_receipt_stop_interrupt_and_control_closure_cancel_during_collection() {
    for behavior in [
        UploadBehavior::Stop,
        UploadBehavior::Interrupt,
        UploadBehavior::CloseControl,
        UploadBehavior::CloseBeforeRead,
    ] {
        let fixture = ReceiptFixture::new().await;
        let fingerprint = fixture
            .assignment
            .expected_workspace_fingerprint
            .clone()
            .unwrap();
        let events = run_receipt_fixture(&fixture, behavior).await;
        assert!(
            events
                .iter()
                .any(|(kind, _)| kind == "run.cancelled" || kind == "run.failed")
        );
        assert!(
            !events
                .iter()
                .any(|(kind, _)| kind == "run.verification_evidence" || kind == "run.completed")
        );
        if behavior == UploadBehavior::CloseBeforeRead {
            assert!(!events.iter().any(|(kind, _)| kind == "run.artifact_upload"));
        }
        assert_eq!(
            fixture
                .workspaces
                .fingerprint(&fixture.workspace)
                .await
                .unwrap(),
            fingerprint
        );
        assert_eq!(
            std::fs::read(fixture.workspace.path.join(RETAINED_COPILOT_RECEIPT_FILE)).unwrap(),
            fixture.bytes
        );
        fixture.cleanup();
    }
}

#[tokio::test]
async fn issue211_retained_receipt_missing_ack_withholds_artifact_checks_and_retries_are_identical()
{
    for behavior in [UploadBehavior::CloseAck, UploadBehavior::Withhold] {
        let fixture = ReceiptFixture::new().await;
        let events = run_receipt_fixture(&fixture, behavior).await;
        let uploads = events
            .iter()
            .filter(|(kind, _)| kind == "run.artifact_upload")
            .map(|(_, payload)| payload)
            .collect::<Vec<_>>();
        assert_eq!(
            uploads.len(),
            if behavior == UploadBehavior::Withhold {
                6
            } else {
                1
            }
        );
        assert!(uploads.iter().all(|payload| *payload == uploads[0]));
        assert!(events.iter().any(|(kind, _)| kind == "run.failed"));
        assert!(
            !events
                .iter()
                .any(|(kind, _)| kind == "run.verification_evidence" || kind == "run.completed")
        );
        fixture.cleanup();
    }
}

#[tokio::test]
async fn issue211_retained_receipt_source_drift_or_identical_file_replacement_is_rejected_at_ack() {
    for behavior in [UploadBehavior::ChangeSource, UploadBehavior::ReplaceReceipt] {
        let fixture = ReceiptFixture::new().await;
        let events = run_receipt_fixture(&fixture, behavior).await;
        assert!(
            !events
                .iter()
                .any(|(kind, _)| kind == "run.verification_evidence" || kind == "run.completed")
        );
        assert!(
            events
                .iter()
                .any(|(kind, _)| kind == "run.failed" || kind == "run.cancelled")
        );
        if events.iter().any(|(kind, _)| kind == "run.failed") {
            assert!(
                events
                    .iter()
                    .any(|(kind, payload)| kind == "run.workspace_preserved"
                        && payload["workspace_quarantined"] == true)
            );
        }
        fixture.cleanup();
    }
}

#[tokio::test]
async fn issue211_retained_receipt_is_not_implicitly_collected_without_a_grant() {
    let mut fixture = ReceiptFixture::new().await;
    fixture.assignment.retained_provider_receipt = None;
    fixture.assignment.verification_command_id = None;
    let events = run_receipt_fixture(&fixture, UploadBehavior::Acknowledge).await;
    assert!(!events.iter().any(|(kind, _)| kind == "run.artifact_upload"));
    assert!(
        events
            .iter()
            .any(|(kind, _)| kind == "run.verification_failed")
    );
    assert!(
        events
            .iter()
            .any(|(kind, payload)| kind == "run.verification_evidence"
                && payload["kind"] == "artifact"
                && payload["status"] == "failed")
    );
    assert_eq!(
        std::fs::read(fixture.workspace.path.join(RETAINED_COPILOT_RECEIPT_FILE)).unwrap(),
        fixture.bytes
    );
    fixture.cleanup();
}

#[tokio::test]
async fn issue211_existing_provider_artifact_reference_remains_a_transfer_not_a_collection() {
    let mut fixture = ReceiptFixture::new().await;
    fixture.assignment.retained_provider_receipt = None;
    fixture.assignment.provider_artifact = Some(VerificationArtifactReference {
        path: RETAINED_COPILOT_RECEIPT_FILE.to_owned(),
        sha256: hex::encode(sha2::Sha256::digest(&fixture.bytes)),
        bytes: fixture.bytes.len(),
        media_type: "application/json".to_owned(),
        data_base64: Some(BASE64.encode(&fixture.bytes)),
    });
    let fingerprint = fixture
        .assignment
        .expected_workspace_fingerprint
        .clone()
        .unwrap();
    let events = run_receipt_fixture(&fixture, UploadBehavior::Acknowledge).await;
    assert!(!events.iter().any(|(kind, _)| kind == "run.artifact_upload"));
    assert!(
        events
            .iter()
            .any(|(kind, payload)| kind == "run.verification_evidence"
                && payload["kind"] == "artifact"
                && payload["status"] == "passed")
    );
    assert!(events.iter().any(|(kind, _)| kind == "run.completed"));
    assert!(!events.iter().any(|(_, payload)| {
        payload
            .to_string()
            .contains("Historical native provider receipt collected now")
    }));
    assert_eq!(
        fixture
            .workspaces
            .fingerprint(&fixture.workspace)
            .await
            .unwrap(),
        fingerprint
    );
    fixture.cleanup();
}

#[cfg(unix)]
#[tokio::test]
async fn issue211_retained_receipt_rejects_even_a_contained_receipt_symlink() {
    let mut fixture = ReceiptFixture::new().await;
    let receipt = fixture.workspace.path.join(RETAINED_COPILOT_RECEIPT_FILE);
    std::fs::rename(&receipt, fixture.workspace.path.join("receipt-target.json")).unwrap();
    std::os::unix::fs::symlink("receipt-target.json", &receipt).unwrap();
    fixture.reseal().await;
    let events = run_receipt_fixture(&fixture, UploadBehavior::Acknowledge).await;
    assert!(events.iter().any(|(kind, _)| kind == "run.failed"));
    assert!(
        !events
            .iter()
            .any(|(kind, _)| kind == "run.artifact_upload" || kind == "run.completed")
    );
    fixture.cleanup();
}

#[test]
fn issue211_retained_receipt_ack_identity_requires_new_run_role_digest_and_stored_id() {
    let run_id = Uuid::new_v4();
    let sha256 = "a".repeat(64);
    let mut ack = ArtifactAck {
        run_id,
        artifact_id: Uuid::new_v4(),
        artifact_role: "provider_evidence".to_owned(),
        sha256: sha256.clone(),
    };
    assert!(retained_provider_receipt::acknowledgment_matches(
        &ack, run_id, &sha256
    ));
    assert!(!retained_provider_receipt::acknowledgment_matches(
        &ack,
        Uuid::new_v4(),
        &sha256
    ));
    assert!(!retained_provider_receipt::acknowledgment_matches(
        &ack,
        run_id,
        &"b".repeat(64)
    ));
    ack.artifact_role = "source_deliverable".to_owned();
    assert!(!retained_provider_receipt::acknowledgment_matches(
        &ack, run_id, &sha256
    ));
    ack.artifact_role = "provider_evidence".to_owned();
    ack.artifact_id = Uuid::nil();
    assert!(!retained_provider_receipt::acknowledgment_matches(
        &ack, run_id, &sha256
    ));
}

#[test]
fn issue211_retained_receipt_capability_and_workspace_only_route_are_wired() {
    let main = include_str!("main.rs");
    assert!(main.contains("name: \"retained-provider-receipt-v1\""));
    let route = main
        .split("async fn execute_connected_verification_assignment(")
        .nth(1)
        .unwrap()
        .split("async fn checkpoint_connected_workspace(")
        .next()
        .unwrap();
    assert!(route.contains(".resolve_workspace("));
    assert!(!route.contains(".resolve("));
    let collection = include_str!("retained_provider_receipt.rs");
    assert!(!collection.contains("Client::"));
    assert!(!collection.contains("client_options("));
    assert!(!collection.contains(".resume("));
    assert!(!collection.contains(".execute("));
}
