use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result, anyhow};
use chrono::{Duration, Utc};
use crony_domain::{
    Actor, ActorKind, Agent, AgentStatus, ControlLease, Corp, CorpSnapshot, DomainEvent,
    EntityLink, ManualVerificationGate, Mission, MissionStatus, NewEvent, QueuedMessage, Room,
    RoomMessage, Run, RunStatus, Task, TaskContract, TaskGraphPlan, TaskStatus,
    VerificationEvidence, VerificationPolicy, VerificationRequest,
};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Row, Transaction, postgres::PgPoolOptions};
use uuid::Uuid;

const DEMO_CORP_ID: &str = "00000000-0000-4000-8000-000000000001";
const DEMO_ALICE_ID: &str = "00000000-0000-4000-8000-000000000011";
const DEMO_BOB_ID: &str = "00000000-0000-4000-8000-000000000012";
const DEMO_EVE_ID: &str = "00000000-0000-4000-8000-000000000013";
const DEMO_MANAGER_ACTOR_ID: &str = "00000000-0000-4000-8000-000000000021";
const DEMO_WORKER_ACTOR_ID: &str = "00000000-0000-4000-8000-000000000022";
const DEMO_CODEX_ACTOR_ID: &str = "00000000-0000-4000-8000-000000000023";
const DEMO_MANAGER_AGENT_ID: &str = "00000000-0000-4000-8000-000000000031";
const DEMO_WORKER_AGENT_ID: &str = "00000000-0000-4000-8000-000000000032";
const DEMO_CODEX_AGENT_ID: &str = "00000000-0000-4000-8000-000000000033";
const DEMO_ROOM_ID: &str = "00000000-0000-4000-8000-000000000041";
const DEMO_ADVISORY_LOCK: i64 = 0x4352_4F4E_5944_4D4F;

#[derive(Clone)]
pub struct PgStore {
    pool: PgPool,
}

#[derive(Debug, Clone)]
pub struct DemoIds {
    pub corp_id: Uuid,
    pub room_id: Uuid,
    pub alice_actor_id: Uuid,
    pub bob_actor_id: Uuid,
    pub eve_actor_id: Uuid,
    pub manager_agent_id: Uuid,
    pub worker_agent_id: Uuid,
    pub codex_agent_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct MissionPlanIds {
    pub mission_id: Uuid,
    pub task_ids: Vec<Uuid>,
}

#[derive(Debug, Clone)]
pub struct LaunchRecord {
    pub corp_id: Uuid,
    pub room_id: Uuid,
    pub mission_id: Uuid,
    pub task_id: Uuid,
    pub run_id: Uuid,
    pub agent_id: Uuid,
    pub assignment_token: Uuid,
    pub attempt: i32,
    pub adapter: String,
    pub mission_title: String,
    pub verification_policy: VerificationPolicy,
}

#[derive(Debug, Clone)]
pub struct SchedulableTask {
    pub task_id: Uuid,
    pub required_adapter: String,
}

#[derive(Debug, Clone)]
pub struct ResumeLaunchRecord {
    pub corp_id: Uuid,
    pub room_id: Uuid,
    pub mission_id: Uuid,
    pub task_id: Uuid,
    pub run_id: Uuid,
    pub source_run_id: Uuid,
    pub workspace_run_id: Uuid,
    pub agent_id: Uuid,
    pub runner_id: String,
    pub assignment_token: Uuid,
    pub adapter: String,
    pub provider_session_id: String,
    pub verification_policy: VerificationPolicy,
}

#[derive(Debug, Clone)]
pub struct LeaseOutcome {
    pub acquired: bool,
    pub lease: ControlLease,
    pub event: Option<DomainEvent>,
}

#[derive(Debug, Clone)]
pub struct LeaseMutationOutcome {
    pub lease: Option<ControlLease>,
    pub event: DomainEvent,
}

#[derive(Debug, Clone)]
pub struct MessageOutcome {
    pub message: QueuedMessage,
    pub delivery: String,
    pub run_id: Option<Uuid>,
    pub runner_id: Option<String>,
    pub event: DomainEvent,
}

#[derive(Debug, Clone)]
pub struct StopRequestOutcome {
    pub run_id: Uuid,
    pub runner_id: String,
    pub event: DomainEvent,
}

#[derive(Debug, Clone)]
pub struct VerificationDecisionOutcome {
    pub run_id: Uuid,
    pub corp_id: Uuid,
    pub mission_id: Uuid,
    pub status: String,
    pub event: DomainEvent,
}

#[derive(Debug, Clone)]
pub struct RoomMessageOutcome {
    pub message: RoomMessage,
    pub event: DomainEvent,
}

#[derive(Debug, Clone)]
pub struct NewRoomMessageInput {
    pub corp_id: Uuid,
    pub room_id: Uuid,
    pub actor_id: Uuid,
    pub body: String,
    pub reply_to_id: Option<Uuid>,
    pub mentions: Vec<Uuid>,
    pub link: Option<EntityLink>,
}

#[derive(Debug, Clone)]
pub struct RunnerRecord {
    pub id: String,
    pub hostname: String,
    pub os: String,
    pub capabilities: Value,
    pub connection_epoch: Uuid,
    pub status: String,
    pub last_seen_at: chrono::DateTime<Utc>,
    pub grace_expires_at: Option<chrono::DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct RunClaim {
    pub run_id: Uuid,
    pub assignment_token: Uuid,
}

#[derive(Debug, Clone)]
pub struct RunnerConnectInput {
    pub id: String,
    pub hostname: String,
    pub os: String,
    pub capabilities: Value,
    pub connection_epoch: Uuid,
}

#[derive(Debug, Clone)]
pub struct RunnerReconcileOutcome {
    pub accepted: Vec<Uuid>,
    pub stale: Vec<Uuid>,
    pub events: Vec<DomainEvent>,
}

#[derive(Debug, Clone)]
pub struct RunnerEventInput {
    pub event_id: Uuid,
    pub runner_id: String,
    pub corp_id: Uuid,
    pub run_id: Uuid,
    pub agent_id: Uuid,
    pub event_type: String,
    pub payload: Value,
}

impl PgStore {
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await
            .context("connect to Postgres")?;
        Ok(Self { pool })
    }

    pub async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("../../db/migrations")
            .run(&self.pool)
            .await
            .context("run database migrations")
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn agents_for_planning(&self, corp_id: Uuid) -> Result<Vec<Agent>> {
        sqlx::query(
            r#"
            SELECT id, corp_id, actor_id, name, role, adapter, status, station,
                   current_run_id, accent, created_at
            FROM agents
            WHERE corp_id = $1
            ORDER BY role, adapter, name, id
            "#,
        )
        .bind(corp_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(map_agent)
        .collect()
    }

    pub async fn bootstrap_demo(&self) -> Result<(DemoIds, Option<DomainEvent>)> {
        let ids = DemoIds {
            corp_id: parse_id(DEMO_CORP_ID)?,
            alice_actor_id: parse_id(DEMO_ALICE_ID)?,
            bob_actor_id: parse_id(DEMO_BOB_ID)?,
            eve_actor_id: parse_id(DEMO_EVE_ID)?,
            manager_agent_id: parse_id(DEMO_MANAGER_AGENT_ID)?,
            worker_agent_id: parse_id(DEMO_WORKER_AGENT_ID)?,
            codex_agent_id: parse_id(DEMO_CODEX_AGENT_ID)?,
            room_id: parse_id(DEMO_ROOM_ID)?,
        };
        let manager_actor_id = parse_id(DEMO_MANAGER_ACTOR_ID)?;
        let worker_actor_id = parse_id(DEMO_WORKER_ACTOR_ID)?;
        let codex_actor_id = parse_id(DEMO_CODEX_ACTOR_ID)?;

        let mut tx = self.pool.begin().await?;
        lock_demo_tx(&mut tx).await?;
        sqlx::query(
            r#"
            INSERT INTO corps (id, slug, name)
            VALUES ($1, 'crony-demo', 'Crony Corp Demonstration Office')
            ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name
            "#,
        )
        .bind(ids.corp_id)
        .execute(&mut *tx)
        .await?;

        for (id, name, kind, role) in [
            (ids.alice_actor_id, "Alice", "human", "owner"),
            (ids.bob_actor_id, "Bob", "human", "reviewer"),
            (ids.eve_actor_id, "Eve", "human", "guest"),
            (manager_actor_id, "Margo", "agent", "manager"),
            (worker_actor_id, "Wally", "agent", "engineer"),
            (codex_actor_id, "Cody", "agent", "engineer"),
        ] {
            sqlx::query(
                r#"
                INSERT INTO actors (id, corp_id, name, kind, role)
                VALUES ($1, $2, $3, $4, $5)
                ON CONFLICT (id) DO UPDATE
                SET name = EXCLUDED.name, kind = EXCLUDED.kind, role = EXCLUDED.role
                "#,
            )
            .bind(id)
            .bind(ids.corp_id)
            .bind(name)
            .bind(kind)
            .bind(role)
            .execute(&mut *tx)
            .await?;
        }

        sqlx::query(
            r#"
            INSERT INTO rooms (id, corp_id, name, purpose)
            VALUES ($1, $2, 'Product Lab', 'Shared room for the first multiplayer mission')
            ON CONFLICT (id) DO UPDATE SET purpose = EXCLUDED.purpose
            "#,
        )
        .bind(ids.room_id)
        .bind(ids.corp_id)
        .execute(&mut *tx)
        .await?;

        for actor_id in [
            ids.alice_actor_id,
            ids.bob_actor_id,
            manager_actor_id,
            worker_actor_id,
            codex_actor_id,
        ] {
            sqlx::query(
                r#"
                INSERT INTO room_memberships (room_id, actor_id, role)
                VALUES ($1, $2, 'member')
                ON CONFLICT (room_id, actor_id) DO NOTHING
                "#,
            )
            .bind(ids.room_id)
            .bind(actor_id)
            .execute(&mut *tx)
            .await?;
        }

        for (id, actor_id, name, role, adapter, accent) in [
            (
                ids.manager_agent_id,
                manager_actor_id,
                "Margo",
                "manager",
                "fake-process",
                "marigold",
            ),
            (
                ids.worker_agent_id,
                worker_actor_id,
                "Wally",
                "engineer",
                "fake-process",
                "cobalt",
            ),
            (
                ids.codex_agent_id,
                codex_actor_id,
                "Cody",
                "engineer",
                "codex",
                "signal",
            ),
        ] {
            sqlx::query(
                r#"
                INSERT INTO agents
                    (id, corp_id, actor_id, name, role, adapter, status, accent)
                VALUES ($1, $2, $3, $4, $5, $6, 'idle', $7)
                ON CONFLICT (id) DO UPDATE
                SET name = EXCLUDED.name, role = EXCLUDED.role, adapter = EXCLUDED.adapter,
                    accent = EXCLUDED.accent
                "#,
            )
            .bind(id)
            .bind(ids.corp_id)
            .bind(actor_id)
            .bind(name)
            .bind(role)
            .bind(adapter)
            .bind(accent)
            .execute(&mut *tx)
            .await?;
        }

        let event = append_event_tx(
            &mut tx,
            NewEvent::new(
                ids.corp_id,
                Some(ids.alice_actor_id),
                "corp.demo_bootstrapped",
                "corp",
                ids.corp_id,
                "demo-bootstrap-v1",
                json!({
                    "name": "Crony Corp Demonstration Office",
                    "actors": ["Alice", "Bob", "Eve", "Margo", "Wally", "Cody"]
                }),
            ),
        )
        .await?;
        tx.commit().await?;
        Ok((ids, event))
    }

    pub async fn reset_demo(&self) -> Result<(DemoIds, Option<DomainEvent>)> {
        let corp_id = parse_id(DEMO_CORP_ID)?;
        let mut tx = self.pool.begin().await?;
        lock_demo_tx(&mut tx).await?;
        sqlx::query("DELETE FROM control_leases WHERE corp_id = $1")
            .bind(corp_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM queued_messages WHERE corp_id = $1")
            .bind(corp_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM room_messages WHERE corp_id = $1")
            .bind(corp_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM runs WHERE corp_id = $1")
            .bind(corp_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM tasks WHERE corp_id = $1")
            .bind(corp_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM missions WHERE corp_id = $1")
            .bind(corp_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM events WHERE corp_id = $1")
            .bind(corp_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE agents SET status = 'idle', station = NULL, current_run_id = NULL WHERE corp_id = $1",
        )
        .bind(corp_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.bootstrap_demo().await
    }

    pub async fn snapshot(&self, corp_id: Uuid, viewer_actor_id: Uuid) -> Result<CorpSnapshot> {
        if !self.actor_belongs_to_corp(corp_id, viewer_actor_id).await? {
            return Err(anyhow!(
                "viewer actor does not belong to the requested Corp"
            ));
        }
        let corp = map_corp(
            sqlx::query("SELECT id, slug, name, created_at FROM corps WHERE id = $1")
                .bind(corp_id)
                .fetch_one(&self.pool)
                .await
                .context("corp not found")?,
        );

        let actors = sqlx::query(
            "SELECT id, corp_id, name, kind, role, created_at FROM actors WHERE corp_id = $1 ORDER BY created_at, name",
        )
        .bind(corp_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(map_actor)
        .collect::<Result<Vec<_>>>()?;

        let rooms = sqlx::query(
            r#"
            SELECT r.id, r.corp_id, r.name, r.purpose, r.created_at
            FROM rooms r
            JOIN room_memberships rm ON rm.room_id = r.id
            WHERE r.corp_id = $1 AND rm.actor_id = $2
            ORDER BY r.name
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(map_room)
        .collect();

        let agents = sqlx::query(
            r#"
            SELECT id, corp_id, actor_id, name, role, adapter, status, station,
                   current_run_id, accent, created_at
            FROM agents WHERE corp_id = $1 ORDER BY role, name
            "#,
        )
        .bind(corp_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(map_agent)
        .collect::<Result<Vec<_>>>()?;

        let missions = sqlx::query(
            r#"
            SELECT m.id, m.corp_id, m.room_id, m.requested_by, m.title, m.strategy,
                   m.max_nodes, m.max_depth, m.budget_tokens, m.status,
                   m.created_at, m.updated_at
            FROM missions m
            JOIN room_memberships rm ON rm.room_id = m.room_id
            WHERE m.corp_id = $1 AND rm.actor_id = $2
            ORDER BY m.created_at DESC
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(map_mission)
        .collect::<Result<Vec<_>>>()?;

        let mut tasks = sqlx::query(
            r#"
            SELECT t.id, t.mission_id, t.corp_id, t.title, t.objective, t.plan_key,
                   t.contract, t.depth, t.max_attempts, t.attempt_count,
                   t.required_adapter, t.verification_policy, t.verification_status,
                   t.status, t.assigned_agent_id,
                   t.created_at, t.updated_at
            FROM tasks t
            JOIN missions m ON m.id = t.mission_id
            JOIN room_memberships rm ON rm.room_id = m.room_id
            WHERE t.corp_id = $1 AND rm.actor_id = $2
            ORDER BY t.created_at DESC
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(map_task)
        .collect::<Result<Vec<_>>>()?;

        let dependency_rows = sqlx::query(
            r#"
            SELECT dependency.task_id, dependency.depends_on_task_id
            FROM task_dependencies dependency
            JOIN tasks task ON task.id = dependency.task_id
            JOIN missions mission ON mission.id = task.mission_id
            JOIN room_memberships membership ON membership.room_id = mission.room_id
            WHERE task.corp_id = $1 AND membership.actor_id = $2
            ORDER BY dependency.task_id, dependency.depends_on_task_id
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&self.pool)
        .await?;
        let mut dependencies = HashMap::<Uuid, Vec<Uuid>>::new();
        for row in dependency_rows {
            dependencies
                .entry(row.get("task_id"))
                .or_default()
                .push(row.get("depends_on_task_id"));
        }
        for task in &mut tasks {
            task.depends_on = dependencies.remove(&task.id).unwrap_or_default();
        }

        let runs = sqlx::query(
            r#"
            SELECT r.id, r.corp_id, r.task_id, r.agent_id, r.runner_id,
                   r.assignment_token, r.provider_session_id, r.resumed_from_run_id,
                   r.workspace_run_id, r.input_tokens, r.output_tokens, r.cost_microusd,
                   r.workspace_path, r.workspace_branch, r.workspace_base_ref,
                   r.workspace_base_commit, r.workspace_disposition, r.workspace_detail,
                   r.verification_status, r.verification_summary, r.status,
                   r.summary, r.artifact_path, r.artifact_sha256, r.created_at, r.updated_at
            FROM runs r
            JOIN tasks t ON t.id = r.task_id
            JOIN missions m ON m.id = t.mission_id
            JOIN room_memberships rm ON rm.room_id = m.room_id
            WHERE r.corp_id = $1 AND rm.actor_id = $2
            ORDER BY r.created_at DESC
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(map_run)
        .collect::<Result<Vec<_>>>()?;

        let leases = sqlx::query(
            r#"
            SELECT agent_id, corp_id, actor_id, token, expires_at
            FROM control_leases WHERE corp_id = $1 AND expires_at > now()
            ORDER BY agent_id
            "#,
        )
        .bind(corp_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(map_lease)
        .collect();

        let queued_messages = sqlx::query(
            r#"
            SELECT id, corp_id, agent_id, actor_id, text, status, created_at
            FROM queued_messages WHERE corp_id = $1 ORDER BY created_at DESC LIMIT 100
            "#,
        )
        .bind(corp_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(map_queued_message)
        .collect();

        let room_messages = sqlx::query(
            r#"
            SELECT msg.id, msg.corp_id, msg.room_id, msg.actor_id, msg.thread_root_id,
                   msg.reply_to_id, msg.body, msg.mentions, msg.link_kind, msg.link_id,
                   msg.created_at
            FROM room_messages msg
            JOIN room_memberships rm ON rm.room_id = msg.room_id
            WHERE msg.corp_id = $1 AND rm.actor_id = $2
            ORDER BY msg.created_at ASC
            LIMIT 500
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(map_room_message)
        .collect();

        let verification_evidence = sqlx::query(
            r#"
            SELECT evidence.id, evidence.corp_id, evidence.task_id, evidence.run_id,
                   evidence.check_index, evidence.kind, evidence.status, evidence.summary,
                   evidence.payload, evidence.created_at
            FROM verification_evidence evidence
            JOIN tasks task ON task.id = evidence.task_id
            JOIN missions mission ON mission.id = task.mission_id
            JOIN room_memberships membership ON membership.room_id = mission.room_id
            WHERE evidence.corp_id = $1 AND membership.actor_id = $2
            ORDER BY evidence.created_at, evidence.check_index
            LIMIT 1000
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(map_verification_evidence)
        .collect();

        let verification_requests = sqlx::query(
            r#"
            SELECT request.run_id, request.corp_id, request.task_id, request.gate_type,
                   request.gate, request.status, request.requested_at, request.decided_by,
                   request.decision_note, request.decided_at
            FROM verification_requests request
            JOIN tasks task ON task.id = request.task_id
            JOIN missions mission ON mission.id = task.mission_id
            JOIN room_memberships membership ON membership.room_id = mission.room_id
            WHERE request.corp_id = $1 AND membership.actor_id = $2
            ORDER BY request.requested_at DESC
            LIMIT 500
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(map_verification_request)
        .collect::<Result<Vec<_>>>()?;

        let mut events = sqlx::query(
            r#"
            SELECT seq, id, schema_version, corp_id, room_id, actor_id, type,
                   aggregate_type, aggregate_id, aggregate_version, correlation_id,
                   causation_id, idempotency_key, visibility, payload, created_at
            FROM events e
            WHERE e.corp_id = $1
              AND (
                e.room_id IS NULL
                OR EXISTS (
                    SELECT 1 FROM room_memberships rm
                    WHERE rm.room_id = e.room_id AND rm.actor_id = $2
                )
              )
            ORDER BY e.seq DESC
            LIMIT 200
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(map_event)
        .collect::<Vec<_>>();
        events.reverse();

        Ok(CorpSnapshot {
            corp,
            actors,
            rooms,
            agents,
            missions,
            tasks,
            runs,
            room_messages,
            leases,
            queued_messages,
            verification_evidence,
            verification_requests,
            events,
        })
    }

    pub async fn events_after(
        &self,
        corp_id: Uuid,
        viewer_actor_id: Uuid,
        after_seq: i64,
        limit: i64,
    ) -> Result<Vec<DomainEvent>> {
        let bounded_limit = limit.clamp(1, 1_000);
        let events = sqlx::query(
            r#"
            SELECT seq, id, schema_version, corp_id, room_id, actor_id, type,
                   aggregate_type, aggregate_id, aggregate_version, correlation_id,
                   causation_id, idempotency_key, visibility, payload, created_at
            FROM events e
            WHERE e.corp_id = $1 AND e.seq > $2
              AND (
                e.room_id IS NULL
                OR EXISTS (
                    SELECT 1 FROM room_memberships rm
                    WHERE rm.room_id = e.room_id AND rm.actor_id = $3
                )
              )
            ORDER BY e.seq ASC
            LIMIT $4
            "#,
        )
        .bind(corp_id)
        .bind(after_seq)
        .bind(viewer_actor_id)
        .bind(bounded_limit)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(map_event)
        .collect();
        Ok(events)
    }

    pub async fn actor_belongs_to_corp(&self, corp_id: Uuid, actor_id: Uuid) -> Result<bool> {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM actors WHERE id = $1 AND corp_id = $2)",
        )
        .bind(actor_id)
        .bind(corp_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(exists)
    }

    pub async fn visible_room_ids(&self, corp_id: Uuid, actor_id: Uuid) -> Result<Vec<Uuid>> {
        let rooms = sqlx::query_scalar(
            r#"
            SELECT rm.room_id
            FROM room_memberships rm
            JOIN rooms r ON r.id = rm.room_id
            WHERE r.corp_id = $1 AND rm.actor_id = $2
            "#,
        )
        .bind(corp_id)
        .bind(actor_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rooms)
    }

    pub async fn runner_connected(&self, input: RunnerConnectInput) -> Result<RunnerRecord> {
        let row = sqlx::query(
            r#"
            INSERT INTO runner_nodes
                (id, hostname, os, capabilities, connection_epoch, status,
                 connected_at, last_seen_at, disconnected_at, grace_expires_at)
            VALUES ($1, $2, $3, $4, $5, 'connected', now(), now(), NULL, NULL)
            ON CONFLICT (id) DO UPDATE
            SET hostname = EXCLUDED.hostname,
                os = EXCLUDED.os,
                capabilities = EXCLUDED.capabilities,
                connection_epoch = EXCLUDED.connection_epoch,
                status = 'connected',
                connected_at = now(),
                last_seen_at = now(),
                disconnected_at = NULL,
                grace_expires_at = NULL
            RETURNING id, hostname, os, capabilities, connection_epoch, status,
                      last_seen_at, grace_expires_at
            "#,
        )
        .bind(input.id)
        .bind(input.hostname)
        .bind(input.os)
        .bind(input.capabilities)
        .bind(input.connection_epoch)
        .fetch_one(&self.pool)
        .await?;
        Ok(map_runner(row))
    }

    pub async fn runner_heartbeat(&self, runner_id: &str, connection_epoch: Uuid) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE runner_nodes
            SET last_seen_at = now()
            WHERE id = $1 AND connection_epoch = $2 AND status = 'connected'
            "#,
        )
        .bind(runner_id)
        .bind(connection_epoch)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn runner_records(&self) -> Result<Vec<RunnerRecord>> {
        let records = sqlx::query(
            r#"
            SELECT id, hostname, os, capabilities, connection_epoch, status,
                   last_seen_at, grace_expires_at
            FROM runner_nodes
            ORDER BY id
            "#,
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(map_runner)
        .collect();
        Ok(records)
    }

    pub async fn runner_disconnected(
        &self,
        runner_id: &str,
        connection_epoch: Uuid,
        grace_seconds: i64,
    ) -> Result<Vec<DomainEvent>> {
        let mut tx = self.pool.begin().await?;
        let grace_expires_at = Utc::now() + Duration::seconds(grace_seconds.max(1));
        let result = sqlx::query(
            r#"
            UPDATE runner_nodes
            SET status = 'grace', disconnected_at = now(), grace_expires_at = $3
            WHERE id = $1 AND connection_epoch = $2 AND status = 'connected'
            "#,
        )
        .bind(runner_id)
        .bind(connection_epoch)
        .bind(grace_expires_at)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() == 0 {
            tx.commit().await?;
            return Ok(Vec::new());
        }

        let rows = active_runner_run_rows(&mut tx, runner_id).await?;
        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            let run_id: Uuid = row.get("run_id");
            let corp_id: Uuid = row.get("corp_id");
            let room_id: Uuid = row.get("room_id");
            let mission_id: Uuid = row.get("mission_id");
            if let Some(event) = append_event_tx(
                &mut tx,
                NewEvent {
                    room_id: Some(room_id),
                    correlation_id: Some(mission_id),
                    ..NewEvent::new(
                        corp_id,
                        None,
                        "runner.grace_started",
                        "run",
                        run_id,
                        format!("runner-grace:{runner_id}:{connection_epoch}:{run_id}"),
                        json!({
                            "runner_id": runner_id,
                            "grace_expires_at": grace_expires_at
                        }),
                    )
                },
            )
            .await?
            {
                events.push(event);
            }
        }
        tx.commit().await?;
        Ok(events)
    }

    pub async fn expire_runner_grace(
        &self,
        runner_id: &str,
        connection_epoch: Uuid,
    ) -> Result<Vec<DomainEvent>> {
        let mut tx = self.pool.begin().await?;
        let expired = sqlx::query(
            r#"
            UPDATE runner_nodes
            SET status = 'offline', grace_expires_at = NULL
            WHERE id = $1 AND connection_epoch = $2 AND status = 'grace'
              AND grace_expires_at <= now()
            RETURNING id
            "#,
        )
        .bind(runner_id)
        .bind(connection_epoch)
        .fetch_optional(&mut *tx)
        .await?;
        if expired.is_none() {
            tx.commit().await?;
            return Ok(Vec::new());
        }
        let events =
            mark_runner_runs_lost_tx(&mut tx, runner_id, &[], "runner grace period expired")
                .await?;
        tx.commit().await?;
        Ok(events)
    }

    pub async fn reconcile_runner_claims(
        &self,
        runner_id: &str,
        connection_epoch: Uuid,
        claims: &[RunClaim],
    ) -> Result<RunnerReconcileOutcome> {
        let mut tx = self.pool.begin().await?;
        let connected: bool = sqlx::query_scalar(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM runner_nodes
                WHERE id = $1 AND connection_epoch = $2 AND status = 'connected'
            )
            "#,
        )
        .bind(runner_id)
        .bind(connection_epoch)
        .fetch_one(&mut *tx)
        .await?;
        if !connected {
            return Err(anyhow!("runner connection epoch is not active"));
        }

        let mut accepted = Vec::new();
        let mut stale = Vec::new();
        let mut events = Vec::new();
        for claim in claims {
            let row = sqlx::query(
                r#"
                SELECT r.id, r.corp_id, r.assignment_token, t.mission_id, m.room_id
                FROM runs r
                JOIN tasks t ON t.id = r.task_id
                JOIN missions m ON m.id = t.mission_id
                WHERE r.id = $1 AND r.runner_id = $2
                  AND r.status IN ('provisioning', 'starting', 'running',
                                   'waiting_for_input', 'waiting_for_approval', 'verifying')
                FOR UPDATE OF r
                "#,
            )
            .bind(claim.run_id)
            .bind(runner_id)
            .fetch_optional(&mut *tx)
            .await?;
            let Some(row) = row else {
                stale.push(claim.run_id);
                continue;
            };
            let expected: Uuid = row.get("assignment_token");
            if expected != claim.assignment_token {
                stale.push(claim.run_id);
                continue;
            }
            accepted.push(claim.run_id);
            let corp_id: Uuid = row.get("corp_id");
            let mission_id: Uuid = row.get("mission_id");
            let room_id: Uuid = row.get("room_id");
            if let Some(event) = append_event_tx(
                &mut tx,
                NewEvent {
                    room_id: Some(room_id),
                    correlation_id: Some(mission_id),
                    ..NewEvent::new(
                        corp_id,
                        None,
                        "run.reconciled",
                        "run",
                        claim.run_id,
                        format!("run-reconciled:{}:{connection_epoch}", claim.run_id),
                        json!({"runner_id": runner_id}),
                    )
                },
            )
            .await?
            {
                events.push(event);
            }
        }
        tx.commit().await?;
        Ok(RunnerReconcileOutcome {
            accepted,
            stale,
            events,
        })
    }

    pub async fn mark_unclaimed_runner_runs_lost(
        &self,
        runner_id: &str,
        connection_epoch: Uuid,
        accepted_claims: &[Uuid],
    ) -> Result<Vec<DomainEvent>> {
        let mut tx = self.pool.begin().await?;
        let connected: bool = sqlx::query_scalar(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM runner_nodes
                WHERE id = $1 AND connection_epoch = $2 AND status = 'connected'
            )
            "#,
        )
        .bind(runner_id)
        .bind(connection_epoch)
        .fetch_one(&mut *tx)
        .await?;
        if !connected {
            tx.commit().await?;
            return Ok(Vec::new());
        }
        let events = mark_runner_runs_lost_tx(
            &mut tx,
            runner_id,
            accepted_claims,
            "runner reconnected without an active claim",
        )
        .await?;
        tx.commit().await?;
        Ok(events)
    }

    pub async fn create_mission(
        &self,
        corp_id: Uuid,
        requested_by: Uuid,
        title: &str,
        plan: &TaskGraphPlan,
    ) -> Result<(MissionPlanIds, Vec<DomainEvent>)> {
        let title = title.trim();
        if title.is_empty() {
            return Err(anyhow!("mission title cannot be empty"));
        }
        if title.len() > 240 {
            return Err(anyhow!("mission title cannot exceed 240 characters"));
        }

        let mut tx = self.pool.begin().await?;
        let room_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM rooms WHERE corp_id = $1 ORDER BY created_at LIMIT 1",
        )
        .bind(corp_id)
        .fetch_one(&mut *tx)
        .await
        .context("corp has no room")?;
        assert_room_membership_tx(&mut tx, corp_id, room_id, requested_by).await?;

        let mission_id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO missions
                (id, corp_id, room_id, requested_by, title, strategy,
                 max_nodes, max_depth, budget_tokens, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'ready')
            "#,
        )
        .bind(mission_id)
        .bind(corp_id)
        .bind(room_id)
        .bind(requested_by)
        .bind(title)
        .bind(&plan.strategy)
        .bind(plan.max_nodes)
        .bind(plan.max_depth)
        .bind(plan.budget_tokens)
        .execute(&mut *tx)
        .await?;

        let mission_event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(room_id),
                correlation_id: Some(mission_id),
                ..NewEvent::new(
                    corp_id,
                    Some(requested_by),
                    "mission.created",
                    "mission",
                    mission_id,
                    format!("mission:{mission_id}:created"),
                    json!({
                        "title": title,
                        "strategy": plan.strategy,
                        "max_nodes": plan.max_nodes,
                        "max_depth": plan.max_depth,
                        "budget_tokens": plan.budget_tokens,
                        "status": "ready"
                    }),
                )
            },
        )
        .await?
        .context("mission created event unexpectedly existed")?;

        let mut task_ids = HashMap::new();
        for task in &plan.tasks {
            task_ids.insert(task.key.clone(), Uuid::new_v4());
        }
        let mut events = vec![mission_event.clone()];
        for task in &plan.tasks {
            let task_id = *task_ids
                .get(&task.key)
                .context("planned task id unexpectedly missing")?;
            let adapter: String =
                sqlx::query_scalar("SELECT adapter FROM agents WHERE id = $1 AND corp_id = $2")
                    .bind(task.assigned_agent_id)
                    .bind(corp_id)
                    .fetch_one(&mut *tx)
                    .await
                    .with_context(|| {
                        format!(
                            "planned task {} references an agent outside the Corp",
                            task.key
                        )
                    })?;
            if adapter != task.required_adapter {
                return Err(anyhow!(
                    "planned task {} requires adapter {} but assigned agent uses {adapter}",
                    task.key,
                    task.required_adapter
                ));
            }
            let status = if task.depends_on.is_empty() {
                "ready"
            } else {
                "pending"
            };
            sqlx::query(
                r#"
                INSERT INTO tasks
                    (id, mission_id, corp_id, title, objective, plan_key, contract,
                     depth, max_attempts, attempt_count, required_adapter, status,
                     assigned_agent_id, verification_policy, verification_status)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 0, $10, $11, $12, $13, 'pending')
                "#,
            )
            .bind(task_id)
            .bind(mission_id)
            .bind(corp_id)
            .bind(&task.title)
            .bind(&task.contract.objective)
            .bind(&task.key)
            .bind(serde_json::to_value(&task.contract)?)
            .bind(task.depth)
            .bind(task.max_attempts)
            .bind(&task.required_adapter)
            .bind(status)
            .bind(task.assigned_agent_id)
            .bind(serde_json::to_value(&task.verification_policy)?)
            .execute(&mut *tx)
            .await?;

            let event = append_event_tx(
                &mut tx,
                NewEvent {
                    room_id: Some(room_id),
                    correlation_id: Some(mission_id),
                    causation_id: Some(mission_event.id),
                    ..NewEvent::new(
                        corp_id,
                        Some(requested_by),
                        "task.created",
                        "task",
                        task_id,
                        format!("task:{task_id}:created"),
                        json!({
                            "plan_key": task.key,
                            "title": task.title,
                            "assigned_agent_id": task.assigned_agent_id,
                            "required_adapter": task.required_adapter,
                            "depends_on": task.depends_on,
                            "depth": task.depth,
                            "max_attempts": task.max_attempts,
                            "budget_tokens": task.contract.budget_tokens,
                            "status": status
                        }),
                    )
                },
            )
            .await?
            .context("task created event unexpectedly existed")?;
            events.push(event);
        }

        for task in &plan.tasks {
            let task_id = *task_ids
                .get(&task.key)
                .context("planned task id unexpectedly missing")?;
            for dependency in &task.depends_on {
                let dependency_id = *task_ids
                    .get(dependency)
                    .with_context(|| format!("unknown planned dependency {dependency}"))?;
                sqlx::query(
                    r#"
                    INSERT INTO task_dependencies (task_id, depends_on_task_id)
                    VALUES ($1, $2)
                    "#,
                )
                .bind(task_id)
                .bind(dependency_id)
                .execute(&mut *tx)
                .await?;
            }
        }

        let planned_event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(room_id),
                correlation_id: Some(mission_id),
                causation_id: Some(mission_event.id),
                ..NewEvent::new(
                    corp_id,
                    Some(requested_by),
                    "mission.planned",
                    "mission",
                    mission_id,
                    format!("mission:{mission_id}:planned"),
                    json!({
                        "strategy": plan.strategy,
                        "task_count": plan.tasks.len(),
                        "task_ids": plan.tasks.iter().filter_map(|task| {
                            task_ids.get(&task.key).copied()
                        }).collect::<Vec<_>>()
                    }),
                )
            },
        )
        .await?
        .context("mission planned event unexpectedly existed")?;
        events.push(planned_event);
        tx.commit().await?;

        Ok((
            MissionPlanIds {
                mission_id,
                task_ids: plan
                    .tasks
                    .iter()
                    .filter_map(|task| task_ids.get(&task.key).copied())
                    .collect(),
            },
            events,
        ))
    }

    pub async fn schedulable_mission_ids(&self, corp_id: Uuid) -> Result<Vec<Uuid>> {
        sqlx::query_scalar(
            r#"
            SELECT DISTINCT t.mission_id
            FROM tasks t
            JOIN missions m ON m.id = t.mission_id
            JOIN agents a ON a.id = t.assigned_agent_id
            WHERE t.corp_id = $1
              AND m.status IN ('ready', 'running')
              AND t.status IN ('pending', 'ready')
              AND a.status = 'idle'
              AND t.attempt_count < t.max_attempts
              AND NOT EXISTS (
                SELECT 1 FROM task_dependencies dependency
                JOIN tasks parent ON parent.id = dependency.depends_on_task_id
                WHERE dependency.task_id = t.id
                  AND parent.status <> 'completed'
              )
              AND NOT EXISTS (
                SELECT 1 FROM runs task_run
                WHERE task_run.task_id = t.id
                  AND task_run.status IN ('provisioning', 'starting', 'running',
                                          'waiting_for_input', 'waiting_for_approval', 'verifying')
              )
              AND NOT EXISTS (
                SELECT 1 FROM runs agent_run
                WHERE agent_run.agent_id = t.assigned_agent_id
                  AND agent_run.status IN ('provisioning', 'starting', 'running',
                                           'waiting_for_input', 'waiting_for_approval', 'verifying')
              )
            ORDER BY t.mission_id
            "#,
        )
        .bind(corp_id)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn schedulable_tasks(
        &self,
        corp_id: Uuid,
        mission_id: Uuid,
    ) -> Result<Vec<SchedulableTask>> {
        sqlx::query(
            r#"
            SELECT t.id AS task_id, COALESCE(t.required_adapter, a.adapter) AS required_adapter
            FROM tasks t
            JOIN missions m ON m.id = t.mission_id
            JOIN agents a ON a.id = t.assigned_agent_id
            WHERE t.corp_id = $1 AND t.mission_id = $2
              AND m.status IN ('ready', 'running')
              AND t.status IN ('pending', 'ready')
              AND a.status = 'idle'
              AND t.attempt_count < t.max_attempts
              AND NOT EXISTS (
                SELECT 1 FROM task_dependencies dependency
                JOIN tasks parent ON parent.id = dependency.depends_on_task_id
                WHERE dependency.task_id = t.id
                  AND parent.status <> 'completed'
              )
              AND NOT EXISTS (
                SELECT 1 FROM runs task_run
                WHERE task_run.task_id = t.id
                  AND task_run.status IN ('provisioning', 'starting', 'running',
                                          'waiting_for_input', 'waiting_for_approval', 'verifying')
              )
              AND NOT EXISTS (
                SELECT 1 FROM runs agent_run
                WHERE agent_run.agent_id = t.assigned_agent_id
                  AND agent_run.status IN ('provisioning', 'starting', 'running',
                                           'waiting_for_input', 'waiting_for_approval', 'verifying')
              )
            ORDER BY t.depth, t.created_at, t.id
            "#,
        )
        .bind(corp_id)
        .bind(mission_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|row| {
            Ok(SchedulableTask {
                task_id: row.get("task_id"),
                required_adapter: row.get("required_adapter"),
            })
        })
        .collect()
    }

    pub async fn create_task_run(
        &self,
        corp_id: Uuid,
        mission_id: Uuid,
        task_id: Uuid,
        requested_by: Option<Uuid>,
        runner_id: &str,
    ) -> Result<(LaunchRecord, DomainEvent)> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            r#"
            SELECT m.room_id, m.title AS mission_title, m.status AS mission_status,
                   t.title AS task_title, t.contract, t.verification_policy,
                   t.status AS task_status,
                   t.assigned_agent_id, t.attempt_count, t.max_attempts,
                   COALESCE(t.required_adapter, a.adapter) AS required_adapter,
                   a.adapter
            FROM missions m
            JOIN tasks t ON t.mission_id = m.id
            JOIN agents a ON a.id = t.assigned_agent_id
            WHERE m.id = $1 AND m.corp_id = $2 AND t.id = $3
            FOR UPDATE OF m, t, a
            "#,
        )
        .bind(mission_id)
        .bind(corp_id)
        .bind(task_id)
        .fetch_one(&mut *tx)
        .await
        .context("mission or task not found")?;

        let room_id: Uuid = row.get("room_id");
        if let Some(actor_id) = requested_by {
            assert_room_membership_tx(&mut tx, corp_id, room_id, actor_id).await?;
        }
        let mission_status: String = row.get("mission_status");
        if matches!(
            mission_status.as_str(),
            "completed" | "failed" | "cancelled"
        ) {
            return Err(anyhow!("mission is already {mission_status}"));
        }
        let task_status: String = row.get("task_status");
        if !matches!(task_status.as_str(), "pending" | "ready") {
            return Err(anyhow!("task is not schedulable from status {task_status}"));
        }
        let agent_id: Uuid = row
            .try_get::<Option<Uuid>, _>("assigned_agent_id")?
            .context("task has no assigned agent")?;
        let adapter: String = row.get("adapter");
        let required_adapter: String = row.get("required_adapter");
        if adapter != required_adapter {
            return Err(anyhow!(
                "assigned agent adapter {adapter} does not satisfy {required_adapter}"
            ));
        }
        let dependencies_ready: bool = sqlx::query_scalar(
            r#"
            SELECT NOT EXISTS (
                SELECT 1 FROM task_dependencies dependency
                JOIN tasks parent ON parent.id = dependency.depends_on_task_id
                WHERE dependency.task_id = $1
                  AND parent.status <> 'completed'
            )
            "#,
        )
        .bind(task_id)
        .fetch_one(&mut *tx)
        .await?;
        if !dependencies_ready {
            return Err(anyhow!("task dependencies are not complete"));
        }

        let active: Option<Uuid> = sqlx::query_scalar(
            r#"
            SELECT id FROM runs
            WHERE task_id = $1 AND status IN ('provisioning', 'starting', 'running',
                                             'waiting_for_input', 'waiting_for_approval', 'verifying')
            LIMIT 1
            "#,
        )
        .bind(task_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(active_run) = active {
            return Err(anyhow!("task already has active run {active_run}"));
        }
        let agent_active: Option<Uuid> = sqlx::query_scalar(
            r#"
            SELECT id FROM runs
            WHERE agent_id = $1 AND status IN ('provisioning', 'starting', 'running',
                                              'waiting_for_input', 'waiting_for_approval', 'verifying')
            LIMIT 1
            "#,
        )
        .bind(agent_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(active_run) = agent_active {
            return Err(anyhow!(
                "assigned agent already has active run {active_run}"
            ));
        }
        let attempt_count: i32 = row.get("attempt_count");
        let max_attempts: i32 = row.get("max_attempts");
        if attempt_count >= max_attempts {
            return Err(anyhow!("task exhausted its retry limit"));
        }
        let attempt = attempt_count + 1;
        let contract: TaskContract =
            serde_json::from_value(row.get("contract")).context("decode task contract")?;
        let verification_policy: VerificationPolicy =
            serde_json::from_value(row.get("verification_policy"))
                .context("decode verification policy")?;
        let mission_title: String = row.get("mission_title");
        let task_title: String = row.get("task_title");
        let task_prompt = format_task_prompt(&mission_title, &task_title, &contract, attempt);

        let run_id = Uuid::new_v4();
        let assignment_token = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO runs
                (id, corp_id, task_id, agent_id, runner_id, assignment_token, status,
                 workspace_run_id)
            VALUES ($1, $2, $3, $4, $5, $6, 'starting', $1)
            "#,
        )
        .bind(run_id)
        .bind(corp_id)
        .bind(task_id)
        .bind(agent_id)
        .bind(runner_id)
        .bind(assignment_token)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE missions SET status = 'running', updated_at = now() WHERE id = $1 AND status IN ('ready', 'running')",
        )
            .bind(mission_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE tasks SET status = 'claimed', attempt_count = $1, verification_status = 'pending', updated_at = now() WHERE id = $2",
        )
            .bind(attempt)
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE agents SET status = 'starting', station = 'dispatch', current_run_id = $1 WHERE id = $2",
        )
        .bind(run_id)
        .bind(agent_id)
        .execute(&mut *tx)
        .await?;

        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(room_id),
                correlation_id: Some(mission_id),
                ..NewEvent::new(
                    corp_id,
                    requested_by,
                    "run.requested",
                    "run",
                    run_id,
                    format!("run:{run_id}:requested"),
                    json!({
                        "task_id": task_id,
                        "agent_id": agent_id,
                        "runner_id": runner_id,
                        "attempt": attempt,
                        "max_attempts": max_attempts,
                        "status": "starting"
                    }),
                )
            },
        )
        .await?
        .context("run requested event unexpectedly existed")?;
        tx.commit().await?;

        Ok((
            LaunchRecord {
                corp_id,
                room_id,
                mission_id,
                task_id,
                run_id,
                agent_id,
                assignment_token,
                attempt,
                adapter,
                mission_title: task_prompt,
                verification_policy,
            },
            event,
        ))
    }

    pub async fn create_resume_run(
        &self,
        corp_id: Uuid,
        source_run_id: Uuid,
        requested_by: Uuid,
    ) -> Result<(ResumeLaunchRecord, DomainEvent)> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            r#"
            SELECT r.task_id, r.agent_id, r.runner_id, r.provider_session_id,
                   r.workspace_run_id,
                   t.mission_id, t.verification_policy, m.room_id, a.adapter
            FROM runs r
            JOIN tasks t ON t.id = r.task_id
            JOIN missions m ON m.id = t.mission_id
            JOIN agents a ON a.id = r.agent_id
            WHERE r.id = $1 AND r.corp_id = $2
            FOR UPDATE OF r, t, m, a
            "#,
        )
        .bind(source_run_id)
        .bind(corp_id)
        .fetch_one(&mut *tx)
        .await
        .context("source run not found")?;
        let task_id: Uuid = row.get("task_id");
        let agent_id: Uuid = row.get("agent_id");
        let runner_id: String = row.get("runner_id");
        let provider_session_id: String = row
            .try_get::<Option<String>, _>("provider_session_id")?
            .context("source run has no resumable provider session")?;
        let workspace_run_id: Uuid = row.get("workspace_run_id");
        let verification_policy: VerificationPolicy =
            serde_json::from_value(row.get("verification_policy"))
                .context("decode verification policy")?;
        let mission_id: Uuid = row.get("mission_id");
        let room_id: Uuid = row.get("room_id");
        let adapter: String = row.get("adapter");
        assert_room_membership_tx(&mut tx, corp_id, room_id, requested_by).await?;

        let active: bool = sqlx::query_scalar(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM runs
                WHERE task_id = $1
                  AND status IN ('provisioning', 'starting', 'running',
                                 'waiting_for_input', 'waiting_for_approval', 'verifying')
            )
            "#,
        )
        .bind(task_id)
        .fetch_one(&mut *tx)
        .await?;
        if active {
            return Err(anyhow!("task already has an active run"));
        }

        let run_id = Uuid::new_v4();
        let assignment_token = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO runs
                (id, corp_id, task_id, agent_id, runner_id, assignment_token, status,
                 provider_session_id, resumed_from_run_id, workspace_run_id)
            VALUES ($1, $2, $3, $4, $5, $6, 'starting', $7, $8, $9)
            "#,
        )
        .bind(run_id)
        .bind(corp_id)
        .bind(task_id)
        .bind(agent_id)
        .bind(&runner_id)
        .bind(assignment_token)
        .bind(&provider_session_id)
        .bind(source_run_id)
        .bind(workspace_run_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE missions SET status = 'running', updated_at = now() WHERE id = $1")
            .bind(mission_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE tasks SET status = 'claimed', verification_status = 'pending', updated_at = now() WHERE id = $1",
        )
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE agents SET status = 'starting', station = 'dispatch', current_run_id = $1 WHERE id = $2",
        )
        .bind(run_id)
        .bind(agent_id)
        .execute(&mut *tx)
        .await?;

        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(room_id),
                correlation_id: Some(mission_id),
                ..NewEvent::new(
                    corp_id,
                    Some(requested_by),
                    "run.resume_requested",
                    "run",
                    run_id,
                    format!("run:{run_id}:resume-requested"),
                    json!({
                        "source_run_id": source_run_id,
                        "workspace_run_id": workspace_run_id,
                        "task_id": task_id,
                        "agent_id": agent_id,
                        "runner_id": runner_id
                    }),
                )
            },
        )
        .await?
        .context("run resume event unexpectedly existed")?;
        tx.commit().await?;
        Ok((
            ResumeLaunchRecord {
                corp_id,
                room_id,
                mission_id,
                task_id,
                run_id,
                source_run_id,
                workspace_run_id,
                agent_id,
                runner_id,
                assignment_token,
                adapter,
                provider_session_id,
                verification_policy,
            },
            event,
        ))
    }

    pub async fn apply_runner_event(&self, input: RunnerEventInput) -> Result<Option<DomainEvent>> {
        let RunnerEventInput {
            event_id,
            runner_id,
            corp_id,
            run_id,
            agent_id,
            event_type,
            payload,
        } = input;
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            r#"
            SELECT r.task_id, r.verification_status AS run_verification_status,
                   t.mission_id, t.verification_policy, m.room_id,
                   t.attempt_count, t.max_attempts
            FROM runs r
            JOIN tasks t ON t.id = r.task_id
            JOIN missions m ON m.id = t.mission_id
            WHERE r.id = $1 AND r.corp_id = $2 AND r.agent_id = $3 AND r.runner_id = $4
              AND (
                r.status IN ('provisioning', 'starting', 'running',
                             'waiting_for_input', 'waiting_for_approval', 'verifying')
                OR (
                  $5::text IN ('run.workspace_preserved', 'run.workspace_removed')
                  AND r.status IN ('completed', 'failed', 'cancelled', 'lost')
                )
              )
            FOR UPDATE OF r, t, m
            "#,
        )
        .bind(run_id)
        .bind(corp_id)
        .bind(agent_id)
        .bind(&runner_id)
        .bind(&event_type)
        .fetch_one(&mut *tx)
        .await
        .context("runner event does not match an active run")?;
        let task_id: Uuid = row.get("task_id");
        let mission_id: Uuid = row.get("mission_id");
        let room_id: Uuid = row.get("room_id");
        let attempt_count: i32 = row.get("attempt_count");
        let max_attempts: i32 = row.get("max_attempts");
        let run_verification_status: String = row.get("run_verification_status");
        let verification_policy: VerificationPolicy =
            serde_json::from_value(row.get("verification_policy"))
                .context("decode task verification policy")?;

        let event = append_event_tx(
            &mut tx,
            NewEvent {
                id: event_id,
                room_id: Some(room_id),
                correlation_id: Some(mission_id),
                ..NewEvent::new(
                    corp_id,
                    None,
                    &event_type,
                    "run",
                    run_id,
                    format!("runner:{runner_id}:event:{event_id}"),
                    payload.clone(),
                )
            },
        )
        .await?;
        let Some(event) = event else {
            tx.rollback().await?;
            return Ok(None);
        };

        match event_type.as_str() {
            "run.session" => {
                let session_id = payload
                    .get("session_id")
                    .and_then(Value::as_str)
                    .context("run.session event omitted session_id")?;
                sqlx::query(
                    "UPDATE runs SET provider_session_id = $1, updated_at = now() WHERE id = $2",
                )
                .bind(session_id)
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
            }
            "run.started" => {
                let workspace_path = payload.get("workspace").and_then(Value::as_str);
                let workspace_branch = payload.get("workspace_branch").and_then(Value::as_str);
                let workspace_base_ref = payload.get("workspace_base_ref").and_then(Value::as_str);
                let workspace_base_commit =
                    payload.get("workspace_base_commit").and_then(Value::as_str);
                sqlx::query(
                    r#"
                    UPDATE runs
                    SET status = 'running',
                        workspace_path = $1,
                        workspace_branch = $2,
                        workspace_base_ref = $3,
                        workspace_base_commit = $4,
                        workspace_disposition = 'active',
                        workspace_detail = NULL,
                        updated_at = now()
                    WHERE id = $5
                    "#,
                )
                .bind(workspace_path)
                .bind(workspace_branch)
                .bind(workspace_base_ref)
                .bind(workspace_base_commit)
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE tasks SET status = 'running', updated_at = now() WHERE id = $1",
                )
                .bind(task_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE agents SET status = 'working', station = 'terminal' WHERE id = $1",
                )
                .bind(agent_id)
                .execute(&mut *tx)
                .await?;
            }
            "run.status" => {
                let status = payload
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("working");
                let agent_status = match status {
                    "blocked" => "blocked",
                    "reviewing" => "reviewing",
                    "idle" => "idle",
                    _ => "working",
                };
                let station = payload
                    .get("station")
                    .and_then(Value::as_str)
                    .unwrap_or("terminal");
                sqlx::query("UPDATE agents SET status = $1, station = $2 WHERE id = $3")
                    .bind(agent_status)
                    .bind(station)
                    .bind(agent_id)
                    .execute(&mut *tx)
                    .await?;
            }
            "run.artifact" => {
                let path = payload.get("path").and_then(Value::as_str);
                let sha = payload.get("sha256").and_then(Value::as_str);
                sqlx::query(
                    "UPDATE runs SET artifact_path = $1, artifact_sha256 = $2, status = 'verifying', updated_at = now() WHERE id = $3",
                )
                .bind(path)
                .bind(sha)
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query("UPDATE tasks SET status = 'review', updated_at = now() WHERE id = $1")
                    .bind(task_id)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query(
                    "UPDATE agents SET status = 'reviewing', station = 'review', current_run_id = NULL WHERE id = $1",
                )
                .bind(agent_id)
                .execute(&mut *tx)
                .await?;
            }
            "run.usage" => {
                let input_tokens = payload
                    .get("input_tokens")
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
                    .max(0);
                let output_tokens = payload
                    .get("output_tokens")
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
                    .max(0);
                let cost_microusd = payload
                    .get("cost_microusd")
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
                    .max(0);
                sqlx::query(
                    r#"
                    UPDATE runs
                    SET input_tokens = input_tokens + $1,
                        output_tokens = output_tokens + $2,
                        cost_microusd = cost_microusd + $3,
                        updated_at = now()
                    WHERE id = $4
                    "#,
                )
                .bind(input_tokens)
                .bind(output_tokens)
                .bind(cost_microusd)
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
            }
            "run.verification_started" => {
                sqlx::query(
                    "UPDATE runs SET status = 'verifying', verification_status = 'running', verification_summary = NULL, updated_at = now() WHERE id = $1",
                )
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE tasks SET status = 'review', verification_status = 'running', updated_at = now() WHERE id = $1",
                )
                .bind(task_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE agents SET status = 'reviewing', station = 'review' WHERE id = $1",
                )
                .bind(agent_id)
                .execute(&mut *tx)
                .await?;
            }
            "run.verification_evidence" => {
                let evidence_id = payload
                    .get("evidence_id")
                    .and_then(Value::as_str)
                    .context("verification evidence omitted evidence_id")
                    .and_then(|value| {
                        Uuid::parse_str(value).context("verification evidence id is invalid")
                    })?;
                let check_index = payload
                    .get("check_index")
                    .and_then(Value::as_i64)
                    .context("verification evidence omitted check_index")?;
                let check_index = i32::try_from(check_index)
                    .context("verification check index is out of range")?;
                let expected = verification_policy
                    .checks
                    .get(check_index as usize)
                    .context("verification evidence references an unknown check")?;
                let kind = payload
                    .get("kind")
                    .and_then(Value::as_str)
                    .context("verification evidence omitted kind")?;
                if kind != expected.kind() {
                    return Err(anyhow!(
                        "verification evidence kind {kind} does not match policy {}",
                        expected.kind()
                    ));
                }
                let status = payload
                    .get("status")
                    .and_then(Value::as_str)
                    .context("verification evidence omitted status")?;
                if !matches!(status, "passed" | "failed") {
                    return Err(anyhow!("verification evidence status is invalid"));
                }
                let summary = payload
                    .get("summary")
                    .and_then(Value::as_str)
                    .context("verification evidence omitted summary")?;
                if summary.len() > 4_000 {
                    return Err(anyhow!("verification evidence summary is too long"));
                }
                sqlx::query(
                    r#"
                    INSERT INTO verification_evidence
                        (id, corp_id, task_id, run_id, check_index, kind, status, summary, payload)
                    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                    "#,
                )
                .bind(evidence_id)
                .bind(corp_id)
                .bind(task_id)
                .bind(run_id)
                .bind(check_index)
                .bind(kind)
                .bind(status)
                .bind(summary)
                .bind(payload.get("payload").cloned().unwrap_or_else(|| json!({})))
                .execute(&mut *tx)
                .await?;
            }
            "run.verification_passed" => {
                let evidence_rows = sqlx::query(
                    r#"
                    SELECT check_index, kind, status
                    FROM verification_evidence
                    WHERE run_id = $1
                    ORDER BY check_index
                    "#,
                )
                .bind(run_id)
                .fetch_all(&mut *tx)
                .await?;
                if evidence_rows.len() != verification_policy.checks.len() {
                    return Err(anyhow!(
                        "verification passed without complete evidence: expected {}, found {}",
                        verification_policy.checks.len(),
                        evidence_rows.len()
                    ));
                }
                for (index, row) in evidence_rows.into_iter().enumerate() {
                    let check_index: i32 = row.get("check_index");
                    let kind: String = row.get("kind");
                    let status: String = row.get("status");
                    let expected = &verification_policy.checks[index];
                    if check_index != index as i32 || kind != expected.kind() || status != "passed"
                    {
                        return Err(anyhow!("verification evidence does not satisfy policy"));
                    }
                }
                let summary = payload
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or("all verifier checks passed");
                sqlx::query(
                    "UPDATE runs SET verification_status = 'passed', verification_summary = $1, updated_at = now() WHERE id = $2",
                )
                .bind(summary)
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE tasks SET verification_status = 'passed', updated_at = now() WHERE id = $1",
                )
                .bind(task_id)
                .execute(&mut *tx)
                .await?;
            }
            "run.verification_waiting" => {
                if run_verification_status != "passed" {
                    return Err(anyhow!(
                        "manual verification gate requested before automated checks passed"
                    ));
                }
                let gate: ManualVerificationGate = serde_json::from_value(
                    payload
                        .get("gate")
                        .cloned()
                        .context("verification waiting event omitted gate")?,
                )
                .context("decode manual verification gate")?;
                if verification_policy.manual_gate.as_ref() != Some(&gate) {
                    return Err(anyhow!(
                        "manual verification gate does not match task policy"
                    ));
                }
                let gate_type = payload
                    .get("gate_type")
                    .and_then(Value::as_str)
                    .context("verification waiting event omitted gate_type")?;
                if gate.kind() != gate_type {
                    return Err(anyhow!("manual verification gate type mismatch"));
                }
                let verification_summary = payload
                    .get("verification_summary")
                    .and_then(Value::as_str)
                    .unwrap_or("automated verification passed");
                let completion_summary = payload
                    .get("completion_summary")
                    .and_then(Value::as_str)
                    .unwrap_or("Agent completed pending approval");
                sqlx::query(
                    r#"
                    INSERT INTO verification_requests
                        (run_id, corp_id, task_id, gate_type, gate, status)
                    VALUES ($1, $2, $3, $4, $5, 'pending')
                    "#,
                )
                .bind(run_id)
                .bind(corp_id)
                .bind(task_id)
                .bind(gate_type)
                .bind(serde_json::to_value(&gate)?)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE runs SET status = 'waiting_for_approval', verification_status = 'waiting_for_approval', verification_summary = $1, summary = $2, updated_at = now() WHERE id = $3",
                )
                .bind(verification_summary)
                .bind(completion_summary)
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE tasks SET status = 'awaiting_approval', verification_status = 'waiting_for_approval', updated_at = now() WHERE id = $1",
                )
                .bind(task_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE agents SET status = 'reviewing', station = 'review' WHERE id = $1",
                )
                .bind(agent_id)
                .execute(&mut *tx)
                .await?;
            }
            "run.verification_failed" => {
                let evidence_statuses: Vec<String> = sqlx::query_scalar(
                    "SELECT status FROM verification_evidence WHERE run_id = $1 ORDER BY check_index",
                )
                .bind(run_id)
                .fetch_all(&mut *tx)
                .await?;
                if evidence_statuses.len() != verification_policy.checks.len()
                    || !evidence_statuses.iter().any(|status| status == "failed")
                {
                    return Err(anyhow!(
                        "verification failure does not have complete failing evidence"
                    ));
                }
                let summary = payload
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("verification policy failed");
                sqlx::query(
                    "UPDATE runs SET status = 'failed', verification_status = 'failed', verification_summary = $1, summary = $1, updated_at = now() WHERE id = $2",
                )
                .bind(summary)
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE tasks SET status = 'verification_failed', verification_status = 'failed', updated_at = now() WHERE id = $1",
                )
                .bind(task_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE missions SET status = 'failed', updated_at = now() WHERE id = $1 AND status IN ('ready', 'running')",
                )
                .bind(mission_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE agents SET status = 'idle', station = NULL, current_run_id = NULL WHERE id = $1",
                )
                .bind(agent_id)
                .execute(&mut *tx)
                .await?;
            }
            "run.workspace_preserved" | "run.workspace_removed" => {
                let disposition = if event_type == "run.workspace_removed" {
                    "removed"
                } else {
                    "preserved"
                };
                let detail = payload
                    .get("detail")
                    .and_then(Value::as_str)
                    .unwrap_or("runner did not provide cleanup detail");
                sqlx::query(
                    r#"
                    UPDATE runs
                    SET workspace_disposition = $1,
                        workspace_detail = $2,
                        updated_at = now()
                    WHERE id = $3
                    "#,
                )
                .bind(disposition)
                .bind(detail)
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
            }
            "run.completed" => {
                if run_verification_status != "passed" {
                    return Err(anyhow!("run cannot complete before verification passes"));
                }
                if verification_policy.manual_gate.is_some() {
                    return Err(anyhow!(
                        "run cannot complete before its manual verification gate"
                    ));
                }
                let summary = payload
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or("Mission artifact completed");
                sqlx::query(
                    "UPDATE runs SET status = 'completed', summary = $1, updated_at = now() WHERE id = $2",
                )
                .bind(summary)
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE tasks SET status = 'completed', verification_status = 'passed', updated_at = now() WHERE id = $1",
                )
                .bind(task_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    r#"
                    UPDATE tasks child
                    SET status = 'ready', updated_at = now()
                    WHERE child.mission_id = $1 AND child.status = 'pending'
                      AND NOT EXISTS (
                        SELECT 1 FROM task_dependencies dependency
                        JOIN tasks parent ON parent.id = dependency.depends_on_task_id
                        WHERE dependency.task_id = child.id
                          AND parent.status <> 'completed'
                      )
                    "#,
                )
                .bind(mission_id)
                .execute(&mut *tx)
                .await?;
                let mission_complete: bool = sqlx::query_scalar(
                    "SELECT NOT EXISTS(SELECT 1 FROM tasks WHERE mission_id = $1 AND status <> 'completed')",
                )
                .bind(mission_id)
                .fetch_one(&mut *tx)
                .await?;
                sqlx::query(
                    r#"
                    UPDATE missions
                    SET status = $1, updated_at = now()
                    WHERE id = $2 AND status IN ('ready', 'running')
                    "#,
                )
                .bind(if mission_complete {
                    "completed"
                } else {
                    "running"
                })
                .bind(mission_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE agents SET status = 'idle', station = NULL, current_run_id = NULL WHERE id = $1",
                )
                .bind(agent_id)
                .execute(&mut *tx)
                .await?;
            }
            "run.failed" => {
                let summary = payload
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("Runner reported failure");
                sqlx::query(
                    "UPDATE runs SET status = 'failed', summary = $1, updated_at = now() WHERE id = $2",
                )
                .bind(summary)
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
                let retry = attempt_count < max_attempts;
                sqlx::query("UPDATE tasks SET status = $1, updated_at = now() WHERE id = $2")
                    .bind(if retry { "ready" } else { "failed" })
                    .bind(task_id)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query(
                    "UPDATE missions SET status = $1, updated_at = now() WHERE id = $2 AND status IN ('ready', 'running')",
                )
                .bind(if retry { "running" } else { "failed" })
                .bind(mission_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE agents SET status = 'idle', station = NULL, current_run_id = NULL WHERE id = $1",
                )
                .bind(agent_id)
                .execute(&mut *tx)
                .await?;
            }
            "run.cancelled" => {
                let summary = payload
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("Run cancelled by an authorized operator");
                sqlx::query(
                    "UPDATE runs SET status = 'cancelled', summary = $1, updated_at = now() WHERE id = $2",
                )
                .bind(summary)
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE tasks SET status = 'cancelled', updated_at = now() WHERE id = $1",
                )
                .bind(task_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE missions SET status = 'cancelled', updated_at = now() WHERE id = $1",
                )
                .bind(mission_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE agents SET status = 'idle', station = NULL, current_run_id = NULL WHERE id = $1",
                )
                .bind(agent_id)
                .execute(&mut *tx)
                .await?;
            }
            _ => {}
        }

        tx.commit().await?;
        Ok(Some(event))
    }

    pub async fn decide_verification(
        &self,
        corp_id: Uuid,
        run_id: Uuid,
        actor_id: Uuid,
        approved: bool,
        note: &str,
    ) -> Result<VerificationDecisionOutcome> {
        let note = note.trim();
        if note.len() > 1_000 {
            return Err(anyhow!(
                "verification decision note cannot exceed 1,000 characters"
            ));
        }
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            r#"
            SELECT request.task_id, request.gate_type, request.gate,
                   request.status AS request_status,
                   task.mission_id, task.status AS task_status,
                   mission.room_id, mission.requested_by, mission.status AS mission_status,
                   run.status AS run_status, run.agent_id,
                   producer.actor_id AS producer_actor_id
            FROM verification_requests request
            JOIN runs run ON run.id = request.run_id
            JOIN tasks task ON task.id = request.task_id
            JOIN missions mission ON mission.id = task.mission_id
            JOIN agents producer ON producer.id = run.agent_id
            WHERE request.run_id = $1 AND request.corp_id = $2
            FOR UPDATE OF request, run, task, mission
            "#,
        )
        .bind(run_id)
        .bind(corp_id)
        .fetch_one(&mut *tx)
        .await
        .context("pending verification request not found")?;
        let task_id: Uuid = row.get("task_id");
        let mission_id: Uuid = row.get("mission_id");
        let room_id: Uuid = row.get("room_id");
        assert_room_membership_tx(&mut tx, corp_id, room_id, actor_id).await?;
        let actor = sqlx::query("SELECT kind, role FROM actors WHERE id = $1 AND corp_id = $2")
            .bind(actor_id)
            .bind(corp_id)
            .fetch_one(&mut *tx)
            .await
            .context("verification actor not found")?;
        let actor_kind: String = actor.get("kind");
        let actor_role: String = actor.get("role");
        if actor_kind != "human" {
            return Err(anyhow!(
                "forbidden: manual verification decisions require a human actor"
            ));
        }
        let gate: ManualVerificationGate =
            serde_json::from_value(row.get("gate")).context("decode verification gate")?;
        let roles = match &gate {
            ManualVerificationGate::HumanApproval { roles }
            | ManualVerificationGate::IndependentReview { roles, .. } => roles,
        };
        if !roles.iter().any(|role| role == &actor_role) {
            return Err(anyhow!(
                "forbidden: actor role {actor_role} cannot decide this verification"
            ));
        }
        if let ManualVerificationGate::IndependentReview {
            exclude_requester, ..
        } = &gate
        {
            let requester: Uuid = row.get("requested_by");
            let producer_actor_id: Uuid = row.get("producer_actor_id");
            if (*exclude_requester && actor_id == requester) || actor_id == producer_actor_id {
                return Err(anyhow!(
                    "forbidden: independent review requires a different actor"
                ));
            }
        }
        let request_status: String = row.get("request_status");
        let run_status: String = row.get("run_status");
        let task_status: String = row.get("task_status");
        if request_status != "pending"
            || run_status != "waiting_for_approval"
            || task_status != "awaiting_approval"
        {
            return Err(anyhow!("verification request is no longer pending"));
        }
        let mission_status: String = row.get("mission_status");
        if matches!(
            mission_status.as_str(),
            "completed" | "failed" | "cancelled"
        ) {
            return Err(anyhow!("mission is already {mission_status}"));
        }
        let agent_id: Uuid = row.get("agent_id");
        let decision_status = if approved { "approved" } else { "rejected" };
        sqlx::query(
            r#"
            UPDATE verification_requests
            SET status = $1, decided_by = $2, decision_note = $3, decided_at = now()
            WHERE run_id = $4
            "#,
        )
        .bind(decision_status)
        .bind(actor_id)
        .bind((!note.is_empty()).then_some(note))
        .bind(run_id)
        .execute(&mut *tx)
        .await?;

        if approved {
            sqlx::query(
                "UPDATE runs SET status = 'completed', verification_status = 'passed', updated_at = now() WHERE id = $1",
            )
            .bind(run_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE tasks SET status = 'completed', verification_status = 'passed', updated_at = now() WHERE id = $1",
            )
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                r#"
                UPDATE tasks child
                SET status = 'ready', updated_at = now()
                WHERE child.mission_id = $1 AND child.status = 'pending'
                  AND NOT EXISTS (
                    SELECT 1 FROM task_dependencies dependency
                    JOIN tasks parent ON parent.id = dependency.depends_on_task_id
                    WHERE dependency.task_id = child.id
                      AND parent.status <> 'completed'
                  )
                "#,
            )
            .bind(mission_id)
            .execute(&mut *tx)
            .await?;
            let mission_complete: bool = sqlx::query_scalar(
                "SELECT NOT EXISTS(SELECT 1 FROM tasks WHERE mission_id = $1 AND status <> 'completed')",
            )
            .bind(mission_id)
            .fetch_one(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE missions SET status = $1, updated_at = now() WHERE id = $2 AND status IN ('ready', 'running')",
            )
            .bind(if mission_complete {
                "completed"
            } else {
                "running"
            })
            .bind(mission_id)
            .execute(&mut *tx)
            .await?;
        } else {
            let rejection = if note.is_empty() {
                "verification rejected by an authorized reviewer"
            } else {
                note
            };
            sqlx::query(
                "UPDATE runs SET status = 'failed', verification_status = 'failed', verification_summary = $1, summary = $1, updated_at = now() WHERE id = $2",
            )
            .bind(rejection)
            .bind(run_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE tasks SET status = 'verification_failed', verification_status = 'failed', updated_at = now() WHERE id = $1",
            )
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE missions SET status = 'failed', updated_at = now() WHERE id = $1 AND status IN ('ready', 'running')",
            )
            .bind(mission_id)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "UPDATE agents SET status = 'idle', station = NULL, current_run_id = NULL WHERE id = $1",
        )
        .bind(agent_id)
        .execute(&mut *tx)
        .await?;
        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(room_id),
                correlation_id: Some(mission_id),
                ..NewEvent::new(
                    corp_id,
                    Some(actor_id),
                    if approved {
                        "verification.approved"
                    } else {
                        "verification.rejected"
                    },
                    "run",
                    run_id,
                    format!("verification-decision:{run_id}:{decision_status}"),
                    json!({
                        "task_id": task_id,
                        "gate_type": row.get::<String, _>("gate_type"),
                        "status": decision_status,
                        "note": note
                    }),
                )
            },
        )
        .await?
        .context("verification decision event unexpectedly existed")?;
        tx.commit().await?;
        Ok(VerificationDecisionOutcome {
            run_id,
            corp_id,
            mission_id,
            status: decision_status.to_owned(),
            event,
        })
    }

    pub async fn acquire_lease(
        &self,
        corp_id: Uuid,
        agent_id: Uuid,
        actor_id: Uuid,
    ) -> Result<LeaseOutcome> {
        let mut tx = self.pool.begin().await?;
        assert_actor_agent_scope_tx(&mut tx, corp_id, actor_id, agent_id).await?;
        let token = Uuid::new_v4();
        let expires_at = Utc::now() + Duration::minutes(5);
        let acquired_row = sqlx::query(
            r#"
            INSERT INTO control_leases (agent_id, corp_id, actor_id, token, expires_at)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (agent_id) DO UPDATE
            SET corp_id = EXCLUDED.corp_id,
                actor_id = EXCLUDED.actor_id,
                token = EXCLUDED.token,
                expires_at = EXCLUDED.expires_at
            WHERE control_leases.expires_at <= now()
               OR control_leases.actor_id = EXCLUDED.actor_id
            RETURNING agent_id, corp_id, actor_id, token, expires_at
            "#,
        )
        .bind(agent_id)
        .bind(corp_id)
        .bind(actor_id)
        .bind(token)
        .bind(expires_at)
        .fetch_optional(&mut *tx)
        .await?;

        let (acquired, lease) = if let Some(row) = acquired_row {
            (true, map_lease(row))
        } else {
            let row = sqlx::query(
                "SELECT agent_id, corp_id, actor_id, token, expires_at FROM control_leases WHERE agent_id = $1",
            )
            .bind(agent_id)
            .fetch_one(&mut *tx)
            .await?;
            (false, map_lease(row))
        };

        let event = if acquired {
            append_event_tx(
                &mut tx,
                NewEvent::new(
                    corp_id,
                    Some(actor_id),
                    "control.lease_acquired",
                    "agent",
                    agent_id,
                    format!("lease-acquire:{agent_id}:{}", Uuid::new_v4()),
                    json!({
                        "actor_id": actor_id,
                        "expires_at": expires_at
                    }),
                ),
            )
            .await?
        } else {
            None
        };
        tx.commit().await?;
        Ok(LeaseOutcome {
            acquired,
            lease,
            event,
        })
    }

    pub async fn release_lease(
        &self,
        corp_id: Uuid,
        agent_id: Uuid,
        actor_id: Uuid,
        token: Uuid,
    ) -> Result<LeaseMutationOutcome> {
        let mut tx = self.pool.begin().await?;
        assert_actor_agent_scope_tx(&mut tx, corp_id, actor_id, agent_id).await?;
        let released = sqlx::query(
            r#"
            DELETE FROM control_leases
            WHERE agent_id = $1 AND corp_id = $2 AND actor_id = $3
              AND token = $4 AND expires_at > now()
            RETURNING agent_id
            "#,
        )
        .bind(agent_id)
        .bind(corp_id)
        .bind(actor_id)
        .bind(token)
        .fetch_optional(&mut *tx)
        .await?;
        if released.is_none() {
            return Err(anyhow!("stale or unauthorized control lease"));
        }

        let event = append_event_tx(
            &mut tx,
            NewEvent::new(
                corp_id,
                Some(actor_id),
                "control.lease_released",
                "agent",
                agent_id,
                format!("lease-release:{agent_id}:{}", Uuid::new_v4()),
                json!({"actor_id": actor_id}),
            ),
        )
        .await?
        .context("lease release event unexpectedly existed")?;
        tx.commit().await?;
        Ok(LeaseMutationOutcome { lease: None, event })
    }

    pub async fn transfer_lease(
        &self,
        corp_id: Uuid,
        agent_id: Uuid,
        actor_id: Uuid,
        token: Uuid,
        to_actor_id: Uuid,
    ) -> Result<LeaseMutationOutcome> {
        let mut tx = self.pool.begin().await?;
        assert_actor_agent_scope_tx(&mut tx, corp_id, actor_id, agent_id).await?;
        assert_actor_scope_tx(&mut tx, corp_id, to_actor_id).await?;
        if actor_id == to_actor_id {
            return Err(anyhow!("lease transfer target must be another actor"));
        }

        let existing = sqlx::query(
            r#"
            SELECT agent_id, corp_id, actor_id, token, expires_at
            FROM control_leases
            WHERE agent_id = $1 AND corp_id = $2
            FOR UPDATE
            "#,
        )
        .bind(agent_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?
        .map(map_lease)
        .context("agent has no active control lease")?;
        if existing.actor_id != actor_id
            || existing.token != token
            || existing.expires_at <= Utc::now()
        {
            return Err(anyhow!("stale or unauthorized control lease"));
        }

        let new_token = Uuid::new_v4();
        let expires_at = Utc::now() + Duration::minutes(5);
        let row = sqlx::query(
            r#"
            UPDATE control_leases
            SET actor_id = $1, token = $2, expires_at = $3
            WHERE agent_id = $4 AND corp_id = $5
            RETURNING agent_id, corp_id, actor_id, token, expires_at
            "#,
        )
        .bind(to_actor_id)
        .bind(new_token)
        .bind(expires_at)
        .bind(agent_id)
        .bind(corp_id)
        .fetch_one(&mut *tx)
        .await?;
        let lease = map_lease(row);
        let event = append_event_tx(
            &mut tx,
            NewEvent::new(
                corp_id,
                Some(actor_id),
                "control.lease_transferred",
                "agent",
                agent_id,
                format!("lease-transfer:{agent_id}:{}", Uuid::new_v4()),
                json!({
                    "from_actor_id": actor_id,
                    "to_actor_id": to_actor_id,
                    "expires_at": expires_at
                }),
            ),
        )
        .await?
        .context("lease transfer event unexpectedly existed")?;
        tx.commit().await?;
        Ok(LeaseMutationOutcome {
            lease: Some(lease),
            event,
        })
    }

    pub async fn queue_message(
        &self,
        corp_id: Uuid,
        agent_id: Uuid,
        actor_id: Uuid,
        lease_token: Option<Uuid>,
        text: &str,
    ) -> Result<MessageOutcome> {
        let text = text.trim();
        if text.is_empty() {
            return Err(anyhow!("message cannot be empty"));
        }
        if text.len() > 4_000 {
            return Err(anyhow!("message cannot exceed 4,000 characters"));
        }

        let mut tx = self.pool.begin().await?;
        assert_actor_agent_scope_tx(&mut tx, corp_id, actor_id, agent_id).await?;
        let active_lease = sqlx::query(
            r#"
            SELECT agent_id, corp_id, actor_id, token, expires_at
            FROM control_leases
            WHERE agent_id = $1 AND corp_id = $2
            FOR UPDATE
            "#,
        )
        .bind(agent_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?
        .map(map_lease)
        .filter(|lease| lease.expires_at > Utc::now());

        let holds_lease = match (&active_lease, lease_token) {
            (Some(lease), Some(supplied))
                if lease.actor_id == actor_id && lease.token == supplied =>
            {
                true
            }
            (Some(lease), None) if lease.actor_id != actor_id => false,
            (None, None) => false,
            _ => return Err(anyhow!("stale or unauthorized control lease token")),
        };

        let active_run = sqlx::query(
            r#"
            SELECT id, runner_id FROM runs
            WHERE agent_id = $1 AND corp_id = $2
              AND status IN ('starting', 'running', 'waiting_for_input',
                             'waiting_for_approval', 'verifying')
            ORDER BY created_at DESC LIMIT 1
            "#,
        )
        .bind(agent_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?;

        let delivery = if holds_lease && active_run.is_some() {
            "immediate"
        } else {
            "queued"
        };
        let message_id = Uuid::new_v4();
        let row = sqlx::query(
            r#"
            INSERT INTO queued_messages (id, corp_id, agent_id, actor_id, text, status)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, corp_id, agent_id, actor_id, text, status, created_at
            "#,
        )
        .bind(message_id)
        .bind(corp_id)
        .bind(agent_id)
        .bind(actor_id)
        .bind(text)
        .bind(delivery)
        .fetch_one(&mut *tx)
        .await?;
        let message = map_queued_message(row);
        let event_type = if delivery == "immediate" {
            "control.message_accepted"
        } else {
            "control.message_queued"
        };
        let event = append_event_tx(
            &mut tx,
            NewEvent::new(
                corp_id,
                Some(actor_id),
                event_type,
                "agent",
                agent_id,
                format!("control-message:{message_id}"),
                json!({
                    "message_id": message_id,
                    "delivery": delivery,
                    "text": text
                }),
            ),
        )
        .await?
        .context("control message event unexpectedly existed")?;
        tx.commit().await?;

        let (run_id, runner_id) = active_run
            .map(|row| (Some(row.get("id")), Some(row.get("runner_id"))))
            .unwrap_or((None, None));
        Ok(MessageOutcome {
            message,
            delivery: delivery.to_owned(),
            run_id,
            runner_id,
            event,
        })
    }

    pub async fn create_room_message(
        &self,
        input: NewRoomMessageInput,
    ) -> Result<RoomMessageOutcome> {
        let NewRoomMessageInput {
            corp_id,
            room_id,
            actor_id,
            body,
            reply_to_id,
            mentions,
            link,
        } = input;
        let body = body.trim();
        if body.is_empty() {
            return Err(anyhow!("room message cannot be empty"));
        }
        if body.len() > 4_000 {
            return Err(anyhow!("room message cannot exceed 4,000 characters"));
        }

        let mut tx = self.pool.begin().await?;
        assert_room_membership_tx(&mut tx, corp_id, room_id, actor_id).await?;

        let mut seen_mentions = HashSet::new();
        let mentions = mentions
            .into_iter()
            .filter(|mentioned| seen_mentions.insert(*mentioned))
            .collect::<Vec<_>>();
        if mentions.len() > 20 {
            return Err(anyhow!("room message cannot mention more than 20 actors"));
        }
        for mentioned in &mentions {
            assert_room_membership_tx(&mut tx, corp_id, room_id, *mentioned).await?;
        }

        let (thread_root_id, reply_to_id) = if let Some(reply_to_id) = reply_to_id {
            let row = sqlx::query(
                r#"
                SELECT id, thread_root_id
                FROM room_messages
                WHERE id = $1 AND corp_id = $2 AND room_id = $3
                "#,
            )
            .bind(reply_to_id)
            .bind(corp_id)
            .bind(room_id)
            .fetch_optional(&mut *tx)
            .await?
            .context("reply target is not visible in this room")?;
            let root = row
                .try_get::<Option<Uuid>, _>("thread_root_id")?
                .unwrap_or_else(|| row.get("id"));
            (Some(root), Some(reply_to_id))
        } else {
            (None, None)
        };

        if let Some(link) = &link {
            validate_room_link_tx(&mut tx, corp_id, room_id, link).await?;
        }

        let message_id = Uuid::new_v4();
        let link_kind = link.as_ref().map(|link| link.kind.as_str());
        let link_id = link.as_ref().map(|link| link.id);
        let row = sqlx::query(
            r#"
            INSERT INTO room_messages
                (id, corp_id, room_id, actor_id, thread_root_id, reply_to_id,
                 body, mentions, link_kind, link_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            RETURNING id, corp_id, room_id, actor_id, thread_root_id, reply_to_id,
                      body, mentions, link_kind, link_id, created_at
            "#,
        )
        .bind(message_id)
        .bind(corp_id)
        .bind(room_id)
        .bind(actor_id)
        .bind(thread_root_id)
        .bind(reply_to_id)
        .bind(body)
        .bind(&mentions)
        .bind(link_kind)
        .bind(link_id)
        .fetch_one(&mut *tx)
        .await?;
        let message = map_room_message(row);
        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(room_id),
                ..NewEvent::new(
                    corp_id,
                    Some(actor_id),
                    "room.message_posted",
                    "room_message",
                    message_id,
                    format!("room-message:{message_id}"),
                    json!({
                        "message_id": message_id,
                        "thread_root_id": thread_root_id,
                        "reply_to_id": reply_to_id,
                        "mentions": mentions,
                        "link": link,
                        "body": body
                    }),
                )
            },
        )
        .await?
        .context("room message event unexpectedly existed")?;
        tx.commit().await?;
        Ok(RoomMessageOutcome { message, event })
    }

    pub async fn request_interrupt(
        &self,
        corp_id: Uuid,
        agent_id: Uuid,
        actor_id: Uuid,
        lease_token: Uuid,
        reason: &str,
    ) -> Result<StopRequestOutcome> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(anyhow!("interrupt reason cannot be empty"));
        }
        if reason.len() > 500 {
            return Err(anyhow!("interrupt reason cannot exceed 500 characters"));
        }

        let mut tx = self.pool.begin().await?;
        assert_actor_agent_scope_tx(&mut tx, corp_id, actor_id, agent_id).await?;
        let lease = sqlx::query(
            r#"
            SELECT agent_id, corp_id, actor_id, token, expires_at
            FROM control_leases
            WHERE agent_id = $1 AND corp_id = $2
            FOR UPDATE
            "#,
        )
        .bind(agent_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?
        .map(map_lease)
        .context("agent has no active control lease")?;
        if lease.actor_id != actor_id
            || lease.token != lease_token
            || lease.expires_at <= Utc::now()
        {
            return Err(anyhow!("stale or unauthorized control lease token"));
        }

        let row = sqlx::query(
            r#"
            SELECT r.id, r.runner_id, t.mission_id, m.room_id
            FROM runs r
            JOIN tasks t ON t.id = r.task_id
            JOIN missions m ON m.id = t.mission_id
            WHERE r.agent_id = $1 AND r.corp_id = $2
              AND r.status IN ('provisioning', 'starting', 'running',
                               'waiting_for_input', 'waiting_for_approval', 'verifying')
            ORDER BY r.created_at DESC
            LIMIT 1
            FOR UPDATE OF r
            "#,
        )
        .bind(agent_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?
        .context("agent has no active run to interrupt")?;
        let run_id: Uuid = row.get("id");
        let runner_id: String = row.get("runner_id");
        let mission_id: Uuid = row.get("mission_id");
        let room_id: Uuid = row.get("room_id");
        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(room_id),
                correlation_id: Some(mission_id),
                ..NewEvent::new(
                    corp_id,
                    Some(actor_id),
                    "run.interrupt_requested",
                    "run",
                    run_id,
                    format!("run-interrupt:{run_id}:{}", Uuid::new_v4()),
                    json!({"agent_id": agent_id, "reason": reason}),
                )
            },
        )
        .await?
        .context("run interrupt event unexpectedly existed")?;
        tx.commit().await?;
        Ok(StopRequestOutcome {
            run_id,
            runner_id,
            event,
        })
    }

    pub async fn request_emergency_stop(
        &self,
        corp_id: Uuid,
        agent_id: Uuid,
        actor_id: Uuid,
        reason: &str,
    ) -> Result<StopRequestOutcome> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(anyhow!("emergency stop reason cannot be empty"));
        }
        if reason.len() > 500 {
            return Err(anyhow!(
                "emergency stop reason cannot exceed 500 characters"
            ));
        }

        let mut tx = self.pool.begin().await?;
        assert_actor_agent_scope_tx(&mut tx, corp_id, actor_id, agent_id).await?;
        let role: String =
            sqlx::query_scalar("SELECT role FROM actors WHERE id = $1 AND corp_id = $2")
                .bind(actor_id)
                .bind(corp_id)
                .fetch_one(&mut *tx)
                .await?;
        if !matches!(role.as_str(), "owner" | "admin" | "manager") {
            return Err(anyhow!(
                "forbidden: actor role {role} cannot emergency-stop runs"
            ));
        }

        let row = sqlx::query(
            r#"
            SELECT r.id, r.runner_id, t.mission_id, m.room_id
            FROM runs r
            JOIN tasks t ON t.id = r.task_id
            JOIN missions m ON m.id = t.mission_id
            WHERE r.agent_id = $1 AND r.corp_id = $2
              AND r.status IN ('provisioning', 'starting', 'running',
                               'waiting_for_input', 'waiting_for_approval', 'verifying')
            ORDER BY r.created_at DESC
            LIMIT 1
            FOR UPDATE OF r
            "#,
        )
        .bind(agent_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?
        .context("agent has no active run to stop")?;
        let run_id: Uuid = row.get("id");
        let runner_id: String = row.get("runner_id");
        let mission_id: Uuid = row.get("mission_id");
        let room_id: Uuid = row.get("room_id");
        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(room_id),
                correlation_id: Some(mission_id),
                ..NewEvent::new(
                    corp_id,
                    Some(actor_id),
                    "run.stop_requested",
                    "run",
                    run_id,
                    format!("run-stop:{run_id}:{}", Uuid::new_v4()),
                    json!({"agent_id": agent_id, "reason": reason}),
                )
            },
        )
        .await?
        .context("run stop event unexpectedly existed")?;
        tx.commit().await?;
        Ok(StopRequestOutcome {
            run_id,
            runner_id,
            event,
        })
    }
}

fn format_task_prompt(
    mission_title: &str,
    task_title: &str,
    contract: &TaskContract,
    attempt: i32,
) -> String {
    let list = |values: &[String]| {
        values
            .iter()
            .map(|value| format!("- {value}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "MISSION: {mission_title}\n\
         TASK: {task_title}\n\
         ATTEMPT: {attempt}\n\
         OBJECTIVE: {}\n\
         EXPECTED OUTPUT: {}\n\
         ACCEPTANCE TESTS:\n{}\n\
         ALLOWED TOOLS:\n{}\n\
         PROHIBITED ACTIONS:\n{}\n\
         REFERENCES:\n{}\n\
         WRITE SCOPE:\n{}\n\
         TOKEN BUDGET: {}\n\
         DEADLINE: {}\n\
         ESCALATION: {}",
        contract.objective,
        contract.expected_output,
        list(&contract.acceptance_tests),
        list(&contract.allowed_tools),
        list(&contract.prohibited_actions),
        if contract.references.is_empty() {
            "- none".to_owned()
        } else {
            list(&contract.references)
        },
        list(&contract.write_scope),
        contract.budget_tokens,
        contract
            .deadline_at
            .as_ref()
            .map(|deadline| deadline.to_rfc3339())
            .unwrap_or_else(|| "none".to_owned()),
        contract.escalation,
    )
}

async fn lock_demo_tx(tx: &mut Transaction<'_, Postgres>) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(DEMO_ADVISORY_LOCK)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn active_runner_run_rows(
    tx: &mut Transaction<'_, Postgres>,
    runner_id: &str,
) -> Result<Vec<sqlx::postgres::PgRow>> {
    let rows = sqlx::query(
        r#"
        SELECT r.id AS run_id, r.corp_id, r.task_id, r.agent_id,
               t.mission_id, m.room_id
        FROM runs r
        JOIN tasks t ON t.id = r.task_id
        JOIN missions m ON m.id = t.mission_id
        WHERE r.runner_id = $1
          AND r.status IN ('provisioning', 'starting', 'running',
                           'waiting_for_input', 'waiting_for_approval', 'verifying')
        ORDER BY r.created_at
        FOR UPDATE OF r, t, m
        "#,
    )
    .bind(runner_id)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows)
}

async fn mark_runner_runs_lost_tx(
    tx: &mut Transaction<'_, Postgres>,
    runner_id: &str,
    excluded_run_ids: &[Uuid],
    reason: &str,
) -> Result<Vec<DomainEvent>> {
    let rows = sqlx::query(
        r#"
        SELECT r.id AS run_id, r.corp_id, r.task_id, r.agent_id,
               t.mission_id, m.room_id
        FROM runs r
        JOIN tasks t ON t.id = r.task_id
        JOIN missions m ON m.id = t.mission_id
        WHERE r.runner_id = $1
          AND r.status IN ('provisioning', 'starting', 'running',
                           'waiting_for_input', 'waiting_for_approval', 'verifying')
          AND (
            cardinality($2::uuid[]) = 0
            OR NOT (r.id = ANY($2::uuid[]))
          )
        ORDER BY r.created_at
        FOR UPDATE OF r, t, m
        "#,
    )
    .bind(runner_id)
    .bind(excluded_run_ids)
    .fetch_all(&mut **tx)
    .await?;

    let mut events = Vec::with_capacity(rows.len());
    for row in rows {
        let run_id: Uuid = row.get("run_id");
        let corp_id: Uuid = row.get("corp_id");
        let task_id: Uuid = row.get("task_id");
        let agent_id: Uuid = row.get("agent_id");
        let mission_id: Uuid = row.get("mission_id");
        let room_id: Uuid = row.get("room_id");
        sqlx::query(
            "UPDATE runs SET status = 'lost', summary = $1, updated_at = now() WHERE id = $2",
        )
        .bind(reason)
        .bind(run_id)
        .execute(&mut **tx)
        .await?;
        sqlx::query("UPDATE tasks SET status = 'blocked', updated_at = now() WHERE id = $1")
            .bind(task_id)
            .execute(&mut **tx)
            .await?;
        sqlx::query(
            "UPDATE agents SET status = 'offline', station = NULL, current_run_id = NULL WHERE id = $1",
        )
        .bind(agent_id)
        .execute(&mut **tx)
        .await?;
        if let Some(event) = append_event_tx(
            tx,
            NewEvent {
                room_id: Some(room_id),
                correlation_id: Some(mission_id),
                ..NewEvent::new(
                    corp_id,
                    None,
                    "run.lost",
                    "run",
                    run_id,
                    format!("run-lost:{run_id}"),
                    json!({"runner_id": runner_id, "reason": reason}),
                )
            },
        )
        .await?
        {
            events.push(event);
        }
    }
    Ok(events)
}

async fn assert_actor_agent_scope_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    actor_id: Uuid,
    agent_id: Uuid,
) -> Result<()> {
    assert_actor_scope_tx(tx, corp_id, actor_id).await?;
    let agent_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agents WHERE id = $1 AND corp_id = $2)")
            .bind(agent_id)
            .bind(corp_id)
            .fetch_one(&mut **tx)
            .await?;
    if !agent_exists {
        return Err(anyhow!("agent does not belong to the requested Corp"));
    }
    Ok(())
}

async fn assert_actor_scope_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    actor_id: Uuid,
) -> Result<()> {
    let actor_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM actors WHERE id = $1 AND corp_id = $2)")
            .bind(actor_id)
            .bind(corp_id)
            .fetch_one(&mut **tx)
            .await?;
    if !actor_exists {
        return Err(anyhow!("actor does not belong to the requested Corp"));
    }
    Ok(())
}

async fn assert_room_membership_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    room_id: Uuid,
    actor_id: Uuid,
) -> Result<()> {
    let member: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM room_memberships rm
            JOIN rooms r ON r.id = rm.room_id
            WHERE rm.room_id = $1 AND rm.actor_id = $2 AND r.corp_id = $3
        )
        "#,
    )
    .bind(room_id)
    .bind(actor_id)
    .bind(corp_id)
    .fetch_one(&mut **tx)
    .await?;
    if !member {
        return Err(anyhow!("forbidden: actor is not a member of this room"));
    }
    Ok(())
}

async fn validate_room_link_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    room_id: Uuid,
    link: &EntityLink,
) -> Result<()> {
    let valid: bool = match link.kind.as_str() {
        "mission" => sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM missions WHERE id = $1 AND corp_id = $2 AND room_id = $3)",
        )
        .bind(link.id)
        .bind(corp_id)
        .bind(room_id)
        .fetch_one(&mut **tx)
        .await?,
        "task" => {
            sqlx::query_scalar(
                r#"
                SELECT EXISTS(
                    SELECT 1 FROM tasks t
                    JOIN missions m ON m.id = t.mission_id
                    WHERE t.id = $1 AND t.corp_id = $2 AND m.room_id = $3
                )
                "#,
            )
            .bind(link.id)
            .bind(corp_id)
            .bind(room_id)
            .fetch_one(&mut **tx)
            .await?
        }
        "run" => {
            sqlx::query_scalar(
                r#"
                SELECT EXISTS(
                    SELECT 1 FROM runs r
                    JOIN tasks t ON t.id = r.task_id
                    JOIN missions m ON m.id = t.mission_id
                    WHERE r.id = $1 AND r.corp_id = $2 AND m.room_id = $3
                )
                "#,
            )
            .bind(link.id)
            .bind(corp_id)
            .bind(room_id)
            .fetch_one(&mut **tx)
            .await?
        }
        "artifact" => {
            sqlx::query_scalar(
                r#"
                SELECT EXISTS(
                    SELECT 1 FROM runs r
                    JOIN tasks t ON t.id = r.task_id
                    JOIN missions m ON m.id = t.mission_id
                    WHERE r.id = $1 AND r.corp_id = $2 AND m.room_id = $3
                      AND r.artifact_sha256 IS NOT NULL
                )
                "#,
            )
            .bind(link.id)
            .bind(corp_id)
            .bind(room_id)
            .fetch_one(&mut **tx)
            .await?
        }
        other => return Err(anyhow!("unsupported room message link kind {other}")),
    };
    if !valid {
        return Err(anyhow!("linked entity is not visible in this room"));
    }
    Ok(())
}

async fn append_event_tx(
    tx: &mut Transaction<'_, Postgres>,
    event: NewEvent,
) -> Result<Option<DomainEvent>> {
    let row = sqlx::query(
        r#"
        INSERT INTO events
            (id, schema_version, corp_id, room_id, actor_id, type, aggregate_type,
             aggregate_id, aggregate_version, correlation_id, causation_id,
             idempotency_key, visibility, payload)
        VALUES ($1, 1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
        ON CONFLICT (corp_id, idempotency_key) DO NOTHING
        RETURNING seq, id, schema_version, corp_id, room_id, actor_id, type,
                  aggregate_type, aggregate_id, aggregate_version, correlation_id,
                  causation_id, idempotency_key, visibility, payload, created_at
        "#,
    )
    .bind(event.id)
    .bind(event.corp_id)
    .bind(event.room_id)
    .bind(event.actor_id)
    .bind(event.event_type)
    .bind(event.aggregate_type)
    .bind(event.aggregate_id)
    .bind(event.aggregate_version)
    .bind(event.correlation_id)
    .bind(event.causation_id)
    .bind(event.idempotency_key)
    .bind(event.visibility)
    .bind(event.payload)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(map_event))
}

fn parse_id(value: &str) -> Result<Uuid> {
    Uuid::parse_str(value).with_context(|| format!("invalid fixed UUID {value}"))
}

fn map_corp(row: sqlx::postgres::PgRow) -> Corp {
    Corp {
        id: row.get("id"),
        slug: row.get("slug"),
        name: row.get("name"),
        created_at: row.get("created_at"),
    }
}

fn map_actor(row: sqlx::postgres::PgRow) -> Result<Actor> {
    Ok(Actor {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        name: row.get("name"),
        kind: parse_actor_kind(row.get::<String, _>("kind").as_str())?,
        role: row.get("role"),
        created_at: row.get("created_at"),
    })
}

fn map_room(row: sqlx::postgres::PgRow) -> Room {
    Room {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        name: row.get("name"),
        purpose: row.get("purpose"),
        created_at: row.get("created_at"),
    }
}

fn map_agent(row: sqlx::postgres::PgRow) -> Result<Agent> {
    Ok(Agent {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        actor_id: row.get("actor_id"),
        name: row.get("name"),
        role: row.get("role"),
        adapter: row.get("adapter"),
        status: parse_agent_status(row.get::<String, _>("status").as_str())?,
        station: row.get("station"),
        current_run_id: row.get("current_run_id"),
        accent: row.get("accent"),
        created_at: row.get("created_at"),
    })
}

fn map_mission(row: sqlx::postgres::PgRow) -> Result<Mission> {
    Ok(Mission {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        room_id: row.get("room_id"),
        requested_by: row.get("requested_by"),
        title: row.get("title"),
        strategy: row.get("strategy"),
        max_nodes: row.get("max_nodes"),
        max_depth: row.get("max_depth"),
        budget_tokens: row.get("budget_tokens"),
        status: parse_mission_status(row.get::<String, _>("status").as_str())?,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn map_task(row: sqlx::postgres::PgRow) -> Result<Task> {
    Ok(Task {
        id: row.get("id"),
        mission_id: row.get("mission_id"),
        corp_id: row.get("corp_id"),
        title: row.get("title"),
        objective: row.get("objective"),
        plan_key: row.get("plan_key"),
        contract: serde_json::from_value(row.get("contract")).context("decode task contract")?,
        depth: row.get("depth"),
        max_attempts: row.get("max_attempts"),
        attempt_count: row.get("attempt_count"),
        required_adapter: row.get("required_adapter"),
        depends_on: Vec::new(),
        verification_policy: serde_json::from_value(row.get("verification_policy"))
            .context("decode verification policy")?,
        verification_status: row.get("verification_status"),
        status: parse_task_status(row.get::<String, _>("status").as_str())?,
        assigned_agent_id: row.get("assigned_agent_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn map_run(row: sqlx::postgres::PgRow) -> Result<Run> {
    Ok(Run {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        task_id: row.get("task_id"),
        agent_id: row.get("agent_id"),
        runner_id: row.get("runner_id"),
        assignment_token: row.get("assignment_token"),
        provider_session_id: row.get("provider_session_id"),
        resumed_from_run_id: row.get("resumed_from_run_id"),
        workspace_run_id: row.get("workspace_run_id"),
        input_tokens: row.get("input_tokens"),
        output_tokens: row.get("output_tokens"),
        cost_microusd: row.get("cost_microusd"),
        workspace_path: row.get("workspace_path"),
        workspace_branch: row.get("workspace_branch"),
        workspace_base_ref: row.get("workspace_base_ref"),
        workspace_base_commit: row.get("workspace_base_commit"),
        workspace_disposition: row.get("workspace_disposition"),
        workspace_detail: row.get("workspace_detail"),
        verification_status: row.get("verification_status"),
        verification_summary: row.get("verification_summary"),
        status: parse_run_status(row.get::<String, _>("status").as_str())?,
        summary: row.get("summary"),
        artifact_path: row.get("artifact_path"),
        artifact_sha256: row.get("artifact_sha256"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn map_verification_evidence(row: sqlx::postgres::PgRow) -> VerificationEvidence {
    VerificationEvidence {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        task_id: row.get("task_id"),
        run_id: row.get("run_id"),
        check_index: row.get("check_index"),
        kind: row.get("kind"),
        status: row.get("status"),
        summary: row.get("summary"),
        payload: row.get("payload"),
        created_at: row.get("created_at"),
    }
}

fn map_verification_request(row: sqlx::postgres::PgRow) -> Result<VerificationRequest> {
    Ok(VerificationRequest {
        run_id: row.get("run_id"),
        corp_id: row.get("corp_id"),
        task_id: row.get("task_id"),
        gate_type: row.get("gate_type"),
        gate: serde_json::from_value(row.get("gate")).context("decode verification gate")?,
        status: row.get("status"),
        requested_at: row.get("requested_at"),
        decided_by: row.get("decided_by"),
        decision_note: row.get("decision_note"),
        decided_at: row.get("decided_at"),
    })
}

fn map_lease(row: sqlx::postgres::PgRow) -> ControlLease {
    ControlLease {
        agent_id: row.get("agent_id"),
        corp_id: row.get("corp_id"),
        actor_id: row.get("actor_id"),
        token: row.get("token"),
        expires_at: row.get("expires_at"),
    }
}

fn map_queued_message(row: sqlx::postgres::PgRow) -> QueuedMessage {
    QueuedMessage {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        agent_id: row.get("agent_id"),
        actor_id: row.get("actor_id"),
        text: row.get("text"),
        status: row.get("status"),
        created_at: row.get("created_at"),
    }
}

fn map_room_message(row: sqlx::postgres::PgRow) -> RoomMessage {
    let link_kind: Option<String> = row.get("link_kind");
    let link_id: Option<Uuid> = row.get("link_id");
    let link = match (link_kind, link_id) {
        (Some(kind), Some(id)) => Some(EntityLink { kind, id }),
        _ => None,
    };
    RoomMessage {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        room_id: row.get("room_id"),
        actor_id: row.get("actor_id"),
        thread_root_id: row.get("thread_root_id"),
        reply_to_id: row.get("reply_to_id"),
        body: row.get("body"),
        mentions: row.get("mentions"),
        link,
        created_at: row.get("created_at"),
    }
}

fn map_runner(row: sqlx::postgres::PgRow) -> RunnerRecord {
    RunnerRecord {
        id: row.get("id"),
        hostname: row.get("hostname"),
        os: row.get("os"),
        capabilities: row.get("capabilities"),
        connection_epoch: row.get("connection_epoch"),
        status: row.get("status"),
        last_seen_at: row.get("last_seen_at"),
        grace_expires_at: row.get("grace_expires_at"),
    }
}

fn map_event(row: sqlx::postgres::PgRow) -> DomainEvent {
    DomainEvent {
        seq: row.get("seq"),
        id: row.get("id"),
        schema_version: row.get("schema_version"),
        corp_id: row.get("corp_id"),
        room_id: row.get("room_id"),
        actor_id: row.get("actor_id"),
        event_type: row.get("type"),
        aggregate_type: row.get("aggregate_type"),
        aggregate_id: row.get("aggregate_id"),
        aggregate_version: row.get("aggregate_version"),
        correlation_id: row.get("correlation_id"),
        causation_id: row.get("causation_id"),
        idempotency_key: row.get("idempotency_key"),
        visibility: row.get("visibility"),
        payload: row.get("payload"),
        created_at: row.get("created_at"),
    }
}

fn parse_actor_kind(value: &str) -> Result<ActorKind> {
    match value {
        "human" => Ok(ActorKind::Human),
        "agent" => Ok(ActorKind::Agent),
        "service" => Ok(ActorKind::Service),
        other => Err(anyhow!("unknown actor kind {other}")),
    }
}

fn parse_agent_status(value: &str) -> Result<AgentStatus> {
    match value {
        "idle" => Ok(AgentStatus::Idle),
        "starting" => Ok(AgentStatus::Starting),
        "working" => Ok(AgentStatus::Working),
        "blocked" => Ok(AgentStatus::Blocked),
        "reviewing" => Ok(AgentStatus::Reviewing),
        "offline" => Ok(AgentStatus::Offline),
        other => Err(anyhow!("unknown agent status {other}")),
    }
}

fn parse_mission_status(value: &str) -> Result<MissionStatus> {
    match value {
        "draft" => Ok(MissionStatus::Draft),
        "ready" => Ok(MissionStatus::Ready),
        "running" => Ok(MissionStatus::Running),
        "completed" => Ok(MissionStatus::Completed),
        "failed" => Ok(MissionStatus::Failed),
        "cancelled" => Ok(MissionStatus::Cancelled),
        other => Err(anyhow!("unknown mission status {other}")),
    }
}

fn parse_task_status(value: &str) -> Result<TaskStatus> {
    match value {
        "pending" => Ok(TaskStatus::Pending),
        "ready" => Ok(TaskStatus::Ready),
        "claimed" => Ok(TaskStatus::Claimed),
        "running" => Ok(TaskStatus::Running),
        "blocked" => Ok(TaskStatus::Blocked),
        "awaiting_approval" => Ok(TaskStatus::AwaitingApproval),
        "verification_failed" => Ok(TaskStatus::VerificationFailed),
        "review" => Ok(TaskStatus::Review),
        "completed" => Ok(TaskStatus::Completed),
        "failed" => Ok(TaskStatus::Failed),
        "cancelled" => Ok(TaskStatus::Cancelled),
        other => Err(anyhow!("unknown task status {other}")),
    }
}

fn parse_run_status(value: &str) -> Result<RunStatus> {
    match value {
        "provisioning" => Ok(RunStatus::Provisioning),
        "starting" => Ok(RunStatus::Starting),
        "running" => Ok(RunStatus::Running),
        "waiting_for_input" => Ok(RunStatus::WaitingForInput),
        "waiting_for_approval" => Ok(RunStatus::WaitingForApproval),
        "verifying" => Ok(RunStatus::Verifying),
        "completed" => Ok(RunStatus::Completed),
        "failed" => Ok(RunStatus::Failed),
        "cancelled" => Ok(RunStatus::Cancelled),
        "lost" => Ok(RunStatus::Lost),
        other => Err(anyhow!("unknown run status {other}")),
    }
}
