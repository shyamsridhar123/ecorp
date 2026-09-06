use std::{
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use cap_fs_ext::{FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::{ambient_authority, fs::Dir};
use chrono::{DateTime, SecondsFormat, Utc};
use github_copilot_sdk::session_fs::{
    DirEntry, DirEntryKind, FileInfo, FsError, FsErrorKind, SessionFsProvider,
};

pub(super) struct ContainedSessionFs {
    workspace: PathBuf,
    state_directory: PathBuf,
    workspace_dir: Arc<Dir>,
    state_dir: Arc<Dir>,
    write_scope: Vec<String>,
}

struct ScopedPath {
    dir: Arc<Dir>,
    relative: PathBuf,
    write_allowed: bool,
}

impl ScopedPath {
    fn check_links(&self) -> Result<(), FsError> {
        let mut path = PathBuf::new();
        for component in self.relative.components() {
            path.push(component);
            match self.dir.symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(outside_boundary("symbolic link"));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    fn open_file(&self, write: bool, append: bool) -> Result<cap_std::fs::File, FsError> {
        self.check_links()?;
        if write && let Some(parent) = self.relative.parent() {
            self.dir.create_dir_all(parent)?;
        }
        let mut options = cap_std::fs::OpenOptions::new();
        options
            .read(!write)
            .write(write)
            .append(append)
            .create(write)
            .follow(FollowSymlinks::No);
        // Do not truncate before checking the opened object's identity. A
        // lexical path check alone cannot protect an external hard-link target.
        let file = self.dir.open_with(&self.relative, &options)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err(outside_boundary("non-regular or multiply-linked file"));
        }
        Ok(file)
    }
}

impl ContainedSessionFs {
    pub(super) fn new(
        workspace: PathBuf,
        state_directory: PathBuf,
        write_scope: Vec<String>,
    ) -> std::io::Result<Self> {
        // These two roots are selected by the trusted runner, never by a tool
        // request. All subsequent I/O is relative to the retained capabilities.
        let workspace_dir = Arc::new(Dir::open_ambient_dir(&workspace, ambient_authority())?);
        let state_dir = Arc::new(Dir::open_ambient_dir(
            &state_directory,
            ambient_authority(),
        )?);
        Ok(Self {
            workspace,
            state_directory,
            workspace_dir,
            state_dir,
            write_scope,
        })
    }

    fn resolve(&self, raw: &str) -> Result<ScopedPath, FsError> {
        if raw.is_empty()
            || raw.contains('\0')
            || raw.split(['/', '\\']).any(|component| component == "..")
        {
            return Err(outside_boundary(raw));
        }

        let candidate = map_virtual_root(&self.workspace, "/workspace", raw)
            .or_else(|| map_virtual_root(&self.state_directory, "/session-state", raw))
            .unwrap_or_else(|| {
                let candidate = PathBuf::from(raw);
                if candidate.is_absolute() {
                    candidate
                } else {
                    self.workspace.join(candidate)
                }
            });
        for (root, dir) in [
            (&self.workspace, &self.workspace_dir),
            (&self.state_directory, &self.state_dir),
        ] {
            if let Some(relative) = relative_to(&candidate, root) {
                // Forbid Windows device names, ADS, trailing-dot/space aliases,
                // and Git control files on every platform, not just Windows.
                if relative.components().any(|component| match component {
                    Component::Normal(name) => unsafe_component(&name.to_string_lossy()),
                    _ => true,
                }) {
                    return Err(outside_boundary(raw));
                }
                return Ok(ScopedPath {
                    dir: dir.clone(),
                    write_allowed: Arc::ptr_eq(dir, &self.state_dir)
                        || self.write_scope.iter().any(|scope| {
                            crony_domain::write_scope_allows_path(
                                scope,
                                &relative.to_string_lossy().replace('\\', "/"),
                            )
                        }),
                    relative,
                });
            }
        }
        Err(outside_boundary(raw))
    }

    fn resolve_destructive(&self, raw: &str) -> Result<ScopedPath, FsError> {
        let path = self.resolve(raw)?;
        if path.relative.as_os_str().is_empty() {
            return Err(FsError::with_message(
                FsErrorKind::Other,
                "refusing to mutate a session filesystem root",
            ));
        }
        if !path.write_allowed {
            return Err(FsError::with_message(
                FsErrorKind::Other,
                "path is outside the persisted task write scope",
            ));
        }
        Ok(path)
    }

    async fn access<T, F>(&self, raw: &str, mutation: bool, operation: F) -> Result<T, FsError>
    where
        T: Send + 'static,
        F: FnOnce(ScopedPath) -> Result<T, FsError> + Send + 'static,
    {
        let path = if mutation {
            self.resolve_destructive(raw)?
        } else {
            self.resolve(raw)?
        };
        tokio::task::spawn_blocking(move || {
            path.check_links()?;
            operation(path)
        })
        .await
        .map_err(|_| {
            FsError::with_message(FsErrorKind::Other, "session filesystem worker failed")
        })?
    }
}

#[async_trait]
impl SessionFsProvider for ContainedSessionFs {
    async fn read_file(&self, path: &str) -> Result<String, FsError> {
        self.access(path, false, |path| {
            let mut text = String::new();
            path.open_file(false, false)?.read_to_string(&mut text)?;
            Ok(text)
        })
        .await
    }

    async fn write_file(
        &self,
        path: &str,
        content: &str,
        mode: Option<i64>,
    ) -> Result<(), FsError> {
        let mode = validated_mode(mode)?;
        let content = content.to_owned();
        self.access(path, true, move |path| {
            let mut file = path.open_file(true, false)?;
            apply_file_mode(&file, mode)?;
            file.set_len(0)?;
            file.write_all(content.as_bytes())?;
            file.flush()?;
            Ok(())
        })
        .await
    }

    async fn append_file(
        &self,
        path: &str,
        content: &str,
        mode: Option<i64>,
    ) -> Result<(), FsError> {
        let mode = validated_mode(mode)?;
        let content = content.to_owned();
        self.access(path, true, move |path| {
            let mut file = path.open_file(true, true)?;
            apply_file_mode(&file, mode)?;
            file.write_all(content.as_bytes())?;
            file.flush()?;
            Ok(())
        })
        .await
    }

    async fn exists(&self, path: &str) -> Result<bool, FsError> {
        self.access(path, false, |path| {
            match path.dir.symlink_metadata(directory_path(&path.relative)) {
                Ok(_) => Ok(true),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(error.into()),
            }
        })
        .await
    }

    async fn stat(&self, path: &str) -> Result<FileInfo, FsError> {
        self.access(path, false, |path| {
            let metadata = path.dir.symlink_metadata(directory_path(&path.relative))?;
            let modified = metadata
                .modified()
                .map(|time| time.into_std())
                .unwrap_or(UNIX_EPOCH);
            let created = metadata
                .created()
                .map(|time| time.into_std())
                .unwrap_or(modified);
            Ok(FileInfo::new(
                metadata.is_file(),
                metadata.is_dir(),
                i64::try_from(metadata.len()).unwrap_or(i64::MAX),
                timestamp(modified),
                timestamp(created),
            ))
        })
        .await
    }

    async fn mkdir(&self, path: &str, recursive: bool, mode: Option<i64>) -> Result<(), FsError> {
        let mode = validated_mode(mode)?;
        let scoped = self.resolve(path)?;
        let directory = scoped.relative.to_string_lossy().replace('\\', "/");
        if !scoped.write_allowed
            && !directory.is_empty()
            && !self
                .write_scope
                .iter()
                .any(|scope| scope.starts_with(&format!("{directory}/")))
        {
            return Err(FsError::with_message(
                FsErrorKind::Other,
                "directory is outside the persisted task write scope",
            ));
        }
        self.access(path, false, move |path| {
            let mut builder = cap_std::fs::DirBuilder::new();
            builder.recursive(recursive);
            #[cfg(unix)]
            if let Some(mode) = mode {
                use cap_std::fs::DirBuilderExt;
                builder.mode(mode);
            }
            #[cfg(not(unix))]
            let _ = mode;
            // Idempotent recursive mkdir of a root is harmless; never chmod it.
            if path.relative.as_os_str().is_empty() && recursive {
                return Ok(());
            }
            path.dir.create_dir_with(&path.relative, &builder)?;
            Ok(())
        })
        .await
    }

    async fn readdir(&self, path: &str) -> Result<Vec<String>, FsError> {
        self.access(path, false, |path| {
            let mut entries = path
                .dir
                .read_dir(directory_path(&path.relative))?
                .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
                .collect::<Result<Vec<_>, _>>()?;
            entries.sort();
            Ok(entries)
        })
        .await
    }

    async fn readdir_with_types(&self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        self.access(path, false, |path| {
            let mut entries = Vec::new();
            for entry in path.dir.read_dir(directory_path(&path.relative))? {
                let entry = entry?;
                let kind = if entry.file_type()?.is_dir() {
                    DirEntryKind::Directory
                } else {
                    DirEntryKind::File
                };
                entries.push(DirEntry::new(entry.file_name().to_string_lossy(), kind));
            }
            entries.sort_by(|left, right| left.name.cmp(&right.name));
            Ok(entries)
        })
        .await
    }

    async fn rm(&self, path: &str, recursive: bool, force: bool) -> Result<(), FsError> {
        self.access(path, true, move |path| {
            let metadata = match path.dir.symlink_metadata(&path.relative) {
                Ok(metadata) => metadata,
                Err(error) if force && error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(());
                }
                Err(error) => return Err(error.into()),
            };
            if metadata.is_dir() {
                if recursive {
                    path.dir.remove_dir_all(&path.relative)?;
                } else {
                    path.dir.remove_dir(&path.relative)?;
                }
            } else {
                path.dir.remove_file(&path.relative)?;
            }
            Ok(())
        })
        .await
    }

    async fn rename(&self, src: &str, dest: &str) -> Result<(), FsError> {
        let dest = self.resolve_destructive(dest)?;
        self.access(src, true, move |src| {
            dest.check_links()?;
            if let Some(parent) = dest.relative.parent() {
                dest.dir.create_dir_all(parent)?;
            }
            src.dir.rename(&src.relative, &dest.dir, &dest.relative)?;
            Ok(())
        })
        .await
    }
}

fn map_virtual_root(root: &Path, virtual_root: &str, raw: &str) -> Option<PathBuf> {
    let normalized = raw.replace('\\', "/");
    if normalized.eq_ignore_ascii_case(virtual_root) {
        return Some(root.to_path_buf());
    }
    let prefix = format!("{virtual_root}/");
    if normalized
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(&prefix))
    {
        let relative = &normalized[prefix.len()..];
        if relative.split('/').any(|component| component == "..") {
            return None;
        }
        return Some(
            relative
                .split('/')
                .filter(|component| !component.is_empty())
                .fold(root.to_path_buf(), |path, component| path.join(component)),
        );
    }
    None
}

fn outside_boundary(path: &str) -> FsError {
    FsError::with_message(
        FsErrorKind::Other,
        format!("path is outside the assigned ECorp worktree: {path}"),
    )
}

fn relative_to(path: &Path, base: &Path) -> Option<PathBuf> {
    let mut path = path.components().filter(|part| *part != Component::CurDir);
    for expected in base.components().filter(|part| *part != Component::CurDir) {
        let actual = path.next()?;
        #[cfg(windows)]
        let matches = actual
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&expected.as_os_str().to_string_lossy());
        #[cfg(not(windows))]
        let matches = actual == expected;
        if !matches {
            return None;
        }
    }
    Some(path.collect())
}

fn unsafe_component(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    let stem = upper.split('.').next().unwrap_or("");
    name.contains([':', '\0'])
        || name.ends_with(['.', ' '])
        || name.eq_ignore_ascii_case(".git")
        || matches!(stem, "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
}

fn directory_path(relative: &Path) -> &Path {
    if relative.as_os_str().is_empty() {
        Path::new(".")
    } else {
        relative
    }
}

fn validated_mode(mode: Option<i64>) -> Result<Option<u32>, FsError> {
    if mode.is_some_and(|mode| !(0..=0o777).contains(&mode)) {
        return Err(FsError::with_message(
            FsErrorKind::Other,
            "only ordinary permission bits are allowed",
        ));
    }
    Ok(mode.map(|mode| mode as u32))
}

fn apply_file_mode(file: &cap_std::fs::File, mode: Option<u32>) -> Result<(), FsError> {
    #[cfg(unix)]
    if let Some(mode) = mode {
        use cap_std::fs::PermissionsExt;
        file.set_permissions(cap_std::fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = (file, mode);
    Ok(())
}

fn timestamp(value: SystemTime) -> String {
    DateTime::<Utc>::from(value).to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::fs;

    #[cfg(any(unix, windows))]
    fn symlink_file(target: &Path, link: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).expect("create file symlink");
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(target, link).expect("create file symlink");
    }

    #[cfg(any(unix, windows))]
    #[tokio::test]
    async fn dangling_symlinks_cannot_create_external_files() {
        let root =
            std::env::temp_dir().join(format!("crony-copilot-link-{}", uuid::Uuid::new_v4()));
        let workspace = root.join("workspace");
        let state = root.join("state");
        fs::create_dir_all(&workspace).await.expect("workspace");
        fs::create_dir_all(&state).await.expect("state");
        let outside = root.join("outside.txt");
        symlink_file(&outside, &workspace.join("escape.txt"));
        let provider = ContainedSessionFs::new(workspace.clone(), state, vec!["**".to_owned()])
            .expect("capability roots");
        let result = provider.write_file("escape.txt", "ESCAPED", None).await;
        let escaped = outside.exists();
        drop(provider);
        fs::remove_dir_all(&root).await.expect("cleanup");
        assert!(result.is_err(), "dangling symlink write must be denied");
        assert!(
            !escaped,
            "denial must happen before creating an external file"
        );
    }

    #[tokio::test]
    async fn hard_links_cannot_mutate_external_files() {
        let root =
            std::env::temp_dir().join(format!("crony-copilot-hardlink-{}", uuid::Uuid::new_v4()));
        let workspace = root.join("workspace");
        let state = root.join("state");
        fs::create_dir_all(&workspace).await.expect("workspace");
        fs::create_dir_all(&state).await.expect("state");
        let outside = root.join("outside.txt");
        fs::write(&outside, "PRESERVE").await.expect("sentinel");
        fs::hard_link(&outside, workspace.join("escape.txt"))
            .await
            .expect("hard link");
        let provider = ContainedSessionFs::new(workspace.clone(), state, vec!["**".to_owned()])
            .expect("capability roots");
        let result = provider.append_file("escape.txt", "ESCAPED", None).await;
        let bytes = fs::read_to_string(&outside).await.expect("sentinel bytes");
        drop(provider);
        fs::remove_dir_all(&root).await.expect("cleanup");
        assert!(result.is_err(), "multiply-linked files must be denied");
        assert_eq!(bytes, "PRESERVE");
    }

    #[cfg(any(unix, windows))]
    #[tokio::test]
    async fn all_operations_reject_linked_paths_and_protect_roots() {
        let root =
            std::env::temp_dir().join(format!("crony-copilot-links-{}", uuid::Uuid::new_v4()));
        let workspace = root.join("workspace");
        let state = root.join("state");
        let outside = root.join("outside");
        for dir in [&workspace, &state, &outside] {
            fs::create_dir_all(dir).await.expect("fixture directory");
        }
        fs::write(outside.join("secret.txt"), "PRESERVE")
            .await
            .expect("sentinel");
        symlink_file(&outside.join("missing.txt"), &workspace.join("dangling"));
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, workspace.join("linked")).expect("directory symlink");
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&outside, workspace.join("linked"))
            .expect("directory symlink");
        let provider =
            ContainedSessionFs::new(workspace.clone(), state.clone(), vec!["**".to_owned()])
                .expect("capability roots");
        provider
            .write_file("normal.txt", "OK", None)
            .await
            .expect("normal file");

        for path in ["dangling", "linked/secret.txt", "linked/new.txt"] {
            assert!(provider.read_file(path).await.is_err(), "read {path}");
            assert!(
                provider.write_file(path, "ESCAPE", None).await.is_err(),
                "write {path}"
            );
            assert!(
                provider.append_file(path, "ESCAPE", None).await.is_err(),
                "append {path}"
            );
            assert!(provider.stat(path).await.is_err(), "stat {path}");
            assert!(provider.exists(path).await.is_err(), "exists {path}");
            assert!(provider.rm(path, false, true).await.is_err(), "rm {path}");
            assert!(
                provider.rename("normal.txt", path).await.is_err(),
                "rename destination {path}"
            );
            assert!(
                provider.rename(path, "renamed.txt").await.is_err(),
                "rename source {path}"
            );
        }
        assert!(provider.mkdir("linked/newdir", true, None).await.is_err());
        assert!(provider.readdir("linked").await.is_err());
        assert!(provider.readdir_with_types("linked").await.is_err());
        for root in ["/workspace", "/session-state"] {
            assert!(provider.exists(root).await.expect("root exists"));
            assert!(provider.stat(root).await.expect("root stat").is_directory);
            assert!(provider.write_file(root, "NO", None).await.is_err());
            assert!(provider.rm(root, true, true).await.is_err());
            assert!(provider.rename(root, "moved").await.is_err());
        }
        for path in [
            ".git/config",
            "file.txt:stream",
            "NUL",
            "COM1.txt",
            "trailing.",
        ] {
            assert!(
                provider.write_file(path, "NO", None).await.is_err(),
                "{path}"
            );
        }
        assert_eq!(
            fs::read_to_string(outside.join("secret.txt"))
                .await
                .expect("unchanged"),
            "PRESERVE"
        );
        assert!(!outside.join("missing.txt").exists());
        assert!(!outside.join("new.txt").exists());
        assert!(!outside.join("newdir").exists());
        drop(provider);
        fs::remove_dir_all(root).await.expect("cleanup");
    }

    #[test]
    fn file_modes_reject_privilege_bits_and_invalid_values() {
        for mode in [-1, 0o1000, 0o2000, 0o4000, 0o4777, i64::MAX] {
            assert!(validated_mode(Some(mode)).is_err());
        }
        assert_eq!(
            validated_mode(Some(0o755)).expect("ordinary mode"),
            Some(0o755)
        );
        assert_eq!(validated_mode(None).expect("default mode"), None);
    }

    #[tokio::test]
    async fn native_writes_obey_the_persisted_task_scope() {
        let root =
            std::env::temp_dir().join(format!("crony-copilot-scope-{}", uuid::Uuid::new_v4()));
        let workspace = root.join("workspace");
        let state = root.join("state");
        fs::create_dir_all(&workspace).await.expect("workspace");
        fs::create_dir_all(&state).await.expect("state");
        fs::write(workspace.join("README.md"), "PRESERVE")
            .await
            .expect("source file");
        let provider = ContainedSessionFs::new(
            workspace.clone(),
            state,
            vec!["scenarios/piper-kingdom/**".to_owned()],
        )
        .expect("capability roots");
        provider
            .mkdir("scenarios", true, None)
            .await
            .expect("allowed ancestor mkdir");
        provider
            .write_file("scenarios/piper-kingdom/game.js", "OK", None)
            .await
            .expect("allowed write");
        assert!(provider.write_file("README.md", "NO", None).await.is_err());
        assert!(provider.append_file("README.md", "NO", None).await.is_err());
        assert!(provider.rm("README.md", false, false).await.is_err());
        assert!(provider.mkdir("scenarios/other", true, None).await.is_err());
        assert!(provider.rm("scenarios", true, false).await.is_err());
        assert!(
            provider
                .rename("scenarios/piper-kingdom/game.js", "README.md")
                .await
                .is_err()
        );
        assert!(
            provider
                .rename("README.md", "scenarios/piper-kingdom/stolen.md")
                .await
                .is_err()
        );
        assert_eq!(
            provider
                .read_file("README.md")
                .await
                .expect("source reads allowed"),
            "PRESERVE"
        );
        drop(provider);
        fs::remove_dir_all(root).await.expect("cleanup");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn native_files_preserve_requested_executable_modes() {
        use std::os::unix::fs::PermissionsExt;
        let root =
            std::env::temp_dir().join(format!("crony-copilot-mode-{}", uuid::Uuid::new_v4()));
        let workspace = root.join("workspace");
        let state = root.join("state");
        fs::create_dir_all(&workspace).await.expect("workspace");
        fs::create_dir_all(&state).await.expect("state");
        let provider = ContainedSessionFs::new(workspace.clone(), state, vec!["**".to_owned()])
            .expect("capability roots");
        provider
            .write_file("check.sh", "#!/bin/sh\n", Some(0o755))
            .await
            .expect("executable write");
        assert_eq!(
            fs::metadata(workspace.join("check.sh"))
                .await
                .expect("mode")
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        provider
            .append_file("check.sh", "exit 0\n", Some(0o700))
            .await
            .expect("append mode");
        assert_eq!(
            fs::metadata(workspace.join("check.sh"))
                .await
                .expect("mode")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert!(
            provider
                .write_file("privileged.sh", "NO", Some(0o4755))
                .await
                .is_err()
        );
        assert!(!workspace.join("privileged.sh").exists());
        drop(provider);
        fs::remove_dir_all(root).await.expect("cleanup");
    }

    #[tokio::test]
    async fn filesystem_provider_contains_reads_writes_and_renames() {
        let root = std::env::temp_dir().join(format!("crony-copilot-fs-{}", uuid::Uuid::new_v4()));
        let workspace = root.join("workspace with spaces");
        let state = root.join("state");
        fs::create_dir_all(&workspace).await.expect("workspace");
        fs::create_dir_all(&state).await.expect("state");
        let provider =
            ContainedSessionFs::new(workspace.clone(), state.clone(), vec!["**".to_owned()])
                .expect("capability roots");

        provider
            .write_file("proof/result.txt", "OK", None)
            .await
            .expect("contained write");
        assert_eq!(
            provider
                .read_file("/workspace/proof/result.txt")
                .await
                .expect("aliased read"),
            "OK"
        );
        provider
            .rename(
                "proof/result.txt",
                &workspace.join("proof/final.txt").to_string_lossy(),
            )
            .await
            .expect("contained rename");
        assert!(
            provider
                .exists("proof/final.txt")
                .await
                .expect("contained exists")
        );

        let outside = root.join("outside.txt");
        assert!(
            provider
                .write_file(&outside.to_string_lossy(), "NO", None)
                .await
                .is_err()
        );
        assert!(
            provider
                .write_file("../escape.txt", "NO", None)
                .await
                .is_err()
        );
        assert!(!outside.exists());
        assert!(provider.rm("/workspace", true, false).await.is_err());

        drop(provider);
        fs::remove_dir_all(root).await.expect("cleanup");
    }
}
