//! Consume native stopped-source evidence through the existing recovery aggregate.
//! No provider restart, budget revision, new task, or inferred completion.
use super::*;
use crony_domain::StoppedSourceCheckpoint;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Authority {
    pub checkpoint: StoppedSourceCheckpoint,
    pub checkpoint_event_id: Uuid,
    pub termination_event_id: Uuid,
}

fn digest(value: &impl Serialize) -> Result<String> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(value)?)))
}

fn budget_metric(metric: &str) -> bool {
    matches!(
        metric,
        "run_tokens"
            | "run_cost"
            | "mission_tokens"
            | "mission_cost"
            | "actor_tokens_24h"
            | "actor_cost_24h"
            | "corp_tokens_24h"
            | "corp_cost_24h"
    )
}

async fn ensure_no_explicit_stop_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    workspace_run_id: Uuid,
) -> Result<()> {
    // A measured budget boundary does not supersede an operator's stop. Check
    // the complete preserved lineage, including earlier failed verifier retries.
    let stopped: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM runs lineage
            JOIN events stop_event
              ON stop_event.corp_id=lineage.corp_id AND stop_event.aggregate_id=lineage.id
             AND stop_event.aggregate_type='run' AND stop_event.type='run.stop_requested'
            WHERE lineage.corp_id=$1 AND lineage.workspace_run_id=$2
        )
        "#,
    )
    .bind(corp_id)
    .bind(workspace_run_id)
    .fetch_one(&mut **tx)
    .await?;
    if stopped {
        return Err(anyhow!(
            "checkpoint verification cannot bypass an explicit stop request"
        ));
    }
    Ok(())
}

pub(super) async fn source_authority_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    work_item_id: Uuid,
    source_run_id: Uuid,
) -> Result<Authority> {
    let source = sqlx::query(
        r#"
        SELECT run.id, run.task_id, task.mission_id, mission.room_id, run.execution_mode, run.status,
               run.agent_id, run.runner_id, run.workspace_run_id,
               run.workspace_path, run.workspace_branch, run.workspace_base_commit,
               run.workspace_disposition, run.workspace_fingerprint,
               run.source_repository, run.source_base_ref, run.source_base_commit,
               task.contract, task.verification_policy,
               recovery.checkpoint_authority, recovery.source_run_id AS parent_id,
               run.resumed_from_run_id
        FROM runs run
        JOIN tasks task ON task.id = run.task_id AND task.corp_id = run.corp_id
        JOIN missions mission ON mission.id=task.mission_id AND mission.corp_id=run.corp_id
        JOIN factory_work_items item
          ON item.mission_id = task.mission_id AND item.corp_id = run.corp_id
         AND item.id = $3
        LEFT JOIN factory_verification_recoveries recovery
          ON recovery.replacement_run_id = run.id AND recovery.corp_id = run.corp_id
         AND recovery.factory_work_item_id = item.id
         AND recovery.mission_id = task.mission_id AND recovery.task_id = task.id
         AND recovery.mode = 'checkpoint_verification' AND recovery.status = 'failed'
        WHERE run.corp_id = $1 AND run.id = $2
        "#,
    )
    .bind(corp_id)
    .bind(source_run_id)
    .bind(work_item_id)
    .fetch_optional(&mut **tx)
    .await?
    .context("checkpoint verification source is not in this factory item")?;
    let source_mode: String = source.get("execution_mode");
    if !matches!(
        source.get::<String, _>("status").as_str(),
        "failed" | "cancelled" | "lost"
    ) || source
        .get::<Option<String>, _>("workspace_disposition")
        .as_deref()
        != Some("preserved")
        || source
            .get::<Option<String>, _>("workspace_path")
            .is_none_or(|p| p.is_empty())
    {
        return Err(anyhow!(
            "checkpoint verification requires a terminal preserved source"
        ));
    }
    ensure_no_explicit_stop_tx(tx, corp_id, source.get("workspace_run_id")).await?;
    let inherited = if source_mode == "verification_only" {
        if source.get::<Option<Uuid>, _>("parent_id")
            != source.get::<Option<Uuid>, _>("resumed_from_run_id")
            || source.get::<Option<Uuid>, _>("parent_id").is_none()
        {
            return Err(anyhow!("checkpoint retry has no exact recovery parent"));
        }
        Some(serde_json::from_value::<Authority>(
            source
                .get::<Option<Value>, _>("checkpoint_authority")
                .context("checkpoint retry has no persisted checkpoint authority")?,
        )?)
    } else if source_mode == "provider" && source.get::<String, _>("status") != "lost" {
        None
    } else {
        return Err(anyhow!(
            "checkpoint verification has no proven provider source"
        ));
    };
    let origin_id = inherited
        .as_ref()
        .map_or(source_run_id, |a| a.checkpoint.run_id);
    let origin = sqlx::query(
        r#"
        SELECT run.task_id, run.agent_id, run.runner_id, run.workspace_run_id, run.status,
               run.workspace_branch, run.workspace_base_commit, run.breaker_stage,
               run.workspace_disposition, run.workspace_fingerprint, run.execution_mode,
               run.source_repository, run.source_base_ref, run.source_base_commit,
               run.no_progress_events, run.repeated_tool_count,
               COALESCE(policy.no_progress_event_limit, 8) AS no_progress_limit,
               COALESCE(policy.repeated_tool_limit, 5) AS repeated_tool_limit
        FROM runs run
        LEFT JOIN corp_budget_policies policy ON policy.corp_id = run.corp_id
        WHERE run.id = $1 AND run.corp_id = $2
        "#,
    )
    .bind(origin_id)
    .bind(corp_id)
    .fetch_optional(&mut **tx)
    .await?
    .context("checkpoint origin is missing")?;
    let stage: String = origin.get("breaker_stage");
    if !matches!(stage.as_str(), "suspend" | "stop")
        || !matches!(
            origin.get::<String, _>("status").as_str(),
            "failed" | "cancelled"
        )
        || origin.get::<String, _>("execution_mode") != "provider"
        || origin
            .get::<Option<String>, _>("workspace_disposition")
            .as_deref()
            != Some("preserved")
    {
        return Err(anyhow!(
            "checkpoint origin is not preserved budget-stopped provider work"
        ));
    }
    for (count, limit) in [
        (
            origin.get::<i32, _>("no_progress_events"),
            origin.get::<i32, _>("no_progress_limit"),
        ),
        (
            origin.get::<i32, _>("repeated_tool_count"),
            origin.get::<i32, _>("repeated_tool_limit"),
        ),
    ] {
        if limit > 0 && count >= limit {
            return Err(anyhow!(
                "checkpoint verification cannot waive a loop breaker"
            ));
        }
    }
    let incident: Value = sqlx::query_scalar(
        "SELECT input FROM circuit_breaker_incidents
         WHERE corp_id=$1 AND run_id=$2 AND stage=$3",
    )
    .bind(corp_id)
    .bind(origin_id)
    .bind(&stage)
    .fetch_optional(&mut **tx)
    .await?
    .context("checkpoint origin has no native budget incident")?;
    if !incident
        .get("metric")
        .and_then(Value::as_str)
        .is_some_and(budget_metric)
        || !matches!(
            (incident.get("used").and_then(Value::as_i64),
             incident.get("limit").and_then(Value::as_i64)),
            (Some(used), Some(limit)) if limit > 0 && used >= limit
        )
    {
        return Err(anyhow!(
            "checkpoint verification requires a measured model-budget boundary"
        ));
    }
    let event = sqlx::query(
        "SELECT id, seq, type AS event_type, payload, room_id, correlation_id FROM events
         WHERE corp_id=$1 AND aggregate_id=$2 AND aggregate_type='run'
           AND type IN ('run.workspace_preserved','run.workspace_removed','run.teardown_uncertain')
         ORDER BY seq DESC LIMIT 1",
    )
    .bind(corp_id)
    .bind(origin_id)
    .fetch_optional(&mut **tx)
    .await?
    .context("checkpoint origin has no native preservation event")?;
    if event.get::<String, _>("event_type") != "run.workspace_preserved" {
        return Err(anyhow!(
            "checkpoint origin has a newer unverified cleanup state"
        ));
    }
    let payload: Value = event.get("payload");
    let proof: StoppedSourceCheckpoint = serde_json::from_value(
        payload
            .get("source_checkpoint")
            .cloned()
            .context("source preservation has no stopped-source checkpoint proof")?,
    )
    .context("invalid stopped-source checkpoint proof")?;
    let termination = sqlx::query(
        "SELECT id,seq,payload,room_id,correlation_id FROM events
         WHERE corp_id=$1 AND aggregate_id=$2 AND aggregate_type='run'
           AND type='run.session_terminated' ORDER BY seq DESC LIMIT 1",
    )
    .bind(corp_id)
    .bind(origin_id)
    .fetch_optional(&mut **tx)
    .await?
    .context("checkpoint source provider termination is not proven")?;
    let termination_payload: Value = termination.get("payload");
    if termination.get::<i64, _>("seq") >= event.get::<i64, _>("seq")
        || termination_payload
            .get("provider_process_alive")
            .and_then(Value::as_bool)
            != Some(false)
    {
        return Err(anyhow!(
            "checkpoint source provider quiescence is not proven"
        ));
    }
    for journal_row in [&event, &termination] {
        if journal_row.get::<Option<Uuid>, _>("room_id") != Some(source.get("room_id"))
            || journal_row.get::<Option<Uuid>, _>("correlation_id")
                != Some(source.get("mission_id"))
        {
            return Err(anyhow!(
                "checkpoint journal does not match the source mission room"
            ));
        }
    }
    terminal_accounting::validate_termination_tx(
        tx,
        corp_id,
        source.get("task_id"),
        origin_id,
        "provider",
        &termination_payload,
    )
    .await?;
    let contract: TaskContract = serde_json::from_value(source.get("contract"))?;
    let policy: VerificationPolicy = serde_json::from_value(source.get("verification_policy"))?;
    if proof.schema_version != 1
        || proof.corp_id != corp_id
        || proof.mission_id != source.get::<Uuid, _>("mission_id")
        || proof.task_id != source.get::<Uuid, _>("task_id")
        || proof.task_id != origin.get::<Uuid, _>("task_id")
        || proof.run_id != origin_id
        || proof.agent_id != source.get::<Uuid, _>("agent_id")
        || proof.agent_id != origin.get::<Uuid, _>("agent_id")
        || proof.runner_id != source.get::<String, _>("runner_id")
        || proof.runner_id != origin.get::<String, _>("runner_id")
        || proof.workspace_run_id != source.get::<Uuid, _>("workspace_run_id")
        || proof.workspace_run_id != origin.get::<Uuid, _>("workspace_run_id")
        || Some(&proof.source_repository) != contract.source_repository.as_ref()
        || Some(&proof.source_base_ref) != contract.source_base_ref.as_ref()
        || Some(&proof.source_base_commit) != contract.source_base_commit.as_ref()
        || proof.source_base_commit != proof.workspace_base_commit
        || proof.verification_policy_sha256 != digest(&policy)?
        || proof.write_scope_sha256 != digest(&contract.write_scope)?
        || proof.deliverable_policy_sha256 != digest(&contract.deliverable)?
        || !valid_sha256(&proof.workspace_fingerprint)
    {
        return Err(anyhow!(
            "stopped checkpoint does not match its exact assignment and policy"
        ));
    }
    validate_factory_base_commit(&proof.head_commit)?;
    validate_factory_base_commit(&proof.source_base_commit)?;
    for row in [&source, &origin] {
        for (field, expected) in [
            ("source_repository", &proof.source_repository),
            ("source_base_ref", &proof.source_base_ref),
            ("source_base_commit", &proof.source_base_commit),
            ("workspace_base_commit", &proof.workspace_base_commit),
            ("workspace_branch", &proof.branch),
            ("workspace_fingerprint", &proof.workspace_fingerprint),
        ] {
            if row.get::<Option<String>, _>(field).as_ref() != Some(expected) {
                return Err(anyhow!("stopped checkpoint {field} no longer matches"));
            }
        }
    }
    if payload.get("head_commit").and_then(Value::as_str) != Some(proof.head_commit.as_str())
        || payload.get("workspace_fingerprint").and_then(Value::as_str)
            != Some(proof.workspace_fingerprint.as_str())
    {
        return Err(anyhow!(
            "stopped checkpoint disagrees with its native preservation event"
        ));
    }
    let authority = Authority {
        checkpoint: proof,
        checkpoint_event_id: event.get("id"),
        termination_event_id: termination.get("id"),
    };
    if let Some(inherited) = inherited
        && serde_json::to_value(inherited)? != serde_json::to_value(&authority)?
    {
        return Err(anyhow!(
            "checkpoint authority changed after recovery authorization"
        ));
    }
    Ok(authority)
}

pub(super) async fn validate_lineage_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    work_item_id: Uuid,
    workspace_run_id: Uuid,
    authority: &Authority,
) -> Result<()> {
    ensure_no_explicit_stop_tx(tx, corp_id, workspace_run_id).await?;
    let rows = sqlx::query(
        r#"
        SELECT run.id, run.task_id, run.agent_id, run.runner_id, run.status,
               run.execution_mode, run.breaker_stage, run.workspace_disposition,
               run.source_repository, run.source_base_ref, run.source_base_commit,
               run.created_at > origin.created_at AS after_origin,
               recovery.checkpoint_authority,
               recovery.source_run_id = run.resumed_from_run_id AS parent_matches
        FROM runs run
        JOIN runs origin ON origin.id=$3 AND origin.corp_id=run.corp_id
        LEFT JOIN factory_verification_recoveries recovery
          ON recovery.replacement_run_id=run.id AND recovery.corp_id=run.corp_id
         AND recovery.task_id=run.task_id AND recovery.factory_work_item_id=$4
         AND recovery.mode='checkpoint_verification' AND recovery.status='failed'
        WHERE run.corp_id=$1 AND run.workspace_run_id=$2
        ORDER BY run.created_at, run.id LIMIT 65
        "#,
    )
    .bind(corp_id)
    .bind(workspace_run_id)
    .bind(authority.checkpoint.run_id)
    .bind(work_item_id)
    .fetch_all(&mut **tx)
    .await?;
    if rows.is_empty() || rows.len() > 64 {
        return Err(anyhow!(
            "checkpoint lineage is missing or exceeds the native bound"
        ));
    }
    let proof = &authority.checkpoint;
    let expected = serde_json::to_value(authority)?;
    for row in rows {
        let id: Uuid = row.get("id");
        let stage: String = row.get("breaker_stage");
        let after_origin: bool = row.get("after_origin");
        if row.get::<Uuid, _>("task_id") != proof.task_id
            || row.get::<Uuid, _>("agent_id") != proof.agent_id
            || row.get::<String, _>("runner_id") != proof.runner_id
            || row.get::<Option<String>, _>("source_repository").as_ref()
                != Some(&proof.source_repository)
            || row.get::<Option<String>, _>("source_base_ref").as_ref()
                != Some(&proof.source_base_ref)
            || row.get::<Option<String>, _>("source_base_commit").as_ref()
                != Some(&proof.source_base_commit)
            || !matches!(
                row.get::<String, _>("status").as_str(),
                "failed" | "cancelled" | "lost"
            )
            || row
                .get::<Option<String>, _>("workspace_disposition")
                .as_deref()
                == Some("quarantined")
            || (id != proof.run_id && stage == "stop")
            || (after_origin && stage == "suspend")
        {
            return Err(anyhow!(
                "checkpoint lineage contains active, changed, quarantined or stopped work"
            ));
        }
        if after_origin
            && (row.get::<String, _>("execution_mode") != "verification_only"
                || row.get::<Option<bool>, _>("parent_matches") != Some(true)
                || row.get::<Option<Value>, _>("checkpoint_authority").as_ref() != Some(&expected))
        {
            return Err(anyhow!(
                "checkpoint lineage contains an unauthorized replacement"
            ));
        }
    }
    Ok(())
}

/// True only for an exactly bound zero-provider allocation. This is not a
/// generic exemption for verification_only, nor permission to ignore a stop.
pub(super) async fn zero_provider_allocation_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    run_id: Uuid,
) -> Result<bool> {
    Ok(sqlx::query_scalar(
        r#"
        SELECT EXISTS (
          SELECT 1 FROM runs run
          JOIN tasks task ON task.id=run.task_id AND task.corp_id=run.corp_id
          JOIN factory_verification_recoveries recovery
            ON recovery.replacement_run_id=run.id AND recovery.corp_id=run.corp_id
           AND recovery.task_id=run.task_id AND recovery.mission_id=task.mission_id
          JOIN factory_work_items item
            ON item.id=recovery.factory_work_item_id AND item.corp_id=run.corp_id
           AND item.mission_id=task.mission_id
          JOIN runs source
            ON source.id=recovery.source_run_id AND source.corp_id=run.corp_id
           AND source.id=run.resumed_from_run_id AND source.task_id=run.task_id
           AND source.agent_id=run.agent_id AND source.runner_id=run.runner_id
           AND source.workspace_run_id=run.workspace_run_id
           AND source.source_repository IS NOT DISTINCT FROM run.source_repository
           AND source.source_base_ref IS NOT DISTINCT FROM run.source_base_ref
           AND source.source_base_commit IS NOT DISTINCT FROM run.source_base_commit
          JOIN runs origin
            ON origin.id::text=recovery.checkpoint_authority->'checkpoint'->>'run_id'
           AND origin.corp_id=run.corp_id AND origin.task_id=run.task_id
           AND origin.agent_id=run.agent_id AND origin.runner_id=run.runner_id
           AND origin.workspace_run_id=run.workspace_run_id
          WHERE run.id=$1 AND run.corp_id=$2
            AND run.execution_mode='verification_only'
            AND run.provider_session_id IS NULL AND run.model IS NULL
            AND run.reasoning_effort IS NULL
            AND run.budget_tokens_limit=0 AND run.budget_cost_microusd_limit=0
            AND run.input_tokens=0 AND run.output_tokens=0 AND run.cost_microusd=0
            AND recovery.mode='checkpoint_verification'
            AND recovery.status IN ('authorized','running','completed')
            AND recovery.checkpoint_authority IS NOT NULL
            AND recovery.contract_revision_id IS NULL
            AND recovery.previous_verification_policy=recovery.replacement_verification_policy
            AND recovery.replacement_verification_policy=task.verification_policy
            AND recovery.request->>'expected_workspace_fingerprint'=source.workspace_fingerprint
            AND recovery.request->>'expected_workspace_fingerprint'
                =recovery.checkpoint_authority->'checkpoint'->>'workspace_fingerprint'
            AND recovery.request->>'expected_head_commit'
                =recovery.checkpoint_authority->'checkpoint'->>'head_commit'
            AND source.workspace_disposition='preserved'
            AND origin.workspace_disposition='preserved'
            AND origin.execution_mode='provider' AND origin.breaker_stage IN ('suspend','stop')
            AND run.workspace_disposition IS DISTINCT FROM 'quarantined'
            AND NOT EXISTS (
                SELECT 1 FROM runs lineage
                JOIN events stop_event
                  ON stop_event.corp_id=lineage.corp_id AND stop_event.aggregate_id=lineage.id
                 AND stop_event.aggregate_type='run' AND stop_event.type='run.stop_requested'
                WHERE lineage.corp_id=run.corp_id
                  AND lineage.workspace_run_id=run.workspace_run_id
            )
        )
        "#,
    )
    .bind(run_id)
    .bind(corp_id)
    .fetch_one(&mut **tx)
    .await?)
}
