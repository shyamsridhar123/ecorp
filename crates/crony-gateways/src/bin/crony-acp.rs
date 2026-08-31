use std::{collections::HashMap, io::Write};

use anyhow::{Context, Result};
use clap::Parser;
use crony_gateways::{
    ACP_PROTOCOL_VERSION, GatewayClient, JsonRpcRequest, acp_initialize, failure, success,
};
use reqwest::Method;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use uuid::Uuid;

#[derive(Debug, Parser)]
struct Args {
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

#[derive(Debug, Default)]
struct Session {
    mission_id: Option<Uuid>,
    run_id: Option<Uuid>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let client = GatewayClient::new(args.server, args.corp_id, args.actor_id, args.access_token);
    let mut sessions = HashMap::<Uuid, Session>::new();
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let response = match serde_json::from_str::<JsonRpcRequest>(&line) {
            Ok(request) => handle(&client, &mut sessions, request).await,
            Err(error) => failure(None, -32700, error.to_string()),
        };
        println!("{}", serde_json::to_string(&response)?);
        std::io::stdout().flush()?;
    }
    Ok(())
}

async fn handle(
    client: &GatewayClient,
    sessions: &mut HashMap<Uuid, Session>,
    request: JsonRpcRequest,
) -> Value {
    let id = request.id.clone();
    let result = match request.method.as_str() {
        "initialize" => {
            let requested = request
                .params
                .get("protocolVersion")
                .and_then(Value::as_u64)
                .unwrap_or(ACP_PROTOCOL_VERSION.into()) as u32;
            return success(id, acp_initialize(requested));
        }
        "session/new" => {
            let session_id = Uuid::new_v4();
            sessions.insert(session_id, Session::default());
            Ok(json!({"sessionId":session_id}))
        }
        "session/prompt" => prompt(client, sessions, &request.params).await,
        "session/load" => load(client, sessions, &request.params).await,
        "session/cancel" => cancel(sessions, &request.params),
        _ => Err(anyhow::anyhow!("method not found")),
    };
    match result {
        Ok(value) => success(id, value),
        Err(error) => failure(id, -32000, error.to_string()),
    }
}

async fn prompt(
    client: &GatewayClient,
    sessions: &mut HashMap<Uuid, Session>,
    params: &Value,
) -> Result<Value> {
    let session_id = parse_session_id(params)?;
    let text = params
        .get("prompt")
        .and_then(Value::as_str)
        .context("ACP prompt omitted prompt")?;
    let mission = client
        .request(
            Method::POST,
            &format!("/api/corps/{}/missions", client.corp_id),
            Some(json!({
                "requested_by":client.actor_id,
                "title":text,
                "preferred_adapter":params.get("adapter"),
                "strategy":"single"
            })),
        )
        .await?;
    let mission_id = mission
        .get("mission_id")
        .and_then(Value::as_str)
        .context("ECorp mission response omitted id")
        .and_then(|value| Uuid::parse_str(value).context("mission id is invalid"))?;
    let launch = client
        .request(
            Method::POST,
            &format!("/api/corps/{}/missions/{mission_id}/launch", client.corp_id),
            Some(json!({"requested_by":client.actor_id})),
        )
        .await?;
    let run_id = launch
        .get("run_id")
        .and_then(Value::as_str)
        .context("ECorp launch response omitted run id")
        .and_then(|value| Uuid::parse_str(value).context("run id is invalid"))?;
    sessions.insert(
        session_id,
        Session {
            mission_id: Some(mission_id),
            run_id: Some(run_id),
        },
    );
    Ok(
        json!({"stopReason":"end_turn","sessionId":session_id,"missionId":mission_id,"runId":run_id}),
    )
}

async fn load(
    client: &GatewayClient,
    sessions: &HashMap<Uuid, Session>,
    params: &Value,
) -> Result<Value> {
    let session_id = parse_session_id(params)?;
    let session = sessions.get(&session_id).context("unknown ACP session")?;
    let snapshot = client
        .request(
            Method::GET,
            &format!(
                "/api/corps/{}/snapshot?actor_id={}",
                client.corp_id, client.actor_id
            ),
            None,
        )
        .await?;
    Ok(json!({
        "sessionId":session_id,
        "missionId":session.mission_id,
        "runId":session.run_id,
        "snapshot":snapshot
    }))
}

fn cancel(sessions: &HashMap<Uuid, Session>, params: &Value) -> Result<Value> {
    let session_id = parse_session_id(params)?;
    let session = sessions.get(&session_id).context("unknown ACP session")?;
    Ok(json!({
        "sessionId":session_id,
        "runId":session.run_id,
        "cancelRequested":false,
        "reason":"ECorp cancellation requires an authenticated agent control or emergency-stop command"
    }))
}

fn parse_session_id(params: &Value) -> Result<Uuid> {
    params
        .get("sessionId")
        .and_then(Value::as_str)
        .context("ACP request omitted sessionId")
        .and_then(|value| Uuid::parse_str(value).context("ACP session id is invalid"))
}
