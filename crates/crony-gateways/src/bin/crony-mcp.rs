use std::io::Write;

use anyhow::Result;
use clap::Parser;
use crony_gateways::{GatewayClient, JsonRpcRequest, handle_mcp};
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

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let client = GatewayClient::new(args.server, args.corp_id, args.actor_id, args.access_token);
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let response = match serde_json::from_str::<JsonRpcRequest>(&line) {
            Ok(request) => handle_mcp(&client, request).await,
            Err(error) => crony_gateways::failure(None, -32700, error.to_string()),
        };
        println!("{}", serde_json::to_string(&response)?);
        std::io::stdout().flush()?;
    }
    Ok(())
}
