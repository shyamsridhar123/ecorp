//! Saved execution connections and fixed native setup operations.
//! Account credentials are deliberately absent from every shared type.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodingAgent {
    #[serde(rename = "github-copilot")]
    GitHubCopilot,
    #[serde(rename = "codex")]
    Codex,
    #[serde(rename = "claude-code")]
    ClaudeCode,
}

impl CodingAgent {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GitHubCopilot => "github-copilot",
            Self::Codex => "codex",
            Self::ClaudeCode => "claude-code",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::GitHubCopilot => "GitHub Copilot",
            Self::Codex => "Codex",
            Self::ClaudeCode => "Claude Code",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunnerModel {
    pub id: String,
    pub name: String,
    pub policy_state: Option<String>,
    pub policy_terms: Option<String>,
    pub supports_vision: bool,
    pub supports_reasoning_effort: bool,
    pub max_prompt_tokens: Option<u64>,
    pub max_context_window_tokens: Option<u64>,
    #[serde(default)]
    pub supported_reasoning_efforts: Vec<String>,
    pub default_reasoning_effort: Option<String>,
    pub billing_multiplier: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceConnectionStatus {
    Connecting,
    Ready,
    NeedsSignIn,
    NotInstalled,
    Offline,
    Incompatible,
    Failed,
}

impl WorkspaceConnectionStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Ready => "ready",
            Self::NeedsSignIn => "needs_sign_in",
            Self::NotInstalled => "not_installed",
            Self::Offline => "offline",
            Self::Incompatible => "incompatible",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSourceIdentity {
    pub repository: String,
    #[serde(default)]
    pub repository_id: Option<String>,
    pub base_ref: String,
    pub base_commit: String,
}

/// Public, room-scoped connection state. Local paths, account tokens and native
/// configuration files belong to the private setup/runner boundary, not this DTO.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConnection {
    pub id: Uuid,
    pub corp_id: Uuid,
    pub room_id: Uuid,
    pub created_by: Uuid,
    pub runner_id: String,
    pub label: String,
    pub agent: CodingAgent,
    pub source: Option<WorkspaceSourceIdentity>,
    pub status: WorkspaceConnectionStatus,
    pub detail: String,
    pub models: Vec<RunnerModel>,
    pub version: i64,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub runner_connected: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitHubAccountSource {
    /// Explicit use of the account already configured by this trusted node's
    /// operator. Setup must not switch or overwrite that native account.
    Machine,
    /// Native CLI state isolated for the authenticated actor on the selected node.
    Personal,
}

/// Private configuration, carried only by authorized setup commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkspaceRepositorySetup {
    #[serde(rename = "github")]
    GitHub {
        repository: String,
        #[serde(default)]
        repository_id: Option<String>,
        base_ref: String,
        account: GitHubAccountSource,
    },
    Local {
        directory: String,
        base_ref: String,
    },
    Advertised {
        source: WorkspaceSourceIdentity,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceConnectionConfiguration {
    pub repository: WorkspaceRepositorySetup,
    pub agent: CodingAgent,
    /// Select a known installed native agent, never a browser-supplied command.
    #[serde(default)]
    pub use_system_installation: bool,
    /// Account scope is independent of executable selection. Missing values
    /// preserve the initial connection schema's coupled behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_machine_account: Option<bool>,
}

/// Only fixed setup operations can cross this boundary. There is no shell,
/// executable, arbitrary argument list, environment map or credential value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkspaceSetupAction {
    #[serde(rename = "inspect_github")]
    InspectGitHub { account: GitHubAccountSource },
    #[serde(rename = "sign_in_github")]
    SignInGitHub,
    #[serde(rename = "list_github_repositories")]
    ListGitHubRepositories { account: GitHubAccountSource },
    Connect {
        connection_id: Uuid,
        configuration: WorkspaceConnectionConfiguration,
    },
    Test {
        connection_id: Uuid,
        configuration: WorkspaceConnectionConfiguration,
    },
    SignInAgent {
        connection_id: Uuid,
        configuration: WorkspaceConnectionConfiguration,
    },
}

impl WorkspaceSetupAction {
    pub const fn connection_id(&self) -> Option<Uuid> {
        match self {
            Self::Connect { connection_id, .. }
            | Self::Test { connection_id, .. }
            | Self::SignInAgent { connection_id, .. } => Some(*connection_id),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceSetupStatus {
    Queued,
    Running,
    NeedsSignIn,
    Succeeded,
    Failed,
    Cancelled,
}

impl WorkspaceSetupStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::NeedsSignIn => "needs_sign_in",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub const fn terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitHubRepositoryChoice {
    pub id: String,
    pub repository: String,
    pub default_branch: String,
    pub private: bool,
    pub can_push: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeSignInInputKind {
    AuthorizationCode,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSignInInstruction {
    pub provider: String,
    pub verification_uri: String,
    pub user_code: Option<String>,
    pub expires_at: DateTime<Utc>,
    #[serde(default)]
    pub input_kind: Option<NativeSignInInputKind>,
    #[serde(default)]
    pub input_id: Option<Uuid>,
}

impl std::fmt::Debug for NativeSignInInstruction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeSignInInstruction")
            .field("provider", &self.provider)
            .field("expires_at", &self.expires_at)
            .field("input_kind", &self.input_kind)
            .field("input_id", &self.input_id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSignInResponse {
    pub input_id: Uuid,
    pub authorization_code: String,
}

impl std::fmt::Debug for NativeSignInResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeSignInResponse")
            .field("input_id", &self.input_id)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSetupReport {
    pub status: WorkspaceSetupStatus,
    pub detail: String,
    pub connection_status: Option<WorkspaceConnectionStatus>,
    pub source: Option<WorkspaceSourceIdentity>,
    #[serde(default)]
    pub models: Vec<RunnerModel>,
    pub account_login: Option<String>,
    /// Returned only to the authenticated actor who initiated this operation.
    pub sign_in: Option<NativeSignInInstruction>,
    #[serde(default)]
    pub repositories: Vec<GitHubRepositoryChoice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSetupOperation {
    pub id: Uuid,
    pub corp_id: Uuid,
    pub room_id: Uuid,
    pub actor_id: Uuid,
    pub runner_id: String,
    pub connection_id: Option<Uuid>,
    pub kind: String,
    pub status: WorkspaceSetupStatus,
    pub report: Option<WorkspaceSetupReport>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSetupCommand {
    pub operation_id: Uuid,
    pub corp_id: Uuid,
    pub room_id: Uuid,
    pub actor_id: Uuid,
    pub connection_owner_id: Option<Uuid>,
    pub runner_id: String,
    pub expected_connection_version: Option<i64>,
    pub action: WorkspaceSetupAction,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConnections {
    pub connections: Vec<WorkspaceConnection>,
    pub operations: Vec<WorkspaceSetupOperation>,
    pub selected_connection_id: Option<Uuid>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn workspace_setup_is_a_fixed_native_surface_not_remote_shell() {
        assert!(
            serde_json::from_value::<WorkspaceSetupAction>(json!({
                "kind":"shell","command":"do something"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<WorkspaceSetupAction>(json!({
                "kind":"inspect_github","account":"machine","environment":{"TOKEN":"secret"}
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<WorkspaceConnectionConfiguration>(json!({
                "repository":{"kind":"local","directory":"project","base_ref":"HEAD"},
                "agent":"codex","command":"another executable"
            }))
            .is_err()
        );
    }

    #[test]
    fn setup_progress_is_not_terminal_or_authenticated_readiness() {
        for state in [
            WorkspaceSetupStatus::Queued,
            WorkspaceSetupStatus::Running,
            WorkspaceSetupStatus::NeedsSignIn,
        ] {
            assert!(!state.terminal());
        }
        assert!(WorkspaceSetupStatus::Succeeded.terminal());
        assert_eq!(CodingAgent::GitHubCopilot.as_str(), "github-copilot");
        assert_eq!(
            serde_json::to_value(WorkspaceSetupAction::SignInGitHub).unwrap(),
            json!({"kind":"sign_in_github"})
        );
    }
}
