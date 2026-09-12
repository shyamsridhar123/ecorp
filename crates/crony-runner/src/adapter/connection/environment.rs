use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    path::{Component, Path, PathBuf},
};

use anyhow::{Result, bail};
use crony_domain::CodingAgent;
use tokio::process::Command;

// These are operating-system/transport settings, not provider credentials,
// provider routing, node preload hooks, or native account configuration.
const INHERITED_OS_ENV: &[&str] = &[
    "PATH",
    "PATHEXT",
    "SYSTEMROOT",
    "WINDIR",
    "COMSPEC",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TZ",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "HTTPS_PROXY",
    "HTTP_PROXY",
    "NO_PROXY",
];

#[derive(Clone)]
pub(crate) struct ProfileEnvironment {
    pub(super) agent: CodingAgent,
    pub(super) root: PathBuf,
    pub(super) system: bool,
}

impl ProfileEnvironment {
    pub(super) fn new(agent: CodingAgent, root: PathBuf, system: bool) -> Result<Self> {
        validate_absolute_path(&root)?;
        Ok(Self {
            agent,
            root,
            system,
        })
    }

    pub(crate) fn native_home(&self) -> PathBuf {
        self.root.join("native")
    }

    pub(crate) fn session_root(&self) -> PathBuf {
        // Native credentials are siblings, NEVER part of the SDK session-FS root.
        self.root.join("session-fs")
    }

    pub(crate) fn is_system(&self) -> bool {
        self.system
    }

    pub(crate) fn validate_workspace(&self, workspace: &Path) -> Result<()> {
        validate_absolute_path(workspace)?;
        let home = resolve_existing_ancestor(&self.root)?;
        let workspace = resolve_existing_ancestor(workspace)?;
        if paths_overlap(&home, &workspace) {
            bail!("native account storage and source/worktree storage must be disjoint");
        }
        Ok(())
    }

    pub(crate) async fn prepare(&self, workspace: &Path) -> Result<()> {
        self.validate_workspace(workspace)?;
        // The manager creates/protects this root. Never copy, inspect, or migrate
        // credentials; only the selected native authentication flow writes them.
        if !self.root.is_dir() {
            bail!("protected native profile root has not been prepared by the runner");
        }
        if !self.system {
            for directory in self.private_directories() {
                self.validate_private_directory(&directory)?;
                tokio::fs::create_dir_all(&directory)
                    .await
                    .map_err(|_| anyhow::anyhow!("cannot prepare native profile directories"))?;
                self.validate_private_directory(&directory)?;
            }
        }
        Ok(())
    }

    fn validate_private_directory(&self, directory: &Path) -> Result<()> {
        let root = resolve_existing_ancestor(&self.root)?;
        let relative = directory
            .strip_prefix(&self.root)
            .map_err(|_| anyhow::anyhow!("native account directory escaped its protected root"))?;
        let expected = root.join(relative);
        let actual = resolve_existing_ancestor(directory)?;
        #[cfg(windows)]
        let exact = actual
            .as_os_str()
            .eq_ignore_ascii_case(expected.as_os_str());
        #[cfg(not(windows))]
        let exact = actual == expected;
        if !exact {
            bail!("native account directories must not redirect outside their protected profile");
        }
        Ok(())
    }

    fn private_directories(&self) -> Vec<PathBuf> {
        [
            self.native_home(),
            self.root.join("user"),
            self.root.join("config"),
            self.root.join("data"),
            self.root.join("cache"),
            self.root.join("temp"),
            self.root.join("roaming"),
            self.root.join("local"),
        ]
        .into()
    }

    pub(crate) fn values(&self) -> Vec<(OsString, OsString)> {
        if self.system {
            return Vec::new();
        }
        let mut values: Vec<(OsString, OsString)> = [
            ("HOME", self.root.join("user")),
            ("USERPROFILE", self.root.join("user")),
            ("XDG_CONFIG_HOME", self.root.join("config")),
            ("XDG_DATA_HOME", self.root.join("data")),
            ("XDG_CACHE_HOME", self.root.join("cache")),
            ("TEMP", self.root.join("temp")),
            ("TMP", self.root.join("temp")),
            ("TMPDIR", self.root.join("temp")),
            ("APPDATA", self.root.join("roaming")),
            ("LOCALAPPDATA", self.root.join("local")),
            ("GH_CONFIG_DIR", self.root.join("config").join("gh")),
        ]
        .into_iter()
        .map(|(key, path)| (key.into(), path.into_os_string()))
        .collect();
        let key = match self.agent {
            CodingAgent::Codex => "CODEX_HOME",
            CodingAgent::ClaudeCode => "CLAUDE_CONFIG_DIR",
            CodingAgent::GitHubCopilot => "COPILOT_HOME",
        };
        values.push((key.into(), self.native_home().into_os_string()));
        if self.agent == CodingAgent::GitHubCopilot {
            // This is the SDK 1.0.11 Empty-mode credential policy. Native CLI
            // login MUST use the same policy as subsequent SDK clients.
            values.push(("COPILOT_DISABLE_KEYTAR".into(), "1".into()));
        }
        values
    }

    pub(crate) fn apply(&self, command: &mut Command) {
        if !self.system {
            command.env_clear();
            for key in INHERITED_OS_ENV {
                if let Some(value) = std::env::var_os(key) {
                    command.env(key, value);
                }
            }
            command.envs(self.values());
        }
    }

    pub(crate) fn apply_request(
        &self,
        command: &mut Command,
        environment: &HashMap<String, String>,
    ) -> Result<()> {
        self.validate_request(environment)?;
        self.apply(command);
        command.envs(environment);
        Ok(())
    }

    pub(crate) fn validate_request(&self, environment: &HashMap<String, String>) -> Result<()> {
        // Neither a task secret grant nor a resume can redirect a connection's
        // selected account, executable, provider, or configuration.
        if environment.keys().any(|key| reserved_environment_key(key)) {
            bail!(
                "task environment cannot override a native connection's identity or configuration"
            );
        }
        Ok(())
    }

    pub(crate) fn sdk_environment_removals(&self) -> Vec<OsString> {
        if self.system {
            return Vec::new();
        }
        let overrides = self.values();
        // ClientOptions has env_remove rather than env_clear. Inspect names
        // only, never retain or report ambient values.
        std::env::vars_os()
            .map(|(key, _)| key)
            .filter(|key| {
                !INHERITED_OS_ENV
                    .iter()
                    .any(|allowed| key.eq_ignore_ascii_case(OsStr::new(allowed)))
                    && !overrides
                        .iter()
                        .any(|(override_key, _)| key.eq_ignore_ascii_case(override_key))
            })
            .collect()
    }
}

fn reserved_environment_key(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    key.starts_with("CODEX_")
        || key.starts_with("CLAUDE_")
        || key.starts_with("ANTHROPIC_")
        || key.starts_with("OPENAI_")
        || key.starts_with("COPILOT_")
        || key.starts_with("GH_")
        || key.starts_with("GITHUB_")
        || key.starts_with("XDG_")
        || key.starts_with("AWS_")
        || key.starts_with("AZURE_")
        || key.starts_with("GOOGLE_")
        || matches!(
            key.as_str(),
            "HOME"
                | "USERPROFILE"
                | "HOMEDRIVE"
                | "HOMEPATH"
                | "APPDATA"
                | "LOCALAPPDATA"
                | "PATH"
                | "PATHEXT"
                | "COMSPEC"
                | "NODE_OPTIONS"
                | "NODE_PATH"
                | "LD_PRELOAD"
                | "LD_LIBRARY_PATH"
                | "DYLD_INSERT_LIBRARIES"
                | "BROWSER"
                | "SYSTEMROOT"
                | "WINDIR"
                | "TEMP"
                | "TMP"
                | "TMPDIR"
                | "HTTP_PROXY"
                | "HTTPS_PROXY"
                | "ALL_PROXY"
                | "NO_PROXY"
                | "SSL_CERT_FILE"
                | "SSL_CERT_DIR"
                | "GIT_ASKPASS"
                | "SSH_ASKPASS"
        )
}

pub(super) fn validate_absolute_path(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        bail!("native profile and workspace paths must be absolute without parent traversal");
    }
    Ok(())
}

fn resolve_existing_ancestor(path: &Path) -> Result<PathBuf> {
    let mut candidate = path;
    let mut tail = Vec::new();
    loop {
        match std::fs::symlink_metadata(candidate) {
            Ok(_) => {
                let mut resolved = std::fs::canonicalize(candidate)
                    .map_err(|_| anyhow::anyhow!("native profile path cannot be verified"))?;
                for part in tail.into_iter().rev() {
                    resolved.push(part);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tail.push(
                    candidate.file_name().ok_or_else(|| {
                        anyhow::anyhow!("native profile path has no existing root")
                    })?,
                );
                candidate = candidate
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("native profile path has no existing root"))?;
            }
            Err(_) => bail!("native profile path cannot be verified"),
        }
    }
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        let left = PathBuf::from(left.as_os_str().to_ascii_lowercase());
        let right = PathBuf::from(right.as_os_str().to_ascii_lowercase());
        left.starts_with(&right) || right.starts_with(&left)
    }
    #[cfg(not(windows))]
    {
        left.starts_with(right) || right.starts_with(left)
    }
}

#[derive(Clone)]
pub(super) struct NativeCommand {
    // None is an installation failure, never permission to fall back to a
    // executable in the repository or change the selected native agent.
    pub(super) program: Option<PathBuf>,
    pub(super) prefix: Vec<OsString>,
}

impl NativeCommand {
    pub(super) fn configured(command: PathBuf, prefix: Vec<OsString>) -> Result<Self> {
        if command.as_os_str().is_empty() {
            bail!("native agent executable must be configured by the runner operator");
        }
        let program = if command.is_absolute() {
            Some(command)
        } else if command.components().count() == 1 {
            resolve_on_operator_path(&command)
        } else {
            // Resolve an operator-relative path NOW, before any source cwd is
            // selected. This never resolves a browser-supplied executable.
            Some(
                std::env::current_dir()
                    .map_err(|_| anyhow::anyhow!("cannot resolve configured native executable"))?
                    .join(command),
            )
        };
        Ok(Self { program, prefix })
    }

    pub(super) fn build(
        &self,
        scope: &ProfileEnvironment,
        cwd: &Path,
    ) -> super::ProbeResult<Command> {
        let program = self
            .program
            .as_ref()
            .filter(|path| path.is_file())
            .ok_or(super::ProbeError::NotInstalled)?;
        let mut command = Command::new(program);
        command.args(&self.prefix).current_dir(cwd);
        scope.apply(&mut command);
        Ok(command)
    }
}

fn resolve_on_operator_path(command: &Path) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path).filter(|path| path.is_absolute()) {
        let candidate = directory.join(command);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        if command.extension().is_none() {
            for extension in ["exe", "com", "cmd", "bat"] {
                let candidate = candidate.with_extension(extension);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}
