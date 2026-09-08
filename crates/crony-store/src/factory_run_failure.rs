//! Ordinary execution failures remain distinct from verifier rejection. These
//! projections use the existing blocked/running states and native resume path.

use super::*;

pub(super) struct RunScope {
    mission_id: Uuid,
    work_item_id: Option<Uuid>,
}

impl RunScope {
    pub(super) async fn read_tx(
        tx: &mut Transaction<'_, Postgres>,
        corp_id: Uuid,
        run_id: Uuid,
    ) -> Result<Self> {
        let row = sqlx::query(
            "SELECT mission.id AS mission_id, item.id AS factory_id
             FROM runs run
             JOIN tasks task ON task.id = run.task_id AND task.corp_id = run.corp_id
             JOIN missions mission ON mission.id = task.mission_id
                                  AND mission.corp_id = task.corp_id
             LEFT JOIN factory_work_items item ON item.mission_id = mission.id
                                              AND item.corp_id = mission.corp_id
             WHERE run.id = $1 AND run.corp_id = $2",
        )
        .bind(run_id)
        .bind(corp_id)
        .fetch_one(&mut **tx)
        .await
        .context("run not found for Factory reconciliation")?;
        Ok(Self {
            mission_id: row.get("mission_id"),
            work_item_id: row.get("factory_id"),
        })
    }

    pub(super) fn add_lock_keys(&self, corp_id: Uuid, keys: &mut Vec<String>) {
        if let Some(item_id) = self.work_item_id {
            keys.push(format!("factory:item:{corp_id}:{item_id}"));
            keys.push(format!("publication:factory:{corp_id}:{item_id}"));
        }
    }

    pub(super) async fn lock_tx(
        tx: &mut Transaction<'_, Postgres>,
        corp_id: Uuid,
        run_id: Uuid,
    ) -> Result<Self> {
        let scope = Self::read_tx(tx, corp_id, run_id).await?;
        let mut keys = Vec::new();
        scope.add_lock_keys(corp_id, &mut keys);
        // Recovery/publication take their gates before locking Factory and
        // run/task/mission rows. All sibling lifecycle events use this order.
        lock_factory_keys_tx(tx, &keys).await?;
        Ok(scope)
    }

    pub(super) async fn validate_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        corp_id: Uuid,
        mission_id: Uuid,
    ) -> Result<()> {
        let item_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM factory_work_items WHERE corp_id = $1 AND mission_id = $2",
        )
        .bind(corp_id)
        .bind(mission_id)
        .fetch_optional(&mut **tx)
        .await?;
        if self.mission_id != mission_id || self.work_item_id != item_id {
            return Err(anyhow!("run Factory scope changed during authorization"));
        }
        Ok(())
    }
}

pub(super) fn event_reconciles_factory(event_type: &str) -> bool {
    matches!(
        event_type,
        "run.failed"
            | "run.cancelled"
            | "run.completed"
            | "run.verification_failed"
            | "run.verification_waiting"
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn reconcile_tx(
    tx: &mut Transaction<'_, Postgres>,
    scope: &RunScope,
    corp_id: Uuid,
    room_id: Uuid,
    task_id: Uuid,
    run_id: Uuid,
    reason: &str,
) -> Result<Option<DomainEvent>> {
    let Some(item_id) = scope.work_item_id else {
        return Ok(None);
    };
    let failed: bool =
        sqlx::query_scalar("SELECT status = 'failed' FROM missions WHERE id = $1 AND corp_id = $2")
            .bind(scope.mission_id)
            .bind(corp_id)
            .fetch_one(&mut **tx)
            .await?;
    if !failed {
        return Ok(None);
    }
    let (item, _) = factory_work_item_tx(tx, corp_id, item_id, true)
        .await?
        .context("run Factory item disappeared")?;
    if !matches!(
        item.state,
        FactoryWorkItemState::Claimed
            | FactoryWorkItemState::MissionCreated
            | FactoryWorkItemState::Running
            | FactoryWorkItemState::AwaitingApproval
    ) {
        // Preserve verifier/publication outcomes and existing policy blocks.
        // Another exhausted root need not overwrite the first useful failure.
        return Ok(None);
    }
    let detail = normalize_bounded_failure_reason(reason, "Runner reported failure");
    let version: i64 = sqlx::query_scalar(
        "UPDATE factory_work_items
         SET state = 'blocked', version = version + 1, failure_detail = $1, updated_at = now()
         WHERE id = $2 AND corp_id = $3 AND mission_id = $4 RETURNING version",
    )
    .bind(&detail)
    .bind(item_id)
    .bind(corp_id)
    .bind(scope.mission_id)
    .fetch_one(&mut **tx)
    .await?;
    let event = append_event_tx(
        tx,
        NewEvent {
            room_id: Some(room_id),
            aggregate_version: version,
            correlation_id: Some(scope.mission_id),
            causation_id: Some(run_id),
            ..NewEvent::new(
                corp_id,
                None,
                "factory.blocked",
                "factory_work_item",
                item_id,
                format!("factory:{item_id}:run-failed:{run_id}"),
                json!({
                    "previous_state": item.state.as_str(),
                    "state": "blocked",
                    "mission_id": scope.mission_id,
                    "task_id": task_id,
                    "run_id": run_id,
                    "cause": "ordinary_run_failure",
                    "mission_status": "failed",
                    "failure_detail": detail,
                    "failure_authority_sha256": failure_authority_sha256(&item)?,
                }),
            )
        },
    )
    .await?
    .context("Factory run-failure event unexpectedly existed")?;
    Ok(Some(event))
}

struct FailureOrigin {
    run_id: Uuid,
    version: i64,
}

pub(super) async fn owns_block_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    claim_token: Uuid,
    source_run_id: Uuid,
) -> Result<bool> {
    Ok(failure_origin_tx(tx, item, claim_token)
        .await?
        .is_some_and(|origin| origin.run_id == source_run_id))
}

fn failure_authority_sha256(item: &FactoryWorkItem) -> Result<String> {
    // Lease-only metadata is intentionally excluded; source and policy are not.
    let authority = json!({
        "corp_id": item.corp_id,
        "work_item_id": item.id,
        "mission_id": item.mission_id,
        "source_kind": item.source_kind,
        "source_project_owner": item.source_project_owner,
        "source_project_number": item.source_project_number,
        "source_project_item_id": item.source_project_item_id,
        "source_repository_owner": item.source_repository_owner,
        "source_repository_name": item.source_repository_name,
        "source_issue_number": item.source_issue_number,
        "source_issue_node_id": item.source_issue_node_id,
        "source_issue_url": item.source_issue_url,
        "source_title": item.source_title,
        "source_revision": item.source_revision,
        "policy": item.policy,
    });
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(&authority)?)))
}

async fn failure_origin_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    claim_token: Uuid,
) -> Result<Option<FailureOrigin>> {
    // Only lease renewal can preserve a failure origin across versions. Start
    // at the latest non-renewal event, not any matching historical diagnostic.
    let event = sqlx::query(
        "SELECT id, type, payload, aggregate_version FROM events
         WHERE corp_id = $1 AND aggregate_type = 'factory_work_item' AND aggregate_id = $2
           AND aggregate_version <= $3 AND type <> 'factory.claim_renewed'
         ORDER BY aggregate_version DESC, seq DESC LIMIT 1",
    )
    .bind(item.corp_id)
    .bind(item.id)
    .bind(item.version)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(event) = event else {
        return Ok(None);
    };
    let payload: Value = event.get("payload");
    if event.get::<String, _>("type") != "factory.blocked"
        || payload.get("cause").and_then(Value::as_str) != Some("ordinary_run_failure")
        || payload.get("mission_id") != Some(&json!(item.mission_id))
        || payload.get("failure_detail") != Some(&json!(item.failure_detail))
        || payload.get("failure_authority_sha256") != Some(&json!(failure_authority_sha256(item)?))
    {
        return Ok(None);
    }
    let Some(run_id) = payload
        .get("run_id")
        .and_then(Value::as_str)
        .and_then(|id| id.parse().ok())
    else {
        return Ok(None);
    };
    let version: i64 = event.get("aggregate_version");
    // Prove every intervening version, without transferring an unbounded event
    // history. A gap, duplicate version, unknown event, reclaim, changed owner/
    // token, or missing native renewal operation invalidates the origin.
    let chain = sqlx::query(
        r#"
        SELECT COUNT(*) AS event_count,
               COUNT(DISTINCT event.aggregate_version) AS version_count,
               COALESCE(BOOL_AND(COALESCE(
                   event.aggregate_version > $4
                   AND event.type = 'factory.claim_renewed'
                   AND event.actor_id = $6
                   AND event.payload->'claim_owner_id' = to_jsonb($6::uuid)
                   AND event.payload->>'state' = 'blocked'
                   AND EXISTS (
                       SELECT 1 FROM factory_operations operation
                       WHERE operation.corp_id = event.corp_id
                         AND operation.work_item_id = event.aggregate_id
                         AND operation.operation = 'renew'
                         AND operation.resulting_version = event.aggregate_version
                         AND operation.actor_id = event.actor_id
                         AND operation.claim_token = $7
                         AND operation.request->'work_item_id' = to_jsonb(event.aggregate_id)
                         AND operation.request->'expected_version'
                             = to_jsonb(event.aggregate_version - 1)
                   ), FALSE)), TRUE) AS only_renewals
        FROM events event
        WHERE event.corp_id = $1 AND event.aggregate_type = 'factory_work_item'
          AND event.aggregate_id = $2 AND event.aggregate_version BETWEEN $4 AND $3
          AND event.id <> $5
        "#,
    )
    .bind(item.corp_id)
    .bind(item.id)
    .bind(item.version)
    .bind(version)
    .bind(event.get::<Uuid, _>("id"))
    .bind(item.claim_owner_id)
    .bind(claim_token)
    .fetch_one(&mut **tx)
    .await?;
    let revisions = item.version - version;
    if chain.get::<i64, _>("event_count") != revisions
        || chain.get::<i64, _>("version_count") != revisions
        || !chain.get::<bool, _>("only_renewals")
    {
        return Ok(None);
    }
    Ok(Some(FailureOrigin { run_id, version }))
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn resume_tx(
    tx: &mut Transaction<'_, Postgres>,
    scope: &RunScope,
    corp_id: Uuid,
    room_id: Uuid,
    source_run_id: Uuid,
    source_status: &str,
    run_id: Uuid,
    actor_id: Uuid,
) -> Result<()> {
    let Some(item_id) = scope.work_item_id else {
        return Ok(());
    };
    let (item, claim_token) = factory_work_item_tx(tx, corp_id, item_id, true)
        .await?
        .context("resume Factory item disappeared")?;
    if matches!(
        item.state,
        FactoryWorkItemState::Verified
            | FactoryWorkItemState::Publishing
            | FactoryWorkItemState::Published
            | FactoryWorkItemState::Failed
            | FactoryWorkItemState::Cancelled
    ) {
        return Err(anyhow!(
            "native resume cannot reopen Factory outcome {}",
            item.state.as_str()
        ));
    }
    if item.state != FactoryWorkItemState::Blocked || source_status != "failed" {
        return Ok(());
    }
    let Some(origin) = failure_origin_tx(tx, &item, claim_token).await? else {
        return Ok(());
    };
    if origin.run_id != source_run_id {
        return Ok(());
    }
    let version: i64 = sqlx::query_scalar(
        "UPDATE factory_work_items
         SET state = 'running', version = version + 1, failure_detail = NULL, updated_at = now()
         WHERE id = $1 AND corp_id = $2 AND mission_id = $3 RETURNING version",
    )
    .bind(item_id)
    .bind(corp_id)
    .bind(scope.mission_id)
    .fetch_one(&mut **tx)
    .await?;
    append_event_tx(
        tx,
        NewEvent {
            room_id: Some(room_id),
            aggregate_version: version,
            correlation_id: Some(scope.mission_id),
            causation_id: Some(run_id),
            ..NewEvent::new(
                corp_id,
                Some(actor_id),
                "factory.running",
                "factory_work_item",
                item_id,
                format!("factory:{item_id}:resume:{run_id}"),
                json!({
                    "previous_state": "blocked",
                    "state": "running",
                    "mission_id": scope.mission_id,
                    "run_id": run_id,
                    "source_run_id": source_run_id,
                    "failure_origin_version": origin.version,
                    "resumed_factory_version": item.version,
                    "cause": "native_resume",
                }),
            )
        },
    )
    .await?
    .context("Factory native-resume event unexpectedly existed")?;
    Ok(())
}

pub(super) async fn release_failed_run_agent_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    agent_id: Uuid,
    run_id: Uuid,
) -> Result<()> {
    // Keep this lock at the old failure-cleanup UPDATE position. Native run
    // admission also locks the agent before allocating an assignment. Check
    // detached ownership in a fresh statement after any wait on that lock.
    let agent = sqlx::query(
        "SELECT status, current_run_id FROM agents WHERE id=$1 AND corp_id=$2 FOR UPDATE",
    )
    .bind(agent_id)
    .bind(corp_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(agent) = agent else {
        return Ok(());
    };
    let current_run_id: Option<Uuid> = agent.get("current_run_id");
    if current_run_id != Some(run_id) {
        if current_run_id.is_some() || agent.get::<String, _>("status") != "reviewing" {
            return Ok(());
        }
        // Artifact finalization legitimately detaches a reviewing agent. Only
        // its uniquely latest, now-failed assignment can release that state;
        // equal timestamps are ambiguous, not authority from a UUID tie-break.
        let owns_detached_review: bool = sqlx::query_scalar(
            r#"
            SELECT EXISTS (
                SELECT 1 FROM runs failed
                WHERE failed.id=$1 AND failed.corp_id=$2 AND failed.agent_id=$3
                  AND failed.status='failed'
                  AND NOT EXISTS (
                      SELECT 1 FROM runs other
                      WHERE other.corp_id=failed.corp_id AND other.agent_id=failed.agent_id
                        AND other.id<>failed.id
                        AND (
                            other.created_at>=failed.created_at
                            OR other.status IN (
                                'provisioning', 'starting', 'running',
                                'waiting_for_input', 'waiting_for_approval', 'verifying'
                            )
                        )
                  )
            )
            "#,
        )
        .bind(run_id)
        .bind(corp_id)
        .bind(agent_id)
        .fetch_one(&mut **tx)
        .await?;
        if !owns_detached_review {
            return Ok(());
        }
    }
    sqlx::query(
        "UPDATE agents SET status='idle', station=NULL, current_run_id=NULL
         WHERE id=$1 AND corp_id=$2 AND current_run_id IS NOT DISTINCT FROM $3",
    )
    .bind(agent_id)
    .bind(corp_id)
    .bind(current_run_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::task::Poll;
    use std::time::{Duration as StdDuration, Instant};

    const CORP: Uuid = Uuid::from_u128(1);
    const MISSION: Uuid = Uuid::from_u128(2);
    const TASK: Uuid = Uuid::from_u128(3);
    const RUN: Uuid = Uuid::from_u128(4);
    const AGENT: Uuid = Uuid::from_u128(5);
    const ROOM: Uuid = Uuid::from_u128(6);
    const FACTORY: Uuid = Uuid::from_u128(7);
    const TOKEN: Uuid = Uuid::from_u128(8);
    const OWNER: Uuid = Uuid::from_u128(9);
    const OUTSIDER: Uuid = Uuid::from_u128(12);
    const ACK_FAILURE: &str = "source deliverable storage acknowledgment timed out";

    // SQLx creates a disposable database and applies the actual migrations.
    // No application, runner, provider, filesystem worktree or object store is
    // involved: evidence rows here test persistence, not signed-byte acceptance.
    async fn fixture(pool: PgPool) -> PgStore {
        sqlx::raw_sql(
            r#"
            INSERT INTO corps (id,slug,name) VALUES
                ('00000000-0000-0000-0000-000000000001','issue169','Issue 169 fixture');
            INSERT INTO actors (id,corp_id,name,kind,role) VALUES
                ('00000000-0000-0000-0000-000000000009',
                 '00000000-0000-0000-0000-000000000001','Owner','human','owner'),
                ('00000000-0000-0000-0000-00000000000a',
                 '00000000-0000-0000-0000-000000000001','Worker','agent','worker'),
                ('00000000-0000-0000-0000-00000000000b',
                 '00000000-0000-0000-0000-000000000001','Reviewer','human','member'),
                ('00000000-0000-0000-0000-00000000000c',
                 '00000000-0000-0000-0000-000000000001','Outsider','human','manager');
            INSERT INTO rooms (id,corp_id,name,purpose) VALUES
                ('00000000-0000-0000-0000-000000000006',
                 '00000000-0000-0000-0000-000000000001','Fixture','Store regression');
            INSERT INTO room_memberships (room_id,actor_id) VALUES
                ('00000000-0000-0000-0000-000000000006','00000000-0000-0000-0000-000000000009'),
                ('00000000-0000-0000-0000-000000000006','00000000-0000-0000-0000-00000000000b');
            INSERT INTO agents (id,corp_id,actor_id,name,role,adapter,status,current_run_id,accent)
            VALUES ('00000000-0000-0000-0000-000000000005',
                    '00000000-0000-0000-0000-000000000001',
                    '00000000-0000-0000-0000-00000000000a',
                    'Worker','worker','openai-codex','reviewing',
                    '00000000-0000-0000-0000-000000000004','#123456');
            INSERT INTO queued_messages (id,corp_id,agent_id,actor_id,text,status)
            VALUES ('00000000-0000-0000-0000-000000000011',
                    '00000000-0000-0000-0000-000000000001',
                    '00000000-0000-0000-0000-000000000005',
                    '00000000-0000-0000-0000-000000000009','Retain this operator note','queued');
            INSERT INTO missions (id,corp_id,room_id,requested_by,title,status,max_nodes,budget_tokens,
                                  original_budget_tokens,original_budget_cost_microusd)
            VALUES ('00000000-0000-0000-0000-000000000002',
                    '00000000-0000-0000-0000-000000000001',
                    '00000000-0000-0000-0000-000000000006',
                    '00000000-0000-0000-0000-000000000009','Acknowledgment failure','running',4,1000000,
                    1000000,5000000);
            INSERT INTO factory_work_items
                (id,corp_id,source_kind,source_project_owner,source_project_number,
                 source_project_item_id,source_repository_owner,source_repository_name,
                 source_issue_number,source_issue_node_id,source_issue_url,source_title,
                 source_revision,state,claim_owner_id,claim_token,lease_expires_at,mission_id,policy)
            VALUES ('00000000-0000-0000-0000-000000000007',
                    '00000000-0000-0000-0000-000000000001','github_project_issue',
                    'fixture',169,'item-169','fixture','source',169,'issue-169',
                    'https://github.com/fixture/source/issues/169','Store fixture',
                    'original-revision','running','00000000-0000-0000-0000-000000000009',
                    '00000000-0000-0000-0000-000000000010',now()+interval '1 hour',
                    '00000000-0000-0000-0000-000000000002','{"fixture":true}');
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();
        let contract = json!({
            "objective": "preserve result.md",
            "expected_output": "result.md",
            "source_repository": "fixture/source",
            "source_base_ref": "main",
            "source_base_commit": "a".repeat(40),
            "acceptance_tests": ["result.md is present"],
            "allowed_tools": ["filesystem"],
            "prohibited_actions": ["write outside result.md"],
            "references": [], "write_scope": ["result.md"],
            "budget_tokens": 100000, "budget_cost_microusd": 1000000,
            "deadline_at": null, "escalation": "ask an operator",
            "model": "fixture-model", "reasoning_effort": "medium"
        });
        sqlx::query(
            "INSERT INTO tasks
                (id,corp_id,mission_id,title,objective,status,assigned_agent_id,
                 plan_key,contract,verification_policy,attempt_count,max_attempts)
             VALUES ($1,$2,$3,'Root','preserve result.md','review',$4,'root',$5,$6,2,2)",
        )
        .bind(TASK)
        .bind(CORP)
        .bind(MISSION)
        .bind(AGENT)
        .bind(contract)
        .bind(
            json!({"checks":[{"type":"file","path":"result.md","min_bytes":1}],"manual_gate":null}),
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO runs
                (id,corp_id,task_id,agent_id,runner_id,assignment_token,status,
                 workspace_run_id,provider_session_id,workspace_disposition,
                 workspace_path,workspace_branch,workspace_base_ref,workspace_base_commit,
                 workspace_fingerprint,source_repository,source_base_ref,source_base_commit,
                 model,reasoning_effort,input_tokens,output_tokens,cost_microusd,verification_status)
             VALUES ($1,$2,$3,$4,'issue169-runner',$5,'verifying',$1,'fixture-session','preserved',
                     'fixture-worktree','crony/fixture','main',$6,$7,'fixture/source','main',$6,
                     'fixture-model','medium',11,13,17,'running')",
        )
        .bind(RUN)
        .bind(CORP)
        .bind(TASK)
        .bind(AGENT)
        .bind(TOKEN)
        .bind("a".repeat(40))
        .bind("b".repeat(64))
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO verification_evidence
                (id,corp_id,task_id,run_id,check_index,kind,status,summary)
             VALUES ($1,$2,$3,$4,0,'file','passed','fixture file check passed')",
        )
        .bind(Uuid::new_v4())
        .bind(CORP)
        .bind(TASK)
        .bind(RUN)
        .execute(&pool)
        .await
        .unwrap();
        PgStore { pool }
    }

    fn input_for(run_id: Uuid, token: Uuid, event_type: &str, payload: Value) -> RunnerEventInput {
        RunnerEventInput {
            event_id: Uuid::new_v4(),
            runner_id: "issue169-runner".to_owned(),
            corp_id: CORP,
            connection_epoch: Uuid::from_u128(20),
            run_id,
            agent_id: AGENT,
            assignment_token: token,
            event_type: event_type.to_owned(),
            payload,
        }
    }

    fn failure() -> RunnerEventInput {
        input_for(RUN, TOKEN, "run.failed", json!({"error": ACK_FAILURE}))
    }

    async fn state(store: &PgStore) -> Value {
        sqlx::query_scalar(
            "SELECT jsonb_build_object(
              'runs',(SELECT jsonb_agg(to_jsonb(r) ORDER BY created_at,id) FROM runs r),
              'tasks',(SELECT jsonb_agg(to_jsonb(t) ORDER BY id) FROM tasks t),
              'missions',(SELECT jsonb_agg(to_jsonb(m) ORDER BY id) FROM missions m),
              'agents',(SELECT jsonb_agg(to_jsonb(a) ORDER BY id) FROM agents a),
              'factory',(SELECT jsonb_agg(to_jsonb(f) ORDER BY id) FROM factory_work_items f),
              'events',(SELECT jsonb_agg(to_jsonb(e) ORDER BY seq) FROM events e),
              'evidence',(SELECT jsonb_agg(to_jsonb(v) ORDER BY id) FROM verification_evidence v),
              'requests',(SELECT jsonb_agg(to_jsonb(v) ORDER BY run_id) FROM verification_requests v),
              'queued',(SELECT jsonb_agg(to_jsonb(q) ORDER BY id) FROM queued_messages q),
              'commands',(SELECT jsonb_agg(to_jsonb(c) ORDER BY id) FROM runner_commands c),
              'recoveries',(SELECT jsonb_agg(to_jsonb(r) ORDER BY id) FROM factory_verification_recoveries r),
              'deliverables',(SELECT jsonb_agg(to_jsonb(d) ORDER BY id) FROM source_deliverables d))",
        )
        .fetch_one(&store.pool)
        .await
        .unwrap()
    }

    async fn restore_active(store: &PgStore, factory_state: &str) {
        // Only the SQLx-created database, with this test's fixed fixture IDs.
        sqlx::raw_sql(
            "DELETE FROM events;
             DELETE FROM verification_requests;
             UPDATE runs SET status='verifying',summary=NULL,verification_status='running';
             UPDATE tasks SET status='review',verification_status='pending',attempt_count=2;
             UPDATE missions SET status='running';
             UPDATE agents SET status='reviewing',station='review',
                 current_run_id='00000000-0000-0000-0000-000000000004';",
        )
        .execute(&store.pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE factory_work_items SET state=$1,version=1,failure_detail=NULL WHERE id=$2",
        )
        .bind(factory_state)
        .bind(FACTORY)
        .execute(&store.pool)
        .await
        .unwrap();
    }

    async fn verify(store: &PgStore, run_id: Uuid, token: Uuid) {
        for (kind, payload) in [
            (
                "run.started",
                json!({
                    "workspace":"fixture-worktree", "workspace_branch":"crony/fixture",
                    "workspace_base_ref":"main", "workspace_base_commit":"a".repeat(40)
                }),
            ),
            ("run.verification_started", json!({})),
            (
                "run.verification_evidence",
                json!({
                    "evidence_id":Uuid::new_v4(),"check_index":0,"kind":"file",
                    "status":"passed","summary":"fixture file check passed","payload":{}
                }),
            ),
            (
                "run.verification_passed",
                json!({"summary":"fixture checks passed"}),
            ),
        ] {
            store
                .apply_runner_event(input_for(run_id, token, kind, payload))
                .await
                .unwrap();
        }
    }

    async fn complete(store: &PgStore, run_id: Uuid, token: Uuid) {
        verify(store, run_id, token).await;
        store
            .apply_runner_event(input_for(
                run_id,
                token,
                "run.completed",
                json!({"summary":"fixture verified completion"}),
            ))
            .await
            .unwrap();
    }

    async fn renew(store: &PgStore, expected_version: i64) -> FactoryWorkItemOutcome {
        store
            .renew_factory_work_item(RenewFactoryWorkItemInput {
                corp_id: CORP,
                work_item_id: FACTORY,
                actor_id: OWNER,
                claim_token: Uuid::from_u128(16),
                expected_version,
                idempotency_key: format!("issue169-native-renew:{expected_version}"),
                lease_seconds: 60,
            })
            .await
            .unwrap()
    }

    #[test]
    fn issue169_only_factory_lifecycle_events_take_factory_gates() {
        for kind in [
            "run.failed",
            "run.cancelled",
            "run.completed",
            "run.verification_failed",
            "run.verification_waiting",
        ] {
            assert!(event_reconciles_factory(kind));
        }
        for kind in [
            "run.output",
            "run.usage",
            "run.status",
            "run.verification_evidence",
            "run.session",
            "run.workspace_preserved",
        ] {
            assert!(!event_reconciles_factory(kind));
        }
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_exhausted_failure_is_atomic_scoped_and_preserves_evidence(pool: PgPool) {
        let store = fixture(pool).await;
        let before = state(&store).await;
        let reason = format!(" \r\n{ACK_FAILURE}\t\u{7} {}", "δ".repeat(2_000));
        let result = store
            .apply_runner_event(input_for(RUN, TOKEN, "run.failed", json!({"error":reason})))
            .await
            .unwrap();
        let after = state(&store).await;
        assert_eq!(after["runs"][0]["status"], "failed");
        assert_eq!(after["runs"][0]["verification_status"], "running");
        assert_eq!(after["tasks"][0]["status"], "failed");
        assert_eq!(after["missions"][0]["status"], "failed");
        assert_eq!(after["factory"][0]["state"], "blocked");
        assert_eq!(after["factory"][0]["version"], 2);
        let detail = after["factory"][0]["failure_detail"].as_str().unwrap();
        assert!(detail.starts_with(ACK_FAILURE));
        assert!(detail.len() <= 2_000);
        assert!(!detail.chars().any(char::is_control));
        for field in [
            "workspace_run_id",
            "workspace_disposition",
            "workspace_fingerprint",
            "workspace_path",
            "workspace_branch",
            "workspace_base_commit",
            "source_repository",
            "source_base_ref",
            "source_base_commit",
            "provider_session_id",
            "input_tokens",
            "output_tokens",
            "cost_microusd",
            "budget_tokens_limit",
            "budget_cost_microusd_limit",
        ] {
            assert_eq!(after["runs"][0][field], before["runs"][0][field], "{field}");
        }
        for field in [
            "attempt_count",
            "max_attempts",
            "contract",
            "verification_policy",
        ] {
            assert_eq!(
                after["tasks"][0][field], before["tasks"][0][field],
                "{field}"
            );
        }
        for key in [
            "evidence",
            "recoveries",
            "deliverables",
            "requests",
            "commands",
            "queued",
        ] {
            assert_eq!(after[key], before[key], "{key}");
        }
        for field in [
            "policy",
            "source_revision",
            "mission_id",
            "claim_token",
            "lease_expires_at",
        ] {
            assert_eq!(
                after["factory"][0][field], before["factory"][0][field],
                "{field}"
            );
        }
        assert!(after["agents"][0]["current_run_id"].is_null());
        assert_eq!(after["agents"][0]["status"], "idle");
        let factory_event = &result.related_events[0];
        assert_eq!(result.related_events.len(), 1);
        assert!(result.event.as_ref().unwrap().seq < factory_event.seq);
        assert_eq!(factory_event.corp_id, CORP);
        assert_eq!(factory_event.room_id, Some(ROOM));
        assert_eq!(factory_event.aggregate_id, FACTORY);
        assert_eq!(factory_event.aggregate_version, 2);
        assert_eq!(factory_event.correlation_id, Some(MISSION));
        assert_eq!(factory_event.causation_id, Some(RUN));
        assert_eq!(factory_event.payload["cause"], "ordinary_run_failure");
        assert_eq!(factory_event.payload["task_id"], json!(TASK));
        assert_eq!(factory_event.payload["run_id"], json!(RUN));
        assert_eq!(factory_event.payload["failure_detail"], detail);
        assert!(
            !factory_event
                .payload
                .to_string()
                .contains(&TOKEN.to_string())
        );
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_retryable_failure_retains_running_factory_and_retry_budget(pool: PgPool) {
        let store = fixture(pool).await;
        sqlx::query("UPDATE tasks SET attempt_count=1 WHERE id=$1")
            .bind(TASK)
            .execute(&store.pool)
            .await
            .unwrap();
        let before = state(&store).await;
        let result = store.apply_runner_event(failure()).await.unwrap();
        let after = state(&store).await;
        assert!(result.related_events.is_empty());
        assert_eq!(after["runs"][0]["status"], "failed");
        assert_eq!(after["tasks"][0]["status"], "ready");
        assert_eq!(after["missions"][0]["status"], "running");
        assert_eq!(after["factory"], before["factory"]);
        assert_eq!(after["tasks"][0]["attempt_count"], 1);
        assert_eq!(after["tasks"][0]["max_attempts"], 2);
        assert_eq!(after["runs"][0]["input_tokens"], 11);
        assert_eq!(after["events"].as_array().unwrap().len(), 1);
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_hard_breaker_failure_is_not_retried_or_resumable(pool: PgPool) {
        let store = fixture(pool).await;
        sqlx::raw_sql(
            "UPDATE tasks SET attempt_count=1;
             UPDATE runs SET breaker_stage='stop';",
        )
        .execute(&store.pool)
        .await
        .unwrap();
        store.apply_runner_event(failure()).await.unwrap();
        let after = state(&store).await;
        assert_eq!(after["tasks"][0]["status"], "failed");
        assert_eq!(after["tasks"][0]["attempt_count"], 1);
        assert_eq!(after["missions"][0]["status"], "failed");
        assert_eq!(after["factory"][0]["state"], "blocked");
        assert_eq!(after["runs"][0]["breaker_stage"], "stop");
        assert!(store.create_resume_run(CORP, RUN, OWNER).await.is_err());
        assert_eq!(state(&store).await, after);
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_empty_diagnostics_still_persist_a_useful_block(pool: PgPool) {
        let store = fixture(pool).await;
        for payload in [json!({"error":" \r\n\t "}), json!({})] {
            restore_active(&store, "running").await;
            store
                .apply_runner_event(input_for(RUN, TOKEN, "run.failed", payload))
                .await
                .unwrap();
            assert_eq!(
                state(&store).await["factory"][0]["failure_detail"],
                "Runner reported failure"
            );
        }
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_nonfactory_and_foreign_factory_are_untouched(pool: PgPool) {
        let store = fixture(pool).await;
        sqlx::query(
            "INSERT INTO missions (id,corp_id,room_id,requested_by,title,status,
                                   original_budget_tokens,original_budget_cost_microusd)
             VALUES ($1,$2,$3,$4,'Unrelated mission','running',100000,5000000)",
        )
        .bind(Uuid::from_u128(30))
        .bind(CORP)
        .bind(ROOM)
        .bind(OWNER)
        .execute(&store.pool)
        .await
        .unwrap();
        sqlx::query("UPDATE factory_work_items SET mission_id=$1 WHERE id=$2")
            .bind(Uuid::from_u128(30))
            .bind(FACTORY)
            .execute(&store.pool)
            .await
            .unwrap();
        let before = state(&store).await;
        let outcome = store.apply_runner_event(failure()).await.unwrap();
        let after = state(&store).await;
        assert!(outcome.related_events.is_empty());
        assert_eq!(after["factory"], before["factory"]);
        assert_eq!(after["missions"][1], before["missions"][1]);
        assert_eq!(after["tasks"][0]["status"], "failed");

        restore_active(&store, "running").await;
        sqlx::query("INSERT INTO corps (id,slug,name) VALUES ($1,'foreign','Foreign fixture')")
            .bind(Uuid::from_u128(99))
            .execute(&store.pool)
            .await
            .unwrap();
        // Corrupt cross-Corp linkage is never accepted as ownership.
        sqlx::query("UPDATE factory_work_items SET mission_id=$1,corp_id=$2 WHERE id=$3")
            .bind(MISSION)
            .bind(Uuid::from_u128(99))
            .bind(FACTORY)
            .execute(&store.pool)
            .await
            .unwrap();
        let before = state(&store).await;
        assert!(
            store
                .apply_runner_event(failure())
                .await
                .unwrap()
                .related_events
                .is_empty()
        );
        assert_eq!(state(&store).await["factory"], before["factory"]);
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_duplicate_late_and_foreign_callbacks_have_no_effect(pool: PgPool) {
        let store = fixture(pool).await;
        for field in ["corp", "run", "agent", "runner", "assignment"] {
            let mut callback = failure();
            match field {
                "corp" => callback.corp_id = Uuid::from_u128(99),
                "run" => callback.run_id = Uuid::from_u128(99),
                "agent" => callback.agent_id = Uuid::from_u128(99),
                "runner" => callback.runner_id = "foreign-runner".to_owned(),
                "assignment" => callback.assignment_token = Uuid::from_u128(99),
                _ => unreachable!(),
            }
            let before = state(&store).await;
            assert!(store.apply_runner_event(callback).await.is_err(), "{field}");
            assert_eq!(state(&store).await, before, "{field}");
        }
        let callback = failure();
        store.apply_runner_event(callback.clone()).await.unwrap();
        let failed = state(&store).await;
        assert!(store.apply_runner_event(callback).await.is_err());
        assert_eq!(state(&store).await, failed);
        for kind in [
            "run.failed",
            "run.completed",
            "run.verification_passed",
            "run.verification_waiting",
        ] {
            assert!(
                store
                    .apply_runner_event(input_for(RUN, TOKEN, kind, json!({})))
                    .await
                    .is_err()
            );
            assert_eq!(state(&store).await, failed, "{kind}");
        }
        for terminal in ["completed", "cancelled", "lost"] {
            sqlx::query("UPDATE runs SET status=$1 WHERE id=$2")
                .bind(terminal)
                .bind(RUN)
                .execute(&store.pool)
                .await
                .unwrap();
            let before = state(&store).await;
            assert!(store.apply_runner_event(failure()).await.is_err());
            assert_eq!(state(&store).await, before, "{terminal}");
        }
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_superseded_active_callback_cannot_fail_newer_assignment(pool: PgPool) {
        let store = fixture(pool).await;
        let newer = Uuid::from_u128(40);
        sqlx::query(
            "INSERT INTO runs (id,corp_id,task_id,agent_id,runner_id,assignment_token,
                               status,workspace_run_id,created_at)
             VALUES ($1,$2,$3,$4,'issue169-runner',$5,'running',$1,now()+interval '1 second')",
        )
        .bind(newer)
        .bind(CORP)
        .bind(TASK)
        .bind(AGENT)
        .bind(Uuid::from_u128(41))
        .execute(&store.pool)
        .await
        .unwrap();
        sqlx::query("UPDATE agents SET current_run_id=$1 WHERE id=$2")
            .bind(newer)
            .bind(AGENT)
            .execute(&store.pool)
            .await
            .unwrap();
        let before = state(&store).await;
        assert!(store.apply_runner_event(failure()).await.is_err());
        assert_eq!(state(&store).await, before);
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_failures_preserve_accepted_outcomes_and_unrelated_blocks(pool: PgPool) {
        let store = fixture(pool).await;
        for protected in [
            "verified",
            "publishing",
            "published",
            "failed",
            "cancelled",
            "verification_failed",
            "blocked",
        ] {
            restore_active(&store, protected).await;
            sqlx::query(
                "UPDATE factory_work_items SET failure_detail='unrelated policy' WHERE id=$1",
            )
            .bind(FACTORY)
            .execute(&store.pool)
            .await
            .unwrap();
            let before = state(&store).await;
            let outcome = store.apply_runner_event(failure()).await.unwrap();
            let after = state(&store).await;
            assert!(outcome.related_events.is_empty(), "{protected}");
            assert_eq!(after["factory"], before["factory"], "{protected}");
            assert_eq!(after["runs"][0]["status"], "failed");
        }
        for terminal in ["completed", "cancelled"] {
            restore_active(&store, "published").await;
            sqlx::query("UPDATE missions SET status=$1 WHERE id=$2")
                .bind(terminal)
                .bind(MISSION)
                .execute(&store.pool)
                .await
                .unwrap();
            let before = state(&store).await;
            assert!(store.apply_runner_event(failure()).await.is_err());
            assert_eq!(state(&store).await, before);
        }
        restore_active(&store, "verified").await;
        sqlx::query("UPDATE tasks SET status='completed' WHERE id=$1")
            .bind(TASK)
            .execute(&store.pool)
            .await
            .unwrap();
        let before = state(&store).await;
        assert!(store.apply_runner_event(failure()).await.is_err());
        assert_eq!(state(&store).await, before);
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_failure_rolls_back_all_state_and_events_on_reconciliation_error(
        pool: PgPool,
    ) {
        let store = fixture(pool).await;
        for injection in [
            "ALTER TABLE factory_work_items ADD CONSTRAINT injected_failure CHECK (state <> 'blocked')",
            "ALTER TABLE events ADD CONSTRAINT injected_failure CHECK (type <> 'factory.blocked')",
        ] {
            sqlx::query(injection).execute(&store.pool).await.unwrap();
            let before = state(&store).await;
            assert!(store.apply_runner_event(failure()).await.is_err());
            assert_eq!(state(&store).await, before);
            sqlx::raw_sql(
                "ALTER TABLE factory_work_items DROP CONSTRAINT IF EXISTS injected_failure;
                 ALTER TABLE events DROP CONSTRAINT IF EXISTS injected_failure;",
            )
            .execute(&store.pool)
            .await
            .unwrap();
        }
        store.apply_runner_event(failure()).await.unwrap();
        assert_eq!(state(&store).await["factory"][0]["state"], "blocked");
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_native_same_run_resume_reaches_verified_without_rewriting_history(
        pool: PgPool,
    ) {
        let store = fixture(pool).await;
        store.apply_runner_event(failure()).await.unwrap();
        let failed = state(&store).await;
        let (launch, event) = store.create_resume_run(CORP, RUN, OWNER).await.unwrap();
        let resumed = state(&store).await;
        assert_eq!(resumed["runs"][0], failed["runs"][0]);
        assert_eq!(resumed["factory"][0]["state"], "running");
        assert!(resumed["factory"][0]["failure_detail"].is_null());
        assert_eq!(resumed["factory"][0]["version"], 3);
        assert_eq!(resumed["missions"][0]["status"], "running");
        assert_eq!(resumed["tasks"][0]["attempt_count"], 2);
        assert_eq!(launch.source_run_id, RUN);
        assert_eq!(launch.workspace_run_id, RUN);
        assert_eq!(launch.provider_session_id, "fixture-session");
        assert_eq!(
            launch.source_base_commit.as_deref(),
            Some("a".repeat(40).as_str())
        );
        assert_eq!(launch.model.as_deref(), Some("fixture-model"));
        assert_eq!(launch.reasoning_effort.as_deref(), Some("medium"));
        assert_eq!(launch.write_scope, vec!["result.md"]);
        let events = resumed["events"].as_array().unwrap();
        assert_eq!(events[0], failed["events"][0]);
        assert_eq!(events[1], failed["events"][1]);
        assert_eq!(events[2]["type"], "factory.running");
        assert_eq!(events[2]["actor_id"], json!(OWNER));
        assert_eq!(events[2]["payload"]["source_run_id"], json!(RUN));
        assert_eq!(events[3]["type"], "run.resume_requested");
        assert_eq!(events[3]["seq"], event.seq);
        assert!(store.create_resume_run(CORP, RUN, OWNER).await.is_err());
        assert!(store.apply_runner_event(failure()).await.is_err());
        assert_eq!(state(&store).await, resumed);
        complete(&store, launch.run_id, launch.assignment_token).await;
        let completed = state(&store).await;
        assert_eq!(completed["factory"][0]["state"], "verified");
        assert_eq!(completed["missions"][0]["status"], "completed");
        assert_eq!(completed["tasks"][0]["status"], "completed");
        assert_eq!(completed["runs"][0], failed["runs"][0]);
        assert_eq!(completed["runs"].as_array().unwrap().len(), 2);
        assert_eq!(completed["tasks"][0]["attempt_count"], 2);
        assert_eq!(completed["recoveries"], Value::Null);
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_native_resume_does_not_clear_stale_version_or_policy_block(pool: PgPool) {
        let store = fixture(pool).await;
        store.apply_runner_event(failure()).await.unwrap();
        // A newer policy generation with the same diagnostic must not inherit
        // the old failure event's authority.
        sqlx::query("UPDATE factory_work_items SET version=version+1 WHERE id=$1")
            .bind(FACTORY)
            .execute(&store.pool)
            .await
            .unwrap();
        let blocked = state(&store).await["factory"].clone();
        let (launch, _) = store.create_resume_run(CORP, RUN, OWNER).await.unwrap();
        assert_eq!(state(&store).await["factory"], blocked);
        complete(&store, launch.run_id, launch.assignment_token).await;
        let after = state(&store).await;
        assert_eq!(after["factory"], blocked);
        assert_eq!(after["missions"][0]["status"], "completed");
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_native_lease_renewal_preserves_exact_failure_origin_through_resume(
        pool: PgPool,
    ) {
        let store = fixture(pool).await;
        store.apply_runner_event(failure()).await.unwrap();
        let failed = state(&store).await;
        assert_eq!(renew(&store, 2).await.work_item.version, 3);
        assert_eq!(renew(&store, 3).await.work_item.version, 4);
        let renewed = state(&store).await;
        assert!(renew(&store, 3).await.replayed);
        assert_eq!(state(&store).await, renewed);
        assert_eq!(renewed["factory"][0]["state"], "blocked");
        assert_eq!(renewed["factory"][0]["failure_detail"], ACK_FAILURE);
        for field in ["source_revision", "policy", "mission_id", "claim_token"] {
            assert_eq!(renewed["factory"][0][field], failed["factory"][0][field]);
        }
        let (launch, _) = store.create_resume_run(CORP, RUN, OWNER).await.unwrap();
        let resumed = state(&store).await;
        assert_eq!(resumed["factory"][0]["state"], "running");
        assert_eq!(resumed["factory"][0]["version"], 5);
        let resume_event = resumed["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|event| event["type"] == "factory.running")
            .unwrap();
        assert_eq!(resume_event["payload"]["source_run_id"], json!(RUN));
        assert_eq!(resume_event["payload"]["failure_origin_version"], 2);
        assert_eq!(resume_event["payload"]["resumed_factory_version"], 4);
        complete(&store, launch.run_id, launch.assignment_token).await;
        let completed = state(&store).await;
        assert_eq!(completed["factory"][0]["state"], "verified");
        assert_eq!(completed["missions"][0]["status"], "completed");
        assert_eq!(completed["runs"][0], failed["runs"][0]);
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_lease_chain_rejects_changed_authority_or_unproven_revisions(pool: PgPool) {
        let store = fixture(pool).await;
        store.apply_runner_event(failure()).await.unwrap();
        renew(&store, 2).await;
        let unchanged = state(&store).await;
        for mutation in [
            "UPDATE factory_work_items SET version=version+1",
            "UPDATE factory_work_items SET policy='{\"changed\":true}'",
            "UPDATE factory_work_items SET source_revision='replacement-revision'",
            "UPDATE factory_work_items SET failure_detail='replacement detail'",
            "DELETE FROM factory_operations WHERE operation='renew'",
            "UPDATE factory_operations SET claim_token='00000000-0000-0000-0000-000000000099' WHERE operation='renew'",
            "UPDATE factory_operations SET request=jsonb_set(request,'{expected_version}','99') WHERE operation='renew'",
            "UPDATE events SET type='factory.unknown' WHERE type='factory.claim_renewed'",
            "UPDATE events SET aggregate_id='00000000-0000-0000-0000-000000000099' WHERE type='factory.claim_renewed'",
        ] {
            let mut tx = store.pool.begin().await.unwrap();
            sqlx::query(mutation).execute(&mut *tx).await.unwrap();
            let (item, token) = factory_work_item_tx(&mut tx, CORP, FACTORY, true)
                .await
                .unwrap()
                .unwrap();
            assert!(
                failure_origin_tx(&mut tx, &item, token)
                    .await
                    .unwrap()
                    .is_none(),
                "{mutation}"
            );
            tx.rollback().await.unwrap();
        }
        assert_eq!(state(&store).await, unchanged);
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_native_resume_requires_exact_origin_run_and_preserves_other_failure(
        pool: PgPool,
    ) {
        let store = fixture(pool).await;
        store.apply_runner_event(failure()).await.unwrap();
        let other_task = Uuid::from_u128(50);
        let other_run = Uuid::from_u128(51);
        sqlx::query(
            "INSERT INTO tasks
                (id,corp_id,mission_id,title,objective,status,assigned_agent_id,
                 plan_key,contract,verification_policy,attempt_count,max_attempts)
             SELECT $1,corp_id,mission_id,'Other root',objective,'failed',assigned_agent_id,
                    'other-root',contract,verification_policy,2,2 FROM tasks WHERE id=$2",
        )
        .bind(other_task)
        .bind(TASK)
        .execute(&store.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO runs
                (id,corp_id,task_id,agent_id,runner_id,assignment_token,status,workspace_run_id,
                 provider_session_id,workspace_disposition,workspace_path,workspace_base_commit)
             VALUES ($1,$2,$3,$4,'issue169-runner',$5,'failed',$1,
                     'other-fixture-session','preserved','other-fixture-worktree',$6)",
        )
        .bind(other_run)
        .bind(CORP)
        .bind(other_task)
        .bind(AGENT)
        .bind(Uuid::from_u128(52))
        .bind("a".repeat(40))
        .execute(&store.pool)
        .await
        .unwrap();
        let blocked = state(&store).await["factory"].clone();
        store
            .create_resume_run(CORP, other_run, OWNER)
            .await
            .unwrap();
        assert_eq!(state(&store).await["factory"], blocked);
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_native_resume_preserves_authorization_budgets_and_protected_outcomes(
        pool: PgPool,
    ) {
        let store = fixture(pool).await;
        store.apply_runner_event(failure()).await.unwrap();
        let before = state(&store).await;
        assert!(store.create_resume_run(CORP, RUN, OUTSIDER).await.is_err());
        assert!(
            store
                .create_resume_run(Uuid::from_u128(99), RUN, OWNER)
                .await
                .is_err()
        );
        assert_eq!(state(&store).await, before);
        for protected in ["verified", "publishing", "published", "failed", "cancelled"] {
            sqlx::query("UPDATE factory_work_items SET state=$1 WHERE id=$2")
                .bind(protected)
                .bind(FACTORY)
                .execute(&store.pool)
                .await
                .unwrap();
            let before = state(&store).await;
            assert!(
                store.create_resume_run(CORP, RUN, OWNER).await.is_err(),
                "{protected}"
            );
            assert_eq!(state(&store).await, before, "{protected}");
        }
        sqlx::query("UPDATE factory_work_items SET state='blocked' WHERE id=$1")
            .bind(FACTORY)
            .execute(&store.pool)
            .await
            .unwrap();
        for mutation in [
            "UPDATE runs SET breaker_stage='stop'",
            "UPDATE runs SET verification_status='failed'",
        ] {
            sqlx::query(mutation).execute(&store.pool).await.unwrap();
            let before = state(&store).await;
            assert!(store.create_resume_run(CORP, RUN, OWNER).await.is_err());
            assert_eq!(state(&store).await, before);
            sqlx::raw_sql("UPDATE runs SET breaker_stage='healthy',verification_status='running';")
                .execute(&store.pool)
                .await
                .unwrap();
        }
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_resume_rejects_accounted_budget_exhaustion(pool: PgPool) {
        let store = fixture(pool).await;
        let before = state(&store).await;
        // Account an overrun through the actual store event path, not by
        // lowering immutable original authority or rewriting stored spend.
        store
            .apply_runner_event(input_for(
                RUN,
                TOKEN,
                "run.usage",
                json!({"input_tokens":999976,"output_tokens":0,"cost_microusd":0}),
            ))
            .await
            .unwrap();
        store.apply_runner_event(failure()).await.unwrap();
        let exhausted = state(&store).await;
        assert_eq!(exhausted["runs"][0]["input_tokens"], 999987);
        assert_eq!(exhausted["runs"][0]["output_tokens"], 13);
        for field in [
            "original_budget_tokens",
            "original_budget_cost_microusd",
            "budget_tokens",
            "budget_cost_microusd",
        ] {
            assert_eq!(
                exhausted["missions"][0][field],
                before["missions"][0][field]
            );
        }
        let error = store.create_resume_run(CORP, RUN, OWNER).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("mission has no remaining authorized budget")
        );
        assert_eq!(state(&store).await, exhausted);
    }

    async fn review_resumed_run(pool: PgPool, preserve_policy_block: bool, prefix: &str) {
        let store = fixture(pool).await;
        let gate = json!({
            "type":"independent_review","roles":["owner","member"],"exclude_requester":true
        });
        sqlx::query("UPDATE tasks SET verification_policy=$1 WHERE id=$2")
            .bind(json!({"checks":[{"type":"file","path":"result.md","min_bytes":1}],"manual_gate":gate}))
            .bind(TASK)
            .execute(&store.pool)
            .await
            .unwrap();
        store.apply_runner_event(failure()).await.unwrap();
        if preserve_policy_block {
            // A real controller may replace an execution block with a policy
            // block, even with identical text. Renewals on either side cannot
            // turn the older execution origin into authority for that block.
            renew(&store, 2).await;
            store
                .transition_factory_work_item(TransitionFactoryWorkItemInput {
                    corp_id: CORP,
                    work_item_id: FACTORY,
                    actor_id: OWNER,
                    claim_token: Uuid::from_u128(16),
                    expected_version: 3,
                    state: FactoryWorkItemState::Running,
                    failure_detail: None,
                    idempotency_key: "issue169-controller-running".to_owned(),
                })
                .await
                .unwrap();
            store
                .transition_factory_work_item(TransitionFactoryWorkItemInput {
                    corp_id: CORP,
                    work_item_id: FACTORY,
                    actor_id: OWNER,
                    claim_token: Uuid::from_u128(16),
                    expected_version: 4,
                    state: FactoryWorkItemState::Blocked,
                    // Deliberately identical text is not the same origin.
                    failure_detail: Some(ACK_FAILURE.to_owned()),
                    idempotency_key: "issue169-policy-block".to_owned(),
                })
                .await
                .unwrap();
            renew(&store, 5).await;
        }
        let failed = state(&store).await;
        let (launch, _) = store.create_resume_run(CORP, RUN, OWNER).await.unwrap();
        verify(&store, launch.run_id, launch.assignment_token).await;
        store
            .apply_runner_event(input_for(
                launch.run_id,
                launch.assignment_token,
                "run.verification_waiting",
                json!({"gate":gate,"gate_type":"independent_review"}),
            ))
            .await
            .unwrap();
        let waiting = state(&store).await;
        assert_eq!(
            waiting["factory"][0]["state"],
            if preserve_policy_block {
                "blocked"
            } else {
                "awaiting_approval"
            }
        );
        assert!(
            store
                .decide_verification(CORP, launch.run_id, OWNER, true, "", Some(Uuid::new_v4()))
                .await
                .is_err()
        );
        assert!(
            store
                .decide_verification(
                    CORP,
                    launch.run_id,
                    OUTSIDER,
                    true,
                    "",
                    Some(Uuid::new_v4())
                )
                .await
                .is_err()
        );
        assert_eq!(state(&store).await, waiting);
        sqlx::query("INSERT INTO room_memberships (room_id,actor_id) VALUES ($1,$2)")
            .bind(ROOM)
            .bind(OUTSIDER)
            .execute(&store.pool)
            .await
            .unwrap();
        let role_error = store
            .decide_verification(
                CORP,
                launch.run_id,
                OUTSIDER,
                true,
                "",
                Some(Uuid::new_v4()),
            )
            .await
            .unwrap_err();
        assert!(
            role_error
                .to_string()
                .contains("actor role manager cannot decide")
        );
        sqlx::query("DELETE FROM room_memberships WHERE room_id=$1 AND actor_id=$2")
            .bind(ROOM)
            .bind(OUTSIDER)
            .execute(&store.pool)
            .await
            .unwrap();
        assert_eq!(state(&store).await, waiting);
        let reviewer = Uuid::from_u128(11);
        let key = Uuid::new_v4();
        gated_before_rows(
            &store,
            prefix,
            store.decide_verification(CORP, launch.run_id, reviewer, true, "reviewed", Some(key)),
        )
        .await;
        let after = state(&store).await;
        assert_eq!(after["missions"][0]["status"], "completed");
        assert_eq!(after["runs"][0], failed["runs"][0]);
        if preserve_policy_block {
            assert_eq!(after["factory"], failed["factory"]);
        } else {
            assert_eq!(after["factory"][0]["state"], "verified");
            assert!(after["factory"][0]["failure_detail"].is_null());
        }
        assert!(
            store
                .decide_verification(CORP, launch.run_id, reviewer, true, "reviewed", Some(key))
                .await
                .unwrap()
                .replayed
        );
        assert_eq!(state(&store).await, after);
        sqlx::query("DELETE FROM room_memberships WHERE actor_id=$1 AND room_id=$2")
            .bind(reviewer)
            .bind(ROOM)
            .execute(&store.pool)
            .await
            .unwrap();
        assert!(
            store
                .decide_verification(CORP, launch.run_id, reviewer, true, "reviewed", Some(key))
                .await
                .is_err()
        );
        assert_eq!(state(&store).await, after);
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_resumed_manual_review_retains_authority_and_recovery_lock_order(
        pool: PgPool,
    ) {
        review_resumed_run(pool, false, "factory:item").await;
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_resumed_review_cannot_clear_policy_block_and_obeys_publication_gate(
        pool: PgPool,
    ) {
        review_resumed_run(pool, true, "publication:factory").await;
    }

    async fn gated_before_rows<T>(
        store: &PgStore,
        prefix: &str,
        operation: impl Future<Output = Result<T>>,
    ) -> T {
        let mut blocker = store.pool.begin().await.unwrap();
        lock_factory_keys_tx(&mut blocker, &[format!("{prefix}:{CORP}:{FACTORY}")])
            .await
            .unwrap();
        sqlx::query("SELECT id FROM factory_work_items WHERE id=$1 FOR UPDATE")
            .bind(FACTORY)
            .fetch_one(&mut *blocker)
            .await
            .unwrap();
        let mut operation = Box::pin(operation);
        let mut probe = Box::pin(async {
            let deadline = Instant::now() + StdDuration::from_secs(10);
            loop {
                let waiting: Option<String> = sqlx::query_scalar(
                    "SELECT query FROM pg_stat_activity
                     WHERE datname=current_database() AND pid<>pg_backend_pid()
                       AND wait_event_type='Lock' ORDER BY query_start LIMIT 1",
                )
                .fetch_optional(&store.pool)
                .await?;
                if let Some(query) = waiting {
                    if !query.contains("pg_advisory_xact_lock") {
                        return Err(anyhow!("Factory lifecycle took row locks before its gate"));
                    }
                    for query in [
                        "SELECT id FROM runs FOR UPDATE NOWAIT",
                        "SELECT id FROM tasks FOR UPDATE NOWAIT",
                        "SELECT id FROM missions FOR UPDATE NOWAIT",
                    ] {
                        sqlx::query(query).fetch_all(&mut *blocker).await?;
                    }
                    blocker.commit().await?;
                    return Ok(());
                }
                if Instant::now() >= deadline {
                    return Err(anyhow!("Factory lifecycle never waited at its shared gate"));
                }
            }
        });
        let mut finished = None;
        std::future::poll_fn(|context| {
            if finished.is_none()
                && let Poll::Ready(result) = operation.as_mut().poll(context)
            {
                finished = Some(result);
            }
            probe.as_mut().poll(context)
        })
        .await
        .unwrap();
        match finished {
            Some(result) => result,
            None => operation.await,
        }
        .unwrap()
    }

    async fn exercise_lifecycle_gate(pool: PgPool, prefix: &str) {
        let store = fixture(pool).await;
        for kind in [
            "run.failed",
            "run.cancelled",
            "run.completed",
            "run.verification_failed",
            "run.verification_waiting",
        ] {
            restore_active(&store, "running").await;
            let gate = (kind == "run.verification_waiting")
                .then(|| json!({"type":"human_approval","roles":["owner"]}));
            sqlx::query("UPDATE tasks SET verification_policy=$1 WHERE id=$2")
                .bind(json!({"checks":[{"type":"file","path":"result.md","min_bytes":1}],"manual_gate":gate}))
                .bind(TASK).execute(&store.pool).await.unwrap();
            sqlx::query("UPDATE runs SET verification_status='passed' WHERE id=$1")
                .bind(RUN)
                .execute(&store.pool)
                .await
                .unwrap();
            sqlx::query("UPDATE verification_evidence SET status=$1 WHERE run_id=$2")
                .bind(if kind == "run.verification_failed" {
                    "failed"
                } else {
                    "passed"
                })
                .bind(RUN)
                .execute(&store.pool)
                .await
                .unwrap();
            let payload = if kind == "run.verification_waiting" {
                json!({"gate":gate,"gate_type":"human_approval"})
            } else {
                json!({"error":ACK_FAILURE,"summary":"fixture complete","reason":"fixture cancelled"})
            };
            let outcome = gated_before_rows(
                &store,
                prefix,
                store.apply_runner_event(input_for(RUN, TOKEN, kind, payload)),
            )
            .await;
            assert_eq!(outcome.event.unwrap().event_type, kind);
        }
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_all_five_lifecycle_events_take_recovery_gate_before_rows(pool: PgPool) {
        exercise_lifecycle_gate(pool, "factory:item").await;
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_all_five_lifecycle_events_take_publication_gate_before_rows(pool: PgPool) {
        exercise_lifecycle_gate(pool, "publication:factory").await;
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_native_resume_joins_existing_gate_order(pool: PgPool) {
        let store = fixture(pool).await;
        store.apply_runner_event(failure()).await.unwrap();
        let (launch, _) = gated_before_rows(
            &store,
            "publication:factory",
            store.create_resume_run(CORP, RUN, OWNER),
        )
        .await;
        assert_eq!(launch.source_run_id, RUN);
        assert_eq!(state(&store).await["factory"][0]["state"], "running");
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_active_descendant_rejects_parent_resume_without_lineage_lock_wait(
        pool: PgPool,
    ) {
        let store = fixture(pool).await;
        store.apply_runner_event(failure()).await.unwrap();
        // The quiescent, authorized preserved-lineage resume remains valid.
        let (child, _) = store.create_resume_run(CORP, RUN, OWNER).await.unwrap();
        let before = state(&store).await;
        let mut callback_rows = store.pool.begin().await.unwrap();
        // Model the first row lock of S's streaming callback. Parent admission
        // must reject without waiting for S while holding task/mission rows.
        sqlx::query("SELECT id FROM runs WHERE id=$1 FOR UPDATE")
            .bind(child.run_id)
            .fetch_one(&mut *callback_rows)
            .await
            .unwrap();
        let mut resume = Box::pin(store.create_resume_run(CORP, RUN, OWNER));
        let mut probe = Box::pin(async {
            let deadline = Instant::now() + StdDuration::from_secs(10);
            loop {
                let waiting: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM pg_stat_activity
                     WHERE datname=current_database() AND pid<>pg_backend_pid()
                       AND wait_event_type='Lock' AND query LIKE '%workspace_run_id%'
                       AND query LIKE '%FOR UPDATE%')",
                )
                .fetch_one(&store.pool)
                .await?;
                if waiting || Instant::now() >= deadline {
                    return Err(anyhow!(
                        "parent resume waited on an active descendant's row"
                    ));
                }
            }
        });
        let error = std::future::poll_fn(|context| {
            if let Poll::Ready(result) = resume.as_mut().poll(context) {
                return Poll::Ready(result);
            }
            probe.as_mut().poll(context)
        })
        .await
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("task or assigned agent already has an active run")
        );
        callback_rows.rollback().await.unwrap();
        assert_eq!(state(&store).await, before);
        let output = store
            .apply_runner_event(input_for(
                child.run_id,
                child.assignment_token,
                "run.output",
                json!({"text":"descendant output after release"}),
            ))
            .await
            .unwrap();
        assert_eq!(output.event.unwrap().event_type, "run.output");
        assert!(output.related_events.is_empty());
        let after = state(&store).await;
        assert_eq!(after["runs"], before["runs"]);
        assert_eq!(after["tasks"], before["tasks"]);
        assert_eq!(after["factory"], before["factory"]);
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_output_and_usage_never_wait_on_factory_gates(pool: PgPool) {
        let store = fixture(pool).await;
        let mut blocker = store.pool.begin().await.unwrap();
        lock_factory_keys_tx(
            &mut blocker,
            &[
                format!("factory:item:{CORP}:{FACTORY}"),
                format!("publication:factory:{CORP}:{FACTORY}"),
            ],
        )
        .await
        .unwrap();
        sqlx::query("SELECT id FROM factory_work_items FOR UPDATE")
            .fetch_all(&mut *blocker)
            .await
            .unwrap();
        for kind in ["run.output", "run.usage"] {
            let mut operation = Box::pin(store.apply_runner_event(input_for(
                RUN,
                TOKEN,
                kind,
                json!({"text":"fixture output","input_tokens":1,"output_tokens":1}),
            )));
            let mut probe = Box::pin(async {
                let deadline = Instant::now() + StdDuration::from_secs(10);
                loop {
                    let waiting: bool = sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM pg_stat_activity
                         WHERE datname=current_database() AND pid<>pg_backend_pid()
                           AND wait_event_type='Lock')",
                    )
                    .fetch_one(&store.pool)
                    .await?;
                    if waiting || Instant::now() >= deadline {
                        return Err(anyhow!("high-volume event waited on Factory"));
                    }
                }
            });
            let outcome = std::future::poll_fn(|context| {
                if let Poll::Ready(result) = operation.as_mut().poll(context) {
                    return Poll::Ready(result);
                }
                probe.as_mut().poll(context)
            })
            .await
            .unwrap();
            assert!(outcome.related_events.is_empty());
        }
        blocker.rollback().await.unwrap();
        assert_eq!(state(&store).await["factory"][0]["state"], "running");
    }

    async fn detached_review_fixture(pool: PgPool) -> PgStore {
        let store = fixture(pool).await;
        sqlx::query(
            "INSERT INTO runner_nodes (id,corp_id,hostname,os,connection_epoch,status)
             VALUES ('issue169-runner',$1,'isolated-sqlx-fixture','test',$2,'connected')",
        )
        .bind(CORP)
        .bind(Uuid::from_u128(20))
        .execute(&store.pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE tasks SET attempt_count=1, status='running',
                contract=jsonb_set(contract,'{deliverable}',$1) WHERE id=$2",
        )
        .bind(json!({
            "form":"typed_artifact_set","commit_after_verification":false,"paths":["result.md"]
        }))
        .bind(TASK)
        .execute(&store.pool)
        .await
        .unwrap();
        sqlx::query("UPDATE runs SET status='running' WHERE id=$1")
            .bind(RUN)
            .execute(&store.pool)
            .await
            .unwrap();
        sqlx::query("UPDATE agents SET status='working',station='terminal' WHERE id=$1")
            .bind(AGENT)
            .execute(&store.pool)
            .await
            .unwrap();
        assert_eq!(
            state(&store).await["agents"][0]["current_run_id"],
            json!(RUN)
        );

        // Exercise the actual store reservation/finalization lifecycle, not a
        // seeded detached agent. These metadata fixtures do not verify object
        // bytes/signatures or claim server/runner transport acceptance.
        for role in ["provider_evidence", "source_deliverable"] {
            let event_type = if role == "provider_evidence" {
                "run.artifact_upload"
            } else {
                "run.deliverable_upload"
            };
            let bytes =
                serde_json::to_vec(&json!({"fixture":"detached-review","role":role})).unwrap();
            let digest = hex::encode(Sha256::digest(&bytes));
            let metadata = if role == "source_deliverable" {
                json!({
                    "form":"typed_artifact_set","verification_sha256":"c".repeat(64),
                    "base_commit":"a".repeat(40),"head_commit":null,"branch":"crony/fixture",
                    "integration_state":"ready_for_review"
                })
            } else {
                json!({})
            };
            let input = input_for(
                RUN,
                TOKEN,
                event_type,
                json!({"artifact_role":role,"sha256":digest,"bytes":bytes.len(),"metadata":metadata}),
            );
            let artifact = StoredArtifact {
                id: input.event_id,
                corp_id: CORP,
                task_id: TASK,
                run_id: RUN,
                producer_agent_id: AGENT,
                producer_runner_id: "issue169-runner".to_owned(),
                verifier: "isolated-store-fixture".to_owned(),
                object_key: format!("corps/{CORP}/{role}/{digest}"),
                uri: format!("/api/corps/{CORP}/artifacts/{}", input.event_id),
                sha256: digest,
                media_type: "application/json".to_owned(),
                bytes: bytes.len() as i64,
                artifact_role: role.to_owned(),
                file_name: format!("{role}.json"),
                metadata,
                provenance_signature: "d".repeat(64),
                retention_until: Utc::now() + chrono::Duration::days(1),
            };
            let staging_key = format!("staging/corps/{CORP}/{}", artifact.id);
            let prepared = store
                .prepare_artifact_upload(input, artifact.clone(), &staging_key)
                .await
                .unwrap();
            assert_eq!(prepared.status, "staged");
            let event = store
                .finalize_artifact_upload(CORP, artifact.id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(event.payload["artifact_role"], role);
            let detached = state(&store).await;
            assert_eq!(detached["agents"][0]["status"], "reviewing");
            assert_eq!(detached["agents"][0]["station"], "review");
            assert!(detached["agents"][0]["current_run_id"].is_null());
            if role == "provider_evidence" {
                store
                    .apply_runner_event(input_for(
                        RUN,
                        TOKEN,
                        "run.verification_started",
                        json!({}),
                    ))
                    .await
                    .unwrap();
                assert!(state(&store).await["agents"][0]["current_run_id"].is_null());
            }
        }
        store
    }

    async fn detached_review_other_assignment(store: &PgStore, status: &str, offset_seconds: f64) {
        sqlx::query(
            "INSERT INTO tasks
                (id,corp_id,mission_id,title,objective,status,assigned_agent_id,
                 plan_key,contract,verification_policy,attempt_count,max_attempts)
             SELECT $1,corp_id,mission_id,'Other assignment',objective,'review',assigned_agent_id,
                    'other-detached-review',contract,verification_policy,1,2
             FROM tasks WHERE id=$2",
        )
        .bind(Uuid::from_u128(50))
        .bind(TASK)
        .execute(&store.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO runs
                (id,corp_id,task_id,agent_id,runner_id,assignment_token,status,
                 workspace_run_id,created_at)
             SELECT $1,corp_id,$2,agent_id,runner_id,$3,$4,$1,
                    created_at+make_interval(secs=>$5)
             FROM runs WHERE id=$6",
        )
        .bind(Uuid::from_u128(51))
        .bind(Uuid::from_u128(50))
        .bind(Uuid::from_u128(52))
        .bind(status)
        .bind(offset_seconds)
        .bind(RUN)
        .execute(&store.pool)
        .await
        .unwrap();
    }

    async fn assert_detached_review_preserved(store: &PgStore) {
        let before = state(store).await;
        store.apply_runner_event(failure()).await.unwrap();
        let after = state(store).await;
        assert_eq!(after["agents"], before["agents"]);
        assert_eq!(after["factory"], before["factory"]);
        assert_eq!(after["tasks"][0]["status"], "ready");
        assert_eq!(after["missions"][0]["status"], "running");
        assert!(
            store
                .schedulable_mission_ids(CORP)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .schedulable_tasks(CORP, MISSION, false)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_detached_review_artifact_failure_releases_native_retry(pool: PgPool) {
        let store = detached_review_fixture(pool).await;
        let before = state(&store).await;
        let artifacts_before: Value = sqlx::query_scalar(
            "SELECT jsonb_agg(to_jsonb(artifact) ORDER BY id) FROM artifacts artifact",
        )
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(artifacts_before.as_array().unwrap().len(), 2);
        assert!(
            artifacts_before
                .as_array()
                .unwrap()
                .iter()
                .all(|artifact| artifact["status"] == "ready")
        );
        assert!(
            store
                .schedulable_mission_ids(CORP)
                .await
                .unwrap()
                .is_empty()
        );
        let outcome = store.apply_runner_event(failure()).await.unwrap();
        let after = state(&store).await;
        let missions = store.schedulable_mission_ids(CORP).await.unwrap();
        let tasks = store.schedulable_tasks(CORP, MISSION, false).await.unwrap();
        assert_eq!(
            (
                after["agents"][0]["status"].clone(),
                after["agents"][0]["current_run_id"].clone(),
                missions,
                tasks.iter().map(|task| task.task_id).collect::<Vec<_>>(),
            ),
            (json!("idle"), Value::Null, vec![MISSION], vec![TASK]),
        );
        assert!(after["agents"][0]["station"].is_null());
        assert_eq!(after["tasks"][0]["status"], "ready");
        assert_eq!(after["tasks"][0]["attempt_count"], 1);
        assert_eq!(after["tasks"][0]["max_attempts"], 2);
        assert_eq!(after["missions"][0]["status"], "running");
        assert_eq!(after["factory"], before["factory"]);
        assert!(outcome.related_events.is_empty());
        for key in [
            "evidence",
            "deliverables",
            "queued",
            "requests",
            "recoveries",
        ] {
            assert_eq!(after[key], before[key], "{key}");
        }
        for field in [
            "provider_session_id",
            "workspace_run_id",
            "workspace_path",
            "workspace_branch",
            "workspace_fingerprint",
            "source_repository",
            "source_base_ref",
            "source_base_commit",
            "input_tokens",
            "output_tokens",
            "cost_microusd",
            "budget_tokens_limit",
            "budget_cost_microusd_limit",
        ] {
            assert_eq!(after["runs"][0][field], before["runs"][0][field], "{field}");
        }
        let artifacts_after: Value = sqlx::query_scalar(
            "SELECT jsonb_agg(to_jsonb(artifact) ORDER BY id) FROM artifacts artifact",
        )
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(artifacts_after, artifacts_before);
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_detached_review_keeps_attached_exact_run_cleanup(pool: PgPool) {
        let store = fixture(pool).await;
        store.apply_runner_event(failure()).await.unwrap();
        let after = state(&store).await;
        assert_eq!(after["agents"][0]["status"], "idle");
        assert!(after["agents"][0]["current_run_id"].is_null());
        assert!(after["agents"][0]["station"].is_null());
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_detached_review_preserves_other_pointer(pool: PgPool) {
        let store = detached_review_fixture(pool).await;
        // The failed run is newer and the other run is not active: pointer
        // fencing must protect this assignment independently of both queries.
        detached_review_other_assignment(&store, "completed", -1.0).await;
        sqlx::query("UPDATE agents SET current_run_id=$1 WHERE id=$2")
            .bind(Uuid::from_u128(51))
            .bind(AGENT)
            .execute(&store.pool)
            .await
            .unwrap();
        assert_detached_review_preserved(&store).await;
    }

    async fn detached_review_active_negative(pool: PgPool, status: &str) {
        let store = detached_review_fixture(pool).await;
        // Older active work must block cleanup even when this failed run is
        // the uniquely newest assignment and the agent has no current pointer.
        detached_review_other_assignment(&store, status, -1.0).await;
        assert_detached_review_preserved(&store).await;
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_detached_review_preserves_other_provisioning(pool: PgPool) {
        detached_review_active_negative(pool, "provisioning").await;
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_detached_review_preserves_other_starting(pool: PgPool) {
        detached_review_active_negative(pool, "starting").await;
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_detached_review_preserves_other_running(pool: PgPool) {
        detached_review_active_negative(pool, "running").await;
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_detached_review_preserves_other_waiting_for_input(pool: PgPool) {
        detached_review_active_negative(pool, "waiting_for_input").await;
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_detached_review_preserves_other_waiting_for_approval(pool: PgPool) {
        detached_review_active_negative(pool, "waiting_for_approval").await;
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_detached_review_preserves_other_verifying(pool: PgPool) {
        detached_review_active_negative(pool, "verifying").await;
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_detached_review_preserves_later_terminal_assignment(pool: PgPool) {
        let store = detached_review_fixture(pool).await;
        detached_review_other_assignment(&store, "completed", 1.0).await;
        assert_detached_review_preserved(&store).await;
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the approved isolated issue169 SQLx loader"]
    async fn issue169_detached_review_rejects_ambiguous_assignment_order(pool: PgPool) {
        let store = detached_review_fixture(pool).await;
        detached_review_other_assignment(&store, "completed", 0.0).await;
        assert_detached_review_preserved(&store).await;
    }
}
