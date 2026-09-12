//! Reconcile an explicitly requested, proven legacy controller projection.
//! Generic cancelled work remains terminal. This never resumes a provider,
//! changes source/mission/task/run accounting, or grants publication authority.
use super::*;

#[derive(Debug, Clone)]
pub struct ReconcileCheckpointCancellationInput {
    pub corp_id: Uuid,
    pub work_item_id: Uuid,
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

/// This is provenance recognition, not authorization. The mutating method below
/// separately requires current native recovery authority and stopped-source proof.
pub(super) async fn cancellation_event_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    source_run_id: Uuid,
) -> Result<Option<Uuid>> {
    if item.state != FactoryWorkItemState::Cancelled
        || item.failure_detail.is_some()
        || item.version <= 1
    {
        return Ok(None);
    }
    let Some(mission_id) = item.mission_id else {
        return Ok(None);
    };
    let expected_key = format!(
        "github-project:{}:{}:{}:{}:transition:cancelled:{}",
        item.source_project_owner,
        item.source_project_number,
        item.source_project_item_id,
        item.source_revision,
        item.version - 1,
    );
    let expected_request = json!({
        "state": "cancelled",
        "work_item_id": item.id,
        "failure_detail": null,
        "expected_version": item.version - 1,
    });
    let candidate: Option<Uuid> = sqlx::query_scalar(
        r#"
        SELECT event.id
        FROM events event
        JOIN factory_operations operation
          ON operation.corp_id=event.corp_id
         AND operation.work_item_id=event.aggregate_id
         AND operation.resulting_version=event.aggregate_version
        JOIN factory_work_items current
          ON current.corp_id=operation.corp_id AND current.id=operation.work_item_id
         AND current.claim_token=operation.claim_token
        JOIN runs source ON source.id=$7 AND source.corp_id=event.corp_id
        JOIN tasks task ON task.id=source.task_id AND task.corp_id=source.corp_id
        JOIN missions mission ON mission.id=task.mission_id AND mission.corp_id=task.corp_id
        WHERE event.corp_id=$1 AND event.aggregate_id=$2
          AND event.aggregate_type='factory_work_item'
          AND event.type='factory.state_changed' AND event.aggregate_version=$3
          AND event.actor_id=$4 AND event.correlation_id=$5
          AND event.payload=jsonb_build_object(
            'state','cancelled','mission_id',$5::uuid,
            'failure_detail',NULL,'previous_state','running')
          AND operation.operation='transition' AND operation.actor_id=$4
          AND operation.idempotency_key=$6 AND operation.request=$8
          AND mission.id=$5 AND mission.status='cancelled'
          AND task.status='cancelled' AND source.status='cancelled'
          AND source.execution_mode='provider' AND source.breaker_stage IN ('suspend','stop')
          AND NOT EXISTS (
            SELECT 1 FROM events newer
            WHERE newer.corp_id=event.corp_id AND newer.aggregate_id=event.aggregate_id
              AND newer.aggregate_type='factory_work_item' AND newer.seq>event.seq)
          AND EXISTS (
            SELECT 1 FROM events cancelled
            WHERE cancelled.corp_id=source.corp_id AND cancelled.aggregate_id=source.id
              AND cancelled.aggregate_type='run' AND cancelled.type='run.cancelled'
              AND cancelled.actor_id IS NULL AND cancelled.correlation_id=mission.id
              AND cancelled.seq<event.seq)
          AND NOT EXISTS (
            SELECT 1 FROM pull_request_publications publication
            WHERE publication.corp_id=event.corp_id
              AND publication.factory_work_item_id=event.aggregate_id)
        "#,
    )
    .bind(item.corp_id)
    .bind(item.id)
    .bind(item.version)
    .bind(item.claim_owner_id)
    .bind(mission_id)
    .bind(expected_key)
    .bind(source_run_id)
    .bind(expected_request)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(candidate)
}

impl PgStore {
    pub async fn reconcile_checkpoint_cancellation(
        &self,
        input: ReconcileCheckpointCancellationInput,
    ) -> Result<FactoryWorkItemOutcome> {
        if input.expected_factory_version <= 1
            || !valid_sha256(&input.expected_workspace_fingerprint)
        {
            return Err(anyhow!(
                "invalid checkpoint reconciliation version or fingerprint"
            ));
        }
        validate_factory_base_commit(&input.expected_head_commit)?;
        let reason =
            normalize_factory_text(&input.reason, "checkpoint reconciliation reason", 4000)?;
        let key = normalize_factory_idempotency_key(&input.idempotency_key)?;
        let source_revision = normalize_factory_text(
            &input.observed_source_revision,
            "observed source revision",
            200,
        )?;
        let request = json!({
            "intent": "checkpoint_cancellation_reconciliation",
            "work_item_id": input.work_item_id,
            "source_run_id": input.source_run_id,
            "expected_factory_version": input.expected_factory_version,
            "cancellation_event_id": input.cancellation_event_id,
            "expected_workspace_fingerprint": input.expected_workspace_fingerprint,
            "expected_head_commit": input.expected_head_commit,
            "observed_source_revision": source_revision,
            "reason": reason,
        });
        let scope = sqlx::query(
            "SELECT run.workspace_run_id,mission.id AS mission_id,mission.requested_by,mission.room_id
             FROM runs run JOIN tasks task ON task.id=run.task_id AND task.corp_id=run.corp_id
             JOIN missions mission ON mission.id=task.mission_id AND mission.corp_id=task.corp_id
             WHERE run.corp_id=$1 AND run.id=$2",
        )
        .bind(input.corp_id)
        .bind(input.source_run_id)
        .fetch_optional(&self.pool)
        .await?
        .context("checkpoint reconciliation source was not found")?;
        let mission_id: Uuid = scope.get("mission_id");
        let room_id: Uuid = scope.get("room_id");
        let workspace_run_id: Uuid = scope.get("workspace_run_id");
        let mut keys = budget_scope_lock_keys(input.corp_id, scope.get("requested_by"));
        keys.extend([
            format!("factory:idempotency:{}:{key}", input.corp_id),
            format!("factory:item:{}:{}", input.corp_id, input.work_item_id),
            format!(
                "publication:factory:{}:{}",
                input.corp_id, input.work_item_id
            ),
            format!("resume:workspace:{}:{workspace_run_id}", input.corp_id),
        ]);
        let mut tx = self.pool.begin().await?;
        lock_factory_keys_tx(&mut tx, &keys).await?;
        // Retain role/kind authority while later source/Factory locks may wait.
        sqlx::query("SELECT id FROM actors WHERE corp_id=$1 AND id=$2 FOR SHARE")
            .bind(input.corp_id)
            .bind(input.actor_id)
            .fetch_optional(&mut *tx)
            .await?;
        ensure_factory_recovery_authorizer_tx(&mut tx, input.corp_id, mission_id, input.actor_id)
            .await?;
        let (current, _) = factory_work_item_tx(&mut tx, input.corp_id, input.work_item_id, true)
            .await?
            .context("checkpoint reconciliation work item was not found")?;
        if current.mission_id != Some(mission_id) {
            return Err(anyhow!(
                "checkpoint reconciliation belongs to another mission"
            ));
        }
        if let Some(operation) = factory_operation_tx(&mut tx, input.corp_id, &key).await? {
            ensure_factory_operation_matches(
                &operation,
                "transition",
                input.actor_id,
                Some(input.work_item_id),
                None,
                &request,
            )?;
            tx.commit().await?;
            return Ok(FactoryWorkItemOutcome {
                work_item: current,
                claim_token: None,
                event: None,
                replayed: true,
            });
        }
        if current.version != input.expected_factory_version
            || current.source_revision != source_revision
            || (current.claim_owner_id != input.actor_id && current.lease_expires_at > Utc::now())
        {
            return Err(anyhow!(
                "conflict: checkpoint reconciliation scope or version changed"
            ));
        }
        let lineage = workspace_lineage_tx(&mut tx, input.corp_id, workspace_run_id).await?;
        if latest_workspace_source_id(&lineage)? != input.source_run_id {
            return Err(anyhow!(
                "checkpoint reconciliation requires the latest preserved source"
            ));
        }
        let active: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM runs run JOIN tasks task
             ON task.id=run.task_id AND task.corp_id=run.corp_id
             WHERE task.corp_id=$1 AND task.mission_id=$2
             AND run.status IN ('provisioning','starting','running','waiting_for_input',
                                'waiting_for_approval','verifying'))",
        )
        .bind(input.corp_id)
        .bind(mission_id)
        .fetch_one(&mut *tx)
        .await?;
        if active {
            return Err(anyhow!(
                "checkpoint reconciliation cannot replace active work"
            ));
        }
        let authority = budget_checkpoint::source_authority_tx(
            &mut tx,
            input.corp_id,
            input.work_item_id,
            input.source_run_id,
        )
        .await?;
        budget_checkpoint::validate_lineage_tx(
            &mut tx,
            input.corp_id,
            input.work_item_id,
            workspace_run_id,
            &authority,
        )
        .await?;
        let binding = sqlx::query(
            "SELECT run.workspace_connection_id,task.contract
             FROM runs run JOIN tasks task ON task.id=run.task_id AND task.corp_id=run.corp_id
             WHERE run.corp_id=$1 AND run.id=$2",
        )
        .bind(input.corp_id)
        .bind(input.source_run_id)
        .fetch_one(&mut *tx)
        .await?;
        let run_connection: Option<Uuid> = binding.get("workspace_connection_id");
        let contract: TaskContract = serde_json::from_value(binding.get("contract"))?;
        if run_connection != contract.workspace_connection_id
            || run_connection
                != factory_workspace_connection_id(&current.policy).map_err(anyhow::Error::msg)?
        {
            return Err(anyhow!(
                "checkpoint reconciliation cannot change the saved connection"
            ));
        }
        if authority.checkpoint.workspace_fingerprint != input.expected_workspace_fingerprint
            || authority.checkpoint.head_commit != input.expected_head_commit
        {
            return Err(anyhow!("checkpoint reconciliation source evidence changed"));
        }
        if cancellation_event_tx(&mut tx, &current, input.source_run_id).await?
            != Some(input.cancellation_event_id)
        {
            return Err(anyhow!(
                "checkpoint reconciliation cannot clear an unrelated Factory cancellation"
            ));
        }
        let row = sqlx::query(
            "UPDATE factory_work_items SET state='running',version=version+1,updated_at=now()
             WHERE corp_id=$1 AND id=$2 RETURNING *",
        )
        .bind(input.corp_id)
        .bind(input.work_item_id)
        .fetch_one(&mut *tx)
        .await?;
        let work_item = map_factory_work_item(row)?;
        record_factory_operation_tx(
            &mut tx,
            NewFactoryOperation {
                corp_id: input.corp_id,
                idempotency_key: &key,
                work_item_id: input.work_item_id,
                actor_id: input.actor_id,
                operation: "transition",
                resulting_version: work_item.version,
                claim_token: None,
                request: &request,
            },
        )
        .await?;
        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(room_id),
                aggregate_version: work_item.version,
                correlation_id: Some(mission_id),
                causation_id: Some(input.cancellation_event_id),
                ..NewEvent::new(
                    input.corp_id,
                    Some(input.actor_id),
                    "factory.checkpoint_cancellation_reconciled",
                    "factory_work_item",
                    input.work_item_id,
                    format!(
                        "factory:{}:checkpoint-reconciled:{}",
                        input.work_item_id, work_item.version
                    ),
                    json!({
                        "previous_state": "cancelled", "state": "running",
                        "source_run_id": input.source_run_id,
                        "cancellation_event_id": input.cancellation_event_id,
                        "checkpoint_event_id": authority.checkpoint_event_id,
                        "termination_event_id": authority.termination_event_id,
                        "mission_id": mission_id, "reason": reason,
                    }),
                )
            },
        )
        .await?
        .context("checkpoint reconciliation event unexpectedly existed")?;
        tx.commit().await?;
        Ok(FactoryWorkItemOutcome {
            work_item,
            claim_token: None,
            event: Some(event),
            replayed: false,
        })
    }
}
