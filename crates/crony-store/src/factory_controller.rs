use anyhow::{Context, Result, anyhow};
use chrono::{Duration, Utc};
use crony_domain::{FactoryController, NewEvent};
use serde_json::{Value, json};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use super::{
    ConfigureFactoryControllerInput, ControlFactoryControllerInput, FactoryControllerOutcome,
    HeartbeatFactoryControllerInput, PgStore, append_event_tx,
};

impl PgStore {
    pub async fn configure_factory_controller(
        &self,
        input: ConfigureFactoryControllerInput,
    ) -> Result<FactoryControllerOutcome> {
        validate_lease_seconds(input.lease_seconds)?;
        validate_component(&input.source_project_owner, "source project owner")?;
        validate_component(&input.source_repository_owner, "source repository owner")?;
        validate_component(&input.source_repository_name, "source repository name")?;
        validate_idempotency_key(&input.idempotency_key)?;
        if input.source_project_number <= 0 {
            return Err(anyhow!("source project number must be positive"));
        }
        let request = json!({
            "controller_id": input.controller_id,
            "source_project_owner": input.source_project_owner,
            "source_project_number": input.source_project_number,
            "source_repository_owner": input.source_repository_owner,
            "source_repository_name": input.source_repository_name,
            "connection_epoch": input.connection_epoch,
            "lease_seconds": input.lease_seconds
        });
        let mut tx = self.pool.begin().await?;
        if let Some(controller) = replay_operation_tx(
            &mut tx,
            input.corp_id,
            &input.idempotency_key,
            "configure",
            &request,
        )
        .await?
        {
            tx.commit().await?;
            return Ok(FactoryControllerOutcome {
                controller,
                event: None,
                replayed: true,
            });
        }
        let now = Utc::now();
        let lease_expires_at = now + Duration::seconds(input.lease_seconds);
        let row = sqlx::query(
            r#"
            INSERT INTO factory_controllers
                (id, corp_id, service_actor_id, configured_by,
                 source_project_owner, source_project_number,
                 source_repository_owner, source_repository_name,
                 desired_state, version, connection_epoch, lease_expires_at,
                 last_heartbeat_at, reconcile_generation,
                 completed_reconcile_generation, created_at, updated_at)
            VALUES
                ($1, $2, $3, $3, $4, $5, $6, $7, 'running', 1, $8, $9,
                 $10, 0, 0, $10, $10)
            ON CONFLICT (id) DO UPDATE
            SET service_actor_id = EXCLUDED.service_actor_id,
                configured_by = EXCLUDED.configured_by,
                source_project_owner = EXCLUDED.source_project_owner,
                source_project_number = EXCLUDED.source_project_number,
                source_repository_owner = EXCLUDED.source_repository_owner,
                source_repository_name = EXCLUDED.source_repository_name,
                connection_epoch = EXCLUDED.connection_epoch,
                lease_expires_at = EXCLUDED.lease_expires_at,
                last_heartbeat_at = EXCLUDED.last_heartbeat_at,
                last_error = NULL,
                version = factory_controllers.version + 1,
                updated_at = EXCLUDED.updated_at
            WHERE factory_controllers.corp_id = EXCLUDED.corp_id
            RETURNING id, corp_id, service_actor_id, configured_by,
                      source_project_owner, source_project_number,
                      source_repository_owner, source_repository_name,
                      desired_state, version, lease_expires_at, last_heartbeat_at,
                      reconcile_generation, completed_reconcile_generation,
                      active_work_item_id, reconcile_started_at, last_reconciled_at,
                      last_reconcile_result, last_error, created_at, updated_at,
                      false AS needs_decision
            "#,
        )
        .bind(input.controller_id)
        .bind(input.corp_id)
        .bind(input.actor_id)
        .bind(&input.source_project_owner)
        .bind(input.source_project_number)
        .bind(&input.source_repository_owner)
        .bind(&input.source_repository_name)
        .bind(input.connection_epoch)
        .bind(lease_expires_at)
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?
        .context("factory controller id belongs to another Corp")?;
        let controller = map_factory_controller(row);
        record_operation_tx(
            &mut tx,
            input.corp_id,
            input.controller_id,
            &input.idempotency_key,
            input.actor_id,
            "configure",
            &request,
            controller.version,
        )
        .await?;
        let event = append_event_tx(
            &mut tx,
            NewEvent {
                aggregate_version: controller.version,
                ..NewEvent::new(
                    input.corp_id,
                    Some(input.actor_id),
                    "factory.controller_configured",
                    "factory_controller",
                    input.controller_id,
                    format!(
                        "factory-controller:{}:configure:{}",
                        input.controller_id, input.idempotency_key
                    ),
                    json!({
                        "desired_state": controller.desired_state,
                        "source_project_owner": controller.source_project_owner,
                        "source_project_number": controller.source_project_number,
                        "source_repository_owner": controller.source_repository_owner,
                        "source_repository_name": controller.source_repository_name,
                        "lease_expires_at": controller.lease_expires_at
                    }),
                )
            },
        )
        .await?;
        tx.commit().await?;
        Ok(FactoryControllerOutcome {
            controller,
            event,
            replayed: false,
        })
    }

    pub async fn heartbeat_factory_controller(
        &self,
        input: HeartbeatFactoryControllerInput,
    ) -> Result<FactoryControllerOutcome> {
        validate_lease_seconds(input.lease_seconds)?;
        let error = input.error.as_deref().map(sanitize_error).transpose()?;
        if let Some(result) = input.reconcile_result.as_deref()
            && !matches!(result, "succeeded" | "failed")
        {
            return Err(anyhow!(
                "factory controller reconcile result must be succeeded or failed"
            ));
        }
        let mut tx = self.pool.begin().await?;
        let current = controller_for_update_tx(&mut tx, input.corp_id, input.controller_id)
            .await?
            .context("factory controller does not exist")?;
        let current_epoch: Uuid = current.get("connection_epoch");
        let service_actor_id: Uuid = current.get("service_actor_id");
        if service_actor_id != input.actor_id {
            return Err(anyhow!(
                "factory controller heartbeat actor does not match its service identity"
            ));
        }
        if current_epoch != input.connection_epoch {
            return Err(anyhow!("factory controller connection epoch is stale"));
        }
        let reconcile_generation: i64 = current.get("reconcile_generation");
        let completed_generation: i64 = current.get("completed_reconcile_generation");
        let next_completed = input
            .completed_reconcile_generation
            .unwrap_or(completed_generation);
        if next_completed < completed_generation || next_completed > reconcile_generation {
            return Err(anyhow!(
                "factory controller reconciliation completion is outside the authorized generation"
            ));
        }
        if input.reconcile_result.is_some() && input.completed_reconcile_generation.is_none() {
            return Err(anyhow!(
                "factory controller reconcile result requires a completed generation"
            ));
        }
        if let Some(work_item_id) = input.active_work_item_id {
            let belongs = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM factory_work_items WHERE id = $1 AND corp_id = $2)",
            )
            .bind(work_item_id)
            .bind(input.corp_id)
            .fetch_one(&mut *tx)
            .await?;
            if !belongs {
                return Err(anyhow!(
                    "factory controller active work item does not belong to the Corp"
                ));
            }
        }
        let now = Utc::now();
        let lease_expires_at = now + Duration::seconds(input.lease_seconds);
        let row = sqlx::query(
            r#"
            UPDATE factory_controllers
            SET lease_expires_at = $1,
                last_heartbeat_at = $2,
                active_work_item_id = $3,
                completed_reconcile_generation = $4,
                reconcile_started_at = CASE
                    WHEN $4 < reconcile_generation AND reconcile_started_at IS NULL THEN $2
                    WHEN $4 = reconcile_generation THEN NULL
                    ELSE reconcile_started_at
                END,
                last_reconciled_at = CASE
                    WHEN $4 > completed_reconcile_generation THEN $2
                    ELSE last_reconciled_at
                END,
                last_reconcile_result = COALESCE($5, last_reconcile_result),
                last_error = $6,
                updated_at = $2
            WHERE id = $7 AND corp_id = $8 AND connection_epoch = $9
            RETURNING id, corp_id, service_actor_id, configured_by,
                      source_project_owner, source_project_number,
                      source_repository_owner, source_repository_name,
                      desired_state, version, lease_expires_at, last_heartbeat_at,
                      reconcile_generation, completed_reconcile_generation,
                      active_work_item_id, reconcile_started_at, last_reconciled_at,
                      last_reconcile_result, last_error, created_at, updated_at,
                      false AS needs_decision
            "#,
        )
        .bind(lease_expires_at)
        .bind(now)
        .bind(input.active_work_item_id)
        .bind(next_completed)
        .bind(input.reconcile_result.as_deref())
        .bind(error.as_deref())
        .bind(input.controller_id)
        .bind(input.corp_id)
        .bind(input.connection_epoch)
        .fetch_optional(&mut *tx)
        .await?
        .context("factory controller heartbeat lost its connection epoch")?;
        let controller = map_factory_controller(row);
        let changed = current.get::<Option<Uuid>, _>("active_work_item_id")
            != controller.active_work_item_id
            || completed_generation != controller.completed_reconcile_generation
            || current.get::<Option<String>, _>("last_reconcile_result")
                != controller.last_reconcile_result
            || current.get::<Option<String>, _>("last_error") != controller.last_error;
        let event = if changed {
            append_event_tx(
                &mut tx,
                NewEvent {
                    aggregate_version: controller.version,
                    ..NewEvent::new(
                        input.corp_id,
                        Some(input.actor_id),
                        "factory.controller_status_changed",
                        "factory_controller",
                        input.controller_id,
                        format!("factory-controller:{}:status:{}", input.controller_id, Uuid::new_v4()),
                        json!({
                            "active_work_item_id": controller.active_work_item_id,
                            "completed_reconcile_generation": controller.completed_reconcile_generation,
                            "last_reconcile_result": controller.last_reconcile_result,
                            "last_error": controller.last_error
                        }),
                    )
                },
            )
            .await?
        } else {
            None
        };
        tx.commit().await?;
        Ok(FactoryControllerOutcome {
            controller,
            event,
            replayed: false,
        })
    }

    pub async fn control_factory_controller(
        &self,
        input: ControlFactoryControllerInput,
    ) -> Result<FactoryControllerOutcome> {
        validate_idempotency_key(&input.idempotency_key)?;
        if !matches!(input.action.as_str(), "pause" | "resume" | "reconcile") {
            return Err(anyhow!("unknown factory controller control action"));
        }
        let request = json!({
            "controller_id": input.controller_id,
            "expected_version": input.expected_version,
            "action": input.action
        });
        let mut tx = self.pool.begin().await?;
        if let Some(controller) = replay_operation_tx(
            &mut tx,
            input.corp_id,
            &input.idempotency_key,
            &input.action,
            &request,
        )
        .await?
        {
            tx.commit().await?;
            return Ok(FactoryControllerOutcome {
                controller,
                event: None,
                replayed: true,
            });
        }
        let current = controller_for_update_tx(&mut tx, input.corp_id, input.controller_id)
            .await?
            .context("factory controller does not exist")?;
        let version: i64 = current.get("version");
        if version != input.expected_version {
            return Err(anyhow!(
                "factory controller version changed: expected {}, current {}",
                input.expected_version,
                version
            ));
        }
        let desired_state = match input.action.as_str() {
            "pause" => "paused".to_owned(),
            "resume" => "running".to_owned(),
            _ => current.get::<String, _>("desired_state"),
        };
        let request_reconcile = matches!(input.action.as_str(), "resume" | "reconcile");
        let row = sqlx::query(
            r#"
            UPDATE factory_controllers
            SET desired_state = $1,
                reconcile_generation = reconcile_generation + CASE WHEN $2 THEN 1 ELSE 0 END,
                reconcile_started_at = CASE WHEN $2 THEN NULL ELSE reconcile_started_at END,
                last_error = CASE WHEN $1 = 'running' THEN NULL ELSE last_error END,
                version = version + 1,
                updated_at = now()
            WHERE id = $3 AND corp_id = $4 AND version = $5
            RETURNING id, corp_id, service_actor_id, configured_by,
                      source_project_owner, source_project_number,
                      source_repository_owner, source_repository_name,
                      desired_state, version, lease_expires_at, last_heartbeat_at,
                      reconcile_generation, completed_reconcile_generation,
                      active_work_item_id, reconcile_started_at, last_reconciled_at,
                      last_reconcile_result, last_error, created_at, updated_at,
                      false AS needs_decision
            "#,
        )
        .bind(&desired_state)
        .bind(request_reconcile)
        .bind(input.controller_id)
        .bind(input.corp_id)
        .bind(input.expected_version)
        .fetch_optional(&mut *tx)
        .await?
        .context("factory controller version changed during control")?;
        let controller = map_factory_controller(row);
        record_operation_tx(
            &mut tx,
            input.corp_id,
            input.controller_id,
            &input.idempotency_key,
            input.actor_id,
            &input.action,
            &request,
            controller.version,
        )
        .await?;
        let event = append_event_tx(
            &mut tx,
            NewEvent {
                aggregate_version: controller.version,
                ..NewEvent::new(
                    input.corp_id,
                    Some(input.actor_id),
                    format!("factory.controller_{}", input.action),
                    "factory_controller",
                    input.controller_id,
                    format!(
                        "factory-controller:{}:{}:{}",
                        input.controller_id, input.action, input.idempotency_key
                    ),
                    json!({
                        "desired_state": controller.desired_state,
                        "reconcile_generation": controller.reconcile_generation
                    }),
                )
            },
        )
        .await?;
        tx.commit().await?;
        Ok(FactoryControllerOutcome {
            controller,
            event,
            replayed: false,
        })
    }
}

pub(super) fn map_factory_controller(row: sqlx::postgres::PgRow) -> FactoryController {
    let desired_state: String = row.get("desired_state");
    let lease_expires_at = row.get("lease_expires_at");
    let active_work_item_id: Option<Uuid> = row.get("active_work_item_id");
    let last_reconcile_result: Option<String> = row.get("last_reconcile_result");
    let needs_decision: bool = row.try_get("needs_decision").unwrap_or(false);
    let status = if lease_expires_at <= Utc::now() {
        "offline"
    } else if needs_decision {
        "needs_decision"
    } else if last_reconcile_result.as_deref() == Some("failed")
        || row.get::<Option<String>, _>("last_error").is_some()
    {
        "blocked"
    } else if active_work_item_id.is_some() {
        "working"
    } else {
        "watching"
    };
    FactoryController {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        service_actor_id: row.get("service_actor_id"),
        configured_by: row.get("configured_by"),
        source_project_owner: row.get("source_project_owner"),
        source_project_number: row.get("source_project_number"),
        source_repository_owner: row.get("source_repository_owner"),
        source_repository_name: row.get("source_repository_name"),
        desired_state,
        status: status.to_owned(),
        version: row.get("version"),
        lease_expires_at,
        last_heartbeat_at: row.get("last_heartbeat_at"),
        reconcile_generation: row.get("reconcile_generation"),
        completed_reconcile_generation: row.get("completed_reconcile_generation"),
        active_work_item_id,
        reconcile_started_at: row.get("reconcile_started_at"),
        last_reconciled_at: row.get("last_reconciled_at"),
        last_reconcile_result,
        last_error: row.get("last_error"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

async fn controller_for_update_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    controller_id: Uuid,
) -> Result<Option<sqlx::postgres::PgRow>> {
    Ok(sqlx::query(
        r#"
        SELECT id, corp_id, service_actor_id, configured_by,
               source_project_owner, source_project_number,
               source_repository_owner, source_repository_name,
               desired_state, version, connection_epoch, lease_expires_at,
               last_heartbeat_at, reconcile_generation,
               completed_reconcile_generation, active_work_item_id,
               reconcile_started_at, last_reconciled_at,
               last_reconcile_result, last_error, created_at, updated_at,
               false AS needs_decision
        FROM factory_controllers
        WHERE id = $1 AND corp_id = $2
        FOR UPDATE
        "#,
    )
    .bind(controller_id)
    .bind(corp_id)
    .fetch_optional(&mut **tx)
    .await?)
}

async fn replay_operation_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    idempotency_key: &str,
    operation: &str,
    request: &Value,
) -> Result<Option<FactoryController>> {
    let row = sqlx::query(
        r#"
        SELECT controller_id, operation, request
        FROM factory_controller_operations
        WHERE corp_id = $1 AND idempotency_key = $2
        "#,
    )
    .bind(corp_id)
    .bind(idempotency_key)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.get::<String, _>("operation") != operation || row.get::<Value, _>("request") != *request
    {
        return Err(anyhow!(
            "factory controller idempotency key was reused with another request"
        ));
    }
    let controller_id: Uuid = row.get("controller_id");
    let row = controller_for_update_tx(tx, corp_id, controller_id)
        .await?
        .context("factory controller operation lost its controller")?;
    Ok(Some(map_factory_controller(row)))
}

#[allow(clippy::too_many_arguments)]
async fn record_operation_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    controller_id: Uuid,
    idempotency_key: &str,
    actor_id: Uuid,
    operation: &str,
    request: &Value,
    resulting_version: i64,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO factory_controller_operations
            (corp_id, controller_id, idempotency_key, actor_id, operation,
             request, resulting_version)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
    )
    .bind(corp_id)
    .bind(controller_id)
    .bind(idempotency_key)
    .bind(actor_id)
    .bind(operation)
    .bind(request)
    .bind(resulting_version)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn validate_lease_seconds(value: i64) -> Result<()> {
    if !(10..=300).contains(&value) {
        return Err(anyhow!(
            "factory controller lease seconds must be between 10 and 300"
        ));
    }
    Ok(())
}

fn validate_component(value: &str, label: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.len() > 100
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        return Err(anyhow!("{label} is invalid"));
    }
    Ok(())
}

fn validate_idempotency_key(value: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > 256 {
        return Err(anyhow!("factory controller idempotency key is invalid"));
    }
    Ok(())
}

fn sanitize_error(value: &str) -> Result<String> {
    let sanitized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if sanitized.len() > 2_000 {
        return Err(anyhow!("factory controller error exceeds 2000 bytes"));
    }
    Ok(sanitized)
}
