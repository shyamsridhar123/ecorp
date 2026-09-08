//! Retain an exact native checkpoint export without changing its original authority.
use super::*;

/// This is a relational provenance check, not fresh signature/object-byte verification.
pub(super) async fn exported_head_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    run_id: Uuid,
) -> Result<Option<String>> {
    let heads: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT deliverable.head_commit
        FROM runs run
        JOIN tasks task ON task.id=run.task_id AND task.corp_id=run.corp_id
        JOIN factory_verification_recoveries recovery
          ON recovery.replacement_run_id=run.id AND recovery.corp_id=run.corp_id
         AND recovery.task_id=task.id AND recovery.mission_id=task.mission_id
         AND recovery.mode='checkpoint_verification'
        JOIN factory_work_items item
          ON item.id=recovery.factory_work_item_id AND item.corp_id=run.corp_id
         AND item.mission_id=task.mission_id
        JOIN runs source
          ON source.id=recovery.source_run_id AND source.id=run.resumed_from_run_id
         AND source.corp_id=run.corp_id AND source.task_id=run.task_id
         AND source.agent_id=run.agent_id AND source.runner_id=run.runner_id
         AND source.workspace_run_id=run.workspace_run_id
         AND source.workspace_branch=run.workspace_branch
         AND source.workspace_base_commit=run.workspace_base_commit
         AND source.source_repository=run.source_repository
         AND source.source_base_ref=run.source_base_ref
         AND source.source_base_commit=run.source_base_commit
         AND source.workspace_disposition='preserved'
         AND source.workspace_fingerprint=recovery.request->>'expected_workspace_fingerprint'
        JOIN source_deliverables deliverable
          ON deliverable.run_id=run.id AND deliverable.task_id=run.task_id
         AND deliverable.corp_id=run.corp_id
        JOIN artifacts artifact
          ON artifact.id=deliverable.artifact_id AND artifact.corp_id=run.corp_id
         AND artifact.run_id=run.id AND artifact.task_id=run.task_id
         AND artifact.producer_agent_id=run.agent_id AND artifact.producer_runner_id=run.runner_id
         AND artifact.artifact_role='source_deliverable' AND artifact.status='ready'
        WHERE run.id=$1 AND run.corp_id=$2 AND run.execution_mode='verification_only'
          AND run.provider_session_id IS NULL AND run.model IS NULL AND run.reasoning_effort IS NULL
          AND run.workspace_disposition IS DISTINCT FROM 'quarantined'
          AND recovery.checkpoint_authority IS NOT NULL
          AND recovery.contract_revision_id IS NULL
          AND recovery.previous_verification_policy=recovery.replacement_verification_policy
          AND recovery.replacement_verification_policy=task.verification_policy
          AND recovery.request->>'expected_workspace_fingerprint'
              =recovery.checkpoint_authority->'checkpoint'->>'workspace_fingerprint'
          AND artifact.sha256=run.deliverable_sha256
          AND deliverable.verification_sha256=run.verification_sha256
          AND artifact.metadata->>'verification_sha256'=deliverable.verification_sha256
          AND artifact.metadata->>'head_commit'=deliverable.head_commit
          AND artifact.metadata->>'base_commit'=deliverable.base_commit
          AND artifact.metadata->>'branch'=deliverable.branch
          AND artifact.metadata->>'form'=deliverable.form
          AND deliverable.base_commit=run.source_base_commit
          AND deliverable.base_commit=run.workspace_base_commit
          AND deliverable.branch=run.workspace_branch
          AND deliverable.form=task.contract#>>'{deliverable,form}'
          AND (deliverable.form='commit_branch'
               OR task.contract#>>'{deliverable,commit_after_verification}'='true')
          AND task.contract->>'source_repository'=run.source_repository
          AND task.contract->>'source_base_ref'=run.source_base_ref
          AND task.contract->>'source_base_commit'=run.source_base_commit
          AND deliverable.head_commit IS NOT NULL
          AND artifact.retention_until > now()
        LIMIT 2
        "#,
    )
    .bind(run_id)
    .bind(corp_id)
    .fetch_all(&mut **tx)
    .await?;
    match heads.as_slice() {
        [] => Ok(None),
        [head] => {
            validate_factory_base_commit(head)?;
            Ok(Some(head.clone()))
        }
        _ => Err(anyhow!(
            "checkpoint export has ambiguous artifact provenance"
        )),
    }
}

/// Waiting for a human is not a claim that a provider process is still running.
pub(super) async fn review_ready_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    run_id: Uuid,
) -> Result<bool> {
    let waiting: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
          SELECT 1 FROM runs run
          JOIN tasks task ON task.id=run.task_id AND task.corp_id=run.corp_id
          JOIN missions mission ON mission.id=task.mission_id AND mission.corp_id=run.corp_id
          JOIN verification_requests request ON request.run_id=run.id AND request.corp_id=run.corp_id
          JOIN factory_verification_recoveries recovery
            ON recovery.replacement_run_id=run.id AND recovery.corp_id=run.corp_id
           AND recovery.task_id=run.task_id AND recovery.mission_id=task.mission_id
          JOIN factory_work_items item
            ON item.id=recovery.factory_work_item_id AND item.corp_id=run.corp_id
           AND item.mission_id=mission.id
          WHERE run.id=$1 AND run.corp_id=$2 AND run.status='waiting_for_approval'
            AND run.verification_status='waiting_for_approval' AND request.status='pending'
            AND task.status='awaiting_approval' AND task.verification_status='waiting_for_approval'
            AND mission.status='running' AND item.state='awaiting_approval'
            AND recovery.mode='checkpoint_verification' AND recovery.status='running'
        )
        "#,
    )
    .bind(run_id)
    .bind(corp_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(waiting
        && exported_head_tx(tx, corp_id, run_id).await?.is_some()
        && budget_checkpoint::zero_provider_allocation_tx(tx, corp_id, run_id).await?)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn request_review_checkpoint_tx(
    tx: &mut Transaction<'_, Postgres>,
    input: &CheckpointFactoryWorkspaceInput,
    expected_head: &str,
    operation_key: &str,
    command_key: &str,
    operation_request: &Value,
) -> Result<Option<FactoryWorkspaceCheckpointOutcome>> {
    if !review_ready_tx(tx, input.corp_id, input.source_run_id).await? {
        return Ok(None);
    }
    let (item, token) = factory_work_item_tx(tx, input.corp_id, input.work_item_id, true)
        .await?
        .context("checkpoint review work item not found")?;
    ensure_active_factory_control(
        &item,
        token,
        input.actor_id,
        input.claim_token,
        input.expected_version,
        Utc::now(),
    )?;
    let mission_id = item
        .mission_id
        .context("checkpoint review has no mission")?;
    ensure_factory_recovery_authorizer_tx(tx, input.corp_id, mission_id, input.actor_id).await?;
    let row = sqlx::query(
        r#"
        SELECT mission.room_id, run.task_id, run.runner_id, run.agent_id, run.assignment_token,
               run.workspace_run_id, run.workspace_base_commit,
               run.source_repository, run.source_base_ref, run.source_base_commit
        FROM runs run
        JOIN tasks task ON task.id=run.task_id AND task.corp_id=run.corp_id
        JOIN missions mission ON mission.id=task.mission_id AND mission.corp_id=run.corp_id
        JOIN factory_verification_recoveries recovery
          ON recovery.replacement_run_id=run.id AND recovery.corp_id=run.corp_id
         AND recovery.factory_work_item_id=$4
        WHERE run.id=$1 AND run.corp_id=$2 AND mission.id=$3
        FOR UPDATE OF mission, task, run
        "#,
    )
    .bind(input.source_run_id)
    .bind(input.corp_id)
    .bind(mission_id)
    .bind(input.work_item_id)
    .fetch_optional(&mut **tx)
    .await?
    .context("checkpoint review does not belong to this work item")?;
    if !review_ready_tx(tx, input.corp_id, input.source_run_id).await? {
        return Err(anyhow!("checkpoint review authority changed"));
    }
    let checkpoint = source_workspace_checkpoint_tx(tx, input.corp_id, input.source_run_id).await?;
    if checkpoint.expected_head_commit.as_deref() != Some(expected_head) {
        return Err(anyhow!("checkpoint review export head mismatch"));
    }
    let other_active: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM runs WHERE corp_id=$1 AND workspace_run_id=$2
         AND id<>$3 AND status IN ('provisioning','starting','running','waiting_for_input',
                                  'waiting_for_approval','verifying'))",
    )
    .bind(input.corp_id)
    .bind(row.get::<Uuid, _>("workspace_run_id"))
    .bind(input.source_run_id)
    .fetch_one(&mut **tx)
    .await?;
    if other_active {
        return Err(anyhow!(
            "checkpoint review has another active workspace assignment"
        ));
    }
    let lineage = workspace_lineage_tx(tx, input.corp_id, row.get("workspace_run_id")).await?;
    if latest_workspace_source_id(&lineage)? != input.source_run_id
        || lineage.iter().any(|candidate| {
            candidate.get::<Uuid, _>("id") != input.source_run_id
                && matches!(
                    candidate.get::<String, _>("status").as_str(),
                    "provisioning"
                        | "starting"
                        | "running"
                        | "waiting_for_input"
                        | "waiting_for_approval"
                        | "verifying"
                )
                || candidate
                    .get::<Option<String>, _>("workspace_disposition")
                    .as_deref()
                    == Some("quarantined")
        })
    {
        return Err(anyhow!(
            "checkpoint review has a newer, active or quarantined lineage"
        ));
    }
    let payload = json!({
        "corp_id":input.corp_id, "room_id":row.get::<Uuid,_>("room_id"), "mission_id":mission_id,
        "task_id":row.get::<Uuid,_>("task_id"), "run_id":input.source_run_id,
        "workspace_run_id":row.get::<Uuid,_>("workspace_run_id"),
        "agent_id":row.get::<Uuid,_>("agent_id"), "assignment_token":row.get::<Uuid,_>("assignment_token"),
        "source_repository":row.get::<String,_>("source_repository"),
        "source_base_ref":row.get::<String,_>("source_base_ref"),
        "source_base_commit":row.get::<String,_>("source_base_commit"),
        "workspace_base_commit":row.get::<String,_>("workspace_base_commit"),
        "expected_head_commit":expected_head, "review_re_attestation":true,
        "authorized_by":input.actor_id, "factory_work_item_id":input.work_item_id,
    });
    let existing = sqlx::query(
        "SELECT id, payload FROM runner_commands WHERE corp_id=$1 AND idempotency_key=$2",
    )
    .bind(input.corp_id)
    .bind(command_key)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(existing) = existing {
        if existing.get::<Value, _>("payload") != payload {
            return Err(anyhow!(
                "checkpoint re-attestation key was reused with different authority"
            ));
        }
        return Ok(Some(FactoryWorkspaceCheckpointOutcome {
            work_item: item,
            claim_token: Some(input.claim_token),
            source_run_id: input.source_run_id,
            runner_id: row.get("runner_id"),
            command_id: Some(existing.get("id")),
            workspace_fingerprint: checkpoint.fingerprint,
            event: None,
            replayed: true,
        }));
    }
    if factory_operation_tx(tx, input.corp_id, operation_key)
        .await?
        .is_some()
    {
        return Err(anyhow!(
            "checkpoint operation exists without its native command"
        ));
    }
    if checkpoint.fingerprint.is_some() {
        checkpoint.ensure_preserved()?;
        return Ok(Some(FactoryWorkspaceCheckpointOutcome {
            work_item: item,
            claim_token: Some(input.claim_token),
            source_run_id: input.source_run_id,
            runner_id: row.get("runner_id"),
            command_id: None,
            workspace_fingerprint: checkpoint.fingerprint,
            event: None,
            replayed: true,
        }));
    }
    let command_id = Uuid::new_v4();
    sqlx::query("INSERT INTO runner_commands(id,corp_id,runner_id,run_id,command_kind,payload,idempotency_key)
                VALUES($1,$2,$3,$4,'factory_workspace_checkpoint',$5,$6)")
        .bind(command_id).bind(input.corp_id).bind(row.get::<String,_>("runner_id"))
        .bind(input.source_run_id).bind(&payload).bind(command_key).execute(&mut **tx).await?;
    record_factory_operation_tx(
        tx,
        NewFactoryOperation {
            corp_id: input.corp_id,
            idempotency_key: operation_key,
            work_item_id: item.id,
            actor_id: input.actor_id,
            operation: "checkpoint_workspace",
            resulting_version: item.version,
            claim_token: Some(input.claim_token),
            request: operation_request,
        },
    )
    .await?;
    let event = append_event_tx(
        tx,
        NewEvent {
            room_id: Some(row.get("room_id")),
            aggregate_version: item.version,
            correlation_id: Some(mission_id),
            causation_id: Some(input.source_run_id),
            ..NewEvent::new(
                input.corp_id,
                Some(input.actor_id),
                "factory.workspace_checkpoint_requested",
                "factory_work_item",
                item.id,
                format!("factory:{}:review-checkpoint:{command_id}", item.id),
                json!({"source_run_id":input.source_run_id,"command_id":command_id,
                   "expected_head_commit":expected_head,"review_re_attestation":true}),
            )
        },
    )
    .await?;
    Ok(Some(FactoryWorkspaceCheckpointOutcome {
        work_item: item,
        claim_token: Some(input.claim_token),
        source_run_id: input.source_run_id,
        runner_id: row.get("runner_id"),
        command_id: Some(command_id),
        workspace_fingerprint: None,
        event,
        replayed: false,
    }))
}

impl PgStore {
    pub async fn checkpoint_review_command_authorized(
        &self,
        command: &PendingRunnerCommand,
    ) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let stored: Option<Value> = sqlx::query_scalar(
            "SELECT command.payload FROM runner_commands command
             JOIN runs run ON run.id=command.run_id AND run.corp_id=command.corp_id
               AND run.runner_id=command.runner_id
             JOIN tasks task ON task.id=run.task_id AND task.corp_id=run.corp_id
             JOIN missions mission ON mission.id=task.mission_id AND mission.corp_id=run.corp_id
             JOIN factory_verification_recoveries recovery
               ON recovery.replacement_run_id=run.id AND recovery.corp_id=run.corp_id
              AND recovery.task_id=task.id AND recovery.mission_id=mission.id
             WHERE command.id=$1 AND command.corp_id=$2 AND command.run_id=$3 AND command.runner_id=$4
              AND command.command_kind='factory_workspace_checkpoint' AND command.status='pending'
              AND command.payload->>'review_re_attestation'='true'
              AND command.payload->>'factory_work_item_id'=recovery.factory_work_item_id::text
              AND command.payload->>'task_id'=task.id::text
              AND command.payload->>'mission_id'=mission.id::text
              AND command.payload->>'room_id'=mission.room_id::text
              AND command.payload->>'agent_id'=run.agent_id::text
              AND command.payload->>'assignment_token'=run.assignment_token::text
              AND command.payload->>'workspace_run_id'=run.workspace_run_id::text
              AND command.payload->>'workspace_base_commit'=run.workspace_base_commit
              AND command.payload->>'source_repository'=run.source_repository
              AND command.payload->>'source_base_ref'=run.source_base_ref
              AND command.payload->>'source_base_commit'=run.source_base_commit",
        )
        .bind(command.id)
        .bind(command.corp_id)
        .bind(command.run_id)
        .bind(&command.runner_id)
        .fetch_optional(&mut *tx)
        .await?;
        if stored.as_ref() != Some(&command.payload) {
            return Ok(false);
        }
        let actor = command
            .payload
            .get("authorized_by")
            .and_then(Value::as_str)
            .and_then(|s| Uuid::parse_str(s).ok());
        let mission = command
            .payload
            .get("mission_id")
            .and_then(Value::as_str)
            .and_then(|s| Uuid::parse_str(s).ok());
        let (Some(actor), Some(mission)) = (actor, mission) else {
            return Ok(false);
        };
        if let Err(error) =
            ensure_factory_recovery_authorizer_tx(&mut tx, command.corp_id, mission, actor).await
        {
            // A database outage is not an authorization decision. Keep the
            // durable command retryable rather than retiring it as denied.
            if error.downcast_ref::<sqlx::Error>().is_some() {
                return Err(error);
            }
            return Ok(false);
        }
        if !review_ready_tx(&mut tx, command.corp_id, command.run_id).await? {
            return Ok(false);
        }
        let head = exported_head_tx(&mut tx, command.corp_id, command.run_id).await?;
        Ok(head.as_deref()
            == command
                .payload
                .get("expected_head_commit")
                .and_then(Value::as_str))
    }
}
