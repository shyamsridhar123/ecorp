use std::collections::HashSet;

use super::*;

impl PgStore {
    pub async fn create_mission_contract_revision(
        &self,
        input: CreateMissionContractRevisionInput,
    ) -> Result<MissionContractRevisionOutcome> {
        if input.expected_contract_version <= 0 {
            return Err(anyhow!("expected task contract version must be positive"));
        }
        let reason = normalize_factory_text(&input.reason, "contract revision reason", 4_000)?;
        let description = normalize_mission_description(&input.description)?;
        validate_revised_contract(&input.contract, &input.verification_policy)?;
        let request = json!({
            "mission_id": input.mission_id,
            "task_id": input.task_id,
            "expected_contract_version": input.expected_contract_version,
            "next_action": input.next_action,
            "source_run_id": input.source_run_id,
            "reason": reason,
            "description": description,
            "contract": input.contract,
            "verification_policy": input.verification_policy,
        });

        let mut tx = self.pool.begin().await?;
        assert_actor_scope_tx(&mut tx, input.corp_id, input.actor_id).await?;
        lock_factory_keys_tx(
            &mut tx,
            &[
                format!(
                    "mission-contract:revision:{}:{}",
                    input.corp_id, input.idempotency_key
                ),
                format!(
                    "mission-contract:mission:{}:{}",
                    input.corp_id, input.mission_id
                ),
            ],
        )
        .await?;

        let mission = sqlx::query(
            r#"
            SELECT mission.room_id, mission.requested_by, mission.status,
                   mission.description, mission.specification_version,
                   mission.budget_tokens, mission.budget_cost_microusd,
                   task.contract, task.contract_version, task.verification_policy,
                   task.status AS task_status, task.required_adapter
            FROM missions mission
            JOIN tasks task ON task.mission_id = mission.id
            WHERE mission.id = $1 AND mission.corp_id = $2
              AND task.id = $3 AND task.corp_id = mission.corp_id
            FOR UPDATE OF mission, task
            "#,
        )
        .bind(input.mission_id)
        .bind(input.corp_id)
        .bind(input.task_id)
        .fetch_optional(&mut *tx)
        .await?
        .context("mission task was not found for contract revision")?;
        let room_id: Uuid = mission.get("room_id");
        assert_room_membership_tx(&mut tx, input.corp_id, room_id, input.actor_id).await?;
        ensure_contract_revision_operator_tx(
            &mut tx,
            input.corp_id,
            input.actor_id,
            mission.get("requested_by"),
        )
        .await?;

        if let Some(row) = sqlx::query(
            r#"
            SELECT id, mission_id, task_id, revised_by, request
            FROM mission_contract_revisions
            WHERE corp_id = $1 AND idempotency_key = $2
            "#,
        )
        .bind(input.corp_id)
        .bind(input.idempotency_key)
        .fetch_optional(&mut *tx)
        .await?
        {
            let revision_id: Uuid = row.get("id");
            if row.get::<Uuid, _>("mission_id") != input.mission_id
                || row.get::<Uuid, _>("task_id") != input.task_id
                || row.get::<Uuid, _>("revised_by") != input.actor_id
                || row.get::<Value, _>("request") != request
            {
                return Err(anyhow!(
                    "mission contract revision idempotency key was reused with a different request"
                ));
            }
            let revision = contract_revision_by_id_tx(&mut tx, input.corp_id, revision_id)
                .await?
                .context("idempotent contract revision references a missing row")?;
            tx.commit().await?;
            return Ok(MissionContractRevisionOutcome {
                revision,
                event: None,
                replayed: true,
            });
        }

        ensure_no_active_mission_run_tx(&mut tx, input.corp_id, input.mission_id).await?;
        let current_contract_version: i64 = mission.get("contract_version");
        if current_contract_version != input.expected_contract_version {
            return Err(anyhow!(
                "task contract version is {current_contract_version}, not {}",
                input.expected_contract_version
            ));
        }
        let current_contract_value: Value = mission.get("contract");
        let current_contract: TaskContract = serde_json::from_value(current_contract_value.clone())
            .context("decode current task contract")?;
        let current_description: String = mission.get("description");
        let mut replacement_contract = input.contract.clone();
        replacement_contract.objective = compose_revised_task_objective(
            &current_description,
            &description,
            &replacement_contract.objective,
        );
        validate_revised_contract(&replacement_contract, &input.verification_policy)?;
        let current_policy_value: Value = mission.get("verification_policy");
        let current_verification_policy: VerificationPolicy =
            serde_json::from_value(current_policy_value.clone())
                .context("decode current mission verifier policy")?;
        let mission_status: String = mission.get("status");
        let task_status: String = mission.get("task_status");
        let required_adapter: String = mission.get("required_adapter");
        if task_status == "completed" || mission_status == "completed" {
            return Err(anyhow!(
                "completed mission work cannot be revised; create a new mission"
            ));
        }

        match input.next_action {
            MissionContractRevisionAction::Redispatch => {
                if input.source_run_id.is_some() {
                    return Err(anyhow!(
                        "redispatch contract revisions cannot name a source run"
                    ));
                }
                if mission_status != "ready" {
                    return Err(anyhow!(
                        "redispatch contract revisions require a ready mission with no prior run"
                    ));
                }
                let run_count: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM runs WHERE corp_id = $1 AND task_id = $2",
                )
                .bind(input.corp_id)
                .bind(input.task_id)
                .fetch_one(&mut *tx)
                .await?;
                if run_count != 0 {
                    return Err(anyhow!(
                        "redispatch contract revisions are only allowed before the first run"
                    ));
                }
            }
            MissionContractRevisionAction::Resume => {
                let source_run_id = input
                    .source_run_id
                    .context("resume contract revisions require a source run")?;
                ensure_resumable_contract_revision_source_tx(
                    &mut tx,
                    input.corp_id,
                    input.task_id,
                    source_run_id,
                )
                .await?;
                ensure_resume_contract_is_narrow(
                    &current_contract,
                    &replacement_contract,
                    &required_adapter,
                )?;
                let factory_linked: bool = sqlx::query_scalar(
                    "SELECT EXISTS(
                        SELECT 1 FROM factory_work_items
                        WHERE corp_id = $1 AND mission_id = $2
                    )",
                )
                .bind(input.corp_id)
                .bind(input.mission_id)
                .fetch_one(&mut *tx)
                .await?;
                if factory_linked {
                    ensure_factory_recovery_verification_policy_not_weakened(
                        &current_verification_policy,
                        &input.verification_policy,
                    )?;
                }
            }
        }
        if required_adapter == "fake-process"
            && (replacement_contract.model.is_some()
                || replacement_contract.reasoning_effort.is_some())
        {
            return Err(anyhow!(
                "fake-process contract revisions cannot select a model or reasoning effort"
            ));
        }

        ensure_revised_task_budget_fits_mission_tx(
            &mut tx,
            input.corp_id,
            input.mission_id,
            input.task_id,
            mission.get("budget_tokens"),
            mission.get("budget_cost_microusd"),
            &replacement_contract,
        )
        .await?;

        let next_version = mission
            .get::<i64, _>("specification_version")
            .max(current_contract_version)
            + 1;
        let replacement_contract_value = serde_json::to_value(&replacement_contract)?;
        let replacement_policy_value = serde_json::to_value(&input.verification_policy)?;
        sqlx::query(
            r#"
            UPDATE missions
            SET description = $1, specification_version = $2, updated_at = now()
            WHERE id = $3 AND corp_id = $4
            "#,
        )
        .bind(&description)
        .bind(next_version)
        .bind(input.mission_id)
        .bind(input.corp_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"
            UPDATE tasks
            SET objective = $1, contract = $2, contract_version = $3,
                verification_policy = $4, verification_status = 'pending',
                updated_at = now()
            WHERE id = $5 AND mission_id = $6 AND corp_id = $7
            "#,
        )
        .bind(&replacement_contract.objective)
        .bind(&replacement_contract_value)
        .bind(next_version)
        .bind(&replacement_policy_value)
        .bind(input.task_id)
        .bind(input.mission_id)
        .bind(input.corp_id)
        .execute(&mut *tx)
        .await?;

        let revision_id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO mission_contract_revisions (
                id, corp_id, mission_id, task_id, version, revised_by,
                next_action, source_run_id, reason, previous_description,
                replacement_description, previous_contract, replacement_contract,
                previous_verification_policy, replacement_verification_policy,
                idempotency_key, request
            )
            VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
                $13, $14, $15, $16, $17
            )
            "#,
        )
        .bind(revision_id)
        .bind(input.corp_id)
        .bind(input.mission_id)
        .bind(input.task_id)
        .bind(next_version)
        .bind(input.actor_id)
        .bind(input.next_action.as_str())
        .bind(input.source_run_id)
        .bind(&reason)
        .bind(&current_description)
        .bind(&description)
        .bind(&current_contract_value)
        .bind(&replacement_contract_value)
        .bind(&current_policy_value)
        .bind(&replacement_policy_value)
        .bind(input.idempotency_key)
        .bind(&request)
        .execute(&mut *tx)
        .await?;

        let revision = contract_revision_by_id_tx(&mut tx, input.corp_id, revision_id)
            .await?
            .context("inserted mission contract revision disappeared")?;
        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(room_id),
                aggregate_version: next_version,
                correlation_id: Some(input.mission_id),
                ..NewEvent::new(
                    input.corp_id,
                    Some(input.actor_id),
                    "mission.contract_revised",
                    "mission_contract_revision",
                    revision.id,
                    format!("mission-contract-revision:{}", revision.id),
                    json!({
                        "mission_id": input.mission_id,
                        "task_id": input.task_id,
                        "version": next_version,
                        "next_action": input.next_action,
                        "source_run_id": input.source_run_id,
                        "description_sha256": hex::encode(Sha256::digest(description.as_bytes())),
                        "contract_sha256": hex::encode(Sha256::digest(
                            serde_json::to_vec(&replacement_contract)?
                        )),
                        "verification_policy_sha256": hex::encode(Sha256::digest(
                            serde_json::to_vec(&input.verification_policy)?
                        )),
                    }),
                )
            },
        )
        .await?
        .context("mission contract revision event unexpectedly existed")?;
        tx.commit().await?;
        Ok(MissionContractRevisionOutcome {
            revision,
            event: Some(event),
            replayed: false,
        })
    }
}

async fn contract_revision_by_id_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    revision_id: Uuid,
) -> Result<Option<MissionContractRevision>> {
    sqlx::query(
        r#"
        SELECT id, corp_id, mission_id, task_id, version, revised_by,
               next_action, source_run_id, reason, previous_description,
               replacement_description, previous_contract, replacement_contract,
               previous_verification_policy, replacement_verification_policy,
               created_at
        FROM mission_contract_revisions
        WHERE id = $1 AND corp_id = $2
        "#,
    )
    .bind(revision_id)
    .bind(corp_id)
    .fetch_optional(&mut **tx)
    .await?
    .map(map_mission_contract_revision)
    .transpose()
}

async fn ensure_contract_revision_operator_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    actor_id: Uuid,
    requester: Uuid,
) -> Result<()> {
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM actors WHERE id = $1 AND corp_id = $2 AND kind = 'human'",
    )
    .bind(actor_id)
    .bind(corp_id)
    .fetch_optional(&mut **tx)
    .await?;
    if actor_id == requester || matches!(role.as_deref(), Some("owner" | "admin" | "manager")) {
        return Ok(());
    }
    Err(anyhow!(
        "forbidden: only the mission requester, owner, admin, or manager can revise its contract"
    ))
}

async fn ensure_no_active_mission_run_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    mission_id: Uuid,
) -> Result<()> {
    let active: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1
            FROM runs run
            JOIN tasks task ON task.id = run.task_id
            WHERE run.corp_id = $1 AND task.mission_id = $2
              AND run.status IN (
                'provisioning', 'starting', 'running',
                'waiting_for_input', 'waiting_for_approval', 'verifying'
              )
        )
        "#,
    )
    .bind(corp_id)
    .bind(mission_id)
    .fetch_one(&mut **tx)
    .await?;
    if active {
        return Err(anyhow!(
            "mission contract cannot be revised while a run is active"
        ));
    }
    Ok(())
}

async fn ensure_resumable_contract_revision_source_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    task_id: Uuid,
    source_run_id: Uuid,
) -> Result<()> {
    let source = sqlx::query(
        r#"
        SELECT source.task_id, source.workspace_run_id,
               COALESCE(source.provider_session_id, provider_lineage.provider_session_id)
                   AS provider_session_id,
               source.breaker_stage, source.status, source.workspace_disposition
        FROM runs source
        LEFT JOIN LATERAL (
            WITH RECURSIVE lineage AS (
                SELECT ancestor.id, ancestor.resumed_from_run_id,
                       ancestor.provider_session_id, 0 AS depth,
                       ARRAY[ancestor.id] AS visited
                FROM runs ancestor
                WHERE ancestor.id = source.id AND ancestor.corp_id = source.corp_id
                  AND source.execution_mode = 'verification_only'
                UNION ALL
                SELECT parent.id, parent.resumed_from_run_id,
                       parent.provider_session_id, lineage.depth + 1,
                       lineage.visited || parent.id
                FROM runs parent
                JOIN lineage ON lineage.resumed_from_run_id = parent.id
                WHERE parent.corp_id = source.corp_id
                  AND parent.task_id = source.task_id
                  AND parent.agent_id = source.agent_id
                  AND parent.workspace_run_id = source.workspace_run_id
                  AND NOT parent.id = ANY(lineage.visited)
                  AND lineage.depth < 64
            )
            SELECT provider_session_id FROM lineage
            WHERE provider_session_id IS NOT NULL
            ORDER BY depth
            LIMIT 1
        ) provider_lineage ON TRUE
        WHERE source.id = $1 AND source.corp_id = $2
        FOR UPDATE OF source
        "#,
    )
    .bind(source_run_id)
    .bind(corp_id)
    .fetch_optional(&mut **tx)
    .await?
    .context("contract revision source run was not found")?;
    if source.get::<Uuid, _>("task_id") != task_id {
        return Err(anyhow!(
            "contract revision source run belongs to another task"
        ));
    }
    if source
        .get::<Option<String>, _>("provider_session_id")
        .is_none()
        || source
            .get::<Option<String>, _>("workspace_disposition")
            .as_deref()
            != Some("preserved")
        || !matches!(
            source.get::<String, _>("status").as_str(),
            "failed" | "cancelled" | "lost"
        )
    {
        return Err(anyhow!(
            "resume contract revisions require a terminal preserved provider session"
        ));
    }
    if source.get::<String, _>("breaker_stage") == "stop" {
        return Err(anyhow!(
            "stop-stage provider work cannot be revised for resume"
        ));
    }
    let workspace_run_id: Uuid = source.get("workspace_run_id");
    let lineage = sqlx::query(
        r#"
        SELECT id, breaker_stage, status, workspace_path,
               workspace_disposition, workspace_detail
        FROM runs
        WHERE corp_id = $1 AND workspace_run_id = $2
        ORDER BY created_at DESC, id DESC
        FOR UPDATE
        "#,
    )
    .bind(corp_id)
    .bind(workspace_run_id)
    .fetch_all(&mut **tx)
    .await?;
    if lineage.iter().any(|run| {
        run.get::<String, _>("breaker_stage") == "stop"
            || run
                .get::<Option<String>, _>("workspace_disposition")
                .as_deref()
                == Some("quarantined")
    }) {
        return Err(anyhow!(
            "provider workspace lineage reached stop or quarantine and cannot be revised for resume"
        ));
    }
    let latest = lineage
        .iter()
        .find(|run| {
            !lineage_run_is_pre_dispatch_failure(
                run.get::<String, _>("status").as_str(),
                run.get::<Option<String>, _>("workspace_path").as_deref(),
                run.get::<Option<String>, _>("workspace_disposition")
                    .as_deref(),
                run.get::<Option<String>, _>("workspace_detail").as_deref(),
            )
        })
        .map(|run| run.get::<Uuid, _>("id"))
        .context("provider workspace lineage is empty")?;
    if latest != source_run_id {
        return Err(anyhow!(
            "contract revision source run is not the latest resumable lineage checkpoint"
        ));
    }
    Ok(())
}

fn ensure_resume_contract_is_narrow(
    current: &TaskContract,
    replacement: &TaskContract,
    required_adapter: &str,
) -> Result<()> {
    if replacement.budget_tokens != current.budget_tokens
        || replacement.budget_cost_microusd != current.budget_cost_microusd
        || replacement.source_repository != current.source_repository
        || replacement.source_base_ref != current.source_base_ref
        || replacement.source_base_commit != current.source_base_commit
        || replacement.secret_refs != current.secret_refs
        || replacement.model != current.model
        || replacement.reasoning_effort != current.reasoning_effort
        || replacement.deliverable != current.deliverable
    {
        return Err(anyhow!(
            "resume contract revisions cannot change budget, source, secrets, model, reasoning, or deliverable authority"
        ));
    }
    if replacement
        .allowed_tools
        .iter()
        .any(|tool| !current.allowed_tools.contains(tool))
    {
        return Err(anyhow!(
            "resume contract revisions cannot widen allowed tools"
        ));
    }
    if current
        .prohibited_actions
        .iter()
        .any(|action| !replacement.prohibited_actions.contains(action))
    {
        return Err(anyhow!(
            "resume contract revisions cannot remove prohibited actions"
        ));
    }
    if replacement.write_scope.iter().any(|candidate| {
        !current
            .write_scope
            .iter()
            .any(|authorized| write_scope_allows_path(authorized, candidate))
    }) {
        return Err(anyhow!(
            "resume contract revisions cannot widen the task write scope"
        ));
    }
    if required_adapter == "fake-process"
        && (replacement.model.is_some() || replacement.reasoning_effort.is_some())
    {
        return Err(anyhow!(
            "fake-process contract revisions cannot select a model or reasoning effort"
        ));
    }
    Ok(())
}

async fn ensure_revised_task_budget_fits_mission_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    mission_id: Uuid,
    task_id: Uuid,
    mission_budget_tokens: i64,
    mission_budget_cost_microusd: i64,
    contract: &TaskContract,
) -> Result<()> {
    let row = sqlx::query(
        r#"
        SELECT COALESCE(SUM((task.contract->>'budget_tokens')::BIGINT), 0)::BIGINT AS tokens,
               COALESCE(SUM((task.contract->>'budget_cost_microusd')::BIGINT), 0)::BIGINT AS cost
        FROM tasks task
        WHERE task.corp_id = $1 AND task.mission_id = $2 AND task.id <> $3
        "#,
    )
    .bind(corp_id)
    .bind(mission_id)
    .bind(task_id)
    .fetch_one(&mut **tx)
    .await?;
    let total_tokens = row
        .get::<i64, _>("tokens")
        .checked_add(contract.budget_tokens)
        .context("revised task token budget overflow")?;
    let total_cost = row
        .get::<i64, _>("cost")
        .checked_add(contract.budget_cost_microusd)
        .context("revised task cost budget overflow")?;
    if total_tokens > mission_budget_tokens || total_cost > mission_budget_cost_microusd {
        return Err(anyhow!("revised task contract exceeds the mission budget"));
    }
    Ok(())
}

fn compose_revised_task_objective(
    previous_description: &str,
    replacement_description: &str,
    requested_objective: &str,
) -> String {
    const SEPARATOR: &str = "\n\nTASK-SPECIFIC OBJECTIVE:\n";
    let previous_description = previous_description
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let previous_description = previous_description.trim();
    let requested_objective = requested_objective.trim();
    let task_objective =
        if !previous_description.is_empty() && requested_objective == previous_description {
            ""
        } else if !previous_description.is_empty() {
            requested_objective
                .strip_prefix(previous_description)
                .and_then(|remainder| remainder.strip_prefix(SEPARATOR))
                .unwrap_or(requested_objective)
        } else {
            requested_objective
        };
    if replacement_description.is_empty() {
        return task_objective.to_owned();
    }
    if task_objective.is_empty() {
        return replacement_description.to_owned();
    }
    if replacement_description.len() + SEPARATOR.len() + task_objective.len() <= 100_000 {
        format!("{replacement_description}{SEPARATOR}{task_objective}")
    } else {
        replacement_description.to_owned()
    }
}

fn validate_revised_contract(
    contract: &TaskContract,
    verification_policy: &VerificationPolicy,
) -> Result<()> {
    if contract.objective.trim().is_empty()
        || contract.objective.len() > 100_000
        || contract.expected_output.trim().is_empty()
        || contract.expected_output.len() > 10_000
        || contract.escalation.trim().is_empty()
        || contract.escalation.len() > 2_000
        || contract.acceptance_tests.is_empty()
        || contract.allowed_tools.is_empty()
        || contract.prohibited_actions.is_empty()
        || contract.write_scope.is_empty()
        || contract.budget_tokens <= 0
        || contract.budget_tokens > 2_000_000
        || contract.budget_cost_microusd <= 0
        || contract.budget_cost_microusd > 10_000_000
    {
        return Err(anyhow!(
            "mission contract revision is incomplete or out of bounds"
        ));
    }
    for values in [
        &contract.acceptance_tests,
        &contract.allowed_tools,
        &contract.prohibited_actions,
        &contract.references,
        &contract.write_scope,
    ] {
        if values.len() > 64
            || values
                .iter()
                .any(|value| value.trim().is_empty() || value.len() > 500)
        {
            return Err(anyhow!(
                "mission contract revision contains invalid list entries"
            ));
        }
    }
    if contract
        .write_scope
        .iter()
        .any(|scope| !write_scope_is_valid(scope))
    {
        return Err(anyhow!(
            "mission contract revision contains an invalid write scope"
        ));
    }
    let source = (
        contract.source_repository.as_deref(),
        contract.source_base_ref.as_deref(),
        contract.source_base_commit.as_deref(),
    );
    if let (Some(repository), Some(base_ref), Some(base_commit)) = source {
        let mut repository_parts = repository.split('/');
        let owner = repository_parts.next().unwrap_or_default();
        let name = repository_parts.next().unwrap_or_default();
        let valid_component = |value: &str| {
            !value.is_empty()
                && value.len() <= 100
                && value.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
                })
        };
        if !valid_component(owner)
            || !valid_component(name)
            || repository_parts.next().is_some()
            || base_ref.is_empty()
            || base_ref.len() > 240
            || base_ref.starts_with('-')
            || base_ref.starts_with('/')
            || base_ref.ends_with('/')
            || base_ref.ends_with('.')
            || base_ref.contains("..")
            || base_ref.contains("@{")
            || base_ref.chars().any(|character| {
                matches!(character, '\\' | ' ' | '~' | '^' | ':' | '?' | '*' | '[')
            })
            || !matches!(base_commit.len(), 40 | 64)
            || !base_commit.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(anyhow!(
                "mission contract revision has an invalid source repository requirement"
            ));
        }
    } else if source.0.is_some() || source.1.is_some() || source.2.is_some() {
        return Err(anyhow!(
            "mission contract revision must preserve a complete source identity"
        ));
    }
    if contract
        .model
        .as_ref()
        .is_some_and(|model| model.trim().is_empty() || model.len() > 128)
    {
        return Err(anyhow!("mission contract revision has an invalid model"));
    }
    if contract.reasoning_effort.as_ref().is_some_and(|effort| {
        !matches!(
            effort.as_str(),
            "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
        )
    }) {
        return Err(anyhow!(
            "mission contract revision has an invalid reasoning effort"
        ));
    }
    if let Some(deliverable) = &contract.deliverable
        && (deliverable.paths.len() > 128
            || deliverable
                .paths
                .iter()
                .any(|path| !repository_relative_path_is_valid(path)))
    {
        return Err(anyhow!(
            "mission contract revision has invalid deliverable paths"
        ));
    }
    let mut environment_names = HashSet::new();
    for secret in &contract.secret_refs {
        if secret.env_name.is_empty()
            || secret.env_name.len() > 128
            || !secret.env_name.chars().all(|character| {
                character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
            })
            || secret
                .env_name
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_digit())
            || !environment_names.insert(secret.env_name.as_str())
            || secret.tool.trim().is_empty()
            || secret.tool.len() > 64
            || secret.resource.trim().is_empty()
            || secret.resource.len() > 512
        {
            return Err(anyhow!(
                "mission contract revision has an invalid secret reference"
            ));
        }
    }
    validate_mission_verification_policy(verification_policy)
}

pub(super) fn validate_mission_verification_policy(
    verification_policy: &VerificationPolicy,
) -> Result<()> {
    if verification_policy.checks.is_empty() || verification_policy.checks.len() > 16 {
        return Err(anyhow!(
            "mission contract revision verifier must contain between 1 and 16 checks"
        ));
    }
    for check in &verification_policy.checks {
        match check {
            VerifierCheck::Artifact { min_bytes } => {
                if *min_bytes == 0 {
                    return Err(anyhow!("artifact verifier requires a byte floor"));
                }
            }
            VerifierCheck::File { path, min_bytes }
            | VerifierCheck::Screenshot { path, min_bytes } => {
                if !repository_relative_path_is_valid(path) || *min_bytes == 0 {
                    return Err(anyhow!("file verifier is invalid"));
                }
            }
            VerifierCheck::JsonSchema {
                path,
                required_keys,
            } => {
                if !repository_relative_path_is_valid(path)
                    || required_keys.is_empty()
                    || required_keys.len() > 32
                    || required_keys
                        .iter()
                        .any(|key| key.trim().is_empty() || key.len() > 128)
                {
                    return Err(anyhow!("JSON schema verifier is invalid"));
                }
            }
            VerifierCheck::Command {
                program,
                args,
                timeout_ms,
            }
            | VerifierCheck::Test {
                program,
                args,
                timeout_ms,
            } => {
                if program.trim().is_empty()
                    || program.len() > 256
                    || program.starts_with('-')
                    || args.len() > 32
                    || args.iter().any(|arg| arg.len() > 2_000)
                    || !(100..=60_000).contains(timeout_ms)
                {
                    return Err(anyhow!("command verifier is invalid"));
                }
            }
        }
    }
    if let Some(gate) = &verification_policy.manual_gate {
        let roles = match gate {
            ManualVerificationGate::HumanApproval { roles }
            | ManualVerificationGate::IndependentReview { roles, .. } => roles,
        };
        if roles.is_empty()
            || roles.len() > 16
            || roles
                .iter()
                .any(|role| !matches!(role.as_str(), "owner" | "admin" | "manager" | "member"))
        {
            return Err(anyhow!("manual verification gate is invalid"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::compose_revised_task_objective;

    #[test]
    fn revised_descriptions_replace_the_prior_generated_prefix() {
        let prior = "Old specification\n\nTASK-SPECIFIC OBJECTIVE:\nImplement the bounded task.";
        assert_eq!(
            compose_revised_task_objective("Old specification", "New specification", prior,),
            "New specification\n\nTASK-SPECIFIC OBJECTIVE:\nImplement the bounded task."
        );
        assert_eq!(
            compose_revised_task_objective(
                "Old specification",
                "New specification",
                "A newly authored objective.",
            ),
            "New specification\n\nTASK-SPECIFIC OBJECTIVE:\nA newly authored objective."
        );
        assert_eq!(
            compose_revised_task_objective("Old specification", "", prior),
            "Implement the bounded task."
        );
    }
}
