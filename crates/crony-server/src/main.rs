use std::{net::SocketAddr, sync::Arc};

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use clap::Parser;
use crony_domain::DomainEvent;
use crony_protocol::{
    BrowserSocketMessage, ClaimLeaseRequest, ClaimLeaseResponse, CreateMissionRequest,
    CreateMissionResponse, DemoBootstrapResponse, EmergencyStopRequest, EmergencyStopResponse,
    LaunchMissionRequest, LaunchMissionResponse, LeaseMutationResponse, QueueMessageRequest,
    QueueMessageResponse, ReleaseLeaseRequest, RunnerSummary, RunnerToServer, ServerToRunner,
    SnapshotResponse, TransferLeaseRequest,
};
use crony_store::{PgStore, RunnerEventInput};
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{broadcast, mpsc};
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing::{error, info, warn};
use uuid::Uuid;

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
}

#[derive(Clone)]
struct AppState {
    store: PgStore,
    event_tx: broadcast::Sender<DomainEvent>,
    runners: Arc<DashMap<String, RunnerConnection>>,
}

#[derive(Clone)]
struct RunnerConnection {
    summary: RunnerSummary,
    tx: mpsc::UnboundedSender<ServerToRunner>,
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

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
    runners: usize,
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
    let (event_tx, _) = broadcast::channel(2_048);
    let state = AppState {
        store,
        event_tx,
        runners: Arc::new(DashMap::new()),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/api/demo/bootstrap", post(bootstrap_demo))
        .route("/api/demo/reset", post(reset_demo))
        .route("/api/corps/{corp_id}/snapshot", get(snapshot))
        .route("/api/corps/{corp_id}/missions", post(create_mission))
        .route(
            "/api/corps/{corp_id}/missions/{mission_id}/launch",
            post(launch_mission),
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
        .route("/ws/corps/{corp_id}", get(browser_websocket))
        .route("/ws/runner", get(runner_websocket))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_headers(Any)
                .allow_methods(Any),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state);

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
    })
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
        manager_agent_id: ids.manager_agent_id,
        worker_agent_id: ids.worker_agent_id,
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
        manager_agent_id: ids.manager_agent_id,
        worker_agent_id: ids.worker_agent_id,
    }))
}

async fn snapshot(
    State(state): State<AppState>,
    Path(corp_id): Path<Uuid>,
) -> Result<Json<SnapshotResponse>, ApiError> {
    let snapshot = state
        .store
        .snapshot(corp_id)
        .await
        .map_err(ApiError::internal)?;
    let runners = state
        .runners
        .iter()
        .map(|entry| entry.summary.clone())
        .collect();
    Ok(Json(SnapshotResponse { snapshot, runners }))
}

async fn create_mission(
    State(state): State<AppState>,
    Path(corp_id): Path<Uuid>,
    Json(request): Json<CreateMissionRequest>,
) -> Result<Json<CreateMissionResponse>, ApiError> {
    let (ids, events) = state
        .store
        .create_mission(corp_id, request.requested_by, &request.title)
        .await
        .map_err(ApiError::bad_request)?;
    for event in events {
        publish(&state, event);
    }
    Ok(Json(CreateMissionResponse {
        mission_id: ids.mission_id,
        task_id: ids.task_id,
    }))
}

async fn launch_mission(
    State(state): State<AppState>,
    Path((corp_id, mission_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<LaunchMissionRequest>,
) -> Result<Json<LaunchMissionResponse>, ApiError> {
    let runner = state
        .runners
        .iter()
        .next()
        .map(|entry| (entry.key().clone(), entry.tx.clone()))
        .ok_or_else(|| ApiError::conflict("no runner is connected"))?;

    let (record, event) = state
        .store
        .create_run(corp_id, mission_id, request.requested_by, &runner.0)
        .await
        .map_err(ApiError::conflict)?;
    publish(&state, event);

    runner
        .1
        .send(ServerToRunner::StartRun {
            corp_id: record.corp_id,
            room_id: record.room_id,
            mission_id: record.mission_id,
            task_id: record.task_id,
            run_id: record.run_id,
            agent_id: record.agent_id,
            mission_title: record.mission_title,
        })
        .map_err(|_| ApiError::conflict("runner disconnected before accepting the run"))?;

    Ok(Json(LaunchMissionResponse {
        run_id: record.run_id,
        runner_id: runner.0,
    }))
}

async fn claim_lease(
    State(state): State<AppState>,
    Path((corp_id, agent_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<ClaimLeaseRequest>,
) -> Result<Json<ClaimLeaseResponse>, ApiError> {
    let outcome = state
        .store
        .acquire_lease(corp_id, agent_id, request.actor_id)
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
    Path((corp_id, agent_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<ReleaseLeaseRequest>,
) -> Result<Json<LeaseMutationResponse>, ApiError> {
    let outcome = state
        .store
        .release_lease(corp_id, agent_id, request.actor_id, request.token)
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
    Path((corp_id, agent_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<TransferLeaseRequest>,
) -> Result<Json<LeaseMutationResponse>, ApiError> {
    let outcome = state
        .store
        .transfer_lease(
            corp_id,
            agent_id,
            request.actor_id,
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
    Path((corp_id, agent_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<QueueMessageRequest>,
) -> Result<Json<QueueMessageResponse>, ApiError> {
    let outcome = state
        .store
        .queue_message(
            corp_id,
            agent_id,
            request.actor_id,
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
            actor_id: request.actor_id,
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
    Path((corp_id, agent_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<EmergencyStopRequest>,
) -> Result<Json<EmergencyStopResponse>, ApiError> {
    let outcome = match state
        .store
        .request_emergency_stop(corp_id, agent_id, request.actor_id, &request.reason)
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
            corp_id,
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

async fn browser_websocket(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Path(corp_id): Path<Uuid>,
    Query(query): Query<BrowserQuery>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| browser_socket(socket, state, corp_id, query.after_seq))
}

#[derive(Debug, Deserialize)]
struct BrowserQuery {
    #[serde(default)]
    after_seq: i64,
}

async fn browser_socket(socket: WebSocket, state: AppState, corp_id: Uuid, after_seq: i64) {
    let (mut sender, mut receiver) = socket.split();
    let mut events = state.event_tx.subscribe();
    let mut cursor = after_seq.max(0);

    loop {
        let replay = match state.store.events_after(corp_id, cursor, 500).await {
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
                    Ok(event) if event.corp_id == corp_id && event.seq > cursor => {
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

    let mut registered_id: Option<String> = None;
    while let Some(message) = receiver.next().await {
        let message = match message {
            Ok(Message::Text(text)) => text,
            Ok(Message::Close(_)) => break,
            Ok(_) => continue,
            Err(error) => {
                warn!(%error, "runner websocket read failed");
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
                hostname,
                os,
                capabilities,
            } => {
                registered_id = Some(runner_id.clone());
                state.runners.insert(
                    runner_id.clone(),
                    RunnerConnection {
                        summary: RunnerSummary {
                            id: runner_id.clone(),
                            hostname,
                            os,
                            capabilities,
                            connected: true,
                        },
                        tx: command_tx.clone(),
                    },
                );
                let _ = command_tx.send(ServerToRunner::Registered {
                    runner_id: runner_id.clone(),
                });
                info!(%runner_id, "runner connected");
            }
            RunnerToServer::Heartbeat { runner_id } => {
                if registered_id.as_deref() != Some(runner_id.as_str()) {
                    warn!(%runner_id, "heartbeat from unregistered runner");
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
                if registered_id.as_deref() != Some(runner_id.as_str()) {
                    warn!(%runner_id, "run event runner id does not match registered socket");
                    continue;
                }
                match state
                    .store
                    .apply_runner_event(RunnerEventInput {
                        event_id,
                        runner_id,
                        corp_id,
                        run_id,
                        agent_id,
                        event_type: event_type.clone(),
                        payload,
                    })
                    .await
                {
                    Ok(Some(event)) => publish(&state, event),
                    Ok(None) => {}
                    Err(error) => {
                        warn!(%error, %run_id, %event_type, "failed to apply runner event")
                    }
                }
            }
        }
    }

    if let Some(runner_id) = registered_id {
        state.runners.remove(&runner_id);
        info!(%runner_id, "runner disconnected");
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
