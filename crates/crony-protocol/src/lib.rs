use crony_domain::{CorpSnapshot, DomainEvent, EntityLink};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerCapability {
    pub name: String,
    pub available: bool,
    pub detail: Option<String>,
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
    RunEvent {
        event_id: Uuid,
        runner_id: String,
        corp_id: Uuid,
        run_id: Uuid,
        agent_id: Uuid,
        event_type: String,
        payload: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerToRunner {
    Registered {
        runner_id: String,
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
    Disconnect {
        reason: String,
        reconnect_delay_ms: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerSummary {
    pub id: String,
    pub hostname: String,
    pub os: String,
    pub capabilities: Vec<RunnerCapability>,
    pub connected: bool,
    pub status: String,
    pub last_seen_at: String,
    pub grace_expires_at: Option<String>,
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
    pub strategy: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMissionResponse {
    pub mission_id: Uuid,
    pub task_id: Uuid,
    pub task_ids: Vec<Uuid>,
    pub strategy: String,
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
