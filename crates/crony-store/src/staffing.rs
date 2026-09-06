use super::*;

pub(super) fn validate_staffing(plan: &TaskGraphPlan) -> Result<()> {
    if plan.staffing.len() > 8 || plan.staffing.len() > plan.tasks.len() {
        return Err(anyhow!("mission staffing exceeds the bounded task graph"));
    }
    let mut ids = HashSet::new();
    for agent in &plan.staffing {
        if agent.id.is_nil() || !ids.insert(agent.id) {
            return Err(anyhow!(
                "mission staffing contains an invalid or duplicate identity"
            ));
        }
        for (value, limit) in [
            (&agent.name, 80),
            (&agent.role, 64),
            (&agent.adapter, 64),
            (&agent.accent, 32),
        ] {
            if value.is_empty()
                || value != value.trim()
                || value.len() > limit
                || value.chars().any(char::is_control)
            {
                return Err(anyhow!("mission staffing metadata is invalid"));
            }
        }
        let tasks = plan
            .tasks
            .iter()
            .filter(|task| task.assigned_agent_id == agent.id)
            .collect::<Vec<_>>();
        if tasks.is_empty()
            || tasks
                .iter()
                .any(|task| task.required_adapter != agent.adapter)
        {
            return Err(anyhow!(
                "provisional worker is unused or mismatches its assigned runtime"
            ));
        }
    }
    Ok(())
}

pub(super) async fn persist_staffing_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    mission_id: Uuid,
    room_id: Uuid,
    requested_by: Uuid,
    plan: &TaskGraphPlan,
) -> Result<Vec<DomainEvent>> {
    validate_staffing(plan)?;
    let mut events = Vec::new();
    for agent in &plan.staffing {
        let actor_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO actors (id, corp_id, name, kind, role) VALUES ($1, $2, $3, 'agent', $4)",
        )
        .bind(actor_id)
        .bind(corp_id)
        .bind(&agent.name)
        .bind(&agent.role)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "INSERT INTO room_memberships (room_id, actor_id, role) VALUES ($1, $2, 'member')",
        )
        .bind(room_id)
        .bind(actor_id)
        .execute(&mut **tx)
        .await?;
        // Never upsert: a collision must roll back the entire graph, not adopt or
        // overwrite a different mission's worker.
        sqlx::query(
            r#"
            INSERT INTO agents
                (id, corp_id, actor_id, name, role, adapter, status, accent, mission_id)
            VALUES ($1, $2, $3, $4, $5, $6, 'idle', $7, $8)
            "#,
        )
        .bind(agent.id)
        .bind(corp_id)
        .bind(actor_id)
        .bind(&agent.name)
        .bind(&agent.role)
        .bind(&agent.adapter)
        .bind(&agent.accent)
        .bind(mission_id)
        .execute(&mut **tx)
        .await?;
        if let Some(event) = append_event_tx(
            tx,
            NewEvent {
                room_id: Some(room_id),
                correlation_id: Some(mission_id),
                ..NewEvent::new(
                    corp_id,
                    Some(requested_by),
                    "agent.staffed",
                    "agent",
                    agent.id,
                    format!("agent:{}:staffed", agent.id),
                    json!({
                        "mission_id": mission_id,
                        "actor_id": actor_id,
                        "name": agent.name,
                        "role": agent.role,
                        "adapter": agent.adapter,
                        "pinned": false,
                    }),
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

impl PgStore {
    pub async fn record_dependency_context(
        &self,
        corp_id: Uuid,
        run_id: Uuid,
        context_sha256: &str,
        handoffs: Vec<Value>,
    ) -> Result<Option<DomainEvent>> {
        if handoffs.is_empty() || handoffs.len() > 8 || context_sha256.len() != 64 {
            return Err(anyhow!("invalid bounded dependency context receipt"));
        }
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            r#"
            SELECT t.mission_id, m.room_id FROM runs r
            JOIN tasks t ON t.id = r.task_id AND t.corp_id = r.corp_id
            JOIN missions m ON m.id = t.mission_id AND m.corp_id = t.corp_id
            WHERE r.id = $1 AND r.corp_id = $2 AND r.status = 'starting'
              AND m.status = 'running'
            FOR UPDATE OF r, t, m
            "#,
        )
        .bind(run_id)
        .bind(corp_id)
        .fetch_one(&mut *tx)
        .await
        .context("dependency context run is not awaiting dispatch")?;
        let mission_id: Uuid = row.get("mission_id");
        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(row.get("room_id")),
                correlation_id: Some(mission_id),
                ..NewEvent::new(
                    corp_id,
                    None,
                    "run.dependency_context",
                    "run",
                    run_id,
                    format!("run:{run_id}:dependency-context"),
                    json!({"context_sha256": context_sha256, "handoffs": handoffs}),
                )
            },
        )
        .await?;
        tx.commit().await?;
        Ok(event)
    }

    pub async fn factory_staffing_source(
        &self,
        corp_id: Uuid,
        work_item_id: Uuid,
        actor_id: Uuid,
    ) -> Result<(String, String, String)> {
        let mut tx = self.pool.begin().await?;
        assert_mission_operator_tx(&mut tx, corp_id, actor_id).await?;
        let row = sqlx::query(
            r#"
            SELECT source_repository_owner, source_repository_name, policy, mission_id
            FROM factory_work_items WHERE id = $1 AND corp_id = $2
            "#,
        )
        .bind(work_item_id)
        .bind(corp_id)
        .fetch_one(&mut *tx)
        .await?;
        if let Some(mission_id) = row.get::<Option<Uuid>, _>("mission_id") {
            let room_id: Uuid =
                sqlx::query_scalar("SELECT room_id FROM missions WHERE id = $1 AND corp_id = $2")
                    .bind(mission_id)
                    .bind(corp_id)
                    .fetch_one(&mut *tx)
                    .await?;
            assert_room_membership_tx(&mut tx, corp_id, room_id, actor_id).await?;
        } else {
            mission_room_for_actor_tx(&mut tx, corp_id, actor_id).await?;
        }
        let policy: Value = row.get("policy");
        let base_ref = policy["source_base_ref"]
            .as_str()
            .context("factory source ref missing")?;
        let base_commit = policy["source_base_commit"]
            .as_str()
            .context("factory source commit missing")?;
        let result = (
            format!(
                "{}/{}",
                row.get::<String, _>("source_repository_owner"),
                row.get::<String, _>("source_repository_name")
            ),
            base_ref.to_owned(),
            base_commit.to_owned(),
        );
        tx.commit().await?;
        Ok(result)
    }

    /// System lifecycle reconciliation, not a user-controlled deletion endpoint.
    /// Locks candidates without waiting on live mission/agent transactions.
    pub async fn retire_terminal_mission_agents(&self) -> Result<Vec<DomainEvent>> {
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query(
            r#"
            SELECT a.id, a.corp_id, a.mission_id, m.room_id
            FROM agents a
            JOIN missions m ON m.id = a.mission_id AND m.corp_id = a.corp_id
            WHERE a.retired_at IS NULL AND NOT a.pinned
              AND a.current_run_id IS NULL AND a.status = 'idle'
              AND m.status IN ('completed', 'failed', 'cancelled')
              AND NOT EXISTS (
                SELECT 1 FROM runs r WHERE r.agent_id = a.id AND r.corp_id = a.corp_id
                  AND r.status IN ('provisioning', 'starting', 'running',
                    'waiting_for_input', 'waiting_for_approval', 'verifying')
              )
              AND NOT EXISTS (
                SELECT 1 FROM control_leases l WHERE l.agent_id = a.id
                  AND l.corp_id = a.corp_id AND l.expires_at > now()
              )
              AND NOT EXISTS (
                SELECT 1 FROM queued_messages q WHERE q.agent_id = a.id
                  AND q.corp_id = a.corp_id AND q.status IN ('queued', 'reserved', 'pending')
              )
              AND NOT EXISTS (
                SELECT 1 FROM action_approvals p JOIN runs r ON r.id = p.run_id
                WHERE r.agent_id = a.id AND p.corp_id = a.corp_id AND p.status = 'pending'
              )
              AND NOT EXISTS (
                SELECT 1 FROM verification_requests v JOIN runs r ON r.id = v.run_id
                WHERE r.agent_id = a.id AND v.corp_id = a.corp_id AND v.status = 'pending'
              )
              AND NOT EXISTS (
                SELECT 1 FROM runner_commands c JOIN runs r ON r.id = c.run_id
                WHERE r.agent_id = a.id AND c.corp_id = a.corp_id AND c.status = 'pending'
              )
              AND NOT EXISTS (
                SELECT 1 FROM events e JOIN runs r ON r.id = e.aggregate_id
                WHERE r.agent_id = a.id AND e.corp_id = a.corp_id
                  AND e.type = 'run.teardown_uncertain'
                  AND NOT EXISTS (
                    SELECT 1 FROM events done WHERE done.aggregate_id = r.id
                      AND done.corp_id = a.corp_id AND done.type = 'run.session_terminated'
                      AND done.seq > e.seq
                  )
              )
            ORDER BY a.corp_id, a.id
            LIMIT 100
            FOR UPDATE OF m, a SKIP LOCKED
            "#,
        )
        .fetch_all(&mut *tx)
        .await?;
        let mut events = Vec::new();
        for row in rows {
            let agent_id: Uuid = row.get("id");
            let corp_id: Uuid = row.get("corp_id");
            let mission_id: Uuid = row.get("mission_id");
            let room_id: Uuid = row.get("room_id");
            sqlx::query(
                "UPDATE agents SET retired_at = now(), station = NULL WHERE id = $1 AND corp_id = $2",
            )
            .bind(agent_id)
            .bind(corp_id)
            .execute(&mut *tx)
            .await?;
            if let Some(event) = append_event_tx(
                &mut tx,
                NewEvent {
                    room_id: Some(room_id),
                    correlation_id: Some(mission_id),
                    ..NewEvent::new(
                        corp_id,
                        None,
                        "agent.retired",
                        "agent",
                        agent_id,
                        format!("agent:{agent_id}:retired:{}", Uuid::new_v4()),
                        json!({"mission_id": mission_id, "reason": "terminal mission, no live authority"}),
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
}
