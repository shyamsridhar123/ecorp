use std::{
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use axum::http::{HeaderMap, header::AUTHORIZATION};
use dashmap::DashMap;
use reqwest::Url;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ServerMode {
    Development,
    Production,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Read,
    PostMessage,
    Operate,
    Approve,
    EmergencyStop,
    Recover,
    Publish,
    Manage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorpRole {
    Owner,
    Admin,
    Manager,
    Member,
    Guest,
    Spectator,
}

impl CorpRole {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "owner" => Ok(Self::Owner),
            "admin" => Ok(Self::Admin),
            "manager" => Ok(Self::Manager),
            "member" => Ok(Self::Member),
            "guest" => Ok(Self::Guest),
            "spectator" => Ok(Self::Spectator),
            _ => Err(anyhow!("unsupported human Corp role {value}")),
        }
    }

    pub const fn allows(self, permission: Permission) -> bool {
        match permission {
            Permission::Read => true,
            Permission::PostMessage => !matches!(self, Self::Spectator),
            Permission::Operate | Permission::Approve => {
                matches!(
                    self,
                    Self::Owner | Self::Admin | Self::Manager | Self::Member
                )
            }
            Permission::EmergencyStop => {
                matches!(self, Self::Owner | Self::Admin | Self::Manager)
            }
            Permission::Recover => {
                matches!(self, Self::Owner | Self::Admin | Self::Manager)
            }
            Permission::Publish => {
                matches!(self, Self::Owner | Self::Admin | Self::Manager)
            }
            Permission::Manage => matches!(self, Self::Owner | Self::Admin),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Principal {
    Development,
    Oidc {
        issuer: String,
        subject: String,
        email: Option<String>,
    },
}

#[derive(Debug, Clone)]
struct CachedPrincipal {
    principal: Principal,
    valid_until: Instant,
}

#[derive(Debug, Clone)]
struct WebSocketTicket {
    corp_id: Uuid,
    actor_id: Uuid,
    valid_until: Instant,
}

#[derive(Clone)]
pub struct AuthService {
    mode: ServerMode,
    issuer: Option<String>,
    userinfo_endpoint: Option<Url>,
    client: reqwest::Client,
    cache: Arc<DashMap<String, CachedPrincipal>>,
    websocket_tickets: Arc<DashMap<String, WebSocketTicket>>,
}

#[derive(Debug, Deserialize)]
struct DiscoveryDocument {
    issuer: String,
    userinfo_endpoint: String,
}

#[derive(Debug, Deserialize)]
struct UserInfo {
    sub: String,
    email: Option<String>,
}

impl AuthService {
    pub async fn initialize(
        mode: ServerMode,
        issuer: Option<String>,
        allow_insecure_oidc: bool,
    ) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .context("build OIDC HTTP client")?;
        if mode == ServerMode::Development {
            return Ok(Self {
                mode,
                issuer: None,
                userinfo_endpoint: None,
                client,
                cache: Arc::new(DashMap::new()),
                websocket_tickets: Arc::new(DashMap::new()),
            });
        }

        let issuer = issuer
            .map(|value| value.trim_end_matches('/').to_owned())
            .filter(|value| !value.is_empty())
            .context("CRONY_OIDC_ISSUER is required in production mode")?;
        let issuer_url = Url::from_str(&issuer).context("parse OIDC issuer URL")?;
        if issuer_url.scheme() != "https" && !allow_insecure_oidc {
            return Err(anyhow!(
                "production OIDC issuer must use https; use --allow-insecure-oidc only for isolated tests"
            ));
        }
        let discovery_url = issuer_url
            .join(".well-known/openid-configuration")
            .context("construct OIDC discovery URL")?;
        let discovery = client
            .get(discovery_url)
            .send()
            .await
            .context("fetch OIDC discovery document")?
            .error_for_status()
            .context("OIDC discovery returned an error")?
            .json::<DiscoveryDocument>()
            .await
            .context("decode OIDC discovery document")?;
        if discovery.issuer.trim_end_matches('/') != issuer {
            return Err(anyhow!(
                "OIDC discovery issuer mismatch: expected {issuer}, received {}",
                discovery.issuer
            ));
        }
        let userinfo_endpoint =
            Url::from_str(&discovery.userinfo_endpoint).context("parse OIDC userinfo endpoint")?;
        if userinfo_endpoint.scheme() != "https" && !allow_insecure_oidc {
            return Err(anyhow!("OIDC userinfo endpoint must use https"));
        }

        Ok(Self {
            mode,
            issuer: Some(issuer),
            userinfo_endpoint: Some(userinfo_endpoint),
            client,
            cache: Arc::new(DashMap::new()),
            websocket_tickets: Arc::new(DashMap::new()),
        })
    }

    pub const fn mode(&self) -> ServerMode {
        self.mode
    }

    pub async fn authenticate_headers(&self, headers: &HeaderMap) -> Result<Principal> {
        if self.mode == ServerMode::Development {
            return Ok(Principal::Development);
        }
        let value = headers
            .get(AUTHORIZATION)
            .context("missing Authorization bearer token")?
            .to_str()
            .context("Authorization header is not valid UTF-8")?;
        let token = value
            .strip_prefix("Bearer ")
            .filter(|value| !value.trim().is_empty())
            .context("Authorization must use a Bearer token")?;
        self.authenticate_bearer(token).await
    }

    pub async fn authenticate_bearer(&self, token: &str) -> Result<Principal> {
        if self.mode == ServerMode::Development {
            return Ok(Principal::Development);
        }
        let digest = hex::encode(Sha256::digest(token.as_bytes()));
        if let Some(cached) = self.cache.get(&digest)
            && cached.valid_until > Instant::now()
        {
            return Ok(cached.principal.clone());
        }

        let userinfo = self
            .client
            .get(
                self.userinfo_endpoint
                    .as_ref()
                    .context("OIDC userinfo endpoint is not configured")?
                    .clone(),
            )
            .bearer_auth(token)
            .send()
            .await
            .context("request OIDC userinfo")?
            .error_for_status()
            .context("OIDC bearer token was rejected")?
            .json::<UserInfo>()
            .await
            .context("decode OIDC userinfo response")?;
        if userinfo.sub.trim().is_empty() {
            return Err(anyhow!("OIDC userinfo response omitted sub"));
        }
        let principal = Principal::Oidc {
            issuer: self
                .issuer
                .clone()
                .context("OIDC issuer is not configured")?,
            subject: userinfo.sub,
            email: userinfo.email,
        };
        self.cache.insert(
            digest,
            CachedPrincipal {
                principal: principal.clone(),
                valid_until: Instant::now() + Duration::from_secs(30),
            },
        );
        Ok(principal)
    }

    pub fn issue_websocket_ticket(&self, corp_id: Uuid, actor_id: Uuid) -> (String, u64) {
        let ttl_seconds = 30;
        let ticket = format!(
            "crony_ws_{}{}",
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        );
        self.websocket_tickets.insert(
            hex::encode(Sha256::digest(ticket.as_bytes())),
            WebSocketTicket {
                corp_id,
                actor_id,
                valid_until: Instant::now() + Duration::from_secs(ttl_seconds),
            },
        );
        (ticket, ttl_seconds)
    }

    pub fn consume_websocket_ticket(&self, corp_id: Uuid, ticket: &str) -> Result<Uuid> {
        let digest = hex::encode(Sha256::digest(ticket.as_bytes()));
        let (_, ticket) = self
            .websocket_tickets
            .remove(&digest)
            .context("unknown or already-consumed WebSocket ticket")?;
        if ticket.valid_until <= Instant::now() {
            return Err(anyhow!("WebSocket ticket expired"));
        }
        if ticket.corp_id != corp_id {
            return Err(anyhow!("WebSocket ticket belongs to a different Corp"));
        }
        Ok(ticket.actor_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_matrix_is_fail_closed() {
        assert!(CorpRole::Manager.allows(Permission::EmergencyStop));
        assert!(CorpRole::Manager.allows(Permission::Recover));
        assert!(!CorpRole::Member.allows(Permission::Recover));
        assert!(CorpRole::Owner.allows(Permission::Manage));
        assert!(CorpRole::Manager.allows(Permission::Publish));
        assert!(!CorpRole::Member.allows(Permission::Publish));
        assert!(CorpRole::Member.allows(Permission::Operate));
        assert!(CorpRole::Guest.allows(Permission::PostMessage));
        assert!(!CorpRole::Guest.allows(Permission::Operate));
        assert!(CorpRole::Spectator.allows(Permission::Read));
        assert!(!CorpRole::Spectator.allows(Permission::PostMessage));
        assert!(CorpRole::parse("reviewer").is_err());
    }
}
