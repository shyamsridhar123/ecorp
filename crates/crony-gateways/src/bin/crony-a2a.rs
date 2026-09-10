use std::{convert::Infallible, net::SocketAddr};

use anyhow::Result;
use axum::{
    Json, Router,
    extract::State,
    response::{
        Sse,
        sse::{Event, KeepAlive},
    },
    routing::{get, post},
};
use clap::Parser;
use crony_gateways::{
    A2A_PROTOCOL_VERSION, GatewayClient, JsonRpcRequest, a2a_agent_card, failure,
    reject_max_task_attempts, success, with_max_task_attempts,
};
use futures_util::stream;
use reqwest::Method;
use serde_json::{Value, json};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, env = "CRONY_A2A_BIND", default_value = "127.0.0.1:8794")]
    bind: SocketAddr,
    #[arg(
        long,
        env = "CRONY_A2A_PUBLIC_URL",
        default_value = "http://127.0.0.1:8794"
    )]
    public_url: String,
    #[arg(
        long,
        env = "CRONY_SERVER_HTTP",
        default_value = "http://127.0.0.1:8791"
    )]
    server: String,
    #[arg(long, env = "CRONY_CORP_ID")]
    corp_id: Uuid,
    #[arg(long, env = "CRONY_ACTOR_ID")]
    actor_id: Uuid,
    #[arg(long, env = "CRONY_ACCESS_TOKEN")]
    access_token: Option<String>,
}

#[derive(Clone)]
struct AppState {
    client: GatewayClient,
    public_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let state = AppState {
        client: GatewayClient::new(args.server, args.corp_id, args.actor_id, args.access_token),
        public_url: args.public_url,
    };
    let app = Router::new()
        .route("/.well-known/agent-card.json", get(agent_card))
        .route("/a2a", post(a2a))
        .route("/a2a/stream", post(a2a_stream))
        .layer(TraceLayer::new_for_http())
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(args.bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn agent_card(State(state): State<AppState>) -> Json<Value> {
    Json(a2a_agent_card(&state.public_url))
}

async fn a2a(State(state): State<AppState>, Json(request): Json<JsonRpcRequest>) -> Json<Value> {
    Json(handle_a2a(&state.client, request).await)
}

async fn a2a_stream(
    State(state): State<AppState>,
    Json(request): Json<JsonRpcRequest>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let response = handle_a2a(&state.client, request).await;
    let values = vec![
        json!({"kind":"status-update","state":"submitted","final":false}),
        json!({"kind":"artifact-update","artifact":response,"final":true}),
    ];
    Sse::new(stream::iter(values.into_iter().map(|value| {
        Ok(Event::default()
            .json_data(value)
            .expect("serialize A2A event"))
    })))
    .keep_alive(KeepAlive::default())
}

async fn handle_a2a(client: &GatewayClient, request: JsonRpcRequest) -> Value {
    let id = request.id.clone();
    let requested_version = request
        .params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(A2A_PROTOCOL_VERSION);
    if requested_version != A2A_PROTOCOL_VERSION {
        return failure(id, -32602, "unsupported A2A protocol version");
    }
    if request.method != "message/send"
        && let Err(error) = reject_max_task_attempts(&request.params)
    {
        return failure(id, -32000, error.to_string());
    }
    let result = match request.method.as_str() {
        "message/send" => send_message(client, &request.params).await,
        "tasks/get" => {
            client
                .request(
                    Method::GET,
                    &format!(
                        "/api/corps/{}/snapshot?actor_id={}",
                        client.corp_id, client.actor_id
                    ),
                    None,
                )
                .await
        }
        "tasks/cancel" => Err(anyhow::anyhow!(
            "task cancellation requires a ECorp agent control lease or emergency-stop policy"
        )),
        _ => Err(anyhow::anyhow!("method not found")),
    };
    match result {
        Ok(value) => success(id, value),
        Err(error) => failure(id, -32000, error.to_string()),
    }
}

async fn send_message(client: &GatewayClient, params: &Value) -> Result<Value> {
    client
        .request(
            Method::POST,
            &format!("/api/corps/{}/missions", client.corp_id),
            Some(a2a_mission_request(client.actor_id, params)?),
        )
        .await
}

fn a2a_mission_request(actor_id: Uuid, params: &Value) -> Result<Value> {
    let text = params
        .pointer("/message/parts/0/text")
        .and_then(Value::as_str)
        .or_else(|| params.get("text").and_then(Value::as_str))
        .ok_or_else(|| anyhow::anyhow!("A2A message omitted text"))?;
    with_max_task_attempts(
        json!({
            "requested_by":actor_id,
            "title":text,
            "strategy":"single"
        }),
        params,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crony_domain::MAX_TASK_ATTEMPTS;

    #[test]
    fn issue224_a2a_creation_keeps_legacy_shape_and_forwards_attempt_choice() {
        let actor = Uuid::from_u128(1);
        for mut params in [
            json!({"text": "A bounded mission"}),
            json!({"message": {"parts": [{"text": "A bounded mission"}]}}),
        ] {
            let legacy = a2a_mission_request(actor, &params).unwrap();
            assert!(legacy.get("max_task_attempts").is_none());
            assert_eq!(legacy["strategy"], "single");
            params["max_task_attempts"] = Value::Null;
            assert_eq!(a2a_mission_request(actor, &params).unwrap(), legacy);
            for value in 1..=MAX_TASK_ATTEMPTS {
                params["max_task_attempts"] = json!(value);
                let mut body = a2a_mission_request(actor, &params).unwrap();
                assert_eq!(body["max_task_attempts"], value);
                body.as_object_mut().unwrap().remove("max_task_attempts");
                assert_eq!(body, legacy);
            }
        }
    }

    #[tokio::test]
    async fn issue224_a2a_invalid_or_postplanning_attempt_fields_fail_before_api_calls() {
        let client = GatewayClient::new(
            "invalid-unused-server".to_owned(),
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            None,
        );
        for (method, value) in [
            ("message/send", json!(MAX_TASK_ATTEMPTS + 1)),
            ("tasks/get", Value::Null),
            ("tasks/cancel", json!(MAX_TASK_ATTEMPTS)),
        ] {
            let response = handle_a2a(
                &client,
                JsonRpcRequest {
                    jsonrpc: "2.0".to_owned(),
                    id: Some(json!(1)),
                    method: method.to_owned(),
                    params: json!({"text": "A bounded mission", "max_task_attempts": value}),
                },
            )
            .await;
            let message = response["error"]["message"].as_str().unwrap();
            assert!(message.contains("max_task_attempts") || message.contains("only be chosen"));
        }
    }
}
