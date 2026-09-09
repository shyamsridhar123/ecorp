//! GitHub CLI owns authentication and credential exchange. No token extraction.
use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
};

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use crony_domain::{GitHubRepositoryChoice, NativeSignInInstruction, WorkspaceSourceIdentity};
use serde_json::Value;

use super::process::{LineObserver, NativeOutput, bounded_deadline, run_owned};

const PERSONAL_AUTH_ENV: &[&str] = &[
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
    "GH_CONFIG_DIR",
    "GH_HOST",
    "GH_PROMPT_DISABLED",
];
const GIT_ENV: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_SYSTEM",
    "GIT_CONFIG_GLOBAL",
    "GIT_ASKPASS",
    "SSH_ASKPASS",
];

#[derive(Clone)]
pub(super) struct NativeGitHub {
    program: PathBuf,
    cwd: PathBuf,
    home: Option<PathBuf>,
    expected_login: Option<String>,
}

pub(super) struct GitHubIdentity {
    pub login: String,
    /// Native CLI metadata, not an independent assertion of OS/account isolation.
    pub storage: CredentialStorage,
}

#[derive(Clone, Copy)]
pub(super) enum CredentialStorage {
    NativeSecure,
    NativeFile,
    Unknown,
}

impl CredentialStorage {
    pub(super) fn detail(self) -> &'static str {
        match self {
            Self::NativeSecure => {
                "Native GitHub account checked; the CLI reports its secure credential store. Configuration isolation is not a separate OS identity."
            }
            Self::NativeFile => {
                "Native GitHub account checked; the CLI reports file-based credential storage (reduced assurance). Configuration isolation is not a separate OS identity."
            }
            Self::Unknown => {
                "Native GitHub account checked; credential-store assurance was not confirmed. Configuration isolation is not a separate OS identity."
            }
        }
    }
}

pub(super) struct Repository {
    pub choice: GitHubRepositoryChoice,
    pub source: WorkspaceSourceIdentity,
}

pub(super) fn validate_repository(value: &str) -> Result<()> {
    let parts = value.split('/').collect::<Vec<_>>();
    if parts.len() != 2
        || parts.iter().any(|part| {
            part.is_empty()
                || part.len() > 100
                || part.starts_with('-')
                || matches!(*part, "." | "..")
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
    {
        return Err(anyhow!("GitHub repository must be an owner/name identity"));
    }
    Ok(())
}

pub(super) fn valid_commit(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn validate_ref(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 240
        || value.starts_with('-')
        || value.starts_with('/')
        || value.ends_with('/')
        || value.ends_with('.')
        || value.ends_with(".lock")
        || value.contains("..")
        || value.contains("@{")
        || value.contains("//")
        || value == "@"
        || value
            .bytes()
            .any(|byte| byte <= b' ' || byte == 0x7f || b"~^:?*[\\\"".contains(&byte))
    {
        return Err(anyhow!("invalid repository base reference"));
    }
    Ok(())
}

fn safe_account(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn repository_choice(value: &Value) -> Result<GitHubRepositoryChoice> {
    let repository = value["full_name"]
        .as_str()
        .context("GitHub omitted repository identity")?;
    validate_repository(repository)?;
    let id = value["node_id"]
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 200)
        .context("GitHub omitted immutable repository identity")?;
    let default_branch = value["default_branch"]
        .as_str()
        .context("GitHub omitted repository default branch")?;
    validate_ref(default_branch)?;
    Ok(GitHubRepositoryChoice {
        id: id.to_owned(),
        repository: repository.to_owned(),
        default_branch: default_branch.to_owned(),
        private: value["private"]
            .as_bool()
            .context("GitHub omitted repository visibility")?,
        can_push: value["permissions"]["push"].as_bool().unwrap_or(false),
    })
}

impl NativeGitHub {
    pub(super) fn new(
        program: PathBuf,
        cwd: PathBuf,
        home: Option<PathBuf>,
        expected_login: Option<String>,
    ) -> Self {
        Self {
            program,
            cwd,
            home,
            expected_login,
        }
    }

    fn environment(&self, interactive: bool) -> Result<BTreeMap<String, OsString>> {
        let empty_config = self.cwd.join("setup.gitconfig");
        super::storage::reject_links(&empty_config)?;
        if !empty_config.exists() {
            match std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&empty_config)
            {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
        super::storage::reject_links(&empty_config)?;
        let mut env = BTreeMap::from([
            ("GH_HOST".to_owned(), OsString::from("github.com")),
            ("GH_PAGER".to_owned(), OsString::from("cat")),
            ("GIT_TERMINAL_PROMPT".to_owned(), OsString::from("0")),
            ("GIT_CONFIG_NOSYSTEM".to_owned(), OsString::from("1")),
            (
                "GIT_CONFIG_GLOBAL".to_owned(),
                empty_config.into_os_string(),
            ),
        ]);
        if !interactive {
            env.insert("GH_PROMPT_DISABLED".to_owned(), OsString::from("1"));
        }
        if let Some(home) = &self.home {
            env.insert("GH_CONFIG_DIR".to_owned(), home.as_os_str().to_owned());
        }
        Ok(env)
    }

    async fn gh(&self, args: &[&str], expires: DateTime<Utc>) -> Result<NativeOutput> {
        self.gh_observed(args, expires, None).await
    }

    async fn gh_observed(
        &self,
        args: &[&str],
        expires: DateTime<Utc>,
        observer: Option<LineObserver>,
    ) -> Result<NativeOutput> {
        let args = args.iter().map(OsString::from).collect::<Vec<_>>();
        let mut remove = GIT_ENV.to_vec();
        if self.home.is_some() {
            remove.extend_from_slice(PERSONAL_AUTH_ENV);
        }
        run_owned(
            &self.program,
            &[],
            &args,
            &self.cwd,
            &self.environment(false)?,
            &remove,
            bounded_deadline(expires, 30),
            observer,
        )
        .await
    }

    async fn json(&self, endpoint: &str, expires: DateTime<Utc>) -> Result<Value> {
        let result = self
            .gh(
                &[
                    "api",
                    "--hostname",
                    "github.com",
                    "--method",
                    "GET",
                    endpoint,
                ],
                expires,
            )
            .await?;
        if !result.success {
            return Err(anyhow!("native GitHub request was denied or unavailable"));
        }
        serde_json::from_slice(&result.stdout).context("native GitHub response was not valid JSON")
    }

    pub(super) async fn inspect(&self, expires: DateTime<Utc>) -> Result<Option<GitHubIdentity>> {
        if let Some(home) = &self.home {
            let native_state = home.join("hosts.yml");
            super::storage::reject_links(&native_state)?;
            // An empty personal profile must not discover the operator's
            // process-wide keychain identity. Do not read credential bytes.
            if !native_state.is_file() {
                return Ok(None);
            }
        }
        let storage = Arc::new(AtomicU8::new(0));
        let observe_storage = storage.clone();
        let observer: LineObserver = Arc::new(move |line| {
            observe_credential_storage(line, &observe_storage);
        });
        // The plain status exit code reflects authentication errors; --json's does not.
        if !self
            .gh_observed(
                &["auth", "status", "--active", "--hostname", "github.com"],
                expires,
                Some(observer),
            )
            .await?
            .success
        {
            return Ok(None);
        }
        let account = self.json("user", expires).await?;
        let login = account["login"]
            .as_str()
            .filter(|login| safe_account(login))
            .context("native GitHub account identity was missing")?;
        if self
            .expected_login
            .as_ref()
            .is_some_and(|expected| !expected.eq_ignore_ascii_case(login))
        {
            return Err(anyhow!(
                "native GitHub identity changed from its bound actor profile"
            ));
        }
        Ok(Some(GitHubIdentity {
            login: login.to_owned(),
            storage: credential_storage(storage.load(Ordering::Acquire)),
        }))
    }

    pub(super) async fn list(&self, expires: DateTime<Utc>) -> Result<Vec<GitHubRepositoryChoice>> {
        let result = self.json(
            "user/repos?per_page=100&sort=updated&affiliation=owner,collaborator,organization_member",
            expires,
        ).await?;
        let repositories = result
            .as_array()
            .context("GitHub repository response was not a list")?;
        if repositories.len() > 100 {
            return Err(anyhow!("GitHub repository response exceeded its bound"));
        }
        repositories.iter().map(repository_choice).collect()
    }

    pub(super) async fn repository(
        &self,
        name: &str,
        expected_id: Option<&str>,
        base_ref: &str,
        expires: DateTime<Utc>,
    ) -> Result<Repository> {
        validate_repository(name)?;
        validate_ref(base_ref)?;
        let choice = repository_choice(&self.json(&format!("repos/{name}"), expires).await?)?;
        if !choice.repository.eq_ignore_ascii_case(name)
            || expected_id.is_some_and(|expected| expected != choice.id)
        {
            return Err(anyhow!(
                "GitHub repository no longer matches the selected identity"
            ));
        }
        let escaped: String = url::form_urlencoded::byte_serialize(base_ref.as_bytes()).collect();
        let commit = self
            .json(
                &format!("repos/{}/commits/{escaped}", choice.repository),
                expires,
            )
            .await?;
        let sha = commit["sha"]
            .as_str()
            .filter(|sha| valid_commit(sha))
            .context("GitHub omitted a full immutable source commit")?;
        let source = WorkspaceSourceIdentity {
            repository: choice.repository.clone(),
            repository_id: Some(choice.id.clone()),
            base_ref: base_ref.to_owned(),
            base_commit: sha.to_ascii_lowercase(),
        };
        Ok(Repository { choice, source })
    }

    pub(super) async fn clone_repository(
        &self,
        repository: &str,
        target: &Path,
        expires: DateTime<Utc>,
    ) -> Result<()> {
        validate_repository(repository)?;
        let mut args = vec![
            OsString::from("repo"),
            OsString::from("clone"),
            OsString::from(format!("https://github.com/{repository}.git")),
            target.as_os_str().to_owned(),
        ];
        args.extend(["--", "--no-checkout", "--no-hardlinks", "--no-tags"].map(OsString::from));
        // The protected owner/connection namespace can make pack and worktree
        // paths longer than MAX_PATH. This is local to the new managed clone,
        // not a change to the operator's global Git configuration.
        #[cfg(windows)]
        args.push(OsString::from("--config=core.longpaths=true"));
        let mut remove = GIT_ENV.to_vec();
        if self.home.is_some() {
            remove.extend_from_slice(PERSONAL_AUTH_ENV);
        }
        let output = run_owned(
            &self.program,
            &[],
            &args,
            &self.cwd,
            &self.environment(false)?,
            &remove,
            bounded_deadline(expires, 120),
            None,
        )
        .await?;
        if !output.success {
            return Err(anyhow!("native GitHub clone did not complete"));
        }
        Ok(())
    }

    pub(super) async fn git(
        &self,
        cwd: &Path,
        args: &[OsString],
        expires: DateTime<Utc>,
    ) -> Result<NativeOutput> {
        let helper = format!("!{} auth git-credential", shell_word(&self.program)?);
        let mut native_args = vec![
            OsString::from("-c"),
            OsString::from("credential.helper="),
            OsString::from("-c"),
            OsString::from(format!("credential.https://github.com.helper={helper}")),
            OsString::from("-c"),
            OsString::from("core.hooksPath="),
            OsString::from("-c"),
            OsString::from("protocol.ext.allow=never"),
        ];
        #[cfg(windows)]
        native_args.extend(["-c", "core.longpaths=true"].map(OsString::from));
        native_args.extend_from_slice(args);
        let mut remove = GIT_ENV.to_vec();
        if self.home.is_some() {
            remove.extend_from_slice(PERSONAL_AUTH_ENV);
        }
        run_owned(
            Path::new("git"),
            &[],
            &native_args,
            cwd,
            &self.environment(false)?,
            &remove,
            bounded_deadline(expires, 120),
            None,
        )
        .await
    }

    pub(super) async fn sign_in(
        &self,
        expires: DateTime<Utc>,
        progress: Arc<dyn Fn(NativeSignInInstruction) + Send + Sync>,
    ) -> Result<Option<GitHubIdentity>> {
        if self.home.is_none() {
            return Err(anyhow!("machine GitHub login is read-only"));
        }
        if let Some(login) = self.inspect(expires).await? {
            return Ok(Some(login));
        }
        let seen = Arc::new(Mutex::new(None::<String>));
        let storage = Arc::new(AtomicU8::new(0));
        let observe_storage = storage.clone();
        let observe: LineObserver = Arc::new(move |line| {
            observe_credential_storage(line, &observe_storage);
            if let Some(code) = github_device_code(line) {
                let mut last = seen.lock().unwrap_or_else(|error| error.into_inner());
                if last.as_deref() != Some(&code) {
                    *last = Some(code.clone());
                    progress(NativeSignInInstruction {
                        provider: "github".to_owned(),
                        verification_uri: "https://github.com/login/device".to_owned(),
                        user_code: Some(code),
                        expires_at: expires,
                        input_kind: None,
                        input_id: None,
                    });
                }
            }
        });
        // Keep the native secure-store default. GH_CONFIG_DIR scopes configuration,
        // not an OS identity; any native file fallback is reported explicitly.
        let args = [
            "auth",
            "login",
            "--hostname",
            "github.com",
            "--web",
            "--git-protocol",
            "https",
        ]
        .map(OsString::from);
        let mut remove = GIT_ENV.to_vec();
        remove.extend_from_slice(PERSONAL_AUTH_ENV);
        let output = run_owned(
            &self.program,
            &[],
            &args,
            &self.cwd,
            &self.environment(true)?,
            &remove,
            expires,
            Some(observe),
        )
        .await?;
        if !output.success {
            return Ok(None);
        }
        let mut identity = self.inspect(expires).await?;
        if storage.load(Ordering::Acquire) == 2
            && let Some(identity) = &mut identity
        {
            identity.storage = CredentialStorage::NativeFile;
        }
        Ok(identity)
    }
}

fn shell_word(path: &Path) -> Result<String> {
    let text = path
        .to_str()
        .context("GitHub executable path is not valid Unicode")?;
    if text.chars().any(char::is_control) {
        return Err(anyhow!(
            "GitHub executable path contains control characters"
        ));
    }
    // Git credential helpers are native shell snippets. Only the operator-owned
    // executable is quoted here; no setup request or credential is interpolated.
    Ok(format!(
        "'{}'",
        text.replace('\\', "/").replace('\'', "'\"'\"'")
    ))
}

fn github_device_code(line: &str) -> Option<String> {
    if !line.to_ascii_lowercase().contains("one-time code") {
        return None;
    }
    line.split_whitespace().find_map(|word| {
        let word = word.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-');
        (word.len() == 9
            && word.as_bytes()[4] == b'-'
            && word.bytes().enumerate().all(|(index, byte)| {
                index == 4 || byte.is_ascii_uppercase() || byte.is_ascii_digit()
            }))
        .then(|| word.to_owned())
    })
}

fn observe_credential_storage(line: &str, storage: &AtomicU8) {
    let line = line.to_ascii_lowercase();
    if line.contains("hosts.yml") || line.contains("plain text") || line.contains("plaintext") {
        storage.store(2, Ordering::Release);
    } else if line.contains("(keyring)") && storage.load(Ordering::Acquire) != 2 {
        storage.store(1, Ordering::Release);
    }
}

fn credential_storage(value: u8) -> CredentialStorage {
    match value {
        1 => CredentialStorage::NativeSecure,
        2 => CredentialStorage::NativeFile,
        _ => CredentialStorage::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_and_device_instruction_parsing_never_accept_shell_or_token_text() {
        for bad in [
            "../repo",
            "owner/../../repo",
            "https://github.com/a/b",
            "owner/-flag",
            "a/b;c",
        ] {
            assert!(validate_repository(bad).is_err());
        }
        assert!(validate_repository("Team/repository.name").is_ok());
        assert_eq!(
            github_device_code("! First copy your one-time code: ABCD-1234"),
            Some("ABCD-1234".to_owned())
        );
        assert!(github_device_code("token: ABCD-1234").is_none());
        assert!(github_device_code("one-time code: ghp_this_is_not_a_device_code").is_none());
        assert_eq!(shell_word(Path::new("a'b")).unwrap(), "'a'\"'\"'b'");
    }

    #[test]
    fn personal_scope_removes_machine_tokens_without_changing_global_login() {
        assert!(PERSONAL_AUTH_ENV.contains(&"GH_TOKEN"));
        assert!(PERSONAL_AUTH_ENV.contains(&"GITHUB_TOKEN"));
        assert!(PERSONAL_AUTH_ENV.contains(&"GH_CONFIG_DIR"));
        assert!(GIT_ENV.contains(&"GIT_CONFIG_PARAMETERS"));
    }
}
