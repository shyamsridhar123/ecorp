use super::*;
use serde_json::json;

fn fixture() -> (Registry, WorkspaceSetupCommand, Snapshot) {
    let command = WorkspaceSetupCommand {
        operation_id: Uuid::from_u128(1),
        corp_id: Uuid::from_u128(2),
        room_id: Uuid::from_u128(3),
        actor_id: Uuid::from_u128(4),
        connection_owner_id: Some(Uuid::from_u128(4)),
        runner_id: "connection-fixture".to_owned(),
        expected_connection_version: Some(1),
        action: WorkspaceSetupAction::Connect {
            connection_id: Uuid::from_u128(5),
            configuration: WorkspaceConnectionConfiguration {
                repository: WorkspaceRepositorySetup::GitHub {
                    repository: "team/source".to_owned(),
                    repository_id: Some("R_fixture".to_owned()),
                    base_ref: "main".to_owned(),
                    account: GitHubAccountSource::Personal,
                },
                agent: CodingAgent::Codex,
                use_system_installation: false,
                use_machine_account: None,
            },
        },
        expires_at: Utc::now() + chrono::Duration::minutes(5),
    };
    let state = Registry {
        schema_version: REGISTRY_SCHEMA,
        corp_id: command.corp_id,
        runner_id: command.runner_id.clone(),
        github_accounts: BTreeMap::new(),
        connections: BTreeMap::new(),
        operations: BTreeMap::new(),
    };
    let snapshot = Snapshot {
        source: WorkspaceSourceIdentity {
            repository: "team/source".to_owned(),
            repository_id: Some("R_fixture".to_owned()),
            base_ref: "main".to_owned(),
            base_commit: "a".repeat(40),
        },
        models: vec![RunnerModel {
            id: "fixture-model".to_owned(),
            name: "Fixture model".to_owned(),
            policy_state: None,
            policy_terms: None,
            supports_vision: false,
            supports_reasoning_effort: false,
            max_prompt_tokens: None,
            max_context_window_tokens: None,
            supported_reasoning_efforts: vec![],
            default_reasoning_effort: None,
            billing_multiplier: None,
        }],
        agent: CodingAgent::Codex,
        use_system_installation: false,
        use_machine_account: None,
        advertised: false,
        source_accepted: false,
        accepted: false,
    };
    (state, command, snapshot)
}

fn complete(state: &mut Registry, command: &WorkspaceSetupCommand, snapshot: Snapshot) -> String {
    let id = command.action.connection_id().unwrap();
    let key = snapshot_key(&snapshot).unwrap();
    let mut result = report(
        WorkspaceSetupStatus::Succeeded,
        Some(WorkspaceConnectionStatus::Ready),
        "Fixture native readiness; no inference.",
    );
    result.source = Some(snapshot.source.clone());
    result.models = snapshot.models.clone();
    state
        .connections
        .get_mut(&id)
        .unwrap()
        .snapshots
        .insert(key.clone(), snapshot);
    let operation = state.operations.get_mut(&command.operation_id).unwrap();
    operation.report = Some(result);
    operation.candidate = Some(key.clone());
    key
}

#[test]
fn installed_executable_and_native_account_are_independent_choices() {
    let (_, command, mut snapshot) = fixture();
    let configuration = configuration(&command.action).unwrap();
    let mut private_installed = configuration.clone();
    private_installed.use_system_installation = true;
    private_installed.use_machine_account = Some(false);
    assert!(
        !private_installed
            .use_machine_account
            .unwrap_or(private_installed.use_system_installation)
    );
    let (program, prefix) = system_agent_command(CodingAgent::Codex);
    assert_eq!(program, crate::default_codex_command());
    assert!(
        prefix.is_empty(),
        "a configured fake/proxy command prefix must not leak into the installed executable"
    );
    let old = snapshot_key(&snapshot).unwrap();
    snapshot.use_machine_account = Some(false);
    assert_ne!(
        old,
        snapshot_key(&snapshot).unwrap(),
        "explicit account selection is part of the accepted snapshot"
    );
    let mut machine = private_installed.clone();
    machine.use_machine_account = Some(true);
    assert!(!immutable_configuration_matches(
        &private_installed,
        &machine
    ));
}

#[test]
fn connection_ready_requires_exact_server_ack_and_survives_registry_roundtrip() {
    let (mut state, command, snapshot) = fixture();
    begin_operation(&mut state, &command).unwrap();
    let key = complete(&mut state, &command, snapshot);
    let id = command.action.connection_id().unwrap();
    assert!(state.connections[&id].active_snapshot.is_none());
    assert!(!state.connections[&id].snapshots[&key].accepted);
    assert!(acknowledge(&mut state, Uuid::new_v4(), true).is_err());
    acknowledge(&mut state, command.operation_id, true).unwrap();
    let encoded = serde_json::to_vec(&state).unwrap();
    let restored: Registry = serde_json::from_slice(&encoded).unwrap();
    validate_registry(&restored, command.corp_id, &command.runner_id).unwrap();
    assert!(restored.connections[&id].snapshots[&key].accepted);
    assert_eq!(
        restored.connections[&id].active_snapshot.as_ref(),
        Some(&key)
    );
    let before = serde_json::to_value(&state).unwrap();
    acknowledge(&mut state, command.operation_id, true).unwrap();
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
    assert!(acknowledge(&mut state, command.operation_id, false).is_err());
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
}

#[test]
fn rejected_ready_and_forged_acceptance_are_not_dispatch_authority() {
    let (mut state, command, snapshot) = fixture();
    begin_operation(&mut state, &command).unwrap();
    let key = complete(&mut state, &command, snapshot);
    acknowledge(&mut state, command.operation_id, false).unwrap();
    let connection = state
        .connections
        .get_mut(&command.action.connection_id().unwrap())
        .unwrap();
    assert!(connection.active_snapshot.is_none());
    assert!(!connection.snapshots[&key].accepted);
    connection.snapshots.get_mut(&key).unwrap().accepted = true;
    assert!(validate_registry(&state, command.corp_id, &command.runner_id).is_err());
}

#[test]
fn branch_refresh_keeps_old_accepted_pin_and_fences_older_ready_ack() {
    let (mut state, first, old) = fixture();
    begin_operation(&mut state, &first).unwrap();
    let old_key = complete(&mut state, &first, old.clone());
    acknowledge(&mut state, first.operation_id, true).unwrap();
    let mut next = first.clone();
    next.operation_id = Uuid::new_v4();
    next.expected_connection_version = Some(2);
    let configuration = configuration(&next.action).unwrap().clone();
    next.action = WorkspaceSetupAction::Test {
        connection_id: next.action.connection_id().unwrap(),
        configuration,
    };
    begin_operation(&mut state, &next).unwrap();
    let mut new = old.clone();
    new.source.base_commit = "b".repeat(40);
    let new_key = complete(&mut state, &next, new.clone());
    let id = first.action.connection_id().unwrap();
    assert!(state.connections[&id].active_snapshot.is_none());
    assert!(state.connections[&id].snapshots[&old_key].accepted);
    assert!(!state.connections[&id].snapshots[&new_key].accepted);
    acknowledge(&mut state, next.operation_id, true).unwrap();
    assert!(source_matches(
        &state.connections[&id].snapshots[&old_key].source,
        &old.source
    ));
    assert!(!source_matches(
        &state.connections[&id].snapshots[&new_key].source,
        &old.source
    ));
    validate_registry(&state, first.corp_id, &first.runner_id).unwrap();

    let mut pending = next.clone();
    pending.operation_id = Uuid::new_v4();
    pending.expected_connection_version = Some(3);
    begin_operation(&mut state, &pending).unwrap();
    complete(&mut state, &pending, new);
    let mut superseding = pending.clone();
    superseding.operation_id = Uuid::new_v4();
    // Same version does not let an older operation supersede a newer private operation.
    begin_operation(&mut state, &superseding).unwrap();
    let previous_source = state.connections[&id].source_snapshot.clone();
    acknowledge(&mut state, pending.operation_id, true).unwrap();
    assert_eq!(state.connections[&id].source_snapshot, previous_source);
    assert!(state.connections[&id].active_snapshot.is_none());
}

#[test]
fn operation_replay_preserves_native_login_receipt_and_rejects_changed_scope() {
    let (mut state, command, _) = fixture();
    begin_operation(&mut state, &command).unwrap();
    state
        .operations
        .get_mut(&command.operation_id)
        .unwrap()
        .login_started = true;
    let before = serde_json::to_value(&state).unwrap();
    begin_operation(&mut state, &command).unwrap();
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
    for field in ["actor", "owner", "room", "version", "expiry"] {
        let mut changed = command.clone();
        match field {
            "actor" => changed.actor_id = Uuid::new_v4(),
            "owner" => changed.connection_owner_id = Some(Uuid::new_v4()),
            "room" => changed.room_id = Uuid::new_v4(),
            "version" => changed.expected_connection_version = Some(2),
            "expiry" => changed.expires_at += chrono::Duration::seconds(1),
            _ => unreachable!(),
        }
        assert!(begin_operation(&mut state, &changed).is_err(), "{field}");
        assert_eq!(serde_json::to_value(&state).unwrap(), before);
    }
    let mut another_owner = command.clone();
    another_owner.operation_id = Uuid::new_v4();
    another_owner.connection_owner_id = Some(Uuid::new_v4());
    another_owner.expected_connection_version = Some(2);
    assert!(begin_operation(&mut state, &another_owner).is_err());
    assert!(validate_registry(&state, Uuid::new_v4(), &command.runner_id).is_err());
}

fn flight() -> (Flight, mpsc::Receiver<NativeSignInResponse>, Uuid) {
    let (done, _) = watch::channel(None);
    let (code_sender, receiver) = mpsc::channel(1);
    let id = Uuid::new_v4();
    (
        Flight {
            done,
            recent: Mutex::new(None),
            observer: Mutex::new(None),
            code_sender,
            code_waiting: Arc::new(AtomicBool::new(true)),
            code_prompt_id: Arc::new(Mutex::new(Some(id))),
            submitted_codes: Mutex::new(HashMap::new()),
            claude_sign_in: true,
            expires_at: Utc::now() + chrono::Duration::minutes(1),
        },
        receiver,
        id,
    )
}

#[tokio::test]
async fn native_code_is_prompt_bound_one_shot_and_exact_retries_never_feed_another_prompt() {
    let (flight, mut receiver, id) = flight();
    let response = NativeSignInResponse {
        input_id: id,
        authorization_code: "fixture-one-time-code".to_owned(),
    };
    assert!(
        submit_code(
            &flight,
            NativeSignInResponse {
                input_id: Uuid::new_v4(),
                authorization_code: response.authorization_code.clone(),
            }
        )
        .is_err()
    );
    submit_code(&flight, response.clone()).unwrap();
    assert_eq!(receiver.recv().await.unwrap().input_id, id);
    assert!(!flight.code_waiting.load(Ordering::Acquire));
    let next = Uuid::new_v4();
    *flight.code_prompt_id.lock().unwrap() = Some(next);
    flight.code_waiting.store(true, Ordering::Release);
    submit_code(&flight, response.clone()).unwrap();
    assert!(receiver.try_recv().is_err());
    assert!(flight.code_waiting.load(Ordering::Acquire));
    let changed = NativeSignInResponse {
        input_id: id,
        authorization_code: "different-fixture-code".to_owned(),
    };
    assert!(submit_code(&flight, changed).is_err());
    submit_code(
        &flight,
        NativeSignInResponse {
            input_id: next,
            authorization_code: "next-fixture-code".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(receiver.recv().await.unwrap().input_id, next);
}

#[test]
fn native_code_cannot_target_non_claude_or_expired_login_and_is_not_serialized() {
    let (mut flight, _receiver, id) = flight();
    let input = NativeSignInResponse {
        input_id: id,
        authorization_code: "fixture-code-not-a-token".to_owned(),
    };
    flight.claude_sign_in = false;
    assert!(submit_code(&flight, input.clone()).is_err());
    flight.claude_sign_in = true;
    flight.expires_at = Utc::now() - chrono::Duration::seconds(1);
    assert!(submit_code(&flight, input).is_err());
    let (state, _, _) = fixture();
    let serialized = serde_json::to_string(&state).unwrap();
    assert!(!serialized.contains("authorization_code"));
    assert!(!serialized.contains("verification_uri"));
}

#[test]
fn saved_native_identity_never_falls_back_to_another_source_or_profile() {
    let (_, command, snapshot) = fixture();
    let configuration = configuration(&command.action).unwrap();
    let mut changed = configuration.clone();
    changed.use_system_installation = true;
    assert!(!immutable_configuration_matches(configuration, &changed));
    changed = configuration.clone();
    changed.agent = CodingAgent::ClaudeCode;
    assert!(!immutable_configuration_matches(configuration, &changed));
    let mut foreign = snapshot.source.clone();
    foreign.repository_id = Some("R_other".to_owned());
    assert!(!source_matches(&snapshot.source, &foreign));
    foreign = snapshot.source.clone();
    foreign.base_ref = "other".to_owned();
    assert!(!source_matches(&snapshot.source, &foreign));
    assert!(
        validate_source(&WorkspaceSourceIdentity {
            repository: "team/source".to_owned(),
            repository_id: None,
            base_ref: "--bad".to_owned(),
            base_commit: "a".repeat(40),
        })
        .is_err()
    );
    assert_eq!(
        json!(WorkspaceSetupAction::SignInGitHub)["kind"],
        "sign_in_github"
    );
}

#[test]
fn completed_auth_check_is_terminal_and_source_survives_later_agent_failure() {
    let missing = needs_sign_in("Native login is absent.");
    assert!(missing.status.terminal());
    assert_eq!(
        missing.connection_status,
        Some(WorkspaceConnectionStatus::NeedsSignIn)
    );
    assert!(missing.sign_in.is_none());

    let (mut state, first, source) = fixture();
    begin_operation(&mut state, &first).unwrap();
    let original = complete(&mut state, &first, source.clone());
    acknowledge(&mut state, first.operation_id, true).unwrap();
    let mut failed = first.clone();
    failed.operation_id = Uuid::new_v4();
    failed.expected_connection_version = Some(2);
    begin_operation(&mut state, &failed).unwrap();
    let mut unauthenticated = source.clone();
    unauthenticated.models.clear();
    let retained = complete(&mut state, &failed, unauthenticated);
    let receipt = state
        .operations
        .get_mut(&failed.operation_id)
        .unwrap()
        .report
        .as_mut()
        .unwrap();
    receipt.status = WorkspaceSetupStatus::Succeeded;
    receipt.connection_status = Some(WorkspaceConnectionStatus::NeedsSignIn);
    acknowledge(&mut state, failed.operation_id, true).unwrap();
    let connection = &state.connections[&first.action.connection_id().unwrap()];
    assert!(connection.active_snapshot.is_none());
    assert!(connection.snapshots[&original].source_accepted);
    assert!(connection.snapshots[&retained].source_accepted);
    assert!(!connection.snapshots[&retained].accepted);
    validate_registry(&state, first.corp_id, &first.runner_id).unwrap();
}

#[test]
fn late_ack_recognizes_server_committed_history_without_reselecting_old_profile() {
    let (mut state, first, source) = fixture();
    begin_operation(&mut state, &first).unwrap();
    let first_key = complete(&mut state, &first, source.clone());
    let mut newer = first.clone();
    newer.operation_id = Uuid::new_v4();
    newer.expected_connection_version = Some(2);
    begin_operation(&mut state, &newer).unwrap();
    let mut new_source = source;
    new_source.source.base_commit = "b".repeat(40);
    let newer_key = complete(&mut state, &newer, new_source);
    acknowledge(&mut state, newer.operation_id, true).unwrap();
    acknowledge(&mut state, first.operation_id, true).unwrap();
    let connection = &state.connections[&first.action.connection_id().unwrap()];
    assert_eq!(connection.active_snapshot.as_ref(), Some(&newer_key));
    assert_eq!(connection.source_snapshot.as_ref(), Some(&newer_key));
    assert!(connection.snapshots[&first_key].source_accepted);
    validate_registry(&state, first.corp_id, &first.runner_id).unwrap();
}
