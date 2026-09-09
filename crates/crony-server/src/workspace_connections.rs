//! Browser setup uses actor-scoped metadata and fixed operations on enrolled nodes.
use super::*;
use crony_domain::{
    GitHubAccountSource, NativeSignInResponse, WorkspaceConnectionConfiguration,
    WorkspaceConnections, WorkspaceSetupAction, WorkspaceSetupOperation,
};
use crony_store::{CreateWorkspaceConnectionInput, WorkspaceSetupInput, WorkspaceSetupMutation};
use serde_json::Value;
use tokio::sync::oneshot;

pub(super) struct PendingSignIn {
    pub corp_id: Uuid,
    pub runner_id: String,
    pub connection_epoch: Uuid,
    pub operation_id: Uuid,
    pub response: oneshot::Sender<bool>,
}

#[derive(Deserialize)]
struct Viewer {
    actor_id: Option<Uuid>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateConnectionRequest {
    actor_id: Option<Uuid>,
    runner_id: String,
    label: String,
    configuration: WorkspaceConnectionConfiguration,
    idempotency_key: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationRequest {
    actor_id: Option<Uuid>,
    idempotency_key: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum GitHubOperation {
    Inspect,
    SignIn,
    Repositories,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GitHubRequest {
    actor_id: Option<Uuid>,
    runner_id: String,
    action: GitHubOperation,
    account: GitHubAccountSource,
    idempotency_key: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectionRequest {
    actor_id: Option<Uuid>,
    connection_id: Uuid,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignInResponseRequest {
    actor_id: Option<Uuid>,
    input_id: Uuid,
    authorization_code: String,
}

#[derive(Serialize)]
struct SetupResponse {
    connection: Option<crony_domain::WorkspaceConnection>,
    operation: WorkspaceSetupOperation,
    replayed: bool,
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/corps/{corp_id}/rooms/{room_id}/connections",
            get(list).post(create),
        )
        .route(
            "/api/corps/{corp_id}/rooms/{room_id}/connections/selection",
            post(select),
        )
        .route(
            "/api/corps/{corp_id}/rooms/{room_id}/connections/github",
            post(github),
        )
        .route(
            "/api/corps/{corp_id}/connections/{connection_id}/check",
            post(check),
        )
        .route(
            "/api/corps/{corp_id}/connections/{connection_id}/sign-in",
            post(sign_in),
        )
        .route(
            "/api/corps/{corp_id}/setup-operations/{operation_id}",
            get(operation),
        )
        .route(
            "/api/corps/{corp_id}/setup-operations/{operation_id}/sign-in-response",
            post(sign_in_response),
        )
}

async fn list(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, room_id)): Path<(Uuid, Uuid)>,
    Query(viewer): Query<Viewer>,
) -> Result<Json<WorkspaceConnections>, ApiError> {
    let actor = authorize_actor(
        &state,
        &principal,
        corp_id,
        viewer.actor_id,
        Permission::Operate,
    )
    .await?;
    let mut result = state
        .store
        .workspace_connections(corp_id, room_id, actor)
        .await
        .map_err(map_store_error)?;
    for connection in &mut result.connections {
        connection.runner_connected = state
            .runners
            .get(&connection.runner_id)
            .is_some_and(|runner| runner.corp_id == corp_id && runner.dispatch_ready);
    }
    Ok(Json(result))
}

async fn finish(
    state: &AppState,
    outcome: WorkspaceSetupMutation,
) -> Result<Json<SetupResponse>, ApiError> {
    for event in outcome.events {
        publish(state, event);
    }
    // The request remains durable while a node is offline. A failed send is not
    // permission to create another operation or claim it completed.
    if let Err(error) = dispatch(state, &outcome.operation.runner_id).await {
        warn!(%error, operation_id=%outcome.operation.id, "connection setup awaits runner delivery");
    }
    Ok(Json(SetupResponse {
        connection: outcome.connection,
        operation: outcome.operation,
        replayed: outcome.replayed,
    }))
}

async fn create(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, room_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<CreateConnectionRequest>,
) -> Result<Json<SetupResponse>, ApiError> {
    let actor = authorize_actor(
        &state,
        &principal,
        corp_id,
        request.actor_id,
        Permission::Operate,
    )
    .await?;
    let outcome = state
        .store
        .create_workspace_connection(CreateWorkspaceConnectionInput {
            corp_id,
            room_id,
            actor_id: actor,
            runner_id: request.runner_id,
            label: request.label,
            configuration: request.configuration,
            idempotency_key: request.idempotency_key,
        })
        .await
        .map_err(map_store_error)?;
    finish(&state, outcome).await
}

async fn github(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, room_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<GitHubRequest>,
) -> Result<Json<SetupResponse>, ApiError> {
    let actor = authorize_actor(
        &state,
        &principal,
        corp_id,
        request.actor_id,
        Permission::Operate,
    )
    .await?;
    let action = match request.action {
        GitHubOperation::Inspect => WorkspaceSetupAction::InspectGitHub {
            account: request.account,
        },
        GitHubOperation::SignIn => WorkspaceSetupAction::SignInGitHub,
        GitHubOperation::Repositories => WorkspaceSetupAction::ListGitHubRepositories {
            account: request.account,
        },
    };
    let outcome = state
        .store
        .begin_workspace_setup(WorkspaceSetupInput {
            corp_id,
            room_id,
            actor_id: actor,
            runner_id: request.runner_id,
            action,
            idempotency_key: request.idempotency_key,
        })
        .await
        .map_err(map_store_error)?;
    finish(&state, outcome).await
}

async fn check(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, connection_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<OperationRequest>,
) -> Result<Json<SetupResponse>, ApiError> {
    connection_operation(&state, &principal, corp_id, connection_id, request, false).await
}

async fn sign_in(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, connection_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<OperationRequest>,
) -> Result<Json<SetupResponse>, ApiError> {
    connection_operation(&state, &principal, corp_id, connection_id, request, true).await
}

async fn connection_operation(
    state: &AppState,
    principal: &Principal,
    corp_id: Uuid,
    connection_id: Uuid,
    request: OperationRequest,
    sign_in: bool,
) -> Result<Json<SetupResponse>, ApiError> {
    let actor = authorize_actor(
        state,
        principal,
        corp_id,
        request.actor_id,
        Permission::Operate,
    )
    .await?;
    let (connection, configuration) = state
        .store
        .workspace_connection_settings(corp_id, actor, connection_id)
        .await
        .map_err(map_store_error)?;
    let action = if sign_in {
        WorkspaceSetupAction::SignInAgent {
            connection_id,
            configuration,
        }
    } else {
        WorkspaceSetupAction::Test {
            connection_id,
            configuration,
        }
    };
    let outcome = state
        .store
        .begin_workspace_setup(WorkspaceSetupInput {
            corp_id,
            room_id: connection.room_id,
            actor_id: actor,
            runner_id: connection.runner_id,
            action,
            idempotency_key: request.idempotency_key,
        })
        .await
        .map_err(map_store_error)?;
    finish(state, outcome).await
}

async fn select(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, room_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<SelectionRequest>,
) -> Result<Json<Value>, ApiError> {
    let actor = authorize_actor(
        &state,
        &principal,
        corp_id,
        request.actor_id,
        Permission::Operate,
    )
    .await?;
    state
        .store
        .select_workspace_connection(corp_id, room_id, actor, request.connection_id)
        .await
        .map_err(map_store_error)?;
    Ok(Json(
        json!({"selected_connection_id": request.connection_id}),
    ))
}

async fn operation(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, operation_id)): Path<(Uuid, Uuid)>,
    Query(viewer): Query<Viewer>,
) -> Result<Json<WorkspaceSetupOperation>, ApiError> {
    let actor = authorize_actor(
        &state,
        &principal,
        corp_id,
        viewer.actor_id,
        Permission::Operate,
    )
    .await?;
    let operation = state
        .store
        .workspace_setup_operation(corp_id, actor, operation_id)
        .await
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found("connection check was not found"))?;
    Ok(Json(operation))
}

async fn sign_in_response(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, operation_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<SignInResponseRequest>,
) -> Result<Json<Value>, ApiError> {
    let actor = authorize_actor(
        &state,
        &principal,
        corp_id,
        request.actor_id,
        Permission::Operate,
    )
    .await?;
    let code = request.authorization_code.trim();
    if code.is_empty()
        || code.len() > 4096
        || code.bytes().any(|byte| !byte.is_ascii_graphic())
        || code.starts_with("sk-")
        || code.starts_with("ghp_")
        || code.starts_with("github_pat_")
    {
        return Err(ApiError::bad_request(
            "enter the one-time code from the native sign-in page, not an API key",
        ));
    }
    let operation = state
        .store
        .workspace_sign_in_operation(corp_id, actor, operation_id, request.input_id)
        .await
        .map_err(map_store_error)?;
    let runner = state
        .runners
        .get(&operation.runner_id)
        .filter(|runner| runner.corp_id == corp_id && runner.dispatch_ready)
        .map(|runner| runner.clone())
        .ok_or_else(|| {
            ApiError::conflict("the sign-in machine is offline; the code was not submitted")
        })?;
    if state.workspace_sign_in.len() >= 64 {
        return Err(ApiError::conflict(
            "another sign-in response is still being delivered",
        ));
    }
    let request_id = Uuid::new_v4();
    let (sender, receiver) = oneshot::channel();
    state.workspace_sign_in.insert(
        request_id,
        PendingSignIn {
            corp_id,
            runner_id: operation.runner_id.clone(),
            connection_epoch: runner.connection_epoch,
            operation_id,
            response: sender,
        },
    );
    let sent = runner.tx.send(ServerToRunner::WorkspaceSignInInput {
        operation_id,
        request_id,
        response: NativeSignInResponse {
            input_id: request.input_id,
            authorization_code: code.to_owned(),
        },
    });
    if sent.is_err() {
        state.workspace_sign_in.remove(&request_id);
        return Err(ApiError::conflict(
            "the sign-in machine disconnected; check the current sign-in step",
        ));
    }
    let delivered = tokio::time::timeout(StdDuration::from_secs(15), receiver).await;
    state.workspace_sign_in.remove(&request_id);
    match delivered {
        Ok(Ok(true)) => Ok(Json(json!({"submitted": true}))),
        _ => Err(ApiError::conflict(
            "the native sign-in step did not confirm delivery; check its current status before retrying",
        )),
    }
}

pub(super) async fn dispatch(state: &AppState, runner_id: &str) -> anyhow::Result<()> {
    let Some(runner) = state
        .runners
        .get(runner_id)
        .filter(|runner| {
            runner.dispatch_ready
                && runner
                    .capabilities
                    .iter()
                    .any(|cap| cap.name == "workspace-setup-v1" && cap.available)
        })
        .map(|runner| runner.clone())
    else {
        return Ok(());
    };
    let commands = state
        .store
        .pending_workspace_setup(runner.corp_id, runner_id, runner.connection_epoch)
        .await?;
    for command in commands {
        if !runner_epoch_is_ready(&state.runners, runner_id, runner.connection_epoch) {
            break;
        }
        runner
            .tx
            .send(ServerToRunner::WorkspaceSetup { command })
            .map_err(|_| anyhow::anyhow!("runner disconnected before connection setup delivery"))?;
    }
    Ok(())
}

pub(super) async fn filter_capabilities(
    state: &AppState,
    corp_id: Uuid,
    runner_id: &str,
    capabilities: Vec<RunnerCapability>,
) -> anyhow::Result<Vec<RunnerCapability>> {
    anyhow::ensure!(
        capabilities.len() <= 1024,
        "runner capability update exceeds its bound"
    );
    let ids = capabilities
        .iter()
        .filter_map(|cap| cap.workspace_connection_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let bindings = state
        .store
        .workspace_runtime_bindings(corp_id, runner_id, &ids)
        .await?
        .into_iter()
        .map(|connection| (connection.id, connection))
        .collect::<std::collections::HashMap<_, _>>();
    let mut result = Vec::with_capacity(capabilities.len());
    for mut cap in capabilities {
        let Some(id) = cap.workspace_connection_id else {
            result.push(cap);
            continue;
        };
        let Some(binding) = bindings.get(&id) else {
            continue;
        };
        let Some(source) = &binding.source else {
            continue;
        };
        if cap.name == "workspace-isolation" {
            if cap.source_repository.as_deref() != Some(source.repository.as_str())
                || cap.source_base_ref.as_deref() != Some(source.base_ref.as_str())
                || cap.source_base_commit.as_deref() != Some(source.base_commit.as_str())
            {
                continue;
            }
            // Retained source is useful for verifier-only work even if model
            // authentication later needs repair.
        } else if cap.name == binding.agent.as_str() {
            if serde_json::to_value(&cap.models)? != serde_json::to_value(&binding.models)? {
                continue;
            }
            cap.available &= binding.status == crony_domain::WorkspaceConnectionStatus::Ready;
        } else {
            continue;
        }
        result.push(cap);
    }
    Ok(result)
}

pub(super) fn acknowledge_sign_in(
    state: &AppState,
    corp_id: Uuid,
    runner_id: &str,
    epoch: Uuid,
    operation_id: Uuid,
    request_id: Uuid,
    applied: bool,
) {
    let matches = state
        .workspace_sign_in
        .get(&request_id)
        .is_some_and(|pending| {
            pending.corp_id == corp_id
                && pending.runner_id == runner_id
                && pending.connection_epoch == epoch
                && pending.operation_id == operation_id
        });
    if matches && let Some((_, pending)) = state.workspace_sign_in.remove(&request_id) {
        let _ = pending.response.send(applied);
    }
}
