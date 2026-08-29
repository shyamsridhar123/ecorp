use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use crony_protocol::{
    ClaimLeaseRequest, CreateMissionRequest, EmergencyStopRequest, LaunchMissionRequest,
    QueueMessageRequest, ReleaseLeaseRequest, TransferLeaseRequest,
};
use reqwest::{Client, Method};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(name = "crony")]
struct Args {
    #[arg(
        long,
        env = "CRONY_SERVER_HTTP",
        default_value = "http://127.0.0.1:8791"
    )]
    server: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Health,
    Bootstrap,
    Snapshot {
        corp_id: Uuid,
    },
    Mission {
        corp_id: Uuid,
        actor_id: Uuid,
        title: String,
    },
    Launch {
        corp_id: Uuid,
        mission_id: Uuid,
        actor_id: Uuid,
    },
    Lease {
        corp_id: Uuid,
        agent_id: Uuid,
        actor_id: Uuid,
    },
    ReleaseLease {
        corp_id: Uuid,
        agent_id: Uuid,
        actor_id: Uuid,
        token: Uuid,
    },
    TransferLease {
        corp_id: Uuid,
        agent_id: Uuid,
        actor_id: Uuid,
        token: Uuid,
        to_actor_id: Uuid,
    },
    Message {
        corp_id: Uuid,
        agent_id: Uuid,
        actor_id: Uuid,
        #[arg(long)]
        lease_token: Option<Uuid>,
        text: String,
    },
    EmergencyStop {
        corp_id: Uuid,
        agent_id: Uuid,
        actor_id: Uuid,
        reason: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let client = Client::new();
    let response = match args.command {
        Command::Health => {
            request(
                &client,
                Method::GET,
                format!("{}/health", args.server),
                None,
            )
            .await?
        }
        Command::Bootstrap => {
            request(
                &client,
                Method::POST,
                format!("{}/api/demo/bootstrap", args.server),
                Some(Value::Object(Default::default())),
            )
            .await?
        }
        Command::Snapshot { corp_id } => {
            request(
                &client,
                Method::GET,
                format!("{}/api/corps/{corp_id}/snapshot", args.server),
                None,
            )
            .await?
        }
        Command::Mission {
            corp_id,
            actor_id,
            title,
        } => {
            request(
                &client,
                Method::POST,
                format!("{}/api/corps/{corp_id}/missions", args.server),
                Some(serde_json::to_value(CreateMissionRequest {
                    title,
                    requested_by: actor_id,
                })?),
            )
            .await?
        }
        Command::Launch {
            corp_id,
            mission_id,
            actor_id,
        } => {
            request(
                &client,
                Method::POST,
                format!(
                    "{}/api/corps/{corp_id}/missions/{mission_id}/launch",
                    args.server
                ),
                Some(serde_json::to_value(LaunchMissionRequest {
                    requested_by: actor_id,
                })?),
            )
            .await?
        }
        Command::Lease {
            corp_id,
            agent_id,
            actor_id,
        } => {
            request(
                &client,
                Method::POST,
                format!(
                    "{}/api/corps/{corp_id}/agents/{agent_id}/lease",
                    args.server
                ),
                Some(serde_json::to_value(ClaimLeaseRequest { actor_id })?),
            )
            .await?
        }
        Command::ReleaseLease {
            corp_id,
            agent_id,
            actor_id,
            token,
        } => {
            request(
                &client,
                Method::POST,
                format!(
                    "{}/api/corps/{corp_id}/agents/{agent_id}/lease/release",
                    args.server
                ),
                Some(serde_json::to_value(ReleaseLeaseRequest {
                    actor_id,
                    token,
                })?),
            )
            .await?
        }
        Command::TransferLease {
            corp_id,
            agent_id,
            actor_id,
            token,
            to_actor_id,
        } => {
            request(
                &client,
                Method::POST,
                format!(
                    "{}/api/corps/{corp_id}/agents/{agent_id}/lease/transfer",
                    args.server
                ),
                Some(serde_json::to_value(TransferLeaseRequest {
                    actor_id,
                    token,
                    to_actor_id,
                })?),
            )
            .await?
        }
        Command::Message {
            corp_id,
            agent_id,
            actor_id,
            lease_token,
            text,
        } => {
            request(
                &client,
                Method::POST,
                format!(
                    "{}/api/corps/{corp_id}/agents/{agent_id}/messages",
                    args.server
                ),
                Some(serde_json::to_value(QueueMessageRequest {
                    actor_id,
                    lease_token,
                    text,
                })?),
            )
            .await?
        }
        Command::EmergencyStop {
            corp_id,
            agent_id,
            actor_id,
            reason,
        } => {
            request(
                &client,
                Method::POST,
                format!(
                    "{}/api/corps/{corp_id}/agents/{agent_id}/emergency-stop",
                    args.server
                ),
                Some(serde_json::to_value(EmergencyStopRequest {
                    actor_id,
                    reason,
                })?),
            )
            .await?
        }
    };
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

async fn request(
    client: &Client,
    method: Method,
    url: String,
    body: Option<Value>,
) -> Result<Value> {
    let mut request = client.request(method, &url);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .with_context(|| format!("request {url}"))?;
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        anyhow::bail!("{status}: {text}");
    }
    serde_json::from_str(&text).with_context(|| format!("decode response from {url}: {text}"))
}
