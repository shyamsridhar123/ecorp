use std::{
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use crony_domain::{ManualVerificationGate, VerificationPolicy, VerifierCheck};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::process::Command;

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
    let result = match check {
        VerifierCheck::Artifact { min_bytes } => verify_artifact(artifacts, *min_bytes).await,
        VerifierCheck::File { path, min_bytes } => verify_file(workspace, path, *min_bytes).await,
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
        } => verify_json_schema(workspace, path, required_keys).await,
        VerifierCheck::Screenshot { path, min_bytes } => {
            verify_screenshot(workspace, path, *min_bytes).await
        }
    };
    match result {
        Ok((summary, payload)) => VerificationCheckResult {
            check_index,
            kind: check.kind().to_owned(),
            passed: true,
            summary,
            payload,
        },
        Err(error) => VerificationCheckResult {
            check_index,
            kind: check.kind().to_owned(),
            passed: false,
            summary: error.to_string(),
            payload: json!({"error": format!("{error:#}")}),
        },
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
) -> Result<(String, Value)> {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_millis(timeout_ms), command.output())
        .await
        .with_context(|| format!("{program} timed out after {timeout_ms} ms"))?
        .with_context(|| format!("spawn verifier command {program}"))?;
    let stdout = truncate(&output.stdout);
    let stderr = truncate(&output.stderr);
    if !output.status.success() {
        return Err(anyhow!(
            "{program} exited with {}; stderr={stderr}",
            output.status
        ));
    }
    Ok((
        format!("{program} exited successfully"),
        json!({
            "program": program,
            "args": args,
            "exit_code": output.status.code(),
            "stdout": stdout,
            "stderr": stderr
        }),
    ))
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
                        timeout_ms: 5_000,
                    },
                    VerifierCheck::Test {
                        program: "node".to_owned(),
                        args: vec!["-e".to_owned(), "process.exit(0)".to_owned()],
                        timeout_ms: 5_000,
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
}
