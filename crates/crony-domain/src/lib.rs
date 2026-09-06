use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Human,
    Agent,
    Service,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Idle,
    Starting,
    Working,
    Blocked,
    Reviewing,
    Offline,
}

impl AgentStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Starting => "starting",
            Self::Working => "working",
            Self::Blocked => "blocked",
            Self::Reviewing => "reviewing",
            Self::Offline => "offline",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionStatus {
    Draft,
    Ready,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl MissionStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Ready,
    Claimed,
    Running,
    Blocked,
    AwaitingApproval,
    VerificationFailed,
    Review,
    Completed,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ready => "ready",
            Self::Claimed => "claimed",
            Self::Running => "running",
            Self::Blocked => "blocked",
            Self::AwaitingApproval => "awaiting_approval",
            Self::VerificationFailed => "verification_failed",
            Self::Review => "review",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Provisioning,
    Starting,
    Running,
    WaitingForInput,
    WaitingForApproval,
    Verifying,
    Completed,
    Failed,
    Cancelled,
    Lost,
}

impl RunStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Provisioning => "provisioning",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::WaitingForInput => "waiting_for_input",
            Self::WaitingForApproval => "waiting_for_approval",
            Self::Verifying => "verifying",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Lost => "lost",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactoryWorkItemState {
    Claimed,
    MissionCreated,
    Running,
    Blocked,
    AwaitingApproval,
    VerificationFailed,
    Verified,
    Publishing,
    Published,
    Failed,
    Cancelled,
}

impl FactoryWorkItemState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claimed => "claimed",
            Self::MissionCreated => "mission_created",
            Self::Running => "running",
            Self::Blocked => "blocked",
            Self::AwaitingApproval => "awaiting_approval",
            Self::VerificationFailed => "verification_failed",
            Self::Verified => "verified",
            Self::Publishing => "publishing",
            Self::Published => "published",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PullRequestPublicationState {
    Requested,
    Publishing,
    BranchPushed,
    PullRequestCreated,
    Published,
}

impl PullRequestPublicationState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Publishing => "publishing",
            Self::BranchPushed => "branch_pushed",
            Self::PullRequestCreated => "pull_request_created",
            Self::Published => "published",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Corp {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Actor {
    pub id: Uuid,
    pub corp_id: Uuid,
    pub name: String,
    pub kind: ActorKind,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Room {
    pub id: Uuid,
    pub corp_id: Uuid,
    pub name: String,
    pub purpose: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: Uuid,
    pub corp_id: Uuid,
    pub actor_id: Uuid,
    pub name: String,
    pub role: String,
    pub adapter: String,
    pub status: AgentStatus,
    pub station: Option<String>,
    pub current_run_id: Option<Uuid>,
    pub accent: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub mission_id: Option<Uuid>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub retired_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mission {
    pub id: Uuid,
    pub corp_id: Uuid,
    pub room_id: Uuid,
    pub requested_by: Uuid,
    pub title: String,
    pub description: String,
    pub specification_version: i64,
    pub strategy: String,
    pub max_nodes: i32,
    pub max_depth: i32,
    pub original_budget_tokens: i64,
    pub original_budget_cost_microusd: i64,
    pub budget_tokens: i64,
    pub budget_cost_microusd: i64,
    pub status: MissionStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionContractRevisionAction {
    Redispatch,
    Resume,
}

impl MissionContractRevisionAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Redispatch => "redispatch",
            Self::Resume => "resume",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionContractRevision {
    pub id: Uuid,
    pub corp_id: Uuid,
    pub mission_id: Uuid,
    pub task_id: Uuid,
    pub version: i64,
    pub revised_by: Uuid,
    pub next_action: MissionContractRevisionAction,
    pub source_run_id: Option<Uuid>,
    pub reason: String,
    pub previous_description: String,
    pub replacement_description: String,
    pub previous_contract: TaskContract,
    pub replacement_contract: TaskContract,
    pub previous_verification_policy: VerificationPolicy,
    pub replacement_verification_policy: VerificationPolicy,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionBudgetRevision {
    pub id: Uuid,
    pub corp_id: Uuid,
    pub mission_id: Uuid,
    pub proposed_by: Uuid,
    pub status: String,
    pub version: i64,
    pub current_budget_tokens: i64,
    pub current_budget_cost_microusd: i64,
    pub proposed_budget_tokens: i64,
    pub proposed_budget_cost_microusd: i64,
    pub consumed_tokens_at_proposal: i64,
    pub consumed_cost_microusd_at_proposal: i64,
    pub rationale: String,
    pub replacement_task_id: Option<Uuid>,
    pub previous_contract: Option<TaskContract>,
    pub replacement_contract: Option<TaskContract>,
    pub previous_verification_policy: Option<VerificationPolicy>,
    pub replacement_verification_policy: Option<VerificationPolicy>,
    pub decided_by: Option<Uuid>,
    pub decision_note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactoryWorkItem {
    pub id: Uuid,
    pub corp_id: Uuid,
    pub source_kind: String,
    pub source_project_owner: String,
    pub source_project_number: i64,
    pub source_project_item_id: String,
    pub source_repository_owner: String,
    pub source_repository_name: String,
    pub source_issue_number: i64,
    pub source_issue_node_id: String,
    pub source_issue_url: String,
    pub source_title: String,
    pub source_revision: String,
    pub state: FactoryWorkItemState,
    pub version: i64,
    pub claim_owner_id: Uuid,
    pub lease_expires_at: DateTime<Utc>,
    pub policy: Value,
    pub mission_id: Option<Uuid>,
    pub failure_detail: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactoryController {
    pub id: Uuid,
    pub corp_id: Uuid,
    pub service_actor_id: Uuid,
    pub configured_by: Uuid,
    pub source_project_owner: String,
    pub source_project_number: i64,
    pub source_repository_owner: String,
    pub source_repository_name: String,
    pub desired_state: String,
    pub status: String,
    pub version: i64,
    pub lease_expires_at: DateTime<Utc>,
    pub last_heartbeat_at: DateTime<Utc>,
    pub reconcile_generation: i64,
    pub completed_reconcile_generation: i64,
    pub active_work_item_id: Option<Uuid>,
    pub reconcile_started_at: Option<DateTime<Utc>>,
    pub last_reconciled_at: Option<DateTime<Utc>>,
    pub last_reconcile_result: Option<String>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskContract {
    pub objective: String,
    pub expected_output: String,
    #[serde(default)]
    pub source_repository: Option<String>,
    #[serde(default)]
    pub source_base_ref: Option<String>,
    #[serde(default)]
    pub source_base_commit: Option<String>,
    pub acceptance_tests: Vec<String>,
    pub allowed_tools: Vec<String>,
    pub prohibited_actions: Vec<String>,
    pub references: Vec<String>,
    pub write_scope: Vec<String>,
    pub budget_tokens: i64,
    #[serde(default = "default_task_cost_budget")]
    pub budget_cost_microusd: i64,
    pub deadline_at: Option<DateTime<Utc>>,
    pub escalation: String,
    #[serde(default)]
    pub secret_refs: Vec<TaskSecretReference>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub deliverable: Option<DeliverableSpec>,
}

impl TaskContract {
    pub fn normalize_for_adapter(&mut self, adapter: &str) {
        if adapter == "fake-process" {
            self.model = None;
            self.reasoning_effort = None;
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliverableForm {
    CommitBranch,
    Patch,
    Archive,
    TypedArtifactSet,
    #[default]
    ReviewOnlyReport,
}

impl DeliverableForm {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CommitBranch => "commit_branch",
            Self::Patch => "patch",
            Self::Archive => "archive",
            Self::TypedArtifactSet => "typed_artifact_set",
            Self::ReviewOnlyReport => "review_only_report",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliverableSpec {
    #[serde(default)]
    pub form: DeliverableForm,
    #[serde(default)]
    pub commit_after_verification: bool,
    #[serde(default)]
    pub paths: Vec<String>,
}

pub fn repository_relative_path_is_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 500
        && value == value.trim()
        && !value.starts_with('/')
        && !value.starts_with('\\')
        && !value.contains('\\')
        && !value.contains(':')
        && !value.contains("//")
        && !value.chars().any(char::is_control)
        && value
            .split('/')
            .all(|component| !component.is_empty() && !matches!(component, "." | ".."))
}

pub fn write_scope_is_valid(value: &str) -> bool {
    if value == "**" {
        return true;
    }
    let path = value.strip_suffix("/**").unwrap_or(value);
    repository_relative_path_is_valid(path)
        && !path
            .chars()
            .any(|character| matches!(character, '*' | '?' | '[' | ']'))
}

pub fn write_scope_allows_path(scope: &str, path: &str) -> bool {
    if scope == "**" || scope == path {
        return true;
    }
    let Some(prefix) = scope.strip_suffix("/**") else {
        return false;
    };
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSecretReference {
    pub secret_id: Uuid,
    pub env_name: String,
    pub tool: String,
    pub resource: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedTask {
    pub key: String,
    pub title: String,
    pub contract: TaskContract,
    pub assigned_agent_id: Uuid,
    pub required_adapter: String,
    pub depends_on: Vec<String>,
    pub depth: i32,
    pub max_attempts: i32,
    pub verification_policy: VerificationPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedAgent {
    pub id: Uuid,
    pub name: String,
    pub role: String,
    pub adapter: String,
    pub accent: String,
}

pub fn write_scope_is_subset(requested: &str, allowed: &str) -> bool {
    if !write_scope_is_valid(requested) || !write_scope_is_valid(allowed) {
        return false;
    }
    if requested == "**" {
        return allowed == "**";
    }
    if let Some(prefix) = requested.strip_suffix("/**") {
        return (allowed == "**" || allowed.ends_with("/**"))
            && write_scope_allows_path(allowed, prefix);
    }
    write_scope_allows_path(allowed, requested)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskGraphPlan {
    pub strategy: String,
    pub max_nodes: i32,
    pub max_depth: i32,
    pub budget_tokens: i64,
    pub budget_cost_microusd: i64,
    /// Provisional identities. Only mission materialization may persist them.
    #[serde(default)]
    pub staffing: Vec<PlannedAgent>,
    pub tasks: Vec<PlannedTask>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VerifierCheck {
    Artifact {
        min_bytes: u64,
    },
    File {
        path: String,
        min_bytes: u64,
    },
    Command {
        program: String,
        args: Vec<String>,
        timeout_ms: u64,
    },
    Test {
        program: String,
        args: Vec<String>,
        timeout_ms: u64,
    },
    JsonSchema {
        path: String,
        required_keys: Vec<String>,
    },
    Screenshot {
        path: String,
        min_bytes: u64,
    },
}

impl VerifierCheck {
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Artifact { .. } => "artifact",
            Self::File { .. } => "file",
            Self::Command { .. } => "command",
            Self::Test { .. } => "test",
            Self::JsonSchema { .. } => "json_schema",
            Self::Screenshot { .. } => "screenshot",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ManualVerificationGate {
    HumanApproval {
        roles: Vec<String>,
    },
    IndependentReview {
        roles: Vec<String>,
        exclude_requester: bool,
    },
}

impl ManualVerificationGate {
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::HumanApproval { .. } => "human_approval",
            Self::IndependentReview { .. } => "independent_review",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationPolicy {
    pub checks: Vec<VerifierCheck>,
    pub manual_gate: Option<ManualVerificationGate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub mission_id: Uuid,
    pub corp_id: Uuid,
    pub title: String,
    pub objective: String,
    pub plan_key: String,
    pub contract: TaskContract,
    pub contract_version: i64,
    pub depth: i32,
    pub max_attempts: i32,
    pub attempt_count: i32,
    pub required_adapter: Option<String>,
    pub depends_on: Vec<Uuid>,
    pub verification_policy: VerificationPolicy,
    pub verification_status: String,
    pub status: TaskStatus,
    pub assigned_agent_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub id: Uuid,
    pub corp_id: Uuid,
    pub task_id: Uuid,
    pub agent_id: Uuid,
    pub runner_id: String,
    #[serde(skip_serializing)]
    pub assignment_token: Uuid,
    pub provider_session_id: Option<String>,
    pub resumed_from_run_id: Option<Uuid>,
    pub workspace_run_id: Uuid,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_microusd: i64,
    pub budget_tokens_limit: i64,
    pub budget_cost_microusd_limit: i64,
    pub breaker_stage: String,
    pub no_progress_events: i32,
    pub repeated_tool_count: i32,
    pub source_repository: Option<String>,
    pub source_base_ref: Option<String>,
    pub source_base_commit: Option<String>,
    pub workspace_path: Option<String>,
    pub workspace_branch: Option<String>,
    pub workspace_base_ref: Option<String>,
    pub workspace_base_commit: Option<String>,
    pub workspace_disposition: Option<String>,
    pub workspace_detail: Option<String>,
    pub verification_status: String,
    pub verification_summary: Option<String>,
    pub verification_sha256: Option<String>,
    pub deliverable_sha256: Option<String>,
    pub status: RunStatus,
    pub summary: Option<String>,
    pub artifact_id: Option<Uuid>,
    pub artifact_uri: Option<String>,
    pub artifact_media_type: Option<String>,
    pub artifact_signature: Option<String>,
    #[serde(skip_serializing)]
    pub artifact_path: Option<String>,
    pub artifact_sha256: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlLease {
    pub agent_id: Uuid,
    pub corp_id: Uuid,
    pub actor_id: Uuid,
    #[serde(skip_serializing)]
    pub token: Uuid,
    #[serde(skip_serializing)]
    pub lease_version: i64,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedMessage {
    pub id: Uuid,
    pub corp_id: Uuid,
    pub agent_id: Uuid,
    pub actor_id: Uuid,
    pub text: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityLink {
    pub kind: String,
    pub id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomMessage {
    pub id: Uuid,
    pub corp_id: Uuid,
    pub room_id: Uuid,
    pub actor_id: Uuid,
    pub thread_root_id: Option<Uuid>,
    pub reply_to_id: Option<Uuid>,
    pub body: String,
    pub mentions: Vec<Uuid>,
    pub link: Option<EntityLink>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationEvidence {
    pub id: Uuid,
    pub corp_id: Uuid,
    pub task_id: Uuid,
    pub run_id: Uuid,
    pub check_index: i32,
    pub kind: String,
    pub status: String,
    pub summary: String,
    pub payload: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationRequest {
    pub run_id: Uuid,
    pub corp_id: Uuid,
    pub task_id: Uuid,
    pub gate_type: String,
    pub gate: ManualVerificationGate,
    pub status: String,
    pub requested_at: DateTime<Utc>,
    pub decided_by: Option<Uuid>,
    pub decision_note: Option<String>,
    pub decided_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceDeliverable {
    pub id: Uuid,
    pub corp_id: Uuid,
    pub task_id: Uuid,
    pub run_id: Uuid,
    pub artifact_id: Uuid,
    pub form: DeliverableForm,
    pub file_name: String,
    pub uri: String,
    pub sha256: String,
    pub media_type: String,
    pub bytes: i64,
    pub provenance_signature: String,
    pub verification_sha256: String,
    pub base_commit: String,
    pub head_commit: Option<String>,
    pub branch: String,
    pub integration_state: String,
    pub retention_until: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequestPublication {
    pub id: Uuid,
    pub corp_id: Uuid,
    pub factory_work_item_id: Uuid,
    pub mission_id: Uuid,
    pub source_deliverable_id: Uuid,
    pub artifact_id: Uuid,
    pub task_id: Uuid,
    pub run_id: Uuid,
    pub source_issue_number: i64,
    pub source_issue_url: String,
    pub target_repository: String,
    pub base_ref: String,
    pub branch: String,
    pub commit_sha: String,
    pub title: String,
    pub body: String,
    pub actor_id: Uuid,
    pub authorization_id: Uuid,
    pub authorization_snapshot: Value,
    pub effect_key: String,
    pub idempotency_key: String,
    pub state: PullRequestPublicationState,
    pub version: i64,
    pub attempt_count: i32,
    pub publisher_id: Option<String>,
    pub publisher_lease_expires_at: Option<DateTime<Utc>>,
    pub failure_detail: Option<String>,
    pub branch_pushed_at: Option<DateTime<Utc>>,
    pub pull_request_number: Option<i64>,
    pub pull_request_node_id: Option<String>,
    pub pull_request_url: Option<String>,
    pub pull_request_state: Option<String>,
    pub pull_request_draft: Option<bool>,
    pub pull_request_base_ref: Option<String>,
    pub pull_request_head_sha: Option<String>,
    pub pull_request_head_repository_owner: Option<String>,
    pub pull_request_is_cross_repository: Option<bool>,
    pub project_owner: String,
    pub project_number: i64,
    pub project_item_id: String,
    pub project_status_before: String,
    pub project_status_after: Option<String>,
    pub project_status_updated_at: Option<DateTime<Utc>>,
    pub auto_merge_enabled: bool,
    pub merge_authorized: bool,
    pub deployment_authorized: bool,
    pub provenance: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequestPublicationAttempt {
    pub id: Uuid,
    pub corp_id: Uuid,
    pub publication_id: Uuid,
    pub attempt: i32,
    pub actor_id: Uuid,
    pub authorization_id: Uuid,
    pub authorization_snapshot: Value,
    pub publisher_id: String,
    pub state: String,
    pub failure_detail: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

const fn default_task_cost_budget() -> i64 {
    1_000_000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionApproval {
    pub id: Uuid,
    pub corp_id: Uuid,
    pub room_id: Uuid,
    pub mission_id: Uuid,
    pub task_id: Uuid,
    pub run_id: Uuid,
    pub agent_id: Uuid,
    pub action_key: String,
    pub action: String,
    pub risk: String,
    pub rationale: String,
    pub required_roles: Vec<String>,
    pub status: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub decided_by: Option<Uuid>,
    pub decision_note: Option<String>,
    pub decided_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerIncident {
    pub id: Uuid,
    pub corp_id: Uuid,
    pub mission_id: Uuid,
    pub task_id: Uuid,
    pub run_id: Uuid,
    pub stage: String,
    pub reason: String,
    pub input: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainEvent {
    pub seq: i64,
    pub id: Uuid,
    pub schema_version: i32,
    pub corp_id: Uuid,
    pub room_id: Option<Uuid>,
    pub actor_id: Option<Uuid>,
    #[serde(rename = "type")]
    pub event_type: String,
    pub aggregate_type: String,
    pub aggregate_id: Uuid,
    pub aggregate_version: i64,
    pub correlation_id: Option<Uuid>,
    pub causation_id: Option<Uuid>,
    pub idempotency_key: String,
    pub visibility: String,
    pub payload: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewEvent {
    pub id: Uuid,
    pub corp_id: Uuid,
    pub room_id: Option<Uuid>,
    pub actor_id: Option<Uuid>,
    pub event_type: String,
    pub aggregate_type: String,
    pub aggregate_id: Uuid,
    pub aggregate_version: i64,
    pub correlation_id: Option<Uuid>,
    pub causation_id: Option<Uuid>,
    pub idempotency_key: String,
    pub visibility: String,
    pub payload: Value,
}

impl NewEvent {
    pub fn new(
        corp_id: Uuid,
        actor_id: Option<Uuid>,
        event_type: impl Into<String>,
        aggregate_type: impl Into<String>,
        aggregate_id: Uuid,
        idempotency_key: impl Into<String>,
        payload: Value,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            corp_id,
            room_id: None,
            actor_id,
            event_type: event_type.into(),
            aggregate_type: aggregate_type.into(),
            aggregate_id,
            aggregate_version: 1,
            correlation_id: None,
            causation_id: None,
            idempotency_key: idempotency_key.into(),
            visibility: "corp".to_owned(),
            payload,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpSnapshot {
    pub corp: Corp,
    pub actors: Vec<Actor>,
    pub rooms: Vec<Room>,
    pub agents: Vec<Agent>,
    pub missions: Vec<Mission>,
    #[serde(default)]
    pub mission_contract_revisions: Vec<MissionContractRevision>,
    pub mission_budget_revisions: Vec<MissionBudgetRevision>,
    pub tasks: Vec<Task>,
    pub runs: Vec<Run>,
    pub room_messages: Vec<RoomMessage>,
    pub leases: Vec<ControlLease>,
    pub queued_messages: Vec<QueuedMessage>,
    pub verification_evidence: Vec<VerificationEvidence>,
    pub verification_requests: Vec<VerificationRequest>,
    #[serde(default)]
    pub source_deliverables: Vec<SourceDeliverable>,
    #[serde(default)]
    pub pull_request_publications: Vec<PullRequestPublication>,
    #[serde(default)]
    pub pull_request_publication_attempts: Vec<PullRequestPublicationAttempt>,
    pub action_approvals: Vec<ActionApproval>,
    pub circuit_breaker_incidents: Vec<CircuitBreakerIncident>,
    #[serde(default)]
    pub factory_work_items: Vec<FactoryWorkItem>,
    #[serde(default)]
    pub factory_controllers: Vec<FactoryController>,
    pub events: Vec<DomainEvent>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_serialization_is_stable() {
        assert_eq!(
            serde_json::to_string(&TaskStatus::AwaitingApproval).expect("serialize"),
            "\"awaiting_approval\""
        );
        assert_eq!(RunStatus::WaitingForInput.as_str(), "waiting_for_input");
        assert_eq!(
            serde_json::to_string(&FactoryWorkItemState::MissionCreated)
                .expect("serialize factory state"),
            "\"mission_created\""
        );
        assert_eq!(
            serde_json::to_string(&PullRequestPublicationState::PullRequestCreated)
                .expect("serialize publication state"),
            "\"pull_request_created\""
        );
    }

    #[test]
    fn portable_paths_and_write_scopes_have_one_shared_grammar() {
        assert!(repository_relative_path_is_valid("src/main.rs"));
        assert!(repository_relative_path_is_valid(
            "folder with spaces/file.txt"
        ));
        assert!(!repository_relative_path_is_valid("../escape"));
        assert!(!repository_relative_path_is_valid(":(exclude)secret.txt"));
        assert!(!repository_relative_path_is_valid("C:/outside.txt"));
        assert!(!repository_relative_path_is_valid("src\\outside.txt"));

        assert!(write_scope_is_valid("**"));
        assert!(write_scope_is_valid("src/**"));
        assert!(write_scope_is_valid("README.md"));
        assert!(!write_scope_is_valid("src/*.rs"));
        assert!(!write_scope_is_valid("src/[ab].rs"));
        assert!(!write_scope_is_valid("src/../secret/**"));
        assert!(write_scope_allows_path("src/**", "src/lib.rs"));
        assert!(!write_scope_allows_path("src/**", "src2/lib.rs"));
        assert!(write_scope_is_subset("src/handoffs/visual.md", "src/**"));
        assert!(write_scope_is_subset("src/handoffs/**", "src/**"));
        assert!(!write_scope_is_subset("src/**", "src/handoffs/**"));
        assert!(!write_scope_is_subset("src/**", "src"));
        assert!(!write_scope_is_subset("src2/**", "src/**"));
        assert!(!write_scope_is_subset("**", "src/**"));
        assert!(!write_scope_is_subset("src/../private.txt", "**"));
    }
}
