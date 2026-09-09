use std::{path::PathBuf, str::FromStr, sync::Arc};

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use crony_domain::{
    RETAINED_COPILOT_RECEIPT_FILE, RetainedProviderReceiptGrant, repository_relative_path_is_valid,
    retained_provider_receipt_metadata, validate_retained_copilot_receipt,
};
use crony_store::StoredArtifact;
use futures_util::TryStreamExt;
use hmac::{Hmac, Mac};
use object_store::{
    GetOptions, GetRange, GetResult, ObjectStore, aws::AmazonS3Builder, local::LocalFileSystem,
    path::Path as ObjectPath,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;
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

#[derive(Debug, Clone)]
pub struct StagedArtifact {
    pub artifact: StoredArtifact,
    pub staging_key: String,
    pub bytes: Bytes,
}

#[derive(Debug, Error)]
enum PermanentArtifactError {
    #[error("artifact provenance metadata is invalid: {0}")]
    InvalidProvenance(String),
    #[error("staged and final artifact objects are both unavailable")]
    MissingObjects,
    #[error("{0} failed integrity verification")]
    Integrity(String),
    #[error("{0} media validation failed: {1}")]
    Media(String, String),
    #[error("{0} media type changed")]
    MediaTypeChanged(String),
}

pub fn artifact_error_is_permanent(error: &anyhow::Error) -> bool {
    error.downcast_ref::<PermanentArtifactError>().is_some()
}

pub fn artifact_error_is_missing_objects(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<PermanentArtifactError>(),
        Some(PermanentArtifactError::MissingObjects)
    )
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

    #[cfg(test)]
    pub async fn ingest(
        &self,
        identity: ArtifactIdentity<'_>,
        payload: &Value,
        retention_until: DateTime<Utc>,
    ) -> Result<StoredArtifact> {
        let staged = self.prepare_staging(identity, payload, retention_until)?;
        self.write_staged(&staged).await?;
        self.finalize_staged(&staged.artifact, &staged.staging_key)
            .await?;
        self.discard_staged(&staged.staging_key).await?;
        Ok(staged.artifact)
    }

    pub fn prepare_staging(
        &self,
        identity: ArtifactIdentity<'_>,
        payload: &Value,
        retention_until: DateTime<Utc>,
    ) -> Result<StagedArtifact> {
        self.prepare_staging_with_receipt(identity, payload, retention_until, None)
    }

    /// Only the server's current store-authorized collection path may sign a
    /// historical receipt. Preparing bytes performs no object-store writes.
    pub fn prepare_retained_provider_receipt_staging(
        &self,
        identity: ArtifactIdentity<'_>,
        payload: &Value,
        retention_until: DateTime<Utc>,
        grant: &RetainedProviderReceiptGrant,
    ) -> Result<StagedArtifact> {
        self.prepare_staging_with_receipt(identity, payload, retention_until, Some(grant))
    }

    fn prepare_staging_with_receipt(
        &self,
        identity: ArtifactIdentity<'_>,
        payload: &Value,
        retention_until: DateTime<Utc>,
        retained_receipt: Option<&RetainedProviderReceiptGrant>,
    ) -> Result<StagedArtifact> {
        if payload.get("retained_provider_receipt").is_some() && retained_receipt.is_none() {
            return Err(anyhow!(
                "retained provider receipt requires current collection authority"
            ));
        }
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
        let artifact_role = payload
            .get("artifact_role")
            .and_then(Value::as_str)
            .unwrap_or("provider_evidence");
        if !matches!(artifact_role, "provider_evidence" | "source_deliverable") {
            return Err(anyhow!("artifact role is invalid"));
        }
        let file_name = payload
            .get("file_name")
            .and_then(Value::as_str)
            .unwrap_or("artifact.bin");
        if !valid_artifact_file_name(file_name) {
            return Err(anyhow!("artifact file name is invalid"));
        }
        let metadata = if let Some(grant) = retained_receipt {
            validate_retained_copilot_receipt(&bytes, grant).map_err(anyhow::Error::msg)?;
            let metadata = retained_provider_receipt_metadata(grant);
            if identity.corp_id != grant.corp_id
                || identity.task_id != grant.task_id
                || identity.run_id != grant.run_id
                || artifact_role != "provider_evidence"
                || payload.get("artifact_role").and_then(Value::as_str) != Some("provider_evidence")
                || file_name != RETAINED_COPILOT_RECEIPT_FILE
                || media_type != "application/json"
                || payload.get("workspace_relative_path") != metadata.get("workspace_relative_path")
                || payload.get("retained_provider_receipt")
                    != metadata.get("retained_provider_receipt")
            {
                return Err(anyhow!(
                    "retained provider receipt upload does not match its authorized collection"
                ));
            }
            metadata
        } else if artifact_role == "source_deliverable" {
            source_deliverable_metadata(payload)?
        } else if let Some(path) = payload
            .get("workspace_relative_path")
            .and_then(Value::as_str)
        {
            if !repository_relative_path_is_valid(path) {
                return Err(anyhow!(
                    "provider artifact workspace-relative path is invalid"
                ));
            }
            json!({"workspace_relative_path": path})
        } else {
            json!({})
        };
        let object_key = format!(
            "corps/{}/sha256/{}/{}",
            identity.corp_id,
            &actual_sha[..2],
            actual_sha
        );
        let staging_key = format!("staging/corps/{}/{}", identity.corp_id, identity.id);
        let uri = format!("/api/corps/{}/artifacts/{}", identity.corp_id, identity.id);
        let byte_count = i64::try_from(declared_bytes).context("artifact size exceeds i64")?;
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
            bytes: byte_count,
            artifact_role: artifact_role.to_owned(),
            file_name: file_name.to_owned(),
            metadata,
            provenance_signature: String::new(),
            retention_until,
        };
        let provenance_signature = self.sign(&artifact)?;
        Ok(StagedArtifact {
            artifact: StoredArtifact {
                provenance_signature,
                ..artifact
            },
            staging_key,
            bytes: Bytes::from(bytes),
        })
    }

    pub async fn write_staged(&self, staged: &StagedArtifact) -> Result<()> {
        verify_artifact_bytes(&staged.artifact, &staged.bytes, "prepared artifact")?;
        self.store
            .put(
                &ObjectPath::from(staged.staging_key.clone()),
                staged.bytes.clone().into(),
            )
            .await
            .context("write staged artifact object")?;
        Ok(())
    }

    pub async fn finalize_staged(
        &self,
        artifact: &StoredArtifact,
        staging_key: &str,
    ) -> Result<()> {
        self.verify_signature(artifact).map_err(|error| {
            anyhow!(PermanentArtifactError::InvalidProvenance(error.to_string()))
        })?;
        let bytes = match self.store.get(&ObjectPath::from(staging_key)).await {
            Ok(result) => result.bytes().await.context("read staged artifact bytes")?,
            Err(object_store::Error::NotFound { .. }) => {
                let final_object = match self
                    .store
                    .get(&ObjectPath::from(artifact.object_key.clone()))
                    .await
                {
                    Ok(final_object) => final_object,
                    Err(object_store::Error::NotFound { .. }) => {
                        return Err(anyhow!(PermanentArtifactError::MissingObjects));
                    }
                    Err(error) => {
                        return Err(error).context("read final artifact object during recovery");
                    }
                };
                let bytes = final_object
                    .bytes()
                    .await
                    .context("read final artifact bytes during recovery")?;
                verify_artifact_bytes(artifact, &bytes, "final artifact")?;
                return Ok(());
            }
            Err(error) => return Err(error).context("read staged artifact object"),
        };
        verify_artifact_bytes(artifact, &bytes, "staged artifact")?;
        self.store
            .put(&ObjectPath::from(artifact.object_key.clone()), bytes.into())
            .await
            .context("publish final artifact object")?;
        Ok(())
    }

    pub async fn discard_staged(&self, staging_key: &str) -> Result<()> {
        match self.store.delete(&ObjectPath::from(staging_key)).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(error) => Err(error).context("delete staged artifact object"),
        }
    }

    pub async fn staged_keys(&self) -> Result<Vec<String>> {
        let prefix = ObjectPath::from("staging");
        let mut objects = self.store.list(Some(&prefix));
        let mut keys = Vec::new();
        while let Some(object) = objects
            .try_next()
            .await
            .context("list staged artifact objects")?
        {
            let key = object.location.to_string();
            if is_staging_key(&key) {
                keys.push(key);
            }
        }
        keys.sort();
        Ok(keys)
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
        verify_artifact_bytes(artifact, &bytes, "stored artifact")?;
        Ok(bytes)
    }

    pub async fn read_verified_bounded(
        &self,
        artifact: &StoredArtifact,
        max_bytes: usize,
    ) -> Result<Bytes> {
        if artifact.retention_until <= Utc::now() {
            return Err(anyhow!("artifact retention period has expired"));
        }
        self.verify_signature(artifact)?;
        let expected_bytes =
            usize::try_from(artifact.bytes).context("artifact byte count is out of range")?;
        if expected_bytes == 0 || expected_bytes > max_bytes {
            return Err(anyhow!(
                "artifact size {expected_bytes} is outside the bounded range 1..={max_bytes}"
            ));
        }
        // Ask the native store for one extra byte so a longer object cannot be
        // accepted as a valid prefix, even if its returned size is understated.
        let range_end = u64::try_from(expected_bytes)?
            .checked_add(1)
            .context("artifact read range is out of bounds")?;
        let result = self
            .store
            .get_opts(
                &ObjectPath::from(artifact.object_key.clone()),
                GetOptions {
                    range: Some(GetRange::Bounded(0..range_end)),
                    ..Default::default()
                },
            )
            .await
            .context("read bounded artifact object")?;
        let bytes = collect_bounded_artifact_bytes(result, expected_bytes).await?;
        if artifact.retention_until <= Utc::now() {
            return Err(anyhow!("artifact retention period has expired"));
        }
        verify_artifact_bytes(artifact, &bytes, "stored artifact")?;
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

#[cfg(test)]
#[path = "artifact_receipt_tests.rs"]
mod artifact_receipt_tests;

async fn collect_bounded_artifact_bytes(result: GetResult, expected_bytes: usize) -> Result<Bytes> {
    let expected_size = u64::try_from(expected_bytes)?;
    if result.meta.size != expected_size || result.range != (0..expected_size) {
        return Err(anyhow!(PermanentArtifactError::Integrity(
            "stored artifact".to_owned()
        )));
    }
    let mut stream = result.into_stream();
    let mut bytes = Vec::with_capacity(expected_bytes);
    while let Some(chunk) = stream
        .try_next()
        .await
        .context("read bounded artifact bytes")?
    {
        // Do not trust response metadata or grow the buffer beyond the signed
        // length. Keep polling at the exact limit to reject trailing bytes.
        if chunk.len() > expected_bytes - bytes.len() {
            return Err(anyhow!(PermanentArtifactError::Integrity(
                "stored artifact".to_owned()
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.len() != expected_bytes {
        return Err(anyhow!(PermanentArtifactError::Integrity(
            "stored artifact".to_owned()
        )));
    }
    Ok(bytes.into())
}

fn verify_artifact_bytes(artifact: &StoredArtifact, bytes: &[u8], label: &str) -> Result<()> {
    let actual_sha = hex::encode(Sha256::digest(bytes));
    if actual_sha != artifact.sha256 || bytes.len() as i64 != artifact.bytes {
        return Err(anyhow!(PermanentArtifactError::Integrity(label.to_owned())));
    }
    let verified_media =
        normalize_and_verify_media_type(&artifact.media_type, bytes).map_err(|error| {
            anyhow!(PermanentArtifactError::Media(
                label.to_owned(),
                error.to_string(),
            ))
        })?;
    if verified_media != artifact.media_type {
        return Err(anyhow!(PermanentArtifactError::MediaTypeChanged(
            label.to_owned()
        )));
    }
    Ok(())
}

fn is_staging_key(key: &str) -> bool {
    let parts = key.split('/').collect::<Vec<_>>();
    parts.len() == 4
        && parts[0] == "staging"
        && parts[1] == "corps"
        && Uuid::parse_str(parts[2]).is_ok()
        && Uuid::parse_str(parts[3]).is_ok()
}

fn provenance_message(artifact: &StoredArtifact) -> String {
    if artifact.artifact_role == "provider_evidence"
        && artifact.file_name == "artifact.bin"
        && artifact.metadata == json!({})
    {
        return format!(
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
        );
    }
    format!(
        "v2|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
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
        artifact.artifact_role,
        artifact.file_name,
        hex::encode(Sha256::digest(
            serde_json::to_vec(&artifact.metadata).expect("artifact metadata serializes")
        )),
        artifact.retention_until.timestamp_micros()
    )
}

fn source_deliverable_metadata(payload: &Value) -> Result<Value> {
    let required = |name: &str| {
        payload
            .get(name)
            .and_then(Value::as_str)
            .with_context(|| format!("source deliverable omitted {name}"))
    };
    let form = required("form")?;
    if !matches!(
        form,
        "commit_branch" | "patch" | "archive" | "typed_artifact_set" | "review_only_report"
    ) {
        return Err(anyhow!("source deliverable form is invalid"));
    }
    let verification_sha256 = required("verification_sha256")?;
    let base_commit = required("base_commit")?;
    let head_commit = payload.get("head_commit").and_then(Value::as_str);
    let branch = required("branch")?;
    let integration_state = required("integration_state")?;
    let git_bundle_sha256 = payload.get("git_bundle_sha256").and_then(Value::as_str);
    let publication_ready = payload
        .get("publication_ready")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !valid_hex(verification_sha256, 64, 64)
        || !valid_hex(base_commit, 40, 64)
        || head_commit.is_some_and(|value| !valid_hex(value, 40, 64))
        || branch.is_empty()
        || branch.len() > 512
        || !matches!(
            integration_state,
            "not_applicable" | "ready_for_review" | "published" | "integrated"
        )
    {
        return Err(anyhow!("source deliverable metadata is invalid"));
    }
    if form == "commit_branch" {
        if head_commit.is_none()
            || git_bundle_sha256.is_none_or(|value| !valid_hex(value, 64, 64))
            || !publication_ready
        {
            return Err(anyhow!(
                "commit/branch deliverable omitted its portable publication bundle"
            ));
        }
    } else if git_bundle_sha256.is_some() || publication_ready {
        return Err(anyhow!(
            "only commit/branch deliverables may be publication-ready"
        ));
    }
    Ok(json!({
        "form": form,
        "verification_sha256": verification_sha256,
        "base_commit": base_commit,
        "head_commit": head_commit,
        "branch": branch,
        "integration_state": integration_state,
        "git_bundle_sha256": git_bundle_sha256,
        "publication_ready": publication_ready,
    }))
}

fn valid_hex(value: &str, minimum: usize, maximum: usize) -> bool {
    (minimum..=maximum).contains(&value.len())
        && value.chars().all(|character| character.is_ascii_hexdigit())
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

fn valid_artifact_file_name(file_name: &str) -> bool {
    !file_name.is_empty()
        && file_name.len() <= 255
        && !file_name.contains(['/', '\\', '"'])
        && !file_name.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use futures_util::{StreamExt, stream};
    use object_store::{GetResultPayload, ObjectMeta, memory::InMemory};
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

    fn memory_store() -> ArtifactStore {
        ArtifactStore {
            store: Arc::new(InMemory::new()),
            signing_key: Arc::new(vec![0x4c; 32]),
            max_bytes: 1_024,
        }
    }

    async fn small_signed_artifact(store: &ArtifactStore) -> StoredArtifact {
        let content = br#"{"verified":true}"#;
        store
            .ingest(
                identity(),
                &json!({
                    "sha256": hex::encode(Sha256::digest(content)),
                    "bytes": content.len(),
                    "media_type": "application/json",
                    "content_base64": BASE64.encode(content),
                }),
                Utc::now() + chrono::Duration::days(30),
            )
            .await
            .expect("ingest small signed artifact")
    }

    fn streamed_result(
        size: u64,
        range: std::ops::Range<u64>,
        chunk_sizes: &'static [usize],
        polled: Arc<AtomicUsize>,
    ) -> GetResult {
        let chunks = stream::iter(chunk_sizes.iter().copied())
            .map(move |size| {
                polled.fetch_add(1, Ordering::SeqCst);
                Ok(Bytes::from(vec![b'x'; size]))
            })
            .boxed();
        GetResult {
            payload: GetResultPayload::Stream(chunks),
            meta: ObjectMeta {
                location: ObjectPath::from("bounded-artifact-test"),
                last_modified: Utc::now(),
                size,
                e_tag: None,
                version: None,
            },
            range,
            attributes: Default::default(),
        }
    }

    #[tokio::test]
    async fn bounded_verified_read_round_trips_small_artifact_without_limiting_downloads() {
        let store = memory_store();
        let artifact = small_signed_artifact(&store).await;
        let content = Bytes::from_static(br#"{"verified":true}"#);
        assert_eq!(
            store
                .read_verified_bounded(&artifact, content.len())
                .await
                .expect("read artifact at the exact transfer limit"),
            content
        );
        assert!(
            store
                .read_verified_bounded(&artifact, content.len() - 1)
                .await
                .is_err()
        );
        assert_eq!(
            store
                .read_verified(&artifact)
                .await
                .expect("ordinary download is not constrained by a transfer limit"),
            content
        );
    }

    #[tokio::test]
    async fn bounded_verified_read_rejects_oversized_object_with_small_signed_metadata() {
        let store = memory_store();
        let artifact = small_signed_artifact(&store).await;
        let limit = crony_protocol::MAX_VERIFICATION_ARTIFACT_BYTES;
        store
            .store
            .put(
                &ObjectPath::from(artifact.object_key.clone()),
                Bytes::from(vec![b'x'; limit + 1]).into(),
            )
            .await
            .expect("replace stored object without changing signed metadata");
        store
            .verify_signature(&artifact)
            .expect("small metadata still has its valid signature");
        let error = store
            .read_verified_bounded(&artifact, limit)
            .await
            .expect_err("oversized backing object must not be buffered");
        assert!(artifact_error_is_permanent(&error));
    }

    #[tokio::test]
    async fn bounded_verified_read_preserves_signature_retention_digest_and_media_checks() {
        let store = memory_store();
        let artifact = small_signed_artifact(&store).await;
        let mut tampered = artifact.clone();
        tampered.bytes += 1;
        assert!(
            store
                .read_verified_bounded(&tampered, 1_024)
                .await
                .expect_err("reject invalid provenance")
                .to_string()
                .contains("signature")
        );

        let mut expired = artifact.clone();
        expired.retention_until = Utc::now() - chrono::Duration::seconds(1);
        expired.provenance_signature = store.sign(&expired).expect("sign expired metadata");
        assert!(
            store
                .read_verified_bounded(&expired, 1_024)
                .await
                .expect_err("reject expired artifact")
                .to_string()
                .contains("retention")
        );

        let mut wrong_media = artifact.clone();
        wrong_media.media_type = "image/png".to_owned();
        wrong_media.provenance_signature = store.sign(&wrong_media).expect("sign media metadata");
        let error = store
            .read_verified_bounded(&wrong_media, 1_024)
            .await
            .expect_err("verify media after bounded collection");
        assert!(matches!(
            error.downcast_ref::<PermanentArtifactError>(),
            Some(PermanentArtifactError::Media(_, _))
        ));

        store
            .store
            .put(
                &ObjectPath::from(artifact.object_key.clone()),
                Bytes::from_static(br#"{"verified":null}"#).into(),
            )
            .await
            .expect("replace content with equal-length bytes");
        let error = store
            .read_verified_bounded(&artifact, 1_024)
            .await
            .expect_err("verify digest after bounded collection");
        assert!(artifact_error_is_permanent(&error));
    }

    #[tokio::test]
    async fn bounded_artifact_stream_rejects_size_or_range_mismatch_before_polling() {
        for (size, range) in [(u64::MAX, 0..4), (4, 0..u64::MAX), (4, 1..4)] {
            let polled = Arc::new(AtomicUsize::new(0));
            let result = streamed_result(size, range, &[4], polled.clone());
            let error = collect_bounded_artifact_bytes(result, 4)
                .await
                .expect_err("reject unbounded metadata without consuming the body");
            assert!(artifact_error_is_permanent(&error));
            assert_eq!(polled.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn bounded_artifact_stream_rejects_understated_body_without_draining() {
        for (chunks, expected_polls) in [
            (&[5, 1_024][..], 1),
            (&[2, 2, 1, 1_024][..], 3),
            (&[4, 1_024, 1_024][..], 2),
        ] {
            let polled = Arc::new(AtomicUsize::new(0));
            // Both response size and range lie; only counting the actual chunks
            // catches the excess, including one byte after an exact-size prefix.
            let result = streamed_result(4, 0..4, chunks, polled.clone());
            let error = collect_bounded_artifact_bytes(result, 4)
                .await
                .expect_err("reject overflow before growing the collection buffer");
            assert!(artifact_error_is_permanent(&error));
            assert_eq!(polled.load(Ordering::SeqCst), expected_polls);
        }
    }

    #[tokio::test]
    async fn bounded_artifact_stream_requires_exact_length_and_eof() {
        let polled = Arc::new(AtomicUsize::new(0));
        let result = streamed_result(4, 0..4, &[2, 0, 2], polled.clone());
        assert_eq!(
            collect_bounded_artifact_bytes(result, 4)
                .await
                .expect("accept complete bounded stream"),
            Bytes::from_static(b"xxxx")
        );
        assert_eq!(polled.load(Ordering::SeqCst), 3);

        let result = streamed_result(4, 0..4, &[3], polled);
        assert!(collect_bounded_artifact_bytes(result, 4).await.is_err());
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
        assert_eq!(
            store
                .read_verified_bounded(&artifact, content.len())
                .await
                .expect("stream local artifact at its exact bound"),
            Bytes::from_static(content)
        );

        let mut tampered = artifact.clone();
        tampered.bytes += 1;
        assert!(store.read_verified(&tampered).await.is_err());
        std::fs::remove_dir_all(root).expect("remove artifact test directory");
    }

    #[tokio::test]
    async fn source_deliverable_role_and_exact_linkage_are_signed() {
        let root = std::env::temp_dir()
            .join("crony-source-deliverable-tests")
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
            4_096,
            false,
        )
        .expect("initialize artifact store");
        let content = br#"{"form":"archive"}"#;
        let artifact = store
            .ingest(
                identity(),
                &json!({
                    "sha256": hex::encode(Sha256::digest(content)),
                    "bytes": content.len(),
                    "media_type": "application/vnd.ecorp.deliverable+json",
                    "content_base64": BASE64.encode(content),
                    "artifact_role": "source_deliverable",
                    "file_name": "ecorp-source-archive.json",
                    "form": "archive",
                    "verification_sha256": "a".repeat(64),
                    "base_commit": "b".repeat(40),
                    "head_commit": Value::Null,
                    "branch": "crony/task-test/run-test",
                    "integration_state": "ready_for_review",
                    "git_bundle_sha256": Value::Null,
                    "publication_ready": false,
                }),
                Utc::now() + chrono::Duration::days(30),
            )
            .await
            .expect("ingest source deliverable");
        assert_eq!(artifact.artifact_role, "source_deliverable");
        assert_eq!(artifact.file_name, "ecorp-source-archive.json");
        assert_eq!(
            artifact.metadata["verification_sha256"].as_str(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(
            artifact.metadata["publication_ready"].as_bool(),
            Some(false)
        );
        let mut tampered = artifact.clone();
        tampered.metadata["verification_sha256"] = json!("c".repeat(64));
        assert!(store.read_verified(&tampered).await.is_err());
        std::fs::remove_dir_all(root).expect("remove source deliverable test directory");
    }

    #[tokio::test]
    async fn staged_objects_finalize_idempotently_and_cleanup_safely() {
        let root = std::env::temp_dir()
            .join("crony-artifact-staging-tests")
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
        let content = br#"{"staged":true}"#;
        let staged = store
            .prepare_staging(
                identity(),
                &json!({
                    "sha256": hex::encode(Sha256::digest(content)),
                    "bytes": content.len(),
                    "media_type": "application/json",
                    "content_base64": BASE64.encode(content),
                }),
                Utc::now() + chrono::Duration::days(30),
            )
            .expect("prepare artifact staging");
        store.write_staged(&staged).await.expect("stage artifact");
        assert!(store.read_verified(&staged.artifact).await.is_err());
        store
            .finalize_staged(&staged.artifact, &staged.staging_key)
            .await
            .expect("finalize staged artifact");
        store
            .discard_staged(&staged.staging_key)
            .await
            .expect("discard staging");
        store
            .finalize_staged(&staged.artifact, &staged.staging_key)
            .await
            .expect("recover finalization after staging cleanup");
        store
            .discard_staged(&staged.staging_key)
            .await
            .expect("repeat staging cleanup");
        assert!(store.staged_keys().await.expect("list staging").is_empty());
        assert_eq!(
            store
                .read_verified(&staged.artifact)
                .await
                .expect("read final artifact"),
            Bytes::from_static(content)
        );

        let duplicate = store
            .prepare_staging(
                ArtifactIdentity {
                    id: Uuid::new_v4(),
                    ..identity()
                },
                &json!({
                    "sha256": hex::encode(Sha256::digest(content)),
                    "bytes": content.len(),
                    "media_type": "application/json",
                    "content_base64": BASE64.encode(content),
                }),
                Utc::now() + chrono::Duration::days(30),
            )
            .expect("prepare duplicate digest");
        store
            .write_staged(&duplicate)
            .await
            .expect("stage duplicate digest");
        assert_eq!(
            store.staged_keys().await.expect("list duplicate staging"),
            vec![duplicate.staging_key.clone()]
        );
        store
            .discard_staged(&duplicate.staging_key)
            .await
            .expect("discard rejected duplicate staging");
        assert_eq!(
            store
                .read_verified(&staged.artifact)
                .await
                .expect("accepted shared digest remains"),
            Bytes::from_static(content)
        );
        std::fs::remove_dir_all(root).expect("remove artifact staging test directory");
    }

    #[test]
    fn media_validation_rejects_mismatches() {
        assert!(normalize_and_verify_media_type("text/plain", b"hello").is_ok());
        assert!(normalize_and_verify_media_type("application/json", br#"{"ok":true}"#).is_ok());
        assert!(normalize_and_verify_media_type("application/json", b"not json").is_err());
        assert!(normalize_and_verify_media_type("image/png", b"not png").is_err());
    }

    #[test]
    fn artifact_file_names_reject_content_disposition_injection() {
        assert!(valid_artifact_file_name("report 2026.txt"));
        assert!(!valid_artifact_file_name(
            "safe.txt\"; filename=\"payload.html"
        ));
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
