use super::*;

const MAX_MISSION_BUDGET_TOKENS: i64 = 20_000_000;
const MAX_MISSION_BUDGET_COST_MICROUSD: i64 = 100_000_000;
const MAX_TASK_BUDGET_TOKENS: i64 = 2_000_000;
const MAX_TASK_BUDGET_COST_MICROUSD: i64 = 10_000_000;

const REVISION_SELECT: &str = r#"
    SELECT revision.id, revision.corp_id, revision.mission_id,
           revision.proposed_by, revision.status, revision.version,
           revision.current_budget_tokens, revision.current_budget_cost_microusd,
           revision.proposed_budget_tokens, revision.proposed_budget_cost_microusd,
           revision.consumed_tokens_at_proposal,
           revision.consumed_cost_microusd_at_proposal, revision.rationale,
           revision.replacement_task_id, revision.previous_contract,
           revision.replacement_contract, revision.previous_verification_policy,
           revision.replacement_verification_policy, revision.decided_by,
           revision.decision_note, revision.created_at, revision.decided_at,
           revision.updated_at
    FROM mission_budget_revisions revision
"#;

#[derive(Debug, Clone)]
struct NormalizedProposal {
    corp_id: Uuid,
    mission_id: Uuid,
    actor_id: Uuid,
    expected_budget_tokens: i64,
    expected_budget_cost_microusd: i64,
    proposed_budget_tokens: i64,
    proposed_budget_cost_microusd: i64,
    rationale: String,
    idempotency_key: Uuid,
    finish_scope: Option<MissionFinishScopeInput>,
}

impl PgStore {
    pub async fn propose_mission_budget_revision(
        &self,
        input: ProposeMissionBudgetRevisionInput,
    ) -> Result<MissionBudgetRevisionOutcome> {
        let input = normalize_proposal(input)?;
        let request = proposal_request(&input)?;
        let mut tx = self.pool.begin().await?;
        assert_actor_scope_tx(&mut tx, input.corp_id, input.actor_id).await?;
        ensure_budget_manager_tx(&mut tx, input.corp_id, input.actor_id).await?;
        lock_factory_keys_tx(
            &mut tx,
            &[
                format!(
                    "mission-budget:proposal:{}:{}",
                    input.corp_id, input.idempotency_key
                ),
                format!(
                    "mission-budget:mission:{}:{}",
                    input.corp_id, input.mission_id
                ),
            ],
        )
        .await?;
        assert_mission_room_membership_tx(&mut tx, input.corp_id, input.mission_id, input.actor_id)
            .await?;

        if let Some(row) = sqlx::query(
            r#"
            SELECT id, mission_id, proposed_by, proposal_request
            FROM mission_budget_revisions
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
                || row.get::<Uuid, _>("proposed_by") != input.actor_id
                || row.get::<Value, _>("proposal_request") != request
            {
                return Err(anyhow!(
                    "mission budget proposal idempotency key was reused with a different request"
                ));
            }
            let revision =
                revision_by_id_tx(&mut tx, input.corp_id, input.mission_id, revision_id, false)
                    .await?
                    .context("idempotent mission budget proposal references a missing revision")?;
            tx.commit().await?;
            return Ok(MissionBudgetRevisionOutcome {
                revision,
                event: None,
                replayed: true,
            });
        }

        let mission = sqlx::query(
            r#"
            SELECT room_id, status, budget_tokens, budget_cost_microusd
            FROM missions
            WHERE id = $1 AND corp_id = $2
            FOR UPDATE
            "#,
        )
        .bind(input.mission_id)
        .bind(input.corp_id)
        .fetch_optional(&mut *tx)
        .await?
        .context("mission not found for budget revision")?;
        let room_id: Uuid = mission.get("room_id");
        let mission_status: String = mission.get("status");
        if mission_status == "completed" {
            return Err(anyhow!(
                "mission budget cannot be revised after the mission is {mission_status}"
            ));
        }
        let current_budget_tokens: i64 = mission.get("budget_tokens");
        let current_budget_cost_microusd: i64 = mission.get("budget_cost_microusd");
        if current_budget_tokens != input.expected_budget_tokens
            || current_budget_cost_microusd != input.expected_budget_cost_microusd
        {
            return Err(anyhow!(
                "mission budget changed: current token/cost limits are {current_budget_tokens}/{current_budget_cost_microusd}"
            ));
        }
        ensure_no_active_mission_run_tx(&mut tx, input.corp_id, input.mission_id).await?;
        let suspended_task_id =
            ensure_latest_run_is_resumable_suspension_tx(&mut tx, input.corp_id, input.mission_id)
                .await?;

        if sqlx::query_scalar::<_, bool>(
            r#"
            SELECT EXISTS (
                SELECT 1 FROM mission_budget_revisions
                WHERE corp_id = $1 AND mission_id = $2 AND status = 'pending'
            )
            "#,
        )
        .bind(input.corp_id)
        .bind(input.mission_id)
        .fetch_one(&mut *tx)
        .await?
        {
            return Err(anyhow!("mission already has a pending budget revision"));
        }

        let (consumed_tokens, consumed_cost_microusd) =
            mission_usage_tx(&mut tx, input.corp_id, input.mission_id).await?;
        if input.proposed_budget_tokens <= consumed_tokens {
            return Err(anyhow!(
                "proposed mission token budget must exceed the {consumed_tokens} tokens already consumed"
            ));
        }
        if input.proposed_budget_cost_microusd <= consumed_cost_microusd {
            return Err(anyhow!(
                "proposed mission cost budget must exceed the {consumed_cost_microusd} microusd already consumed"
            ));
        }

        let (
            replacement_task_id,
            previous_contract,
            replacement_contract,
            previous_verification_policy,
            replacement_verification_policy,
        ) = if let Some(scope) = input.finish_scope.as_ref() {
            ensure_finish_scope_targets_suspension(scope.task_id, suspended_task_id)?;
            let row = sqlx::query(
                r#"
                SELECT status, contract, verification_policy
                FROM tasks
                WHERE id = $1 AND mission_id = $2 AND corp_id = $3
                FOR UPDATE
                "#,
            )
            .bind(scope.task_id)
            .bind(input.mission_id)
            .bind(input.corp_id)
            .fetch_optional(&mut *tx)
            .await?
            .context("finish-scope task does not belong to the mission")?;
            let task_status: String = row.get("status");
            if task_status == "completed" {
                return Err(anyhow!(
                    "finish-scope task cannot be revised from status {task_status}"
                ));
            }
            let previous_contract_value: Value = row.get("contract");
            let previous_contract: TaskContract =
                serde_json::from_value(previous_contract_value.clone())
                    .context("decode task contract for budget revision")?;
            let previous_policy_value: Value = row.get("verification_policy");
            let previous_policy: VerificationPolicy =
                serde_json::from_value(previous_policy_value.clone())
                    .context("decode verifier policy for budget revision")?;
            if scope.verification_policy != previous_policy {
                return Err(anyhow!(
                    "budget finish scope cannot widen or replace the verifier policy; use a separately authorized contract revision"
                ));
            }
            let remaining_tokens = input.proposed_budget_tokens - consumed_tokens;
            let remaining_cost_microusd =
                input.proposed_budget_cost_microusd - consumed_cost_microusd;
            if scope.budget_tokens > remaining_tokens
                || scope.budget_cost_microusd > remaining_cost_microusd
            {
                return Err(anyhow!(
                    "finish-scope budget exceeds the budget remaining after already consumed usage"
                ));
            }
            if scope.budget_tokens > previous_contract.budget_tokens
                || scope.budget_cost_microusd > previous_contract.budget_cost_microusd
            {
                return Err(anyhow!(
                    "finish scope may reduce but cannot increase the task budget"
                ));
            }
            if scope.write_scope.iter().any(|candidate| {
                !previous_contract
                    .write_scope
                    .iter()
                    .any(|authorized| write_scope_contains(authorized, candidate))
            }) {
                return Err(anyhow!("finish scope cannot widen the task write boundary"));
            }
            let mut replacement = previous_contract.clone();
            replacement.objective = scope.objective.clone();
            replacement.expected_output = scope.expected_output.clone();
            replacement.acceptance_tests = scope.acceptance_tests.clone();
            replacement.write_scope = scope.write_scope.clone();
            replacement.budget_tokens = scope.budget_tokens;
            replacement.budget_cost_microusd = scope.budget_cost_microusd;
            (
                Some(scope.task_id),
                Some(previous_contract_value),
                Some(serde_json::to_value(replacement)?),
                Some(previous_policy_value.clone()),
                Some(previous_policy_value),
            )
        } else {
            (None, None, None, None, None)
        };

        let revision_id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO mission_budget_revisions (
                id, corp_id, mission_id, proposed_by, status, version,
                current_budget_tokens, current_budget_cost_microusd,
                proposed_budget_tokens, proposed_budget_cost_microusd,
                consumed_tokens_at_proposal, consumed_cost_microusd_at_proposal,
                rationale, replacement_task_id, previous_contract, replacement_contract,
                previous_verification_policy, replacement_verification_policy,
                idempotency_key, proposal_request
            )
            VALUES (
                $1, $2, $3, $4, 'pending', 1, $5, $6, $7, $8, $9, $10,
                $11, $12, $13, $14, $15, $16, $17, $18
            )
            "#,
        )
        .bind(revision_id)
        .bind(input.corp_id)
        .bind(input.mission_id)
        .bind(input.actor_id)
        .bind(current_budget_tokens)
        .bind(current_budget_cost_microusd)
        .bind(input.proposed_budget_tokens)
        .bind(input.proposed_budget_cost_microusd)
        .bind(consumed_tokens)
        .bind(consumed_cost_microusd)
        .bind(&input.rationale)
        .bind(replacement_task_id)
        .bind(previous_contract)
        .bind(replacement_contract)
        .bind(previous_verification_policy)
        .bind(replacement_verification_policy)
        .bind(input.idempotency_key)
        .bind(&request)
        .execute(&mut *tx)
        .await?;

        let revision =
            revision_by_id_tx(&mut tx, input.corp_id, input.mission_id, revision_id, false)
                .await?
                .context("inserted mission budget revision disappeared")?;
        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(room_id),
                aggregate_version: revision.version,
                correlation_id: Some(input.mission_id),
                ..NewEvent::new(
                    input.corp_id,
                    Some(input.actor_id),
                    "mission.budget_revision_proposed",
                    "mission_budget_revision",
                    revision.id,
                    format!("mission-budget-revision:{}:proposed", revision.id),
                    json!({
                        "mission_id": input.mission_id,
                        "current_budget_tokens": current_budget_tokens,
                        "current_budget_cost_microusd": current_budget_cost_microusd,
                        "proposed_budget_tokens": input.proposed_budget_tokens,
                        "proposed_budget_cost_microusd": input.proposed_budget_cost_microusd,
                        "consumed_tokens": consumed_tokens,
                        "consumed_cost_microusd": consumed_cost_microusd,
                        "replacement_task_id": replacement_task_id,
                    }),
                )
            },
        )
        .await?
        .context("mission budget proposal event unexpectedly existed")?;
        tx.commit().await?;
        Ok(MissionBudgetRevisionOutcome {
            revision,
            event: Some(event),
            replayed: false,
        })
    }

    pub async fn decide_mission_budget_revision(
        &self,
        input: DecideMissionBudgetRevisionInput,
    ) -> Result<MissionBudgetRevisionOutcome> {
        if input.expected_version <= 0 {
            return Err(anyhow!("expected budget revision version must be positive"));
        }
        let note = normalize_text(&input.note, "budget revision decision note", 4_000)?;
        let request = json!({
            "revision_id": input.revision_id,
            "expected_version": input.expected_version,
            "approved": input.approved,
            "note": note,
        });
        let mut tx = self.pool.begin().await?;
        assert_actor_scope_tx(&mut tx, input.corp_id, input.actor_id).await?;
        ensure_budget_manager_tx(&mut tx, input.corp_id, input.actor_id).await?;
        lock_factory_keys_tx(
            &mut tx,
            &[
                format!(
                    "mission-budget:decision:{}:{}",
                    input.corp_id, input.decision_key
                ),
                format!(
                    "mission-budget:revision:{}:{}",
                    input.corp_id, input.revision_id
                ),
            ],
        )
        .await?;
        assert_mission_room_membership_tx(&mut tx, input.corp_id, input.mission_id, input.actor_id)
            .await?;

        if let Some(row) = sqlx::query(
            r#"
            SELECT id, mission_id, decided_by, decision_request
            FROM mission_budget_revisions
            WHERE corp_id = $1 AND decision_key = $2
            "#,
        )
        .bind(input.corp_id)
        .bind(input.decision_key)
        .fetch_optional(&mut *tx)
        .await?
        {
            let revision_id: Uuid = row.get("id");
            if revision_id != input.revision_id
                || row.get::<Uuid, _>("mission_id") != input.mission_id
                || row.get::<Option<Uuid>, _>("decided_by") != Some(input.actor_id)
                || row.get::<Option<Value>, _>("decision_request") != Some(request.clone())
            {
                return Err(anyhow!(
                    "mission budget decision key was reused with a different request"
                ));
            }
            let revision =
                revision_by_id_tx(&mut tx, input.corp_id, input.mission_id, revision_id, false)
                    .await?
                    .context("idempotent budget decision references a missing revision")?;
            tx.commit().await?;
            return Ok(MissionBudgetRevisionOutcome {
                revision,
                event: None,
                replayed: true,
            });
        }

        let row = sqlx::query(
            r#"
            SELECT revision.status, revision.version, revision.current_budget_tokens,
                   revision.current_budget_cost_microusd, revision.proposed_budget_tokens,
                   revision.proposed_budget_cost_microusd, revision.replacement_task_id,
                   revision.previous_contract, revision.replacement_contract,
                   revision.previous_verification_policy,
                   revision.replacement_verification_policy,
                   mission.room_id, mission.status AS mission_status,
                   mission.budget_tokens, mission.budget_cost_microusd
            FROM mission_budget_revisions revision
            JOIN missions mission ON mission.id = revision.mission_id
            WHERE revision.id = $1 AND revision.corp_id = $2 AND revision.mission_id = $3
            FOR UPDATE OF revision, mission
            "#,
        )
        .bind(input.revision_id)
        .bind(input.corp_id)
        .bind(input.mission_id)
        .fetch_optional(&mut *tx)
        .await?
        .context("mission budget revision not found")?;
        let status: String = row.get("status");
        if status != "pending" {
            return Err(anyhow!("mission budget revision was already {status}"));
        }
        let version: i64 = row.get("version");
        if version != input.expected_version {
            return Err(anyhow!(
                "mission budget revision version is {version}, not {}",
                input.expected_version
            ));
        }
        let mission_status: String = row.get("mission_status");
        if mission_status == "completed" {
            return Err(anyhow!(
                "mission budget cannot be decided after the mission is {mission_status}"
            ));
        }

        if input.approved {
            let current_budget_tokens: i64 = row.get("current_budget_tokens");
            let current_budget_cost_microusd: i64 = row.get("current_budget_cost_microusd");
            if row.get::<i64, _>("budget_tokens") != current_budget_tokens
                || row.get::<i64, _>("budget_cost_microusd") != current_budget_cost_microusd
            {
                return Err(anyhow!(
                    "mission budget changed after the revision was proposed"
                ));
            }
            ensure_no_active_mission_run_tx(&mut tx, input.corp_id, input.mission_id).await?;
            let suspended_task_id = ensure_latest_run_is_resumable_suspension_tx(
                &mut tx,
                input.corp_id,
                input.mission_id,
            )
            .await?;
            let (consumed_tokens, consumed_cost_microusd) =
                mission_usage_tx(&mut tx, input.corp_id, input.mission_id).await?;
            let proposed_budget_tokens: i64 = row.get("proposed_budget_tokens");
            let proposed_budget_cost_microusd: i64 = row.get("proposed_budget_cost_microusd");
            if proposed_budget_tokens <= consumed_tokens
                || proposed_budget_cost_microusd <= consumed_cost_microusd
            {
                return Err(anyhow!(
                    "mission consumed budget beyond the proposed revision before approval"
                ));
            }
            sqlx::query(
                r#"
                UPDATE missions
                SET budget_tokens = $1, budget_cost_microusd = $2, updated_at = now()
                WHERE id = $3 AND corp_id = $4
                "#,
            )
            .bind(proposed_budget_tokens)
            .bind(proposed_budget_cost_microusd)
            .bind(input.mission_id)
            .bind(input.corp_id)
            .execute(&mut *tx)
            .await?;

            if let Some(task_id) = row.get::<Option<Uuid>, _>("replacement_task_id") {
                ensure_finish_scope_targets_suspension(task_id, suspended_task_id)?;
                let expected_contract: Value = row
                    .get::<Option<Value>, _>("previous_contract")
                    .context("approved finish scope omitted previous contract")?;
                let contract: Value = row
                    .get::<Option<Value>, _>("replacement_contract")
                    .context("approved finish scope omitted replacement contract")?;
                let expected_verification_policy: Value = row
                    .get::<Option<Value>, _>("previous_verification_policy")
                    .context("approved finish scope omitted previous verifier policy")?;
                let verification_policy: Value = row
                    .get::<Option<Value>, _>("replacement_verification_policy")
                    .context("approved finish scope omitted verifier policy")?;
                let current_task = sqlx::query(
                    r#"
                    SELECT status, contract, verification_policy
                    FROM tasks
                    WHERE id = $1 AND mission_id = $2 AND corp_id = $3
                    FOR UPDATE
                    "#,
                )
                .bind(task_id)
                .bind(input.mission_id)
                .bind(input.corp_id)
                .fetch_optional(&mut *tx)
                .await?
                .context("finish-scope task no longer belongs to the mission")?;
                let current_task_status: String = current_task.get("status");
                if current_task_status == "completed" {
                    return Err(anyhow!(
                        "finish-scope task cannot be approved from status {current_task_status}"
                    ));
                }
                if current_task.get::<Value, _>("contract") != expected_contract
                    || current_task.get::<Value, _>("verification_policy")
                        != expected_verification_policy
                {
                    return Err(anyhow!(
                        "conflict: finish-scope task contract or verifier policy changed after proposal"
                    ));
                }
                let objective = contract
                    .get("objective")
                    .and_then(Value::as_str)
                    .context("replacement task contract omitted objective")?
                    .to_owned();
                sqlx::query(
                    r#"
                    UPDATE tasks
                    SET objective = $1, contract = $2, verification_policy = $3,
                        verification_status = 'pending', updated_at = now()
                    WHERE id = $4 AND mission_id = $5 AND corp_id = $6
                    "#,
                )
                .bind(&objective)
                .bind(contract)
                .bind(verification_policy)
                .bind(task_id)
                .bind(input.mission_id)
                .bind(input.corp_id)
                .execute(&mut *tx)
                .await?;
            }
        }

        let decided_status = if input.approved {
            "approved"
        } else {
            "rejected"
        };
        sqlx::query(
            r#"
            UPDATE mission_budget_revisions
            SET status = $1, version = version + 1, decided_by = $2,
                decision_note = $3, decision_key = $4, decision_request = $5,
                decided_at = now(), updated_at = now()
            WHERE id = $6 AND corp_id = $7
            "#,
        )
        .bind(decided_status)
        .bind(input.actor_id)
        .bind(&note)
        .bind(input.decision_key)
        .bind(&request)
        .bind(input.revision_id)
        .bind(input.corp_id)
        .execute(&mut *tx)
        .await?;
        let revision = revision_by_id_tx(
            &mut tx,
            input.corp_id,
            input.mission_id,
            input.revision_id,
            false,
        )
        .await?
        .context("decided mission budget revision disappeared")?;
        let room_id: Uuid = row.get("room_id");
        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(room_id),
                aggregate_version: revision.version,
                correlation_id: Some(input.mission_id),
                ..NewEvent::new(
                    input.corp_id,
                    Some(input.actor_id),
                    if input.approved {
                        "mission.budget_revision_approved"
                    } else {
                        "mission.budget_revision_rejected"
                    },
                    "mission_budget_revision",
                    revision.id,
                    format!("mission-budget-revision:{}:{decided_status}", revision.id),
                    json!({
                        "mission_id": input.mission_id,
                        "status": decided_status,
                        "proposed_budget_tokens": revision.proposed_budget_tokens,
                        "proposed_budget_cost_microusd": revision.proposed_budget_cost_microusd,
                        "replacement_task_id": revision.replacement_task_id,
                    }),
                )
            },
        )
        .await?
        .context("mission budget decision event unexpectedly existed")?;
        tx.commit().await?;
        Ok(MissionBudgetRevisionOutcome {
            revision,
            event: Some(event),
            replayed: false,
        })
    }
}

fn normalize_proposal(input: ProposeMissionBudgetRevisionInput) -> Result<NormalizedProposal> {
    for (field, value, maximum) in [
        (
            "expected mission token budget",
            input.expected_budget_tokens,
            MAX_MISSION_BUDGET_TOKENS,
        ),
        (
            "proposed mission token budget",
            input.proposed_budget_tokens,
            MAX_MISSION_BUDGET_TOKENS,
        ),
        (
            "expected mission cost budget",
            input.expected_budget_cost_microusd,
            MAX_MISSION_BUDGET_COST_MICROUSD,
        ),
        (
            "proposed mission cost budget",
            input.proposed_budget_cost_microusd,
            MAX_MISSION_BUDGET_COST_MICROUSD,
        ),
    ] {
        if !(1..=maximum).contains(&value) {
            return Err(anyhow!("{field} is out of range"));
        }
    }
    if input.proposed_budget_tokens < input.expected_budget_tokens
        || input.proposed_budget_cost_microusd < input.expected_budget_cost_microusd
        || (input.proposed_budget_tokens == input.expected_budget_tokens
            && input.proposed_budget_cost_microusd == input.expected_budget_cost_microusd)
    {
        return Err(anyhow!(
            "mission budget revision must increase at least one current limit without reducing another"
        ));
    }
    let finish_scope = input.finish_scope.map(normalize_finish_scope).transpose()?;
    Ok(NormalizedProposal {
        corp_id: input.corp_id,
        mission_id: input.mission_id,
        actor_id: input.actor_id,
        expected_budget_tokens: input.expected_budget_tokens,
        expected_budget_cost_microusd: input.expected_budget_cost_microusd,
        proposed_budget_tokens: input.proposed_budget_tokens,
        proposed_budget_cost_microusd: input.proposed_budget_cost_microusd,
        rationale: normalize_text(&input.rationale, "budget revision rationale", 4_000)?,
        idempotency_key: input.idempotency_key,
        finish_scope,
    })
}

fn normalize_finish_scope(input: MissionFinishScopeInput) -> Result<MissionFinishScopeInput> {
    if !(1..=MAX_TASK_BUDGET_TOKENS).contains(&input.budget_tokens) {
        return Err(anyhow!("finish-scope token budget is out of range"));
    }
    if !(1..=MAX_TASK_BUDGET_COST_MICROUSD).contains(&input.budget_cost_microusd) {
        return Err(anyhow!("finish-scope cost budget is out of range"));
    }
    if input.acceptance_tests.is_empty() || input.acceptance_tests.len() > 64 {
        return Err(anyhow!(
            "finish scope must contain between 1 and 64 acceptance tests"
        ));
    }
    if input.write_scope.is_empty() || input.write_scope.len() > 64 {
        return Err(anyhow!(
            "finish scope must contain between 1 and 64 write-scope entries"
        ));
    }
    let normalize_entries = |values: Vec<String>, field: &str| -> Result<Vec<String>> {
        values
            .into_iter()
            .map(|value| normalize_text(&value, field, 500))
            .collect()
    };
    let write_scope = input
        .write_scope
        .into_iter()
        .map(normalize_write_scope)
        .collect::<Result<Vec<_>>>()?;
    Ok(MissionFinishScopeInput {
        task_id: input.task_id,
        objective: normalize_text(&input.objective, "finish-scope objective", 100_000)?,
        expected_output: normalize_text(
            &input.expected_output,
            "finish-scope expected output",
            10_000,
        )?,
        acceptance_tests: normalize_entries(
            input.acceptance_tests,
            "finish-scope acceptance test",
        )?,
        write_scope,
        budget_tokens: input.budget_tokens,
        budget_cost_microusd: input.budget_cost_microusd,
        verification_policy: input.verification_policy,
    })
}

fn normalize_text(value: &str, field: &str, max_len: usize) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(anyhow!("{field} cannot be empty"));
    }
    if value.len() > max_len {
        return Err(anyhow!("{field} cannot exceed {max_len} bytes"));
    }
    if value.chars().any(char::is_control) {
        return Err(anyhow!("{field} cannot contain control characters"));
    }
    Ok(value.to_owned())
}

fn normalize_write_scope(value: String) -> Result<String> {
    let value = normalize_text(&value, "finish-scope write scope", 500)?;
    if value == "**" {
        return Ok(value);
    }
    if value.starts_with('/')
        || value.starts_with('\\')
        || value.contains('\\')
        || value.contains(':')
        || value.contains("//")
    {
        return Err(anyhow!(
            "finish-scope write scope must be a normalized relative path"
        ));
    }
    let path = value.strip_suffix("/**").unwrap_or(&value);
    if path.is_empty()
        || path.contains('*')
        || path
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return Err(anyhow!(
            "finish-scope write scope must be an exact relative path or end in /**"
        ));
    }
    Ok(value)
}

fn ensure_finish_scope_targets_suspension(
    finish_task_id: Uuid,
    suspended_task_id: Uuid,
) -> Result<()> {
    if finish_task_id != suspended_task_id {
        return Err(anyhow!(
            "finish scope must target the task from the latest budget suspension"
        ));
    }
    Ok(())
}

fn write_scope_contains(authorized: &str, candidate: &str) -> bool {
    if authorized == "**" || authorized == candidate {
        return true;
    }
    let Some(prefix) = authorized.strip_suffix("/**") else {
        return false;
    };
    candidate == prefix
        || candidate
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn proposal_request(input: &NormalizedProposal) -> Result<Value> {
    let finish_scope = input.finish_scope.as_ref().map(|scope| {
        json!({
            "task_id": scope.task_id,
            "objective": scope.objective,
            "expected_output": scope.expected_output,
            "acceptance_tests": scope.acceptance_tests,
            "write_scope": scope.write_scope,
            "budget_tokens": scope.budget_tokens,
            "budget_cost_microusd": scope.budget_cost_microusd,
            "verification_policy": scope.verification_policy,
        })
    });
    Ok(json!({
        "mission_id": input.mission_id,
        "expected_budget_tokens": input.expected_budget_tokens,
        "expected_budget_cost_microusd": input.expected_budget_cost_microusd,
        "proposed_budget_tokens": input.proposed_budget_tokens,
        "proposed_budget_cost_microusd": input.proposed_budget_cost_microusd,
        "rationale": input.rationale,
        "finish_scope": finish_scope,
    }))
}

async fn revision_by_id_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    mission_id: Uuid,
    revision_id: Uuid,
    for_update: bool,
) -> Result<Option<MissionBudgetRevision>> {
    let query = format!(
        "{REVISION_SELECT} WHERE revision.id = $1 AND revision.corp_id = $2 AND revision.mission_id = $3{}",
        if for_update { " FOR UPDATE" } else { "" }
    );
    sqlx::query(&query)
        .bind(revision_id)
        .bind(corp_id)
        .bind(mission_id)
        .fetch_optional(&mut **tx)
        .await?
        .map(map_mission_budget_revision)
        .transpose()
}

async fn ensure_budget_manager_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    actor_id: Uuid,
) -> Result<()> {
    let role: Option<String> =
        sqlx::query_scalar("SELECT role FROM actors WHERE id = $1 AND corp_id = $2")
            .bind(actor_id)
            .bind(corp_id)
            .fetch_optional(&mut **tx)
            .await?;
    if !matches!(role.as_deref(), Some("owner" | "admin")) {
        return Err(anyhow!(
            "mission budget revisions require an owner or admin"
        ));
    }
    Ok(())
}

async fn assert_mission_room_membership_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    mission_id: Uuid,
    actor_id: Uuid,
) -> Result<Uuid> {
    let room_id: Uuid =
        sqlx::query_scalar("SELECT room_id FROM missions WHERE id = $1 AND corp_id = $2")
            .bind(mission_id)
            .bind(corp_id)
            .fetch_optional(&mut **tx)
            .await?
            .context("mission not found for budget revision")?;
    assert_room_membership_tx(tx, corp_id, room_id, actor_id).await?;
    Ok(room_id)
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
            "mission budget cannot be revised while a run is active"
        ));
    }
    Ok(())
}

async fn ensure_latest_run_is_resumable_suspension_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    mission_id: Uuid,
) -> Result<Uuid> {
    let row = sqlx::query(
        r#"
        SELECT run.task_id, run.breaker_stage, run.workspace_disposition,
               run.provider_session_id, run.status
        FROM runs run
        JOIN tasks task ON task.id = run.task_id
        WHERE run.corp_id = $1 AND task.mission_id = $2
        ORDER BY run.created_at DESC, run.id DESC
        LIMIT 1
        FOR UPDATE OF run
        "#,
    )
    .bind(corp_id)
    .bind(mission_id)
    .fetch_optional(&mut **tx)
    .await?
    .context("mission has no run to recover")?;
    let breaker_stage: String = row.get("breaker_stage");
    let disposition: Option<String> = row.get("workspace_disposition");
    let session_id: Option<String> = row.get("provider_session_id");
    let status: String = row.get("status");
    if breaker_stage != "suspend"
        || disposition.as_deref() != Some("preserved")
        || session_id.is_none()
        || !matches!(status.as_str(), "failed" | "cancelled")
    {
        return Err(anyhow!(
            "latest mission run is not a terminated, preserved budget suspension"
        ));
    }
    Ok(row.get("task_id"))
}

pub(super) async fn mission_usage_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    mission_id: Uuid,
) -> Result<(i64, i64)> {
    let row = sqlx::query(
        r#"
        SELECT COALESCE(SUM(run.input_tokens + run.output_tokens), 0)::BIGINT AS tokens,
               COALESCE(SUM(run.cost_microusd), 0)::BIGINT AS cost_microusd
        FROM runs run
        JOIN tasks task ON task.id = run.task_id
        WHERE run.corp_id = $1 AND task.mission_id = $2
        "#,
    )
    .bind(corp_id)
    .bind(mission_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok((row.get("tokens"), row.get("cost_microusd")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proposal_normalization_requires_a_monotonic_bounded_increase() {
        let normalized = normalize_proposal(ProposeMissionBudgetRevisionInput {
            corp_id: Uuid::new_v4(),
            mission_id: Uuid::new_v4(),
            actor_id: Uuid::new_v4(),
            expected_budget_tokens: 500_000,
            expected_budget_cost_microusd: 1_000_000,
            proposed_budget_tokens: 750_000,
            proposed_budget_cost_microusd: 1_000_000,
            rationale: "Finish the verified application after review.".to_owned(),
            idempotency_key: Uuid::new_v4(),
            finish_scope: None,
        })
        .expect("valid budget revision");
        assert_eq!(normalized.proposed_budget_tokens, 750_000);

        let invalid = ProposeMissionBudgetRevisionInput {
            proposed_budget_tokens: 500_000,
            ..ProposeMissionBudgetRevisionInput {
                corp_id: Uuid::new_v4(),
                mission_id: Uuid::new_v4(),
                actor_id: Uuid::new_v4(),
                expected_budget_tokens: 500_000,
                expected_budget_cost_microusd: 1_000_000,
                proposed_budget_tokens: 750_000,
                proposed_budget_cost_microusd: 1_000_000,
                rationale: "No increase".to_owned(),
                idempotency_key: Uuid::new_v4(),
                finish_scope: None,
            }
        };
        assert!(
            normalize_proposal(invalid)
                .unwrap_err()
                .to_string()
                .contains("increase")
        );
    }

    #[test]
    fn finish_scope_containment_is_fail_closed() {
        assert!(write_scope_contains("**", "crates/**"));
        assert!(write_scope_contains("crates/**", "crates/server/**"));
        assert!(write_scope_contains("crates/**", "crates"));
        assert!(write_scope_contains("docs/file.md", "docs/file.md"));
        assert!(!write_scope_contains("crates/server/**", "crates/web/**"));
        assert!(!write_scope_contains("docs/file.md", "docs/other.md"));
        assert!(normalize_write_scope(".github/workflows/**".to_owned()).is_ok());
        assert!(normalize_write_scope("docs/file.md".to_owned()).is_ok());
        assert!(normalize_write_scope("src/../secrets/**".to_owned()).is_err());
        assert!(normalize_write_scope("C:/outside/**".to_owned()).is_err());
        assert!(normalize_write_scope("src\\outside/**".to_owned()).is_err());
        assert!(normalize_write_scope("src/*.rs".to_owned()).is_err());
        let suspended_task_id = Uuid::new_v4();
        assert!(
            ensure_finish_scope_targets_suspension(suspended_task_id, suspended_task_id).is_ok()
        );
        assert!(ensure_finish_scope_targets_suspension(Uuid::new_v4(), suspended_task_id).is_err());
    }
}
