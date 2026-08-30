use anyhow::{Context, Result, anyhow};
use reqwest::{Client, Method};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
pub const ACP_PROTOCOL_VERSION: u32 = 1;
pub const A2A_PROTOCOL_VERSION: &str = "1.0";

#[derive(Debug, Clone)]
pub struct GatewayClient {
    pub server: String,
    pub corp_id: Uuid,
    pub actor_id: Uuid,
    pub access_token: Option<String>,
    http: Client,
}

impl GatewayClient {
    pub fn new(
        server: String,
        corp_id: Uuid,
        actor_id: Uuid,
        access_token: Option<String>,
    ) -> Self {
        Self {
            server: server.trim_end_matches('/').to_owned(),
            corp_id,
            actor_id,
            access_token,
            http: Client::new(),
        }
    }

    pub async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        let mut request = self
            .http
            .request(method, format!("{}{}", self.server, path))
            .header("content-type", "application/json");
        if let Some(token) = &self.access_token {
            request = request.bearer_auth(token);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.context("send Crony API request")?;
        let status = response.status();
        let value = response
            .json::<Value>()
            .await
            .context("decode Crony API response")?;
        if !status.is_success() {
            return Err(anyhow!("Crony API returned {status}: {value}"));
        }
        Ok(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

pub fn success(id: Option<Value>, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}

pub fn failure(id: Option<Value>, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message.into()}})
}

pub fn mcp_capabilities() -> Value {
    json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "serverInfo": {"name":"crony-mcp","version":env!("CARGO_PKG_VERSION")},
        "capabilities":{"tools":{"listChanged":false},"resources":{}}
    })
}

pub fn mcp_tools() -> Value {
    json!({
        "tools":[
            {
                "name":"crony_snapshot",
                "description":"Read the authenticated actor's Corp-scoped operational snapshot.",
                "inputSchema":{"type":"object","properties":{},"additionalProperties":false}
            },
            {
                "name":"crony_create_mission",
                "description":"Create a bounded Crony mission.",
                "inputSchema":{
                    "type":"object",
                    "required":["title"],
                    "properties":{
                        "title":{"type":"string"},
                        "adapter":{"type":"string"},
                        "strategy":{"type":"string"}
                    },
                    "additionalProperties":false
                }
            },
            {
                "name":"crony_post_room_message",
                "description":"Post a durable message to a room in the configured Corp.",
                "inputSchema":{
                    "type":"object",
                    "required":["room_id","body"],
                    "properties":{
                        "room_id":{"type":"string","format":"uuid"},
                        "body":{"type":"string"}
                    },
                    "additionalProperties":false
                }
            }
        ]
    })
}

pub async fn handle_mcp(client: &GatewayClient, request: JsonRpcRequest) -> Value {
    let id = request.id.clone();
    let result = match request.method.as_str() {
        "initialize" => Ok(mcp_capabilities()),
        "tools/list" => Ok(mcp_tools()),
        "tools/call" => handle_mcp_tool(client, &request.params).await,
        "resources/list" => Ok(json!({"resources":[]})),
        _ => return failure(id, -32601, "method not found"),
    };
    match result {
        Ok(result) => success(id, result),
        Err(error) => failure(id, -32000, error.to_string()),
    }
}

async fn handle_mcp_tool(client: &GatewayClient, params: &Value) -> Result<Value> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .context("tool call omitted name")?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let value = match name {
        "crony_snapshot" => {
            client
                .request(
                    Method::GET,
                    &format!(
                        "/api/corps/{}/snapshot?actor_id={}",
                        client.corp_id, client.actor_id
                    ),
                    None,
                )
                .await?
        }
        "crony_create_mission" => {
            let title = arguments
                .get("title")
                .and_then(Value::as_str)
                .context("mission tool omitted title")?;
            client
                .request(
                    Method::POST,
                    &format!("/api/corps/{}/missions", client.corp_id),
                    Some(json!({
                        "requested_by":client.actor_id,
                        "title":title,
                        "preferred_adapter":arguments.get("adapter"),
                        "strategy":arguments.get("strategy")
                    })),
                )
                .await?
        }
        "crony_post_room_message" => {
            let room_id = arguments
                .get("room_id")
                .and_then(Value::as_str)
                .context("message tool omitted room_id")?;
            let body = arguments
                .get("body")
                .and_then(Value::as_str)
                .context("message tool omitted body")?;
            client
                .request(
                    Method::POST,
                    &format!("/api/corps/{}/rooms/{room_id}/messages", client.corp_id),
                    Some(json!({
                        "actor_id":client.actor_id,
                        "body":body,
                        "reply_to_id":null,
                        "mentions":[],
                        "link":null
                    })),
                )
                .await?
        }
        _ => return Err(anyhow!("unknown Crony MCP tool {name}")),
    };
    Ok(json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value}))
}

pub fn negotiate_version(requested: &str, supported: &[&str]) -> Result<String> {
    if supported.contains(&requested) {
        Ok(requested.to_owned())
    } else {
        Err(anyhow!(
            "unsupported protocol version {requested}; supported: {}",
            supported.join(", ")
        ))
    }
}

pub fn a2a_agent_card(base_url: &str) -> Value {
    json!({
        "name":"Crony Corp gateway",
        "description":"Corp-scoped missions, messages, task status, and streaming events.",
        "url":format!("{}/a2a",base_url.trim_end_matches('/')),
        "protocolVersion":A2A_PROTOCOL_VERSION,
        "capabilities":{"streaming":true,"pushNotifications":false},
        "defaultInputModes":["text/plain","application/json"],
        "defaultOutputModes":["application/json"],
        "skills":[
            {"id":"mission","name":"Mission execution","description":"Create and inspect Crony missions"},
            {"id":"room-message","name":"Room messaging","description":"Post durable Corp room messages"}
        ]
    })
}

pub fn acp_initialize(requested_version: u32) -> Value {
    if requested_version == ACP_PROTOCOL_VERSION {
        json!({
            "protocolVersion":ACP_PROTOCOL_VERSION,
            "agentCapabilities":{
                "loadSession":true,
                "promptCapabilities":{"image":false,"audio":false,"embeddedContext":true},
                "mcpCapabilities":{"http":true,"sse":true}
            }
        })
    } else {
        json!({
            "error":{
                "code":-32602,
                "message":format!("unsupported ACP version {requested_version}")
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_versions_fail_closed() {
        assert_eq!(
            negotiate_version(MCP_PROTOCOL_VERSION, &[MCP_PROTOCOL_VERSION]).expect("version"),
            MCP_PROTOCOL_VERSION
        );
        assert!(negotiate_version("1900-01-01", &[MCP_PROTOCOL_VERSION]).is_err());
        assert!(acp_initialize(ACP_PROTOCOL_VERSION).get("error").is_none());
        assert!(acp_initialize(99).get("error").is_some());
    }

    #[test]
    fn gateways_do_not_expose_internal_database_shapes() {
        let tools = mcp_tools().to_string();
        assert!(!tools.contains("assignment_token"));
        assert!(!tools.contains("control_leases"));
        let card = a2a_agent_card("https://example.test").to_string();
        assert!(!card.contains("verification_requests"));
        assert!(card.contains("\"streaming\":true"));
    }
}
