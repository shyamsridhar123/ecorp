//! A new collection attestation for a historical native-adapter receipt.
//! This is not a provider resume, a backdated artifact, or proof of completion.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const RETAINED_COPILOT_RECEIPT_FILE: &str = "copilot-evidence.json";
pub const RETAINED_COPILOT_RECEIPT_KIND: &str = "historical_copilot_receipt_v1";
pub const MAX_RETAINED_PROVIDER_RECEIPT_BYTES: usize = 16_777_216;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetainedProviderReceiptGrant {
    pub schema_version: u32,
    /// The durable verifier command, not the historical run or artifact event.
    pub collection_id: Uuid,
    pub corp_id: Uuid,
    pub task_id: Uuid,
    pub run_id: Uuid,
    pub workspace_run_id: Uuid,
    /// The exact source selected for this new collection/recovery.
    pub source_run_id: Uuid,
    /// The measured budget-checkpoint origin, which may differ from the source.
    pub checkpoint_run_id: Uuid,
    pub provider_session_id: String,
    pub historical_run_id: Uuid,
    pub historical_termination_event_id: Uuid,
    pub historical_checkpoint_event_id: Uuid,
    /// The selected source's latest native preservation event.
    pub source_checkpoint_event_id: Uuid,
    pub expected_workspace_fingerprint: String,
    pub expected_head_commit: String,
    pub historical_model: Option<String>,
    pub historical_reasoning_effort: Option<String>,
}

impl RetainedProviderReceiptGrant {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != 1
            || [
                self.collection_id,
                self.corp_id,
                self.task_id,
                self.run_id,
                self.workspace_run_id,
                self.source_run_id,
                self.checkpoint_run_id,
                self.historical_run_id,
                self.historical_termination_event_id,
                self.historical_checkpoint_event_id,
                self.source_checkpoint_event_id,
            ]
            .iter()
            .any(Uuid::is_nil)
            || self.run_id == self.source_run_id
            || self.run_id == self.checkpoint_run_id
            || self.run_id == self.historical_run_id
            || !Uuid::parse_str(&self.provider_session_id).is_ok_and(|id| !id.is_nil())
            || !hex_digest(&self.expected_workspace_fingerprint, &[64])
            || !hex_digest(&self.expected_head_commit, &[40, 64])
            || self
                .historical_model
                .as_ref()
                .is_some_and(|value| !bounded_text(value, 200))
            || self
                .historical_reasoning_effort
                .as_ref()
                .is_some_and(|value| !bounded_text(value, 80))
        {
            return Err("invalid retained-provider-receipt collection scope");
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CopilotReceipt {
    provider: String,
    sdk_version: String,
    session_id: String,
    model: String,
    reasoning_effort: Option<String>,
    discovered_model_count: u64,
    assistant_messages: u64,
    tool_calls: u64,
    changed_paths: Vec<String>,
    diff_sha256: Option<String>,
}

fn bounded_text(value: &str, limit: usize) -> bool {
    !value.is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}

fn hex_digest(value: &str, lengths: &[usize]) -> bool {
    lengths.contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Check the native adapter's existing receipt format; never synthesize or
/// rewrite its contents. Counts remain historical reported observations.
pub fn validate_retained_copilot_receipt(
    bytes: &[u8],
    grant: &RetainedProviderReceiptGrant,
) -> Result<(), &'static str> {
    grant.validate()?;
    if bytes.is_empty() || bytes.len() > MAX_RETAINED_PROVIDER_RECEIPT_BYTES {
        return Err("retained native receipt exceeds its byte boundary");
    }
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| "retained native receipt is not valid JSON")?;
    for field in [
        "provider",
        "sdk_version",
        "session_id",
        "model",
        "reasoning_effort",
        "discovered_model_count",
        "assistant_messages",
        "tool_calls",
        "changed_paths",
        "diff_sha256",
    ] {
        if value.get(field).is_none() {
            return Err("retained native receipt has an incomplete native format");
        }
    }
    // Deserialize the original bytes, not the Value: Value normalizes duplicate
    // keys, but an ambiguous native receipt must never acquire our attestation.
    let receipt: CopilotReceipt = serde_json::from_slice(bytes)
        .map_err(|_| "retained native receipt does not match the native adapter format")?;
    if receipt.provider != "github-copilot"
        || receipt.sdk_version != "1.0.11"
        || receipt.session_id != grant.provider_session_id
        || receipt.model
            != grant
                .historical_model
                .as_deref()
                .unwrap_or("copilot-default")
        || receipt.reasoning_effort != grant.historical_reasoning_effort
        || receipt.changed_paths.len() > 20_000
        || receipt.changed_paths.iter().any(|path| {
            !bounded_text(path, 4096)
                || path.starts_with(['/', '\\'])
                || path.as_bytes().get(1) == Some(&b':')
                || path.split(['/', '\\']).any(|part| part == "..")
        })
        || receipt
            .diff_sha256
            .as_ref()
            .is_some_and(|digest| !hex_digest(digest, &[64]))
        || [
            receipt.discovered_model_count,
            receipt.assistant_messages,
            receipt.tool_calls,
        ]
        .iter()
        .any(|count| *count > i64::MAX as u64)
    {
        return Err("retained native receipt does not match its authorized historical context");
    }
    Ok(())
}

/// Metadata describes a collection performed now against the current sealed
/// source, not a prior server acceptance or a new provider completion.
pub fn retained_provider_receipt_metadata(
    grant: &RetainedProviderReceiptGrant,
) -> serde_json::Value {
    serde_json::json!({
        "workspace_relative_path": RETAINED_COPILOT_RECEIPT_FILE,
        "retained_provider_receipt": {
            "kind": RETAINED_COPILOT_RECEIPT_KIND,
            "grant": grant,
            "attestation": "collected_now_from_current_sealed_workspace",
            "receipt_generation": "historical",
            "original_server_artifact_acceptance_claimed": false,
            "historical_digest_attestation_claimed": false,
            "provider_completion_claimed": false,
            "provider_inference_started": false
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn grant() -> RetainedProviderReceiptGrant {
        RetainedProviderReceiptGrant {
            schema_version: 1,
            collection_id: Uuid::from_u128(1),
            corp_id: Uuid::from_u128(2),
            task_id: Uuid::from_u128(3),
            run_id: Uuid::from_u128(4),
            workspace_run_id: Uuid::from_u128(5),
            source_run_id: Uuid::from_u128(6),
            checkpoint_run_id: Uuid::from_u128(6),
            provider_session_id: Uuid::from_u128(7).to_string(),
            historical_run_id: Uuid::from_u128(5),
            historical_termination_event_id: Uuid::from_u128(8),
            historical_checkpoint_event_id: Uuid::from_u128(9),
            source_checkpoint_event_id: Uuid::from_u128(10),
            expected_workspace_fingerprint: "a".repeat(64),
            expected_head_commit: "b".repeat(40),
            historical_model: Some("test-model".to_owned()),
            historical_reasoning_effort: None,
        }
    }

    fn receipt() -> serde_json::Value {
        json!({
            "provider":"github-copilot", "sdk_version":"1.0.11",
            "session_id":Uuid::from_u128(7), "model":"test-model",
            "reasoning_effort":null, "discovered_model_count":26,
            "assistant_messages":18, "tool_calls":22,
            "changed_paths":["scenarios/example/server.mjs"],
            "diff_sha256":"c".repeat(64)
        })
    }

    #[test]
    fn issue211_retained_receipt_validates_exact_bytes_without_rewriting() {
        let bytes = serde_json::to_vec_pretty(&receipt()).unwrap();
        let before = bytes.clone();
        validate_retained_copilot_receipt(&bytes, &grant()).unwrap();
        assert_eq!(bytes, before);
        let metadata = retained_provider_receipt_metadata(&grant());
        assert_eq!(
            metadata["workspace_relative_path"],
            RETAINED_COPILOT_RECEIPT_FILE
        );
        let provenance = &metadata["retained_provider_receipt"];
        assert_eq!(provenance["receipt_generation"], "historical");
        assert_eq!(
            provenance["original_server_artifact_acceptance_claimed"],
            false
        );
        assert_eq!(provenance["historical_digest_attestation_claimed"], false);
        assert_eq!(provenance["provider_completion_claimed"], false);
        assert_eq!(provenance["provider_inference_started"], false);
        assert_eq!(
            provenance["grant"]["run_id"],
            Uuid::from_u128(4).to_string()
        );
        assert_eq!(
            provenance["grant"]["source_run_id"],
            Uuid::from_u128(6).to_string()
        );
        assert_eq!(
            provenance["grant"]["historical_run_id"],
            Uuid::from_u128(5).to_string()
        );
    }

    #[test]
    fn issue211_retained_receipt_rejects_foreign_or_falsely_completed_content() {
        for (field, value) in [
            ("provider", json!("other-provider")),
            ("sdk_version", json!("unsupported")),
            ("session_id", json!(Uuid::new_v4())),
            ("model", json!("other-model")),
            ("reasoning_effort", json!("changed")),
            ("provider_completed", json!(true)),
            ("environment", json!({"synthetic":"private"})),
            ("diff_sha256", json!("not-a-digest")),
            ("assistant_messages", json!(-1)),
        ] {
            let mut candidate = receipt();
            candidate[field] = value;
            assert!(
                validate_retained_copilot_receipt(
                    &serde_json::to_vec(&candidate).unwrap(),
                    &grant()
                )
                .is_err(),
                "{field}"
            );
        }
    }

    #[test]
    fn issue211_retained_receipt_requires_complete_native_shape_and_safe_relative_paths() {
        for field in receipt().as_object().unwrap().keys() {
            let mut candidate = receipt();
            candidate.as_object_mut().unwrap().remove(field);
            assert!(
                validate_retained_copilot_receipt(
                    &serde_json::to_vec(&candidate).unwrap(),
                    &grant()
                )
                .is_err(),
                "{field}"
            );
        }
        for path in [
            "/private/native",
            "C:\\native",
            "..\\secret",
            "app/../secret",
            "bad\npath",
        ] {
            let mut candidate = receipt();
            candidate["changed_paths"] = json!([path]);
            assert!(
                validate_retained_copilot_receipt(
                    &serde_json::to_vec(&candidate).unwrap(),
                    &grant()
                )
                .is_err()
            );
        }
        assert!(validate_retained_copilot_receipt(b"", &grant()).is_err());
        assert!(validate_retained_copilot_receipt(b"not-json", &grant()).is_err());
        assert!(
            validate_retained_copilot_receipt(
                &vec![b' '; MAX_RETAINED_PROVIDER_RECEIPT_BYTES + 1],
                &grant()
            )
            .is_err()
        );
    }

    #[test]
    fn issue211_retained_receipt_rejects_duplicate_native_fields_in_literal_bytes() {
        let original = serde_json::to_string(&receipt()).unwrap();
        for (field, value) in receipt().as_object().unwrap() {
            let duplicate = format!(
                "{},{}:{}}}",
                original.strip_suffix('}').unwrap(),
                serde_json::to_string(field).unwrap(),
                serde_json::to_string(value).unwrap()
            );
            assert!(
                validate_retained_copilot_receipt(duplicate.as_bytes(), &grant()).is_err(),
                "{field}"
            );
        }
        let ambiguous = original.replacen(
            "\"provider\":\"github-copilot\"",
            "\"provider\":\"foreign\",\"provider\":\"github-copilot\"",
            1,
        );
        assert_ne!(ambiguous, original);
        assert!(validate_retained_copilot_receipt(ambiguous.as_bytes(), &grant()).is_err());
    }

    #[test]
    fn issue211_retained_receipt_grant_rejects_missing_or_conflated_generations() {
        let mut candidate = grant();
        candidate.run_id = candidate.source_run_id;
        assert!(candidate.validate().is_err());
        candidate = grant();
        candidate.collection_id = Uuid::nil();
        assert!(candidate.validate().is_err());
        candidate = grant();
        candidate.provider_session_id = Uuid::nil().to_string();
        assert!(candidate.validate().is_err());
        candidate = grant();
        candidate.expected_workspace_fingerprint = "invalid".to_owned();
        assert!(candidate.validate().is_err());
        candidate = grant();
        candidate.expected_head_commit.clear();
        assert!(candidate.validate().is_err());
        let mut serialized = serde_json::to_value(grant()).unwrap();
        serialized["extra_authority"] = json!(true);
        assert!(serde_json::from_value::<RetainedProviderReceiptGrant>(serialized).is_err());
    }
}
