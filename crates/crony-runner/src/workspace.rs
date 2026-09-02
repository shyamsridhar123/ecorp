use std::{
    ffi::OsString,
    path::{Component, Path, PathBuf},
    process::{Output, Stdio},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use tokio::{process::Command, sync::Mutex};
use url::Url;
use uuid::Uuid;

const GIT_TIMEOUT: Duration = Duration::from_secs(30);

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
        manager.base_commit = manager.resolve_base_commit().await?;
        manager.repository_identity = manager
            .git_text(
                &manager.repository,
                &["config", "--get", "remote.origin.url"],
            )
            .await
            .ok()
            .and_then(|remote| parse_github_repository_identity(&remote));
        Ok(manager)
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
    fn path_and_ref_guards_fail_closed() {
        assert!(validate_ref("main").is_ok());
        assert!(validate_ref("release/2026-08-29").is_ok());
        assert!(validate_ref("-dangerous").is_err());
        assert!(validate_ref("main:evil").is_err());
        let root = PathBuf::from("safe").join("root");
        assert!(ensure_descendant(&root, &root.join("child")).is_ok());
        assert!(ensure_descendant(&root, &root).is_err());
        assert!(ensure_descendant(&root, &PathBuf::from("escape")).is_err());
    }
}
