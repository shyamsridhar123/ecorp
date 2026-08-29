use std::{collections::HashSet, net::SocketAddr, sync::Arc};

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
    CreateMissionResponse, CreateRoomMessageRequest, CreateRoomMessageResponse,
    DemoBootstrapResponse, EmergencyStopRequest, EmergencyStopResponse, LaunchMissionRequest,
    LaunchMissionResponse, LeaseMutationResponse, QueueMessageRequest, QueueMessageResponse,
    ReleaseLeaseRequest, RunnerCapability, RunnerSummary, RunnerToServer, ServerToRunner,
    SnapshotResponse, TransferLeaseRequest,
};
use crony_store::{NewRoomMessageInput, PgStore, RunClaim, RunnerConnectInput, RunnerEventInput};
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

    #[arg(long, env = "CRONY_RUNNER_GRACE_SECS", default_value_t = 5)]
    runner_grace_secs: i64,
}

#[derive(Clone)]
struct AppState {
    store: PgStore,
    event_tx: broadcast::Sender<DomainEvent>,
    runners: Arc<DashMap<String, RunnerConnection>>,
    runner_grace_secs: i64,
}

#[derive(Clone)]
struct RunnerConnection {
    connection_epoch: Uuid,
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
        runner_grace_secs: args.runner_grace_secs.max(1),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/api/demo/bootstrap", post(bootstrap_demo))
        .route("/api/demo/reset", post(reset_demo))
        .route(
            "/api/demo/runners/{runner_id}/disconnect",
            post(debug_disconnect_runner),
        )
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
        eve_actor_id: ids.eve_actor_id,
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
        eve_actor_id: ids.eve_actor_id,
        manager_agent_id: ids.manager_agent_id,
        worker_agent_id: ids.worker_agent_id,
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

async fn snapshot(
    State(state): State<AppState>,
    Path(corp_id): Path<Uuid>,
    Query(query): Query<SnapshotQuery>,
) -> Result<Json<SnapshotResponse>, ApiError> {
    let snapshot = state
        .store
        .snapshot(corp_id, query.actor_id)
        .await
        .map_err(map_store_error)?;
    let runners = state
        .store
        .runner_records()
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(|record| RunnerSummary {
            id: record.id,
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
    Path(corp_id): Path<Uuid>,
    Json(request): Json<CreateMissionRequest>,
) -> Result<Json<CreateMissionResponse>, ApiError> {
    let (ids, events) = state
        .store
        .create_mission(corp_id, request.requested_by, &request.title)
        .await
        .map_err(map_store_error)?;
    for event in events {
        publish(&state, event);
    }
    Ok(Json(CreateMissionResponse {
        mission_id: ids.mission_id,
        task_id: ids.task_id,
    }))
}

async fn create_room_message(
    State(state): State<AppState>,
    Path((corp_id, room_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<CreateRoomMessageRequest>,
) -> Result<Json<CreateRoomMessageResponse>, ApiError> {
    let outcome = state
        .store
        .create_room_message(NewRoomMessageInput {
            corp_id,
            room_id,
            actor_id: request.actor_id,
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
            assignment_token: record.assignment_token,
            adapter: record.adapter,
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
) -> Result<Response, ApiError> {
    if !state
        .store
        .actor_belongs_to_corp(corp_id, query.actor_id)
        .await
        .map_err(ApiError::internal)?
    {
        return Err(ApiError::forbidden(
            "actor cannot subscribe to the requested Corp",
        ));
    }
    let visible_rooms = state
        .store
        .visible_room_ids(corp_id, query.actor_id)
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
                query.actor_id,
                query.after_seq,
                visible_rooms,
            )
        })
        .into_response())
}

#[derive(Debug, Deserialize)]
struct BrowserQuery {
    actor_id: Uuid,
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

    let mut registered: Option<(String, Uuid)> = None;
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
                connection_epoch,
                hostname,
                os,
                capabilities,
                active_runs,
            } => {
                let record = match state
                    .store
                    .runner_connected(RunnerConnectInput {
                        id: runner_id.clone(),
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
                        connection_epoch,
                        tx: command_tx.clone(),
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
                registered = Some((runner_id.clone(), connection_epoch));
                let _ = command_tx.send(ServerToRunner::Registered {
                    runner_id: runner_id.clone(),
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
                if !registered.as_ref().is_some_and(|(registered_id, epoch)| {
                    registered_id == &runner_id && *epoch == connection_epoch
                }) {
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
                if registered.as_ref().map(|value| value.0.as_str()) != Some(runner_id.as_str()) {
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

    if let Some((runner_id, connection_epoch)) = registered {
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
