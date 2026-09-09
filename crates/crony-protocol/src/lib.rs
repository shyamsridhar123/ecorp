use crony_domain::{
    CorpSnapshot, DeliverableSpec, DomainEvent, EntityLink, FactoryController,
    FactoryVerificationRecovery, FactoryVerificationRecoveryMode, FactoryWorkItem,
    FactoryWorkItemState, MissionBudgetRevision, MissionContractRevision,
    MissionContractRevisionAction, PullRequestPublication, RetainedProviderReceiptGrant,
    SourceDeliverable, TaskContract, TaskSecretReference, VerificationPolicy,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub use crony_domain::RunnerModel;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_connection_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveRunClaim {
    pub run_id: Uuid,
    pub assignment_token: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunnerToServer {
    WorkspaceSetupReport {
        runner_id: String,
        corp_id: Uuid,
        connection_epoch: Uuid,
        operation_id: Uuid,
        report: crony_domain::WorkspaceSetupReport,
    },
    WorkspaceSignInAck {
        runner_id: String,
        corp_id: Uuid,
        connection_epoch: Uuid,
        operation_id: Uuid,
        request_id: Uuid,
        applied: bool,
    },
    CapabilitiesUpdated {
        runner_id: String,
        corp_id: Uuid,
        connection_epoch: Uuid,
        capabilities: Vec<RunnerCapability>,
    },
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
    WorkspaceSetup {
        command: crony_domain::WorkspaceSetupCommand,
    },
    WorkspaceSetupAck {
        operation_id: Uuid,
        accepted: bool,
    },
    WorkspaceSignInInput {
        operation_id: Uuid,
        request_id: Uuid,
        response: crony_domain::NativeSignInResponse,
    },
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_connection_id: Option<Uuid>,
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
        #[serde(default)]
        write_scope: Vec<String>,
        deliverable: Option<DeliverableSpec>,
        secrets: Vec<ResolvedSecret>,
    },
    ResumeRun {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_connection_id: Option<Uuid>,
        #[serde(default)]
        command_id: Option<Uuid>,
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
        #[serde(default)]
        expected_workspace_fingerprint: Option<String>,
        #[serde(default)]
        expected_head_commit: Option<String>,
        verification_policy: VerificationPolicy,
        #[serde(default)]
        write_scope: Vec<String>,
        deliverable: Option<DeliverableSpec>,
        secrets: Vec<ResolvedSecret>,
    },
    VerifyRun {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_connection_id: Option<Uuid>,
        command_id: Uuid,
        corp_id: Uuid,
        room_id: Uuid,
        mission_id: Uuid,
        task_id: Uuid,
        run_id: Uuid,
        workspace_run_id: Uuid,
        agent_id: Uuid,
        assignment_token: Uuid,
        source_repository: Option<String>,
        source_base_ref: Option<String>,
        source_base_commit: Option<String>,
        workspace_base_commit: String,
        expected_workspace_fingerprint: String,
        expected_head_commit: Option<String>,
        /// A separately authorized stopped-source checkpoint may create its first
        /// verification commit. The expected HEAD still fences source admission.
        #[serde(default)]
        checkpoint_verification: bool,
        verification_policy: VerificationPolicy,
        #[serde(default)]
        write_scope: Vec<String>,
        deliverable: Option<DeliverableSpec>,
        provider_artifact: Option<VerificationArtifactReference>,
        /// Optional, separately scoped collection of an existing historical
        /// native receipt from this exact stopped-source checkpoint. No provider
        /// session is created or resumed by this operation.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retained_provider_receipt: Option<Box<RetainedProviderReceiptGrant>>,
    },
    CheckpointWorkspace {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_connection_id: Option<Uuid>,
        command_id: Uuid,
        corp_id: Uuid,
        room_id: Uuid,
        mission_id: Uuid,
        task_id: Uuid,
        run_id: Uuid,
        workspace_run_id: Uuid,
        agent_id: Uuid,
        assignment_token: Uuid,
        source_repository: Option<String>,
        source_base_ref: Option<String>,
        source_base_commit: Option<String>,
        workspace_base_commit: String,
        expected_head_commit: String,
    },
    ControlMessage {
        #[serde(default)]
        command_id: Option<Uuid>,
        #[serde(default)]
        message_id: Option<Uuid>,
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

pub const MAX_VERIFICATION_ARTIFACT_BYTES: usize = 16_777_216;

#[derive(Clone, Serialize, Deserialize)]
pub struct VerificationArtifactReference {
    pub path: String,
    pub sha256: String,
    pub bytes: usize,
    pub media_type: String,
    /// Hydrated from verified object storage only for the authenticated runner
    /// dispatch. Durable command records retain metadata, not artifact contents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_base64: Option<String>,
}

impl std::fmt::Debug for VerificationArtifactReference {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerificationArtifactReference")
            .field("path", &self.path)
            .field("sha256", &self.sha256)
            .field("bytes", &self.bytes)
            .field("media_type", &self.media_type)
            .field("has_inline_data", &self.data_base64.is_some())
            .finish()
    }
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
    #[serde(default)]
    pub idempotency_key: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRoomMessageResponse {
    pub message_id: Uuid,
    pub replayed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionSource {
    pub repository: String,
    pub base_ref: String,
    pub base_commit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMissionRequest {
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub requested_by: Uuid,
    pub preferred_adapter: Option<String>,
    pub preferred_model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub strategy: Option<String>,
    #[serde(default)]
    pub source: Option<MissionSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_connection_id: Option<Uuid>,
    #[serde(default)]
    pub secret_refs: Vec<TaskSecretReference>,
    pub budget_tokens: Option<i64>,
    pub budget_cost_microusd: Option<i64>,
    #[serde(default)]
    pub deliverable: Option<DeliverableSpec>,
    #[serde(default)]
    pub contract: Option<FactoryMissionContract>,
    #[serde(default)]
    pub verification_policy: Option<VerificationPolicy>,
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

/// Informational only: creation replans and revalidates current authority.
/// Provisional staffing identities and task contracts are deliberately omitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewMissionResponse {
    pub strategy: String,
    pub budget_tokens: i64,
    pub budget_cost_microusd: i64,
    pub tasks: Vec<PreviewMissionTask>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewMissionTask {
    pub key: String,
    pub title: String,
    pub budget_tokens: i64,
    pub budget_cost_microusd: i64,
    pub depends_on: Vec<String>,
    pub max_attempts: i32,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LookupFactoryWorkItemsRequest {
    pub actor_id: Uuid,
    pub source_project_owner: String,
    pub source_project_number: i64,
    #[serde(default)]
    pub source_project_item_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LookupFactoryWorkItemsResponse {
    pub items: Vec<FactoryWorkItem>,
    pub total_count: usize,
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
pub struct ConfigureFactoryControllerRequest {
    pub actor_id: Uuid,
    pub controller_id: Uuid,
    pub source_project_owner: String,
    pub source_project_number: i64,
    pub source_repository_owner: String,
    pub source_repository_name: String,
    pub connection_epoch: Uuid,
    #[serde(default = "default_factory_controller_lease_seconds")]
    pub lease_seconds: i64,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactoryControllerHeartbeatRequest {
    pub actor_id: Uuid,
    pub connection_epoch: Uuid,
    #[serde(default = "default_factory_controller_lease_seconds")]
    pub lease_seconds: i64,
    pub active_work_item_id: Option<Uuid>,
    pub completed_reconcile_generation: Option<i64>,
    pub reconcile_result: Option<String>,
    pub error: Option<String>,
    /// Omission preserves durable state for legacy clients. A supplied value
    /// is validated and fenced by the controller connection epoch.
    #[serde(default)]
    pub polling: Option<crony_domain::FactoryPollingState>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactoryControllerControlAction {
    Pause,
    Resume,
    Reconcile,
}

impl FactoryControllerControlAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Reconcile => "reconcile",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlFactoryControllerRequest {
    pub actor_id: Uuid,
    pub expected_version: i64,
    pub action: FactoryControllerControlAction,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactoryControllerResponse {
    pub controller: FactoryController,
    pub replayed: bool,
}

const fn default_factory_controller_lease_seconds() -> i64 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointFactoryWorkspaceRequest {
    pub actor_id: Uuid,
    pub claim_token: Uuid,
    pub expected_version: i64,
    pub idempotency_key: String,
    pub source_run_id: Uuid,
    pub expected_head_commit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointFactoryWorkspaceResponse {
    pub work_item: FactoryWorkItem,
    pub claim_token: Option<Uuid>,
    pub source_run_id: Uuid,
    pub command_id: Option<Uuid>,
    pub workspace_fingerprint: Option<String>,
    pub replayed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactoryVerificationRecoveryContextResponse {
    pub work_item: FactoryWorkItem,
    pub recoveries: Vec<FactoryVerificationRecovery>,
    pub mission_id: Uuid,
    pub task_id: Uuid,
    pub source_run_id: Uuid,
    pub remaining_attempts: i32,
    pub remaining_mission_tokens: i64,
    pub remaining_mission_cost_microusd: i64,
    pub workspace_fingerprint: Option<String>,
    pub expected_head_commit: Option<String>,
    #[serde(default)]
    pub checkpoint_verification: bool,
    #[serde(default)]
    pub checkpoint_source_correction: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_verification_available: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_cancellation_event_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconcileFactoryCheckpointCancellationRequest {
    pub actor_id: Uuid,
    pub source_run_id: Uuid,
    pub expected_factory_version: i64,
    pub cancellation_event_id: Uuid,
    pub expected_workspace_fingerprint: String,
    pub expected_head_commit: String,
    pub observed_source_revision: String,
    pub idempotency_key: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateFactoryVerificationRecoveryRequest {
    pub actor_id: Uuid,
    pub claim_token: Uuid,
    pub expected_factory_version: i64,
    pub idempotency_key: Uuid,
    pub source_run_id: Uuid,
    pub mode: FactoryVerificationRecoveryMode,
    pub reason: String,
    pub observed_source_revision: String,
    #[serde(default)]
    pub reviewed_source_snapshot: Value,
    pub contract_revision_id: Option<Uuid>,
    pub expected_workspace_fingerprint: String,
    pub expected_head_commit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateFactoryVerificationRecoveryResponse {
    pub recovery: FactoryVerificationRecovery,
    pub work_item: FactoryWorkItem,
    pub run_id: Uuid,
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
    #[serde(default)]
    pub description: String,
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
    #[serde(default)]
    pub verification_policy: Option<VerificationPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightFactoryMissionRequest {
    pub actor_id: Uuid,
    pub source_repository_owner: String,
    pub source_repository_name: String,
    #[serde(default)]
    pub policy: Value,
    pub title: String,
    #[serde(default)]
    pub description: String,
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
    #[serde(default)]
    pub verification_policy: Option<VerificationPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightFactoryMissionResponse {
    pub valid: bool,
    pub strategy: String,
    pub task_count: usize,
    pub budget_tokens: i64,
    pub budget_cost_microusd: i64,
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
pub struct FactoryPublicationContextResponse {
    pub work_item: FactoryWorkItem,
    pub publication: Option<PullRequestPublication>,
    pub source_deliverables: Vec<SourceDeliverable>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePublicationPublisherCredentialRequest {
    pub actor_id: Uuid,
    pub publisher_id: String,
    #[serde(default = "default_publication_publisher_credential_ttl_seconds")]
    pub expires_in_seconds: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePublicationPublisherCredentialResponse {
    pub credential_id: Uuid,
    pub publisher_id: String,
    pub credential: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevokePublicationPublisherCredentialRequest {
    pub actor_id: Uuid,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevokePublicationPublisherCredentialResponse {
    pub credential_id: Uuid,
    pub publisher_id: String,
    pub revoked: bool,
}

const fn default_publication_publisher_credential_ttl_seconds() -> i64 {
    86_400
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartPullRequestPublicationRequest {
    pub actor_id: Uuid,
    pub source_deliverable_id: Uuid,
    pub target_repository: String,
    pub base_ref: String,
    pub branch: String,
    pub title: String,
    pub body: String,
    pub authorization_id: Uuid,
    pub authorization_reason: String,
    pub effect_key: String,
    pub idempotency_key: String,
    pub publisher_id: String,
    #[serde(default = "default_publication_lease_seconds")]
    pub lease_seconds: i64,
}

const fn default_publication_lease_seconds() -> i64 {
    300
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenewPullRequestPublicationRequest {
    pub actor_id: Uuid,
    pub publisher_token: Uuid,
    pub expected_version: i64,
    pub idempotency_key: String,
    #[serde(default = "default_publication_lease_seconds")]
    pub lease_seconds: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PullRequestPublicationCheckpoint {
    BranchPushed {
        commit_sha: String,
    },
    PullRequestCreated {
        number: i64,
        node_id: String,
        url: String,
        state: String,
        draft: bool,
        title: String,
        body: String,
        head_ref: String,
        base_ref: String,
        head_sha: String,
        head_repository_owner: String,
        is_cross_repository: bool,
        auto_merge_enabled: bool,
    },
    Published {
        project_status: String,
        project_field_id: String,
        project_option_id: String,
    },
    Failed {
        failure_detail: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordPullRequestPublicationCheckpointRequest {
    pub actor_id: Uuid,
    pub publisher_token: Uuid,
    pub expected_version: i64,
    pub idempotency_key: String,
    pub checkpoint: PullRequestPublicationCheckpoint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequestPublicationResponse {
    pub publication: PullRequestPublication,
    pub publisher_token: Option<Uuid>,
    pub replayed: bool,
    pub busy: bool,
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
    #[serde(default)]
    pub replayed: bool,
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
pub struct CreateMissionContractRevisionRequest {
    pub actor_id: Uuid,
    pub task_id: Uuid,
    pub expected_contract_version: i64,
    pub next_action: MissionContractRevisionAction,
    pub source_run_id: Option<Uuid>,
    pub reason: String,
    pub idempotency_key: Uuid,
    pub description: String,
    pub contract: TaskContract,
    pub verification_policy: VerificationPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionContractRevisionResponse {
    pub revision: MissionContractRevision,
    pub replayed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionFinishScopeRevision {
    pub task_id: Uuid,
    pub objective: String,
    pub expected_output: String,
    pub acceptance_tests: Vec<String>,
    pub write_scope: Vec<String>,
    pub budget_tokens: i64,
    pub budget_cost_microusd: i64,
    pub verification_policy: VerificationPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposeMissionBudgetRevisionRequest {
    pub actor_id: Uuid,
    pub expected_budget_tokens: i64,
    pub expected_budget_cost_microusd: i64,
    pub proposed_budget_tokens: i64,
    pub proposed_budget_cost_microusd: i64,
    pub rationale: String,
    pub idempotency_key: Uuid,
    pub finish_scope: Option<MissionFinishScopeRevision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecideMissionBudgetRevisionRequest {
    pub actor_id: Uuid,
    pub expected_version: i64,
    pub approved: bool,
    pub note: String,
    pub decision_key: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionBudgetRevisionResponse {
    pub revision: MissionBudgetRevision,
    pub replayed: bool,
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
    #[serde(default)]
    pub idempotency_key: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueMessageResponse {
    pub message_id: Uuid,
    pub delivery: String,
    pub replayed: bool,
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
    #[serde(default)]
    pub decision_key: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationDecisionResponse {
    pub run_id: Uuid,
    pub status: String,
    pub replayed: bool,
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

#[cfg(test)]
mod tests {
    #[test]
    fn mission_preview_wire_shape_is_only_the_public_graph_summary() {
        let response = super::PreviewMissionResponse {
            strategy: "parallel-specialists".to_owned(),
            budget_tokens: 10_001,
            budget_cost_microusd: 20_003,
            tasks: vec![
                super::PreviewMissionTask {
                    key: "prepare".to_owned(),
                    title: "Prepare".to_owned(),
                    budget_tokens: 4_000,
                    budget_cost_microusd: 8_000,
                    depends_on: Vec::new(),
                    max_attempts: 2,
                },
                super::PreviewMissionTask {
                    key: "deliver".to_owned(),
                    title: "Deliver".to_owned(),
                    budget_tokens: 6_001,
                    budget_cost_microusd: 12_003,
                    depends_on: vec!["prepare".to_owned()],
                    max_attempts: 1,
                },
            ],
        };
        let wire = serde_json::to_value(&response).expect("serialize preview");
        assert_eq!(
            wire,
            serde_json::json!({
                "strategy": "parallel-specialists",
                "budget_tokens": 10_001,
                "budget_cost_microusd": 20_003,
                "tasks": [
                    {
                        "key": "prepare",
                        "title": "Prepare",
                        "budget_tokens": 4_000,
                        "budget_cost_microusd": 8_000,
                        "depends_on": [],
                        "max_attempts": 2
                    },
                    {
                        "key": "deliver",
                        "title": "Deliver",
                        "budget_tokens": 6_001,
                        "budget_cost_microusd": 12_003,
                        "depends_on": ["prepare"],
                        "max_attempts": 1
                    }
                ]
            })
        );
        assert_eq!(
            serde_json::from_value::<super::PreviewMissionResponse>(wire)
                .expect("deserialize preview"),
            response
        );
    }

    #[test]
    fn launch_replay_flag_is_backward_compatible_with_prior_responses() {
        let run_id = uuid::Uuid::new_v4();
        let response: super::LaunchMissionResponse = serde_json::from_value(serde_json::json!({
            "run_id": run_id,
            "runner_id": "fixture",
            "run_ids": [run_id],
            "runner_ids": ["fixture"]
        }))
        .expect("an older server response remains readable");
        assert!(!response.replayed);
        let replay = super::LaunchMissionResponse {
            replayed: true,
            ..response
        };
        assert_eq!(
            serde_json::to_value(replay).expect("serialize replay")["replayed"],
            true
        );
    }

    use super::{ServerToRunner, VerificationArtifactReference};

    #[test]
    fn verifier_artifact_reference_preserves_metadata_only_wire_compatibility() {
        let metadata = serde_json::json!({
            "path": "provider.json",
            "sha256": "0".repeat(64),
            "bytes": 2,
            "media_type": "application/json",
        });
        let reference: VerificationArtifactReference =
            serde_json::from_value(metadata.clone()).expect("legacy artifact reference");
        assert!(reference.data_base64.is_none());
        assert_eq!(
            serde_json::to_value(reference).expect("metadata-only serialization"),
            metadata
        );
    }

    #[test]
    fn verifier_artifact_transfer_is_explicit_in_the_runner_wire_shape() {
        let reference = VerificationArtifactReference {
            path: "provider.json".to_owned(),
            sha256: "0".repeat(64),
            bytes: 2,
            media_type: "application/json".to_owned(),
            data_base64: Some("e30=".to_owned()),
        };
        let wire = serde_json::to_value(reference).expect("inline runner transfer");
        assert_eq!(wire["data_base64"], "e30=");
        let decoded: VerificationArtifactReference =
            serde_json::from_value(wire).expect("inline runner transfer decode");
        assert_eq!(decoded.data_base64.as_deref(), Some("e30="));
    }

    #[test]
    fn verifier_artifact_debug_never_discloses_transferred_bytes() {
        let reference = VerificationArtifactReference {
            path: "provider.json".to_owned(),
            sha256: "0".repeat(64),
            bytes: 2,
            media_type: "application/json".to_owned(),
            data_base64: Some("PRIVATE_ARTIFACT_BYTES".to_owned()),
        };
        let debug = format!("{reference:?}");
        assert!(debug.contains("has_inline_data: true"));
        assert!(!debug.contains("PRIVATE_ARTIFACT_BYTES"));
    }

    #[test]
    fn durable_control_message_accepts_the_legacy_wire_shape() {
        let legacy = serde_json::json!({
            "type": "control_message",
            "corp_id": "00000000-0000-4000-8000-000000000001",
            "run_id": "00000000-0000-4000-8000-000000000002",
            "agent_id": "00000000-0000-4000-8000-000000000003",
            "actor_id": "00000000-0000-4000-8000-000000000004",
            "lease_token": "00000000-0000-4000-8000-000000000005",
            "text": "Continue within the assigned worktree."
        });
        let message: ServerToRunner =
            serde_json::from_value(legacy).expect("legacy control message");
        match message {
            ServerToRunner::ControlMessage {
                command_id,
                message_id,
                ..
            } => {
                assert_eq!(command_id, None);
                assert_eq!(message_id, None);
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }
}
