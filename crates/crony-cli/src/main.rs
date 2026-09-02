mod factory;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use crony_domain::{DeliverableForm, DeliverableSpec, EntityLink};
use crony_protocol::{
    ClaimLeaseRequest, CreateMissionRequest, CreateRoomMessageRequest, EmergencyStopRequest,
    InterruptRunRequest, LaunchMissionRequest, QueueMessageRequest, ReleaseLeaseRequest,
    ResumeRunRequest, TransferLeaseRequest, VerificationDecisionRequest,
};
use reqwest::{
    Client, Method,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
};
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

    #[arg(long, env = "CRONY_ACCESS_TOKEN")]
    access_token: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Health,
    Bootstrap,
    Snapshot {
        corp_id: Uuid,
        actor_id: Uuid,
    },
    Factory {
        #[command(flatten)]
        args: Box<factory::FactoryArgs>,
    },
    Mission {
        corp_id: Uuid,
        actor_id: Uuid,
        #[arg(long)]
        adapter: Option<String>,
        #[arg(long)]
        strategy: Option<String>,
        #[arg(long, value_enum, default_value_t = DeliverableArg::Archive)]
        deliverable: DeliverableArg,
        #[arg(long)]
        commit_after_verification: bool,
        title: String,
    },
    RoomMessage {
        corp_id: Uuid,
        room_id: Uuid,
        actor_id: Uuid,
        body: String,
        #[arg(long)]
        reply_to: Option<Uuid>,
        #[arg(long)]
        mention: Vec<Uuid>,
        #[arg(long, requires = "link_id")]
        link_kind: Option<String>,
        #[arg(long, requires = "link_kind")]
        link_id: Option<Uuid>,
    },
    Launch {
        corp_id: Uuid,
        mission_id: Uuid,
        actor_id: Uuid,
    },
    Resume {
        corp_id: Uuid,
        run_id: Uuid,
        actor_id: Uuid,
        prompt: String,
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
    Interrupt {
        corp_id: Uuid,
        agent_id: Uuid,
        actor_id: Uuid,
        token: Uuid,
        reason: String,
    },
    VerificationDecision {
        corp_id: Uuid,
        run_id: Uuid,
        actor_id: Uuid,
        #[arg(long)]
        approve: bool,
        note: String,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum DeliverableArg {
    CommitBranch,
    Patch,
    Archive,
    TypedArtifactSet,
    ReviewOnlyReport,
}

impl From<DeliverableArg> for DeliverableForm {
    fn from(value: DeliverableArg) -> Self {
        match value {
            DeliverableArg::CommitBranch => Self::CommitBranch,
            DeliverableArg::Patch => Self::Patch,
            DeliverableArg::Archive => Self::Archive,
            DeliverableArg::TypedArtifactSet => Self::TypedArtifactSet,
            DeliverableArg::ReviewOnlyReport => Self::ReviewOnlyReport,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let mut headers = HeaderMap::new();
    if let Some(token) = args.access_token.as_deref() {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}"))
                .context("access token cannot be encoded as an HTTP header")?,
        );
    }
    let client = Client::builder().default_headers(headers).build()?;
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
        Command::Snapshot { corp_id, actor_id } => {
            request(
                &client,
                Method::GET,
                format!(
                    "{}/api/corps/{corp_id}/snapshot?actor_id={actor_id}",
                    args.server
                ),
                None,
            )
            .await?
        }
        Command::Factory { args: factory_args } => {
            factory::run(&client, &args.server, *factory_args).await?
        }
        Command::Mission {
            corp_id,
            actor_id,
            adapter,
            strategy,
            deliverable,
            commit_after_verification,
            title,
        } => {
            request(
                &client,
                Method::POST,
                format!("{}/api/corps/{corp_id}/missions", args.server),
                Some(serde_json::to_value(CreateMissionRequest {
                    title,
                    requested_by: actor_id,
                    preferred_adapter: adapter,
                    preferred_model: None,
                    reasoning_effort: None,
                    strategy,
                    secret_refs: Vec::new(),
                    budget_tokens: None,
                    budget_cost_microusd: None,
                    deliverable: Some(DeliverableSpec {
                        form: deliverable.into(),
                        commit_after_verification: commit_after_verification
                            || matches!(deliverable, DeliverableArg::CommitBranch),
                        paths: Vec::new(),
                    }),
                })?),
            )
            .await?
        }
        Command::RoomMessage {
            corp_id,
            room_id,
            actor_id,
            body,
            reply_to,
            mention,
            link_kind,
            link_id,
        } => {
            let link = match (link_kind, link_id) {
                (Some(kind), Some(id)) => Some(EntityLink { kind, id }),
                (None, None) => None,
                _ => anyhow::bail!("link kind and link id must be supplied together"),
            };
            request(
                &client,
                Method::POST,
                format!(
                    "{}/api/corps/{corp_id}/rooms/{room_id}/messages",
                    args.server
                ),
                Some(serde_json::to_value(CreateRoomMessageRequest {
                    actor_id,
                    body,
                    reply_to_id: reply_to,
                    mentions: mention,
                    link,
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
        Command::Resume {
            corp_id,
            run_id,
            actor_id,
            prompt,
        } => {
            request(
                &client,
                Method::POST,
                format!("{}/api/corps/{corp_id}/runs/{run_id}/resume", args.server),
                Some(serde_json::to_value(ResumeRunRequest {
                    requested_by: actor_id,
                    prompt,
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
        Command::Interrupt {
            corp_id,
            agent_id,
            actor_id,
            token,
            reason,
        } => {
            request(
                &client,
                Method::POST,
                format!(
                    "{}/api/corps/{corp_id}/agents/{agent_id}/interrupt",
                    args.server
                ),
                Some(serde_json::to_value(InterruptRunRequest {
                    actor_id,
                    lease_token: token,
                    reason,
                })?),
            )
            .await?
        }
        Command::VerificationDecision {
            corp_id,
            run_id,
            actor_id,
            approve,
            note,
        } => {
            request(
                &client,
                Method::POST,
                format!(
                    "{}/api/corps/{corp_id}/runs/{run_id}/verification-decision",
                    args.server
                ),
                Some(serde_json::to_value(VerificationDecisionRequest {
                    actor_id,
                    approved: approve,
                    note,
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
