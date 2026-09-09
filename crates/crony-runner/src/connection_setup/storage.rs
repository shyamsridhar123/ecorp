//! Runner-private metadata. Native credentials never pass through this module.
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

const MAX_STATE_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct PrivateRoot {
    path: PathBuf,
}

pub(super) fn reject_user_path(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(anyhow!(
            "a local repository must be an absolute, non-traversing path"
        ));
    }
    #[cfg(windows)]
    if !matches!(
        path.components().next(),
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), std::path::Prefix::Disk(_))
    ) {
        return Err(anyhow!(
            "UNC, device and verbatim repository paths are not accepted"
        ));
    }
    Ok(())
}

pub(super) fn reject_links(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        let metadata = match fs::symlink_metadata(ancestor) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() {
            return Err(anyhow!("linked paths are outside the connection boundary"));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(anyhow!("reparse paths are outside the connection boundary"));
            }
        }
    }
    Ok(())
}

pub(super) fn canonical_directory(path: &Path) -> Result<PathBuf> {
    reject_links(path)?;
    let canonical = fs::canonicalize(path).context("source directory is unavailable")?;
    // Match WorkspaceManager's canonical path representation. Windows emits
    // a verbatim prefix here even for an ordinary drive path; retaining it
    // would defeat comparisons with the default source/workspace roots.
    #[cfg(windows)]
    let canonical = {
        let text = canonical.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            PathBuf::from(format!(r"\\{rest}"))
        } else if let Some(rest) = text.strip_prefix(r"\\?\") {
            PathBuf::from(rest)
        } else {
            canonical
        }
    };
    if !canonical.is_dir() {
        return Err(anyhow!("source directory is not a directory"));
    }
    Ok(canonical)
}

impl PrivateRoot {
    pub(super) async fn open(path: PathBuf, forbidden: &[PathBuf]) -> Result<Self> {
        reject_user_path(&path)?;
        reject_links(&path)?;
        let mut existing = path.as_path();
        let mut missing = Vec::new();
        while !existing.exists() {
            missing.push(
                existing
                    .file_name()
                    .context("invalid connection root")?
                    .to_owned(),
            );
            existing = existing.parent().context("invalid connection root")?;
        }
        let mut prospective = canonical_directory(existing)?;
        for part in missing.into_iter().rev() {
            prospective.push(part);
        }
        for source in forbidden {
            if let Ok(source) = canonical_directory(source)
                && prospective.starts_with(source)
            {
                return Err(anyhow!("connection state must be outside source checkouts"));
            }
        }
        if path.exists()
            && !path.join("registry.json").is_file()
            && fs::read_dir(&path)?.next().is_some()
        {
            return Err(anyhow!("connection root contains unowned existing files"));
        }
        fs::create_dir_all(&path).context("create private connection root")?;
        let path = canonical_directory(&path)?;
        protect_directory(&path).await?;
        Ok(Self { path })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// All segments are internal names, UUIDs, or digests, never browser paths.
    pub(super) fn directory(&self, relative: &Path) -> Result<PathBuf> {
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(anyhow!("invalid managed connection directory"));
        }
        let path = self.path.join(relative);
        reject_links(&path)?;
        fs::create_dir_all(&path).context("create managed connection directory")?;
        let path = canonical_directory(&path)?;
        if !path.starts_with(&self.path) || path == self.path {
            return Err(anyhow!("connection directory escaped its private root"));
        }
        Ok(path)
    }

    pub(super) fn read<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>> {
        let path = self.file(name)?;
        reject_links(&path)?;
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_file() || metadata.len() > MAX_STATE_BYTES {
            return Err(anyhow!("connection registry has an invalid size or type"));
        }
        let bytes = fs::read(path).context("read connection registry")?;
        Ok(Some(
            serde_json::from_slice(&bytes).context("decode connection registry")?,
        ))
    }

    pub(super) fn write<T: Serialize>(&self, name: &str, value: &T) -> Result<()> {
        let destination = self.file(name)?;
        reject_links(&destination)?;
        let bytes = serde_json::to_vec(value).context("encode connection registry")?;
        if bytes.len() as u64 > MAX_STATE_BYTES {
            return Err(anyhow!("connection registry exceeds its retention bound"));
        }
        let temporary = self.path.join(format!("{}.tmp", Uuid::new_v4().simple()));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let result = (|| -> Result<()> {
            let mut file = options.open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, destination).context("replace connection registry")?;
            #[cfg(unix)]
            fs::File::open(&self.path)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            // Only the create_new, UUID-named file owned by this write attempt.
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    fn file(&self, name: &str) -> Result<PathBuf> {
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.'))
            || name == "."
            || name == ".."
        {
            return Err(anyhow!("invalid connection registry name"));
        }
        Ok(self.path.join(name))
    }
}

#[cfg(unix)]
async fn protect_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(windows)]
async fn protect_directory(path: &Path) -> Result<()> {
    use chrono::Utc;
    use std::{collections::BTreeMap, ffi::OsString};
    // A constant, noninteractive ACL operation. The path is data in a private
    // child environment, never interpolated into PowerShell or supplied by a browser.
    let script = r#"
$ErrorActionPreference='Stop'
$acl=[System.Security.AccessControl.DirectorySecurity]::new()
$owner=[System.Security.Principal.WindowsIdentity]::GetCurrent().User
$acl.SetOwner($owner)
$acl.SetAccessRuleProtection($true,$false)
foreach($sid in @($owner,[System.Security.Principal.SecurityIdentifier]::new('S-1-5-18'))) {
  $rule=[System.Security.AccessControl.FileSystemAccessRule]::new(
    $sid,'FullControl','ContainerInherit,ObjectInherit','None','Allow')
  $acl.AddAccessRule($rule)
}
[System.IO.Directory]::SetAccessControl($env:ECORP_CONNECTION_ACL_TARGET,$acl)
"#;
    let executable = PathBuf::from(
        std::env::var_os("SystemRoot").unwrap_or_else(|| OsString::from(r"C:\Windows")),
    )
    .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let environment = BTreeMap::from([(
        "ECORP_CONNECTION_ACL_TARGET".to_owned(),
        path.as_os_str().to_owned(),
    )]);
    let args = [
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        script,
    ]
    .map(OsString::from);
    let output = super::process::run_owned(
        &executable,
        &[],
        &args,
        path,
        &environment,
        &[],
        Utc::now() + chrono::Duration::seconds(10),
        None,
    )
    .await?;
    if !output.success {
        return Err(anyhow!(
            "private connection directory permissions could not be established"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_paths_cannot_use_traversal_or_windows_device_names() {
        assert!(reject_user_path(Path::new("relative/repository")).is_err());
        assert!(reject_user_path(&std::env::temp_dir().join("a/../b")).is_err());
        #[cfg(windows)]
        for path in [r"\\server\share\repo", r"\\?\C:\repo", r"\\.\C:\repo"] {
            assert!(reject_user_path(Path::new(path)).is_err());
        }
    }

    #[test]
    fn canonical_roots_keep_native_workspace_path_representation() {
        let root = std::env::temp_dir().join(format!("ecorp-canonical-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let canonical = canonical_directory(&root).unwrap();
        assert_eq!(canonical_directory(&canonical).unwrap(), canonical);
        assert!(reject_user_path(&canonical).is_ok());
        #[cfg(windows)]
        assert!(matches!(
            canonical.components().next(),
            Some(Component::Prefix(prefix)) if matches!(prefix.kind(), std::path::Prefix::Disk(_))
        ));
        fs::remove_dir(root).unwrap();
    }

    #[tokio::test]
    async fn private_root_cannot_hide_inside_a_source_checkout() {
        let source = std::env::temp_dir().join(format!("ecorp-source-boundary-{}", Uuid::new_v4()));
        fs::create_dir(&source).unwrap();
        let private = source.join("private").join("connections");
        assert!(
            PrivateRoot::open(private.clone(), std::slice::from_ref(&source))
                .await
                .is_err()
        );
        assert!(!private.exists());
        fs::remove_dir(source).unwrap();
    }

    #[test]
    fn registry_replacement_is_exact_and_rejects_corrupt_or_escaping_files() {
        let path = std::env::temp_dir().join(format!("ecorp-registry-{}", Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        let root = PrivateRoot {
            path: fs::canonicalize(&path).unwrap(),
        };
        root.write("registry.json", &vec!["first"]).unwrap();
        root.write("registry.json", &vec!["second"]).unwrap();
        assert_eq!(
            root.read::<Vec<String>>("registry.json").unwrap().unwrap(),
            vec!["second"]
        );
        assert!(root.directory(Path::new("../escape")).is_err());
        assert!(root.read::<Vec<String>>("../outside").is_err());
        fs::write(path.join("registry.json"), b"invalid").unwrap();
        assert!(root.read::<Vec<String>>("registry.json").is_err());
        fs::remove_file(path.join("registry.json")).unwrap();
        fs::remove_dir(path).unwrap();
    }
}
