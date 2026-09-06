mod artifacts;
mod auth;
mod planning;
mod secrets;

use std::{
    collections::HashSet, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration as StdDuration,
};

use anyhow::Context;
use axum::{
    Extension, Json, Router,
    body::Body,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{
        HeaderMap, HeaderValue, Method, Request, StatusCode,
        header::{
            AUTHORIZATION, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE, ETAG, HeaderName,
        },
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{Duration as ChronoDuration, Utc};
use clap::Parser;
use crony_domain::{
    DomainEvent, ManualVerificationGate, TaskGraphPlan, TaskSecretReference, VerificationPolicy,
};
use crony_protocol::{
    ActionApprovalDecisionRequest, ActionApprovalDecisionResponse, BrowserSocketMessage,
    ClaimFactoryWorkItemRequest, ClaimLeaseRequest, ClaimLeaseResponse,
    ConfigureFactoryControllerRequest, ControlFactoryControllerRequest,
    CreateMissionContractRevisionRequest, CreateMissionRequest, CreateMissionResponse,
    CreatePublicationPublisherCredentialRequest, CreatePublicationPublisherCredentialResponse,
    CreateRoomMessageRequest, CreateRoomMessageResponse, CreateRunnerEnrollmentRequest,
    CreateRunnerEnrollmentResponse, CreateSecretRequest, CreateSecretResponse,
    DecideMissionBudgetRevisionRequest, DemoBootstrapResponse, EmergencyStopRequest,
    EmergencyStopResponse, FactoryControllerHeartbeatRequest, FactoryControllerResponse,
    FactoryMissionContract, FactoryPublicationContextResponse, FactoryWorkItemResponse,
    InterruptRunRequest, InterruptRunResponse, LaunchMissionRequest, LaunchMissionResponse,
    LeaseMutationResponse, LookupFactoryWorkItemsRequest, LookupFactoryWorkItemsResponse,
    MaterializeFactoryMissionRequest, MaterializeFactoryMissionResponse,
    MissionBudgetRevisionResponse, MissionContractRevisionResponse, MissionSource,
    PreflightFactoryMissionRequest, PreflightFactoryMissionResponse,
    ProposeMissionBudgetRevisionRequest, PullRequestPublicationCheckpoint,
    PullRequestPublicationResponse, QueueMessageRequest, QueueMessageResponse,
    RecordPullRequestPublicationCheckpointRequest, ReleaseLeaseRequest,
    RenewFactoryWorkItemRequest, RenewPullRequestPublicationRequest, ResolvedSecret,
    ResumeRunRequest, ResumeRunResponse, RevokePublicationPublisherCredentialRequest,
    RevokePublicationPublisherCredentialResponse, RevokeRunnerRequest, RevokeRunnerResponse,
    RevokeSecretRequest, RunnerCapability, RunnerSummary, RunnerToServer, ServerToRunner,
    SetBudgetPolicyRequest, SnapshotResponse, StartPullRequestPublicationRequest,
    TransferLeaseRequest, TransitionFactoryWorkItemRequest, UpgradeFactorySourceCommitRequest,
    VerificationDecisionRequest, VerificationDecisionResponse,
};
use crony_store::{
    ClaimFactoryWorkItemInput, ConfigureFactoryControllerInput, ControlFactoryControllerInput,
    CreateMissionContractRevisionInput, DecideMissionBudgetRevisionInput, FactorySourceInput,
    HeartbeatFactoryControllerInput, LaunchRecord, MaterializeFactoryMissionInput,
    MissionFinishScopeInput, NewRoomMessageInput, PendingRunnerCommand, PgStore,
    PreflightFactoryMissionInput, ProposeMissionBudgetRevisionInput,
    PullRequestPublicationCheckpointInput, PullRequestPublicationOutcome, QueuedRunMessage,
    RecordPullRequestPublicationCheckpointInput, RejectFactoryMaterializationInput,
    RenewFactoryWorkItemInput, RenewPullRequestPublicationInput, RunClaim, RunnerConnectInput,
    RunnerEventInput, StartPullRequestPublicationInput, TransitionFactoryWorkItemInput,
    UpgradeFactorySourceCommitInput,
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

use artifacts::{
    ArtifactIdentity, ArtifactStore, StagedArtifact, artifact_error_is_missing_objects,
    artifact_error_is_permanent,
};
use auth::{AuthService, CorpRole, Permission, Principal, ServerMode};
use planning::{PlanningRequest, StrategyRegistry, uses_deterministic_harness, validate_plan};
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
        env = "CRONY_RUNNER_STARTUP_RECOVERY",
        default_value_t = true,
        action = clap::ArgAction::Set
    )]
    runner_startup_recovery: bool,

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

    #[arg(
        long,
        env = "CRONY_PUBLICATION_PUBLISHER_CREDENTIAL_TTL_SECS",
        default_value_t = 86_400
    )]
    publication_publisher_credential_ttl_secs: i64,

    #[arg(long, env = "CRONY_SECRET_MASTER_KEY_HEX")]
    secret_master_key_hex: Option<String>,

    #[arg(long, env = "CRONY_OBJECT_STORE_BACKEND", default_value = "local")]
    object_store_backend: String,

    #[arg(
        long,
        env = "CRONY_OBJECT_STORE_LOCAL_ROOT",
        default_value = "./output/artifact-objects"
    )]
    object_store_local_root: PathBuf,

    #[arg(long, env = "CRONY_OBJECT_STORE_ENDPOINT")]
    object_store_endpoint: Option<String>,

    #[arg(long, env = "CRONY_OBJECT_STORE_BUCKET")]
    object_store_bucket: Option<String>,

    #[arg(long, env = "CRONY_OBJECT_STORE_REGION", default_value = "us-east-1")]
    object_store_region: String,

    #[arg(long, env = "CRONY_OBJECT_STORE_ACCESS_KEY")]
    object_store_access_key: Option<String>,

    #[arg(long, env = "CRONY_OBJECT_STORE_SECRET_KEY")]
    object_store_secret_key: Option<String>,

    #[arg(long, env = "CRONY_OBJECT_STORE_ALLOW_HTTP", default_value_t = false)]
    object_store_allow_http: bool,

    #[arg(long, env = "CRONY_ARTIFACT_SIGNING_KEY_HEX")]
    artifact_signing_key_hex: Option<String>,

    #[arg(long, env = "CRONY_ARTIFACT_MAX_BYTES", default_value_t = 16_777_216)]
    artifact_max_bytes: usize,

    #[arg(long, env = "CRONY_ARTIFACT_RETENTION_DAYS", default_value_t = 30)]
    artifact_retention_days: i64,

    #[arg(
        long,
        env = "CRONY_ARTIFACT_RECOVERY_GRACE_SECS",
        default_value_t = 300,
        value_parser = clap::value_parser!(i64).range(0..=3_600)
    )]
    artifact_recovery_grace_secs: i64,

    #[arg(
        long,
        env = "CRONY_ARTIFACT_RECOVERY_INTERVAL_SECS",
        default_value_t = 60,
        value_parser = clap::value_parser!(u64).range(1..=3_600)
    )]
    artifact_recovery_interval_secs: u64,
}

#[derive(Clone)]
struct AppState {
    store: PgStore,
    event_tx: broadcast::Sender<DomainEvent>,
    runners: Arc<DashMap<String, RunnerConnection>>,
    strategies: StrategyRegistry,
    runner_grace_secs: i64,
    runner_credential_ttl_secs: i64,
    publication_publisher_credential_ttl_secs: i64,
    auth: AuthService,
    secret_cipher: SecretCipher,
    artifacts: ArtifactStore,
    artifact_retention_days: i64,
}

#[derive(Clone)]
struct RunnerConnection {
    corp_id: Uuid,
    connection_epoch: Uuid,
    tx: mpsc::UnboundedSender<ServerToRunner>,
    capabilities: Vec<RunnerCapability>,
}

struct AuthenticatedPublicationPublisher {
    publisher_id: String,
    credential_hash: String,
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

    fn not_found(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
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
    } else if error.to_string().starts_with("conflict:") {
        ApiError::conflict(error)
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
    let persisted_runner_recovery = if args.runner_startup_recovery {
        store.runner_records_requiring_recovery().await?
    } else {
        Vec::new()
    };
    let auth = AuthService::initialize(
        args.mode,
        args.oidc_issuer.clone(),
        args.allow_insecure_oidc,
    )
    .await?;
    let secret_cipher = SecretCipher::initialize(args.mode, args.secret_master_key_hex.as_deref())?;
    let artifacts = ArtifactStore::initialize(
        &args.object_store_backend,
        args.object_store_local_root.clone(),
        args.object_store_endpoint.as_deref(),
        args.object_store_bucket.as_deref(),
        Some(&args.object_store_region),
        args.object_store_access_key.as_deref(),
        args.object_store_secret_key.as_deref(),
        args.object_store_allow_http,
        args.artifact_signing_key_hex.as_deref(),
        args.artifact_max_bytes,
        args.mode == ServerMode::Production,
    )?;
    let artifact_recovery_grace = ChronoDuration::seconds(args.artifact_recovery_grace_secs);
    let (event_tx, _) = broadcast::channel(2_048);
    for event in recover_pending_artifacts(&store, &artifacts, artifact_recovery_grace).await? {
        let _ = event_tx.send(event);
    }
    let recovery_store = store.clone();
    let recovery_artifacts = artifacts.clone();
    let recovery_event_tx = event_tx.clone();
    let artifact_recovery_interval = StdDuration::from_secs(args.artifact_recovery_interval_secs);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(artifact_recovery_interval);
        interval.tick().await;
        loop {
            interval.tick().await;
            match recover_pending_artifacts(
                &recovery_store,
                &recovery_artifacts,
                artifact_recovery_grace,
            )
            .await
            {
                Ok(events) => {
                    for event in events {
                        let _ = recovery_event_tx.send(event);
                    }
                }
                Err(error) => warn!(%error, "periodic artifact recovery pass failed"),
            }
        }
    });
    let state = AppState {
        store,
        event_tx,
        runners: Arc::new(DashMap::new()),
        strategies: StrategyRegistry::new(),
        runner_grace_secs: args.runner_grace_secs.max(1),
        runner_credential_ttl_secs: args.runner_credential_ttl_secs.clamp(300, 604_800),
        publication_publisher_credential_ttl_secs: args
            .publication_publisher_credential_ttl_secs
            .clamp(300, 604_800),
        auth,
        secret_cipher,
        artifacts,
        artifact_retention_days: args.artifact_retention_days.clamp(1, 3_650),
    };
    for runner in persisted_runner_recovery {
        let grace_deadline = if runner.status == "connected" {
            let events = state
                .store
                .runner_disconnected(&runner.id, runner.connection_epoch, state.runner_grace_secs)
                .await?;
            for event in events {
                publish(&state, event);
            }
            Utc::now() + ChronoDuration::seconds(state.runner_grace_secs)
        } else {
            runner.grace_expires_at.unwrap_or_else(Utc::now)
        };
        let delay_millis = (grace_deadline - Utc::now()).num_milliseconds().max(0) as u64;
        let recovery_state = state.clone();
        let recovery_runner_id = runner.id.clone();
        let recovery_epoch = runner.connection_epoch;
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(delay_millis)).await;
            match recovery_state
                .store
                .expire_runner_grace(&recovery_runner_id, recovery_epoch)
                .await
            {
                Ok(events) => {
                    for event in events {
                        publish(&recovery_state, event);
                    }
                }
                Err(error) => warn!(
                    %error,
                    runner_id = %recovery_runner_id,
                    "startup runner recovery failed"
                ),
            }
        });
    }

    let protected = Router::new()
        .route("/api/corps/{corp_id}/snapshot", get(snapshot))
        .route(
            "/api/corps/{corp_id}/artifacts/{artifact_id}",
            get(download_artifact),
        )
        .route("/api/corps/{corp_id}/missions", post(create_mission))
        .route(
            "/api/corps/{corp_id}/factory/work-items/lookup",
            post(lookup_factory_work_items),
        )
        .route(
            "/api/corps/{corp_id}/factory/controllers",
            post(configure_factory_controller),
        )
        .route(
            "/api/corps/{corp_id}/factory/controllers/{controller_id}/heartbeat",
            post(heartbeat_factory_controller),
        )
        .route(
            "/api/corps/{corp_id}/factory/controllers/{controller_id}/control",
            post(control_factory_controller),
        )
        .route(
            "/api/corps/{corp_id}/factory/preflight",
            post(preflight_factory_mission),
        )
        .route(
            "/api/corps/{corp_id}/factory/work-items/claim",
            post(claim_factory_work_item),
        )
        .route(
            "/api/corps/{corp_id}/factory/work-items/{work_item_id}/renew",
            post(renew_factory_work_item),
        )
        .route(
            "/api/corps/{corp_id}/factory/work-items/{work_item_id}/upgrade-source-commit",
            post(upgrade_factory_source_commit),
        )
        .route(
            "/api/corps/{corp_id}/factory/work-items/{work_item_id}/transition",
            post(transition_factory_work_item),
        )
        .route(
            "/api/corps/{corp_id}/factory/work-items/{work_item_id}/materialize",
            post(materialize_factory_mission),
        )
        .route(
            "/api/corps/{corp_id}/factory/work-items/{work_item_id}/publication",
            get(get_pull_request_publication).post(start_pull_request_publication),
        )
        .route(
            "/api/corps/{corp_id}/factory/work-items/{work_item_id}/publication-context",
            get(get_factory_publication_context),
        )
        .route(
            "/api/corps/{corp_id}/factory/publication-publishers/credentials",
            post(create_publication_publisher_credential),
        )
        .route(
            "/api/corps/{corp_id}/factory/publication-publishers/credentials/{credential_id}/revoke",
            post(revoke_publication_publisher_credential),
        )
        .route(
            "/api/corps/{corp_id}/factory/publications/{publication_id}/renew",
            post(renew_pull_request_publication),
        )
        .route(
            "/api/corps/{corp_id}/factory/publications/{publication_id}/checkpoint",
            post(record_pull_request_publication_checkpoint),
        )
        .route(
            "/api/corps/{corp_id}/rooms/{room_id}/messages",
            post(create_room_message),
        )
        .route(
            "/api/corps/{corp_id}/missions/{mission_id}/launch",
            post(launch_mission),
        )
        .route(
            "/api/corps/{corp_id}/missions/{mission_id}/budget-revisions",
            post(propose_mission_budget_revision),
        )
        .route(
            "/api/corps/{corp_id}/missions/{mission_id}/contract-revisions",
            post(create_mission_contract_revision),
        )
        .route(
            "/api/corps/{corp_id}/missions/{mission_id}/budget-revisions/{revision_id}/decision",
            post(decide_mission_budget_revision),
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
                HeaderName::from_static("x-crony-publication-publisher-credential"),
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
    info!(address = %args.bind, "ECorp server listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let runners = match state.store.connected_runner_count().await {
        Ok(count) => usize::try_from(count).unwrap_or_default(),
        Err(error) => {
            warn!(%error, "health check could not read authoritative runner count");
            state.runners.len()
        }
    };
    Json(HealthResponse {
        status: "ok",
        service: "crony-server",
        runners,
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
    for event in outcome.run_events {
        publish(&state, event);
    }
    if outcome.revoked
        && let Some((_, connection)) = state.runners.remove(&runner_id)
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

async fn propose_mission_budget_revision(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, mission_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<ProposeMissionBudgetRevisionRequest>,
) -> Result<Json<MissionBudgetRevisionResponse>, ApiError> {
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
        .propose_mission_budget_revision(ProposeMissionBudgetRevisionInput {
            corp_id,
            mission_id,
            actor_id,
            expected_budget_tokens: request.expected_budget_tokens,
            expected_budget_cost_microusd: request.expected_budget_cost_microusd,
            proposed_budget_tokens: request.proposed_budget_tokens,
            proposed_budget_cost_microusd: request.proposed_budget_cost_microusd,
            rationale: request.rationale,
            idempotency_key: request.idempotency_key,
            finish_scope: request.finish_scope.map(|scope| MissionFinishScopeInput {
                task_id: scope.task_id,
                objective: scope.objective,
                expected_output: scope.expected_output,
                acceptance_tests: scope.acceptance_tests,
                write_scope: scope.write_scope,
                budget_tokens: scope.budget_tokens,
                budget_cost_microusd: scope.budget_cost_microusd,
                verification_policy: scope.verification_policy,
            }),
        })
        .await
        .map_err(map_store_error)?;
    if let Some(event) = outcome.event {
        publish(&state, event);
    }
    Ok(Json(MissionBudgetRevisionResponse {
        revision: outcome.revision,
        replayed: outcome.replayed,
    }))
}

async fn create_mission_contract_revision(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, mission_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<CreateMissionContractRevisionRequest>,
) -> Result<Json<MissionContractRevisionResponse>, ApiError> {
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
        .create_mission_contract_revision(CreateMissionContractRevisionInput {
            corp_id,
            mission_id,
            task_id: request.task_id,
            actor_id,
            expected_contract_version: request.expected_contract_version,
            next_action: request.next_action,
            source_run_id: request.source_run_id,
            reason: request.reason,
            idempotency_key: request.idempotency_key,
            description: request.description,
            contract: request.contract,
            verification_policy: request.verification_policy,
        })
        .await
        .map_err(map_store_error)?;
    if let Some(event) = outcome.event {
        publish(&state, event);
    }
    Ok(Json(MissionContractRevisionResponse {
        revision: outcome.revision,
        replayed: outcome.replayed,
    }))
}

async fn decide_mission_budget_revision(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, mission_id, revision_id)): Path<(Uuid, Uuid, Uuid)>,
    Json(request): Json<DecideMissionBudgetRevisionRequest>,
) -> Result<Json<MissionBudgetRevisionResponse>, ApiError> {
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
        .decide_mission_budget_revision(DecideMissionBudgetRevisionInput {
            corp_id,
            mission_id,
            revision_id,
            actor_id,
            expected_version: request.expected_version,
            approved: request.approved,
            note: request.note,
            decision_key: request.decision_key,
        })
        .await
        .map_err(map_store_error)?;
    if let Some(event) = outcome.event {
        publish(&state, event);
    }
    Ok(Json(MissionBudgetRevisionResponse {
        revision: outcome.revision,
        replayed: outcome.replayed,
    }))
}

fn valid_scope_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
}

async fn dispatch_pending_runner_commands(state: &AppState, runner_id: &str) -> anyhow::Result<()> {
    let Some((sender, durable_control)) = state.runners.get(runner_id).map(|connection| {
        (
            connection.tx.clone(),
            connection
                .capabilities
                .iter()
                .any(|capability| capability.name == "durable-control-v1" && capability.available),
        )
    }) else {
        return Ok(());
    };
    loop {
        let commands = state.store.pending_runner_commands(runner_id).await?;
        let batch_len = commands.len();
        if batch_len == 0 {
            break;
        }
        let mut dispatched = false;
        for command in commands {
            let control_lease_token = if command.command_kind == "control_message" {
                match state.store.control_command_lease_token(&command).await? {
                    Some(token) => Some(token),
                    None => {
                        if let Some(event) = state
                            .store
                            .fail_runner_command(
                                command.id,
                                runner_id,
                                "control lease changed or expired before durable steering dispatch",
                            )
                            .await?
                        {
                            publish(state, event);
                        }
                        continue;
                    }
                }
            } else {
                None
            };
            let outgoing = decode_runner_command(&command, control_lease_token, durable_control)?;
            if sender.send(outgoing).is_err() {
                return Ok(());
            }
            if command.command_kind == "control_message"
                && !durable_control
                && let Some(event) = state
                    .store
                    .acknowledge_runner_command(command.id, runner_id)
                    .await?
            {
                publish(state, event);
            }
            dispatched = true;
        }
        if dispatched || batch_len < 100 {
            break;
        }
    }
    Ok(())
}

fn decode_runner_command(
    command: &PendingRunnerCommand,
    control_lease_token: Option<Uuid>,
    durable_control: bool,
) -> anyhow::Result<ServerToRunner> {
    match command.command_kind.as_str() {
        "control_message" => Ok(ServerToRunner::ControlMessage {
            command_id: durable_control.then_some(command.id),
            message_id: if durable_control {
                Some(
                    command
                        .payload
                        .get("message_id")
                        .and_then(serde_json::Value::as_str)
                        .context("control message command omitted message_id")
                        .and_then(|value| {
                            Uuid::parse_str(value).context("message id is invalid")
                        })?,
                )
            } else {
                None
            },
            corp_id: command.corp_id,
            run_id: command.run_id,
            agent_id: command
                .payload
                .get("agent_id")
                .and_then(serde_json::Value::as_str)
                .context("control message command omitted agent_id")
                .and_then(|value| Uuid::parse_str(value).context("agent id is invalid"))?,
            actor_id: command
                .payload
                .get("actor_id")
                .and_then(serde_json::Value::as_str)
                .context("control message command omitted actor_id")
                .and_then(|value| Uuid::parse_str(value).context("actor id is invalid"))?,
            lease_token: control_lease_token
                .context("control message command omitted current lease token")?,
            text: command
                .payload
                .get("text")
                .and_then(serde_json::Value::as_str)
                .context("control message command omitted text")?
                .to_owned(),
        }),
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

async fn download_artifact(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, artifact_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<ArtifactDownloadQuery>,
) -> Result<Response, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(query.actor_id),
        Permission::Read,
    )
    .await?;
    let artifact = state
        .store
        .artifact_for_download(corp_id, artifact_id, actor_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("artifact was not found"))?;
    let bytes = state
        .artifacts
        .read_verified(&artifact)
        .await
        .map_err(ApiError::internal)?;
    let mut response = Response::new(Body::from(bytes.clone()));
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(&artifact.media_type).map_err(ApiError::internal)?,
    );
    headers.insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&bytes.len().to_string()).map_err(ApiError::internal)?,
    );
    headers.insert(
        CONTENT_DISPOSITION,
        artifact_content_disposition(&artifact.file_name).map_err(ApiError::internal)?,
    );
    headers.insert(
        ETAG,
        HeaderValue::from_str(&format!("\"{}\"", artifact.sha256)).map_err(ApiError::internal)?,
    );
    headers.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        HeaderName::from_static("x-crony-artifact-signature"),
        HeaderValue::from_str(&artifact.provenance_signature).map_err(ApiError::internal)?,
    );
    headers.insert(
        HeaderName::from_static("x-crony-artifact-role"),
        HeaderValue::from_str(&artifact.artifact_role).map_err(ApiError::internal)?,
    );
    Ok(response)
}

fn artifact_content_disposition(file_name: &str) -> anyhow::Result<HeaderValue> {
    let fallback = file_name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(file_name.len());
    for &byte in file_name.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    HeaderValue::from_str(&format!(
        "attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}"
    ))
    .context("build artifact Content-Disposition header")
}

#[derive(Debug, Deserialize)]
struct SnapshotQuery {
    actor_id: Uuid,
}

#[derive(Debug, Deserialize)]
struct ArtifactDownloadQuery {
    actor_id: Uuid,
}

struct MissionPlanInput<'a> {
    title: &'a str,
    description: &'a str,
    preferred_adapter: Option<&'a str>,
    preferred_model: Option<&'a str>,
    reasoning_effort: Option<&'a str>,
    strategy: Option<&'a str>,
    source: Option<&'a MissionSource>,
    secret_refs: &'a [TaskSecretReference],
    budget_tokens: Option<i64>,
    budget_cost_microusd: Option<i64>,
    deliverable: Option<&'a crony_domain::DeliverableSpec>,
    contract: Option<&'a FactoryMissionContract>,
    verification_policy: Option<&'a VerificationPolicy>,
    require_factory_manual_gate: bool,
}

async fn plan_mission(
    state: &AppState,
    corp_id: Uuid,
    input: MissionPlanInput<'_>,
) -> Result<TaskGraphPlan, ApiError> {
    let strategy = input.strategy.unwrap_or("single");
    if !input.secret_refs.is_empty() && strategy != "single" {
        return Err(ApiError::bad_request(
            "secret references currently require the single-task strategy",
        ));
    }
    let (preferred_adapter, preferred_model, reasoning_effort) =
        if uses_deterministic_harness(strategy) {
            (Some("fake-process"), None, None)
        } else {
            (
                input.preferred_adapter,
                input.preferred_model,
                input.reasoning_effort,
            )
        };
    let source = input
        .source
        .map(|source| resolve_mission_source(state, corp_id, source))
        .transpose()?;
    validate_requested_model(
        state,
        corp_id,
        preferred_adapter,
        preferred_model,
        reasoning_effort,
    )?;
    let agents = state
        .store
        .agents_for_planning(corp_id)
        .await
        .map_err(ApiError::internal)?;
    let mut plan = state
        .strategies
        .plan(
            strategy,
            &PlanningRequest {
                mission_title: input.title,
                preferred_adapter,
                preferred_model,
                reasoning_effort,
                secret_refs: input.secret_refs,
                budget_tokens: input.budget_tokens,
                budget_cost_microusd: input.budget_cost_microusd,
                deliverable: input.deliverable,
            },
            &agents,
        )
        .map_err(ApiError::bad_request)?;
    if let Some(contract) = input.contract {
        apply_mission_contract(&mut plan, contract)?;
    }
    if let Some(source) = &source {
        apply_mission_source(&mut plan, source);
    }
    apply_mission_description(&mut plan, input.description)?;
    if let Some(policy) = input.verification_policy {
        apply_verification_policy(&mut plan, policy);
    }
    if input.require_factory_manual_gate {
        enforce_factory_manual_gate(&mut plan);
    }
    validate_plan(&plan, &agents).map_err(ApiError::bad_request)?;
    if source.is_some() {
        validate_plan_runner_compatibility(state, corp_id, &plan)?;
    }
    Ok(plan)
}

fn resolve_mission_source(
    state: &AppState,
    corp_id: Uuid,
    requested: &MissionSource,
) -> Result<MissionSource, ApiError> {
    if requested.repository.trim().is_empty()
        || requested.base_ref.trim().is_empty()
        || requested.base_commit.trim().is_empty()
    {
        return Err(ApiError::bad_request(
            "mission source requires repository, base ref, and immutable base commit",
        ));
    }
    state
        .runners
        .iter()
        .filter(|entry| entry.corp_id == corp_id)
        .flat_map(|entry| entry.capabilities.clone())
        .find(|capability| {
            capability.name == "workspace-isolation"
                && capability.available
                && capability
                    .source_repository
                    .as_deref()
                    .is_some_and(|repository| {
                        repository.eq_ignore_ascii_case(requested.repository.trim())
                    })
                && capability.source_base_ref.as_deref() == Some(requested.base_ref.trim())
                && capability
                    .source_base_commit
                    .as_deref()
                    .is_some_and(|commit| commit.eq_ignore_ascii_case(requested.base_commit.trim()))
        })
        .and_then(|capability| {
            Some(MissionSource {
                repository: capability.source_repository?,
                base_ref: capability.source_base_ref?,
                base_commit: capability.source_base_commit?,
            })
        })
        .ok_or_else(|| {
            ApiError::bad_request(format!(
                "selected repository {} @ {} ({}) is not available from a connected runner",
                requested.repository.trim(),
                requested.base_ref.trim(),
                requested.base_commit.trim()
            ))
        })
}

fn apply_mission_source(plan: &mut TaskGraphPlan, source: &MissionSource) {
    for task in &mut plan.tasks {
        task.contract.source_repository = Some(source.repository.clone());
        task.contract.source_base_ref = Some(source.base_ref.clone());
        task.contract.source_base_commit = Some(source.base_commit.clone());
    }
}

fn validate_plan_runner_compatibility(
    state: &AppState,
    corp_id: Uuid,
    plan: &TaskGraphPlan,
) -> Result<(), ApiError> {
    for task in &plan.tasks {
        let requirements = RunnerRequirements {
            adapter: &task.required_adapter,
            model: task.contract.model.as_deref(),
            reasoning_effort: task.contract.reasoning_effort.as_deref(),
            source_repository: task.contract.source_repository.as_deref(),
            source_base_ref: task.contract.source_base_ref.as_deref(),
            source_base_commit: task.contract.source_base_commit.as_deref(),
        };
        if select_runner(state, corp_id, &requirements).is_none() {
            return Err(ApiError::bad_request(format!(
                "task {} {}",
                task.key,
                runner_requirement_mismatch(
                    &runner_capabilities(state, corp_id),
                    &task.required_adapter,
                    task.contract.model.as_deref(),
                    task.contract.reasoning_effort.as_deref(),
                    task.contract.source_repository.as_deref(),
                    task.contract.source_base_ref.as_deref(),
                    task.contract.source_base_commit.as_deref(),
                )
            )));
        }
    }
    Ok(())
}

fn apply_mission_contract(
    plan: &mut TaskGraphPlan,
    contract: &FactoryMissionContract,
) -> Result<(), ApiError> {
    if contract.objective.len() > 100_000 || contract.expected_output.len() > 10_000 {
        return Err(ApiError::bad_request(
            "mission objective or expected output is too large",
        ));
    }
    for (name, values) in [
        ("acceptance tests", &contract.acceptance_tests),
        ("allowed tools", &contract.allowed_tools),
        ("prohibited actions", &contract.prohibited_actions),
        ("references", &contract.references),
        ("write scope", &contract.write_scope),
    ] {
        if values.len() > 64 {
            return Err(ApiError::bad_request(format!(
                "mission {name} cannot contain more than 64 entries"
            )));
        }
    }

    let multi_task = plan.tasks.len() > 1;
    for task in &mut plan.tasks {
        if !contract.objective.trim().is_empty() {
            task.contract.objective = if multi_task {
                format!(
                    "{}\n\nROLE-SPECIFIC OBJECTIVE:\n{}",
                    contract.objective.trim(),
                    task.contract.objective
                )
            } else {
                contract.objective.trim().to_owned()
            };
        }
        if !contract.expected_output.trim().is_empty() {
            task.contract.expected_output = contract.expected_output.trim().to_owned();
        }
        append_unique(
            &mut task.contract.acceptance_tests,
            &contract.acceptance_tests,
        );
        if !contract.allowed_tools.is_empty() {
            task.contract.allowed_tools = contract.allowed_tools.clone();
        }
        append_unique(
            &mut task.contract.prohibited_actions,
            &contract.prohibited_actions,
        );
        append_unique(&mut task.contract.references, &contract.references);
        if !contract.write_scope.is_empty() {
            task.contract.write_scope = contract.write_scope.clone();
        }
    }
    Ok(())
}

fn apply_mission_description(plan: &mut TaskGraphPlan, description: &str) -> Result<(), ApiError> {
    let description = description.replace("\r\n", "\n").replace('\r', "\n");
    let description = description.trim();
    if description.len() > 100_000 {
        return Err(ApiError::bad_request(
            "mission description cannot exceed 100000 bytes",
        ));
    }
    if description
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
    {
        return Err(ApiError::bad_request(
            "mission description cannot contain unsupported control characters",
        ));
    }
    if description.is_empty() {
        return Ok(());
    }
    const SEPARATOR: &str = "\n\nTASK-SPECIFIC OBJECTIVE:\n";
    for task in &mut plan.tasks {
        let original = task.contract.objective.trim();
        task.contract.objective = if description.len() + SEPARATOR.len() + original.len() <= 100_000
        {
            format!("{description}{SEPARATOR}{original}")
        } else {
            description.to_owned()
        };
    }
    Ok(())
}

fn apply_verification_policy(plan: &mut TaskGraphPlan, policy: &VerificationPolicy) {
    let delivery_depth = plan.tasks.iter().map(|task| task.depth).max().unwrap_or(0);
    for task in &mut plan.tasks {
        if task.depth == delivery_depth {
            task.verification_policy = policy.clone();
        }
    }
}

fn enforce_factory_manual_gate(plan: &mut TaskGraphPlan) {
    for task in &mut plan.tasks {
        if task.required_adapter != "fake-process" && task.verification_policy.manual_gate.is_none()
        {
            task.verification_policy.manual_gate =
                Some(ManualVerificationGate::IndependentReview {
                    roles: vec!["member".to_owned(), "owner".to_owned(), "admin".to_owned()],
                    exclude_requester: true,
                });
        }
    }
}

fn append_unique(target: &mut Vec<String>, values: &[String]) {
    for value in values {
        if !target.contains(value) {
            target.push(value.clone());
        }
    }
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
    let plan = plan_mission(
        &state,
        corp_id,
        MissionPlanInput {
            title: &request.title,
            description: &request.description,
            preferred_adapter: request.preferred_adapter.as_deref(),
            preferred_model: request.preferred_model.as_deref(),
            reasoning_effort: request.reasoning_effort.as_deref(),
            strategy: request.strategy.as_deref(),
            source: request.source.as_ref(),
            secret_refs: &request.secret_refs,
            budget_tokens: request.budget_tokens,
            budget_cost_microusd: request.budget_cost_microusd,
            deliverable: request.deliverable.as_ref(),
            contract: request.contract.as_ref(),
            verification_policy: request.verification_policy.as_ref(),
            require_factory_manual_gate: false,
        },
    )
    .await?;
    let (ids, events) = state
        .store
        .create_mission(
            corp_id,
            requested_by,
            &request.title,
            &request.description,
            &plan,
        )
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

async fn claim_factory_work_item(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(corp_id): Path<Uuid>,
    Json(request): Json<ClaimFactoryWorkItemRequest>,
) -> Result<Json<FactoryWorkItemResponse>, ApiError> {
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
        .claim_factory_work_item(ClaimFactoryWorkItemInput {
            corp_id,
            actor_id,
            source: FactorySourceInput {
                project_owner: request.source_project_owner,
                project_number: request.source_project_number,
                project_item_id: request.source_project_item_id,
                repository_owner: request.source_repository_owner,
                repository_name: request.source_repository_name,
                issue_number: request.source_issue_number,
                issue_node_id: request.source_issue_node_id,
                issue_url: request.source_issue_url,
                title: request.source_title,
                revision: request.source_revision,
            },
            idempotency_key: request.idempotency_key,
            lease_seconds: request.lease_seconds,
            policy: request.policy,
        })
        .await
        .map_err(map_store_error)?;
    if let Some(event) = outcome.event {
        publish(&state, event);
    }
    Ok(Json(FactoryWorkItemResponse {
        work_item: outcome.work_item,
        claim_token: outcome.claim_token,
        replayed: outcome.replayed,
    }))
}

async fn lookup_factory_work_items(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(corp_id): Path<Uuid>,
    Json(request): Json<LookupFactoryWorkItemsRequest>,
) -> Result<Json<LookupFactoryWorkItemsResponse>, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.actor_id),
        Permission::Operate,
    )
    .await?;
    let items = state
        .store
        .factory_work_items_by_project_item_ids(
            corp_id,
            actor_id,
            &request.source_project_owner,
            request.source_project_number,
            &request.source_project_item_ids,
        )
        .await
        .map_err(map_store_error)?;
    let total_count = items.len();
    Ok(Json(LookupFactoryWorkItemsResponse { items, total_count }))
}

async fn configure_factory_controller(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(corp_id): Path<Uuid>,
    Json(request): Json<ConfigureFactoryControllerRequest>,
) -> Result<Json<FactoryControllerResponse>, ApiError> {
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
        .configure_factory_controller(ConfigureFactoryControllerInput {
            corp_id,
            actor_id,
            controller_id: request.controller_id,
            source_project_owner: request.source_project_owner,
            source_project_number: request.source_project_number,
            source_repository_owner: request.source_repository_owner,
            source_repository_name: request.source_repository_name,
            connection_epoch: request.connection_epoch,
            lease_seconds: request.lease_seconds,
            idempotency_key: request.idempotency_key,
        })
        .await
        .map_err(map_store_error)?;
    if let Some(event) = outcome.event {
        publish(&state, event);
    }
    Ok(Json(FactoryControllerResponse {
        controller: outcome.controller,
        replayed: outcome.replayed,
    }))
}

async fn heartbeat_factory_controller(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, controller_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<FactoryControllerHeartbeatRequest>,
) -> Result<Json<FactoryControllerResponse>, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.actor_id),
        Permission::ControlFactory,
    )
    .await?;
    let outcome = state
        .store
        .heartbeat_factory_controller(HeartbeatFactoryControllerInput {
            corp_id,
            actor_id,
            controller_id,
            connection_epoch: request.connection_epoch,
            lease_seconds: request.lease_seconds,
            active_work_item_id: request.active_work_item_id,
            completed_reconcile_generation: request.completed_reconcile_generation,
            reconcile_result: request.reconcile_result,
            error: request.error,
        })
        .await
        .map_err(map_store_error)?;
    if let Some(event) = outcome.event {
        publish(&state, event);
    }
    Ok(Json(FactoryControllerResponse {
        controller: outcome.controller,
        replayed: false,
    }))
}

async fn control_factory_controller(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, controller_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<ControlFactoryControllerRequest>,
) -> Result<Json<FactoryControllerResponse>, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.actor_id),
        Permission::ControlFactory,
    )
    .await?;
    let outcome = state
        .store
        .control_factory_controller(ControlFactoryControllerInput {
            corp_id,
            actor_id,
            controller_id,
            expected_version: request.expected_version,
            action: request.action.as_str().to_owned(),
            idempotency_key: request.idempotency_key,
        })
        .await
        .map_err(map_store_error)?;
    if let Some(event) = outcome.event {
        publish(&state, event);
    }
    Ok(Json(FactoryControllerResponse {
        controller: outcome.controller,
        replayed: outcome.replayed,
    }))
}

async fn renew_factory_work_item(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, work_item_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<RenewFactoryWorkItemRequest>,
) -> Result<Json<FactoryWorkItemResponse>, ApiError> {
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
        .renew_factory_work_item(RenewFactoryWorkItemInput {
            corp_id,
            work_item_id,
            actor_id,
            claim_token: request.claim_token,
            expected_version: request.expected_version,
            idempotency_key: request.idempotency_key,
            lease_seconds: request.lease_seconds,
        })
        .await
        .map_err(map_store_error)?;
    if let Some(event) = outcome.event {
        publish(&state, event);
    }
    Ok(Json(FactoryWorkItemResponse {
        work_item: outcome.work_item,
        claim_token: outcome.claim_token,
        replayed: outcome.replayed,
    }))
}

async fn upgrade_factory_source_commit(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, work_item_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<UpgradeFactorySourceCommitRequest>,
) -> Result<Json<FactoryWorkItemResponse>, ApiError> {
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
        .upgrade_factory_source_commit(UpgradeFactorySourceCommitInput {
            corp_id,
            work_item_id,
            actor_id,
            claim_token: request.claim_token,
            expected_version: request.expected_version,
            idempotency_key: request.idempotency_key,
            source_base_commit: request.source_base_commit,
        })
        .await
        .map_err(map_store_error)?;
    if let Some(event) = outcome.event {
        publish(&state, event);
    }
    Ok(Json(FactoryWorkItemResponse {
        work_item: outcome.work_item,
        claim_token: outcome.claim_token,
        replayed: outcome.replayed,
    }))
}

async fn transition_factory_work_item(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, work_item_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<TransitionFactoryWorkItemRequest>,
) -> Result<Json<FactoryWorkItemResponse>, ApiError> {
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
        .transition_factory_work_item(TransitionFactoryWorkItemInput {
            corp_id,
            work_item_id,
            actor_id,
            claim_token: request.claim_token,
            expected_version: request.expected_version,
            idempotency_key: request.idempotency_key,
            state: request.state,
            failure_detail: request.failure_detail,
        })
        .await
        .map_err(map_store_error)?;
    if let Some(event) = outcome.event {
        publish(&state, event);
    }
    Ok(Json(FactoryWorkItemResponse {
        work_item: outcome.work_item,
        claim_token: outcome.claim_token,
        replayed: outcome.replayed,
    }))
}

#[allow(clippy::too_many_arguments)]
fn factory_materialization_operation_request(
    title: &str,
    description: &str,
    preferred_adapter: Option<&str>,
    preferred_model: Option<&str>,
    reasoning_effort: Option<&str>,
    strategy: Option<&str>,
    secret_refs: &[TaskSecretReference],
    budget_tokens: Option<i64>,
    budget_cost_microusd: Option<i64>,
    deliverable: Option<&crony_domain::DeliverableSpec>,
    contract: &FactoryMissionContract,
    verification_policy: Option<&VerificationPolicy>,
) -> serde_json::Value {
    json!({
        "title": title,
        "description": description,
        "preferred_adapter": preferred_adapter,
        "preferred_model": preferred_model,
        "reasoning_effort": reasoning_effort,
        "strategy": strategy,
        "secret_refs": secret_refs,
        "budget_tokens": budget_tokens,
        "budget_cost_microusd": budget_cost_microusd,
        "deliverable": deliverable,
        "contract": contract,
        "verification_policy": verification_policy
    })
}

fn factory_materialization_failure_detail(rejection: &str) -> String {
    let sanitized = rejection
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let sanitized = if sanitized.is_empty() {
        "unspecified pre-mission validation error"
    } else {
        sanitized.as_str()
    };
    let mut detail =
        format!("factory materialization rejected before mission creation: {sanitized}");
    if detail.len() > 2_000 {
        let mut end = 2_000;
        while !detail.is_char_boundary(end) {
            end -= 1;
        }
        detail.truncate(end);
    }
    detail
}

async fn preflight_factory_mission(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(corp_id): Path<Uuid>,
    Json(request): Json<PreflightFactoryMissionRequest>,
) -> Result<Json<PreflightFactoryMissionResponse>, ApiError> {
    authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.actor_id),
        Permission::Operate,
    )
    .await?;
    let plan = plan_mission(
        &state,
        corp_id,
        MissionPlanInput {
            title: &request.title,
            description: &request.description,
            preferred_adapter: request.preferred_adapter.as_deref(),
            preferred_model: request.preferred_model.as_deref(),
            reasoning_effort: request.reasoning_effort.as_deref(),
            strategy: request.strategy.as_deref(),
            source: None,
            secret_refs: &request.secret_refs,
            budget_tokens: request.budget_tokens,
            budget_cost_microusd: request.budget_cost_microusd,
            deliverable: request.deliverable.as_ref(),
            contract: Some(&request.contract),
            verification_policy: request.verification_policy.as_ref(),
            require_factory_manual_gate: true,
        },
    )
    .await?;
    let operation_request = factory_materialization_operation_request(
        &request.title,
        &request.description,
        request.preferred_adapter.as_deref(),
        request.preferred_model.as_deref(),
        request.reasoning_effort.as_deref(),
        request.strategy.as_deref(),
        &request.secret_refs,
        request.budget_tokens,
        request.budget_cost_microusd,
        request.deliverable.as_ref(),
        &request.contract,
        request.verification_policy.as_ref(),
    );
    let constrained_plan = state
        .store
        .preflight_factory_mission(
            PreflightFactoryMissionInput {
                corp_id,
                actor_id: request.actor_id,
                source_repository_owner: request.source_repository_owner,
                source_repository_name: request.source_repository_name,
                policy: request.policy,
                title: request.title,
                description: request.description,
                request: operation_request,
            },
            &plan,
        )
        .await
        .map_err(map_store_error)?;
    Ok(Json(PreflightFactoryMissionResponse {
        valid: true,
        strategy: constrained_plan.strategy,
        task_count: constrained_plan.tasks.len(),
        budget_tokens: constrained_plan.budget_tokens,
        budget_cost_microusd: constrained_plan.budget_cost_microusd,
    }))
}

async fn materialize_factory_mission(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, work_item_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<MaterializeFactoryMissionRequest>,
) -> Result<Json<MaterializeFactoryMissionResponse>, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.actor_id),
        Permission::Operate,
    )
    .await?;
    let operation_request = factory_materialization_operation_request(
        &request.title,
        &request.description,
        request.preferred_adapter.as_deref(),
        request.preferred_model.as_deref(),
        request.reasoning_effort.as_deref(),
        request.strategy.as_deref(),
        &request.secret_refs,
        request.budget_tokens,
        request.budget_cost_microusd,
        request.deliverable.as_ref(),
        &request.contract,
        request.verification_policy.as_ref(),
    );
    let materialize_input = MaterializeFactoryMissionInput {
        corp_id,
        work_item_id,
        actor_id,
        claim_token: request.claim_token,
        expected_version: request.expected_version,
        idempotency_key: request.idempotency_key.clone(),
        title: request.title.clone(),
        description: request.description.clone(),
        request: operation_request,
    };
    let replay = match state
        .store
        .replay_factory_materialization(&materialize_input)
        .await
    {
        Ok(replay) => replay,
        Err(error) => {
            let error = map_store_error(error);
            return Err(
                reject_factory_materialization_error(&state, &materialize_input, error).await,
            );
        }
    };
    if let Some(outcome) = replay {
        return Ok(Json(MaterializeFactoryMissionResponse {
            mission_id: outcome.ids.mission_id,
            task_id: outcome.ids.task_ids[0],
            task_ids: outcome.ids.task_ids,
            strategy: outcome.strategy,
            work_item: outcome.work_item,
            replayed: true,
        }));
    }
    let plan = match plan_mission(
        &state,
        corp_id,
        MissionPlanInput {
            title: &request.title,
            description: &request.description,
            preferred_adapter: request.preferred_adapter.as_deref(),
            preferred_model: request.preferred_model.as_deref(),
            reasoning_effort: request.reasoning_effort.as_deref(),
            strategy: request.strategy.as_deref(),
            source: None,
            secret_refs: &request.secret_refs,
            budget_tokens: request.budget_tokens,
            budget_cost_microusd: request.budget_cost_microusd,
            deliverable: request.deliverable.as_ref(),
            contract: Some(&request.contract),
            verification_policy: request.verification_policy.as_ref(),
            require_factory_manual_gate: true,
        },
    )
    .await
    {
        Ok(plan) => plan,
        Err(error) => {
            return Err(
                reject_factory_materialization_error(&state, &materialize_input, error).await,
            );
        }
    };
    let outcome = match state
        .store
        .materialize_factory_mission(materialize_input.clone(), &plan)
        .await
    {
        Ok(outcome) => outcome,
        Err(error) => {
            let error = map_store_error(error);
            return Err(
                reject_factory_materialization_error(&state, &materialize_input, error).await,
            );
        }
    };
    for event in outcome.events {
        publish(&state, event);
    }
    Ok(Json(MaterializeFactoryMissionResponse {
        mission_id: outcome.ids.mission_id,
        task_id: outcome.ids.task_ids[0],
        task_ids: outcome.ids.task_ids,
        strategy: outcome.strategy,
        work_item: outcome.work_item,
        replayed: outcome.replayed,
    }))
}

async fn reject_factory_materialization_error(
    state: &AppState,
    input: &MaterializeFactoryMissionInput,
    error: ApiError,
) -> ApiError {
    if error.status == StatusCode::CONFLICT {
        return error;
    }
    match persist_factory_materialization_rejection(state, input, &error.message).await {
        Ok(()) => error,
        Err(compensation) if compensation.status == StatusCode::CONFLICT => {
            warn!(
                work_item_id = %input.work_item_id,
                original_error = %error.message,
                compensation_error = %compensation.message,
                "factory materialization rejection raced with a newer claim or mission"
            );
            error
        }
        Err(compensation) => ApiError::internal(format!(
            "{}; additionally failed to persist materialization rejection: {}",
            error.message, compensation.message
        )),
    }
}

async fn persist_factory_materialization_rejection(
    state: &AppState,
    input: &MaterializeFactoryMissionInput,
    rejection: &str,
) -> Result<(), ApiError> {
    let request_digest = hex::encode(Sha256::digest(input.idempotency_key.as_bytes()));
    let claim_digest = hex::encode(Sha256::digest(input.claim_token.as_bytes()));
    let failure_detail = factory_materialization_failure_detail(rejection);
    let outcome = state
        .store
        .reject_factory_materialization(RejectFactoryMaterializationInput {
            corp_id: input.corp_id,
            work_item_id: input.work_item_id,
            actor_id: input.actor_id,
            claim_token: input.claim_token,
            attempted_version: input.expected_version,
            idempotency_key: format!(
                "materialize-rejected:{}:{}:{}",
                input.expected_version,
                &request_digest[..16],
                &claim_digest[..16]
            ),
            failure_detail,
        })
        .await
        .map_err(map_store_error)?;
    if let Some(event) = outcome.event {
        publish(state, event);
    }
    Ok(())
}

async fn get_pull_request_publication(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, work_item_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<SnapshotQuery>,
) -> Result<Json<PullRequestPublicationResponse>, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(query.actor_id),
        Permission::Operate,
    )
    .await?;
    let publication = state
        .store
        .pull_request_publication_for_work_item(corp_id, actor_id, work_item_id)
        .await
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found("pull-request publication was not found"))?;
    let busy = publication.state != crony_domain::PullRequestPublicationState::Published
        && publication
            .publisher_lease_expires_at
            .is_some_and(|expiry| expiry > Utc::now());
    Ok(Json(PullRequestPublicationResponse {
        publication,
        publisher_token: None,
        replayed: false,
        busy,
    }))
}

async fn get_factory_publication_context(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, work_item_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<SnapshotQuery>,
) -> Result<Json<FactoryPublicationContextResponse>, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(query.actor_id),
        Permission::Operate,
    )
    .await?;
    let context = state
        .store
        .factory_publication_context(corp_id, actor_id, work_item_id)
        .await
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found("factory publication context was not found"))?;
    Ok(Json(FactoryPublicationContextResponse {
        work_item: context.work_item,
        publication: context.publication,
        source_deliverables: context.source_deliverables,
    }))
}

async fn create_publication_publisher_credential(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(corp_id): Path<Uuid>,
    Json(request): Json<CreatePublicationPublisherCredentialRequest>,
) -> Result<Json<CreatePublicationPublisherCredentialResponse>, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.actor_id),
        Permission::Manage,
    )
    .await?;
    let ttl = request
        .expires_in_seconds
        .clamp(300, state.publication_publisher_credential_ttl_secs);
    let expires_at = Utc::now() + ChronoDuration::seconds(ttl);
    let credential = new_runner_token("publisher");
    let outcome = state
        .store
        .create_publication_publisher_credential(
            corp_id,
            actor_id,
            &request.publisher_id,
            &hash_secret(&credential),
            expires_at,
        )
        .await
        .map_err(map_store_error)?;
    publish(&state, outcome.event);
    Ok(Json(CreatePublicationPublisherCredentialResponse {
        credential_id: outcome.credential_id,
        publisher_id: outcome.publisher_id,
        credential,
        expires_at: outcome.expires_at.to_rfc3339(),
    }))
}

async fn revoke_publication_publisher_credential(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, credential_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<RevokePublicationPublisherCredentialRequest>,
) -> Result<Json<RevokePublicationPublisherCredentialResponse>, ApiError> {
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
        .revoke_publication_publisher_credential(corp_id, actor_id, credential_id, &request.reason)
        .await
        .map_err(map_store_error)?;
    if let Some(event) = outcome.event {
        publish(&state, event);
    }
    Ok(Json(RevokePublicationPublisherCredentialResponse {
        credential_id: outcome.credential_id,
        publisher_id: outcome.publisher_id,
        revoked: outcome.revoked,
    }))
}

async fn authenticate_publication_publisher(
    state: &AppState,
    headers: &HeaderMap,
    corp_id: Uuid,
) -> Result<AuthenticatedPublicationPublisher, ApiError> {
    let credential = headers
        .get(HeaderName::from_static(
            "x-crony-publication-publisher-credential",
        ))
        .ok_or_else(|| ApiError::forbidden("trusted publication publisher credential is required"))?
        .to_str()
        .map_err(|_| ApiError::forbidden("trusted publication publisher credential was rejected"))?
        .trim();
    if credential.is_empty() || credential.len() > 256 || credential.chars().any(char::is_control) {
        return Err(ApiError::forbidden(
            "trusted publication publisher credential was rejected",
        ));
    }
    let credential_hash = hash_secret(credential);
    let publisher_id = state
        .store
        .authenticate_publication_publisher(corp_id, &credential_hash)
        .await
        .map_err(|_| {
            ApiError::forbidden("trusted publication publisher credential was rejected")
        })?;
    Ok(AuthenticatedPublicationPublisher {
        publisher_id,
        credential_hash,
    })
}

async fn start_pull_request_publication(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, work_item_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    Json(request): Json<StartPullRequestPublicationRequest>,
) -> Result<Json<PullRequestPublicationResponse>, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.actor_id),
        Permission::Publish,
    )
    .await?;
    let publisher = authenticate_publication_publisher(&state, &headers, corp_id).await?;
    if request.publisher_id.trim() != publisher.publisher_id {
        return Err(ApiError::forbidden(
            "trusted publication publisher identity does not match the request",
        ));
    }
    let actor_role = state
        .store
        .human_authorization(corp_id, actor_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::forbidden("publication actor is not a human Corp member"))?
        .role;
    let outcome = state
        .store
        .start_pull_request_publication(StartPullRequestPublicationInput {
            corp_id,
            work_item_id,
            actor_id,
            actor_role,
            source_deliverable_id: request.source_deliverable_id,
            target_repository: request.target_repository,
            base_ref: request.base_ref,
            branch: request.branch,
            title: request.title,
            body: request.body,
            authorization_id: request.authorization_id,
            authorization_reason: request.authorization_reason,
            effect_key: request.effect_key,
            idempotency_key: request.idempotency_key,
            publisher_id: publisher.publisher_id,
            publisher_credential_hash: publisher.credential_hash,
            lease_seconds: request.lease_seconds,
        })
        .await
        .map_err(map_store_error)?;
    publish_publication_events(&state, &outcome);
    Ok(Json(publication_response(outcome)))
}

async fn renew_pull_request_publication(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, publication_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    Json(request): Json<RenewPullRequestPublicationRequest>,
) -> Result<Json<PullRequestPublicationResponse>, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.actor_id),
        Permission::Publish,
    )
    .await?;
    let publisher = authenticate_publication_publisher(&state, &headers, corp_id).await?;
    let outcome = state
        .store
        .renew_pull_request_publication(RenewPullRequestPublicationInput {
            corp_id,
            publication_id,
            actor_id,
            publisher_id: publisher.publisher_id,
            publisher_credential_hash: publisher.credential_hash,
            publisher_token: request.publisher_token,
            expected_version: request.expected_version,
            idempotency_key: request.idempotency_key,
            lease_seconds: request.lease_seconds,
        })
        .await
        .map_err(map_store_error)?;
    publish_publication_events(&state, &outcome);
    Ok(Json(publication_response(outcome)))
}

async fn record_pull_request_publication_checkpoint(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((corp_id, publication_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    Json(request): Json<RecordPullRequestPublicationCheckpointRequest>,
) -> Result<Json<PullRequestPublicationResponse>, ApiError> {
    let actor_id = authorize_actor(
        &state,
        &principal,
        corp_id,
        Some(request.actor_id),
        Permission::Publish,
    )
    .await?;
    let publisher = authenticate_publication_publisher(&state, &headers, corp_id).await?;
    let checkpoint = match request.checkpoint {
        PullRequestPublicationCheckpoint::BranchPushed { commit_sha } => {
            PullRequestPublicationCheckpointInput::BranchPushed { commit_sha }
        }
        PullRequestPublicationCheckpoint::PullRequestCreated {
            number,
            node_id,
            url,
            state,
            draft,
            title,
            body,
            head_ref,
            base_ref,
            head_sha,
            head_repository_owner,
            is_cross_repository,
            auto_merge_enabled,
        } => PullRequestPublicationCheckpointInput::PullRequestCreated {
            number,
            node_id,
            url,
            state,
            draft,
            title,
            body,
            head_ref,
            base_ref,
            head_sha,
            head_repository_owner,
            is_cross_repository,
            auto_merge_enabled,
        },
        PullRequestPublicationCheckpoint::Published {
            project_status,
            project_field_id,
            project_option_id,
        } => PullRequestPublicationCheckpointInput::Published {
            project_status,
            project_field_id,
            project_option_id,
        },
        PullRequestPublicationCheckpoint::Failed { failure_detail } => {
            PullRequestPublicationCheckpointInput::Failed { failure_detail }
        }
    };
    let outcome = state
        .store
        .record_pull_request_publication_checkpoint(RecordPullRequestPublicationCheckpointInput {
            corp_id,
            publication_id,
            actor_id,
            publisher_id: publisher.publisher_id,
            publisher_credential_hash: publisher.credential_hash,
            publisher_token: request.publisher_token,
            expected_version: request.expected_version,
            idempotency_key: request.idempotency_key,
            checkpoint,
        })
        .await
        .map_err(map_store_error)?;
    publish_publication_events(&state, &outcome);
    Ok(Json(publication_response(outcome)))
}

fn publish_publication_events(state: &AppState, outcome: &PullRequestPublicationOutcome) {
    for event in outcome.events.iter().cloned() {
        publish(state, event);
    }
}

fn publication_response(outcome: PullRequestPublicationOutcome) -> PullRequestPublicationResponse {
    PullRequestPublicationResponse {
        publication: outcome.publication,
        publisher_token: outcome.publisher_token,
        replayed: outcome.replayed,
        busy: outcome.busy,
    }
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
            idempotency_key: request.idempotency_key.unwrap_or_else(Uuid::new_v4),
        })
        .await
        .map_err(map_store_error)?;
    if let Some(event) = outcome.event {
        publish(&state, event);
    }
    Ok(Json(CreateRoomMessageResponse {
        message_id: outcome.message.id,
        replayed: outcome.replayed,
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
    let outcome = schedule_ready_tasks(&state, corp_id, mission_id, Some(requested_by))
        .await
        .map_err(ApiError::conflict)?;
    if let Some((first, runner_id)) = outcome.records.first() {
        return Ok(Json(LaunchMissionResponse {
            run_id: first.run_id,
            runner_id: runner_id.clone(),
            run_ids: outcome
                .records
                .iter()
                .map(|(record, _)| record.run_id)
                .collect(),
            runner_ids: outcome
                .records
                .iter()
                .map(|(_, runner_id)| runner_id.clone())
                .collect(),
            replayed: false,
        }));
    }
    let initial_runs = state
        .store
        .mission_launch_runs(corp_id, mission_id, requested_by)
        .await
        .map_err(ApiError::conflict)?;
    let first = initial_runs
        .first()
        .ok_or_else(|| ApiError::conflict(outcome.failure_message()))?;

    Ok(Json(LaunchMissionResponse {
        run_id: first.0,
        runner_id: first.1.clone(),
        run_ids: initial_runs.iter().map(|(run_id, _)| *run_id).collect(),
        runner_ids: initial_runs
            .iter()
            .map(|(_, runner_id)| runner_id.clone())
            .collect(),
        replayed: true,
    }))
}

#[derive(Default)]
struct ScheduleOutcome {
    records: Vec<(LaunchRecord, String)>,
    candidate_count: usize,
    failures: Vec<String>,
}

impl ScheduleOutcome {
    fn failure_message(&self) -> String {
        if self.candidate_count == 0 {
            return "mission has no schedulable tasks; tasks may be waiting on dependencies, assigned to a busy agent, already active, or finished"
                .to_owned();
        }
        if self.failures.is_empty() {
            return "mission had ready tasks, but none could be dispatched".to_owned();
        }
        format!(
            "mission had ready tasks, but none could be dispatched: {}",
            self.failures.join("; ")
        )
    }
}

async fn schedule_ready_tasks(
    state: &AppState,
    corp_id: Uuid,
    mission_id: Uuid,
    requested_by: Option<Uuid>,
) -> anyhow::Result<ScheduleOutcome> {
    let candidates = state
        .store
        .schedulable_tasks(corp_id, mission_id, requested_by.is_some())
        .await?;
    let mut outcome = ScheduleOutcome {
        candidate_count: candidates.len(),
        ..ScheduleOutcome::default()
    };
    for candidate in candidates {
        let requirements = RunnerRequirements {
            adapter: &candidate.required_adapter,
            model: candidate.required_model.as_deref(),
            reasoning_effort: candidate.required_reasoning_effort.as_deref(),
            source_repository: candidate.required_source_repository.as_deref(),
            source_base_ref: candidate.required_source_base_ref.as_deref(),
            source_base_commit: candidate.required_source_base_commit.as_deref(),
        };
        let Some((runner_id, runner_tx)) = select_runner(state, corp_id, &requirements) else {
            outcome.failures.push(format!(
                "task {} {}",
                candidate.task_id,
                runner_requirement_mismatch(
                    &runner_capabilities(state, corp_id),
                    &candidate.required_adapter,
                    candidate.required_model.as_deref(),
                    candidate.required_reasoning_effort.as_deref(),
                    candidate.required_source_repository.as_deref(),
                    candidate.required_source_base_ref.as_deref(),
                    candidate.required_source_base_commit.as_deref(),
                )
            ));
            continue;
        };
        let (mut record, event) = match state
            .store
            .create_task_run(
                corp_id,
                mission_id,
                candidate.task_id,
                requested_by,
                &runner_id,
            )
            .await
        {
            Ok(result) => result,
            Err(error) => {
                outcome.failures.push(format!(
                    "task {} could not create a run: {error}",
                    candidate.task_id
                ));
                continue;
            }
        };
        let dependency_context = match resolve_dependency_context(state, &record).await {
            Ok(context) => context,
            Err(error) => {
                if let Ok(event) = state
                    .store
                    .fail_run_before_dispatch(
                        corp_id,
                        record.run_id,
                        &format!("dependency context could not be verified: {error}"),
                    )
                    .await
                {
                    publish(state, event);
                }
                outcome.failures.push(format!(
                    "task {} dependency context failed: {error}",
                    candidate.task_id
                ));
                continue;
            }
        };
        if !dependency_context.is_empty() {
            record.mission_title.push_str(&dependency_context);
        }
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
                outcome.failures.push(format!(
                    "task {} secret assignment failed: {error}",
                    candidate.task_id
                ));
                continue;
            }
        };
        if runner_tx
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
                source_repository: record.source_repository.clone(),
                source_base_ref: record.source_base_ref.clone(),
                source_base_commit: record.source_base_commit.clone(),
                verification_policy: record.verification_policy.clone(),
                write_scope: record.write_scope.clone(),
                deliverable: record.deliverable.clone(),
                secrets,
            })
            .is_err()
        {
            let reason = "runner disconnected before accepting the run";
            if let Ok(event) = state
                .store
                .fail_run_before_dispatch(corp_id, record.run_id, reason)
                .await
            {
                publish(state, event);
            }
            outcome
                .failures
                .push(format!("task {} {reason}", candidate.task_id));
            continue;
        }
        publish(state, event);
        outcome.records.push((record, runner_id));
    }
    Ok(outcome)
}

async fn resolve_dependency_context(
    state: &AppState,
    record: &LaunchRecord,
) -> anyhow::Result<String> {
    const MAX_CONTEXT_BYTES: usize = 64 * 1024;
    let dependencies = state
        .store
        .dependency_artifacts(record.corp_id, record.task_id)
        .await?;
    if dependencies.is_empty() {
        return Ok(String::new());
    }
    let mut context = String::from(
        "\n\nVERIFIED DEPENDENCY OUTPUTS:\n\
         Use these completed specialist artifacts as source material. Reconcile their tradeoffs \
         and do not claim synthesis without addressing each one.\n",
    );
    for dependency in dependencies {
        let bytes = state.artifacts.read_verified(&dependency.artifact).await?;
        let header = format!(
            "\n--- {} / {} / task {} / run {} / sha256 {} ---\n",
            dependency.plan_key,
            dependency.task_title,
            dependency.task_id,
            dependency.artifact.run_id,
            dependency.artifact.sha256,
        );
        if context.len().saturating_add(header.len()) >= MAX_CONTEXT_BYTES {
            break;
        }
        context.push_str(&header);
        if let Some(summary) = dependency.run_summary {
            context.push_str("Completion summary: ");
            context.push_str(&summary);
            context.push('\n');
        }
        if dependency.artifact.media_type.starts_with("text/")
            || dependency.artifact.media_type == "application/json"
        {
            let text = String::from_utf8(bytes.to_vec())
                .context("dependency artifact text is not valid UTF-8")?;
            let remaining = MAX_CONTEXT_BYTES.saturating_sub(context.len());
            if remaining == 0 {
                break;
            }
            if text.len() <= remaining {
                context.push_str(&text);
            } else {
                let mut boundary = remaining;
                while boundary > 0 && !text.is_char_boundary(boundary) {
                    boundary -= 1;
                }
                context.push_str(&text[..boundary]);
                context
                    .push_str("\n[dependency artifact truncated at the bounded context limit]\n");
                break;
            }
        } else {
            context.push_str(&format!(
                "Binary artifact: {} bytes, media type {}\n",
                dependency.artifact.bytes, dependency.artifact.media_type
            ));
        }
    }
    Ok(context)
}

fn append_operator_notes(prompt: &mut String, messages: &[QueuedRunMessage]) {
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
        let _ = schedule_ready_tasks(state, corp_id, mission_id, None).await?;
    }
    Ok(())
}

fn runner_capabilities(state: &AppState, corp_id: Uuid) -> Vec<RunnerCapability> {
    state
        .runners
        .iter()
        .filter(|entry| entry.corp_id == corp_id)
        .flat_map(|entry| entry.capabilities.clone())
        .collect()
}

fn capability_satisfies_requirement(
    capability: &RunnerCapability,
    required_adapter: &str,
    required_model: Option<&str>,
    required_reasoning_effort: Option<&str>,
) -> bool {
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
}

fn runner_requirement_mismatch(
    capabilities: &[RunnerCapability],
    required_adapter: &str,
    required_model: Option<&str>,
    required_reasoning_effort: Option<&str>,
    required_source_repository: Option<&str>,
    required_source_base_ref: Option<&str>,
    required_source_base_commit: Option<&str>,
) -> String {
    if required_source_repository.is_some()
        && !runner_workspace_satisfies_requirement(
            capabilities,
            required_source_repository,
            required_source_base_ref,
            required_source_base_commit,
        )
    {
        return format!(
            "requires source repository {} at {} ({}) but no connected runner advertises that immutable checkout",
            required_source_repository.unwrap_or("unknown"),
            required_source_base_ref.unwrap_or("unknown"),
            required_source_base_commit.unwrap_or("unknown")
        );
    }
    let adapter_capabilities = capabilities
        .iter()
        .filter(|capability| capability.name == required_adapter)
        .collect::<Vec<_>>();
    if adapter_capabilities.is_empty() {
        return format!(
            "requires adapter {required_adapter}, which no connected runner advertises"
        );
    }
    let available = adapter_capabilities
        .into_iter()
        .filter(|capability| capability.available)
        .collect::<Vec<_>>();
    if available.is_empty() {
        return format!("requires adapter {required_adapter}, but that adapter is unavailable");
    }
    let Some(model_id) = required_model else {
        return format!(
            "requires adapter {required_adapter}, but no matching runner was selectable"
        );
    };
    let matching_models = available
        .iter()
        .flat_map(|capability| capability.models.iter())
        .filter(|model| model.id == model_id && model.policy_state.as_deref() != Some("disabled"))
        .collect::<Vec<_>>();
    if matching_models.is_empty() {
        let advertised_models = available
            .iter()
            .flat_map(|capability| capability.models.iter())
            .filter(|model| model.policy_state.as_deref() != Some("disabled"))
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>();
        if advertised_models.is_empty() {
            return format!(
                "requires model {model_id} on {required_adapter}, but connected runners expose no selectable models for that adapter"
            );
        }
        return format!(
            "requires model {model_id} on {required_adapter}; available models are {}",
            advertised_models.join(", ")
        );
    }
    if let Some(effort) = required_reasoning_effort
        && !matching_models.iter().any(|model| {
            model.supports_reasoning_effort
                && model
                    .supported_reasoning_efforts
                    .iter()
                    .any(|supported| supported == effort)
        })
    {
        return format!(
            "requires reasoning effort {effort} for model {model_id} on {required_adapter}, but connected runners do not support it"
        );
    }
    format!("requires {required_adapter}/{model_id}, but no matching runner was selectable")
}

struct RunnerRequirements<'a> {
    adapter: &'a str,
    model: Option<&'a str>,
    reasoning_effort: Option<&'a str>,
    source_repository: Option<&'a str>,
    source_base_ref: Option<&'a str>,
    source_base_commit: Option<&'a str>,
}

fn select_runner(
    state: &AppState,
    corp_id: Uuid,
    requirements: &RunnerRequirements<'_>,
) -> Option<(String, mpsc::UnboundedSender<ServerToRunner>)> {
    let mut runners = state
        .runners
        .iter()
        .filter(|entry| {
            entry.corp_id == corp_id
                && runner_workspace_satisfies_requirement(
                    &entry.capabilities,
                    requirements.source_repository,
                    requirements.source_base_ref,
                    requirements.source_base_commit,
                )
                && entry.capabilities.iter().any(|capability| {
                    capability_satisfies_requirement(
                        capability,
                        requirements.adapter,
                        requirements.model,
                        requirements.reasoning_effort,
                    )
                })
        })
        .map(|entry| (entry.key().clone(), entry.tx.clone()))
        .collect::<Vec<_>>();
    runners.sort_by(|left, right| left.0.cmp(&right.0));
    runners.into_iter().next()
}

fn runner_workspace_satisfies_requirement(
    capabilities: &[RunnerCapability],
    required_repository: Option<&str>,
    required_base_ref: Option<&str>,
    required_base_commit: Option<&str>,
) -> bool {
    let Some(required_repository) = required_repository else {
        return required_base_ref.is_none() && required_base_commit.is_none();
    };
    let (Some(required_base_ref), Some(required_base_commit)) =
        (required_base_ref, required_base_commit)
    else {
        return false;
    };
    capabilities.iter().any(|capability| {
        capability.name == "workspace-isolation"
            && capability.available
            && capability
                .source_repository
                .as_deref()
                .is_some_and(|repository| repository.eq_ignore_ascii_case(required_repository))
            && capability.source_base_ref.as_deref() == Some(required_base_ref)
            && capability
                .source_base_commit
                .as_deref()
                .is_some_and(|commit| commit.eq_ignore_ascii_case(required_base_commit))
    })
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
    let Some(runner) = state.runners.get(&record.runner_id) else {
        let failure = state
            .store
            .fail_run_before_dispatch(
                corp_id,
                record.run_id,
                "source run's runner is disconnected",
            )
            .await
            .map_err(ApiError::internal)?;
        publish(&state, failure);
        return Err(ApiError::conflict("source run's runner is disconnected"));
    };
    if runner.corp_id != corp_id {
        drop(runner);
        let failure = state
            .store
            .fail_run_before_dispatch(
                corp_id,
                record.run_id,
                "source run's runner is enrolled to a different Corp",
            )
            .await
            .map_err(ApiError::internal)?;
        publish(&state, failure);
        return Err(ApiError::forbidden(
            "source run's runner is enrolled to a different Corp",
        ));
    }
    if !runner_workspace_satisfies_requirement(
        &runner.capabilities,
        record.source_repository.as_deref(),
        record.source_base_ref.as_deref(),
        record.source_base_commit.as_deref(),
    ) {
        drop(runner);
        let failure = state
            .store
            .fail_run_before_dispatch(
                corp_id,
                record.run_id,
                "source run's runner no longer advertises the required repository checkout",
            )
            .await
            .map_err(ApiError::internal)?;
        publish(&state, failure);
        return Err(ApiError::conflict(
            "source run's runner no longer advertises the required repository checkout",
        ));
    }
    let runner_tx = runner.tx.clone();
    drop(runner);
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
        mission_title: record.task_prompt.clone(),
        model: record.model.clone(),
        reasoning_effort: record.reasoning_effort.clone(),
        source_repository: record.source_repository.clone(),
        source_base_ref: record.source_base_ref.clone(),
        source_base_commit: record.source_base_commit.clone(),
        verification_policy: record.verification_policy.clone(),
        write_scope: record.write_scope.clone(),
        deliverable: record.deliverable.clone(),
        secret_refs: record.secret_refs.clone(),
        queued_messages: record.queued_messages.clone(),
    };
    let mut resume_prompt = format!(
        "{}\n\nRESUME INSTRUCTION:\n{}",
        record.task_prompt,
        prompt.trim()
    );
    append_operator_notes(&mut resume_prompt, &record.queued_messages);
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
    if runner_tx
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
            prompt: resume_prompt,
            model: record.model,
            reasoning_effort: record.reasoning_effort,
            source_repository: record.source_repository,
            source_base_ref: record.source_base_ref,
            source_base_commit: record.source_base_commit,
            workspace_base_commit: Some(record.workspace_base_commit),
            verification_policy: record.verification_policy,
            write_scope: record.write_scope,
            deliverable: record.deliverable,
            secrets,
        })
        .is_err()
    {
        let failure = state
            .store
            .fail_run_before_dispatch(
                corp_id,
                record.run_id,
                "runner disconnected before accepting resume",
            )
            .await
            .map_err(ApiError::internal)?;
        publish(&state, failure);
        return Err(ApiError::conflict(
            "runner disconnected before accepting resume",
        ));
    }
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
            request.idempotency_key.unwrap_or_else(Uuid::new_v4),
        )
        .await
        .map_err(ApiError::conflict)?;
    let message_id = outcome.message.id;
    if let Some(event) = outcome.event {
        publish(&state, event);
    }
    if outcome.command_queued
        && let Some(runner_id) = outcome.runner_id.as_deref()
    {
        dispatch_pending_runner_commands(&state, runner_id)
            .await
            .map_err(ApiError::internal)?;
    }

    Ok(Json(QueueMessageResponse {
        message_id,
        delivery: outcome.delivery,
        replayed: outcome.replayed,
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
                        loop {
                            let replay = match state
                                .store
                                .events_after(corp_id, actor_id, cursor, 500)
                                .await
                            {
                                Ok(replay) => replay,
                                Err(error) => {
                                    warn!(
                                        %error,
                                        %corp_id,
                                        cursor,
                                        "browser lag recovery query failed"
                                    );
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
                    Ok(false) => {
                        warn!(%runner_id, "runner heartbeat epoch was rejected");
                        let is_current = state
                            .runners
                            .get(&runner_id)
                            .is_some_and(|entry| entry.connection_epoch == connection_epoch);
                        if is_current
                            && let Some((_, connection)) = state.runners.remove(&runner_id)
                        {
                            let _ = connection.tx.send(ServerToRunner::Disconnect {
                                reason: "runner heartbeat was rejected by authoritative state"
                                    .to_owned(),
                                reconnect_delay_ms: 2_000,
                            });
                        }
                    }
                    Err(error) => warn!(%error, %runner_id, "runner heartbeat persistence failed"),
                }
            }
            RunnerToServer::CommandAck {
                runner_id,
                connection_epoch,
                command_id,
                applied,
                detail,
            } => {
                let registered_current =
                    registered
                        .as_ref()
                        .is_some_and(|(registered_id, _, epoch)| {
                            registered_id == &runner_id && *epoch == connection_epoch
                        })
                        && state
                            .runners
                            .get(&runner_id)
                            .is_some_and(|entry| entry.connection_epoch == connection_epoch);
                if !registered_current {
                    warn!(%runner_id, %command_id, "command ack came from a stale runner socket");
                    continue;
                }
                if !applied {
                    warn!(%runner_id, %command_id, %detail, "runner could not apply durable command");
                    match state
                        .store
                        .fail_runner_command(command_id, &runner_id, &detail)
                        .await
                    {
                        Ok(Some(event)) => publish(&state, event),
                        Ok(None) => {}
                        Err(error) => {
                            warn!(%error, %runner_id, %command_id, "runner command failure persistence failed")
                        }
                    }
                    continue;
                }
                match state
                    .store
                    .acknowledge_runner_command(command_id, &runner_id)
                    .await
                {
                    Ok(Some(event)) => {
                        publish(&state, event);
                        info!(%runner_id, %command_id, %detail, "durable runner command acknowledged");
                    }
                    Ok(None) => {
                        tracing::debug!(%runner_id, %command_id, "runner command ack was duplicate")
                    }
                    Err(error) => {
                        warn!(%error, %runner_id, %command_id, "runner command ack persistence failed")
                    }
                }
            }
            RunnerToServer::RunEvent {
                event_id,
                runner_id,
                corp_id,
                connection_epoch,
                run_id,
                agent_id,
                assignment_token,
                event_type,
                payload,
            } => {
                let registered_epoch =
                    registered
                        .as_ref()
                        .and_then(|(registered_id, registered_corp, epoch)| {
                            (registered_id == &runner_id
                                && *registered_corp == corp_id
                                && *epoch == connection_epoch)
                                .then_some(*epoch)
                        });
                let current_epoch = state
                    .runners
                    .get(&runner_id)
                    .map(|entry| entry.connection_epoch);
                if registered_epoch.is_none() || registered_epoch != current_epoch {
                    warn!(
                        %runner_id,
                        ?registered_epoch,
                        ?current_epoch,
                        "run event came from an unregistered or superseded runner socket"
                    );
                    continue;
                }
                let deliverable_ack_sha = (event_type == "run.deliverable_upload")
                    .then(|| payload.get("sha256").and_then(serde_json::Value::as_str))
                    .flatten()
                    .map(str::to_owned);
                let input = RunnerEventInput {
                    event_id,
                    runner_id: runner_id.clone(),
                    corp_id,
                    connection_epoch,
                    run_id,
                    agent_id,
                    assignment_token,
                    event_type: event_type.clone(),
                    payload,
                };
                let mut applied_event_type = event_type.clone();
                let mut result = process_runner_event(&state, input).await;
                if matches!(
                    event_type.as_str(),
                    "run.artifact_upload" | "run.deliverable_upload"
                ) && let Err(error) = &result
                {
                    let reason = format!("artifact upload rejected: {error}");
                    warn!(%error, %run_id, "artifact upload failed verification");
                    applied_event_type = "run.failed".to_owned();
                    result = state
                        .store
                        .apply_runner_event(RunnerEventInput {
                            event_id,
                            runner_id: runner_id.clone(),
                            corp_id,
                            connection_epoch,
                            run_id,
                            agent_id,
                            assignment_token,
                            event_type: applied_event_type.clone(),
                            payload: json!({"error": reason}),
                        })
                        .await;
                }
                match result {
                    Ok(Some(event)) => {
                        if event.event_type == "run.deliverable"
                            && let (Some(artifact_id), Some(artifact_role), Some(sha256)) = (
                                event
                                    .payload
                                    .get("artifact_id")
                                    .and_then(serde_json::Value::as_str)
                                    .and_then(|value| Uuid::parse_str(value).ok()),
                                event
                                    .payload
                                    .get("artifact_role")
                                    .and_then(serde_json::Value::as_str),
                                event
                                    .payload
                                    .get("sha256")
                                    .and_then(serde_json::Value::as_str),
                            )
                        {
                            let _ = command_tx.send(ServerToRunner::ArtifactStored {
                                run_id,
                                artifact_id,
                                artifact_role: artifact_role.to_owned(),
                                sha256: sha256.to_owned(),
                            });
                        }
                        let approval_expiry = if applied_event_type == "run.approval_requested" {
                            event
                                .payload
                                .get("approval_id")
                                .and_then(serde_json::Value::as_str)
                                .and_then(|value| Uuid::parse_str(value).ok())
                                .map(|approval_id| {
                                    let delay = event
                                        .payload
                                        .get("expires_in_seconds")
                                        .and_then(serde_json::Value::as_u64)
                                        .unwrap_or(300)
                                        .clamp(1, 3_600);
                                    (approval_id, delay)
                                })
                        } else {
                            None
                        };
                        publish(&state, event);
                        if let Some((approval_id, delay)) = approval_expiry {
                            let expiry_state = state.clone();
                            let expiry_runner_id = runner_id.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                                match expiry_state.store.expire_action_approval(approval_id).await {
                                    Ok(Some(outcome)) => {
                                        if let Some(event) = outcome.event {
                                            publish(&expiry_state, event);
                                        }
                                        if let Err(error) = dispatch_pending_runner_commands(
                                            &expiry_state,
                                            &expiry_runner_id,
                                        )
                                        .await
                                        {
                                            warn!(
                                                %error,
                                                %approval_id,
                                                "expired approval command dispatch failed"
                                            );
                                        }
                                    }
                                    Ok(None) => {}
                                    Err(error) => warn!(
                                        %error,
                                        %approval_id,
                                        "action approval expiry failed"
                                    ),
                                }
                            });
                        }
                        if matches!(
                            applied_event_type.as_str(),
                            "run.usage" | "run.tool_activity"
                        ) {
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
                        if matches!(applied_event_type.as_str(), "run.completed" | "run.failed") {
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
                    Ok(None) => {
                        if let Some(sha256) = deliverable_ack_sha {
                            match state
                                .store
                                .ready_artifact_for_run_role_digest(
                                    corp_id,
                                    run_id,
                                    "source_deliverable",
                                    &sha256,
                                )
                                .await
                            {
                                Ok(Some(artifact)) => {
                                    let _ = command_tx.send(ServerToRunner::ArtifactStored {
                                        run_id,
                                        artifact_id: artifact.id,
                                        artifact_role: artifact.artifact_role,
                                        sha256: artifact.sha256,
                                    });
                                }
                                Ok(None) => {}
                                Err(error) => {
                                    warn!(%error, %run_id, "deliverable acknowledgment lookup failed")
                                }
                            }
                        }
                    }
                    Err(error) => {
                        if error
                            .to_string()
                            .contains("runner event does not match an active run")
                        {
                            info!(%run_id, event_type = %applied_event_type, "ignored stale runner event");
                        } else {
                            warn!(%error, %run_id, event_type = %applied_event_type, "failed to apply runner event")
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

async fn process_runner_event(
    state: &AppState,
    input: RunnerEventInput,
) -> anyhow::Result<Option<DomainEvent>> {
    if !matches!(
        input.event_type.as_str(),
        "run.artifact_upload" | "run.deliverable_upload"
    ) {
        return state.store.apply_runner_event(input).await;
    }
    let context = state
        .store
        .artifact_context(
            input.corp_id,
            input.run_id,
            input.agent_id,
            &input.runner_id,
            input.assignment_token,
        )
        .await?;
    let retention_until = Utc::now() + ChronoDuration::days(state.artifact_retention_days);
    let staged = state.artifacts.prepare_staging(
        ArtifactIdentity {
            id: input.event_id,
            corp_id: input.corp_id,
            task_id: context.task_id,
            run_id: input.run_id,
            agent_id: input.agent_id,
            runner_id: &input.runner_id,
        },
        &input.payload,
        retention_until,
    )?;
    let prepared = state
        .store
        .prepare_artifact_upload(input, staged.artifact.clone(), &staged.staging_key)
        .await?;
    match prepared.status.as_str() {
        "ready" => {
            cleanup_prepared_artifact(&state.store, &state.artifacts, &prepared).await;
            Ok(None)
        }
        "rejected" => {
            cleanup_prepared_artifact(&state.store, &state.artifacts, &prepared).await;
            Err(anyhow::anyhow!("artifact upload was previously rejected"))
        }
        "staged" => {
            let staging_key = prepared
                .staging_key
                .as_deref()
                .context("staged artifact omitted its staging key")?;
            let reserved = StagedArtifact {
                artifact: prepared.artifact.clone(),
                staging_key: staging_key.to_owned(),
                bytes: staged.bytes,
            };
            state.artifacts.write_staged(&reserved).await?;
            if let Err(error) = state
                .artifacts
                .finalize_staged(&prepared.artifact, staging_key)
                .await
            {
                if artifact_error_is_permanent(&error) {
                    let cleanup_key = state
                        .store
                        .reject_staged_artifact(
                            prepared.artifact.corp_id,
                            prepared.artifact.id,
                            &error.to_string(),
                        )
                        .await;
                    match cleanup_key {
                        Ok(Some(cleanup_key))
                            if state.artifacts.discard_staged(&cleanup_key).await.is_ok() =>
                        {
                            let _ = state
                                .store
                                .clear_artifact_staging_key(
                                    prepared.artifact.corp_id,
                                    prepared.artifact.id,
                                    &cleanup_key,
                                )
                                .await;
                        }
                        Ok(_) => {}
                        Err(reject_error) => {
                            return Err(error).context(format!(
                                "artifact rejection persistence also failed: {reject_error}"
                            ));
                        }
                    }
                }
                return Err(error);
            }
            let event = state
                .store
                .finalize_artifact_upload(prepared.artifact.corp_id, prepared.artifact.id)
                .await?;
            cleanup_prepared_artifact(&state.store, &state.artifacts, &prepared).await;
            Ok(event)
        }
        status => Err(anyhow::anyhow!("unknown prepared artifact status {status}")),
    }
}

async fn recover_pending_artifacts(
    store: &PgStore,
    artifacts: &ArtifactStore,
    recovery_grace: ChronoDuration,
) -> anyhow::Result<Vec<DomainEvent>> {
    let mut recovered_events = Vec::new();
    let pending = store.pending_artifact_uploads(500).await?;
    let known_staging_keys = pending
        .iter()
        .filter_map(|prepared| prepared.staging_key.clone())
        .collect::<HashSet<_>>();
    match artifacts.staged_keys().await {
        Ok(staging_keys) => {
            for staging_key in staging_keys {
                if !known_staging_keys.contains(&staging_key) {
                    match store.artifact_staging_key_is_reserved(&staging_key).await {
                        Ok(true) => continue,
                        Ok(false) => match artifacts.discard_staged(&staging_key).await {
                            Ok(()) => {
                                info!(%staging_key, "removed orphan artifact staging object")
                            }
                            Err(error) => warn!(
                                %error,
                                %staging_key,
                                "orphan artifact staging cleanup deferred"
                            ),
                        },
                        Err(error) => warn!(
                            %error,
                            %staging_key,
                            "orphan artifact reservation check deferred"
                        ),
                    }
                }
            }
        }
        Err(error) => warn!(%error, "artifact staging discovery deferred"),
    }

    for prepared in pending {
        if prepared.status == "staged" && prepared.created_at > Utc::now() - recovery_grace {
            tracing::debug!(
                artifact_id = %prepared.artifact.id,
                created_at = %prepared.created_at,
                "staged artifact remains inside the recovery grace window"
            );
            continue;
        }
        match prepared.status.as_str() {
            "staged" => {
                let staging_key = prepared
                    .staging_key
                    .as_deref()
                    .context("staged artifact omitted its staging key")?;
                if let Err(error) = artifacts
                    .finalize_staged(&prepared.artifact, staging_key)
                    .await
                {
                    if artifact_error_is_missing_objects(&error) {
                        warn!(
                            %error,
                            artifact_id = %prepared.artifact.id,
                            %staging_key,
                            "expired artifact reservation has no bytes; releasing it for retry"
                        );
                        match store
                            .abandon_staged_artifact(
                                prepared.artifact.corp_id,
                                prepared.artifact.id,
                            )
                            .await
                        {
                            Ok(Some(cleanup_key)) => {
                                if let Err(cleanup_error) =
                                    artifacts.discard_staged(&cleanup_key).await
                                {
                                    warn!(
                                        %cleanup_error,
                                        %cleanup_key,
                                        "released artifact staging cleanup deferred"
                                    );
                                }
                            }
                            Ok(None) => {}
                            Err(abandon_error) => warn!(
                                %abandon_error,
                                artifact_id = %prepared.artifact.id,
                                "artifact reservation release deferred"
                            ),
                        }
                    } else if artifact_error_is_permanent(&error) {
                        warn!(
                            %error,
                            artifact_id = %prepared.artifact.id,
                            %staging_key,
                            "staged artifact recovery failed permanently; rejecting metadata"
                        );
                        match store
                            .reject_staged_artifact(
                                prepared.artifact.corp_id,
                                prepared.artifact.id,
                                &error.to_string(),
                            )
                            .await
                        {
                            Ok(Some(cleanup_key))
                                if artifacts.discard_staged(&cleanup_key).await.is_ok() =>
                            {
                                let _ = store
                                    .clear_artifact_staging_key(
                                        prepared.artifact.corp_id,
                                        prepared.artifact.id,
                                        &cleanup_key,
                                    )
                                    .await;
                            }
                            Ok(_) => {}
                            Err(reject_error) => warn!(
                                %reject_error,
                                artifact_id = %prepared.artifact.id,
                                "artifact recovery rejection persistence deferred"
                            ),
                        }
                    } else {
                        warn!(
                            %error,
                            artifact_id = %prepared.artifact.id,
                            %staging_key,
                            "transient staged artifact recovery deferred"
                        );
                    }
                    continue;
                }
                match store
                    .finalize_artifact_upload(prepared.artifact.corp_id, prepared.artifact.id)
                    .await
                {
                    Ok(Some(event)) => recovered_events.push(event),
                    Ok(None) => {}
                    Err(error) => {
                        warn!(
                            %error,
                            artifact_id = %prepared.artifact.id,
                            "artifact metadata finalization recovery deferred"
                        );
                        continue;
                    }
                }
            }
            "ready" | "rejected" => {}
            status => {
                warn!(
                    artifact_id = %prepared.artifact.id,
                    %status,
                    "artifact recovery skipped unknown status"
                );
                continue;
            }
        }
        cleanup_prepared_artifact(store, artifacts, &prepared).await;
    }
    Ok(recovered_events)
}

async fn cleanup_prepared_artifact(
    store: &PgStore,
    artifacts: &ArtifactStore,
    prepared: &crony_store::PreparedArtifactUpload,
) {
    let Some(staging_key) = prepared.staging_key.as_deref() else {
        return;
    };
    match artifacts.discard_staged(staging_key).await {
        Ok(()) => {
            if let Err(error) = store
                .clear_artifact_staging_key(
                    prepared.artifact.corp_id,
                    prepared.artifact.id,
                    staging_key,
                )
                .await
            {
                warn!(
                    %error,
                    artifact_id = %prepared.artifact.id,
                    %staging_key,
                    "artifact staging metadata cleanup failed"
                );
            }
        }
        Err(error) => warn!(
            %error,
            artifact_id = %prepared.artifact.id,
            %staging_key,
            "artifact staging object cleanup deferred"
        ),
    }
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

#[cfg(test)]
mod tests {
    use crony_domain::{
        ManualVerificationGate, PlannedTask, TaskContract, TaskGraphPlan, VerificationPolicy,
        VerifierCheck,
    };
    use crony_protocol::{FactoryMissionContract, MissionSource, RunnerCapability, RunnerModel};
    use serde_json::json;
    use uuid::Uuid;

    use super::{
        apply_mission_contract, apply_mission_description, apply_mission_source,
        artifact_content_disposition, capability_satisfies_requirement,
        enforce_factory_manual_gate, factory_materialization_failure_detail,
        runner_requirement_mismatch,
    };

    fn model(id: &str, efforts: &[&str]) -> RunnerModel {
        RunnerModel {
            id: id.to_owned(),
            name: id.to_owned(),
            policy_state: Some("enabled".to_owned()),
            policy_terms: None,
            supports_vision: false,
            supports_reasoning_effort: !efforts.is_empty(),
            max_prompt_tokens: None,
            max_context_window_tokens: None,
            supported_reasoning_efforts: efforts
                .iter()
                .map(|effort| (*effort).to_owned())
                .collect(),
            default_reasoning_effort: None,
            billing_multiplier: None,
        }
    }

    #[test]
    fn model_less_adapter_reports_the_invalid_model_requirement() {
        let capabilities = vec![RunnerCapability {
            name: "fake-process".to_owned(),
            available: true,
            detail: None,
            models: Vec::new(),
            source_repository: None,
            source_base_ref: None,
            source_base_commit: None,
        }];
        assert_eq!(
            runner_requirement_mismatch(
                &capabilities,
                "fake-process",
                Some("gpt-5.6-sol"),
                Some("max"),
                None,
                None,
                None,
            ),
            "requires model gpt-5.6-sol on fake-process, but connected runners expose no selectable models for that adapter"
        );
    }

    #[test]
    fn runner_matching_enforces_model_and_reasoning_support() {
        let capability = RunnerCapability {
            name: "github-copilot".to_owned(),
            available: true,
            detail: None,
            models: vec![model("gpt-5.6-sol", &["high", "max"])],
            source_repository: None,
            source_base_ref: None,
            source_base_commit: None,
        };
        assert!(capability_satisfies_requirement(
            &capability,
            "github-copilot",
            Some("gpt-5.6-sol"),
            Some("max"),
        ));
        assert!(!capability_satisfies_requirement(
            &capability,
            "github-copilot",
            Some("gpt-5.6-sol"),
            Some("minimal"),
        ));
    }

    #[test]
    fn runner_matching_enforces_repository_and_base_ref() {
        let capabilities = vec![RunnerCapability {
            name: "workspace-isolation".to_owned(),
            available: true,
            detail: Some(
                "root=/tmp/runner; repository=/src/ecorp; remote=shyamsridhar123/ecorp; base=HEAD; commit=1111111111111111111111111111111111111111"
                    .to_owned(),
            ),
            models: Vec::new(),
            source_repository: Some("shyamsridhar123/ecorp".to_owned()),
            source_base_ref: Some("HEAD".to_owned()),
            source_base_commit: Some(
                "1111111111111111111111111111111111111111".to_owned(),
            ),
        }];
        assert!(super::runner_workspace_satisfies_requirement(
            &capabilities,
            Some("shyamsridhar123/ecorp"),
            Some("HEAD"),
            Some("1111111111111111111111111111111111111111"),
        ));
        assert!(!super::runner_workspace_satisfies_requirement(
            &capabilities,
            Some("acme/widget"),
            Some("HEAD"),
            Some("1111111111111111111111111111111111111111"),
        ));
        assert!(!super::runner_workspace_satisfies_requirement(
            &capabilities,
            Some("shyamsridhar123/ecorp"),
            Some("main"),
            Some("1111111111111111111111111111111111111111"),
        ));
        assert!(!super::runner_workspace_satisfies_requirement(
            &capabilities,
            Some("shyamsridhar123/ecorp"),
            Some("HEAD"),
            Some("2222222222222222222222222222222222222222"),
        ));
    }

    #[test]
    fn provider_backed_factory_tasks_require_independent_verification() {
        let mut plan: TaskGraphPlan = serde_json::from_value(json!({
            "strategy": "single",
            "max_nodes": 1,
            "max_depth": 0,
            "budget_tokens": 1_000,
            "budget_cost_microusd": 1_000_000,
            "tasks": [{
                "key": "deliver",
                "title": "Deliver",
                "contract": {
                    "objective": "Implement the issue",
                    "expected_output": "A verified change",
                    "source_repository": null,
                    "source_base_ref": null,
                    "source_base_commit": null,
                    "acceptance_tests": ["tests pass"],
                    "allowed_tools": ["filesystem", "shell"],
                    "prohibited_actions": ["do not merge"],
                    "references": [],
                    "write_scope": ["**"],
                    "budget_tokens": 1_000,
                    "budget_cost_microusd": 1_000_000,
                    "deadline_at": null,
                    "escalation": "ask the operator",
                    "secret_refs": [],
                    "model": null,
                    "reasoning_effort": null
                },
                "assigned_agent_id": Uuid::new_v4(),
                "required_adapter": "codex",
                "depends_on": [],
                "depth": 0,
                "max_attempts": 1,
                "verification_policy": {
                    "checks": [{"type": "artifact", "min_bytes": 1}],
                    "manual_gate": null
                }
            }]
        }))
        .expect("valid plan");
        apply_mission_contract(
            &mut plan,
            &FactoryMissionContract {
                objective: String::new(),
                expected_output: String::new(),
                acceptance_tests: Vec::new(),
                allowed_tools: Vec::new(),
                prohibited_actions: Vec::new(),
                references: Vec::new(),
                write_scope: Vec::new(),
            },
        )
        .expect("factory contract");
        enforce_factory_manual_gate(&mut plan);
        assert!(matches!(
            plan.tasks[0].verification_policy.manual_gate,
            Some(ManualVerificationGate::IndependentReview {
                exclude_requester: true,
                ..
            })
        ));
    }

    #[test]
    fn mission_description_is_normalized_and_delivered_to_each_task() {
        let mut plan = TaskGraphPlan {
            strategy: "single".to_owned(),
            max_nodes: 1,
            max_depth: 0,
            budget_tokens: 1_000,
            budget_cost_microusd: 1_000_000,
            tasks: vec![PlannedTask {
                key: "deliver".to_owned(),
                title: "Deliver".to_owned(),
                contract: TaskContract {
                    objective: "Produce the role-specific output.".to_owned(),
                    expected_output: "Output".to_owned(),
                    source_repository: None,
                    source_base_ref: None,
                    source_base_commit: None,
                    acceptance_tests: vec!["tests pass".to_owned()],
                    allowed_tools: vec!["filesystem".to_owned()],
                    prohibited_actions: vec!["do not escape".to_owned()],
                    references: Vec::new(),
                    write_scope: vec!["**".to_owned()],
                    budget_tokens: 1_000,
                    budget_cost_microusd: 1_000_000,
                    deadline_at: None,
                    escalation: "ask".to_owned(),
                    secret_refs: Vec::new(),
                    model: None,
                    reasoning_effort: None,
                    deliverable: None,
                },
                assigned_agent_id: Uuid::new_v4(),
                required_adapter: "fake-process".to_owned(),
                depends_on: Vec::new(),
                depth: 0,
                max_attempts: 1,
                verification_policy: VerificationPolicy {
                    checks: vec![VerifierCheck::Artifact { min_bytes: 1 }],
                    manual_gate: None,
                },
            }],
        };
        assert!(apply_mission_description(&mut plan, "line one\r\nline two").is_ok());
        assert_eq!(
            plan.tasks[0].contract.objective,
            "line one\nline two\n\nTASK-SPECIFIC OBJECTIVE:\nProduce the role-specific output."
        );
        assert!(apply_mission_description(&mut plan, "bad\u{0007}value").is_err());
    }

    #[test]
    fn mission_source_is_pinned_to_every_planned_task() {
        let mut plan: TaskGraphPlan = serde_json::from_value(json!({
            "strategy": "parallel-specialists",
            "max_nodes": 2,
            "max_depth": 0,
            "budget_tokens": 2_000,
            "budget_cost_microusd": 2_000_000,
            "tasks": [
                {
                    "key": "one",
                    "title": "One",
                    "contract": {
                        "objective": "First task",
                        "expected_output": "Output",
                        "source_repository": null,
                        "source_base_ref": null,
                        "source_base_commit": null,
                        "acceptance_tests": ["passes"],
                        "allowed_tools": ["filesystem"],
                        "prohibited_actions": ["escape"],
                        "references": [],
                        "write_scope": ["**"],
                        "budget_tokens": 1_000,
                        "budget_cost_microusd": 1_000_000,
                        "deadline_at": null,
                        "escalation": "ask",
                        "secret_refs": [],
                        "model": null,
                        "reasoning_effort": null,
                        "deliverable": null
                    },
                    "assigned_agent_id": Uuid::new_v4(),
                    "required_adapter": "codex",
                    "depends_on": [],
                    "depth": 0,
                    "max_attempts": 1,
                    "verification_policy": {
                        "checks": [{"type": "artifact", "min_bytes": 1}],
                        "manual_gate": null
                    }
                },
                {
                    "key": "two",
                    "title": "Two",
                    "contract": {
                        "objective": "Second task",
                        "expected_output": "Output",
                        "source_repository": null,
                        "source_base_ref": null,
                        "source_base_commit": null,
                        "acceptance_tests": ["passes"],
                        "allowed_tools": ["filesystem"],
                        "prohibited_actions": ["escape"],
                        "references": [],
                        "write_scope": ["**"],
                        "budget_tokens": 1_000,
                        "budget_cost_microusd": 1_000_000,
                        "deadline_at": null,
                        "escalation": "ask",
                        "secret_refs": [],
                        "model": null,
                        "reasoning_effort": null,
                        "deliverable": null
                    },
                    "assigned_agent_id": Uuid::new_v4(),
                    "required_adapter": "claude-code",
                    "depends_on": [],
                    "depth": 0,
                    "max_attempts": 1,
                    "verification_policy": {
                        "checks": [{"type": "artifact", "min_bytes": 1}],
                        "manual_gate": null
                    }
                }
            ]
        }))
        .expect("plan");
        let source = MissionSource {
            repository: "local/dogfood-1234".to_owned(),
            base_ref: "HEAD".to_owned(),
            base_commit: "1".repeat(40),
        };
        apply_mission_source(&mut plan, &source);
        assert!(plan.tasks.iter().all(|task| {
            task.contract.source_repository.as_deref() == Some(source.repository.as_str())
                && task.contract.source_base_ref.as_deref() == Some(source.base_ref.as_str())
                && task.contract.source_base_commit.as_deref() == Some(source.base_commit.as_str())
        }));
    }

    #[test]
    fn factory_materialization_failure_detail_is_bounded_single_line_text() {
        let detail =
            factory_materialization_failure_detail(&format!("bad\r\n{}\u{0007}", "🙂".repeat(800)));
        assert!(
            detail.starts_with("factory materialization rejected before mission creation: bad ")
        );
        assert!(detail.len() <= 2_000);
        assert!(!detail.chars().any(char::is_control));
    }

    #[test]
    fn artifact_content_disposition_encodes_untrusted_file_names() {
        let value =
            artifact_content_disposition("safe.txt\"; filename=\"payload.html").expect("header");
        assert_eq!(
            value.to_str().expect("header text"),
            "attachment; filename=\"safe.txt___filename__payload.html\"; filename*=UTF-8''safe.txt%22%3B%20filename%3D%22payload.html"
        );
    }
}
