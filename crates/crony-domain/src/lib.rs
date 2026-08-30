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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mission {
    pub id: Uuid,
    pub corp_id: Uuid,
    pub room_id: Uuid,
    pub requested_by: Uuid,
    pub title: String,
    pub strategy: String,
    pub max_nodes: i32,
    pub max_depth: i32,
    pub budget_tokens: i64,
    pub budget_cost_microusd: i64,
    pub status: MissionStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskContract {
    pub objective: String,
    pub expected_output: String,
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
pub struct TaskGraphPlan {
    pub strategy: String,
    pub max_nodes: i32,
    pub max_depth: i32,
    pub budget_tokens: i64,
    pub budget_cost_microusd: i64,
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
    pub workspace_path: Option<String>,
    pub workspace_branch: Option<String>,
    pub workspace_base_ref: Option<String>,
    pub workspace_base_commit: Option<String>,
    pub workspace_disposition: Option<String>,
    pub workspace_detail: Option<String>,
    pub verification_status: String,
    pub verification_summary: Option<String>,
    pub status: RunStatus,
    pub summary: Option<String>,
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
    pub tasks: Vec<Task>,
    pub runs: Vec<Run>,
    pub room_messages: Vec<RoomMessage>,
    pub leases: Vec<ControlLease>,
    pub queued_messages: Vec<QueuedMessage>,
    pub verification_evidence: Vec<VerificationEvidence>,
    pub verification_requests: Vec<VerificationRequest>,
    pub action_approvals: Vec<ActionApproval>,
    pub circuit_breaker_incidents: Vec<CircuitBreakerIncident>,
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
    }
}
