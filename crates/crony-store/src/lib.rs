use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result, anyhow};
use chrono::{Duration, Utc};
use crony_domain::{
    ActionApproval, Actor, ActorKind, Agent, AgentStatus, CircuitBreakerIncident, ControlLease,
    Corp, CorpSnapshot, DeliverableForm, DeliverableSpec, DomainEvent, EntityLink, FactoryWorkItem,
    FactoryWorkItemState, ManualVerificationGate, Mission, MissionBudgetRevision,
    MissionContractRevision, MissionContractRevisionAction, MissionStatus, NewEvent,
    PullRequestPublication, PullRequestPublicationAttempt, PullRequestPublicationState,
    QueuedMessage, Room, RoomMessage, Run, RunStatus, SourceDeliverable, Task, TaskContract,
    TaskGraphPlan, TaskSecretReference, TaskStatus, VerificationEvidence, VerificationPolicy,
    VerificationRequest, VerifierCheck, repository_relative_path_is_valid, write_scope_allows_path,
    write_scope_is_valid,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction, postgres::PgPoolOptions};
use uuid::Uuid;

mod budget_revision;
mod contract_revision;
mod publication;

const DEMO_CORP_ID: &str = "00000000-0000-4000-8000-000000000001";
const DEMO_ALICE_ID: &str = "00000000-0000-4000-8000-000000000011";
const DEMO_BOB_ID: &str = "00000000-0000-4000-8000-000000000012";
const DEMO_EVE_ID: &str = "00000000-0000-4000-8000-000000000013";
const DEMO_MANAGER_ACTOR_ID: &str = "00000000-0000-4000-8000-000000000021";
const DEMO_WORKER_ACTOR_ID: &str = "00000000-0000-4000-8000-000000000022";
const DEMO_CODEX_ACTOR_ID: &str = "00000000-0000-4000-8000-000000000023";
const DEMO_CLAUDE_ACTOR_ID: &str = "00000000-0000-4000-8000-000000000024";
const DEMO_OPENCODE_ACTOR_ID: &str = "00000000-0000-4000-8000-000000000025";
const DEMO_COPILOT_ACTOR_ID: &str = "00000000-0000-4000-8000-000000000026";
const DEMO_MANAGER_AGENT_ID: &str = "00000000-0000-4000-8000-000000000031";
const DEMO_WORKER_AGENT_ID: &str = "00000000-0000-4000-8000-000000000032";
const DEMO_CODEX_AGENT_ID: &str = "00000000-0000-4000-8000-000000000033";
const DEMO_CLAUDE_AGENT_ID: &str = "00000000-0000-4000-8000-000000000034";
const DEMO_OPENCODE_AGENT_ID: &str = "00000000-0000-4000-8000-000000000035";
const DEMO_COPILOT_AGENT_ID: &str = "00000000-0000-4000-8000-000000000036";
const DEMO_ROOM_ID: &str = "00000000-0000-4000-8000-000000000041";
const DEMO_ADVISORY_LOCK: i64 = 0x4352_4F4E_5944_4D4F;

#[derive(Clone)]
pub struct PgStore {
    pool: PgPool,
}

#[derive(Debug, Clone)]
pub struct DemoIds {
    pub corp_id: Uuid,
    pub room_id: Uuid,
    pub alice_actor_id: Uuid,
    pub bob_actor_id: Uuid,
    pub eve_actor_id: Uuid,
    pub manager_agent_id: Uuid,
    pub worker_agent_id: Uuid,
    pub codex_agent_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct MissionPlanIds {
    pub mission_id: Uuid,
    pub task_ids: Vec<Uuid>,
}

#[derive(Debug, Clone)]
pub struct CreateMissionContractRevisionInput {
    pub corp_id: Uuid,
    pub mission_id: Uuid,
    pub task_id: Uuid,
    pub actor_id: Uuid,
    pub expected_contract_version: i64,
    pub next_action: MissionContractRevisionAction,
    pub source_run_id: Option<Uuid>,
    pub reason: String,
    pub idempotency_key: Uuid,
    pub description: String,
    pub contract: TaskContract,
    pub verification_policy: VerificationPolicy,
}

#[derive(Debug, Clone)]
pub struct MissionContractRevisionOutcome {
    pub revision: MissionContractRevision,
    pub event: Option<DomainEvent>,
    pub replayed: bool,
}

#[derive(Debug, Clone)]
pub struct FactorySourceInput {
    pub project_owner: String,
    pub project_number: i64,
    pub project_item_id: String,
    pub repository_owner: String,
    pub repository_name: String,
    pub issue_number: i64,
    pub issue_node_id: String,
    pub issue_url: String,
    pub title: String,
    pub revision: String,
}

#[derive(Debug, Clone)]
pub struct ClaimFactoryWorkItemInput {
    pub corp_id: Uuid,
    pub actor_id: Uuid,
    pub source: FactorySourceInput,
    pub idempotency_key: String,
    pub lease_seconds: i64,
    pub policy: Value,
}

#[derive(Debug, Clone)]
pub struct RenewFactoryWorkItemInput {
    pub corp_id: Uuid,
    pub work_item_id: Uuid,
    pub actor_id: Uuid,
    pub claim_token: Uuid,
    pub expected_version: i64,
    pub idempotency_key: String,
    pub lease_seconds: i64,
}

#[derive(Debug, Clone)]
pub struct UpgradeFactorySourceCommitInput {
    pub corp_id: Uuid,
    pub work_item_id: Uuid,
    pub actor_id: Uuid,
    pub claim_token: Uuid,
    pub expected_version: i64,
    pub idempotency_key: String,
    pub source_base_commit: String,
}

#[derive(Debug, Clone)]
pub struct TransitionFactoryWorkItemInput {
    pub corp_id: Uuid,
    pub work_item_id: Uuid,
    pub actor_id: Uuid,
    pub claim_token: Uuid,
    pub expected_version: i64,
    pub idempotency_key: String,
    pub state: FactoryWorkItemState,
    pub failure_detail: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MaterializeFactoryMissionInput {
    pub corp_id: Uuid,
    pub work_item_id: Uuid,
    pub actor_id: Uuid,
    pub claim_token: Uuid,
    pub expected_version: i64,
    pub idempotency_key: String,
    pub title: String,
    pub description: String,
    pub request: Value,
}

#[derive(Debug, Clone)]
pub struct FactoryWorkItemOutcome {
    pub work_item: FactoryWorkItem,
    pub claim_token: Option<Uuid>,
    pub event: Option<DomainEvent>,
    pub replayed: bool,
}

#[derive(Debug, Clone)]
pub struct FactoryMissionOutcome {
    pub work_item: FactoryWorkItem,
    pub ids: MissionPlanIds,
    pub strategy: String,
    pub events: Vec<DomainEvent>,
    pub replayed: bool,
}

#[derive(Debug, Clone)]
pub struct StartPullRequestPublicationInput {
    pub corp_id: Uuid,
    pub work_item_id: Uuid,
    pub actor_id: Uuid,
    pub actor_role: String,
    pub source_deliverable_id: Uuid,
    pub target_repository: String,
    pub base_ref: String,
    pub branch: String,
    pub title: String,
    pub body: String,
    pub authorization_id: Uuid,
    pub authorization_reason: String,
    pub effect_key: String,
    pub idempotency_key: String,
    pub publisher_id: String,
    pub publisher_credential_hash: String,
    pub lease_seconds: i64,
}

#[derive(Debug, Clone)]
pub struct RenewPullRequestPublicationInput {
    pub corp_id: Uuid,
    pub publication_id: Uuid,
    pub actor_id: Uuid,
    pub publisher_id: String,
    pub publisher_credential_hash: String,
    pub publisher_token: Uuid,
    pub expected_version: i64,
    pub idempotency_key: String,
    pub lease_seconds: i64,
}

#[derive(Debug, Clone)]
pub enum PullRequestPublicationCheckpointInput {
    BranchPushed {
        commit_sha: String,
    },
    PullRequestCreated {
        number: i64,
        node_id: String,
        url: String,
        state: String,
        draft: bool,
        title: String,
        body: String,
        head_ref: String,
        base_ref: String,
        head_sha: String,
        head_repository_owner: String,
        is_cross_repository: bool,
        auto_merge_enabled: bool,
    },
    Published {
        project_status: String,
        project_field_id: String,
        project_option_id: String,
    },
    Failed {
        failure_detail: String,
    },
}

#[derive(Debug, Clone)]
pub struct RecordPullRequestPublicationCheckpointInput {
    pub corp_id: Uuid,
    pub publication_id: Uuid,
    pub actor_id: Uuid,
    pub publisher_id: String,
    pub publisher_credential_hash: String,
    pub publisher_token: Uuid,
    pub expected_version: i64,
    pub idempotency_key: String,
    pub checkpoint: PullRequestPublicationCheckpointInput,
}

#[derive(Debug, Clone)]
pub struct PullRequestPublicationOutcome {
    pub publication: PullRequestPublication,
    pub publisher_token: Option<Uuid>,
    pub events: Vec<DomainEvent>,
    pub replayed: bool,
    pub busy: bool,
}

#[derive(Debug, Clone)]
pub struct FactoryPublicationContext {
    pub work_item: FactoryWorkItem,
    pub publication: Option<PullRequestPublication>,
    pub source_deliverables: Vec<SourceDeliverable>,
}

#[derive(Debug, Clone)]
pub struct PublicationPublisherCredentialOutcome {
    pub credential_id: Uuid,
    pub publisher_id: String,
    pub expires_at: chrono::DateTime<Utc>,
    pub event: DomainEvent,
}

#[derive(Debug, Clone)]
pub struct PublicationPublisherCredentialRevocationOutcome {
    pub credential_id: Uuid,
    pub publisher_id: String,
    pub revoked: bool,
    pub event: Option<DomainEvent>,
}

#[derive(Debug, Clone)]
struct FactoryOperation {
    work_item_id: Uuid,
    actor_id: Uuid,
    operation: String,
    resulting_version: i64,
    claim_token: Option<Uuid>,
    request: Value,
}

struct NewFactoryOperation<'a> {
    corp_id: Uuid,
    idempotency_key: &'a str,
    work_item_id: Uuid,
    actor_id: Uuid,
    operation: &'a str,
    resulting_version: i64,
    claim_token: Option<Uuid>,
    request: &'a Value,
}

#[derive(Debug, Clone)]
pub struct LaunchRecord {
    pub corp_id: Uuid,
    pub room_id: Uuid,
    pub mission_id: Uuid,
    pub task_id: Uuid,
    pub run_id: Uuid,
    pub agent_id: Uuid,
    pub assignment_token: Uuid,
    pub attempt: i32,
    pub adapter: String,
    pub mission_title: String,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub source_repository: Option<String>,
    pub source_base_ref: Option<String>,
    pub source_base_commit: Option<String>,
    pub verification_policy: VerificationPolicy,
    pub write_scope: Vec<String>,
    pub deliverable: Option<DeliverableSpec>,
    pub secret_refs: Vec<TaskSecretReference>,
    pub queued_messages: Vec<QueuedRunMessage>,
}

#[derive(Debug, Clone)]
pub struct SchedulableTask {
    pub task_id: Uuid,
    pub required_adapter: String,
    pub required_model: Option<String>,
    pub required_reasoning_effort: Option<String>,
    pub required_source_repository: Option<String>,
    pub required_source_base_ref: Option<String>,
    pub required_source_base_commit: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResumeLaunchRecord {
    pub corp_id: Uuid,
    pub room_id: Uuid,
    pub mission_id: Uuid,
    pub task_id: Uuid,
    pub run_id: Uuid,
    pub source_run_id: Uuid,
    pub workspace_run_id: Uuid,
    pub agent_id: Uuid,
    pub runner_id: String,
    pub assignment_token: Uuid,
    pub adapter: String,
    pub provider_session_id: String,
    pub task_prompt: String,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub source_repository: Option<String>,
    pub source_base_ref: Option<String>,
    pub source_base_commit: Option<String>,
    pub workspace_base_commit: String,
    pub verification_policy: VerificationPolicy,
    pub write_scope: Vec<String>,
    pub deliverable: Option<DeliverableSpec>,
    pub secret_refs: Vec<TaskSecretReference>,
    pub queued_messages: Vec<QueuedRunMessage>,
}

#[derive(Debug, Clone, Copy)]
struct RollingBudgetRemaining {
    actor_tokens: i64,
    actor_cost_microusd: i64,
    corp_tokens: i64,
    corp_cost_microusd: i64,
}

#[derive(Debug, Clone)]
pub struct QueuedRunMessage {
    pub id: Uuid,
    pub actor_id: Uuid,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct LeaseOutcome {
    pub acquired: bool,
    pub lease: ControlLease,
    pub event: Option<DomainEvent>,
}

#[derive(Debug, Clone)]
pub struct LeaseMutationOutcome {
    pub lease: Option<ControlLease>,
    pub event: DomainEvent,
}

#[derive(Debug, Clone)]
pub struct MessageOutcome {
    pub message: QueuedMessage,
    pub delivery: String,
    pub run_id: Option<Uuid>,
    pub runner_id: Option<String>,
    pub event: DomainEvent,
}

#[derive(Debug, Clone)]
pub struct StopRequestOutcome {
    pub run_id: Uuid,
    pub runner_id: String,
    pub event: DomainEvent,
}

#[derive(Debug, Clone)]
pub struct VerificationDecisionOutcome {
    pub run_id: Uuid,
    pub corp_id: Uuid,
    pub mission_id: Uuid,
    pub status: String,
    pub event: DomainEvent,
}

#[derive(Debug, Clone)]
pub struct RoomMessageOutcome {
    pub message: RoomMessage,
    pub event: DomainEvent,
}

#[derive(Debug, Clone)]
pub struct NewRoomMessageInput {
    pub corp_id: Uuid,
    pub room_id: Uuid,
    pub actor_id: Uuid,
    pub body: String,
    pub reply_to_id: Option<Uuid>,
    pub mentions: Vec<Uuid>,
    pub link: Option<EntityLink>,
}

#[derive(Debug, Clone)]
pub struct RunnerRecord {
    pub id: String,
    pub corp_id: Uuid,
    pub hostname: String,
    pub os: String,
    pub capabilities: Value,
    pub connection_epoch: Uuid,
    pub status: String,
    pub last_seen_at: chrono::DateTime<Utc>,
    pub grace_expires_at: Option<chrono::DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct RunClaim {
    pub run_id: Uuid,
    pub assignment_token: Uuid,
}

#[derive(Debug, Clone)]
pub struct RunnerConnectInput {
    pub id: String,
    pub corp_id: Uuid,
    pub hostname: String,
    pub os: String,
    pub capabilities: Value,
    pub connection_epoch: Uuid,
}

#[derive(Debug, Clone)]
pub struct RunnerReconcileOutcome {
    pub accepted: Vec<Uuid>,
    pub stale: Vec<Uuid>,
    pub events: Vec<DomainEvent>,
}

#[derive(Debug, Clone)]
pub struct RunnerEnrollmentOutcome {
    pub enrollment_id: Uuid,
    pub expires_at: chrono::DateTime<Utc>,
    pub event: DomainEvent,
}

#[derive(Debug, Clone)]
pub struct RunnerAuthenticationOutcome {
    pub corp_id: Uuid,
    pub expires_at: chrono::DateTime<Utc>,
    pub event: DomainEvent,
}

#[derive(Debug, Clone)]
pub struct RunnerRevocationOutcome {
    pub revoked: bool,
    pub event: Option<DomainEvent>,
    pub run_events: Vec<DomainEvent>,
}

#[derive(Debug, Clone)]
pub struct SecretGrantRecord {
    pub grant_id: Uuid,
    pub secret_id: Uuid,
    pub name: String,
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
    pub env_name: String,
    pub tool: String,
    pub resource: String,
    pub expires_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ActionApprovalDecisionOutcome {
    pub approval_id: Uuid,
    pub run_id: Uuid,
    pub runner_id: String,
    pub status: String,
    pub effect_queued: bool,
    pub event: Option<DomainEvent>,
}

#[derive(Debug, Clone)]
pub struct PendingRunnerCommand {
    pub id: Uuid,
    pub runner_id: String,
    pub run_id: Uuid,
    pub command_kind: String,
    pub payload: Value,
}

#[derive(Debug, Clone)]
pub struct CircuitBreakerOutcome {
    pub command: Option<PendingRunnerCommand>,
    pub event: Option<DomainEvent>,
}

#[derive(Debug, Clone)]
pub struct ArtifactContext {
    pub task_id: Uuid,
    pub mission_id: Uuid,
    pub room_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct StoredArtifact {
    pub id: Uuid,
    pub corp_id: Uuid,
    pub task_id: Uuid,
    pub run_id: Uuid,
    pub producer_agent_id: Uuid,
    pub producer_runner_id: String,
    pub verifier: String,
    pub object_key: String,
    pub uri: String,
    pub sha256: String,
    pub media_type: String,
    pub bytes: i64,
    pub artifact_role: String,
    pub file_name: String,
    pub metadata: Value,
    pub provenance_signature: String,
    pub retention_until: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct PreparedArtifactUpload {
    pub artifact: StoredArtifact,
    pub staging_key: Option<String>,
    pub status: String,
    pub created_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct DependencyArtifactContext {
    pub task_id: Uuid,
    pub plan_key: String,
    pub task_title: String,
    pub run_summary: Option<String>,
    pub artifact: StoredArtifact,
}

#[derive(Debug, Clone)]
pub struct RunnerEventInput {
    pub event_id: Uuid,
    pub runner_id: String,
    pub corp_id: Uuid,
    pub connection_epoch: Uuid,
    pub run_id: Uuid,
    pub agent_id: Uuid,
    pub assignment_token: Uuid,
    pub event_type: String,
    pub payload: Value,
}

#[derive(Debug, Clone)]
pub struct HumanAuthorization {
    pub actor_id: Uuid,
    pub role: String,
}

impl PgStore {
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await
            .context("connect to Postgres")?;
        Ok(Self { pool })
    }

    pub async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("../../db/migrations")
            .run(&self.pool)
            .await
            .context("run database migrations")
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn human_authorization(
        &self,
        corp_id: Uuid,
        actor_id: Uuid,
    ) -> Result<Option<HumanAuthorization>> {
        let row = sqlx::query(
            r#"
            SELECT id, role
            FROM actors
            WHERE id = $1 AND corp_id = $2 AND kind = 'human'
            "#,
        )
        .bind(actor_id)
        .bind(corp_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| HumanAuthorization {
            actor_id: row.get("id"),
            role: row.get("role"),
        }))
    }

    pub async fn resolve_human_identity(
        &self,
        corp_id: Uuid,
        issuer: &str,
        subject: &str,
    ) -> Result<Option<HumanAuthorization>> {
        let row = sqlx::query(
            r#"
            UPDATE human_identities identity
            SET last_authenticated_at = now()
            FROM actors actor
            WHERE identity.issuer = $1
              AND identity.subject = $2
              AND identity.actor_id = actor.id
              AND actor.corp_id = $3
              AND actor.kind = 'human'
            RETURNING actor.id, actor.role
            "#,
        )
        .bind(issuer)
        .bind(subject)
        .bind(corp_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| HumanAuthorization {
            actor_id: row.get("id"),
            role: row.get("role"),
        }))
    }

    pub async fn link_human_identity(
        &self,
        actor_id: Uuid,
        issuer: &str,
        subject: &str,
        email: Option<&str>,
    ) -> Result<()> {
        let kind: Option<String> = sqlx::query_scalar("SELECT kind FROM actors WHERE id = $1")
            .bind(actor_id)
            .fetch_optional(&self.pool)
            .await?;
        if kind.as_deref() != Some("human") {
            return Err(anyhow!("identity can only be linked to a human actor"));
        }
        sqlx::query(
            r#"
            INSERT INTO human_identities (issuer, subject, actor_id, email)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (issuer, subject) DO UPDATE
            SET actor_id = EXCLUDED.actor_id, email = EXCLUDED.email
            "#,
        )
        .bind(issuer)
        .bind(subject)
        .bind(actor_id)
        .bind(email)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_secret(
        &self,
        corp_id: Uuid,
        actor_id: Uuid,
        secret_id: Uuid,
        name: &str,
        ciphertext: &[u8],
        nonce: &[u8],
        allowed_actor_ids: &[Uuid],
        allowed_tools: &[String],
        resource_prefix: &str,
        max_ttl_seconds: i32,
    ) -> Result<DomainEvent> {
        let mut tx = self.pool.begin().await?;
        let role: Option<String> = sqlx::query_scalar(
            "SELECT role FROM actors WHERE id = $1 AND corp_id = $2 AND kind = 'human'",
        )
        .bind(actor_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?;
        if !matches!(role.as_deref(), Some("owner" | "admin")) {
            return Err(anyhow!(
                "forbidden: only a Corp owner or admin can create secrets"
            ));
        }
        sqlx::query(
            r#"
            INSERT INTO secrets
                (id, corp_id, name, ciphertext, nonce, allowed_actor_ids, allowed_tools,
                 resource_prefix, max_ttl_seconds, created_by)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "#,
        )
        .bind(secret_id)
        .bind(corp_id)
        .bind(name)
        .bind(ciphertext)
        .bind(nonce)
        .bind(allowed_actor_ids)
        .bind(allowed_tools)
        .bind(resource_prefix)
        .bind(max_ttl_seconds)
        .bind(actor_id)
        .execute(&mut *tx)
        .await?;
        let event = append_event_tx(
            &mut tx,
            NewEvent::new(
                corp_id,
                Some(actor_id),
                "secret.created",
                "secret",
                secret_id,
                format!("secret-created:{secret_id}"),
                json!({
                    "name": name,
                    "allowed_actor_count": allowed_actor_ids.len(),
                    "allowed_tools": allowed_tools,
                    "resource_prefix": resource_prefix,
                    "max_ttl_seconds": max_ttl_seconds,
                }),
            ),
        )
        .await?
        .context("secret created event unexpectedly existed")?;
        tx.commit().await?;
        Ok(event)
    }

    pub async fn revoke_secret(
        &self,
        corp_id: Uuid,
        actor_id: Uuid,
        secret_id: Uuid,
        reason: &str,
    ) -> Result<Option<DomainEvent>> {
        let mut tx = self.pool.begin().await?;
        let role: Option<String> = sqlx::query_scalar(
            "SELECT role FROM actors WHERE id = $1 AND corp_id = $2 AND kind = 'human'",
        )
        .bind(actor_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?;
        if !matches!(role.as_deref(), Some("owner" | "admin")) {
            return Err(anyhow!(
                "forbidden: only a Corp owner or admin can revoke secrets"
            ));
        }
        let updated = sqlx::query(
            "UPDATE secrets SET revoked_at = now() WHERE id = $1 AND corp_id = $2 AND revoked_at IS NULL RETURNING id",
        )
        .bind(secret_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?;
        if updated.is_none() {
            tx.commit().await?;
            return Ok(None);
        }
        let event = append_event_tx(
            &mut tx,
            NewEvent::new(
                corp_id,
                Some(actor_id),
                "secret.revoked",
                "secret",
                secret_id,
                format!("secret-revoked:{secret_id}"),
                json!({"reason": reason}),
            ),
        )
        .await?;
        tx.commit().await?;
        Ok(event)
    }

    pub async fn grant_run_secrets(
        &self,
        corp_id: Uuid,
        task_id: Uuid,
        run_id: Uuid,
        runner_id: &str,
        refs: &[TaskSecretReference],
    ) -> Result<(Vec<SecretGrantRecord>, Vec<DomainEvent>)> {
        if refs.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let mut tx = self.pool.begin().await?;
        let requester: Uuid = sqlx::query_scalar(
            r#"
            SELECT mission.requested_by
            FROM runs run
            JOIN tasks task ON task.id = run.task_id
            JOIN missions mission ON mission.id = task.mission_id
            WHERE run.id = $1
              AND run.task_id = $2
              AND run.corp_id = $3
              AND run.runner_id = $4
              AND run.status IN ('provisioning', 'starting')
            FOR UPDATE OF run
            "#,
        )
        .bind(run_id)
        .bind(task_id)
        .bind(corp_id)
        .bind(runner_id)
        .fetch_one(&mut *tx)
        .await
        .context("active run is not eligible for secret grants")?;

        let mut grants = Vec::with_capacity(refs.len());
        let mut events = Vec::with_capacity(refs.len());
        for reference in refs {
            let row = sqlx::query(
                r#"
                SELECT id, name, ciphertext, nonce, max_ttl_seconds
                FROM secrets
                WHERE id = $1
                  AND corp_id = $2
                  AND revoked_at IS NULL
                  AND $3 = ANY(allowed_actor_ids)
                  AND $4 = ANY(allowed_tools)
                  AND left($5, char_length(resource_prefix)) = resource_prefix
                "#,
            )
            .bind(reference.secret_id)
            .bind(corp_id)
            .bind(requester)
            .bind(&reference.tool)
            .bind(&reference.resource)
            .fetch_optional(&mut *tx)
            .await?
            .with_context(|| {
                format!(
                    "forbidden: secret {} is not authorized for this actor, task, tool, or resource",
                    reference.secret_id
                )
            })?;
            let grant_id = Uuid::new_v4();
            let max_ttl_seconds: i32 = row.get("max_ttl_seconds");
            let expires_at = Utc::now() + Duration::seconds(i64::from(max_ttl_seconds.min(300)));
            sqlx::query(
                r#"
                INSERT INTO secret_access_grants
                    (id, corp_id, secret_id, task_id, run_id, actor_id, runner_id,
                     tool, resource, expires_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                "#,
            )
            .bind(grant_id)
            .bind(corp_id)
            .bind(reference.secret_id)
            .bind(task_id)
            .bind(run_id)
            .bind(requester)
            .bind(runner_id)
            .bind(&reference.tool)
            .bind(&reference.resource)
            .bind(expires_at)
            .execute(&mut *tx)
            .await?;
            if let Some(event) = append_event_tx(
                &mut tx,
                NewEvent::new(
                    corp_id,
                    Some(requester),
                    "secret.access_granted",
                    "secret_grant",
                    grant_id,
                    format!("secret-grant:{grant_id}"),
                    json!({
                        "secret_id": reference.secret_id,
                        "task_id": task_id,
                        "run_id": run_id,
                        "runner_id": runner_id,
                        "tool": reference.tool,
                        "resource": reference.resource,
                        "expires_at": expires_at,
                        "assurance": "environment_reduced_assurance",
                    }),
                ),
            )
            .await?
            {
                events.push(event);
            }
            grants.push(SecretGrantRecord {
                grant_id,
                secret_id: reference.secret_id,
                name: row.get("name"),
                ciphertext: row.get("ciphertext"),
                nonce: row.get("nonce"),
                env_name: reference.env_name.clone(),
                tool: reference.tool.clone(),
                resource: reference.resource.clone(),
                expires_at,
            });
        }
        tx.commit().await?;
        Ok((grants, events))
    }

    pub async fn agents_for_planning(&self, corp_id: Uuid) -> Result<Vec<Agent>> {
        sqlx::query(
            r#"
            SELECT id, corp_id, actor_id, name, role, adapter, status, station,
                   current_run_id, accent, created_at
            FROM agents
            WHERE corp_id = $1
            ORDER BY role, adapter, name, id
            "#,
        )
        .bind(corp_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(map_agent)
        .collect()
    }

    pub async fn bootstrap_demo(&self) -> Result<(DemoIds, Option<DomainEvent>)> {
        let ids = DemoIds {
            corp_id: parse_id(DEMO_CORP_ID)?,
            alice_actor_id: parse_id(DEMO_ALICE_ID)?,
            bob_actor_id: parse_id(DEMO_BOB_ID)?,
            eve_actor_id: parse_id(DEMO_EVE_ID)?,
            manager_agent_id: parse_id(DEMO_MANAGER_AGENT_ID)?,
            worker_agent_id: parse_id(DEMO_WORKER_AGENT_ID)?,
            codex_agent_id: parse_id(DEMO_CODEX_AGENT_ID)?,
            room_id: parse_id(DEMO_ROOM_ID)?,
        };
        let manager_actor_id = parse_id(DEMO_MANAGER_ACTOR_ID)?;
        let worker_actor_id = parse_id(DEMO_WORKER_ACTOR_ID)?;
        let codex_actor_id = parse_id(DEMO_CODEX_ACTOR_ID)?;
        let claude_actor_id = parse_id(DEMO_CLAUDE_ACTOR_ID)?;
        let opencode_actor_id = parse_id(DEMO_OPENCODE_ACTOR_ID)?;
        let copilot_actor_id = parse_id(DEMO_COPILOT_ACTOR_ID)?;

        let mut tx = self.pool.begin().await?;
        lock_demo_tx(&mut tx).await?;
        sqlx::query(
            r#"
            INSERT INTO corps (id, slug, name)
            VALUES ($1, 'ecorp-demo', 'ECorp Operations Network')
            ON CONFLICT (id) DO UPDATE
            SET slug = EXCLUDED.slug, name = EXCLUDED.name
            "#,
        )
        .bind(ids.corp_id)
        .execute(&mut *tx)
        .await?;

        for (id, name, kind, role) in [
            (ids.alice_actor_id, "Alice", "human", "owner"),
            (ids.bob_actor_id, "Bob", "human", "member"),
            (ids.eve_actor_id, "Eve", "human", "guest"),
            (manager_actor_id, "Margo", "agent", "manager"),
            (worker_actor_id, "Wally", "agent", "engineer"),
            (codex_actor_id, "Cody", "agent", "engineer"),
            (claude_actor_id, "Claudia", "agent", "engineer"),
            (opencode_actor_id, "Opal", "agent", "engineer"),
            (copilot_actor_id, "Piper", "agent", "engineer"),
        ] {
            sqlx::query(
                r#"
                INSERT INTO actors (id, corp_id, name, kind, role)
                VALUES ($1, $2, $3, $4, $5)
                ON CONFLICT (id) DO UPDATE
                SET name = EXCLUDED.name, kind = EXCLUDED.kind, role = EXCLUDED.role
                "#,
            )
            .bind(id)
            .bind(ids.corp_id)
            .bind(name)
            .bind(kind)
            .bind(role)
            .execute(&mut *tx)
            .await?;
        }

        sqlx::query(
            r#"
            INSERT INTO rooms (id, corp_id, name, purpose)
            VALUES ($1, $2, 'Automation Division', 'Governed workspace for human and autonomous operations')
            ON CONFLICT (id) DO UPDATE
            SET name = EXCLUDED.name, purpose = EXCLUDED.purpose
            "#,
        )
        .bind(ids.room_id)
        .bind(ids.corp_id)
        .execute(&mut *tx)
        .await?;

        for actor_id in [
            ids.alice_actor_id,
            ids.bob_actor_id,
            manager_actor_id,
            worker_actor_id,
            codex_actor_id,
            claude_actor_id,
            opencode_actor_id,
            copilot_actor_id,
        ] {
            sqlx::query(
                r#"
                INSERT INTO room_memberships (room_id, actor_id, role)
                VALUES ($1, $2, 'member')
                ON CONFLICT (room_id, actor_id) DO NOTHING
                "#,
            )
            .bind(ids.room_id)
            .bind(actor_id)
            .execute(&mut *tx)
            .await?;
        }

        for (id, actor_id, name, role, adapter, accent) in [
            (
                ids.manager_agent_id,
                manager_actor_id,
                "Margo",
                "manager",
                "fake-process",
                "marigold",
            ),
            (
                ids.worker_agent_id,
                worker_actor_id,
                "Wally",
                "engineer",
                "fake-process",
                "cobalt",
            ),
            (
                ids.codex_agent_id,
                codex_actor_id,
                "Cody",
                "engineer",
                "codex",
                "signal",
            ),
            (
                parse_id(DEMO_CLAUDE_AGENT_ID)?,
                claude_actor_id,
                "Claudia",
                "engineer",
                "claude-code",
                "violet",
            ),
            (
                parse_id(DEMO_OPENCODE_AGENT_ID)?,
                opencode_actor_id,
                "Opal",
                "engineer",
                "opencode",
                "mint",
            ),
            (
                parse_id(DEMO_COPILOT_AGENT_ID)?,
                copilot_actor_id,
                "Piper",
                "engineer",
                "github-copilot",
                "violet",
            ),
        ] {
            sqlx::query(
                r#"
                INSERT INTO agents
                    (id, corp_id, actor_id, name, role, adapter, status, accent)
                VALUES ($1, $2, $3, $4, $5, $6, 'idle', $7)
                ON CONFLICT (id) DO UPDATE
                SET name = EXCLUDED.name, role = EXCLUDED.role, adapter = EXCLUDED.adapter,
                    accent = EXCLUDED.accent
                "#,
            )
            .bind(id)
            .bind(ids.corp_id)
            .bind(actor_id)
            .bind(name)
            .bind(role)
            .bind(adapter)
            .bind(accent)
            .execute(&mut *tx)
            .await?;
        }

        let event = append_event_tx(
            &mut tx,
            NewEvent::new(
                ids.corp_id,
                Some(ids.alice_actor_id),
                "corp.demo_bootstrapped",
                "corp",
                ids.corp_id,
                "demo-bootstrap-v1",
                json!({
                    "name": "ECorp Operations Network",
                    "actors": ["Alice", "Bob", "Eve", "Margo", "Wally", "Cody", "Claudia", "Opal", "Piper"]
                }),
            ),
        )
        .await?;
        tx.commit().await?;
        Ok((ids, event))
    }

    pub async fn reset_demo(&self) -> Result<(DemoIds, Option<DomainEvent>)> {
        let corp_id = parse_id(DEMO_CORP_ID)?;
        let mut tx = self.pool.begin().await?;
        lock_demo_tx(&mut tx).await?;
        sqlx::query("DELETE FROM corp_budget_policies WHERE corp_id = $1")
            .bind(corp_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM control_leases WHERE corp_id = $1")
            .bind(corp_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM queued_messages WHERE corp_id = $1")
            .bind(corp_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM room_messages WHERE corp_id = $1")
            .bind(corp_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM pull_request_publication_attempts WHERE corp_id = $1")
            .bind(corp_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM pull_request_publications WHERE corp_id = $1")
            .bind(corp_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM factory_work_items WHERE corp_id = $1")
            .bind(corp_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM mission_contract_revisions WHERE corp_id = $1")
            .bind(corp_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM mission_budget_revisions WHERE corp_id = $1")
            .bind(corp_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM runs WHERE corp_id = $1")
            .bind(corp_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM tasks WHERE corp_id = $1")
            .bind(corp_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM missions WHERE corp_id = $1")
            .bind(corp_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM events WHERE corp_id = $1")
            .bind(corp_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE agents SET status = 'idle', station = NULL, current_run_id = NULL WHERE corp_id = $1",
        )
        .bind(corp_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.bootstrap_demo().await
    }

    pub async fn snapshot(&self, corp_id: Uuid, viewer_actor_id: Uuid) -> Result<CorpSnapshot> {
        if !self.actor_belongs_to_corp(corp_id, viewer_actor_id).await? {
            return Err(anyhow!(
                "viewer actor does not belong to the requested Corp"
            ));
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        let corp = map_corp(
            sqlx::query("SELECT id, slug, name, created_at FROM corps WHERE id = $1")
                .bind(corp_id)
                .fetch_one(&mut *tx)
                .await
                .context("corp not found")?,
        );

        let actors = sqlx::query(
            "SELECT id, corp_id, name, kind, role, created_at FROM actors WHERE corp_id = $1 ORDER BY created_at, name",
        )
        .bind(corp_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(map_actor)
        .collect::<Result<Vec<_>>>()?;

        let rooms = sqlx::query(
            r#"
            SELECT r.id, r.corp_id, r.name, r.purpose, r.created_at
            FROM rooms r
            JOIN room_memberships rm ON rm.room_id = r.id
            WHERE r.corp_id = $1 AND rm.actor_id = $2
            ORDER BY r.name
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(map_room)
        .collect();

        let agents = sqlx::query(
            r#"
            SELECT id, corp_id, actor_id, name, role, adapter, status, station,
                   current_run_id, accent, created_at
            FROM agents WHERE corp_id = $1 ORDER BY role, name
            "#,
        )
        .bind(corp_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(map_agent)
        .collect::<Result<Vec<_>>>()?;

        let missions = sqlx::query(
            r#"
            SELECT m.id, m.corp_id, m.room_id, m.requested_by, m.title,
                   m.description, m.specification_version, m.strategy,
                   m.max_nodes, m.max_depth, m.original_budget_tokens,
                   m.original_budget_cost_microusd, m.budget_tokens,
                   m.budget_cost_microusd, m.status, m.created_at, m.updated_at
            FROM missions m
            JOIN room_memberships rm ON rm.room_id = m.room_id
            WHERE m.corp_id = $1 AND rm.actor_id = $2
            ORDER BY m.created_at DESC
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(map_mission)
        .collect::<Result<Vec<_>>>()?;

        let mission_contract_revisions = sqlx::query(
            r#"
            SELECT revision.id, revision.corp_id, revision.mission_id, revision.task_id,
                   revision.version, revision.revised_by, revision.next_action,
                   revision.source_run_id, revision.reason, revision.previous_description,
                   revision.replacement_description, revision.previous_contract,
                   revision.replacement_contract, revision.previous_verification_policy,
                   revision.replacement_verification_policy, revision.created_at
            FROM mission_contract_revisions revision
            JOIN missions mission ON mission.id = revision.mission_id
            JOIN room_memberships membership ON membership.room_id = mission.room_id
            WHERE revision.corp_id = $1 AND membership.actor_id = $2
            ORDER BY revision.created_at DESC
            LIMIT 500
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(map_mission_contract_revision)
        .collect::<Result<Vec<_>>>()?;

        let mission_budget_revisions = sqlx::query(
            r#"
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
            JOIN missions mission ON mission.id = revision.mission_id
            JOIN room_memberships membership ON membership.room_id = mission.room_id
            WHERE revision.corp_id = $1 AND membership.actor_id = $2
            ORDER BY revision.created_at DESC
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(map_mission_budget_revision)
        .collect::<Result<Vec<_>>>()?;

        let mut tasks = sqlx::query(
            r#"
            SELECT t.id, t.mission_id, t.corp_id, t.title, t.objective, t.plan_key,
                   t.contract, t.contract_version, t.depth, t.max_attempts, t.attempt_count,
                   t.required_adapter, t.verification_policy, t.verification_status,
                   t.status, t.assigned_agent_id,
                   t.created_at, t.updated_at
            FROM tasks t
            JOIN missions m ON m.id = t.mission_id
            JOIN room_memberships rm ON rm.room_id = m.room_id
            WHERE t.corp_id = $1 AND rm.actor_id = $2
            ORDER BY t.created_at DESC
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(map_task)
        .collect::<Result<Vec<_>>>()?;

        let dependency_rows = sqlx::query(
            r#"
            SELECT dependency.task_id, dependency.depends_on_task_id
            FROM task_dependencies dependency
            JOIN tasks task ON task.id = dependency.task_id
            JOIN missions mission ON mission.id = task.mission_id
            JOIN room_memberships membership ON membership.room_id = mission.room_id
            WHERE task.corp_id = $1 AND membership.actor_id = $2
            ORDER BY dependency.task_id, dependency.depends_on_task_id
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&mut *tx)
        .await?;
        let mut dependencies = HashMap::<Uuid, Vec<Uuid>>::new();
        for row in dependency_rows {
            dependencies
                .entry(row.get("task_id"))
                .or_default()
                .push(row.get("depends_on_task_id"));
        }
        for task in &mut tasks {
            task.depends_on = dependencies.remove(&task.id).unwrap_or_default();
        }

        let runs = sqlx::query(
            r#"
            SELECT r.id, r.corp_id, r.task_id, r.agent_id, r.runner_id,
                   r.assignment_token, r.provider_session_id, r.resumed_from_run_id,
                   r.workspace_run_id, r.model, r.reasoning_effort,
                   r.input_tokens, r.output_tokens, r.cost_microusd,
                   r.budget_tokens_limit, r.budget_cost_microusd_limit, r.breaker_stage,
                   r.no_progress_events, r.repeated_tool_count,
                   r.source_repository, r.source_base_ref, r.source_base_commit,
                   r.workspace_path, r.workspace_branch, r.workspace_base_ref,
                   r.workspace_base_commit, r.workspace_disposition, r.workspace_detail,
                   r.verification_status, r.verification_summary,
                   r.verification_sha256, r.deliverable_sha256, r.status,
                   r.summary, r.artifact_id, r.artifact_uri, r.artifact_media_type,
                   r.artifact_signature, r.artifact_path, r.artifact_sha256,
                   r.created_at, r.updated_at
            FROM runs r
            JOIN tasks t ON t.id = r.task_id
            JOIN missions m ON m.id = t.mission_id
            JOIN room_memberships rm ON rm.room_id = m.room_id
            WHERE r.corp_id = $1 AND rm.actor_id = $2
            ORDER BY r.created_at DESC
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(map_run)
        .collect::<Result<Vec<_>>>()?;

        let leases = sqlx::query(
            r#"
            SELECT agent_id, corp_id, actor_id, token, expires_at
            FROM control_leases WHERE corp_id = $1 AND expires_at > now()
            ORDER BY agent_id
            "#,
        )
        .bind(corp_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(map_lease)
        .collect();

        let queued_messages = sqlx::query(
            r#"
            SELECT id, corp_id, agent_id, actor_id, text, status, created_at
            FROM queued_messages WHERE corp_id = $1 ORDER BY created_at DESC LIMIT 100
            "#,
        )
        .bind(corp_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(map_queued_message)
        .collect();

        let room_messages = sqlx::query(
            r#"
            WITH latest AS (
                SELECT msg.id, msg.corp_id, msg.room_id, msg.actor_id, msg.thread_root_id,
                       msg.reply_to_id, msg.body, msg.mentions, msg.link_kind, msg.link_id,
                       msg.created_at
                FROM room_messages msg
                JOIN room_memberships rm ON rm.room_id = msg.room_id
                WHERE msg.corp_id = $1 AND rm.actor_id = $2
                ORDER BY msg.created_at DESC
                LIMIT 500
            )
            SELECT id, corp_id, room_id, actor_id, thread_root_id, reply_to_id,
                   body, mentions, link_kind, link_id, created_at
            FROM latest
            ORDER BY created_at ASC
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(map_room_message)
        .collect();

        let verification_evidence = sqlx::query(
            r#"
            SELECT evidence.id, evidence.corp_id, evidence.task_id, evidence.run_id,
                   evidence.check_index, evidence.kind, evidence.status, evidence.summary,
                   evidence.payload, evidence.created_at
            FROM verification_evidence evidence
            JOIN tasks task ON task.id = evidence.task_id
            JOIN missions mission ON mission.id = task.mission_id
            JOIN room_memberships membership ON membership.room_id = mission.room_id
            WHERE evidence.corp_id = $1 AND membership.actor_id = $2
            ORDER BY evidence.created_at, evidence.check_index
            LIMIT 1000
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(map_verification_evidence)
        .collect();

        let verification_requests = sqlx::query(
            r#"
            SELECT request.run_id, request.corp_id, request.task_id, request.gate_type,
                   request.gate, request.status, request.requested_at, request.decided_by,
                   request.decision_note, request.decided_at
            FROM verification_requests request
            JOIN tasks task ON task.id = request.task_id
            JOIN missions mission ON mission.id = task.mission_id
            JOIN room_memberships membership ON membership.room_id = mission.room_id
            WHERE request.corp_id = $1 AND membership.actor_id = $2
            ORDER BY request.requested_at DESC
            LIMIT 500
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(map_verification_request)
        .collect::<Result<Vec<_>>>()?;

        let source_deliverables = sqlx::query(
            r#"
            SELECT deliverable.id, deliverable.corp_id, deliverable.task_id, deliverable.run_id,
                   deliverable.artifact_id, deliverable.form, deliverable.file_name,
                   artifact.uri, artifact.sha256, artifact.media_type, artifact.bytes,
                   artifact.provenance_signature, deliverable.verification_sha256,
                   deliverable.base_commit, deliverable.head_commit, deliverable.branch,
                   deliverable.integration_state, artifact.retention_until,
                   deliverable.created_at
            FROM source_deliverables deliverable
            JOIN artifacts artifact
              ON artifact.id = deliverable.artifact_id AND artifact.status = 'ready'
            JOIN tasks task ON task.id = deliverable.task_id
            JOIN missions mission ON mission.id = task.mission_id
            JOIN room_memberships membership ON membership.room_id = mission.room_id
            WHERE deliverable.corp_id = $1 AND membership.actor_id = $2
            ORDER BY deliverable.created_at DESC
            LIMIT 500
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(map_source_deliverable)
        .collect::<Result<Vec<_>>>()?;

        let pull_request_publications = sqlx::query(
            r#"
            SELECT publication.id, publication.corp_id, publication.factory_work_item_id,
                   publication.mission_id, publication.source_deliverable_id,
                   publication.artifact_id, publication.task_id, publication.run_id,
                   publication.source_issue_number, publication.source_issue_url,
                   publication.target_repository, publication.base_ref, publication.branch,
                   publication.commit_sha, publication.title, publication.body,
                   publication.actor_id, publication.authorization_id,
                   publication.authorization_snapshot, publication.effect_key,
                   publication.idempotency_key, publication.state, publication.version,
                   publication.attempt_count, publication.publisher_id,
                   publication.publisher_lease_expires_at, publication.failure_detail,
                   publication.branch_pushed_at, publication.pull_request_number,
                   publication.pull_request_node_id, publication.pull_request_url,
                   publication.pull_request_state, publication.pull_request_draft,
                   publication.pull_request_base_ref,
                   publication.pull_request_head_sha,
                   publication.pull_request_head_repository_owner,
                   publication.pull_request_is_cross_repository,
                   publication.project_owner, publication.project_number,
                   publication.project_item_id, publication.project_status_before,
                   publication.project_status_after, publication.project_status_updated_at,
                   publication.auto_merge_enabled, publication.merge_authorized,
                   publication.deployment_authorized, publication.provenance,
                   publication.created_at, publication.updated_at
            FROM pull_request_publications publication
            JOIN missions mission ON mission.id = publication.mission_id
            JOIN room_memberships membership ON membership.room_id = mission.room_id
            WHERE publication.corp_id = $1
              AND membership.actor_id = $2
              AND EXISTS (
                  SELECT 1
                  FROM actors viewer
                  WHERE viewer.id = $2
                    AND viewer.corp_id = publication.corp_id
                    AND viewer.kind = 'human'
                    AND viewer.role IN ('owner', 'admin', 'manager', 'member')
              )
            ORDER BY publication.created_at DESC
            LIMIT 500
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(map_pull_request_publication)
        .collect::<Result<Vec<_>>>()?;

        let pull_request_publication_attempts = sqlx::query(
            r#"
            SELECT attempt.id, attempt.corp_id, attempt.publication_id, attempt.attempt,
                   attempt.actor_id, attempt.authorization_id, attempt.authorization_snapshot,
                   attempt.publisher_id, attempt.state, attempt.failure_detail,
                   attempt.started_at, attempt.finished_at
            FROM pull_request_publication_attempts attempt
            JOIN pull_request_publications publication ON publication.id = attempt.publication_id
            JOIN missions mission ON mission.id = publication.mission_id
            JOIN room_memberships membership ON membership.room_id = mission.room_id
            WHERE attempt.corp_id = $1
              AND membership.actor_id = $2
              AND EXISTS (
                  SELECT 1
                  FROM actors viewer
                  WHERE viewer.id = $2
                    AND viewer.corp_id = attempt.corp_id
                    AND viewer.kind = 'human'
                    AND viewer.role IN ('owner', 'admin', 'manager', 'member')
              )
            ORDER BY attempt.started_at DESC
            LIMIT 1000
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(map_pull_request_publication_attempt)
        .collect();

        let action_approvals = sqlx::query(
            r#"
            SELECT approval.id, approval.corp_id, approval.room_id, approval.mission_id,
                   approval.task_id, approval.run_id, approval.agent_id, approval.action_key,
                   approval.action, approval.risk, approval.rationale, approval.required_roles,
                   approval.status, approval.expires_at, approval.created_at,
                   approval.decided_by, approval.decision_note, approval.decided_at
            FROM action_approvals approval
            JOIN room_memberships membership ON membership.room_id = approval.room_id
            WHERE approval.corp_id = $1 AND membership.actor_id = $2
            ORDER BY approval.created_at DESC
            LIMIT 500
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(map_action_approval)
        .collect();

        let circuit_breaker_incidents = sqlx::query(
            r#"
            SELECT incident.id, incident.corp_id, incident.mission_id, incident.task_id,
                   incident.run_id, incident.stage, incident.reason, incident.input,
                   incident.created_at
            FROM circuit_breaker_incidents incident
            JOIN tasks task ON task.id = incident.task_id
            JOIN missions mission ON mission.id = task.mission_id
            JOIN room_memberships membership ON membership.room_id = mission.room_id
            WHERE incident.corp_id = $1 AND membership.actor_id = $2
            ORDER BY incident.created_at DESC
            LIMIT 500
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(map_circuit_breaker_incident)
        .collect();

        let factory_work_items = sqlx::query(
            r#"
            SELECT id, corp_id, source_kind, source_project_owner, source_project_number,
                   source_project_item_id, source_repository_owner, source_repository_name,
                   source_issue_number, source_issue_node_id, source_issue_url, source_title,
                   source_revision, state, version, claim_owner_id, lease_expires_at, policy,
                   mission_id, failure_detail, created_at, updated_at
            FROM factory_work_items
            WHERE corp_id = $1
              AND EXISTS (
                  SELECT 1
                  FROM actors viewer
                  WHERE viewer.id = $2
                    AND viewer.corp_id = factory_work_items.corp_id
                    AND viewer.kind = 'human'
                    AND viewer.role IN ('owner', 'admin', 'manager', 'member')
              )
            ORDER BY created_at DESC
            LIMIT 500
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(map_factory_work_item)
        .collect::<Result<Vec<_>>>()?;

        let mut events = sqlx::query(
            r#"
            SELECT seq, id, schema_version, corp_id, room_id, actor_id, type,
                   aggregate_type, aggregate_id, aggregate_version, correlation_id,
                   causation_id, idempotency_key, visibility, payload, created_at
            FROM events e
            WHERE e.corp_id = $1
              AND (
                e.room_id IS NULL
                OR EXISTS (
                    SELECT 1 FROM room_memberships rm
                    WHERE rm.room_id = e.room_id AND rm.actor_id = $2
                )
              )
            ORDER BY e.seq DESC
            LIMIT 200
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(map_event)
        .collect::<Vec<_>>();
        events.reverse();

        let snapshot = CorpSnapshot {
            corp,
            actors,
            rooms,
            agents,
            missions,
            mission_contract_revisions,
            mission_budget_revisions,
            tasks,
            runs,
            room_messages,
            leases,
            queued_messages,
            verification_evidence,
            verification_requests,
            source_deliverables,
            pull_request_publications,
            pull_request_publication_attempts,
            action_approvals,
            circuit_breaker_incidents,
            factory_work_items,
            events,
        };
        tx.commit().await?;
        Ok(snapshot)
    }

    pub async fn factory_work_items_by_project_item_ids(
        &self,
        corp_id: Uuid,
        viewer_actor_id: Uuid,
        source_project_owner: &str,
        source_project_number: i64,
        source_project_item_ids: &[String],
    ) -> Result<Vec<FactoryWorkItem>> {
        if source_project_number <= 0 {
            return Err(anyhow!("factory source project number must be positive"));
        }
        let source_project_owner =
            normalize_github_component(source_project_owner, "source project owner", 100)?;
        if source_project_item_ids.len() > 1_000 {
            return Err(anyhow!(
                "factory work-item lookup cannot exceed 1,000 Project item ids"
            ));
        }
        let mut normalized_ids = source_project_item_ids
            .iter()
            .map(|value| normalize_factory_identifier(value, "source Project item id", 160))
            .collect::<Result<Vec<_>>>()?;
        normalized_ids.sort();
        normalized_ids.dedup();
        if normalized_ids.is_empty() {
            return Ok(Vec::new());
        }

        sqlx::query(
            r#"
            SELECT id, corp_id, source_kind, source_project_owner, source_project_number,
                   source_project_item_id, source_repository_owner, source_repository_name,
                   source_issue_number, source_issue_node_id, source_issue_url, source_title,
                   source_revision, state, version, claim_owner_id, lease_expires_at, policy,
                   mission_id, failure_detail, created_at, updated_at
            FROM factory_work_items
            WHERE corp_id = $1
              AND source_kind = 'github_project_issue'
              AND source_project_owner = $3
              AND source_project_number = $4
              AND source_project_item_id = ANY($5)
              AND EXISTS (
                  SELECT 1
                  FROM actors viewer
                  WHERE viewer.id = $2
                    AND viewer.corp_id = factory_work_items.corp_id
                    AND viewer.kind = 'human'
                    AND viewer.role IN ('owner', 'admin', 'manager', 'member')
              )
            ORDER BY created_at DESC
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .bind(source_project_owner)
        .bind(source_project_number)
        .bind(&normalized_ids)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(map_factory_work_item)
        .collect()
    }

    pub async fn events_after(
        &self,
        corp_id: Uuid,
        viewer_actor_id: Uuid,
        after_seq: i64,
        limit: i64,
    ) -> Result<Vec<DomainEvent>> {
        let bounded_limit = limit.clamp(1, 1_000);
        let events = sqlx::query(
            r#"
            SELECT seq, id, schema_version, corp_id, room_id, actor_id, type,
                   aggregate_type, aggregate_id, aggregate_version, correlation_id,
                   causation_id, idempotency_key, visibility, payload, created_at
            FROM events e
            WHERE e.corp_id = $1 AND e.seq > $2
              AND (
                e.room_id IS NULL
                OR EXISTS (
                    SELECT 1 FROM room_memberships rm
                    WHERE rm.room_id = e.room_id AND rm.actor_id = $3
                )
              )
            ORDER BY e.seq ASC
            LIMIT $4
            "#,
        )
        .bind(corp_id)
        .bind(after_seq)
        .bind(viewer_actor_id)
        .bind(bounded_limit)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(map_event)
        .collect();
        Ok(events)
    }

    pub async fn actor_belongs_to_corp(&self, corp_id: Uuid, actor_id: Uuid) -> Result<bool> {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM actors WHERE id = $1 AND corp_id = $2)",
        )
        .bind(actor_id)
        .bind(corp_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(exists)
    }

    pub async fn visible_room_ids(&self, corp_id: Uuid, actor_id: Uuid) -> Result<Vec<Uuid>> {
        let rooms = sqlx::query_scalar(
            r#"
            SELECT rm.room_id
            FROM room_memberships rm
            JOIN rooms r ON r.id = rm.room_id
            WHERE r.corp_id = $1 AND rm.actor_id = $2
            "#,
        )
        .bind(corp_id)
        .bind(actor_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rooms)
    }

    pub async fn create_publication_publisher_credential(
        &self,
        corp_id: Uuid,
        actor_id: Uuid,
        publisher_id: &str,
        credential_hash: &str,
        expires_at: chrono::DateTime<Utc>,
    ) -> Result<PublicationPublisherCredentialOutcome> {
        let publisher_id = normalize_factory_identifier(publisher_id, "trusted publisher id", 160)?;
        let credential_hash = credential_hash.trim().to_ascii_lowercase();
        if credential_hash.len() != 64
            || !credential_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(anyhow!(
                "publication publisher credential hash must be a SHA-256 digest"
            ));
        }
        if expires_at <= Utc::now() {
            return Err(anyhow!(
                "publication publisher credential expiry must be in the future"
            ));
        }

        let mut tx = self.pool.begin().await?;
        let role: Option<String> = sqlx::query_scalar(
            "SELECT role FROM actors WHERE id = $1 AND corp_id = $2 AND kind = 'human'",
        )
        .bind(actor_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?;
        if !matches!(role.as_deref(), Some("owner" | "admin")) {
            return Err(anyhow!(
                "forbidden: only a Corp owner or admin can enroll a publication publisher"
            ));
        }

        let credential_id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO publication_publisher_credentials
                (id, corp_id, publisher_id, credential_hash, created_by, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(credential_id)
        .bind(corp_id)
        .bind(&publisher_id)
        .bind(&credential_hash)
        .bind(actor_id)
        .bind(expires_at)
        .execute(&mut *tx)
        .await?;
        let event = append_event_tx(
            &mut tx,
            NewEvent::new(
                corp_id,
                Some(actor_id),
                "factory.publication_publisher_enrolled",
                "publication_publisher",
                credential_id,
                format!("publication-publisher-enrolled:{credential_id}"),
                json!({
                    "publisher_id": publisher_id,
                    "expires_at": expires_at
                }),
            ),
        )
        .await?
        .context("publication publisher enrollment event was unexpectedly deduplicated")?;
        tx.commit().await?;
        Ok(PublicationPublisherCredentialOutcome {
            credential_id,
            publisher_id,
            expires_at,
            event,
        })
    }

    pub async fn authenticate_publication_publisher(
        &self,
        corp_id: Uuid,
        credential_hash: &str,
    ) -> Result<String> {
        let credential_hash = credential_hash.trim().to_ascii_lowercase();
        if credential_hash.len() != 64
            || !credential_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(anyhow!(
                "forbidden: invalid publication publisher credential"
            ));
        }
        let publisher_id = sqlx::query_scalar(
            r#"
            UPDATE publication_publisher_credentials
            SET last_used_at = now()
            WHERE id = (
                SELECT id
                FROM publication_publisher_credentials
                WHERE corp_id = $1
                  AND credential_hash = $2
                  AND revoked_at IS NULL
                  AND expires_at > now()
                FOR UPDATE
            )
            RETURNING publisher_id
            "#,
        )
        .bind(corp_id)
        .bind(&credential_hash)
        .fetch_optional(&self.pool)
        .await?
        .context("forbidden: unknown, expired, or revoked publication publisher credential")?;
        Ok(publisher_id)
    }

    pub async fn revoke_publication_publisher_credential(
        &self,
        corp_id: Uuid,
        actor_id: Uuid,
        credential_id: Uuid,
        reason: &str,
    ) -> Result<PublicationPublisherCredentialRevocationOutcome> {
        let reason =
            normalize_factory_text(reason, "publisher credential revocation reason", 2_000)?;
        let mut tx = self.pool.begin().await?;
        let role: Option<String> = sqlx::query_scalar(
            "SELECT role FROM actors WHERE id = $1 AND corp_id = $2 AND kind = 'human'",
        )
        .bind(actor_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?;
        if !matches!(role.as_deref(), Some("owner" | "admin")) {
            return Err(anyhow!(
                "forbidden: only a Corp owner or admin can revoke a publication publisher"
            ));
        }
        let row = sqlx::query(
            r#"
            SELECT publisher_id, revoked_at
            FROM publication_publisher_credentials
            WHERE id = $1 AND corp_id = $2
            FOR UPDATE
            "#,
        )
        .bind(credential_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?
        .context("publication publisher credential was not found")?;
        let publisher_id: String = row.get("publisher_id");
        let already_revoked = row
            .get::<Option<chrono::DateTime<Utc>>, _>("revoked_at")
            .is_some();
        let event = if already_revoked {
            None
        } else {
            sqlx::query(
                "UPDATE publication_publisher_credentials SET revoked_at = now() WHERE id = $1",
            )
            .bind(credential_id)
            .execute(&mut *tx)
            .await?;
            append_event_tx(
                &mut tx,
                NewEvent::new(
                    corp_id,
                    Some(actor_id),
                    "factory.publication_publisher_revoked",
                    "publication_publisher",
                    credential_id,
                    format!("publication-publisher-revoked:{credential_id}"),
                    json!({
                        "publisher_id": publisher_id.clone(),
                        "reason": reason
                    }),
                ),
            )
            .await?
        };
        tx.commit().await?;
        Ok(PublicationPublisherCredentialRevocationOutcome {
            credential_id,
            publisher_id,
            revoked: !already_revoked,
            event,
        })
    }

    pub async fn create_runner_enrollment(
        &self,
        corp_id: Uuid,
        actor_id: Uuid,
        runner_id: &str,
        token_hash: &str,
        expires_at: chrono::DateTime<Utc>,
    ) -> Result<RunnerEnrollmentOutcome> {
        let runner_id = runner_id.trim();
        if runner_id.is_empty() || runner_id.len() > 128 {
            return Err(anyhow!(
                "runner id must contain between 1 and 128 characters"
            ));
        }
        let mut tx = self.pool.begin().await?;
        let role: Option<String> = sqlx::query_scalar(
            "SELECT role FROM actors WHERE id = $1 AND corp_id = $2 AND kind = 'human'",
        )
        .bind(actor_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?;
        if !matches!(role.as_deref(), Some("owner" | "admin")) {
            return Err(anyhow!(
                "forbidden: only a Corp owner or admin can enroll a runner"
            ));
        }
        let enrollment_id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO runner_enrollment_tokens
                (id, corp_id, runner_id, token_hash, created_by, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(enrollment_id)
        .bind(corp_id)
        .bind(runner_id)
        .bind(token_hash)
        .bind(actor_id)
        .bind(expires_at)
        .execute(&mut *tx)
        .await?;
        let event = append_event_tx(
            &mut tx,
            NewEvent::new(
                corp_id,
                Some(actor_id),
                "runner.enrollment_created",
                "runner_enrollment",
                enrollment_id,
                format!("runner-enrollment-created:{enrollment_id}"),
                json!({
                    "runner_id": runner_id,
                    "expires_at": expires_at,
                }),
            ),
        )
        .await?
        .context("new runner enrollment event was unexpectedly deduplicated")?;
        tx.commit().await?;
        Ok(RunnerEnrollmentOutcome {
            enrollment_id,
            expires_at,
            event,
        })
    }

    pub async fn authenticate_and_rotate_runner(
        &self,
        corp_id: Uuid,
        runner_id: &str,
        presented_token_hash: &str,
        next_token_hash: &str,
        next_expires_at: chrono::DateTime<Utc>,
    ) -> Result<RunnerAuthenticationOutcome> {
        let mut tx = self.pool.begin().await?;

        let credential = sqlx::query(
            r#"
            SELECT id, enrolled_by
            FROM runner_credentials
            WHERE runner_id = $1
              AND corp_id = $2
              AND token_hash = $3
              AND revoked_at IS NULL
              AND expires_at > now()
            FOR UPDATE
            "#,
        )
        .bind(runner_id)
        .bind(corp_id)
        .bind(presented_token_hash)
        .fetch_optional(&mut *tx)
        .await?;

        let (credential_id, actor_id, event_type) = if let Some(row) = credential {
            (
                row.get::<Uuid, _>("id"),
                row.get::<Uuid, _>("enrolled_by"),
                "runner.credential_rotated",
            )
        } else {
            let enrollment = sqlx::query(
                r#"
                SELECT id, created_by
                FROM runner_enrollment_tokens
                WHERE corp_id = $1
                  AND runner_id = $2
                  AND token_hash = $3
                  AND used_at IS NULL
                  AND expires_at > now()
                FOR UPDATE
                "#,
            )
            .bind(corp_id)
            .bind(runner_id)
            .bind(presented_token_hash)
            .fetch_optional(&mut *tx)
            .await?
            .context("forbidden: unknown, expired, replayed, or revoked runner credential")?;
            let enrollment_id: Uuid = enrollment.get("id");
            let actor_id: Uuid = enrollment.get("created_by");
            sqlx::query(
                "UPDATE runner_enrollment_tokens SET used_at = now() WHERE id = $1 AND used_at IS NULL",
            )
            .bind(enrollment_id)
            .execute(&mut *tx)
            .await?;
            let credential_id = Uuid::new_v4();
            sqlx::query(
                r#"
                INSERT INTO runner_credentials
                    (id, runner_id, corp_id, token_hash, expires_at, enrolled_by)
                VALUES ($1, $2, $3, $4, $5, $6)
                ON CONFLICT (runner_id) DO UPDATE
                SET id = EXCLUDED.id,
                    corp_id = EXCLUDED.corp_id,
                    token_hash = EXCLUDED.token_hash,
                    expires_at = EXCLUDED.expires_at,
                    revoked_at = NULL,
                    enrolled_by = EXCLUDED.enrolled_by,
                    rotated_at = now()
                "#,
            )
            .bind(credential_id)
            .bind(runner_id)
            .bind(corp_id)
            .bind(next_token_hash)
            .bind(next_expires_at)
            .bind(actor_id)
            .execute(&mut *tx)
            .await?;
            (credential_id, actor_id, "runner.enrolled")
        };

        if event_type == "runner.credential_rotated" {
            sqlx::query(
                r#"
                UPDATE runner_credentials
                SET token_hash = $1, expires_at = $2, rotated_at = now()
                WHERE id = $3
                "#,
            )
            .bind(next_token_hash)
            .bind(next_expires_at)
            .bind(credential_id)
            .execute(&mut *tx)
            .await?;
        }

        let event = append_event_tx(
            &mut tx,
            NewEvent::new(
                corp_id,
                Some(actor_id),
                event_type,
                "runner",
                credential_id,
                format!("{event_type}:{credential_id}:{}", Uuid::new_v4()),
                json!({
                    "runner_id": runner_id,
                    "credential_expires_at": next_expires_at,
                }),
            ),
        )
        .await?
        .context("runner authentication event was unexpectedly deduplicated")?;
        tx.commit().await?;
        Ok(RunnerAuthenticationOutcome {
            corp_id,
            expires_at: next_expires_at,
            event,
        })
    }

    pub async fn revoke_runner(
        &self,
        corp_id: Uuid,
        actor_id: Uuid,
        runner_id: &str,
        reason: &str,
    ) -> Result<RunnerRevocationOutcome> {
        let mut tx = self.pool.begin().await?;
        let role: Option<String> = sqlx::query_scalar(
            "SELECT role FROM actors WHERE id = $1 AND corp_id = $2 AND kind = 'human'",
        )
        .bind(actor_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?;
        if !matches!(role.as_deref(), Some("owner" | "admin")) {
            return Err(anyhow!(
                "forbidden: only a Corp owner or admin can revoke a runner"
            ));
        }
        let row = sqlx::query(
            r#"
            UPDATE runner_credentials
            SET revoked_at = now()
            WHERE runner_id = $1 AND corp_id = $2 AND revoked_at IS NULL
            RETURNING id
            "#,
        )
        .bind(runner_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            tx.commit().await?;
            return Ok(RunnerRevocationOutcome {
                revoked: false,
                event: None,
                run_events: Vec::new(),
            });
        };
        let credential_id: Uuid = row.get("id");
        let run_events = mark_runner_runs_lost_tx(
            &mut tx,
            runner_id,
            &[],
            None,
            &format!("runner credential revoked: {reason}"),
        )
        .await?;
        sqlx::query("UPDATE runner_nodes SET status = 'offline' WHERE id = $1 AND corp_id = $2")
            .bind(runner_id)
            .bind(corp_id)
            .execute(&mut *tx)
            .await?;
        let event = append_event_tx(
            &mut tx,
            NewEvent::new(
                corp_id,
                Some(actor_id),
                "runner.revoked",
                "runner",
                credential_id,
                format!("runner-revoked:{credential_id}"),
                json!({"runner_id": runner_id, "reason": reason}),
            ),
        )
        .await?;
        tx.commit().await?;
        Ok(RunnerRevocationOutcome {
            revoked: true,
            event,
            run_events,
        })
    }

    pub async fn runner_connected(&self, input: RunnerConnectInput) -> Result<RunnerRecord> {
        let row = sqlx::query(
            r#"
            INSERT INTO runner_nodes
                (id, corp_id, hostname, os, capabilities, connection_epoch, status,
                 connected_at, last_seen_at, disconnected_at, grace_expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, 'connected', now(), now(), NULL, NULL)
            ON CONFLICT (id) DO UPDATE
            SET corp_id = EXCLUDED.corp_id,
                hostname = EXCLUDED.hostname,
                os = EXCLUDED.os,
                capabilities = EXCLUDED.capabilities,
                connection_epoch = EXCLUDED.connection_epoch,
                status = 'connected',
                connected_at = now(),
                last_seen_at = now(),
                disconnected_at = NULL,
                grace_expires_at = NULL
            RETURNING id, corp_id, hostname, os, capabilities, connection_epoch, status,
                      last_seen_at, grace_expires_at
            "#,
        )
        .bind(input.id)
        .bind(input.corp_id)
        .bind(input.hostname)
        .bind(input.os)
        .bind(input.capabilities)
        .bind(input.connection_epoch)
        .fetch_one(&self.pool)
        .await?;
        Ok(map_runner(row))
    }

    pub async fn runner_heartbeat(&self, runner_id: &str, connection_epoch: Uuid) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE runner_nodes
            SET last_seen_at = now()
            WHERE id = $1 AND connection_epoch = $2 AND status = 'connected'
            "#,
        )
        .bind(runner_id)
        .bind(connection_epoch)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn runner_records(&self, corp_id: Uuid) -> Result<Vec<RunnerRecord>> {
        let records = sqlx::query(
            r#"
            SELECT id, corp_id, hostname, os, capabilities, connection_epoch, status,
                   last_seen_at, grace_expires_at
            FROM runner_nodes
            WHERE corp_id = $1
            ORDER BY id
            "#,
        )
        .bind(corp_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(map_runner)
        .collect();
        Ok(records)
    }

    pub async fn connected_runner_count(&self) -> Result<i64> {
        sqlx::query_scalar("SELECT count(*) FROM runner_nodes WHERE status = 'connected'")
            .fetch_one(&self.pool)
            .await
            .context("count connected runners")
    }

    pub async fn runner_records_requiring_recovery(&self) -> Result<Vec<RunnerRecord>> {
        let records = sqlx::query(
            r#"
            SELECT id, corp_id, hostname, os, capabilities, connection_epoch, status,
                   last_seen_at, grace_expires_at
            FROM runner_nodes
            WHERE status IN ('connected', 'grace')
            ORDER BY id
            "#,
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(map_runner)
        .collect();
        Ok(records)
    }

    pub async fn runner_disconnected(
        &self,
        runner_id: &str,
        connection_epoch: Uuid,
        grace_seconds: i64,
    ) -> Result<Vec<DomainEvent>> {
        let mut tx = self.pool.begin().await?;
        let grace_expires_at = Utc::now() + Duration::seconds(grace_seconds.max(1));
        let result = sqlx::query(
            r#"
            UPDATE runner_nodes
            SET status = 'grace', disconnected_at = now(), grace_expires_at = $3
            WHERE id = $1 AND connection_epoch = $2 AND status = 'connected'
            "#,
        )
        .bind(runner_id)
        .bind(connection_epoch)
        .bind(grace_expires_at)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() == 0 {
            tx.commit().await?;
            return Ok(Vec::new());
        }

        let rows = active_runner_run_rows(&mut tx, runner_id).await?;
        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            let run_id: Uuid = row.get("run_id");
            let corp_id: Uuid = row.get("corp_id");
            let room_id: Uuid = row.get("room_id");
            let mission_id: Uuid = row.get("mission_id");
            if let Some(event) = append_event_tx(
                &mut tx,
                NewEvent {
                    room_id: Some(room_id),
                    correlation_id: Some(mission_id),
                    ..NewEvent::new(
                        corp_id,
                        None,
                        "runner.grace_started",
                        "run",
                        run_id,
                        format!("runner-grace:{runner_id}:{connection_epoch}:{run_id}"),
                        json!({
                            "runner_id": runner_id,
                            "grace_expires_at": grace_expires_at
                        }),
                    )
                },
            )
            .await?
            {
                events.push(event);
            }
        }
        tx.commit().await?;
        Ok(events)
    }

    pub async fn expire_runner_grace(
        &self,
        runner_id: &str,
        connection_epoch: Uuid,
    ) -> Result<Vec<DomainEvent>> {
        let mut tx = self.pool.begin().await?;
        let expired = sqlx::query(
            r#"
            UPDATE runner_nodes
            SET status = 'offline', grace_expires_at = NULL
            WHERE id = $1 AND connection_epoch = $2 AND status = 'grace'
              AND grace_expires_at <= now()
            RETURNING id
            "#,
        )
        .bind(runner_id)
        .bind(connection_epoch)
        .fetch_optional(&mut *tx)
        .await?;
        if expired.is_none() {
            tx.commit().await?;
            return Ok(Vec::new());
        }
        let events =
            mark_runner_runs_lost_tx(&mut tx, runner_id, &[], None, "runner grace period expired")
                .await?;
        tx.commit().await?;
        Ok(events)
    }

    pub async fn reconcile_runner_claims(
        &self,
        runner_id: &str,
        connection_epoch: Uuid,
        claims: &[RunClaim],
    ) -> Result<RunnerReconcileOutcome> {
        let mut tx = self.pool.begin().await?;
        let connected: bool = sqlx::query_scalar(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM runner_nodes
                WHERE id = $1 AND connection_epoch = $2 AND status = 'connected'
            )
            "#,
        )
        .bind(runner_id)
        .bind(connection_epoch)
        .fetch_one(&mut *tx)
        .await?;
        if !connected {
            return Err(anyhow!("runner connection epoch is not active"));
        }

        let mut accepted = Vec::new();
        let mut stale = Vec::new();
        let mut events = Vec::new();
        for claim in claims {
            let row = sqlx::query(
                r#"
                SELECT r.id, r.corp_id, r.assignment_token, t.mission_id, m.room_id
                FROM runs r
                JOIN tasks t ON t.id = r.task_id
                JOIN missions m ON m.id = t.mission_id
                WHERE r.id = $1 AND r.runner_id = $2
                  AND r.status IN ('provisioning', 'starting', 'running',
                                   'waiting_for_input', 'waiting_for_approval', 'verifying')
                FOR UPDATE OF r
                "#,
            )
            .bind(claim.run_id)
            .bind(runner_id)
            .fetch_optional(&mut *tx)
            .await?;
            let Some(row) = row else {
                stale.push(claim.run_id);
                continue;
            };
            let expected: Uuid = row.get("assignment_token");
            if expected != claim.assignment_token {
                stale.push(claim.run_id);
                continue;
            }
            accepted.push(claim.run_id);
            let corp_id: Uuid = row.get("corp_id");
            let mission_id: Uuid = row.get("mission_id");
            let room_id: Uuid = row.get("room_id");
            if let Some(event) = append_event_tx(
                &mut tx,
                NewEvent {
                    room_id: Some(room_id),
                    correlation_id: Some(mission_id),
                    ..NewEvent::new(
                        corp_id,
                        None,
                        "run.reconciled",
                        "run",
                        claim.run_id,
                        format!("run-reconciled:{}:{connection_epoch}", claim.run_id),
                        json!({"runner_id": runner_id}),
                    )
                },
            )
            .await?
            {
                events.push(event);
            }
        }
        tx.commit().await?;
        Ok(RunnerReconcileOutcome {
            accepted,
            stale,
            events,
        })
    }

    pub async fn mark_unclaimed_runner_runs_lost(
        &self,
        runner_id: &str,
        connection_epoch: Uuid,
        accepted_claims: &[Uuid],
    ) -> Result<Vec<DomainEvent>> {
        let mut tx = self.pool.begin().await?;
        let connected_at: Option<chrono::DateTime<Utc>> = sqlx::query_scalar(
            r#"
            SELECT connected_at FROM runner_nodes
            WHERE id = $1 AND connection_epoch = $2 AND status = 'connected'
            "#,
        )
        .bind(runner_id)
        .bind(connection_epoch)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(connected_at) = connected_at else {
            tx.commit().await?;
            return Ok(Vec::new());
        };
        let events = mark_runner_runs_lost_tx(
            &mut tx,
            runner_id,
            accepted_claims,
            Some(connected_at),
            "runner reconnected without an active claim",
        )
        .await?;
        tx.commit().await?;
        Ok(events)
    }

    pub async fn claim_factory_work_item(
        &self,
        input: ClaimFactoryWorkItemInput,
    ) -> Result<FactoryWorkItemOutcome> {
        let source = normalize_factory_source(input.source)?;
        let idempotency_key = normalize_factory_idempotency_key(&input.idempotency_key)?;
        let lease_seconds = validate_factory_lease_seconds(input.lease_seconds)?;
        let policy = normalize_factory_policy(input.policy)?;
        let operation_request = json!({
            "source_project_owner": &source.project_owner,
            "source_project_number": source.project_number,
            "source_project_item_id": &source.project_item_id,
            "source_repository_owner": &source.repository_owner,
            "source_repository_name": &source.repository_name,
            "source_issue_number": source.issue_number,
            "source_issue_node_id": &source.issue_node_id,
            "source_issue_url": &source.issue_url,
            "source_title": &source.title,
            "source_revision": &source.revision,
            "lease_seconds": lease_seconds,
            "policy": &policy
        });
        let now = Utc::now();
        let lease_expires_at = now + Duration::seconds(lease_seconds);

        let mut tx = self.pool.begin().await?;
        assert_actor_scope_tx(&mut tx, input.corp_id, input.actor_id).await?;
        lock_factory_keys_tx(
            &mut tx,
            &[
                format!("factory:idempotency:{}:{idempotency_key}", input.corp_id),
                format!(
                    "factory:source:{}:{}:{}:{}",
                    input.corp_id,
                    source.project_owner,
                    source.project_number,
                    source.project_item_id
                ),
            ],
        )
        .await?;
        if let Some(operation) =
            factory_operation_tx(&mut tx, input.corp_id, &idempotency_key).await?
        {
            ensure_factory_operation_matches(
                &operation,
                "claim",
                input.actor_id,
                None,
                None,
                &operation_request,
            )?;
            let (work_item, current_token) =
                factory_work_item_tx(&mut tx, input.corp_id, operation.work_item_id, false)
                    .await?
                    .context("idempotent factory claim references a missing work item")?;
            ensure_factory_source_matches(&work_item, &source)?;
            if work_item.policy != policy {
                return Err(anyhow!(
                    "factory idempotency key was reused with a different policy snapshot"
                ));
            }
            let claim_token =
                replayable_factory_claim_token(&work_item, current_token, &operation, now);
            tx.commit().await?;
            return Ok(FactoryWorkItemOutcome {
                work_item,
                claim_token,
                event: None,
                replayed: true,
            });
        }

        let existing = sqlx::query(
            r#"
            SELECT id, corp_id, source_kind, source_project_owner, source_project_number,
                   source_project_item_id, source_repository_owner, source_repository_name,
                   source_issue_number, source_issue_node_id, source_issue_url, source_title,
                   source_revision, state, version, claim_owner_id, claim_token,
                   lease_expires_at, policy, mission_id, failure_detail, created_at, updated_at
            FROM factory_work_items
            WHERE corp_id = $1
              AND source_kind = 'github_project_issue'
              AND source_project_owner = $2
              AND source_project_number = $3
              AND source_project_item_id = $4
            FOR UPDATE
            "#,
        )
        .bind(input.corp_id)
        .bind(&source.project_owner)
        .bind(source.project_number)
        .bind(&source.project_item_id)
        .fetch_optional(&mut *tx)
        .await?;

        let claim_token = Uuid::new_v4();
        let (work_item, event_type) = if let Some(row) = existing {
            let current_token: Uuid = row.get("claim_token");
            let current = map_factory_work_item(row)?;
            if factory_state_is_terminal(current.state) {
                return Err(anyhow!(
                    "conflict: factory work item {} is terminal in state {}",
                    current.id,
                    current.state.as_str()
                ));
            }
            if current.lease_expires_at > now {
                if current.claim_owner_id == input.actor_id {
                    ensure_factory_source_matches(&current, &source)?;
                    if current.policy != policy {
                        return Err(anyhow!(
                            "factory recovery policy does not match the persisted policy snapshot"
                        ));
                    }
                    record_factory_operation_tx(
                        &mut tx,
                        NewFactoryOperation {
                            corp_id: input.corp_id,
                            idempotency_key: &idempotency_key,
                            work_item_id: current.id,
                            actor_id: input.actor_id,
                            operation: "claim",
                            resulting_version: current.version,
                            claim_token: Some(current_token),
                            request: &operation_request,
                        },
                    )
                    .await?;
                    tx.commit().await?;
                    return Ok(FactoryWorkItemOutcome {
                        work_item: current,
                        claim_token: Some(current_token),
                        event: None,
                        replayed: true,
                    });
                }
                return Err(anyhow!(
                    "conflict: factory work item {} is claimed until {}",
                    current.id,
                    current.lease_expires_at
                ));
            }
            if current_token == claim_token {
                return Err(anyhow!("factory claim token collision"));
            }
            ensure_factory_source_matches(&current, &source)?;
            if current.policy != policy {
                return Err(anyhow!(
                    "factory recovery policy does not match the persisted policy snapshot"
                ));
            }
            let preserves_materialized_state = current.mission_id.is_some();
            let row = if preserves_materialized_state {
                sqlx::query(
                    r#"
                    UPDATE factory_work_items
                    SET version = version + 1,
                        claim_owner_id = $1,
                        claim_token = $2,
                        lease_expires_at = $3,
                        updated_at = now()
                    WHERE id = $4 AND corp_id = $5
                    RETURNING id, corp_id, source_kind, source_project_owner,
                              source_project_number, source_project_item_id,
                              source_repository_owner, source_repository_name,
                              source_issue_number, source_issue_node_id, source_issue_url,
                              source_title, source_revision, state, version, claim_owner_id,
                              lease_expires_at, policy, mission_id, failure_detail,
                              created_at, updated_at
                    "#,
                )
                .bind(input.actor_id)
                .bind(claim_token)
                .bind(lease_expires_at)
                .bind(current.id)
                .bind(input.corp_id)
                .fetch_one(&mut *tx)
                .await?
            } else {
                sqlx::query(
                    r#"
                    UPDATE factory_work_items
                    SET state = 'claimed',
                        version = version + 1,
                        claim_owner_id = $1,
                        claim_token = $2,
                        lease_expires_at = $3,
                        failure_detail = NULL,
                        updated_at = now()
                    WHERE id = $4 AND corp_id = $5
                    RETURNING id, corp_id, source_kind, source_project_owner,
                              source_project_number, source_project_item_id,
                              source_repository_owner, source_repository_name,
                              source_issue_number, source_issue_node_id, source_issue_url,
                              source_title, source_revision, state, version, claim_owner_id,
                              lease_expires_at, policy, mission_id, failure_detail,
                              created_at, updated_at
                    "#,
                )
                .bind(input.actor_id)
                .bind(claim_token)
                .bind(lease_expires_at)
                .bind(current.id)
                .bind(input.corp_id)
                .fetch_one(&mut *tx)
                .await?
            };
            (map_factory_work_item(row)?, "factory.work_item_reclaimed")
        } else {
            ensure_new_factory_policy_is_pinned(&policy)?;
            let work_item_id = Uuid::new_v4();
            let row = sqlx::query(
                r#"
                INSERT INTO factory_work_items
                    (id, corp_id, source_kind, source_project_owner,
                     source_project_number, source_project_item_id,
                     source_repository_owner, source_repository_name,
                     source_issue_number, source_issue_node_id, source_issue_url,
                     source_title, source_revision, state, version, claim_owner_id,
                     claim_token, lease_expires_at, policy)
                VALUES
                    ($1, $2, 'github_project_issue', $3, $4, $5, $6, $7, $8,
                     $9, $10, $11, $12, 'claimed', 1, $13, $14, $15, $16)
                RETURNING id, corp_id, source_kind, source_project_owner,
                          source_project_number, source_project_item_id,
                          source_repository_owner, source_repository_name,
                          source_issue_number, source_issue_node_id, source_issue_url,
                          source_title, source_revision, state, version, claim_owner_id,
                          lease_expires_at, policy, mission_id, failure_detail,
                          created_at, updated_at
                "#,
            )
            .bind(work_item_id)
            .bind(input.corp_id)
            .bind(&source.project_owner)
            .bind(source.project_number)
            .bind(&source.project_item_id)
            .bind(&source.repository_owner)
            .bind(&source.repository_name)
            .bind(source.issue_number)
            .bind(&source.issue_node_id)
            .bind(&source.issue_url)
            .bind(&source.title)
            .bind(&source.revision)
            .bind(input.actor_id)
            .bind(claim_token)
            .bind(lease_expires_at)
            .bind(&policy)
            .fetch_one(&mut *tx)
            .await?;
            (map_factory_work_item(row)?, "factory.work_item_claimed")
        };

        record_factory_operation_tx(
            &mut tx,
            NewFactoryOperation {
                corp_id: input.corp_id,
                idempotency_key: &idempotency_key,
                work_item_id: work_item.id,
                actor_id: input.actor_id,
                operation: "claim",
                resulting_version: work_item.version,
                claim_token: Some(claim_token),
                request: &operation_request,
            },
        )
        .await?;
        let event_room_id =
            factory_event_room_id_tx(&mut tx, input.corp_id, work_item.mission_id).await?;
        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: event_room_id,
                aggregate_version: work_item.version,
                ..NewEvent::new(
                    input.corp_id,
                    Some(input.actor_id),
                    event_type,
                    "factory_work_item",
                    work_item.id,
                    format!("factory:{}:claim:{}", work_item.id, work_item.version),
                    json!({
                        "claim_owner_id": work_item.claim_owner_id,
                        "lease_expires_at": work_item.lease_expires_at,
                        "state": work_item.state.as_str()
                    }),
                )
            },
        )
        .await?
        .context("factory claim event unexpectedly existed")?;
        tx.commit().await?;
        Ok(FactoryWorkItemOutcome {
            work_item,
            claim_token: Some(claim_token),
            event: Some(event),
            replayed: false,
        })
    }

    pub async fn renew_factory_work_item(
        &self,
        input: RenewFactoryWorkItemInput,
    ) -> Result<FactoryWorkItemOutcome> {
        if input.expected_version <= 0 {
            return Err(anyhow!(
                "expected factory work-item version must be positive"
            ));
        }
        let idempotency_key = normalize_factory_idempotency_key(&input.idempotency_key)?;
        let lease_seconds = validate_factory_lease_seconds(input.lease_seconds)?;
        let operation_request = json!({
            "work_item_id": input.work_item_id,
            "expected_version": input.expected_version,
            "lease_seconds": lease_seconds
        });
        let now = Utc::now();
        let lease_expires_at = now + Duration::seconds(lease_seconds);
        let mut tx = self.pool.begin().await?;
        assert_actor_scope_tx(&mut tx, input.corp_id, input.actor_id).await?;
        lock_factory_keys_tx(
            &mut tx,
            &[
                format!("factory:idempotency:{}:{idempotency_key}", input.corp_id),
                format!("factory:item:{}:{}", input.corp_id, input.work_item_id),
            ],
        )
        .await?;

        if let Some(operation) =
            factory_operation_tx(&mut tx, input.corp_id, &idempotency_key).await?
        {
            ensure_factory_operation_matches(
                &operation,
                "renew",
                input.actor_id,
                Some(input.work_item_id),
                Some(input.claim_token),
                &operation_request,
            )?;
            let (work_item, current_token) =
                factory_work_item_tx(&mut tx, input.corp_id, input.work_item_id, false)
                    .await?
                    .context("idempotent factory renewal references a missing work item")?;
            let claim_token =
                replayable_factory_claim_token(&work_item, current_token, &operation, now);
            tx.commit().await?;
            return Ok(FactoryWorkItemOutcome {
                work_item,
                claim_token,
                event: None,
                replayed: true,
            });
        }

        let (current, current_token) =
            factory_work_item_tx(&mut tx, input.corp_id, input.work_item_id, true)
                .await?
                .context("factory work item not found")?;
        ensure_active_factory_control(
            &current,
            current_token,
            input.actor_id,
            input.claim_token,
            input.expected_version,
            now,
        )?;

        let row = sqlx::query(
            r#"
            UPDATE factory_work_items
            SET version = version + 1, lease_expires_at = $1, updated_at = now()
            WHERE id = $2 AND corp_id = $3
            RETURNING id, corp_id, source_kind, source_project_owner,
                      source_project_number, source_project_item_id,
                      source_repository_owner, source_repository_name,
                      source_issue_number, source_issue_node_id, source_issue_url,
                      source_title, source_revision, state, version, claim_owner_id,
                      lease_expires_at, policy, mission_id, failure_detail,
                      created_at, updated_at
            "#,
        )
        .bind(lease_expires_at)
        .bind(input.work_item_id)
        .bind(input.corp_id)
        .fetch_one(&mut *tx)
        .await?;
        let work_item = map_factory_work_item(row)?;
        record_factory_operation_tx(
            &mut tx,
            NewFactoryOperation {
                corp_id: input.corp_id,
                idempotency_key: &idempotency_key,
                work_item_id: work_item.id,
                actor_id: input.actor_id,
                operation: "renew",
                resulting_version: work_item.version,
                claim_token: Some(input.claim_token),
                request: &operation_request,
            },
        )
        .await?;
        let event_room_id =
            factory_event_room_id_tx(&mut tx, input.corp_id, work_item.mission_id).await?;
        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: event_room_id,
                aggregate_version: work_item.version,
                ..NewEvent::new(
                    input.corp_id,
                    Some(input.actor_id),
                    "factory.claim_renewed",
                    "factory_work_item",
                    work_item.id,
                    format!("factory:{}:renew:{}", work_item.id, work_item.version),
                    json!({
                        "claim_owner_id": work_item.claim_owner_id,
                        "lease_expires_at": work_item.lease_expires_at,
                        "state": work_item.state.as_str()
                    }),
                )
            },
        )
        .await?
        .context("factory renewal event unexpectedly existed")?;
        tx.commit().await?;
        Ok(FactoryWorkItemOutcome {
            work_item,
            claim_token: Some(input.claim_token),
            event: Some(event),
            replayed: false,
        })
    }

    pub async fn upgrade_factory_source_commit(
        &self,
        input: UpgradeFactorySourceCommitInput,
    ) -> Result<FactoryWorkItemOutcome> {
        if input.expected_version <= 0 {
            return Err(anyhow!(
                "expected factory work-item version must be positive"
            ));
        }
        let source_base_commit = input.source_base_commit.trim().to_ascii_lowercase();
        validate_factory_base_commit(&source_base_commit)?;
        let idempotency_key = normalize_factory_idempotency_key(&input.idempotency_key)?;
        let operation_request = json!({
            "work_item_id": input.work_item_id,
            "expected_version": input.expected_version,
            "source_base_commit": source_base_commit
        });
        let now = Utc::now();
        let mut tx = self.pool.begin().await?;
        assert_actor_scope_tx(&mut tx, input.corp_id, input.actor_id).await?;
        lock_factory_keys_tx(
            &mut tx,
            &[
                format!("factory:idempotency:{}:{idempotency_key}", input.corp_id),
                format!("factory:item:{}:{}", input.corp_id, input.work_item_id),
            ],
        )
        .await?;

        if let Some(operation) =
            factory_operation_tx(&mut tx, input.corp_id, &idempotency_key).await?
        {
            ensure_factory_operation_matches(
                &operation,
                "upgrade_source_commit",
                input.actor_id,
                Some(input.work_item_id),
                Some(input.claim_token),
                &operation_request,
            )?;
            let (work_item, current_token) =
                factory_work_item_tx(&mut tx, input.corp_id, input.work_item_id, false)
                    .await?
                    .context("idempotent source upgrade references a missing work item")?;
            let claim_token =
                replayable_factory_claim_token(&work_item, current_token, &operation, now);
            tx.commit().await?;
            return Ok(FactoryWorkItemOutcome {
                work_item,
                claim_token,
                event: None,
                replayed: true,
            });
        }

        let (current, current_token) =
            factory_work_item_tx(&mut tx, input.corp_id, input.work_item_id, true)
                .await?
                .context("factory work item not found")?;
        ensure_active_factory_control(
            &current,
            current_token,
            input.actor_id,
            input.claim_token,
            input.expected_version,
            now,
        )?;
        let mut policy = current
            .policy
            .as_object()
            .cloned()
            .context("factory policy snapshot must be a JSON object")?;
        if policy
            .get("source_base_commit")
            .and_then(Value::as_str)
            .is_some()
        {
            return Err(anyhow!(
                "factory source policy already has an immutable base commit"
            ));
        }
        if policy
            .get("source_commit_upgrade_required")
            .and_then(Value::as_bool)
            != Some(true)
        {
            return Err(anyhow!(
                "factory source policy is not marked for an authorized legacy upgrade"
            ));
        }
        let source_base_ref = factory_policy_required_string(&policy, "source_base_ref", 240)?;
        validate_factory_base_ref(&source_base_ref)?;
        let source_repository = format!(
            "{}/{}",
            current.source_repository_owner, current.source_repository_name
        );

        if let Some(mission_id) = current.mission_id {
            let has_run: bool = sqlx::query_scalar(
                r#"
                SELECT EXISTS (
                    SELECT 1
                    FROM runs r
                    JOIN tasks t ON t.id = r.task_id
                    WHERE t.corp_id = $1 AND t.mission_id = $2
                )
                "#,
            )
            .bind(input.corp_id)
            .bind(mission_id)
            .fetch_one(&mut *tx)
            .await?;
            if has_run {
                return Err(anyhow!(
                    "legacy source commit cannot be freshly pinned after a run exists"
                ));
            }
            let incompatible_task: bool = sqlx::query_scalar(
                r#"
                SELECT EXISTS (
                    SELECT 1
                    FROM tasks
                    WHERE corp_id = $1 AND mission_id = $2
                      AND (
                        contract->>'source_repository' IS DISTINCT FROM $3
                        OR contract->>'source_base_ref' IS DISTINCT FROM $4
                        OR (
                            contract ? 'source_base_commit'
                            AND contract->>'source_base_commit' IS NOT NULL
                        )
                      )
                )
                "#,
            )
            .bind(input.corp_id)
            .bind(mission_id)
            .bind(&source_repository)
            .bind(&source_base_ref)
            .fetch_one(&mut *tx)
            .await?;
            if incompatible_task {
                return Err(anyhow!(
                    "legacy mission tasks do not preserve the claimed repository and base ref"
                ));
            }
            sqlx::query(
                r#"
                UPDATE tasks
                SET contract = jsonb_set(
                        contract,
                        '{source_base_commit}',
                        to_jsonb($1::text),
                        true
                    ),
                    updated_at = now()
                WHERE corp_id = $2 AND mission_id = $3
                "#,
            )
            .bind(&source_base_commit)
            .bind(input.corp_id)
            .bind(mission_id)
            .execute(&mut *tx)
            .await?;
        }

        policy.insert(
            "source_base_commit".to_owned(),
            Value::String(source_base_commit.clone()),
        );
        policy.insert(
            "source_commit_upgrade_required".to_owned(),
            Value::Bool(false),
        );
        let row = sqlx::query(
            r#"
            UPDATE factory_work_items
            SET policy = $1,
                version = version + 1,
                failure_detail = NULL,
                updated_at = now()
            WHERE id = $2 AND corp_id = $3
            RETURNING id, corp_id, source_kind, source_project_owner,
                      source_project_number, source_project_item_id,
                      source_repository_owner, source_repository_name,
                      source_issue_number, source_issue_node_id, source_issue_url,
                      source_title, source_revision, state, version, claim_owner_id,
                      lease_expires_at, policy, mission_id, failure_detail,
                      created_at, updated_at
            "#,
        )
        .bind(Value::Object(policy))
        .bind(input.work_item_id)
        .bind(input.corp_id)
        .fetch_one(&mut *tx)
        .await?;
        let work_item = map_factory_work_item(row)?;
        record_factory_operation_tx(
            &mut tx,
            NewFactoryOperation {
                corp_id: input.corp_id,
                idempotency_key: &idempotency_key,
                work_item_id: work_item.id,
                actor_id: input.actor_id,
                operation: "upgrade_source_commit",
                resulting_version: work_item.version,
                claim_token: Some(input.claim_token),
                request: &operation_request,
            },
        )
        .await?;
        let event_room_id =
            factory_event_room_id_tx(&mut tx, input.corp_id, work_item.mission_id).await?;
        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: event_room_id,
                aggregate_version: work_item.version,
                correlation_id: work_item.mission_id,
                ..NewEvent::new(
                    input.corp_id,
                    Some(input.actor_id),
                    "factory.source_commit_pinned",
                    "factory_work_item",
                    work_item.id,
                    format!(
                        "factory:{}:source-commit:{}",
                        work_item.id, work_item.version
                    ),
                    json!({
                        "state": work_item.state.as_str(),
                        "legacy_upgrade": true
                    }),
                )
            },
        )
        .await?
        .context("factory source upgrade event unexpectedly existed")?;
        tx.commit().await?;
        Ok(FactoryWorkItemOutcome {
            work_item,
            claim_token: Some(input.claim_token),
            event: Some(event),
            replayed: false,
        })
    }

    pub async fn transition_factory_work_item(
        &self,
        input: TransitionFactoryWorkItemInput,
    ) -> Result<FactoryWorkItemOutcome> {
        if input.expected_version <= 0 {
            return Err(anyhow!(
                "expected factory work-item version must be positive"
            ));
        }
        if matches!(
            input.state,
            FactoryWorkItemState::Claimed
                | FactoryWorkItemState::MissionCreated
                | FactoryWorkItemState::Publishing
                | FactoryWorkItemState::Published
        ) {
            return Err(anyhow!(
                "claim, materialization, and publication states require their dedicated operations"
            ));
        }
        let failure_detail = input
            .failure_detail
            .as_deref()
            .map(|detail| normalize_factory_text(detail, "factory failure detail", 2_000))
            .transpose()?;
        if matches!(
            input.state,
            FactoryWorkItemState::Blocked
                | FactoryWorkItemState::VerificationFailed
                | FactoryWorkItemState::Failed
        ) && failure_detail.is_none()
        {
            return Err(anyhow!(
                "blocked and failed factory states require a failure detail"
            ));
        }
        let idempotency_key = normalize_factory_idempotency_key(&input.idempotency_key)?;
        let operation_request = json!({
            "work_item_id": input.work_item_id,
            "expected_version": input.expected_version,
            "state": input.state.as_str(),
            "failure_detail": &failure_detail
        });
        let now = Utc::now();
        let mut tx = self.pool.begin().await?;
        assert_actor_scope_tx(&mut tx, input.corp_id, input.actor_id).await?;
        lock_factory_keys_tx(
            &mut tx,
            &[
                format!("factory:idempotency:{}:{idempotency_key}", input.corp_id),
                format!("factory:item:{}:{}", input.corp_id, input.work_item_id),
            ],
        )
        .await?;

        if let Some(operation) =
            factory_operation_tx(&mut tx, input.corp_id, &idempotency_key).await?
        {
            ensure_factory_operation_matches(
                &operation,
                "transition",
                input.actor_id,
                Some(input.work_item_id),
                Some(input.claim_token),
                &operation_request,
            )?;
            let (work_item, current_token) =
                factory_work_item_tx(&mut tx, input.corp_id, input.work_item_id, false)
                    .await?
                    .context("idempotent factory transition references a missing work item")?;
            let claim_token =
                replayable_factory_claim_token(&work_item, current_token, &operation, now);
            tx.commit().await?;
            return Ok(FactoryWorkItemOutcome {
                work_item,
                claim_token,
                event: None,
                replayed: true,
            });
        }

        let (current, current_token) =
            factory_work_item_tx(&mut tx, input.corp_id, input.work_item_id, true)
                .await?
                .context("factory work item not found")?;
        ensure_active_factory_control(
            &current,
            current_token,
            input.actor_id,
            input.claim_token,
            input.expected_version,
            now,
        )?;
        if !factory_transition_allowed(current.state, input.state) {
            return Err(anyhow!(
                "conflict: factory transition {} -> {} is not allowed",
                current.state.as_str(),
                input.state.as_str()
            ));
        }
        if current.mission_id.is_none()
            && !matches!(
                input.state,
                FactoryWorkItemState::Blocked
                    | FactoryWorkItemState::Failed
                    | FactoryWorkItemState::Cancelled
            )
        {
            return Err(anyhow!(
                "conflict: factory work item must link a mission before entering {}",
                input.state.as_str()
            ));
        }
        if input.state == FactoryWorkItemState::Verified {
            let mission_id = current
                .mission_id
                .context("factory work item has no linked mission")?;
            ensure_factory_mission_verified_tx(&mut tx, input.corp_id, mission_id).await?;
        }
        let row = sqlx::query(
            r#"
            UPDATE factory_work_items
            SET state = $1,
                version = version + 1,
                failure_detail = $2,
                updated_at = now()
            WHERE id = $3 AND corp_id = $4
            RETURNING id, corp_id, source_kind, source_project_owner,
                      source_project_number, source_project_item_id,
                      source_repository_owner, source_repository_name,
                      source_issue_number, source_issue_node_id, source_issue_url,
                      source_title, source_revision, state, version, claim_owner_id,
                      lease_expires_at, policy, mission_id, failure_detail,
                      created_at, updated_at
            "#,
        )
        .bind(input.state.as_str())
        .bind(&failure_detail)
        .bind(input.work_item_id)
        .bind(input.corp_id)
        .fetch_one(&mut *tx)
        .await?;
        let work_item = map_factory_work_item(row)?;
        record_factory_operation_tx(
            &mut tx,
            NewFactoryOperation {
                corp_id: input.corp_id,
                idempotency_key: &idempotency_key,
                work_item_id: work_item.id,
                actor_id: input.actor_id,
                operation: "transition",
                resulting_version: work_item.version,
                claim_token: Some(input.claim_token),
                request: &operation_request,
            },
        )
        .await?;
        let event_room_id =
            factory_event_room_id_tx(&mut tx, input.corp_id, work_item.mission_id).await?;
        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: event_room_id,
                aggregate_version: work_item.version,
                correlation_id: work_item.mission_id,
                ..NewEvent::new(
                    input.corp_id,
                    Some(input.actor_id),
                    "factory.state_changed",
                    "factory_work_item",
                    work_item.id,
                    format!(
                        "factory:{}:state:{}:{}",
                        work_item.id,
                        work_item.state.as_str(),
                        work_item.version
                    ),
                    json!({
                        "previous_state": current.state.as_str(),
                        "state": work_item.state.as_str(),
                        "mission_id": work_item.mission_id,
                        "failure_detail": &work_item.failure_detail
                    }),
                )
            },
        )
        .await?
        .context("factory state-change event unexpectedly existed")?;
        tx.commit().await?;
        Ok(FactoryWorkItemOutcome {
            work_item,
            claim_token: Some(input.claim_token),
            event: Some(event),
            replayed: false,
        })
    }

    pub async fn replay_factory_materialization(
        &self,
        input: &MaterializeFactoryMissionInput,
    ) -> Result<Option<FactoryMissionOutcome>> {
        if input.expected_version <= 0 {
            return Err(anyhow!(
                "expected factory work-item version must be positive"
            ));
        }
        let idempotency_key = normalize_factory_idempotency_key(&input.idempotency_key)?;
        let operation_request = normalize_factory_operation_request(input.request.clone())?;
        let mut tx = self.pool.begin().await?;
        assert_actor_scope_tx(&mut tx, input.corp_id, input.actor_id).await?;
        lock_factory_keys_tx(
            &mut tx,
            &[
                format!("factory:idempotency:{}:{idempotency_key}", input.corp_id),
                format!("factory:item:{}:{}", input.corp_id, input.work_item_id),
            ],
        )
        .await?;
        let Some(operation) =
            factory_operation_tx(&mut tx, input.corp_id, &idempotency_key).await?
        else {
            tx.commit().await?;
            return Ok(None);
        };
        ensure_factory_operation_matches(
            &operation,
            "materialize",
            input.actor_id,
            Some(input.work_item_id),
            Some(input.claim_token),
            &operation_request,
        )?;
        let (work_item, _) =
            factory_work_item_tx(&mut tx, input.corp_id, input.work_item_id, false)
                .await?
                .context("idempotent factory materialization references a missing work item")?;
        let mission_id = work_item
            .mission_id
            .context("idempotent factory materialization has no mission linkage")?;
        let (ids, strategy) =
            factory_mission_details_tx(&mut tx, input.corp_id, mission_id).await?;
        tx.commit().await?;
        Ok(Some(FactoryMissionOutcome {
            work_item,
            ids,
            strategy,
            events: Vec::new(),
            replayed: true,
        }))
    }

    pub async fn materialize_factory_mission(
        &self,
        input: MaterializeFactoryMissionInput,
        plan: &TaskGraphPlan,
    ) -> Result<FactoryMissionOutcome> {
        if input.expected_version <= 0 {
            return Err(anyhow!(
                "expected factory work-item version must be positive"
            ));
        }
        let title = normalize_mission_title(&input.title)?;
        let idempotency_key = normalize_factory_idempotency_key(&input.idempotency_key)?;
        let operation_request = normalize_factory_operation_request(input.request)?;
        let now = Utc::now();
        let mut tx = self.pool.begin().await?;
        assert_actor_scope_tx(&mut tx, input.corp_id, input.actor_id).await?;
        lock_factory_keys_tx(
            &mut tx,
            &[
                format!("factory:idempotency:{}:{idempotency_key}", input.corp_id),
                format!("factory:item:{}:{}", input.corp_id, input.work_item_id),
            ],
        )
        .await?;

        if let Some(operation) =
            factory_operation_tx(&mut tx, input.corp_id, &idempotency_key).await?
        {
            ensure_factory_operation_matches(
                &operation,
                "materialize",
                input.actor_id,
                Some(input.work_item_id),
                Some(input.claim_token),
                &operation_request,
            )?;
            let (work_item, _) =
                factory_work_item_tx(&mut tx, input.corp_id, input.work_item_id, false)
                    .await?
                    .context("idempotent factory materialization references a missing work item")?;
            let mission_id = work_item
                .mission_id
                .context("idempotent factory materialization has no mission linkage")?;
            let (ids, strategy) =
                factory_mission_details_tx(&mut tx, input.corp_id, mission_id).await?;
            tx.commit().await?;
            return Ok(FactoryMissionOutcome {
                work_item,
                ids,
                strategy,
                events: Vec::new(),
                replayed: true,
            });
        }

        let (current, current_token) =
            factory_work_item_tx(&mut tx, input.corp_id, input.work_item_id, true)
                .await?
                .context("factory work item not found")?;
        ensure_materializable_factory_claim(
            &current,
            current_token,
            input.actor_id,
            input.claim_token,
            input.expected_version,
            now,
        )?;
        let mut constrained_plan = plan.clone();
        apply_factory_source_constraints(&current, &mut constrained_plan)?;
        validate_factory_plan_against_policy(&current, &constrained_plan)?;

        let (ids, mut events) = create_mission_tx(
            &mut tx,
            input.corp_id,
            input.actor_id,
            &title,
            &input.description,
            &constrained_plan,
        )
        .await?;
        let row = sqlx::query(
            r#"
            UPDATE factory_work_items
            SET state = 'mission_created',
                version = version + 1,
                mission_id = $1,
                failure_detail = NULL,
                updated_at = now()
            WHERE id = $2 AND corp_id = $3
            RETURNING id, corp_id, source_kind, source_project_owner,
                      source_project_number, source_project_item_id,
                      source_repository_owner, source_repository_name,
                      source_issue_number, source_issue_node_id, source_issue_url,
                      source_title, source_revision, state, version, claim_owner_id,
                      lease_expires_at, policy, mission_id, failure_detail,
                      created_at, updated_at
            "#,
        )
        .bind(ids.mission_id)
        .bind(input.work_item_id)
        .bind(input.corp_id)
        .fetch_one(&mut *tx)
        .await?;
        let work_item = map_factory_work_item(row)?;
        record_factory_operation_tx(
            &mut tx,
            NewFactoryOperation {
                corp_id: input.corp_id,
                idempotency_key: &idempotency_key,
                work_item_id: work_item.id,
                actor_id: input.actor_id,
                operation: "materialize",
                resulting_version: work_item.version,
                claim_token: Some(input.claim_token),
                request: &operation_request,
            },
        )
        .await?;
        let factory_event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: events.first().and_then(|event| event.room_id),
                aggregate_version: work_item.version,
                correlation_id: Some(ids.mission_id),
                causation_id: events.last().map(|event| event.id),
                ..NewEvent::new(
                    input.corp_id,
                    Some(input.actor_id),
                    "factory.mission_linked",
                    "factory_work_item",
                    work_item.id,
                    format!("factory:{}:mission:{}", work_item.id, ids.mission_id),
                    json!({
                        "mission_id": ids.mission_id,
                        "task_ids": &ids.task_ids,
                        "state": work_item.state.as_str()
                    }),
                )
            },
        )
        .await?
        .context("factory mission-link event unexpectedly existed")?;
        events.push(factory_event);
        tx.commit().await?;
        Ok(FactoryMissionOutcome {
            work_item,
            ids,
            strategy: constrained_plan.strategy,
            events,
            replayed: false,
        })
    }

    pub async fn create_mission(
        &self,
        corp_id: Uuid,
        requested_by: Uuid,
        title: &str,
        description: &str,
        plan: &TaskGraphPlan,
    ) -> Result<(MissionPlanIds, Vec<DomainEvent>)> {
        let mut tx = self.pool.begin().await?;
        let outcome =
            create_mission_tx(&mut tx, corp_id, requested_by, title, description, plan).await?;
        tx.commit().await?;
        Ok(outcome)
    }

    pub async fn schedulable_mission_ids(&self, corp_id: Uuid) -> Result<Vec<Uuid>> {
        sqlx::query_scalar(
            r#"
            SELECT DISTINCT t.mission_id
            FROM tasks t
            JOIN missions m ON m.id = t.mission_id
            JOIN agents a ON a.id = t.assigned_agent_id
            WHERE t.corp_id = $1
              AND m.status IN ('ready', 'running')
              AND t.status IN ('pending', 'ready')
              AND a.status = 'idle'
              AND t.attempt_count < t.max_attempts
              AND NOT EXISTS (
                SELECT 1 FROM task_dependencies dependency
                JOIN tasks parent ON parent.id = dependency.depends_on_task_id
                WHERE dependency.task_id = t.id
                  AND parent.status <> 'completed'
              )
              AND NOT EXISTS (
                SELECT 1 FROM runs task_run
                WHERE task_run.task_id = t.id
                  AND task_run.status IN ('provisioning', 'starting', 'running',
                                          'waiting_for_input', 'waiting_for_approval', 'verifying')
              )
              AND NOT EXISTS (
                SELECT 1 FROM runs agent_run
                WHERE agent_run.agent_id = t.assigned_agent_id
                  AND agent_run.status IN ('provisioning', 'starting', 'running',
                                           'waiting_for_input', 'waiting_for_approval', 'verifying')
              )
            ORDER BY t.mission_id
            "#,
        )
        .bind(corp_id)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn schedulable_tasks(
        &self,
        corp_id: Uuid,
        mission_id: Uuid,
    ) -> Result<Vec<SchedulableTask>> {
        sqlx::query(
            r#"
            SELECT t.id AS task_id,
                   COALESCE(t.required_adapter, a.adapter) AS required_adapter,
                   t.contract->>'model' AS required_model,
                   t.contract->>'reasoning_effort' AS required_reasoning_effort,
                   t.contract->>'source_repository' AS required_source_repository,
                   t.contract->>'source_base_ref' AS required_source_base_ref,
                   t.contract->>'source_base_commit' AS required_source_base_commit
            FROM tasks t
            JOIN missions m ON m.id = t.mission_id
            JOIN agents a ON a.id = t.assigned_agent_id
            WHERE t.corp_id = $1 AND t.mission_id = $2
              AND m.status IN ('ready', 'running')
              AND t.status IN ('pending', 'ready')
              AND a.status = 'idle'
              AND t.attempt_count < t.max_attempts
              AND NOT EXISTS (
                SELECT 1 FROM task_dependencies dependency
                JOIN tasks parent ON parent.id = dependency.depends_on_task_id
                WHERE dependency.task_id = t.id
                  AND parent.status <> 'completed'
              )
              AND NOT EXISTS (
                SELECT 1 FROM runs task_run
                WHERE task_run.task_id = t.id
                  AND task_run.status IN ('provisioning', 'starting', 'running',
                                          'waiting_for_input', 'waiting_for_approval', 'verifying')
              )
              AND NOT EXISTS (
                SELECT 1 FROM runs agent_run
                WHERE agent_run.agent_id = t.assigned_agent_id
                  AND agent_run.status IN ('provisioning', 'starting', 'running',
                                           'waiting_for_input', 'waiting_for_approval', 'verifying')
              )
            ORDER BY t.depth, t.created_at, t.id
            "#,
        )
        .bind(corp_id)
        .bind(mission_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|row| {
            Ok(SchedulableTask {
                task_id: row.get("task_id"),
                required_adapter: row.get("required_adapter"),
                required_model: row.get("required_model"),
                required_reasoning_effort: row.get("required_reasoning_effort"),
                required_source_repository: row.get("required_source_repository"),
                required_source_base_ref: row.get("required_source_base_ref"),
                required_source_base_commit: row.get("required_source_base_commit"),
            })
        })
        .collect()
    }

    pub async fn create_task_run(
        &self,
        corp_id: Uuid,
        mission_id: Uuid,
        task_id: Uuid,
        requested_by: Option<Uuid>,
        runner_id: &str,
    ) -> Result<(LaunchRecord, DomainEvent)> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            r#"
            SELECT m.room_id, m.title AS mission_title, m.status AS mission_status,
                   t.title AS task_title, t.contract, t.verification_policy,
                   t.status AS task_status,
                   t.assigned_agent_id, t.attempt_count, t.max_attempts,
                   COALESCE(t.required_adapter, a.adapter) AS required_adapter,
                   a.adapter
            FROM missions m
            JOIN tasks t ON t.mission_id = m.id
            JOIN agents a ON a.id = t.assigned_agent_id
            WHERE m.id = $1 AND m.corp_id = $2 AND t.id = $3
            FOR UPDATE OF m, t, a
            "#,
        )
        .bind(mission_id)
        .bind(corp_id)
        .bind(task_id)
        .fetch_one(&mut *tx)
        .await
        .context("mission or task not found")?;

        let room_id: Uuid = row.get("room_id");
        if let Some(actor_id) = requested_by {
            assert_room_membership_tx(&mut tx, corp_id, room_id, actor_id).await?;
        }
        let mission_status: String = row.get("mission_status");
        if matches!(
            mission_status.as_str(),
            "completed" | "failed" | "cancelled"
        ) {
            return Err(anyhow!("mission is already {mission_status}"));
        }
        let task_status: String = row.get("task_status");
        if !matches!(task_status.as_str(), "pending" | "ready") {
            return Err(anyhow!("task is not schedulable from status {task_status}"));
        }
        let agent_id: Uuid = row
            .try_get::<Option<Uuid>, _>("assigned_agent_id")?
            .context("task has no assigned agent")?;
        let adapter: String = row.get("adapter");
        let required_adapter: String = row.get("required_adapter");
        if adapter != required_adapter {
            return Err(anyhow!(
                "assigned agent adapter {adapter} does not satisfy {required_adapter}"
            ));
        }
        let dependencies_ready: bool = sqlx::query_scalar(
            r#"
            SELECT NOT EXISTS (
                SELECT 1 FROM task_dependencies dependency
                JOIN tasks parent ON parent.id = dependency.depends_on_task_id
                WHERE dependency.task_id = $1
                  AND parent.status <> 'completed'
            )
            "#,
        )
        .bind(task_id)
        .fetch_one(&mut *tx)
        .await?;
        if !dependencies_ready {
            return Err(anyhow!("task dependencies are not complete"));
        }

        let active: Option<Uuid> = sqlx::query_scalar(
            r#"
            SELECT id FROM runs
            WHERE task_id = $1 AND status IN ('provisioning', 'starting', 'running',
                                             'waiting_for_input', 'waiting_for_approval', 'verifying')
            LIMIT 1
            "#,
        )
        .bind(task_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(active_run) = active {
            return Err(anyhow!("task already has active run {active_run}"));
        }
        let agent_active: Option<Uuid> = sqlx::query_scalar(
            r#"
            SELECT id FROM runs
            WHERE agent_id = $1 AND status IN ('provisioning', 'starting', 'running',
                                              'waiting_for_input', 'waiting_for_approval', 'verifying')
            LIMIT 1
            "#,
        )
        .bind(agent_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(active_run) = agent_active {
            return Err(anyhow!(
                "assigned agent already has active run {active_run}"
            ));
        }
        let attempt_count: i32 = row.get("attempt_count");
        let max_attempts: i32 = row.get("max_attempts");
        if attempt_count >= max_attempts {
            return Err(anyhow!("task exhausted its retry limit"));
        }
        let attempt = attempt_count + 1;
        let contract: TaskContract =
            serde_json::from_value(row.get("contract")).context("decode task contract")?;
        let verification_policy: VerificationPolicy =
            serde_json::from_value(row.get("verification_policy"))
                .context("decode verification policy")?;
        let mission_title: String = row.get("mission_title");
        let task_title: String = row.get("task_title");
        let mut task_prompt = format_task_prompt(&mission_title, &task_title, &contract, attempt);
        let model = contract.model.clone();
        let reasoning_effort = contract.reasoning_effort.clone();

        let run_id = Uuid::new_v4();
        let assignment_token = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO runs
                (id, corp_id, task_id, agent_id, runner_id, assignment_token, status,
                 workspace_run_id, budget_tokens_limit, budget_cost_microusd_limit,
                 model, reasoning_effort, source_repository, source_base_ref,
                 source_base_commit)
            VALUES ($1, $2, $3, $4, $5, $6, 'starting', $1, $7, $8, $9, $10,
                    $11, $12, $13)
            "#,
        )
        .bind(run_id)
        .bind(corp_id)
        .bind(task_id)
        .bind(agent_id)
        .bind(runner_id)
        .bind(assignment_token)
        .bind(contract.budget_tokens)
        .bind(contract.budget_cost_microusd)
        .bind(&model)
        .bind(&reasoning_effort)
        .bind(&contract.source_repository)
        .bind(&contract.source_base_ref)
        .bind(&contract.source_base_commit)
        .execute(&mut *tx)
        .await?;
        let queued_messages =
            reserve_queued_messages_tx(&mut tx, corp_id, agent_id, run_id).await?;
        append_queued_messages(&mut task_prompt, &queued_messages);
        sqlx::query(
            "UPDATE missions SET status = 'running', updated_at = now() WHERE id = $1 AND status IN ('ready', 'running')",
        )
            .bind(mission_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE tasks SET status = 'claimed', attempt_count = $1, verification_status = 'pending', updated_at = now() WHERE id = $2",
        )
            .bind(attempt)
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE agents SET status = 'starting', station = 'dispatch', current_run_id = $1 WHERE id = $2",
        )
        .bind(run_id)
        .bind(agent_id)
        .execute(&mut *tx)
        .await?;

        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(room_id),
                correlation_id: Some(mission_id),
                ..NewEvent::new(
                    corp_id,
                    requested_by,
                    "run.requested",
                    "run",
                    run_id,
                    format!("run:{run_id}:requested"),
                    json!({
                        "task_id": task_id,
                        "agent_id": agent_id,
                        "runner_id": runner_id,
                        "attempt": attempt,
                        "max_attempts": max_attempts,
                        "model": model,
                        "reasoning_effort": reasoning_effort,
                        "status": "starting"
                    }),
                )
            },
        )
        .await?
        .context("run requested event unexpectedly existed")?;
        tx.commit().await?;

        Ok((
            LaunchRecord {
                corp_id,
                room_id,
                mission_id,
                task_id,
                run_id,
                agent_id,
                assignment_token,
                attempt,
                adapter,
                mission_title: task_prompt,
                model,
                reasoning_effort,
                source_repository: contract.source_repository.clone(),
                source_base_ref: contract.source_base_ref.clone(),
                source_base_commit: contract.source_base_commit.clone(),
                verification_policy,
                write_scope: contract.write_scope,
                deliverable: contract.deliverable,
                secret_refs: contract.secret_refs,
                queued_messages,
            },
            event,
        ))
    }

    pub async fn create_resume_run(
        &self,
        corp_id: Uuid,
        source_run_id: Uuid,
        requested_by: Uuid,
    ) -> Result<(ResumeLaunchRecord, DomainEvent)> {
        let mut tx = self.pool.begin().await?;
        let resume_context = sqlx::query(
            r#"
            SELECT run.workspace_run_id, mission.requested_by
            FROM runs run
            JOIN tasks task ON task.id = run.task_id
            JOIN missions mission ON mission.id = task.mission_id
            WHERE run.id = $1 AND run.corp_id = $2
            "#,
        )
        .bind(source_run_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?
        .context("source run not found")?;
        let expected_workspace_run_id: Uuid = resume_context.get("workspace_run_id");
        let expected_requester: Uuid = resume_context.get("requested_by");
        let mut resume_lock_keys = budget_scope_lock_keys(corp_id, expected_requester);
        resume_lock_keys.push(format!(
            "resume:workspace:{corp_id}:{expected_workspace_run_id}"
        ));
        lock_factory_keys_tx(&mut tx, &resume_lock_keys).await?;

        let row = sqlx::query(
            r#"
            SELECT r.task_id, r.agent_id, r.runner_id, r.provider_session_id,
                   r.workspace_run_id, r.workspace_disposition, r.workspace_base_commit,
                   r.breaker_stage AS source_breaker_stage,
                   t.mission_id, t.title AS task_title, t.attempt_count,
                   t.contract, t.verification_policy, m.room_id,
                   m.requested_by, m.title AS mission_title,
                   m.budget_tokens AS mission_budget_tokens,
                   m.budget_cost_microusd AS mission_budget_cost_microusd,
                   a.adapter
            FROM runs r
            JOIN tasks t ON t.id = r.task_id
            JOIN missions m ON m.id = t.mission_id
            JOIN agents a ON a.id = r.agent_id
            WHERE r.id = $1 AND r.corp_id = $2
            FOR UPDATE OF r, t, m, a
            "#,
        )
        .bind(source_run_id)
        .bind(corp_id)
        .fetch_one(&mut *tx)
        .await
        .context("source run not found")?;
        let task_id: Uuid = row.get("task_id");
        let agent_id: Uuid = row.get("agent_id");
        let runner_id: String = row.get("runner_id");
        let provider_session_id: String = row
            .try_get::<Option<String>, _>("provider_session_id")?
            .context("source run has no resumable provider session")?;
        let workspace_run_id: Uuid = row.get("workspace_run_id");
        if workspace_run_id != expected_workspace_run_id {
            return Err(anyhow!(
                "source run workspace lineage changed during resume authorization"
            ));
        }
        let workspace_base_commit: String = row
            .try_get::<Option<String>, _>("workspace_base_commit")?
            .context("source run has no persisted workspace base commit")?;
        let workspace_disposition: Option<String> = row.get("workspace_disposition");
        if workspace_disposition.as_deref() != Some("preserved") {
            return Err(anyhow!(
                "source run workspace is not preserved and cannot be resumed safely"
            ));
        }
        let verification_policy: VerificationPolicy =
            serde_json::from_value(row.get("verification_policy"))
                .context("decode verification policy")?;
        let contract: TaskContract =
            serde_json::from_value(row.get("contract")).context("decode task contract")?;
        let task_prompt = format_task_prompt(
            &row.get::<String, _>("mission_title"),
            &row.get::<String, _>("task_title"),
            &contract,
            row.get::<i32, _>("attempt_count").max(1),
        );
        let source_breaker_stage: String = row.get("source_breaker_stage");
        if source_breaker_stage == "stop" {
            return Err(anyhow!(
                "source run reached a stop-stage breaker and cannot be resumed"
            ));
        }
        let mission_id: Uuid = row.get("mission_id");
        let room_id: Uuid = row.get("room_id");
        let requester: Uuid = row.get("requested_by");
        if requester != expected_requester {
            return Err(anyhow!(
                "source run requester changed during resume authorization"
            ));
        }
        let adapter: String = row.get("adapter");
        let model = contract.model.clone();
        let reasoning_effort = contract.reasoning_effort.clone();
        assert_room_membership_tx(&mut tx, corp_id, room_id, requested_by).await?;

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
        .fetch_all(&mut *tx)
        .await?;
        if lineage
            .iter()
            .any(|candidate| candidate.get::<String, _>("breaker_stage") == "stop")
        {
            return Err(anyhow!(
                "provider workspace lineage reached a stop-stage breaker and cannot be resumed"
            ));
        }
        let latest_lineage_run_id = lineage
            .iter()
            .find(|candidate| {
                !lineage_run_is_pre_dispatch_failure(
                    candidate.get::<String, _>("status").as_str(),
                    candidate
                        .get::<Option<String>, _>("workspace_path")
                        .as_deref(),
                    candidate
                        .get::<Option<String>, _>("workspace_disposition")
                        .as_deref(),
                    candidate
                        .get::<Option<String>, _>("workspace_detail")
                        .as_deref(),
                )
            })
            .map(|candidate| candidate.get::<Uuid, _>("id"))
            .context("source run workspace lineage is empty")?;
        if latest_lineage_run_id != source_run_id {
            return Err(anyhow!(
                "source run is not the latest run in its provider workspace lineage"
            ));
        }

        let active: bool = sqlx::query_scalar(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM runs
                WHERE task_id = $1
                  AND status IN ('provisioning', 'starting', 'running',
                                 'waiting_for_input', 'waiting_for_approval', 'verifying')
            )
            "#,
        )
        .bind(task_id)
        .fetch_one(&mut *tx)
        .await?;
        if active {
            return Err(anyhow!("task already has an active run"));
        }

        let (mission_tokens_used, mission_cost_used) =
            budget_revision::mission_usage_tx(&mut tx, corp_id, mission_id).await?;
        let mission_budget_tokens: i64 = row.get("mission_budget_tokens");
        let mission_budget_cost_microusd: i64 = row.get("mission_budget_cost_microusd");
        let remaining_mission_tokens = mission_budget_tokens - mission_tokens_used;
        let remaining_mission_cost_microusd = mission_budget_cost_microusd - mission_cost_used;
        if remaining_mission_tokens <= 0 || remaining_mission_cost_microusd <= 0 {
            return Err(anyhow!(
                "mission has no remaining authorized budget; an owner or admin must approve a budget revision before resume"
            ));
        }
        let rolling = rolling_budget_remaining_tx(&mut tx, corp_id, requester).await?;
        if rolling.actor_tokens <= 0
            || rolling.actor_cost_microusd <= 0
            || rolling.corp_tokens <= 0
            || rolling.corp_cost_microusd <= 0
        {
            return Err(anyhow!(
                "requester or Corp rolling budget has no remaining authority; wait for the window to clear or revise Corp policy before resume"
            ));
        }
        let resume_budget_tokens = contract
            .budget_tokens
            .min(remaining_mission_tokens)
            .min(rolling.actor_tokens)
            .min(rolling.corp_tokens);
        let resume_budget_cost_microusd = contract
            .budget_cost_microusd
            .min(remaining_mission_cost_microusd)
            .min(rolling.actor_cost_microusd)
            .min(rolling.corp_cost_microusd);

        let run_id = Uuid::new_v4();
        let assignment_token = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO runs
                (id, corp_id, task_id, agent_id, runner_id, assignment_token, status,
                 provider_session_id, resumed_from_run_id, workspace_run_id,
                 budget_tokens_limit, budget_cost_microusd_limit, model, reasoning_effort,
                 source_repository, source_base_ref, source_base_commit)
            VALUES ($1, $2, $3, $4, $5, $6, 'starting', $7, $8, $9, $10, $11, $12, $13,
                    $14, $15, $16)
            "#,
        )
        .bind(run_id)
        .bind(corp_id)
        .bind(task_id)
        .bind(agent_id)
        .bind(&runner_id)
        .bind(assignment_token)
        .bind(&provider_session_id)
        .bind(source_run_id)
        .bind(workspace_run_id)
        .bind(resume_budget_tokens)
        .bind(resume_budget_cost_microusd)
        .bind(&model)
        .bind(&reasoning_effort)
        .bind(&contract.source_repository)
        .bind(&contract.source_base_ref)
        .bind(&contract.source_base_commit)
        .execute(&mut *tx)
        .await?;
        let queued_messages =
            reserve_queued_messages_tx(&mut tx, corp_id, agent_id, run_id).await?;
        sqlx::query("UPDATE missions SET status = 'running', updated_at = now() WHERE id = $1")
            .bind(mission_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE tasks SET status = 'claimed', verification_status = 'pending', updated_at = now() WHERE id = $1",
        )
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE agents SET status = 'starting', station = 'dispatch', current_run_id = $1 WHERE id = $2",
        )
        .bind(run_id)
        .bind(agent_id)
        .execute(&mut *tx)
        .await?;

        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(room_id),
                correlation_id: Some(mission_id),
                ..NewEvent::new(
                    corp_id,
                    Some(requested_by),
                    "run.resume_requested",
                    "run",
                    run_id,
                    format!("run:{run_id}:resume-requested"),
                    json!({
                        "source_run_id": source_run_id,
                        "workspace_run_id": workspace_run_id,
                        "task_id": task_id,
                        "agent_id": agent_id,
                        "runner_id": runner_id,
                        "model": model,
                        "reasoning_effort": reasoning_effort,
                        "mission_tokens_used": mission_tokens_used,
                        "mission_cost_microusd_used": mission_cost_used,
                        "remaining_mission_tokens": remaining_mission_tokens,
                        "remaining_mission_cost_microusd": remaining_mission_cost_microusd,
                        "remaining_actor_tokens_24h": rolling.actor_tokens,
                        "remaining_actor_cost_microusd_24h": rolling.actor_cost_microusd,
                        "remaining_corp_tokens_24h": rolling.corp_tokens,
                        "remaining_corp_cost_microusd_24h": rolling.corp_cost_microusd,
                        "resume_budget_tokens": resume_budget_tokens,
                        "resume_budget_cost_microusd": resume_budget_cost_microusd,
                    }),
                )
            },
        )
        .await?
        .context("run resume event unexpectedly existed")?;
        tx.commit().await?;
        Ok((
            ResumeLaunchRecord {
                corp_id,
                room_id,
                mission_id,
                task_id,
                run_id,
                source_run_id,
                workspace_run_id,
                agent_id,
                runner_id,
                assignment_token,
                adapter,
                provider_session_id,
                task_prompt,
                model,
                reasoning_effort,
                source_repository: contract.source_repository.clone(),
                source_base_ref: contract.source_base_ref.clone(),
                source_base_commit: contract.source_base_commit.clone(),
                workspace_base_commit,
                verification_policy,
                write_scope: contract.write_scope,
                deliverable: contract.deliverable,
                secret_refs: contract.secret_refs,
                queued_messages,
            },
            event,
        ))
    }

    pub async fn fail_run_before_dispatch(
        &self,
        corp_id: Uuid,
        run_id: Uuid,
        reason: &str,
    ) -> Result<DomainEvent> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            r#"
            SELECT run.task_id, run.agent_id, task.mission_id, mission.room_id
            FROM runs run
            JOIN tasks task ON task.id = run.task_id
            JOIN missions mission ON mission.id = task.mission_id
            WHERE run.id = $1 AND run.corp_id = $2
            FOR UPDATE OF run, task, mission
            "#,
        )
        .bind(run_id)
        .bind(corp_id)
        .fetch_one(&mut *tx)
        .await
        .context("run not found for dispatch failure")?;
        let task_id: Uuid = row.get("task_id");
        let agent_id: Uuid = row.get("agent_id");
        let mission_id: Uuid = row.get("mission_id");
        let room_id: Uuid = row.get("room_id");
        sqlx::query(
            "UPDATE runs SET status = 'failed', summary = $1,
             workspace_detail = 'dispatch_not_started', updated_at = now() WHERE id = $2",
        )
        .bind(reason)
        .bind(run_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE queued_messages SET status = 'queued', run_id = NULL WHERE run_id = $1 AND status = 'reserved'",
        )
        .bind(run_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE tasks SET status = 'failed', updated_at = now() WHERE id = $1")
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE missions SET status = 'failed', updated_at = now() WHERE id = $1")
            .bind(mission_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE agents SET status = 'idle', station = NULL, current_run_id = NULL WHERE id = $1 AND current_run_id = $2",
        )
        .bind(agent_id)
        .bind(run_id)
        .execute(&mut *tx)
        .await?;
        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(room_id),
                correlation_id: Some(mission_id),
                ..NewEvent::new(
                    corp_id,
                    None,
                    "run.failed",
                    "run",
                    run_id,
                    format!("run:{run_id}:dispatch-failed"),
                    json!({"error": reason}),
                )
            },
        )
        .await?
        .context("dispatch failure event unexpectedly existed")?;
        tx.commit().await?;
        Ok(event)
    }

    pub async fn artifact_context(
        &self,
        corp_id: Uuid,
        run_id: Uuid,
        agent_id: Uuid,
        runner_id: &str,
        assignment_token: Uuid,
    ) -> Result<ArtifactContext> {
        let row = sqlx::query(
            r#"
            SELECT r.task_id, t.mission_id, m.room_id
            FROM runs r
            JOIN tasks t ON t.id = r.task_id
            JOIN missions m ON m.id = t.mission_id
            WHERE r.id = $1 AND r.corp_id = $2 AND r.agent_id = $3 AND r.runner_id = $4
              AND r.assignment_token = $5
              AND r.status IN ('provisioning', 'starting', 'running',
                               'waiting_for_input', 'waiting_for_approval', 'verifying')
              AND r.breaker_stage NOT IN ('suspend', 'stop')
            "#,
        )
        .bind(run_id)
        .bind(corp_id)
        .bind(agent_id)
        .bind(runner_id)
        .bind(assignment_token)
        .fetch_one(&self.pool)
        .await
        .context("artifact upload does not match an active run")?;
        Ok(ArtifactContext {
            task_id: row.get("task_id"),
            mission_id: row.get("mission_id"),
            room_id: row.get("room_id"),
        })
    }

    pub async fn prepare_artifact_upload(
        &self,
        input: RunnerEventInput,
        artifact: StoredArtifact,
        staging_key: &str,
    ) -> Result<PreparedArtifactUpload> {
        let RunnerEventInput {
            event_id,
            runner_id,
            corp_id,
            connection_epoch: _,
            run_id,
            agent_id,
            assignment_token,
            event_type,
            ..
        } = input;
        let expected_role = match event_type.as_str() {
            "run.artifact_upload" => "provider_evidence",
            "run.deliverable_upload" => "source_deliverable",
            _ => return Err(anyhow!("unsupported artifact upload event")),
        };
        if artifact.artifact_role != expected_role
            || artifact.id != event_id
            || artifact.corp_id != corp_id
            || artifact.run_id != run_id
            || artifact.producer_agent_id != agent_id
            || artifact.producer_runner_id != runner_id
        {
            return Err(anyhow!("artifact metadata does not match the runner event"));
        }
        let expected_staging_key = format!("staging/corps/{corp_id}/{event_id}");
        if staging_key != expected_staging_key {
            return Err(anyhow!(
                "artifact staging key does not match the runner event"
            ));
        }

        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            r#"
            SELECT r.task_id, r.breaker_stage, t.mission_id, m.room_id
            FROM runs r
            JOIN tasks t ON t.id = r.task_id
            JOIN missions m ON m.id = t.mission_id
            WHERE r.id = $1 AND r.corp_id = $2 AND r.agent_id = $3 AND r.runner_id = $4
              AND r.assignment_token = $5
              AND r.status IN ('provisioning', 'starting', 'running',
                               'waiting_for_input', 'waiting_for_approval', 'verifying')
              AND r.breaker_stage NOT IN ('suspend', 'stop')
            FOR UPDATE OF r, t, m
            "#,
        )
        .bind(run_id)
        .bind(corp_id)
        .bind(agent_id)
        .bind(&runner_id)
        .bind(assignment_token)
        .fetch_one(&mut *tx)
        .await
        .context("artifact upload does not match an active run")?;
        let task_id: Uuid = row.get("task_id");
        let breaker_stage: String = row.get("breaker_stage");
        ensure_run_not_hard_blocked_tx(&mut tx, corp_id, run_id, &breaker_stage, "artifact upload")
            .await?;
        if artifact.task_id != task_id {
            return Err(anyhow!("artifact task does not match the active run"));
        }

        let existing_by_id = sqlx::query(
            r#"
            SELECT id, corp_id, task_id, run_id, producer_agent_id, producer_runner_id,
                   verifier, object_key, uri, sha256, media_type, bytes,
                   artifact_role, file_name, metadata, provenance_signature, retention_until,
                   status, staging_key, created_at
            FROM artifacts
            WHERE corp_id = $1 AND id = $2
            FOR UPDATE
            "#,
        )
        .bind(corp_id)
        .bind(artifact.id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = existing_by_id {
            let prepared = map_prepared_artifact(row)?;
            ensure_artifact_upload_matches(&prepared.artifact, &artifact, true)?;
            tx.commit().await?;
            return Ok(prepared);
        }

        let existing_by_digest = sqlx::query(
            r#"
            SELECT id, corp_id, task_id, run_id, producer_agent_id, producer_runner_id,
                   verifier, object_key, uri, sha256, media_type, bytes,
                   artifact_role, file_name, metadata, provenance_signature, retention_until,
                   status, staging_key, created_at
            FROM artifacts
            WHERE corp_id = $1 AND run_id = $2 AND artifact_role = $3 AND sha256 = $4
            FOR UPDATE
            "#,
        )
        .bind(corp_id)
        .bind(run_id)
        .bind(&artifact.artifact_role)
        .bind(&artifact.sha256)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = existing_by_digest {
            let prepared = map_prepared_artifact(row)?;
            ensure_artifact_upload_matches(&prepared.artifact, &artifact, false)?;
            tx.commit().await?;
            return Ok(prepared);
        }

        let created_at = sqlx::query_scalar(
            r#"
            INSERT INTO artifacts
                (id, corp_id, task_id, run_id, producer_agent_id, producer_runner_id,
                 verifier, object_key, uri, sha256, media_type, bytes, artifact_role, file_name,
                 metadata, retention_until, provenance_signature, status, staging_key)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                    $15, $16, $17, 'staged', $18)
            RETURNING created_at
            "#,
        )
        .bind(artifact.id)
        .bind(artifact.corp_id)
        .bind(artifact.task_id)
        .bind(artifact.run_id)
        .bind(artifact.producer_agent_id)
        .bind(&artifact.producer_runner_id)
        .bind(&artifact.verifier)
        .bind(&artifact.object_key)
        .bind(&artifact.uri)
        .bind(&artifact.sha256)
        .bind(&artifact.media_type)
        .bind(artifact.bytes)
        .bind(&artifact.artifact_role)
        .bind(&artifact.file_name)
        .bind(&artifact.metadata)
        .bind(artifact.retention_until)
        .bind(&artifact.provenance_signature)
        .bind(staging_key)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(PreparedArtifactUpload {
            artifact,
            staging_key: Some(staging_key.to_owned()),
            status: "staged".to_owned(),
            created_at,
        })
    }

    pub async fn finalize_artifact_upload(
        &self,
        corp_id: Uuid,
        artifact_id: Uuid,
    ) -> Result<Option<DomainEvent>> {
        let mut tx = self.pool.begin().await?;
        let run_id: Uuid =
            sqlx::query_scalar("SELECT run_id FROM artifacts WHERE id = $1 AND corp_id = $2")
                .bind(artifact_id)
                .bind(corp_id)
                .fetch_one(&mut *tx)
                .await
                .context("staged artifact not found")?;
        sqlx::query("SELECT id FROM runs WHERE id = $1 AND corp_id = $2 FOR UPDATE")
            .bind(run_id)
            .bind(corp_id)
            .fetch_one(&mut *tx)
            .await
            .context("staged artifact run not found")?;
        let row = sqlx::query(
            r#"
            SELECT artifact.id, artifact.corp_id, artifact.task_id, artifact.run_id,
                   artifact.producer_agent_id, artifact.producer_runner_id,
                   artifact.verifier, artifact.object_key, artifact.uri,
                   artifact.sha256, artifact.media_type, artifact.bytes,
                   artifact.artifact_role, artifact.file_name, artifact.metadata,
                   artifact.provenance_signature, artifact.retention_until,
                   artifact.status, artifact.staging_key,
                   run.status AS run_status, run.agent_id,
                   task.mission_id, mission.room_id
            FROM artifacts artifact
            JOIN runs run ON run.id = artifact.run_id
            JOIN tasks task ON task.id = artifact.task_id
            JOIN missions mission ON mission.id = task.mission_id
            WHERE artifact.id = $1 AND artifact.corp_id = $2 AND artifact.run_id = $3
            FOR UPDATE OF artifact, task, mission
            "#,
        )
        .bind(artifact_id)
        .bind(corp_id)
        .bind(run_id)
        .fetch_one(&mut *tx)
        .await
        .context("staged artifact not found")?;
        let status: String = row.get("status");
        if status == "ready" {
            tx.commit().await?;
            return Ok(None);
        }
        if status != "staged" {
            return Err(anyhow!("artifact cannot finalize from status {status}"));
        }
        let run_status: String = row.get("run_status");
        let agent_id: Uuid = row.get("agent_id");
        let mission_id: Uuid = row.get("mission_id");
        let room_id: Uuid = row.get("room_id");
        let artifact = map_stored_artifact(row);
        let ready_event_type = if artifact.artifact_role == "source_deliverable" {
            "run.deliverable"
        } else {
            "run.artifact"
        };

        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(room_id),
                correlation_id: Some(mission_id),
                ..NewEvent::new(
                    artifact.corp_id,
                    None,
                    ready_event_type,
                    "run",
                    artifact.run_id,
                    format!("artifact:{}:ready", artifact.id),
                    json!({
                        "artifact_id": artifact.id,
                        "uri": artifact.uri,
                        "sha256": artifact.sha256,
                        "bytes": artifact.bytes,
                        "media_type": artifact.media_type,
                        "artifact_role": artifact.artifact_role,
                        "file_name": artifact.file_name,
                        "metadata": artifact.metadata,
                        "producer_agent_id": artifact.producer_agent_id,
                        "producer_runner_id": artifact.producer_runner_id,
                        "verifier": artifact.verifier,
                        "retention_until": artifact.retention_until,
                        "provenance_signature": artifact.provenance_signature,
                    }),
                )
            },
        )
        .await?
        .context("artifact finalization event unexpectedly existed")?;

        sqlx::query(
            r#"
            UPDATE artifacts
            SET status = 'ready', finalized_at = now(),
                rejection_reason = NULL
            WHERE id = $1 AND status = 'staged'
            "#,
        )
        .bind(artifact.id)
        .execute(&mut *tx)
        .await?;
        let run_active = matches!(
            run_status.as_str(),
            "provisioning"
                | "starting"
                | "running"
                | "waiting_for_input"
                | "waiting_for_approval"
                | "verifying"
        );
        if artifact.artifact_role == "source_deliverable" {
            let form = artifact
                .metadata
                .get("form")
                .and_then(Value::as_str)
                .context("source deliverable omitted form")?;
            let verification_sha256 = artifact
                .metadata
                .get("verification_sha256")
                .and_then(Value::as_str)
                .context("source deliverable omitted verification digest")?;
            let base_commit = artifact
                .metadata
                .get("base_commit")
                .and_then(Value::as_str)
                .context("source deliverable omitted base commit")?;
            let head_commit = artifact.metadata.get("head_commit").and_then(Value::as_str);
            let branch = artifact
                .metadata
                .get("branch")
                .and_then(Value::as_str)
                .context("source deliverable omitted branch")?;
            let integration_state = artifact
                .metadata
                .get("integration_state")
                .and_then(Value::as_str)
                .context("source deliverable omitted integration state")?;
            sqlx::query(
                r#"
                INSERT INTO source_deliverables
                    (id, corp_id, task_id, run_id, artifact_id, form, file_name,
                     verification_sha256, base_commit, head_commit, branch, integration_state)
                VALUES ($1, $2, $3, $4, $1, $5, $6, $7, $8, $9, $10, $11)
                ON CONFLICT (run_id) DO NOTHING
                "#,
            )
            .bind(artifact.id)
            .bind(artifact.corp_id)
            .bind(artifact.task_id)
            .bind(artifact.run_id)
            .bind(form)
            .bind(&artifact.file_name)
            .bind(verification_sha256)
            .bind(base_commit)
            .bind(head_commit)
            .bind(branch)
            .bind(integration_state)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE runs SET deliverable_sha256 = $1, updated_at = now() WHERE id = $2",
            )
            .bind(&artifact.sha256)
            .bind(artifact.run_id)
            .execute(&mut *tx)
            .await?;
        } else {
            sqlx::query(
                r#"
                UPDATE runs
                SET artifact_id = $1,
                    artifact_uri = $2,
                    artifact_media_type = $3,
                    artifact_signature = $4,
                    artifact_path = NULL,
                    artifact_sha256 = $5,
                    updated_at = now()
                WHERE id = $6
                "#,
            )
            .bind(artifact.id)
            .bind(&artifact.uri)
            .bind(&artifact.media_type)
            .bind(&artifact.provenance_signature)
            .bind(&artifact.sha256)
            .bind(artifact.run_id)
            .execute(&mut *tx)
            .await?;
        }
        if run_active {
            sqlx::query("UPDATE runs SET status = 'verifying', updated_at = now() WHERE id = $1")
                .bind(artifact.run_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE tasks SET status = 'review', updated_at = now() WHERE id = $1")
                .bind(artifact.task_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "UPDATE agents SET status = 'reviewing', station = 'review', current_run_id = NULL WHERE id = $1",
            )
            .bind(agent_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(Some(event))
    }

    pub async fn reject_staged_artifact(
        &self,
        corp_id: Uuid,
        artifact_id: Uuid,
        reason: &str,
    ) -> Result<Option<String>> {
        let reason = normalize_artifact_rejection_reason(reason);
        let mut tx = self.pool.begin().await?;
        let staging_key: Option<String> = sqlx::query_scalar(
            "SELECT staging_key FROM artifacts WHERE id = $1 AND corp_id = $2 AND status = 'staged' FOR UPDATE",
        )
        .bind(artifact_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?
        .flatten();
        if staging_key.is_some() {
            sqlx::query(
                r#"
                UPDATE artifacts
                SET status = 'rejected', rejection_reason = $1
                WHERE id = $2 AND corp_id = $3 AND status = 'staged'
                "#,
            )
            .bind(&reason)
            .bind(artifact_id)
            .bind(corp_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(staging_key)
    }

    pub async fn abandon_staged_artifact(
        &self,
        corp_id: Uuid,
        artifact_id: Uuid,
    ) -> Result<Option<String>> {
        Ok(sqlx::query_scalar(
            r#"
            DELETE FROM artifacts
            WHERE id = $1 AND corp_id = $2 AND status = 'staged'
            RETURNING staging_key
            "#,
        )
        .bind(artifact_id)
        .bind(corp_id)
        .fetch_optional(&self.pool)
        .await?
        .flatten())
    }

    pub async fn artifact_staging_key_is_reserved(&self, staging_key: &str) -> Result<bool> {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM artifacts WHERE staging_key = $1)")
            .bind(staging_key)
            .fetch_one(&self.pool)
            .await
            .map_err(Into::into)
    }

    pub async fn clear_artifact_staging_key(
        &self,
        corp_id: Uuid,
        artifact_id: Uuid,
        staging_key: &str,
    ) -> Result<bool> {
        Ok(sqlx::query(
            "UPDATE artifacts SET staging_key = NULL WHERE id = $1 AND corp_id = $2 AND staging_key = $3 AND status <> 'staged'",
        )
        .bind(artifact_id)
        .bind(corp_id)
        .bind(staging_key)
        .execute(&self.pool)
        .await?
        .rows_affected()
            == 1)
    }

    pub async fn pending_artifact_uploads(
        &self,
        limit: i64,
    ) -> Result<Vec<PreparedArtifactUpload>> {
        if !(1..=1_000).contains(&limit) {
            return Err(anyhow!(
                "artifact recovery limit must be between 1 and 1000"
            ));
        }
        sqlx::query(
            r#"
            SELECT id, corp_id, task_id, run_id, producer_agent_id, producer_runner_id,
                   verifier, object_key, uri, sha256, media_type, bytes,
                   artifact_role, file_name, metadata, provenance_signature, retention_until,
                   status, staging_key, created_at
            FROM artifacts
            WHERE status = 'staged' OR staging_key IS NOT NULL
            ORDER BY created_at, id
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(map_prepared_artifact)
        .collect()
    }

    pub async fn artifact_for_download(
        &self,
        corp_id: Uuid,
        artifact_id: Uuid,
        actor_id: Uuid,
    ) -> Result<Option<StoredArtifact>> {
        let row = sqlx::query(
            r#"
            SELECT artifact.id, artifact.corp_id, artifact.task_id, artifact.run_id,
                   artifact.producer_agent_id, artifact.producer_runner_id,
                   artifact.verifier, artifact.object_key, artifact.uri,
                    artifact.sha256, artifact.media_type, artifact.bytes,
                    artifact.artifact_role, artifact.file_name, artifact.metadata,
                    artifact.provenance_signature, artifact.retention_until
            FROM artifacts artifact
            JOIN tasks task ON task.id = artifact.task_id
            JOIN missions mission ON mission.id = task.mission_id
            JOIN room_memberships membership
              ON membership.room_id = mission.room_id AND membership.actor_id = $3
            WHERE artifact.id = $1 AND artifact.corp_id = $2
              AND artifact.status = 'ready'
            "#,
        )
        .bind(artifact_id)
        .bind(corp_id)
        .bind(actor_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(map_stored_artifact))
    }

    pub async fn ready_artifact_for_run_role_digest(
        &self,
        corp_id: Uuid,
        run_id: Uuid,
        artifact_role: &str,
        sha256: &str,
    ) -> Result<Option<StoredArtifact>> {
        let row = sqlx::query(
            r#"
            SELECT id, corp_id, task_id, run_id, producer_agent_id, producer_runner_id,
                   verifier, object_key, uri, sha256, media_type, bytes,
                   artifact_role, file_name, metadata, provenance_signature, retention_until
            FROM artifacts
            WHERE corp_id = $1 AND run_id = $2 AND artifact_role = $3 AND sha256 = $4
              AND status = 'ready'
            "#,
        )
        .bind(corp_id)
        .bind(run_id)
        .bind(artifact_role)
        .bind(sha256)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(map_stored_artifact))
    }

    pub async fn dependency_artifacts(
        &self,
        corp_id: Uuid,
        task_id: Uuid,
    ) -> Result<Vec<DependencyArtifactContext>> {
        let rows = sqlx::query(
            r#"
            WITH ranked AS (
                SELECT parent.id AS dependency_task_id,
                       parent.plan_key,
                       parent.title AS task_title,
                       run.summary AS run_summary,
                       artifact.id,
                       artifact.corp_id,
                       artifact.task_id,
                       artifact.run_id,
                       artifact.producer_agent_id,
                       artifact.producer_runner_id,
                       artifact.verifier,
                       artifact.object_key,
                       artifact.uri,
                       artifact.sha256,
                        artifact.media_type,
                        artifact.bytes,
                        artifact.artifact_role,
                        artifact.file_name,
                        artifact.metadata,
                        artifact.provenance_signature,
                       artifact.retention_until,
                       row_number() OVER (
                           PARTITION BY parent.id
                           ORDER BY run.created_at DESC
                       ) AS rank
                FROM task_dependencies dependency
                JOIN tasks child ON child.id = dependency.task_id
                JOIN tasks parent ON parent.id = dependency.depends_on_task_id
                JOIN runs run ON run.task_id = parent.id AND run.status = 'completed'
                JOIN artifacts artifact
                  ON artifact.run_id = run.id
                 AND artifact.status = 'ready'
                 AND artifact.artifact_role = 'provider_evidence'
                WHERE child.id = $1 AND child.corp_id = $2
            )
            SELECT dependency_task_id, plan_key, task_title, run_summary,
                   id, corp_id, task_id, run_id, producer_agent_id,
                    producer_runner_id, verifier, object_key, uri, sha256,
                    media_type, bytes, artifact_role, file_name, metadata,
                    provenance_signature, retention_until
            FROM ranked
            WHERE rank = 1
            ORDER BY plan_key
            "#,
        )
        .bind(task_id)
        .bind(corp_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let dependency_task_id = row.get("dependency_task_id");
                let plan_key = row.get("plan_key");
                let task_title = row.get("task_title");
                let run_summary = row.get("run_summary");
                let artifact = map_stored_artifact(row);
                DependencyArtifactContext {
                    task_id: dependency_task_id,
                    plan_key,
                    task_title,
                    run_summary,
                    artifact,
                }
            })
            .collect())
    }

    pub async fn apply_runner_event(&self, input: RunnerEventInput) -> Result<Option<DomainEvent>> {
        let RunnerEventInput {
            event_id,
            runner_id,
            corp_id,
            connection_epoch: _,
            run_id,
            agent_id,
            assignment_token,
            event_type,
            mut payload,
        } = input;
        let mut tx = self.pool.begin().await?;
        if event_type == "run.usage" {
            let requester: Uuid = sqlx::query_scalar(
                r#"
                SELECT mission.requested_by
                FROM runs run
                JOIN tasks task ON task.id = run.task_id
                JOIN missions mission ON mission.id = task.mission_id
                WHERE run.id = $1 AND run.corp_id = $2 AND run.agent_id = $3
                  AND run.runner_id = $4 AND run.assignment_token = $5
                  AND run.status IN (
                    'provisioning', 'starting', 'running',
                    'waiting_for_input', 'waiting_for_approval', 'verifying'
                  )
                "#,
            )
            .bind(run_id)
            .bind(corp_id)
            .bind(agent_id)
            .bind(&runner_id)
            .bind(assignment_token)
            .fetch_one(&mut *tx)
            .await
            .context("runner usage event does not match an active run")?;
            lock_factory_keys_tx(&mut tx, &budget_scope_lock_keys(corp_id, requester)).await?;
        }
        let row = sqlx::query(
            r#"
            SELECT r.task_id, r.verification_status AS run_verification_status,
                   r.breaker_stage,
                   t.mission_id, t.contract, t.verification_policy, m.room_id,
                   t.attempt_count, t.max_attempts
            FROM runs r
            JOIN tasks t ON t.id = r.task_id
            JOIN missions m ON m.id = t.mission_id
            WHERE r.id = $1 AND r.corp_id = $2 AND r.agent_id = $3 AND r.runner_id = $4
              AND r.assignment_token = $5
              AND (
                r.status IN ('provisioning', 'starting', 'running',
                             'waiting_for_input', 'waiting_for_approval', 'verifying')
                OR (
                  $6::text IN ('run.workspace_preserved', 'run.workspace_removed')
                  AND r.status IN ('completed', 'failed', 'cancelled', 'lost')
                )
              )
            FOR UPDATE OF r, t, m
            "#,
        )
        .bind(run_id)
        .bind(corp_id)
        .bind(agent_id)
        .bind(&runner_id)
        .bind(assignment_token)
        .bind(&event_type)
        .fetch_one(&mut *tx)
        .await
        .context("runner event does not match an active run")?;
        let task_id: Uuid = row.get("task_id");
        let mission_id: Uuid = row.get("mission_id");
        let room_id: Uuid = row.get("room_id");
        let attempt_count: i32 = row.get("attempt_count");
        let max_attempts: i32 = row.get("max_attempts");
        let run_verification_status: String = row.get("run_verification_status");
        let breaker_stage: String = row.get("breaker_stage");
        let verification_policy: VerificationPolicy =
            serde_json::from_value(row.get("verification_policy"))
                .context("decode task verification policy")?;
        let task_contract: TaskContract =
            serde_json::from_value(row.get("contract")).context("decode task contract")?;
        if breaker_blocks_runner_progress(&breaker_stage, &event_type) {
            return Err(anyhow!(
                "run event {event_type} is blocked by breaker stage {breaker_stage}"
            ));
        }
        if runner_event_advances_run(&event_type) {
            ensure_run_not_hard_blocked_tx(
                &mut tx,
                corp_id,
                run_id,
                &breaker_stage,
                &format!("run event {event_type}"),
            )
            .await?;
        }
        if event_type == "run.verification_evidence" {
            payload = sanitize_verification_evidence_tx(&mut tx, run_id, payload).await?;
        }

        let event = append_event_tx(
            &mut tx,
            NewEvent {
                id: event_id,
                room_id: Some(room_id),
                correlation_id: Some(mission_id),
                ..NewEvent::new(
                    corp_id,
                    None,
                    &event_type,
                    "run",
                    run_id,
                    format!("runner:{runner_id}:event:{event_id}"),
                    payload.clone(),
                )
            },
        )
        .await?;
        let Some(event) = event else {
            tx.rollback().await?;
            return Ok(None);
        };

        match event_type.as_str() {
            "run.session" => {
                let session_id = payload
                    .get("session_id")
                    .and_then(Value::as_str)
                    .context("run.session event omitted session_id")?;
                sqlx::query(
                    "UPDATE runs SET provider_session_id = $1, updated_at = now() WHERE id = $2",
                )
                .bind(session_id)
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
            }
            "run.started" => {
                let workspace_path = payload.get("workspace").and_then(Value::as_str);
                let workspace_branch = payload.get("workspace_branch").and_then(Value::as_str);
                let workspace_base_ref = payload.get("workspace_base_ref").and_then(Value::as_str);
                let workspace_base_commit =
                    payload.get("workspace_base_commit").and_then(Value::as_str);
                sqlx::query(
                    r#"
                    UPDATE runs
                    SET status = 'running',
                        workspace_path = $1,
                        workspace_branch = $2,
                        workspace_base_ref = $3,
                        workspace_base_commit = $4,
                        workspace_disposition = 'active',
                        workspace_detail = NULL,
                        updated_at = now()
                    WHERE id = $5
                    "#,
                )
                .bind(workspace_path)
                .bind(workspace_branch)
                .bind(workspace_base_ref)
                .bind(workspace_base_commit)
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE tasks SET status = 'running', updated_at = now() WHERE id = $1",
                )
                .bind(task_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE agents SET status = 'working', station = 'terminal' WHERE id = $1",
                )
                .bind(agent_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE queued_messages SET status = 'delivered', delivered_at = now() WHERE run_id = $1 AND status = 'reserved'",
                )
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
            }
            "run.status" => {
                let status = payload
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("working");
                let agent_status = match status {
                    "blocked" => "blocked",
                    "reviewing" => "reviewing",
                    "idle" => "idle",
                    _ => "working",
                };
                let station = payload
                    .get("station")
                    .and_then(Value::as_str)
                    .unwrap_or("terminal");
                sqlx::query("UPDATE agents SET status = $1, station = $2 WHERE id = $3")
                    .bind(agent_status)
                    .bind(station)
                    .bind(agent_id)
                    .execute(&mut *tx)
                    .await?;
                if status == "working" {
                    sqlx::query(
                        r#"
                        UPDATE runs
                        SET status = 'running', updated_at = now()
                        WHERE id = $1
                          AND status IN ('waiting_for_input', 'waiting_for_approval')
                          AND NOT EXISTS (
                            SELECT 1 FROM action_approvals
                            WHERE run_id = $1 AND status = 'pending'
                          )
                        "#,
                    )
                    .bind(run_id)
                    .execute(&mut *tx)
                    .await?;
                    sqlx::query(
                        r#"
                        UPDATE tasks
                        SET status = 'running', updated_at = now()
                        WHERE id = $1
                          AND status = 'awaiting_approval'
                          AND NOT EXISTS (
                            SELECT 1 FROM action_approvals
                            WHERE task_id = $1 AND status = 'pending'
                          )
                        "#,
                    )
                    .bind(task_id)
                    .execute(&mut *tx)
                    .await?;
                }
            }
            "run.approval_requested" => {
                let approval_id = payload
                    .get("approval_id")
                    .and_then(Value::as_str)
                    .context("approval request omitted approval_id")
                    .and_then(|value| Uuid::parse_str(value).context("approval id is invalid"))?;
                let action_key = payload
                    .get("action_key")
                    .and_then(Value::as_str)
                    .context("approval request omitted action_key")?;
                let action = payload
                    .get("action")
                    .and_then(Value::as_str)
                    .context("approval request omitted action")?;
                let risk = payload
                    .get("risk")
                    .and_then(Value::as_str)
                    .context("approval request omitted risk")?;
                let rationale = payload
                    .get("rationale")
                    .and_then(Value::as_str)
                    .context("approval request omitted rationale")?;
                let required_roles = payload
                    .get("required_roles")
                    .and_then(Value::as_array)
                    .context("approval request omitted required_roles")?
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .map(str::to_owned)
                            .context("approval role is not a string")
                    })
                    .collect::<Result<Vec<_>>>()?;
                if action_key.is_empty()
                    || action_key.len() > 128
                    || action.is_empty()
                    || action.len() > 2_000
                    || rationale.is_empty()
                    || rationale.len() > 4_000
                    || !matches!(risk, "low" | "medium" | "high" | "critical")
                    || required_roles.is_empty()
                    || required_roles.iter().any(|role| {
                        !matches!(
                            role.as_str(),
                            "owner" | "admin" | "manager" | "member" | "guest" | "spectator"
                        )
                    })
                {
                    return Err(anyhow!("approval request is invalid"));
                }
                let expires_in_seconds = payload
                    .get("expires_in_seconds")
                    .and_then(Value::as_i64)
                    .unwrap_or(300)
                    .clamp(1, 3_600);
                let expires_at = Utc::now() + Duration::seconds(expires_in_seconds);
                sqlx::query(
                    r#"
                    INSERT INTO action_approvals
                        (id, corp_id, room_id, mission_id, task_id, run_id, agent_id,
                         action_key, action, risk, rationale, required_roles, expires_at)
                    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                    "#,
                )
                .bind(approval_id)
                .bind(corp_id)
                .bind(room_id)
                .bind(mission_id)
                .bind(task_id)
                .bind(run_id)
                .bind(agent_id)
                .bind(action_key)
                .bind(action)
                .bind(risk)
                .bind(rationale)
                .bind(&required_roles)
                .bind(expires_at)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE runs SET status = 'waiting_for_approval', updated_at = now() WHERE id = $1",
                )
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE tasks SET status = 'awaiting_approval', updated_at = now() WHERE id = $1",
                )
                .bind(task_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE agents SET status = 'blocked', station = 'approval' WHERE id = $1",
                )
                .bind(agent_id)
                .execute(&mut *tx)
                .await?;
            }
            "run.tool_activity" => {
                let human_conversation = payload
                    .get("human_conversation")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if !human_conversation {
                    let progressed = payload
                        .get("progressed")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let signature = payload
                        .get("signature")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    if progressed {
                        sqlx::query(
                            "UPDATE runs SET no_progress_events = 0, repeated_tool_count = 0, last_tool_signature = $1, last_progress_at = now(), updated_at = now() WHERE id = $2",
                        )
                        .bind(signature)
                        .bind(run_id)
                        .execute(&mut *tx)
                        .await?;
                    } else {
                        sqlx::query(
                            r#"
                            UPDATE runs
                            SET no_progress_events = no_progress_events + 1,
                                repeated_tool_count = CASE
                                    WHEN last_tool_signature = $1 THEN repeated_tool_count + 1
                                    ELSE 1
                                END,
                                last_tool_signature = $1,
                                updated_at = now()
                            WHERE id = $2
                            "#,
                        )
                        .bind(signature)
                        .bind(run_id)
                        .execute(&mut *tx)
                        .await?;
                    }
                }
            }
            "run.artifact"
            | "run.artifact_upload"
            | "run.deliverable"
            | "run.deliverable_upload" => {
                return Err(anyhow!(
                    "artifact events must pass through server-side object storage verification"
                ));
            }
            "run.usage" => {
                let input_tokens = payload
                    .get("input_tokens")
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
                    .max(0);
                let output_tokens = payload
                    .get("output_tokens")
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
                    .max(0);
                let cost_microusd = payload
                    .get("cost_microusd")
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
                    .max(0);
                sqlx::query(
                    r#"
                    UPDATE runs
                    SET input_tokens = input_tokens + $1,
                        output_tokens = output_tokens + $2,
                        cost_microusd = cost_microusd + $3,
                        updated_at = now()
                    WHERE id = $4
                    "#,
                )
                .bind(input_tokens)
                .bind(output_tokens)
                .bind(cost_microusd)
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
            }
            "run.verification_started" => {
                sqlx::query(
                    "UPDATE runs SET status = 'verifying', verification_status = 'running', verification_summary = NULL, updated_at = now() WHERE id = $1",
                )
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE tasks SET status = 'review', verification_status = 'running', updated_at = now() WHERE id = $1",
                )
                .bind(task_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE agents SET status = 'reviewing', station = 'review' WHERE id = $1",
                )
                .bind(agent_id)
                .execute(&mut *tx)
                .await?;
            }
            "run.verification_evidence" => {
                let evidence_id = payload
                    .get("evidence_id")
                    .and_then(Value::as_str)
                    .context("verification evidence omitted evidence_id")
                    .and_then(|value| {
                        Uuid::parse_str(value).context("verification evidence id is invalid")
                    })?;
                let check_index = payload
                    .get("check_index")
                    .and_then(Value::as_i64)
                    .context("verification evidence omitted check_index")?;
                let check_index = i32::try_from(check_index)
                    .context("verification check index is out of range")?;
                let expected = verification_policy
                    .checks
                    .get(check_index as usize)
                    .context("verification evidence references an unknown check")?;
                let kind = payload
                    .get("kind")
                    .and_then(Value::as_str)
                    .context("verification evidence omitted kind")?;
                if kind != expected.kind() {
                    return Err(anyhow!(
                        "verification evidence kind {kind} does not match policy {}",
                        expected.kind()
                    ));
                }
                let status = payload
                    .get("status")
                    .and_then(Value::as_str)
                    .context("verification evidence omitted status")?;
                if !matches!(status, "passed" | "failed") {
                    return Err(anyhow!("verification evidence status is invalid"));
                }
                let summary = payload
                    .get("summary")
                    .and_then(Value::as_str)
                    .context("verification evidence omitted summary")?;
                if summary.len() > 4_000 {
                    return Err(anyhow!("verification evidence summary is too long"));
                }
                sqlx::query(
                    r#"
                    INSERT INTO verification_evidence
                        (id, corp_id, task_id, run_id, check_index, kind, status, summary, payload)
                    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                    "#,
                )
                .bind(evidence_id)
                .bind(corp_id)
                .bind(task_id)
                .bind(run_id)
                .bind(check_index)
                .bind(kind)
                .bind(status)
                .bind(summary)
                .bind(payload.get("payload").cloned().unwrap_or_else(|| json!({})))
                .execute(&mut *tx)
                .await?;
            }
            "run.verification_passed" => {
                let (verification_sha256, deliverable_sha256) =
                    if task_contract.deliverable.is_some() {
                        let verification_sha256 = payload
                            .get("verification_sha256")
                            .and_then(Value::as_str)
                            .context("verification passed event omitted verification digest")?
                            .to_owned();
                        let deliverable_sha256 = payload
                            .get("deliverable_sha256")
                            .and_then(Value::as_str)
                            .context("verification passed event omitted deliverable digest")?
                            .to_owned();
                        let linked: bool = sqlx::query_scalar(
                            r#"
                            SELECT EXISTS(
                                SELECT 1
                                FROM source_deliverables deliverable
                                JOIN artifacts artifact ON artifact.id = deliverable.artifact_id
                                WHERE deliverable.run_id = $1
                                  AND deliverable.verification_sha256 = $2
                                  AND artifact.sha256 = $3
                                  AND artifact.status = 'ready'
                                  AND artifact.artifact_role = 'source_deliverable'
                            )
                            "#,
                        )
                        .bind(run_id)
                        .bind(&verification_sha256)
                        .bind(&deliverable_sha256)
                        .fetch_one(&mut *tx)
                        .await?;
                        if !linked {
                            return Err(anyhow!(
                                "verification passed without an exact ready deliverable linkage"
                            ));
                        }
                        (Some(verification_sha256), Some(deliverable_sha256))
                    } else {
                        (None, None)
                    };
                let requires_artifact = verification_policy
                    .checks
                    .iter()
                    .any(|check| matches!(check, crony_domain::VerifierCheck::Artifact { .. }));
                if requires_artifact {
                    let artifact_exists: bool = sqlx::query_scalar(
                        "SELECT artifact_id IS NOT NULL FROM runs WHERE id = $1",
                    )
                    .bind(run_id)
                    .fetch_one(&mut *tx)
                    .await?;
                    if !artifact_exists {
                        return Err(anyhow!(
                            "artifact verification passed before durable artifact storage"
                        ));
                    }
                }
                let evidence_rows = sqlx::query(
                    r#"
                    SELECT check_index, kind, status
                    FROM verification_evidence
                    WHERE run_id = $1
                    ORDER BY check_index
                    "#,
                )
                .bind(run_id)
                .fetch_all(&mut *tx)
                .await?;
                if evidence_rows.len() != verification_policy.checks.len() {
                    return Err(anyhow!(
                        "verification passed without complete evidence: expected {}, found {}",
                        verification_policy.checks.len(),
                        evidence_rows.len()
                    ));
                }
                for (index, row) in evidence_rows.into_iter().enumerate() {
                    let check_index: i32 = row.get("check_index");
                    let kind: String = row.get("kind");
                    let status: String = row.get("status");
                    let expected = &verification_policy.checks[index];
                    if check_index != index as i32 || kind != expected.kind() || status != "passed"
                    {
                        return Err(anyhow!("verification evidence does not satisfy policy"));
                    }
                }
                let summary = payload
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or("all verifier checks passed");
                sqlx::query(
                    "UPDATE runs SET verification_status = 'passed', verification_summary = $1, verification_sha256 = $2, deliverable_sha256 = $3, updated_at = now() WHERE id = $4",
                )
                .bind(summary)
                .bind(&verification_sha256)
                .bind(&deliverable_sha256)
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE tasks SET verification_status = 'passed', updated_at = now() WHERE id = $1",
                )
                .bind(task_id)
                .execute(&mut *tx)
                .await?;
            }
            "run.verification_waiting" => {
                if run_verification_status != "passed" {
                    return Err(anyhow!(
                        "manual verification gate requested before automated checks passed"
                    ));
                }
                let gate: ManualVerificationGate = serde_json::from_value(
                    payload
                        .get("gate")
                        .cloned()
                        .context("verification waiting event omitted gate")?,
                )
                .context("decode manual verification gate")?;
                if verification_policy.manual_gate.as_ref() != Some(&gate) {
                    return Err(anyhow!(
                        "manual verification gate does not match task policy"
                    ));
                }
                let gate_type = payload
                    .get("gate_type")
                    .and_then(Value::as_str)
                    .context("verification waiting event omitted gate_type")?;
                if gate.kind() != gate_type {
                    return Err(anyhow!("manual verification gate type mismatch"));
                }
                let verification_summary = payload
                    .get("verification_summary")
                    .and_then(Value::as_str)
                    .unwrap_or("automated verification passed");
                let completion_summary = payload
                    .get("completion_summary")
                    .and_then(Value::as_str)
                    .unwrap_or("Agent completed pending approval");
                sqlx::query(
                    r#"
                    INSERT INTO verification_requests
                        (run_id, corp_id, task_id, gate_type, gate, status)
                    VALUES ($1, $2, $3, $4, $5, 'pending')
                    "#,
                )
                .bind(run_id)
                .bind(corp_id)
                .bind(task_id)
                .bind(gate_type)
                .bind(serde_json::to_value(&gate)?)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE runs SET status = 'waiting_for_approval', verification_status = 'waiting_for_approval', verification_summary = $1, summary = $2, updated_at = now() WHERE id = $3",
                )
                .bind(verification_summary)
                .bind(completion_summary)
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE tasks SET status = 'awaiting_approval', verification_status = 'waiting_for_approval', updated_at = now() WHERE id = $1",
                )
                .bind(task_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE agents SET status = 'reviewing', station = 'review' WHERE id = $1",
                )
                .bind(agent_id)
                .execute(&mut *tx)
                .await?;
            }
            "run.verification_failed" => {
                let evidence_statuses: Vec<String> = sqlx::query_scalar(
                    "SELECT status FROM verification_evidence WHERE run_id = $1 ORDER BY check_index",
                )
                .bind(run_id)
                .fetch_all(&mut *tx)
                .await?;
                if evidence_statuses.len() != verification_policy.checks.len()
                    || !evidence_statuses.iter().any(|status| status == "failed")
                {
                    return Err(anyhow!(
                        "verification failure does not have complete failing evidence"
                    ));
                }
                let summary = payload
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("verification policy failed");
                sqlx::query(
                    "UPDATE runs SET status = 'failed', verification_status = 'failed', verification_summary = $1, summary = $1, updated_at = now() WHERE id = $2",
                )
                .bind(summary)
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE tasks SET status = 'verification_failed', verification_status = 'failed', updated_at = now() WHERE id = $1",
                )
                .bind(task_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE missions SET status = 'failed', updated_at = now() WHERE id = $1 AND status IN ('ready', 'running')",
                )
                .bind(mission_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE agents SET status = 'idle', station = NULL, current_run_id = NULL WHERE id = $1",
                )
                .bind(agent_id)
                .execute(&mut *tx)
                .await?;
            }
            "run.teardown_uncertain" => {
                let process_state = payload
                    .get("provider_process_state")
                    .and_then(Value::as_str);
                if process_state != Some("unverified") {
                    return Err(anyhow!(
                        "teardown uncertainty must report an unverified provider process state"
                    ));
                }
                let detail = payload
                    .get("detail")
                    .and_then(Value::as_str)
                    .context("teardown uncertainty omitted detail")?;
                sqlx::query(
                    "UPDATE runs SET workspace_disposition = 'preserved', workspace_detail = $1, updated_at = now() WHERE id = $2",
                )
                .bind(detail)
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
            }
            "run.workspace_preserved" | "run.workspace_removed" => {
                let disposition = if event_type == "run.workspace_removed" {
                    "removed"
                } else {
                    "preserved"
                };
                let detail = payload
                    .get("detail")
                    .and_then(Value::as_str)
                    .unwrap_or("runner did not provide cleanup detail");
                sqlx::query(
                    r#"
                    UPDATE runs
                    SET workspace_disposition = $1,
                        workspace_detail = $2,
                        updated_at = now()
                    WHERE id = $3
                    "#,
                )
                .bind(disposition)
                .bind(detail)
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
            }
            "run.completed" => {
                if run_verification_status != "passed" {
                    return Err(anyhow!("run cannot complete before verification passes"));
                }
                if verification_policy.manual_gate.is_some() {
                    return Err(anyhow!(
                        "run cannot complete before its manual verification gate"
                    ));
                }
                let summary = payload
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or("Mission artifact completed");
                sqlx::query(
                    "UPDATE runs SET status = 'completed', summary = $1, updated_at = now() WHERE id = $2",
                )
                .bind(summary)
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE tasks SET status = 'completed', verification_status = 'passed', updated_at = now() WHERE id = $1",
                )
                .bind(task_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    r#"
                    UPDATE tasks child
                    SET status = 'ready', updated_at = now()
                    WHERE child.mission_id = $1 AND child.status = 'pending'
                      AND NOT EXISTS (
                        SELECT 1 FROM task_dependencies dependency
                        JOIN tasks parent ON parent.id = dependency.depends_on_task_id
                        WHERE dependency.task_id = child.id
                          AND parent.status <> 'completed'
                      )
                    "#,
                )
                .bind(mission_id)
                .execute(&mut *tx)
                .await?;
                let mission_complete: bool = sqlx::query_scalar(
                    "SELECT NOT EXISTS(SELECT 1 FROM tasks WHERE mission_id = $1 AND status <> 'completed')",
                )
                .bind(mission_id)
                .fetch_one(&mut *tx)
                .await?;
                sqlx::query(
                    r#"
                    UPDATE missions
                    SET status = $1, updated_at = now()
                    WHERE id = $2 AND status IN ('ready', 'running')
                    "#,
                )
                .bind(if mission_complete {
                    "completed"
                } else {
                    "running"
                })
                .bind(mission_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE agents SET status = 'idle', station = NULL, current_run_id = NULL WHERE id = $1",
                )
                .bind(agent_id)
                .execute(&mut *tx)
                .await?;
            }
            "run.failed" => {
                let summary = payload
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("Runner reported failure");
                sqlx::query(
                    "UPDATE runs SET status = 'failed', summary = $1, updated_at = now() WHERE id = $2",
                )
                .bind(summary)
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
                let retry =
                    should_retry_runner_failure(&breaker_stage, attempt_count, max_attempts);
                sqlx::query("UPDATE tasks SET status = $1, updated_at = now() WHERE id = $2")
                    .bind(if retry { "ready" } else { "failed" })
                    .bind(task_id)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query(
                    "UPDATE missions SET status = $1, updated_at = now() WHERE id = $2 AND status IN ('ready', 'running')",
                )
                .bind(if retry { "running" } else { "failed" })
                .bind(mission_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE agents SET status = 'idle', station = NULL, current_run_id = NULL WHERE id = $1",
                )
                .bind(agent_id)
                .execute(&mut *tx)
                .await?;
            }
            "run.cancelled" => {
                let summary = payload
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("Run cancelled by an authorized operator");
                sqlx::query(
                    "UPDATE runs SET status = 'cancelled', summary = $1, updated_at = now() WHERE id = $2",
                )
                .bind(summary)
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE tasks SET status = 'cancelled', updated_at = now() WHERE id = $1",
                )
                .bind(task_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE missions SET status = 'cancelled', updated_at = now() WHERE id = $1",
                )
                .bind(mission_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE agents SET status = 'idle', station = NULL, current_run_id = NULL WHERE id = $1",
                )
                .bind(agent_id)
                .execute(&mut *tx)
                .await?;
            }
            _ => {}
        }

        tx.commit().await?;
        Ok(Some(event))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn decide_action_approval(
        &self,
        corp_id: Uuid,
        approval_id: Uuid,
        actor_id: Uuid,
        approved: bool,
        note: &str,
        decision_key: Uuid,
    ) -> Result<ActionApprovalDecisionOutcome> {
        let note = note.trim();
        if note.len() > 1_000 {
            return Err(anyhow!(
                "approval decision note cannot exceed 1,000 characters"
            ));
        }
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            r#"
            SELECT approval.run_id, approval.room_id, approval.status, approval.expires_at,
                    approval.required_roles, approval.decision_key,
                    run.runner_id, run.breaker_stage
            FROM action_approvals approval
            JOIN runs run ON run.id = approval.run_id
            WHERE approval.id = $1 AND approval.corp_id = $2
            FOR UPDATE OF approval, run
            "#,
        )
        .bind(approval_id)
        .bind(corp_id)
        .fetch_one(&mut *tx)
        .await
        .context("action approval not found")?;
        let run_id: Uuid = row.get("run_id");
        let room_id: Uuid = row.get("room_id");
        let runner_id: String = row.get("runner_id");
        let status: String = row.get("status");
        let existing_key: Option<Uuid> = row.get("decision_key");
        let requested_status = if approved { "approved" } else { "rejected" };
        if status != "pending" {
            if existing_key == Some(decision_key) && status == requested_status {
                tx.commit().await?;
                return Ok(ActionApprovalDecisionOutcome {
                    approval_id,
                    run_id,
                    runner_id,
                    status,
                    effect_queued: false,
                    event: None,
                });
            }
            return Err(anyhow!("approval was already decided as {status}"));
        }
        let breaker_stage: String = row.get("breaker_stage");
        ensure_run_not_hard_blocked_tx(&mut tx, corp_id, run_id, &breaker_stage, "action approval")
            .await?;
        let expires_at: chrono::DateTime<Utc> = row.get("expires_at");
        if expires_at <= Utc::now() {
            tx.commit().await?;
            let _ = self.expire_action_approval(approval_id).await?;
            return Err(anyhow!("approval request expired"));
        }
        assert_room_membership_tx(&mut tx, corp_id, room_id, actor_id).await?;
        let actor = sqlx::query("SELECT kind, role FROM actors WHERE id = $1 AND corp_id = $2")
            .bind(actor_id)
            .bind(corp_id)
            .fetch_one(&mut *tx)
            .await?;
        let kind: String = actor.get("kind");
        let role: String = actor.get("role");
        let required_roles: Vec<String> = row.get("required_roles");
        if kind != "human" || !required_roles.iter().any(|required| required == &role) {
            return Err(anyhow!(
                "forbidden: actor role {role} cannot decide this approval"
            ));
        }

        sqlx::query(
            r#"
            UPDATE action_approvals
            SET status = $1, decided_by = $2, decision_note = $3,
                decision_key = $4, decided_at = now()
            WHERE id = $5
            "#,
        )
        .bind(requested_status)
        .bind(actor_id)
        .bind(note)
        .bind(decision_key)
        .bind(approval_id)
        .execute(&mut *tx)
        .await?;
        let command_id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO runner_commands
                (id, corp_id, runner_id, run_id, command_kind, payload, idempotency_key)
            VALUES ($1, $2, $3, $4, 'approval_decision', $5, $6)
            "#,
        )
        .bind(command_id)
        .bind(corp_id)
        .bind(&runner_id)
        .bind(run_id)
        .bind(json!({
            "approval_id": approval_id,
            "approved": approved,
            "note": note,
        }))
        .bind(format!("approval-decision:{approval_id}:{decision_key}"))
        .execute(&mut *tx)
        .await?;
        let event = append_event_tx(
            &mut tx,
            NewEvent::new(
                corp_id,
                Some(actor_id),
                "approval.decided",
                "approval",
                approval_id,
                format!("approval-decision-event:{approval_id}:{decision_key}"),
                json!({
                    "run_id": run_id,
                    "status": requested_status,
                    "decision_note": note,
                    "command_id": command_id,
                }),
            ),
        )
        .await?;
        tx.commit().await?;
        Ok(ActionApprovalDecisionOutcome {
            approval_id,
            run_id,
            runner_id,
            status: requested_status.to_owned(),
            effect_queued: true,
            event,
        })
    }

    pub async fn expire_action_approval(
        &self,
        approval_id: Uuid,
    ) -> Result<Option<ActionApprovalDecisionOutcome>> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            r#"
            SELECT approval.corp_id, approval.room_id, approval.mission_id,
                   approval.task_id, approval.run_id, approval.agent_id,
                   approval.status, approval.expires_at, run.runner_id
            FROM action_approvals approval
            JOIN runs run ON run.id = approval.run_id
            WHERE approval.id = $1
            FOR UPDATE OF approval, run
            "#,
        )
        .bind(approval_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            tx.commit().await?;
            return Ok(None);
        };
        let status: String = row.get("status");
        let expires_at: chrono::DateTime<Utc> = row.get("expires_at");
        if status != "pending" || expires_at > Utc::now() {
            tx.commit().await?;
            return Ok(None);
        }
        let corp_id: Uuid = row.get("corp_id");
        let room_id: Uuid = row.get("room_id");
        let mission_id: Uuid = row.get("mission_id");
        let task_id: Uuid = row.get("task_id");
        let run_id: Uuid = row.get("run_id");
        let agent_id: Uuid = row.get("agent_id");
        let runner_id: String = row.get("runner_id");
        let reason = "Action approval expired before an authorized decision.";
        sqlx::query(
            "UPDATE action_approvals SET status = 'expired', decision_note = $1, decided_at = now() WHERE id = $2",
        )
        .bind(reason)
        .bind(approval_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE runs SET status = 'cancelled', summary = $1, updated_at = now() WHERE id = $2 AND status = 'waiting_for_approval'",
        )
        .bind(reason)
        .bind(run_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE tasks SET status = 'cancelled', updated_at = now() WHERE id = $1 AND status = 'awaiting_approval'",
        )
        .bind(task_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE missions SET status = 'cancelled', updated_at = now() WHERE id = $1 AND status = 'running'",
        )
        .bind(mission_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE agents SET status = 'idle', station = NULL, current_run_id = NULL WHERE id = $1 AND current_run_id = $2",
        )
        .bind(agent_id)
        .bind(run_id)
        .execute(&mut *tx)
        .await?;
        let command_id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO runner_commands
                (id, corp_id, runner_id, run_id, command_kind, payload, idempotency_key)
            VALUES ($1, $2, $3, $4, 'approval_decision', $5, $6)
            ON CONFLICT (corp_id, idempotency_key) DO NOTHING
            "#,
        )
        .bind(command_id)
        .bind(corp_id)
        .bind(&runner_id)
        .bind(run_id)
        .bind(json!({
            "approval_id": approval_id,
            "approved": false,
            "note": reason,
        }))
        .bind(format!("approval-expired:{approval_id}"))
        .execute(&mut *tx)
        .await?;
        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(room_id),
                correlation_id: Some(mission_id),
                ..NewEvent::new(
                    corp_id,
                    None,
                    "run.approval_expired",
                    "run",
                    run_id,
                    format!("approval-expired-event:{approval_id}"),
                    json!({
                        "approval_id": approval_id,
                        "reason": reason,
                        "command_id": command_id,
                    }),
                )
            },
        )
        .await?
        .context("approval expiry event unexpectedly existed")?;
        tx.commit().await?;
        Ok(Some(ActionApprovalDecisionOutcome {
            approval_id,
            run_id,
            runner_id,
            status: "expired".to_owned(),
            effect_queued: true,
            event: Some(event),
        }))
    }

    pub async fn pending_runner_commands(
        &self,
        runner_id: &str,
    ) -> Result<Vec<PendingRunnerCommand>> {
        Ok(sqlx::query(
            r#"
            SELECT id, runner_id, run_id, command_kind, payload
            FROM runner_commands
            WHERE runner_id = $1 AND status = 'pending'
            ORDER BY created_at, id
            LIMIT 100
            "#,
        )
        .bind(runner_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|row| PendingRunnerCommand {
            id: row.get("id"),
            runner_id: row.get("runner_id"),
            run_id: row.get("run_id"),
            command_kind: row.get("command_kind"),
            payload: row.get("payload"),
        })
        .collect())
    }

    pub async fn acknowledge_runner_command(
        &self,
        command_id: Uuid,
        runner_id: &str,
    ) -> Result<Option<DomainEvent>> {
        let mut tx = self.pool.begin().await?;
        let command = sqlx::query(
            r#"
            UPDATE runner_commands
            SET status = 'dispatched', dispatched_at = now()
            WHERE id = $1 AND runner_id = $2 AND status = 'pending'
            RETURNING corp_id, run_id
            "#,
        )
        .bind(command_id)
        .bind(runner_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(command) = command else {
            tx.commit().await?;
            return Ok(None);
        };
        let corp_id: Uuid = command.get("corp_id");
        let run_id: Uuid = command.get("run_id");
        let context = sqlx::query(
            r#"
            SELECT task.mission_id, mission.room_id
            FROM runs run
            JOIN tasks task ON task.id = run.task_id
            JOIN missions mission ON mission.id = task.mission_id
            WHERE run.id = $1 AND run.corp_id = $2
            "#,
        )
        .bind(run_id)
        .bind(corp_id)
        .fetch_one(&mut *tx)
        .await?;
        let mission_id: Uuid = context.get("mission_id");
        let room_id: Uuid = context.get("room_id");
        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(room_id),
                correlation_id: Some(mission_id),
                ..NewEvent::new(
                    corp_id,
                    None,
                    "runner.command_acknowledged",
                    "run",
                    run_id,
                    format!("runner-command-ack:{command_id}"),
                    json!({"command_id": command_id, "runner_id": runner_id}),
                )
            },
        )
        .await?
        .context("runner command acknowledgment event unexpectedly existed")?;
        tx.commit().await?;
        Ok(Some(event))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn set_budget_policy(
        &self,
        corp_id: Uuid,
        actor_id: Uuid,
        actor_tokens_per_24h: i64,
        actor_cost_microusd_per_24h: i64,
        corp_tokens_per_24h: i64,
        corp_cost_microusd_per_24h: i64,
        no_progress_event_limit: i32,
        repeated_tool_limit: i32,
    ) -> Result<DomainEvent> {
        let mut tx = self.pool.begin().await?;
        let role: Option<String> = sqlx::query_scalar(
            "SELECT role FROM actors WHERE id = $1 AND corp_id = $2 AND kind = 'human'",
        )
        .bind(actor_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?;
        if !matches!(role.as_deref(), Some("owner" | "admin")) {
            return Err(anyhow!(
                "forbidden: only a Corp owner or admin can change budget policy"
            ));
        }
        lock_factory_keys_tx(&mut tx, &budget_scope_lock_keys(corp_id, actor_id)).await?;
        if actor_tokens_per_24h <= 0
            || actor_cost_microusd_per_24h <= 0
            || corp_tokens_per_24h <= 0
            || corp_cost_microusd_per_24h <= 0
            || !(2..=100).contains(&no_progress_event_limit)
            || !(2..=100).contains(&repeated_tool_limit)
        {
            return Err(anyhow!("budget policy values are out of range"));
        }
        sqlx::query(
            r#"
            INSERT INTO corp_budget_policies
                (corp_id, actor_tokens_per_24h, actor_cost_microusd_per_24h,
                 corp_tokens_per_24h, corp_cost_microusd_per_24h,
                 no_progress_event_limit, repeated_tool_limit)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (corp_id) DO UPDATE
            SET actor_tokens_per_24h = EXCLUDED.actor_tokens_per_24h,
                actor_cost_microusd_per_24h = EXCLUDED.actor_cost_microusd_per_24h,
                corp_tokens_per_24h = EXCLUDED.corp_tokens_per_24h,
                corp_cost_microusd_per_24h = EXCLUDED.corp_cost_microusd_per_24h,
                no_progress_event_limit = EXCLUDED.no_progress_event_limit,
                repeated_tool_limit = EXCLUDED.repeated_tool_limit,
                updated_at = now()
            "#,
        )
        .bind(corp_id)
        .bind(actor_tokens_per_24h)
        .bind(actor_cost_microusd_per_24h)
        .bind(corp_tokens_per_24h)
        .bind(corp_cost_microusd_per_24h)
        .bind(no_progress_event_limit)
        .bind(repeated_tool_limit)
        .execute(&mut *tx)
        .await?;
        let event = append_event_tx(
            &mut tx,
            NewEvent::new(
                corp_id,
                Some(actor_id),
                "budget.policy_updated",
                "corp",
                corp_id,
                format!("budget-policy-updated:{}", Uuid::new_v4()),
                json!({
                    "actor_tokens_per_24h": actor_tokens_per_24h,
                    "actor_cost_microusd_per_24h": actor_cost_microusd_per_24h,
                    "corp_tokens_per_24h": corp_tokens_per_24h,
                    "corp_cost_microusd_per_24h": corp_cost_microusd_per_24h,
                    "no_progress_event_limit": no_progress_event_limit,
                    "repeated_tool_limit": repeated_tool_limit,
                }),
            ),
        )
        .await?
        .context("budget policy event unexpectedly existed")?;
        tx.commit().await?;
        Ok(event)
    }

    pub async fn evaluate_circuit_breaker(
        &self,
        corp_id: Uuid,
        run_id: Uuid,
    ) -> Result<CircuitBreakerOutcome> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            r#"
            SELECT run.task_id, run.runner_id, run.breaker_stage,
                   run.input_tokens + run.output_tokens AS run_tokens,
                   run.cost_microusd AS run_cost,
                   run.budget_tokens_limit AS run_token_limit,
                   run.budget_cost_microusd_limit AS run_cost_limit,
                   run.no_progress_events, run.repeated_tool_count,
                   task.mission_id, mission.room_id, mission.requested_by,
                   mission.budget_tokens AS mission_token_limit,
                   mission.budget_cost_microusd AS mission_cost_limit,
                   COALESCE(policy.actor_tokens_per_24h, 4000000) AS actor_token_limit,
                   COALESCE(policy.actor_cost_microusd_per_24h, 10000000) AS actor_cost_limit,
                   COALESCE(policy.corp_tokens_per_24h, 20000000) AS corp_token_limit,
                   COALESCE(policy.corp_cost_microusd_per_24h, 100000000) AS corp_cost_limit,
                   COALESCE(policy.no_progress_event_limit, 8) AS no_progress_limit,
                   COALESCE(policy.repeated_tool_limit, 5) AS repeated_tool_limit
            FROM runs run
            JOIN tasks task ON task.id = run.task_id
            JOIN missions mission ON mission.id = task.mission_id
            LEFT JOIN corp_budget_policies policy ON policy.corp_id = run.corp_id
            WHERE run.id = $1 AND run.corp_id = $2
              AND run.status IN ('starting', 'running', 'waiting_for_input',
                                 'waiting_for_approval', 'verifying')
            FOR UPDATE OF run
            "#,
        )
        .bind(run_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            tx.commit().await?;
            return Ok(CircuitBreakerOutcome {
                command: None,
                event: None,
            });
        };
        let task_id: Uuid = row.get("task_id");
        let mission_id: Uuid = row.get("mission_id");
        let room_id: Uuid = row.get("room_id");
        let requester: Uuid = row.get("requested_by");
        let runner_id: String = row.get("runner_id");
        let current_stage: String = row.get("breaker_stage");

        let mission_usage = sqlx::query(
            r#"
            SELECT COALESCE(SUM(run.input_tokens + run.output_tokens), 0)::BIGINT AS tokens,
                   COALESCE(SUM(run.cost_microusd), 0)::BIGINT AS cost
            FROM runs run
            JOIN tasks task ON task.id = run.task_id
            WHERE task.mission_id = $1
            "#,
        )
        .bind(mission_id)
        .fetch_one(&mut *tx)
        .await?;
        let actor_usage = sqlx::query(
            r#"
            SELECT COALESCE(SUM(run.input_tokens + run.output_tokens), 0)::BIGINT AS tokens,
                   COALESCE(SUM(run.cost_microusd), 0)::BIGINT AS cost
            FROM runs run
            JOIN tasks task ON task.id = run.task_id
            JOIN missions mission ON mission.id = task.mission_id
            WHERE mission.corp_id = $1 AND mission.requested_by = $2
              AND run.created_at >= now() - interval '24 hours'
            "#,
        )
        .bind(corp_id)
        .bind(requester)
        .fetch_one(&mut *tx)
        .await?;
        let corp_usage = sqlx::query(
            r#"
            SELECT COALESCE(SUM(input_tokens + output_tokens), 0)::BIGINT AS tokens,
                   COALESCE(SUM(cost_microusd), 0)::BIGINT AS cost
            FROM runs
            WHERE corp_id = $1 AND created_at >= now() - interval '24 hours'
            "#,
        )
        .bind(corp_id)
        .fetch_one(&mut *tx)
        .await?;

        let inputs = [
            (
                "run_tokens",
                row.get::<i64, _>("run_tokens"),
                row.get::<i64, _>("run_token_limit"),
            ),
            (
                "run_cost",
                row.get::<i64, _>("run_cost"),
                row.get::<i64, _>("run_cost_limit"),
            ),
            (
                "mission_tokens",
                mission_usage.get::<i64, _>("tokens"),
                row.get::<i64, _>("mission_token_limit"),
            ),
            (
                "mission_cost",
                mission_usage.get::<i64, _>("cost"),
                row.get::<i64, _>("mission_cost_limit"),
            ),
            (
                "actor_tokens_24h",
                actor_usage.get::<i64, _>("tokens"),
                row.get::<i64, _>("actor_token_limit"),
            ),
            (
                "actor_cost_24h",
                actor_usage.get::<i64, _>("cost"),
                row.get::<i64, _>("actor_cost_limit"),
            ),
            (
                "corp_tokens_24h",
                corp_usage.get::<i64, _>("tokens"),
                row.get::<i64, _>("corp_token_limit"),
            ),
            (
                "corp_cost_24h",
                corp_usage.get::<i64, _>("cost"),
                row.get::<i64, _>("corp_cost_limit"),
            ),
            (
                "no_progress",
                i64::from(row.get::<i32, _>("no_progress_events")),
                i64::from(row.get::<i32, _>("no_progress_limit")),
            ),
            (
                "repeated_tool",
                i64::from(row.get::<i32, _>("repeated_tool_count")),
                i64::from(row.get::<i32, _>("repeated_tool_limit")),
            ),
        ];
        let Some((stage, reason, used, limit)) = strongest_breaker_stage(&inputs) else {
            tx.commit().await?;
            return Ok(CircuitBreakerOutcome {
                command: None,
                event: None,
            });
        };
        if breaker_rank(stage) <= breaker_rank(&current_stage) {
            tx.commit().await?;
            return Ok(CircuitBreakerOutcome {
                command: None,
                event: None,
            });
        }
        sqlx::query("UPDATE runs SET breaker_stage = $1, updated_at = now() WHERE id = $2")
            .bind(stage)
            .bind(run_id)
            .execute(&mut *tx)
            .await?;
        if stage == "suspend" {
            sqlx::query(
                "UPDATE runs SET status = 'waiting_for_input' WHERE id = $1 AND status = 'running'",
            )
            .bind(run_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query("UPDATE tasks SET status = 'blocked' WHERE id = $1 AND status = 'running'")
                .bind(task_id)
                .execute(&mut *tx)
                .await?;
        }
        let incident_id = Uuid::new_v4();
        let input = json!({"metric": reason, "used": used, "limit": limit});
        sqlx::query(
            r#"
            INSERT INTO circuit_breaker_incidents
                (id, corp_id, mission_id, task_id, run_id, stage, reason, input)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
        )
        .bind(incident_id)
        .bind(corp_id)
        .bind(mission_id)
        .bind(task_id)
        .bind(run_id)
        .bind(stage)
        .bind(format!("{reason} reached {used} of {limit}"))
        .bind(input.clone())
        .execute(&mut *tx)
        .await?;
        let command_id = Uuid::new_v4();
        let command = PendingRunnerCommand {
            id: command_id,
            runner_id: runner_id.clone(),
            run_id,
            command_kind: "circuit_breaker".to_owned(),
            payload: json!({
                "stage": stage,
                "reason": format!("{reason} reached {used} of {limit}"),
            }),
        };
        sqlx::query(
            r#"
            INSERT INTO runner_commands
                (id, corp_id, runner_id, run_id, command_kind, payload, idempotency_key)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
        )
        .bind(command.id)
        .bind(corp_id)
        .bind(&command.runner_id)
        .bind(run_id)
        .bind(&command.command_kind)
        .bind(&command.payload)
        .bind(format!("breaker:{run_id}:{stage}"))
        .execute(&mut *tx)
        .await?;
        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(room_id),
                correlation_id: Some(mission_id),
                ..NewEvent::new(
                    corp_id,
                    None,
                    "run.breaker_transition",
                    "run",
                    run_id,
                    format!("breaker-event:{run_id}:{stage}"),
                    json!({
                        "stage": stage,
                        "reason": format!("{reason} reached {used} of {limit}"),
                        "input": input,
                        "command_id": command_id,
                    }),
                )
            },
        )
        .await?;
        tx.commit().await?;
        Ok(CircuitBreakerOutcome {
            command: Some(command),
            event,
        })
    }

    pub async fn decide_verification(
        &self,
        corp_id: Uuid,
        run_id: Uuid,
        actor_id: Uuid,
        approved: bool,
        note: &str,
    ) -> Result<VerificationDecisionOutcome> {
        let note = note.trim();
        if note.len() > 1_000 {
            return Err(anyhow!(
                "verification decision note cannot exceed 1,000 characters"
            ));
        }
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            r#"
            SELECT request.task_id, request.gate_type, request.gate,
                   request.status AS request_status,
                   task.mission_id, task.status AS task_status,
                   mission.room_id, mission.requested_by, mission.status AS mission_status,
                    run.status AS run_status, run.breaker_stage, run.agent_id,
                   producer.actor_id AS producer_actor_id
            FROM verification_requests request
            JOIN runs run ON run.id = request.run_id
            JOIN tasks task ON task.id = request.task_id
            JOIN missions mission ON mission.id = task.mission_id
            JOIN agents producer ON producer.id = run.agent_id
            WHERE request.run_id = $1 AND request.corp_id = $2
            FOR UPDATE OF request, run, task, mission
            "#,
        )
        .bind(run_id)
        .bind(corp_id)
        .fetch_one(&mut *tx)
        .await
        .context("pending verification request not found")?;
        let task_id: Uuid = row.get("task_id");
        let mission_id: Uuid = row.get("mission_id");
        let room_id: Uuid = row.get("room_id");
        assert_room_membership_tx(&mut tx, corp_id, room_id, actor_id).await?;
        let actor = sqlx::query("SELECT kind, role FROM actors WHERE id = $1 AND corp_id = $2")
            .bind(actor_id)
            .bind(corp_id)
            .fetch_one(&mut *tx)
            .await
            .context("verification actor not found")?;
        let actor_kind: String = actor.get("kind");
        let actor_role: String = actor.get("role");
        if actor_kind != "human" {
            return Err(anyhow!(
                "forbidden: manual verification decisions require a human actor"
            ));
        }
        let gate: ManualVerificationGate =
            serde_json::from_value(row.get("gate")).context("decode verification gate")?;
        let roles = match &gate {
            ManualVerificationGate::HumanApproval { roles }
            | ManualVerificationGate::IndependentReview { roles, .. } => roles,
        };
        if !roles.iter().any(|role| role == &actor_role) {
            return Err(anyhow!(
                "forbidden: actor role {actor_role} cannot decide this verification"
            ));
        }
        if let ManualVerificationGate::IndependentReview {
            exclude_requester, ..
        } = &gate
        {
            let requester: Uuid = row.get("requested_by");
            let producer_actor_id: Uuid = row.get("producer_actor_id");
            if (*exclude_requester && actor_id == requester) || actor_id == producer_actor_id {
                return Err(anyhow!(
                    "forbidden: independent review requires a different actor"
                ));
            }
        }
        let request_status: String = row.get("request_status");
        let run_status: String = row.get("run_status");
        let task_status: String = row.get("task_status");
        if request_status != "pending"
            || run_status != "waiting_for_approval"
            || task_status != "awaiting_approval"
        {
            return Err(anyhow!("verification request is no longer pending"));
        }
        let breaker_stage: String = row.get("breaker_stage");
        ensure_run_not_hard_blocked_tx(
            &mut tx,
            corp_id,
            run_id,
            &breaker_stage,
            "verification decision",
        )
        .await?;
        let mission_status: String = row.get("mission_status");
        if matches!(
            mission_status.as_str(),
            "completed" | "failed" | "cancelled"
        ) {
            return Err(anyhow!("mission is already {mission_status}"));
        }
        let agent_id: Uuid = row.get("agent_id");
        let decision_status = if approved { "approved" } else { "rejected" };
        sqlx::query(
            r#"
            UPDATE verification_requests
            SET status = $1, decided_by = $2, decision_note = $3, decided_at = now()
            WHERE run_id = $4
            "#,
        )
        .bind(decision_status)
        .bind(actor_id)
        .bind((!note.is_empty()).then_some(note))
        .bind(run_id)
        .execute(&mut *tx)
        .await?;

        if approved {
            sqlx::query(
                "UPDATE runs SET status = 'completed', verification_status = 'passed', updated_at = now() WHERE id = $1",
            )
            .bind(run_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE tasks SET status = 'completed', verification_status = 'passed', updated_at = now() WHERE id = $1",
            )
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                r#"
                UPDATE tasks child
                SET status = 'ready', updated_at = now()
                WHERE child.mission_id = $1 AND child.status = 'pending'
                  AND NOT EXISTS (
                    SELECT 1 FROM task_dependencies dependency
                    JOIN tasks parent ON parent.id = dependency.depends_on_task_id
                    WHERE dependency.task_id = child.id
                      AND parent.status <> 'completed'
                  )
                "#,
            )
            .bind(mission_id)
            .execute(&mut *tx)
            .await?;
            let mission_complete: bool = sqlx::query_scalar(
                "SELECT NOT EXISTS(SELECT 1 FROM tasks WHERE mission_id = $1 AND status <> 'completed')",
            )
            .bind(mission_id)
            .fetch_one(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE missions SET status = $1, updated_at = now() WHERE id = $2 AND status IN ('ready', 'running')",
            )
            .bind(if mission_complete {
                "completed"
            } else {
                "running"
            })
            .bind(mission_id)
            .execute(&mut *tx)
            .await?;
        } else {
            let rejection = if note.is_empty() {
                "verification rejected by an authorized reviewer"
            } else {
                note
            };
            sqlx::query(
                "UPDATE runs SET status = 'failed', verification_status = 'failed', verification_summary = $1, summary = $1, updated_at = now() WHERE id = $2",
            )
            .bind(rejection)
            .bind(run_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE tasks SET status = 'verification_failed', verification_status = 'failed', updated_at = now() WHERE id = $1",
            )
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE missions SET status = 'failed', updated_at = now() WHERE id = $1 AND status IN ('ready', 'running')",
            )
            .bind(mission_id)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "UPDATE agents SET status = 'idle', station = NULL, current_run_id = NULL WHERE id = $1",
        )
        .bind(agent_id)
        .execute(&mut *tx)
        .await?;
        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(room_id),
                correlation_id: Some(mission_id),
                ..NewEvent::new(
                    corp_id,
                    Some(actor_id),
                    if approved {
                        "verification.approved"
                    } else {
                        "verification.rejected"
                    },
                    "run",
                    run_id,
                    format!("verification-decision:{run_id}:{decision_status}"),
                    json!({
                        "task_id": task_id,
                        "gate_type": row.get::<String, _>("gate_type"),
                        "status": decision_status,
                        "note": note
                    }),
                )
            },
        )
        .await?
        .context("verification decision event unexpectedly existed")?;
        tx.commit().await?;
        Ok(VerificationDecisionOutcome {
            run_id,
            corp_id,
            mission_id,
            status: decision_status.to_owned(),
            event,
        })
    }

    pub async fn acquire_lease(
        &self,
        corp_id: Uuid,
        agent_id: Uuid,
        actor_id: Uuid,
    ) -> Result<LeaseOutcome> {
        let mut tx = self.pool.begin().await?;
        assert_actor_agent_scope_tx(&mut tx, corp_id, actor_id, agent_id).await?;
        let token = Uuid::new_v4();
        let expires_at = Utc::now() + Duration::minutes(5);
        let acquired_row = sqlx::query(
            r#"
            INSERT INTO control_leases (agent_id, corp_id, actor_id, token, expires_at)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (agent_id) DO UPDATE
            SET corp_id = EXCLUDED.corp_id,
                actor_id = EXCLUDED.actor_id,
                token = EXCLUDED.token,
                expires_at = EXCLUDED.expires_at
            WHERE control_leases.expires_at <= now()
               OR control_leases.actor_id = EXCLUDED.actor_id
            RETURNING agent_id, corp_id, actor_id, token, expires_at
            "#,
        )
        .bind(agent_id)
        .bind(corp_id)
        .bind(actor_id)
        .bind(token)
        .bind(expires_at)
        .fetch_optional(&mut *tx)
        .await?;

        let (acquired, lease) = if let Some(row) = acquired_row {
            (true, map_lease(row))
        } else {
            let row = sqlx::query(
                "SELECT agent_id, corp_id, actor_id, token, expires_at FROM control_leases WHERE agent_id = $1",
            )
            .bind(agent_id)
            .fetch_one(&mut *tx)
            .await?;
            (false, map_lease(row))
        };

        let event = if acquired {
            append_event_tx(
                &mut tx,
                NewEvent::new(
                    corp_id,
                    Some(actor_id),
                    "control.lease_acquired",
                    "agent",
                    agent_id,
                    format!("lease-acquire:{agent_id}:{}", Uuid::new_v4()),
                    json!({
                        "actor_id": actor_id,
                        "expires_at": expires_at
                    }),
                ),
            )
            .await?
        } else {
            None
        };
        tx.commit().await?;
        Ok(LeaseOutcome {
            acquired,
            lease,
            event,
        })
    }

    pub async fn release_lease(
        &self,
        corp_id: Uuid,
        agent_id: Uuid,
        actor_id: Uuid,
        token: Uuid,
    ) -> Result<LeaseMutationOutcome> {
        let mut tx = self.pool.begin().await?;
        assert_actor_agent_scope_tx(&mut tx, corp_id, actor_id, agent_id).await?;
        let released = sqlx::query(
            r#"
            DELETE FROM control_leases
            WHERE agent_id = $1 AND corp_id = $2 AND actor_id = $3
              AND token = $4 AND expires_at > now()
            RETURNING agent_id
            "#,
        )
        .bind(agent_id)
        .bind(corp_id)
        .bind(actor_id)
        .bind(token)
        .fetch_optional(&mut *tx)
        .await?;
        if released.is_none() {
            return Err(anyhow!("stale or unauthorized control lease"));
        }

        let event = append_event_tx(
            &mut tx,
            NewEvent::new(
                corp_id,
                Some(actor_id),
                "control.lease_released",
                "agent",
                agent_id,
                format!("lease-release:{agent_id}:{}", Uuid::new_v4()),
                json!({"actor_id": actor_id}),
            ),
        )
        .await?
        .context("lease release event unexpectedly existed")?;
        tx.commit().await?;
        Ok(LeaseMutationOutcome { lease: None, event })
    }

    pub async fn transfer_lease(
        &self,
        corp_id: Uuid,
        agent_id: Uuid,
        actor_id: Uuid,
        token: Uuid,
        to_actor_id: Uuid,
    ) -> Result<LeaseMutationOutcome> {
        let mut tx = self.pool.begin().await?;
        assert_actor_agent_scope_tx(&mut tx, corp_id, actor_id, agent_id).await?;
        assert_actor_scope_tx(&mut tx, corp_id, to_actor_id).await?;
        if actor_id == to_actor_id {
            return Err(anyhow!("lease transfer target must be another actor"));
        }

        let existing = sqlx::query(
            r#"
            SELECT agent_id, corp_id, actor_id, token, expires_at
            FROM control_leases
            WHERE agent_id = $1 AND corp_id = $2
            FOR UPDATE
            "#,
        )
        .bind(agent_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?
        .map(map_lease)
        .context("agent has no active control lease")?;
        if existing.actor_id != actor_id
            || existing.token != token
            || existing.expires_at <= Utc::now()
        {
            return Err(anyhow!("stale or unauthorized control lease"));
        }

        let new_token = Uuid::new_v4();
        let expires_at = Utc::now() + Duration::minutes(5);
        let row = sqlx::query(
            r#"
            UPDATE control_leases
            SET actor_id = $1, token = $2, expires_at = $3
            WHERE agent_id = $4 AND corp_id = $5
            RETURNING agent_id, corp_id, actor_id, token, expires_at
            "#,
        )
        .bind(to_actor_id)
        .bind(new_token)
        .bind(expires_at)
        .bind(agent_id)
        .bind(corp_id)
        .fetch_one(&mut *tx)
        .await?;
        let lease = map_lease(row);
        let event = append_event_tx(
            &mut tx,
            NewEvent::new(
                corp_id,
                Some(actor_id),
                "control.lease_transferred",
                "agent",
                agent_id,
                format!("lease-transfer:{agent_id}:{}", Uuid::new_v4()),
                json!({
                    "from_actor_id": actor_id,
                    "to_actor_id": to_actor_id,
                    "expires_at": expires_at
                }),
            ),
        )
        .await?
        .context("lease transfer event unexpectedly existed")?;
        tx.commit().await?;
        Ok(LeaseMutationOutcome {
            lease: Some(lease),
            event,
        })
    }

    pub async fn queue_message(
        &self,
        corp_id: Uuid,
        agent_id: Uuid,
        actor_id: Uuid,
        lease_token: Option<Uuid>,
        text: &str,
    ) -> Result<MessageOutcome> {
        let text = text.trim();
        if text.is_empty() {
            return Err(anyhow!("message cannot be empty"));
        }
        if text.len() > 4_000 {
            return Err(anyhow!("message cannot exceed 4,000 characters"));
        }

        let mut tx = self.pool.begin().await?;
        assert_actor_agent_scope_tx(&mut tx, corp_id, actor_id, agent_id).await?;
        let active_lease = sqlx::query(
            r#"
            SELECT agent_id, corp_id, actor_id, token, expires_at
            FROM control_leases
            WHERE agent_id = $1 AND corp_id = $2
            FOR UPDATE
            "#,
        )
        .bind(agent_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?
        .map(map_lease)
        .filter(|lease| lease.expires_at > Utc::now());

        let holds_lease = match (&active_lease, lease_token) {
            (Some(lease), Some(supplied))
                if lease.actor_id == actor_id && lease.token == supplied =>
            {
                true
            }
            (Some(lease), None) if lease.actor_id != actor_id => false,
            (None, None) => false,
            _ => return Err(anyhow!("stale or unauthorized control lease token")),
        };

        let active_run = sqlx::query(
            r#"
            SELECT run.id, run.runner_id, agent.adapter
            FROM runs run
            JOIN agents agent ON agent.id = run.agent_id
            WHERE run.agent_id = $1 AND run.corp_id = $2
              AND run.status IN ('starting', 'running', 'waiting_for_input',
                                 'waiting_for_approval', 'verifying')
            ORDER BY run.created_at DESC LIMIT 1
            "#,
        )
        .bind(agent_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?;

        let adapter_supports_steer = active_run.as_ref().is_some_and(|row| {
            matches!(
                row.get::<String, _>("adapter").as_str(),
                "codex" | "github-copilot" | "fake-process"
            )
        });
        let delivery = if holds_lease && active_run.is_some() && adapter_supports_steer {
            "immediate"
        } else {
            "queued"
        };
        let message_id = Uuid::new_v4();
        let row = sqlx::query(
            r#"
            INSERT INTO queued_messages (id, corp_id, agent_id, actor_id, text, status)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, corp_id, agent_id, actor_id, text, status, created_at
            "#,
        )
        .bind(message_id)
        .bind(corp_id)
        .bind(agent_id)
        .bind(actor_id)
        .bind(text)
        .bind(delivery)
        .fetch_one(&mut *tx)
        .await?;
        let message = map_queued_message(row);
        let event_type = if delivery == "immediate" {
            "control.message_accepted"
        } else {
            "control.message_queued"
        };
        let event = append_event_tx(
            &mut tx,
            NewEvent::new(
                corp_id,
                Some(actor_id),
                event_type,
                "agent",
                agent_id,
                format!("control-message:{message_id}"),
                json!({
                    "message_id": message_id,
                    "delivery": delivery,
                    "text": text
                }),
            ),
        )
        .await?
        .context("control message event unexpectedly existed")?;
        tx.commit().await?;

        let (run_id, runner_id) = active_run
            .map(|row| (Some(row.get("id")), Some(row.get("runner_id"))))
            .unwrap_or((None, None));
        Ok(MessageOutcome {
            message,
            delivery: delivery.to_owned(),
            run_id,
            runner_id,
            event,
        })
    }

    pub async fn requeue_control_message(
        &self,
        corp_id: Uuid,
        message_id: Uuid,
        reason: &str,
    ) -> Result<DomainEvent> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            r#"
            UPDATE queued_messages
            SET status = 'queued', run_id = NULL, delivered_at = NULL
            WHERE id = $1 AND corp_id = $2
            RETURNING agent_id, actor_id
            "#,
        )
        .bind(message_id)
        .bind(corp_id)
        .fetch_one(&mut *tx)
        .await
        .context("control message not found for requeue")?;
        let agent_id: Uuid = row.get("agent_id");
        let actor_id: Uuid = row.get("actor_id");
        let event = append_event_tx(
            &mut tx,
            NewEvent::new(
                corp_id,
                Some(actor_id),
                "control.message_requeued",
                "agent",
                agent_id,
                format!("control-message-requeued:{message_id}"),
                json!({"message_id": message_id, "reason": reason}),
            ),
        )
        .await?
        .context("control message requeue event unexpectedly existed")?;
        tx.commit().await?;
        Ok(event)
    }

    pub async fn create_room_message(
        &self,
        input: NewRoomMessageInput,
    ) -> Result<RoomMessageOutcome> {
        let NewRoomMessageInput {
            corp_id,
            room_id,
            actor_id,
            body,
            reply_to_id,
            mentions,
            link,
        } = input;
        let body = body.trim();
        if body.is_empty() {
            return Err(anyhow!("room message cannot be empty"));
        }
        if body.len() > 4_000 {
            return Err(anyhow!("room message cannot exceed 4,000 characters"));
        }

        let mut tx = self.pool.begin().await?;
        assert_room_membership_tx(&mut tx, corp_id, room_id, actor_id).await?;

        let mut seen_mentions = HashSet::new();
        let mentions = mentions
            .into_iter()
            .filter(|mentioned| seen_mentions.insert(*mentioned))
            .collect::<Vec<_>>();
        if mentions.len() > 20 {
            return Err(anyhow!("room message cannot mention more than 20 actors"));
        }
        for mentioned in &mentions {
            assert_room_membership_tx(&mut tx, corp_id, room_id, *mentioned).await?;
        }

        let (thread_root_id, reply_to_id) = if let Some(reply_to_id) = reply_to_id {
            let row = sqlx::query(
                r#"
                SELECT id, thread_root_id
                FROM room_messages
                WHERE id = $1 AND corp_id = $2 AND room_id = $3
                "#,
            )
            .bind(reply_to_id)
            .bind(corp_id)
            .bind(room_id)
            .fetch_optional(&mut *tx)
            .await?
            .context("reply target is not visible in this room")?;
            let root = row
                .try_get::<Option<Uuid>, _>("thread_root_id")?
                .unwrap_or_else(|| row.get("id"));
            (Some(root), Some(reply_to_id))
        } else {
            (None, None)
        };

        if let Some(link) = &link {
            validate_room_link_tx(&mut tx, corp_id, room_id, link).await?;
        }

        let message_id = Uuid::new_v4();
        let link_kind = link.as_ref().map(|link| link.kind.as_str());
        let link_id = link.as_ref().map(|link| link.id);
        let row = sqlx::query(
            r#"
            INSERT INTO room_messages
                (id, corp_id, room_id, actor_id, thread_root_id, reply_to_id,
                 body, mentions, link_kind, link_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            RETURNING id, corp_id, room_id, actor_id, thread_root_id, reply_to_id,
                      body, mentions, link_kind, link_id, created_at
            "#,
        )
        .bind(message_id)
        .bind(corp_id)
        .bind(room_id)
        .bind(actor_id)
        .bind(thread_root_id)
        .bind(reply_to_id)
        .bind(body)
        .bind(&mentions)
        .bind(link_kind)
        .bind(link_id)
        .fetch_one(&mut *tx)
        .await?;
        let message = map_room_message(row);
        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(room_id),
                ..NewEvent::new(
                    corp_id,
                    Some(actor_id),
                    "room.message_posted",
                    "room_message",
                    message_id,
                    format!("room-message:{message_id}"),
                    json!({
                        "message_id": message_id,
                        "thread_root_id": thread_root_id,
                        "reply_to_id": reply_to_id,
                        "mentions": mentions,
                        "link": link,
                        "body": body
                    }),
                )
            },
        )
        .await?
        .context("room message event unexpectedly existed")?;
        tx.commit().await?;
        Ok(RoomMessageOutcome { message, event })
    }

    pub async fn request_interrupt(
        &self,
        corp_id: Uuid,
        agent_id: Uuid,
        actor_id: Uuid,
        lease_token: Uuid,
        reason: &str,
    ) -> Result<StopRequestOutcome> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(anyhow!("interrupt reason cannot be empty"));
        }
        if reason.len() > 500 {
            return Err(anyhow!("interrupt reason cannot exceed 500 characters"));
        }

        let mut tx = self.pool.begin().await?;
        assert_actor_agent_scope_tx(&mut tx, corp_id, actor_id, agent_id).await?;
        let lease = sqlx::query(
            r#"
            SELECT agent_id, corp_id, actor_id, token, expires_at
            FROM control_leases
            WHERE agent_id = $1 AND corp_id = $2
            FOR UPDATE
            "#,
        )
        .bind(agent_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?
        .map(map_lease)
        .context("agent has no active control lease")?;
        if lease.actor_id != actor_id
            || lease.token != lease_token
            || lease.expires_at <= Utc::now()
        {
            return Err(anyhow!("stale or unauthorized control lease token"));
        }

        let row = sqlx::query(
            r#"
            SELECT r.id, r.runner_id, t.mission_id, m.room_id
            FROM runs r
            JOIN tasks t ON t.id = r.task_id
            JOIN missions m ON m.id = t.mission_id
            WHERE r.agent_id = $1 AND r.corp_id = $2
              AND r.status IN ('provisioning', 'starting', 'running',
                               'waiting_for_input', 'waiting_for_approval', 'verifying')
            ORDER BY r.created_at DESC
            LIMIT 1
            FOR UPDATE OF r
            "#,
        )
        .bind(agent_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?
        .context("agent has no active run to interrupt")?;
        let run_id: Uuid = row.get("id");
        let runner_id: String = row.get("runner_id");
        let mission_id: Uuid = row.get("mission_id");
        let room_id: Uuid = row.get("room_id");
        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(room_id),
                correlation_id: Some(mission_id),
                ..NewEvent::new(
                    corp_id,
                    Some(actor_id),
                    "run.interrupt_requested",
                    "run",
                    run_id,
                    format!("run-interrupt:{run_id}:{}", Uuid::new_v4()),
                    json!({"agent_id": agent_id, "reason": reason}),
                )
            },
        )
        .await?
        .context("run interrupt event unexpectedly existed")?;
        tx.commit().await?;
        Ok(StopRequestOutcome {
            run_id,
            runner_id,
            event,
        })
    }

    pub async fn request_emergency_stop(
        &self,
        corp_id: Uuid,
        agent_id: Uuid,
        actor_id: Uuid,
        reason: &str,
    ) -> Result<StopRequestOutcome> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(anyhow!("emergency stop reason cannot be empty"));
        }
        if reason.len() > 500 {
            return Err(anyhow!(
                "emergency stop reason cannot exceed 500 characters"
            ));
        }

        let mut tx = self.pool.begin().await?;
        assert_actor_agent_scope_tx(&mut tx, corp_id, actor_id, agent_id).await?;
        let role: String =
            sqlx::query_scalar("SELECT role FROM actors WHERE id = $1 AND corp_id = $2")
                .bind(actor_id)
                .bind(corp_id)
                .fetch_one(&mut *tx)
                .await?;
        if !matches!(role.as_str(), "owner" | "admin" | "manager") {
            return Err(anyhow!(
                "forbidden: actor role {role} cannot emergency-stop runs"
            ));
        }

        let row = sqlx::query(
            r#"
            SELECT r.id, r.runner_id, t.mission_id, m.room_id
            FROM runs r
            JOIN tasks t ON t.id = r.task_id
            JOIN missions m ON m.id = t.mission_id
            WHERE r.agent_id = $1 AND r.corp_id = $2
              AND r.status IN ('provisioning', 'starting', 'running',
                               'waiting_for_input', 'waiting_for_approval', 'verifying')
            ORDER BY r.created_at DESC
            LIMIT 1
            FOR UPDATE OF r
            "#,
        )
        .bind(agent_id)
        .bind(corp_id)
        .fetch_optional(&mut *tx)
        .await?
        .context("agent has no active run to stop")?;
        let run_id: Uuid = row.get("id");
        let runner_id: String = row.get("runner_id");
        let mission_id: Uuid = row.get("mission_id");
        let room_id: Uuid = row.get("room_id");
        let event = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(room_id),
                correlation_id: Some(mission_id),
                ..NewEvent::new(
                    corp_id,
                    Some(actor_id),
                    "run.stop_requested",
                    "run",
                    run_id,
                    format!("run-stop:{run_id}:{}", Uuid::new_v4()),
                    json!({"agent_id": agent_id, "reason": reason}),
                )
            },
        )
        .await?
        .context("run stop event unexpectedly existed")?;
        tx.commit().await?;
        Ok(StopRequestOutcome {
            run_id,
            runner_id,
            event,
        })
    }
}

fn format_task_prompt(
    mission_title: &str,
    task_title: &str,
    contract: &TaskContract,
    attempt: i32,
) -> String {
    let list = |values: &[String]| {
        values
            .iter()
            .map(|value| format!("- {value}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let secret_refs = if contract.secret_refs.is_empty() {
        "- none".to_owned()
    } else {
        contract
            .secret_refs
            .iter()
            .map(|reference| {
                format!(
                    "- {} via {} for {} (secret {})",
                    reference.env_name, reference.tool, reference.resource, reference.secret_id
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "MISSION: {mission_title}\n\
         TASK: {task_title}\n\
         ATTEMPT: {attempt}\n\
         SOURCE REPOSITORY: {}\n\
         SOURCE BASE REF: {}\n\
         SOURCE BASE COMMIT: {}\n\
         OBJECTIVE: {}\n\
         EXPECTED OUTPUT: {}\n\
         ACCEPTANCE TESTS:\n{}\n\
         ALLOWED TOOLS:\n{}\n\
         PROHIBITED ACTIONS:\n{}\n\
         REFERENCES:\n{}\n\
         WRITE SCOPE:\n{}\n\
         TOKEN BUDGET: {}\n\
         COST BUDGET (MICROUSD): {}\n\
         MODEL: {}\n\
         REASONING EFFORT: {}\n\
         DELIVERABLE FORM: {}\n\
         COMMIT AFTER VERIFICATION: {}\n\
         SECRET CAPABILITIES:\n{}\n\
         DEADLINE: {}\n\
         ESCALATION: {}",
        contract
            .source_repository
            .as_deref()
            .unwrap_or("runner default"),
        contract
            .source_base_ref
            .as_deref()
            .unwrap_or("runner default"),
        contract
            .source_base_commit
            .as_deref()
            .unwrap_or("runner default"),
        contract.objective,
        contract.expected_output,
        list(&contract.acceptance_tests),
        list(&contract.allowed_tools),
        list(&contract.prohibited_actions),
        if contract.references.is_empty() {
            "- none".to_owned()
        } else {
            list(&contract.references)
        },
        list(&contract.write_scope),
        contract.budget_tokens,
        contract.budget_cost_microusd,
        contract.model.as_deref().unwrap_or("provider default"),
        contract
            .reasoning_effort
            .as_deref()
            .unwrap_or("provider default"),
        contract
            .deliverable
            .as_ref()
            .map(|deliverable| deliverable.form.as_str())
            .unwrap_or("none"),
        contract
            .deliverable
            .as_ref()
            .is_some_and(|deliverable| deliverable.commit_after_verification),
        secret_refs,
        contract
            .deadline_at
            .as_ref()
            .map(|deadline| deadline.to_rfc3339())
            .unwrap_or_else(|| "none".to_owned()),
        contract.escalation,
    )
}

async fn reserve_queued_messages_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    agent_id: Uuid,
    run_id: Uuid,
) -> Result<Vec<QueuedRunMessage>> {
    let rows = sqlx::query(
        r#"
        WITH queued AS (
            SELECT id
            FROM queued_messages
            WHERE corp_id = $1 AND agent_id = $2 AND status = 'queued'
            ORDER BY created_at
            LIMIT 20
            FOR UPDATE SKIP LOCKED
        )
        UPDATE queued_messages message
        SET status = 'reserved', run_id = $3, delivered_at = NULL
        FROM queued
        WHERE message.id = queued.id
        RETURNING message.id, message.actor_id, message.text
        "#,
    )
    .bind(corp_id)
    .bind(agent_id)
    .bind(run_id)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| QueuedRunMessage {
            id: row.get("id"),
            actor_id: row.get("actor_id"),
            text: row.get("text"),
        })
        .collect())
}

fn append_queued_messages(prompt: &mut String, messages: &[QueuedRunMessage]) {
    if messages.is_empty() {
        return;
    }
    prompt.push_str(
        "\n\nQUEUED OPERATOR NOTES:\n\
         These durable notes were queued while the agent was off shift. Address each note in this turn.\n",
    );
    for message in messages {
        prompt.push_str(&format!(
            "- message {} from actor {}: {}\n",
            message.id, message.actor_id, message.text
        ));
    }
}

async fn lock_demo_tx(tx: &mut Transaction<'_, Postgres>) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(DEMO_ADVISORY_LOCK)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn active_runner_run_rows(
    tx: &mut Transaction<'_, Postgres>,
    runner_id: &str,
) -> Result<Vec<sqlx::postgres::PgRow>> {
    let rows = sqlx::query(
        r#"
        SELECT r.id AS run_id, r.corp_id, r.task_id, r.agent_id,
               t.mission_id, m.room_id
        FROM runs r
        JOIN tasks t ON t.id = r.task_id
        JOIN missions m ON m.id = t.mission_id
        WHERE r.runner_id = $1
          AND r.status IN ('provisioning', 'starting', 'running',
                           'waiting_for_input', 'waiting_for_approval', 'verifying')
        ORDER BY r.created_at
        FOR UPDATE OF r, t, m
        "#,
    )
    .bind(runner_id)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows)
}

async fn mark_runner_runs_lost_tx(
    tx: &mut Transaction<'_, Postgres>,
    runner_id: &str,
    excluded_run_ids: &[Uuid],
    created_before: Option<chrono::DateTime<Utc>>,
    reason: &str,
) -> Result<Vec<DomainEvent>> {
    let rows = sqlx::query(
        r#"
        SELECT r.id AS run_id, r.corp_id, r.task_id, r.agent_id,
               t.mission_id, m.room_id
        FROM runs r
        JOIN tasks t ON t.id = r.task_id
        JOIN missions m ON m.id = t.mission_id
        WHERE r.runner_id = $1
          AND r.status IN ('provisioning', 'starting', 'running',
                           'waiting_for_input', 'waiting_for_approval', 'verifying')
          AND (
            cardinality($2::uuid[]) = 0
            OR NOT (r.id = ANY($2::uuid[]))
          )
          AND ($3::timestamptz IS NULL OR r.created_at <= $3)
        ORDER BY r.created_at
        FOR UPDATE OF r, t, m
        "#,
    )
    .bind(runner_id)
    .bind(excluded_run_ids)
    .bind(created_before)
    .fetch_all(&mut **tx)
    .await?;

    let mut events = Vec::with_capacity(rows.len());
    for row in rows {
        let run_id: Uuid = row.get("run_id");
        let corp_id: Uuid = row.get("corp_id");
        let task_id: Uuid = row.get("task_id");
        let agent_id: Uuid = row.get("agent_id");
        let mission_id: Uuid = row.get("mission_id");
        let room_id: Uuid = row.get("room_id");
        sqlx::query(
            "UPDATE runs SET status = 'lost', summary = $1, updated_at = now() WHERE id = $2",
        )
        .bind(reason)
        .bind(run_id)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "UPDATE queued_messages SET status = 'queued', run_id = NULL WHERE run_id = $1 AND status = 'reserved'",
        )
        .bind(run_id)
        .execute(&mut **tx)
        .await?;
        sqlx::query("UPDATE tasks SET status = 'blocked', updated_at = now() WHERE id = $1")
            .bind(task_id)
            .execute(&mut **tx)
            .await?;
        sqlx::query(
            "UPDATE missions SET status = 'failed', updated_at = now() WHERE id = $1 AND status IN ('ready', 'running')",
        )
        .bind(mission_id)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "UPDATE agents SET status = 'idle', station = NULL, current_run_id = NULL WHERE id = $1",
        )
        .bind(agent_id)
        .execute(&mut **tx)
        .await?;
        if let Some(event) = append_event_tx(
            tx,
            NewEvent {
                room_id: Some(room_id),
                correlation_id: Some(mission_id),
                ..NewEvent::new(
                    corp_id,
                    None,
                    "run.lost",
                    "run",
                    run_id,
                    format!("run-lost:{run_id}"),
                    json!({"runner_id": runner_id, "reason": reason}),
                )
            },
        )
        .await?
        {
            events.push(event);
        }
    }
    Ok(events)
}

async fn create_mission_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    requested_by: Uuid,
    title: &str,
    description: &str,
    plan: &TaskGraphPlan,
) -> Result<(MissionPlanIds, Vec<DomainEvent>)> {
    let title = normalize_mission_title(title)?;
    let description = normalize_mission_description(description)?;

    let room_id: Uuid =
        sqlx::query_scalar("SELECT id FROM rooms WHERE corp_id = $1 ORDER BY created_at LIMIT 1")
            .bind(corp_id)
            .fetch_one(&mut **tx)
            .await
            .context("corp has no room")?;
    assert_room_membership_tx(tx, corp_id, room_id, requested_by).await?;

    let mission_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO missions
            (id, corp_id, room_id, requested_by, title, description, strategy,
             max_nodes, max_depth, original_budget_tokens,
             original_budget_cost_microusd, budget_tokens, budget_cost_microusd, status)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $10, $11, 'ready')
        "#,
    )
    .bind(mission_id)
    .bind(corp_id)
    .bind(room_id)
    .bind(requested_by)
    .bind(&title)
    .bind(&description)
    .bind(&plan.strategy)
    .bind(plan.max_nodes)
    .bind(plan.max_depth)
    .bind(plan.budget_tokens)
    .bind(plan.budget_cost_microusd)
    .execute(&mut **tx)
    .await?;

    let mission_event = append_event_tx(
        tx,
        NewEvent {
            room_id: Some(room_id),
            correlation_id: Some(mission_id),
            ..NewEvent::new(
                corp_id,
                Some(requested_by),
                "mission.created",
                "mission",
                mission_id,
                format!("mission:{mission_id}:created"),
                json!({
                        "title": title,
                        "description_bytes": description.len(),
                        "description_sha256": hex::encode(Sha256::digest(description.as_bytes())),
                        "specification_version": 1,
                        "strategy": plan.strategy,
                    "max_nodes": plan.max_nodes,
                    "max_depth": plan.max_depth,
                    "budget_tokens": plan.budget_tokens,
                    "budget_cost_microusd": plan.budget_cost_microusd,
                    "status": "ready"
                }),
            )
        },
    )
    .await?
    .context("mission created event unexpectedly existed")?;

    let mut task_ids = HashMap::new();
    for task in &plan.tasks {
        task_ids.insert(task.key.clone(), Uuid::new_v4());
    }
    let mut events = vec![mission_event.clone()];
    for task in &plan.tasks {
        let task_id = *task_ids
            .get(&task.key)
            .context("planned task id unexpectedly missing")?;
        let adapter: String =
            sqlx::query_scalar("SELECT adapter FROM agents WHERE id = $1 AND corp_id = $2")
                .bind(task.assigned_agent_id)
                .bind(corp_id)
                .fetch_one(&mut **tx)
                .await
                .with_context(|| {
                    format!(
                        "planned task {} references an agent outside the Corp",
                        task.key
                    )
                })?;
        if adapter != task.required_adapter {
            return Err(anyhow!(
                "planned task {} requires adapter {} but assigned agent uses {adapter}",
                task.key,
                task.required_adapter
            ));
        }
        let status = if task.depends_on.is_empty() {
            "ready"
        } else {
            "pending"
        };
        let mut contract = task.contract.clone();
        contract.normalize_for_adapter(&task.required_adapter);
        sqlx::query(
            r#"
            INSERT INTO tasks
                (id, mission_id, corp_id, title, objective, plan_key, contract,
                 contract_version, depth, max_attempts, attempt_count, required_adapter, status,
                 assigned_agent_id, verification_policy, verification_status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 1, $8, $9, 0, $10, $11, $12, $13, 'pending')
            "#,
        )
        .bind(task_id)
        .bind(mission_id)
        .bind(corp_id)
        .bind(&task.title)
        .bind(&task.contract.objective)
        .bind(&task.key)
        .bind(serde_json::to_value(&contract)?)
        .bind(task.depth)
        .bind(task.max_attempts)
        .bind(&task.required_adapter)
        .bind(status)
        .bind(task.assigned_agent_id)
        .bind(serde_json::to_value(&task.verification_policy)?)
        .execute(&mut **tx)
        .await?;

        let event = append_event_tx(
            tx,
            NewEvent {
                room_id: Some(room_id),
                correlation_id: Some(mission_id),
                causation_id: Some(mission_event.id),
                ..NewEvent::new(
                    corp_id,
                    Some(requested_by),
                    "task.created",
                    "task",
                    task_id,
                    format!("task:{task_id}:created"),
                    json!({
                        "plan_key": task.key,
                        "title": task.title,
                        "assigned_agent_id": task.assigned_agent_id,
                        "required_adapter": task.required_adapter,
                        "depends_on": task.depends_on,
                        "depth": task.depth,
                        "max_attempts": task.max_attempts,
                        "budget_tokens": task.contract.budget_tokens,
                        "status": status
                    }),
                )
            },
        )
        .await?
        .context("task created event unexpectedly existed")?;
        events.push(event);
    }

    for task in &plan.tasks {
        let task_id = *task_ids
            .get(&task.key)
            .context("planned task id unexpectedly missing")?;
        for dependency in &task.depends_on {
            let dependency_id = *task_ids
                .get(dependency)
                .with_context(|| format!("unknown planned dependency {dependency}"))?;
            sqlx::query(
                r#"
                INSERT INTO task_dependencies (task_id, depends_on_task_id)
                VALUES ($1, $2)
                "#,
            )
            .bind(task_id)
            .bind(dependency_id)
            .execute(&mut **tx)
            .await?;
        }
    }

    let planned_event = append_event_tx(
        tx,
        NewEvent {
            room_id: Some(room_id),
            correlation_id: Some(mission_id),
            causation_id: Some(mission_event.id),
            ..NewEvent::new(
                corp_id,
                Some(requested_by),
                "mission.planned",
                "mission",
                mission_id,
                format!("mission:{mission_id}:planned"),
                json!({
                    "strategy": plan.strategy,
                    "task_count": plan.tasks.len(),
                    "task_ids": plan.tasks.iter().filter_map(|task| {
                        task_ids.get(&task.key).copied()
                    }).collect::<Vec<_>>()
                }),
            )
        },
    )
    .await?
    .context("mission planned event unexpectedly existed")?;
    events.push(planned_event);
    Ok((
        MissionPlanIds {
            mission_id,
            task_ids: plan
                .tasks
                .iter()
                .filter_map(|task| task_ids.get(&task.key).copied())
                .collect(),
        },
        events,
    ))
}

fn normalize_mission_title(value: &str) -> Result<String> {
    normalize_factory_text(value, "mission title", 240)
}

fn normalize_mission_description(value: &str) -> Result<String> {
    let value = value.replace("\r\n", "\n").replace('\r', "\n");
    let value = value.trim();
    if value.len() > 100_000 {
        return Err(anyhow!("mission description cannot exceed 100000 bytes"));
    }
    if value
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
    {
        return Err(anyhow!(
            "mission description cannot contain unsupported control characters"
        ));
    }
    Ok(value.to_owned())
}

fn apply_factory_source_constraints(
    work_item: &FactoryWorkItem,
    plan: &mut TaskGraphPlan,
) -> Result<()> {
    let policy = work_item
        .policy
        .as_object()
        .context("factory policy snapshot must be a JSON object")?;
    let source_repository = format!(
        "{}/{}",
        work_item.source_repository_owner, work_item.source_repository_name
    );
    let source_base_ref = factory_policy_required_string(policy, "source_base_ref", 240)?;
    validate_factory_base_ref(&source_base_ref)?;
    let source_base_commit =
        factory_policy_required_string(policy, "source_base_commit", 64)?.to_ascii_lowercase();
    validate_factory_base_commit(&source_base_commit)?;
    for task in &mut plan.tasks {
        task.contract.source_repository = Some(source_repository.clone());
        task.contract.source_base_ref = Some(source_base_ref.clone());
        task.contract.source_base_commit = Some(source_base_commit.clone());
    }
    Ok(())
}

fn validate_factory_plan_against_policy(
    work_item: &FactoryWorkItem,
    plan: &TaskGraphPlan,
) -> Result<()> {
    let policy = work_item
        .policy
        .as_object()
        .context("factory policy snapshot must be a JSON object")?;
    if policy.get("schema_version").and_then(Value::as_i64) != Some(1) {
        return Err(anyhow!("factory policy schema_version must be 1"));
    }
    if policy.get("source_of_truth").and_then(Value::as_str) != Some("github_project") {
        return Err(anyhow!(
            "factory policy source_of_truth must be github_project"
        ));
    }
    if policy.get("auto_merge").and_then(Value::as_bool) != Some(false) {
        return Err(anyhow!("factory policy must explicitly disable auto_merge"));
    }
    let repositories = factory_policy_string_array(policy, "repository_allowlist")?;
    let source_repository = format!(
        "{}/{}",
        work_item.source_repository_owner, work_item.source_repository_name
    );
    if !repositories.contains(&source_repository) {
        return Err(anyhow!(
            "factory policy does not allow source repository {source_repository}"
        ));
    }
    let source_base_ref = factory_policy_required_string(policy, "source_base_ref", 240)?;
    let source_base_commit =
        factory_policy_required_string(policy, "source_base_commit", 64)?.to_ascii_lowercase();
    validate_factory_base_commit(&source_base_commit)?;
    if let Some(task) = plan.tasks.iter().find(|task| {
        task.contract.source_repository.as_deref() != Some(source_repository.as_str())
            || task.contract.source_base_ref.as_deref() != Some(source_base_ref.as_str())
            || task.contract.source_base_commit.as_deref() != Some(source_base_commit.as_str())
    }) {
        return Err(anyhow!(
            "factory task {} does not preserve the claimed repository, base ref, and immutable base commit",
            task.key
        ));
    }
    let adapters = factory_policy_string_array(policy, "adapter_allowlist")?;
    if let Some(task) = plan
        .tasks
        .iter()
        .find(|task| !adapters.contains(&task.required_adapter))
    {
        return Err(anyhow!(
            "factory policy does not allow adapter {} for task {}",
            task.required_adapter,
            task.key
        ));
    }
    let strategies = factory_policy_string_array(policy, "strategy_allowlist")?;
    if !strategies.contains(&plan.strategy) {
        return Err(anyhow!(
            "factory policy does not allow strategy {}",
            plan.strategy
        ));
    }
    let allowed_model = factory_policy_optional_string(policy, "model")?;
    match allowed_model.as_deref() {
        Some(allowed_model) => {
            if let Some(task) = plan
                .tasks
                .iter()
                .find(|task| task.contract.model.as_deref() != Some(allowed_model))
            {
                return Err(anyhow!(
                    "factory task {} must preserve policy model {allowed_model}; got {}",
                    task.key,
                    task.contract.model.as_deref().unwrap_or("provider default")
                ));
            }
        }
        None => {
            if let Some(task) = plan.tasks.iter().find(|task| task.contract.model.is_some()) {
                return Err(anyhow!(
                    "factory policy does not allow a model override for task {}",
                    task.key
                ));
            }
        }
    }
    let allowed_reasoning = factory_policy_optional_string(policy, "reasoning_effort")?;
    match allowed_reasoning.as_deref() {
        Some(allowed_reasoning) => {
            if let Some(task) = plan
                .tasks
                .iter()
                .find(|task| task.contract.reasoning_effort.as_deref() != Some(allowed_reasoning))
            {
                return Err(anyhow!(
                    "factory task {} must preserve policy reasoning effort {allowed_reasoning}; got {}",
                    task.key,
                    task.contract
                        .reasoning_effort
                        .as_deref()
                        .unwrap_or("provider default")
                ));
            }
        }
        None => {
            if let Some(task) = plan
                .tasks
                .iter()
                .find(|task| task.contract.reasoning_effort.is_some())
            {
                return Err(anyhow!(
                    "factory policy does not allow a reasoning override for task {}",
                    task.key
                ));
            }
        }
    }
    let write_scope = factory_policy_string_array(policy, "write_scope")?;
    if let Some((task, scope)) = plan.tasks.iter().find_map(|task| {
        task.contract
            .write_scope
            .iter()
            .find(|scope| !write_scope.contains(*scope))
            .map(|scope| (task, scope))
    }) {
        return Err(anyhow!(
            "factory policy does not allow write scope {scope} for task {}",
            task.key
        ));
    }
    let allowed_tools = factory_policy_string_array(policy, "allowed_tools")?;
    if let Some((task, tool)) = plan.tasks.iter().find_map(|task| {
        task.contract
            .allowed_tools
            .iter()
            .find(|tool| !allowed_tools.contains(*tool))
            .map(|tool| (task, tool))
    }) {
        return Err(anyhow!(
            "factory policy does not allow tool {tool} for task {}",
            task.key
        ));
    }
    let required_prohibitions = factory_policy_string_array(policy, "prohibited_actions")?;
    if let Some((task, prohibition)) = plan.tasks.iter().find_map(|task| {
        required_prohibitions
            .iter()
            .find(|prohibition| !task.contract.prohibited_actions.contains(*prohibition))
            .map(|prohibition| (task, prohibition))
    }) {
        return Err(anyhow!(
            "factory task {} omits policy prohibition {prohibition}",
            task.key
        ));
    }
    let allowed_secret_ids = factory_policy_string_list(policy, "secret_ids", true)?;
    if let Some((task, secret_id)) = plan.tasks.iter().find_map(|task| {
        task.contract
            .secret_refs
            .iter()
            .map(|secret| secret.secret_id.to_string())
            .find(|secret_id| !allowed_secret_ids.contains(secret_id))
            .map(|secret_id| (task, secret_id))
    }) {
        return Err(anyhow!(
            "factory policy does not allow secret {secret_id} for task {}",
            task.key
        ));
    }
    if let Some(policy_value) = policy
        .get("verification_policy")
        .filter(|value| !value.is_null())
    {
        let expected: VerificationPolicy = serde_json::from_value(policy_value.clone())
            .context("decode factory policy verification policy")?;
        let delivery_depth = plan.tasks.iter().map(|task| task.depth).max().unwrap_or(0);
        if let Some(task) = plan
            .tasks
            .iter()
            .find(|task| task.depth == delivery_depth && task.verification_policy != expected)
        {
            return Err(anyhow!(
                "factory task {} does not preserve the persisted delivery verification policy",
                task.key
            ));
        }
    }
    if policy.get("verification_required").and_then(Value::as_bool) != Some(true)
        || plan
            .tasks
            .iter()
            .any(|task| task.verification_policy.checks.is_empty())
    {
        return Err(anyhow!(
            "factory policy requires a non-empty verification policy for every task"
        ));
    }
    if let Some(task) = plan.tasks.iter().find(|task| {
        task.required_adapter != "fake-process" && task.verification_policy.manual_gate.is_none()
    }) {
        return Err(anyhow!(
            "factory task {} requires a manual verification gate for provider-backed execution",
            task.key
        ));
    }
    let budget_tokens = factory_policy_positive_i64(policy, "budget_tokens")?;
    if plan.budget_tokens > budget_tokens {
        return Err(anyhow!(
            "factory plan token budget {} exceeds policy {}",
            plan.budget_tokens,
            budget_tokens
        ));
    }
    let budget_cost_microusd = factory_policy_positive_i64(policy, "budget_cost_microusd")?;
    if plan.budget_cost_microusd > budget_cost_microusd {
        return Err(anyhow!(
            "factory plan cost budget {} exceeds policy {}",
            plan.budget_cost_microusd,
            budget_cost_microusd
        ));
    }
    Ok(())
}

fn factory_policy_string_array(
    policy: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Vec<String>> {
    factory_policy_string_list(policy, key, false)
}

fn factory_policy_string_list(
    policy: &serde_json::Map<String, Value>,
    key: &str,
    allow_empty: bool,
) -> Result<Vec<String>> {
    let values = policy
        .get(key)
        .and_then(Value::as_array)
        .with_context(|| format!("factory policy {key} must be an array"))?;
    if (!allow_empty && values.is_empty()) || values.len() > 64 {
        return Err(anyhow!(
            "factory policy {key} must contain between 1 and 64 entries"
        ));
    }
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.trim().is_empty() && value.len() <= 500)
                .map(str::to_owned)
                .with_context(|| format!("factory policy {key} contains an invalid entry"))
        })
        .collect()
}

fn factory_policy_optional_string(
    policy: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<String>> {
    match policy.get(key) {
        Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.trim().is_empty() && value.len() <= 128 => {
            Ok(Some(value.clone()))
        }
        _ => Err(anyhow!(
            "factory policy {key} must be null or a non-empty string"
        )),
    }
}

fn factory_policy_required_string(
    policy: &serde_json::Map<String, Value>,
    key: &str,
    max_len: usize,
) -> Result<String> {
    policy
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty() && value.len() <= max_len)
        .map(str::to_owned)
        .with_context(|| format!("factory policy {key} must be a non-empty string"))
}

fn validate_factory_base_ref(value: &str) -> Result<()> {
    if value.starts_with('-')
        || value.starts_with('/')
        || value.ends_with('/')
        || value.ends_with('.')
        || value.contains("..")
        || value.contains("@{")
        || value.chars().any(char::is_control)
        || value
            .chars()
            .any(|character| matches!(character, '\\' | ' ' | '~' | '^' | ':' | '?' | '*' | '['))
    {
        return Err(anyhow!("factory source_base_ref is not a safe Git ref"));
    }
    Ok(())
}

fn validate_factory_publication_base_ref(value: &str) -> Result<()> {
    validate_factory_base_ref(value)?;
    if value == "HEAD" {
        return Ok(());
    }
    if let Some(branch) = value.strip_prefix("refs/heads/") {
        if branch.is_empty() {
            return Err(anyhow!(
                "factory publication base ref must be HEAD or a branch ref"
            ));
        }
        return validate_factory_branch_ref(branch);
    }
    if value.starts_with("refs/") {
        return Err(anyhow!(
            "factory publication base ref must be HEAD or a branch ref"
        ));
    }
    validate_factory_branch_ref(value)
}

fn validate_factory_branch_ref(value: &str) -> Result<()> {
    validate_factory_base_ref(value)?;
    if value == "HEAD"
        || value.contains("//")
        || value
            .split('/')
            .any(|component| component.starts_with('.') || component.ends_with(".lock"))
    {
        return Err(anyhow!("publication branch is not a valid Git branch name"));
    }
    Ok(())
}

fn validate_factory_base_commit(value: &str) -> Result<()> {
    if !matches!(value.len(), 40 | 64) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(anyhow!(
            "factory source_base_commit must be a full 40- or 64-character hexadecimal Git object id"
        ));
    }
    Ok(())
}

fn factory_policy_positive_i64(policy: &serde_json::Map<String, Value>, key: &str) -> Result<i64> {
    policy
        .get(key)
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
        .with_context(|| format!("factory policy {key} must be a positive integer"))
}

fn normalize_factory_source(input: FactorySourceInput) -> Result<FactorySourceInput> {
    if input.project_number <= 0 {
        return Err(anyhow!("factory source project number must be positive"));
    }
    if input.issue_number <= 0 {
        return Err(anyhow!("factory source issue number must be positive"));
    }
    let project_owner =
        normalize_github_component(&input.project_owner, "source project owner", 100)?;
    let project_item_id =
        normalize_factory_identifier(&input.project_item_id, "source project item id", 160)?;
    let repository_owner =
        normalize_github_component(&input.repository_owner, "source repository owner", 100)?;
    let repository_name =
        normalize_github_component(&input.repository_name, "source repository name", 100)?;
    let issue_node_id =
        normalize_factory_identifier(&input.issue_node_id, "source issue node id", 160)?;
    let title = normalize_factory_text(&input.title, "source issue title", 240)?;
    let revision = normalize_factory_text(&input.revision, "source issue revision", 160)?;
    let issue_url = normalize_factory_text(&input.issue_url, "source issue URL", 500)?;
    let expected_url = format!(
        "https://github.com/{repository_owner}/{repository_name}/issues/{}",
        input.issue_number
    );
    if !issue_url
        .trim_end_matches('/')
        .eq_ignore_ascii_case(&expected_url)
    {
        return Err(anyhow!(
            "source issue URL must identify the declared GitHub repository and issue"
        ));
    }
    Ok(FactorySourceInput {
        project_owner,
        project_number: input.project_number,
        project_item_id,
        repository_owner,
        repository_name,
        issue_number: input.issue_number,
        issue_node_id,
        issue_url: expected_url,
        title,
        revision,
    })
}

fn normalize_github_component(value: &str, field: &str, max_len: usize) -> Result<String> {
    let value = normalize_factory_text(value, field, max_len)?;
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return Err(anyhow!(
            "{field} may contain only ASCII letters, numbers, hyphen, underscore, or period"
        ));
    }
    Ok(value.to_ascii_lowercase())
}

fn normalize_factory_identifier(value: &str, field: &str, max_len: usize) -> Result<String> {
    let value = normalize_factory_text(value, field, max_len)?;
    if value.chars().any(char::is_whitespace) {
        return Err(anyhow!("{field} cannot contain whitespace"));
    }
    Ok(value)
}

fn normalize_factory_text(value: &str, field: &str, max_len: usize) -> Result<String> {
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

fn normalize_factory_idempotency_key(value: &str) -> Result<String> {
    normalize_factory_identifier(value, "factory idempotency key", 240)
}

fn validate_factory_lease_seconds(value: i64) -> Result<i64> {
    if !(30..=3_600).contains(&value) {
        return Err(anyhow!(
            "factory claim lease must be between 30 and 3600 seconds"
        ));
    }
    Ok(value)
}

fn normalize_factory_policy(policy: Value) -> Result<Value> {
    let mut policy = match policy {
        Value::Null => json!({}),
        Value::Object(_) => policy,
        _ => return Err(anyhow!("factory policy snapshot must be a JSON object")),
    };
    let policy_object = policy
        .as_object_mut()
        .context("factory policy snapshot must be a JSON object")?;
    let source_base_ref = factory_policy_required_string(policy_object, "source_base_ref", 240)?;
    validate_factory_base_ref(&source_base_ref)?;
    if let Some(publication) = policy_object.get("publication") {
        let publication = publication
            .as_object()
            .context("factory publication policy must be a JSON object")?;
        if publication.get("allowed").and_then(Value::as_bool) == Some(true) {
            let base_ref = factory_policy_required_string(publication, "base_ref", 240)?;
            validate_factory_publication_base_ref(&base_ref)?;
        }
    }
    if let Some(verification_policy) = policy_object
        .get("verification_policy")
        .filter(|value| !value.is_null())
    {
        let verification_policy: VerificationPolicy =
            serde_json::from_value(verification_policy.clone())
                .context("factory verification policy is invalid")?;
        contract_revision::validate_mission_verification_policy(&verification_policy)
            .context("factory verification policy is invalid")?;
    }
    if policy_object.contains_key("write_scope")
        && let Some(scope) = factory_policy_string_array(policy_object, "write_scope")?
            .iter()
            .find(|scope| !write_scope_is_valid(scope))
    {
        return Err(anyhow!(
            "factory policy contains invalid write scope {scope}"
        ));
    }
    let upgrade_required = match policy_object.get("source_commit_upgrade_required") {
        None => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => {
            return Err(anyhow!(
                "factory source_commit_upgrade_required must be a boolean"
            ));
        }
    };
    match policy_object.get("source_base_commit") {
        Some(Value::String(source_base_commit)) if !source_base_commit.trim().is_empty() => {
            if upgrade_required {
                return Err(anyhow!(
                    "factory source policy cannot be pinned and require a legacy upgrade"
                ));
            }
            let source_base_commit = source_base_commit.to_ascii_lowercase();
            validate_factory_base_commit(&source_base_commit)?;
            policy_object.insert(
                "source_base_commit".to_owned(),
                Value::String(source_base_commit),
            );
            policy_object.insert(
                "source_commit_upgrade_required".to_owned(),
                Value::Bool(false),
            );
        }
        None if upgrade_required => {}
        _ => {
            return Err(anyhow!(
                "factory policy source_base_commit must be a non-empty string"
            ));
        }
    }
    if serde_json::to_vec(&policy)?.len() > 65_536 {
        return Err(anyhow!("factory policy snapshot cannot exceed 65536 bytes"));
    }
    Ok(policy)
}

fn ensure_new_factory_policy_is_pinned(policy: &Value) -> Result<()> {
    let policy = policy
        .as_object()
        .context("factory policy snapshot must be a JSON object")?;
    let source_base_commit =
        factory_policy_required_string(policy, "source_base_commit", 64)?.to_ascii_lowercase();
    validate_factory_base_commit(&source_base_commit)?;
    if policy
        .get("source_commit_upgrade_required")
        .and_then(Value::as_bool)
        != Some(false)
    {
        return Err(anyhow!(
            "new factory claims cannot require a legacy source upgrade"
        ));
    }
    Ok(())
}

fn normalize_factory_operation_request(request: Value) -> Result<Value> {
    if !request.is_object() {
        return Err(anyhow!(
            "factory operation request snapshot must be a JSON object"
        ));
    }
    if serde_json::to_vec(&request)?.len() > 65_536 {
        return Err(anyhow!(
            "factory operation request snapshot cannot exceed 65536 bytes"
        ));
    }
    Ok(request)
}

async fn lock_factory_keys_tx(tx: &mut Transaction<'_, Postgres>, keys: &[String]) -> Result<()> {
    let mut keys = keys.to_vec();
    keys.sort();
    keys.dedup();
    for key in keys {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(key)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

async fn factory_operation_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    idempotency_key: &str,
) -> Result<Option<FactoryOperation>> {
    Ok(sqlx::query(
        r#"
        SELECT work_item_id, actor_id, operation, resulting_version, claim_token, request
        FROM factory_operations
        WHERE corp_id = $1 AND idempotency_key = $2
        "#,
    )
    .bind(corp_id)
    .bind(idempotency_key)
    .fetch_optional(&mut **tx)
    .await?
    .map(|row| FactoryOperation {
        work_item_id: row.get("work_item_id"),
        actor_id: row.get("actor_id"),
        operation: row.get("operation"),
        resulting_version: row.get("resulting_version"),
        claim_token: row.get("claim_token"),
        request: row.get("request"),
    }))
}

fn ensure_factory_operation_matches(
    operation: &FactoryOperation,
    expected_operation: &str,
    actor_id: Uuid,
    work_item_id: Option<Uuid>,
    claim_token: Option<Uuid>,
    request: &Value,
) -> Result<()> {
    if operation.operation != expected_operation
        || operation.actor_id != actor_id
        || work_item_id.is_some_and(|expected| expected != operation.work_item_id)
        || claim_token.is_some_and(|expected| Some(expected) != operation.claim_token)
        || &operation.request != request
    {
        return Err(anyhow!(
            "factory idempotency key was already used for a different operation"
        ));
    }
    Ok(())
}

async fn record_factory_operation_tx(
    tx: &mut Transaction<'_, Postgres>,
    input: NewFactoryOperation<'_>,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO factory_operations
            (corp_id, idempotency_key, work_item_id, actor_id, operation,
             resulting_version, claim_token, request)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(input.corp_id)
    .bind(input.idempotency_key)
    .bind(input.work_item_id)
    .bind(input.actor_id)
    .bind(input.operation)
    .bind(input.resulting_version)
    .bind(input.claim_token)
    .bind(input.request)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn factory_work_item_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    work_item_id: Uuid,
    for_update: bool,
) -> Result<Option<(FactoryWorkItem, Uuid)>> {
    let suffix = if for_update { " FOR UPDATE" } else { "" };
    let query = format!(
        r#"
        SELECT id, corp_id, source_kind, source_project_owner, source_project_number,
               source_project_item_id, source_repository_owner, source_repository_name,
               source_issue_number, source_issue_node_id, source_issue_url, source_title,
               source_revision, state, version, claim_owner_id, claim_token,
               lease_expires_at, policy, mission_id, failure_detail, created_at, updated_at
        FROM factory_work_items
        WHERE id = $1 AND corp_id = $2{suffix}
        "#
    );
    let row = sqlx::query(&query)
        .bind(work_item_id)
        .bind(corp_id)
        .fetch_optional(&mut **tx)
        .await?;
    row.map(|row| {
        let claim_token = row.get("claim_token");
        Ok((map_factory_work_item(row)?, claim_token))
    })
    .transpose()
}

fn replayable_factory_claim_token(
    work_item: &FactoryWorkItem,
    current_token: Uuid,
    operation: &FactoryOperation,
    now: chrono::DateTime<Utc>,
) -> Option<Uuid> {
    let operation_token = operation.claim_token?;
    (!factory_state_is_terminal(work_item.state)
        && work_item.claim_owner_id == operation.actor_id
        && work_item.version >= operation.resulting_version
        && work_item.lease_expires_at > now
        && current_token == operation_token)
        .then_some(operation_token)
}

fn factory_state_is_terminal(state: FactoryWorkItemState) -> bool {
    matches!(
        state,
        FactoryWorkItemState::Published
            | FactoryWorkItemState::Failed
            | FactoryWorkItemState::Cancelled
    )
}

fn factory_transition_allowed(from: FactoryWorkItemState, to: FactoryWorkItemState) -> bool {
    match from {
        FactoryWorkItemState::Claimed => matches!(
            to,
            FactoryWorkItemState::Blocked
                | FactoryWorkItemState::Failed
                | FactoryWorkItemState::Cancelled
        ),
        FactoryWorkItemState::MissionCreated => matches!(
            to,
            FactoryWorkItemState::Running
                | FactoryWorkItemState::Blocked
                | FactoryWorkItemState::AwaitingApproval
                | FactoryWorkItemState::VerificationFailed
                | FactoryWorkItemState::Failed
                | FactoryWorkItemState::Cancelled
        ),
        FactoryWorkItemState::Running => matches!(
            to,
            FactoryWorkItemState::Blocked
                | FactoryWorkItemState::AwaitingApproval
                | FactoryWorkItemState::VerificationFailed
                | FactoryWorkItemState::Verified
                | FactoryWorkItemState::Failed
                | FactoryWorkItemState::Cancelled
        ),
        FactoryWorkItemState::Blocked => matches!(
            to,
            FactoryWorkItemState::Running
                | FactoryWorkItemState::AwaitingApproval
                | FactoryWorkItemState::VerificationFailed
                | FactoryWorkItemState::Failed
                | FactoryWorkItemState::Cancelled
        ),
        FactoryWorkItemState::AwaitingApproval => matches!(
            to,
            FactoryWorkItemState::Running
                | FactoryWorkItemState::Blocked
                | FactoryWorkItemState::VerificationFailed
                | FactoryWorkItemState::Verified
                | FactoryWorkItemState::Failed
                | FactoryWorkItemState::Cancelled
        ),
        FactoryWorkItemState::VerificationFailed => matches!(
            to,
            FactoryWorkItemState::Running
                | FactoryWorkItemState::Blocked
                | FactoryWorkItemState::Failed
                | FactoryWorkItemState::Cancelled
        ),
        FactoryWorkItemState::Verified => matches!(
            to,
            FactoryWorkItemState::Publishing
                | FactoryWorkItemState::Published
                | FactoryWorkItemState::Failed
                | FactoryWorkItemState::Cancelled
        ),
        FactoryWorkItemState::Publishing => matches!(
            to,
            FactoryWorkItemState::Verified
                | FactoryWorkItemState::Published
                | FactoryWorkItemState::Failed
                | FactoryWorkItemState::Cancelled
        ),
        FactoryWorkItemState::Published
        | FactoryWorkItemState::Failed
        | FactoryWorkItemState::Cancelled => false,
    }
}

fn ensure_active_factory_control(
    work_item: &FactoryWorkItem,
    current_token: Uuid,
    actor_id: Uuid,
    presented_token: Uuid,
    expected_version: i64,
    now: chrono::DateTime<Utc>,
) -> Result<()> {
    if factory_state_is_terminal(work_item.state) {
        return Err(anyhow!(
            "conflict: factory work item {} is terminal in state {}",
            work_item.id,
            work_item.state.as_str()
        ));
    }
    if work_item.claim_owner_id != actor_id || current_token != presented_token {
        return Err(anyhow!(
            "conflict: stale or unauthorized factory claim token"
        ));
    }
    if work_item.version != expected_version {
        return Err(anyhow!(
            "conflict: factory work item version is {}, not {}",
            work_item.version,
            expected_version
        ));
    }
    if work_item.lease_expires_at <= now {
        return Err(anyhow!("conflict: factory claim lease has expired"));
    }
    Ok(())
}

fn ensure_materializable_factory_claim(
    work_item: &FactoryWorkItem,
    current_token: Uuid,
    actor_id: Uuid,
    presented_token: Uuid,
    expected_version: i64,
    now: chrono::DateTime<Utc>,
) -> Result<()> {
    ensure_active_factory_control(
        work_item,
        current_token,
        actor_id,
        presented_token,
        expected_version,
        now,
    )?;
    if work_item.state != FactoryWorkItemState::Claimed || work_item.mission_id.is_some() {
        return Err(anyhow!(
            "conflict: factory work item {} cannot materialize from state {}",
            work_item.id,
            work_item.state.as_str()
        ));
    }
    Ok(())
}

fn ensure_factory_source_matches(
    work_item: &FactoryWorkItem,
    source: &FactorySourceInput,
) -> Result<()> {
    if work_item.source_kind != "github_project_issue"
        || work_item.source_project_owner != source.project_owner
        || work_item.source_project_number != source.project_number
        || work_item.source_project_item_id != source.project_item_id
        || work_item.source_repository_owner != source.repository_owner
        || work_item.source_repository_name != source.repository_name
        || work_item.source_issue_number != source.issue_number
        || work_item.source_issue_node_id != source.issue_node_id
        || work_item.source_issue_url != source.issue_url
        || work_item.source_title != source.title
        || work_item.source_revision != source.revision
    {
        return Err(anyhow!(
            "factory idempotency key was reused for a different source revision"
        ));
    }
    Ok(())
}

async fn factory_mission_details_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    mission_id: Uuid,
) -> Result<(MissionPlanIds, String)> {
    let strategy: Option<String> =
        sqlx::query_scalar("SELECT strategy FROM missions WHERE id = $1 AND corp_id = $2")
            .bind(mission_id)
            .bind(corp_id)
            .fetch_optional(&mut **tx)
            .await?;
    let strategy = strategy.context("factory mission linkage references a missing mission")?;
    let task_ids = sqlx::query_scalar(
        "SELECT id FROM tasks WHERE mission_id = $1 AND corp_id = $2 ORDER BY created_at, id",
    )
    .bind(mission_id)
    .bind(corp_id)
    .fetch_all(&mut **tx)
    .await?;
    if task_ids.is_empty() {
        return Err(anyhow!("factory mission linkage has no tasks"));
    }
    Ok((
        MissionPlanIds {
            mission_id,
            task_ids,
        },
        strategy,
    ))
}

async fn factory_event_room_id_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    mission_id: Option<Uuid>,
) -> Result<Option<Uuid>> {
    let Some(mission_id) = mission_id else {
        return Ok(None);
    };
    sqlx::query_scalar("SELECT room_id FROM missions WHERE id = $1 AND corp_id = $2")
        .bind(mission_id)
        .bind(corp_id)
        .fetch_optional(&mut **tx)
        .await?
        .with_context(|| format!("factory mission {mission_id} has no scoped room"))
        .map(Some)
}

async fn ensure_factory_mission_verified_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    mission_id: Uuid,
) -> Result<()> {
    let verified: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1
            FROM missions mission
            WHERE mission.id = $1
              AND mission.corp_id = $2
              AND mission.status = 'completed'
              AND EXISTS (
                  SELECT 1 FROM tasks task
                  WHERE task.mission_id = mission.id
                    AND task.corp_id = mission.corp_id
              )
              AND NOT EXISTS (
                  SELECT 1 FROM tasks task
                  WHERE task.mission_id = mission.id
                    AND task.corp_id = mission.corp_id
                    AND (
                        task.status <> 'completed'
                        OR task.verification_status <> 'passed'
                    )
              )
        )
        "#,
    )
    .bind(mission_id)
    .bind(corp_id)
    .fetch_one(&mut **tx)
    .await?;
    if !verified {
        return Err(anyhow!(
            "conflict: factory work item cannot enter verified before its mission and task verification pass"
        ));
    }
    Ok(())
}

async fn assert_actor_agent_scope_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    actor_id: Uuid,
    agent_id: Uuid,
) -> Result<()> {
    assert_actor_scope_tx(tx, corp_id, actor_id).await?;
    let agent_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agents WHERE id = $1 AND corp_id = $2)")
            .bind(agent_id)
            .bind(corp_id)
            .fetch_one(&mut **tx)
            .await?;
    if !agent_exists {
        return Err(anyhow!("agent does not belong to the requested Corp"));
    }
    Ok(())
}

async fn assert_actor_scope_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    actor_id: Uuid,
) -> Result<()> {
    let actor_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM actors WHERE id = $1 AND corp_id = $2)")
            .bind(actor_id)
            .bind(corp_id)
            .fetch_one(&mut **tx)
            .await?;
    if !actor_exists {
        return Err(anyhow!("actor does not belong to the requested Corp"));
    }
    Ok(())
}

async fn assert_room_membership_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    room_id: Uuid,
    actor_id: Uuid,
) -> Result<()> {
    let member = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT TRUE
        FROM room_memberships rm
        JOIN rooms r ON r.id = rm.room_id
        WHERE rm.room_id = $1 AND rm.actor_id = $2 AND r.corp_id = $3
        FOR KEY SHARE OF rm
        "#,
    )
    .bind(room_id)
    .bind(actor_id)
    .bind(corp_id)
    .fetch_optional(&mut **tx)
    .await?;
    if member.is_none() {
        return Err(anyhow!("forbidden: actor is not a member of this room"));
    }
    Ok(())
}

async fn validate_room_link_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    room_id: Uuid,
    link: &EntityLink,
) -> Result<()> {
    let valid: bool = match link.kind.as_str() {
        "mission" => sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM missions WHERE id = $1 AND corp_id = $2 AND room_id = $3)",
        )
        .bind(link.id)
        .bind(corp_id)
        .bind(room_id)
        .fetch_one(&mut **tx)
        .await?,
        "task" => {
            sqlx::query_scalar(
                r#"
                SELECT EXISTS(
                    SELECT 1 FROM tasks t
                    JOIN missions m ON m.id = t.mission_id
                    WHERE t.id = $1 AND t.corp_id = $2 AND m.room_id = $3
                )
                "#,
            )
            .bind(link.id)
            .bind(corp_id)
            .bind(room_id)
            .fetch_one(&mut **tx)
            .await?
        }
        "run" => {
            sqlx::query_scalar(
                r#"
                SELECT EXISTS(
                    SELECT 1 FROM runs r
                    JOIN tasks t ON t.id = r.task_id
                    JOIN missions m ON m.id = t.mission_id
                    WHERE r.id = $1 AND r.corp_id = $2 AND m.room_id = $3
                )
                "#,
            )
            .bind(link.id)
            .bind(corp_id)
            .bind(room_id)
            .fetch_one(&mut **tx)
            .await?
        }
        "artifact" => {
            sqlx::query_scalar(
                r#"
                SELECT EXISTS(
                    SELECT 1 FROM artifacts artifact
                    JOIN tasks t ON t.id = artifact.task_id
                    JOIN missions m ON m.id = t.mission_id
                    WHERE artifact.id = $1
                      AND artifact.corp_id = $2
                      AND artifact.status = 'ready'
                      AND m.room_id = $3
                )
                "#,
            )
            .bind(link.id)
            .bind(corp_id)
            .bind(room_id)
            .fetch_one(&mut **tx)
            .await?
        }
        other => return Err(anyhow!("unsupported room message link kind {other}")),
    };
    if !valid {
        return Err(anyhow!("linked entity is not visible in this room"));
    }
    Ok(())
}

async fn sanitize_verification_evidence_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: Uuid,
    mut payload: Value,
) -> Result<Value> {
    if payload.get("kind").and_then(Value::as_str) != Some("artifact") {
        return Ok(payload);
    }
    let status = payload
        .get("status")
        .and_then(Value::as_str)
        .context("artifact verification evidence omitted status")?;
    let artifact = sqlx::query(
        r#"
        SELECT id, corp_id, task_id, run_id, producer_agent_id, producer_runner_id,
               verifier, object_key, uri, sha256, media_type, bytes,
               artifact_role, file_name, metadata, provenance_signature, retention_until
        FROM artifacts
        WHERE run_id = $1
          AND status = 'ready'
          AND artifact_role = 'provider_evidence'
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(run_id)
    .fetch_optional(&mut **tx)
    .await?
    .map(map_stored_artifact);

    let sanitized = match artifact {
        Some(artifact) => {
            if status == "passed" {
                let evidence = payload
                    .get("payload")
                    .and_then(Value::as_object)
                    .context("artifact verification evidence omitted payload")?;
                let evidence_sha = evidence
                    .get("sha256")
                    .and_then(Value::as_str)
                    .context("artifact verification evidence omitted sha256")?;
                let evidence_bytes = evidence
                    .get("bytes")
                    .and_then(Value::as_u64)
                    .context("artifact verification evidence omitted bytes")?;
                let evidence_media_type = evidence
                    .get("media_type")
                    .and_then(Value::as_str)
                    .context("artifact verification evidence omitted media_type")?;
                if evidence_sha != artifact.sha256
                    || i64::try_from(evidence_bytes).ok() != Some(artifact.bytes)
                    || evidence_media_type != artifact.media_type
                {
                    return Err(anyhow!(
                        "artifact verification evidence does not match durable storage"
                    ));
                }
            }
            payload
                .as_object_mut()
                .context("verification evidence payload is not an object")?
                .insert(
                    "summary".to_owned(),
                    Value::String(format!(
                        "durable artifact {} has {} verified bytes",
                        artifact.id, artifact.bytes
                    )),
                );
            json!({
                "artifact_id": artifact.id,
                "uri": artifact.uri,
                "sha256": artifact.sha256,
                "bytes": artifact.bytes,
                "media_type": artifact.media_type,
                "producer_agent_id": artifact.producer_agent_id,
                "producer_runner_id": artifact.producer_runner_id,
                "verifier": artifact.verifier,
                "retention_until": artifact.retention_until,
                "provenance_signature": artifact.provenance_signature,
            })
        }
        None if status == "passed" => {
            return Err(anyhow!(
                "artifact verification evidence passed without durable storage"
            ));
        }
        None => {
            payload
                .as_object_mut()
                .context("verification evidence payload is not an object")?
                .insert(
                    "summary".to_owned(),
                    Value::String("durable artifact verification failed".to_owned()),
                );
            json!({"stored": false})
        }
    };
    payload
        .as_object_mut()
        .context("verification evidence payload is not an object")?
        .insert("payload".to_owned(), sanitized);
    Ok(payload)
}

fn redact_artifact_event_payload(event_type: &str, payload: &mut Value) {
    match event_type {
        "run.artifact" | "run.artifact_upload" | "run.deliverable" | "run.deliverable_upload" => {
            redact_path_fields(payload)
        }
        "run.verification_evidence"
            if payload.get("kind").and_then(Value::as_str) == Some("artifact") =>
        {
            if let Some(evidence) = payload.get_mut("payload") {
                redact_path_fields(evidence);
            }
            if payload
                .get("summary")
                .and_then(Value::as_str)
                .is_some_and(looks_like_local_path)
                && let Some(object) = payload.as_object_mut()
            {
                object.insert(
                    "summary".to_owned(),
                    Value::String(
                        "artifact verification evidence (local path redacted)".to_owned(),
                    ),
                );
            }
        }
        _ => {}
    }
}

fn redact_path_fields(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for key in ["path", "artifact_path", "content_base64", "object_key"] {
                object.remove(key);
            }
            for value in object.values_mut() {
                redact_path_fields(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                redact_path_fields(value);
            }
        }
        _ => {}
    }
}

fn looks_like_local_path(value: &str) -> bool {
    value.contains(":\\")
        || value.contains(":/")
        || value.contains("/output/runner/")
        || value.contains("\\output\\runner\\")
}

async fn append_event_tx(
    tx: &mut Transaction<'_, Postgres>,
    event: NewEvent,
) -> Result<Option<DomainEvent>> {
    let row = sqlx::query(
        r#"
        INSERT INTO events
            (id, schema_version, corp_id, room_id, actor_id, type, aggregate_type,
             aggregate_id, aggregate_version, correlation_id, causation_id,
             idempotency_key, visibility, payload)
        VALUES ($1, 1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
        ON CONFLICT (corp_id, idempotency_key) DO NOTHING
        RETURNING seq, id, schema_version, corp_id, room_id, actor_id, type,
                  aggregate_type, aggregate_id, aggregate_version, correlation_id,
                  causation_id, idempotency_key, visibility, payload, created_at
        "#,
    )
    .bind(event.id)
    .bind(event.corp_id)
    .bind(event.room_id)
    .bind(event.actor_id)
    .bind(event.event_type)
    .bind(event.aggregate_type)
    .bind(event.aggregate_id)
    .bind(event.aggregate_version)
    .bind(event.correlation_id)
    .bind(event.causation_id)
    .bind(event.idempotency_key)
    .bind(event.visibility)
    .bind(event.payload)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(map_event))
}

fn parse_id(value: &str) -> Result<Uuid> {
    Uuid::parse_str(value).with_context(|| format!("invalid fixed UUID {value}"))
}

fn map_corp(row: sqlx::postgres::PgRow) -> Corp {
    Corp {
        id: row.get("id"),
        slug: row.get("slug"),
        name: row.get("name"),
        created_at: row.get("created_at"),
    }
}

fn map_actor(row: sqlx::postgres::PgRow) -> Result<Actor> {
    Ok(Actor {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        name: row.get("name"),
        kind: parse_actor_kind(row.get::<String, _>("kind").as_str())?,
        role: row.get("role"),
        created_at: row.get("created_at"),
    })
}

fn map_room(row: sqlx::postgres::PgRow) -> Room {
    Room {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        name: row.get("name"),
        purpose: row.get("purpose"),
        created_at: row.get("created_at"),
    }
}

fn map_agent(row: sqlx::postgres::PgRow) -> Result<Agent> {
    Ok(Agent {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        actor_id: row.get("actor_id"),
        name: row.get("name"),
        role: row.get("role"),
        adapter: row.get("adapter"),
        status: parse_agent_status(row.get::<String, _>("status").as_str())?,
        station: row.get("station"),
        current_run_id: row.get("current_run_id"),
        accent: row.get("accent"),
        created_at: row.get("created_at"),
    })
}

fn map_mission(row: sqlx::postgres::PgRow) -> Result<Mission> {
    Ok(Mission {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        room_id: row.get("room_id"),
        requested_by: row.get("requested_by"),
        title: row.get("title"),
        description: row.get("description"),
        specification_version: row.get("specification_version"),
        strategy: row.get("strategy"),
        max_nodes: row.get("max_nodes"),
        max_depth: row.get("max_depth"),
        original_budget_tokens: row.get("original_budget_tokens"),
        original_budget_cost_microusd: row.get("original_budget_cost_microusd"),
        budget_tokens: row.get("budget_tokens"),
        budget_cost_microusd: row.get("budget_cost_microusd"),
        status: parse_mission_status(row.get::<String, _>("status").as_str())?,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

#[derive(Debug, Clone)]
pub struct MissionFinishScopeInput {
    pub task_id: Uuid,
    pub objective: String,
    pub expected_output: String,
    pub acceptance_tests: Vec<String>,
    pub write_scope: Vec<String>,
    pub budget_tokens: i64,
    pub budget_cost_microusd: i64,
    pub verification_policy: VerificationPolicy,
}

#[derive(Debug, Clone)]
pub struct ProposeMissionBudgetRevisionInput {
    pub corp_id: Uuid,
    pub mission_id: Uuid,
    pub actor_id: Uuid,
    pub expected_budget_tokens: i64,
    pub expected_budget_cost_microusd: i64,
    pub proposed_budget_tokens: i64,
    pub proposed_budget_cost_microusd: i64,
    pub rationale: String,
    pub idempotency_key: Uuid,
    pub finish_scope: Option<MissionFinishScopeInput>,
}

#[derive(Debug, Clone)]
pub struct DecideMissionBudgetRevisionInput {
    pub corp_id: Uuid,
    pub mission_id: Uuid,
    pub revision_id: Uuid,
    pub actor_id: Uuid,
    pub expected_version: i64,
    pub approved: bool,
    pub note: String,
    pub decision_key: Uuid,
}

#[derive(Debug, Clone)]
pub struct MissionBudgetRevisionOutcome {
    pub revision: MissionBudgetRevision,
    pub event: Option<DomainEvent>,
    pub replayed: bool,
}

fn map_mission_budget_revision(row: sqlx::postgres::PgRow) -> Result<MissionBudgetRevision> {
    let previous_contract = row
        .try_get::<Option<Value>, _>("previous_contract")?
        .map(serde_json::from_value)
        .transpose()
        .context("decode previous mission budget revision contract")?;
    let replacement_contract = row
        .try_get::<Option<Value>, _>("replacement_contract")?
        .map(serde_json::from_value)
        .transpose()
        .context("decode replacement mission budget revision contract")?;
    let previous_verification_policy = row
        .try_get::<Option<Value>, _>("previous_verification_policy")?
        .map(serde_json::from_value)
        .transpose()
        .context("decode previous mission budget revision verifier policy")?;
    let replacement_verification_policy = row
        .try_get::<Option<Value>, _>("replacement_verification_policy")?
        .map(serde_json::from_value)
        .transpose()
        .context("decode replacement mission budget revision verifier policy")?;
    Ok(MissionBudgetRevision {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        mission_id: row.get("mission_id"),
        proposed_by: row.get("proposed_by"),
        status: row.get("status"),
        version: row.get("version"),
        current_budget_tokens: row.get("current_budget_tokens"),
        current_budget_cost_microusd: row.get("current_budget_cost_microusd"),
        proposed_budget_tokens: row.get("proposed_budget_tokens"),
        proposed_budget_cost_microusd: row.get("proposed_budget_cost_microusd"),
        consumed_tokens_at_proposal: row.get("consumed_tokens_at_proposal"),
        consumed_cost_microusd_at_proposal: row.get("consumed_cost_microusd_at_proposal"),
        rationale: row.get("rationale"),
        replacement_task_id: row.get("replacement_task_id"),
        previous_contract,
        replacement_contract,
        previous_verification_policy,
        replacement_verification_policy,
        decided_by: row.get("decided_by"),
        decision_note: row.get("decision_note"),
        created_at: row.get("created_at"),
        decided_at: row.get("decided_at"),
        updated_at: row.get("updated_at"),
    })
}

fn map_factory_work_item(row: sqlx::postgres::PgRow) -> Result<FactoryWorkItem> {
    Ok(FactoryWorkItem {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        source_kind: row.get("source_kind"),
        source_project_owner: row.get("source_project_owner"),
        source_project_number: row.get("source_project_number"),
        source_project_item_id: row.get("source_project_item_id"),
        source_repository_owner: row.get("source_repository_owner"),
        source_repository_name: row.get("source_repository_name"),
        source_issue_number: row.get("source_issue_number"),
        source_issue_node_id: row.get("source_issue_node_id"),
        source_issue_url: row.get("source_issue_url"),
        source_title: row.get("source_title"),
        source_revision: row.get("source_revision"),
        state: parse_factory_work_item_state(row.get::<String, _>("state").as_str())?,
        version: row.get("version"),
        claim_owner_id: row.get("claim_owner_id"),
        lease_expires_at: row.get("lease_expires_at"),
        policy: row.get("policy"),
        mission_id: row.get("mission_id"),
        failure_detail: row.get("failure_detail"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn map_mission_contract_revision(row: sqlx::postgres::PgRow) -> Result<MissionContractRevision> {
    let next_action = match row.get::<String, _>("next_action").as_str() {
        "redispatch" => MissionContractRevisionAction::Redispatch,
        "resume" => MissionContractRevisionAction::Resume,
        other => return Err(anyhow!("unknown mission contract revision action {other}")),
    };
    Ok(MissionContractRevision {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        mission_id: row.get("mission_id"),
        task_id: row.get("task_id"),
        version: row.get("version"),
        revised_by: row.get("revised_by"),
        next_action,
        source_run_id: row.get("source_run_id"),
        reason: row.get("reason"),
        previous_description: row.get("previous_description"),
        replacement_description: row.get("replacement_description"),
        previous_contract: serde_json::from_value(row.get("previous_contract"))
            .context("decode previous mission contract")?,
        replacement_contract: serde_json::from_value(row.get("replacement_contract"))
            .context("decode replacement mission contract")?,
        previous_verification_policy: serde_json::from_value(
            row.get("previous_verification_policy"),
        )
        .context("decode previous mission verifier policy")?,
        replacement_verification_policy: serde_json::from_value(
            row.get("replacement_verification_policy"),
        )
        .context("decode replacement mission verifier policy")?,
        created_at: row.get("created_at"),
    })
}

fn map_task(row: sqlx::postgres::PgRow) -> Result<Task> {
    Ok(Task {
        id: row.get("id"),
        mission_id: row.get("mission_id"),
        corp_id: row.get("corp_id"),
        title: row.get("title"),
        objective: row.get("objective"),
        plan_key: row.get("plan_key"),
        contract: serde_json::from_value(row.get("contract")).context("decode task contract")?,
        contract_version: row.get("contract_version"),
        depth: row.get("depth"),
        max_attempts: row.get("max_attempts"),
        attempt_count: row.get("attempt_count"),
        required_adapter: row.get("required_adapter"),
        depends_on: Vec::new(),
        verification_policy: serde_json::from_value(row.get("verification_policy"))
            .context("decode verification policy")?,
        verification_status: row.get("verification_status"),
        status: parse_task_status(row.get::<String, _>("status").as_str())?,
        assigned_agent_id: row.get("assigned_agent_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn map_run(row: sqlx::postgres::PgRow) -> Result<Run> {
    Ok(Run {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        task_id: row.get("task_id"),
        agent_id: row.get("agent_id"),
        runner_id: row.get("runner_id"),
        assignment_token: row.get("assignment_token"),
        provider_session_id: row.get("provider_session_id"),
        resumed_from_run_id: row.get("resumed_from_run_id"),
        workspace_run_id: row.get("workspace_run_id"),
        model: row.get("model"),
        reasoning_effort: row.get("reasoning_effort"),
        input_tokens: row.get("input_tokens"),
        output_tokens: row.get("output_tokens"),
        cost_microusd: row.get("cost_microusd"),
        budget_tokens_limit: row.get("budget_tokens_limit"),
        budget_cost_microusd_limit: row.get("budget_cost_microusd_limit"),
        breaker_stage: row.get("breaker_stage"),
        no_progress_events: row.get("no_progress_events"),
        repeated_tool_count: row.get("repeated_tool_count"),
        source_repository: row.get("source_repository"),
        source_base_ref: row.get("source_base_ref"),
        source_base_commit: row.get("source_base_commit"),
        workspace_path: row.get("workspace_path"),
        workspace_branch: row.get("workspace_branch"),
        workspace_base_ref: row.get("workspace_base_ref"),
        workspace_base_commit: row.get("workspace_base_commit"),
        workspace_disposition: row.get("workspace_disposition"),
        workspace_detail: row.get("workspace_detail"),
        verification_status: row.get("verification_status"),
        verification_summary: row.get("verification_summary"),
        verification_sha256: row.get("verification_sha256"),
        deliverable_sha256: row.get("deliverable_sha256"),
        status: parse_run_status(row.get::<String, _>("status").as_str())?,
        summary: row.get("summary"),
        artifact_id: row.get("artifact_id"),
        artifact_uri: row.get("artifact_uri"),
        artifact_media_type: row.get("artifact_media_type"),
        artifact_signature: row.get("artifact_signature"),
        artifact_path: row.get("artifact_path"),
        artifact_sha256: row.get("artifact_sha256"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn map_stored_artifact(row: sqlx::postgres::PgRow) -> StoredArtifact {
    StoredArtifact {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        task_id: row.get("task_id"),
        run_id: row.get("run_id"),
        producer_agent_id: row.get("producer_agent_id"),
        producer_runner_id: row.get("producer_runner_id"),
        verifier: row.get("verifier"),
        object_key: row.get("object_key"),
        uri: row.get("uri"),
        sha256: row.get("sha256"),
        media_type: row.get("media_type"),
        bytes: row.get("bytes"),
        artifact_role: row.get("artifact_role"),
        file_name: row.get("file_name"),
        metadata: row.get("metadata"),
        provenance_signature: row.get("provenance_signature"),
        retention_until: row.get("retention_until"),
    }
}

fn map_prepared_artifact(row: sqlx::postgres::PgRow) -> Result<PreparedArtifactUpload> {
    let status: String = row.get("status");
    let staging_key: Option<String> = row.get("staging_key");
    if status == "staged" && staging_key.is_none() {
        return Err(anyhow!("staged artifact omitted its staging key"));
    }
    let created_at = row.get("created_at");
    Ok(PreparedArtifactUpload {
        artifact: map_stored_artifact(row),
        staging_key,
        status,
        created_at,
    })
}

fn ensure_artifact_upload_matches(
    existing: &StoredArtifact,
    candidate: &StoredArtifact,
    require_event_id: bool,
) -> Result<()> {
    if (require_event_id && existing.id != candidate.id)
        || existing.corp_id != candidate.corp_id
        || existing.task_id != candidate.task_id
        || existing.run_id != candidate.run_id
        || existing.producer_agent_id != candidate.producer_agent_id
        || existing.producer_runner_id != candidate.producer_runner_id
        || existing.verifier != candidate.verifier
        || existing.sha256 != candidate.sha256
        || existing.media_type != candidate.media_type
        || existing.bytes != candidate.bytes
        || existing.artifact_role != candidate.artifact_role
        || existing.file_name != candidate.file_name
        || existing.metadata != candidate.metadata
    {
        return Err(anyhow!(
            "duplicate artifact upload conflicts with persisted metadata"
        ));
    }
    Ok(())
}

fn normalize_artifact_rejection_reason(reason: &str) -> String {
    let normalized = reason
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let mut normalized = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        normalized = "artifact finalization failed permanently".to_owned();
    }
    if normalized.len() > 2_000 {
        let mut end = 2_000;
        while !normalized.is_char_boundary(end) {
            end -= 1;
        }
        normalized.truncate(end);
    }
    normalized
}

fn map_verification_evidence(row: sqlx::postgres::PgRow) -> VerificationEvidence {
    let kind: String = row.get("kind");
    let mut payload: Value = row.get("payload");
    if kind == "artifact" {
        redact_path_fields(&mut payload);
    }
    let summary: String = row.get("summary");
    VerificationEvidence {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        task_id: row.get("task_id"),
        run_id: row.get("run_id"),
        check_index: row.get("check_index"),
        kind: kind.clone(),
        status: row.get("status"),
        summary: if kind == "artifact" && looks_like_local_path(&summary) {
            "artifact verification evidence (local path redacted)".to_owned()
        } else {
            summary
        },
        payload,
        created_at: row.get("created_at"),
    }
}

fn map_verification_request(row: sqlx::postgres::PgRow) -> Result<VerificationRequest> {
    Ok(VerificationRequest {
        run_id: row.get("run_id"),
        corp_id: row.get("corp_id"),
        task_id: row.get("task_id"),
        gate_type: row.get("gate_type"),
        gate: serde_json::from_value(row.get("gate")).context("decode verification gate")?,
        status: row.get("status"),
        requested_at: row.get("requested_at"),
        decided_by: row.get("decided_by"),
        decision_note: row.get("decision_note"),
        decided_at: row.get("decided_at"),
    })
}

fn map_source_deliverable(row: sqlx::postgres::PgRow) -> Result<SourceDeliverable> {
    let form = match row.get::<String, _>("form").as_str() {
        "commit_branch" => DeliverableForm::CommitBranch,
        "patch" => DeliverableForm::Patch,
        "archive" => DeliverableForm::Archive,
        "typed_artifact_set" => DeliverableForm::TypedArtifactSet,
        "review_only_report" => DeliverableForm::ReviewOnlyReport,
        other => return Err(anyhow!("unknown source deliverable form {other}")),
    };
    Ok(SourceDeliverable {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        task_id: row.get("task_id"),
        run_id: row.get("run_id"),
        artifact_id: row.get("artifact_id"),
        form,
        file_name: row.get("file_name"),
        uri: row.get("uri"),
        sha256: row.get("sha256"),
        media_type: row.get("media_type"),
        bytes: row.get("bytes"),
        provenance_signature: row.get("provenance_signature"),
        verification_sha256: row.get("verification_sha256"),
        base_commit: row.get("base_commit"),
        head_commit: row.get("head_commit"),
        branch: row.get("branch"),
        integration_state: row.get("integration_state"),
        retention_until: row.get("retention_until"),
        created_at: row.get("created_at"),
    })
}

fn map_pull_request_publication(row: sqlx::postgres::PgRow) -> Result<PullRequestPublication> {
    Ok(PullRequestPublication {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        factory_work_item_id: row.get("factory_work_item_id"),
        mission_id: row.get("mission_id"),
        source_deliverable_id: row.get("source_deliverable_id"),
        artifact_id: row.get("artifact_id"),
        task_id: row.get("task_id"),
        run_id: row.get("run_id"),
        source_issue_number: row.get("source_issue_number"),
        source_issue_url: row.get("source_issue_url"),
        target_repository: row.get("target_repository"),
        base_ref: row.get("base_ref"),
        branch: row.get("branch"),
        commit_sha: row.get("commit_sha"),
        title: row.get("title"),
        body: row.get("body"),
        actor_id: row.get("actor_id"),
        authorization_id: row.get("authorization_id"),
        authorization_snapshot: row.get("authorization_snapshot"),
        effect_key: row.get("effect_key"),
        idempotency_key: row.get("idempotency_key"),
        state: parse_pull_request_publication_state(row.get::<String, _>("state").as_str())?,
        version: row.get("version"),
        attempt_count: row.get("attempt_count"),
        publisher_id: row.get("publisher_id"),
        publisher_lease_expires_at: row.get("publisher_lease_expires_at"),
        failure_detail: row.get("failure_detail"),
        branch_pushed_at: row.get("branch_pushed_at"),
        pull_request_number: row.get("pull_request_number"),
        pull_request_node_id: row.get("pull_request_node_id"),
        pull_request_url: row.get("pull_request_url"),
        pull_request_state: row.get("pull_request_state"),
        pull_request_draft: row.get("pull_request_draft"),
        pull_request_base_ref: row.get("pull_request_base_ref"),
        pull_request_head_sha: row.get("pull_request_head_sha"),
        pull_request_head_repository_owner: row.get("pull_request_head_repository_owner"),
        pull_request_is_cross_repository: row.get("pull_request_is_cross_repository"),
        project_owner: row.get("project_owner"),
        project_number: row.get("project_number"),
        project_item_id: row.get("project_item_id"),
        project_status_before: row.get("project_status_before"),
        project_status_after: row.get("project_status_after"),
        project_status_updated_at: row.get("project_status_updated_at"),
        auto_merge_enabled: row.get("auto_merge_enabled"),
        merge_authorized: row.get("merge_authorized"),
        deployment_authorized: row.get("deployment_authorized"),
        provenance: row.get("provenance"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn map_pull_request_publication_attempt(
    row: sqlx::postgres::PgRow,
) -> PullRequestPublicationAttempt {
    PullRequestPublicationAttempt {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        publication_id: row.get("publication_id"),
        attempt: row.get("attempt"),
        actor_id: row.get("actor_id"),
        authorization_id: row.get("authorization_id"),
        authorization_snapshot: row.get("authorization_snapshot"),
        publisher_id: row.get("publisher_id"),
        state: row.get("state"),
        failure_detail: row.get("failure_detail"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
    }
}

fn map_action_approval(row: sqlx::postgres::PgRow) -> ActionApproval {
    ActionApproval {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        room_id: row.get("room_id"),
        mission_id: row.get("mission_id"),
        task_id: row.get("task_id"),
        run_id: row.get("run_id"),
        agent_id: row.get("agent_id"),
        action_key: row.get("action_key"),
        action: row.get("action"),
        risk: row.get("risk"),
        rationale: row.get("rationale"),
        required_roles: row.get("required_roles"),
        status: row.get("status"),
        expires_at: row.get("expires_at"),
        created_at: row.get("created_at"),
        decided_by: row.get("decided_by"),
        decision_note: row.get("decision_note"),
        decided_at: row.get("decided_at"),
    }
}

fn map_circuit_breaker_incident(row: sqlx::postgres::PgRow) -> CircuitBreakerIncident {
    CircuitBreakerIncident {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        mission_id: row.get("mission_id"),
        task_id: row.get("task_id"),
        run_id: row.get("run_id"),
        stage: row.get("stage"),
        reason: row.get("reason"),
        input: row.get("input"),
        created_at: row.get("created_at"),
    }
}

fn map_lease(row: sqlx::postgres::PgRow) -> ControlLease {
    ControlLease {
        agent_id: row.get("agent_id"),
        corp_id: row.get("corp_id"),
        actor_id: row.get("actor_id"),
        token: row.get("token"),
        expires_at: row.get("expires_at"),
    }
}

fn map_queued_message(row: sqlx::postgres::PgRow) -> QueuedMessage {
    QueuedMessage {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        agent_id: row.get("agent_id"),
        actor_id: row.get("actor_id"),
        text: row.get("text"),
        status: row.get("status"),
        created_at: row.get("created_at"),
    }
}

fn map_room_message(row: sqlx::postgres::PgRow) -> RoomMessage {
    let link_kind: Option<String> = row.get("link_kind");
    let link_id: Option<Uuid> = row.get("link_id");
    let link = match (link_kind, link_id) {
        (Some(kind), Some(id)) => Some(EntityLink { kind, id }),
        _ => None,
    };
    RoomMessage {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        room_id: row.get("room_id"),
        actor_id: row.get("actor_id"),
        thread_root_id: row.get("thread_root_id"),
        reply_to_id: row.get("reply_to_id"),
        body: row.get("body"),
        mentions: row.get("mentions"),
        link,
        created_at: row.get("created_at"),
    }
}

fn map_runner(row: sqlx::postgres::PgRow) -> RunnerRecord {
    RunnerRecord {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        hostname: row.get("hostname"),
        os: row.get("os"),
        capabilities: row.get("capabilities"),
        connection_epoch: row.get("connection_epoch"),
        status: row.get("status"),
        last_seen_at: row.get("last_seen_at"),
        grace_expires_at: row.get("grace_expires_at"),
    }
}

fn map_event(row: sqlx::postgres::PgRow) -> DomainEvent {
    let event_type: String = row.get("type");
    let mut payload: Value = row.get("payload");
    redact_artifact_event_payload(&event_type, &mut payload);
    DomainEvent {
        seq: row.get("seq"),
        id: row.get("id"),
        schema_version: row.get("schema_version"),
        corp_id: row.get("corp_id"),
        room_id: row.get("room_id"),
        actor_id: row.get("actor_id"),
        event_type,
        aggregate_type: row.get("aggregate_type"),
        aggregate_id: row.get("aggregate_id"),
        aggregate_version: row.get("aggregate_version"),
        correlation_id: row.get("correlation_id"),
        causation_id: row.get("causation_id"),
        idempotency_key: row.get("idempotency_key"),
        visibility: row.get("visibility"),
        payload,
        created_at: row.get("created_at"),
    }
}

fn parse_actor_kind(value: &str) -> Result<ActorKind> {
    match value {
        "human" => Ok(ActorKind::Human),
        "agent" => Ok(ActorKind::Agent),
        "service" => Ok(ActorKind::Service),
        other => Err(anyhow!("unknown actor kind {other}")),
    }
}

fn parse_agent_status(value: &str) -> Result<AgentStatus> {
    match value {
        "idle" => Ok(AgentStatus::Idle),
        "starting" => Ok(AgentStatus::Starting),
        "working" => Ok(AgentStatus::Working),
        "blocked" => Ok(AgentStatus::Blocked),
        "reviewing" => Ok(AgentStatus::Reviewing),
        "offline" => Ok(AgentStatus::Offline),
        other => Err(anyhow!("unknown agent status {other}")),
    }
}

fn parse_mission_status(value: &str) -> Result<MissionStatus> {
    match value {
        "draft" => Ok(MissionStatus::Draft),
        "ready" => Ok(MissionStatus::Ready),
        "running" => Ok(MissionStatus::Running),
        "completed" => Ok(MissionStatus::Completed),
        "failed" => Ok(MissionStatus::Failed),
        "cancelled" => Ok(MissionStatus::Cancelled),
        other => Err(anyhow!("unknown mission status {other}")),
    }
}

fn parse_task_status(value: &str) -> Result<TaskStatus> {
    match value {
        "pending" => Ok(TaskStatus::Pending),
        "ready" => Ok(TaskStatus::Ready),
        "claimed" => Ok(TaskStatus::Claimed),
        "running" => Ok(TaskStatus::Running),
        "blocked" => Ok(TaskStatus::Blocked),
        "awaiting_approval" => Ok(TaskStatus::AwaitingApproval),
        "verification_failed" => Ok(TaskStatus::VerificationFailed),
        "review" => Ok(TaskStatus::Review),
        "completed" => Ok(TaskStatus::Completed),
        "failed" => Ok(TaskStatus::Failed),
        "cancelled" => Ok(TaskStatus::Cancelled),
        other => Err(anyhow!("unknown task status {other}")),
    }
}

fn parse_run_status(value: &str) -> Result<RunStatus> {
    match value {
        "provisioning" => Ok(RunStatus::Provisioning),
        "starting" => Ok(RunStatus::Starting),
        "running" => Ok(RunStatus::Running),
        "waiting_for_input" => Ok(RunStatus::WaitingForInput),
        "waiting_for_approval" => Ok(RunStatus::WaitingForApproval),
        "verifying" => Ok(RunStatus::Verifying),
        "completed" => Ok(RunStatus::Completed),
        "failed" => Ok(RunStatus::Failed),
        "cancelled" => Ok(RunStatus::Cancelled),
        "lost" => Ok(RunStatus::Lost),
        other => Err(anyhow!("unknown run status {other}")),
    }
}

fn parse_factory_work_item_state(value: &str) -> Result<FactoryWorkItemState> {
    match value {
        "claimed" => Ok(FactoryWorkItemState::Claimed),
        "mission_created" => Ok(FactoryWorkItemState::MissionCreated),
        "running" => Ok(FactoryWorkItemState::Running),
        "blocked" => Ok(FactoryWorkItemState::Blocked),
        "awaiting_approval" => Ok(FactoryWorkItemState::AwaitingApproval),
        "verification_failed" => Ok(FactoryWorkItemState::VerificationFailed),
        "verified" => Ok(FactoryWorkItemState::Verified),
        "publishing" => Ok(FactoryWorkItemState::Publishing),
        "published" => Ok(FactoryWorkItemState::Published),
        "failed" => Ok(FactoryWorkItemState::Failed),
        "cancelled" => Ok(FactoryWorkItemState::Cancelled),
        other => Err(anyhow!("unknown factory work-item state {other}")),
    }
}

fn parse_pull_request_publication_state(value: &str) -> Result<PullRequestPublicationState> {
    match value {
        "requested" => Ok(PullRequestPublicationState::Requested),
        "publishing" => Ok(PullRequestPublicationState::Publishing),
        "branch_pushed" => Ok(PullRequestPublicationState::BranchPushed),
        "pull_request_created" => Ok(PullRequestPublicationState::PullRequestCreated),
        "published" => Ok(PullRequestPublicationState::Published),
        other => Err(anyhow!("unknown pull-request publication state {other}")),
    }
}

fn breaker_rank(stage: &str) -> u8 {
    match stage {
        "steer" => 1,
        "constrain" => 2,
        "suspend" => 3,
        "stop" => 4,
        _ => 0,
    }
}

fn breaker_is_hard(stage: &str) -> bool {
    matches!(stage, "suspend" | "stop")
}

fn should_retry_runner_failure(stage: &str, attempt_count: i32, max_attempts: i32) -> bool {
    attempt_count < max_attempts && !breaker_is_hard(stage)
}

fn ensure_breaker_allows_human_progress(stage: &str, action: &str) -> Result<()> {
    if breaker_is_hard(stage) {
        return Err(anyhow!(
            "{action} is blocked by circuit breaker stage {stage}"
        ));
    }
    Ok(())
}

fn budget_scope_lock_keys(corp_id: Uuid, requester: Uuid) -> Vec<String> {
    vec![
        format!("budget:actor:{corp_id}:{requester}"),
        format!("budget:corp:{corp_id}"),
    ]
}

fn lineage_run_is_pre_dispatch_failure(
    status: &str,
    workspace_path: Option<&str>,
    workspace_disposition: Option<&str>,
    workspace_detail: Option<&str>,
) -> bool {
    status == "failed"
        && workspace_path.is_none()
        && workspace_disposition.is_none()
        && workspace_detail == Some("dispatch_not_started")
}

async fn rolling_budget_remaining_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    requester: Uuid,
) -> Result<RollingBudgetRemaining> {
    let row = sqlx::query(
        r#"
        SELECT COALESCE(policy.actor_tokens_per_24h, 4000000) AS actor_token_limit,
               COALESCE(policy.actor_cost_microusd_per_24h, 10000000) AS actor_cost_limit,
               COALESCE(policy.corp_tokens_per_24h, 20000000) AS corp_token_limit,
               COALESCE(policy.corp_cost_microusd_per_24h, 100000000) AS corp_cost_limit,
               (
                   SELECT COALESCE(SUM(run.input_tokens + run.output_tokens), 0)::BIGINT
                   FROM runs run
                   JOIN tasks task ON task.id = run.task_id
                   JOIN missions mission ON mission.id = task.mission_id
                   WHERE mission.corp_id = corp.id
                     AND mission.requested_by = $2
                     AND run.created_at >= now() - interval '24 hours'
               ) AS actor_tokens_used,
               (
                   SELECT COALESCE(SUM(run.cost_microusd), 0)::BIGINT
                   FROM runs run
                   JOIN tasks task ON task.id = run.task_id
                   JOIN missions mission ON mission.id = task.mission_id
                   WHERE mission.corp_id = corp.id
                     AND mission.requested_by = $2
                     AND run.created_at >= now() - interval '24 hours'
               ) AS actor_cost_used,
               (
                   SELECT COALESCE(SUM(run.input_tokens + run.output_tokens), 0)::BIGINT
                   FROM runs run
                   WHERE run.corp_id = corp.id
                     AND run.created_at >= now() - interval '24 hours'
               ) AS corp_tokens_used,
               (
                   SELECT COALESCE(SUM(run.cost_microusd), 0)::BIGINT
                   FROM runs run
                   WHERE run.corp_id = corp.id
                     AND run.created_at >= now() - interval '24 hours'
               ) AS corp_cost_used
        FROM corps corp
        LEFT JOIN corp_budget_policies policy ON policy.corp_id = corp.id
        WHERE corp.id = $1
        "#,
    )
    .bind(corp_id)
    .bind(requester)
    .fetch_optional(&mut **tx)
    .await?
    .context("Corp not found while evaluating rolling resume budget")?;
    Ok(RollingBudgetRemaining {
        actor_tokens: row
            .get::<i64, _>("actor_token_limit")
            .saturating_sub(row.get("actor_tokens_used")),
        actor_cost_microusd: row
            .get::<i64, _>("actor_cost_limit")
            .saturating_sub(row.get("actor_cost_used")),
        corp_tokens: row
            .get::<i64, _>("corp_token_limit")
            .saturating_sub(row.get("corp_tokens_used")),
        corp_cost_microusd: row
            .get::<i64, _>("corp_cost_limit")
            .saturating_sub(row.get("corp_cost_used")),
    })
}

async fn ensure_run_not_hard_blocked_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    run_id: Uuid,
    stage: &str,
    action: &str,
) -> Result<()> {
    ensure_breaker_allows_human_progress(stage, action)?;
    if hard_breaker_reached_tx(tx, corp_id, run_id).await? {
        return Err(anyhow!(
            "{action} is blocked because current budget or loop metrics require a hard breaker"
        ));
    }
    Ok(())
}

async fn hard_breaker_reached_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    run_id: Uuid,
) -> Result<bool> {
    let reached = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT
            (run.budget_tokens_limit > 0
             AND run.input_tokens + run.output_tokens >= run.budget_tokens_limit)
         OR (run.budget_cost_microusd_limit > 0
             AND run.cost_microusd >= run.budget_cost_microusd_limit)
         OR (mission.budget_tokens > 0 AND (
                SELECT COALESCE(SUM(other.input_tokens + other.output_tokens), 0)::BIGINT
                FROM runs other
                JOIN tasks other_task ON other_task.id = other.task_id
                WHERE other_task.mission_id = mission.id
            ) >= mission.budget_tokens)
         OR (mission.budget_cost_microusd > 0 AND (
                SELECT COALESCE(SUM(other.cost_microusd), 0)::BIGINT
                FROM runs other
                JOIN tasks other_task ON other_task.id = other.task_id
                WHERE other_task.mission_id = mission.id
            ) >= mission.budget_cost_microusd)
         OR (COALESCE(policy.actor_tokens_per_24h, 4000000) > 0 AND (
                SELECT COALESCE(SUM(other.input_tokens + other.output_tokens), 0)::BIGINT
                FROM runs other
                JOIN tasks other_task ON other_task.id = other.task_id
                JOIN missions other_mission ON other_mission.id = other_task.mission_id
                WHERE other_mission.corp_id = run.corp_id
                  AND other_mission.requested_by = mission.requested_by
                  AND other.created_at >= now() - interval '24 hours'
            ) >= COALESCE(policy.actor_tokens_per_24h, 4000000))
         OR (COALESCE(policy.actor_cost_microusd_per_24h, 10000000) > 0 AND (
                SELECT COALESCE(SUM(other.cost_microusd), 0)::BIGINT
                FROM runs other
                JOIN tasks other_task ON other_task.id = other.task_id
                JOIN missions other_mission ON other_mission.id = other_task.mission_id
                WHERE other_mission.corp_id = run.corp_id
                  AND other_mission.requested_by = mission.requested_by
                  AND other.created_at >= now() - interval '24 hours'
            ) >= COALESCE(policy.actor_cost_microusd_per_24h, 10000000))
         OR (COALESCE(policy.corp_tokens_per_24h, 20000000) > 0 AND (
                SELECT COALESCE(SUM(other.input_tokens + other.output_tokens), 0)::BIGINT
                FROM runs other
                WHERE other.corp_id = run.corp_id
                  AND other.created_at >= now() - interval '24 hours'
            ) >= COALESCE(policy.corp_tokens_per_24h, 20000000))
         OR (COALESCE(policy.corp_cost_microusd_per_24h, 100000000) > 0 AND (
                SELECT COALESCE(SUM(other.cost_microusd), 0)::BIGINT
                FROM runs other
                WHERE other.corp_id = run.corp_id
                  AND other.created_at >= now() - interval '24 hours'
            ) >= COALESCE(policy.corp_cost_microusd_per_24h, 100000000))
         OR (COALESCE(policy.no_progress_event_limit, 8) > 0
             AND run.no_progress_events >= COALESCE(policy.no_progress_event_limit, 8))
         OR (COALESCE(policy.repeated_tool_limit, 5) > 0
             AND run.repeated_tool_count >= COALESCE(policy.repeated_tool_limit, 5))
        FROM runs run
        JOIN tasks task ON task.id = run.task_id
        JOIN missions mission ON mission.id = task.mission_id
        LEFT JOIN corp_budget_policies policy ON policy.corp_id = run.corp_id
        WHERE run.id = $1 AND run.corp_id = $2
        FOR UPDATE OF run
        "#,
    )
    .bind(run_id)
    .bind(corp_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(reached)
}

fn runner_event_advances_run(event_type: &str) -> bool {
    matches!(
        event_type,
        "run.started"
            | "run.status"
            | "run.approval_requested"
            | "run.artifact_upload"
            | "run.deliverable_upload"
            | "run.verification_started"
            | "run.verification_evidence"
            | "run.verification_passed"
            | "run.verification_waiting"
            | "run.completed"
    )
}

fn breaker_blocks_runner_progress(stage: &str, event_type: &str) -> bool {
    breaker_is_hard(stage) && runner_event_advances_run(event_type)
}

fn strongest_breaker_stage<'a>(
    inputs: &'a [(&'a str, i64, i64)],
) -> Option<(&'static str, &'a str, i64, i64)> {
    let mut selected: Option<(&'static str, &'a str, i64, i64)> = None;
    for (name, used, limit) in inputs {
        if *limit <= 0 {
            continue;
        }
        let basis_points = used.saturating_mul(10_000) / limit;
        let stage = if basis_points >= 11_000 {
            Some("stop")
        } else if basis_points >= 10_000 {
            Some("suspend")
        } else if basis_points >= 9_000 {
            Some("constrain")
        } else if basis_points >= 7_500 {
            Some("steer")
        } else {
            None
        };
        if let Some(stage) = stage
            && selected
                .as_ref()
                .is_none_or(|current| breaker_rank(stage) > breaker_rank(current.0))
        {
            selected = Some((stage, *name, *used, *limit));
        }
    }
    selected
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use crony_domain::{
        FactoryWorkItem, FactoryWorkItemState, ManualVerificationGate, PlannedTask, TaskContract,
        TaskGraphPlan, VerificationPolicy, VerifierCheck,
    };
    use serde_json::json;
    use uuid::Uuid;

    use super::{
        FactorySourceInput, breaker_blocks_runner_progress, ensure_active_factory_control,
        ensure_breaker_allows_human_progress, ensure_new_factory_policy_is_pinned,
        factory_transition_allowed, lineage_run_is_pre_dispatch_failure,
        normalize_artifact_rejection_reason, normalize_factory_policy, normalize_factory_source,
        should_retry_runner_failure, validate_factory_lease_seconds,
        validate_factory_plan_against_policy,
    };

    #[test]
    fn hard_breakers_fence_progress_but_allow_terminal_cleanup() {
        for stage in ["suspend", "stop"] {
            for event_type in [
                "run.started",
                "run.status",
                "run.approval_requested",
                "run.artifact_upload",
                "run.deliverable_upload",
                "run.verification_started",
                "run.verification_evidence",
                "run.verification_passed",
                "run.verification_waiting",
                "run.completed",
            ] {
                assert!(
                    breaker_blocks_runner_progress(stage, event_type),
                    "{stage} should block {event_type}"
                );
            }
            for event_type in [
                "run.output",
                "run.usage",
                "run.teardown_uncertain",
                "run.session_terminated",
                "run.failed",
                "run.cancelled",
                "run.workspace_preserved",
                "run.workspace_removed",
            ] {
                assert!(
                    !breaker_blocks_runner_progress(stage, event_type),
                    "{stage} should allow terminal accounting event {event_type}"
                );
            }
        }
        assert!(!breaker_blocks_runner_progress(
            "constrain",
            "run.completed"
        ));
    }

    #[test]
    fn hard_breakers_block_retries_and_human_completion_paths() {
        for stage in ["suspend", "stop"] {
            assert!(!should_retry_runner_failure(stage, 1, 3));
            assert!(ensure_breaker_allows_human_progress(stage, "decision").is_err());
        }
        assert!(should_retry_runner_failure("healthy", 1, 3));
        assert!(!should_retry_runner_failure("healthy", 3, 3));
        assert!(ensure_breaker_allows_human_progress("constrain", "decision").is_ok());
        assert!(lineage_run_is_pre_dispatch_failure(
            "failed",
            None,
            None,
            Some("dispatch_not_started")
        ));
        assert!(!lineage_run_is_pre_dispatch_failure(
            "failed",
            Some("workspace"),
            None,
            Some("dispatch_not_started")
        ));
        assert!(!lineage_run_is_pre_dispatch_failure(
            "failed",
            None,
            Some("preserved"),
            Some("dispatch_not_started")
        ));
    }

    fn factory_work_item(lease_expires_at: chrono::DateTime<Utc>) -> (FactoryWorkItem, Uuid) {
        let corp_id = Uuid::new_v4();
        let actor_id = Uuid::new_v4();
        let token = Uuid::new_v4();
        (
            FactoryWorkItem {
                id: Uuid::new_v4(),
                corp_id,
                source_kind: "github_project_issue".to_owned(),
                source_project_owner: "owner".to_owned(),
                source_project_number: 3,
                source_project_item_id: "PVTI_test".to_owned(),
                source_repository_owner: "owner".to_owned(),
                source_repository_name: "repo".to_owned(),
                source_issue_number: 59,
                source_issue_node_id: "I_test".to_owned(),
                source_issue_url: "https://github.com/owner/repo/issues/59".to_owned(),
                source_title: "Factory claim".to_owned(),
                source_revision: "2026-09-01T00:00:00Z".to_owned(),
                state: FactoryWorkItemState::Claimed,
                version: 2,
                claim_owner_id: actor_id,
                lease_expires_at,
                policy: json!({}),
                mission_id: None,
                failure_detail: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            token,
        )
    }

    fn factory_policy_plan(
        model: Option<&str>,
        reasoning_effort: Option<&str>,
    ) -> (FactoryWorkItem, TaskGraphPlan) {
        let (mut work_item, _) = factory_work_item(Utc::now() + Duration::minutes(5));
        work_item.policy = json!({
            "schema_version": 1,
            "source_of_truth": "github_project",
            "auto_merge": false,
            "repository_allowlist": ["owner/repo"],
            "source_base_ref": "HEAD",
            "source_base_commit": "1111111111111111111111111111111111111111",
            "adapter_allowlist": ["codex"],
            "strategy_allowlist": ["single"],
            "model": model,
            "reasoning_effort": reasoning_effort,
            "write_scope": ["src/**"],
            "allowed_tools": ["filesystem"],
            "prohibited_actions": ["merge requires separate authorization"],
            "secret_ids": [],
            "verification_required": true,
            "budget_tokens": 1_000,
            "budget_cost_microusd": 1_000_000
        });
        let plan = TaskGraphPlan {
            strategy: "single".to_owned(),
            max_nodes: 1,
            max_depth: 0,
            budget_tokens: 1_000,
            budget_cost_microusd: 1_000_000,
            tasks: vec![PlannedTask {
                key: "deliver".to_owned(),
                title: "Deliver".to_owned(),
                contract: TaskContract {
                    objective: "Deliver the issue".to_owned(),
                    expected_output: "A verified change".to_owned(),
                    source_repository: Some("owner/repo".to_owned()),
                    source_base_ref: Some("HEAD".to_owned()),
                    source_base_commit: Some("1111111111111111111111111111111111111111".to_owned()),
                    acceptance_tests: vec!["tests pass".to_owned()],
                    allowed_tools: vec!["filesystem".to_owned()],
                    prohibited_actions: vec!["merge requires separate authorization".to_owned()],
                    references: Vec::new(),
                    write_scope: vec!["src/**".to_owned()],
                    budget_tokens: 1_000,
                    budget_cost_microusd: 1_000_000,
                    deadline_at: None,
                    escalation: "ask the operator".to_owned(),
                    secret_refs: Vec::new(),
                    model: model.map(str::to_owned),
                    reasoning_effort: reasoning_effort.map(str::to_owned),
                    deliverable: None,
                },
                assigned_agent_id: Uuid::new_v4(),
                required_adapter: "codex".to_owned(),
                depends_on: Vec::new(),
                depth: 0,
                max_attempts: 1,
                verification_policy: VerificationPolicy {
                    checks: vec![VerifierCheck::Artifact { min_bytes: 1 }],
                    manual_gate: Some(ManualVerificationGate::IndependentReview {
                        roles: vec!["member".to_owned()],
                        exclude_requester: true,
                    }),
                },
            }],
        };
        (work_item, plan)
    }

    #[test]
    fn factory_claims_reject_expired_tokens_and_stale_versions() {
        let now = Utc::now();
        let (expired, token) = factory_work_item(now - Duration::seconds(1));
        assert!(
            ensure_active_factory_control(
                &expired,
                token,
                expired.claim_owner_id,
                token,
                expired.version,
                now,
            )
            .unwrap_err()
            .to_string()
            .contains("expired")
        );

        let (active, token) = factory_work_item(now + Duration::minutes(5));
        assert!(
            ensure_active_factory_control(
                &active,
                token,
                active.claim_owner_id,
                token,
                active.version - 1,
                now,
            )
            .unwrap_err()
            .to_string()
            .contains("version")
        );
    }

    #[test]
    fn factory_source_and_lease_validation_are_bounded() {
        let source = normalize_factory_source(FactorySourceInput {
            project_owner: " OwNeR ".to_owned(),
            project_number: 3,
            project_item_id: "PVTI_test".to_owned(),
            repository_owner: "OWNER".to_owned(),
            repository_name: "RePo".to_owned(),
            issue_number: 59,
            issue_node_id: "I_test".to_owned(),
            issue_url: "https://github.com/OWNER/RePo/issues/59/".to_owned(),
            title: " Factory claim ".to_owned(),
            revision: "2026-09-01T00:00:00Z".to_owned(),
        })
        .expect("valid source");
        assert_eq!(source.project_owner, "owner");
        assert_eq!(source.repository_owner, "owner");
        assert_eq!(source.repository_name, "repo");
        assert_eq!(source.issue_url, "https://github.com/owner/repo/issues/59");
        assert_eq!(source.title, "Factory claim");
        assert!(validate_factory_lease_seconds(29).is_err());
        assert!(validate_factory_lease_seconds(30).is_ok());
        assert!(validate_factory_lease_seconds(3_600).is_ok());
        assert!(validate_factory_lease_seconds(3_601).is_err());
    }

    #[test]
    fn new_factory_claim_policies_require_an_immutable_source_commit() {
        let normalized = normalize_factory_policy(json!({
            "source_base_ref": "HEAD",
            "source_base_commit": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        }))
        .expect("pinned source policy");
        assert_eq!(
            normalized
                .get("source_base_commit")
                .and_then(|value| value.as_str()),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(
            normalized
                .get("source_commit_upgrade_required")
                .and_then(|value| value.as_bool()),
            Some(false)
        );

        assert!(
            normalize_factory_policy(json!({"source_base_ref": "HEAD"}))
                .unwrap_err()
                .to_string()
                .contains("source_base_commit")
        );
        let migrated = normalize_factory_policy(json!({
            "source_base_ref": "HEAD",
            "source_commit_upgrade_required": true
        }))
        .expect("marked migrated legacy policy");
        assert!(
            ensure_new_factory_policy_is_pinned(&migrated)
                .unwrap_err()
                .to_string()
                .contains("source_base_commit")
        );
        assert!(
            normalize_factory_policy(json!({
                "source_base_ref": "HEAD",
                "source_base_commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "source_commit_upgrade_required": true
            }))
            .unwrap_err()
            .to_string()
            .contains("cannot be pinned and require a legacy upgrade")
        );
        assert!(
            normalize_factory_policy(json!({
                "source_base_ref": "HEAD",
                "source_base_commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "publication": {
                    "allowed": true,
                    "base_ref": "refs/tags/v1"
                }
            }))
            .unwrap_err()
            .to_string()
            .contains("HEAD or a branch ref")
        );
        assert!(
            normalize_factory_policy(json!({
                "source_base_ref": "HEAD",
                "source_base_commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "publication": {
                    "allowed": true,
                    "base_ref": "main\tbad"
                }
            }))
            .unwrap_err()
            .to_string()
            .contains("safe Git ref")
        );
        assert!(
            normalize_factory_policy(json!({
                "source_base_ref": "HEAD",
                "source_base_commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "write_scope": ["src/*.rs"]
            }))
            .unwrap_err()
            .to_string()
            .contains("invalid write scope")
        );
        assert!(
            normalize_factory_policy(json!({
                "source_base_ref": "HEAD",
                "source_base_commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "verification_policy": {
                    "checks": [{
                        "type": "file",
                        "path": "../outside.txt",
                        "min_bytes": 1
                    }],
                    "manual_gate": null
                }
            }))
            .unwrap_err()
            .to_string()
            .contains("factory verification policy is invalid")
        );
    }

    #[test]
    fn artifact_rejection_reasons_are_bounded_single_line_text() {
        let normalized = normalize_artifact_rejection_reason("object store failed\r\nretry\tlater");
        assert_eq!(normalized, "object store failed retry later");
        assert!(normalize_artifact_rejection_reason(&"🙂".repeat(1_000)).len() <= 2_000);
    }

    #[test]
    fn factory_policy_requires_every_task_to_retain_pinned_provider_settings() {
        let (work_item, plan) = factory_policy_plan(Some("gpt-5.6-sol"), Some("high"));
        validate_factory_plan_against_policy(&work_item, &plan)
            .expect("matching provider settings should pass");

        let mut missing_model = plan.clone();
        missing_model.tasks[0].contract.model = None;
        assert!(
            validate_factory_plan_against_policy(&work_item, &missing_model)
                .unwrap_err()
                .to_string()
                .contains("must preserve policy model")
        );

        let mut missing_reasoning = plan;
        missing_reasoning.tasks[0].contract.reasoning_effort = None;
        assert!(
            validate_factory_plan_against_policy(&work_item, &missing_reasoning)
                .unwrap_err()
                .to_string()
                .contains("must preserve policy reasoning effort")
        );

        let (_, mut missing_gate) = factory_policy_plan(Some("gpt-5.6-sol"), Some("high"));
        missing_gate.tasks[0].verification_policy.manual_gate = None;
        assert!(
            validate_factory_plan_against_policy(&work_item, &missing_gate)
                .unwrap_err()
                .to_string()
                .contains("requires a manual verification gate")
        );
    }

    #[test]
    fn factory_state_transitions_do_not_skip_governance_stages() {
        assert!(factory_transition_allowed(
            FactoryWorkItemState::MissionCreated,
            FactoryWorkItemState::Running,
        ));
        assert!(factory_transition_allowed(
            FactoryWorkItemState::Running,
            FactoryWorkItemState::Verified,
        ));
        assert!(factory_transition_allowed(
            FactoryWorkItemState::AwaitingApproval,
            FactoryWorkItemState::Blocked,
        ));
        assert!(factory_transition_allowed(
            FactoryWorkItemState::MissionCreated,
            FactoryWorkItemState::VerificationFailed,
        ));
        assert!(factory_transition_allowed(
            FactoryWorkItemState::MissionCreated,
            FactoryWorkItemState::AwaitingApproval,
        ));
        assert!(factory_transition_allowed(
            FactoryWorkItemState::Blocked,
            FactoryWorkItemState::VerificationFailed,
        ));
        assert!(factory_transition_allowed(
            FactoryWorkItemState::Blocked,
            FactoryWorkItemState::AwaitingApproval,
        ));
        assert!(factory_transition_allowed(
            FactoryWorkItemState::Verified,
            FactoryWorkItemState::Publishing,
        ));
        assert!(!factory_transition_allowed(
            FactoryWorkItemState::MissionCreated,
            FactoryWorkItemState::Published,
        ));
        assert!(!factory_transition_allowed(
            FactoryWorkItemState::Published,
            FactoryWorkItemState::Running,
        ));
    }
}
