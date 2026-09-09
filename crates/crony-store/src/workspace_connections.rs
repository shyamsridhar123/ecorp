//! Room-scoped saved connections and actor-private, durable native setup.
use super::*;
use crony_domain::{
    GitHubAccountSource, WorkspaceConnection, WorkspaceConnectionConfiguration,
    WorkspaceConnectionStatus, WorkspaceConnections, WorkspaceRepositorySetup,
    WorkspaceSetupAction, WorkspaceSetupCommand, WorkspaceSetupOperation, WorkspaceSetupReport,
    WorkspaceSetupStatus, WorkspaceSourceIdentity,
};

const SETUP_CAPABILITY: &str = "workspace-setup-v1";
const MAX_CONNECTIONS: i64 = 200;
const MAX_REPORT_BYTES: usize = 262_144;

#[derive(Debug, Clone)]
pub struct CreateWorkspaceConnectionInput {
    pub corp_id: Uuid,
    pub room_id: Uuid,
    pub actor_id: Uuid,
    pub runner_id: String,
    pub label: String,
    pub configuration: WorkspaceConnectionConfiguration,
    pub idempotency_key: String,
}

#[derive(Debug, Clone)]
pub struct WorkspaceSetupInput {
    pub corp_id: Uuid,
    pub room_id: Uuid,
    pub actor_id: Uuid,
    pub runner_id: String,
    pub action: WorkspaceSetupAction,
    pub idempotency_key: String,
}

#[derive(Debug, Clone)]
pub struct WorkspaceSetupMutation {
    pub connection: Option<WorkspaceConnection>,
    pub operation: WorkspaceSetupOperation,
    pub events: Vec<DomainEvent>,
    pub replayed: bool,
}

fn text(value: &str, label: &str, limit: usize) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > limit || value.chars().any(char::is_control) {
        return Err(anyhow!(
            "{label} must be non-empty printable text within {limit} bytes"
        ));
    }
    Ok(value.to_owned())
}

fn repository(value: &str) -> Result<String> {
    let value = value.trim();
    let value = value.strip_prefix("https://github.com/").unwrap_or(value);
    let value = value
        .trim_end_matches('/')
        .strip_suffix(".git")
        .unwrap_or(value.trim_end_matches('/'));
    let parts = value.split('/').collect::<Vec<_>>();
    if parts.len() != 2
        || parts
            .iter()
            .any(|part| part.is_empty() || part.len() > 100 || matches!(*part, "." | ".."))
        || !parts[0]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || parts[0].starts_with('-')
        || parts[0].ends_with('-')
        || !parts[1]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(anyhow!(
            "repository must be owner/name or an HTTPS github.com repository URL without credentials"
        ));
    }
    Ok(value.to_ascii_lowercase())
}

fn source_identity(source: &WorkspaceSourceIdentity) -> Result<()> {
    if repository(&source.repository)? != source.repository {
        return Err(anyhow!("connection source repository must be canonical"));
    }
    text(&source.base_ref, "source ref", 240)?;
    validate_factory_base_ref(&source.base_ref)?;
    validate_factory_base_commit(&source.base_commit)?;
    if source.base_commit != source.base_commit.to_ascii_lowercase() {
        return Err(anyhow!("connection source commit must be canonical"));
    }
    if let Some(id) = &source.repository_id {
        text(id, "repository identity", 160)?;
    }
    Ok(())
}

fn shared_connection_detail(status: WorkspaceConnectionStatus) -> &'static str {
    match status {
        WorkspaceConnectionStatus::Connecting => "Checking the repository and coding agent.",
        WorkspaceConnectionStatus::Ready => {
            "The repository and native coding-agent connection are ready."
        }
        WorkspaceConnectionStatus::NeedsSignIn => {
            "Sign-in is needed on the selected machine. The connection owner can continue setup."
        }
        WorkspaceConnectionStatus::NotInstalled => {
            "The selected coding agent needs installation or configuration on this machine."
        }
        WorkspaceConnectionStatus::Offline => {
            "The selected machine is offline. This connection remains saved."
        }
        WorkspaceConnectionStatus::Incompatible => {
            "This execution environment needs attention before it can run the project."
        }
        WorkspaceConnectionStatus::Failed => {
            "The connection check failed. Its owner can review the private details and retry."
        }
    }
}

fn normalize_configuration(
    mut configuration: WorkspaceConnectionConfiguration,
) -> Result<WorkspaceConnectionConfiguration> {
    match &mut configuration.repository {
        WorkspaceRepositorySetup::GitHub {
            repository: repo,
            repository_id,
            base_ref,
            ..
        } => {
            *repo = repository(repo)?;
            *base_ref = text(base_ref, "source ref", 240)?;
            validate_factory_base_ref(base_ref)?;
            if let Some(id) = repository_id {
                *id = text(id, "repository identity", 160)?;
            }
        }
        WorkspaceRepositorySetup::Local {
            directory,
            base_ref,
        } => {
            *directory = text(directory, "repository directory", 2_048)?;
            let bytes = directory.as_bytes();
            let absolute = directory.starts_with('/')
                || (bytes.len() >= 3
                    && bytes[0].is_ascii_alphabetic()
                    && bytes[1] == b':'
                    && matches!(bytes[2], b'/' | b'\\'));
            if !absolute
                || directory.starts_with("//")
                || directory.starts_with("\\\\")
                || directory.contains("://")
            {
                return Err(anyhow!(
                    "choose an absolute local repository directory on the selected machine"
                ));
            }
            *base_ref = text(base_ref, "source ref", 240)?;
            validate_factory_base_ref(base_ref)?;
        }
        WorkspaceRepositorySetup::Advertised { source } => {
            source.repository = repository(&source.repository)?;
            source.base_commit = source.base_commit.to_ascii_lowercase();
            source_identity(source)?;
        }
    }
    Ok(configuration)
}

fn action_kind(action: &WorkspaceSetupAction) -> &'static str {
    match action {
        WorkspaceSetupAction::InspectGitHub { .. } => "inspect_github",
        WorkspaceSetupAction::SignInGitHub => "sign_in_github",
        WorkspaceSetupAction::ListGitHubRepositories { .. } => "list_github_repositories",
        WorkspaceSetupAction::Connect { .. } => "connect",
        WorkspaceSetupAction::Test { .. } => "test",
        WorkspaceSetupAction::SignInAgent { .. } => "sign_in_agent",
    }
}

fn needs_machine_authority(action: &WorkspaceSetupAction) -> bool {
    match action {
        WorkspaceSetupAction::InspectGitHub { account }
        | WorkspaceSetupAction::ListGitHubRepositories { account } => {
            *account == GitHubAccountSource::Machine
        }
        WorkspaceSetupAction::Connect { configuration, .. } => {
            configuration.use_system_installation
                || configuration.use_machine_account == Some(true)
                || !matches!(
                    configuration.repository,
                    WorkspaceRepositorySetup::GitHub {
                        account: GitHubAccountSource::Personal,
                        ..
                    } | WorkspaceRepositorySetup::Advertised { .. }
                )
        }
        _ => false,
    }
}

async fn actor_role_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    room_id: Uuid,
    actor_id: Uuid,
) -> Result<String> {
    assert_room_membership_tx(tx, corp_id, room_id, actor_id).await?;
    let role: Option<String> =
        sqlx::query_scalar("SELECT role FROM actors WHERE corp_id=$1 AND id=$2 AND kind='human'")
            .bind(corp_id)
            .bind(actor_id)
            .fetch_optional(&mut **tx)
            .await?;
    match role {
        Some(role) if matches!(role.as_str(), "owner" | "admin" | "manager" | "member") => Ok(role),
        _ => Err(anyhow!(
            "forbidden: connection setup requires a human room operator"
        )),
    }
}

async fn authorize_setup_tx(
    tx: &mut Transaction<'_, Postgres>,
    input: &WorkspaceSetupInput,
) -> Result<()> {
    let role = actor_role_tx(tx, input.corp_id, input.room_id, input.actor_id).await?;
    if needs_machine_authority(&input.action) && !matches!(role.as_str(), "owner" | "admin") {
        return Err(anyhow!(
            "forbidden: an owner or admin must authorize this machine's account or local checkout"
        ));
    }
    let runner: Option<Value> =
        sqlx::query_scalar("SELECT capabilities FROM runner_nodes WHERE id=$1 AND corp_id=$2")
            .bind(&input.runner_id)
            .bind(input.corp_id)
            .fetch_optional(&mut **tx)
            .await?;
    let capable = runner
        .as_ref()
        .and_then(Value::as_array)
        .is_some_and(|caps| {
            caps.iter()
                .any(|cap| cap["name"] == SETUP_CAPABILITY && cap["available"] == true)
        });
    if !capable {
        return Err(anyhow!(
            "this runner does not support connection setup; update its ECorp runner before connecting a new project"
        ));
    }
    Ok(())
}

async fn connection_row_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    id: Uuid,
    lock: bool,
) -> Result<sqlx::postgres::PgRow> {
    let lock_clause = if lock {
        " FOR UPDATE OF connection"
    } else {
        ""
    };
    sqlx::query(&format!(
        "SELECT connection.*, runner.status='connected' AS runner_connected
         FROM workspace_connections connection
         JOIN runner_nodes runner ON runner.id=connection.runner_id AND runner.corp_id=connection.corp_id
         WHERE connection.corp_id=$1 AND connection.id=$2{lock_clause}"
    )).bind(corp_id).bind(id).fetch_optional(&mut **tx).await?
        .context("connection was not found in this Corp")
}

fn map_connection(row: &sqlx::postgres::PgRow) -> Result<WorkspaceConnection> {
    let source = row
        .get::<Option<String>, _>("source_repository")
        .map(|repository| WorkspaceSourceIdentity {
            repository,
            repository_id: row.get("source_repository_id"),
            base_ref: row.get("source_base_ref"),
            base_commit: row.get("source_base_commit"),
        });
    Ok(WorkspaceConnection {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        room_id: row.get("room_id"),
        created_by: row.get("created_by"),
        runner_id: row.get("runner_id"),
        label: row.get("label"),
        agent: serde_json::from_value(json!(row.get::<String, _>("agent")))?,
        source,
        status: serde_json::from_value(json!(row.get::<String, _>("status")))?,
        detail: row.get("detail"),
        models: serde_json::from_value(row.get("models"))?,
        version: row.get("version"),
        last_checked_at: row.get("last_checked_at"),
        runner_connected: row.get("runner_connected"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn map_operation(row: &sqlx::postgres::PgRow) -> Result<WorkspaceSetupOperation> {
    Ok(WorkspaceSetupOperation {
        id: row.get("id"),
        corp_id: row.get("corp_id"),
        room_id: row.get("room_id"),
        actor_id: row.get("actor_id"),
        runner_id: row.get("runner_id"),
        connection_id: row.get("connection_id"),
        kind: row.get("kind"),
        status: serde_json::from_value(json!(row.get::<String, _>("status")))?,
        report: row
            .get::<Option<Value>, _>("report")
            .map(serde_json::from_value)
            .transpose()?,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        expires_at: row.get("expires_at"),
    })
}

async fn setup_event_tx(
    tx: &mut Transaction<'_, Postgres>,
    operation: &WorkspaceSetupOperation,
) -> Result<Option<DomainEvent>> {
    // Native sign-in codes, account catalogues, paths and diagnostics are actor-private.
    // The shared event carries only a refresh hint for the linked project connection.
    let Some(connection_id) = operation.connection_id else {
        return Ok(None);
    };
    let mut event = NewEvent::new(
        operation.corp_id,
        Some(operation.actor_id),
        "workspace.connection_updated",
        "workspace_connection",
        connection_id,
        format!(
            "workspace-setup:{}:{}",
            operation.id,
            operation.status.as_str()
        ),
        json!({"connection_id":connection_id,"operation_id":operation.id,"status":operation.status.as_str()}),
    );
    event.room_id = Some(operation.room_id);
    event.visibility = "room".to_owned();
    append_event_tx(tx, event).await
}

async fn operation_by_key_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    actor_id: Uuid,
    key: &str,
    request: &Value,
) -> Result<Option<WorkspaceSetupOperation>> {
    let row = sqlx::query(
        "SELECT * FROM workspace_setup_operations WHERE corp_id=$1 AND actor_id=$2 AND idempotency_key=$3",
    ).bind(corp_id).bind(actor_id).bind(key).fetch_optional(&mut **tx).await?;
    row.map(|row| {
        if row.get::<Value, _>("request") != *request {
            return Err(anyhow!(
                "conflict: this connection request key belongs to different settings"
            ));
        }
        map_operation(&row)
    })
    .transpose()
}

async fn insert_operation_tx(
    tx: &mut Transaction<'_, Postgres>,
    input: &WorkspaceSetupInput,
    expected_version: Option<i64>,
    request: &Value,
) -> Result<WorkspaceSetupOperation> {
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM workspace_setup_operations WHERE corp_id=$1 AND actor_id=$2
         AND status IN ('queued','running','needs_sign_in') AND expires_at>now()",
    )
    .bind(input.corp_id)
    .bind(input.actor_id)
    .fetch_one(&mut **tx)
    .await?;
    if active >= 8 {
        return Err(anyhow!(
            "finish or cancel an existing connection check before starting another"
        ));
    }
    let row = sqlx::query(
        "INSERT INTO workspace_setup_operations
          (id,corp_id,room_id,actor_id,runner_id,connection_id,expected_connection_version,
           idempotency_key,kind,action,request,expires_at)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,now()+interval '15 minutes') RETURNING *",
    )
    .bind(Uuid::new_v4())
    .bind(input.corp_id)
    .bind(input.room_id)
    .bind(input.actor_id)
    .bind(&input.runner_id)
    .bind(input.action.connection_id())
    .bind(expected_version)
    .bind(&input.idempotency_key)
    .bind(action_kind(&input.action))
    .bind(serde_json::to_value(&input.action)?)
    .bind(request)
    .fetch_one(&mut **tx)
    .await?;
    map_operation(&row)
}

async fn fail_setup_tx(
    tx: &mut Transaction<'_, Postgres>,
    operation: &WorkspaceSetupOperation,
    expected_version: Option<i64>,
    detail: &str,
) -> Result<()> {
    let report = WorkspaceSetupReport {
        status: WorkspaceSetupStatus::Failed,
        detail: detail.to_owned(),
        connection_status: operation
            .connection_id
            .map(|_| WorkspaceConnectionStatus::Failed),
        source: None,
        models: vec![],
        account_login: None,
        sign_in: None,
        repositories: vec![],
    };
    sqlx::query("UPDATE workspace_setup_operations SET status='failed',report=$3,updated_at=now() WHERE corp_id=$1 AND id=$2")
        .bind(operation.corp_id).bind(operation.id).bind(serde_json::to_value(report)?)
        .execute(&mut **tx).await?;
    if let Some(connection_id) = operation.connection_id {
        sqlx::query(
            "UPDATE workspace_connections SET status='failed',detail=$4,updated_at=now()
            WHERE corp_id=$1 AND id=$2 AND version=$3",
        )
        .bind(operation.corp_id)
        .bind(connection_id)
        .bind(expected_version)
        .bind(detail)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn expire_setup_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    runner_id: &str,
) -> Result<()> {
    let rows = sqlx::query(
        "SELECT * FROM workspace_setup_operations
        WHERE corp_id=$1 AND runner_id=$2 AND status IN ('queued','running','needs_sign_in')
          AND expires_at<=now() ORDER BY created_at,id LIMIT 64 FOR UPDATE",
    )
    .bind(corp_id)
    .bind(runner_id)
    .fetch_all(&mut **tx)
    .await?;
    for row in rows {
        let operation = map_operation(&row)?;
        fail_setup_tx(
            tx,
            &operation,
            row.get("expected_connection_version"),
            "This connection check expired. Check the saved connection again.",
        )
        .await?;
    }
    Ok(())
}

impl PgStore {
    pub async fn workspace_runtime_bindings(
        &self,
        corp_id: Uuid,
        runner_id: &str,
        ids: &[Uuid],
    ) -> Result<Vec<WorkspaceConnection>> {
        if ids.len() > 512 {
            return Err(anyhow!("too many workspace bindings in one runner update"));
        }
        let rows=sqlx::query(
            "SELECT connection.*,runner.status='connected' AS runner_connected
             FROM workspace_connections connection
             JOIN runner_nodes runner ON runner.id=connection.runner_id AND runner.corp_id=connection.corp_id
             WHERE connection.corp_id=$1 AND connection.runner_id=$2 AND connection.id=ANY($3)",
        ).bind(corp_id).bind(runner_id).bind(ids).fetch_all(&self.pool).await?;
        rows.iter().map(map_connection).collect()
    }

    pub async fn visible_workspace_connections(
        &self,
        corp_id: Uuid,
        actor_id: Uuid,
    ) -> Result<HashSet<Uuid>> {
        let ids:Vec<Uuid>=sqlx::query_scalar(
            "SELECT connection.id FROM workspace_connections connection
             JOIN room_memberships member ON member.room_id=connection.room_id AND member.actor_id=$2
             JOIN actors actor ON actor.id=member.actor_id AND actor.corp_id=connection.corp_id
             WHERE connection.corp_id=$1 AND actor.kind='human'
               AND actor.role IN ('owner','admin','manager','member')",
        ).bind(corp_id).bind(actor_id).fetch_all(&self.pool).await?;
        Ok(ids.into_iter().collect())
    }

    pub async fn update_runner_capabilities(
        &self,
        corp_id: Uuid,
        runner_id: &str,
        epoch: Uuid,
        capabilities: Value,
    ) -> Result<(bool, Option<DomainEvent>)> {
        let mut tx = self.pool.begin().await?;
        let digest = hex::encode(Sha256::digest(serde_json::to_vec(&capabilities)?));
        let result = sqlx::query(
            "UPDATE runner_nodes SET capabilities=$4 WHERE corp_id=$1 AND id=$2
             AND connection_epoch=$3 AND status='connected'",
        )
        .bind(corp_id)
        .bind(runner_id)
        .bind(epoch)
        .bind(capabilities)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            return Ok((false, None));
        }
        let event = append_event_tx(
            &mut tx,
            NewEvent::new(
                corp_id,
                None,
                "runner.capabilities_updated",
                "runner",
                epoch,
                format!("runner-capabilities:{runner_id}:{epoch}:{digest}"),
                json!({"runner_id":runner_id}),
            ),
        )
        .await?;
        tx.commit().await?;
        Ok((true, event))
    }

    pub async fn workspace_connection_settings(
        &self,
        corp_id: Uuid,
        actor_id: Uuid,
        connection_id: Uuid,
    ) -> Result<(WorkspaceConnection, WorkspaceConnectionConfiguration)> {
        let mut tx = self.pool.begin().await?;
        let row = connection_row_tx(&mut tx, corp_id, connection_id, false).await?;
        actor_role_tx(&mut tx, corp_id, row.get("room_id"), actor_id).await?;
        let connection = map_connection(&row)?;
        let configuration = serde_json::from_value(row.get("configuration"))?;
        tx.commit().await?;
        Ok((connection, configuration))
    }

    pub async fn workspace_sign_in_operation(
        &self,
        corp_id: Uuid,
        actor_id: Uuid,
        operation_id: Uuid,
        input_id: Uuid,
    ) -> Result<WorkspaceSetupOperation> {
        let operation = self
            .workspace_setup_operation(corp_id, actor_id, operation_id)
            .await?
            .context("native sign-in operation was not found")?;
        let instruction = operation
            .report
            .as_ref()
            .and_then(|report| report.sign_in.as_ref())
            .context("this operation is not waiting for a native authorization code")?;
        if operation.status != WorkspaceSetupStatus::NeedsSignIn
            || operation.expires_at <= Utc::now()
            || instruction.provider != "claude-code"
            || instruction.input_kind
                != Some(crony_domain::NativeSignInInputKind::AuthorizationCode)
            || instruction.input_id != Some(input_id)
            || instruction.expires_at <= Utc::now()
        {
            return Err(anyhow!(
                "conflict: this native sign-in step changed or expired"
            ));
        }
        Ok(operation)
    }

    pub async fn workspace_connections(
        &self,
        corp_id: Uuid,
        room_id: Uuid,
        actor_id: Uuid,
    ) -> Result<WorkspaceConnections> {
        let mut tx = self.pool.begin().await?;
        actor_role_tx(&mut tx, corp_id, room_id, actor_id).await?;
        let connections = sqlx::query(
            "SELECT connection.*, runner.status='connected' AS runner_connected
             FROM workspace_connections connection
             JOIN runner_nodes runner ON runner.id=connection.runner_id AND runner.corp_id=connection.corp_id
             WHERE connection.corp_id=$1 AND connection.room_id=$2 ORDER BY connection.created_at,connection.id LIMIT $3",
        ).bind(corp_id).bind(room_id).bind(MAX_CONNECTIONS).fetch_all(&mut *tx).await?
            .iter().map(map_connection).collect::<Result<Vec<_>>>()?;
        let operations = sqlx::query(
            "SELECT * FROM workspace_setup_operations WHERE corp_id=$1 AND room_id=$2 AND actor_id=$3
             ORDER BY created_at DESC,id DESC LIMIT 30",
        ).bind(corp_id).bind(room_id).bind(actor_id).fetch_all(&mut *tx).await?
            .iter().map(map_operation).collect::<Result<Vec<_>>>()?;
        let selected_connection_id = sqlx::query_scalar(
            "SELECT preference.connection_id FROM workspace_connection_preferences preference
             JOIN workspace_connections connection ON connection.id=preference.connection_id
               AND connection.corp_id=preference.corp_id AND connection.room_id=preference.room_id
             WHERE preference.corp_id=$1 AND preference.room_id=$2 AND preference.actor_id=$3",
        )
        .bind(corp_id)
        .bind(room_id)
        .bind(actor_id)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(WorkspaceConnections {
            connections,
            operations,
            selected_connection_id,
        })
    }

    pub async fn workspace_setup_operation(
        &self,
        corp_id: Uuid,
        actor_id: Uuid,
        operation_id: Uuid,
    ) -> Result<Option<WorkspaceSetupOperation>> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT * FROM workspace_setup_operations WHERE corp_id=$1 AND actor_id=$2 AND id=$3",
        )
        .bind(corp_id)
        .bind(actor_id)
        .bind(operation_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else { return Ok(None) };
        actor_role_tx(&mut tx, corp_id, row.get("room_id"), actor_id).await?;
        let result = map_operation(&row)?;
        tx.commit().await?;
        Ok(Some(result))
    }

    pub async fn select_workspace_connection(
        &self,
        corp_id: Uuid,
        room_id: Uuid,
        actor_id: Uuid,
        connection_id: Uuid,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        actor_role_tx(&mut tx, corp_id, room_id, actor_id).await?;
        let row = connection_row_tx(&mut tx, corp_id, connection_id, false).await?;
        if row.get::<Uuid, _>("room_id") != room_id {
            return Err(anyhow!("forbidden: connection belongs to another room"));
        }
        // Saving an offline choice is intentional; it grants no dispatch authority.
        sqlx::query(
            "INSERT INTO workspace_connection_preferences(corp_id,room_id,actor_id,connection_id)
             VALUES($1,$2,$3,$4) ON CONFLICT(corp_id,room_id,actor_id)
             DO UPDATE SET connection_id=EXCLUDED.connection_id,updated_at=now()",
        )
        .bind(corp_id)
        .bind(room_id)
        .bind(actor_id)
        .bind(connection_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn create_workspace_connection(
        &self,
        mut input: CreateWorkspaceConnectionInput,
    ) -> Result<WorkspaceSetupMutation> {
        input.label = text(&input.label, "connection name", 120)?;
        input.runner_id = text(&input.runner_id, "runner identity", 160)?;
        input.idempotency_key = text(&input.idempotency_key, "connection request key", 160)?;
        input.configuration = normalize_configuration(input.configuration)?;
        let request = json!({"room_id":input.room_id,"runner_id":input.runner_id,
            "label":input.label,"configuration":input.configuration,"kind":"connect"});
        let id = Uuid::new_v4();
        let setup = WorkspaceSetupInput {
            corp_id: input.corp_id,
            room_id: input.room_id,
            actor_id: input.actor_id,
            runner_id: input.runner_id.clone(),
            idempotency_key: input.idempotency_key.clone(),
            action: WorkspaceSetupAction::Connect {
                connection_id: id,
                configuration: input.configuration.clone(),
            },
        };
        let mut tx = self.pool.begin().await?;
        authorize_setup_tx(&mut tx, &setup).await?;
        lock_factory_keys_tx(
            &mut tx,
            &[format!(
                "workspace-setup:{}:{}:{}",
                input.corp_id, input.actor_id, input.idempotency_key
            )],
        )
        .await?;
        if let Some(operation) = operation_by_key_tx(
            &mut tx,
            input.corp_id,
            input.actor_id,
            &input.idempotency_key,
            &request,
        )
        .await?
        {
            let connection = map_connection(
                &connection_row_tx(
                    &mut tx,
                    input.corp_id,
                    operation
                        .connection_id
                        .context("connection replay lost its connection")?,
                    false,
                )
                .await?,
            )?;
            tx.commit().await?;
            return Ok(WorkspaceSetupMutation {
                connection: Some(connection),
                operation,
                events: vec![],
                replayed: true,
            });
        }
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM workspace_connections WHERE corp_id=$1 AND room_id=$2",
        )
        .bind(input.corp_id)
        .bind(input.room_id)
        .fetch_one(&mut *tx)
        .await?;
        if count >= MAX_CONNECTIONS {
            return Err(anyhow!("this room has reached its saved connection limit"));
        }
        sqlx::query(
            "INSERT INTO workspace_connections(id,corp_id,room_id,created_by,runner_id,label,agent,configuration,status,detail)
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,'connecting','Waiting for the selected machine')",
        ).bind(id).bind(input.corp_id).bind(input.room_id).bind(input.actor_id).bind(&input.runner_id)
            .bind(&input.label).bind(input.configuration.agent.as_str()).bind(serde_json::to_value(&input.configuration)?)
            .execute(&mut *tx).await?;
        let operation = insert_operation_tx(&mut tx, &setup, Some(1), &request).await?;
        let connection =
            map_connection(&connection_row_tx(&mut tx, input.corp_id, id, false).await?)?;
        let events = setup_event_tx(&mut tx, &operation)
            .await?
            .into_iter()
            .collect();
        tx.commit().await?;
        Ok(WorkspaceSetupMutation {
            connection: Some(connection),
            operation,
            events,
            replayed: false,
        })
    }

    pub async fn begin_workspace_setup(
        &self,
        mut input: WorkspaceSetupInput,
    ) -> Result<WorkspaceSetupMutation> {
        input.runner_id = text(&input.runner_id, "runner identity", 160)?;
        input.idempotency_key = text(&input.idempotency_key, "connection request key", 160)?;
        if matches!(input.action, WorkspaceSetupAction::Connect { .. }) {
            return Err(anyhow!("use the saved-connection creation path"));
        }
        let request = json!({"kind":action_kind(&input.action),"room_id":input.room_id,
        "runner_id":input.runner_id,"connection_id":input.action.connection_id(),
        "account":match &input.action {
            WorkspaceSetupAction::InspectGitHub{account}|WorkspaceSetupAction::ListGitHubRepositories{account}=>Some(account),
            _=>None
        }});
        let mut tx = self.pool.begin().await?;
        authorize_setup_tx(&mut tx, &input).await?;
        expire_setup_tx(&mut tx, input.corp_id, &input.runner_id).await?;
        let mut keys = vec![format!(
            "workspace-setup:{}:{}:{}",
            input.corp_id, input.actor_id, input.idempotency_key
        )];
        if let Some(id) = input.action.connection_id() {
            keys.push(format!("workspace-connection:{}:{id}", input.corp_id));
        }
        lock_factory_keys_tx(&mut tx, &keys).await?;
        if let Some(operation) = operation_by_key_tx(
            &mut tx,
            input.corp_id,
            input.actor_id,
            &input.idempotency_key,
            &request,
        )
        .await?
        {
            let connection = if let Some(id) = operation.connection_id {
                Some(map_connection(
                    &connection_row_tx(&mut tx, input.corp_id, id, false).await?,
                )?)
            } else {
                None
            };
            tx.commit().await?;
            return Ok(WorkspaceSetupMutation {
                connection,
                operation,
                events: vec![],
                replayed: true,
            });
        }
        let expected_version = if let Some(id) = input.action.connection_id() {
            let row = connection_row_tx(&mut tx, input.corp_id, id, true).await?;
            if row.get::<Uuid, _>("room_id") != input.room_id
                || row.get::<String, _>("runner_id") != input.runner_id
            {
                return Err(anyhow!(
                    "forbidden: connection belongs to another room or machine"
                ));
            }
            if matches!(input.action, WorkspaceSetupAction::SignInAgent { .. }) {
                if row.get::<Uuid, _>("created_by") != input.actor_id {
                    return Err(anyhow!(
                        "forbidden: only the connection owner can change its sign-in"
                    ));
                }
                let active:bool=sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM runs WHERE corp_id=$1 AND workspace_connection_id=$2
                     AND status IN ('provisioning','starting','running','waiting_for_input','waiting_for_approval','verifying'))",
                ).bind(input.corp_id).bind(id).fetch_one(&mut *tx).await?;
                if active {
                    return Err(anyhow!(
                        "wait for current work before changing this agent's sign-in"
                    ));
                }
            }
            let active=sqlx::query(
                "SELECT * FROM workspace_setup_operations WHERE corp_id=$1 AND connection_id=$2
                 AND status IN ('queued','running','needs_sign_in') ORDER BY created_at DESC LIMIT 1",
            ).bind(input.corp_id).bind(id).fetch_optional(&mut *tx).await?;
            if let Some(active) = active {
                let operation = map_operation(&active)?;
                if operation.actor_id == input.actor_id
                    && operation.kind == action_kind(&input.action)
                {
                    let connection = Some(map_connection(&row)?);
                    tx.commit().await?;
                    return Ok(WorkspaceSetupMutation {
                        connection,
                        operation,
                        events: vec![],
                        replayed: true,
                    });
                }
                return Err(anyhow!("a connection check is already running"));
            }
            let configuration: WorkspaceConnectionConfiguration =
                serde_json::from_value(row.get("configuration"))?;
            input.action = if matches!(input.action, WorkspaceSetupAction::Test { .. }) {
                WorkspaceSetupAction::Test {
                    connection_id: id,
                    configuration,
                }
            } else {
                WorkspaceSetupAction::SignInAgent {
                    connection_id: id,
                    configuration,
                }
            };
            let version:i64=sqlx::query_scalar(
                "UPDATE workspace_connections SET status='connecting',detail='Checking connection',version=version+1,updated_at=now()
                 WHERE corp_id=$1 AND id=$2 RETURNING version",
            ).bind(input.corp_id).bind(id).fetch_one(&mut *tx).await?;
            Some(version)
        } else {
            None
        };
        let operation = insert_operation_tx(&mut tx, &input, expected_version, &request).await?;
        let connection = if let Some(id) = operation.connection_id {
            Some(map_connection(
                &connection_row_tx(&mut tx, input.corp_id, id, false).await?,
            )?)
        } else {
            None
        };
        let events = setup_event_tx(&mut tx, &operation)
            .await?
            .into_iter()
            .collect();
        tx.commit().await?;
        Ok(WorkspaceSetupMutation {
            connection,
            operation,
            events,
            replayed: false,
        })
    }

    pub async fn pending_workspace_setup(
        &self,
        corp_id: Uuid,
        runner_id: &str,
        connection_epoch: Uuid,
    ) -> Result<Vec<WorkspaceSetupCommand>> {
        let mut tx = self.pool.begin().await?;
        let current:Option<Uuid>=sqlx::query_scalar(
            "SELECT connection_epoch FROM runner_nodes WHERE corp_id=$1 AND id=$2 AND status='connected' FOR SHARE",
        ).bind(corp_id).bind(runner_id).fetch_optional(&mut *tx).await?;
        if current != Some(connection_epoch) {
            return Ok(vec![]);
        }
        expire_setup_tx(&mut tx, corp_id, runner_id).await?;
        let rows=sqlx::query(
            "SELECT operation.*, connection.created_by AS connection_owner_id
             FROM workspace_setup_operations operation
             LEFT JOIN workspace_connections connection ON connection.id=operation.connection_id AND connection.corp_id=operation.corp_id
             WHERE operation.corp_id=$1 AND operation.runner_id=$2
               AND operation.status IN ('queued','running','needs_sign_in') AND operation.expires_at>now()
             ORDER BY operation.created_at,operation.id LIMIT 32",
        ).bind(corp_id).bind(runner_id).fetch_all(&mut *tx).await?;
        let mut commands = Vec::new();
        for row in rows {
            let action: WorkspaceSetupAction = serde_json::from_value(row.get("action"))?;
            let input = WorkspaceSetupInput {
                corp_id,
                room_id: row.get("room_id"),
                actor_id: row.get("actor_id"),
                runner_id: runner_id.to_owned(),
                action: action.clone(),
                idempotency_key: row.get("idempotency_key"),
            };
            if let Err(error) = authorize_setup_tx(&mut tx, &input).await {
                if error.downcast_ref::<sqlx::Error>().is_some() {
                    return Err(error);
                }
                let detail = error.to_string();
                if !detail.starts_with("forbidden:")
                    && !detail.starts_with("this runner does not support")
                {
                    return Err(error);
                }
                fail_setup_tx(
                    &mut tx,
                    &map_operation(&row)?,
                    row.get("expected_connection_version"),
                    "Connection setup is no longer authorized for this room or machine.",
                )
                .await?;
                continue;
            }
            commands.push(WorkspaceSetupCommand {
                operation_id: row.get("id"),
                corp_id,
                room_id: row.get("room_id"),
                actor_id: row.get("actor_id"),
                connection_owner_id: row.get("connection_owner_id"),
                runner_id: runner_id.to_owned(),
                expected_connection_version: row.get("expected_connection_version"),
                action,
                expires_at: row.get("expires_at"),
            });
        }
        tx.commit().await?;
        Ok(commands)
    }
}

pub(super) async fn plan_room_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    actor_id: Uuid,
    plan: &TaskGraphPlan,
) -> Result<Option<Uuid>> {
    let mut room_id = None;
    for task in &plan.tasks {
        let Some(connection_id) = task.contract.workspace_connection_id else {
            if room_id.is_some() {
                return Err(anyhow!(
                    "a connected project plan cannot mix unbound execution environments"
                ));
            }
            continue;
        };
        let row = connection_row_tx(tx, corp_id, connection_id, true).await?;
        let connection = map_connection(&row)?;
        actor_role_tx(tx, corp_id, connection.room_id, actor_id).await?;
        if room_id.is_some_and(|room| room != connection.room_id) {
            return Err(anyhow!("a mission cannot span unrelated project rooms"));
        }
        if connection.status != WorkspaceConnectionStatus::Ready || !connection.runner_connected {
            return Err(anyhow!(
                "the selected project connection is not ready; test or reconnect its machine"
            ));
        }
        let source = connection
            .source
            .context("connection has no checked repository")?;
        if task.required_adapter != connection.agent.as_str()
            || task.contract.source_repository.as_ref() != Some(&source.repository)
            || task.contract.source_base_ref.as_ref() != Some(&source.base_ref)
            || task.contract.source_base_commit.as_ref() != Some(&source.base_commit)
        {
            return Err(anyhow!(
                "mission adapter or source does not match the saved connection"
            ));
        }
        room_id = Some(connection.room_id);
    }
    if room_id.is_some()
        && plan
            .tasks
            .iter()
            .any(|task| task.contract.workspace_connection_id.is_none())
    {
        return Err(anyhow!(
            "every task must retain its project execution connection"
        ));
    }
    Ok(room_id)
}

/// Called inside the same transaction that creates a new run. Resume/recovery
/// inherits the original binding; mutable task metadata cannot switch its home.
pub(super) async fn bind_new_run_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    run_id: Uuid,
) -> Result<Option<Uuid>> {
    let row=sqlx::query(
        "SELECT run.task_id,run.runner_id,run.resumed_from_run_id,run.execution_mode,
           run.source_repository,run.source_base_ref,run.source_base_commit,
           mission.room_id,task.required_adapter,task.contract,
           source.id AS source_id,source.workspace_connection_id AS source_connection
         FROM runs run JOIN tasks task ON task.id=run.task_id AND task.corp_id=run.corp_id
         JOIN missions mission ON mission.id=task.mission_id AND mission.corp_id=run.corp_id
         LEFT JOIN runs source ON source.id=run.resumed_from_run_id AND source.corp_id=run.corp_id
           AND source.task_id=run.task_id AND source.agent_id=run.agent_id AND source.runner_id=run.runner_id
           AND source.workspace_run_id=run.workspace_run_id
         WHERE run.corp_id=$1 AND run.id=$2",
    ).bind(corp_id).bind(run_id).fetch_one(&mut **tx).await?;
    let contract: TaskContract = serde_json::from_value(row.get("contract"))?;
    let connection_id = if row.get::<Option<Uuid>, _>("resumed_from_run_id").is_some() {
        if row.get::<Option<Uuid>, _>("source_id").is_none() {
            return Err(anyhow!(
                "resumed connection lost its exact source assignment"
            ));
        }
        let inherited: Option<Uuid> = row.get("source_connection");
        if inherited != contract.workspace_connection_id {
            return Err(anyhow!(
                "resume cannot switch the saved execution connection"
            ));
        }
        inherited
    } else {
        contract.workspace_connection_id
    };
    if let Some(connection_id) = connection_id {
        let connection =
            map_connection(&connection_row_tx(tx, corp_id, connection_id, true).await?)?;
        if connection.room_id != row.get::<Uuid, _>("room_id")
            || connection.runner_id != row.get::<String, _>("runner_id")
            || connection.agent.as_str() != row.get::<String, _>("required_adapter")
        {
            return Err(anyhow!(
                "run does not belong to the selected connection's room, machine and agent"
            ));
        }
        if !connection.runner_connected {
            return Err(anyhow!("the saved connection's runner is not connected"));
        }
        if row.get::<String, _>("execution_mode") == "provider"
            && connection.status != WorkspaceConnectionStatus::Ready
        {
            return Err(anyhow!(
                "the coding-agent connection needs attention before provider execution"
            ));
        }
        let source = connection
            .source
            .context("execution connection has no accepted repository")?;
        if Some(source.repository) != row.get::<Option<String>, _>("source_repository")
            || Some(source.base_ref) != row.get::<Option<String>, _>("source_base_ref")
        {
            return Err(anyhow!(
                "execution connection belongs to another source repository"
            ));
        }
        // A new run uses today's checked revision. A resumed run retains the
        // exact persisted old revision and worktree, not a moving source ref.
        if row.get::<Option<Uuid>, _>("resumed_from_run_id").is_none()
            && Some(source.base_commit) != row.get::<Option<String>, _>("source_base_commit")
        {
            return Err(anyhow!("the saved source revision changed before dispatch"));
        }
    }
    sqlx::query("UPDATE runs SET workspace_connection_id=$3 WHERE corp_id=$1 AND id=$2")
        .bind(corp_id)
        .bind(run_id)
        .bind(connection_id)
        .execute(&mut **tx)
        .await?;
    Ok(connection_id)
}

fn validate_report(report: &WorkspaceSetupReport, expires_at: chrono::DateTime<Utc>) -> Result<()> {
    text(&report.detail, "connection report", 800)?;
    if serde_json::to_vec(report)?.len() > MAX_REPORT_BYTES
        || report.models.len() > 128
        || report.repositories.len() > 1000
    {
        return Err(anyhow!(
            "connection report exceeds its bounded metadata allowance"
        ));
    }
    if matches!(
        report.status,
        WorkspaceSetupStatus::Queued | WorkspaceSetupStatus::Cancelled
    ) {
        return Err(anyhow!("runner cannot assert this setup state"));
    }
    if let Some(source) = &report.source {
        source_identity(source)?;
    }
    if report.connection_status == Some(WorkspaceConnectionStatus::Ready)
        && (report.status != WorkspaceSetupStatus::Succeeded || report.source.is_none())
    {
        return Err(anyhow!(
            "a ready connection needs completed native source and agent checks"
        ));
    }
    if let Some(sign_in) = &report.sign_in {
        if report.status != WorkspaceSetupStatus::NeedsSignIn
            || sign_in.expires_at > expires_at
            || sign_in.expires_at <= Utc::now()
        {
            return Err(anyhow!(
                "native sign-in instructions are outside this operation's lifetime"
            ));
        }
        let url =
            url::Url::parse(&sign_in.verification_uri).context("invalid native sign-in URL")?;
        let allowed = match sign_in.provider.as_str() {
            "github" | "github-copilot" => url.host_str() == Some("github.com"),
            "codex" => url.host_str() == Some("auth.openai.com"),
            "claude-code" => matches!(
                url.host_str(),
                Some("claude.ai" | "console.anthropic.com" | "platform.claude.com")
            ),
            _ => false,
        };
        if !allowed
            || url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some_and(|port| port != 443)
            || sign_in.verification_uri.len() > 8192
        {
            return Err(anyhow!(
                "native sign-in URL is not on the supported provider's HTTPS origin"
            ));
        }
        if sign_in.input_kind.is_some() != sign_in.input_id.is_some()
            || (sign_in.input_kind.is_some() && sign_in.provider != "claude-code")
        {
            return Err(anyhow!(
                "only the native Claude authorization-code prompt accepts a response"
            ));
        }
        if let Some(code) = &sign_in.user_code {
            text(code, "native sign-in code", 128)?;
        }
    }
    for repo in &report.repositories {
        if repository(&repo.repository)? != repo.repository.to_ascii_lowercase() {
            return Err(anyhow!("invalid repository catalogue entry"));
        }
        text(&repo.id, "repository identity", 160)?;
        text(&repo.default_branch, "repository default branch", 240)?;
        validate_factory_base_ref(&repo.default_branch)?;
    }
    Ok(())
}

impl PgStore {
    pub async fn apply_workspace_setup_report(
        &self,
        corp_id: Uuid,
        runner_id: &str,
        connection_epoch: Uuid,
        operation_id: Uuid,
        report: WorkspaceSetupReport,
    ) -> Result<WorkspaceSetupMutation> {
        let mut tx = self.pool.begin().await?;
        let current:Option<Uuid>=sqlx::query_scalar(
            "SELECT connection_epoch FROM runner_nodes WHERE corp_id=$1 AND id=$2 AND status='connected' FOR SHARE",
        ).bind(corp_id).bind(runner_id).fetch_optional(&mut *tx).await?;
        if current != Some(connection_epoch) {
            return Err(anyhow!("forbidden: stale or foreign setup runner"));
        }
        let initial = sqlx::query(
            "SELECT * FROM workspace_setup_operations WHERE corp_id=$1 AND runner_id=$2 AND id=$3",
        )
        .bind(corp_id)
        .bind(runner_id)
        .bind(operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .context("setup operation does not belong to this runner")?;
        let connection_id: Option<Uuid> = initial.get("connection_id");
        if let Some(id) = connection_id {
            lock_factory_keys_tx(&mut tx, &[format!("workspace-connection:{corp_id}:{id}")])
                .await?;
        }
        let row = sqlx::query(
            "SELECT * FROM workspace_setup_operations WHERE corp_id=$1 AND id=$2 FOR UPDATE",
        )
        .bind(corp_id)
        .bind(operation_id)
        .fetch_one(&mut *tx)
        .await?;
        let operation = map_operation(&row)?;
        validate_report(&report, operation.expires_at)?;
        let report_json = serde_json::to_value(&report)?;
        if operation.status.terminal() {
            if row.get::<Option<Value>, _>("report") != Some(report_json) {
                return Err(anyhow!(
                    "conflict: setup operation already has a different terminal result"
                ));
            }
            let connection = if let Some(id) = connection_id {
                Some(map_connection(
                    &connection_row_tx(&mut tx, corp_id, id, false).await?,
                )?)
            } else {
                None
            };
            tx.commit().await?;
            return Ok(WorkspaceSetupMutation {
                connection,
                operation,
                events: vec![],
                replayed: true,
            });
        }
        if operation.expires_at <= Utc::now() {
            return Err(anyhow!(
                "connection operation expired; check the saved connection again"
            ));
        }
        let action: WorkspaceSetupAction = serde_json::from_value(row.get("action"))?;
        if !report.repositories.is_empty()
            && !matches!(action, WorkspaceSetupAction::ListGitHubRepositories { .. })
        {
            return Err(anyhow!(
                "repository catalogues belong only to their private discovery operation"
            ));
        }
        if report.sign_in.is_some()
            && !matches!(
                action,
                WorkspaceSetupAction::SignInGitHub
                    | WorkspaceSetupAction::Connect { .. }
                    | WorkspaceSetupAction::SignInAgent { .. }
            )
        {
            return Err(anyhow!("this operation did not authorize native sign-in"));
        }
        if connection_id.is_none()
            && (report.source.is_some() || report.connection_status.is_some())
        {
            return Err(anyhow!(
                "account discovery cannot change a project connection"
            ));
        }
        if report.status != WorkspaceSetupStatus::Failed {
            authorize_setup_tx(
                &mut tx,
                &WorkspaceSetupInput {
                    corp_id,
                    room_id: operation.room_id,
                    actor_id: operation.actor_id,
                    runner_id: runner_id.to_owned(),
                    action: action.clone(),
                    idempotency_key: row.get("idempotency_key"),
                },
            )
            .await?;
        }
        if let Some(id) = connection_id {
            let connection_row = connection_row_tx(&mut tx, corp_id, id, true).await?;
            if connection_row.get::<Uuid, _>("room_id") != operation.room_id
                || connection_row.get::<String, _>("runner_id") != runner_id
                || Some(connection_row.get::<i64, _>("version"))
                    != row.get::<Option<i64>, _>("expected_connection_version")
            {
                return Err(anyhow!(
                    "conflict: the saved connection changed during setup"
                ));
            }
            let configuration: WorkspaceConnectionConfiguration =
                serde_json::from_value(connection_row.get("configuration"))?;
            if let Some(source) = &report.source {
                match &configuration.repository {
                    WorkspaceRepositorySetup::GitHub {
                        repository,
                        repository_id,
                        base_ref,
                        ..
                    } => {
                        if source.repository != *repository
                            || source.base_ref != *base_ref
                            || repository_id
                                .as_ref()
                                .is_some_and(|id| source.repository_id.as_ref() != Some(id))
                        {
                            return Err(anyhow!(
                                "setup result does not match the selected GitHub repository"
                            ));
                        }
                    }
                    WorkspaceRepositorySetup::Advertised { source: expected }
                        if source != expected =>
                    {
                        return Err(anyhow!(
                            "setup result changed the selected advertised source"
                        ));
                    }
                    WorkspaceRepositorySetup::Local { base_ref, .. }
                        if source.base_ref != *base_ref =>
                    {
                        return Err(anyhow!("setup result changed the selected source ref"));
                    }
                    _ => {}
                }
            }
            let status = report.connection_status.unwrap_or(match report.status {
                WorkspaceSetupStatus::NeedsSignIn => WorkspaceConnectionStatus::NeedsSignIn,
                WorkspaceSetupStatus::Failed => WorkspaceConnectionStatus::Failed,
                _ => WorkspaceConnectionStatus::Connecting,
            });
            sqlx::query(
                "UPDATE workspace_connections SET status=$3,detail=$4,
                 source_repository=COALESCE($5,source_repository),source_repository_id=COALESCE($6,source_repository_id),
                 source_base_ref=COALESCE($7,source_base_ref),source_base_commit=COALESCE($8,source_base_commit),
                 models=$9,last_checked_at=CASE WHEN $10 THEN now() ELSE last_checked_at END,updated_at=now()
                 WHERE corp_id=$1 AND id=$2",
            ).bind(corp_id).bind(id).bind(status.as_str()).bind(shared_connection_detail(status))
                .bind(report.source.as_ref().map(|s|&s.repository))
                .bind(report.source.as_ref().and_then(|s|s.repository_id.as_ref()))
                .bind(report.source.as_ref().map(|s|&s.base_ref))
                .bind(report.source.as_ref().map(|s|&s.base_commit))
                .bind(serde_json::to_value(&report.models)?).bind(report.status.terminal())
                .execute(&mut *tx).await?;
        }
        let row=sqlx::query(
            "UPDATE workspace_setup_operations SET status=$3,report=$4,dispatch_epoch=$5,updated_at=now()
             WHERE corp_id=$1 AND id=$2 RETURNING *",
        ).bind(corp_id).bind(operation_id).bind(report.status.as_str()).bind(report_json)
            .bind(connection_epoch).fetch_one(&mut *tx).await?;
        let operation = map_operation(&row)?;
        let connection = if let Some(id) = connection_id {
            Some(map_connection(
                &connection_row_tx(&mut tx, corp_id, id, false).await?,
            )?)
        } else {
            None
        };
        let events = setup_event_tx(&mut tx, &operation)
            .await?
            .into_iter()
            .collect();
        tx.commit().await?;
        Ok(WorkspaceSetupMutation {
            connection,
            operation,
            events,
            replayed: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crony_domain::CodingAgent;
    #[test]
    fn workspace_repositories_never_accept_credentials_flags_or_other_hosts() {
        assert_eq!(
            repository("https://github.com/Owner/Project.git").unwrap(),
            "owner/project"
        );
        for input in [
            "https://token@github.com/owner/project",
            "https://github.com.evil/owner/project",
            "owner/project?token=x",
            "-owner/project",
            "owner/../project",
            "owner/project\n--config=x",
        ] {
            assert!(repository(input).is_err(), "{input}");
        }
    }
    #[test]
    fn machine_account_and_local_paths_are_explicit_operator_actions() {
        assert!(needs_machine_authority(
            &WorkspaceSetupAction::InspectGitHub {
                account: GitHubAccountSource::Machine
            }
        ));
        assert!(!needs_machine_authority(
            &WorkspaceSetupAction::SignInGitHub
        ));
        assert!(!needs_machine_authority(
            &WorkspaceSetupAction::InspectGitHub {
                account: GitHubAccountSource::Personal
            }
        ));
    }
    #[test]
    fn repository_sources_use_absolute_paths_and_safe_immutable_refs() {
        let valid = WorkspaceConnectionConfiguration {
            repository: WorkspaceRepositorySetup::Local {
                directory: "C:\\code\\team project".to_owned(),
                base_ref: "main".to_owned(),
            },
            agent: CodingAgent::Codex,
            use_system_installation: false,
            use_machine_account: None,
        };
        assert!(normalize_configuration(valid.clone()).is_ok());
        for path in [
            "relative/project",
            "\\\\server\\share",
            "//server/share",
            "https://github.com/owner/repo",
        ] {
            let mut invalid = valid.clone();
            invalid.repository = WorkspaceRepositorySetup::Local {
                directory: path.to_owned(),
                base_ref: "main".to_owned(),
            };
            assert!(normalize_configuration(invalid).is_err());
        }
    }
}
