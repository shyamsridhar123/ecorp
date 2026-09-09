use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use chrono::{Duration as ChronoDuration, Utc};
use crony_domain::{
    CodingAgent, NativeSignInInputKind, NativeSignInResponse, WorkspaceConnectionStatus,
};
use serde_json::{Value, json};
use tokio::{io::BufReader, process::Command, sync::mpsc};
use uuid::Uuid;

use super::{
    AgentProfile, NativeCodeReceiver, catalog,
    environment::{NativeCommand, ProfileEnvironment},
};
use crate::adapter::{AdapterRegistryConfig, CopilotSdkAdapter, CopilotSdkConfig};

struct Fixture {
    root: PathBuf,
    home: PathBuf,
    workspace: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir()
            .join("ecorp-native-profile-tests")
            .join(Uuid::new_v4().to_string());
        let home = root.join("profile");
        let workspace = root.join("source");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        Self {
            root,
            home,
            workspace,
        }
    }

    fn scope(&self, agent: CodingAgent, system: bool) -> ProfileEnvironment {
        ProfileEnvironment::new(agent, self.home.clone(), system).unwrap()
    }

    fn config(&self) -> AdapterRegistryConfig {
        let missing = self.root.join("not-installed-native-agent");
        AdapterRegistryConfig {
            fake_agent_script: missing.clone(),
            codex_command: missing.clone(),
            codex_prefix_args: Vec::new(),
            claude_command: missing.clone(),
            claude_prefix_args: Vec::new(),
            opencode_command: missing.clone(),
            opencode_prefix_args: Vec::new(),
            copilot: CopilotSdkConfig {
                cli_path: Some(missing),
                cli_prefix_args: Vec::new(),
                external_host: None,
                external_port: None,
                github_token_file: None,
                connection_token_file: None,
                base_directory: self.root.join("legacy-sdk-state"),
                use_logged_in_user: true,
                log_level: "error".to_owned(),
                fixture: false,
            },
        }
    }

    fn profile(&self, agent: CodingAgent, system: bool) -> AgentProfile {
        AgentProfile::new(
            self.config(),
            agent,
            self.home.clone(),
            self.workspace.clone(),
            system,
        )
        .unwrap()
    }

    #[cfg(windows)]
    fn native_config(&self, agent: CodingAgent) -> AdapterRegistryConfig {
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/adapter/connection/fixtures/native-profile.mjs");
        let provider = match agent {
            CodingAgent::Codex => "codex",
            CodingAgent::ClaudeCode => "claude",
            CodingAgent::GitHubCopilot => "copilot",
        };
        let native = NativeCommand::configured("node".into(), Vec::new()).unwrap();
        let node = native
            .program
            .expect("protocol fixture requires installed node");
        let prefix = vec![
            script.into_os_string(),
            provider.into(),
            self.root.clone().into_os_string(),
        ];
        let mut config = self.config();
        config.codex_command = node.clone();
        config.codex_prefix_args = prefix.clone();
        config.claude_command = node.clone();
        config.claude_prefix_args = prefix.clone();
        config.copilot.cli_path = Some(node);
        config.copilot.cli_prefix_args = prefix;
        config
    }

    #[cfg(windows)]
    fn native_profile(&self, agent: CodingAgent) -> AgentProfile {
        AgentProfile::new(
            self.native_config(agent),
            agent,
            self.home.clone(),
            self.workspace.clone(),
            false,
        )
        .unwrap()
    }

    #[cfg(windows)]
    fn methods(&self) -> Vec<String> {
        std::fs::read_to_string(self.root.join("calls.jsonl"))
            .unwrap()
            .lines()
            .map(|line| {
                let value: Value = serde_json::from_str(line).unwrap();
                assert_eq!(
                    PathBuf::from(value["home"].as_str().unwrap()),
                    self.home.join("native")
                );
                value["method"].as_str().unwrap().to_owned()
            })
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!(
                "Preserved native profile test fixture after panic: {}",
                self.root.display()
            );
            return;
        }
        assert!(
            self.root
                .starts_with(std::env::temp_dir().join("ecorp-native-profile-tests"))
        );
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn inputs() -> (mpsc::Sender<NativeSignInResponse>, NativeCodeReceiver) {
    let (sender, receiver) = mpsc::channel(1);
    (
        sender,
        NativeCodeReceiver {
            receiver,
            waiting: Arc::new(AtomicBool::new(false)),
            prompt_id: Arc::new(Mutex::new(None)),
        },
    )
}

fn command_env(command: &Command, key: &str) -> Option<OsString> {
    command
        .as_std()
        .get_envs()
        .find(|(name, _)| name.eq_ignore_ascii_case(OsStr::new(key)))
        .and_then(|(_, value)| value.map(OsStr::to_os_string))
}

#[test]
fn native_profile_constructor_is_lazy_and_rejects_overlapping_storage() {
    let fixture = Fixture::new();
    for agent in [
        CodingAgent::Codex,
        CodingAgent::ClaudeCode,
        CodingAgent::GitHubCopilot,
    ] {
        let _profile = fixture.profile(agent, false);
        assert!(!fixture.home.join("native").exists());
    }
    assert!(
        AgentProfile::new(
            fixture.config(),
            CodingAgent::Codex,
            fixture.workspace.join("credentials"),
            fixture.workspace.clone(),
            false
        )
        .is_err()
    );
    assert!(
        AgentProfile::new(
            fixture.config(),
            CodingAgent::Codex,
            PathBuf::from("relative-home"),
            fixture.workspace.clone(),
            false
        )
        .is_err()
    );
}

#[test]
fn native_profile_environment_is_shared_by_probe_execute_and_resume() {
    let fixture = Fixture::new();
    for (agent, key) in [
        (CodingAgent::Codex, "CODEX_HOME"),
        (CodingAgent::ClaudeCode, "CLAUDE_CONFIG_DIR"),
        (CodingAgent::GitHubCopilot, "COPILOT_HOME"),
    ] {
        let scope = fixture.scope(agent, false);
        let mut probe = Command::new("operator-command");
        scope.apply(&mut probe);
        let mut execution = Command::new("operator-command");
        scope
            .apply_request(&mut execution, &HashMap::new())
            .unwrap();
        assert_eq!(
            command_env(&probe, key),
            Some(fixture.home.join("native").into_os_string())
        );
        assert_eq!(command_env(&probe, key), command_env(&execution, key));
        assert!(command_env(&probe, "OPENAI_API_KEY").is_none());
        for key in [
            "CODEX_HOME",
            "CLAUDE_CONFIG_DIR",
            "COPILOT_DISABLE_KEYTAR",
            "gh_token",
            "PATH",
            "NODE_OPTIONS",
            "HTTPS_PROXY",
        ] {
            assert!(
                scope
                    .apply_request(
                        &mut execution,
                        &HashMap::from([(key.to_owned(), "override".to_owned())])
                    )
                    .is_err()
            );
        }
        // The preserved worktree is not required to equal today's source path.
        scope
            .validate_workspace(&fixture.root.join("preserved-old-worktree"))
            .unwrap();
        let system = fixture.scope(agent, true);
        let mut native_system = Command::new("operator-command");
        system.apply(&mut native_system);
        assert_eq!(native_system.as_std().get_envs().count(), 0);
    }
}

#[tokio::test]
async fn native_system_sign_in_and_expired_inspection_spawn_nothing() {
    let fixture = Fixture::new();
    let (_sender, input) = inputs();
    let system = fixture.profile(CodingAgent::Codex, true);
    assert!(
        system
            .sign_in(
                Utc::now() + ChronoDuration::seconds(30),
                Arc::new(|_| panic!("unexpected login")),
                input
            )
            .await
            .is_err()
    );
    let personal = fixture.profile(CodingAgent::Codex, false);
    assert!(
        personal
            .inspect_before(Utc::now() - ChronoDuration::seconds(1))
            .await
            .is_err()
    );
    assert!(!fixture.home.join("native").exists());
}

#[tokio::test]
async fn native_missing_install_is_not_ready() {
    let fixture = Fixture::new();
    let readiness = fixture
        .profile(CodingAgent::Codex, false)
        .inspect()
        .await
        .unwrap();
    assert_eq!(readiness.status, WorkspaceConnectionStatus::NotInstalled);
    assert!(readiness.models.is_empty());
}

#[tokio::test]
async fn native_copilot_home_is_not_its_per_worktree_session_filesystem() {
    let fixture = Fixture::new();
    let mut config = fixture.config().copilot;
    config.cli_path = Some(std::env::current_exe().unwrap()); // Construction only.
    config.github_token_file = Some(fixture.root.join("must-not-be-read"));
    let adapter =
        CopilotSdkAdapter::for_connection(config, fixture.scope(CodingAgent::GitHubCopilot, false));
    let first = adapter
        .connection_options(&fixture.workspace)
        .await
        .unwrap();
    let preserved = fixture.root.join("preserved-worktree");
    let second = adapter.connection_options(&preserved).await.unwrap();
    assert_eq!(first.base_directory, Some(fixture.home.join("native")));
    assert_eq!(first.base_directory, second.base_directory);
    assert_eq!(first.mode, github_copilot_sdk::ClientMode::Empty);
    assert!(first.github_token.is_none());
    assert_ne!(
        first.session_fs.as_ref().unwrap().session_state_path,
        second.session_fs.as_ref().unwrap().session_state_path
    );
    assert!(
        !PathBuf::from(&first.session_fs.as_ref().unwrap().session_state_path)
            .starts_with(fixture.home.join("native"))
    );
    let probe = super::copilot::probe_command(&first).unwrap();
    assert_eq!(
        command_env(&probe, "COPILOT_HOME"),
        first.base_directory.map(PathBuf::into_os_string)
    );
    assert_eq!(
        command_env(&probe, "COPILOT_DISABLE_KEYTAR"),
        Some("1".into())
    );
    let args = probe.as_std().get_args().collect::<Vec<_>>();
    assert!(args.contains(&OsStr::new("--no-auto-update")));
    assert!(args.contains(&OsStr::new("--stdio")));
    assert!(!args.contains(&OsStr::new("--prompt")));
}

#[test]
fn native_copilot_never_falls_back_to_path_or_a_fixture_as_readiness() {
    let fixture = Fixture::new();
    let mut config = fixture.config();
    config.copilot.cli_path = Some("copilot".into());
    assert!(
        AgentProfile::new(
            config,
            CodingAgent::GitHubCopilot,
            fixture.home.clone(),
            fixture.workspace.clone(),
            false
        )
        .is_err()
    );
    let mut config = fixture.config();
    config.copilot.fixture = true;
    assert!(
        AgentProfile::new(
            config,
            CodingAgent::GitHubCopilot,
            fixture.home.clone(),
            fixture.workspace.clone(),
            false
        )
        .is_err()
    );
    assert!(!crate::adapter::copilot::runtime_version_is_supported(
        "1.0.83"
    ));
}

#[test]
fn native_auth_and_catalog_parsing_fail_closed() {
    use catalog::AccountState;
    assert!(matches!(
        catalog::codex_account(&json!({"account":null})).unwrap(),
        AccountState::SignedOut
    ));
    assert!(catalog::codex_account(&json!({"account":{"type":"unrecognized"}})).is_err());
    assert!(matches!(
        catalog::claude_account(&json!({"loggedIn":false}), false).unwrap(),
        AccountState::SignedOut
    ));
    assert!(catalog::claude_account(&json!({"loggedIn":true}), false).is_err());
    assert!(catalog::claude_account(&json!({"status":"authenticated"}), true).is_err());
    let models = catalog::codex_models(&json!({"data":[{
        "id":"native-codex", "displayName":"Native Codex",
        "supportedReasoningEfforts":[{"reasoningEffort":"high"}],
        "defaultReasoningEffort":"high", "inputModalities":["text","image"]
    }]}))
    .unwrap();
    assert!(models[0].supports_vision);
    assert_eq!(models[0].supported_reasoning_efforts, ["high"]);
    let claude = catalog::claude_models(&json!({"models":[{
        "value":"native-claude", "displayName":"Native Claude", "supportsEffort":true,
        "supportedEffortLevels":["low","high"]
    }]}))
    .unwrap();
    assert!(claude[0].supports_reasoning_effort);
    assert!(catalog::claude_models(&json!({"models":[{"value":"invalid\nmodel"}]})).is_err());
    assert!(catalog::validate_models(&[models[0].clone(), models[0].clone()]).is_err());
    assert!(super::AgentReadiness::ready(Vec::new(), None).is_err());
    assert!(super::claude::initialization_response(&json!({
        "type":"control_response", "response":{"subtype":"success","request_id":"foreign","response":{}}
    }), "owned").is_err());
}

#[test]
fn native_ready_rejects_disabled_or_unknown_policy_only_catalogs() {
    for state in ["disabled", "disallowed", "future-policy-state"] {
        let model = native_policy_model("native-policy-blocked", Some(state));
        let readiness = super::AgentReadiness::ready(vec![model], None)
            .unwrap_err()
            .readiness();
        assert_eq!(readiness.status, WorkspaceConnectionStatus::Incompatible);
        assert!(readiness.models.is_empty());
    }
}

#[test]
fn native_ready_mixed_catalog_advertises_only_usable_models() {
    let readiness = super::AgentReadiness::ready(
        vec![
            native_policy_model("native-disabled", Some("disabled")),
            native_policy_model("native-enabled", Some("enabled")),
            native_policy_model("native-unknown", Some("future-policy-state")),
            native_policy_model("native-unconfigured", Some("unconfigured")),
            native_policy_model("native-no-policy", None),
        ],
        None,
    )
    .unwrap();
    assert_eq!(readiness.status, WorkspaceConnectionStatus::Ready);
    assert_eq!(
        readiness
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        ["native-enabled", "native-unconfigured", "native-no-policy"]
    );
}

#[test]
fn native_ready_preserves_enabled_and_policy_absent_native_catalogs() {
    let codex = catalog::codex_models(&json!({
        "data":[{"id":"native-codex","displayName":"Native Codex"}]
    }))
    .unwrap();
    let claude = catalog::claude_models(&json!({
        "models":[{"value":"native-claude","displayName":"Native Claude"}]
    }))
    .unwrap();
    for models in [
        vec![native_policy_model("native-enabled", Some("enabled"))],
        vec![native_policy_model(
            "native-unconfigured",
            Some("unconfigured"),
        )],
        codex,
        claude,
    ] {
        let expected = models.clone();
        let readiness = super::AgentReadiness::ready(models, None).unwrap();
        assert_eq!(readiness.status, WorkspaceConnectionStatus::Ready);
        assert_eq!(readiness.models, expected);
    }
}

fn native_policy_model(id: &str, state: Option<&str>) -> crony_domain::RunnerModel {
    let mut value = json!({"id":id,"name":id,"capabilities":{}});
    if let Some(state) = state {
        value["policy"] = json!({"state":state});
    }
    // Exercise the pinned SDK's policy enum and the production normalization,
    // including its forward-compatible Unknown variant; no native client starts.
    let model: github_copilot_sdk::Model = serde_json::from_value(value).unwrap();
    catalog::runner_model(crate::adapter::copilot::model_from_sdk(model))
}

#[test]
fn native_login_instructions_allow_only_native_urls_and_safe_codes() {
    use catalog::{sign_in_instruction, verification_uri_allowed};
    let expiry = Utc::now() + ChronoDuration::seconds(60);
    let codex = sign_in_instruction(
        CodingAgent::Codex,
        "https://auth.openai.com/codex/device",
        Some("TEST-1234"),
        expiry,
        None,
    )
    .unwrap();
    assert!(codex.input_id.is_none());
    assert!(codex.input_kind.is_none());
    for uri in [
        "http://github.com/login/device",
        "https://github.com.evil.test/login/device",
        "https://secret@github.com/login/device",
        "https://github.com/login/device?token=secret",
        "https://github.com/login/device#secret",
    ] {
        assert!(!verification_uri_allowed(CodingAgent::GitHubCopilot, uri));
    }
    assert!(
        sign_in_instruction(
            CodingAgent::Codex,
            "https://auth.openai.com/codex/device",
            Some("code\ninjection"),
            expiry,
            None
        )
        .is_err()
    );
    let native = "https://claude.ai/oauth/authorize?code=true&client_id=fixture&response_type=code&redirect_uri=https%3A%2F%2Fconsole.anthropic.com%2Foauth%2Fcode%2Fcallback&state=fixture&code_challenge=fixture&code_challenge_method=S256";
    assert!(verification_uri_allowed(CodingAgent::ClaudeCode, native));
    assert!(!verification_uri_allowed(
        CodingAgent::ClaudeCode,
        &format!("{native}&access_token=secret")
    ));
    assert!(!verification_uri_allowed(
        CodingAgent::ClaudeCode,
        &native.replace("code=true", "code=actual-authorization-code")
    ));
    assert!(!verification_uri_allowed(
        CodingAgent::ClaudeCode,
        &native.replace("response_type=code", "response_type=token")
    ));
    let mut transcript = catalog::LoginTranscript::default();
    transcript.push(native).unwrap();
    assert!(transcript.uri(CodingAgent::ClaudeCode).is_none()); // Not yet delimited.
    transcript.push("\nPaste code here if prompted > ").unwrap();
    assert!(transcript.claude_code_prompt());
    assert_eq!(
        transcript.uri(CodingAgent::ClaudeCode).as_deref(),
        Some(native)
    );
}

#[test]
fn native_code_input_is_bounded_and_cleared_on_cancellation() {
    for code in ["", "code\r\nother-input", "code\n", "code\0", "code other"] {
        assert!(!catalog::authorization_code_allowed(code));
    }
    assert!(!catalog::authorization_code_allowed(&"x".repeat(2049)));
    assert!(catalog::authorization_code_allowed(
        "native-code#native-state"
    ));
    let (_sender, input) = inputs();
    let waiting = input.waiting.clone();
    let prompt_id = input.prompt_id.clone();
    waiting.store(true, Ordering::Release);
    *prompt_id.lock().unwrap() = Some(Uuid::new_v4());
    drop(input);
    assert!(!waiting.load(Ordering::Acquire));
    assert!(prompt_id.lock().unwrap().is_none());
}

#[tokio::test]
async fn native_protocol_reader_bounds_before_allocating_untrusted_lines() {
    let data = vec![b'x'; 64 * 1024 + 1];
    let mut reader = super::process::BoundedLines::new(BufReader::new(data.as_slice()));
    assert!(reader.next().await.is_err());
}

#[cfg(windows)]
#[tokio::test]
async fn native_codex_device_login_and_inspection_never_send_a_turn() {
    let fixture = Fixture::new();
    let profile = fixture.native_profile(CodingAgent::Codex);
    assert!(!fixture.root.join("calls.jsonl").exists()); // No constructor probe.
    let reports = Arc::new(Mutex::new(Vec::new()));
    let capture = reports.clone();
    let (_sender, input) = inputs();
    let readiness = profile
        .sign_in(
            Utc::now() + ChronoDuration::seconds(30),
            Arc::new(move |report| capture.lock().unwrap().push(report)),
            input,
        )
        .await
        .unwrap();
    assert_eq!(readiness.status, WorkspaceConnectionStatus::Ready);
    assert_eq!(readiness.models[0].id, "fixture-codex");
    assert_eq!(
        reports.lock().unwrap()[0].user_code.as_deref(),
        Some("TEST-1234")
    );
    assert!(reports.lock().unwrap()[0].input_id.is_none());
    assert!(fixture.methods().iter().all(|method| matches!(
        method.as_str(),
        "initialize" | "initialized" | "account/read" | "account/login/start" | "model/list"
    )));
}

#[cfg(windows)]
#[tokio::test]
async fn native_claude_code_bridge_binds_prompt_and_adopts_persisted_auth() {
    let fixture = Fixture::new();
    let profile = fixture.native_profile(CodingAgent::ClaudeCode);
    let (sender, input) = inputs();
    let waiting = input.waiting.clone();
    let prompt_id = input.prompt_id.clone();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let callback_waiting = waiting.clone();
    let callback_id = prompt_id.clone();
    let callback_observed = observed.clone();
    let readiness = profile
        .sign_in(
            Utc::now() + ChronoDuration::seconds(30),
            Arc::new(move |report| {
                if report.input_kind == Some(NativeSignInInputKind::AuthorizationCode) {
                    let id = report.input_id.unwrap();
                    assert!(callback_waiting.load(Ordering::Acquire));
                    assert_eq!(*callback_id.lock().unwrap(), Some(id));
                    callback_observed.lock().unwrap().push(id);
                    sender
                        .try_send(NativeSignInResponse {
                            input_id: id,
                            authorization_code: "fixture-authorization-code".to_owned(),
                        })
                        .unwrap();
                }
            }),
            input,
        )
        .await
        .unwrap();
    assert_eq!(readiness.status, WorkspaceConnectionStatus::Ready);
    assert_eq!(observed.lock().unwrap().len(), 1);
    assert!(!waiting.load(Ordering::Acquire));
    assert!(prompt_id.lock().unwrap().is_none());
    let reconstructed = fixture.native_profile(CodingAgent::ClaudeCode);
    let (_sender, input) = inputs();
    assert_eq!(
        reconstructed
            .sign_in(
                Utc::now() + ChronoDuration::seconds(30),
                Arc::new(|_| panic!("must adopt existing sign-in")),
                input
            )
            .await
            .unwrap()
            .status,
        WorkspaceConnectionStatus::Ready
    );
    let methods = fixture.methods();
    assert_eq!(
        methods
            .iter()
            .filter(|method| *method == "claude.auth.login")
            .count(),
        1
    );
    assert!(methods.iter().all(|method| matches!(
        method.as_str(),
        "claude.auth.status"
            | "claude.auth.login"
            | "claude.initialize"
            | "claude.authorization-code-received"
    )));
}

#[cfg(windows)]
#[tokio::test]
async fn native_claude_rejects_foreign_prompt_input_without_writing_it() {
    let fixture = Fixture::new();
    let profile = fixture.native_profile(CodingAgent::ClaudeCode);
    let (sender, input) = inputs();
    let waiting = input.waiting.clone();
    let prompt_id = input.prompt_id.clone();
    let readiness = profile
        .sign_in(
            Utc::now() + ChronoDuration::seconds(30),
            Arc::new(move |report| {
                if report.input_kind.is_some() {
                    sender
                        .try_send(NativeSignInResponse {
                            input_id: Uuid::new_v4(),
                            authorization_code: "fixture-authorization-code".to_owned(),
                        })
                        .unwrap();
                }
            }),
            input,
        )
        .await
        .unwrap();
    assert_eq!(readiness.status, WorkspaceConnectionStatus::Failed);
    assert!(
        !fixture
            .methods()
            .contains(&"claude.authorization-code-received".to_owned())
    );
    assert!(!waiting.load(Ordering::Acquire));
    assert!(prompt_id.lock().unwrap().is_none());
}

#[cfg(windows)]
#[tokio::test]
async fn native_copilot_device_login_uses_same_empty_profile_and_no_sessions() {
    let fixture = Fixture::new();
    let profile = fixture.native_profile(CodingAgent::GitHubCopilot);
    let (_sender, input) = inputs();
    let readiness = profile
        .sign_in(
            Utc::now() + ChronoDuration::seconds(30),
            Arc::new(|report| {
                assert_eq!(report.verification_uri, "https://github.com/login/device");
                assert_eq!(report.user_code.as_deref(), Some("TEST-1234"));
                assert!(report.input_id.is_none());
            }),
            input,
        )
        .await
        .unwrap();
    assert_eq!(readiness.status, WorkspaceConnectionStatus::Ready);
    assert_eq!(readiness.models[0].id, "fixture-copilot");
    assert!(fixture.methods().iter().all(|method| matches!(
        method.as_str(),
        "connect" | "ping" | "status.get" | "auth.getStatus" | "models.list" | "copilot.login"
    )));
    std::fs::write(fixture.root.join("wrong-version"), "fixture").unwrap();
    assert_eq!(
        profile.inspect().await.unwrap().status,
        WorkspaceConnectionStatus::Incompatible
    );
}
