//! Current-authority collection of an existing, sealed historical adapter receipt.
//! The server/runner validate native bytes. This module binds metadata and the
//! first newly observed digest; it never asserts prior artifact acceptance.
use super::*;
use crony_domain::{
    MAX_RETAINED_PROVIDER_RECEIPT_BYTES, RETAINED_COPILOT_RECEIPT_FILE,
    RetainedProviderReceiptGrant, StoppedSourceCheckpoint, retained_provider_receipt_metadata,
};
use sqlx::postgres::PgRow;
use std::collections::HashSet;

const GRANT: &str = "retained_provider_receipt";
const UPLOAD: &str = "retained_provider_receipt_upload";

pub(super) struct Admission<'a> {
    pub item: &'a FactoryWorkItem,
    pub authority: &'a budget_checkpoint::Authority,
    pub source_run_id: Uuid,
    pub run_id: Uuid,
    pub command_id: Uuid,
    pub actor_id: Uuid,
    pub fingerprint: &'a str,
    pub head: Option<&'a str>,
}

fn denied(detail: &str) -> anyhow::Error {
    anyhow!("retained provider receipt: {detail}")
}

fn digest(value: &impl serde::Serialize) -> Result<String> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(value)?)))
}

pub(super) fn database_error(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<sqlx::Error>()
        .is_some_and(|error| !matches!(error, sqlx::Error::RowNotFound))
}

pub(super) fn has_collection(payload: &Value) -> bool {
    // Presence marks the collection path. A malformed/null grant must not
    // downgrade to ordinary verifier dispatch; ordinary commands omit the key.
    payload.get(GRANT).is_some() || payload.get(UPLOAD).is_some()
}

fn upload_binding(payload: &Value) -> Result<Option<&Value>> {
    let Some(binding) = payload.get(UPLOAD) else {
        return Ok(None);
    };
    if !binding.as_object().is_some_and(|object| object.len() == 2)
        || !binding
            .get("sha256")
            .and_then(Value::as_str)
            .is_some_and(valid_sha256)
        || !binding
            .get("bytes")
            .and_then(Value::as_i64)
            .is_some_and(|bytes| bytes > 0 && bytes <= MAX_RETAINED_PROVIDER_RECEIPT_BYTES as i64)
    {
        return Err(denied("stored first-upload binding is malformed"));
    }
    Ok(Some(binding))
}

fn executable_payload(payload: &Value) -> Result<Value> {
    upload_binding(payload)?;
    let mut payload = payload.clone();
    payload
        .as_object_mut()
        .context("retained receipt command is not an object")?
        .remove(UPLOAD);
    Ok(payload)
}

async fn lock_author_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    actor_id: Uuid,
) -> Result<()> {
    sqlx::query("SELECT id FROM actors WHERE corp_id=$1 AND id=$2 FOR SHARE")
        .bind(item.corp_id)
        .bind(actor_id)
        .fetch_optional(&mut **tx)
        .await?
        .context("retained receipt author no longer exists")?;
    ensure_factory_recovery_authorizer_tx(
        tx,
        item.corp_id,
        item.mission_id.context("retained receipt has no mission")?,
        actor_id,
    )
    .await
}

/// Only verifier ingestion uses these gates; ordinary output/usage/provider
/// uploads do not acquire them. Hints are revalidated after acquiring the gates.
pub(super) async fn lock_for_run_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    run_id: Uuid,
) -> Result<bool> {
    let scope_query =
        "SELECT run.execution_mode,run.workspace_run_id,mission.requested_by,item.id AS item_id
         FROM runs run JOIN tasks task ON task.id=run.task_id AND task.corp_id=run.corp_id
         JOIN missions mission ON mission.id=task.mission_id AND mission.corp_id=run.corp_id
         LEFT JOIN factory_work_items item ON item.mission_id=mission.id AND item.corp_id=run.corp_id
         WHERE run.corp_id=$1 AND run.id=$2";
    let row = sqlx::query(scope_query)
        .bind(corp_id)
        .bind(run_id)
        .fetch_optional(&mut **tx)
        .await?
        .context("retained receipt assignment was not found")?;
    if row.get::<String, _>("execution_mode") != "verification_only" {
        return Ok(false);
    }
    let item_id = row
        .get::<Option<Uuid>, _>("item_id")
        .context("verifier evidence requires an explicit Factory collection")?;
    let mut keys = budget_scope_lock_keys(corp_id, row.get("requested_by"));
    keys.extend([
        format!("factory:item:{corp_id}:{item_id}"),
        format!("publication:factory:{corp_id}:{item_id}"),
        format!(
            "resume:workspace:{corp_id}:{}",
            row.get::<Uuid, _>("workspace_run_id")
        ),
    ]);
    lock_factory_keys_tx(tx, &keys).await?;
    let current = sqlx::query(scope_query)
        .bind(corp_id)
        .bind(run_id)
        .fetch_optional(&mut **tx)
        .await?
        .context("retained receipt assignment disappeared while acquiring its gates")?;
    if current.get::<String, _>("execution_mode") != "verification_only"
        || current.get::<Uuid, _>("requested_by") != row.get::<Uuid, _>("requested_by")
        || current.get::<Uuid, _>("workspace_run_id") != row.get::<Uuid, _>("workspace_run_id")
        || current.get::<Option<Uuid>, _>("item_id") != Some(item_id)
    {
        return Err(denied(
            "collection scope changed while acquiring its native gates",
        ));
    }
    Ok(true)
}

async fn run_tx(tx: &mut Transaction<'_, Postgres>, corp_id: Uuid, run_id: Uuid) -> Result<PgRow> {
    sqlx::query(
        "SELECT run.*,task.mission_id,mission.room_id,task.required_adapter,
                task.contract,task.verification_policy,agent.adapter AS current_adapter
         FROM runs run JOIN tasks task ON task.id=run.task_id AND task.corp_id=run.corp_id
         JOIN missions mission ON mission.id=task.mission_id AND mission.corp_id=run.corp_id
         JOIN agents agent ON agent.id=run.agent_id AND agent.corp_id=run.corp_id
         WHERE run.corp_id=$1 AND run.id=$2",
    )
    .bind(corp_id)
    .bind(run_id)
    .fetch_optional(&mut **tx)
    .await?
    .context("retained receipt source lineage is missing")
}

async fn seal_tx(tx: &mut Transaction<'_, Postgres>, corp_id: Uuid, run: &PgRow) -> Result<PgRow> {
    let row = sqlx::query(
        "SELECT id,seq,type,payload,room_id,correlation_id FROM events
         WHERE corp_id=$1 AND aggregate_type='run' AND aggregate_id=$2
           AND type IN ('run.workspace_preserved','run.workspace_removed','run.teardown_uncertain')
         ORDER BY seq DESC LIMIT 1",
    )
    .bind(corp_id)
    .bind(run.get::<Uuid, _>("id"))
    .fetch_optional(&mut **tx)
    .await?
    .context("retained receipt lacks a native source seal")?;
    let payload: Value = row.get("payload");
    if row.get::<String, _>("type") != "run.workspace_preserved"
        || row.get::<Option<Uuid>, _>("room_id") != Some(run.get("room_id"))
        || row.get::<Option<Uuid>, _>("correlation_id") != Some(run.get("mission_id"))
        || run
            .get::<Option<String>, _>("workspace_disposition")
            .as_deref()
            != Some("preserved")
        || payload.get("workspace_fingerprint").and_then(Value::as_str)
            != run
                .get::<Option<String>, _>("workspace_fingerprint")
                .as_deref()
        || !payload
            .get("workspace_fingerprint")
            .and_then(Value::as_str)
            .is_some_and(valid_sha256)
        || payload
            .get("workspace_quarantined")
            .and_then(Value::as_bool)
            == Some(true)
    {
        return Err(denied("native source seal changed or is not preserved"));
    }
    Ok(row)
}

fn same_lineage(source: &PgRow, candidate: &PgRow) -> bool {
    ["task_id", "agent_id", "workspace_run_id"]
        .into_iter()
        .all(|field| source.get::<Uuid, _>(field) == candidate.get::<Uuid, _>(field))
        && source.get::<String, _>("runner_id") == candidate.get::<String, _>("runner_id")
        && source.get::<Option<Uuid>, _>("workspace_connection_id")
            == candidate.get::<Option<Uuid>, _>("workspace_connection_id")
        && [
            "source_repository",
            "source_base_ref",
            "source_base_commit",
            "workspace_base_commit",
            "workspace_branch",
        ]
        .into_iter()
        .all(|field| {
            source.get::<Option<String>, _>(field) == candidate.get::<Option<String>, _>(field)
        })
}

/// Reconstruct only a persisted #210 historical transition. This does not
/// reauthorize its provider or waive the current checkpoint's budget boundary.
/// The original immutable revision supplies the older proof's policy, while the
/// caller continues to validate today's collection policy independently.
async fn previous_correction_context_tx(
    tx: &mut Transaction<'_, Postgres>,
    item: &FactoryWorkItem,
    replacement: &PgRow,
    contract: &TaskContract,
    policy: &VerificationPolicy,
) -> Result<Option<(TaskContract, VerificationPolicy, Uuid)>> {
    let correction = sqlx::query(
        "SELECT factory_work_item_id,mission_id,task_id,source_run_id,authorized_by,
                contract_revision_id,status,previous_verification_policy,
                replacement_verification_policy,source_correction_authority
         FROM factory_verification_recoveries
         WHERE corp_id=$1 AND replacement_run_id=$2 AND mode='source_correction'",
    )
    .bind(item.corp_id)
    .bind(replacement.get::<Uuid, _>("id"))
    .fetch_optional(&mut **tx)
    .await?;
    let Some(correction) = correction else {
        return Ok(None);
    };
    let Some(saved) = correction.get::<Option<Value>, _>("source_correction_authority") else {
        // An ordinary correction grants no exception for an older suspension.
        return Ok(None);
    };
    let authority: checkpoint_correction::Authority = serde_json::from_value(saved.clone())
        .context("historical source-correction authority is malformed")?;
    let source_id: Uuid = correction.get("source_run_id");
    if correction.get::<Uuid, _>("factory_work_item_id") != item.id
        || Some(correction.get::<Uuid, _>("mission_id")) != item.mission_id
        || correction.get::<Uuid, _>("task_id") != replacement.get::<Uuid, _>("task_id")
        || correction.get::<String, _>("status") != "failed"
        || Some(source_id) != replacement.get::<Option<Uuid>, _>("resumed_from_run_id")
        || correction.get::<Option<Uuid>, _>("contract_revision_id")
            != Some(authority.contract_revision_id)
        || replacement.get::<Option<Uuid>, _>("workspace_connection_id")
            != authority.workspace_connection_id
        || replacement
            .get::<Option<String>, _>("provider_session_id")
            .as_ref()
            != Some(&authority.provider_session_id)
        || replacement.get::<Option<String>, _>("model") != contract.model
        || replacement.get::<Option<String>, _>("reasoning_effort") != contract.reasoning_effort
    {
        return Err(denied(
            "historical correction does not match its exact replacement",
        ));
    }
    let revision = sqlx::query(
        "SELECT revision.previous_contract,revision.replacement_contract,
                revision.previous_verification_policy,revision.replacement_verification_policy,
                checkpoint.id AS checkpoint_id,checkpoint.checkpoint_authority
         FROM mission_contract_revisions revision
         JOIN factory_verification_recoveries checkpoint
           ON checkpoint.replacement_run_id=revision.source_run_id
          AND checkpoint.corp_id=revision.corp_id AND checkpoint.task_id=revision.task_id
          AND checkpoint.mission_id=revision.mission_id
          AND checkpoint.factory_work_item_id=$4
          AND checkpoint.mode='checkpoint_verification' AND checkpoint.status='failed'
         JOIN runs source ON source.id=revision.source_run_id AND source.corp_id=revision.corp_id
          AND source.task_id=revision.task_id AND source.execution_mode='verification_only'
          AND source.status='failed' AND source.verification_status='failed'
         WHERE revision.corp_id=$1 AND revision.id=$2 AND revision.source_run_id=$3
           AND revision.revised_by=$5 AND revision.task_id=$6 AND revision.mission_id=$7
           AND revision.next_action='resume'",
    )
    .bind(item.corp_id)
    .bind(authority.contract_revision_id)
    .bind(source_id)
    .bind(item.id)
    .bind(correction.get::<Uuid, _>("authorized_by"))
    .bind(replacement.get::<Uuid, _>("task_id"))
    .bind(item.mission_id)
    .fetch_optional(&mut **tx)
    .await?
    .context("historical correction has no exact immutable Resume revision")?;
    let previous_contract: TaskContract =
        serde_json::from_value(revision.get("previous_contract"))?;
    let previous_policy: VerificationPolicy =
        serde_json::from_value(revision.get("previous_verification_policy"))?;
    if revision.get::<Value, _>("replacement_contract") != serde_json::to_value(contract)?
        || revision.get::<Value, _>("replacement_verification_policy")
            != serde_json::to_value(policy)?
        || correction.get::<Value, _>("previous_verification_policy")
            != serde_json::to_value(&previous_policy)?
        || correction.get::<Value, _>("replacement_verification_policy")
            != serde_json::to_value(policy)?
        || previous_contract.workspace_connection_id != authority.workspace_connection_id
        || contract.workspace_connection_id != authority.workspace_connection_id
    {
        return Err(denied(
            "historical correction contract or policy chain changed",
        ));
    }
    ensure_factory_recovery_verification_policy_not_weakened(&previous_policy, policy)?;
    let proof = budget_checkpoint::source_authority_with_contract_tx(
        tx,
        item.corp_id,
        item.id,
        source_id,
        Some((&previous_contract, &previous_policy)),
    )
    .await?;
    if revision.get::<Option<Value>, _>("checkpoint_authority")
        != Some(serde_json::to_value(&proof)?)
    {
        return Err(denied("historical failed checkpoint proof changed"));
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
    let prefix = sqlx::query(
        "SELECT run.id,run.breaker_stage,run.workspace_disposition
         FROM runs run JOIN runs boundary ON boundary.id=$3 AND boundary.corp_id=run.corp_id
         WHERE run.corp_id=$1 AND run.workspace_run_id=$2
           AND (run.created_at,run.id)<=(boundary.created_at,boundary.id)
         ORDER BY run.created_at,run.id LIMIT 65",
    )
    .bind(item.corp_id)
    .bind(proof.checkpoint.workspace_run_id)
    .bind(source_id)
    .fetch_all(&mut **tx)
    .await?;
    let prefix_ids: Vec<Uuid> = prefix.iter().map(|row| row.get("id")).collect();
    if prefix_ids.is_empty()
        || prefix_ids.len() > 64
        || prefix_ids.last() != Some(&source_id)
        || prefix.iter().any(|row| {
            row.get::<Option<String>, _>("workspace_disposition")
                .as_deref()
                == Some("quarantined")
                || row.get::<String, _>("breaker_stage") == "stop"
                || (row.get::<String, _>("breaker_stage") == "suspend"
                    && row.get::<Uuid, _>("id") != proof.checkpoint.run_id)
        })
    {
        return Err(denied(
            "historical correction prefix has unrelated protected work",
        ));
    }
    let origin_id = proof.checkpoint.run_id;
    let reconstructed = checkpoint_correction::Authority {
        schema_version: 1,
        checkpoint_recovery_id: revision.get("checkpoint_id"),
        checkpoint: proof,
        suspension_incident_id: origin.get("id"),
        suspension_input_sha256: digest(&origin.get::<Value, _>("input"))?,
        contract_revision_id: authority.contract_revision_id,
        previous_contract_sha256: digest(&previous_contract)?,
        replacement_contract_sha256: digest(contract)?,
        previous_policy_sha256: digest(&previous_policy)?,
        replacement_policy_sha256: digest(policy)?,
        factory_policy_sha256: digest(&item.policy)?,
        workspace_connection_id: origin.get("workspace_connection_id"),
        provider_session_id: origin
            .get::<Option<String>, _>("provider_session_id")
            .context("historical correction origin lost its provider session")?,
        prefix_run_ids: prefix_ids,
    };
    if serde_json::to_value(reconstructed)? != saved {
        return Err(denied(
            "historical correction no longer matches its persisted authority",
        ));
    }
    Ok(Some((previous_contract, previous_policy, origin_id)))
}

pub(super) async fn derive_tx(
    tx: &mut Transaction<'_, Postgres>,
    input: Admission<'_>,
) -> Result<Option<RetainedProviderReceiptGrant>> {
    let source = run_tx(tx, input.item.corp_id, input.source_run_id).await?;
    if source
        .get::<Option<String>, _>("required_adapter")
        .as_deref()
        != Some("github-copilot")
        || source.get::<String, _>("current_adapter") != "github-copilot"
    {
        return Ok(None);
    }
    // Absence of a ready join is NOT permission to replace a damaged or expired
    // signed artifact. Preserve the existing reuse/fail-closed path verbatim.
    if source.get::<Option<Uuid>, _>("artifact_id").is_some()
        || [
            "artifact_uri",
            "artifact_signature",
            "artifact_path",
            "artifact_sha256",
            "artifact_media_type",
        ]
        .into_iter()
        .any(|field| source.get::<Option<String>, _>(field).is_some())
    {
        return Ok(None);
    }
    let existing: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM artifacts artifact
         WHERE artifact.run_id=$1 AND artifact.corp_id=$2
            AND artifact.artifact_role='provider_evidence')",
    )
    .bind(input.source_run_id)
    .bind(input.item.corp_id)
    .fetch_one(&mut **tx)
    .await?;
    if existing {
        return Ok(None);
    }
    lock_author_tx(tx, input.item, input.actor_id).await?;
    let mut contract: TaskContract = serde_json::from_value(source.get("contract"))?;
    let mut policy: VerificationPolicy = serde_json::from_value(source.get("verification_policy"))?;
    ensure_factory_recovery_policy(input.item, &contract, &policy)?;
    budget_checkpoint::validate_lineage_until_tx(
        tx,
        input.item.corp_id,
        input.item.id,
        source.get("workspace_run_id"),
        input.authority,
        input.source_run_id,
    )
    .await?;
    let source_seal = seal_tx(tx, input.item.corp_id, &source).await?;
    let source_payload: Value = source_seal.get("payload");
    let head = input
        .head
        .context("retained receipt requires an exact current HEAD")?;
    if source_payload.get("head_commit").and_then(Value::as_str) != Some(head)
        || source
            .get::<Option<String>, _>("workspace_fingerprint")
            .as_deref()
            != Some(input.fingerprint)
    {
        return Err(denied("current source HEAD or fingerprint changed"));
    }
    let origin = run_tx(tx, input.item.corp_id, input.authority.checkpoint.run_id).await?;
    let Some(session) = origin.get::<Option<String>, _>("provider_session_id") else {
        return Ok(None);
    };
    if !Uuid::parse_str(&session).is_ok_and(|id| !id.is_nil()) {
        return Ok(None);
    }
    let mut next = Some(input.source_run_id);
    let mut seen = HashSet::new();
    let mut historical = None;
    let mut found_origin = false;
    let mut authorized_suspensions = HashSet::new();
    while let Some(id) = next {
        if seen.len() == 64 || !seen.insert(id) {
            return Err(denied("source ancestry is cyclic or exceeds its bound"));
        }
        let candidate = run_tx(tx, input.item.corp_id, id).await?;
        if !same_lineage(&source, &candidate)
            || !matches!(
                candidate.get::<String, _>("status").as_str(),
                "failed" | "cancelled" | "lost"
            )
            || candidate
                .get::<Option<String>, _>("workspace_disposition")
                .as_deref()
                == Some("quarantined")
            || (id != input.authority.checkpoint.run_id
                && (candidate.get::<String, _>("breaker_stage") == "stop"
                    || (candidate.get::<String, _>("breaker_stage") == "suspend"
                        && !authorized_suspensions.contains(&id))))
        {
            return Err(denied("source ancestry crosses the authorized assignment"));
        }
        found_origin |= id == input.authority.checkpoint.run_id;
        next = candidate.get("resumed_from_run_id");
        if candidate.get::<String, _>("execution_mode") == "provider" {
            if candidate
                .get::<Option<String>, _>("provider_session_id")
                .as_ref()
                != Some(&session)
            {
                return Err(denied("source ancestry changes the provider session"));
            }
            let termination = sqlx::query(
                "SELECT id,seq,payload,room_id,correlation_id FROM events
                 WHERE corp_id=$1 AND aggregate_type='run' AND aggregate_id=$2
                   AND type='run.session_terminated' ORDER BY seq DESC LIMIT 1",
            )
            .bind(input.item.corp_id)
            .bind(id)
            .fetch_optional(&mut **tx)
            .await?;
            if let Some(termination) = termination {
                let payload: Value = termination.get("payload");
                if payload.get("outcome").and_then(Value::as_str) == Some("completed")
                    && historical.is_none()
                {
                    terminal_accounting::validate_termination_tx(
                        tx,
                        input.item.corp_id,
                        candidate.get("task_id"),
                        id,
                        "provider",
                        &payload,
                    )
                    .await?;
                    let seal = seal_tx(tx, input.item.corp_id, &candidate).await?;
                    let seal_payload: Value = seal.get("payload");
                    let proof: StoppedSourceCheckpoint = serde_json::from_value(
                        seal_payload
                            .get("source_checkpoint")
                            .cloned()
                            .context("historical completed generation has no native checkpoint")?,
                    )?;
                    if termination.get::<i64, _>("seq") >= seal.get::<i64, _>("seq")
                        || termination.get::<Option<Uuid>, _>("room_id")
                            != Some(source.get("room_id"))
                        || termination.get::<Option<Uuid>, _>("correlation_id")
                            != Some(source.get("mission_id"))
                        || proof.schema_version != 1
                        || proof.corp_id != input.item.corp_id
                        || proof.mission_id != source.get::<Uuid, _>("mission_id")
                        || proof.task_id != source.get::<Uuid, _>("task_id")
                        || proof.run_id != id
                        || proof.workspace_run_id != source.get::<Uuid, _>("workspace_run_id")
                        || proof.agent_id != source.get::<Uuid, _>("agent_id")
                        || proof.runner_id != source.get::<String, _>("runner_id")
                        || Some(&proof.source_repository) != contract.source_repository.as_ref()
                        || Some(&proof.source_base_ref) != contract.source_base_ref.as_ref()
                        || Some(&proof.source_base_commit) != contract.source_base_commit.as_ref()
                        || proof.source_base_commit != proof.workspace_base_commit
                        || proof.verification_policy_sha256 != digest(&policy)?
                        || proof.write_scope_sha256 != digest(&contract.write_scope)?
                        || proof.deliverable_policy_sha256 != digest(&contract.deliverable)?
                        || Some(&proof.workspace_base_commit)
                            != candidate
                                .get::<Option<String>, _>("workspace_base_commit")
                                .as_ref()
                        || Some(&proof.branch)
                            != candidate
                                .get::<Option<String>, _>("workspace_branch")
                                .as_ref()
                        || Some(&proof.workspace_fingerprint)
                            != candidate
                                .get::<Option<String>, _>("workspace_fingerprint")
                                .as_ref()
                        || seal_payload.get("head_commit").and_then(Value::as_str)
                            != Some(proof.head_commit.as_str())
                    {
                        return Err(denied("historical termination and checkpoint do not agree"));
                    }
                    validate_factory_base_commit(&proof.head_commit)?;
                    historical = Some((
                        id,
                        termination.get::<Uuid, _>("id"),
                        seal.get::<Uuid, _>("id"),
                        candidate.get::<Option<String>, _>("model"),
                        candidate.get::<Option<String>, _>("reasoning_effort"),
                    ));
                }
            }
            if let Some((previous_contract, previous_policy, suspended_origin)) =
                previous_correction_context_tx(tx, input.item, &candidate, &contract, &policy)
                    .await?
            {
                // Applies only when walking behind this exact persisted
                // replacement, never to an unrelated suspended ancestor.
                authorized_suspensions.insert(suspended_origin);
                contract = previous_contract;
                policy = previous_policy;
            }
        } else if candidate.get::<String, _>("execution_mode") != "verification_only"
            || candidate
                .get::<Option<String>, _>("provider_session_id")
                .is_some()
        {
            return Err(denied("source ancestry contains an unproven session"));
        }
    }
    if !found_origin
        || !seen.contains(&source.get::<Uuid, _>("workspace_run_id"))
        || !authorized_suspensions.is_subset(&seen)
    {
        return Err(denied(
            "source ancestry does not contain its native workspace origin",
        ));
    }
    let Some((historical_run_id, termination_id, checkpoint_id, model, reasoning)) = historical
    else {
        return Ok(None);
    };
    let grant = RetainedProviderReceiptGrant {
        schema_version: 1,
        collection_id: input.command_id,
        corp_id: input.item.corp_id,
        task_id: source.get("task_id"),
        run_id: input.run_id,
        workspace_run_id: source.get("workspace_run_id"),
        source_run_id: input.source_run_id,
        checkpoint_run_id: input.authority.checkpoint.run_id,
        provider_session_id: session,
        historical_run_id,
        historical_termination_event_id: termination_id,
        historical_checkpoint_event_id: checkpoint_id,
        source_checkpoint_event_id: source_seal.get("id"),
        expected_workspace_fingerprint: input.fingerprint.to_owned(),
        expected_head_commit: head.to_owned(),
        historical_model: model,
        historical_reasoning_effort: reasoning,
    };
    grant.validate().map_err(denied)?;
    Ok(Some(grant))
}

async fn current_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    run_id: Uuid,
    dispatch: Option<&PendingRunnerCommand>,
) -> Result<(RetainedProviderReceiptGrant, Value)> {
    let identity_query =
        "SELECT command.id,command.payload,command.status,command.runner_id,
                recovery.factory_work_item_id,recovery.authorized_by,recovery.source_run_id,
                recovery.request,recovery.checkpoint_authority,to_jsonb(recovery) AS recovery_identity
         FROM runner_commands command
         JOIN factory_verification_recoveries recovery
           ON recovery.replacement_run_id=command.run_id AND recovery.corp_id=command.corp_id
         WHERE command.corp_id=$1 AND command.run_id=$2
           AND command.command_kind='factory_verification_recovery'
           AND recovery.mode='checkpoint_verification'
           AND recovery.status IN ('authorized','running')";
    let record = sqlx::query(identity_query)
        .bind(corp_id)
        .bind(run_id)
        .fetch_optional(&mut **tx)
        .await?
        .context("retained receipt has no active native checkpoint command")?;
    let payload: Value = record.get("payload");
    let grant: RetainedProviderReceiptGrant = serde_json::from_value(
        payload
            .get(GRANT)
            .filter(|value| !value.is_null())
            .cloned()
            .context("verifier provider evidence requires a retained receipt grant")?,
    )
    .context("retained receipt grant is malformed")?;
    grant.validate().map_err(denied)?;
    let executable = executable_payload(&payload)?;
    let command_id: Uuid = record.get("id");
    if grant.collection_id != command_id
        || grant.run_id != run_id
        || grant.corp_id != corp_id
        || grant.source_run_id != record.get::<Uuid, _>("source_run_id")
        || payload.get(GRANT) != Some(&serde_json::to_value(&grant)?)
        || !matches!(
            record.get::<String, _>("status").as_str(),
            "pending" | "dispatched"
        )
    {
        return Err(denied("collection does not match its exact command"));
    }
    if let Some(command) = dispatch
        && (record.get::<String, _>("status") != "pending"
            || command.id != command_id
            || command.run_id != run_id
            || command.corp_id != corp_id
            || command.runner_id != record.get::<String, _>("runner_id")
            || command.command_kind != "factory_verification_recovery"
            || executable_payload(&command.payload)? != executable)
    {
        return Err(denied(
            "collection dispatch is not the exact pending command",
        ));
    }
    let (item, _) = factory_work_item_tx(tx, corp_id, record.get("factory_work_item_id"), true)
        .await?
        .context("retained receipt Factory item is missing")?;
    if !matches!(
        item.state,
        FactoryWorkItemState::Running | FactoryWorkItemState::AwaitingApproval
    ) {
        return Err(denied("Factory collection authority is no longer active"));
    }
    lock_author_tx(tx, &item, record.get("authorized_by")).await?;
    let row = sqlx::query(
        "SELECT run.task_id,run.agent_id,run.runner_id,run.assignment_token,run.workspace_run_id,
                run.workspace_connection_id,run.source_repository,run.source_base_ref,run.source_base_commit,
                task.contract,task.verification_policy,mission.room_id,mission.id AS mission_id,
                recovery.previous_verification_policy,recovery.replacement_verification_policy
         FROM runs run JOIN tasks task ON task.id=run.task_id AND task.corp_id=run.corp_id
         JOIN missions mission ON mission.id=task.mission_id AND mission.corp_id=run.corp_id
         JOIN agents agent ON agent.id=run.agent_id AND agent.corp_id=run.corp_id
         JOIN factory_verification_recoveries recovery ON recovery.replacement_run_id=run.id
              AND recovery.corp_id=run.corp_id AND recovery.task_id=task.id AND recovery.mission_id=mission.id
         WHERE run.corp_id=$1 AND run.id=$2
           AND run.execution_mode='verification_only' AND run.provider_session_id IS NULL
           AND run.model IS NULL AND run.reasoning_effort IS NULL
           AND run.budget_tokens_limit=0 AND run.budget_cost_microusd_limit=0
           AND run.input_tokens=0 AND run.output_tokens=0 AND run.cost_microusd=0
           AND run.breaker_stage NOT IN ('suspend','stop')
           AND run.workspace_disposition IS DISTINCT FROM 'quarantined'
           AND run.status IN ('provisioning','starting','running','verifying','waiting_for_input','waiting_for_approval')
            AND task.status IN ('claimed','running','review','awaiting_approval') AND mission.status='running'
            AND task.required_adapter='github-copilot' AND agent.adapter='github-copilot'
            AND task.assigned_agent_id=run.agent_id AND agent.retired_at IS NULL
            AND (agent.current_run_id=run.id OR (
                agent.current_run_id IS NULL
                AND run.status IN ('verifying','waiting_for_input','waiting_for_approval')
                AND task.status IN ('review','awaiting_approval')
                AND EXISTS(SELECT 1 FROM artifacts ready
                           WHERE ready.id=run.artifact_id AND ready.run_id=run.id
                             AND ready.corp_id=run.corp_id AND ready.status='ready'
                             AND ready.artifact_role='provider_evidence')))
            AND run.resumed_from_run_id=$3
           AND recovery.mode='checkpoint_verification' AND recovery.status IN ('authorized','running')
           AND recovery.factory_work_item_id=$4 AND recovery.contract_revision_id IS NULL
         FOR UPDATE OF run,task,mission,agent",
    )
    .bind(corp_id).bind(run_id).bind(grant.source_run_id).bind(item.id)
    .fetch_optional(&mut **tx).await?
    .context("retained receipt collector assignment is no longer active")?;
    let contract: TaskContract = serde_json::from_value(row.get("contract"))?;
    let policy: Value = row.get("verification_policy");
    let request: Value = record.get("request");
    for (field, expected) in [
        ("mode", json!("checkpoint_verification")),
        ("adapter", json!("github-copilot")),
        ("corp_id", json!(corp_id)),
        ("run_id", json!(run_id)),
        ("task_id", json!(row.get::<Uuid, _>("task_id"))),
        ("mission_id", json!(row.get::<Uuid, _>("mission_id"))),
        ("room_id", json!(row.get::<Uuid, _>("room_id"))),
        ("agent_id", json!(row.get::<Uuid, _>("agent_id"))),
        (
            "assignment_token",
            json!(row.get::<Uuid, _>("assignment_token")),
        ),
        (
            "workspace_run_id",
            json!(row.get::<Uuid, _>("workspace_run_id")),
        ),
        (
            "workspace_connection_id",
            json!(row.get::<Option<Uuid>, _>("workspace_connection_id")),
        ),
        ("source_run_id", json!(grant.source_run_id)),
        (
            "source_repository",
            json!(row.get::<Option<String>, _>("source_repository")),
        ),
        (
            "source_base_ref",
            json!(row.get::<Option<String>, _>("source_base_ref")),
        ),
        (
            "source_base_commit",
            json!(row.get::<Option<String>, _>("source_base_commit")),
        ),
        ("verification_policy", policy.clone()),
        ("write_scope", json!(contract.write_scope)),
        ("deliverable", json!(contract.deliverable)),
        ("secret_refs", json!([])),
        ("provider_session_id", Value::Null),
        ("model", Value::Null),
        ("reasoning_effort", Value::Null),
        ("provider_artifact", Value::Null),
        (
            "expected_workspace_fingerprint",
            json!(grant.expected_workspace_fingerprint),
        ),
        ("expected_head_commit", json!(grant.expected_head_commit)),
    ] {
        if payload.get(field) != Some(&expected) {
            return Err(denied(
                "collection command or current task authority changed",
            ));
        }
    }
    if policy != row.get::<Value, _>("previous_verification_policy")
        || policy != row.get::<Value, _>("replacement_verification_policy")
        || request.get("expected_workspace_fingerprint")
            != payload.get("expected_workspace_fingerprint")
        || request.get("expected_head_commit") != payload.get("expected_head_commit")
        || request.get("source_run_id") != Some(&json!(grant.source_run_id))
        || request.get("mode") != Some(&json!("checkpoint_verification"))
        || request.get("work_item_id") != Some(&json!(item.id))
        || request.get("observed_source_revision") != Some(&json!(item.source_revision))
        || contract.source_repository != row.get::<Option<String>, _>("source_repository")
        || contract.source_base_ref != row.get::<Option<String>, _>("source_base_ref")
        || contract.source_base_commit != row.get::<Option<String>, _>("source_base_commit")
        || contract.workspace_connection_id != row.get::<Option<Uuid>, _>("workspace_connection_id")
        || row.get::<Uuid, _>("task_id") != grant.task_id
        || row.get::<Uuid, _>("workspace_run_id") != grant.workspace_run_id
        || row.get::<String, _>("runner_id") != record.get::<String, _>("runner_id")
    {
        return Err(denied("collection policy or source generation changed"));
    }
    sqlx::query(
        "SELECT id FROM runner_nodes WHERE corp_id=$1 AND id=$2 AND status='connected' FOR SHARE",
    )
    .bind(corp_id)
    .bind(row.get::<String, _>("runner_id"))
    .fetch_optional(&mut **tx)
    .await?
    .context("retained receipt collector runner is no longer connected")?;
    let connection = workspace_connections::validate_run_connection_tx(tx, corp_id, run_id).await?;
    if connection != row.get::<Option<Uuid>, _>("workspace_connection_id") {
        return Err(denied("collection connection changed"));
    }
    let latest: Uuid = sqlx::query_scalar(
        "SELECT id FROM runs WHERE corp_id=$1 AND workspace_run_id=$2
         ORDER BY created_at DESC,id DESC LIMIT 1",
    )
    .bind(corp_id)
    .bind(grant.workspace_run_id)
    .fetch_one(&mut **tx)
    .await?;
    if latest != run_id {
        return Err(denied("collector generation was superseded"));
    }
    let authority =
        budget_checkpoint::source_authority_tx(tx, corp_id, item.id, grant.source_run_id).await?;
    if record.get::<Option<Value>, _>("checkpoint_authority")
        != Some(serde_json::to_value(&authority)?)
    {
        return Err(denied("native checkpoint authority changed"));
    }
    let current = derive_tx(
        tx,
        Admission {
            item: &item,
            authority: &authority,
            source_run_id: grant.source_run_id,
            run_id,
            command_id,
            actor_id: record.get("authorized_by"),
            fingerprint: &grant.expected_workspace_fingerprint,
            head: Some(&grant.expected_head_commit),
        },
    )
    .await?
    .context("retained receipt is no longer eligible")?;
    if current != grant {
        return Err(denied("historical collection grant changed"));
    }
    let source = run_tx(tx, corp_id, grant.source_run_id).await?;
    if payload.get("workspace_base_commit")
        != Some(&json!(
            source.get::<Option<String>, _>("workspace_base_commit")
        ))
    {
        return Err(denied("collection workspace base changed"));
    }
    // The command can be acknowledged while connection/source reads wait. Hold
    // its row only after run locks, and repeat status/payload before committing.
    let command = sqlx::query(&format!("{identity_query} FOR SHARE OF command,recovery"))
        .bind(corp_id)
        .bind(run_id)
        .fetch_optional(&mut **tx)
        .await?
        .context("retained receipt command or recovery changed during validation")?;
    let status: String = command.get("status");
    if (dispatch.is_some() && status != "pending")
        || (dispatch.is_none() && !matches!(status.as_str(), "pending" | "dispatched"))
        || command.get::<Uuid, _>("id") != command_id
        || command.get::<String, _>("runner_id") != record.get::<String, _>("runner_id")
        || command.get::<Value, _>("recovery_identity")
            != record.get::<Value, _>("recovery_identity")
        || executable_payload(&command.get::<Value, _>("payload"))? != executable
    {
        return Err(denied("collection command changed during validation"));
    }
    Ok((grant, command.get("payload")))
}

pub(super) async fn validate_artifact_tx(
    tx: &mut Transaction<'_, Postgres>,
    artifact: &StoredArtifact,
    reserve: bool,
) -> Result<()> {
    let (grant, payload) = current_tx(tx, artifact.corp_id, artifact.run_id, None).await?;
    validate_binding_tx(tx, artifact, reserve, &grant, &payload).await
}

async fn validate_binding_tx(
    tx: &mut Transaction<'_, Postgres>,
    artifact: &StoredArtifact,
    reserve: bool,
    grant: &RetainedProviderReceiptGrant,
    payload: &Value,
) -> Result<()> {
    if artifact.artifact_role != "provider_evidence"
        || artifact.task_id != grant.task_id
        || artifact.file_name != RETAINED_COPILOT_RECEIPT_FILE
        || artifact.media_type != "application/json"
        || artifact.bytes <= 0
        || artifact.bytes > MAX_RETAINED_PROVIDER_RECEIPT_BYTES as i64
        || !valid_sha256(&artifact.sha256)
        || artifact.retention_until <= Utc::now()
        || artifact.provenance_signature.is_empty()
        || artifact.metadata != retained_provider_receipt_metadata(grant)
    {
        return Err(denied(
            "collected artifact metadata does not match its exact grant",
        ));
    }
    let assignment = run_tx(tx, artifact.corp_id, artifact.run_id).await?;
    if artifact.producer_agent_id != assignment.get::<Uuid, _>("agent_id")
        || artifact.producer_runner_id != assignment.get::<String, _>("runner_id")
    {
        return Err(denied("collected artifact producer changed"));
    }
    let binding = json!({"sha256": artifact.sha256, "bytes": artifact.bytes});
    if let Some(first) = upload_binding(payload)? {
        if first != &binding {
            return Err(denied(
                "collection cannot replace its first observed digest or byte count",
            ));
        }
    } else if reserve {
        let changed = sqlx::query(
            "UPDATE runner_commands SET payload=jsonb_set(payload,'{retained_provider_receipt_upload}',$3)
             WHERE corp_id=$1 AND id=$2 AND NOT (payload ? 'retained_provider_receipt_upload')",
        ).bind(artifact.corp_id).bind(grant.collection_id).bind(&binding).execute(&mut **tx).await?;
        if changed.rows_affected() != 1 {
            return Err(denied(
                "collection first-upload binding changed during reservation",
            ));
        }
    } else {
        return Err(denied("collected artifact has no first-upload binding"));
    }
    let conflict: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM artifacts WHERE corp_id=$1 AND run_id=$2
          AND artifact_role='provider_evidence'
          AND (sha256<>$3 OR bytes<>$4 OR metadata<>$5::jsonb OR status='rejected'))",
    )
    .bind(artifact.corp_id)
    .bind(artifact.run_id)
    .bind(&artifact.sha256)
    .bind(artifact.bytes)
    .bind(&artifact.metadata)
    .fetch_one(&mut **tx)
    .await?;
    if conflict {
        return Err(denied(
            "collection conflicts with an existing or rejected upload",
        ));
    }
    Ok(())
}

async fn validate_upload_input_tx(
    tx: &mut Transaction<'_, Postgres>,
    input: &RunnerEventInput,
    grant: &RetainedProviderReceiptGrant,
) -> Result<()> {
    let metadata = retained_provider_receipt_metadata(grant);
    if input.event_type != "run.artifact_upload"
        || input.corp_id != grant.corp_id
        || input.run_id != grant.run_id
        || input.payload.get(GRANT) != metadata.get(GRANT)
        || input.payload.get("workspace_relative_path") != metadata.get("workspace_relative_path")
        || input.payload.get("file_name").and_then(Value::as_str)
            != Some(RETAINED_COPILOT_RECEIPT_FILE)
        || input.payload.get("artifact_role").and_then(Value::as_str) != Some("provider_evidence")
        || input.payload.get("media_type").and_then(Value::as_str) != Some("application/json")
        || input.payload.get(UPLOAD).is_some()
    {
        return Err(denied(
            "upload does not contain the exact canonical collection metadata",
        ));
    }
    let assignment = sqlx::query(
        "SELECT run.id FROM runs run JOIN runner_nodes node
            ON node.id=run.runner_id AND node.corp_id=run.corp_id
         WHERE run.corp_id=$1 AND run.id=$2 AND run.runner_id=$3 AND run.agent_id=$4
           AND run.assignment_token=$5 AND node.connection_epoch=$6 AND node.status='connected'
           AND run.task_id=$7 FOR SHARE OF node",
    )
    .bind(input.corp_id)
    .bind(input.run_id)
    .bind(&input.runner_id)
    .bind(input.agent_id)
    .bind(input.assignment_token)
    .bind(input.connection_epoch)
    .bind(grant.task_id)
    .fetch_optional(&mut **tx)
    .await?;
    if assignment.is_none() {
        return Err(denied(
            "upload does not match the current collector assignment",
        ));
    }
    Ok(())
}

/// This fence is taken before the existing artifact/run row locks. It applies
/// only to provider evidence, never the high-frequency output or usage paths.
pub(super) async fn fence_artifact_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    run_id: Uuid,
    role: &str,
    metadata: &Value,
) -> Result<bool> {
    if role != "provider_evidence" {
        if metadata.get(GRANT).is_some() {
            return Err(denied("a retained receipt must be provider evidence"));
        }
        return Ok(false);
    }
    let collector = lock_for_run_tx(tx, corp_id, run_id).await?;
    if !collector && metadata.get(GRANT).is_some() {
        return Err(denied(
            "retained evidence cannot be attributed to a provider run",
        ));
    }
    Ok(collector)
}

pub(super) async fn prepare_tx(
    tx: &mut Transaction<'_, Postgres>,
    input: &RunnerEventInput,
    artifact: &StoredArtifact,
) -> Result<bool> {
    let collector = fence_artifact_tx(
        tx,
        input.corp_id,
        input.run_id,
        &artifact.artifact_role,
        &artifact.metadata,
    )
    .await?;
    if collector {
        let (grant, payload) = current_tx(tx, input.corp_id, input.run_id, None).await?;
        validate_upload_input_tx(tx, input, &grant).await?;
        validate_binding_tx(tx, artifact, true, &grant, &payload).await?;
    } else if input.payload.get(GRANT).is_some() || input.payload.get(UPLOAD).is_some() {
        return Err(denied(
            "retained-marked upload requires a current collection grant",
        ));
    }
    Ok(collector)
}

/// A definite authority denial must not leave a staged object available for
/// later background adoption. SQLx failures instead roll the transaction back.
pub(super) async fn reject_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    artifact_id: Uuid,
    error: &anyhow::Error,
) -> Result<()> {
    sqlx::query(
        "UPDATE artifacts SET status='rejected',rejection_reason=$3
         WHERE corp_id=$1 AND id=$2 AND status='staged'",
    )
    .bind(corp_id)
    .bind(artifact_id)
    .bind(normalize_artifact_rejection_reason(&error.to_string()))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

impl PgStore {
    /// Read-only, exact pending-command validation. The private upload binding
    /// is the sole non-executable sibling omitted from payload comparison.
    pub async fn retained_provider_receipt_dispatch_authorized(
        &self,
        command: &PendingRunnerCommand,
    ) -> Result<bool> {
        if command.command_kind != "factory_verification_recovery"
            || command.payload.get("mode").and_then(Value::as_str)
                != Some("checkpoint_verification")
        {
            return Ok(false);
        }
        let mut tx = self.pool.begin().await?;
        let result = async {
            if !lock_for_run_tx(&mut tx, command.corp_id, command.run_id).await? {
                return Err(denied("collection dispatch must target a verifier"));
            }
            current_tx(&mut tx, command.corp_id, command.run_id, Some(command)).await?;
            Ok::<(), anyhow::Error>(())
        }
        .await;
        match result {
            Ok(()) => {
                tx.commit().await?;
                Ok(true)
            }
            Err(error) if database_error(&error) => Err(error),
            Err(_) => Ok(false),
        }
    }

    /// Recheck before preparing/signing native bytes. Explicit retained markers
    /// and bare verifier uploads fail closed; they never return an ordinary-upload
    /// fallback. Acknowledged collectors remain eligible only while active.
    pub async fn retained_provider_receipt_grant_for_upload(
        &self,
        input: &RunnerEventInput,
    ) -> Result<Option<RetainedProviderReceiptGrant>> {
        if input.event_type != "run.artifact_upload" {
            if input.payload.get(GRANT).is_some() || input.payload.get(UPLOAD).is_some() {
                return Err(denied(
                    "retained-marked input is not a provider artifact upload",
                ));
            }
            return Ok(None);
        }
        let mut tx = self.pool.begin().await?;
        if !lock_for_run_tx(&mut tx, input.corp_id, input.run_id).await? {
            if input.payload.get(GRANT).is_some() || input.payload.get(UPLOAD).is_some() {
                return Err(denied(
                    "retained-marked upload requires a current verifier grant",
                ));
            }
            tx.commit().await?;
            return Ok(None);
        }
        let (grant, _) = current_tx(&mut tx, input.corp_id, input.run_id, None).await?;
        validate_upload_input_tx(&mut tx, input, &grant).await?;
        tx.commit().await?;
        Ok(Some(grant))
    }
}
