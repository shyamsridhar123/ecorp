//! Explicitly owned native-compatibility probe. Never run in the ordinary suite.
//! This reads a stopped session; it cannot authorize or publish a Factory result.

use super::*;
use std::io::Write;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnedProbe {
    schema_version: u32,
    test_owned: bool,
    source_run_id: Uuid,
    provider_session_id: String,
    workspace: PathBuf,
    expected_workspace_fingerprint: String,
    expected_head_commit: String,
    profile_root: PathBuf,
    copilot_home: PathBuf,
    expected_account_sha256: String,
    receipt_path: PathBuf,
    #[serde(default)]
    read: adapter::StoppedSessionRead,
}

async fn head(workspace: &std::path::Path) -> String {
    let output = tokio::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(workspace)
        .output()
        .await
        .expect("read preserved HEAD");
    assert!(output.status.success(), "preserved HEAD is unreadable");
    String::from_utf8(output.stdout)
        .expect("HEAD is UTF-8")
        .trim()
        .to_owned()
}

#[tokio::test]
#[ignore = "requires an explicit owned stopped-session probe file; never starts inference"]
async fn issue211_native_stopped_session_read_without_inference() {
    let path = std::env::var_os("ECORP_ISSUE211_NATIVE_PROBE_FILE")
        .map(PathBuf::from)
        .expect("explicit owned probe file required");
    assert!(path.is_absolute() && path.is_file());
    let bytes = std::fs::read(&path).expect("read owned probe parameters");
    assert!(bytes.len() <= 16_384);
    let probe: OwnedProbe = serde_json::from_slice(&bytes).expect("typed owned probe");
    assert!(probe.test_owned && probe.schema_version == 1);
    assert!(!probe.source_run_id.is_nil());
    assert!(Uuid::parse_str(&probe.provider_session_id).is_ok_and(|id| !id.is_nil()));
    for directory in [&probe.workspace, &probe.profile_root, &probe.copilot_home] {
        assert!(directory.is_absolute() && directory.is_dir());
    }
    let workspace = probe.workspace.canonicalize().expect("owned workspace");
    let profile = probe.profile_root.canonicalize().expect("owned profile");
    let home = probe
        .copilot_home
        .canonicalize()
        .expect("owned native home");
    assert!(!workspace.starts_with(&profile) && !profile.starts_with(&workspace));
    assert!(!workspace.starts_with(&home) && !home.starts_with(&workspace));
    let output_parent = probe
        .receipt_path
        .parent()
        .expect("receipt directory")
        .canonicalize()
        .expect("pre-existing owned evidence directory");
    assert!(
        !output_parent.starts_with(&workspace)
            && !output_parent.starts_with(&profile)
            && !output_parent.starts_with(&home)
    );
    assert!(
        !probe.receipt_path.exists(),
        "never overwrite earlier evidence"
    );
    assert_eq!(
        std::env::var_os("COPILOT_HOME").map(PathBuf::from),
        Some(probe.copilot_home.clone()),
        "native home must be explicitly selected by the approved launcher"
    );
    let before = workspace::fingerprint_path(&probe.workspace)
        .await
        .expect("fingerprint preserved source before read");
    assert_eq!(before, probe.expected_workspace_fingerprint);
    assert_eq!(head(&probe.workspace).await, probe.expected_head_commit);
    let copilot = CopilotSdkConfig {
        cli_path: None,
        cli_prefix_args: Vec::new(),
        external_host: None,
        external_port: None,
        github_token_file: None,
        connection_token_file: None,
        base_directory: probe.profile_root.join("session-fs"),
        use_logged_in_user: true,
        log_level: "error".to_owned(),
        fixture: false,
    };
    let config = AdapterRegistryConfig {
        fake_agent_script: PathBuf::new(),
        codex_command: PathBuf::new(),
        codex_prefix_args: Vec::new(),
        claude_command: PathBuf::new(),
        claude_prefix_args: Vec::new(),
        opencode_command: PathBuf::new(),
        opencode_prefix_args: Vec::new(),
        copilot,
    };
    let profile = adapter::connection::AgentProfile::new(
        config,
        crony_domain::CodingAgent::GitHubCopilot,
        probe.profile_root.clone(),
        probe.workspace.clone(),
        true,
    )
    .expect("same owning system-account profile");
    let started = Utc::now();
    let result = profile
        .adapter()
        .collect_stopped_session_evidence(
            &probe.workspace,
            &probe.provider_session_id,
            &probe.expected_account_sha256,
            probe.read,
        )
        .await;
    let after = workspace::fingerprint_path(&probe.workspace)
        .await
        .expect("fingerprint preserved source after read");
    let head_after = head(&probe.workspace).await;
    let receipt = json!({
        "schema_version": 1,
        "probe": "native_stopped_session_read_without_inference",
        "started_at": started,
        "finished_at": Utc::now(),
        "source_run_id": probe.source_run_id,
        "provider_session_id": probe.provider_session_id,
        "read": probe.read,
        "workspace_fingerprint_before": before,
        "workspace_fingerprint_after": after,
        "head_after": head_after,
        "collection": match &result {
            Ok(evidence) => json!({"ok":true,"evidence":evidence}),
            Err(error) => json!({"ok":false,"error":error.to_string()}),
        },
        "acceptance_scope": "native read compatibility only; no Factory acceptance or publication"
    });
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&probe.receipt_path)
        .expect("create unique owned probe receipt");
    file.write_all(&serde_json::to_vec_pretty(&receipt).expect("safe receipt"))
        .expect("retain probe result");
    assert_eq!(after, probe.expected_workspace_fingerprint);
    assert_eq!(head_after, probe.expected_head_commit);
    assert!(
        result.is_ok(),
        "native read did not produce evidence; inspect the retained bounded receipt"
    );
}
