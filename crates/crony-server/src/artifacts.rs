use std::{path::PathBuf, str::FromStr, sync::Arc};

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use crony_store::StoredArtifact;
use hmac::{Hmac, Mac};
use object_store::{
    ObjectStore, aws::AmazonS3Builder, local::LocalFileSystem, path::Path as ObjectPath,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const ARTIFACT_VERIFIER: &str = "crony-server:artifact-ingest-v1";

#[derive(Clone)]
pub struct ArtifactStore {
    store: Arc<dyn ObjectStore>,
    signing_key: Arc<Vec<u8>>,
    max_bytes: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct ArtifactIdentity<'a> {
    pub id: Uuid,
    pub corp_id: Uuid,
    pub task_id: Uuid,
    pub run_id: Uuid,
    pub agent_id: Uuid,
    pub runner_id: &'a str,
}

impl ArtifactStore {
    #[allow(clippy::too_many_arguments)]
    pub fn initialize(
        backend: &str,
        local_root: PathBuf,
        endpoint: Option<&str>,
        bucket: Option<&str>,
        region: Option<&str>,
        access_key: Option<&str>,
        secret_key: Option<&str>,
        allow_http: bool,
        signing_key_hex: Option<&str>,
        max_bytes: usize,
        production: bool,
    ) -> Result<Self> {
        let signing_key = match signing_key_hex {
            Some(value) => hex::decode(value).context("decode artifact signing key")?,
            None if !production => vec![0x4c; 32],
            None => return Err(anyhow!("CRONY_ARTIFACT_SIGNING_KEY_HEX is required")),
        };
        if signing_key.len() < 32 {
            return Err(anyhow!(
                "artifact signing key must contain at least 32 bytes"
            ));
        }
        if max_bytes == 0 {
            return Err(anyhow!("artifact byte limit must be positive"));
        }
        if production && backend != "s3" {
            return Err(anyhow!("production requires CRONY_OBJECT_STORE_BACKEND=s3"));
        }
        if production && allow_http {
            return Err(anyhow!(
                "production object storage cannot allow plaintext HTTP"
            ));
        }

        let store: Arc<dyn ObjectStore> = match backend {
            "local" => {
                std::fs::create_dir_all(&local_root)?;
                Arc::new(LocalFileSystem::new_with_prefix(&local_root)?)
            }
            "s3" => {
                let bucket = bucket.context("CRONY_OBJECT_STORE_BUCKET is required for S3")?;
                let mut builder = AmazonS3Builder::new()
                    .with_bucket_name(bucket)
                    .with_region(region.unwrap_or("us-east-1"));
                if let Some(endpoint) = endpoint {
                    builder = builder
                        .with_endpoint(endpoint)
                        .with_virtual_hosted_style_request(false);
                }
                if let Some(access_key) = access_key {
                    builder = builder.with_access_key_id(access_key);
                }
                if let Some(secret_key) = secret_key {
                    builder = builder.with_secret_access_key(secret_key);
                }
                if allow_http {
                    builder = builder.with_allow_http(true);
                }
                Arc::new(builder.build()?)
            }
            other => return Err(anyhow!("unsupported object store backend {other}")),
        };
        Ok(Self {
            store,
            signing_key: Arc::new(signing_key),
            max_bytes,
        })
    }

    pub async fn ingest(
        &self,
        identity: ArtifactIdentity<'_>,
        payload: &Value,
        retention_until: DateTime<Utc>,
    ) -> Result<StoredArtifact> {
        let declared_bytes = payload
            .get("bytes")
            .and_then(Value::as_u64)
            .context("artifact upload omitted bytes")?;
        let declared_bytes =
            usize::try_from(declared_bytes).context("artifact byte count is out of range")?;
        if declared_bytes == 0 || declared_bytes > self.max_bytes {
            return Err(anyhow!(
                "artifact size {declared_bytes} is outside the allowed range 1..={}",
                self.max_bytes
            ));
        }
        let encoded = payload
            .get("content_base64")
            .and_then(Value::as_str)
            .context("artifact upload omitted content")?;
        let maximum_encoded = self.max_bytes.saturating_add(2) / 3 * 4 + 4;
        if encoded.len() > maximum_encoded {
            return Err(anyhow!(
                "encoded artifact exceeds the configured byte limit"
            ));
        }
        let bytes = BASE64
            .decode(encoded)
            .context("artifact content is not valid base64")?;
        if bytes.len() != declared_bytes {
            return Err(anyhow!(
                "artifact byte mismatch: declared {declared_bytes}, received {}",
                bytes.len()
            ));
        }

        let declared_sha = payload
            .get("sha256")
            .and_then(Value::as_str)
            .context("artifact upload omitted sha256")?
            .to_ascii_lowercase();
        if declared_sha.len() != 64
            || !declared_sha
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            return Err(anyhow!("artifact sha256 is invalid"));
        }
        let actual_sha = hex::encode(Sha256::digest(&bytes));
        if actual_sha != declared_sha {
            return Err(anyhow!(
                "artifact digest mismatch: declared {declared_sha}, received {actual_sha}"
            ));
        }

        let media_type = normalize_and_verify_media_type(
            payload
                .get("media_type")
                .and_then(Value::as_str)
                .context("artifact upload omitted media_type")?,
            &bytes,
        )?;
        let object_key = format!(
            "corps/{}/sha256/{}/{}",
            identity.corp_id,
            &actual_sha[..2],
            actual_sha
        );
        self.store
            .put(
                &ObjectPath::from(object_key.clone()),
                Bytes::from(bytes).into(),
            )
            .await
            .context("write artifact object")?;
        let uri = format!("/api/corps/{}/artifacts/{}", identity.corp_id, identity.id);
        let bytes = i64::try_from(declared_bytes).context("artifact size exceeds i64")?;
        let artifact = StoredArtifact {
            id: identity.id,
            corp_id: identity.corp_id,
            task_id: identity.task_id,
            run_id: identity.run_id,
            producer_agent_id: identity.agent_id,
            producer_runner_id: identity.runner_id.to_owned(),
            verifier: ARTIFACT_VERIFIER.to_owned(),
            object_key,
            uri,
            sha256: actual_sha,
            media_type,
            bytes,
            provenance_signature: String::new(),
            retention_until,
        };
        let provenance_signature = self.sign(&artifact)?;
        Ok(StoredArtifact {
            provenance_signature,
            ..artifact
        })
    }

    pub async fn read_verified(&self, artifact: &StoredArtifact) -> Result<Bytes> {
        if artifact.retention_until <= Utc::now() {
            return Err(anyhow!("artifact retention period has expired"));
        }
        self.verify_signature(artifact)?;
        let bytes = self
            .store
            .get(&ObjectPath::from(artifact.object_key.clone()))
            .await
            .context("read artifact object")?
            .bytes()
            .await
            .context("read artifact bytes")?;
        let actual_sha = hex::encode(Sha256::digest(&bytes));
        if actual_sha != artifact.sha256 || bytes.len() as i64 != artifact.bytes {
            return Err(anyhow!("stored artifact failed integrity verification"));
        }
        let verified_media = normalize_and_verify_media_type(&artifact.media_type, &bytes)?;
        if verified_media != artifact.media_type {
            return Err(anyhow!("stored artifact media type changed"));
        }
        Ok(bytes)
    }

    fn sign(&self, artifact: &StoredArtifact) -> Result<String> {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.signing_key)
            .map_err(|_| anyhow!("initialize artifact provenance signer"))?;
        mac.update(provenance_message(artifact).as_bytes());
        Ok(hex::encode(mac.finalize().into_bytes()))
    }

    fn verify_signature(&self, artifact: &StoredArtifact) -> Result<()> {
        let signature =
            hex::decode(&artifact.provenance_signature).context("decode provenance signature")?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.signing_key)
            .map_err(|_| anyhow!("initialize artifact provenance verifier"))?;
        mac.update(provenance_message(artifact).as_bytes());
        mac.verify_slice(&signature)
            .map_err(|_| anyhow!("artifact provenance signature is invalid"))
    }
}

fn provenance_message(artifact: &StoredArtifact) -> String {
    format!(
        "v1|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        artifact.id,
        artifact.corp_id,
        artifact.task_id,
        artifact.run_id,
        artifact.producer_agent_id,
        artifact.producer_runner_id,
        artifact.verifier,
        artifact.object_key,
        artifact.uri,
        artifact.sha256,
        artifact.media_type,
        artifact.bytes,
        artifact.retention_until.timestamp_micros()
    )
}

fn normalize_and_verify_media_type(declared: &str, bytes: &[u8]) -> Result<String> {
    let parsed = mime::Mime::from_str(declared).context("artifact media type is invalid")?;
    let media_type = parsed.essence_str().to_ascii_lowercase();
    match media_type.as_str() {
        value if value == "application/json" || value.ends_with("+json") => {
            serde_json::from_slice::<Value>(bytes)
                .context("artifact declared JSON but content is invalid")?;
        }
        "image/svg+xml" => {
            let text = std::str::from_utf8(bytes)
                .context("artifact declared SVG but content is not UTF-8")?;
            if !text.to_ascii_lowercase().contains("<svg") {
                return Err(anyhow!(
                    "artifact declared SVG but no svg element was found"
                ));
            }
        }
        value if value == "application/xml" || value == "text/xml" || value.ends_with("+xml") => {
            let text = std::str::from_utf8(bytes)
                .context("artifact declared XML but content is not UTF-8")?;
            if !text.trim_start().starts_with('<') {
                return Err(anyhow!("artifact declared XML but content is not markup"));
            }
        }
        value if value.starts_with("text/") => {
            std::str::from_utf8(bytes)
                .context("artifact declared text but content is not UTF-8")?;
        }
        "application/octet-stream" => {}
        value => {
            let detected = detect_binary_media_type(bytes).ok_or_else(|| {
                anyhow!("artifact media type {value} cannot be verified from its content")
            })?;
            if detected != value && !(detected == "application/zip" && is_zip_container(value)) {
                return Err(anyhow!(
                    "artifact media type mismatch: declared {value}, detected {detected}"
                ));
            }
        }
    }
    Ok(media_type)
}

fn detect_binary_media_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"%PDF-") {
        Some("application/pdf")
    } else if bytes.starts_with(b"PK\x03\x04") {
        Some("application/zip")
    } else if bytes.starts_with(b"\x1f\x8b") {
        Some("application/gzip")
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WAVE" {
        Some("audio/wav")
    } else if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        Some("video/mp4")
    } else {
        None
    }
}

fn is_zip_container(media_type: &str) -> bool {
    media_type == "application/epub+zip"
        || media_type == "application/java-archive"
        || media_type.starts_with("application/vnd.openxmlformats-officedocument.")
        || media_type.starts_with("application/vnd.oasis.opendocument.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn identity() -> ArtifactIdentity<'static> {
        ArtifactIdentity {
            id: Uuid::new_v4(),
            corp_id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            run_id: Uuid::new_v4(),
            agent_id: Uuid::new_v4(),
            runner_id: "runner-test",
        }
    }

    #[tokio::test]
    async fn local_store_round_trips_verified_content_and_provenance() {
        let root = std::env::temp_dir()
            .join("crony-artifact-store-tests")
            .join(Uuid::new_v4().to_string());
        let store = ArtifactStore::initialize(
            "local",
            root.clone(),
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            1_024,
            false,
        )
        .expect("initialize artifact store");
        let content = br#"{"verified":true}"#;
        let artifact = store
            .ingest(
                identity(),
                &json!({
                    "sha256": hex::encode(Sha256::digest(content)),
                    "bytes": content.len(),
                    "media_type": "application/json; charset=utf-8",
                    "content_base64": BASE64.encode(content),
                }),
                Utc::now() + chrono::Duration::days(30),
            )
            .await
            .expect("ingest artifact");
        assert_eq!(artifact.media_type, "application/json");
        assert_eq!(
            store.read_verified(&artifact).await.expect("read artifact"),
            Bytes::from_static(content)
        );

        let mut tampered = artifact.clone();
        tampered.bytes += 1;
        assert!(store.read_verified(&tampered).await.is_err());
        std::fs::remove_dir_all(root).expect("remove artifact test directory");
    }

    #[test]
    fn media_validation_rejects_mismatches() {
        assert!(normalize_and_verify_media_type("text/plain", b"hello").is_ok());
        assert!(normalize_and_verify_media_type("application/json", br#"{"ok":true}"#).is_ok());
        assert!(normalize_and_verify_media_type("application/json", b"not json").is_err());
        assert!(normalize_and_verify_media_type("image/png", b"not png").is_err());
    }

    #[test]
    fn production_requires_private_s3_storage() {
        let result = ArtifactStore::initialize(
            "local",
            std::env::temp_dir(),
            None,
            None,
            None,
            None,
            None,
            false,
            Some(&"4c".repeat(32)),
            1_024,
            true,
        );
        assert!(result.is_err());
    }
}
