use std::{
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use github_copilot_sdk::session_fs::{
    DirEntry, DirEntryKind, FileInfo, FsError, FsErrorKind, SessionFsProvider,
};
use tokio::{fs, io::AsyncWriteExt};

use super::permission::path_is_inside;

pub(super) struct ContainedSessionFs {
    workspace: PathBuf,
    state_directory: PathBuf,
}

impl ContainedSessionFs {
    pub(super) fn new(workspace: PathBuf, state_directory: PathBuf) -> Self {
        Self {
            workspace,
            state_directory,
        }
    }

    fn resolve(&self, raw: &str) -> Result<PathBuf, FsError> {
        if raw.is_empty()
            || Path::new(raw)
                .components()
                .any(|component| component == Component::ParentDir)
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
        let candidate_text = candidate.to_string_lossy();
        if path_is_inside(&self.workspace, &candidate_text)
            || path_is_inside(&self.state_directory, &candidate_text)
        {
            Ok(candidate)
        } else {
            Err(outside_boundary(raw))
        }
    }

    fn resolve_destructive(&self, raw: &str) -> Result<PathBuf, FsError> {
        let path = self.resolve(raw)?;
        if same_path(&path, &self.workspace) || same_path(&path, &self.state_directory) {
            return Err(FsError::with_message(
                FsErrorKind::Other,
                "refusing to mutate a session filesystem root",
            ));
        }
        Ok(path)
    }
}

#[async_trait]
impl SessionFsProvider for ContainedSessionFs {
    async fn read_file(&self, path: &str) -> Result<String, FsError> {
        Ok(fs::read_to_string(self.resolve(path)?).await?)
    }

    async fn write_file(
        &self,
        path: &str,
        content: &str,
        _mode: Option<i64>,
    ) -> Result<(), FsError> {
        let path = self.resolve(path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(path, content).await?;
        Ok(())
    }

    async fn append_file(
        &self,
        path: &str,
        content: &str,
        _mode: Option<i64>,
    ) -> Result<(), FsError> {
        let path = self.resolve(path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await?;
        file.write_all(content.as_bytes()).await?;
        file.flush().await?;
        Ok(())
    }

    async fn exists(&self, path: &str) -> Result<bool, FsError> {
        Ok(fs::try_exists(self.resolve(path)?).await?)
    }

    async fn stat(&self, path: &str) -> Result<FileInfo, FsError> {
        let metadata = fs::metadata(self.resolve(path)?).await?;
        let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
        let created = metadata.created().unwrap_or(modified);
        Ok(FileInfo::new(
            metadata.is_file(),
            metadata.is_dir(),
            i64::try_from(metadata.len()).unwrap_or(i64::MAX),
            timestamp(modified),
            timestamp(created),
        ))
    }

    async fn mkdir(&self, path: &str, recursive: bool, _mode: Option<i64>) -> Result<(), FsError> {
        let path = self.resolve(path)?;
        if recursive {
            fs::create_dir_all(path).await?;
        } else {
            fs::create_dir(path).await?;
        }
        Ok(())
    }

    async fn readdir(&self, path: &str) -> Result<Vec<String>, FsError> {
        let mut directory = fs::read_dir(self.resolve(path)?).await?;
        let mut entries = Vec::new();
        while let Some(entry) = directory.next_entry().await? {
            entries.push(entry.file_name().to_string_lossy().into_owned());
        }
        entries.sort();
        Ok(entries)
    }

    async fn readdir_with_types(&self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        let mut directory = fs::read_dir(self.resolve(path)?).await?;
        let mut entries = Vec::new();
        while let Some(entry) = directory.next_entry().await? {
            let kind = if entry.file_type().await?.is_dir() {
                DirEntryKind::Directory
            } else {
                DirEntryKind::File
            };
            entries.push(DirEntry::new(entry.file_name().to_string_lossy(), kind));
        }
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(entries)
    }

    async fn rm(&self, path: &str, recursive: bool, force: bool) -> Result<(), FsError> {
        let path = self.resolve_destructive(path)?;
        let metadata = match fs::symlink_metadata(&path).await {
            Ok(metadata) => metadata,
            Err(error) if force && error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        if metadata.is_dir() {
            if recursive {
                fs::remove_dir_all(path).await?;
            } else {
                fs::remove_dir(path).await?;
            }
        } else {
            fs::remove_file(path).await?;
        }
        Ok(())
    }

    async fn rename(&self, src: &str, dest: &str) -> Result<(), FsError> {
        let src = self.resolve_destructive(src)?;
        let dest = self.resolve_destructive(dest)?;
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::rename(src, dest).await?;
        Ok(())
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

fn same_path(left: &Path, right: &Path) -> bool {
    let left = std::fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = std::fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    #[cfg(windows)]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn timestamp(value: SystemTime) -> String {
    DateTime::<Utc>::from(value).to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn filesystem_provider_contains_reads_writes_and_renames() {
        let root = std::env::temp_dir().join(format!("crony-copilot-fs-{}", uuid::Uuid::new_v4()));
        let workspace = root.join("workspace with spaces");
        let state = root.join("state");
        fs::create_dir_all(&workspace).await.expect("workspace");
        fs::create_dir_all(&state).await.expect("state");
        let provider = ContainedSessionFs::new(workspace.clone(), state.clone());

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

        fs::remove_dir_all(root).await.expect("cleanup");
    }
}
