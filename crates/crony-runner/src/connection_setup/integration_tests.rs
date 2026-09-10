//! Windows native-process fixtures; no real account, provider turn, or network.
//! Wire from connections.rs with #[cfg(all(test, windows))].
#![cfg(all(test, windows))]

use std::{
    fs,
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};

use chrono::Utc;
use crony_domain::{
    CodingAgent, GitHubAccountSource, WorkspaceConnectionConfiguration, WorkspaceConnectionStatus,
    WorkspaceRepositorySetup, WorkspaceSetupAction, WorkspaceSetupCommand, WorkspaceSetupReport,
    WorkspaceSetupStatus, WorkspaceSourceIdentity,
};
use serde_json::{Value, json};
use tokio::process::Command;
use uuid::Uuid;

use crate::{
    adapter::{AdapterRegistry, AdapterRegistryConfig, CopilotSdkConfig},
    connections::{ConnectionManager, ConnectionManagerConfig, SetupProgress},
    workspace::WorkspaceManager,
};

const RUNNER: &str = "native-connection-fixture";
const FIXTURE_PARENT: &str = "ecorp-native-connection-integration";

struct Fixture {
    root: PathBuf,
    source: PathBuf,
    git: PathBuf,
    adapters: AdapterRegistryConfig,
    defaults: Arc<AdapterRegistry>,
    workspaces: Arc<WorkspaceManager>,
    corp: Uuid,
    room: Uuid,
    alice: Uuid,
    bob: Uuid,
    head_a: String,
    head_b: String,
}

impl Fixture {
    async fn new() -> Self {
        let root = std::env::temp_dir()
            .join(FIXTURE_PARENT)
            .join(Uuid::new_v4().to_string());
        let source = root.join("source");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir(root.join("empty-home")).unwrap();
        fs::create_dir(root.join("empty-hooks")).unwrap();
        fs::write(root.join("empty.gitconfig"), "").unwrap();
        let git = executable("git.exe");
        let node = executable("node.exe");
        git_fixture(&git, &root, &source, &["init", "-b", "main"]).await;
        fs::write(source.join("revision.txt"), "A\n").unwrap();
        git_fixture(&git, &root, &source, &["add", "revision.txt"]).await;
        git_fixture(&git, &root, &source, &["commit", "-m", "fixture A"]).await;
        let head_a = git_fixture(&git, &root, &source, &["rev-parse", "HEAD"]).await;
        fs::write(source.join("revision.txt"), "B\n").unwrap();
        git_fixture(&git, &root, &source, &["add", "revision.txt"]).await;
        git_fixture(&git, &root, &source, &["commit", "-m", "fixture B"]).await;
        let head_b = git_fixture(&git, &root, &source, &["rev-parse", "HEAD"]).await;
        assert_ne!(head_a, head_b);
        // A and B exist BEFORE cloning. Only this fake API selector changes later.
        fs::write(root.join("selected-head"), &head_a).unwrap();
        fs::write(
            root.join("fixture.json"),
            serde_json::to_vec(&json!({ "source": source, "git": git })).unwrap(),
        )
        .unwrap();
        let gh_script = root.join("fake-gh.mjs");
        let codex_script = root.join("fake-codex.mjs");
        fs::write(&gh_script, include_str!("fixtures/fake-gh.mjs")).unwrap();
        fs::write(&codex_script, include_str!("fixtures/fake-codex.mjs")).unwrap();
        wrapper(&root.join("fake-gh.cmd"), &node, &[&gh_script, &root]);
        wrapper(&root.join("fixture-node.cmd"), &node, &[]);
        let missing = root.join("not-installed.exe");
        let adapters = AdapterRegistryConfig {
            fake_agent_script: missing.clone(),
            codex_command: root.join("fixture-node.cmd"),
            codex_prefix_args: vec![codex_script.into_os_string(), root.clone().into_os_string()],
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
                base_directory: root.join("unused-copilot"),
                use_logged_in_user: false,
                log_level: "error".to_owned(),
                fixture: false,
            },
        };
        let defaults = Arc::new(AdapterRegistry::new(adapters.clone()));
        let workspaces = Arc::new(
            WorkspaceManager::initialize(
                root.join("default-workspaces"),
                source.clone(),
                "main".into(),
            )
            .await
            .unwrap(),
        );
        Self {
            root,
            source,
            git,
            adapters,
            defaults,
            workspaces,
            corp: Uuid::new_v4(),
            room: Uuid::new_v4(),
            alice: Uuid::new_v4(),
            bob: Uuid::new_v4(),
            head_a,
            head_b,
        }
    }

    async fn manager(&self) -> ConnectionManager {
        ConnectionManager::new(ConnectionManagerConfig {
            root: self.root.join("connections"),
            corp_id: self.corp,
            runner_id: RUNNER.to_owned(),
            default_workspaces: self.workspaces.clone(),
            default_adapters: self.defaults.clone(),
            adapter_config: self.adapters.clone(),
            github_command: self.root.join("fake-gh.cmd"),
            allowed_local_roots: vec![self.source.clone()],
        })
        .await
        .expect("open the real manager with owned native fixture executables")
    }

    fn command(
        &self,
        actor: Uuid,
        action: WorkspaceSetupAction,
        version: Option<i64>,
    ) -> WorkspaceSetupCommand {
        WorkspaceSetupCommand {
            operation_id: Uuid::new_v4(),
            corp_id: self.corp,
            room_id: self.room,
            actor_id: actor,
            connection_owner_id: action.connection_id().map(|_| actor),
            runner_id: RUNNER.to_owned(),
            expected_connection_version: version,
            action,
            expires_at: Utc::now() + chrono::Duration::minutes(5),
        }
    }

    fn configuration(&self) -> WorkspaceConnectionConfiguration {
        WorkspaceConnectionConfiguration {
            repository: WorkspaceRepositorySetup::GitHub {
                repository: "team/repo".to_owned(),
                repository_id: Some("R_fixture_repo".to_owned()),
                base_ref: "main".to_owned(),
                account: GitHubAccountSource::Personal,
            },
            agent: CodingAgent::Codex,
            use_system_installation: false,
            use_machine_account: None,
        }
    }

    fn calls(&self) -> Vec<Value> {
        fs::read_to_string(self.root.join("calls.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn count(&self, method: &str) -> usize {
        self.calls()
            .iter()
            .filter(|call| call["method"] == method)
            .count()
    }

    fn assert_scopes(&self) {
        for call in self.calls() {
            assert_ne!(call["method"], "forbidden", "{call}");
            if call["tool"] == "codex" && call["method"] != "version" {
                assert!(
                    ["initialize", "initialized", "account/read", "model/list"]
                        .contains(&call["method"].as_str().unwrap()),
                    "{call}"
                );
                let home = self
                    .root
                    .join("connections/accounts")
                    .join(call["owner"].as_str().unwrap())
                    .join("agents")
                    .join(call["connection"].as_str().unwrap())
                    .join("native");
                assert_eq!(
                    fs::canonicalize(call["home"].as_str().unwrap()).unwrap(),
                    fs::canonicalize(home).unwrap()
                );
            } else if call["tool"] == "gh" && call["owner"] != "machine" {
                let home = self
                    .root
                    .join("connections/accounts")
                    .join(call["owner"].as_str().unwrap())
                    .join("github/native");
                assert_eq!(
                    fs::canonicalize(call["home"].as_str().unwrap()).unwrap(),
                    fs::canonicalize(home).unwrap()
                );
            }
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("Retaining failed native fixture at {}", self.root.display());
            return;
        }
        let parent = fs::canonicalize(std::env::temp_dir().join(FIXTURE_PARENT)).unwrap();
        let root = fs::canonicalize(&self.root).unwrap();
        assert_eq!(root.parent(), Some(parent.as_path()));
        make_fixture_writable(&root);
        fs::remove_dir_all(root).expect("remove only the verified, owned fixture");
    }
}

// Windows-only cleanup of owned Git object files; never change a user's ACLs.
#[allow(clippy::permissions_set_readonly_false)]
fn make_fixture_writable(path: &Path) {
    let metadata = fs::symlink_metadata(path).unwrap();
    assert_eq!(
        metadata.file_attributes() & 0x400,
        0,
        "refuse reparse-point cleanup"
    );
    if metadata.is_dir() {
        for entry in fs::read_dir(path).unwrap() {
            make_fixture_writable(&entry.unwrap().path());
        }
    } else if metadata.permissions().readonly() {
        let mut permissions = metadata.permissions();
        permissions.set_readonly(false);
        fs::set_permissions(path, permissions).unwrap();
    }
}

fn executable(name: &str) -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").expect("fixture needs operator PATH"))
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join(name))
        .find(|path| path.is_file())
        .unwrap_or_else(|| panic!("native fixture requires installed {name}"))
}

fn wrapper(target: &Path, node: &Path, prefix: &[&Path]) {
    let quote = |path: &Path| {
        let text = path.to_str().unwrap();
        assert!(!text.contains('"') && !text.chars().any(char::is_control));
        format!("\"{}\"", text.replace('%', "%%"))
    };
    let words = std::iter::once(node)
        .chain(prefix.iter().copied())
        .map(quote)
        .collect::<Vec<_>>();
    fs::write(target, format!(
        "@echo off\r\nsetlocal DisableDelayedExpansion\r\nset \"NODE_OPTIONS=\"\r\nset \"NODE_PATH=\"\r\n{} %*\r\nexit /b %errorlevel%\r\n",
        words.join(" ")
    )).unwrap();
}

async fn git_fixture(program: &Path, root: &Path, cwd: &Path, args: &[&str]) -> String {
    assert!(
        fs::canonicalize(cwd)
            .unwrap()
            .starts_with(fs::canonicalize(root).unwrap())
    );
    let mut command = Command::new(program);
    command.env_clear();
    for key in ["PATH", "SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .env("HOME", root.join("empty-home"))
        .env("USERPROFILE", root.join("empty-home"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", root.join("empty.gitconfig"))
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ALLOW_PROTOCOL", "file")
        .args([
            "-c",
            "core.hooksPath=",
            "-c",
            "commit.gpgSign=false",
            "-c",
            "user.name=ECorp Fixture",
            "-c",
            "user.email=fixture@example.invalid",
        ])
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .creation_flags(0x0800_0000)
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(30), command.output())
        .await
        .expect("bounded fixture Git command")
        .unwrap();
    assert!(
        output.status.success(),
        "fixture Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn silent() -> SetupProgress {
    Arc::new(|_| {})
}

fn same_report(left: &WorkspaceSetupReport, right: &WorkspaceSetupReport) {
    assert_eq!(
        serde_json::to_value(left).unwrap(),
        serde_json::to_value(right).unwrap()
    );
}

fn terminal_missing_auth(report: &WorkspaceSetupReport) {
    assert_eq!(report.status, WorkspaceSetupStatus::Succeeded, "{report:?}");
    assert_eq!(
        report.connection_status,
        Some(WorkspaceConnectionStatus::NeedsSignIn)
    );
    assert!(report.status.terminal());
    assert!(report.sign_in.is_none());
}

fn pending_report(
    manager: &ConnectionManager,
    operation_id: Uuid,
    expected: &WorkspaceSetupReport,
) {
    let pending = manager.pending_reports();
    assert!(
        pending
            .iter()
            .all(|(_, report)| report.status.terminal() && report.sign_in.is_none())
    );
    let (_, actual) = pending
        .iter()
        .find(|(id, _)| *id == operation_id)
        .expect("receipt awaits ACK");
    same_report(actual, expected);
}

fn ready(report: &WorkspaceSetupReport, commit: &str) -> WorkspaceSourceIdentity {
    assert_eq!(report.status, WorkspaceSetupStatus::Succeeded, "{report:?}");
    assert_eq!(
        report.connection_status,
        Some(WorkspaceConnectionStatus::Ready)
    );
    assert!(report.sign_in.is_none());
    assert_eq!(report.models.len(), 1);
    assert_eq!(report.models[0].id, "fixture-codex");
    let source = report.source.clone().unwrap();
    assert_eq!(source.repository, "team/repo");
    assert_eq!(source.repository_id.as_deref(), Some("R_fixture_repo"));
    assert_eq!(source.base_ref, "main");
    assert_eq!(source.base_commit, commit);
    source
}

#[tokio::test]
async fn native_fixture_personal_github_profiles_and_login_replay_survive_restart() {
    let fixture = Fixture::new().await;
    let manager = fixture.manager().await;
    let inspect = |actor, account| {
        fixture.command(actor, WorkspaceSetupAction::InspectGitHub { account }, None)
    };
    let machine = manager
        .execute(
            inspect(fixture.alice, GitHubAccountSource::Machine),
            silent(),
        )
        .await
        .unwrap();
    assert_eq!(machine.account_login.as_deref(), Some("fixture-machine"));
    let unsigned = manager
        .execute(
            inspect(fixture.alice, GitHubAccountSource::Personal),
            silent(),
        )
        .await
        .unwrap();
    terminal_missing_auth(&unsigned);
    let login_a = fixture.command(fixture.alice, WorkspaceSetupAction::SignInGitHub, None);
    let progress = Arc::new(Mutex::new(Vec::<WorkspaceSetupReport>::new()));
    let capture = progress.clone();
    let notify: SetupProgress = Arc::new(move |report| capture.lock().unwrap().push(report));
    let (signed, duplicate) = tokio::join!(
        manager.execute(login_a.clone(), notify.clone()),
        manager.execute(login_a.clone(), notify),
    );
    let signed = signed.unwrap();
    same_report(&signed, &duplicate.unwrap());
    assert_eq!(signed.status, WorkspaceSetupStatus::Succeeded);
    assert!(signed.sign_in.is_none());
    assert!(signed.detail.contains("secure credential store"));
    pending_report(&manager, login_a.operation_id, &signed);
    assert!(progress.lock().unwrap().iter().any(|report| {
        report.status == WorkspaceSetupStatus::NeedsSignIn
            && report.sign_in.as_ref().is_some_and(|instruction| {
                instruction.provider == "github"
                    && instruction.user_code.as_deref() == Some("TEST-1234")
            })
    }));
    assert_eq!(fixture.count("auth.login"), 1);
    let unsigned_b = manager
        .execute(
            inspect(fixture.bob, GitHubAccountSource::Personal),
            silent(),
        )
        .await
        .unwrap();
    terminal_missing_auth(&unsigned_b);
    fs::write(
        fixture
            .root
            .join(format!("file-storage-{}", fixture.bob.simple())),
        "",
    )
    .unwrap();
    let login_b = fixture.command(fixture.bob, WorkspaceSetupAction::SignInGitHub, None);
    let signed_b = manager.execute(login_b.clone(), silent()).await.unwrap();
    assert_eq!(signed_b.status, WorkspaceSetupStatus::Succeeded);
    assert!(
        signed_b
            .detail
            .contains("file-based credential storage (reduced assurance)")
    );
    assert_ne!(signed.account_login, signed_b.account_login);
    assert_eq!(fixture.count("auth.login"), 2);
    drop(manager);

    let restarted = fixture.manager().await;
    pending_report(&restarted, login_a.operation_id, &signed);
    let calls = fixture.calls();
    same_report(
        &signed,
        &restarted.execute(login_a.clone(), silent()).await.unwrap(),
    );
    same_report(
        &signed_b,
        &restarted.execute(login_b, silent()).await.unwrap(),
    );
    assert_eq!(
        fixture.calls(),
        calls,
        "receipt replay must not invoke a native command"
    );
    restarted
        .acknowledge(login_a.operation_id, true)
        .await
        .unwrap();
    assert!(
        restarted
            .pending_reports()
            .iter()
            .all(|(id, _)| *id != login_a.operation_id)
    );
    let mut stolen_replay = login_a;
    stolen_replay.actor_id = fixture.bob;
    assert!(restarted.execute(stolen_replay, silent()).await.is_err());
    let machine_after = restarted
        .execute(inspect(fixture.bob, GitHubAccountSource::Machine), silent())
        .await
        .unwrap();
    assert_eq!(machine_after.account_login, machine.account_login);
    let repositories = restarted
        .execute(
            fixture.command(
                fixture.alice,
                WorkspaceSetupAction::ListGitHubRepositories {
                    account: GitHubAccountSource::Personal,
                },
                None,
            ),
            silent(),
        )
        .await
        .unwrap();
    assert_eq!(repositories.repositories.len(), 1);
    assert_eq!(repositories.repositories[0].id, "R_fixture_repo");
    assert_eq!(repositories.repositories[0].repository, "team/repo");
    let changed_login = fixture
        .root
        .join(format!("api-login-{}", fixture.alice.simple()));
    fs::write(&changed_login, "another-fixture-account").unwrap();
    let mismatch = restarted
        .execute(
            inspect(fixture.alice, GitHubAccountSource::Personal),
            silent(),
        )
        .await
        .unwrap();
    assert_eq!(mismatch.status, WorkspaceSetupStatus::Failed);
    assert!(mismatch.account_login.is_none());
    fs::remove_file(changed_login).unwrap();
    let restored = restarted
        .execute(
            inspect(fixture.alice, GitHubAccountSource::Personal),
            silent(),
        )
        .await
        .unwrap();
    assert_eq!(restored.account_login, signed.account_login);
    assert_eq!(fixture.count("auth.login"), 2);
    assert_eq!(fixture.count("repo.clone"), 0);
    assert!(restarted.capabilities().is_empty());
    let logins = fixture
        .calls()
        .into_iter()
        .filter(|call| call["method"] == "auth.login")
        .collect::<Vec<_>>();
    assert_ne!(logins[0]["home"], logins[1]["home"]);
    assert!(logins.iter().all(|call| call["owner"] != "machine"));
    assert!(logins.iter().all(|call| call["insecure_storage"] == false));
    fixture.assert_scopes();
}

#[tokio::test]
async fn native_fixture_ready_ack_refresh_rejection_and_old_run_pins_survive_restart() {
    let fixture = Fixture::new().await;
    let manager = fixture.manager().await;
    let login = fixture.command(fixture.alice, WorkspaceSetupAction::SignInGitHub, None);
    let login_report = manager.execute(login.clone(), silent()).await.unwrap();
    assert_eq!(login_report.status, WorkspaceSetupStatus::Succeeded);
    let id = Uuid::new_v4();
    let connect = fixture.command(
        fixture.alice,
        WorkspaceSetupAction::Connect {
            connection_id: id,
            configuration: fixture.configuration(),
        },
        Some(1),
    );
    let (result, duplicate) = tokio::join!(
        manager.execute(connect.clone(), silent()),
        manager.execute(connect.clone(), silent()),
    );
    let report_a = result.unwrap();
    same_report(&report_a, &duplicate.unwrap());
    let source_a = ready(&report_a, &fixture.head_a);
    pending_report(&manager, connect.operation_id, &report_a);
    assert!(manager.resolve(id, &source_a, "codex").await.is_err());
    assert!(manager.resolve_workspace(id, &source_a).await.is_err());
    assert!(manager.capabilities().is_empty());
    assert_eq!(fixture.count("repo.clone"), 1);
    drop(manager);

    let manager = fixture.manager().await;
    pending_report(&manager, connect.operation_id, &report_a);
    let calls = fixture.calls();
    same_report(
        &report_a,
        &manager.execute(connect.clone(), silent()).await.unwrap(),
    );
    assert_eq!(fixture.calls(), calls);
    assert!(manager.resolve(id, &source_a, "codex").await.is_err());
    manager
        .acknowledge(connect.operation_id, true)
        .await
        .unwrap();
    assert!(
        manager
            .pending_reports()
            .iter()
            .all(|(id, _)| *id != connect.operation_id)
    );
    manager
        .acknowledge(connect.operation_id, true)
        .await
        .unwrap();
    let runtime = manager.resolve(id, &source_a, "codex").await.unwrap();
    assert_eq!(runtime.workspaces.base_commit(), fixture.head_a);
    assert_ne!(
        fs::canonicalize(runtime.workspaces.repository()).unwrap(),
        fs::canonicalize(&fixture.source).unwrap(),
    );
    assert!(
        fs::canonicalize(runtime.workspaces.root())
            .unwrap()
            .starts_with(fs::canonicalize(fixture.root.join("connections")).unwrap())
    );
    assert_eq!(
        runtime.adapter.list_models().await.unwrap()[0].id,
        "fixture-codex"
    );
    assert_eq!(
        git_fixture(
            &fixture.git,
            &fixture.root,
            runtime.workspaces.repository(),
            &["remote", "get-url", "origin"]
        )
        .await,
        "https://github.com/team/repo.git"
    );
    assert_eq!(
        git_fixture(
            &fixture.git,
            &fixture.root,
            runtime.workspaces.repository(),
            &["config", "--local", "--get", "core.longpaths"]
        )
        .await,
        "true"
    );
    assert_eq!(
        git_fixture(
            &fixture.git,
            &fixture.root,
            runtime.workspaces.repository(),
            &["config", "--local", "--get", "protocol.allow"]
        )
        .await,
        "never"
    );
    let task = Uuid::new_v4();
    let run = Uuid::new_v4();
    let workspace = runtime
        .workspaces
        .prepare(task, run, Some(&fixture.head_a), None)
        .await
        .unwrap();
    assert_eq!(
        fs::read_to_string(workspace.path.join("revision.txt"))
            .unwrap()
            .replace("\r\n", "\n"),
        "A\n"
    );
    fs::write(
        workspace.path.join("unfinished.txt"),
        "retain the old run\n",
    )
    .unwrap();
    let fingerprint = runtime.workspaces.fingerprint(&workspace).await.unwrap();
    let delayed = fixture.command(
        fixture.alice,
        WorkspaceSetupAction::Test {
            connection_id: id,
            configuration: fixture.configuration(),
        },
        Some(2),
    );
    let delayed_report = manager.execute(delayed.clone(), silent()).await.unwrap();
    ready(&delayed_report, &fixture.head_a);
    pending_report(&manager, delayed.operation_id, &delayed_report);
    fs::write(fixture.root.join("selected-head"), &fixture.head_b).unwrap();
    let test = fixture.command(
        fixture.alice,
        WorkspaceSetupAction::Test {
            connection_id: id,
            configuration: fixture.configuration(),
        },
        Some(3),
    );
    let report_b = manager.execute(test.clone(), silent()).await.unwrap();
    let source_b = ready(&report_b, &fixture.head_b);
    assert!(manager.resolve(id, &source_b, "codex").await.is_err());
    assert!(manager.resolve(id, &source_a, "codex").await.is_err());
    assert!(manager.resolve_workspace(id, &source_a).await.is_ok());
    manager.acknowledge(test.operation_id, true).await.unwrap();
    assert_eq!(fixture.count("auth.login"), 1);
    assert_eq!(fixture.count("repo.clone"), 1);
    drop(runtime);
    drop(manager);

    let restarted = fixture.manager().await;
    let calls = fixture.calls();
    same_report(
        &login_report,
        &restarted.execute(login, silent()).await.unwrap(),
    );
    same_report(
        &report_a,
        &restarted.execute(connect.clone(), silent()).await.unwrap(),
    );
    same_report(
        &report_b,
        &restarted.execute(test.clone(), silent()).await.unwrap(),
    );
    assert_eq!(fixture.calls(), calls);
    let capabilities = restarted.capabilities();
    for name in ["workspace-isolation", "codex"] {
        let capability = capabilities
            .iter()
            .find(|capability| {
                capability.name == name && capability.workspace_connection_id == Some(id)
            })
            .unwrap();
        assert!(capability.available);
        assert_eq!(
            capability.source_base_commit.as_deref(),
            Some(fixture.head_b.as_str())
        );
        assert_eq!(capability.models.is_empty(), name == "workspace-isolation");
    }
    pending_report(&restarted, delayed.operation_id, &delayed_report);
    assert!(
        !serde_json::to_string(&capabilities)
            .unwrap()
            .contains("fixture@example.invalid")
    );
    let old = restarted.resolve(id, &source_a, "codex").await.unwrap();
    let current = restarted.resolve(id, &source_b, "codex").await.unwrap();
    assert_eq!(old.workspaces.base_commit(), fixture.head_a);
    assert_eq!(current.workspaces.base_commit(), fixture.head_b);
    assert_eq!(old.workspaces.repository(), current.workspaces.repository());
    let resumed = old
        .workspaces
        .prepare(task, run, Some(&fixture.head_a), Some(&fixture.head_a))
        .await
        .unwrap();
    assert_eq!(resumed.path, workspace.path);
    assert_eq!(
        old.workspaces.fingerprint(&resumed).await.unwrap(),
        fingerprint
    );
    assert_eq!(
        old.adapter.list_models().await.unwrap()[0].id,
        "fixture-codex"
    );

    let calls = fixture.calls();
    let mut wrong_owner = test.clone();
    wrong_owner.operation_id = Uuid::new_v4();
    wrong_owner.expected_connection_version = Some(4);
    wrong_owner.connection_owner_id = Some(fixture.bob);
    let mut wrong_room = test.clone();
    wrong_room.operation_id = Uuid::new_v4();
    wrong_room.expected_connection_version = Some(4);
    wrong_room.room_id = Uuid::new_v4();
    let mut stolen_operation = test.clone();
    stolen_operation.actor_id = fixture.bob;
    for invalid in [wrong_owner, wrong_room, stolen_operation] {
        assert!(restarted.execute(invalid, silent()).await.is_err());
    }
    let mut stale = connect.clone();
    stale.operation_id = Uuid::new_v4();
    assert!(restarted.execute(stale, silent()).await.is_err());
    let mut changed = test.clone();
    changed.operation_id = Uuid::new_v4();
    changed.expected_connection_version = Some(4);
    if let WorkspaceSetupAction::Test { configuration, .. } = &mut changed.action
        && let WorkspaceRepositorySetup::GitHub { repository_id, .. } =
            &mut configuration.repository
    {
        *repository_id = Some("R_other".to_owned());
    }
    assert!(restarted.execute(changed, silent()).await.is_err());
    for source in [
        WorkspaceSourceIdentity {
            repository: "other/repo".into(),
            ..source_a.clone()
        },
        WorkspaceSourceIdentity {
            repository_id: Some("R_other".into()),
            ..source_a.clone()
        },
        WorkspaceSourceIdentity {
            base_ref: "other".into(),
            ..source_a.clone()
        },
        WorkspaceSourceIdentity {
            base_commit: "0".repeat(40),
            ..source_a.clone()
        },
    ] {
        assert!(restarted.resolve(id, &source, "codex").await.is_err());
        assert!(restarted.resolve_workspace(id, &source).await.is_err());
    }
    assert!(
        restarted
            .resolve(id, &source_a, "claude-code")
            .await
            .is_err()
    );
    assert_eq!(
        fixture.calls(),
        calls,
        "invalid bindings must not invoke fixture tools"
    );

    let rejected_id = Uuid::new_v4();
    let rejected = fixture.command(
        fixture.alice,
        WorkspaceSetupAction::Connect {
            connection_id: rejected_id,
            configuration: fixture.configuration(),
        },
        Some(1),
    );
    let rejected_report = restarted.execute(rejected.clone(), silent()).await.unwrap();
    ready(&rejected_report, &fixture.head_b);
    pending_report(&restarted, rejected.operation_id, &rejected_report);
    assert!(
        restarted
            .resolve(rejected_id, &source_b, "codex")
            .await
            .is_err()
    );
    restarted
        .acknowledge(rejected.operation_id, false)
        .await
        .unwrap();
    assert!(
        restarted
            .pending_reports()
            .iter()
            .all(|(id, _)| *id != rejected.operation_id)
    );
    assert!(
        restarted
            .resolve_workspace(rejected_id, &source_b)
            .await
            .is_err()
    );
    assert!(
        restarted
            .resolve(rejected_id, &source_b, "codex")
            .await
            .is_err()
    );
    assert!(
        restarted
            .acknowledge(rejected.operation_id, true)
            .await
            .is_err()
    );
    // A later native auth failure must retire provider readiness, not its accepted source.
    fs::write(
        fixture
            .root
            .join(format!("codex-signed-out-{}", id.simple())),
        "",
    )
    .unwrap();
    let missing_auth = fixture.command(
        fixture.alice,
        WorkspaceSetupAction::Test {
            connection_id: id,
            configuration: fixture.configuration(),
        },
        Some(4),
    );
    let missing_report = restarted
        .execute(missing_auth.clone(), silent())
        .await
        .unwrap();
    terminal_missing_auth(&missing_report);
    assert_eq!(missing_report.source.as_ref(), Some(&source_b));
    assert!(missing_report.models.is_empty());
    pending_report(&restarted, missing_auth.operation_id, &missing_report);
    let calls_after_auth = fixture.calls();
    restarted
        .acknowledge(missing_auth.operation_id, true)
        .await
        .unwrap();
    // This Ready A receipt is older than both selected B and the failed auth refresh.
    restarted
        .acknowledge(delayed.operation_id, true)
        .await
        .unwrap();
    assert!(
        restarted
            .pending_reports()
            .iter()
            .all(|(operation, _)| *operation != missing_auth.operation_id
                && *operation != delayed.operation_id)
    );
    assert!(restarted.resolve(id, &source_a, "codex").await.is_err());
    let retained = restarted.resolve_workspace(id, &source_a).await.unwrap();
    assert_eq!(retained.base_commit(), fixture.head_a);
    assert_eq!(retained.fingerprint(&workspace).await.unwrap(), fingerprint);
    assert_eq!(
        fixture.calls(),
        calls_after_auth,
        "workspace recovery and ACK must not inspect agent auth"
    );
    drop(restarted);
    let final_manager = fixture.manager().await;
    let calls = fixture.calls();
    same_report(
        &rejected_report,
        &final_manager.execute(rejected, silent()).await.unwrap(),
    );
    same_report(
        &missing_report,
        &final_manager
            .execute(missing_auth.clone(), silent())
            .await
            .unwrap(),
    );
    assert_eq!(fixture.calls(), calls);
    assert!(
        final_manager
            .resolve(rejected_id, &source_b, "codex")
            .await
            .is_err()
    );
    assert!(
        final_manager
            .resolve_workspace(rejected_id, &source_b)
            .await
            .is_err()
    );
    assert!(final_manager.resolve(id, &source_a, "codex").await.is_err());
    let retained = final_manager
        .resolve_workspace(id, &source_a)
        .await
        .unwrap();
    let resumed = retained
        .prepare(task, run, Some(&fixture.head_a), Some(&fixture.head_a))
        .await
        .unwrap();
    assert_eq!(resumed.path, workspace.path);
    assert_eq!(retained.fingerprint(&resumed).await.unwrap(), fingerprint);
    let capabilities = final_manager.capabilities();
    let source_capability = capabilities
        .iter()
        .find(|capability| {
            capability.name == "workspace-isolation"
                && capability.workspace_connection_id == Some(id)
        })
        .unwrap();
    assert_eq!(
        source_capability.source_base_commit.as_deref(),
        Some(fixture.head_b.as_str())
    );
    assert!(source_capability.models.is_empty());
    assert!(
        capabilities
            .iter()
            .all(|capability| capability.name != "codex"
                && capability.workspace_connection_id != Some(rejected_id))
    );
    assert!(
        final_manager
            .pending_reports()
            .iter()
            .all(|(operation, _)| *operation != missing_auth.operation_id
                && *operation != delayed.operation_id)
    );
    assert_eq!(
        fixture.calls(),
        calls,
        "retained source resolution must not reauthenticate after restart"
    );
    assert_eq!(fixture.count("auth.login"), 1);
    assert_eq!(fixture.count("repo.clone"), 2);
    assert_eq!(
        fs::read_to_string(fixture.source.join("revision.txt")).unwrap(),
        "B\n"
    );
    assert_eq!(
        git_fixture(
            &fixture.git,
            &fixture.root,
            &fixture.source,
            &["rev-parse", "HEAD"]
        )
        .await,
        fixture.head_b
    );
    fixture.assert_scopes();
}
