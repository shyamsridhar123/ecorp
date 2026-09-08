//! Native stopped-source evidence, not recovery or publication authorization.
//!
//! The existing retained-workspace event journals this proof. Consumers must
//! still revalidate current authority, exact source bytes and persisted policy.

use super::*;
use crony_domain::StoppedSourceCheckpoint as SourceCheckpoint;

const CHECKPOINT_REPORT_TIMEOUT: Duration = Duration::from_secs(10);

fn policy_digest(value: &impl Serialize) -> Result<String> {
    Ok(hex::encode(sha2::Sha256::digest(serde_json::to_vec(
        value,
    )?)))
}

async fn capture(
    runner_id: &str,
    assignment: &Assignment,
    workspace: &WorkspaceLease,
    workspaces: &WorkspaceManager,
) -> Result<SourceCheckpoint> {
    validate_assignment_source(workspaces, assignment)?;
    let repository = assignment
        .source_repository
        .as_ref()
        .context("checkpoint requires a pinned source repository")?;
    let source_ref = assignment
        .source_base_ref
        .as_ref()
        .context("checkpoint requires a pinned source ref")?;
    let source_commit = assignment
        .source_base_commit
        .as_ref()
        .context("checkpoint requires a pinned source commit")?;
    if !source_commit.eq_ignore_ascii_case(&workspace.base_commit) {
        return Err(anyhow!(
            "checkpoint cannot replace its original source base"
        ));
    }
    let (head_commit, workspace_fingerprint) = workspaces.checkpoint(workspace).await?;
    Ok(SourceCheckpoint {
        schema_version: 1,
        corp_id: assignment.corp_id,
        mission_id: assignment.mission_id,
        task_id: assignment.task_id,
        run_id: assignment.run_id,
        workspace_run_id: assignment.workspace_run_id,
        agent_id: assignment.agent_id,
        runner_id: runner_id.to_owned(),
        source_repository: repository.clone(),
        source_base_ref: source_ref.clone(),
        source_base_commit: source_commit.clone(),
        workspace_base_commit: workspace.base_commit.clone(),
        branch: workspace.branch.clone(),
        head_commit,
        workspace_fingerprint,
        verification_policy_sha256: policy_digest(&assignment.verification_policy)?,
        write_scope_sha256: policy_digest(&assignment.write_scope)?,
        deliverable_policy_sha256: policy_digest(&assignment.deliverable)?,
    })
}

async fn bounded_capture(
    capture: impl std::future::Future<Output = Result<SourceCheckpoint>>,
    timeout: Duration,
) -> Result<SourceCheckpoint> {
    tokio::time::timeout(timeout, capture)
        .await
        .context("workspace checkpoint capture exceeded its deadline")?
}

pub(super) async fn report(
    outbound: &OutboundBus,
    runner_id: &str,
    assignment: &Assignment,
    workspace: &WorkspaceLease,
    workspaces: &WorkspaceManager,
    checkpoint_capture_allowed: bool,
    workspace_quarantined: bool,
) {
    let proof = if checkpoint_capture_allowed && !workspace_quarantined {
        bounded_capture(
            capture(runner_id, assignment, workspace, workspaces),
            CHECKPOINT_REPORT_TIMEOUT,
        )
        .await
    } else {
        Err(anyhow!(
            "pre-verification source checkpoint cannot be proven for this exit"
        ))
    };
    match proof {
        Ok(proof) => send_run_event(
            outbound,
            runner_id,
            assignment,
            "run.workspace_preserved",
            json!({
                "workspace": workspace.path,
                "workspace_branch": workspace.branch,
                "workspace_base_ref": workspace.base_ref,
                "workspace_base_commit": workspace.base_commit,
                "detail": "Stopped source retained with exact checkpoint evidence; recovery is not yet authorized.",
                "dirty": Value::Null,
                "commits_ahead": Value::Null,
                "branch_deleted": false,
                "workspace_fingerprint": proof.workspace_fingerprint,
                "head_commit": proof.head_commit,
                "source_checkpoint": proof,
            }),
        ),
        Err(error) => send_teardown_workspace_preserved(
            outbound,
            runner_id,
            assignment,
            workspace,
            &format!("hard-boundary source retained without checkpoint proof: {error:#}"),
            None,
            workspace_quarantined,
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, process::Command};

    use async_trait::async_trait;
    use tokio::sync::Notify;

    use super::*;

    fn git(root: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "fixture Git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    async fn fixture() -> (PathBuf, Arc<WorkspaceManager>, WorkspaceLease, Assignment) {
        let root = std::env::temp_dir()
            .join("ecorp-stopped-source-checkpoint-tests")
            .join(Uuid::new_v4().to_string());
        let source = root.join("source");
        std::fs::create_dir_all(&source).unwrap();
        git(&source, &["init", "-b", "main"]);
        std::fs::write(source.join("sentinel.txt"), b"original source\n").unwrap();
        git(&source, &["add", "sentinel.txt"]);
        git(
            &source,
            &[
                "-c",
                "user.name=ECorp Test",
                "-c",
                "user.email=test@ecorp.invalid",
                "commit",
                "-m",
                "source",
            ],
        );
        let workspaces = Arc::new(
            WorkspaceManager::initialize(root.join("managed"), source, "HEAD".to_owned())
                .await
                .unwrap(),
        );
        let run_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();
        let workspace = workspaces
            .prepare(task_id, run_id, Some(workspaces.base_commit()), None)
            .await
            .unwrap();
        let assignment = Assignment {
            corp_id: Uuid::new_v4(),
            connection_epoch: Uuid::new_v4(),
            room_id: Uuid::new_v4(),
            mission_id: Uuid::new_v4(),
            task_id,
            run_id,
            workspace_run_id: run_id,
            agent_id: Uuid::new_v4(),
            assignment_token: Uuid::new_v4(),
            adapter: "checkpoint-fixture".to_owned(),
            mission_title: "Retain stopped source".to_owned(),
            model: None,
            reasoning_effort: None,
            source_repository: workspaces.repository_identity().map(str::to_owned),
            source_base_ref: Some(workspaces.base_ref().to_owned()),
            source_base_commit: Some(workspaces.base_commit().to_owned()),
            resume_workspace_base_commit: None,
            verification_policy: VerificationPolicy {
                checks: vec![crony_domain::VerifierCheck::File {
                    path: "result.md".to_owned(),
                    min_bytes: 1,
                }],
                manual_gate: None,
            },
            write_scope: vec!["result.md".to_owned()],
            deliverable: None,
            secrets: Vec::new(),
            expected_workspace_fingerprint: None,
            expected_head_commit: None,
            provider_artifact: None,
            checkpoint_verification: false,
            hard_boundary_checkpoint: Arc::default(),
        };
        (root, workspaces, workspace, assignment)
    }

    fn events(outbound: &OutboundBus) -> Vec<(String, Value)> {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        outbound.attach(sender, Uuid::new_v4());
        let mut recorded = Vec::new();
        while let Ok(message) = receiver.try_recv() {
            if let RunnerToServer::RunEvent {
                event_type,
                payload,
                ..
            } = message
            {
                recorded.push((event_type, payload));
            }
        }
        recorded
    }

    fn remove_fixture(root: PathBuf) {
        let parent = std::env::temp_dir().join("ecorp-stopped-source-checkpoint-tests");
        let canonical = std::fs::canonicalize(&root).unwrap();
        assert!(
            canonical.starts_with(std::fs::canonicalize(parent).unwrap()),
            "cleanup must stay in this test's owned fixture tree"
        );
        assert!(
            root.file_name()
                .unwrap()
                .to_str()
                .is_some_and(|name| { Uuid::parse_str(name).is_ok() })
        );
        std::fs::remove_dir_all(canonical).unwrap();
    }

    #[derive(Clone, Copy)]
    enum Exit {
        Completed,
        Cancelled,
        Failed,
        RuntimeError,
    }

    struct StoppedAdapter {
        ready: Arc<Notify>,
        exit: Exit,
        uncertain: bool,
    }

    #[async_trait]
    impl AgentAdapter for StoppedAdapter {
        fn id(&self) -> &'static str {
            "checkpoint-fixture"
        }

        fn display_name(&self) -> &'static str {
            "Native checkpoint test"
        }

        fn capabilities(&self) -> adapter::AdapterCapabilities {
            let supported = adapter::FeatureSupport::Supported;
            adapter::AdapterCapabilities {
                spawn: supported.clone(),
                stream: supported.clone(),
                steer: supported.clone(),
                interrupt: supported.clone(),
                stop: supported.clone(),
                resume: adapter::FeatureSupport::Unsupported {
                    reason: "This fixture must never resume a provider".to_owned(),
                },
                usage: supported.clone(),
                artifacts: supported,
            }
        }

        async fn execute(
            &self,
            request: AdapterRunRequest,
            mut controls: mpsc::UnboundedReceiver<AdapterControl>,
            sink: Arc<dyn AdapterEventSink>,
        ) -> Result<AdapterExit, adapter::AdapterError> {
            sink.emit(AdapterEvent::Started {
                workspace: request.workspace.clone(),
            });
            std::fs::write(
                request.workspace.join("result.md"),
                b"completed application source\n",
            )?;
            self.ready.notify_one();
            let control = controls.recv().await.expect("native control");
            assert!(matches!(control, AdapterControl::CircuitBreaker { .. }));
            if matches!(&control, AdapterControl::CircuitBreaker { stage, .. }
                if matches!(stage.as_str(), "suspend" | "stop"))
            {
                // Real adapters can emit their final local transcript even
                // after native interruption. It must not become a late upload.
                let evidence = request.workspace.join(".crony/provider-evidence.txt");
                std::fs::create_dir_all(evidence.parent().unwrap())?;
                let bytes = b"stopped provider transcript\n";
                std::fs::write(&evidence, bytes)?;
                sink.emit(AdapterEvent::Artifact(AdapterArtifact {
                    path: evidence,
                    sha256: hex::encode(sha2::Sha256::digest(bytes)),
                    bytes: bytes.len(),
                    media_type: "text/plain".to_owned(),
                }));
            }
            if self.uncertain {
                sink.emit(AdapterEvent::TeardownUncertain {
                    detail: "fixture cannot prove provider teardown".to_owned(),
                });
            }
            Ok(match self.exit {
                Exit::Completed => {
                    sink.emit(AdapterEvent::Completed {
                        summary: "provider completion racing stop".to_owned(),
                    });
                    AdapterExit::Completed
                }
                Exit::Cancelled => {
                    sink.emit(AdapterEvent::Cancelled {
                        reason: "native circuit breaker stopped provider".to_owned(),
                    });
                    AdapterExit::Cancelled
                }
                Exit::Failed => {
                    sink.emit(AdapterEvent::Failed {
                        error: "native provider failed while stopping".to_owned(),
                    });
                    AdapterExit::Failed
                }
                Exit::RuntimeError => {
                    return Err(adapter::AdapterError::Runtime(anyhow!(
                        "uncertain runtime exit"
                    )));
                }
            })
        }
    }

    async fn terminal_case(stage: &str, exit: Exit, uncertain: bool, pinned: bool) {
        let (root, workspaces, workspace, mut assignment) = fixture().await;
        if !pinned {
            assignment.source_repository = None;
            assignment.source_base_ref = None;
            assignment.source_base_commit = None;
        }
        let original_source = std::fs::read(root.join("source/sentinel.txt")).unwrap();
        let original_head = git(&workspace.path, &["rev-parse", "HEAD"]);
        let ready = Arc::new(Notify::new());
        let adapter: Arc<dyn AgentAdapter> = Arc::new(StoppedAdapter {
            ready: ready.clone(),
            exit,
            uncertain,
        });
        let outbound = OutboundBus::default();
        let (control, controls) = mpsc::unbounded_channel();
        let (artifact_ack, artifact_acks) = mpsc::unbounded_channel();
        let native_control = ActiveRunControl {
            assignment_token: assignment.assignment_token,
            control,
            artifact_ack,
            hard_boundary_checkpoint: assignment.hard_boundary_checkpoint.clone(),
        };
        let executing_assignment = assignment.clone();
        let executing_outbound = outbound.clone();
        let executing_workspaces = workspaces.clone();
        let execution = tokio::spawn(async move {
            execute_assignment(
                executing_workspaces,
                "runner-checkpoint".to_owned(),
                executing_assignment,
                adapter,
                executing_outbound,
                AssignmentChannels {
                    controls,
                    artifact_acks,
                },
                None,
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(10), ready.notified())
            .await
            .unwrap();
        assert!(
            native_control.apply_circuit_breaker(stage.to_owned(), "native boundary".to_owned())
        );
        let result = tokio::time::timeout(Duration::from_secs(20), execution)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.is_err(), matches!(exit, Exit::RuntimeError));
        let recorded = events(&outbound);
        let hard = matches!(stage, "suspend" | "stop");
        let expected_proof = hard && pinned && !uncertain && !matches!(exit, Exit::RuntimeError);
        let checkpoints = recorded
            .iter()
            .enumerate()
            .filter(|(_, (_, payload))| payload.get("source_checkpoint").is_some())
            .collect::<Vec<_>>();
        assert_eq!(checkpoints.len(), usize::from(expected_proof));
        if expected_proof {
            let (index, (kind, payload)) = checkpoints[0];
            assert_eq!(kind, "run.workspace_preserved");
            let proof = &payload["source_checkpoint"];
            assert_eq!(proof["run_id"], assignment.run_id.to_string());
            assert_eq!(
                proof["workspace_run_id"],
                assignment.workspace_run_id.to_string()
            );
            assert_eq!(proof["source_base_commit"], workspace.base_commit);
            assert_eq!(proof["workspace_base_commit"], workspace.base_commit);
            assert_eq!(proof["head_commit"], original_head);
            assert_eq!(proof["branch"], workspace.branch);
            assert_eq!(
                proof["workspace_fingerprint"],
                workspaces.fingerprint(&workspace).await.unwrap()
            );
            assert_eq!(
                proof["verification_policy_sha256"],
                policy_digest(&assignment.verification_policy).unwrap()
            );
            assert_eq!(
                proof["write_scope_sha256"],
                policy_digest(&assignment.write_scope).unwrap()
            );
            assert_eq!(
                proof["deliverable_policy_sha256"],
                policy_digest(&assignment.deliverable).unwrap()
            );
            let serialized = serde_json::to_string(proof).unwrap();
            assert!(!serialized.contains("assignment_token"));
            assert!(!serialized.contains("credential"));
            assert!(!serialized.contains("workspace_path"));
            assert!(!serialized.contains(root.to_string_lossy().as_ref()));
            let terminated = recorded
                .iter()
                .position(|(kind, _)| kind == "run.session_terminated")
                .unwrap();
            let terminal = recorded
                .iter()
                .position(|(kind, _)| matches!(kind.as_str(), "run.cancelled" | "run.failed"))
                .unwrap();
            assert!(terminated < index && index < terminal);
        }
        if hard {
            assert!(!recorded.iter().any(|(kind, _)| {
                matches!(
                    kind.as_str(),
                    "run.verification_started"
                        | "run.verification_passed"
                        | "run.completed"
                        | "run.artifact_upload"
                        | "run.deliverable_upload"
                )
            }));
        } else {
            assert!(recorded.iter().any(|(kind, _)| kind == "run.completed"));
        }
        assert_eq!(
            std::fs::read(workspace.path.join("result.md")).unwrap(),
            b"completed application source\n"
        );
        assert_eq!(git(&workspace.path, &["rev-parse", "HEAD"]), original_head);
        assert_eq!(
            std::fs::read(root.join("source/sentinel.txt")).unwrap(),
            original_source
        );
        assert!(!root.join("source/result.md").exists());
        remove_fixture(root);
    }

    #[tokio::test]
    async fn issue190_suspend_wins_over_provider_completion_and_seals_source() {
        terminal_case("suspend", Exit::Completed, false, true).await;
    }

    #[tokio::test]
    async fn issue190_stop_wins_over_provider_completion_and_seals_source() {
        terminal_case("stop", Exit::Completed, false, true).await;
    }

    #[tokio::test]
    async fn issue190_cancelled_exit_checkpoints_before_terminal_event() {
        terminal_case("suspend", Exit::Cancelled, false, true).await;
    }

    #[tokio::test]
    async fn issue190_failed_exit_checkpoints_before_terminal_event() {
        terminal_case("stop", Exit::Failed, false, true).await;
    }

    #[tokio::test]
    async fn issue190_uncertain_teardown_retains_without_checkpoint_proof() {
        terminal_case("stop", Exit::Cancelled, true, true).await;
    }

    #[tokio::test]
    async fn issue190_runtime_error_retains_without_checkpoint_proof() {
        terminal_case("stop", Exit::RuntimeError, false, true).await;
    }

    #[tokio::test]
    async fn issue190_unpinned_legacy_source_retains_without_checkpoint_proof() {
        terminal_case("suspend", Exit::Completed, false, false).await;
    }

    #[tokio::test]
    async fn issue190_soft_directive_keeps_normal_verification() {
        terminal_case("constrain", Exit::Completed, false, true).await;
    }

    #[tokio::test]
    async fn issue190_checkpoint_rejects_detached_or_replaced_branch_and_source_base() {
        let (root, workspaces, workspace, mut assignment) = fixture().await;
        let original_head = git(&workspace.path, &["rev-parse", "HEAD"]);
        git(&workspace.path, &["checkout", "--detach"]);
        assert!(
            capture("runner-test", &assignment, &workspace, &workspaces)
                .await
                .is_err()
        );
        git(&workspace.path, &["checkout", "-b", "fixture-other"]);
        assert!(
            capture("runner-test", &assignment, &workspace, &workspaces)
                .await
                .is_err()
        );
        git(&workspace.path, &["checkout", &workspace.branch]);
        assignment.source_base_commit = Some("f".repeat(40));
        assert!(
            capture("runner-test", &assignment, &workspace, &workspaces)
                .await
                .is_err()
        );
        assert_eq!(git(&workspace.path, &["rev-parse", "HEAD"]), original_head);
        assert!(workspace.path.exists());
        remove_fixture(root);
    }

    #[tokio::test]
    async fn issue190_checkpoint_binds_changed_policy_and_untracked_bytes() {
        let (root, workspaces, workspace, mut assignment) = fixture().await;
        let first = capture("runner-test", &assignment, &workspace, &workspaces)
            .await
            .unwrap();
        std::fs::write(
            workspace.path.join("result.md"),
            b"retained changed bytes\n",
        )
        .unwrap();
        assignment.write_scope.push("extra.md".to_owned());
        assignment
            .verification_policy
            .checks
            .push(crony_domain::VerifierCheck::File {
                path: "extra.md".to_owned(),
                min_bytes: 1,
            });
        let second = capture("runner-test", &assignment, &workspace, &workspaces)
            .await
            .unwrap();
        assert_ne!(first.workspace_fingerprint, second.workspace_fingerprint);
        assert_ne!(first.write_scope_sha256, second.write_scope_sha256);
        assert_ne!(
            first.verification_policy_sha256,
            second.verification_policy_sha256
        );
        assert_eq!(first.source_base_commit, second.source_base_commit);
        assert_eq!(first.head_commit, second.head_commit);
        remove_fixture(root);
    }

    #[tokio::test]
    async fn issue190_quarantine_never_mints_or_downgrades_a_checkpoint() {
        let (root, workspaces, workspace, assignment) = fixture().await;
        let outbound = OutboundBus::default();
        report(
            &outbound,
            "runner-test",
            &assignment,
            &workspace,
            &workspaces,
            true,
            true,
        )
        .await;
        let recorded = events(&outbound);
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, "run.workspace_preserved");
        assert!(recorded[0].1.get("source_checkpoint").is_none());
        assert_eq!(recorded[0].1["workspace_fingerprint"], Value::Null);
        assert_eq!(recorded[0].1["workspace_quarantined"], true);
        assert!(workspace.path.exists());
        remove_fixture(root);
    }

    async fn late_stop_case(hold_upload: bool) {
        let (root, workspaces, workspace, mut assignment) = fixture().await;
        if hold_upload {
            assignment.deliverable = Some(DeliverableSpec {
                form: crony_domain::DeliverableForm::CommitBranch,
                commit_after_verification: true,
                paths: vec!["result.md".to_owned()],
            });
        } else {
            assignment.verification_policy.checks = vec![crony_domain::VerifierCheck::Command {
                program: "node".to_owned(),
                args: vec![
                    "-e".to_owned(),
                    "require('node:fs').writeFileSync('verifier-started','started'); setInterval(()=>{},1000);"
                        .to_owned(),
                ],
                timeout_ms: 20_000,
            }];
        }
        let ready = Arc::new(Notify::new());
        let adapter: Arc<dyn AgentAdapter> = Arc::new(StoppedAdapter {
            ready: ready.clone(),
            exit: Exit::Completed,
            uncertain: false,
        });
        let outbound = OutboundBus::default();
        let (sender, mut received) = mpsc::unbounded_channel();
        outbound.attach(sender, Uuid::new_v4());
        let (control, controls) = mpsc::unbounded_channel();
        let (artifact_ack, artifact_acks) = mpsc::unbounded_channel();
        let native_control = ActiveRunControl {
            assignment_token: assignment.assignment_token,
            control,
            artifact_ack,
            hard_boundary_checkpoint: assignment.hard_boundary_checkpoint.clone(),
        };
        let execution = tokio::spawn(execute_assignment(
            workspaces,
            "runner-late-stop".to_owned(),
            assignment,
            adapter,
            outbound,
            AssignmentChannels {
                controls,
                artifact_acks,
            },
            None,
        ));
        tokio::time::timeout(Duration::from_secs(10), ready.notified())
            .await
            .unwrap();
        assert!(
            native_control.apply_circuit_breaker("constrain".to_owned(), "continue".to_owned())
        );
        let mut recorded = Vec::new();
        if hold_upload {
            tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    let message = received.recv().await.unwrap();
                    if let RunnerToServer::RunEvent {
                        event_type,
                        payload,
                        ..
                    } = message
                    {
                        let uploading = event_type == "run.deliverable_upload";
                        recorded.push((event_type, payload));
                        if uploading {
                            break;
                        }
                    }
                }
            })
            .await
            .unwrap();
        } else {
            tokio::time::timeout(Duration::from_secs(10), async {
                while !workspace.path.join("verifier-started").exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        }
        // Provider execute has returned; its control receiver is now gone.
        // The same native directive must cancel verification or its held ACK.
        assert!(
            native_control
                .apply_circuit_breaker("stop".to_owned(), "late hard boundary".to_owned())
        );
        tokio::time::timeout(Duration::from_secs(10), execution)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        while let Ok(message) = received.try_recv() {
            if let RunnerToServer::RunEvent {
                event_type,
                payload,
                ..
            } = message
            {
                recorded.push((event_type, payload));
            }
        }
        assert!(recorded.iter().any(|(kind, _)| kind == "run.cancelled"));
        assert!(!recorded.iter().any(|(kind, _)| {
            matches!(
                kind.as_str(),
                "run.verification_passed" | "run.completed" | "run.verification_waiting"
            )
        }));
        assert!(
            !recorded
                .iter()
                .any(|(_, payload)| payload.get("source_checkpoint").is_some())
        );
        let preserved = recorded
            .iter()
            .find(|(kind, _)| kind == "run.workspace_preserved")
            .unwrap();
        assert_eq!(preserved.1["workspace_fingerprint"], Value::Null);
        assert!(workspace.path.exists());
        assert_eq!(
            std::fs::read(workspace.path.join("result.md")).unwrap(),
            b"completed application source\n"
        );
        assert!(!root.join("source/result.md").exists());
        remove_fixture(root);
    }

    #[tokio::test]
    async fn issue190_late_stop_cancels_running_verifier_without_post_verifier_seal() {
        late_stop_case(false).await;
    }

    #[tokio::test]
    async fn issue190_late_stop_cancels_held_ack_without_acceptance_or_post_verifier_seal() {
        late_stop_case(true).await;
    }

    #[tokio::test]
    async fn issue190_checkpoint_read_limits_preserve_source() {
        let (root, workspaces, workspace, assignment) = fixture().await;
        let original = std::fs::read(workspace.path.join("sentinel.txt")).unwrap();
        let bytes_error =
            workspace::fingerprint_checkpoint_path(&workspace.path, 1, Duration::from_secs(5))
                .await
                .unwrap_err();
        assert!(bytes_error.to_string().contains("byte limit"));
        let time_error =
            workspace::fingerprint_checkpoint_path(&workspace.path, 4096, Duration::ZERO)
                .await
                .unwrap_err();
        assert!(time_error.to_string().contains("deadline"));
        assert_eq!(
            std::fs::read(workspace.path.join("sentinel.txt")).unwrap(),
            original
        );
        assert!(
            capture("runner-test", &assignment, &workspace, &workspaces)
                .await
                .is_ok()
        );
        remove_fixture(root);
    }

    #[tokio::test]
    async fn issue190_blocked_checkpoint_capture_has_an_overall_deadline() {
        let started = tokio::time::Instant::now();
        let error = bounded_capture(
            std::future::pending::<Result<SourceCheckpoint>>(),
            Duration::from_millis(10),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("deadline"));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn issue190_stopped_artifact_callback_cannot_fail_an_unreadable_local_artifact() {
        let (root, _workspaces, workspace, assignment) = fixture().await;
        assert!(assignment.hard_boundary_checkpoint.request());
        let outbound = OutboundBus::default();
        let terminal = Arc::new(Mutex::new(None));
        let sink = RunnerEventSink {
            outbound: outbound.clone(),
            runner_id: "runner-stopped-artifact".to_owned(),
            assignment,
            workspace: workspace.clone(),
            artifacts: Arc::new(Mutex::new(Vec::new())),
            terminal: terminal.clone(),
            teardown_uncertain: Arc::new(AtomicBool::new(false)),
        };
        sink.emit(AdapterEvent::Artifact(AdapterArtifact {
            path: workspace.path.join("never-created-transcript.txt"),
            sha256: "a".repeat(64),
            bytes: 10,
            media_type: "text/plain".to_owned(),
        }));
        assert!(events(&outbound).is_empty());
        assert!(terminal.lock().unwrap().is_none());
        assert!(workspace.path.join("sentinel.txt").exists());
        remove_fixture(root);
    }

    #[tokio::test]
    async fn issue190_cleanup_commit_rejects_a_late_retention_acknowledgment() {
        let (root, workspaces, workspace, mut assignment) = fixture().await;
        assignment.verification_policy.checks = vec![crony_domain::VerifierCheck::Command {
            program: "node".to_owned(),
            args: vec![
                "-e".to_owned(),
                "require('node:fs').unlinkSync('result.md')".to_owned(),
            ],
            timeout_ms: 10_000,
        }];
        let ready = Arc::new(Notify::new());
        let adapter: Arc<dyn AgentAdapter> = Arc::new(StoppedAdapter {
            ready: ready.clone(),
            exit: Exit::Completed,
            uncertain: false,
        });
        let outbound = OutboundBus::default();
        let executing_outbound = outbound.clone();
        let (control, controls) = mpsc::unbounded_channel();
        let (artifact_ack, artifact_acks) = mpsc::unbounded_channel();
        let native_control = ActiveRunControl {
            assignment_token: assignment.assignment_token,
            control,
            artifact_ack,
            hard_boundary_checkpoint: assignment.hard_boundary_checkpoint.clone(),
        };
        let run_id = assignment.run_id;
        let execution = tokio::spawn(execute_assignment(
            workspaces.clone(),
            "runner-finalize".to_owned(),
            assignment,
            adapter,
            executing_outbound,
            AssignmentChannels {
                controls,
                artifact_acks,
            },
            None,
        ));
        tokio::time::timeout(Duration::from_secs(10), ready.notified())
            .await
            .unwrap();
        // Native prepare has finished. Hold the real workspace-operation lock
        // so finalize cannot complete until after the late directive is tested.
        let held_finalize = workspaces.hold_operations_for_test().await;
        assert!(
            native_control.apply_circuit_breaker("constrain".to_owned(), "continue".to_owned())
        );
        tokio::time::timeout(Duration::from_secs(10), async {
            while native_control
                .hard_boundary_checkpoint
                .phase
                .load(Ordering::Acquire)
                != HardBoundaryControl::FINALIZING
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(workspace.path.exists());
        let active_runs = Arc::new(DashMap::new());
        active_runs.insert(run_id, native_control.clone());
        let seen_commands = DashMap::new();
        let command_id = Uuid::new_v4();
        for _ in 0..2 {
            assert_eq!(
                apply_circuit_breaker_command(
                    &seen_commands,
                    &active_runs,
                    command_id,
                    run_id,
                    "stop".to_owned(),
                    "too late".to_owned(),
                ),
                (false, false),
            );
        }
        assert!(
            seen_commands.is_empty(),
            "negative ACKs must not be cached as applied"
        );
        assert!(!native_control.hard_boundary_checkpoint.requested());
        drop(held_finalize);
        tokio::time::timeout(Duration::from_secs(10), execution)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let recorded = events(&outbound);
        assert!(
            recorded
                .iter()
                .any(|(kind, _)| kind == "run.workspace_removed")
        );
        assert!(
            !recorded
                .iter()
                .any(|(_, payload)| payload.get("source_checkpoint").is_some())
        );
        assert!(!workspace.path.exists());
        assert!(root.join("source/sentinel.txt").exists());
        remove_fixture(root);
    }
}
