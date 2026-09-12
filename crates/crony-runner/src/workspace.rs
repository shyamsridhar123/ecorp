use std::{
    ffi::OsString,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    process::{Output, Stdio},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use sha2::{Digest, Sha256};
use tokio::{process::Command, sync::Mutex};
use tracing::warn;
use url::Url;
use uuid::Uuid;

const GIT_TIMEOUT: Duration = Duration::from_secs(30);
const VERIFICATION_SNAPSHOT_CLEANUP_TIMEOUT: Duration = Duration::from_secs(10);
const CHECKPOINT_MAX_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const CHECKPOINT_READ_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct WorkspaceManager {
    root: PathBuf,
    worktrees_root: PathBuf,
    repository: PathBuf,
    repository_identity: Option<String>,
    base_ref: String,
    base_commit: String,
    git_lock: Arc<Mutex<()>>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceLease {
    pub path: PathBuf,
    pub branch: String,
    pub base_ref: String,
    pub base_commit: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceDisposition {
    Removed,
    Preserved,
}

#[derive(Debug, Clone)]
pub struct WorkspaceCleanup {
    pub disposition: WorkspaceDisposition,
    pub detail: String,
    pub dirty: Option<bool>,
    pub commits_ahead: Option<u64>,
    pub branch_deleted: bool,
}

impl WorkspaceManager {
    pub async fn initialize(root: PathBuf, repository: PathBuf, base_ref: String) -> Result<Self> {
        Self::initialize_inner(root, repository, base_ref, None).await
    }

    /// Restore a source snapshot without resolving its symbolic ref's current tip.
    /// Use `pinned` for additional snapshots that must share this manager's Git lock.
    pub async fn initialize_pinned(
        root: PathBuf,
        repository: PathBuf,
        base_ref: String,
        base_commit: String,
    ) -> Result<Self> {
        validate_commit(&base_commit)?;
        Self::initialize_inner(root, repository, base_ref, Some(base_commit)).await
    }

    async fn initialize_inner(
        root: PathBuf,
        repository: PathBuf,
        base_ref: String,
        base_commit: Option<String>,
    ) -> Result<Self> {
        validate_ref(&base_ref)?;
        tokio::fs::create_dir_all(&root)
            .await
            .with_context(|| format!("create runner workspace root {}", root.display()))?;
        let root = canonicalize_path(&root)
            .await
            .with_context(|| format!("resolve runner workspace root {}", root.display()))?;
        let worktrees_root = root.join("worktrees");
        tokio::fs::create_dir_all(&worktrees_root)
            .await
            .with_context(|| format!("create worktree root {}", worktrees_root.display()))?;
        let worktrees_root = canonicalize_path(&worktrees_root)
            .await
            .with_context(|| format!("resolve worktree root {}", worktrees_root.display()))?;
        ensure_descendant(&root, &worktrees_root)?;

        let repository = canonicalize_path(&repository)
            .await
            .with_context(|| format!("resolve source repository {}", repository.display()))?;
        let mut manager = Self {
            root,
            worktrees_root,
            repository,
            repository_identity: None,
            base_ref,
            base_commit: String::new(),
            git_lock: Arc::new(Mutex::new(())),
        };
        manager
            .git_success(&manager.repository, &["rev-parse", "--git-dir"])
            .await
            .context("configured source path is not a Git repository")?;
        manager.base_commit = match base_commit {
            Some(commit) => manager.resolve_pinned_commit(&commit).await?,
            None => manager.resolve_base_commit().await?,
        };
        manager.repository_identity = manager
            .git_text(
                &manager.repository,
                &["config", "--get", "remote.origin.url"],
            )
            .await
            .ok()
            .and_then(|remote| parse_github_repository_identity(&remote))
            .or_else(|| Some(local_repository_identity(&manager.repository)));
        Ok(manager)
    }

    /// Select an exact source snapshot while sharing repository ownership and Git serialization.
    /// Pass its commit to `prepare`; an unassigned prepare still follows the live base ref.
    pub async fn pinned(&self, base_ref: &str, base_commit: &str) -> Result<Self> {
        validate_ref(base_ref)?;
        let base_commit = self.resolve_pinned_commit(base_commit).await?;
        Ok(Self {
            base_ref: base_ref.to_owned(),
            base_commit,
            ..self.clone()
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn repository(&self) -> &Path {
        &self.repository
    }

    pub fn repository_identity(&self) -> Option<&str> {
        self.repository_identity.as_deref()
    }

    pub fn base_ref(&self) -> &str {
        &self.base_ref
    }

    pub fn base_commit(&self) -> &str {
        &self.base_commit
    }

    pub fn verify_source_identity(
        &self,
        repository: Option<&str>,
        base_ref: Option<&str>,
        base_commit: Option<&str>,
    ) -> Result<()> {
        let (Some(repository), Some(base_ref), Some(base_commit)) =
            (repository, base_ref, base_commit)
        else {
            if repository.is_none() && base_ref.is_none() && base_commit.is_none() {
                return Ok(());
            }
            return Err(anyhow!(
                "source assignment must include repository, base ref, and immutable base commit"
            ));
        };
        let configured_repository = self
            .repository_identity()
            .context("runner source repository has no resolved GitHub identity")?;
        if !configured_repository.eq_ignore_ascii_case(repository) {
            return Err(anyhow!(
                "source repository mismatch: assignment requires {repository}, runner is configured for {configured_repository}"
            ));
        }
        if self.base_ref != base_ref {
            return Err(anyhow!(
                "source base ref mismatch: assignment requires {base_ref}, runner is configured for {}",
                self.base_ref
            ));
        }
        if !self.base_commit.eq_ignore_ascii_case(base_commit) {
            return Err(anyhow!(
                "source base commit mismatch: assignment requires {base_commit}, runner is pinned to {}",
                self.base_commit
            ));
        }
        Ok(())
    }

    pub async fn prepare(
        &self,
        task_id: Uuid,
        workspace_run_id: Uuid,
        assigned_base_commit: Option<&str>,
        resume_base_commit: Option<&str>,
    ) -> Result<WorkspaceLease> {
        let _guard = self.git_lock.lock().await;
        if assigned_base_commit
            .zip(resume_base_commit)
            .is_some_and(|(assigned, resumed)| !assigned.eq_ignore_ascii_case(resumed))
        {
            return Err(anyhow!(
                "resumed workspace base commit does not match the pinned source assignment"
            ));
        }
        let branch = branch_name(task_id, workspace_run_id);
        let path = self
            .worktrees_root
            .join(task_id.simple().to_string())
            .join(workspace_run_id.simple().to_string());
        self.ensure_target_parent(&path).await?;

        if tokio::fs::try_exists(&path).await? {
            return self
                .verify_existing(path, branch, assigned_base_commit.or(resume_base_commit))
                .await;
        }
        if resume_base_commit.is_some() {
            return Err(anyhow!(
                "preserved workspace is missing and cannot be recreated for resume"
            ));
        }
        let base_commit = match assigned_base_commit {
            Some(commit) => commit.to_owned(),
            None => self.resolve_base_commit_locked().await?,
        };

        let branch_ref = format!("refs/heads/{branch}");
        let branch_exists = self
            .git_exit_success(
                &self.repository,
                &[
                    OsString::from("show-ref"),
                    OsString::from("--verify"),
                    OsString::from("--quiet"),
                    OsString::from(&branch_ref),
                ],
            )
            .await?;
        let mut args = vec![OsString::from("worktree"), OsString::from("add")];
        if branch_exists {
            args.push(path.as_os_str().to_owned());
            args.push(OsString::from(&branch));
        } else {
            args.push(OsString::from("-b"));
            args.push(OsString::from(&branch));
            args.push(path.as_os_str().to_owned());
            args.push(OsString::from(&base_commit));
        }
        self.git_success_os(&self.repository, &args)
            .await
            .with_context(|| format!("create worktree {} on branch {branch}", path.display()))?;
        self.verify_existing(path, branch, Some(&base_commit)).await
    }

    pub async fn finalize(&self, workspace: &WorkspaceLease) -> Result<WorkspaceCleanup> {
        let _guard = self.git_lock.lock().await;
        self.verify_managed_path(&workspace.path).await?;
        if !tokio::fs::try_exists(&workspace.path).await? {
            return Ok(WorkspaceCleanup {
                disposition: WorkspaceDisposition::Removed,
                detail: "worktree was already absent".to_owned(),
                dirty: None,
                commits_ahead: None,
                branch_deleted: false,
            });
        }

        if let Err(error) = self
            .verify_existing(
                workspace.path.clone(),
                workspace.branch.clone(),
                Some(&workspace.base_commit),
            )
            .await
        {
            return Ok(preserved(format!(
                "worktree identity could not be verified: {error:#}"
            )));
        }

        let status = match self
            .git_text_os(
                &workspace.path,
                &[
                    OsString::from("status"),
                    OsString::from("--porcelain=v1"),
                    OsString::from("--untracked-files=all"),
                ],
            )
            .await
        {
            Ok(status) => status,
            Err(error) => {
                return Ok(preserved(format!(
                    "working-tree status could not be verified: {error:#}"
                )));
            }
        };
        let ignored = match self
            .git_text_os(
                &workspace.path,
                &[
                    OsString::from("ls-files"),
                    OsString::from("--others"),
                    OsString::from("--ignored"),
                    OsString::from("--exclude-standard"),
                    OsString::from("-z"),
                ],
            )
            .await
        {
            Ok(ignored) => ignored,
            Err(error) => {
                return Ok(preserved(format!(
                    "ignored-file state could not be verified: {error:#}"
                )));
            }
        };
        let dirty = !status.trim().is_empty() || !ignored.is_empty();
        if dirty {
            return Ok(WorkspaceCleanup {
                disposition: WorkspaceDisposition::Preserved,
                detail: if ignored.is_empty() {
                    "working tree contains uncommitted or untracked changes".to_owned()
                } else {
                    "working tree contains ignored files that cleanup must not discard".to_owned()
                },
                dirty: Some(true),
                commits_ahead: None,
                branch_deleted: false,
            });
        }

        let current_base_commit = match self.resolve_base_commit_locked().await {
            Ok(commit) => commit,
            Err(error) => {
                return Ok(WorkspaceCleanup {
                    disposition: WorkspaceDisposition::Preserved,
                    detail: format!("base ref could not be resolved safely: {error:#}"),
                    dirty: Some(false),
                    commits_ahead: None,
                    branch_deleted: false,
                });
            }
        };
        let ahead = match self
            .git_text_os(
                &workspace.path,
                &[
                    OsString::from("rev-list"),
                    OsString::from("--count"),
                    OsString::from(format!("{current_base_commit}..HEAD")),
                ],
            )
            .await
            .and_then(|value| {
                value
                    .trim()
                    .parse::<u64>()
                    .context("parse commits-ahead count")
            }) {
            Ok(ahead) => ahead,
            Err(error) => {
                return Ok(WorkspaceCleanup {
                    disposition: WorkspaceDisposition::Preserved,
                    detail: format!("commit reachability could not be verified: {error:#}"),
                    dirty: Some(false),
                    commits_ahead: None,
                    branch_deleted: false,
                });
            }
        };
        let tree_integrated = self
            .git_exit_success(
                &workspace.path,
                &[
                    OsString::from("diff"),
                    OsString::from("--quiet"),
                    OsString::from(&current_base_commit),
                    OsString::from("HEAD"),
                    OsString::from("--"),
                ],
            )
            .await
            .unwrap_or(false);
        if ahead > 0 && !tree_integrated {
            return Ok(WorkspaceCleanup {
                disposition: WorkspaceDisposition::Preserved,
                detail: format!(
                    "{ahead} commit(s) are not integrated into {}",
                    workspace.base_ref
                ),
                dirty: Some(false),
                commits_ahead: Some(ahead),
                branch_deleted: false,
            });
        }

        self.verify_managed_path(&workspace.path).await?;
        self.git_success_os(
            &self.repository,
            &[
                OsString::from("worktree"),
                OsString::from("remove"),
                workspace.path.as_os_str().to_owned(),
            ],
        )
        .await
        .with_context(|| format!("remove safe worktree {}", workspace.path.display()))?;
        let branch_deleted = self
            .git_exit_success(
                &self.repository,
                &[
                    OsString::from("branch"),
                    OsString::from("-d"),
                    OsString::from(&workspace.branch),
                ],
            )
            .await
            .unwrap_or(false);
        Ok(WorkspaceCleanup {
            disposition: WorkspaceDisposition::Removed,
            detail: if tree_integrated && ahead > 0 {
                format!(
                    "clean worktree tree is already integrated into {}; branch deletion={branch_deleted}",
                    workspace.base_ref
                )
            } else {
                format!(
                    "clean worktree has no commits ahead of {}; branch deletion={branch_deleted}",
                    workspace.base_ref
                )
            },
            dirty: Some(false),
            commits_ahead: Some(ahead),
            branch_deleted,
        })
    }

    pub async fn fingerprint(&self, workspace: &WorkspaceLease) -> Result<String> {
        self.verify_managed_path(&workspace.path).await?;
        fingerprint_path(&workspace.path).await
    }

    pub async fn head_commit(&self, workspace: &WorkspaceLease) -> Result<String> {
        self.verify_managed_path(&workspace.path).await?;
        self.git_text_os(
            &workspace.path,
            &[OsString::from("rev-parse"), OsString::from("HEAD^{commit}")],
        )
        .await
        .map(|value| value.trim().to_ascii_lowercase())
    }

    pub async fn checkpoint(&self, workspace: &WorkspaceLease) -> Result<(String, String)> {
        self.verify_existing(
            workspace.path.clone(),
            workspace.branch.clone(),
            Some(&workspace.base_commit),
        )
        .await?;
        let head = self.head_commit(workspace).await?;
        let fingerprint = fingerprint_checkpoint_path(
            &workspace.path,
            CHECKPOINT_MAX_BYTES,
            CHECKPOINT_READ_TIMEOUT,
        )
        .await?;
        // Repeat the existing root/branch/base checks after reading the bytes.
        self.verify_existing(
            workspace.path.clone(),
            workspace.branch.clone(),
            Some(&workspace.base_commit),
        )
        .await?;
        if self.head_commit(workspace).await? != head {
            return Err(anyhow!("workspace HEAD changed during checkpoint"));
        }
        Ok((head, fingerprint))
    }

    #[cfg(test)]
    pub(super) async fn hold_operations_for_test(&self) -> tokio::sync::OwnedMutexGuard<()> {
        self.git_lock.clone().lock_owned().await
    }

    async fn verify_existing(
        &self,
        path: PathBuf,
        branch: String,
        expected_base_commit: Option<&str>,
    ) -> Result<WorkspaceLease> {
        self.verify_managed_path(&path).await?;
        let top = self
            .git_text_os(
                &path,
                &[
                    OsString::from("rev-parse"),
                    OsString::from("--show-toplevel"),
                ],
            )
            .await
            .with_context(|| format!("inspect existing worktree {}", path.display()))?;
        let top = canonicalize_path(Path::new(top.trim()))
            .await
            .with_context(|| format!("resolve reported worktree root {}", top.trim()))?;
        let canonical_path = canonicalize_path(&path)
            .await
            .with_context(|| format!("resolve existing worktree {}", path.display()))?;
        ensure_descendant(&self.worktrees_root, &canonical_path)?;
        if top != canonical_path {
            return Err(anyhow!(
                "path {} resolves to Git root {}, not the expected worktree",
                canonical_path.display(),
                top.display()
            ));
        }
        let current_branch = self
            .git_text_os(
                &canonical_path,
                &[
                    OsString::from("symbolic-ref"),
                    OsString::from("--quiet"),
                    OsString::from("--short"),
                    OsString::from("HEAD"),
                ],
            )
            .await
            .context("worktree is detached or its branch cannot be verified")?;
        if current_branch.trim() != branch {
            return Err(anyhow!(
                "worktree branch mismatch: expected {branch}, found {}",
                current_branch.trim()
            ));
        }
        let base_commit = match expected_base_commit {
            Some(commit) => commit.to_owned(),
            None => {
                let current_base_commit = self.resolve_base_commit_locked().await?;
                self.git_text_os(
                    &canonical_path,
                    &[
                        OsString::from("merge-base"),
                        OsString::from("HEAD"),
                        OsString::from(&current_base_commit),
                    ],
                )
                .await?
                .trim()
                .to_owned()
            }
        };
        let base_is_ancestor = self
            .git_exit_success(
                &canonical_path,
                &[
                    OsString::from("merge-base"),
                    OsString::from("--is-ancestor"),
                    OsString::from(&base_commit),
                    OsString::from("HEAD"),
                ],
            )
            .await?;
        if !base_is_ancestor {
            return Err(anyhow!(
                "worktree does not descend from expected base commit {base_commit}"
            ));
        }
        Ok(WorkspaceLease {
            path: canonical_path,
            branch,
            base_ref: self.base_ref.clone(),
            base_commit,
        })
    }

    async fn ensure_target_parent(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .context("managed worktree path has no parent")?;
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create worktree parent {}", parent.display()))?;
        let parent = canonicalize_path(parent)
            .await
            .with_context(|| format!("resolve worktree parent {}", parent.display()))?;
        ensure_descendant(&self.worktrees_root, &parent)?;
        let file_name = path
            .file_name()
            .context("managed worktree path has no final segment")?;
        if Path::new(file_name)
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(anyhow!("worktree final path segment is unsafe"));
        }
        Ok(())
    }

    async fn verify_managed_path(&self, path: &Path) -> Result<()> {
        let absolute = if tokio::fs::try_exists(path).await? {
            canonicalize_path(path)
                .await
                .with_context(|| format!("resolve managed worktree {}", path.display()))?
        } else {
            let parent = path
                .parent()
                .context("managed worktree path has no parent")?;
            let parent = canonicalize_path(parent)
                .await
                .with_context(|| format!("resolve managed parent {}", parent.display()))?;
            parent.join(
                path.file_name()
                    .context("managed worktree path has no final segment")?,
            )
        };
        ensure_descendant(&self.worktrees_root, &absolute)
    }

    async fn resolve_base_commit(&self) -> Result<String> {
        let _guard = self.git_lock.lock().await;
        self.resolve_base_commit_locked().await
    }

    async fn resolve_base_commit_locked(&self) -> Result<String> {
        self.git_text(
            &self.repository,
            &[
                "rev-parse",
                "--verify",
                &format!("{}^{{commit}}", self.base_ref),
            ],
        )
        .await
        .map(|value| value.trim().to_owned())
        .with_context(|| format!("resolve base ref {}", self.base_ref))
    }

    async fn resolve_pinned_commit(&self, base_commit: &str) -> Result<String> {
        validate_commit(base_commit)?;
        let _guard = self.git_lock.lock().await;
        let resolved = self
            .git_text(
                &self.repository,
                &[
                    "rev-parse",
                    "--verify",
                    &format!("{base_commit}^{{commit}}"),
                ],
            )
            .await
            .with_context(|| format!("resolve pinned base commit {base_commit}"))?;
        let resolved = resolved.trim();
        if !resolved.eq_ignore_ascii_case(base_commit) {
            return Err(anyhow!(
                "pinned base commit mismatch: expected {base_commit}, resolved {resolved}"
            ));
        }
        Ok(resolved.to_ascii_lowercase())
    }

    async fn git_success(&self, cwd: &Path, args: &[&str]) -> Result<()> {
        let args = args.iter().map(OsString::from).collect::<Vec<_>>();
        self.git_success_os(cwd, &args).await
    }

    async fn git_success_os(&self, cwd: &Path, args: &[OsString]) -> Result<()> {
        let output = run_git(cwd, args).await?;
        if output.status.success() {
            Ok(())
        } else {
            Err(git_error(args, &output))
        }
    }

    async fn git_text(&self, cwd: &Path, args: &[&str]) -> Result<String> {
        let args = args.iter().map(OsString::from).collect::<Vec<_>>();
        self.git_text_os(cwd, &args).await
    }

    async fn git_text_os(&self, cwd: &Path, args: &[OsString]) -> Result<String> {
        let output = run_git(cwd, args).await?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(git_error(args, &output))
        }
    }

    async fn git_exit_success(&self, cwd: &Path, args: &[OsString]) -> Result<bool> {
        Ok(run_git(cwd, args).await?.status.success())
    }
}

pub async fn fingerprint_path(root: &Path) -> Result<String> {
    let root = root.to_owned();
    tokio::task::spawn_blocking(move || fingerprint_workspace(&root))
        .await
        .context("join workspace fingerprint task")?
}

pub(super) async fn fingerprint_checkpoint_path(
    root: &Path,
    max_bytes: u64,
    timeout: Duration,
) -> Result<String> {
    let root = root.to_owned();
    let deadline = std::time::Instant::now() + timeout;
    tokio::task::spawn_blocking(move || {
        fingerprint_workspace_with_limits(&root, Some(max_bytes), Some(deadline))
    })
    .await
    .context("join bounded workspace checkpoint task")?
}

pub async fn find_file_by_digest(
    root: &Path,
    file_name: &str,
    expected_sha256: &str,
    expected_bytes: usize,
) -> Result<PathBuf> {
    let root = root.to_owned();
    let file_name = file_name.to_owned();
    let expected_sha256 = expected_sha256.to_owned();
    tokio::task::spawn_blocking(move || {
        find_workspace_file_by_digest(&root, &file_name, &expected_sha256, expected_bytes)
    })
    .await
    .context("join workspace artifact search task")?
}

#[derive(Debug)]
pub struct VerificationSnapshot {
    path: Option<PathBuf>,
}

impl VerificationSnapshot {
    pub fn path(&self) -> &Path {
        self.path
            .as_deref()
            .expect("verification snapshot path is unavailable after cleanup")
    }

    pub async fn cleanup(&mut self) -> Result<()> {
        let Some(path) = self.path.clone() else {
            return Ok(());
        };
        remove_verification_snapshot(path).await?;
        self.path = None;
        Ok(())
    }
}

impl Drop for VerificationSnapshot {
    fn drop(&mut self) {
        let Some(path) = self.path.take() else {
            return;
        };
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn(async move {
                    if let Err(error) = remove_verification_snapshot(path.clone()).await {
                        warn!(
                            path = %path.display(),
                            error = %format!("{error:#}"),
                            "background verifier snapshot cleanup failed"
                        );
                    }
                });
            }
            Err(error) => {
                warn!(
                    path = %path.display(),
                    error = %error,
                    "verifier snapshot dropped outside a Tokio runtime"
                );
            }
        }
    }
}

async fn fresh_verification_snapshot_path(run_id: Uuid) -> Result<PathBuf> {
    let temp_root = tokio::fs::canonicalize(std::env::temp_dir())
        .await
        .context("canonicalize system temporary directory")?;
    let parent = temp_root.join("ecorp-verification-snapshots");
    tokio::fs::create_dir_all(&parent)
        .await
        .context("create verifier snapshot parent")?;
    #[cfg(unix)]
    tokio::fs::set_permissions(&parent, std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .await
        .context("secure verifier snapshot parent permissions")?;
    let parent = tokio::fs::canonicalize(&parent)
        .await
        .context("canonicalize verifier snapshot parent")?;
    if !parent.starts_with(&temp_root) {
        return Err(anyhow!(
            "verifier snapshot parent escapes the system temporary directory"
        ));
    }
    Ok(parent.join(format!("{}-{}", run_id.simple(), Uuid::new_v4().simple())))
}

/// A separate private owner for transferred evidence, never part of a source/check snapshot.
pub async fn empty_verification_snapshot(run_id: Uuid) -> Result<VerificationSnapshot> {
    let path = fresh_verification_snapshot_path(run_id).await?;
    tokio::fs::create_dir(&path)
        .await
        .context("create empty private verifier snapshot")?;
    #[cfg(unix)]
    if let Err(error) =
        tokio::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o700)).await
    {
        let cleanup = remove_verification_snapshot(path).await;
        return Err(anyhow!(error).context(match cleanup {
            Ok(()) => "secure empty verifier snapshot permissions".to_owned(),
            Err(cleanup_error) => format!(
                "secure empty verifier snapshot permissions; cleanup also failed: {cleanup_error:#}"
            ),
        }));
    }
    Ok(VerificationSnapshot { path: Some(path) })
}

pub async fn verification_snapshot(root: &Path, run_id: Uuid) -> Result<VerificationSnapshot> {
    let root = tokio::fs::canonicalize(root)
        .await
        .context("canonicalize verifier-only source workspace")?;
    let path = fresh_verification_snapshot_path(run_id).await?;
    let snapshot_root = path.clone();
    let result = tokio::task::spawn_blocking(move || {
        copy_workspace_snapshot(&root, &snapshot_root)?;
        Ok::<_, anyhow::Error>(snapshot_root)
    })
    .await
    .context("join verifier-only workspace snapshot task")?;
    match result {
        Ok(path) => Ok(VerificationSnapshot { path: Some(path) }),
        Err(error) => match remove_verification_snapshot(path).await {
            Ok(()) => Err(error),
            Err(cleanup_error) => Err(error.context(format!(
                "verifier snapshot cleanup also failed: {cleanup_error:#}"
            ))),
        },
    }
}

async fn remove_verification_snapshot(path: PathBuf) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let display = path.display().to_string();
    tokio::time::timeout(
        VERIFICATION_SNAPSHOT_CLEANUP_TIMEOUT,
        tokio::task::spawn_blocking(move || {
            // Only private snapshot copies are made removable; never relax source permissions.
            let result = prepare_snapshot_cleanup(&path).and_then(|()| fs::remove_dir_all(&path));
            match result {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => {
                    Err(error).with_context(|| format!("remove verifier snapshot {display}"))
                }
            }
        }),
    )
    .await
    .context("verifier snapshot cleanup timed out")?
    .context("join verifier snapshot cleanup task")?
}

fn prepare_snapshot_cleanup(path: &Path) -> std::io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if snapshot_entry_is_link(&metadata) {
        return Ok(());
    }
    #[cfg(unix)]
    if metadata.is_dir() {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = metadata.permissions();
        permissions.set_mode(permissions.mode() | 0o700);
        fs::set_permissions(path, permissions)?;
    }
    #[cfg(windows)]
    if metadata.permissions().readonly() {
        let mut permissions = metadata.permissions();
        // This clears only the Windows readonly attribute; Unix uses owner-mode bits above.
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(path, permissions)?;
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            prepare_snapshot_cleanup(&entry?.path())?;
        }
    }
    Ok(())
}

fn copy_workspace_snapshot(root: &Path, destination: &Path) -> Result<()> {
    const MAX_ENTRIES: usize = 100_000;
    const MAX_BYTES: u64 = 4 * 1024 * 1024 * 1024;
    if destination.exists() {
        return Err(anyhow!(
            "verifier-only workspace snapshot target already exists"
        ));
    }
    fs::create_dir(destination).with_context(|| {
        format!(
            "create verifier-only workspace snapshot {}",
            destination.display()
        )
    })?;
    #[cfg(unix)]
    fs::set_permissions(
        destination,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .with_context(|| {
        format!(
            "secure verifier-only workspace snapshot {}",
            destination.display()
        )
    })?;
    let mut entries = 0_usize;
    let mut bytes = 0_u64;
    copy_snapshot_directory(
        root,
        root,
        destination,
        &mut entries,
        &mut bytes,
        MAX_ENTRIES,
        MAX_BYTES,
    )
}

#[allow(clippy::too_many_arguments)]
fn copy_snapshot_directory(
    root: &Path,
    source: &Path,
    destination: &Path,
    entries: &mut usize,
    bytes: &mut u64,
    max_entries: usize,
    max_bytes: u64,
) -> Result<()> {
    for entry in fs::read_dir(source)
        .with_context(|| format!("read verifier snapshot directory {}", source.display()))?
    {
        let entry = entry?;
        let source_path = entry.path();
        if is_git_control_path(root, &source_path) {
            continue;
        }
        *entries += 1;
        if *entries > max_entries {
            return Err(anyhow!(
                "verifier-only workspace snapshot exceeds {max_entries} entries"
            ));
        }
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path).with_context(|| {
            format!("inspect verifier snapshot entry {}", source_path.display())
        })?;
        let file_type = metadata.file_type();
        if snapshot_entry_is_link(&metadata) {
            copy_snapshot_symlink(root, &source_path, &destination_path)?;
        } else if file_type.is_dir() {
            fs::create_dir(&destination_path).with_context(|| {
                format!(
                    "create verifier snapshot directory {}",
                    destination_path.display()
                )
            })?;
            copy_snapshot_directory(
                root,
                &source_path,
                &destination_path,
                entries,
                bytes,
                max_entries,
                max_bytes,
            )?;
            // The root stays private (0700), but entry modes must match the sealed source.
            fs::set_permissions(&destination_path, metadata.permissions()).with_context(|| {
                format!(
                    "preserve verifier snapshot directory permissions {}",
                    destination_path.display()
                )
            })?;
        } else if file_type.is_file() {
            let length = metadata.len();
            *bytes = bytes
                .checked_add(length)
                .context("verifier-only workspace snapshot byte count overflowed")?;
            if *bytes > max_bytes {
                return Err(anyhow!(
                    "verifier-only workspace snapshot exceeds {max_bytes} bytes"
                ));
            }
            fs::copy(&source_path, &destination_path).with_context(|| {
                format!(
                    "copy verifier snapshot file {} to {}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
            fs::set_permissions(&destination_path, metadata.permissions()).with_context(|| {
                format!(
                    "preserve verifier snapshot file permissions {}",
                    destination_path.display()
                )
            })?;
        } else {
            return Err(anyhow!(
                "verifier-only workspace snapshot encountered unsupported entry {}",
                source_path.display()
            ));
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn snapshot_entry_is_link(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
pub(crate) fn snapshot_entry_is_link(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn validated_snapshot_symlink_target(root: &Path, source: &Path) -> Result<PathBuf> {
    let target = fs::read_link(source)
        .with_context(|| format!("read verifier snapshot symlink {}", source.display()))?;
    if target.is_absolute()
        || target
            .components()
            .any(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
    {
        return Err(anyhow!(
            "verifier snapshot symlink {} has an absolute target",
            source.display()
        ));
    }
    let resolved = source
        .parent()
        .context("verifier snapshot symlink has no parent")?
        .join(&target);
    let canonical = fs::canonicalize(&resolved).with_context(|| {
        format!(
            "canonicalize verifier snapshot symlink target {}",
            source.display()
        )
    })?;
    if !canonical.starts_with(root) || is_git_control_path(root, &canonical) {
        return Err(anyhow!(
            "verifier snapshot symlink {} escapes the preserved workspace",
            source.display()
        ));
    }
    Ok(target)
}

#[cfg(unix)]
fn copy_snapshot_symlink(root: &Path, source: &Path, destination: &Path) -> Result<()> {
    let target = validated_snapshot_symlink_target(root, source)?;
    std::os::unix::fs::symlink(&target, destination).with_context(|| {
        format!(
            "copy verifier snapshot symlink {} to {}",
            source.display(),
            destination.display()
        )
    })
}

#[cfg(windows)]
fn copy_snapshot_symlink(root: &Path, source: &Path, destination: &Path) -> Result<()> {
    let target = validated_snapshot_symlink_target(root, source)?;
    let target_is_directory = fs::metadata(source)
        .with_context(|| format!("inspect verifier snapshot symlink {}", source.display()))?
        .is_dir();
    if target_is_directory {
        std::os::windows::fs::symlink_dir(&target, destination)
    } else {
        std::os::windows::fs::symlink_file(&target, destination)
    }
    .with_context(|| {
        format!(
            "copy verifier snapshot symlink {} to {}",
            source.display(),
            destination.display()
        )
    })
}

fn find_workspace_file_by_digest(
    root: &Path,
    file_name: &str,
    expected_sha256: &str,
    expected_bytes: usize,
) -> Result<PathBuf> {
    if file_name.is_empty()
        || file_name.len() > 240
        || file_name.contains(['/', '\\'])
        || expected_sha256.len() != 64
        || !expected_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(anyhow!("legacy workspace artifact identity is invalid"));
    }
    for (_, path) in fingerprint_entries(root)? {
        if path.file_name().and_then(|name| name.to_str()) != Some(file_name) {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("inspect legacy workspace artifact {}", path.display()))?;
        if snapshot_entry_is_link(&metadata)
            || !metadata.is_file()
            || metadata.len() != expected_bytes as u64
        {
            continue;
        }
        let mut file = fs::File::open(&path)
            .with_context(|| format!("open legacy workspace artifact {}", path.display()))?;
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .with_context(|| format!("read legacy workspace artifact {}", path.display()))?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
        if hex::encode(digest.finalize()) == expected_sha256 {
            return Ok(path);
        }
    }
    Err(anyhow!(
        "legacy workspace artifact {file_name} did not match its persisted digest"
    ))
}

fn fingerprint_workspace(root: &Path) -> Result<String> {
    fingerprint_workspace_with_limits(root, None, None)
}

fn fingerprint_workspace_with_limits(
    root: &Path,
    max_bytes: Option<u64>,
    deadline: Option<std::time::Instant>,
) -> Result<String> {
    let root = fs::canonicalize(root).context("resolve workspace fingerprint root")?;
    let mut digest = Sha256::new();
    let mut declared_bytes = 0_u64;
    let mut read_bytes = 0_u64;
    // Root permissions intentionally differ for private snapshots. Bind every copied entry.
    for (relative, path) in fingerprint_entries(&root)? {
        ensure_checkpoint_deadline(deadline)?;
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("inspect workspace fingerprint path {}", path.display()))?;
        if snapshot_entry_is_link(&metadata) {
            let target = validated_snapshot_symlink_target(&root, &path)?;
            digest.update(b"L\0");
            digest.update(relative.as_bytes());
            digest.update(b"\0");
            digest.update(target.as_os_str().as_encoded_bytes());
            digest.update(b"\0");
        } else if metadata.is_dir() {
            digest.update(b"D\0");
            digest.update(relative.as_bytes());
            digest.update(b"\0");
            digest.update(fingerprint_mode(&metadata).to_le_bytes());
        } else if metadata.is_file() {
            declared_bytes = declared_bytes
                .checked_add(metadata.len())
                .context("workspace fingerprint byte count overflow")?;
            if max_bytes.is_some_and(|maximum| declared_bytes > maximum) {
                return Err(anyhow!("workspace checkpoint exceeds its byte limit"));
            }
            digest.update(b"F\0");
            digest.update(relative.as_bytes());
            digest.update(b"\0");
            digest.update(fingerprint_mode(&metadata).to_le_bytes());
            digest.update(metadata.len().to_le_bytes());
            let mut file = fs::File::open(&path)
                .with_context(|| format!("open workspace fingerprint path {}", path.display()))?;
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                ensure_checkpoint_deadline(deadline)?;
                let read = file.read(&mut buffer).with_context(|| {
                    format!("read workspace fingerprint path {}", path.display())
                })?;
                if read == 0 {
                    break;
                }
                read_bytes = read_bytes
                    .checked_add(read as u64)
                    .context("workspace fingerprint read count overflow")?;
                if max_bytes.is_some_and(|maximum| read_bytes > maximum) {
                    return Err(anyhow!("workspace checkpoint exceeds its byte limit"));
                }
                digest.update(&buffer[..read]);
            }
            digest.update(b"\0");
        } else {
            return Err(anyhow!(
                "workspace fingerprint encountered unsupported entry {}",
                path.display()
            ));
        }
    }
    Ok(hex::encode(digest.finalize()))
}

fn ensure_checkpoint_deadline(deadline: Option<std::time::Instant>) -> Result<()> {
    if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
        return Err(anyhow!("workspace checkpoint exceeded its read deadline"));
    }
    Ok(())
}

fn fingerprint_mode(metadata: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        // File type is encoded separately. Include permissions, including executable/special bits,
        // regardless of Git's core.filemode setting; inode, ownership and timestamps are not copied.
        metadata.permissions().mode() & 0o7777
    }
    #[cfg(not(unix))]
    {
        // Windows has no physical POSIX executable bit. Only bind its portable read-only flag,
        // not archive/creation attributes which can legitimately change when a snapshot is copied.
        u32::from(metadata.permissions().readonly())
    }
}

fn fingerprint_entries(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    const MAX_ENTRIES: usize = 100_000;
    let mut paths = Vec::new();
    collect_fingerprint_entries(root, root, &mut paths, MAX_ENTRIES)?;
    let mut entries = paths
        .into_iter()
        .map(|path| Ok((portable_relative(root, &path)?, path)))
        .collect::<Result<Vec<_>>>()?;
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(entries)
}

fn collect_fingerprint_entries(
    root: &Path,
    directory: &Path,
    entries: &mut Vec<PathBuf>,
    max_entries: usize,
) -> Result<()> {
    for entry in fs::read_dir(directory).with_context(|| {
        format!(
            "read workspace fingerprint directory {}",
            directory.display()
        )
    })? {
        let entry = entry?;
        let path = entry.path();
        if is_git_control_path(root, &path) {
            continue;
        }
        entries.push(path.clone());
        if entries.len() > max_entries {
            return Err(anyhow!(
                "workspace fingerprint exceeds {max_entries} entries"
            ));
        }
        let metadata = fs::symlink_metadata(&path)?;
        // A Windows junction may look like a directory. Never enumerate through any reparse point.
        if metadata.is_dir() && !snapshot_entry_is_link(&metadata) {
            collect_fingerprint_entries(root, &path, entries, max_entries)?;
        }
    }
    Ok(())
}

fn is_git_control_path(root: &Path, path: &Path) -> bool {
    let Some(Component::Normal(first)) = path
        .strip_prefix(root)
        .ok()
        .and_then(|relative| relative.components().next())
    else {
        return false;
    };
    #[cfg(windows)]
    {
        first
            .to_str()
            .is_some_and(|name| name.eq_ignore_ascii_case(".git"))
    }
    #[cfg(not(windows))]
    {
        first == ".git"
    }
}

fn portable_relative(root: &Path, path: &Path) -> Result<String> {
    Ok(path
        .strip_prefix(root)
        .context("workspace fingerprint path escapes its root")?
        .components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .context("workspace fingerprint path is not UTF-8"),
            _ => Err(anyhow!("workspace fingerprint path is not relative")),
        })
        .collect::<Result<Vec<_>>>()?
        .join("/"))
}

fn parse_github_repository_identity(remote: &str) -> Option<String> {
    let remote = remote.trim();
    let path = if let Some(path) = remote.strip_prefix("git@github.com:") {
        path.to_owned()
    } else {
        let url = Url::parse(remote).ok()?;
        if !url
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case("github.com"))
        {
            return None;
        }
        url.path().trim_start_matches('/').to_owned()
    };
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    let mut parts = path.split('/');
    let owner = parts.next()?;
    let repository = parts.next()?;
    if parts.next().is_some()
        || owner.is_empty()
        || repository.is_empty()
        || !owner.chars().chain(repository.chars()).all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        return None;
    }
    Some(format!("{owner}/{repository}"))
}

fn local_repository_identity(repository: &Path) -> String {
    let canonical = std::fs::canonicalize(repository).unwrap_or_else(|_| repository.to_path_buf());
    let name = canonical
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("repository")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let name = name.trim_matches('-');
    let name = if name.is_empty() { "repository" } else { name };
    let digest = hex::encode(Sha256::digest(canonical.to_string_lossy().as_bytes()));
    format!("local/{name}-{}", &digest[..12])
}

fn branch_name(task_id: Uuid, workspace_run_id: Uuid) -> String {
    format!(
        "crony/task-{}/run-{}",
        task_id.simple(),
        workspace_run_id.simple()
    )
}

fn validate_ref(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 256
        || value.starts_with('-')
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._/@{}^~-".contains(character))
    {
        return Err(anyhow!("invalid Git base ref"));
    }
    Ok(())
}

fn validate_commit(value: &str) -> Result<()> {
    if !matches!(value.len(), 40 | 64) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(anyhow!(
            "Git base commit must be a full hexadecimal object ID"
        ));
    }
    Ok(())
}

fn ensure_descendant(root: &Path, candidate: &Path) -> Result<()> {
    if candidate == root || !candidate.starts_with(root) {
        return Err(anyhow!(
            "resolved path {} escapes managed workspace root {}",
            candidate.display(),
            root.display()
        ));
    }
    Ok(())
}

async fn canonicalize_path(path: &Path) -> std::io::Result<PathBuf> {
    tokio::fs::canonicalize(path).await.map(normalize_path)
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

fn preserved(detail: String) -> WorkspaceCleanup {
    WorkspaceCleanup {
        disposition: WorkspaceDisposition::Preserved,
        detail,
        dirty: None,
        commits_ahead: None,
        branch_deleted: false,
    }
}

async fn run_git(cwd: &Path, args: &[OsString]) -> Result<Output> {
    let mut command = Command::new("git");
    // Git discovers worktree administrative paths before repository-local
    // configuration is available. Apply native Windows long-path support to
    // this invocation, without modifying user or source-repository settings.
    #[cfg(windows)]
    command.args(["-c", "core.longpaths=true"]);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    tokio::time::timeout(GIT_TIMEOUT, command.output())
        .await
        .with_context(|| format!("git {} timed out in {}", display_args(args), cwd.display()))?
        .with_context(|| format!("spawn git {} in {}", display_args(args), cwd.display()))
}

fn git_error(args: &[OsString], output: &Output) -> anyhow::Error {
    anyhow!(
        "git {} failed with {}: {}",
        display_args(args),
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

fn display_args(args: &[OsString]) -> String {
    args.iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    fn command(cwd: &Path, args: &[&OsStr]) {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn fixture() -> (PathBuf, PathBuf, PathBuf) {
        let root = std::env::temp_dir()
            .join("crony workspace tests")
            .join(Uuid::new_v4().to_string());
        let repository = root.join("source repository");
        let managed = root.join("runner workspace with spaces");
        std::fs::create_dir_all(&repository).expect("create source");
        command(
            &repository,
            &[OsStr::new("init"), OsStr::new("-b"), OsStr::new("main")],
        );
        std::fs::write(repository.join("README.md"), "# source\n").expect("write readme");
        std::fs::write(repository.join(".gitignore"), "*.log\n").expect("write gitignore");
        command(
            &repository,
            &[
                OsStr::new("add"),
                OsStr::new("README.md"),
                OsStr::new(".gitignore"),
            ],
        );
        command(
            &repository,
            &[
                OsStr::new("-c"),
                OsStr::new("user.name=ECorp Test"),
                OsStr::new("-c"),
                OsStr::new("user.email=crony@example.invalid"),
                OsStr::new("commit"),
                OsStr::new("-m"),
                OsStr::new("init"),
            ],
        );
        command(
            &repository,
            &[
                OsStr::new("remote"),
                OsStr::new("add"),
                OsStr::new("origin"),
                OsStr::new("https://github.com/shyamsridhar123/ecorp.git"),
            ],
        );
        (root, repository, managed)
    }

    fn cleanup_fixture(root: &Path, repository: &Path) {
        let output = std::process::Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(repository)
            .output()
            .expect("list worktrees");
        let text = String::from_utf8_lossy(&output.stdout);
        let canonical_repository =
            normalize_path(std::fs::canonicalize(repository).expect("resolve source repository"));
        for line in text
            .lines()
            .filter_map(|line| line.strip_prefix("worktree "))
        {
            let path = PathBuf::from(line);
            let canonical_path = std::fs::canonicalize(&path)
                .map(normalize_path)
                .unwrap_or_else(|_| path.clone());
            if canonical_path != canonical_repository {
                let _ = std::process::Command::new("git")
                    .args(["worktree", "remove", "--force"])
                    .arg(&path)
                    .current_dir(repository)
                    .status();
            }
        }
        let _ = std::fs::remove_dir_all(root);
    }

    fn physical_fixture() -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir()
            .join("ecorp physical workspace tests")
            .join(Uuid::new_v4().to_string());
        let source = root.join("source");
        fs::create_dir_all(&source).expect("create physical source");
        (root, source)
    }

    fn cleanup_physical_fixture(root: &Path) {
        let canonical = fs::canonicalize(root).expect("resolve physical fixture");
        let temporary = fs::canonicalize(std::env::temp_dir()).expect("resolve temporary root");
        assert!(canonical != temporary && canonical.starts_with(&temporary));
        prepare_snapshot_cleanup(&canonical).expect("make owned fixture removable");
        fs::remove_dir_all(&canonical).expect("remove owned physical fixture");
    }

    #[tokio::test]
    async fn provisions_distinct_worktrees_without_touching_source_checkout() {
        let (root, repository, managed) = fixture();
        let source_head = std::fs::read(repository.join("README.md")).expect("source readme");
        let manager =
            WorkspaceManager::initialize(managed.clone(), repository.clone(), "HEAD".to_owned())
                .await
                .expect("initialize manager");
        let task = Uuid::new_v4();
        let first_owner = Uuid::new_v4();
        let first = manager
            .prepare(task, first_owner, None, None)
            .await
            .expect("first worktree");
        let second = manager
            .prepare(task, Uuid::new_v4(), None, None)
            .await
            .expect("second worktree");

        assert_ne!(first.path, second.path);
        assert_ne!(first.branch, second.branch);
        assert!(first.path.starts_with(manager.root()));
        assert!(second.path.starts_with(manager.root()));
        assert!(
            first
                .path
                .to_string_lossy()
                .contains("runner workspace with spaces")
        );
        std::fs::write(first.path.join("first.txt"), "first\n").expect("first change");
        std::fs::write(second.path.join("second.txt"), "second\n").expect("second change");
        let resumed = manager
            .prepare(task, first_owner, None, None)
            .await
            .expect("reuse existing worktree");
        assert_eq!(resumed.path, first.path);
        assert_eq!(resumed.branch, first.branch);
        assert_eq!(
            std::fs::read_to_string(resumed.path.join("first.txt")).expect("preserved work"),
            "first\n"
        );
        assert!(!repository.join("first.txt").exists());
        assert!(!repository.join("second.txt").exists());
        assert!(!first.path.join("second.txt").exists());
        assert_eq!(
            std::fs::read(repository.join("README.md")).expect("source readme after"),
            source_head
        );
        let source_status = std::process::Command::new("git")
            .args(["status", "--porcelain=v1"])
            .current_dir(&repository)
            .output()
            .expect("source status");
        assert!(source_status.stdout.is_empty());

        let first_cleanup = manager.finalize(&first).await.expect("first cleanup");
        let second_cleanup = manager.finalize(&second).await.expect("second cleanup");
        assert_eq!(first_cleanup.disposition, WorkspaceDisposition::Preserved);
        assert_eq!(second_cleanup.disposition, WorkspaceDisposition::Preserved);
        assert!(first.path.exists());
        assert!(second.path.exists());
        cleanup_fixture(&root, &repository);
    }

    #[tokio::test]
    async fn fingerprint_covers_tracked_untracked_and_ignored_bytes() {
        let (root, repository, managed) = fixture();
        let manager = WorkspaceManager::initialize(managed, repository.clone(), "HEAD".to_owned())
            .await
            .expect("initialize manager");
        let workspace = manager
            .prepare(Uuid::new_v4(), Uuid::new_v4(), None, None)
            .await
            .expect("prepare worktree");
        let initial = manager
            .fingerprint(&workspace)
            .await
            .expect("initial fingerprint");
        std::fs::write(workspace.path.join("untracked.txt"), "one\n")
            .expect("write untracked file");
        let with_untracked = manager
            .fingerprint(&workspace)
            .await
            .expect("untracked fingerprint");
        assert_ne!(initial, with_untracked);
        std::fs::write(workspace.path.join("valuable.log"), "ignored\n")
            .expect("write ignored file");
        let with_ignored = manager
            .fingerprint(&workspace)
            .await
            .expect("ignored fingerprint");
        assert_ne!(with_untracked, with_ignored);
        std::fs::write(workspace.path.join("README.md"), "# changed\n")
            .expect("change tracked file");
        let with_tracked = manager
            .fingerprint(&workspace)
            .await
            .expect("tracked fingerprint");
        assert_ne!(with_ignored, with_tracked);
        assert_eq!(with_tracked.len(), 64);
        cleanup_fixture(&root, &repository);
    }

    #[test]
    fn fingerprint_is_reproducible_and_excludes_only_root_git_control() {
        let (root, source) = physical_fixture();
        let other = root.join("other");
        let files = [
            ("z.txt", "last\n"),
            ("nested/a.txt", "first\n"),
            ("ignored.log", "ignored bytes\n"),
            ("nested/.git/marker", "nested control is source\n"),
        ];
        for (path, bytes) in files {
            let path = source.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        for (path, bytes) in files.into_iter().rev() {
            let path = other.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        fs::write(source.join(".git"), "linked worktree control\n").unwrap();
        fs::create_dir(other.join(".git")).unwrap();
        fs::write(other.join(".git/HEAD"), "different Git control\n").unwrap();

        let expected = fingerprint_workspace(&source).unwrap();
        assert_eq!(fingerprint_workspace(&source).unwrap(), expected);
        assert_eq!(fingerprint_workspace(&other).unwrap(), expected);
        fs::write(other.join("z.txt"), "last\n").unwrap();
        assert_eq!(fingerprint_workspace(&other).unwrap(), expected);
        fs::write(other.join("nested/.git/marker"), "changed nested source\n").unwrap();
        assert_ne!(fingerprint_workspace(&other).unwrap(), expected);
        assert!(portable_relative(&source, &root.join("outside")).is_err());
        assert!(portable_relative(&source, &source.join("../outside")).is_err());
        #[cfg(windows)]
        assert!(is_git_control_path(&source, &source.join(".GiT/HEAD")));
        #[cfg(unix)]
        assert!(!is_git_control_path(&source, &source.join(".GiT/HEAD")));
        cleanup_physical_fixture(&root);
    }

    #[cfg(unix)]
    #[test]
    fn fingerprint_binds_unix_modes_even_when_git_ignores_mode_changes() {
        use std::os::unix::fs::PermissionsExt;

        let (root, repository, _) = fixture();
        command(
            &repository,
            &[
                OsStr::new("config"),
                OsStr::new("core.filemode"),
                OsStr::new("false"),
            ],
        );
        fs::write(repository.join("untracked.sh"), "untracked\n").unwrap();
        fs::write(repository.join("ignored.log"), "ignored\n").unwrap();
        let paths = ["README.md", "untracked.sh", "ignored.log"];
        for path in paths {
            fs::set_permissions(repository.join(path), fs::Permissions::from_mode(0o644)).unwrap();
        }
        let expected = fingerprint_workspace(&repository).unwrap();
        for path in paths {
            let path = repository.join(path);
            let bytes = fs::read(&path).unwrap();
            for mode in [0o755, 0o744, 0o640, 0o645, 0o4755] {
                fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
                assert_eq!(fingerprint_mode(&fs::metadata(&path).unwrap()), mode);
                assert_eq!(fs::read(&path).unwrap(), bytes);
                assert_ne!(fingerprint_workspace(&repository).unwrap(), expected);
            }
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            assert_eq!(fingerprint_workspace(&repository).unwrap(), expected);
        }
        fs::set_permissions(
            repository.join("README.md"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        let diff = std::process::Command::new("git")
            .args(["diff", "--name-only"])
            .current_dir(&repository)
            .output()
            .unwrap();
        assert!(diff.status.success());
        assert!(
            diff.stdout.is_empty(),
            "Git alone hides this executable-mode drift"
        );
        assert_ne!(fingerprint_workspace(&repository).unwrap(), expected);
        cleanup_fixture(&root, &repository);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn snapshots_preserve_unix_modes_without_changing_source_during_cleanup() {
        use std::os::unix::fs::PermissionsExt;

        let (root, source) = physical_fixture();
        fs::create_dir(source.join("nested")).unwrap();
        fs::write(source.join("nested/script.sh"), "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(
            source.join("nested/script.sh"),
            fs::Permissions::from_mode(0o751),
        )
        .unwrap();
        fs::set_permissions(source.join("nested"), fs::Permissions::from_mode(0o550)).unwrap();
        let expected = fingerprint_workspace(&source).unwrap();
        let mut baseline = verification_snapshot(&source, Uuid::new_v4())
            .await
            .unwrap();
        let mut check = verification_snapshot(baseline.path(), Uuid::new_v4())
            .await
            .unwrap();
        assert_eq!(fingerprint_workspace(baseline.path()).unwrap(), expected);
        assert_eq!(fingerprint_workspace(check.path()).unwrap(), expected);
        assert_eq!(
            fingerprint_mode(&fs::metadata(baseline.path()).unwrap()),
            0o700
        );
        assert_eq!(
            fingerprint_mode(&fs::metadata(check.path().join("nested")).unwrap()),
            0o550
        );
        fs::set_permissions(
            check.path().join("nested/script.sh"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert_ne!(fingerprint_workspace(check.path()).unwrap(), expected);
        assert_eq!(fingerprint_workspace(baseline.path()).unwrap(), expected);
        check
            .cleanup()
            .await
            .expect("remove copy with restrictive directory mode");
        baseline.cleanup().await.expect("remove sealed baseline");
        assert_eq!(fingerprint_workspace(&source).unwrap(), expected);
        fs::set_permissions(source.join("nested"), fs::Permissions::from_mode(0o750)).unwrap();
        assert_ne!(fingerprint_workspace(&source).unwrap(), expected);
        cleanup_physical_fixture(&root);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn fingerprint_binds_windows_readonly_modes_and_snapshots_preserve_them() {
        let (root, source) = physical_fixture();
        fs::create_dir(source.join("nested")).unwrap();
        fs::write(source.join("nested/tool.exe"), "same bytes\n").unwrap();
        fs::write(source.join("plain.txt"), "same bytes\n").unwrap();
        assert_eq!(
            fingerprint_mode(&fs::metadata(source.join("nested/tool.exe")).unwrap()),
            fingerprint_mode(&fs::metadata(source.join("plain.txt")).unwrap())
        );
        let writable = fingerprint_workspace(&source).unwrap();
        for path in [source.join("nested/tool.exe"), source.join("nested")] {
            let mut permissions = fs::metadata(&path).unwrap().permissions();
            permissions.set_readonly(true);
            fs::set_permissions(path, permissions).unwrap();
        }
        let expected = fingerprint_workspace(&source).unwrap();
        assert_ne!(expected, writable);
        let mut snapshot = verification_snapshot(&source, Uuid::new_v4())
            .await
            .unwrap();
        assert_eq!(fingerprint_workspace(snapshot.path()).unwrap(), expected);
        assert_eq!(
            fingerprint_mode(&fs::metadata(snapshot.path().join("nested/tool.exe")).unwrap()),
            1
        );
        snapshot
            .cleanup()
            .await
            .expect("remove read-only snapshot entries");
        assert!(
            fs::metadata(source.join("nested"))
                .unwrap()
                .permissions()
                .readonly()
        );
        assert!(
            fs::metadata(source.join("nested/tool.exe"))
                .unwrap()
                .permissions()
                .readonly()
        );
        assert_eq!(fingerprint_workspace(&source).unwrap(), expected);
        cleanup_physical_fixture(&root);
    }

    #[test]
    fn fingerprint_entry_and_snapshot_byte_limits_fail_without_source_mutation() {
        let (root, source) = physical_fixture();
        fs::write(source.join("a.txt"), "1234").unwrap();
        fs::write(source.join("b.txt"), "5678").unwrap();
        let expected = fingerprint_workspace(&source).unwrap();
        let error = collect_fingerprint_entries(&source, &source, &mut Vec::new(), 1)
            .expect_err("entry bound");
        assert!(error.to_string().contains("exceeds 1 entries"));
        let destination = root.join("limited");
        fs::create_dir(&destination).unwrap();
        let mut entries = 0;
        let mut bytes = 0;
        let error = copy_snapshot_directory(
            &source,
            &source,
            &destination,
            &mut entries,
            &mut bytes,
            10,
            3,
        )
        .expect_err("byte bound");
        assert!(error.to_string().contains("exceeds 3 bytes"));
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
        assert_eq!(fingerprint_workspace(&source).unwrap(), expected);
        cleanup_physical_fixture(&root);
    }

    // Linux permits the raw filename needed to reach fingerprint rejection.
    // macOS rejects its creation instead; that distinct boundary is tested below.
    #[cfg(target_os = "linux")]
    #[test]
    fn fingerprint_rejects_lossy_non_utf8_path_names() {
        use std::os::unix::ffi::OsStringExt;

        let (root, source) = physical_fixture();
        fs::write(source.join(OsString::from_vec(vec![b'a', 0xff])), "bytes\n").unwrap();
        assert!(
            fingerprint_workspace(&source)
                .expect_err("non-UTF-8 paths must not collide through lossy encoding")
                .to_string()
                .contains("not UTF-8")
        );
        cleanup_physical_fixture(&root);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_rejects_non_utf8_fixture_names_before_fingerprinting() {
        use std::os::unix::ffi::OsStringExt;

        let (root, source) = physical_fixture();
        let error = fs::write(source.join(OsString::from_vec(vec![b'a', 0xff])), "bytes\n")
            .expect_err("macOS fixture filesystem must reject the non-UTF-8 filename");
        // Darwin EILSEQ, observed on the macOS Actions filesystem. An unrelated
        // I/O failure is not successful coverage, and no fingerprint branch ran.
        assert_eq!(error.raw_os_error(), Some(92), "expected EILSEQ: {error}");
        assert_eq!(fs::read_dir(&source).unwrap().count(), 0);
        cleanup_physical_fixture(&root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fingerprint_and_snapshots_bind_contained_links_and_reject_git_control_links() {
        use std::os::unix::fs::symlink;

        let (root, source) = physical_fixture();
        fs::write(source.join("one.txt"), "same\n").unwrap();
        fs::write(source.join("two.txt"), "same\n").unwrap();
        fs::write(source.join(".git"), "private Git control\n").unwrap();
        symlink("one.txt", source.join("alias.txt")).unwrap();
        symlink(".", source.join("loop")).unwrap();
        let expected = fingerprint_workspace(&source).unwrap();
        assert_eq!(fingerprint_entries(&source).unwrap().len(), 4);
        let mut snapshot = verification_snapshot(&source, Uuid::new_v4())
            .await
            .unwrap();
        assert_eq!(fingerprint_workspace(snapshot.path()).unwrap(), expected);
        fs::write(snapshot.path().join("alias.txt"), "check side effect\n").unwrap();
        snapshot.cleanup().await.unwrap();
        assert_eq!(
            fs::read_to_string(source.join("one.txt")).unwrap(),
            "same\n"
        );
        assert_eq!(fingerprint_workspace(&source).unwrap(), expected);
        fs::remove_file(source.join("alias.txt")).unwrap();
        symlink("two.txt", source.join("alias.txt")).unwrap();
        assert_ne!(fingerprint_workspace(&source).unwrap(), expected);
        fs::remove_file(source.join("alias.txt")).unwrap();
        symlink(".git", source.join("alias.txt")).unwrap();
        assert!(fingerprint_workspace(&source).is_err());
        assert!(
            verification_snapshot(&source, Uuid::new_v4())
                .await
                .is_err()
        );
        cleanup_physical_fixture(&root);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn fingerprint_and_legacy_search_never_traverse_windows_junctions() {
        let (root, source) = physical_fixture();
        let outside = root.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("evidence.txt"), "outside bytes\n").unwrap();
        let junction = source.join("junction");
        let output = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&outside)
            .output()
            .expect("create owned junction fixture");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(snapshot_entry_is_link(
            &fs::symlink_metadata(&junction).unwrap()
        ));
        let entries = fingerprint_entries(&source).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "junction");
        assert!(fingerprint_workspace(&source).is_err());
        assert!(
            verification_snapshot(&source, Uuid::new_v4())
                .await
                .is_err()
        );
        assert!(
            find_file_by_digest(
                &source,
                "evidence.txt",
                &hex::encode(Sha256::digest(b"outside bytes\n")),
                b"outside bytes\n".len(),
            )
            .await
            .is_err()
        );
        assert_eq!(
            fs::read_to_string(outside.join("evidence.txt")).unwrap(),
            "outside bytes\n"
        );
        cleanup_physical_fixture(&root);
    }

    #[tokio::test]
    async fn legacy_artifact_search_is_bounded_by_name_size_and_digest() {
        let (root, repository, managed) = fixture();
        let manager = WorkspaceManager::initialize(managed, repository.clone(), "HEAD".to_owned())
            .await
            .expect("initialize manager");
        let workspace = manager
            .prepare(Uuid::new_v4(), Uuid::new_v4(), None, None)
            .await
            .expect("prepare worktree");
        std::fs::create_dir_all(workspace.path.join("nested")).expect("create nested directory");
        let bytes = b"legacy artifact\n";
        let artifact = workspace.path.join("nested").join("provider.json");
        std::fs::write(&artifact, bytes).expect("write legacy artifact");
        let sha256 = hex::encode(Sha256::digest(bytes));
        let found = find_file_by_digest(&workspace.path, "provider.json", &sha256, bytes.len())
            .await
            .expect("find legacy artifact");
        assert_eq!(found, artifact);
        assert!(
            find_file_by_digest(
                &workspace.path,
                "provider.json",
                &"0".repeat(64),
                bytes.len(),
            )
            .await
            .expect_err("wrong digest must fail")
            .to_string()
            .contains("did not match")
        );
        cleanup_fixture(&root, &repository);
    }

    #[tokio::test]
    async fn verifier_snapshot_is_physical_isolated_and_excludes_git_control() {
        let (root, repository, managed) = fixture();
        let manager = WorkspaceManager::initialize(managed, repository.clone(), "HEAD".to_owned())
            .await
            .expect("initialize manager");
        let workspace = manager
            .prepare(Uuid::new_v4(), Uuid::new_v4(), None, None)
            .await
            .expect("prepare worktree");
        std::fs::write(workspace.path.join("untracked.txt"), "source\n")
            .expect("write untracked source");
        std::fs::write(workspace.path.join("valuable.log"), "ignored\n")
            .expect("write ignored source");
        let mut snapshot = verification_snapshot(&workspace.path, Uuid::new_v4())
            .await
            .expect("create verifier snapshot");
        let snapshot_path = snapshot.path().to_owned();
        assert!(!snapshot_path.join(".git").exists());
        assert_eq!(
            std::fs::read_to_string(snapshot_path.join("untracked.txt"))
                .expect("read snapshot untracked source"),
            "source\n"
        );
        assert_eq!(
            std::fs::read_to_string(snapshot_path.join("valuable.log"))
                .expect("read snapshot ignored source"),
            "ignored\n"
        );
        std::fs::write(
            snapshot_path.join("untracked.txt"),
            "verifier side effect\n",
        )
        .expect("modify verifier snapshot");
        assert_eq!(
            std::fs::read_to_string(workspace.path.join("untracked.txt"))
                .expect("read unchanged source"),
            "source\n"
        );
        snapshot.cleanup().await.expect("clean verifier snapshot");
        assert!(!snapshot_path.exists());
        cleanup_fixture(&root, &repository);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn verifier_snapshot_rejects_a_symlink_that_escapes_the_workspace() {
        use std::os::unix::fs::symlink;

        let (root, repository, managed) = fixture();
        let manager = WorkspaceManager::initialize(managed, repository.clone(), "HEAD".to_owned())
            .await
            .expect("initialize manager");
        let workspace = manager
            .prepare(Uuid::new_v4(), Uuid::new_v4(), None, None)
            .await
            .expect("prepare worktree");
        let outside = workspace
            .path
            .parent()
            .expect("worktree parent")
            .join("outside.txt");
        std::fs::write(&outside, "outside\n").expect("write outside file");
        symlink("../outside.txt", workspace.path.join("escape.txt"))
            .expect("create escaping symlink");
        assert!(manager.fingerprint(&workspace).await.is_err());
        let error = verification_snapshot(&workspace.path, Uuid::new_v4())
            .await
            .expect_err("escaping symlink must fail");
        assert!(
            error
                .to_string()
                .contains("escapes the preserved workspace")
        );
        std::fs::remove_file(outside).expect("remove outside file");
        cleanup_fixture(&root, &repository);
    }

    #[tokio::test]
    async fn removes_only_clean_integrated_worktrees() {
        let (root, repository, managed) = fixture();
        let manager = WorkspaceManager::initialize(managed, repository.clone(), "main".to_owned())
            .await
            .expect("initialize manager");
        let clean = manager
            .prepare(Uuid::new_v4(), Uuid::new_v4(), None, None)
            .await
            .expect("clean worktree");
        let clean_branch = clean.branch.clone();
        let cleanup = manager.finalize(&clean).await.expect("clean cleanup");
        assert_eq!(cleanup.disposition, WorkspaceDisposition::Removed);
        assert!(cleanup.branch_deleted);
        assert!(!clean.path.exists());
        let branch = std::process::Command::new("git")
            .args(["show-ref", "--verify", "--quiet"])
            .arg(format!("refs/heads/{clean_branch}"))
            .current_dir(&repository)
            .status()
            .expect("check branch");
        assert!(!branch.success());

        let ignored = manager
            .prepare(Uuid::new_v4(), Uuid::new_v4(), None, None)
            .await
            .expect("ignored-file worktree");
        std::fs::write(ignored.path.join("valuable.log"), "ignored but valuable\n")
            .expect("write ignored file");
        let ignored_cleanup = manager
            .finalize(&ignored)
            .await
            .expect("ignored-file cleanup");
        assert_eq!(ignored_cleanup.disposition, WorkspaceDisposition::Preserved);
        assert!(ignored_cleanup.detail.contains("ignored files"));
        assert!(ignored.path.join("valuable.log").exists());

        let committed = manager
            .prepare(Uuid::new_v4(), Uuid::new_v4(), None, None)
            .await
            .expect("committed worktree");
        std::fs::write(committed.path.join("work.txt"), "valuable\n").expect("write work");
        command(
            &committed.path,
            &[OsStr::new("add"), OsStr::new("work.txt")],
        );
        command(
            &committed.path,
            &[
                OsStr::new("-c"),
                OsStr::new("user.name=ECorp Test"),
                OsStr::new("-c"),
                OsStr::new("user.email=crony@example.invalid"),
                OsStr::new("commit"),
                OsStr::new("-m"),
                OsStr::new("valuable work"),
            ],
        );
        let preserved = manager
            .finalize(&committed)
            .await
            .expect("preserve committed work");
        assert_eq!(preserved.disposition, WorkspaceDisposition::Preserved);
        assert_eq!(preserved.commits_ahead, Some(1));
        assert!(committed.path.exists());

        command(
            &repository,
            &[
                OsStr::new("merge"),
                OsStr::new("--ff-only"),
                OsStr::new(&committed.branch),
            ],
        );
        let reclaimed = manager
            .finalize(&committed)
            .await
            .expect("reclaim integrated work");
        assert_eq!(reclaimed.disposition, WorkspaceDisposition::Removed);
        assert!(!committed.path.exists());

        let uncertain = manager
            .prepare(Uuid::new_v4(), Uuid::new_v4(), None, None)
            .await
            .expect("uncertain worktree");
        command(
            &uncertain.path,
            &[OsStr::new("switch"), OsStr::new("--detach")],
        );
        let kept = manager
            .finalize(&uncertain)
            .await
            .expect("uncertain cleanup");
        assert_eq!(kept.disposition, WorkspaceDisposition::Preserved);
        assert!(kept.detail.contains("identity could not be verified"));
        assert!(uncertain.path.exists());
        cleanup_fixture(&root, &repository);
    }

    #[tokio::test]
    async fn occupied_or_tampered_target_never_falls_back_to_the_source_checkout() {
        let (root, repository, managed) = fixture();
        let manager = WorkspaceManager::initialize(managed, repository.clone(), "HEAD".to_owned())
            .await
            .expect("initialize manager");
        let task = Uuid::new_v4();
        let run = Uuid::new_v4();
        let occupied = manager
            .worktrees_root
            .join(task.simple().to_string())
            .join(run.simple().to_string());
        std::fs::create_dir_all(&occupied).expect("create occupied path");
        std::fs::write(occupied.join("not-a-worktree.txt"), "sentinel\n").expect("write sentinel");
        let error = manager
            .prepare(task, run, None, None)
            .await
            .expect_err("occupied non-worktree must fail");
        assert!(error.to_string().contains("inspect existing worktree"));
        assert_eq!(
            std::fs::read_to_string(occupied.join("not-a-worktree.txt"))
                .expect("sentinel preserved"),
            "sentinel\n"
        );
        assert!(!repository.join("not-a-worktree.txt").exists());
        cleanup_fixture(&root, &repository);
    }

    #[tokio::test]
    async fn source_identity_requires_the_exact_resolved_commit() {
        let (root, repository, managed) = fixture();
        let manager = WorkspaceManager::initialize(managed, repository.clone(), "HEAD".to_owned())
            .await
            .expect("initialize manager");
        let commit = manager.base_commit().to_owned();
        manager
            .verify_source_identity(Some("SHYAMSRIDHAR123/ECORP"), Some("HEAD"), Some(&commit))
            .expect("matching immutable source identity");
        assert!(
            manager
                .verify_source_identity(
                    Some("shyamsridhar123/ecorp"),
                    Some("HEAD"),
                    Some("1111111111111111111111111111111111111111"),
                )
                .unwrap_err()
                .to_string()
                .contains("commit mismatch")
        );
        assert!(
            manager
                .verify_source_identity(Some("shyamsridhar123/ecorp"), Some("HEAD"), None,)
                .unwrap_err()
                .to_string()
                .contains("must include")
        );
        cleanup_fixture(&root, &repository);
    }

    #[tokio::test]
    async fn unpinned_worktrees_follow_the_live_ref_while_pinned_and_resumed_work_stays_fixed() {
        let (root, repository, managed) = fixture();
        let manager = WorkspaceManager::initialize(managed, repository.clone(), "HEAD".to_owned())
            .await
            .expect("initialize manager");
        let startup_commit = manager.base_commit().to_owned();
        let first_task = Uuid::new_v4();
        let first_run = Uuid::new_v4();
        let first = manager
            .prepare(first_task, first_run, None, None)
            .await
            .expect("first unpinned worktree");
        assert_eq!(first.base_commit, startup_commit);

        std::fs::write(repository.join("advanced.txt"), "advanced\n")
            .expect("write advanced source");
        command(
            &repository,
            &[OsStr::new("add"), OsStr::new("advanced.txt")],
        );
        command(
            &repository,
            &[
                OsStr::new("-c"),
                OsStr::new("user.name=ECorp Test"),
                OsStr::new("-c"),
                OsStr::new("user.email=crony@example.invalid"),
                OsStr::new("commit"),
                OsStr::new("-m"),
                OsStr::new("advance configured base"),
            ],
        );
        let advanced_commit = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repository)
            .output()
            .expect("resolve advanced commit");
        assert!(advanced_commit.status.success());
        let advanced_commit = String::from_utf8(advanced_commit.stdout)
            .expect("advanced commit utf8")
            .trim()
            .to_owned();

        let later_unpinned = manager
            .prepare(Uuid::new_v4(), Uuid::new_v4(), None, None)
            .await
            .expect("later unpinned worktree");
        assert_eq!(later_unpinned.base_commit, advanced_commit);
        assert!(later_unpinned.path.join("advanced.txt").exists());

        let pinned_task = Uuid::new_v4();
        let pinned_run = Uuid::new_v4();
        let pinned = manager
            .prepare(pinned_task, pinned_run, Some(&startup_commit), None)
            .await
            .expect("pinned worktree");
        assert_eq!(pinned.base_commit, startup_commit);
        assert!(!pinned.path.join("advanced.txt").exists());

        let resumed = manager
            .prepare(
                pinned_task,
                pinned_run,
                Some(&startup_commit),
                Some(&startup_commit),
            )
            .await
            .expect("resume pinned worktree");
        assert_eq!(resumed.path, pinned.path);
        assert_eq!(resumed.base_commit, startup_commit);

        let resumed_unpinned = manager
            .prepare(first_task, first_run, None, Some(&startup_commit))
            .await
            .expect("resume unpinned worktree");
        assert_eq!(resumed_unpinned.path, first.path);
        assert_eq!(resumed_unpinned.base_commit, startup_commit);
        cleanup_fixture(&root, &repository);
    }

    #[tokio::test]
    async fn initialize_pinned_retains_commit_after_ref_advances_or_disappears() {
        let (root, repository, managed) = fixture();
        let original =
            WorkspaceManager::initialize(managed.clone(), repository.clone(), "main".to_owned())
                .await
                .expect("initialize manager");
        let original_commit = original.base_commit().to_owned();
        std::fs::write(repository.join("README.md"), "# advanced source\n").unwrap();
        command(&repository, &[OsStr::new("add"), OsStr::new("README.md")]);
        command(
            &repository,
            &[
                OsStr::new("-c"),
                OsStr::new("user.name=ECorp Test"),
                OsStr::new("-c"),
                OsStr::new("user.email=crony@example.invalid"),
                OsStr::new("commit"),
                OsStr::new("-m"),
                OsStr::new("advance saved branch"),
            ],
        );
        let advanced_commit = original.resolve_base_commit().await.unwrap();
        assert_ne!(advanced_commit, original_commit);
        let worktrees_before = original
            .git_text(&repository, &["worktree", "list", "--porcelain"])
            .await
            .unwrap();

        let pinned = WorkspaceManager::initialize_pinned(
            managed.clone(),
            repository.clone(),
            "main".to_owned(),
            original_commit.clone(),
        )
        .await
        .expect("restore the original commit, not the current main tip");
        assert_eq!(pinned.base_ref(), "main");
        assert_eq!(pinned.base_commit(), original_commit);
        assert_eq!(pinned.repository_identity(), original.repository_identity());
        assert_eq!(
            pinned
                .git_text(&repository, &["worktree", "list", "--porcelain"])
                .await
                .unwrap(),
            worktrees_before
        );

        command(
            &repository,
            &[
                OsStr::new("branch"),
                OsStr::new("-m"),
                OsStr::new("advanced-main"),
            ],
        );
        let restored = WorkspaceManager::initialize_pinned(
            managed,
            repository.clone(),
            "main".to_owned(),
            original_commit.clone(),
        )
        .await
        .expect("restoring a pin does not require its historical ref to exist");
        let workspace = restored
            .prepare(
                Uuid::new_v4(),
                Uuid::new_v4(),
                Some(restored.base_commit()),
                None,
            )
            .await
            .expect("prepare on the exact restored commit");
        assert_eq!(workspace.base_ref, "main");
        assert_eq!(workspace.base_commit, original_commit);
        assert_eq!(
            restored.head_commit(&workspace).await.unwrap(),
            original_commit
        );
        assert_eq!(
            std::fs::read_to_string(workspace.path.join("README.md"))
                .unwrap()
                .replace("\r\n", "\n"),
            "# source\n"
        );
        assert_eq!(
            restored
                .git_text(&repository, &["rev-parse", "HEAD"])
                .await
                .unwrap()
                .trim(),
            advanced_commit
        );
        assert_eq!(
            std::fs::read_to_string(repository.join("README.md")).unwrap(),
            "# advanced source\n"
        );
        cleanup_fixture(&root, &repository);
    }

    #[tokio::test]
    async fn pinned_snapshots_share_identity_lock_and_preserved_dirty_worktrees() {
        let (root, repository, managed) = fixture();
        let original = WorkspaceManager::initialize(managed, repository.clone(), "main".to_owned())
            .await
            .expect("initialize manager");
        let original_commit = original.base_commit().to_owned();
        let task = Uuid::new_v4();
        let run = Uuid::new_v4();
        let workspace = original
            .prepare(task, run, Some(&original_commit), None)
            .await
            .unwrap();
        std::fs::write(workspace.path.join("README.md"), "unfinished work\n").unwrap();
        std::fs::write(workspace.path.join("untracked.txt"), "retain untracked\n").unwrap();
        std::fs::write(workspace.path.join("valuable.log"), "retain ignored\n").unwrap();
        let fingerprint = original.fingerprint(&workspace).await.unwrap();
        std::fs::write(repository.join("README.md"), "# advanced source\n").unwrap();
        command(&repository, &[OsStr::new("add"), OsStr::new("README.md")]);
        command(
            &repository,
            &[
                OsStr::new("-c"),
                OsStr::new("user.name=ECorp Test"),
                OsStr::new("-c"),
                OsStr::new("user.email=crony@example.invalid"),
                OsStr::new("commit"),
                OsStr::new("-m"),
                OsStr::new("advance saved branch"),
            ],
        );
        let advanced_commit = original.resolve_base_commit().await.unwrap();
        command(
            &repository,
            &[
                OsStr::new("remote"),
                OsStr::new("set-url"),
                OsStr::new("origin"),
                OsStr::new("https://github.com/other/repository.git"),
            ],
        );
        let worktrees_before = original
            .git_text(&repository, &["worktree", "list", "--porcelain"])
            .await
            .unwrap();
        let refreshed = original.pinned("main", &advanced_commit).await.unwrap();
        let retained = refreshed.pinned("main", &original_commit).await.unwrap();
        for snapshot in [&refreshed, &retained] {
            assert_eq!(snapshot.root(), original.root());
            assert_eq!(snapshot.repository(), original.repository());
            assert_eq!(
                snapshot.repository_identity(),
                original.repository_identity()
            );
            assert_eq!(snapshot.worktrees_root, original.worktrees_root);
            assert!(Arc::ptr_eq(&snapshot.git_lock, &original.git_lock));
        }
        let guard = original.git_lock.lock().await;
        assert!(retained.git_lock.try_lock().is_err());
        drop(guard);
        assert!(retained.git_lock.try_lock().is_ok());
        assert_eq!(original.base_commit(), original_commit);
        assert_eq!(refreshed.base_commit(), advanced_commit);
        retained
            .verify_source_identity(
                original.repository_identity(),
                Some("main"),
                Some(&original_commit),
            )
            .expect("the old run still resolves its original source");
        assert!(
            refreshed
                .verify_source_identity(
                    original.repository_identity(),
                    Some("main"),
                    Some(&original_commit),
                )
                .is_err()
        );
        assert_eq!(
            retained
                .git_text(&repository, &["worktree", "list", "--porcelain"])
                .await
                .unwrap(),
            worktrees_before
        );
        let resumed = retained
            .prepare(task, run, Some(&original_commit), Some(&original_commit))
            .await
            .expect("resume the exact dirty worktree after connection refresh");
        assert_eq!(resumed.path, workspace.path);
        assert_eq!(resumed.branch, workspace.branch);
        assert_eq!(resumed.base_commit, original_commit);
        assert_eq!(retained.fingerprint(&resumed).await.unwrap(), fingerprint);
        let cleanup = retained.finalize(&resumed).await.unwrap();
        assert_eq!(cleanup.disposition, WorkspaceDisposition::Preserved);
        assert_eq!(cleanup.dirty, Some(true));
        assert_eq!(retained.fingerprint(&resumed).await.unwrap(), fingerprint);
        assert_eq!(
            std::fs::read_to_string(repository.join("README.md")).unwrap(),
            "# advanced source\n"
        );
        cleanup_fixture(&root, &repository);
    }

    #[tokio::test]
    async fn pinned_sources_reject_invalid_unavailable_and_non_commit_ids() {
        let (root, repository, managed) = fixture();
        let manager =
            WorkspaceManager::initialize(managed.clone(), repository.clone(), "main".to_owned())
                .await
                .unwrap();
        let original_commit = manager.base_commit().to_owned();
        command(
            &repository,
            &[
                OsStr::new("-c"),
                OsStr::new("user.name=ECorp Test"),
                OsStr::new("-c"),
                OsStr::new("user.email=crony@example.invalid"),
                OsStr::new("-c"),
                OsStr::new("tag.gpgSign=false"),
                OsStr::new("tag"),
                OsStr::new("-a"),
                OsStr::new("commit-alias"),
                OsStr::new("-m"),
                OsStr::new("tag object is not the exact commit"),
            ],
        );
        let tag = manager
            .git_text(&repository, &["rev-parse", "refs/tags/commit-alias"])
            .await
            .unwrap();
        let tree = manager
            .git_text(&repository, &["rev-parse", "HEAD^{tree}"])
            .await
            .unwrap();
        let worktrees_before = manager
            .git_text(&repository, &["worktree", "list", "--porcelain"])
            .await
            .unwrap();
        for commit in [
            String::new(),
            "HEAD".to_owned(),
            original_commit[..12].to_owned(),
            format!("{original_commit}^{{commit}}"),
            "g".repeat(original_commit.len()),
            "0".repeat(original_commit.len()),
            tag.trim().to_owned(),
            tree.trim().to_owned(),
        ] {
            assert!(manager.pinned("main", &commit).await.is_err(), "{commit}");
            assert!(
                WorkspaceManager::initialize_pinned(
                    managed.clone(),
                    repository.clone(),
                    "main".to_owned(),
                    commit.clone(),
                )
                .await
                .is_err(),
                "{commit}"
            );
        }
        for base_ref in ["", "-main", "main:other", "main\n"] {
            assert!(manager.pinned(base_ref, &original_commit).await.is_err());
            assert!(
                WorkspaceManager::initialize_pinned(
                    managed.clone(),
                    repository.clone(),
                    base_ref.to_owned(),
                    original_commit.clone(),
                )
                .await
                .is_err()
            );
        }
        assert_eq!(manager.base_ref(), "main");
        assert_eq!(manager.base_commit(), original_commit);
        assert_eq!(
            manager
                .git_text(&repository, &["worktree", "list", "--porcelain"])
                .await
                .unwrap(),
            worktrees_before
        );
        cleanup_fixture(&root, &repository);
    }

    #[test]
    fn github_remote_urls_normalize_to_repository_identity() {
        for remote in [
            "https://github.com/shyamsridhar123/ecorp.git",
            "ssh://git@github.com/shyamsridhar123/ecorp.git",
            "git@github.com:shyamsridhar123/ecorp.git",
        ] {
            assert_eq!(
                parse_github_repository_identity(remote).as_deref(),
                Some("shyamsridhar123/ecorp")
            );
        }
        assert_eq!(
            parse_github_repository_identity("https://example.com/acme/repo.git"),
            None
        );
    }

    #[test]
    fn local_repository_identity_is_stable_and_contract_safe() {
        let repository = std::env::temp_dir().join("ECorp dogfood target");
        let first = local_repository_identity(&repository);
        let second = local_repository_identity(&repository);
        assert_eq!(first, second);
        assert!(first.starts_with("local/ECorp-dogfood-target-"));
        assert_eq!(first.split('/').count(), 2);
        assert!(
            first
                .chars()
                .all(|character| character.is_ascii_alphanumeric()
                    || matches!(character, '/' | '-' | '_' | '.'))
        );
    }

    #[test]
    fn path_and_ref_guards_fail_closed() {
        assert!(validate_ref("main").is_ok());
        assert!(validate_ref("release/2026-08-29").is_ok());
        assert!(validate_ref("-dangerous").is_err());
        assert!(validate_ref("main:evil").is_err());
        assert!(validate_commit(&"a".repeat(40)).is_ok());
        assert!(validate_commit(&"a".repeat(64)).is_ok());
        assert!(validate_commit(&"a".repeat(39)).is_err());
        assert!(validate_commit(&"a".repeat(63)).is_err());
        let root = PathBuf::from("safe").join("root");
        assert!(ensure_descendant(&root, &root.join("child")).is_ok());
        assert!(ensure_descendant(&root, &root).is_err());
        assert!(ensure_descendant(&root, &PathBuf::from("escape")).is_err());
    }
}
