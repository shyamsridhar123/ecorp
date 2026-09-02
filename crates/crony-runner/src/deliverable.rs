use std::{
    ffi::OsString,
    path::{Component, Path},
    process::{Output, Stdio},
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use crony_domain::{
    DeliverableForm, DeliverableSpec, repository_relative_path_is_valid, write_scope_allows_path,
    write_scope_is_valid,
};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::process::Command;
use uuid::Uuid;

use crate::{adapter::AdapterArtifact, verifier::VerificationReport, workspace::WorkspaceLease};

const GIT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_DELIVERABLE_BYTES: usize = 16_777_216;

#[derive(Debug)]
pub struct ExportedDeliverable {
    pub bytes: Vec<u8>,
    pub file_name: String,
    pub media_type: String,
    pub form: DeliverableForm,
    pub verification_sha256: String,
    pub base_commit: String,
    pub head_commit: Option<String>,
    pub branch: String,
    pub git_bundle_sha256: Option<String>,
    pub publication_ready: bool,
}

#[derive(Debug, Serialize)]
struct ArchivedChange {
    path: String,
    status: String,
    mode: Option<String>,
    sha256: Option<String>,
    bytes: Option<usize>,
    media_type: Option<String>,
    content_base64: Option<String>,
}

struct TemporaryExportPaths<'a> {
    index: &'a Path,
    bundle: &'a Path,
}

pub async fn export(
    run_id: Uuid,
    spec: &DeliverableSpec,
    workspace: &WorkspaceLease,
    report: &VerificationReport,
    provider_artifacts: &[AdapterArtifact],
    write_scope: &[String],
) -> Result<ExportedDeliverable> {
    let workspace_root = tokio::fs::canonicalize(&workspace.path)
        .await
        .context("resolve deliverable worktree")?;
    let temporary_index = workspace
        .path
        .parent()
        .context("deliverable worktree has no managed parent")?
        .join(format!(".ecorp-deliverable-{}.index", run_id.simple()));
    let temporary_bundle = workspace
        .path
        .parent()
        .context("deliverable worktree has no managed parent")?
        .join(format!(".ecorp-deliverable-{}.bundle", run_id.simple()));
    let _ = tokio::fs::remove_file(&temporary_index).await;
    let _ = tokio::fs::remove_file(&temporary_bundle).await;

    let temporary_paths = TemporaryExportPaths {
        index: &temporary_index,
        bundle: &temporary_bundle,
    };
    let result = export_with_index(
        spec,
        workspace,
        &workspace_root,
        report,
        provider_artifacts,
        write_scope,
        &temporary_paths,
    )
    .await;
    let _ = tokio::fs::remove_file(&temporary_index).await;
    let _ = tokio::fs::remove_file(&temporary_bundle).await;
    result
}

async fn export_with_index(
    spec: &DeliverableSpec,
    workspace: &WorkspaceLease,
    workspace_root: &Path,
    report: &VerificationReport,
    provider_artifacts: &[AdapterArtifact],
    write_scope: &[String],
    temporary_paths: &TemporaryExportPaths<'_>,
) -> Result<ExportedDeliverable> {
    git_success(
        workspace_root,
        temporary_paths.index,
        &[
            OsString::from("read-tree"),
            OsString::from(&workspace.base_commit),
        ],
    )
    .await?;

    let mut add_args = vec![
        OsString::from("add"),
        OsString::from("-A"),
        OsString::from("--"),
    ];
    if spec.paths.is_empty() {
        add_args.push(OsString::from("."));
    } else {
        for path in &spec.paths {
            validate_relative(path)?;
            add_args.push(OsString::from(path));
        }
    }
    git_success(workspace_root, temporary_paths.index, &add_args).await?;

    for artifact in provider_artifacts {
        let Ok(artifact_path) = tokio::fs::canonicalize(&artifact.path).await else {
            continue;
        };
        let Ok(path) = artifact_path.strip_prefix(workspace_root) else {
            continue;
        };
        let relative = portable_path(path)?;
        git_success(
            workspace_root,
            temporary_paths.index,
            &[
                OsString::from("reset"),
                OsString::from("-q"),
                OsString::from(&workspace.base_commit),
                OsString::from("--"),
                OsString::from(relative),
            ],
        )
        .await?;
    }

    let changes = changed_paths(
        workspace_root,
        temporary_paths.index,
        &workspace.base_commit,
    )
    .await?;
    reject_out_of_scope_changes(&changes, write_scope)?;
    reject_unsafe_changes(workspace_root, temporary_paths.index, &changes).await?;
    let verification_bytes =
        serde_json::to_vec(report).context("serialize verification report for linkage")?;
    let verification_sha256 = hex::encode(Sha256::digest(&verification_bytes));

    let should_commit =
        spec.commit_after_verification || spec.form == DeliverableForm::CommitBranch;
    let head_commit = if should_commit {
        commit_index(
            workspace_root,
            temporary_paths.index,
            workspace,
            &verification_sha256,
            &changes,
        )
        .await?
    } else {
        None
    };

    let patch = git_output(
        workspace_root,
        temporary_paths.index,
        &[
            OsString::from("diff"),
            OsString::from("--cached"),
            OsString::from("--binary"),
            OsString::from("--full-index"),
            OsString::from("--no-color"),
            OsString::from(&workspace.base_commit),
            OsString::from("--"),
        ],
    )
    .await?
    .stdout;
    let git_bundle = if spec.form == DeliverableForm::CommitBranch {
        let head_commit = head_commit
            .as_deref()
            .context("commit/branch deliverable omitted its committed head")?;
        Some(
            create_git_bundle(
                workspace_root,
                temporary_paths.index,
                temporary_paths.bundle,
                &workspace.branch,
                &workspace.base_commit,
                head_commit,
            )
            .await?,
        )
    } else {
        None
    };
    let git_bundle_sha256 = git_bundle
        .as_ref()
        .map(|bundle| hex::encode(Sha256::digest(bundle)));

    let (bytes, file_name, media_type) = match spec.form {
        DeliverableForm::Patch => (
            patch,
            "ecorp-deliverable.patch".to_owned(),
            "text/x-diff".to_owned(),
        ),
        form => {
            let include_content = matches!(
                form,
                DeliverableForm::Archive
                    | DeliverableForm::TypedArtifactSet
                    | DeliverableForm::CommitBranch
            );
            let archived = archive_changes(
                workspace_root,
                temporary_paths.index,
                &changes,
                include_content,
            )
            .await?;
            let document = json!({
                "schema_version": 1,
                "form": form.as_str(),
                "base_commit": workspace.base_commit,
                "head_commit": head_commit,
                "branch": workspace.branch,
                "verification_sha256": verification_sha256,
                "patch_sha256": hex::encode(Sha256::digest(&patch)),
                "patch_base64": include_content.then(|| BASE64.encode(&patch)),
                "git_bundle_sha256": git_bundle_sha256,
                "git_bundle_base64": git_bundle.as_ref().map(|bundle| BASE64.encode(bundle)),
                "changes": archived,
            });
            let name = match form {
                DeliverableForm::Archive => "ecorp-source-archive.json",
                DeliverableForm::TypedArtifactSet => "ecorp-artifact-set.json",
                DeliverableForm::CommitBranch => "ecorp-commit-branch.json",
                DeliverableForm::ReviewOnlyReport => "ecorp-review-report.json",
                DeliverableForm::Patch => unreachable!(),
            };
            (
                serde_json::to_vec_pretty(&document)
                    .context("serialize deterministic deliverable")?,
                name.to_owned(),
                "application/vnd.ecorp.deliverable+json".to_owned(),
            )
        }
    };
    if bytes.is_empty() {
        return Err(anyhow!("deliverable export produced no bytes"));
    }
    if bytes.len() > MAX_DELIVERABLE_BYTES {
        return Err(anyhow!(
            "deliverable size {} exceeds the runner limit {}",
            bytes.len(),
            MAX_DELIVERABLE_BYTES
        ));
    }

    Ok(ExportedDeliverable {
        bytes,
        file_name,
        media_type,
        form: spec.form,
        verification_sha256,
        base_commit: workspace.base_commit.clone(),
        head_commit,
        branch: workspace.branch.clone(),
        git_bundle_sha256,
        publication_ready: spec.form == DeliverableForm::CommitBranch,
    })
}

async fn create_git_bundle(
    workspace: &Path,
    index: &Path,
    bundle_path: &Path,
    branch: &str,
    base_commit: &str,
    head_commit: &str,
) -> Result<Vec<u8>> {
    let branch_ref = format!("refs/heads/{branch}");
    let resolved_head = git_text(
        workspace,
        index,
        &[
            OsString::from("rev-parse"),
            OsString::from(format!("{branch_ref}^{{commit}}")),
        ],
    )
    .await?;
    if resolved_head != head_commit {
        return Err(anyhow!(
            "commit/branch deliverable head no longer matches its isolated branch"
        ));
    }
    git_success(
        workspace,
        index,
        &[
            OsString::from("merge-base"),
            OsString::from("--is-ancestor"),
            OsString::from(base_commit),
            OsString::from(head_commit),
        ],
    )
    .await?;
    git_success(
        workspace,
        index,
        &[
            OsString::from("bundle"),
            OsString::from("create"),
            bundle_path.as_os_str().to_owned(),
            OsString::from("HEAD"),
            OsString::from(format!("^{base_commit}")),
        ],
    )
    .await?;
    let bundle = tokio::fs::read(bundle_path)
        .await
        .context("read portable Git bundle")?;
    if bundle.is_empty() {
        return Err(anyhow!("portable Git bundle is empty"));
    }
    Ok(bundle)
}

async fn changed_paths(
    workspace: &Path,
    index: &Path,
    base_commit: &str,
) -> Result<Vec<(String, String)>> {
    let output = git_output(
        workspace,
        index,
        &[
            OsString::from("diff"),
            OsString::from("--cached"),
            OsString::from("--name-status"),
            OsString::from("-z"),
            OsString::from("--no-renames"),
            OsString::from(base_commit),
            OsString::from("--"),
        ],
    )
    .await?
    .stdout;
    let fields = output
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    let mut fields =
        fields.map(|field| String::from_utf8(field.to_vec()).context("Git path is not UTF-8"));
    let mut changes = Vec::new();
    while let Some(status) = fields.next() {
        let status = status?;
        let path = fields
            .next()
            .context("Git name-status output omitted a path")??;
        validate_relative(&path)?;
        changes.push((status, path));
    }
    changes.sort_by(|left, right| left.1.cmp(&right.1).then(left.0.cmp(&right.0)));
    Ok(changes)
}

async fn reject_unsafe_changes(
    workspace: &Path,
    index: &Path,
    changes: &[(String, String)],
) -> Result<()> {
    for (status, path) in changes {
        if sensitive_path(path) {
            return Err(anyhow!(
                "deliverable contains a secret-like or runner-internal path: {path}"
            ));
        }
        if status.starts_with('D') {
            continue;
        }
        let mode = index_mode(workspace, index, path).await?;
        if mode == "120000" {
            return Err(anyhow!("deliverable cannot contain symbolic link {path}"));
        }
        if mode == "160000" {
            return Err(anyhow!("deliverable cannot contain Git link {path}"));
        }
        let candidate = workspace.join(path);
        let canonical = tokio::fs::canonicalize(&candidate)
            .await
            .with_context(|| format!("resolve deliverable path {path}"))?;
        if !canonical.starts_with(workspace) {
            return Err(anyhow!("deliverable path escapes the worktree: {path}"));
        }
    }
    Ok(())
}

async fn archive_changes(
    workspace: &Path,
    index: &Path,
    changes: &[(String, String)],
    include_content: bool,
) -> Result<Vec<ArchivedChange>> {
    let mut archived = Vec::with_capacity(changes.len());
    for (status, path) in changes {
        if status.starts_with('D') {
            archived.push(ArchivedChange {
                path: path.clone(),
                status: status.clone(),
                mode: None,
                sha256: None,
                bytes: None,
                media_type: None,
                content_base64: None,
            });
            continue;
        }
        let content = git_output(
            workspace,
            index,
            &[OsString::from("show"), OsString::from(format!(":{path}"))],
        )
        .await?
        .stdout;
        archived.push(ArchivedChange {
            path: path.clone(),
            status: status.clone(),
            mode: Some(index_mode(workspace, index, path).await?),
            sha256: Some(hex::encode(Sha256::digest(&content))),
            bytes: Some(content.len()),
            media_type: Some(infer_media_type(path).to_owned()),
            content_base64: include_content.then(|| BASE64.encode(content)),
        });
    }
    Ok(archived)
}

async fn index_mode(workspace: &Path, index: &Path, path: &str) -> Result<String> {
    let output = git_output(
        workspace,
        index,
        &[
            OsString::from("ls-files"),
            OsString::from("--stage"),
            OsString::from("--"),
            OsString::from(path),
        ],
    )
    .await?;
    let text = String::from_utf8(output.stdout).context("Git index mode is not UTF-8")?;
    text.split_whitespace()
        .next()
        .map(str::to_owned)
        .context("Git index omitted file mode")
}

async fn commit_index(
    workspace: &Path,
    index: &Path,
    lease: &WorkspaceLease,
    verification_sha256: &str,
    changes: &[(String, String)],
) -> Result<Option<String>> {
    let tree = git_text(workspace, index, &[OsString::from("write-tree")]).await?;
    let base_tree = git_text(
        workspace,
        index,
        &[
            OsString::from("rev-parse"),
            OsString::from(format!("{}^{{tree}}", lease.base_commit)),
        ],
    )
    .await?;
    let old_head = git_text(
        workspace,
        index,
        &[OsString::from("rev-parse"), OsString::from("HEAD^{commit}")],
    )
    .await?;
    let commit = if tree == base_tree {
        lease.base_commit.clone()
    } else {
        let message =
            format!("ECorp verified deliverable\n\nVerification-SHA256: {verification_sha256}\n");
        git_text_with_env(
            workspace,
            index,
            &[
                OsString::from("commit-tree"),
                OsString::from(&tree),
                OsString::from("-p"),
                OsString::from(&lease.base_commit),
                OsString::from("-m"),
                OsString::from(message),
            ],
            &[
                ("GIT_AUTHOR_NAME", "ECorp Runner"),
                ("GIT_AUTHOR_EMAIL", "runner@ecorp.invalid"),
                ("GIT_COMMITTER_NAME", "ECorp Runner"),
                ("GIT_COMMITTER_EMAIL", "runner@ecorp.invalid"),
            ],
        )
        .await?
    };
    git_success(
        workspace,
        index,
        &[
            OsString::from("update-ref"),
            OsString::from(format!("refs/heads/{}", lease.branch)),
            OsString::from(&commit),
            OsString::from(&old_head),
        ],
    )
    .await?;
    if !changes.is_empty() {
        let mut command = Command::new("git");
        command
            .args(["reset", "--mixed", "HEAD", "--"])
            .args(changes.iter().map(|(_, path)| path))
            .current_dir(workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let output = command
            .output()
            .await
            .context("refresh committed paths in the worktree index")?;
        if !output.status.success() {
            return Err(git_error(&output));
        }
    }
    Ok(Some(commit))
}

fn validate_relative(value: &str) -> Result<()> {
    if !repository_relative_path_is_valid(value) {
        return Err(anyhow!("deliverable path must stay inside the worktree"));
    }
    Ok(())
}

fn reject_out_of_scope_changes(changes: &[(String, String)], write_scope: &[String]) -> Result<()> {
    if write_scope.is_empty() {
        return Err(anyhow!(
            "deliverable export requires an explicit task write scope"
        ));
    }
    for scope in write_scope {
        if !write_scope_is_valid(scope) {
            return Err(anyhow!(
                "task write scope must be an exact relative path or end in /**"
            ));
        }
    }
    for (_, path) in changes {
        if !write_scope
            .iter()
            .any(|scope| write_scope_allows_path(scope, path))
        {
            return Err(anyhow!(
                "deliverable contains path outside the task write scope: {path}"
            ));
        }
    }
    Ok(())
}

fn portable_path(path: &Path) -> Result<String> {
    let value = path
        .components()
        .map(|component| match component {
            Component::Normal(value) => Ok(value.to_string_lossy().into_owned()),
            _ => Err(anyhow!(
                "deliverable path contains a non-portable component"
            )),
        })
        .collect::<Result<Vec<_>>>()?
        .join("/");
    validate_relative(&value)?;
    Ok(value)
}

fn sensitive_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    let file_name = normalized.rsplit('/').next().unwrap_or(&normalized);
    let components = normalized.split('/').collect::<Vec<_>>();
    components.iter().any(|component| {
        matches!(
            *component,
            ".git"
                | ".codex"
                | ".claude"
                | ".ssh"
                | ".aws"
                | ".azure"
                | ".kube"
                | ".docker"
                | ".gnupg"
                | ".password-store"
        )
    }) || components
        .windows(2)
        .any(|pair| pair == [".config", "gcloud"])
        || file_name.starts_with(".env")
        || matches!(
            file_name,
            ".npmrc"
                | ".pypirc"
                | "id_rsa"
                | "id_ed25519"
                | "credentials"
                | "credentials.json"
                | "secrets.json"
        )
        || file_name.ends_with(".pem")
        || file_name.ends_with(".key")
        || file_name.ends_with(".p12")
        || file_name.ends_with(".pfx")
}

fn infer_media_type(path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "json" => "application/json",
        "md" | "txt" | "rs" | "ts" | "tsx" | "js" | "mjs" | "css" | "html" | "toml" | "yaml"
        | "yml" => "text/plain",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

async fn git_success(workspace: &Path, index: &Path, args: &[OsString]) -> Result<()> {
    let output = git_output(workspace, index, args).await?;
    if output.status.success() {
        Ok(())
    } else {
        Err(git_error(&output))
    }
}

async fn git_text(workspace: &Path, index: &Path, args: &[OsString]) -> Result<String> {
    git_text_with_env(workspace, index, args, &[]).await
}

async fn git_text_with_env(
    workspace: &Path,
    index: &Path,
    args: &[OsString],
    env: &[(&str, &str)],
) -> Result<String> {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(workspace)
        .env("GIT_INDEX_FILE", index)
        .env("GIT_LITERAL_PATHSPECS", "1")
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(GIT_TIMEOUT, command.output())
        .await
        .context("Git deliverable command timed out")?
        .context("start Git deliverable command")?;
    if !output.status.success() {
        return Err(git_error(&output));
    }
    Ok(String::from_utf8(output.stdout)
        .context("Git deliverable output is not UTF-8")?
        .trim()
        .to_owned())
}

async fn git_output(workspace: &Path, index: &Path, args: &[OsString]) -> Result<Output> {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(workspace)
        .env("GIT_INDEX_FILE", index)
        .env("GIT_LITERAL_PATHSPECS", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(GIT_TIMEOUT, command.output())
        .await
        .context("Git deliverable command timed out")?
        .context("start Git deliverable command")?;
    if !output.status.success() {
        return Err(git_error(&output));
    }
    Ok(output)
}

fn git_error(output: &Output) -> anyhow::Error {
    let stderr = String::from_utf8_lossy(&output.stderr);
    anyhow!("Git deliverable command failed: {}", stderr.trim())
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf, process::Command};

    use crony_domain::{DeliverableForm, DeliverableSpec};
    use serde_json::Value;

    use super::*;
    use crate::verifier::{VerificationCheckResult, VerificationReport};

    fn git(repo: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {:?}: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    fn fixture() -> (PathBuf, WorkspaceLease, VerificationReport) {
        let root = std::env::temp_dir().join(format!("ecorp-deliverable-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create fixture");
        git(&root, &["init", "-b", "main"]);
        git(&root, &["config", "user.name", "Test"]);
        git(&root, &["config", "user.email", "test@example.invalid"]);
        fs::write(root.join("tracked.txt"), b"before\n").expect("write tracked");
        fs::write(root.join("other.txt"), b"other before\n").expect("write other");
        git(&root, &["add", "tracked.txt", "other.txt"]);
        git(&root, &["commit", "-m", "base"]);
        let base_commit = git(&root, &["rev-parse", "HEAD"]);
        let lease = WorkspaceLease {
            path: root.clone(),
            branch: "main".to_owned(),
            base_ref: "main".to_owned(),
            base_commit,
        };
        let report = VerificationReport {
            passed: true,
            summary: "all checks passed".to_owned(),
            checks: vec![VerificationCheckResult {
                check_index: 0,
                kind: "file".to_owned(),
                passed: true,
                summary: "tracked.txt passed".to_owned(),
                payload: json!({"sha256": "fixed"}),
            }],
            manual_gate: None,
        };
        (root, lease, report)
    }

    #[tokio::test]
    async fn archive_is_deterministic_and_includes_tracked_and_untracked_source() {
        let (root, lease, report) = fixture();
        fs::write(root.join("tracked.txt"), b"after\n").expect("modify tracked");
        fs::write(root.join("new.txt"), b"new\n").expect("write untracked");
        fs::write(root.join("provider.md"), b"provider\n").expect("write provider");
        let provider = AdapterArtifact {
            path: root.join("provider.md"),
            sha256: hex::encode(Sha256::digest(b"provider\n")),
            bytes: 9,
            media_type: "text/markdown".to_owned(),
        };
        let spec = DeliverableSpec {
            form: DeliverableForm::Archive,
            commit_after_verification: false,
            paths: Vec::new(),
        };
        let first = export(
            Uuid::new_v4(),
            &spec,
            &lease,
            &report,
            std::slice::from_ref(&provider),
            &["**".to_owned()],
        )
        .await
        .expect("first export");
        let second = export(
            Uuid::new_v4(),
            &spec,
            &lease,
            &report,
            &[provider],
            &["**".to_owned()],
        )
        .await
        .expect("second export");
        assert_eq!(first.bytes, second.bytes);
        let document: Value = serde_json::from_slice(&first.bytes).expect("parse archive");
        let paths = document["changes"]
            .as_array()
            .expect("changes")
            .iter()
            .filter_map(|change| change["path"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(paths, vec!["new.txt", "tracked.txt"]);
        assert_eq!(
            document["verification_sha256"].as_str(),
            Some(first.verification_sha256.as_str())
        );
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[tokio::test]
    async fn secret_like_paths_are_rejected_before_export() {
        let (root, lease, report) = fixture();
        fs::write(root.join(".env"), b"TOKEN=secret\n").expect("write secret");
        let error = export(
            Uuid::new_v4(),
            &DeliverableSpec {
                form: DeliverableForm::Patch,
                commit_after_verification: false,
                paths: Vec::new(),
            },
            &lease,
            &report,
            &[],
            &["**".to_owned()],
        )
        .await
        .expect_err("secret-like path must fail");
        assert!(error.to_string().contains("secret-like"));
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[tokio::test]
    async fn nested_secret_directories_are_rejected_before_export() {
        let (root, lease, report) = fixture();
        let credential_dir = root.join("services").join("api").join(".azure");
        fs::create_dir_all(&credential_dir).expect("create nested credential directory");
        fs::write(
            credential_dir.join("accessTokens.json"),
            b"{\"token\":\"secret\"}\n",
        )
        .expect("write nested credential");
        let error = export(
            Uuid::new_v4(),
            &DeliverableSpec {
                form: DeliverableForm::Patch,
                commit_after_verification: false,
                paths: Vec::new(),
            },
            &lease,
            &report,
            &[],
            &["**".to_owned()],
        )
        .await
        .expect_err("nested secret-like path must fail");
        assert!(error.to_string().contains("secret-like"));
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[test]
    fn sensitive_directories_match_at_every_depth() {
        assert!(sensitive_path(
            "services/api/.config/gcloud/application_default_credentials.json"
        ));
        assert!(sensitive_path("packages/web/.azure/accessTokens.json"));
        assert!(sensitive_path("nested/.ssh/id_ed25519"));
        assert!(sensitive_path("services/api/.kube/config"));
        assert!(sensitive_path("nested/.docker/config.json"));
        assert!(!sensitive_path("docs/azure/accessTokens.json"));
    }

    #[tokio::test]
    async fn git_pathspec_magic_is_rejected_before_staging() {
        let (root, lease, report) = fixture();
        fs::write(root.join("tracked.txt"), b"selected\n").expect("modify selected path");
        fs::write(root.join("other.txt"), b"must remain unselected\n")
            .expect("modify unselected path");
        let error = export(
            Uuid::new_v4(),
            &DeliverableSpec {
                form: DeliverableForm::Archive,
                commit_after_verification: false,
                paths: vec![":(exclude)tracked.txt".to_owned()],
            },
            &lease,
            &report,
            &[],
            &["**".to_owned()],
        )
        .await
        .expect_err("Git pathspec magic must fail");
        assert!(error.to_string().contains("stay inside the worktree"));
        assert_eq!(git(&root, &["diff", "--cached", "--name-only"]), "");
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[tokio::test]
    async fn default_export_rejects_changes_outside_task_write_scope() {
        let (root, lease, report) = fixture();
        fs::create_dir_all(root.join("src")).expect("create scoped directory");
        fs::write(root.join("src").join("allowed.txt"), b"allowed\n").expect("write scoped source");
        fs::write(root.join("outside.txt"), b"outside\n").expect("write outside source");
        let error = export(
            Uuid::new_v4(),
            &DeliverableSpec {
                form: DeliverableForm::Archive,
                commit_after_verification: false,
                paths: Vec::new(),
            },
            &lease,
            &report,
            &[],
            &["src/**".to_owned()],
        )
        .await
        .expect_err("out-of-scope source must fail");
        assert!(
            error
                .to_string()
                .contains("outside the task write scope: outside.txt")
        );
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[tokio::test]
    async fn commit_form_updates_only_the_isolated_branch_after_verification() {
        let (root, lease, report) = fixture();
        fs::write(root.join("tracked.txt"), b"committed\n").expect("modify tracked");
        fs::write(root.join("new.txt"), b"committed new\n").expect("write untracked");
        fs::write(root.join("provider.md"), b"provider\n").expect("write provider");
        let provider = AdapterArtifact {
            path: root.join("provider.md"),
            sha256: hex::encode(Sha256::digest(b"provider\n")),
            bytes: 9,
            media_type: "text/markdown".to_owned(),
        };
        let exported = export(
            Uuid::new_v4(),
            &DeliverableSpec {
                form: DeliverableForm::CommitBranch,
                commit_after_verification: true,
                paths: Vec::new(),
            },
            &lease,
            &report,
            &[provider],
            &["**".to_owned()],
        )
        .await
        .expect("commit export");
        let head = git(&root, &["rev-parse", "HEAD"]);
        assert_eq!(exported.head_commit.as_deref(), Some(head.as_str()));
        assert!(
            git(&root, &["show", "--format=%B", "--no-patch", "HEAD"])
                .contains(&exported.verification_sha256)
        );
        assert!(git(&root, &["show", "--format=", "--name-only", "HEAD"]).contains("new.txt"));
        assert!(exported.publication_ready);
        let document: Value =
            serde_json::from_slice(&exported.bytes).expect("parse commit/branch deliverable");
        let bundle = BASE64
            .decode(
                document["git_bundle_base64"]
                    .as_str()
                    .expect("bundle base64"),
            )
            .expect("decode bundle");
        let bundle_sha256 = hex::encode(Sha256::digest(&bundle));
        assert_eq!(
            document["git_bundle_sha256"].as_str(),
            Some(bundle_sha256.as_str())
        );
        let bundle_path = root.join("published.bundle");
        fs::write(&bundle_path, bundle).expect("write bundle");
        assert!(git(&root, &["bundle", "list-heads", "published.bundle"]).ends_with(" HEAD"));
        fs::remove_file(bundle_path).expect("remove bundle");
        assert_eq!(git(&root, &["status", "--porcelain=v1"]), "?? provider.md");
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[tokio::test]
    async fn scoped_commit_preserves_unselected_staged_work() {
        let (root, lease, report) = fixture();
        fs::write(root.join("tracked.txt"), b"selected\n").expect("modify selected");
        fs::write(root.join("other.txt"), b"other staged\n").expect("modify other");
        git(&root, &["add", "other.txt"]);
        let exported = export(
            Uuid::new_v4(),
            &DeliverableSpec {
                form: DeliverableForm::CommitBranch,
                commit_after_verification: true,
                paths: vec!["tracked.txt".to_owned()],
            },
            &lease,
            &report,
            &[],
            &["**".to_owned()],
        )
        .await
        .expect("scoped commit export");
        assert!(exported.head_commit.is_some());
        assert_eq!(
            git(&root, &["diff", "--cached", "--name-only"]),
            "other.txt"
        );
        assert_eq!(
            git(&root, &["show", "--format=", "--name-only", "HEAD"]),
            "tracked.txt"
        );
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[tokio::test]
    async fn scoped_commit_excludes_unselected_committed_changes() {
        let (root, lease, report) = fixture();
        fs::write(
            root.join("other.txt"),
            b"agent committed outside selection\n",
        )
        .expect("modify unselected path");
        git(&root, &["add", "other.txt"]);
        git(&root, &["commit", "-m", "agent commit outside selection"]);
        let agent_head = git(&root, &["rev-parse", "HEAD"]);
        fs::write(root.join("tracked.txt"), b"selected\n").expect("modify selected path");

        let exported = export(
            Uuid::new_v4(),
            &DeliverableSpec {
                form: DeliverableForm::CommitBranch,
                commit_after_verification: true,
                paths: vec!["tracked.txt".to_owned()],
            },
            &lease,
            &report,
            &[],
            &["tracked.txt".to_owned()],
        )
        .await
        .expect("scoped commit export");

        let head = git(&root, &["rev-parse", "HEAD"]);
        assert_eq!(exported.head_commit.as_deref(), Some(head.as_str()));
        assert_ne!(head, agent_head);
        assert_eq!(
            git(&root, &["rev-list", "--parents", "-n", "1", "HEAD"]),
            format!("{head} {}", lease.base_commit)
        );
        assert_eq!(
            git(
                &root,
                &[
                    "diff",
                    "--name-only",
                    &format!("{}..HEAD", lease.base_commit)
                ]
            ),
            "tracked.txt"
        );
        assert_eq!(
            git(&root, &["diff", "--cached", "--name-only"]),
            "other.txt"
        );
        assert_eq!(
            fs::read_to_string(root.join("other.txt")).expect("read preserved path"),
            "agent committed outside selection\n"
        );
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symbolic_links_are_rejected_before_content_export() {
        use std::os::unix::fs::symlink;

        let (root, lease, report) = fixture();
        let outside = root
            .parent()
            .expect("fixture parent")
            .join(format!("outside-{}", Uuid::new_v4()));
        fs::write(&outside, b"outside\n").expect("write outside target");
        symlink(&outside, root.join("escape-link")).expect("create symlink");
        let error = export(
            Uuid::new_v4(),
            &DeliverableSpec {
                form: DeliverableForm::Archive,
                commit_after_verification: false,
                paths: Vec::new(),
            },
            &lease,
            &report,
            &[],
            &["**".to_owned()],
        )
        .await
        .expect_err("symlink must fail");
        assert!(error.to_string().contains("symbolic link"));
        fs::remove_dir_all(root).expect("remove fixture");
        fs::remove_file(outside).expect("remove outside target");
    }
}
