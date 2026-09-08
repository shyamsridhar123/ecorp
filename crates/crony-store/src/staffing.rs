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

// Reuse exactly the same admission before and after taking the mission/agent
// locks. The second statement gets a fresh snapshot without locking run lineage
// rows behind a streaming callback. Retirement itself remains unchanged.
const REACTIVATABLE_MISSION_AGENTS: &str = r#"
    SELECT a.id, a.corp_id, a.mission_id, a.retired_at, m.room_id,
           retirement.id AS retirement_event_id
    FROM agents a
    JOIN missions m ON m.id = a.mission_id AND m.corp_id = a.corp_id
    JOIN LATERAL (
        SELECT e.* FROM events e
        WHERE e.corp_id = a.corp_id AND e.aggregate_id = a.id
        ORDER BY e.seq DESC LIMIT 1
    ) retirement ON TRUE
    WHERE a.retired_at IS NOT NULL AND NOT a.pinned
      AND a.current_run_id IS NULL AND a.status = 'idle'
      AND m.status = 'running'
      AND EXISTS (
        SELECT 1 FROM tasks t
        WHERE t.corp_id = a.corp_id AND t.mission_id = a.mission_id
          AND t.assigned_agent_id = a.id AND t.status IN ('pending', 'ready')
          AND t.attempt_count < t.max_attempts
      )
      AND retirement.type = 'agent.retired' AND retirement.aggregate_type = 'agent'
      AND retirement.actor_id IS NULL AND retirement.room_id = m.room_id
      AND retirement.correlation_id = m.id AND retirement.causation_id IS NULL
      AND retirement.schema_version = 1 AND retirement.aggregate_version = 1
      AND retirement.visibility = 'corp'
      AND retirement.created_at = a.retired_at
      AND retirement.payload = jsonb_build_object(
        'mission_id', m.id, 'reason', 'terminal mission, no live authority')
      AND retirement.idempotency_key ~ (
        '^agent:' || a.id::text ||
        ':retired:[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$')
      AND NOT EXISTS (
        SELECT 1 FROM events duplicate
        WHERE duplicate.corp_id = a.corp_id AND duplicate.aggregate_id = a.id
          AND duplicate.type = 'agent.retired' AND duplicate.created_at = a.retired_at
          AND duplicate.id <> retirement.id
      )
      AND NOT EXISTS (
        SELECT 1 FROM events consumed
        WHERE consumed.corp_id = a.corp_id
          AND consumed.idempotency_key =
            'agent:' || a.id::text || ':reactivated:' || retirement.id::text
      )
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
        SELECT 1 FROM action_approvals p
        JOIN runs r ON r.id = p.run_id AND r.corp_id = p.corp_id
        WHERE r.agent_id = a.id AND p.corp_id = a.corp_id AND p.status = 'pending'
      )
      AND NOT EXISTS (
        SELECT 1 FROM verification_requests v
        JOIN runs r ON r.id = v.run_id AND r.corp_id = v.corp_id
        WHERE r.agent_id = a.id AND v.corp_id = a.corp_id AND v.status = 'pending'
      )
      AND NOT EXISTS (
        SELECT 1 FROM runner_commands c
        JOIN runs r ON r.id = c.run_id AND r.corp_id = c.corp_id
        WHERE r.agent_id = a.id AND c.corp_id = a.corp_id AND c.status = 'pending'
      )
      AND NOT EXISTS (
        SELECT 1 FROM events e
        JOIN runs r ON r.id = e.aggregate_id AND r.corp_id = e.corp_id
        WHERE r.agent_id = a.id AND e.corp_id = a.corp_id
          AND e.type = 'run.teardown_uncertain'
          AND NOT EXISTS (
            SELECT 1 FROM events done WHERE done.aggregate_id = r.id
              AND done.corp_id = a.corp_id AND done.type = 'run.session_terminated'
              AND done.seq > e.seq
          )
      )
"#;

impl PgStore {
    /// Restore only automatic retirement whose owning mission has returned to
    /// running with unfinished assigned work. This grants no run or new scope:
    /// dependencies, retry limits and dispatch admission still own scheduling.
    pub async fn reactivate_running_mission_agents(&self) -> Result<Vec<DomainEvent>> {
        let mut tx = self.pool.begin().await?;
        let candidates = sqlx::query(&format!(
            "{REACTIVATABLE_MISSION_AGENTS}
             ORDER BY a.corp_id, a.id LIMIT 100
             FOR UPDATE OF m, a SKIP LOCKED"
        ))
        .fetch_all(&mut *tx)
        .await?;
        let mut events = Vec::new();
        for candidate in candidates {
            let agent_id: Uuid = candidate.get("id");
            let corp_id: Uuid = candidate.get("corp_id");
            let retired_at: chrono::DateTime<Utc> = candidate.get("retired_at");
            let retirement_event_id: Uuid = candidate.get("retirement_event_id");
            let Some(row) = sqlx::query(&format!(
                "{REACTIVATABLE_MISSION_AGENTS}
                 AND a.id = $1 AND a.corp_id = $2
                 AND a.retired_at = $3 AND retirement.id = $4"
            ))
            .bind(agent_id)
            .bind(corp_id)
            .bind(retired_at)
            .bind(retirement_event_id)
            .fetch_optional(&mut *tx)
            .await?
            else {
                continue;
            };
            let mission_id: Uuid = row.get("mission_id");
            let room_id: Uuid = row.get("room_id");
            sqlx::query(
                "UPDATE agents SET retired_at = NULL
                 WHERE id = $1 AND corp_id = $2 AND retired_at = $3",
            )
            .bind(agent_id)
            .bind(corp_id)
            .bind(retired_at)
            .execute(&mut *tx)
            .await?;
            let event = append_event_tx(
                &mut tx,
                NewEvent {
                    room_id: Some(room_id),
                    correlation_id: Some(mission_id),
                    causation_id: Some(retirement_event_id),
                    ..NewEvent::new(
                        corp_id,
                        None,
                        "agent.reactivated",
                        "agent",
                        agent_id,
                        format!("agent:{agent_id}:reactivated:{retirement_event_id}"),
                        json!({
                            "mission_id": mission_id,
                            "retirement_event_id": retirement_event_id,
                            "retired_at": retired_at,
                            "reason": "running mission, unfinished assigned work",
                        }),
                    )
                },
            )
            .await?
            .context("mission worker retirement generation was already consumed")?;
            events.push(event);
        }
        tx.commit().await?;
        Ok(events)
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crony_domain::{PlannedAgent, PlannedTask};
    use std::future::Future;
    use std::task::Poll;
    use std::time::{Duration as StdDuration, Instant};

    fn runner(corp: Uuid) -> String {
        format!("issue171-store-fixture-{corp}")
    }

    #[derive(Clone, Copy)]
    struct FixtureRun {
        id: Uuid,
        task: Uuid,
        agent: Uuid,
        token: Uuid,
    }

    impl From<&LaunchRecord> for FixtureRun {
        fn from(run: &LaunchRecord) -> Self {
            Self {
                id: run.run_id,
                task: run.task_id,
                agent: run.agent_id,
                token: run.assignment_token,
            }
        }
    }

    struct Fixture {
        store: PgStore,
        corp: Uuid,
        owner: Uuid,
        room: Uuid,
        mission: Uuid,
        agents: [Uuid; 3],
        tasks: [Uuid; 4],
        failed: FixtureRun,
        gameplay: FixtureRun,
        retirement: DomainEvent,
    }

    fn plan(agents: [Uuid; 3]) -> TaskGraphPlan {
        let keys = ["visual", "gameplay", "quality", "integration"];
        TaskGraphPlan {
            strategy: "studio-swarm".to_owned(),
            max_nodes: 4,
            max_depth: 1,
            budget_tokens: 1_000_000,
            budget_cost_microusd: 5_000_000,
            staffing: agents
                .iter()
                .zip(keys)
                .map(|(id, name)| PlannedAgent {
                    id: *id,
                    name: name.to_owned(),
                    role: name.to_owned(),
                    adapter: "github-copilot".to_owned(),
                    accent: "#123456".to_owned(),
                })
                .collect(),
            tasks: keys
                .iter()
                .enumerate()
                .map(|(index, key)| PlannedTask {
                    key: (*key).to_owned(),
                    title: (*key).to_owned(),
                    assigned_agent_id: agents[if index == 3 { 1 } else { index }],
                    required_adapter: "github-copilot".to_owned(),
                    depends_on: if index == 3 {
                        keys[..3].iter().map(|key| (*key).to_owned()).collect()
                    } else {
                        Vec::new()
                    },
                    depth: i32::from(index == 3),
                    max_attempts: 2,
                    contract: TaskContract {
                        objective: format!("Write {key}.md"),
                        expected_output: format!("{key}.md"),
                        source_repository: Some("issue171/isolated-source".to_owned()),
                        source_base_ref: Some("main".to_owned()),
                        source_base_commit: Some("a".repeat(40)),
                        acceptance_tests: vec![format!("{key}.md is present")],
                        allowed_tools: vec!["filesystem".to_owned()],
                        prohibited_actions: vec!["write outside assigned worktree".to_owned()],
                        references: Vec::new(),
                        write_scope: vec![format!("{key}.md")],
                        budget_tokens: 100_000,
                        budget_cost_microusd: 1_000_000,
                        deadline_at: None,
                        escalation: "ask an operator".to_owned(),
                        secret_refs: Vec::new(),
                        model: Some("fixture-model".to_owned()),
                        reasoning_effort: Some("medium".to_owned()),
                        deliverable: None,
                    },
                    verification_policy: VerificationPolicy {
                        checks: vec![VerifierCheck::File {
                            path: format!("{key}.md"),
                            min_bytes: 1,
                        }],
                        manual_gate: (index == 3).then(|| {
                            ManualVerificationGate::IndependentReview {
                                roles: vec!["member".to_owned()],
                                exclude_requester: true,
                            }
                        }),
                    },
                })
                .collect(),
        }
    }

    async fn identity(pool: &PgPool) -> (Uuid, Uuid, Uuid) {
        let corp = Uuid::new_v4();
        let owner = Uuid::new_v4();
        let room = Uuid::new_v4();
        sqlx::query("INSERT INTO corps (id,slug,name) VALUES ($1,$2,'Issue 171 SQLx')")
            .bind(corp)
            .bind(format!("issue171-{corp}"))
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO actors (id,corp_id,name,kind,role)
             VALUES ($1,$2,'Owner','human','owner')",
        )
        .bind(owner)
        .bind(corp)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO rooms (id,corp_id,name,purpose)
             VALUES ($1,$2,'Issue 171','Isolated store regression')",
        )
        .bind(room)
        .bind(corp)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO room_memberships (room_id,actor_id) VALUES ($1,$2)")
            .bind(room)
            .bind(owner)
            .execute(pool)
            .await
            .unwrap();
        (corp, owner, room)
    }

    fn input(corp: Uuid, run: FixtureRun, kind: &str, payload: Value) -> RunnerEventInput {
        RunnerEventInput {
            event_id: Uuid::new_v4(),
            runner_id: runner(corp),
            corp_id: corp,
            connection_epoch: Uuid::from_u128(171),
            run_id: run.id,
            agent_id: run.agent,
            assignment_token: run.token,
            event_type: kind.to_owned(),
            payload,
        }
    }

    async fn event(store: &PgStore, corp: Uuid, run: FixtureRun, kind: &str, payload: Value) {
        store
            .apply_runner_event(input(corp, run, kind, payload))
            .await
            .unwrap_or_else(|error| panic!("{kind}: {error:#}"));
    }

    async fn start(store: &PgStore, corp: Uuid, run: FixtureRun) {
        for (kind, payload) in [
            (
                "run.session",
                json!({"session_id": format!("fixture-{}", run.id)}),
            ),
            (
                "run.started",
                json!({
                    "workspace": format!("fixture-worktree-{}", run.id),
                    "workspace_branch": format!("crony/fixture-{}", run.id),
                    "workspace_base_ref": "main", "workspace_base_commit": "a".repeat(40),
                }),
            ),
            (
                "run.usage",
                json!({"input_tokens": 11, "output_tokens": 13, "cost_microusd": 17}),
            ),
        ] {
            event(store, corp, run, kind, payload).await;
        }
        // Actual prepare/finalize persistence, not signed-byte transport or a
        // provider execution claim. No filesystem, service or runtime DB is used.
        let upload = input(corp, run, "run.artifact_upload", json!({}));
        let artifact = StoredArtifact {
            id: upload.event_id,
            corp_id: corp,
            task_id: run.task,
            run_id: run.id,
            producer_agent_id: run.agent,
            producer_runner_id: runner(corp),
            verifier: "issue171-store-fixture".to_owned(),
            object_key: format!("corps/{corp}/{}", upload.event_id),
            uri: format!("/api/corps/{corp}/artifacts/{}", upload.event_id),
            sha256: hex::encode(Sha256::digest(b"retained fixture handoff")),
            media_type: "text/markdown".to_owned(),
            bytes: 24,
            artifact_role: "provider_evidence".to_owned(),
            file_name: "handoff.md".to_owned(),
            metadata: json!({"fixture": true}),
            provenance_signature: "d".repeat(64),
            retention_until: Utc::now() + Duration::days(1),
        };
        let artifact_id = artifact.id;
        store
            .prepare_artifact_upload(
                upload,
                artifact,
                &format!("staging/corps/{corp}/{artifact_id}"),
            )
            .await
            .unwrap();
        store
            .finalize_artifact_upload(corp, artifact_id)
            .await
            .unwrap();
    }

    async fn preserve(store: &PgStore, corp: Uuid, run: FixtureRun) {
        event(
            store,
            corp,
            run,
            "run.workspace_preserved",
            json!({
                "detail": "retained fixture workspace", "workspace_fingerprint": "b".repeat(64),
            }),
        )
        .await;
    }

    async fn complete(store: &PgStore, corp: Uuid, run: FixtureRun) {
        for (kind, payload) in [
            ("run.verification_started", json!({})),
            (
                "run.verification_evidence",
                json!({
                    "evidence_id": Uuid::new_v4(), "check_index": 0, "kind": "file",
                    "status": "passed", "summary": "fixture file check passed", "payload": {},
                }),
            ),
            (
                "run.verification_passed",
                json!({"summary": "fixture checks passed"}),
            ),
            (
                "run.completed",
                json!({"summary": "fixture root completed"}),
            ),
        ] {
            event(store, corp, run, kind, payload).await;
        }
        preserve(store, corp, run).await;
    }

    async fn fail(store: &PgStore, corp: Uuid, run: FixtureRun) {
        event(
            store,
            corp,
            run,
            "run.failed",
            json!({
                "error": "source deliverable storage acknowledgment timed out",
            }),
        )
        .await;
        preserve(store, corp, run).await;
    }

    // All identities and data below belong to the SQLx-created database. The
    // positive lifecycle uses native staffing, assignment, accounting, artifact,
    // failure, retirement and resume methods under the immutable migrations.
    async fn failed_fixture(pool: PgPool) -> Fixture {
        let (corp, owner, room) = identity(&pool).await;
        let agents = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        let store = PgStore { pool };
        store
            .runner_connected(RunnerConnectInput {
                id: runner(corp),
                corp_id: corp,
                hostname: "issue171-sqlx".to_owned(),
                os: "fixture".to_owned(),
                capabilities: json!([]),
                connection_epoch: Uuid::from_u128(171),
            })
            .await
            .unwrap();
        let (ids, _) = store
            .create_mission(
                corp,
                owner,
                "Issue 171",
                "Retain native lifecycle history",
                &plan(agents),
            )
            .await
            .unwrap();
        let rows = sqlx::query("SELECT id,plan_key FROM tasks WHERE mission_id=$1")
            .bind(ids.mission_id)
            .fetch_all(&store.pool)
            .await
            .unwrap();
        let tasks = ["visual", "gameplay", "quality", "integration"].map(|key| {
            rows.iter()
                .find(|row| row.get::<String, _>("plan_key") == key)
                .unwrap()
                .get("id")
        });
        sqlx::query(
            "INSERT INTO factory_work_items
                (id,corp_id,source_kind,source_project_owner,source_project_number,
                 source_project_item_id,source_repository_owner,source_repository_name,
                 source_issue_number,source_issue_node_id,source_issue_url,source_title,
                 source_revision,state,claim_owner_id,claim_token,lease_expires_at,mission_id,policy)
             VALUES (gen_random_uuid(),$1,'github_project_issue','issue171',171,$2,
                 'issue171','isolated-source',171,'issue171','https://github.com/issue171/isolated-source/issues/171',
                 'Fixture','original-revision','running',$3,gen_random_uuid(),
                 now()+interval '1 hour',$4,'{\"fixture\":true}')",
        )
        .bind(corp)
        .bind(ids.mission_id.to_string())
        .bind(owner)
        .bind(ids.mission_id)
        .execute(&store.pool)
        .await
        .unwrap();
        let mut roots = Vec::new();
        for task in &tasks[..3] {
            let (launch, _) = store
                .create_task_run(corp, ids.mission_id, *task, Some(owner), &runner(corp))
                .await
                .unwrap();
            let run = FixtureRun::from(&launch);
            start(&store, corp, run).await;
            roots.push(run);
        }
        complete(&store, corp, roots[1]).await;
        complete(&store, corp, roots[2]).await;
        fail(&store, corp, roots[0]).await;
        let (retry, _) = store
            .create_task_run(corp, ids.mission_id, tasks[0], None, &runner(corp))
            .await
            .unwrap();
        let failed = FixtureRun::from(&retry);
        start(&store, corp, failed).await;
        fail(&store, corp, failed).await;
        let retired = store.retire_terminal_mission_agents().await.unwrap();
        assert_eq!(
            retired.iter().filter(|event| event.corp_id == corp).count(),
            3
        );
        let retirement = retired
            .into_iter()
            .find(|event| event.aggregate_id == agents[1])
            .unwrap();
        Fixture {
            store,
            corp,
            owner,
            room,
            mission: ids.mission_id,
            agents,
            tasks,
            failed,
            gameplay: roots[1],
            retirement,
        }
    }

    async fn resume(f: &Fixture) -> FixtureRun {
        let (launch, _) = f
            .store
            .create_resume_run(f.corp, f.failed.id, f.owner)
            .await
            .unwrap();
        assert_eq!(launch.source_run_id, f.failed.id);
        assert_eq!(launch.workspace_run_id, f.failed.id);
        assert_eq!(launch.agent_id, f.agents[0]);
        assert_eq!(
            launch.provider_session_id,
            format!("fixture-{}", f.failed.id)
        );
        let run = FixtureRun {
            id: launch.run_id,
            task: launch.task_id,
            agent: launch.agent_id,
            token: launch.assignment_token,
        };
        // Resume retains the source provider session instead of reporting a new one.
        event(
            &f.store,
            f.corp,
            run,
            "run.started",
            json!({
                "workspace": format!("fixture-worktree-{}", launch.workspace_run_id),
                "workspace_branch": format!("crony/fixture-{}", launch.workspace_run_id),
                "workspace_base_ref": "main", "workspace_base_commit": "a".repeat(40),
            }),
        )
        .await;
        run
    }

    async fn running_fixture(pool: PgPool) -> Fixture {
        let f = failed_fixture(pool).await;
        let run = resume(&f).await;
        complete(&f.store, f.corp, run).await;
        f
    }

    async fn snapshot(f: &Fixture) -> Value {
        sqlx::query_scalar(
            "SELECT jsonb_build_object(
                'missions',(SELECT jsonb_agg(to_jsonb(m) ORDER BY id) FROM missions m WHERE corp_id=$1),
                'agents',(SELECT jsonb_agg(to_jsonb(a) ORDER BY id) FROM agents a WHERE corp_id=$1),
                'tasks',(SELECT jsonb_agg(to_jsonb(t) ORDER BY id) FROM tasks t WHERE corp_id=$1),
                'runs',(SELECT jsonb_agg(to_jsonb(r) ORDER BY created_at,id) FROM runs r WHERE corp_id=$1),
                'artifacts',(SELECT jsonb_agg(to_jsonb(a) ORDER BY id) FROM artifacts a WHERE corp_id=$1),
                'evidence',(SELECT jsonb_agg(to_jsonb(v) ORDER BY id) FROM verification_evidence v WHERE corp_id=$1),
                'factory',(SELECT jsonb_agg(to_jsonb(f) ORDER BY id) FROM factory_work_items f WHERE corp_id=$1),
                'leases',(SELECT jsonb_agg(to_jsonb(l) ORDER BY agent_id) FROM control_leases l WHERE corp_id=$1),
                'messages',(SELECT jsonb_agg(to_jsonb(q) ORDER BY id) FROM queued_messages q WHERE corp_id=$1),
                'approvals',(SELECT jsonb_agg(to_jsonb(p) ORDER BY id) FROM action_approvals p WHERE corp_id=$1),
                'requests',(SELECT jsonb_agg(to_jsonb(v) ORDER BY run_id) FROM verification_requests v WHERE corp_id=$1),
                'commands',(SELECT jsonb_agg(to_jsonb(c) ORDER BY id) FROM runner_commands c WHERE corp_id=$1),
                'events',(SELECT jsonb_agg(to_jsonb(e) ORDER BY seq) FROM events e WHERE corp_id=$1))",
        ).bind(f.corp).fetch_one(&f.store.pool).await.unwrap()
    }

    async fn unchanged(f: &Fixture) {
        let before = snapshot(f).await;
        assert!(
            f.store
                .reactivate_running_mission_agents()
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(snapshot(f).await, before);
    }

    async fn integration_only(f: &Fixture) {
        assert_eq!(
            f.store.schedulable_mission_ids(f.corp).await.unwrap(),
            vec![f.mission]
        );
        let tasks = f
            .store
            .schedulable_tasks(f.corp, f.mission, false)
            .await
            .unwrap();
        assert_eq!(
            tasks.iter().map(|task| task.task_id).collect::<Vec<_>>(),
            vec![f.tasks[3]]
        );
        assert_eq!(tasks[0].required_adapter, "github-copilot");
        assert_eq!(tasks[0].required_source_base_commit, Some("a".repeat(40)));
        assert_eq!(tasks[0].required_model.as_deref(), Some("fixture-model"));
    }

    async fn bounded<T>(pool: &PgPool, operation: impl Future<Output = T>) -> T {
        let mut operation = Box::pin(operation);
        let mut deadline = Box::pin(async {
            let end = Instant::now() + StdDuration::from_secs(5);
            loop {
                assert!(
                    Instant::now() < end,
                    "lifecycle operation waited on a locked row"
                );
                sqlx::query("SELECT 1").execute(pool).await.unwrap();
            }
        });
        std::future::poll_fn(|context| {
            if let Poll::Ready(result) = operation.as_mut().poll(context) {
                return Poll::Ready(result);
            }
            let _ = deadline.as_mut().poll(context);
            Poll::Pending
        })
        .await
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated SQLx loader with filter issue171_"]
    async fn issue171_native_retirement_resume_reopen_selects_only_integration(pool: PgPool) {
        let mut f = failed_fixture(pool).await;
        unchanged(&f).await; // Failed missions do not silently regain authority.
        let resumed = resume(&f).await;
        complete(&f.store, f.corp, resumed).await;
        let before = snapshot(&f).await;
        assert_eq!(before["runs"].as_array().unwrap().len(), 5);
        assert_eq!(before["artifacts"].as_array().unwrap().len(), 4);
        assert!(
            f.store
                .schedulable_mission_ids(f.corp)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            f.store
                .schedulable_tasks(f.corp, f.mission, false)
                .await
                .unwrap()
                .is_empty()
        );

        // A new pool uses only this SQLx database's supplied connection options,
        // never ambient DATABASE_URL. No new resume/completion event wakes it.
        f.store = PgStore {
            pool: PgPoolOptions::new()
                .max_connections(5)
                .connect_with(f.store.pool.connect_options().as_ref().clone())
                .await
                .unwrap(),
        };
        let events = f.store.reactivate_running_mission_agents().await.unwrap();
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.aggregate_id, f.agents[1]);
        assert_eq!(event.corp_id, f.corp);
        assert_eq!(event.room_id, Some(f.room));
        assert_eq!(event.correlation_id, Some(f.mission));
        assert_eq!(event.causation_id, Some(f.retirement.id));
        assert_eq!(event.actor_id, None);
        assert_eq!(event.event_type, "agent.reactivated");
        assert_eq!(
            event.idempotency_key,
            format!("agent:{}:reactivated:{}", f.agents[1], f.retirement.id)
        );
        let after = snapshot(&f).await;
        for field in [
            "missions",
            "tasks",
            "runs",
            "artifacts",
            "evidence",
            "factory",
            "leases",
            "messages",
            "approvals",
            "requests",
            "commands",
        ] {
            assert_eq!(
                after[field], before[field],
                "{field} changed during reactivation"
            );
        }
        let mut expected_agents = before["agents"].clone();
        expected_agents
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|agent| agent["id"] == json!(f.agents[1]))
            .unwrap()["retired_at"] = Value::Null;
        assert_eq!(after["agents"], expected_agents);
        let old_events = before["events"].as_array().unwrap();
        assert_eq!(
            &after["events"].as_array().unwrap()[..old_events.len()],
            old_events
        );
        assert_eq!(
            after["events"].as_array().unwrap().len(),
            old_events.len() + 1
        );
        integration_only(&f).await;
        unchanged(&f).await; // No second generation or duplicate wake-up.
        for task in &f.tasks[..3] {
            assert!(
                f.store
                    .create_task_run(f.corp, f.mission, *task, None, &runner(f.corp))
                    .await
                    .is_err()
            );
        }
        let (launch, _) = f
            .store
            .create_task_run(f.corp, f.mission, f.tasks[3], None, &runner(f.corp))
            .await
            .unwrap();
        assert_eq!(launch.agent_id, f.agents[1]);
        assert_eq!(launch.attempt, 1);
        assert!(matches!(
            launch.verification_policy.manual_gate,
            Some(ManualVerificationGate::IndependentReview {
                exclude_requester: true,
                ..
            })
        ));
        assert!(
            f.store
                .schedulable_mission_ids(f.corp)
                .await
                .unwrap()
                .is_empty()
        );
        f.store.pool.close().await;
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated SQLx loader with filter issue171_"]
    async fn issue171_pending_work_reactivation_does_not_bypass_dependencies(pool: PgPool) {
        let f = failed_fixture(pool).await;
        let run = resume(&f).await;
        let events = f.store.reactivate_running_mission_agents().await.unwrap();
        assert_eq!(
            events
                .iter()
                .map(|event| event.aggregate_id)
                .collect::<Vec<_>>(),
            vec![f.agents[1]]
        );
        assert!(
            f.store
                .schedulable_mission_ids(f.corp)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            f.store
                .schedulable_tasks(f.corp, f.mission, false)
                .await
                .unwrap()
                .is_empty()
        );
        let before = snapshot(&f).await;
        assert!(
            f.store
                .create_task_run(f.corp, f.mission, f.tasks[3], None, &runner(f.corp))
                .await
                .is_err()
        );
        assert_eq!(snapshot(&f).await, before);
        complete(&f.store, f.corp, run).await;
        integration_only(&f).await;
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated SQLx loader with filter issue171_"]
    async fn issue171_held_terminal_and_exhausted_task_boundaries(pool: PgPool) {
        for status in ["ready", "completed", "failed", "cancelled"] {
            let f = running_fixture(pool.clone()).await;
            sqlx::query("UPDATE missions SET status=$1 WHERE id=$2")
                .bind(status)
                .bind(f.mission)
                .execute(&pool)
                .await
                .unwrap();
            unchanged(&f).await;
        }
        for status in [
            "completed",
            "failed",
            "cancelled",
            "claimed",
            "running",
            "review",
            "awaiting_approval",
            "verification_failed",
            "blocked",
        ] {
            let f = running_fixture(pool.clone()).await;
            sqlx::query("UPDATE tasks SET status=$1 WHERE id=$2")
                .bind(status)
                .bind(f.tasks[3])
                .execute(&pool)
                .await
                .unwrap();
            unchanged(&f).await;
        }
        let f = running_fixture(pool.clone()).await;
        sqlx::query("UPDATE tasks SET attempt_count=max_attempts WHERE id=$1")
            .bind(f.tasks[3])
            .execute(&pool)
            .await
            .unwrap();
        unchanged(&f).await;
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated SQLx loader with filter issue171_"]
    async fn issue171_worker_ownership_pinning_status_and_pointer_boundaries(pool: PgPool) {
        for change in [
            "UPDATE agents SET pinned=TRUE WHERE id=$1",
            "UPDATE agents SET mission_id=NULL WHERE id=$1",
            "UPDATE agents SET status='reviewing' WHERE id=$1",
            "UPDATE agents SET status='working' WHERE id=$1",
            "UPDATE agents SET current_run_id=(SELECT id FROM runs WHERE agent_id=agents.id LIMIT 1) WHERE id=$1",
        ] {
            let f = running_fixture(pool.clone()).await;
            sqlx::query(change)
                .bind(f.agents[1])
                .execute(&pool)
                .await
                .unwrap();
            unchanged(&f).await;
        }
        let f = running_fixture(pool.clone()).await;
        let (foreign_corp, _, _) = identity(&pool).await;
        sqlx::query("UPDATE agents SET corp_id=$1 WHERE id=$2")
            .bind(foreign_corp)
            .bind(f.agents[1])
            .execute(&pool)
            .await
            .unwrap();
        unchanged(&f).await;
        let f = running_fixture(pool.clone()).await;
        let mut other_plan = plan([Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()]);
        other_plan.staffing.truncate(1);
        other_plan.tasks.truncate(1);
        other_plan.max_nodes = 2;
        let (other, _) = f
            .store
            .create_mission(f.corp, f.owner, "Other held mission", "", &other_plan)
            .await
            .unwrap();
        // Same Corp is not enough. The negative task belongs to another
        // mission, whose single-task plan has no colliding integration key.
        sqlx::query("UPDATE tasks SET mission_id=$1 WHERE id=$2")
            .bind(other.mission_id)
            .bind(f.tasks[3])
            .execute(&pool)
            .await
            .unwrap();
        unchanged(&f).await;
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated SQLx loader with filter issue171_"]
    async fn issue171_exact_native_retirement_provenance_is_required(pool: PgPool) {
        // Negative snapshots are modified only inside their disposable SQLx DB.
        // They must not be repaired by treating an old retirement as current.
        for change in [
            "UPDATE events SET room_id=NULL WHERE id=$1",
            "UPDATE events SET correlation_id=gen_random_uuid() WHERE id=$1",
            "UPDATE events SET causation_id=gen_random_uuid() WHERE id=$1",
            "UPDATE events SET aggregate_id=gen_random_uuid() WHERE id=$1",
            "UPDATE events SET aggregate_type='mission' WHERE id=$1",
            "UPDATE events SET type='agent.manual_retirement' WHERE id=$1",
            "UPDATE events SET schema_version=2 WHERE id=$1",
            "UPDATE events SET aggregate_version=2 WHERE id=$1",
            "UPDATE events SET visibility='private' WHERE id=$1",
            "UPDATE events SET idempotency_key='not-native-retirement' WHERE id=$1",
            "UPDATE events SET payload=jsonb_set(payload,'{reason}','\"manual retirement\"') WHERE id=$1",
            "UPDATE events SET payload=jsonb_set(payload,'{mission_id}',to_jsonb(gen_random_uuid())) WHERE id=$1",
            "UPDATE events SET created_at=created_at-interval '1 microsecond' WHERE id=$1",
        ] {
            let f = running_fixture(pool.clone()).await;
            sqlx::query(change)
                .bind(f.retirement.id)
                .execute(&pool)
                .await
                .unwrap();
            unchanged(&f).await;
        }
        let f = running_fixture(pool.clone()).await;
        sqlx::query("UPDATE events SET actor_id=$1 WHERE id=$2")
            .bind(f.owner)
            .bind(f.retirement.id)
            .execute(&pool)
            .await
            .unwrap();
        unchanged(&f).await;
        let f = running_fixture(pool.clone()).await;
        let (foreign_corp, _, _) = identity(&pool).await;
        sqlx::query("UPDATE events SET corp_id=$1 WHERE id=$2")
            .bind(foreign_corp)
            .bind(f.retirement.id)
            .execute(&pool)
            .await
            .unwrap();
        unchanged(&f).await;
        let f = running_fixture(pool.clone()).await;
        sqlx::query("UPDATE agents SET retired_at=retired_at+interval '1 microsecond' WHERE id=$1")
            .bind(f.agents[1])
            .execute(&pool)
            .await
            .unwrap();
        unchanged(&f).await;
    }

    async fn append_fixture_event(
        f: &Fixture,
        aggregate_id: Uuid,
        aggregate_type: &str,
        kind: &str,
        actor_id: Option<Uuid>,
        payload: Value,
    ) -> DomainEvent {
        let mut tx = f.store.pool.begin().await.unwrap();
        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(f.room),
                correlation_id: Some(f.mission),
                ..NewEvent::new(
                    f.corp,
                    actor_id,
                    kind,
                    aggregate_type,
                    aggregate_id,
                    format!("issue171-fixture:{}", Uuid::new_v4()),
                    payload,
                )
            },
        )
        .await
        .unwrap()
        .unwrap();
        tx.commit().await.unwrap();
        event
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated SQLx loader with filter issue171_"]
    async fn issue171_later_unknown_or_manual_retirement_never_adopts_old_origin(pool: PgPool) {
        for kind in ["agent.policy_changed", "agent.retired"] {
            let f = running_fixture(pool.clone()).await;
            let mut tx = pool.begin().await.unwrap();
            if kind == "agent.retired" {
                sqlx::query("UPDATE agents SET retired_at=now() WHERE id=$1")
                    .bind(f.agents[1])
                    .execute(&mut *tx)
                    .await
                    .unwrap();
            }
            append_event_tx(
                &mut tx,
                NewEvent {
                    room_id: Some(f.room),
                    correlation_id: Some(f.mission),
                    ..NewEvent::new(
                        f.corp,
                        Some(f.owner),
                        kind,
                        "agent",
                        f.agents[1],
                        format!("agent:{}:retired:{}", f.agents[1], Uuid::new_v4()),
                        json!({"mission_id":f.mission,"reason":"manual retirement"}),
                    )
                },
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
            unchanged(&f).await;
        }
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated SQLx loader with filter issue171_"]
    async fn issue171_replayed_or_ambiguous_retirement_generation_fails_closed(pool: PgPool) {
        let f = running_fixture(pool.clone()).await;
        assert_eq!(
            f.store
                .reactivate_running_mission_agents()
                .await
                .unwrap()
                .len(),
            1
        );
        sqlx::query("UPDATE agents SET retired_at=$1 WHERE id=$2")
            .bind(f.retirement.created_at)
            .bind(f.agents[1])
            .execute(&pool)
            .await
            .unwrap();
        unchanged(&f).await;
        // A later replay with a new envelope/key still cannot turn the consumed
        // transaction timestamp into a fresh native retirement generation.
        sqlx::query(
            "INSERT INTO events (id,corp_id,room_id,type,aggregate_type,aggregate_id,
                correlation_id,idempotency_key,payload,created_at)
             SELECT gen_random_uuid(),corp_id,room_id,type,aggregate_type,aggregate_id,
                correlation_id,$1,payload,created_at FROM events WHERE id=$2",
        )
        .bind(format!("agent:{}:retired:{}", f.agents[1], Uuid::new_v4()))
        .bind(f.retirement.id)
        .execute(&pool)
        .await
        .unwrap();
        unchanged(&f).await;
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated SQLx loader with filter issue171_"]
    async fn issue171_consumed_reactivation_key_cannot_clear_retirement(pool: PgPool) {
        let f = running_fixture(pool.clone()).await;
        let mut tx = pool.begin().await.unwrap();
        append_event_tx(
            &mut tx,
            NewEvent::new(
                f.corp,
                None,
                "fixture.key_conflict",
                "mission",
                f.mission,
                format!("agent:{}:reactivated:{}", f.agents[1], f.retirement.id),
                json!({}),
            ),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        unchanged(&f).await;
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated SQLx loader with filter issue171_"]
    async fn issue171_all_other_native_active_run_statuses_block_reactivation(pool: PgPool) {
        for status in [
            "provisioning",
            "starting",
            "running",
            "waiting_for_input",
            "waiting_for_approval",
            "verifying",
        ] {
            let f = running_fixture(pool.clone()).await;
            sqlx::query(
                "INSERT INTO runs (id,corp_id,task_id,agent_id,runner_id,assignment_token,
                    status,workspace_run_id)
                 VALUES ($1,$2,$3,$4,$5,gen_random_uuid(),$6,$1)",
            )
            .bind(Uuid::new_v4())
            .bind(f.corp)
            .bind(f.tasks[1])
            .bind(f.agents[1])
            .bind(runner(f.corp))
            .bind(status)
            .execute(&pool)
            .await
            .unwrap();
            unchanged(&f).await;
        }
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated SQLx loader with filter issue171_"]
    async fn issue171_control_leases_and_all_queued_message_states_are_preserved(pool: PgPool) {
        let f = running_fixture(pool.clone()).await;
        sqlx::query(
            "INSERT INTO control_leases (agent_id,corp_id,actor_id,token,expires_at)
             VALUES ($1,$2,$3,gen_random_uuid(),now()+interval '1 hour')",
        )
        .bind(f.agents[1])
        .bind(f.corp)
        .bind(f.owner)
        .execute(&pool)
        .await
        .unwrap();
        unchanged(&f).await;
        for status in ["queued", "reserved", "pending"] {
            let f = running_fixture(pool.clone()).await;
            sqlx::query(
                "INSERT INTO queued_messages (id,corp_id,agent_id,actor_id,text,status)
                 VALUES (gen_random_uuid(),$1,$2,$3,'Retain authority',$4)",
            )
            .bind(f.corp)
            .bind(f.agents[1])
            .bind(f.owner)
            .bind(status)
            .execute(&pool)
            .await
            .unwrap();
            unchanged(&f).await;
        }
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated SQLx loader with filter issue171_"]
    async fn issue171_pending_approvals_and_commands_are_preserved(pool: PgPool) {
        let f = running_fixture(pool.clone()).await;
        sqlx::query(
            "INSERT INTO action_approvals (id,corp_id,room_id,mission_id,task_id,run_id,
                agent_id,action_key,action,risk,rationale,required_roles,expires_at)
             VALUES (gen_random_uuid(),$1,$2,$3,$4,$5,$6,'fixture','fixture','low',
                'Retain pending authority',ARRAY['owner'],now()+interval '1 hour')",
        )
        .bind(f.corp)
        .bind(f.room)
        .bind(f.mission)
        .bind(f.tasks[1])
        .bind(f.gameplay.id)
        .bind(f.agents[1])
        .execute(&pool)
        .await
        .unwrap();
        unchanged(&f).await;
        let f = running_fixture(pool.clone()).await;
        sqlx::query(
            "INSERT INTO verification_requests (run_id,corp_id,task_id,gate_type,gate)
             VALUES ($1,$2,$3,'independent_review',
                '{\"type\":\"independent_review\",\"roles\":[\"member\"],\"exclude_requester\":true}')",
        ).bind(f.gameplay.id).bind(f.corp).bind(f.tasks[1]).execute(&pool).await.unwrap();
        unchanged(&f).await;
        let f = running_fixture(pool.clone()).await;
        sqlx::query(
            "INSERT INTO runner_commands (id,corp_id,runner_id,run_id,command_kind,payload,idempotency_key)
             VALUES (gen_random_uuid(),$1,$2,$3,'stop','{}','issue171-pending-command')",
        ).bind(f.corp).bind(runner(f.corp)).bind(f.gameplay.id).execute(&pool).await.unwrap();
        unchanged(&f).await;
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated SQLx loader with filter issue171_"]
    async fn issue171_teardown_uncertainty_requires_later_same_run_termination(pool: PgPool) {
        let f = running_fixture(pool.clone()).await;
        append_fixture_event(
            &f,
            f.gameplay.id,
            "run",
            "run.session_terminated",
            None,
            json!({}),
        )
        .await;
        append_fixture_event(
            &f,
            f.gameplay.id,
            "run",
            "run.teardown_uncertain",
            None,
            json!({}),
        )
        .await;
        append_fixture_event(
            &f,
            f.failed.id,
            "run",
            "run.session_terminated",
            None,
            json!({}),
        )
        .await;
        unchanged(&f).await;
        append_fixture_event(
            &f,
            f.gameplay.id,
            "run",
            "run.session_terminated",
            None,
            json!({}),
        )
        .await;
        assert_eq!(
            f.store
                .reactivate_running_mission_agents()
                .await
                .unwrap()
                .len(),
            1
        );
        integration_only(&f).await;
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated SQLx loader with filter issue171_"]
    async fn issue171_event_failure_rolls_back_retirement_clear_and_history(pool: PgPool) {
        let f = running_fixture(pool.clone()).await;
        sqlx::query(
            "ALTER TABLE events ADD CONSTRAINT issue171_injected_failure
             CHECK (type <> 'agent.reactivated')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let before = snapshot(&f).await;
        assert!(f.store.reactivate_running_mission_agents().await.is_err());
        assert_eq!(snapshot(&f).await, before);
        sqlx::query("ALTER TABLE events DROP CONSTRAINT issue171_injected_failure")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            f.store
                .reactivate_running_mission_agents()
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated SQLx loader with filter issue171_"]
    async fn issue171_committed_reactivation_survives_transient_scheduling_failure(pool: PgPool) {
        let f = running_fixture(pool.clone()).await;
        assert_eq!(
            f.store
                .reactivate_running_mission_agents()
                .await
                .unwrap()
                .len(),
            1
        );
        let reactivated = snapshot(&f).await;
        integration_only(&f).await;
        // Fail normal run allocation, not a verifier or a historical result.
        // This extra rejecting constraint exists only in this SQLx database.
        sqlx::query(&format!(
            "ALTER TABLE runs ADD CONSTRAINT issue171_transient_dispatch
             CHECK (task_id <> '{}')",
            f.tasks[3],
        ))
        .execute(&pool)
        .await
        .unwrap();
        assert!(
            f.store
                .create_task_run(f.corp, f.mission, f.tasks[3], None, &runner(f.corp))
                .await
                .is_err()
        );
        assert_eq!(snapshot(&f).await, reactivated);
        sqlx::query("ALTER TABLE runs DROP CONSTRAINT issue171_transient_dispatch")
            .execute(&pool)
            .await
            .unwrap();

        // The next native tick has no reactivation event and no reconnect/resume.
        // Its independent normal-scheduler retry still sees the exact integration.
        assert!(
            f.store
                .reactivate_running_mission_agents()
                .await
                .unwrap()
                .is_empty()
        );
        integration_only(&f).await;
        let (launch, _) = f
            .store
            .create_task_run(f.corp, f.mission, f.tasks[3], None, &runner(f.corp))
            .await
            .unwrap();
        assert_eq!(launch.task_id, f.tasks[3]);
        assert_eq!(launch.agent_id, f.agents[1]);
        assert_eq!(launch.attempt, 1);
        let after = snapshot(&f).await;
        let originals = reactivated["runs"].as_array().unwrap();
        assert_eq!(
            &after["runs"].as_array().unwrap()[..originals.len()],
            originals
        );
        assert_eq!(after["runs"].as_array().unwrap().len(), 6);
        assert_eq!(after["artifacts"], reactivated["artifacts"]);
        assert_eq!(after["factory"], reactivated["factory"]);
        assert_eq!(
            after["events"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|event| event["type"] == "agent.reactivated")
                .count(),
            1
        );
        assert!(
            f.store
                .schedulable_mission_ids(f.corp)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            f.store
                .schedulable_tasks(f.corp, f.mission, false)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated SQLx loader with filter issue171_"]
    async fn issue171_concurrent_reconciliation_consumes_one_retirement_once(pool: PgPool) {
        let f = running_fixture(pool.clone()).await;
        let mut first = Box::pin(f.store.reactivate_running_mission_agents());
        let mut second = Box::pin(f.store.reactivate_running_mission_agents());
        let mut first_result = None;
        let mut second_result = None;
        bounded(
            &pool,
            std::future::poll_fn(|context| {
                if first_result.is_none()
                    && let Poll::Ready(result) = first.as_mut().poll(context)
                {
                    first_result = Some(result.unwrap());
                }
                if second_result.is_none()
                    && let Poll::Ready(result) = second.as_mut().poll(context)
                {
                    second_result = Some(result.unwrap());
                }
                if first_result.is_some() && second_result.is_some() {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }),
        )
        .await;
        assert_eq!(
            first_result.unwrap().len() + second_result.unwrap().len(),
            1
        );
        integration_only(&f).await;
        unchanged(&f).await;
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated SQLx loader with filter issue171_"]
    async fn issue171_locked_mission_or_agent_is_skipped_and_rechecked_next_tick(pool: PgPool) {
        for lock_mission in [false, true] {
            let f = running_fixture(pool.clone()).await;
            let mut blocker = pool.begin().await.unwrap();
            let (query, id) = if lock_mission {
                ("SELECT id FROM missions WHERE id=$1 FOR UPDATE", f.mission)
            } else {
                ("SELECT id FROM agents WHERE id=$1 FOR UPDATE", f.agents[1])
            };
            sqlx::query(query)
                .bind(id)
                .fetch_one(&mut *blocker)
                .await
                .unwrap();
            assert!(
                bounded(&pool, f.store.reactivate_running_mission_agents())
                    .await
                    .unwrap()
                    .is_empty()
            );
            blocker.commit().await.unwrap();
            assert_eq!(
                f.store
                    .reactivate_running_mission_agents()
                    .await
                    .unwrap()
                    .len(),
                1
            );
        }
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated SQLx loader with filter issue171_"]
    async fn issue171_concurrent_cancellation_or_assignment_is_not_overwritten(pool: PgPool) {
        for cancel in [false, true] {
            let f = running_fixture(pool.clone()).await;
            let mut blocker = pool.begin().await.unwrap();
            sqlx::query("SELECT id FROM missions WHERE id=$1 FOR UPDATE")
                .bind(f.mission)
                .fetch_one(&mut *blocker)
                .await
                .unwrap();
            sqlx::query("SELECT id FROM agents WHERE id=$1 FOR UPDATE")
                .bind(f.agents[1])
                .fetch_one(&mut *blocker)
                .await
                .unwrap();
            if cancel {
                sqlx::query("UPDATE missions SET status='cancelled' WHERE id=$1")
                    .bind(f.mission)
                    .execute(&mut *blocker)
                    .await
                    .unwrap();
            } else {
                sqlx::query("UPDATE agents SET current_run_id=$1,status='reviewing' WHERE id=$2")
                    .bind(f.gameplay.id)
                    .bind(f.agents[1])
                    .execute(&mut *blocker)
                    .await
                    .unwrap();
            }
            assert!(
                bounded(&pool, f.store.reactivate_running_mission_agents())
                    .await
                    .unwrap()
                    .is_empty()
            );
            blocker.commit().await.unwrap();
            unchanged(&f).await;
        }
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated SQLx loader with filter issue171_"]
    async fn issue171_reconciliation_is_bounded_to_one_hundred_workers_per_tick(pool: PgPool) {
        let (corp, owner, _) = identity(&pool).await;
        let store = PgStore { pool: pool.clone() };
        for _ in 0..101 {
            let mut single = plan([Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()]);
            single.tasks.truncate(1);
            single.staffing.truncate(1);
            single.max_nodes = 1;
            single.max_depth = 0;
            store
                .create_mission(corp, owner, "Bounded lifecycle fixture", "", &single)
                .await
                .unwrap();
        }
        // Seed terminal/running snapshots solely to test batch size. The separate
        // five-run regression proves the native transitions into these states.
        sqlx::query("UPDATE missions SET status='failed' WHERE corp_id=$1")
            .bind(corp)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            store.retire_terminal_mission_agents().await.unwrap().len(),
            100
        );
        assert_eq!(
            store.retire_terminal_mission_agents().await.unwrap().len(),
            1
        );
        sqlx::query("UPDATE missions SET status='running' WHERE corp_id=$1")
            .bind(corp)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            store
                .reactivate_running_mission_agents()
                .await
                .unwrap()
                .len(),
            100
        );
        assert_eq!(
            store
                .reactivate_running_mission_agents()
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(
            store
                .reactivate_running_mission_agents()
                .await
                .unwrap()
                .is_empty()
        );
        let retired: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM events WHERE corp_id=$1 AND type='agent.retired'",
        )
        .bind(corp)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(retired, 101);
    }
}
