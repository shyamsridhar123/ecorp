use std::collections::HashSet;

use anyhow::{Context, Result, anyhow};
use chrono::{Duration, Utc};
use crony_domain::{
    Actor, ActorKind, Agent, AgentStatus, ControlLease, Corp, CorpSnapshot, DomainEvent,
    EntityLink, Mission, MissionStatus, NewEvent, QueuedMessage, Room, RoomMessage, Run, RunStatus,
    Task, TaskStatus,
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
const DEMO_MANAGER_AGENT_ID: &str = "00000000-0000-4000-8000-000000000031";
const DEMO_WORKER_AGENT_ID: &str = "00000000-0000-4000-8000-000000000032";
const DEMO_ROOM_ID: &str = "00000000-0000-4000-8000-000000000041";

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
}

#[derive(Debug, Clone)]
pub struct MissionTaskIds {
    pub mission_id: Uuid,
    pub task_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct LaunchRecord {
    pub corp_id: Uuid,
    pub room_id: Uuid,
    pub mission_id: Uuid,
    pub task_id: Uuid,
    pub run_id: Uuid,
    pub agent_id: Uuid,
    pub mission_title: String,
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

    pub async fn bootstrap_demo(&self) -> Result<(DemoIds, Option<DomainEvent>)> {
        let ids = DemoIds {
            corp_id: parse_id(DEMO_CORP_ID)?,
            alice_actor_id: parse_id(DEMO_ALICE_ID)?,
            bob_actor_id: parse_id(DEMO_BOB_ID)?,
            eve_actor_id: parse_id(DEMO_EVE_ID)?,
            manager_agent_id: parse_id(DEMO_MANAGER_AGENT_ID)?,
            worker_agent_id: parse_id(DEMO_WORKER_AGENT_ID)?,
            room_id: parse_id(DEMO_ROOM_ID)?,
        };
        let manager_actor_id = parse_id(DEMO_MANAGER_ACTOR_ID)?;
        let worker_actor_id = parse_id(DEMO_WORKER_ACTOR_ID)?;

        let mut tx = self.pool.begin().await?;
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

        for (id, actor_id, name, role, accent) in [
            (
                ids.manager_agent_id,
                manager_actor_id,
                "Margo",
                "manager",
                "marigold",
            ),
            (
                ids.worker_agent_id,
                worker_actor_id,
                "Wally",
                "engineer",
                "cobalt",
            ),
        ] {
            sqlx::query(
                r#"
                INSERT INTO agents
                    (id, corp_id, actor_id, name, role, adapter, status, accent)
                VALUES ($1, $2, $3, $4, $5, 'fake-process', 'idle', $6)
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
                    "actors": ["Alice", "Bob", "Eve", "Margo", "Wally"]
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
            SELECT m.id, m.corp_id, m.room_id, m.requested_by, m.title, m.status,
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

        let tasks = sqlx::query(
            r#"
            SELECT t.id, t.mission_id, t.corp_id, t.title, t.objective, t.status,
                   t.assigned_agent_id, t.created_at, t.updated_at
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

        let runs = sqlx::query(
            r#"
            SELECT r.id, r.corp_id, r.task_id, r.agent_id, r.runner_id, r.status,
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

    pub async fn create_mission(
        &self,
        corp_id: Uuid,
        requested_by: Uuid,
        title: &str,
    ) -> Result<(MissionTaskIds, Vec<DomainEvent>)> {
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
        let worker_agent_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM agents WHERE corp_id = $1 AND role <> 'manager' ORDER BY created_at LIMIT 1",
        )
        .bind(corp_id)
        .fetch_one(&mut *tx)
        .await
        .context("corp has no worker agent")?;

        let mission_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO missions
                (id, corp_id, room_id, requested_by, title, status)
            VALUES ($1, $2, $3, $4, $5, 'ready')
            "#,
        )
        .bind(mission_id)
        .bind(corp_id)
        .bind(room_id)
        .bind(requested_by)
        .bind(title)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"
            INSERT INTO tasks
                (id, mission_id, corp_id, title, objective, status, assigned_agent_id)
            VALUES ($1, $2, $3, $4, $5, 'ready', $6)
            "#,
        )
        .bind(task_id)
        .bind(mission_id)
        .bind(corp_id)
        .bind("Produce a verified mission artifact")
        .bind(format!(
            "OBJECTIVE: {title}\nOUTPUT: a source-backed markdown artifact\nTOOLS: local process and filesystem\nBOUNDARIES: stay inside the assigned run workspace"
        ))
        .bind(worker_agent_id)
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
                    json!({"title": title, "status": "ready"}),
                )
            },
        )
        .await?
        .context("mission created event unexpectedly existed")?;
        let task_event = append_event_tx(
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
                        "title": "Produce a verified mission artifact",
                        "assigned_agent_id": worker_agent_id,
                        "status": "ready"
                    }),
                )
            },
        )
        .await?
        .context("task created event unexpectedly existed")?;
        tx.commit().await?;

        Ok((
            MissionTaskIds {
                mission_id,
                task_id,
            },
            vec![mission_event, task_event],
        ))
    }

    pub async fn create_run(
        &self,
        corp_id: Uuid,
        mission_id: Uuid,
        requested_by: Uuid,
        runner_id: &str,
    ) -> Result<(LaunchRecord, DomainEvent)> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            r#"
            SELECT m.room_id, m.title, t.id AS task_id, t.assigned_agent_id
            FROM missions m
            JOIN tasks t ON t.mission_id = m.id
            WHERE m.id = $1 AND m.corp_id = $2
            ORDER BY t.created_at LIMIT 1
            FOR UPDATE OF m, t
            "#,
        )
        .bind(mission_id)
        .bind(corp_id)
        .fetch_one(&mut *tx)
        .await
        .context("mission or task not found")?;

        let task_id: Uuid = row.get("task_id");
        let agent_id: Uuid = row
            .try_get::<Option<Uuid>, _>("assigned_agent_id")?
            .context("task has no assigned agent")?;
        let room_id: Uuid = row.get("room_id");
        let mission_title: String = row.get("title");

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

        let run_id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO runs (id, corp_id, task_id, agent_id, runner_id, status)
            VALUES ($1, $2, $3, $4, $5, 'starting')
            "#,
        )
        .bind(run_id)
        .bind(corp_id)
        .bind(task_id)
        .bind(agent_id)
        .bind(runner_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE missions SET status = 'running', updated_at = now() WHERE id = $1")
            .bind(mission_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE tasks SET status = 'claimed', updated_at = now() WHERE id = $1")
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
                    "run.requested",
                    "run",
                    run_id,
                    format!("run:{run_id}:requested"),
                    json!({
                        "task_id": task_id,
                        "agent_id": agent_id,
                        "runner_id": runner_id,
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
                mission_title,
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
            SELECT r.task_id, t.mission_id, m.room_id
            FROM runs r
            JOIN tasks t ON t.id = r.task_id
            JOIN missions m ON m.id = t.mission_id
            WHERE r.id = $1 AND r.corp_id = $2 AND r.agent_id = $3 AND r.runner_id = $4
            FOR UPDATE OF r, t, m
            "#,
        )
        .bind(run_id)
        .bind(corp_id)
        .bind(agent_id)
        .bind(&runner_id)
        .fetch_one(&mut *tx)
        .await
        .context("runner event does not match an active run")?;
        let task_id: Uuid = row.get("task_id");
        let mission_id: Uuid = row.get("mission_id");
        let room_id: Uuid = row.get("room_id");

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
            "run.started" => {
                sqlx::query("UPDATE runs SET status = 'running', updated_at = now() WHERE id = $1")
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
                    "UPDATE agents SET status = 'reviewing', station = 'review' WHERE id = $1",
                )
                .bind(agent_id)
                .execute(&mut *tx)
                .await?;
            }
            "run.completed" => {
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
                    "UPDATE tasks SET status = 'completed', updated_at = now() WHERE id = $1",
                )
                .bind(task_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE missions SET status = 'completed', updated_at = now() WHERE id = $1",
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
                sqlx::query("UPDATE tasks SET status = 'failed', updated_at = now() WHERE id = $1")
                    .bind(task_id)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query(
                    "UPDATE missions SET status = 'failed', updated_at = now() WHERE id = $1",
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
        status: parse_run_status(row.get::<String, _>("status").as_str())?,
        summary: row.get("summary"),
        artifact_path: row.get("artifact_path"),
        artifact_sha256: row.get("artifact_sha256"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
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
        "ready" => Ok(TaskStatus::Ready),
        "claimed" => Ok(TaskStatus::Claimed),
        "running" => Ok(TaskStatus::Running),
        "blocked" => Ok(TaskStatus::Blocked),
        "awaiting_approval" => Ok(TaskStatus::AwaitingApproval),
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
