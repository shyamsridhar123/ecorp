//! Explicit provider correction after failed verification of a suspended source.
//! This grants no checkpoint-mode or model-budget exemption.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Authority {
    pub schema_version: u32,
    pub checkpoint_recovery_id: Uuid,
    pub checkpoint: budget_checkpoint::Authority,
    pub suspension_incident_id: Uuid,
    pub suspension_input_sha256: String,
    pub contract_revision_id: Uuid,
    pub previous_contract_sha256: String,
    pub replacement_contract_sha256: String,
    pub previous_policy_sha256: String,
    pub replacement_policy_sha256: String,
    pub factory_policy_sha256: String,
    pub workspace_connection_id: Option<Uuid>,
    pub provider_session_id: String,
    pub prefix_run_ids: Vec<Uuid>,
}

fn digest(value: &impl Serialize) -> Result<String> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(value)?)))
}

async fn lock_authorizer_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    actor_id: Uuid,
) -> Result<()> {
    // Read the role after acquiring the lock, not before a potentially slow
    // source/provenance wait. Membership uses the existing locked native check.
    sqlx::query("SELECT id FROM actors WHERE id=$1 AND corp_id=$2 FOR SHARE")
        .bind(actor_id)
        .bind(item.corp_id)
        .fetch_optional(&mut **tx)
        .await?
        .context("source correction author is unavailable")?;
    ensure_factory_recovery_authorizer_tx(
        tx,
        item.corp_id,
        item.mission_id
            .context("source correction has no mission")?,
        actor_id,
    )
    .await
}

/// Prove the immutable prefix, separately from the approved replacement policy.
/// Only dispatch/publication may validate a prefix with a later replacement run.
async fn origin_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    source_run_id: Uuid,
    revision_id: Uuid,
    actor_id: Uuid,
    has_replacement: bool,
) -> Result<Authority> {
    origin_with_publication_tx(
        tx,
        item,
        source_run_id,
        revision_id,
        actor_id,
        has_replacement,
        None,
    )
    .await
}

/// Admission, dispatch and replay always use the strict None path above. Only
/// publication may supply a native receipt after validating the exact suffix.
async fn origin_with_publication_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    source_run_id: Uuid,
    revision_id: Uuid,
    actor_id: Uuid,
    has_replacement: bool,
    publication: Option<&checkpoint_publication::CheckpointPublication>,
) -> Result<Authority> {
    lock_authorizer_tx(tx, item, actor_id).await?;
    let row = sqlx::query(
        r#"
        SELECT checkpoint.id AS checkpoint_id, checkpoint.checkpoint_authority,
               source.task_id, source.workspace_run_id, source.workspace_connection_id,
               revision.previous_contract, revision.replacement_contract,
               revision.previous_verification_policy, revision.replacement_verification_policy,
               task.contract, task.verification_policy
        FROM factory_verification_recoveries checkpoint
        JOIN runs source
          ON source.id=checkpoint.replacement_run_id AND source.corp_id=checkpoint.corp_id
         AND source.task_id=checkpoint.task_id
         AND source.resumed_from_run_id=checkpoint.source_run_id
        JOIN tasks task ON task.id=source.task_id AND task.corp_id=source.corp_id
        JOIN mission_contract_revisions revision
          ON revision.id=$4 AND revision.corp_id=source.corp_id
         AND revision.task_id=task.id AND revision.mission_id=task.mission_id
         AND revision.source_run_id=source.id AND revision.revised_by=$5
         AND revision.next_action='resume' AND revision.version=task.contract_version
        WHERE checkpoint.corp_id=$1 AND checkpoint.factory_work_item_id=$2
          AND checkpoint.mission_id=$6 AND task.mission_id=$6
          AND checkpoint.replacement_run_id=$3
          AND checkpoint.mode='checkpoint_verification' AND checkpoint.status='failed'
          AND source.execution_mode='verification_only' AND source.status='failed'
          AND source.verification_status='failed'
          AND source.workspace_disposition='preserved'
          AND source.provider_session_id IS NULL
        "#,
    )
    .bind(item.corp_id)
    .bind(item.id)
    .bind(source_run_id)
    .bind(revision_id)
    .bind(actor_id)
    .bind(
        item.mission_id
            .context("source correction has no mission")?,
    )
    .fetch_optional(&mut **tx)
    .await?
    .context("source correction requires the exact failed checkpoint and current revision")?;
    let previous_contract: TaskContract = serde_json::from_value(row.get("previous_contract"))?;
    let replacement_contract: TaskContract =
        serde_json::from_value(row.get("replacement_contract"))?;
    let previous_policy: VerificationPolicy =
        serde_json::from_value(row.get("previous_verification_policy"))?;
    let replacement_policy: VerificationPolicy =
        serde_json::from_value(row.get("replacement_verification_policy"))?;
    if row.get::<Value, _>("contract") != serde_json::to_value(&replacement_contract)?
        || row.get::<Value, _>("verification_policy") != serde_json::to_value(&replacement_policy)?
    {
        return Err(anyhow!("source correction revision was superseded"));
    }
    ensure_factory_recovery_verification_policy_not_weakened(
        &previous_policy,
        &replacement_policy,
    )?;
    ensure_factory_recovery_policy(item, &replacement_contract, &replacement_policy)?;
    let proof = budget_checkpoint::source_authority_with_contract_tx(
        tx,
        item.corp_id,
        item.id,
        source_run_id,
        Some((&previous_contract, &previous_policy)),
    )
    .await?;
    if row.get::<Option<Value>, _>("checkpoint_authority") != Some(serde_json::to_value(&proof)?) {
        return Err(anyhow!("source correction checkpoint authority changed"));
    }
    let origin = sqlx::query(
        r#"
        SELECT origin.breaker_stage, origin.workspace_connection_id, origin.provider_session_id,
               incident.id AS incident_id, incident.input
        FROM runs origin
        JOIN circuit_breaker_incidents incident
          ON incident.corp_id=origin.corp_id AND incident.run_id=origin.id
         AND incident.stage='suspend'
        WHERE origin.id=$1 AND origin.corp_id=$2
        "#,
    )
    .bind(proof.checkpoint.run_id)
    .bind(item.corp_id)
    .fetch_optional(&mut **tx)
    .await?
    .context("source correction has no native suspension incident")?;
    if origin.get::<String, _>("breaker_stage") != "suspend" {
        return Err(anyhow!("source correction cannot override stop-stage work"));
    }
    let connection: Option<Uuid> = row.get("workspace_connection_id");
    if origin.get::<Option<Uuid>, _>("workspace_connection_id") != connection
        || replacement_contract.workspace_connection_id != connection
        || previous_contract.workspace_connection_id != connection
        || factory_workspace_connection_id(&item.policy).map_err(anyhow::Error::msg)? != connection
    {
        return Err(anyhow!("source correction execution connection changed"));
    }
    if has_replacement {
        budget_checkpoint::validate_lineage_until_tx(
            tx,
            item.corp_id,
            item.id,
            row.get("workspace_run_id"),
            &proof,
            source_run_id,
        )
        .await?;
    } else {
        budget_checkpoint::validate_lineage_tx(
            tx,
            item.corp_id,
            item.id,
            row.get("workspace_run_id"),
            &proof,
        )
        .await?;
    }
    // Only the exact reviewed checkpoint origin may pass the breaker predicate
    // during publication. Quarantine is an independent veto, including there.
    // With None, IS DISTINCT FROM NULL preserves the original strict predicate.
    let invalid_lineage: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM runs run
            WHERE run.corp_id=$1 AND run.workspace_run_id=$2
              AND (run.workspace_disposition='quarantined'
                   OR ((run.breaker_stage='stop'
                        OR (run.breaker_stage='suspend' AND run.id<>$3))
                       AND run.id IS DISTINCT FROM $4))
        )
        "#,
    )
    .bind(item.corp_id)
    .bind(row.get::<Uuid, _>("workspace_run_id"))
    .bind(proof.checkpoint.run_id)
    .bind(publication.map(checkpoint_publication::CheckpointPublication::origin_run_id))
    .fetch_one(&mut **tx)
    .await?;
    if invalid_lineage {
        return Err(anyhow!(
            "source correction lineage has another stop, suspension or quarantine"
        ));
    }
    let prefix_run_ids: Vec<Uuid> = sqlx::query_scalar(
        r#"
        SELECT run.id FROM runs run
        JOIN runs boundary ON boundary.id=$3 AND boundary.corp_id=run.corp_id
        WHERE run.corp_id=$1 AND run.workspace_run_id=$2
          AND (run.created_at,run.id)<=(boundary.created_at,boundary.id)
        ORDER BY run.created_at,run.id LIMIT 65
        "#,
    )
    .bind(item.corp_id)
    .bind(row.get::<Uuid, _>("workspace_run_id"))
    .bind(source_run_id)
    .fetch_all(&mut **tx)
    .await?;
    if prefix_run_ids.is_empty()
        || prefix_run_ids.len() > 64
        || prefix_run_ids.last() != Some(&source_run_id)
    {
        return Err(anyhow!("source correction checkpoint prefix is invalid"));
    }
    Ok(Authority {
        schema_version: 1,
        checkpoint_recovery_id: row.get("checkpoint_id"),
        checkpoint: proof,
        suspension_incident_id: origin.get("incident_id"),
        suspension_input_sha256: digest(&origin.get::<Value, _>("input"))?,
        contract_revision_id: revision_id,
        previous_contract_sha256: digest(&previous_contract)?,
        replacement_contract_sha256: digest(&replacement_contract)?,
        previous_policy_sha256: digest(&previous_policy)?,
        replacement_policy_sha256: digest(&replacement_policy)?,
        factory_policy_sha256: digest(&item.policy)?,
        workspace_connection_id: connection,
        provider_session_id: origin
            .get::<Option<String>, _>("provider_session_id")
            .filter(|session| !session.is_empty())
            .context("source correction origin has no provider session")?,
        prefix_run_ids,
    })
}

pub(super) async fn admit_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    source_run_id: Uuid,
    contract_revision_id: Option<Uuid>,
    actor_id: Uuid,
) -> Result<Authority> {
    origin_tx(
        tx,
        item,
        source_run_id,
        contract_revision_id.context("source correction requires a contract revision")?,
        actor_id,
        false,
    )
    .await
}

pub(super) async fn context_eligible_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    source_run_id: Uuid,
    actor_id: Uuid,
    revised: Option<&Authority>,
) -> Result<bool> {
    // A read-only affordance, never dispatch authority. The mutation repeats the
    // full proof after an explicit versioned contract revision.
    if !matches!(
        item.state,
        FactoryWorkItemState::VerificationFailed | FactoryWorkItemState::Blocked
    ) {
        return Ok(false);
    }
    let eligible: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM runs source
            JOIN tasks task ON task.id=source.task_id AND task.corp_id=source.corp_id
            JOIN missions mission ON mission.id=task.mission_id AND mission.corp_id=task.corp_id
            JOIN factory_verification_recoveries recovery
              ON recovery.replacement_run_id=source.id AND recovery.corp_id=source.corp_id
             AND recovery.task_id=task.id AND recovery.mission_id=mission.id
             AND recovery.factory_work_item_id=$3
            WHERE source.id=$2 AND source.corp_id=$1
              AND recovery.mode='checkpoint_verification' AND recovery.status='failed'
              AND source.execution_mode='verification_only' AND source.status='failed'
              AND source.verification_status='failed' AND task.attempt_count<task.max_attempts
        )
        "#,
    )
    .bind(item.corp_id)
    .bind(source_run_id)
    .bind(item.id)
    .fetch_one(&mut **tx)
    .await?;
    if !eligible {
        return Ok(false);
    }
    let proof = if let Some(revised) = revised {
        revised.checkpoint.clone()
    } else {
        match budget_checkpoint::source_authority_tx(tx, item.corp_id, item.id, source_run_id).await
        {
            Ok(proof) => proof,
            Err(error) => return deny_or_retry(error),
        }
    };
    let origin_ready: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM runs origin
            JOIN runs source ON source.id=$3 AND source.corp_id=origin.corp_id
            JOIN tasks task ON task.id=source.task_id AND task.corp_id=source.corp_id
            WHERE origin.corp_id=$1 AND origin.id=$2 AND origin.breaker_stage='suspend'
              AND NULLIF(origin.provider_session_id,'') IS NOT NULL
              AND origin.workspace_connection_id IS NOT DISTINCT FROM source.workspace_connection_id
              AND task.contract->>'workspace_connection_id'
                  IS NOT DISTINCT FROM source.workspace_connection_id::text
              AND $4::text IS NOT DISTINCT FROM source.workspace_connection_id::text
              AND (source.workspace_connection_id IS NULL OR EXISTS (
                  SELECT 1 FROM workspace_connections connection
                  JOIN runner_nodes runner ON runner.id=connection.runner_id
                                          AND runner.corp_id=connection.corp_id
                  WHERE connection.id=source.workspace_connection_id
                    AND connection.corp_id=source.corp_id AND connection.status='ready'
                    AND connection.runner_id=source.runner_id AND runner.status='connected'
              ))
        )
        "#,
    )
    .bind(item.corp_id)
    .bind(proof.checkpoint.run_id)
    .bind(source_run_id)
    .bind(
        factory_workspace_connection_id(&item.policy)
            .map_err(anyhow::Error::msg)?
            .map(|id| id.to_string()),
    )
    .fetch_one(&mut **tx)
    .await?;
    if !origin_ready {
        return Ok(false);
    }
    if let Err(error) = budget_checkpoint::validate_lineage_tx(
        tx,
        item.corp_id,
        item.id,
        proof.checkpoint.workspace_run_id,
        &proof,
    )
    .await
    {
        return deny_or_retry(error);
    }
    if let Err(error) = ensure_factory_recovery_authorizer_tx(
        tx,
        item.corp_id,
        item.mission_id
            .context("correction context has no mission")?,
        actor_id,
    )
    .await
    {
        return deny_or_retry(error);
    }
    Ok(true)
}

pub(super) async fn revised_context_authority_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    source_run_id: Uuid,
) -> Result<Option<Authority>> {
    let revision = sqlx::query(
        r#"
        SELECT revision.id,revision.revised_by
        FROM mission_contract_revisions revision
        JOIN tasks task ON task.id=revision.task_id AND task.corp_id=revision.corp_id
                       AND task.contract_version=revision.version
        JOIN runs source ON source.id=$3 AND source.task_id=task.id AND source.corp_id=task.corp_id
        WHERE revision.corp_id=$1 AND revision.mission_id=$2
          AND revision.source_run_id=source.id AND revision.next_action='resume'
        "#,
    )
    .bind(item.corp_id)
    .bind(item.mission_id)
    .bind(source_run_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(revision) = revision else {
        return Ok(None);
    };
    let active = sqlx::query(
        r#"
        SELECT recovery.source_correction_authority
        FROM factory_verification_recoveries recovery
        JOIN runs replacement ON replacement.id=recovery.replacement_run_id
                             AND replacement.corp_id=recovery.corp_id
                             AND replacement.resumed_from_run_id=recovery.source_run_id
                             AND replacement.task_id=recovery.task_id
        WHERE recovery.corp_id=$1 AND recovery.factory_work_item_id=$2
          AND recovery.source_run_id=$3 AND recovery.contract_revision_id=$4
          AND recovery.mode='source_correction' AND recovery.status IN ('authorized','running')
          AND replacement.execution_mode='provider'
          AND replacement.status IN ('provisioning','starting','running','waiting_for_input',
                                     'waiting_for_approval','verifying')
        "#,
    )
    .bind(item.corp_id)
    .bind(item.id)
    .bind(source_run_id)
    .bind(revision.get::<Uuid, _>("id"))
    .fetch_optional(&mut **tx)
    .await?;
    let authority = origin_tx(
        tx,
        item,
        source_run_id,
        revision.get("id"),
        revision.get("revised_by"),
        active.is_some(),
    )
    .await?;
    if let Some(active) = active {
        let expected: Authority = serde_json::from_value(
            active
                .get::<Option<Value>, _>("source_correction_authority")
                .context("active correction context has no origin provenance")?,
        )?;
        if serde_json::to_value(expected)? != serde_json::to_value(&authority)? {
            return Err(anyhow!(
                "active correction context origin provenance changed"
            ));
        }
    }
    Ok(Some(authority))
}

fn deny_or_retry(error: anyhow::Error) -> Result<bool> {
    if error.downcast_ref::<sqlx::Error>().is_some() {
        Err(error)
    } else {
        Ok(false)
    }
}

impl PgStore {
    /// Read-only revalidation before and after secret resolution, including
    /// reconnect. False grants no dispatch authority and does not settle work.
    pub async fn source_correction_command_authorized(
        &self,
        command: &PendingRunnerCommand,
    ) -> Result<bool> {
        if command.command_kind != "factory_verification_recovery"
            || command.payload.get("mode").and_then(Value::as_str) != Some("source_correction")
        {
            return Ok(false);
        }
        let mut tx = self.pool.begin().await?;
        // Use the native mutation gates before waiting on any row, then
        // re-read after all waits. Do not accept a stale pre-wait projection.
        let scope = sqlx::query(
            r#"
            SELECT recovery.factory_work_item_id,run.workspace_run_id,mission.requested_by
            FROM runner_commands command
            JOIN runs run ON run.id=command.run_id AND run.corp_id=command.corp_id
            JOIN factory_verification_recoveries recovery
              ON recovery.replacement_run_id=run.id AND recovery.corp_id=run.corp_id
            JOIN missions mission ON mission.id=recovery.mission_id AND mission.corp_id=run.corp_id
            WHERE command.id=$1 AND command.corp_id=$2 AND command.run_id=$3
              AND command.runner_id=$4 AND recovery.mode='source_correction'
            "#,
        )
        .bind(command.id)
        .bind(command.corp_id)
        .bind(command.run_id)
        .bind(&command.runner_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(scope) = scope else {
            return Ok(false);
        };
        let item_id: Uuid = scope.get("factory_work_item_id");
        let workspace_run_id: Uuid = scope.get("workspace_run_id");
        let mut keys = budget_scope_lock_keys(command.corp_id, scope.get("requested_by"));
        keys.extend([
            format!("factory:item:{}:{item_id}", command.corp_id),
            format!("publication:factory:{}:{item_id}", command.corp_id),
            format!("resume:workspace:{}:{workspace_run_id}", command.corp_id),
        ]);
        lock_factory_keys_tx(&mut tx, &keys).await?;
        let dispatch_query = r#"
            SELECT recovery.factory_work_item_id, recovery.source_run_id,
                   recovery.contract_revision_id, recovery.authorized_by,
                   recovery.source_correction_authority, run.breaker_stage,
                   run.provider_session_id, run.workspace_run_id
            FROM runner_commands stored
            JOIN runs run ON run.id=stored.run_id AND run.corp_id=stored.corp_id
                         AND run.runner_id=stored.runner_id
            JOIN factory_verification_recoveries recovery
              ON recovery.replacement_run_id=run.id AND recovery.corp_id=run.corp_id
             AND recovery.task_id=run.task_id AND recovery.source_run_id=run.resumed_from_run_id
            JOIN tasks task ON task.id=run.task_id AND task.corp_id=run.corp_id
                           AND task.mission_id=recovery.mission_id
            JOIN missions mission ON mission.id=task.mission_id AND mission.corp_id=run.corp_id
            JOIN factory_work_items item ON item.id=recovery.factory_work_item_id
                                        AND item.corp_id=run.corp_id AND item.mission_id=mission.id
            JOIN actors author ON author.id=recovery.authorized_by AND author.corp_id=run.corp_id
                              AND author.kind='human' AND author.role IN ('owner','admin','manager')
            JOIN room_memberships member ON member.room_id=mission.room_id AND member.actor_id=author.id
            JOIN runs source ON source.id=recovery.source_run_id AND source.corp_id=run.corp_id
                            AND source.task_id=run.task_id AND source.agent_id=run.agent_id
                            AND source.runner_id=run.runner_id AND source.workspace_run_id=run.workspace_run_id
            JOIN agents agent ON agent.id=run.agent_id AND agent.corp_id=run.corp_id
                             AND agent.current_run_id=run.id
            WHERE stored.id=$1 AND stored.corp_id=$2 AND stored.run_id=$3 AND stored.runner_id=$4
              AND stored.command_kind='factory_verification_recovery'
              AND stored.status='pending' AND stored.payload=$5::jsonb
              AND recovery.mode='source_correction' AND recovery.status IN ('authorized','running')
              AND run.execution_mode='provider' AND run.provider_session_id IS NOT NULL
              AND run.budget_tokens_limit>0 AND run.budget_cost_microusd_limit>0
              AND run.status IN ('provisioning','starting','running','waiting_for_input',
                                 'waiting_for_approval','verifying')
              AND run.workspace_disposition IS DISTINCT FROM 'quarantined'
              AND task.status IN ('claimed','running') AND mission.status='running' AND item.state='running'
              AND task.attempt_count<=task.max_attempts
              AND source.status IN ('failed','cancelled','lost') AND source.workspace_disposition='preserved'
              AND NULLIF(source.workspace_path,'') IS NOT NULL
              AND stored.payload->>'corp_id'=run.corp_id::text
              AND stored.payload->>'run_id'=run.id::text
              AND stored.payload->>'task_id'=task.id::text
              AND stored.payload->>'mission_id'=mission.id::text
              AND stored.payload->>'room_id'=mission.room_id::text
              AND stored.payload->>'agent_id'=run.agent_id::text
              AND stored.payload->>'assignment_token'=run.assignment_token::text
              AND stored.payload->>'workspace_run_id'=run.workspace_run_id::text
              AND stored.payload->>'source_run_id'=source.id::text
              AND stored.payload->>'provider_session_id'=run.provider_session_id
              AND stored.payload->>'adapter'=task.required_adapter
              AND agent.adapter=task.required_adapter
              AND stored.payload->>'model' IS NOT DISTINCT FROM run.model
              AND stored.payload->>'reasoning_effort' IS NOT DISTINCT FROM run.reasoning_effort
              AND stored.payload->>'source_repository' IS NOT DISTINCT FROM run.source_repository
              AND stored.payload->>'source_base_ref' IS NOT DISTINCT FROM run.source_base_ref
              AND stored.payload->>'source_base_commit' IS NOT DISTINCT FROM run.source_base_commit
              AND stored.payload->>'workspace_connection_id' IS NOT DISTINCT FROM run.workspace_connection_id::text
              AND source.source_repository IS NOT DISTINCT FROM run.source_repository
              AND source.source_base_ref IS NOT DISTINCT FROM run.source_base_ref
              AND source.source_base_commit IS NOT DISTINCT FROM run.source_base_commit
              AND source.workspace_connection_id IS NOT DISTINCT FROM run.workspace_connection_id
              AND task.contract->>'workspace_connection_id' IS NOT DISTINCT FROM run.workspace_connection_id::text
              AND task.contract->>'source_repository' IS NOT DISTINCT FROM run.source_repository
              AND task.contract->>'source_base_ref' IS NOT DISTINCT FROM run.source_base_ref
              AND task.contract->>'source_base_commit' IS NOT DISTINCT FROM run.source_base_commit
              AND stored.payload->>'workspace_base_commit'=source.workspace_base_commit
              AND stored.payload->>'expected_workspace_fingerprint'=source.workspace_fingerprint
              AND stored.payload->>'expected_workspace_fingerprint'=recovery.request->>'expected_workspace_fingerprint'
              AND stored.payload->>'expected_head_commit' IS NOT DISTINCT FROM recovery.request->>'expected_head_commit'
              AND stored.payload->'verification_policy'=task.verification_policy
              AND task.verification_policy=recovery.replacement_verification_policy
              AND stored.payload->'write_scope'=task.contract->'write_scope'
              AND COALESCE(stored.payload->'deliverable','null'::jsonb)=COALESCE(task.contract->'deliverable','null'::jsonb)
              AND COALESCE(stored.payload->'secret_refs','[]'::jsonb)=COALESCE(task.contract->'secret_refs','[]'::jsonb)
            "#;
        let row = sqlx::query(dispatch_query)
            .bind(command.id)
            .bind(command.corp_id)
            .bind(command.run_id)
            .bind(&command.runner_id)
            .bind(&command.payload)
            .fetch_optional(&mut *tx)
            .await?;
        let Some(row) = row else {
            return Ok(false);
        };
        let Some((item, _)) = factory_work_item_tx(
            &mut tx,
            command.corp_id,
            row.get("factory_work_item_id"),
            false,
        )
        .await?
        else {
            return Ok(false);
        };
        if let Err(error) = lock_authorizer_tx(&mut tx, &item, row.get("authorized_by")).await {
            return deny_or_retry(error);
        }
        if let Err(error) = workspace_connections::validate_run_connection_tx(
            &mut tx,
            command.corp_id,
            command.run_id,
        )
        .await
        {
            return deny_or_retry(error);
        }
        if let Err(error) = ensure_run_not_hard_blocked_tx(
            &mut tx,
            command.corp_id,
            command.run_id,
            row.get::<String, _>("breaker_stage").as_str(),
            "source-correction dispatch",
        )
        .await
        {
            return deny_or_retry(error);
        }
        let latest: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM runs WHERE corp_id=$1 AND workspace_run_id=$2
             ORDER BY created_at DESC,id DESC LIMIT 1",
        )
        .bind(command.corp_id)
        .bind(row.get::<Uuid, _>("workspace_run_id"))
        .fetch_optional(&mut *tx)
        .await?;
        if latest != Some(command.run_id) {
            return Ok(false);
        }
        if let Some(encoded) = row.get::<Option<Value>, _>("source_correction_authority") {
            let expected: Authority = match serde_json::from_value(encoded) {
                Ok(expected) => expected,
                Err(_) => return Ok(false),
            };
            let current = match origin_tx(
                &mut tx,
                &item,
                row.get("source_run_id"),
                row.get::<Option<Uuid>, _>("contract_revision_id")
                    .context("correction revision is absent")?,
                row.get("authorized_by"),
                true,
            )
            .await
            {
                Ok(current) => current,
                Err(error) => return deny_or_retry(error),
            };
            if serde_json::to_value(&expected)? != serde_json::to_value(&current)?
                || row.get::<Option<String>, _>("provider_session_id").as_ref()
                    != Some(&current.provider_session_id)
            {
                return Ok(false);
            }
        } else {
            let denied: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM runs WHERE corp_id=$1 AND workspace_run_id=$2
                 AND (breaker_stage IN ('suspend','stop') OR workspace_disposition='quarantined'))",
            )
            .bind(command.corp_id)
            .bind(row.get::<Uuid, _>("workspace_run_id"))
            .fetch_one(&mut *tx)
            .await?;
            if denied {
                return Ok(false);
            }
        }
        let current = sqlx::query(dispatch_query)
            .bind(command.id)
            .bind(command.corp_id)
            .bind(command.run_id)
            .bind(&command.runner_id)
            .bind(&command.payload)
            .fetch_optional(&mut *tx)
            .await?;
        let Some(current) = current else {
            return Ok(false);
        };
        if current.get::<Option<Value>, _>("source_correction_authority")
            != row.get::<Option<Value>, _>("source_correction_authority")
        {
            return Ok(false);
        }
        tx.commit().await?;
        Ok(true)
    }
}

pub(super) async fn replay_authority_tx(
    tx: &mut Transaction<'_, Postgres>,
    recovery: &FactoryVerificationRecovery,
) -> Result<()> {
    if recovery.mode != FactoryVerificationRecoveryMode::SourceCorrection {
        return Ok(());
    }
    let row = sqlx::query(
        r#"
        SELECT recovery.source_correction_authority,
               EXISTS (
                 SELECT 1 FROM factory_verification_recoveries checkpoint
                 WHERE checkpoint.corp_id=recovery.corp_id
                   AND checkpoint.factory_work_item_id=recovery.factory_work_item_id
                   AND checkpoint.replacement_run_id=recovery.source_run_id
                   AND checkpoint.mode='checkpoint_verification'
               ) AS requires_origin
        FROM factory_verification_recoveries recovery
        WHERE recovery.id=$1 AND recovery.corp_id=$2
        "#,
    )
    .bind(recovery.id)
    .bind(recovery.corp_id)
    .fetch_one(&mut **tx)
    .await?;
    let encoded: Option<Value> = row.get("source_correction_authority");
    let Some(encoded) = encoded else {
        if row.get::<bool, _>("requires_origin") {
            return Err(anyhow!(
                "checkpoint source-correction provenance is missing"
            ));
        }
        return Ok(());
    };
    let expected: Authority = serde_json::from_value(encoded)?;
    let (item, _) =
        factory_work_item_tx(tx, recovery.corp_id, recovery.factory_work_item_id, false)
            .await?
            .context("source-correction work item is missing")?;
    let current = origin_tx(
        tx,
        &item,
        recovery.source_run_id,
        recovery
            .contract_revision_id
            .context("source-correction revision is absent")?,
        recovery.authorized_by,
        true,
    )
    .await?;
    if serde_json::to_value(expected)? != serde_json::to_value(current)? {
        return Err(anyhow!("source-correction replay authority changed"));
    }
    Ok(())
}

pub(super) async fn publication_authority_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    selected_run_id: Uuid,
) -> Result<()> {
    let row = sqlx::query(
        r#"
        WITH RECURSIVE lineage AS (
            SELECT run.id,run.resumed_from_run_id,run.task_id,run.agent_id,run.runner_id,
                   run.workspace_run_id,0 AS depth,ARRAY[run.id] AS visited
            FROM runs run WHERE run.corp_id=$1 AND run.id=$2
            UNION ALL
            SELECT parent.id,parent.resumed_from_run_id,parent.task_id,parent.agent_id,
                   parent.runner_id,parent.workspace_run_id,lineage.depth+1,lineage.visited||parent.id
            FROM runs parent JOIN lineage ON parent.id=lineage.resumed_from_run_id
            WHERE parent.corp_id=$1 AND parent.task_id=lineage.task_id
              AND parent.agent_id=lineage.agent_id AND parent.runner_id=lineage.runner_id
              AND parent.workspace_run_id=lineage.workspace_run_id
              AND NOT parent.id=ANY(lineage.visited) AND lineage.depth<64
        )
        SELECT recovery.source_run_id,recovery.contract_revision_id,recovery.authorized_by,
               recovery.source_correction_authority
        FROM lineage JOIN factory_verification_recoveries recovery
          ON recovery.replacement_run_id=lineage.id AND recovery.corp_id=$1
         AND recovery.factory_work_item_id=$3 AND recovery.mode='source_correction'
        WHERE recovery.source_correction_authority IS NOT NULL OR EXISTS (
            SELECT 1 FROM factory_verification_recoveries checkpoint
            WHERE checkpoint.corp_id=recovery.corp_id
              AND checkpoint.factory_work_item_id=recovery.factory_work_item_id
              AND checkpoint.replacement_run_id=recovery.source_run_id
              AND checkpoint.mode='checkpoint_verification'
        )
        ORDER BY lineage.depth LIMIT 1
        "#,
    ).bind(item.corp_id).bind(selected_run_id).bind(item.id).fetch_optional(&mut **tx).await?;
    let Some(row) = row else {
        return Ok(());
    };
    let expected: Authority = serde_json::from_value(
        row.get::<Option<Value>, _>("source_correction_authority")
            .context("source-correction publication provenance is missing")?,
    )
    .context("invalid source-correction publication provenance")?;
    let current = origin_tx(
        tx,
        item,
        row.get("source_run_id"),
        row.get::<Option<Uuid>, _>("contract_revision_id")
            .context("publication correction revision is absent")?,
        row.get("authorized_by"),
        true,
    )
    .await?;
    if serde_json::to_value(expected)? != serde_json::to_value(current)? {
        return Err(anyhow!("source-correction publication authority changed"));
    }
    Ok(())
}

/// Unlike the mission-wide edge set or the filtered correction CTE, this walk
/// fails on a broken/cross-scope parent rather than treating a truncated chain
/// as absence of a correction. Publication has already locked the run rows.
async fn checkpoint_publication_lineage_tx(
    tx: &mut Transaction<'_, Postgres>,
    selected_run_id: Uuid,
    authority: &budget_checkpoint::Authority,
    connection_id: Option<Uuid>,
) -> Result<Vec<Uuid>> {
    let proof = &authority.checkpoint;
    let mut next = Some(selected_run_id);
    let mut seen = HashSet::new();
    let mut lineage = Vec::new();
    while let Some(run_id) = next {
        // Native admission bounds the source prefix at 64; the completed
        // verifier itself is the one additional node.
        if seen.len() == 65 || !seen.insert(run_id) {
            return Err(anyhow!(
                "publication correction lineage is cyclic or exceeds its bound"
            ));
        }
        let row = sqlx::query(
            "SELECT run.task_id,task.mission_id,run.agent_id,run.runner_id,
                    run.workspace_run_id,run.workspace_connection_id,run.resumed_from_run_id,
                    run.source_repository,run.source_base_ref,run.source_base_commit
             FROM runs run JOIN tasks task ON task.id=run.task_id AND task.corp_id=run.corp_id
             WHERE run.corp_id=$1 AND run.id=$2",
        )
        .bind(proof.corp_id)
        .bind(run_id)
        .fetch_optional(&mut **tx)
        .await?
        .context("publication correction lineage has a missing or foreign parent")?;
        if row.get::<Uuid, _>("task_id") != proof.task_id
            || row.get::<Uuid, _>("mission_id") != proof.mission_id
            || row.get::<Uuid, _>("agent_id") != proof.agent_id
            || row.get::<String, _>("runner_id") != proof.runner_id
            || row.get::<Uuid, _>("workspace_run_id") != proof.workspace_run_id
            || row.get::<Option<Uuid>, _>("workspace_connection_id") != connection_id
            || row.get::<Option<String>, _>("source_repository").as_ref()
                != Some(&proof.source_repository)
            || row.get::<Option<String>, _>("source_base_ref").as_ref()
                != Some(&proof.source_base_ref)
            || row.get::<Option<String>, _>("source_base_commit").as_ref()
                != Some(&proof.source_base_commit)
        {
            return Err(anyhow!(
                "publication correction lineage changed assignment or source"
            ));
        }
        lineage.push(run_id);
        next = row.get("resumed_from_run_id");
    }
    if lineage.last() != Some(&proof.workspace_run_id) || !seen.contains(&proof.run_id) {
        return Err(anyhow!(
            "publication checkpoint is outside the exact workspace ancestry"
        ));
    }
    Ok(lineage)
}

/// A publication-only reconstruction, consuming the native checkpoint receipt
/// from this same locked transaction. None retains the strict product path.
pub(super) async fn publication_authority_with_checkpoint_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    selected_run_id: Uuid,
    checkpoint: Option<&checkpoint_publication::CheckpointPublication>,
) -> Result<()> {
    let Some(checkpoint) = checkpoint else {
        return publication_authority_tx(tx, item, selected_run_id).await;
    };
    let authority = checkpoint.authority_for(item, selected_run_id)?;
    let proof = &authority.checkpoint;
    let origin = sqlx::query(
        "SELECT run.resumed_from_run_id,run.workspace_connection_id,run.provider_session_id,
                parent.execution_mode AS parent_mode,
                EXISTS (
                    SELECT 1 FROM runs historical
                    WHERE historical.corp_id=run.corp_id
                      AND historical.workspace_run_id=run.workspace_run_id
                      AND historical.id<>run.id AND historical.breaker_stage='suspend'
                ) AS historical_suspension,
                EXISTS (
                    SELECT 1 FROM factory_verification_recoveries prior
                    WHERE prior.corp_id=run.corp_id AND prior.replacement_run_id=parent.id
                      AND prior.mode='checkpoint_verification'
                ) OR EXISTS (
                    SELECT 1 FROM runner_commands predecessor_command
                    WHERE predecessor_command.corp_id=run.corp_id
                      AND predecessor_command.run_id=parent.id
                      AND predecessor_command.command_kind='factory_verification_recovery'
                      AND predecessor_command.payload->>'mode'='checkpoint_verification'
                ) AS checkpoint_predecessor,
                EXISTS (
                    SELECT 1 FROM factory_verification_recoveries prior
                    WHERE prior.corp_id=run.corp_id AND prior.replacement_run_id=parent.id
                      AND prior.mode='verifier_only'
                ) OR EXISTS (
                    SELECT 1 FROM runner_commands predecessor_command
                    WHERE predecessor_command.corp_id=run.corp_id
                      AND predecessor_command.run_id=parent.id
                      AND predecessor_command.command_kind='factory_verification_recovery'
                      AND predecessor_command.payload->>'mode'='verifier_only'
                ) AS ordinary_predecessor_marker,
                predecessor.mode AS predecessor_mode,
                EXISTS (
                    SELECT 1 FROM runner_commands command
                    JOIN factory_verification_recoveries prior
                      ON prior.corp_id=command.corp_id
                     AND prior.replacement_run_id::text=command.payload->>'source_run_id'
                     AND prior.mode='checkpoint_verification'
                    WHERE command.corp_id=run.corp_id AND command.run_id=run.id
                      AND command.command_kind='factory_verification_recovery'
                      AND command.payload->>'mode'='source_correction'
                ) AS checkpoint_correction_command
         FROM runs run LEFT JOIN runs parent
           ON parent.id=run.resumed_from_run_id AND parent.corp_id=run.corp_id
         LEFT JOIN LATERAL (
             SELECT prior.mode
             FROM factory_verification_recoveries prior
             JOIN runner_commands predecessor_command
               ON predecessor_command.corp_id=prior.corp_id
              AND predecessor_command.run_id=prior.replacement_run_id
              AND predecessor_command.runner_id=parent.runner_id
              AND predecessor_command.command_kind='factory_verification_recovery'
              AND predecessor_command.idempotency_key='factory-verification-recovery:'||prior.id::text
             WHERE prior.corp_id=run.corp_id AND prior.factory_work_item_id=$3
               AND prior.mission_id=$4 AND prior.task_id=parent.task_id
               AND prior.replacement_run_id=parent.id
               AND prior.source_run_id=parent.resumed_from_run_id
               AND prior.mode IN ('verifier_only','checkpoint_verification') AND prior.status='failed'
               AND prior.source_correction_authority IS NULL
               AND (prior.mode='checkpoint_verification')=(prior.checkpoint_authority IS NOT NULL)
               AND parent.execution_mode='verification_only'
               AND parent.status='failed' AND parent.verification_status='failed'
               AND prior.request->>'mode'=prior.mode
               AND prior.request->>'work_item_id'=prior.factory_work_item_id::text
               AND prior.request->>'source_run_id'=prior.source_run_id::text
               AND prior.request->>'contract_revision_id' IS NOT DISTINCT FROM prior.contract_revision_id::text
               AND predecessor_command.payload->>'mode'=prior.mode
               AND predecessor_command.payload->>'corp_id'=parent.corp_id::text
               AND predecessor_command.payload->>'mission_id'=prior.mission_id::text
               AND predecessor_command.payload->>'task_id'=parent.task_id::text
               AND predecessor_command.payload->>'run_id'=parent.id::text
               AND predecessor_command.payload->>'agent_id'=parent.agent_id::text
               AND predecessor_command.payload->>'assignment_token'=parent.assignment_token::text
               AND predecessor_command.payload->>'source_run_id'=prior.source_run_id::text
               AND predecessor_command.payload->>'workspace_run_id'=parent.workspace_run_id::text
               AND predecessor_command.payload->>'workspace_connection_id'
                   IS NOT DISTINCT FROM parent.workspace_connection_id::text
               AND predecessor_command.payload->>'source_repository' IS NOT DISTINCT FROM parent.source_repository
               AND predecessor_command.payload->>'source_base_ref' IS NOT DISTINCT FROM parent.source_base_ref
               AND predecessor_command.payload->>'source_base_commit' IS NOT DISTINCT FROM parent.source_base_commit
               AND predecessor_command.payload->'verification_policy'=prior.replacement_verification_policy
         ) predecessor ON TRUE
         WHERE run.corp_id=$1 AND run.id=$2",
    )
    .bind(item.corp_id)
    .bind(proof.run_id)
    .bind(item.id)
    .bind(proof.mission_id)
    .fetch_optional(&mut **tx)
    .await?
    .context("publication checkpoint origin is missing")?;
    let connection_id: Option<Uuid> = origin.get("workspace_connection_id");
    let lineage =
        checkpoint_publication_lineage_tx(tx, selected_run_id, authority, connection_id).await?;

    // Anchor at the proven stopped provider, not a CTE that could hide a
    // correction when its parent link is damaged. Validate item/task/mission
    // after selection so mismatches deny instead of becoming an ordinary run.
    let correction = sqlx::query(
        "SELECT factory_work_item_id,mission_id,task_id,source_run_id,replacement_run_id,
                mode,contract_revision_id,authorized_by,source_correction_authority
         FROM factory_verification_recoveries
         WHERE corp_id=$1 AND replacement_run_id=$2",
    )
    .bind(item.corp_id)
    .bind(proof.run_id)
    .fetch_optional(&mut **tx)
    .await?;
    // Both recovery modes execute as verification_only. Classify the exact
    // native predecessor instead. Historical suspension remains a positive
    // grant requirement even if predecessor recovery/command metadata changed.
    let verifier_predecessor = origin.get::<Option<String>, _>("parent_mode").as_deref()
        == Some("verification_only")
        || origin.get::<bool, _>("checkpoint_predecessor")
        || origin.get::<bool, _>("ordinary_predecessor_marker");
    let predecessor_mode: Option<String> = origin.get("predecessor_mode");
    let requires_origin = origin.get::<bool, _>("historical_suspension")
        || predecessor_mode.as_deref() == Some("checkpoint_verification")
        || origin.get::<bool, _>("checkpoint_predecessor")
        || origin.get::<bool, _>("checkpoint_correction_command");
    if verifier_predecessor {
        match predecessor_mode.as_deref() {
            Some("checkpoint_verification") => {}
            Some("verifier_only") if !requires_origin => {}
            _ => {
                return Err(anyhow!(
                    "publication correction predecessor provenance is missing or inconsistent"
                ));
            }
        }
    }
    let Some(correction) = correction else {
        if requires_origin || verifier_predecessor {
            return Err(anyhow!(
                "checkpoint source-correction publication provenance is missing"
            ));
        }
        return publication_authority_tx(tx, item, selected_run_id).await;
    };
    let source_run_id: Uuid = correction.get("source_run_id");
    if correction.get::<String, _>("mode") != "source_correction"
        || correction.get::<Uuid, _>("factory_work_item_id") != item.id
        || correction.get::<Uuid, _>("mission_id") != proof.mission_id
        || correction.get::<Uuid, _>("task_id") != proof.task_id
        || correction.get::<Option<Uuid>, _>("replacement_run_id") != Some(proof.run_id)
        || origin.get::<Option<Uuid>, _>("resumed_from_run_id") != Some(source_run_id)
    {
        return Err(anyhow!(
            "publication checkpoint lost its exact correction parent binding"
        ));
    }
    let Some(encoded) = correction.get::<Option<Value>, _>("source_correction_authority") else {
        if requires_origin {
            return Err(anyhow!(
                "source-correction publication provenance is missing"
            ));
        }
        return publication_authority_tx(tx, item, selected_run_id).await;
    };
    let expected: Authority = serde_json::from_value(encoded)
        .context("invalid source-correction publication provenance")?;
    let origin_index = lineage
        .iter()
        .position(|id| *id == proof.run_id)
        .context("publication checkpoint origin is outside the selected ancestry")?;
    let historical_index = lineage
        .iter()
        .position(|id| *id == expected.checkpoint.checkpoint.run_id)
        .context("publication correction suspension is outside the selected ancestry")?;
    if lineage.get(origin_index + 1) != Some(&source_run_id)
        || historical_index <= origin_index + 1
        || expected.workspace_connection_id != connection_id
        || origin
            .get::<Option<String>, _>("provider_session_id")
            .as_ref()
            != Some(&expected.provider_session_id)
    {
        return Err(anyhow!(
            "publication correction suffix no longer matches its historical grant"
        ));
    }
    let current = origin_with_publication_tx(
        tx,
        item,
        source_run_id,
        correction
            .get::<Option<Uuid>, _>("contract_revision_id")
            .context("publication correction revision is absent")?,
        correction.get("authorized_by"),
        true,
        Some(checkpoint),
    )
    .await?;
    if serde_json::to_value(expected)? != serde_json::to_value(current)? {
        return Err(anyhow!("source-correction publication authority changed"));
    }
    Ok(())
}
