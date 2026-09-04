use std::{
    env,
    ffi::{OsStr, OsString},
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use anyhow::{Context, Result, anyhow};
use crony_domain::{ManualVerificationGate, VerificationPolicy, VerifierCheck};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::process::Command;
use tokio::time::Instant;

use crate::adapter::AdapterArtifact;

#[derive(Debug, Clone, Serialize)]
pub struct VerificationCheckResult {
    pub check_index: i32,
    pub kind: String,
    pub passed: bool,
    pub summary: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct VerificationReport {
    pub passed: bool,
    pub summary: String,
    pub checks: Vec<VerificationCheckResult>,
    pub manual_gate: Option<ManualVerificationGate>,
}

pub async fn verify(
    policy: &VerificationPolicy,
    workspace: &Path,
    artifacts: &[AdapterArtifact],
) -> VerificationReport {
    let mut checks = Vec::with_capacity(policy.checks.len());
    for (index, check) in policy.checks.iter().enumerate() {
        checks.push(run_check(index as i32, check, workspace, artifacts).await);
    }
    let failed = checks.iter().filter(|check| !check.passed).count();
    VerificationReport {
        passed: failed == 0,
        summary: if failed == 0 {
            format!("all {} verifier checks passed", checks.len())
        } else {
            format!("{failed} of {} verifier checks failed", checks.len())
        },
        checks,
        manual_gate: policy.manual_gate.clone(),
    }
}

async fn run_check(
    check_index: i32,
    check: &VerifierCheck,
    workspace: &Path,
    artifacts: &[AdapterArtifact],
) -> VerificationCheckResult {
    let outcome = match check {
        VerifierCheck::Artifact { min_bytes } => {
            CheckOutcome::from_result(verify_artifact(artifacts, *min_bytes).await)
        }
        VerifierCheck::File { path, min_bytes } => {
            CheckOutcome::from_result(verify_file(workspace, path, *min_bytes).await)
        }
        VerifierCheck::Command {
            program,
            args,
            timeout_ms,
        }
        | VerifierCheck::Test {
            program,
            args,
            timeout_ms,
        } => verify_command(workspace, program, args, *timeout_ms).await,
        VerifierCheck::JsonSchema {
            path,
            required_keys,
        } => CheckOutcome::from_result(verify_json_schema(workspace, path, required_keys).await),
        VerifierCheck::Screenshot { path, min_bytes } => {
            CheckOutcome::from_result(verify_screenshot(workspace, path, *min_bytes).await)
        }
    };
    VerificationCheckResult {
        check_index,
        kind: check.kind().to_owned(),
        passed: outcome.passed,
        summary: outcome.summary,
        payload: outcome.payload,
    }
}

struct CheckOutcome {
    passed: bool,
    summary: String,
    payload: Value,
}

impl CheckOutcome {
    fn from_result(result: Result<(String, Value)>) -> Self {
        match result {
            Ok((summary, payload)) => Self {
                passed: true,
                summary,
                payload,
            },
            Err(error) => Self {
                passed: false,
                summary: error.to_string(),
                payload: json!({"error": format!("{error:#}")}),
            },
        }
    }
}

async fn verify_artifact(artifacts: &[AdapterArtifact], min_bytes: u64) -> Result<(String, Value)> {
    let mut failures = Vec::new();
    for artifact in artifacts {
        let bytes = match tokio::fs::read(&artifact.path).await {
            Ok(bytes) => bytes,
            Err(error) => {
                failures.push(format!("{}: {error}", artifact.path.display()));
                continue;
            }
        };
        let sha256 = hex::encode(Sha256::digest(&bytes));
        if bytes.len() as u64 >= min_bytes && sha256 == artifact.sha256 {
            return Ok((
                format!(
                    "artifact {} exists with {} verified bytes",
                    artifact.path.display(),
                    bytes.len()
                ),
                json!({
                    "path": artifact.path,
                    "bytes": bytes.len(),
                    "sha256": sha256,
                    "media_type": artifact.media_type
                }),
            ));
        }
        failures.push(format!(
            "{}: bytes={}, sha_match={}",
            artifact.path.display(),
            bytes.len(),
            sha256 == artifact.sha256
        ));
    }
    Err(anyhow!(
        "no artifact satisfied min_bytes={min_bytes}; {}",
        failures.join("; ")
    ))
}

async fn verify_file(workspace: &Path, relative: &str, min_bytes: u64) -> Result<(String, Value)> {
    let path = safe_workspace_file(workspace, relative).await?;
    let metadata = tokio::fs::metadata(&path).await?;
    if !metadata.is_file() {
        return Err(anyhow!("{relative} is not a regular file"));
    }
    if metadata.len() < min_bytes {
        return Err(anyhow!(
            "{relative} has {} bytes, below required {min_bytes}",
            metadata.len()
        ));
    }
    let bytes = tokio::fs::read(&path).await?;
    let sha256 = hex::encode(Sha256::digest(&bytes));
    Ok((
        format!("{relative} exists with {} bytes", bytes.len()),
        json!({"path": relative, "bytes": bytes.len(), "sha256": sha256}),
    ))
}

async fn verify_command(
    workspace: &Path,
    program: &str,
    args: &[String],
    timeout_ms: u64,
) -> CheckOutcome {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let attempted_mode = attempted_resolution_mode(program);
    let resolved =
        match tokio::time::timeout_at(deadline, resolve_program(program, workspace)).await {
            Ok(Ok(resolved)) => resolved,
            Ok(Err(error)) => {
                return command_failure(program, args, attempted_mode, None, error);
            }
            Err(_) => {
                return command_failure(
                    program,
                    args,
                    attempted_mode,
                    None,
                    anyhow!("{program:?} timed out during executable resolution"),
                );
            }
        };
    let identity = executable_identity(&resolved.executable);
    let mut command = Command::new(&resolved.executable);
    command
        .args(args)
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = match tokio::time::timeout_at(deadline, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            return command_failure(
                program,
                args,
                resolved.mode,
                Some(identity),
                anyhow!(error).context(format!(
                    "spawn resolved verifier command {program:?} using {}",
                    resolved.mode.as_str()
                )),
            );
        }
        Err(_) => {
            return command_failure(
                program,
                args,
                resolved.mode,
                Some(identity),
                anyhow!("{program:?} timed out after {timeout_ms} ms"),
            );
        }
    };
    let stdout = truncate(&output.stdout);
    let stderr = truncate(&output.stderr);
    if !output.status.success() {
        let error = anyhow!(
            "{program:?} exited with {}; resolution_mode={}; resolved_executable={}; stderr={stderr}",
            output.status,
            resolved.mode.as_str(),
            identity["file_name"].as_str().unwrap_or("unknown"),
        );
        return CheckOutcome {
            passed: false,
            summary: error.to_string(),
            payload: command_payload(
                program,
                args,
                resolved.mode,
                Some(identity),
                output.status.code(),
                Some(stdout),
                Some(stderr),
                Some(format!("{error:#}")),
            ),
        };
    }
    CheckOutcome {
        passed: true,
        summary: format!("{program:?} exited successfully"),
        payload: command_payload(
            program,
            args,
            resolved.mode,
            Some(identity),
            output.status.code(),
            Some(stdout),
            Some(stderr),
            None,
        ),
    }
}

fn command_failure(
    program: &str,
    args: &[String],
    mode: ResolutionMode,
    identity: Option<Value>,
    error: anyhow::Error,
) -> CheckOutcome {
    CheckOutcome {
        passed: false,
        summary: error.to_string(),
        payload: command_payload(
            program,
            args,
            mode,
            identity,
            None,
            None,
            None,
            Some(format!("{error:#}")),
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn command_payload(
    program: &str,
    args: &[String],
    mode: ResolutionMode,
    identity: Option<Value>,
    exit_code: Option<i32>,
    stdout: Option<String>,
    stderr: Option<String>,
    error: Option<String>,
) -> Value {
    json!({
        "program": bounded_text(program, 256),
        "requested_program": bounded_text(program, 256),
        "resolution_mode": mode.as_str(),
        "resolved_executable": identity,
        "args": args,
        "exit_code": exit_code,
        "stdout": stdout,
        "stderr": stderr,
        "error": error,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolutionMode {
    ExplicitPath,
    Path,
    PathPathext,
}

impl ResolutionMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitPath => "explicit_path",
            Self::Path => "path",
            Self::PathPathext => "path_pathext",
        }
    }
}

#[derive(Debug)]
struct ResolvedProgram {
    executable: PathBuf,
    mode: ResolutionMode,
}

#[derive(Debug)]
struct ResolutionEnvironment {
    path: Option<OsString>,
    pathext: Option<OsString>,
}

impl ResolutionEnvironment {
    fn current() -> Self {
        Self {
            path: env::var_os("PATH"),
            pathext: env::var_os("PATHEXT"),
        }
    }
}

async fn resolve_program(program: &str, workspace: &Path) -> Result<ResolvedProgram> {
    resolve_program_with_environment(program, workspace, &ResolutionEnvironment::current()).await
}

async fn resolve_program_with_environment(
    program: &str,
    workspace: &Path,
    environment: &ResolutionEnvironment,
) -> Result<ResolvedProgram> {
    if program_is_explicit_path(program) {
        let requested = Path::new(program);
        let (candidate, workspace_root) = if requested.is_absolute() {
            (requested.to_owned(), None)
        } else {
            if requested
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
            {
                return Err(anyhow!(
                    "workspace-relative verifier program {program:?} contains traversal, rooted, drive-relative, or ambiguous components"
                ));
            }
            let workspace_root = tokio::fs::canonicalize(workspace)
                .await
                .context("canonicalize assigned verifier worktree")?;
            (workspace_root.join(requested), Some(workspace_root))
        };
        let executable = match tokio::fs::canonicalize(&candidate).await {
            Ok(executable) => executable,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(anyhow!(
                    "explicit verifier program {program:?} does not exist"
                ));
            }
            Err(error) => {
                return Err(anyhow!(
                    "explicit verifier program {program:?} cannot be canonicalized: {:?}",
                    error.kind()
                ));
            }
        };
        if workspace_root
            .as_ref()
            .is_some_and(|root| !executable.starts_with(root))
        {
            return Err(anyhow!(
                "workspace-relative verifier program {program:?} escapes the assigned worktree"
            ));
        }
        let metadata = tokio::fs::metadata(&executable).await?;
        if !metadata.is_file() {
            return Err(anyhow!(
                "explicit verifier program {program:?} resolves to a directory or non-regular target"
            ));
        }
        return Ok(ResolvedProgram {
            executable,
            mode: ResolutionMode::ExplicitPath,
        });
    }

    if let Some(path) = environment.path.as_deref() {
        for directory in env::split_paths(path).filter(|directory| directory.is_absolute()) {
            #[cfg(not(windows))]
            if let Some(executable) = canonical_regular_file(&directory.join(program))
                .await
                .with_context(|| {
                    format!(
                        "failed to inspect a candidate for verifier program {program:?} in PATH"
                    )
                })?
            {
                return Ok(ResolvedProgram {
                    executable,
                    mode: ResolutionMode::Path,
                });
            }

            #[cfg(windows)]
            if Path::new(program).extension().is_some() {
                if let Some(executable) = canonical_regular_file(&directory.join(program))
                    .await
                    .with_context(|| {
                        format!(
                            "failed to inspect a candidate for verifier program {program:?} in PATH"
                        )
                    })?
                {
                    return Ok(ResolvedProgram {
                        executable,
                        mode: ResolutionMode::Path,
                    });
                }
            } else {
                for extension in pathext_extensions(environment.pathext.as_deref()) {
                    let mut candidate_name = OsString::from(program);
                    candidate_name.push(extension);
                    if let Some(executable) = canonical_regular_file(
                        &directory.join(candidate_name),
                    )
                    .await
                    .with_context(|| {
                        format!(
                            "failed to inspect a PATHEXT candidate for verifier program {program:?}"
                        )
                    })? {
                        return Ok(ResolvedProgram {
                            executable,
                            mode: ResolutionMode::PathPathext,
                        });
                    }
                }
            }
        }
    }

    Err(anyhow!(
        "verifier program {program:?} was not found as a regular file through the runner's absolute PATH entries{}",
        if cfg!(windows) { " and PATHEXT" } else { "" }
    ))
}

async fn canonical_regular_file(candidate: &Path) -> Result<Option<PathBuf>> {
    let canonical = match tokio::fs::canonicalize(candidate).await {
        Ok(canonical) => canonical,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = tokio::fs::metadata(&canonical).await?;
    if !metadata.is_file() {
        return Ok(None);
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o111 == 0 {
        return Ok(None);
    }
    Ok(Some(canonical))
}

fn program_is_explicit_path(program: &str) -> bool {
    let mut components = Path::new(program).components();
    !matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    )
}

fn attempted_resolution_mode(program: &str) -> ResolutionMode {
    if program_is_explicit_path(program) {
        ResolutionMode::ExplicitPath
    } else if cfg!(windows) && Path::new(program).extension().is_none() {
        ResolutionMode::PathPathext
    } else {
        ResolutionMode::Path
    }
}

#[cfg(windows)]
fn pathext_extensions(pathext: Option<&OsStr>) -> Vec<&str> {
    pathext
        .and_then(OsStr::to_str)
        .into_iter()
        .flat_map(|value| value.split(';'))
        .filter(|extension| {
            let bytes = extension.as_bytes();
            (2..=16).contains(&bytes.len())
                && bytes[0] == b'.'
                && bytes[1..].iter().all(u8::is_ascii_alphanumeric)
        })
        .collect()
}

fn executable_identity(executable: &Path) -> Value {
    let file_name = executable
        .file_name()
        .and_then(OsStr::to_str)
        .map(|name| bounded_text(name, 128))
        .unwrap_or_else(|| "<non-unicode>".to_owned());
    let canonical_path_sha256 = hex::encode(Sha256::digest(
        executable.as_os_str().to_string_lossy().as_bytes(),
    ));
    json!({
        "file_name": file_name,
        "canonical_path_sha256": canonical_path_sha256,
    })
}

fn bounded_text(value: &str, limit: usize) -> String {
    let characters = value.chars().collect::<Vec<_>>();
    if characters.len() <= limit {
        return value.to_owned();
    }
    if limit <= 3 {
        return ".".repeat(limit);
    }
    characters[..limit - 3]
        .iter()
        .chain(['.', '.', '.'].iter())
        .collect()
}

async fn verify_json_schema(
    workspace: &Path,
    relative: &str,
    required_keys: &[String],
) -> Result<(String, Value)> {
    let path = safe_workspace_file(workspace, relative).await?;
    let bytes = tokio::fs::read(&path).await?;
    let value: Value =
        serde_json::from_slice(&bytes).with_context(|| format!("parse {relative} as JSON"))?;
    let object = value
        .as_object()
        .with_context(|| format!("{relative} must contain a JSON object"))?;
    let missing = required_keys
        .iter()
        .filter(|key| !object.contains_key(key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(anyhow!(
            "{relative} is missing required keys: {}",
            missing.join(", ")
        ));
    }
    Ok((
        format!("{relative} satisfies {} required keys", required_keys.len()),
        json!({"path": relative, "required_keys": required_keys}),
    ))
}

async fn verify_screenshot(
    workspace: &Path,
    relative: &str,
    min_bytes: u64,
) -> Result<(String, Value)> {
    let path = safe_workspace_file(workspace, relative).await?;
    let bytes = tokio::fs::read(&path).await?;
    if (bytes.len() as u64) < min_bytes {
        return Err(anyhow!(
            "{relative} has {} bytes, below required {min_bytes}",
            bytes.len()
        ));
    }
    let format = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "jpeg"
    } else {
        return Err(anyhow!("{relative} is not a recognized PNG or JPEG"));
    };
    Ok((
        format!("{relative} is a valid {format} screenshot"),
        json!({"path": relative, "bytes": bytes.len(), "format": format}),
    ))
}

async fn safe_workspace_file(workspace: &Path, relative: &str) -> Result<PathBuf> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(anyhow!("verifier path escapes the assigned worktree"));
    }
    let candidate = workspace.join(relative_path);
    let metadata = tokio::fs::symlink_metadata(&candidate)
        .await
        .with_context(|| format!("{relative} does not exist"))?;
    if metadata.file_type().is_symlink() {
        return Err(anyhow!("{relative} is a symlink"));
    }
    let root = normalize_path(tokio::fs::canonicalize(workspace).await?);
    let canonical = normalize_path(tokio::fs::canonicalize(&candidate).await?);
    if !canonical.starts_with(&root) || canonical == root {
        return Err(anyhow!("verifier path escapes the assigned worktree"));
    }
    Ok(canonical)
}

fn truncate(bytes: &[u8]) -> String {
    const LIMIT: usize = 16 * 1024;
    let bytes = if bytes.len() > LIMIT {
        &bytes[..LIMIT]
    } else {
        bytes
    };
    String::from_utf8_lossy(bytes).to_string()
}

#[cfg(windows)]
fn normalize_path(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path
}

#[cfg(not(windows))]
fn normalize_path(path: PathBuf) -> PathBuf {
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_environment() -> ResolutionEnvironment {
        ResolutionEnvironment {
            path: None,
            pathext: None,
        }
    }

    fn artifact(path: PathBuf, bytes: &[u8]) -> AdapterArtifact {
        AdapterArtifact {
            path,
            sha256: hex::encode(Sha256::digest(bytes)),
            bytes: bytes.len(),
            media_type: "text/plain".to_owned(),
        }
    }

    #[tokio::test]
    async fn matrix_checks_pass_and_missing_or_escaping_files_fail() {
        let workspace = std::env::temp_dir()
            .join("crony verifier tests")
            .join(uuid::Uuid::new_v4().to_string());
        tokio::fs::create_dir_all(&workspace)
            .await
            .expect("create workspace");
        tokio::fs::write(workspace.join("artifact.txt"), b"artifact\n")
            .await
            .expect("write artifact");
        tokio::fs::write(workspace.join("verify.txt"), b"VERIFIED\n")
            .await
            .expect("write verify");
        tokio::fs::write(
            workspace.join("schema.json"),
            br#"{"status":"ok","count":1}"#,
        )
        .await
        .expect("write json");
        tokio::fs::write(
            workspace.join("screenshot.png"),
            b"\x89PNG\r\n\x1a\n0123456789",
        )
        .await
        .expect("write screenshot");
        let artifacts = vec![artifact(workspace.join("artifact.txt"), b"artifact\n")];
        let report = verify(
            &VerificationPolicy {
                checks: vec![
                    VerifierCheck::Artifact { min_bytes: 1 },
                    VerifierCheck::File {
                        path: "verify.txt".to_owned(),
                        min_bytes: 9,
                    },
                    VerifierCheck::Command {
                        program: "node".to_owned(),
                        args: vec!["-e".to_owned(), "process.exit(0)".to_owned()],
                        timeout_ms: 30_000,
                    },
                    VerifierCheck::Test {
                        program: "node".to_owned(),
                        args: vec!["-e".to_owned(), "process.exit(0)".to_owned()],
                        timeout_ms: 30_000,
                    },
                    VerifierCheck::JsonSchema {
                        path: "schema.json".to_owned(),
                        required_keys: vec!["status".to_owned(), "count".to_owned()],
                    },
                    VerifierCheck::Screenshot {
                        path: "screenshot.png".to_owned(),
                        min_bytes: 16,
                    },
                ],
                manual_gate: None,
            },
            &workspace,
            &artifacts,
        )
        .await;
        assert!(report.passed);
        assert_eq!(report.checks.len(), 6);
        assert_eq!(
            report.checks[2].payload["requested_program"],
            Value::String("node".to_owned())
        );
        assert!(report.checks[2].payload["resolution_mode"].is_string());
        assert!(
            report.checks[2].payload["resolved_executable"]["canonical_path_sha256"]
                .as_str()
                .is_some_and(|digest| digest.len() == 64)
        );

        let failed = verify(
            &VerificationPolicy {
                checks: vec![
                    VerifierCheck::File {
                        path: "missing.txt".to_owned(),
                        min_bytes: 1,
                    },
                    VerifierCheck::File {
                        path: "../escape.txt".to_owned(),
                        min_bytes: 1,
                    },
                ],
                manual_gate: None,
            },
            &workspace,
            &artifacts,
        )
        .await;
        assert!(!failed.passed);
        assert_eq!(
            failed.checks.iter().filter(|check| !check.passed).count(),
            2
        );
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn missing_programs_fail_closed_without_searching_the_workspace() {
        let workspace = std::env::temp_dir()
            .join("crony verifier missing program")
            .join(uuid::Uuid::new_v4().to_string());
        tokio::fs::create_dir_all(&workspace)
            .await
            .expect("create workspace");
        let program = format!("missing-{}", uuid::Uuid::new_v4());
        tokio::fs::write(workspace.join(&program), b"not executable")
            .await
            .expect("write workspace file");

        let error = resolve_program_with_environment(&program, &workspace, &empty_environment())
            .await
            .expect_err("bare program must not resolve from the workspace");
        let diagnostic = error.to_string();
        assert!(diagnostic.contains(&format!("verifier program {program:?} was not found")));
        assert!(!diagnostic.contains(&workspace.to_string_lossy().to_string()));

        let relative_path_environment = ResolutionEnvironment {
            path: Some(OsString::from(".")),
            pathext: Some(OsString::from(".TOML")),
        };
        let error =
            resolve_program_with_environment("Cargo.toml", &workspace, &relative_path_environment)
                .await
                .expect_err("relative PATH entry must not search the current directory");
        assert!(
            error
                .to_string()
                .contains("was not found as a regular file")
        );

        let _ = tokio::fs::remove_dir_all(workspace).await;
    }

    #[tokio::test]
    async fn explicit_paths_are_canonicalized_and_directory_targets_are_rejected() {
        let workspace = std::env::temp_dir()
            .join("crony verifier explicit path")
            .join(uuid::Uuid::new_v4().to_string());
        tokio::fs::create_dir_all(&workspace)
            .await
            .expect("create workspace");
        let current_exe = std::env::current_exe().expect("current test executable");
        let resolved = resolve_program_with_environment(
            current_exe.to_str().expect("Unicode test executable path"),
            &workspace,
            &empty_environment(),
        )
        .await
        .expect("resolve explicit executable");
        assert_eq!(resolved.mode, ResolutionMode::ExplicitPath);
        assert_eq!(
            resolved.executable,
            tokio::fs::canonicalize(current_exe)
                .await
                .expect("canonical test executable")
        );

        let error = resolve_program_with_environment(
            workspace.to_str().expect("Unicode workspace path"),
            &workspace,
            &empty_environment(),
        )
        .await
        .expect_err("directory target must be rejected");
        assert!(
            error
                .to_string()
                .contains("resolves to a directory or non-regular target")
        );

        let traversal = Path::new("..").join("outside.exe");
        let error = resolve_program_with_environment(
            traversal.to_str().expect("Unicode traversal path"),
            &workspace,
            &empty_environment(),
        )
        .await
        .expect_err("workspace-relative traversal must be rejected");
        assert!(
            error
                .to_string()
                .contains("contains traversal, rooted, drive-relative, or ambiguous components")
        );

        let _ = tokio::fs::remove_dir_all(workspace).await;
    }

    #[tokio::test]
    async fn command_failure_records_bounded_resolution_evidence() {
        let workspace = std::env::temp_dir()
            .join("crony verifier failure evidence")
            .join(uuid::Uuid::new_v4().to_string());
        tokio::fs::create_dir_all(&workspace)
            .await
            .expect("create workspace");
        let requested = format!("missing-{}", uuid::Uuid::new_v4());
        let outcome = verify_command(&workspace, &requested, &[], 30_000).await;

        assert!(!outcome.passed);
        assert!(outcome.summary.contains("was not found as a regular file"));
        assert_eq!(outcome.payload["requested_program"], requested);
        assert_eq!(
            outcome.payload["resolution_mode"],
            attempted_resolution_mode(&requested).as_str()
        );
        assert!(outcome.payload["resolved_executable"].is_null());
        assert!(!outcome.payload.to_string().contains("PATH="));

        let long_name = format!("{}.exe", "x".repeat(180));
        let identity = executable_identity(Path::new(&long_name));
        let bounded_name = identity["file_name"]
            .as_str()
            .expect("bounded executable name");
        assert_eq!(bounded_name.chars().count(), 128);
        assert!(bounded_name.ends_with("..."));

        let _ = tokio::fs::remove_dir_all(workspace).await;
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn direct_executable_path_resolution_is_unchanged_on_unix() {
        let workspace = std::env::temp_dir()
            .join("crony verifier direct executable")
            .join(uuid::Uuid::new_v4().to_string());
        tokio::fs::create_dir_all(&workspace)
            .await
            .expect("create workspace");
        let current_exe = std::env::current_exe().expect("current test executable");
        let file_name = current_exe
            .file_name()
            .and_then(OsStr::to_str)
            .expect("Unicode test executable name");
        let environment = ResolutionEnvironment {
            path: current_exe
                .parent()
                .map(|parent| parent.as_os_str().to_owned()),
            pathext: None,
        };
        let resolved = resolve_program_with_environment(file_name, &workspace, &environment)
            .await
            .expect("resolve direct executable through PATH");
        assert_eq!(resolved.mode, ResolutionMode::Path);
        assert_eq!(
            resolved.executable,
            tokio::fs::canonicalize(current_exe)
                .await
                .expect("canonical test executable")
        );

        let shadow = workspace.join(file_name);
        tokio::fs::write(&shadow, b"not executable")
            .await
            .expect("write non-executable PATH shadow");
        let mut permissions = tokio::fs::metadata(&shadow)
            .await
            .expect("read shadow metadata")
            .permissions();
        permissions.set_mode(0o644);
        tokio::fs::set_permissions(&shadow, permissions)
            .await
            .expect("set non-executable shadow permissions");
        assert!(
            canonical_regular_file(&shadow)
                .await
                .expect("inspect non-executable shadow")
                .is_none()
        );

        let _ = tokio::fs::remove_dir_all(workspace).await;
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_pathext_resolves_package_manager_and_direct_executable_names() {
        let workspace = std::env::temp_dir()
            .join("crony verifier PATHEXT")
            .join(uuid::Uuid::new_v4().to_string());
        let bin = workspace.join("bin");
        tokio::fs::create_dir_all(&bin)
            .await
            .expect("create fake toolchain");
        for name in ["npm.cmd", "pnpm.CMD", "npx.bat", "runner-tool.EXE"] {
            tokio::fs::write(bin.join(name), b"fixture")
                .await
                .expect("write fake executable");
        }
        let environment = ResolutionEnvironment {
            path: Some(bin.as_os_str().to_owned()),
            pathext: Some(OsString::from(".COM;.EXE;.BAT;.CMD")),
        };

        for (requested, expected_name) in [
            ("npm", "npm.cmd"),
            ("pnpm", "pnpm.CMD"),
            ("npx", "npx.bat"),
            ("runner-tool", "runner-tool.EXE"),
        ] {
            let resolved = resolve_program_with_environment(requested, &workspace, &environment)
                .await
                .expect("resolve PATHEXT-backed program");
            assert_eq!(resolved.mode, ResolutionMode::PathPathext);
            assert_eq!(
                resolved.executable.file_name().and_then(OsStr::to_str),
                Some(expected_name)
            );
        }

        let explicit_extension =
            resolve_program_with_environment("npm.cmd", &workspace, &environment)
                .await
                .expect("resolve explicit npm.cmd extension");
        assert_eq!(explicit_extension.mode, ResolutionMode::Path);
        assert_eq!(
            explicit_extension
                .executable
                .file_name()
                .and_then(OsStr::to_str),
            Some("npm.cmd")
        );
        let explicit_relative =
            resolve_program_with_environment(r"bin\npm.cmd", &workspace, &empty_environment())
                .await
                .expect("resolve explicit workspace-relative npm.cmd path");
        assert_eq!(explicit_relative.mode, ResolutionMode::ExplicitPath);
        assert_eq!(explicit_relative.executable, explicit_extension.executable);

        let ambiguous =
            resolve_program_with_environment(r"C:npm.cmd", &workspace, &empty_environment())
                .await
                .expect_err("drive-relative path must be rejected");
        assert!(ambiguous.to_string().contains("drive-relative"));

        let identity = executable_identity(&explicit_extension.executable);
        assert!(
            !identity
                .to_string()
                .contains(&bin.to_string_lossy().to_string())
        );

        let _ = tokio::fs::remove_dir_all(workspace).await;
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_batch_launch_does_not_inject_malicious_arguments() {
        let workspace = std::env::temp_dir()
            .join("crony verifier batch arguments")
            .join(uuid::Uuid::new_v4().to_string());
        tokio::fs::create_dir_all(&workspace)
            .await
            .expect("create workspace");
        let shim = workspace.join("echo-arguments.cmd");
        tokio::fs::write(
            &shim,
            b"@echo off\r\nnode -e \"process.stdout.write(JSON.stringify(process.argv.slice(1)))\" %*\r\n",
        )
        .await
        .expect("write batch shim");
        let marker = workspace.join("argument-injection-marker.txt");
        let malicious = format!("safe & echo injected>\"{}\"", marker.display());
        let outcome = verify_command(
            &workspace,
            shim.to_str().expect("Unicode shim path"),
            std::slice::from_ref(&malicious),
            10_000,
        )
        .await;

        assert!(outcome.passed, "{}", outcome.summary);
        assert!(outcome.summary.contains("exited successfully"));
        assert!(outcome.payload["stdout"].as_str().is_some_and(|stdout| {
            serde_json::from_str::<Vec<String>>(stdout)
                .is_ok_and(|arguments| arguments == [malicious])
        }));
        assert!(!marker.exists(), "malicious argument created a marker file");

        let _ = tokio::fs::remove_dir_all(workspace).await;
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_installed_npm_shim_runs_incident_command_policy() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root");
        let args = [
            "--prefix".to_owned(),
            "scenarios/incident-command".to_owned(),
            "test".to_owned(),
        ];
        let outcome = verify_command(repository, "npm", &args, 60_000).await;

        assert!(outcome.passed, "{}", outcome.summary);
        assert!(outcome.summary.contains("exited successfully"));
        assert_eq!(outcome.payload["requested_program"], "npm");
        assert_eq!(outcome.payload["resolution_mode"], "path_pathext");
        assert_eq!(
            outcome.payload["resolved_executable"]["file_name"],
            "npm.cmd"
        );
    }
}
