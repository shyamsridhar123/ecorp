use crony_domain::{
    CorpSnapshot, DeliverableSpec, DomainEvent, EntityLink, FactoryWorkItem, FactoryWorkItemState,
    TaskSecretReference, VerificationPolicy,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerCapability {
    pub name: String,
    pub available: bool,
    pub detail: Option<String>,
    #[serde(default)]
    pub models: Vec<RunnerModel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_base_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_base_commit: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveRunClaim {
    pub run_id: Uuid,
    pub assignment_token: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunnerToServer {
    Register {
        runner_id: String,
        corp_id: Uuid,
        credential: String,
        connection_epoch: Uuid,
        hostname: String,
        os: String,
        capabilities: Vec<RunnerCapability>,
        active_runs: Vec<ActiveRunClaim>,
    },
    Heartbeat {
        runner_id: String,
        connection_epoch: Uuid,
        active_runs: Vec<ActiveRunClaim>,
    },
    CommandAck {
        runner_id: String,
        connection_epoch: Uuid,
        command_id: Uuid,
        applied: bool,
        detail: String,
    },
    RunEvent {
        event_id: Uuid,
        runner_id: String,
        corp_id: Uuid,
        connection_epoch: Uuid,
        run_id: Uuid,
        agent_id: Uuid,
        assignment_token: Uuid,
        event_type: String,
        payload: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerToRunner {
    Registered {
        runner_id: String,
        credential: String,
        expires_at: String,
    },
    RegistrationRejected {
        reason: String,
    },
    ArtifactStored {
        run_id: Uuid,
        artifact_id: Uuid,
        artifact_role: String,
        sha256: String,
    },
    StartRun {
        corp_id: Uuid,
        room_id: Uuid,
        mission_id: Uuid,
        task_id: Uuid,
        run_id: Uuid,
        agent_id: Uuid,
        assignment_token: Uuid,
        adapter: String,
        mission_title: String,
        model: Option<String>,
        reasoning_effort: Option<String>,
        source_repository: Option<String>,
        source_base_ref: Option<String>,
        source_base_commit: Option<String>,
        verification_policy: VerificationPolicy,
        deliverable: Option<DeliverableSpec>,
        secrets: Vec<ResolvedSecret>,
    },
    ResumeRun {
        corp_id: Uuid,
        room_id: Uuid,
        mission_id: Uuid,
        task_id: Uuid,
        run_id: Uuid,
        workspace_run_id: Uuid,
        agent_id: Uuid,
        assignment_token: Uuid,
        adapter: String,
        provider_session_id: String,
        prompt: String,
        model: Option<String>,
        reasoning_effort: Option<String>,
        source_repository: Option<String>,
        source_base_ref: Option<String>,
        source_base_commit: Option<String>,
        workspace_base_commit: Option<String>,
        verification_policy: VerificationPolicy,
        deliverable: Option<DeliverableSpec>,
        secrets: Vec<ResolvedSecret>,
    },
    ControlMessage {
        corp_id: Uuid,
        run_id: Uuid,
        agent_id: Uuid,
        actor_id: Uuid,
        lease_token: Uuid,
        text: String,
    },
    StopRun {
        run_id: Uuid,
        reason: String,
    },
    InterruptRun {
        run_id: Uuid,
        reason: String,
    },
    ApprovalDecision {
        command_id: Uuid,
        run_id: Uuid,
        approval_id: Uuid,
        approved: bool,
        note: String,
    },
    CircuitBreaker {
        command_id: Uuid,
        run_id: Uuid,
        stage: String,
        reason: String,
    },
    Disconnect {
        reason: String,
        reconnect_delay_ms: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerSummary {
    pub id: String,
    pub corp_id: Uuid,
    pub hostname: String,
    pub os: String,
    pub capabilities: Vec<RunnerCapability>,
    pub connected: bool,
    pub status: String,
    pub last_seen_at: String,
    pub grace_expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRunnerEnrollmentRequest {
    pub actor_id: Uuid,
    pub runner_id: String,
    #[serde(default = "default_runner_enrollment_ttl_seconds")]
    pub expires_in_seconds: u64,
}

const fn default_runner_enrollment_ttl_seconds() -> u64 {
    600
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRunnerEnrollmentResponse {
    pub runner_id: String,
    pub enrollment_token: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevokeRunnerRequest {
    pub actor_id: Uuid,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevokeRunnerResponse {
    pub runner_id: String,
    pub revoked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotResponse {
    pub snapshot: CorpSnapshot,
    pub runners: Vec<RunnerSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserSocketMessage {
    Ready {
        corp_id: Uuid,
        replayed_through: i64,
    },
    Event {
        event: Box<DomainEvent>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DemoBootstrapResponse {
    pub corp_id: Uuid,
    pub room_id: Uuid,
    pub alice_actor_id: Uuid,
    pub bob_actor_id: Uuid,
    pub eve_actor_id: Uuid,
    pub manager_agent_id: Uuid,
    pub worker_agent_id: Uuid,
    pub codex_agent_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRoomMessageRequest {
    pub actor_id: Uuid,
    pub body: String,
    pub reply_to_id: Option<Uuid>,
    #[serde(default)]
    pub mentions: Vec<Uuid>,
    pub link: Option<EntityLink>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRoomMessageResponse {
    pub message_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMissionRequest {
    pub title: String,
    pub requested_by: Uuid,
    pub preferred_adapter: Option<String>,
    pub preferred_model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub strategy: Option<String>,
    #[serde(default)]
    pub secret_refs: Vec<TaskSecretReference>,
    pub budget_tokens: Option<i64>,
    pub budget_cost_microusd: Option<i64>,
    #[serde(default)]
    pub deliverable: Option<DeliverableSpec>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ResolvedSecret {
    pub grant_id: Uuid,
    pub secret_id: Uuid,
    pub env_name: String,
    pub value: String,
    pub tool: String,
    pub resource: String,
    pub expires_at: String,
    pub assurance: String,
}

impl std::fmt::Debug for ResolvedSecret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedSecret")
            .field("grant_id", &self.grant_id)
            .field("secret_id", &self.secret_id)
            .field("env_name", &self.env_name)
            .field("value", &"[REDACTED]")
            .field("tool", &self.tool)
            .field("resource", &self.resource)
            .field("expires_at", &self.expires_at)
            .field("assurance", &self.assurance)
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CreateSecretRequest {
    pub actor_id: Uuid,
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub allowed_actor_ids: Vec<Uuid>,
    pub allowed_tools: Vec<String>,
    pub resource_prefix: String,
    #[serde(default = "default_secret_ttl_seconds")]
    pub max_ttl_seconds: u64,
}

impl std::fmt::Debug for CreateSecretRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CreateSecretRequest")
            .field("actor_id", &self.actor_id)
            .field("name", &self.name)
            .field("value", &"[REDACTED]")
            .field("allowed_actor_ids", &self.allowed_actor_ids)
            .field("allowed_tools", &self.allowed_tools)
            .field("resource_prefix", &self.resource_prefix)
            .field("max_ttl_seconds", &self.max_ttl_seconds)
            .finish()
    }
}

const fn default_secret_ttl_seconds() -> u64 {
    300
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSecretResponse {
    pub secret_id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevokeSecretRequest {
    pub actor_id: Uuid,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMissionResponse {
    pub mission_id: Uuid,
    pub task_id: Uuid,
    pub task_ids: Vec<Uuid>,
    pub strategy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimFactoryWorkItemRequest {
    pub actor_id: Uuid,
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
    pub idempotency_key: String,
    #[serde(default = "default_factory_lease_seconds")]
    pub lease_seconds: i64,
    #[serde(default)]
    pub policy: Value,
}

const fn default_factory_lease_seconds() -> i64 {
    300
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenewFactoryWorkItemRequest {
    pub actor_id: Uuid,
    pub claim_token: Uuid,
    pub expected_version: i64,
    pub idempotency_key: String,
    #[serde(default = "default_factory_lease_seconds")]
    pub lease_seconds: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradeFactorySourceCommitRequest {
    pub actor_id: Uuid,
    pub claim_token: Uuid,
    pub expected_version: i64,
    pub idempotency_key: String,
    pub source_base_commit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactoryWorkItemResponse {
    pub work_item: FactoryWorkItem,
    pub claim_token: Option<Uuid>,
    pub replayed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionFactoryWorkItemRequest {
    pub actor_id: Uuid,
    pub claim_token: Uuid,
    pub expected_version: i64,
    pub idempotency_key: String,
    pub state: FactoryWorkItemState,
    pub failure_detail: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FactoryMissionContract {
    #[serde(default)]
    pub objective: String,
    #[serde(default)]
    pub expected_output: String,
    #[serde(default)]
    pub acceptance_tests: Vec<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub prohibited_actions: Vec<String>,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub write_scope: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterializeFactoryMissionRequest {
    pub actor_id: Uuid,
    pub claim_token: Uuid,
    pub expected_version: i64,
    pub idempotency_key: String,
    pub title: String,
    pub preferred_adapter: Option<String>,
    pub preferred_model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub strategy: Option<String>,
    #[serde(default)]
    pub secret_refs: Vec<TaskSecretReference>,
    pub budget_tokens: Option<i64>,
    pub budget_cost_microusd: Option<i64>,
    #[serde(default)]
    pub deliverable: Option<DeliverableSpec>,
    #[serde(default)]
    pub contract: FactoryMissionContract,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterializeFactoryMissionResponse {
    pub work_item: FactoryWorkItem,
    pub mission_id: Uuid,
    pub task_id: Uuid,
    pub task_ids: Vec<Uuid>,
    pub strategy: String,
    pub replayed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchMissionRequest {
    pub requested_by: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchMissionResponse {
    pub run_id: Uuid,
    pub runner_id: String,
    pub run_ids: Vec<Uuid>,
    pub runner_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeRunRequest {
    pub requested_by: Uuid,
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeRunResponse {
    pub run_id: Uuid,
    pub runner_id: String,
    pub provider_session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimLeaseRequest {
    pub actor_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimLeaseResponse {
    pub acquired: bool,
    pub token: Option<Uuid>,
    pub holder_actor_id: Uuid,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseLeaseRequest {
    pub actor_id: Uuid,
    pub token: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferLeaseRequest {
    pub actor_id: Uuid,
    pub token: Uuid,
    pub to_actor_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseMutationResponse {
    pub holder_actor_id: Option<Uuid>,
    pub token: Option<Uuid>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueMessageRequest {
    pub actor_id: Uuid,
    pub lease_token: Option<Uuid>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueMessageResponse {
    pub message_id: Uuid,
    pub delivery: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmergencyStopRequest {
    pub actor_id: Uuid,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmergencyStopResponse {
    pub run_id: Uuid,
    pub requested: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterruptRunRequest {
    pub actor_id: Uuid,
    pub lease_token: Uuid,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterruptRunResponse {
    pub run_id: Uuid,
    pub requested: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationDecisionRequest {
    pub actor_id: Uuid,
    pub approved: bool,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationDecisionResponse {
    pub run_id: Uuid,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionApprovalDecisionRequest {
    pub actor_id: Uuid,
    pub approved: bool,
    pub note: String,
    pub decision_key: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionApprovalDecisionResponse {
    pub approval_id: Uuid,
    pub status: String,
    pub effect_queued: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetBudgetPolicyRequest {
    pub actor_id: Uuid,
    pub actor_tokens_per_24h: i64,
    pub actor_cost_microusd_per_24h: i64,
    pub corp_tokens_per_24h: i64,
    pub corp_cost_microusd_per_24h: i64,
    pub no_progress_event_limit: i32,
    pub repeated_tool_limit: i32,
}
