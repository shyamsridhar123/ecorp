mod auth;
mod planning;
mod secrets;

use std::{collections::HashSet, net::SocketAddr, sync::Arc};

use anyhow::Context;
use axum::{
    Extension, Json, Router,
    body::Body,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{
        HeaderValue, Method, Request, StatusCode,
        header::{AUTHORIZATION, CONTENT_TYPE, HeaderName},
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{Duration as ChronoDuration, Utc};
use clap::Parser;
use crony_domain::DomainEvent;
use crony_protocol::{
    ActionApprovalDecisionRequest, ActionApprovalDecisionResponse, BrowserSocketMessage,
    ClaimLeaseRequest, ClaimLeaseResponse, CreateMissionRequest, CreateMissionResponse,
    CreateRoomMessageRequest, CreateRoomMessageResponse, CreateRunnerEnrollmentRequest,
    CreateRunnerEnrollmentResponse, CreateSecretRequest, CreateSecretResponse,
    DemoBootstrapResponse, EmergencyStopRequest, EmergencyStopResponse, InterruptRunRequest,
    InterruptRunResponse, LaunchMissionRequest, LaunchMissionResponse, LeaseMutationResponse,
    QueueMessageRequest, QueueMessageResponse, ReleaseLeaseRequest, ResolvedSecret,
    ResumeRunRequest, ResumeRunResponse, RevokeRunnerRequest, RevokeRunnerResponse,
    RevokeSecretRequest, RunnerCapability, RunnerSummary, RunnerToServer, ServerToRunner,
    SetBudgetPolicyRequest, SnapshotResponse, TransferLeaseRequest, VerificationDecisionRequest,
    VerificationDecisionResponse,
};
use crony_store::{
    LaunchRecord, NewRoomMessageInput, PendingRunnerCommand, PgStore, RunClaim, RunnerConnectInput,
    RunnerEventInput,
};
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, mpsc};
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing::{error, info, warn};
use uuid::Uuid;

use auth::{AuthService, CorpRole, Permission, Principal, ServerMode};
use planning::{PlanningRequest, StrategyRegistry};
use secrets::SecretCipher;

#[derive(Debug, Parser)]
#[command(name = "crony-server")]
struct Args {
    #[arg(
        long,
        env = "DATABASE_URL",
        default_value = "postgres://crony:crony@127.0.0.1:54329/crony"
    )]
    database_url: String,

    #[arg(long, env = "CRONY_BIND", default_value = "127.0.0.1:8791")]
    bind: SocketAddr,

    #[arg(long, env = "CRONY_RUNNER_GRACE_SECS", default_value_t = 5)]
    runner_grace_secs: i64,

    #[arg(
        long,
        env = "CRONY_MODE",
        value_enum,
        default_value_t = ServerMode::Development
    )]
    mode: ServerMode,

    #[arg(long, env = "CRONY_OIDC_ISSUER")]
    oidc_issuer: Option<String>,

    #[arg(long, env = "CRONY_ALLOW_INSECURE_OIDC", default_value_t = false)]
    allow_insecure_oidc: bool,

    #[arg(long, env = "CRONY_CORS_ORIGINS", value_delimiter = ',')]
    cors_origins: Vec<String>,

    #[arg(
        long,
        env = "CRONY_RUNNER_CREDENTIAL_TTL_SECS",
        default_value_t = 86_400
    )]
    runner_credential_ttl_secs: i64,

    #[arg(long, env = "CRONY_SECRET_MASTER_KEY_HEX")]
    secret_master_key_hex: Option<String>,
}

#[derive(Clone)]
struct AppState {
    store: PgStore,
    event_tx: broadcast::Sender<DomainEvent>,
    runners: Arc<DashMap<String, RunnerConnection>>,
    strategies: StrategyRegistry,
    runner_grace_secs: i64,
    runner_credential_ttl_secs: i64,
    auth: AuthService,
    secret_cipher: SecretCipher,
}

#[derive(Clone)]
struct RunnerConnection {
    corp_id: Uuid,
    connection_epoch: Uuid,
    tx: mpsc::UnboundedSender<ServerToRunner>,
    capabilities: Vec<RunnerCapability>,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn internal(error: impl std::fmt::Display) -> Self {
        error!("request failed: {error}");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }

    fn bad_request(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: error.to_string(),
        }
    }

    fn conflict(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: error.to_string(),
        }
    }

    fn forbidden(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: error.to_string(),
        }
    }

    fn unauthorized(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: error.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "error": self.message,
            })),
        )
            .into_response()
    }
}

fn map_store_error(error: anyhow::Error) -> ApiError {
    if error.to_string().starts_with("forbidden:") {
        ApiError::forbidden(error)
    } else {
        ApiError::bad_request(error)
    }
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
    runners: usize,
    mode: &'static str,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "crony_server=info,tower_http=info".into()),
        )
        .init();
    let args = Args::parse();

    let store = PgStore::connect(&args.database_url).await?;
    store.migrate().await?;
    let auth = AuthService::initialize(
        args.mode,
        args.oidc_issuer.clone(),
        args.allow_insecure_oidc,
    )
    .await?;
    let secret_cipher = SecretCipher::initialize(args.mode, args.secret_master_key_hex.as_deref())?;
    let (event_tx, _) = broadcast::channel(2_048);
    let state = AppState {
        store,
        event_tx,
        runners: Arc::new(DashMap::new()),
        strategies: StrategyRegistry::new(),
        runner_grace_secs: args.runner_grace_secs.max(1),
        runner_credential_ttl_secs: args.runner_credential_ttl_secs.clamp(300, 604_800),
        auth,
        secret_cipher,
    };

    let protected = Router::new()
        .route("/api/corps/{corp_id}/snapshot", get(snapshot))
        .route("/api/corps/{corp_id}/missions", post(create_mission))
        .route(
            "/api/corps/{corp_id}/rooms/{room_id}/messages",
            post(create_room_message),
        )
        .route(
            "/api/corps/{corp_id}/missions/{mission_id}/launch",
            post(launch_mission),
        )
        .route(
            "/api/corps/{corp_id}/runs/{run_id}/resume",
            post(resume_run),
        )
        .route(
            "/api/corps/{corp_id}/runs/{run_id}/verification-decision",
            post(decide_verification),
        )
        .route(
            "/api/corps/{corp_id}/agents/{agent_id}/lease",
            post(claim_lease),
        )
        .route(
            "/api/corps/{corp_id}/agents/{agent_id}/lease/release",
            post(release_lease),
        )
        .route(
            "/api/corps/{corp_id}/agents/{agent_id}/lease/transfer",
            post(transfer_lease),
        )
        .route(
            "/api/corps/{corp_id}/agents/{agent_id}/messages",
            post(queue_message),
        )
        .route(
            "/api/corps/{corp_id}/agents/{agent_id}/emergency-stop",
            post(emergency_stop),
        )
        .route(
            "/api/corps/{corp_id}/agents/{agent_id}/interrupt",
            post(interrupt_run),
        )
        .route(
            "/api/corps/{corp_id}/runners/enroll",
            post(create_runner_enrollment),
        )
        .route(
            "/api/corps/{corp_id}/runners/{runner_id}/revoke",
            post(revoke_runner),
        )
        .route(
            "/api/corps/{corp_id}/ws-ticket",
            post(create_websocket_ticket),
        )
        .route("/api/corps/{corp_id}/secrets", post(create_secret))
        .route(
            "/api/corps/{corp_id}/secrets/{secret_id}/revoke",
            post(revoke_secret),
        )
        .route(
            "/api/corps/{corp_id}/approvals/{approval_id}/decision",
            post(decide_action_approval),
        )
        .route(
            "/api/corps/{corp_id}/budget-policy",
            post(set_budget_policy),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            authenticate_http,
        ));

    let mut app = Router::new()
        .route("/health", get(health))
        .merge(protected)
        .route("/ws/corps/{corp_id}", get(browser_websocket))
        .route("/ws/runner", get(runner_websocket))
        .layer(TraceLayer::new_for_http());

    if args.mode == ServerMode::Development {
        app = app
            .route("/api/demo/bootstrap", post(bootstrap_demo))
            .route("/api/demo/reset", post(reset_demo))
            .route("/api/demo/oidc-link", post(debug_link_oidc_identity))
            .route(
                "/api/demo/runners/{runner_id}/disconnect",
                post(debug_disconnect_runner),
            )
            .layer(
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_headers(Any)
                    .allow_methods(Any),
            );
    } else {
        let origins = args
            .cors_origins
            .iter()
            .map(|value| {
                HeaderValue::from_str(value)
                    .with_context(|| format!("invalid CRONY_CORS_ORIGINS entry {value}"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let mut cors = CorsLayer::new()
            .allow_methods([Method::GET, Method::POST])
            .allow_headers([
                AUTHORIZATION,
                CONTENT_TYPE,
                HeaderName::from_static("x-crony-request-id"),
            ]);
        if !origins.is_empty() {
            cors = cors.allow_origin(origins).allow_credentials(true);
        }
        app = app.layer(cors);
    }
    let app = app.with_state(state);

    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("bind server to {}", args.bind))?;
    info!(address = %args.bind, "Crony server listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "crony-server",
        runners: state.runners.len(),
        mode: match state.auth.mode() {
            ServerMode::Development => "development",
            ServerMode::Production => "production",
        },
    })
}

async fn authenticate_http(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    match state.auth.authenticate_headers(request.headers()).await {
        Ok(principal) => {
            request.extensions_mut().insert(principal);
            next.run(request).await
        }
        Err(error) => ApiError::unauthorized(error).into_response(),
    }
}

async fn authorize_actor(
    state: &AppState,
    principal: &Principal,
    corp_id: Uuid,
    claimed_actor_id: Option<Uuid>,
    permission: Permission,
) -> Result<Uuid, ApiError> {
    let authorization = match principal {
        Principal::Development => {
            let actor_id = claimed_actor_id
                .ok_or_else(|| ApiError::unauthorized("development actor identity is missing"))?;
            state
                .store
                .human_authorization(corp_id, actor_id)
                .await
                .map_err(ApiError::internal)?
        }
        Principal::Oidc {
            issuer,
            subject,
            email,
        } => {
            let _ = email;
            state
                .store
                .resolve_human_identity(corp_id, issuer, subject)
                .await
                .map_err(ApiError::internal)?
        }
    }
    .ok_or_else(|| ApiError::forbidden("identity is not a human member of this Corp"))?;

    if let Some(claimed) = claimed_actor_id
        && claimed != authorization.actor_id
    {
        return Err(ApiError::forbidden(
            "request actor does not match the authenticated identity",
        ));
    }
    let role = CorpRole::parse(&authorization.role).map_err(ApiError::forbidden)?;
    if !role.allows(permission) {
        return Err(ApiError::forbidden(format!(
            "Corp role {} is not allowed to perform this action",
            authorization.role
        )));
    }
    Ok(authorization.actor_id)
}

async fn bootstrap_demo(
    State(state): State<AppState>,
) -> Result<Json<DemoBootstrapResponse>, ApiError> {
    let (ids, event) = state
        .store
        .bootstrap_demo()
        .await
        .map_err(ApiError::internal)?;
    if let Some(event) = event {
        publish(&state, event);
    }
    Ok(Json(DemoBootstrapResponse {
        corp_id: ids.corp_id,
        room_id: ids.room_id,
        alice_actor_id: ids.alice_actor_id,
        bob_actor_id: ids.bob_actor_id,
        eve_actor_id: ids.eve_actor_id,
        manager_agent_id: ids.manager_agent_id,
        worker_agent_id: ids.worker_agent_id,
        codex_agent_id: ids.codex_agent_id,
    }))
}

async fn reset_demo(
    State(state): State<AppState>,
) -> Result<Json<DemoBootstrapResponse>, ApiError> {
    let (ids, event) = state.store.reset_demo().await.map_err(ApiError::internal)?;
    if let Some(event) = event {
        publish(&state, event);
    }
    Ok(Json(DemoBootstrapResponse {
        corp_id: ids.corp_id,
        room_id: ids.room_id,
        alice_actor_id: ids.alice_actor_id,
        bob_actor_id: ids.bob_actor_id,
        eve_actor_id: ids.eve_actor_id,
        manager_agent_id: ids.manager_agent_id,
        worker_agent_id: ids.worker_agent_id,
        codex_agent_id: ids.codex_agent_id,
    }))
}

#[derive(Debug, Deserialize)]
struct DebugDisconnectRequest {
    reconnect_delay_ms: u64,
}

async fn debug_disconnect_runner(
    State(state): State<AppState>,
    Path(runner_id): Path<String>,
    Json(request): Json<DebugDisconnectRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let runner = state
        .runners
        .get(&runner_id)
        .ok_or_else(|| ApiError::conflict("runner is not connected"))?;
    runner
        .tx
        .send(ServerToRunner::Disconnect {
            reason: "development reconnect test".to_owned(),
            reconnect_delay_ms: request.reconnect_delay_ms.min(60_000),
        })
        .map_err(|_| ApiError::conflict("runner disconnected before debug command delivery"))?;
    Ok(Json(json!({
        "runner_id": runner_id,
        "disconnect_requested": true,
        "reconnect_delay_ms": request.reconnect_delay_ms.min(60_000)
    })))
}

#[derive(Debug, Deserialize)]
struct DebugOidcLinkRequest {
    actor_id: Uuid,
    issuer: String,
    subject: String,
    email: Option<String>,
}

async fn debug_link_oidc_identity(
    State(state): State<AppState>,
    Json(request): Json<DebugOidcLinkRequest>,
) -> Result<StatusCode, ApiError> {
    state
        .store
        .link_human_identity(
            request.actor_id,
            request.issuer.trim_end_matches('/'),
            &request.subject,
            request.email.as_deref(),
        )
        .await
        .map_err(ApiError::bad_request)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn create_runner_enrollment(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(corp_id): Path<Uuid>,
    Json(request): Json<CreateRunnerEnrollmentRequest>,
) -> Result<Json<CreateRunnerEnrollmentResponse>, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.actor_id),
        Permission::Manage,
    )
    .await?;
    let ttl = request.expires_in_seconds.clamp(60, 3_600) as i64;
    let expires_at = Utc::now() + ChronoDuration::seconds(ttl);
    let enrollment_token = new_runner_token("enroll");
    let outcome = state
        .store
        .create_runner_enrollment(
            corp_id,
            actor_id,
            &request.runner_id,
            &hash_secret(&enrollment_token),
            expires_at,
        )
        .await
        .map_err(map_store_error)?;
    publish(&state, outcome.event);
    Ok(Json(CreateRunnerEnrollmentResponse {
        runner_id: request.runner_id,
        enrollment_token,
        expires_at: outcome.expires_at.to_rfc3339(),
    }))
}

async fn revoke_runner(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, runner_id)): Path<(Uuid, String)>,
    Json(request): Json<RevokeRunnerRequest>,
) -> Result<Json<RevokeRunnerResponse>, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.actor_id),
        Permission::Manage,
    )
    .await?;
    let outcome = state
        .store
        .revoke_runner(corp_id, actor_id, &runner_id, &request.reason)
        .await
        .map_err(map_store_error)?;
    if let Some(event) = outcome.event {
        publish(&state, event);
    }
    if outcome.revoked
        && let Some(connection) = state.runners.get(&runner_id)
        && connection.corp_id == corp_id
    {
        let _ = connection.tx.send(ServerToRunner::Disconnect {
            reason: "runner credential revoked".to_owned(),
            reconnect_delay_ms: 60_000,
        });
    }
    Ok(Json(RevokeRunnerResponse {
        runner_id,
        revoked: outcome.revoked,
    }))
}

fn new_runner_token(kind: &str) -> String {
    format!(
        "crony_{kind}_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}

fn hash_secret(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

#[derive(Debug, Deserialize)]
struct WebSocketTicketRequest {
    actor_id: Uuid,
}

async fn create_websocket_ticket(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(corp_id): Path<Uuid>,
    Json(request): Json<WebSocketTicketRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.actor_id),
        Permission::Read,
    )
    .await?;
    let (ticket, expires_in_seconds) = state.auth.issue_websocket_ticket(corp_id, actor_id);
    Ok(Json(json!({
        "ticket": ticket,
        "expires_in_seconds": expires_in_seconds,
    })))
}

async fn create_secret(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(corp_id): Path<Uuid>,
    Json(request): Json<CreateSecretRequest>,
) -> Result<Json<CreateSecretResponse>, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.actor_id),
        Permission::Manage,
    )
    .await?;
    let name = request.name.trim();
    if name.is_empty() || name.len() > 100 {
        return Err(ApiError::bad_request(
            "secret name must contain between 1 and 100 characters",
        ));
    }
    if request.value.is_empty() || request.value.len() > 65_536 {
        return Err(ApiError::bad_request(
            "secret value must contain between 1 and 65,536 bytes",
        ));
    }
    if request.allowed_tools.is_empty() || request.allowed_tools.len() > 32 {
        return Err(ApiError::bad_request(
            "secret must allow between 1 and 32 tools",
        ));
    }
    if request
        .allowed_tools
        .iter()
        .any(|tool| !valid_scope_component(tool))
    {
        return Err(ApiError::bad_request(
            "secret tool scopes must use letters, digits, dot, dash, or underscore",
        ));
    }
    let resource_prefix = request.resource_prefix.trim();
    if resource_prefix.is_empty() || resource_prefix.len() > 512 {
        return Err(ApiError::bad_request(
            "secret resource prefix must contain between 1 and 512 characters",
        ));
    }
    let allowed_actor_ids = if request.allowed_actor_ids.is_empty() {
        vec![actor_id]
    } else {
        request.allowed_actor_ids
    };
    for allowed_actor_id in &allowed_actor_ids {
        if state
            .store
            .human_authorization(corp_id, *allowed_actor_id)
            .await
            .map_err(ApiError::internal)?
            .is_none()
        {
            return Err(ApiError::bad_request(format!(
                "allowed actor {allowed_actor_id} is not a human member of this Corp"
            )));
        }
    }

    let secret_id = Uuid::new_v4();
    let (ciphertext, nonce) = state
        .secret_cipher
        .encrypt(corp_id, secret_id, name, request.value.as_bytes())
        .map_err(ApiError::internal)?;
    let event = state
        .store
        .create_secret(
            corp_id,
            actor_id,
            secret_id,
            name,
            &ciphertext,
            &nonce,
            &allowed_actor_ids,
            &request.allowed_tools,
            resource_prefix,
            request.max_ttl_seconds.clamp(30, 3_600) as i32,
        )
        .await
        .map_err(map_store_error)?;
    publish(&state, event);
    Ok(Json(CreateSecretResponse {
        secret_id,
        name: name.to_owned(),
    }))
}

async fn revoke_secret(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, secret_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<RevokeSecretRequest>,
) -> Result<StatusCode, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.actor_id),
        Permission::Manage,
    )
    .await?;
    let event = state
        .store
        .revoke_secret(corp_id, actor_id, secret_id, &request.reason)
        .await
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::bad_request("secret is missing or already revoked"))?;
    publish(&state, event);
    Ok(StatusCode::NO_CONTENT)
}

async fn decide_action_approval(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, approval_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<ActionApprovalDecisionRequest>,
) -> Result<Json<ActionApprovalDecisionResponse>, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.actor_id),
        Permission::Approve,
    )
    .await?;
    let outcome = state
        .store
        .decide_action_approval(
            corp_id,
            approval_id,
            actor_id,
            request.approved,
            &request.note,
            request.decision_key,
        )
        .await
        .map_err(map_store_error)?;
    if let Some(event) = outcome.event {
        publish(&state, event);
    }
    if outcome.effect_queued {
        dispatch_pending_runner_commands(&state, &outcome.runner_id)
            .await
            .map_err(ApiError::internal)?;
    }
    Ok(Json(ActionApprovalDecisionResponse {
        approval_id: outcome.approval_id,
        status: outcome.status,
        effect_queued: outcome.effect_queued,
    }))
}

async fn set_budget_policy(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(corp_id): Path<Uuid>,
    Json(request): Json<SetBudgetPolicyRequest>,
) -> Result<StatusCode, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.actor_id),
        Permission::Manage,
    )
    .await?;
    let event = state
        .store
        .set_budget_policy(
            corp_id,
            actor_id,
            request.actor_tokens_per_24h,
            request.actor_cost_microusd_per_24h,
            request.corp_tokens_per_24h,
            request.corp_cost_microusd_per_24h,
            request.no_progress_event_limit,
            request.repeated_tool_limit,
        )
        .await
        .map_err(map_store_error)?;
    publish(&state, event);
    Ok(StatusCode::NO_CONTENT)
}

fn valid_scope_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
}

async fn dispatch_pending_runner_commands(state: &AppState, runner_id: &str) -> anyhow::Result<()> {
    let Some(sender) = state
        .runners
        .get(runner_id)
        .map(|connection| connection.tx.clone())
    else {
        return Ok(());
    };
    for command in state.store.pending_runner_commands(runner_id).await? {
        let outgoing = decode_runner_command(&command)?;
        if sender.send(outgoing).is_err() {
            break;
        }
        state
            .store
            .mark_runner_command_dispatched(command.id)
            .await?;
    }
    Ok(())
}

fn decode_runner_command(command: &PendingRunnerCommand) -> anyhow::Result<ServerToRunner> {
    match command.command_kind.as_str() {
        "approval_decision" => Ok(ServerToRunner::ApprovalDecision {
            command_id: command.id,
            run_id: command.run_id,
            approval_id: command
                .payload
                .get("approval_id")
                .and_then(serde_json::Value::as_str)
                .context("approval command omitted approval_id")
                .and_then(|value| Uuid::parse_str(value).context("approval id is invalid"))?,
            approved: command
                .payload
                .get("approved")
                .and_then(serde_json::Value::as_bool)
                .context("approval command omitted approved")?,
            note: command
                .payload
                .get("note")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        }),
        "circuit_breaker" => Ok(ServerToRunner::CircuitBreaker {
            command_id: command.id,
            run_id: command.run_id,
            stage: command
                .payload
                .get("stage")
                .and_then(serde_json::Value::as_str)
                .context("breaker command omitted stage")?
                .to_owned(),
            reason: command
                .payload
                .get("reason")
                .and_then(serde_json::Value::as_str)
                .context("breaker command omitted reason")?
                .to_owned(),
        }),
        other => Err(anyhow::anyhow!("unknown runner command kind {other}")),
    }
}

async fn snapshot(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(corp_id): Path<Uuid>,
    Query(query): Query<SnapshotQuery>,
) -> Result<Json<SnapshotResponse>, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(query.actor_id),
        Permission::Read,
    )
    .await?;
    let snapshot = state
        .store
        .snapshot(corp_id, actor_id)
        .await
        .map_err(map_store_error)?;
    let runners = state
        .store
        .runner_records(corp_id)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(|record| RunnerSummary {
            id: record.id,
            corp_id: record.corp_id,
            hostname: record.hostname,
            os: record.os,
            capabilities: serde_json::from_value::<Vec<RunnerCapability>>(record.capabilities)
                .unwrap_or_default(),
            connected: record.status == "connected",
            status: record.status,
            last_seen_at: record.last_seen_at.to_rfc3339(),
            grace_expires_at: record.grace_expires_at.map(|value| value.to_rfc3339()),
        })
        .collect();
    Ok(Json(SnapshotResponse { snapshot, runners }))
}

#[derive(Debug, Deserialize)]
struct SnapshotQuery {
    actor_id: Uuid,
}

async fn create_mission(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(corp_id): Path<Uuid>,
    Json(request): Json<CreateMissionRequest>,
) -> Result<Json<CreateMissionResponse>, ApiError> {
    let requested_by = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.requested_by),
        Permission::Operate,
    )
    .await?;
    let strategy = request.strategy.as_deref().unwrap_or("single");
    if !request.secret_refs.is_empty() && strategy != "single" {
        return Err(ApiError::bad_request(
            "secret references currently require the single-task strategy",
        ));
    }
    validate_requested_model(
        &state,
        corp_id,
        request.preferred_adapter.as_deref(),
        request.preferred_model.as_deref(),
        request.reasoning_effort.as_deref(),
    )?;
    let agents = state
        .store
        .agents_for_planning(corp_id)
        .await
        .map_err(ApiError::internal)?;
    let plan = state
        .strategies
        .plan(
            strategy,
            &PlanningRequest {
                mission_title: &request.title,
                preferred_adapter: request.preferred_adapter.as_deref(),
                preferred_model: request.preferred_model.as_deref(),
                reasoning_effort: request.reasoning_effort.as_deref(),
                secret_refs: &request.secret_refs,
                budget_tokens: request.budget_tokens,
                budget_cost_microusd: request.budget_cost_microusd,
            },
            &agents,
        )
        .map_err(ApiError::bad_request)?;
    let (ids, events) = state
        .store
        .create_mission(corp_id, requested_by, &request.title, &plan)
        .await
        .map_err(map_store_error)?;
    for event in events {
        publish(&state, event);
    }
    Ok(Json(CreateMissionResponse {
        mission_id: ids.mission_id,
        task_id: ids.task_ids[0],
        task_ids: ids.task_ids,
        strategy: plan.strategy,
    }))
}

async fn create_room_message(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, room_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<CreateRoomMessageRequest>,
) -> Result<Json<CreateRoomMessageResponse>, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.actor_id),
        Permission::PostMessage,
    )
    .await?;
    let outcome = state
        .store
        .create_room_message(NewRoomMessageInput {
            corp_id,
            room_id,
            actor_id,
            body: request.body,
            reply_to_id: request.reply_to_id,
            mentions: request.mentions,
            link: request.link,
        })
        .await
        .map_err(map_store_error)?;
    publish(&state, outcome.event);
    Ok(Json(CreateRoomMessageResponse {
        message_id: outcome.message.id,
    }))
}

async fn launch_mission(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, mission_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<LaunchMissionRequest>,
) -> Result<Json<LaunchMissionResponse>, ApiError> {
    let requested_by = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.requested_by),
        Permission::Operate,
    )
    .await?;
    let records = schedule_ready_tasks(&state, corp_id, mission_id, Some(requested_by))
        .await
        .map_err(ApiError::conflict)?;
    let first = records
        .first()
        .context("mission has no ready tasks with a compatible runner")
        .map_err(ApiError::conflict)?;

    Ok(Json(LaunchMissionResponse {
        run_id: first.0.run_id,
        runner_id: first.1.clone(),
        run_ids: records.iter().map(|(record, _)| record.run_id).collect(),
        runner_ids: records
            .iter()
            .map(|(_, runner_id)| runner_id.clone())
            .collect(),
    }))
}

async fn schedule_ready_tasks(
    state: &AppState,
    corp_id: Uuid,
    mission_id: Uuid,
    requested_by: Option<Uuid>,
) -> anyhow::Result<Vec<(LaunchRecord, String)>> {
    let candidates = state.store.schedulable_tasks(corp_id, mission_id).await?;
    let mut scheduled = Vec::new();
    for candidate in candidates {
        let Some((runner_id, runner_tx)) = select_runner(
            state,
            corp_id,
            &candidate.required_adapter,
            candidate.required_model.as_deref(),
            candidate.required_reasoning_effort.as_deref(),
        ) else {
            continue;
        };
        let Ok((record, event)) = state
            .store
            .create_task_run(
                corp_id,
                mission_id,
                candidate.task_id,
                requested_by,
                &runner_id,
            )
            .await
        else {
            continue;
        };
        let secrets = match resolve_run_secrets(state, &record, &runner_id).await {
            Ok(secrets) => secrets,
            Err(error) => {
                if let Ok(event) = state
                    .store
                    .fail_run_before_dispatch(
                        corp_id,
                        record.run_id,
                        &format!("secret broker denied assignment: {error}"),
                    )
                    .await
                {
                    publish(state, event);
                }
                continue;
            }
        };
        runner_tx
            .send(ServerToRunner::StartRun {
                corp_id: record.corp_id,
                room_id: record.room_id,
                mission_id: record.mission_id,
                task_id: record.task_id,
                run_id: record.run_id,
                agent_id: record.agent_id,
                assignment_token: record.assignment_token,
                adapter: record.adapter.clone(),
                mission_title: record.mission_title.clone(),
                model: record.model.clone(),
                reasoning_effort: record.reasoning_effort.clone(),
                verification_policy: record.verification_policy.clone(),
                secrets,
            })
            .map_err(|_| anyhow::anyhow!("runner disconnected before accepting the run"))?;
        publish(state, event);
        scheduled.push((record, runner_id));
    }
    Ok(scheduled)
}

async fn resolve_run_secrets(
    state: &AppState,
    record: &LaunchRecord,
    runner_id: &str,
) -> anyhow::Result<Vec<ResolvedSecret>> {
    let (grants, events) = state
        .store
        .grant_run_secrets(
            record.corp_id,
            record.task_id,
            record.run_id,
            runner_id,
            &record.secret_refs,
        )
        .await?;
    let mut resolved = Vec::with_capacity(grants.len());
    for grant in grants {
        let plaintext = state.secret_cipher.decrypt(
            record.corp_id,
            grant.secret_id,
            &grant.name,
            &grant.ciphertext,
            &grant.nonce,
        )?;
        let value = String::from_utf8(plaintext).context("secret value is not valid UTF-8")?;
        resolved.push(ResolvedSecret {
            grant_id: grant.grant_id,
            secret_id: grant.secret_id,
            env_name: grant.env_name,
            value,
            tool: grant.tool,
            resource: grant.resource,
            expires_at: grant.expires_at.to_rfc3339(),
            assurance: "environment_reduced_assurance".to_owned(),
        });
    }
    for event in events {
        publish(state, event);
    }
    Ok(resolved)
}

async fn schedule_ready_corp(state: &AppState, corp_id: Uuid) -> anyhow::Result<()> {
    for mission_id in state.store.schedulable_mission_ids(corp_id).await? {
        schedule_ready_tasks(state, corp_id, mission_id, None).await?;
    }
    Ok(())
}

fn select_runner(
    state: &AppState,
    corp_id: Uuid,
    required_adapter: &str,
    required_model: Option<&str>,
    required_reasoning_effort: Option<&str>,
) -> Option<(String, mpsc::UnboundedSender<ServerToRunner>)> {
    let mut runners = state
        .runners
        .iter()
        .filter(|entry| {
            entry.corp_id == corp_id
                && entry.capabilities.iter().any(|capability| {
                    capability.name == required_adapter
                        && capability.available
                        && required_model.is_none_or(|required_model| {
                            capability.models.iter().any(|model| {
                                model.id == required_model
                                    && model.policy_state.as_deref() != Some("disabled")
                                    && required_reasoning_effort.is_none_or(|effort| {
                                        model.supports_reasoning_effort
                                            && model
                                                .supported_reasoning_efforts
                                                .iter()
                                                .any(|supported| supported == effort)
                                    })
                            })
                        })
                })
        })
        .map(|entry| (entry.key().clone(), entry.tx.clone()))
        .collect::<Vec<_>>();
    runners.sort_by(|left, right| left.0.cmp(&right.0));
    runners.into_iter().next()
}

fn validate_requested_model(
    state: &AppState,
    corp_id: Uuid,
    adapter: Option<&str>,
    model: Option<&str>,
    reasoning_effort: Option<&str>,
) -> Result<(), ApiError> {
    let Some(model_id) = model else {
        if reasoning_effort.is_some() {
            return Err(ApiError::bad_request(
                "reasoning effort requires an explicit model",
            ));
        }
        return Ok(());
    };
    let adapter = adapter.ok_or_else(|| {
        ApiError::bad_request("model selection requires an explicit agent runtime")
    })?;
    let models = state
        .runners
        .iter()
        .filter(|entry| entry.corp_id == corp_id)
        .flat_map(|entry| entry.capabilities.clone())
        .filter(|capability| capability.available && capability.name == adapter)
        .flat_map(|capability| capability.models)
        .filter(|model| model.id == model_id && model.policy_state.as_deref() != Some("disabled"))
        .collect::<Vec<_>>();
    if models.is_empty() {
        return Err(ApiError::bad_request(format!(
            "model {model_id} is not available from a connected {adapter} runner"
        )));
    }
    if let Some(effort) = reasoning_effort
        && !models.iter().any(|model| {
            model.supports_reasoning_effort
                && model
                    .supported_reasoning_efforts
                    .iter()
                    .any(|supported| supported == effort)
        })
    {
        return Err(ApiError::bad_request(format!(
            "model {model_id} does not support reasoning effort {effort} on a connected {adapter} runner"
        )));
    }
    Ok(())
}

async fn resume_run(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, source_run_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<ResumeRunRequest>,
) -> Result<Json<ResumeRunResponse>, ApiError> {
    let requested_by = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.requested_by),
        Permission::Operate,
    )
    .await?;
    let prompt = request.prompt.trim();
    if prompt.is_empty() {
        return Err(ApiError::bad_request("resume prompt cannot be empty"));
    }
    if prompt.len() > 8_000 {
        return Err(ApiError::bad_request(
            "resume prompt cannot exceed 8,000 characters",
        ));
    }
    let (record, event) = state
        .store
        .create_resume_run(corp_id, source_run_id, requested_by)
        .await
        .map_err(ApiError::conflict)?;
    let runner = state
        .runners
        .get(&record.runner_id)
        .ok_or_else(|| ApiError::conflict("source run's runner is disconnected"))?;
    if runner.corp_id != corp_id {
        return Err(ApiError::forbidden(
            "source run's runner is enrolled to a different Corp",
        ));
    }
    let launch_record = LaunchRecord {
        corp_id: record.corp_id,
        room_id: record.room_id,
        mission_id: record.mission_id,
        task_id: record.task_id,
        run_id: record.run_id,
        agent_id: record.agent_id,
        assignment_token: record.assignment_token,
        attempt: 0,
        adapter: record.adapter.clone(),
        mission_title: prompt.to_owned(),
        model: record.model.clone(),
        reasoning_effort: record.reasoning_effort.clone(),
        verification_policy: record.verification_policy.clone(),
        secret_refs: record.secret_refs.clone(),
    };
    let secrets = match resolve_run_secrets(&state, &launch_record, &record.runner_id).await {
        Ok(secrets) => secrets,
        Err(error) => {
            let failure = state
                .store
                .fail_run_before_dispatch(
                    corp_id,
                    record.run_id,
                    &format!("secret broker denied resumed assignment: {error}"),
                )
                .await
                .map_err(ApiError::internal)?;
            publish(&state, failure);
            return Err(ApiError::conflict(
                "secret broker denied resumed assignment",
            ));
        }
    };
    runner
        .tx
        .send(ServerToRunner::ResumeRun {
            corp_id: record.corp_id,
            room_id: record.room_id,
            mission_id: record.mission_id,
            task_id: record.task_id,
            run_id: record.run_id,
            workspace_run_id: record.workspace_run_id,
            agent_id: record.agent_id,
            assignment_token: record.assignment_token,
            adapter: record.adapter,
            provider_session_id: record.provider_session_id.clone(),
            prompt: prompt.to_owned(),
            model: record.model,
            reasoning_effort: record.reasoning_effort,
            verification_policy: record.verification_policy,
            secrets,
        })
        .map_err(|_| ApiError::conflict("runner disconnected before accepting resume"))?;
    publish(&state, event);
    Ok(Json(ResumeRunResponse {
        run_id: record.run_id,
        runner_id: record.runner_id,
        provider_session_id: record.provider_session_id,
    }))
}

async fn decide_verification(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, run_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<VerificationDecisionRequest>,
) -> Result<Json<VerificationDecisionResponse>, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.actor_id),
        Permission::Approve,
    )
    .await?;
    let outcome = state
        .store
        .decide_verification(corp_id, run_id, actor_id, request.approved, &request.note)
        .await
        .map_err(map_store_error)?;
    publish(&state, outcome.event);
    if request.approved {
        schedule_ready_corp(&state, outcome.corp_id)
            .await
            .map_err(ApiError::internal)?;
    }
    Ok(Json(VerificationDecisionResponse {
        run_id: outcome.run_id,
        status: outcome.status,
    }))
}

async fn claim_lease(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, agent_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<ClaimLeaseRequest>,
) -> Result<Json<ClaimLeaseResponse>, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.actor_id),
        Permission::Operate,
    )
    .await?;
    let outcome = state
        .store
        .acquire_lease(corp_id, agent_id, actor_id)
        .await
        .map_err(ApiError::bad_request)?;
    if let Some(event) = outcome.event {
        publish(&state, event);
    }
    Ok(Json(ClaimLeaseResponse {
        acquired: outcome.acquired,
        token: outcome.acquired.then_some(outcome.lease.token),
        holder_actor_id: outcome.lease.actor_id,
        expires_at: outcome.lease.expires_at.to_rfc3339(),
    }))
}

async fn release_lease(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, agent_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<ReleaseLeaseRequest>,
) -> Result<Json<LeaseMutationResponse>, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.actor_id),
        Permission::Operate,
    )
    .await?;
    let outcome = state
        .store
        .release_lease(corp_id, agent_id, actor_id, request.token)
        .await
        .map_err(ApiError::conflict)?;
    publish(&state, outcome.event);
    Ok(Json(LeaseMutationResponse {
        holder_actor_id: None,
        token: None,
        expires_at: None,
    }))
}

async fn transfer_lease(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, agent_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<TransferLeaseRequest>,
) -> Result<Json<LeaseMutationResponse>, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.actor_id),
        Permission::Operate,
    )
    .await?;
    let outcome = state
        .store
        .transfer_lease(
            corp_id,
            agent_id,
            actor_id,
            request.token,
            request.to_actor_id,
        )
        .await
        .map_err(ApiError::conflict)?;
    let lease = outcome
        .lease
        .context("transfer outcome omitted the new lease")
        .map_err(ApiError::internal)?;
    publish(&state, outcome.event);
    Ok(Json(LeaseMutationResponse {
        holder_actor_id: Some(lease.actor_id),
        token: None,
        expires_at: Some(lease.expires_at.to_rfc3339()),
    }))
}

async fn queue_message(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, agent_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<QueueMessageRequest>,
) -> Result<Json<QueueMessageResponse>, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.actor_id),
        Permission::Operate,
    )
    .await?;
    let outcome = state
        .store
        .queue_message(
            corp_id,
            agent_id,
            actor_id,
            request.lease_token,
            &request.text,
        )
        .await
        .map_err(ApiError::conflict)?;
    publish(&state, outcome.event);

    if outcome.delivery == "immediate"
        && let (Some(run_id), Some(runner_id)) = (outcome.run_id, outcome.runner_id)
        && let Some(runner) = state.runners.get(&runner_id)
        && let Some(lease_token) = request.lease_token
    {
        let _ = runner.tx.send(ServerToRunner::ControlMessage {
            corp_id,
            run_id,
            agent_id,
            actor_id,
            lease_token,
            text: request.text,
        });
    }

    Ok(Json(QueueMessageResponse {
        message_id: outcome.message.id,
        delivery: outcome.delivery,
    }))
}

async fn emergency_stop(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, agent_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<EmergencyStopRequest>,
) -> Result<Json<EmergencyStopResponse>, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.actor_id),
        Permission::EmergencyStop,
    )
    .await?;
    let outcome = match state
        .store
        .request_emergency_stop(corp_id, agent_id, actor_id, &request.reason)
        .await
    {
        Ok(outcome) => outcome,
        Err(error) if error.to_string().starts_with("forbidden:") => {
            return Err(ApiError::forbidden(error));
        }
        Err(error) => return Err(ApiError::conflict(error)),
    };
    let runner = state
        .runners
        .get(&outcome.runner_id)
        .ok_or_else(|| ApiError::conflict("run's runner is disconnected"))?;
    runner
        .tx
        .send(ServerToRunner::StopRun {
            run_id: outcome.run_id,
            reason: request.reason,
        })
        .map_err(|_| ApiError::conflict("runner disconnected before stop delivery"))?;
    publish(&state, outcome.event);
    Ok(Json(EmergencyStopResponse {
        run_id: outcome.run_id,
        requested: true,
    }))
}

async fn interrupt_run(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, agent_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<InterruptRunRequest>,
) -> Result<Json<InterruptRunResponse>, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.actor_id),
        Permission::Operate,
    )
    .await?;
    let outcome = state
        .store
        .request_interrupt(
            corp_id,
            agent_id,
            actor_id,
            request.lease_token,
            &request.reason,
        )
        .await
        .map_err(ApiError::conflict)?;
    let runner = state
        .runners
        .get(&outcome.runner_id)
        .ok_or_else(|| ApiError::conflict("run's runner is disconnected"))?;
    runner
        .tx
        .send(ServerToRunner::InterruptRun {
            run_id: outcome.run_id,
            reason: request.reason,
        })
        .map_err(|_| ApiError::conflict("runner disconnected before interrupt delivery"))?;
    publish(&state, outcome.event);
    Ok(Json(InterruptRunResponse {
        run_id: outcome.run_id,
        requested: true,
    }))
}

async fn browser_websocket(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Path(corp_id): Path<Uuid>,
    Query(query): Query<BrowserQuery>,
) -> Result<Response, ApiError> {
    let principal = match state.auth.mode() {
        ServerMode::Development => Principal::Development,
        ServerMode::Production => {
            let token = query
                .ticket
                .as_deref()
                .ok_or_else(|| ApiError::unauthorized("WebSocket ticket is required"))?;
            let actor_id = state
                .auth
                .consume_websocket_ticket(corp_id, token)
                .map_err(ApiError::unauthorized)?;
            let visible_rooms = state
                .store
                .visible_room_ids(corp_id, actor_id)
                .await
                .map_err(ApiError::internal)?
                .into_iter()
                .collect::<HashSet<_>>();
            return Ok(ws
                .on_upgrade(move |socket| {
                    browser_socket(
                        socket,
                        state,
                        corp_id,
                        actor_id,
                        query.after_seq,
                        visible_rooms,
                    )
                })
                .into_response());
        }
    };
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        query.actor_id,
        Permission::Read,
    )
    .await?;
    let visible_rooms = state
        .store
        .visible_room_ids(corp_id, actor_id)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .collect::<HashSet<_>>();
    Ok(ws
        .on_upgrade(move |socket| {
            browser_socket(
                socket,
                state,
                corp_id,
                actor_id,
                query.after_seq,
                visible_rooms,
            )
        })
        .into_response())
}

#[derive(Debug, Deserialize)]
struct BrowserQuery {
    actor_id: Option<Uuid>,
    ticket: Option<String>,
    #[serde(default)]
    after_seq: i64,
}

async fn browser_socket(
    socket: WebSocket,
    state: AppState,
    corp_id: Uuid,
    actor_id: Uuid,
    after_seq: i64,
    visible_rooms: HashSet<Uuid>,
) {
    let (mut sender, mut receiver) = socket.split();
    let mut events = state.event_tx.subscribe();
    let mut cursor = after_seq.max(0);

    loop {
        let replay = match state
            .store
            .events_after(corp_id, actor_id, cursor, 500)
            .await
        {
            Ok(replay) => replay,
            Err(error) => {
                warn!(%error, %corp_id, cursor, "browser replay query failed");
                return;
            }
        };
        if replay.is_empty() {
            break;
        }
        let page_len = replay.len();
        for event in replay {
            cursor = cursor.max(event.seq);
            if send_json(
                &mut sender,
                &BrowserSocketMessage::Event {
                    event: Box::new(event),
                },
            )
            .await
            .is_err()
            {
                return;
            }
        }
        if page_len < 500 {
            break;
        }
    }

    let ready = BrowserSocketMessage::Ready {
        corp_id,
        replayed_through: cursor,
    };
    if send_json(&mut sender, &ready).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(error)) => {
                        warn!(%error, "browser websocket read failed");
                        break;
                    }
                    _ => {}
                }
            }
            event = events.recv() => {
                match event {
                    Ok(event)
                        if event.corp_id == corp_id
                            && event.seq > cursor
                            && event
                                .room_id
                                .is_none_or(|room_id| visible_rooms.contains(&room_id)) =>
                    {
                        cursor = event.seq;
                        if send_json(
                            &mut sender,
                            &BrowserSocketMessage::Event {
                                event: Box::new(event),
                            },
                        )
                        .await
                        .is_err()
                        {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(skipped, "browser event subscriber lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

async fn runner_websocket(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| runner_socket(socket, state))
}

async fn runner_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let (command_tx, mut command_rx) = mpsc::unbounded_channel::<ServerToRunner>();
    let writer = tokio::spawn(async move {
        while let Some(command) = command_rx.recv().await {
            if send_json(&mut sender, &command).await.is_err() {
                break;
            }
        }
    });

    let mut registered: Option<(String, Uuid, Uuid)> = None;
    while let Some(message) = receiver.next().await {
        let message = match message {
            Ok(Message::Text(text)) => text,
            Ok(Message::Close(_)) => break,
            Ok(_) => continue,
            Err(error) => {
                if error
                    .to_string()
                    .contains("reset without closing handshake")
                {
                    info!(%error, "runner transport disconnected");
                } else {
                    warn!(%error, "runner websocket read failed");
                }
                break;
            }
        };
        let incoming: RunnerToServer = match serde_json::from_str(message.as_str()) {
            Ok(incoming) => incoming,
            Err(error) => {
                warn!(%error, "invalid runner message");
                continue;
            }
        };
        match incoming {
            RunnerToServer::Register {
                runner_id,
                corp_id,
                credential,
                connection_epoch,
                hostname,
                os,
                capabilities,
                active_runs,
            } => {
                let next_credential = new_runner_token("runner");
                let credential_expires_at =
                    Utc::now() + ChronoDuration::seconds(state.runner_credential_ttl_secs);
                let authentication = match state
                    .store
                    .authenticate_and_rotate_runner(
                        corp_id,
                        &runner_id,
                        &hash_secret(&credential),
                        &hash_secret(&next_credential),
                        credential_expires_at,
                    )
                    .await
                {
                    Ok(authentication) => authentication,
                    Err(error) => {
                        warn!(%error, %runner_id, %corp_id, "runner authentication rejected");
                        let _ = command_tx.send(ServerToRunner::RegistrationRejected {
                            reason: "runner credential rejected".to_owned(),
                        });
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        break;
                    }
                };
                publish(&state, authentication.event);
                let record = match state
                    .store
                    .runner_connected(RunnerConnectInput {
                        id: runner_id.clone(),
                        corp_id,
                        hostname,
                        os,
                        capabilities: match serde_json::to_value(&capabilities) {
                            Ok(value) => value,
                            Err(error) => {
                                warn!(%error, %runner_id, "failed to serialize runner capabilities");
                                continue;
                            }
                        },
                        connection_epoch,
                    })
                    .await
                {
                    Ok(record) => record,
                    Err(error) => {
                        warn!(%error, %runner_id, "failed to persist runner connection");
                        continue;
                    }
                };

                let previous = state.runners.insert(
                    runner_id.clone(),
                    RunnerConnection {
                        corp_id,
                        connection_epoch,
                        tx: command_tx.clone(),
                        capabilities: capabilities.clone(),
                    },
                );
                if let Some(previous) = previous
                    && previous.connection_epoch != connection_epoch
                {
                    let _ = previous.tx.send(ServerToRunner::Disconnect {
                        reason: "runner connected with a newer epoch".to_owned(),
                        reconnect_delay_ms: 0,
                    });
                }
                registered = Some((runner_id.clone(), corp_id, connection_epoch));
                let _ = command_tx.send(ServerToRunner::Registered {
                    runner_id: runner_id.clone(),
                    credential: next_credential,
                    expires_at: authentication.expires_at.to_rfc3339(),
                });
                let rotation_tx = command_tx.clone();
                let rotate_after = (state.runner_credential_ttl_secs - 60).max(60) as u64;
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(rotate_after)).await;
                    let _ = rotation_tx.send(ServerToRunner::Disconnect {
                        reason: "rotate runner workload credential".to_owned(),
                        reconnect_delay_ms: 0,
                    });
                });
                let claims = active_runs
                    .iter()
                    .map(|claim| RunClaim {
                        run_id: claim.run_id,
                        assignment_token: claim.assignment_token,
                    })
                    .collect::<Vec<_>>();
                match state
                    .store
                    .reconcile_runner_claims(&runner_id, connection_epoch, &claims)
                    .await
                {
                    Ok(outcome) => {
                        for event in outcome.events {
                            publish(&state, event);
                        }
                        for stale_run in outcome.stale {
                            let _ = command_tx.send(ServerToRunner::StopRun {
                                run_id: stale_run,
                                reason: "stale or unknown run assignment after reconnect"
                                    .to_owned(),
                            });
                        }
                        let accepted = outcome.accepted;
                        let finalize_state = state.clone();
                        let finalize_runner = runner_id.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                            match finalize_state
                                .store
                                .mark_unclaimed_runner_runs_lost(
                                    &finalize_runner,
                                    connection_epoch,
                                    &accepted,
                                )
                                .await
                            {
                                Ok(events) => {
                                    for event in events {
                                        publish(&finalize_state, event);
                                    }
                                }
                                Err(error) => {
                                    warn!(%error, runner_id = %finalize_runner, "runner reconciliation finalization failed")
                                }
                            }
                        });
                    }
                    Err(error) => {
                        warn!(%error, %runner_id, "runner active-run reconciliation failed")
                    }
                }
                if let Err(error) = dispatch_pending_runner_commands(&state, &runner_id).await {
                    warn!(%error, %runner_id, "failed to dispatch durable runner commands");
                }
                info!(
                    %runner_id,
                    %connection_epoch,
                    status = %record.status,
                    "runner connected"
                );
            }
            RunnerToServer::Heartbeat {
                runner_id,
                connection_epoch,
                active_runs,
            } => {
                if !registered
                    .as_ref()
                    .is_some_and(|(registered_id, _, epoch)| {
                        registered_id == &runner_id && *epoch == connection_epoch
                    })
                {
                    warn!(%runner_id, "heartbeat from unregistered runner");
                    continue;
                }
                match state
                    .store
                    .runner_heartbeat(&runner_id, connection_epoch)
                    .await
                {
                    Ok(true) => {
                        tracing::debug!(
                            %runner_id,
                            active_run_count = active_runs.len(),
                            "runner heartbeat persisted"
                        );
                    }
                    Ok(false) => warn!(%runner_id, "runner heartbeat epoch was rejected"),
                    Err(error) => warn!(%error, %runner_id, "runner heartbeat persistence failed"),
                }
            }
            RunnerToServer::RunEvent {
                event_id,
                runner_id,
                corp_id,
                run_id,
                agent_id,
                event_type,
                payload,
            } => {
                if !registered
                    .as_ref()
                    .is_some_and(|(registered_id, registered_corp, _)| {
                        registered_id == &runner_id && *registered_corp == corp_id
                    })
                {
                    warn!(%runner_id, "run event runner id does not match registered socket");
                    continue;
                }
                match state
                    .store
                    .apply_runner_event(RunnerEventInput {
                        event_id,
                        runner_id: runner_id.clone(),
                        corp_id,
                        run_id,
                        agent_id,
                        event_type: event_type.clone(),
                        payload,
                    })
                    .await
                {
                    Ok(Some(event)) => {
                        publish(&state, event);
                        if matches!(event_type.as_str(), "run.usage" | "run.tool_activity") {
                            match state.store.evaluate_circuit_breaker(corp_id, run_id).await {
                                Ok(outcome) => {
                                    if let Some(event) = outcome.event {
                                        publish(&state, event);
                                    }
                                    if outcome.command.is_some()
                                        && let Err(error) =
                                            dispatch_pending_runner_commands(&state, &runner_id)
                                                .await
                                    {
                                        warn!(%error, %runner_id, %run_id, "failed to dispatch circuit-breaker command");
                                    }
                                }
                                Err(error) => {
                                    warn!(%error, %run_id, "circuit-breaker evaluation failed")
                                }
                            }
                        }
                        if matches!(event_type.as_str(), "run.completed" | "run.failed") {
                            let schedule_state = state.clone();
                            tokio::spawn(async move {
                                if let Err(error) =
                                    schedule_ready_corp(&schedule_state, corp_id).await
                                {
                                    warn!(%error, %corp_id, "automatic Corp scheduling failed");
                                }
                            });
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        if error
                            .to_string()
                            .contains("runner event does not match an active run")
                        {
                            info!(%run_id, %event_type, "ignored stale runner event");
                        } else {
                            warn!(%error, %run_id, %event_type, "failed to apply runner event")
                        }
                    }
                }
            }
        }
    }

    if let Some((runner_id, _, connection_epoch)) = registered {
        let is_current = state
            .runners
            .get(&runner_id)
            .is_some_and(|entry| entry.connection_epoch == connection_epoch);
        if is_current {
            state.runners.remove(&runner_id);
        }
        match state
            .store
            .runner_disconnected(&runner_id, connection_epoch, state.runner_grace_secs)
            .await
        {
            Ok(events) => {
                for event in events {
                    publish(&state, event);
                }
            }
            Err(error) => warn!(%error, %runner_id, "failed to start runner grace period"),
        }
        let expiry_state = state.clone();
        let expiry_runner = runner_id.clone();
        let grace_seconds = state.runner_grace_secs;
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(grace_seconds as u64)).await;
            match expiry_state
                .store
                .expire_runner_grace(&expiry_runner, connection_epoch)
                .await
            {
                Ok(events) => {
                    for event in events {
                        publish(&expiry_state, event);
                    }
                }
                Err(error) => {
                    warn!(%error, runner_id = %expiry_runner, "runner grace expiry failed")
                }
            }
        });
        info!(%runner_id, %connection_epoch, "runner disconnected into grace");
    }
    writer.abort();
}

fn publish(state: &AppState, event: DomainEvent) {
    let _ = state.event_tx.send(event);
}

async fn send_json<S, T>(sender: &mut S, value: &T) -> Result<(), ()>
where
    S: futures_util::Sink<Message> + Unpin,
    S::Error: std::fmt::Display,
    T: Serialize,
{
    let text = serde_json::to_string(value).map_err(|error| {
        error!(%error, "websocket serialization failed");
    })?;
    sender
        .send(Message::Text(text.into()))
        .await
        .map_err(|error| {
            warn!(%error, "websocket send failed");
        })
}
