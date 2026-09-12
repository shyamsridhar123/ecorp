use std::{collections::HashMap, io::Write};

use anyhow::{Context, Result};
use clap::Parser;
use crony_gateways::{
    ACP_PROTOCOL_VERSION, GatewayClient, JsonRpcRequest, acp_initialize, failure,
    reject_max_task_attempts, success, with_max_task_attempts,
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
    if request.method != "session/prompt"
        && let Err(error) = reject_max_task_attempts(&request.params)
    {
        return failure(id, -32000, error.to_string());
    }
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
    let mission = client
        .request(
            Method::POST,
            &format!("/api/corps/{}/missions", client.corp_id),
            Some(acp_mission_request(client.actor_id, params)?),
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

fn acp_mission_request(actor_id: Uuid, params: &Value) -> Result<Value> {
    let text = params
        .get("prompt")
        .and_then(Value::as_str)
        .context("ACP prompt omitted prompt")?;
    with_max_task_attempts(
        json!({
            "requested_by":actor_id,
            "title":text,
            "preferred_adapter":params.get("adapter"),
            "strategy":"single"
        }),
        params,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crony_domain::MAX_TASK_ATTEMPTS;

    #[test]
    fn issue224_acp_creation_keeps_legacy_shape_and_forwards_attempt_choice() {
        let actor = Uuid::from_u128(1);
        let mut params = json!({"prompt": "A bounded mission", "adapter": "fake-process"});
        let legacy = acp_mission_request(actor, &params).unwrap();
        assert!(legacy.get("max_task_attempts").is_none());
        assert_eq!(legacy["strategy"], "single");
        params["max_task_attempts"] = Value::Null;
        assert_eq!(acp_mission_request(actor, &params).unwrap(), legacy);
        for value in 1..=MAX_TASK_ATTEMPTS {
            params["max_task_attempts"] = json!(value);
            let mut body = acp_mission_request(actor, &params).unwrap();
            assert_eq!(body["max_task_attempts"], value);
            body.as_object_mut().unwrap().remove("max_task_attempts");
            assert_eq!(body, legacy);
        }
    }

    #[tokio::test]
    async fn issue224_acp_invalid_or_postplanning_attempt_fields_leave_sessions_unchanged() {
        let client = GatewayClient::new(
            "invalid-unused-server".to_owned(),
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            None,
        );
        let session_id = Uuid::from_u128(3);
        let mut sessions = HashMap::from([(session_id, Session::default())]);
        for (method, value) in [
            ("session/prompt", json!(MAX_TASK_ATTEMPTS + 1)),
            ("session/new", Value::Null),
            ("session/load", json!(MAX_TASK_ATTEMPTS)),
            ("session/cancel", Value::Null),
        ] {
            let response = handle(
                &client,
                &mut sessions,
                JsonRpcRequest {
                    jsonrpc: "2.0".to_owned(),
                    id: Some(json!(1)),
                    method: method.to_owned(),
                    params: json!({
                        "sessionId": session_id, "prompt": "A bounded mission",
                        "max_task_attempts": value,
                    }),
                },
            )
            .await;
            let message = response["error"]["message"].as_str().unwrap();
            assert!(message.contains("max_task_attempts") || message.contains("only be chosen"));
            assert_eq!(sessions.len(), 1);
            assert!(sessions[&session_id].mission_id.is_none());
            assert!(sessions[&session_id].run_id.is_none());
        }
    }
}
