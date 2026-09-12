//! Private reconstruction of native correction history. No stored grant is
//! rewritten, and an historical revision is never current execution authority.
use super::*;
use sqlx::postgres::PgRow;

pub(super) struct HistoricalCorrection {
    pub authority: Authority,
    pub previous_contract: TaskContract,
    pub previous_policy: VerificationPolicy,
    revision_version: i64,
    contract: TaskContract,
    policy: VerificationPolicy,
}

struct Revision {
    id: Uuid,
    version: i64,
    previous_version: i64,
    source_run_id: Uuid,
    previous_contract: TaskContract,
    contract: TaskContract,
    previous_policy: VerificationPolicy,
    policy: VerificationPolicy,
}

async fn run_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    id: Uuid,
) -> Result<PgRow> {
    sqlx::query(
        "SELECT run.*,task.mission_id,mission.room_id,agent.adapter
         FROM runs run JOIN tasks task ON task.id=run.task_id AND task.corp_id=run.corp_id
         JOIN missions mission ON mission.id=task.mission_id AND mission.corp_id=run.corp_id
         JOIN agents agent ON agent.id=run.agent_id AND agent.corp_id=run.corp_id
         WHERE run.corp_id=$1 AND run.id=$2 AND task.mission_id=$3",
    )
    .bind(item.corp_id)
    .bind(id)
    .bind(item.mission_id)
    .fetch_optional(&mut **tx)
    .await?
    .context("correction history has a missing or foreign run")
}

async fn revision_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    source: Uuid,
    id: Uuid,
    actor: Uuid,
    current: bool,
) -> Result<Revision> {
    let row = sqlx::query(
        "SELECT revision.*,task.contract AS current_contract,
                task.verification_policy AS current_policy,task.contract_version
         FROM mission_contract_revisions revision
         JOIN runs source ON source.id=revision.source_run_id AND source.corp_id=revision.corp_id
                          AND source.task_id=revision.task_id
         JOIN tasks task ON task.id=source.task_id AND task.corp_id=source.corp_id
         WHERE revision.corp_id=$1 AND revision.id=$2 AND revision.source_run_id=$3
           AND revision.revised_by=$4 AND revision.mission_id=$5
           AND task.mission_id=revision.mission_id AND revision.next_action='resume'",
    )
    .bind(item.corp_id)
    .bind(id)
    .bind(source)
    .bind(actor)
    .bind(item.mission_id)
    .fetch_optional(&mut **tx)
    .await?
    .context("source correction has no exact immutable Resume revision")?;
    if current
        && (row.get::<i64, _>("version") != row.get::<i64, _>("contract_version")
            || row.get::<Value, _>("replacement_contract")
                != row.get::<Value, _>("current_contract")
            || row.get::<Value, _>("replacement_verification_policy")
                != row.get::<Value, _>("current_policy"))
    {
        return Err(anyhow!("source correction revision was superseded"));
    }
    let revision = Revision {
        id,
        version: row.get("version"),
        previous_version: row
            .get::<Value, _>("request")
            .get("expected_contract_version")
            .and_then(Value::as_i64)
            .context("correction revision omitted its previous task version")?,
        source_run_id: source,
        previous_contract: serde_json::from_value(row.get("previous_contract"))?,
        contract: serde_json::from_value(row.get("replacement_contract"))?,
        previous_policy: serde_json::from_value(row.get("previous_verification_policy"))?,
        policy: serde_json::from_value(row.get("replacement_verification_policy"))?,
    };
    ensure_factory_recovery_verification_policy_not_weakened(
        &revision.previous_policy,
        &revision.policy,
    )?;
    ensure_factory_recovery_policy(item, &revision.contract, &revision.policy)?;
    validate_revision_record_tx(tx, item, &row, &revision).await?;
    Ok(revision)
}

async fn validate_revision_record_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    row: &PgRow,
    revision: &Revision,
) -> Result<()> {
    if revision.previous_version < 1 || revision.previous_version >= revision.version {
        return Err(anyhow!("correction revision has invalid version ancestry"));
    }
    let request: Value = row.get("request");
    let description: String = row.get("replacement_description");
    for (field, expected) in [
        ("mission_id", json!(item.mission_id)),
        ("task_id", json!(row.get::<Uuid, _>("task_id"))),
        ("next_action", json!("resume")),
        ("source_run_id", json!(revision.source_run_id)),
        ("reason", json!(row.get::<String, _>("reason"))),
        ("description", json!(description)),
        (
            "verification_policy",
            serde_json::to_value(&revision.policy)?,
        ),
    ] {
        if request.get(field) != Some(&expected) {
            return Err(anyhow!(
                "native revision request changed its {field} binding"
            ));
        }
    }
    // Native revision admission composes the task objective from descriptions;
    // the immutable event binds that normalized objective and the whole contract.
    let mut requested: TaskContract = serde_json::from_value(
        request
            .get("contract")
            .cloned()
            .context("native revision request omitted its contract")?,
    )?;
    requested.objective = revision.contract.objective.clone();
    if serde_json::to_value(&requested)? != serde_json::to_value(&revision.contract)? {
        return Err(anyhow!("native revision contract differs from its request"));
    }
    let event: Value = sqlx::query_scalar(
        "SELECT payload FROM events WHERE corp_id=$1 AND aggregate_id=$2
           AND type='mission.contract_revised' AND aggregate_type='mission_contract_revision'
           AND aggregate_version=$3 AND actor_id=$4 AND correlation_id=$5 AND idempotency_key=$6",
    )
    .bind(item.corp_id)
    .bind(revision.id)
    .bind(revision.version)
    .bind(row.get::<Uuid, _>("revised_by"))
    .bind(item.mission_id)
    .bind(format!("mission-contract-revision:{}", revision.id))
    .fetch_optional(&mut **tx)
    .await?
    .context("correction revision omitted its native event")?;
    for (field, expected) in [
        ("mission_id", json!(item.mission_id)),
        ("task_id", json!(row.get::<Uuid, _>("task_id"))),
        ("version", json!(revision.version)),
        ("next_action", json!("resume")),
        ("source_run_id", json!(revision.source_run_id)),
        (
            "description_sha256",
            json!(hex::encode(Sha256::digest(description.as_bytes()))),
        ),
        ("contract_sha256", json!(digest(&revision.contract)?)),
        (
            "verification_policy_sha256",
            json!(digest(&revision.policy)?),
        ),
    ] {
        if event.get(field) != Some(&expected) {
            return Err(anyhow!("native revision event changed its {field} binding"));
        }
    }
    Ok(())
}

async fn revision_bridge_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    source: &PgRow,
    previous: &HistoricalCorrection,
    current: &Revision,
) -> Result<()> {
    let rows = sqlx::query(
        "SELECT id,revised_by FROM mission_contract_revisions
         WHERE corp_id=$1 AND task_id=$2 AND version>$3 AND version<$4
         ORDER BY version LIMIT 65",
    )
    .bind(item.corp_id)
    .bind(source.get::<Uuid, _>("task_id"))
    .bind(previous.revision_version)
    .bind(current.version)
    .fetch_all(&mut **tx)
    .await?;
    if rows.len() > 64 {
        return Err(anyhow!(
            "correction revision bridge exceeds its native bound"
        ));
    }
    let mut version = previous.revision_version;
    let mut contract = serde_json::to_value(&previous.contract)?;
    let mut policy = serde_json::to_value(&previous.policy)?;
    for row in rows {
        let next = revision_tx(
            tx,
            item,
            current.source_run_id,
            row.get("id"),
            row.get("revised_by"),
            false,
        )
        .await?;
        // Version numbers also reflect other tasks' mission-spec revisions.
        // Follow the native expected-task-version links, not numeric +1.
        if next.previous_version != version
            || serde_json::to_value(&next.previous_contract)? != contract
            || serde_json::to_value(&next.previous_policy)? != policy
        {
            return Err(anyhow!("correction revision bridge is not contiguous"));
        }
        version = next.version;
        contract = serde_json::to_value(&next.contract)?;
        policy = serde_json::to_value(&next.policy)?;
    }
    if current.previous_version != version
        || serde_json::to_value(&current.previous_contract)? != contract
        || serde_json::to_value(&current.previous_policy)? != policy
    {
        return Err(anyhow!(
            "correction revision does not continue its historical policy"
        ));
    }
    Ok(())
}

/// Positive native witnesses survive a NULL/deleted grant. The boundary keeps
/// genuinely ordinary corrections before the checkpoint family compatible.
pub(super) async fn requires_authority_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    run_id: Uuid,
) -> Result<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS (
           SELECT 1 FROM runs boundary JOIN runs generation
             ON generation.corp_id=boundary.corp_id
            AND generation.workspace_run_id=boundary.workspace_run_id
            AND (generation.created_at,generation.id)<=(boundary.created_at,boundary.id)
           LEFT JOIN factory_verification_recoveries recovery
             ON recovery.corp_id=generation.corp_id AND recovery.replacement_run_id=generation.id
           WHERE boundary.corp_id=$1 AND boundary.id=$2 AND (
             recovery.source_correction_authority IS NOT NULL OR EXISTS (
               SELECT 1 FROM events event WHERE event.corp_id=generation.corp_id
                 AND event.type='factory.verification_recovery_authorized'
                 AND event.payload->>'replacement_run_id'=generation.id::text
                 AND event.payload->'source_correction_origin_run_id' IS NOT NULL
                 AND event.payload->'source_correction_origin_run_id'<>'null'::jsonb
             ) OR (
               (recovery.mode='source_correction' OR EXISTS (
                 SELECT 1 FROM runner_commands command WHERE command.corp_id=generation.corp_id
                   AND command.run_id=generation.id
                   AND command.command_kind='factory_verification_recovery'
                   AND command.payload->>'mode'='source_correction'
               ) OR (generation.execution_mode='provider' AND EXISTS (
                 SELECT 1 FROM factory_verification_recoveries checkpoint
                 WHERE checkpoint.corp_id=generation.corp_id
                   AND checkpoint.replacement_run_id=generation.resumed_from_run_id
                   AND checkpoint.mode='checkpoint_verification'
               ))) AND EXISTS (
                 SELECT 1 FROM runs origin
                 LEFT JOIN factory_verification_recoveries checkpoint
                   ON checkpoint.corp_id=origin.corp_id AND checkpoint.replacement_run_id=origin.id
                 WHERE origin.corp_id=generation.corp_id
                   AND origin.workspace_run_id=generation.workspace_run_id
                   AND (origin.created_at,origin.id)<=(generation.created_at,generation.id)
                   AND (origin.breaker_stage='suspend' OR checkpoint.mode='checkpoint_verification')
               )
             )
           )
         )",
    )
    .bind(item.corp_id)
    .bind(run_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(Into::into)
}

pub(super) async fn current_authority_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    source: Uuid,
    revision_id: Uuid,
    actor: Uuid,
) -> Result<Authority> {
    let revision = revision_tx(tx, item, source, revision_id, actor, true).await?;
    derive_tx(tx, item, &revision, &mut HashSet::new()).await
}

pub(super) async fn historical_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    replacement_id: Uuid,
) -> Result<Option<HistoricalCorrection>> {
    historical_inner_tx(tx, item, replacement_id, &mut HashSet::new()).await
}

pub(super) async fn source_context_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    source_id: Uuid,
) -> Result<Option<Authority>> {
    let Some(previous) = historical_tx(tx, item, source_id).await? else {
        return Ok(None);
    };
    let current = sqlx::query(
        "SELECT task.contract,task.verification_policy FROM tasks task
         JOIN runs source ON source.task_id=task.id AND source.corp_id=task.corp_id
         WHERE source.corp_id=$1 AND source.id=$2 AND task.mission_id=$3",
    )
    .bind(item.corp_id)
    .bind(source_id)
    .bind(item.mission_id)
    .fetch_one(&mut **tx)
    .await?;
    if current.get::<Value, _>("contract") != serde_json::to_value(&previous.contract)?
        || current.get::<Value, _>("verification_policy") != serde_json::to_value(&previous.policy)?
    {
        return Err(anyhow!(
            "correction context requires the current source-bound revision"
        ));
    }
    validate_admission_suffix_tx(tx, item, source_id).await?;
    Ok(Some(previous.authority))
}

/// Refactored from retained_provider_receipt::previous_correction_context_tx.
/// Only this proven persisted edge may supply a non-current revision.
async fn historical_inner_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    replacement_id: Uuid,
    seen: &mut HashSet<Uuid>,
) -> Result<Option<HistoricalCorrection>> {
    if seen.len() >= 64 || !seen.insert(replacement_id) {
        return Err(anyhow!(
            "correction history is cyclic or exceeds its native bound"
        ));
    }
    let required = requires_authority_tx(tx, item, replacement_id).await?;
    let correction = sqlx::query(
        "SELECT * FROM factory_verification_recoveries
         WHERE corp_id=$1 AND replacement_run_id=$2",
    )
    .bind(item.corp_id)
    .bind(replacement_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(correction) = correction else {
        if required {
            return Err(anyhow!(
                "checkpoint correction history omitted its native recovery"
            ));
        }
        seen.remove(&replacement_id);
        return Ok(None);
    };
    let saved: Option<Value> = correction.get("source_correction_authority");
    let Some(saved) = saved else {
        if required {
            return Err(anyhow!(
                "checkpoint correction history omitted its authority"
            ));
        }
        seen.remove(&replacement_id);
        return Ok(None);
    };
    let expected: Authority = serde_json::from_value(saved.clone())
        .context("historical correction authority is malformed")?;
    let replacement = run_tx(tx, item, replacement_id).await?;
    let source_id: Uuid = correction.get("source_run_id");
    if correction.get::<String, _>("mode") != "source_correction"
        || correction.get::<String, _>("status") != "failed"
        || correction.get::<Uuid, _>("factory_work_item_id") != item.id
        || Some(correction.get::<Uuid, _>("mission_id")) != item.mission_id
        || correction.get::<Uuid, _>("task_id") != replacement.get::<Uuid, _>("task_id")
        || Some(source_id) != replacement.get::<Option<Uuid>, _>("resumed_from_run_id")
        || replacement.get::<String, _>("execution_mode") != "provider"
        || !matches!(
            replacement.get::<String, _>("status").as_str(),
            "failed" | "cancelled" | "lost"
        )
        || expected.schema_version != 1
        || correction.get::<Option<Uuid>, _>("contract_revision_id")
            != Some(expected.contract_revision_id)
    {
        return Err(anyhow!(
            "historical correction does not match its exact replacement"
        ));
    }
    let revision = revision_tx(
        tx,
        item,
        source_id,
        expected.contract_revision_id,
        correction.get("authorized_by"),
        false,
    )
    .await?;
    let proof = &expected.checkpoint.checkpoint;
    if replacement.get::<Uuid, _>("task_id") != proof.task_id
        || replacement.get::<Uuid, _>("agent_id") != proof.agent_id
        || replacement.get::<String, _>("runner_id") != proof.runner_id
        || replacement.get::<Uuid, _>("workspace_run_id") != proof.workspace_run_id
        || replacement
            .get::<Option<String>, _>("source_repository")
            .as_ref()
            != Some(&proof.source_repository)
        || replacement
            .get::<Option<String>, _>("source_base_ref")
            .as_ref()
            != Some(&proof.source_base_ref)
        || replacement
            .get::<Option<String>, _>("source_base_commit")
            .as_ref()
            != Some(&proof.source_base_commit)
        || replacement.get::<i64, _>("budget_tokens_limit") <= 0
        || replacement.get::<i64, _>("budget_cost_microusd_limit") <= 0
        || replacement
            .get::<Option<String>, _>("provider_session_id")
            .as_ref()
            != Some(&expected.provider_session_id)
        || replacement.get::<Option<Uuid>, _>("workspace_connection_id")
            != expected.workspace_connection_id
        || replacement.get::<Option<String>, _>("model") != revision.contract.model
        || replacement.get::<Option<String>, _>("reasoning_effort")
            != revision.contract.reasoning_effort
        || correction.get::<Value, _>("previous_verification_policy")
            != serde_json::to_value(&revision.previous_policy)?
        || correction.get::<Value, _>("replacement_verification_policy")
            != serde_json::to_value(&revision.policy)?
    {
        return Err(anyhow!(
            "historical correction contract or provider binding changed"
        ));
    }
    validate_native_edge_tx(tx, item, &correction, &replacement, &revision, &expected).await?;
    let authority = Box::pin(derive_tx(tx, item, &revision, seen)).await?;
    if serde_json::to_value(&authority)? != saved {
        return Err(anyhow!(
            "historical correction no longer matches its persisted authority"
        ));
    }
    seen.remove(&replacement_id);
    Ok(Some(HistoricalCorrection {
        authority,
        revision_version: revision.version,
        previous_contract: revision.previous_contract,
        previous_policy: revision.previous_policy,
        contract: revision.contract,
        policy: revision.policy,
    }))
}

async fn validate_native_edge_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    recovery: &PgRow,
    replacement: &PgRow,
    revision: &Revision,
    authority: &Authority,
) -> Result<()> {
    let request: Value = recovery.get("request");
    let recovery_id: Uuid = recovery.get("id");
    let run_id: Uuid = replacement.get("id");
    let source = run_tx(tx, item, revision.source_run_id).await?;
    let command: Value = sqlx::query_scalar(
        "SELECT payload FROM runner_commands WHERE corp_id=$1 AND run_id=$2 AND runner_id=$3
           AND command_kind='factory_verification_recovery' AND idempotency_key=$4",
    )
    .bind(item.corp_id)
    .bind(run_id)
    .bind(replacement.get::<String, _>("runner_id"))
    .bind(format!("factory-verification-recovery:{recovery_id}"))
    .fetch_optional(&mut **tx)
    .await?
    .context("historical correction omitted its exact runner command")?;
    for (field, value) in [
        ("work_item_id", json!(item.id)),
        ("source_run_id", json!(revision.source_run_id)),
        ("mode", json!("source_correction")),
        ("contract_revision_id", json!(revision.id)),
        ("reason", json!(recovery.get::<String, _>("reason"))),
        (
            "observed_source_revision",
            json!(recovery.get::<String, _>("observed_source_revision")),
        ),
        (
            "reviewed_source_snapshot",
            recovery.get("reviewed_source_snapshot"),
        ),
    ] {
        if request.get(field) != Some(&value) {
            return Err(anyhow!(
                "historical correction request changed its {field} binding"
            ));
        }
    }
    for (field, value) in [
        ("mode", json!("source_correction")),
        ("corp_id", json!(item.corp_id)),
        ("room_id", json!(replacement.get::<Uuid, _>("room_id"))),
        ("mission_id", json!(item.mission_id)),
        ("task_id", json!(replacement.get::<Uuid, _>("task_id"))),
        ("run_id", json!(run_id)),
        ("source_run_id", json!(revision.source_run_id)),
        (
            "workspace_run_id",
            json!(replacement.get::<Uuid, _>("workspace_run_id")),
        ),
        ("agent_id", json!(replacement.get::<Uuid, _>("agent_id"))),
        (
            "assignment_token",
            json!(replacement.get::<Uuid, _>("assignment_token")),
        ),
        ("adapter", json!(replacement.get::<String, _>("adapter"))),
        ("provider_session_id", json!(authority.provider_session_id)),
        (
            "workspace_connection_id",
            json!(authority.workspace_connection_id),
        ),
        (
            "source_repository",
            json!(revision.contract.source_repository),
        ),
        ("source_base_ref", json!(revision.contract.source_base_ref)),
        (
            "source_base_commit",
            json!(revision.contract.source_base_commit),
        ),
        (
            "workspace_base_commit",
            json!(authority.checkpoint.checkpoint.workspace_base_commit),
        ),
        ("model", json!(revision.contract.model)),
        (
            "reasoning_effort",
            json!(revision.contract.reasoning_effort),
        ),
        (
            "verification_policy",
            serde_json::to_value(&revision.policy)?,
        ),
        ("write_scope", json!(revision.contract.write_scope)),
        ("deliverable", json!(revision.contract.deliverable)),
        ("secret_refs", json!(revision.contract.secret_refs)),
    ] {
        if command.get(field) != Some(&value) {
            return Err(anyhow!(
                "historical correction command changed its {field} binding"
            ));
        }
    }
    for field in ["expected_workspace_fingerprint", "expected_head_commit"] {
        if command.get(field) != request.get(field) {
            return Err(anyhow!(
                "historical correction command changed its source checkpoint"
            ));
        }
    }
    let checkpoint =
        source_workspace_checkpoint_with_lock_tx(tx, item.corp_id, revision.source_run_id, false)
            .await?;
    checkpoint.ensure_preserved()?;
    if request.get("expected_workspace_fingerprint") != Some(&json!(checkpoint.fingerprint))
        || request.get("expected_head_commit") != Some(&json!(checkpoint.expected_head_commit))
        || command.get("workspace_base_commit")
            != Some(&json!(
                source.get::<Option<String>, _>("workspace_base_commit")
            ))
        || source.get::<String, _>("adapter") != replacement.get::<String, _>("adapter")
        || (!pre_dispatch_failure(replacement)
            && replacement.get::<Option<String>, _>("workspace_base_commit")
                != source.get::<Option<String>, _>("workspace_base_commit"))
    {
        return Err(anyhow!(
            "historical command does not match its immutable source checkpoint"
        ));
    }
    let authorized_row = sqlx::query(
        "SELECT seq,payload FROM events WHERE corp_id=$1 AND aggregate_id=$2
           AND type='factory.verification_recovery_authorized'
           AND aggregate_type='factory_verification_recovery' AND actor_id=$3
           AND causation_id=$4 AND correlation_id=$5 AND room_id=$6 AND idempotency_key=$7",
    )
    .bind(item.corp_id)
    .bind(recovery_id)
    .bind(recovery.get::<Uuid, _>("authorized_by"))
    .bind(revision.source_run_id)
    .bind(item.mission_id)
    .bind(replacement.get::<Uuid, _>("room_id"))
    .bind(format!(
        "factory-verification-recovery:{recovery_id}:authorized"
    ))
    .fetch_optional(&mut **tx)
    .await?
    .context("historical correction omitted its native authorization event")?;
    let authorized: Value = authorized_row.get("payload");
    let seal = sqlx::query(
        "SELECT type,payload,room_id,correlation_id FROM events
         WHERE corp_id=$1 AND aggregate_type='run' AND aggregate_id=$2 AND seq<$3
           AND type IN ('run.workspace_preserved','run.workspace_removed','run.teardown_uncertain')
         ORDER BY seq DESC LIMIT 1",
    )
    .bind(item.corp_id)
    .bind(revision.source_run_id)
    .bind(authorized_row.get::<i64, _>("seq"))
    .fetch_optional(&mut **tx)
    .await?
    .context("historical source has no native preservation seal")?;
    let seal_payload: Value = seal.get("payload");
    if seal.get::<String, _>("type") != "run.workspace_preserved"
        || seal.get::<Option<Uuid>, _>("room_id") != Some(source.get("room_id"))
        || seal.get::<Option<Uuid>, _>("correlation_id") != item.mission_id
        || seal_payload.get("workspace_fingerprint")
            != request.get("expected_workspace_fingerprint")
        || seal_payload
            .get("workspace_quarantined")
            .and_then(Value::as_bool)
            == Some(true)
        || !source_seal_head_matches(&checkpoint, &seal_payload)
    {
        return Err(anyhow!(
            "historical request no longer matches its native source seal"
        ));
    }
    for (field, value) in [
        ("factory_work_item_id", json!(item.id)),
        ("mission_id", json!(item.mission_id)),
        ("task_id", json!(replacement.get::<Uuid, _>("task_id"))),
        ("source_run_id", json!(revision.source_run_id)),
        ("replacement_run_id", json!(run_id)),
        ("mode", json!("source_correction")),
        ("contract_revision_id", json!(revision.id)),
        (
            "source_correction_origin_run_id",
            json!(authority.checkpoint.checkpoint.run_id),
        ),
        ("reason", json!(recovery.get::<String, _>("reason"))),
        (
            "observed_source_revision",
            json!(recovery.get::<String, _>("observed_source_revision")),
        ),
    ] {
        if authorized.get(field) != Some(&value) {
            return Err(anyhow!(
                "historical correction event changed its {field} binding"
            ));
        }
    }
    Ok(())
}

fn source_seal_head_matches(checkpoint: &SourceWorkspaceCheckpoint, seal: &Value) -> bool {
    let Some(expected_head) = checkpoint.expected_head_commit.as_deref() else {
        return true;
    };
    match seal.get("head_commit") {
        Some(Value::String(head)) => head == expected_head,
        // Native verifier cleanup reports this exact authorized fingerprint only
        // after checking both the fingerprint and assigned HEAD. Its existing
        // preservation event omits the redundant head field. The source helper
        // retains that HEAD from the exact verifier recovery, and historical
        // authority is independently reconstructed and compared after this edge.
        // Only absence is compatible: null, malformed or contradictory explicit
        // heads, unbound verifiers and provider sources receive no exception.
        None if checkpoint.execution_mode == "verification_only" => checkpoint
            .expected_verifier_fingerprint
            .as_deref()
            .is_some_and(|expected| {
                valid_sha256(expected) && checkpoint.fingerprint.as_deref() == Some(expected)
            }),
        _ => false,
    }
}

fn pre_dispatch_failure(run: &PgRow) -> bool {
    lineage_run_is_pre_dispatch_failure(
        &run.get::<String, _>("status"),
        run.get::<Option<String>, _>("workspace_path").as_deref(),
        run.get::<Option<String>, _>("workspace_disposition")
            .as_deref(),
        run.get::<Option<String>, _>("workspace_detail").as_deref(),
    )
}

async fn derive_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    revision: &Revision,
    seen: &mut HashSet<Uuid>,
) -> Result<Authority> {
    let source_id = revision.source_run_id;
    let source = run_tx(tx, item, source_id).await?;
    let parent = if source.get::<String, _>("execution_mode") == "provider" {
        Some(
            Box::pin(historical_inner_tx(tx, item, source_id, seen))
                .await?
                .context("provider correction has no proven historical authority")?,
        )
    } else {
        // Native pre-dispatch failures do not become the source, but their
        // applied revisions remain history. Reuse only a matching failed edge.
        let prior = sqlx::query(
            "SELECT run.* FROM factory_verification_recoveries recovery
             JOIN runs run ON run.id=recovery.replacement_run_id AND run.corp_id=recovery.corp_id
             JOIN mission_contract_revisions revision ON revision.id=recovery.contract_revision_id
                    AND revision.corp_id=recovery.corp_id
             WHERE recovery.corp_id=$1 AND recovery.factory_work_item_id=$2
               AND recovery.source_run_id=$3 AND recovery.mode='source_correction'
               AND recovery.status='failed' AND revision.version<$4
             ORDER BY revision.version DESC,run.created_at DESC,run.id DESC LIMIT 65",
        )
        .bind(item.corp_id)
        .bind(item.id)
        .bind(source_id)
        .bind(revision.version)
        .fetch_all(&mut **tx)
        .await?;
        if prior.len() > 64 {
            return Err(anyhow!(
                "correction revision history exceeds its native bound"
            ));
        }
        if let Some(prior) = prior.iter().find(|run| pre_dispatch_failure(run)) {
            Some(
                Box::pin(historical_inner_tx(tx, item, prior.get("id"), seen))
                    .await?
                    .context("failed pre-dispatch correction omitted its historical authority")?,
            )
        } else {
            None
        }
    };
    let mut authority = if let Some(parent) = parent {
        revision_bridge_tx(tx, item, &source, &parent, revision).await?;
        parent.authority
    } else {
        // The original retained-receipt reconstruction: immutable revision
        // policy validates the original failed checkpoint, not today's task.
        let checkpoint = sqlx::query(
            "SELECT checkpoint.id,checkpoint.checkpoint_authority
             FROM factory_verification_recoveries checkpoint JOIN runs source
               ON source.id=checkpoint.replacement_run_id AND source.corp_id=checkpoint.corp_id
              AND source.task_id=checkpoint.task_id AND source.resumed_from_run_id=checkpoint.source_run_id
             WHERE checkpoint.corp_id=$1 AND checkpoint.factory_work_item_id=$2
               AND checkpoint.replacement_run_id=$3 AND checkpoint.mission_id=$4
               AND checkpoint.mode='checkpoint_verification' AND checkpoint.status='failed'
               AND source.execution_mode='verification_only' AND source.status='failed'
               AND source.verification_status='failed' AND source.provider_session_id IS NULL",
        ).bind(item.corp_id).bind(item.id).bind(source_id).bind(item.mission_id)
            .fetch_optional(&mut **tx).await?
            .context("source correction requires the exact failed checkpoint")?;
        let proof = budget_checkpoint::source_authority_with_contract_tx(
            tx,
            item.corp_id,
            item.id,
            source_id,
            Some((&revision.previous_contract, &revision.previous_policy)),
        )
        .await?;
        if checkpoint.get::<Option<Value>, _>("checkpoint_authority")
            != Some(serde_json::to_value(&proof)?)
        {
            return Err(anyhow!("historical failed checkpoint proof changed"));
        }
        budget_checkpoint::validate_lineage_until_tx(
            tx,
            item.corp_id,
            item.id,
            proof.checkpoint.workspace_run_id,
            &proof,
            source_id,
        )
        .await?;
        let origin = sqlx::query(
            "SELECT incident.id,incident.input,origin.workspace_connection_id,origin.provider_session_id
             FROM runs origin JOIN circuit_breaker_incidents incident
               ON incident.run_id=origin.id AND incident.corp_id=origin.corp_id AND incident.stage='suspend'
             WHERE origin.corp_id=$1 AND origin.id=$2 AND origin.breaker_stage='suspend'",
        ).bind(item.corp_id).bind(proof.checkpoint.run_id).fetch_optional(&mut **tx).await?
            .context("historical correction no longer has its native suspension")?;
        Authority {
            schema_version: 1,
            checkpoint_recovery_id: checkpoint.get("id"),
            checkpoint: proof,
            suspension_incident_id: origin.get("id"),
            suspension_input_sha256: digest(&origin.get::<Value, _>("input"))?,
            contract_revision_id: revision.id,
            previous_contract_sha256: String::new(),
            replacement_contract_sha256: String::new(),
            previous_policy_sha256: String::new(),
            replacement_policy_sha256: String::new(),
            factory_policy_sha256: String::new(),
            workspace_connection_id: origin.get("workspace_connection_id"),
            provider_session_id: origin
                .get::<Option<String>, _>("provider_session_id")
                .filter(|session| !session.is_empty())
                .context("historical correction origin lost its provider session")?,
            prefix_run_ids: Vec::new(),
        }
    };
    let connection = authority.workspace_connection_id;
    if source.get::<Option<Uuid>, _>("workspace_connection_id") != connection
        || revision.previous_contract.workspace_connection_id != connection
        || revision.contract.workspace_connection_id != connection
        || factory_workspace_connection_id(&item.policy).map_err(anyhow::Error::msg)? != connection
        || (source.get::<String, _>("execution_mode") == "provider"
            && source
                .get::<Option<String>, _>("provider_session_id")
                .as_ref()
                != Some(&authority.provider_session_id))
    {
        return Err(anyhow!(
            "source correction execution connection or session changed"
        ));
    }
    let lineage =
        checkpoint_publication_lineage_tx(tx, source_id, &authority.checkpoint, connection).await?;
    let prefix = sqlx::query(
        "SELECT run.*,COALESCE(policy.no_progress_event_limit,8) AS progress_limit,
                COALESCE(policy.repeated_tool_limit,5) AS tool_limit
         FROM runs run JOIN runs boundary ON boundary.id=$3 AND boundary.corp_id=run.corp_id
         LEFT JOIN corp_budget_policies policy ON policy.corp_id=run.corp_id
         WHERE run.corp_id=$1 AND run.workspace_run_id=$2
           AND (run.created_at,run.id)<=(boundary.created_at,boundary.id)
         ORDER BY run.created_at,run.id LIMIT 65",
    )
    .bind(item.corp_id)
    .bind(authority.checkpoint.checkpoint.workspace_run_id)
    .bind(source_id)
    .fetch_all(&mut **tx)
    .await?;
    let prefix_ids: Vec<Uuid> = prefix.iter().map(|run| run.get("id")).collect();
    if prefix_ids.is_empty() || prefix_ids.len() > 64 || prefix_ids.last() != Some(&source_id) {
        return Err(anyhow!("source correction checkpoint prefix is invalid"));
    }
    for run in &prefix {
        let id: Uuid = run.get("id");
        if run
            .get::<Option<String>, _>("workspace_disposition")
            .as_deref()
            == Some("quarantined")
            || run.get::<String, _>("breaker_stage") == "stop"
            || (run.get::<String, _>("breaker_stage") == "suspend"
                && id != authority.checkpoint.checkpoint.run_id)
            || (run.get::<i32, _>("progress_limit") > 0
                && run.get::<i32, _>("no_progress_events") >= run.get::<i32, _>("progress_limit"))
            || (run.get::<i32, _>("tool_limit") > 0
                && run.get::<i32, _>("repeated_tool_count") >= run.get::<i32, _>("tool_limit"))
        {
            return Err(anyhow!(
                "historical correction prefix has unrelated protected work"
            ));
        }
        if !lineage.contains(&id) {
            if !pre_dispatch_failure(run) {
                return Err(anyhow!(
                    "correction prefix contains an unrelated source generation"
                ));
            }
            Box::pin(historical_inner_tx(tx, item, id, seen))
                .await?
                .context("pre-dispatch generation omitted its historical correction proof")?;
        }
    }
    authority.contract_revision_id = revision.id;
    authority.previous_contract_sha256 = digest(&revision.previous_contract)?;
    authority.replacement_contract_sha256 = digest(&revision.contract)?;
    authority.previous_policy_sha256 = digest(&revision.previous_policy)?;
    authority.replacement_policy_sha256 = digest(&revision.policy)?;
    authority.factory_policy_sha256 = digest(&item.policy)?;
    authority.prefix_run_ids = prefix_ids;
    Ok(authority)
}

pub(super) async fn validate_admission_suffix_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    source: Uuid,
) -> Result<()> {
    let suffix = sqlx::query(
        "SELECT run.* FROM runs run JOIN runs source
           ON source.corp_id=run.corp_id AND source.workspace_run_id=run.workspace_run_id
         WHERE source.corp_id=$1 AND source.id=$2
           AND (run.created_at,run.id)>(source.created_at,source.id)
         ORDER BY run.created_at,run.id LIMIT 65",
    )
    .bind(item.corp_id)
    .bind(source)
    .fetch_all(&mut **tx)
    .await?;
    if suffix.len() > 64 {
        return Err(anyhow!("correction source suffix exceeds its native bound"));
    }
    for run in suffix {
        if !pre_dispatch_failure(&run) {
            return Err(anyhow!("correction source has a newer unproven generation"));
        }
        historical_tx(tx, item, run.get("id"))
            .await?
            .context("failed pre-dispatch generation has no historical correction proof")?;
    }
    Ok(())
}

pub(super) async fn previous_context_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    replacement_id: Uuid,
    contract: &TaskContract,
    policy: &VerificationPolicy,
) -> Result<Option<(TaskContract, VerificationPolicy, Uuid)>> {
    let Some(previous) = historical_tx(tx, item, replacement_id).await? else {
        return Ok(None);
    };
    if serde_json::to_value(&previous.contract)? != serde_json::to_value(contract)?
        || serde_json::to_value(&previous.policy)? != serde_json::to_value(policy)?
    {
        return Err(anyhow!(
            "historical correction contract or policy chain changed"
        ));
    }
    let origin = previous.authority.checkpoint.checkpoint.run_id;
    Ok(Some((
        previous.previous_contract,
        previous.previous_policy,
        origin,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verifier_checkpoint() -> SourceWorkspaceCheckpoint {
        SourceWorkspaceCheckpoint {
            status: "failed".to_owned(),
            execution_mode: "verification_only".to_owned(),
            source_correction_recovery: false,
            verification_status: "failed".to_owned(),
            workspace_path: Some("owned-fixture".to_owned()),
            disposition: Some("preserved".to_owned()),
            fingerprint: Some("b".repeat(64)),
            expected_verifier_fingerprint: Some("b".repeat(64)),
            expected_head_commit: Some("a".repeat(40)),
        }
    }

    #[test]
    fn issue224_legacy_verifier_seal_distinguishes_absence_from_invalid_explicit_head() {
        let checkpoint = verifier_checkpoint();
        assert!(source_seal_head_matches(&checkpoint, &json!({})));
        assert!(source_seal_head_matches(
            &checkpoint,
            &json!({"head_commit":"a".repeat(40)})
        ));
        for head in [Value::Null, json!("c".repeat(40)), json!(42), json!([])] {
            assert!(!source_seal_head_matches(
                &checkpoint,
                &json!({"head_commit":head})
            ));
        }
        assert_eq!(checkpoint.expected_head_commit, Some("a".repeat(40)));
    }

    #[test]
    fn issue224_legacy_verifier_seal_requires_verifier_mode_and_exact_guard() {
        let mut checkpoint = verifier_checkpoint();
        checkpoint.execution_mode = "provider".to_owned();
        assert!(!source_seal_head_matches(&checkpoint, &json!({})));
        checkpoint.execution_mode = "verification_only".to_owned();
        for fingerprint in [None, Some(String::new()), Some("c".repeat(64))] {
            checkpoint.expected_verifier_fingerprint = fingerprint;
            assert!(!source_seal_head_matches(&checkpoint, &json!({})));
        }
        checkpoint.expected_verifier_fingerprint = Some("b".repeat(64));
        checkpoint.fingerprint = None;
        assert!(!source_seal_head_matches(&checkpoint, &json!({})));

        // An ordinary failed provider with no authorized/exported HEAD keeps
        // its existing nullable-head behavior; this adds no new requirement.
        checkpoint.execution_mode = "provider".to_owned();
        checkpoint.expected_verifier_fingerprint = None;
        checkpoint.expected_head_commit = None;
        assert!(source_seal_head_matches(&checkpoint, &json!({})));
    }
}
