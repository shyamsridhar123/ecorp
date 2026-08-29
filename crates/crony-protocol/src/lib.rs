use crony_domain::{CorpSnapshot, DomainEvent};
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
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunnerToServer {
    Register {
        runner_id: String,
        hostname: String,
        os: String,
        capabilities: Vec<RunnerCapability>,
    },
    Heartbeat {
        runner_id: String,
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
        mission_title: String,
    },
    ControlMessage {
        corp_id: Uuid,
        run_id: Uuid,
        agent_id: Uuid,
        actor_id: Uuid,
        text: String,
    },
    StopRun {
        corp_id: Uuid,
        run_id: Uuid,
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerSummary {
    pub id: String,
    pub hostname: String,
    pub os: String,
    pub capabilities: Vec<RunnerCapability>,
    pub connected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotResponse {
    pub snapshot: CorpSnapshot,
    pub runners: Vec<RunnerSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserSocketMessage {
    Ready { corp_id: Uuid },
    Event { event: Box<DomainEvent> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DemoBootstrapResponse {
    pub corp_id: Uuid,
    pub room_id: Uuid,
    pub alice_actor_id: Uuid,
    pub bob_actor_id: Uuid,
    pub manager_agent_id: Uuid,
    pub worker_agent_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMissionRequest {
    pub title: String,
    pub requested_by: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMissionResponse {
    pub mission_id: Uuid,
    pub task_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchMissionRequest {
    pub requested_by: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchMissionResponse {
    pub run_id: Uuid,
    pub runner_id: String,
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
pub struct QueueMessageRequest {
    pub actor_id: Uuid,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueMessageResponse {
    pub message_id: Uuid,
    pub delivery: String,
}
