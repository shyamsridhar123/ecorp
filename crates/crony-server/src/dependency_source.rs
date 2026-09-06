//! Pure, fail-closed decoding of the runner's version-one typed source export.
//!
//! This module performs no filesystem, network, clock, Git, or process operations.
//! Matching hashes and runtime metadata are NOT proof of provenance or completion.
//! Before calling, the parent must use ArtifactService to verify the ready
//! StoredArtifact's signature, source-deliverable role, media type, bytes, digest,
//! and retention. It must independently check authoritative DB Corp/task/run/source
//! lineage and passing verification, and derive expectations from the persisted
//! task contract and source deliverable, not from this document or provider prose.
//!
//! Returned content remains untrusted data, never instructions or executable input.
//! The parent must frame it accordingly and enforce its aggregate 64-KiB prompt
//! budget across dependencies and all other prompt material. No fallback to a
//! worktree, provider report, truncated content, or partial file set is permitted.

use std::{collections::BTreeSet, fmt, marker::PhantomData};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use crony_domain::{DeliverableForm, SourceDeliverable, repository_relative_path_is_valid};
use serde::{
    Deserialize, Deserializer,
    de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor},
};
use sha2::{Digest, Sha256};

pub const TYPED_SOURCE_MEDIA_TYPE: &str = "application/vnd.ecorp.deliverable+json";
pub const MAX_TYPED_SOURCE_FILES: usize = 8;
pub const MAX_TYPED_SOURCE_FILE_BYTES: usize = 12 * 1024;
/// Sum of content, path, digest, and 128 bytes of framing allowance per file.
pub const MAX_TYPED_SOURCE_PROMPT_BYTES: usize = 16 * 1024;
/// Includes the export's redundant patch and JSON/base64 overhead, not prompt text.
pub const MAX_TYPED_SOURCE_ENVELOPE_BYTES: usize = 128 * 1024;

const MAX_PATCH_BYTES: usize = 64 * 1024;
const FILE_FRAMING_BYTES: usize = 128;

/// Explicit authority supplied by the caller; none of it is inferred from bytes.
#[derive(Debug)]
pub struct ExpectedTypedSource<'a> {
    /// Full, lowercase 40- or 64-digit Git object ID, never a symbolic ref.
    pub base_commit: &'a str,
    /// Lowercase SHA-256 of the authoritative normalized verification report.
    pub verification_sha256: &'a str,
    /// Nonempty exact file set from the contract, not directories or write globs.
    pub declared_paths: &'a [String],
    /// Optional additional exact-set check; cannot replace the declared contract.
    /// A runtime changed-path claim alone is not authority.
    pub changed_paths: Option<&'a [String]>,
    /// Optional cross-check against an already-authorized DB source record:
    /// form, MIME, envelope bytes/hash, base/verifier, branch, and optional head.
    /// IDs, role, signature, retention, and lineage are still caller preconditions.
    pub source: Option<&'a SourceDeliverable>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencySourceFile {
    pub path: String,
    pub sha256: String,
    /// Exact UTF-8 bytes as text: no newline normalization, escaping, or trimming.
    pub content: String,
}

/// Diagnostics deliberately contain no untrusted paths, content, or JSON values.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DependencySourceError {
    #[error("invalid typed source expectation: {0}")]
    InvalidExpectation(&'static str),
    #[error("invalid or ambiguous typed source envelope")]
    InvalidEnvelope,
    #[error("typed source exceeds {0}")]
    LimitExceeded(&'static str),
    #[error("typed source does not match expected {0}")]
    ContractMismatch(&'static str),
    #[error("typed source contains an unsafe or unsupported path")]
    UnsafePath,
    #[error("typed source contains duplicate, aliased, or conflicting paths")]
    DuplicatePath,
    #[error("typed source contains an unsupported status or non-regular file mode")]
    UnsupportedChange,
    #[error("typed source MIME is unsupported or inconsistent with its text file")]
    UnsupportedMediaType,
    #[error("typed source base64 is not canonical")]
    InvalidBase64,
    #[error("typed source byte count does not match content")]
    ByteCountMismatch,
    #[error("typed source digest does not match content")]
    DigestMismatch,
    #[error("typed source content is not supported UTF-8 text")]
    InvalidText,
}

type DecodeResult<T> = Result<T, DependencySourceError>;

/// Planner preflight for the decoder's exact declared-file dialect.
///
/// Rejects empty/oversized sets, duplicate or case-aliased paths, file/directory
/// conflicts, unsafe paths, and extensions the typed exporter cannot label as
/// supported text. This does not validate future content or grant source authority.
pub fn validate_typed_source_paths(paths: &[String]) -> Result<(), DependencySourceError> {
    if paths.is_empty() {
        return Err(DependencySourceError::InvalidExpectation(
            "declared file set must not be empty",
        ));
    }
    let validated = checked_paths(paths.iter().map(String::as_str))?;
    if validated.iter().any(|path| text_media_type(path).is_none()) {
        return Err(DependencySourceError::UnsupportedMediaType);
    }
    Ok(())
}

/// Decode **all** declared files, sorted by exact path, or return an error.
///
/// Accepts only the exact v1 `typed_artifact_set` envelope emitted by
/// `crony-runner/src/deliverable.rs`. Unknown/duplicate fields, missing fields
/// (including required nullable metadata), and array-shaped objects are rejected.
/// Only A/M/T changes with modes 100644/100755 and the exporter's text/plain or
/// application/json file mappings are accepted. Hidden, non-ASCII, credential,
/// key, and non-portable paths are intentionally outside this handoff dialect.
///
/// The embedded patch is bounded and integrity-checked, but never interpreted,
/// applied, or included in dependency text. Its semantics are not verified here;
/// the exact declared `changes` set is the sole source of returned file content.
/// A typed set cannot carry a Git bundle. See module-level authority preconditions.
pub fn decode_typed_source(
    bytes: &[u8],
    expected: &ExpectedTypedSource<'_>,
) -> Result<Vec<DependencySourceFile>, DependencySourceError> {
    use DependencySourceError as Error;

    // Bound untrusted JSON before allocating strings or deserializing collections.
    if bytes.len() > MAX_TYPED_SOURCE_ENVELOPE_BYTES {
        return Err(Error::LimitExceeded("envelope byte limit"));
    }
    if !valid_commit(expected.base_commit) || !valid_digest(expected.verification_sha256) {
        return Err(Error::InvalidExpectation(
            "base commit or verification SHA-256",
        ));
    }
    validate_typed_source_paths(expected.declared_paths)?;
    let declared = expected
        .declared_paths
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if let Some(changed) = expected.changed_paths
        && checked_paths(changed.iter().map(String::as_str))? != declared
    {
        return Err(Error::ContractMismatch("changed paths"));
    }

    let Object(document) =
        serde_json::from_slice::<Object<ArtifactSet>>(bytes).map_err(|_| Error::InvalidEnvelope)?;
    if document.schema_version != 1 || document.form != "typed_artifact_set" {
        return Err(Error::InvalidEnvelope);
    }
    if document.base_commit != expected.base_commit {
        return Err(Error::ContractMismatch("base commit"));
    }
    if document.verification_sha256 != expected.verification_sha256 {
        return Err(Error::ContractMismatch("verification SHA-256"));
    }
    if document
        .head_commit
        .as_deref()
        .is_some_and(|head| !valid_commit(head))
        || document.branch.is_empty()
        || document.branch.len() > 512
        || document.branch.trim() != document.branch
        || document.branch.chars().any(char::is_control)
        || !valid_digest(&document.patch_sha256)
    {
        return Err(Error::InvalidEnvelope);
    }
    if let Some(source) = expected.source {
        check_source_record(bytes, &document, source)?;
    }

    let actual = checked_paths(document.changes.iter().map(|change| change.path.as_str()))?;
    if actual != declared {
        return Err(Error::ContractMismatch("declared file set"));
    }

    // Validate every file's metadata and the entire output budget before decoding
    // ANY base64. Declared sizes are not trusted; actual sizes are checked below.
    let mut prompt_bytes = 0;
    for change in &document.changes {
        if !matches!(change.status.as_str(), "A" | "M" | "T")
            || !matches!(change.mode.as_str(), "100644" | "100755")
        {
            return Err(Error::UnsupportedChange);
        }
        if text_media_type(&change.path) != Some(change.media_type.as_str()) {
            return Err(Error::UnsupportedMediaType);
        }
        if !valid_digest(&change.sha256) {
            return Err(Error::InvalidEnvelope);
        }
        if change.bytes > MAX_TYPED_SOURCE_FILE_BYTES as u64
            || change.content_base64.len() > encoded_len(MAX_TYPED_SOURCE_FILE_BYTES)
        {
            return Err(Error::LimitExceeded("per-file byte limit"));
        }
        // Safe conversion and addition: bytes <= 12 KiB, paths <= 500, files <= 8.
        let file_bytes = change.bytes as usize;
        if change.content_base64.len() != encoded_len(file_bytes) {
            return Err(Error::ByteCountMismatch);
        }
        prompt_bytes += file_bytes + change.path.len() + 64 + FILE_FRAMING_BYTES;
        if prompt_bytes > MAX_TYPED_SOURCE_PROMPT_BYTES {
            return Err(Error::LimitExceeded("per-parent prompt byte limit"));
        }
    }

    let patch = decode_base64(&document.patch_base64, MAX_PATCH_BYTES)?;
    if digest(&patch) != document.patch_sha256 {
        return Err(Error::DigestMismatch);
    }
    drop(patch);

    let mut files = Vec::with_capacity(document.changes.len());
    for change in document.changes {
        let decoded = decode_base64(&change.content_base64, MAX_TYPED_SOURCE_FILE_BYTES)?;
        if decoded.len() as u64 != change.bytes {
            return Err(Error::ByteCountMismatch);
        }
        if digest(&decoded) != change.sha256 {
            return Err(Error::DigestMismatch);
        }
        let content = String::from_utf8(decoded).map_err(|_| Error::InvalidText)?;
        // UTF-8 alone does not exclude NUL-filled/binary files or terminal escapes.
        if content
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\t' | '\n' | '\r'))
        {
            return Err(Error::InvalidText);
        }
        if change.media_type == "application/json"
            && serde_json::from_str::<de::IgnoredAny>(&content).is_err()
        {
            return Err(Error::UnsupportedMediaType);
        }
        files.push(DependencySourceFile {
            path: change.path,
            sha256: change.sha256,
            content,
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn check_source_record(
    bytes: &[u8],
    document: &ArtifactSet,
    source: &SourceDeliverable,
) -> DecodeResult<()> {
    use DependencySourceError as Error;

    if source.form != DeliverableForm::TypedArtifactSet
        || source.base_commit != document.base_commit
        || source.verification_sha256 != document.verification_sha256
        || source.branch != document.branch
        || source.head_commit != document.head_commit
    {
        return Err(Error::ContractMismatch("source deliverable identity"));
    }
    if source.media_type != TYPED_SOURCE_MEDIA_TYPE {
        return Err(Error::UnsupportedMediaType);
    }
    if source.bytes != bytes.len() as i64 {
        return Err(Error::ByteCountMismatch);
    }
    if source.sha256 != digest(bytes) {
        return Err(Error::DigestMismatch);
    }
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && lowercase_hex(value)
}

fn valid_commit(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && lowercase_hex(value)
}

fn lowercase_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn encoded_len(bytes: usize) -> usize {
    bytes.div_ceil(3) * 4
}

fn decode_base64(encoded: &str, limit: usize) -> DecodeResult<Vec<u8>> {
    if encoded.len() > encoded_len(limit) {
        return Err(DependencySourceError::LimitExceeded("encoded byte limit"));
    }
    // STANDARD requires canonical padding and zero trailing bits; it rejects
    // whitespace, URL-safe substitutions, misplaced padding, and trailing junk.
    let decoded = BASE64
        .decode(encoded)
        .map_err(|_| DependencySourceError::InvalidBase64)?;
    if decoded.len() > limit {
        return Err(DependencySourceError::LimitExceeded("decoded byte limit"));
    }
    Ok(decoded)
}

fn checked_paths<'a>(values: impl IntoIterator<Item = &'a str>) -> DecodeResult<BTreeSet<&'a str>> {
    let mut paths = BTreeSet::new();
    let mut portable = Vec::<String>::new();
    for value in values {
        if paths.len() == MAX_TYPED_SOURCE_FILES {
            return Err(DependencySourceError::LimitExceeded("file count limit"));
        }
        if !safe_path(value) {
            return Err(DependencySourceError::UnsafePath);
        }
        let folded = value.to_ascii_lowercase();
        if portable.iter().any(|other| {
            folded == *other
                || folded
                    .strip_prefix(other.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
                || other
                    .strip_prefix(folded.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
        }) {
            return Err(DependencySourceError::DuplicatePath);
        }
        paths.insert(value);
        portable.push(folded);
    }
    Ok(paths)
}

fn safe_path(path: &str) -> bool {
    // Deliberately narrower than generic deliverables. No normalization can turn
    // an unsafe/aliased path into an allowed one. Hidden components cover Git,
    // provider homes, runner internals, .env*, cloud configs, and credential stores.
    if !repository_relative_path_is_valid(path)
        || !path.is_ascii()
        || path.bytes().any(|byte| {
            matches!(
                byte,
                b'*' | b'?' | b'[' | b']' | b'<' | b'>' | b'"' | b'|' | b'~' | b'%'
            )
        })
    {
        return false;
    }
    path.split('/').all(|component| {
        if component.starts_with('.') || component.ends_with('.') || component.trim() != component {
            return false;
        }
        let lower = component.to_ascii_lowercase();
        let stem = lower.split('.').next().unwrap_or_default();
        let device = matches!(stem, "con" | "prn" | "aux" | "nul" | "conin$" | "conout$")
            || (stem.len() == 4
                && (stem.starts_with("com") || stem.starts_with("lpt"))
                && stem.as_bytes()[3].is_ascii_digit());
        let secret = matches!(
            lower.as_str(),
            "_netrc"
                | "npmrc"
                | "terraform.rc"
                | "kubeconfig"
                | "accesstokens.json"
                | "application_default_credentials.json"
        ) || [
            "credentials",
            "secrets",
            "id_rsa",
            "id_dsa",
            "id_ecdsa",
            "id_ed25519",
        ]
        .iter()
        .any(|name| {
            lower == *name
                || lower
                    .strip_prefix(*name)
                    .is_some_and(|rest| rest.starts_with('.'))
        }) || [
            ".pem",
            ".key",
            ".p12",
            ".pfx",
            ".p8",
            ".ppk",
            ".jks",
            ".keystore",
        ]
        .iter()
        .any(|suffix| lower.ends_with(*suffix));
        !device && !secret
    })
}

fn text_media_type(path: &str) -> Option<&'static str> {
    // Keep this subset aligned with the runner's infer_media_type. Unknown
    // extensions/octet-stream and image/svg+xml are not prompt-text handoffs.
    let extension = path
        .rsplit('/')
        .next()?
        .rsplit_once('.')?
        .1
        .to_ascii_lowercase();
    match extension.as_str() {
        "json" => Some("application/json"),
        "md" | "txt" | "rs" | "ts" | "tsx" | "js" | "mjs" | "css" | "html" | "toml" | "yaml"
        | "yml" => Some("text/plain"),
        _ => None,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactSet {
    schema_version: u32,
    form: String,
    base_commit: String,
    #[serde(deserialize_with = "required_nullable_string")]
    head_commit: Option<String>,
    branch: String,
    verification_sha256: String,
    patch_sha256: String,
    patch_base64: String,
    // Unit fields require explicit null, not missing fields or embedded bundles.
    #[serde(rename = "git_bundle_sha256")]
    _git_bundle_sha256: (),
    #[serde(rename = "git_bundle_base64")]
    _git_bundle_base64: (),
    #[serde(deserialize_with = "bounded_changes")]
    changes: Vec<ArchivedChange>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchivedChange {
    path: String,
    status: String,
    mode: String,
    sha256: String,
    bytes: u64,
    media_type: String,
    content_base64: String,
}

fn required_nullable_string<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

// Serde structs otherwise also accept positional arrays. Both the envelope and
// each change must be JSON objects, with derived duplicate/unknown-field checks.
struct Object<T>(T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Object<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ObjectVisitor<T>(PhantomData<T>);

        impl<'de, T: Deserialize<'de>> Visitor<'de> for ObjectVisitor<T> {
            type Value = Object<T>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object")
            }

            fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
                T::deserialize(de::value::MapAccessDeserializer::new(map)).map(Object)
            }
        }

        deserializer.deserialize_map(ObjectVisitor::<T>(PhantomData))
    }
}

fn bounded_changes<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<ArchivedChange>, D::Error> {
    struct ChangesVisitor;
    struct ChangeSeed {
        full: bool,
    }

    impl<'de> DeserializeSeed<'de> for ChangeSeed {
        type Value = ArchivedChange;

        fn deserialize<D: Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> Result<Self::Value, D::Error> {
            if self.full {
                return Err(de::Error::custom("too many source files"));
            }
            Object::<ArchivedChange>::deserialize(deserializer).map(|object| object.0)
        }
    }

    impl<'de> Visitor<'de> for ChangesVisitor {
        type Value = Vec<ArchivedChange>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a bounded array of source changes")
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
            let mut changes = Vec::with_capacity(MAX_TYPED_SOURCE_FILES);
            while let Some(change) = sequence.next_element_seed(ChangeSeed {
                full: changes.len() == MAX_TYPED_SOURCE_FILES,
            })? {
                changes.push(change);
            }
            Ok(changes)
        }
    }

    deserializer.deserialize_seq(ChangesVisitor)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    const BASE: &str = "6162bb2ed44a1fe1c7ffc726dd9bedc799b398bd";
    const VERIFIER: &str = "abababababababababababababababababababababababababababababababab";

    fn contract(paths: &[String]) -> ExpectedTypedSource<'_> {
        ExpectedTypedSource {
            base_commit: BASE,
            verification_sha256: VERIFIER,
            declared_paths: paths,
            changed_paths: None,
            source: None,
        }
    }

    fn change(path: &str, content: &[u8]) -> Value {
        json!({
            "path": path,
            "status": "A",
            "mode": "100644",
            "sha256": digest(content),
            "bytes": content.len(),
            "media_type": text_media_type(path).unwrap_or("application/octet-stream"),
            "content_base64": BASE64.encode(content),
        })
    }

    fn document(changes: Vec<Value>) -> Value {
        // The patch is opaque to this decoder, but its exact bytes are hashed.
        let patch = b"diff --git a/handoff.md b/handoff.md\n";
        json!({
            "schema_version": 1,
            "form": "typed_artifact_set",
            "base_commit": BASE,
            "head_commit": null,
            "branch": "crony/task-example",
            "verification_sha256": VERIFIER,
            "patch_sha256": digest(patch),
            "patch_base64": BASE64.encode(patch),
            "git_bundle_sha256": null,
            "git_bundle_base64": null,
            "changes": changes,
        })
    }

    fn fixture() -> (Value, Vec<String>) {
        let paths = vec![
            "handoffs/design.md".to_owned(),
            "handoffs/tests.json".to_owned(),
        ];
        let document = document(vec![
            change(&paths[0], "Design\r\n\tCafé 🛠\nno final newline".as_bytes()),
            change(
                &paths[1],
                b"{\n  \"checks\": [\"isolation\", \"lineage\"]\n}\n",
            ),
        ]);
        (document, paths)
    }

    fn decode(document: &Value, paths: &[String]) -> DecodeResult<Vec<DependencySourceFile>> {
        decode_typed_source(&serde_json::to_vec(document).unwrap(), &contract(paths))
    }

    fn replace_content(change: &mut Value, content: &[u8]) {
        change["sha256"] = json!(digest(content));
        change["bytes"] = json!(content.len());
        change["content_base64"] = json!(BASE64.encode(content));
    }

    fn source_record(document: &Value, bytes: &[u8]) -> SourceDeliverable {
        let time = chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).unwrap();
        SourceDeliverable {
            id: uuid::Uuid::nil(),
            corp_id: uuid::Uuid::nil(),
            task_id: uuid::Uuid::nil(),
            run_id: uuid::Uuid::nil(),
            artifact_id: uuid::Uuid::nil(),
            form: DeliverableForm::TypedArtifactSet,
            file_name: "ecorp-artifact-set.json".to_owned(),
            uri: "/unused".to_owned(),
            sha256: digest(bytes),
            media_type: TYPED_SOURCE_MEDIA_TYPE.to_owned(),
            bytes: bytes.len() as i64,
            provenance_signature: "caller-must-verify-this-separately".to_owned(),
            verification_sha256: VERIFIER.to_owned(),
            base_commit: BASE.to_owned(),
            head_commit: document["head_commit"].as_str().map(str::to_owned),
            branch: document["branch"].as_str().unwrap().to_owned(),
            integration_state: "ready_for_review".to_owned(),
            retention_until: time,
            created_at: time,
        }
    }

    #[test]
    fn multiple_handoffs_are_complete_lossless_and_sorted() {
        let (mut document, mut paths) = fixture();
        let original = document["changes"].as_array().unwrap().clone();
        document["changes"].as_array_mut().unwrap().reverse();
        paths.reverse();
        let files = decode(&document, &paths).unwrap();
        assert_eq!(files.len(), 2);
        for (file, expected) in files.iter().zip(original) {
            assert_eq!(file.path, expected["path"].as_str().unwrap());
            assert_eq!(file.sha256, expected["sha256"].as_str().unwrap());
            assert_eq!(
                file.content.as_bytes(),
                BASE64
                    .decode(expected["content_base64"].as_str().unwrap())
                    .unwrap()
            );
        }
    }

    #[test]
    fn regular_modes_and_non_deleted_export_statuses_are_supported() {
        let (original, paths) = fixture();
        for mode in ["100644", "100755"] {
            for status in ["A", "M", "T"] {
                let mut document = original.clone();
                document["changes"][0]["mode"] = json!(mode);
                document["changes"][0]["status"] = json!(status);
                assert_eq!(decode(&document, &paths).unwrap().len(), 2);
            }
        }
    }

    #[test]
    fn empty_files_bom_and_whitespace_are_not_omitted_or_normalized() {
        let paths = vec!["empty.txt".to_owned(), "whitespace.md".to_owned()];
        let text = "\u{feff} \t\r\n\n ";
        let document = document(vec![
            change(&paths[0], b""),
            change(&paths[1], text.as_bytes()),
        ]);
        let files = decode(&document, &paths).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].content, "");
        assert_eq!(files[0].sha256, digest(b""));
        assert_eq!(files[1].content, text);
    }

    #[test]
    fn schema_form_base_and_verifier_are_exact() {
        let (original, paths) = fixture();
        for (field, bad) in [
            ("schema_version", json!(0)),
            ("schema_version", json!(2)),
            ("schema_version", json!(1.0)),
            ("schema_version", json!("1")),
            ("form", json!("archive")),
            ("form", json!("review_only_report")),
            ("form", json!("commit_branch")),
            ("base_commit", json!("0".repeat(40))),
            ("verification_sha256", json!("0".repeat(64))),
            ("verification_sha256", json!(VERIFIER.to_uppercase())),
        ] {
            let mut document = original.clone();
            document[field] = bad;
            assert!(decode(&document, &paths).is_err(), "{field}");
        }
    }

    #[test]
    fn expectations_require_immutable_ids_and_a_nonempty_exact_set() {
        let (document, paths) = fixture();
        let bytes = serde_json::to_vec(&document).unwrap();
        for base in [
            "HEAD",
            "",
            "abc",
            &"a".repeat(41),
            &"g".repeat(40),
            &BASE.to_uppercase(),
        ] {
            let expected = ExpectedTypedSource {
                base_commit: base,
                ..contract(&paths)
            };
            assert!(decode_typed_source(&bytes, &expected).is_err());
        }
        for verifier in ["", "passed", &"a".repeat(63), &"G".repeat(64)] {
            let expected = ExpectedTypedSource {
                verification_sha256: verifier,
                ..contract(&paths)
            };
            assert!(decode_typed_source(&bytes, &expected).is_err());
        }
        assert!(decode(&document, &[]).is_err());
        let duplicate = vec![paths[0].clone(), paths[0].clone()];
        assert_eq!(
            decode(&document, &duplicate).unwrap_err(),
            DependencySourceError::DuplicatePath
        );
    }

    #[test]
    fn planner_preflight_rejects_the_same_path_dialect_before_provider_work() {
        let (_, paths) = fixture();
        assert!(validate_typed_source_paths(&paths).is_ok());
        for bad in [
            vec![],
            vec![".codex/handoff.md".to_owned()],
            vec!["handoffs/é.md".to_owned()],
            vec!["secrets.json".to_owned()],
            vec!["handoffs/**".to_owned()],
            vec!["unsupported.bin".to_owned()],
            vec!["a.md".to_owned(), "A.md".to_owned()],
            vec!["a.md".to_owned(), "a.md/child.txt".to_owned()],
            (0..=MAX_TYPED_SOURCE_FILES)
                .map(|index| format!("file-{index}.md"))
                .collect(),
        ] {
            assert!(validate_typed_source_paths(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn full_sha256_git_object_ids_are_supported() {
        let (mut document, paths) = fixture();
        let base = "c".repeat(64);
        document["base_commit"] = json!(base);
        document["head_commit"] = json!("d".repeat(64));
        let expected = ExpectedTypedSource {
            base_commit: &base,
            ..contract(&paths)
        };
        assert!(decode_typed_source(&serde_json::to_vec(&document).unwrap(), &expected).is_ok());
    }

    #[test]
    fn changed_paths_are_an_additional_exact_unordered_constraint() {
        let (document, paths) = fixture();
        let bytes = serde_json::to_vec(&document).unwrap();
        let reversed = paths.iter().rev().cloned().collect::<Vec<_>>();
        let expected = ExpectedTypedSource {
            changed_paths: Some(&reversed),
            ..contract(&paths)
        };
        assert!(decode_typed_source(&bytes, &expected).is_ok());
        for changed in [
            vec![],
            vec![paths[0].clone()],
            vec![paths[0].clone(), paths[0].clone()],
            vec![paths[0].clone(), "other.md".to_owned()],
            vec!["../unsafe.md".to_owned()],
        ] {
            let expected = ExpectedTypedSource {
                changed_paths: Some(&changed),
                ..contract(&paths)
            };
            assert!(decode_typed_source(&bytes, &expected).is_err());
        }
    }

    #[test]
    fn optional_source_record_binds_identity_mime_and_exact_envelope() {
        let (mut document, paths) = fixture();
        document["head_commit"] = json!("e".repeat(40));
        let bytes = serde_json::to_vec(&document).unwrap();
        let source = source_record(&document, &bytes);
        let expected = ExpectedTypedSource {
            source: Some(&source),
            ..contract(&paths)
        };
        assert!(decode_typed_source(&bytes, &expected).is_ok());
        for field in [
            "form", "base", "verifier", "head", "branch", "mime", "bytes", "hash",
        ] {
            let mut bad = source.clone();
            match field {
                "form" => bad.form = DeliverableForm::Archive,
                "base" => bad.base_commit = "0".repeat(40),
                "verifier" => bad.verification_sha256 = "0".repeat(64),
                "head" => bad.head_commit = None,
                "branch" => bad.branch = "other-branch".to_owned(),
                "mime" => bad.media_type = "application/json".to_owned(),
                "bytes" => bad.bytes = -1,
                "hash" => bad.sha256 = "0".repeat(64),
                _ => unreachable!(),
            }
            let expected = ExpectedTypedSource {
                source: Some(&bad),
                ..contract(&paths)
            };
            assert!(decode_typed_source(&bytes, &expected).is_err(), "{field}");
        }
    }

    #[test]
    fn every_export_field_is_required_even_nullable_metadata() {
        let (original, paths) = fixture();
        for field in original.as_object().unwrap().keys() {
            let mut document = original.clone();
            document.as_object_mut().unwrap().remove(field);
            assert!(decode(&document, &paths).is_err(), "missing {field}");
        }
        for field in original["changes"][0].as_object().unwrap().keys() {
            let mut document = original.clone();
            document["changes"][0]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(decode(&document, &paths).is_err(), "missing change {field}");
            let mut document = original.clone();
            document["changes"][0][field] = Value::Null;
            assert!(decode(&document, &paths).is_err(), "null change {field}");
        }
    }

    #[test]
    fn duplicate_fields_are_rejected_including_equal_values_and_nulls() {
        let (document, paths) = fixture();
        let serialized = serde_json::to_string(&document).unwrap();
        for (key, value) in document.as_object().unwrap() {
            let duplicate = format!("{{\"{key}\":{value},{}", &serialized[1..]);
            assert!(
                decode_typed_source(duplicate.as_bytes(), &contract(&paths)).is_err(),
                "{key}"
            );
        }
        let change = &document["changes"][0];
        let serialized_change = serde_json::to_string(change).unwrap();
        for (key, value) in change.as_object().unwrap() {
            let duplicate = format!("{{\"{key}\":{value},{}", &serialized_change[1..]);
            let bytes = serialized.replacen(&serialized_change, &duplicate, 1);
            assert!(
                decode_typed_source(bytes.as_bytes(), &contract(&paths)).is_err(),
                "{key}"
            );
        }
        let duplicate = format!(
            "{{\"pa\\u0074h\":{},{}",
            change["path"],
            &serialized_change[1..]
        );
        let bytes = serialized.replacen(&serialized_change, &duplicate, 1);
        assert!(decode_typed_source(bytes.as_bytes(), &contract(&paths)).is_err());
    }

    #[test]
    fn unknown_fields_positional_arrays_trailing_data_and_wrong_types_fail() {
        let (original, paths) = fixture();
        let mut document = original.clone();
        document["runtime_metadata"] = json!({"verified": true});
        assert!(decode(&document, &paths).is_err());
        let mut document = original.clone();
        document["changes"][0]["content"] = json!("unhashed alternative");
        assert!(decode(&document, &paths).is_err());
        assert!(decode(&json!([]), &paths).is_err());
        let mut document = original.clone();
        let file = &original["changes"][0];
        document["changes"][0] = json!([
            file["path"],
            file["status"],
            file["mode"],
            file["sha256"],
            file["bytes"],
            file["media_type"],
            file["content_base64"]
        ]);
        assert!(decode(&document, &paths).is_err());
        for bytes in [b"".as_slice(), b"null", b"{}", b"\xff", b"{"] {
            assert!(decode_typed_source(bytes, &contract(&paths)).is_err());
        }
        let trailing = format!("{} {{}}", serde_json::to_string(&original).unwrap());
        assert!(decode_typed_source(trailing.as_bytes(), &contract(&paths)).is_err());
        for value in [json!(-1), json!(1.0), json!("1"), json!(true), json!(null)] {
            let mut document = original.clone();
            document["changes"][0]["bytes"] = value;
            assert!(decode(&document, &paths).is_err());
        }
    }

    #[test]
    fn extra_missing_duplicate_and_case_aliased_files_fail() {
        let (original, paths) = fixture();
        let mut missing = original.clone();
        missing["changes"].as_array_mut().unwrap().pop();
        assert!(decode(&missing, &paths).is_err());
        let mut extra = original.clone();
        extra["changes"]
            .as_array_mut()
            .unwrap()
            .push(change("extra.md", b"extra"));
        assert!(decode(&extra, &paths).is_err());
        for path in [&paths[0], &paths[0].to_uppercase()] {
            let mut duplicate = original.clone();
            duplicate["changes"]
                .as_array_mut()
                .unwrap()
                .push(change(path, b"duplicate"));
            assert_eq!(
                decode(&duplicate, &paths).unwrap_err(),
                DependencySourceError::DuplicatePath
            );
        }
        let mut none = original;
        none["changes"] = json!([]);
        assert!(decode(&none, &paths).is_err());
        let conflict = ["a.md".to_owned(), "A.md/child.md".to_owned()];
        assert_eq!(
            checked_paths(conflict.iter().map(String::as_str)).unwrap_err(),
            DependencySourceError::DuplicatePath
        );
    }

    #[test]
    fn traversal_git_internal_secret_and_nonportable_paths_are_rejected() {
        let (original, paths) = fixture();
        for path in [
            "",
            "../escape.md",
            "a/../../escape.md",
            "/absolute.md",
            "C:/file.md",
            "\\\\server\\file.md",
            "a\\file.md",
            "a//file.md",
            "a/./file.md",
            "file.md/",
            "./file.md",
            ".git/config",
            "nested/.GiT/config",
            ".gitmodules",
            ".codex/auth.json",
            ".claude/settings.json",
            ".ecorp-runner/state.md",
            ".crony/state.md",
            ".copilot/auth.json",
            ".config/gh/hosts.yml",
            ".ssh/key.txt",
            ".env",
            ".env.example",
            "nested/.azure/tokens.json",
            "credentials.json",
            "nested/secrets/token.md",
            "secrets.json",
            "id_rsa",
            "id_ed25519.pub",
            "nested/key.pem",
            "nested/key.pfx",
            "npmrc",
            "_netrc",
            "kubeconfig",
            "application_default_credentials.json",
            "NUL.txt",
            "a/COM1.md",
            "lpt9.json",
            "CONOUT$.md",
            "git~1/config.md",
            "a/file.md.",
            "a /file.md",
            "a/ file.md",
            "file.md ",
            "file.md:stream",
            "a/**",
            "a/*.md",
            "a/?.md",
            "a/[x].md",
            "a/pipe|x.md",
            "a/<x>.md",
            "a/%2e%2e/file.md",
            "a/\nfile.md",
            "a/\0file.md",
            "a/\u{202e}file.md",
            "a/é.md",
        ] {
            let mut document = original.clone();
            document["changes"][0]["path"] = json!(path);
            assert_eq!(
                decode(&document, &paths).unwrap_err(),
                DependencySourceError::UnsafePath,
                "{path:?}"
            );
            // Even a matching allowlist cannot authorize an unsafe path.
            let declared = vec![path.to_owned()];
            assert!(validate_typed_source_paths(&declared).is_err(), "{path:?}");
            assert!(decode(&document, &declared).is_err(), "{path:?}");
        }
        let long = format!("{}.md", "a".repeat(500));
        assert!(!safe_path(&long));
        for path in [
            "handoffs/spec.md",
            "src/lib.rs",
            "docs/Final plan.txt",
            "a-b_v2+ok.JSON",
        ] {
            assert!(safe_path(path), "{path}");
        }
    }

    #[test]
    fn deleted_renamed_unmerged_and_nonregular_changes_are_rejected() {
        let (original, paths) = fixture();
        for status in [
            "D", "D100", "R100", "C100", "U", "X", "B", "??", "", "m", "M100",
        ] {
            let mut document = original.clone();
            document["changes"][0]["status"] = json!(status);
            assert_eq!(
                decode(&document, &paths).unwrap_err(),
                DependencySourceError::UnsupportedChange
            );
        }
        for mode in ["120000", "160000", "040000", "100600", "644", "", "0100644"] {
            let mut document = original.clone();
            document["changes"][0]["mode"] = json!(mode);
            assert_eq!(
                decode(&document, &paths).unwrap_err(),
                DependencySourceError::UnsupportedChange
            );
        }
        let mut deleted = original;
        deleted["changes"][0] = json!({
            "path": paths[0], "status": "D", "mode": null, "sha256": null,
            "bytes": null, "media_type": null, "content_base64": null
        });
        assert!(decode(&deleted, &paths).is_err());
    }

    #[test]
    fn file_hash_and_actual_decoded_byte_count_are_verified() {
        let paths = vec!["handoff.md".to_owned()];
        let original = document(vec![change(&paths[0], b"abc")]);
        let mut bad_hash = original.clone();
        bad_hash["changes"][0]["sha256"] = json!("0".repeat(64));
        assert_eq!(
            decode(&bad_hash, &paths).unwrap_err(),
            DependencySourceError::DigestMismatch
        );
        // Both declared sizes have the same encoded length as 3; decoding must
        // still check actual bytes rather than trusting only the base64 bound.
        for count in [1, 2, 4] {
            let mut bad_bytes = original.clone();
            bad_bytes["changes"][0]["bytes"] = json!(count);
            assert_eq!(
                decode(&bad_bytes, &paths).unwrap_err(),
                DependencySourceError::ByteCountMismatch
            );
        }
        for hash in [
            "",
            "passed",
            &"z".repeat(64),
            &digest(b"abc").to_uppercase(),
        ] {
            let mut bad = original.clone();
            bad["changes"][0]["sha256"] = json!(hash);
            assert!(decode(&bad, &paths).is_err());
        }
    }

    #[test]
    fn base64_rejects_noncanonical_padding_bits_alphabets_and_whitespace() {
        let paths = vec!["handoff.md".to_owned()];
        let original = document(vec![change(&paths[0], b"f")]);
        for encoded in [
            "Zh==", "Zg", "Zg=", "Zg===", "Zg==\n", "Z g==", "Zg--", "Zg__", "Zg==Zg==", "Zg=!",
            "=Zg=", "é==", "!!!!",
        ] {
            let mut document = original.clone();
            document["changes"][0]["content_base64"] = json!(encoded);
            assert!(decode(&document, &paths).is_err(), "{encoded:?}");
        }
        let mut document = document(vec![change(&paths[0], b"fo")]);
        document["changes"][0]["content_base64"] = json!("Zm9=");
        assert_eq!(
            decode(&document, &paths).unwrap_err(),
            DependencySourceError::InvalidBase64
        );
    }

    #[test]
    fn invalid_utf8_and_utf8_binary_controls_fail_with_correct_hashes() {
        let paths = vec!["handoff.md".to_owned()];
        for content in [
            b"\xff".as_slice(),
            b"\xc0\x80",
            b"\xe2\x82",
            b"\xff\xfeh\0",
            b"text\0binary",
            b"text\x01",
            b"\x1b[31mred",
            b"\x7f",
            "\u{0085}".as_bytes(),
        ] {
            let document = document(vec![change(&paths[0], content)]);
            assert_eq!(
                decode(&document, &paths).unwrap_err(),
                DependencySourceError::InvalidText
            );
        }
    }

    #[test]
    fn mime_must_match_a_supported_text_extension_and_content() {
        let (original, paths) = fixture();
        for mime in [
            "",
            "not mime",
            "application/octet-stream",
            "image/png",
            "image/svg+xml",
            "application/pdf",
            "application/json",
            "text/html",
            "text/markdown",
            "Text/Plain",
            "text/plain; charset=utf-8",
        ] {
            let mut document = original.clone();
            document["changes"][0]["media_type"] = json!(mime);
            assert_eq!(
                decode(&document, &paths).unwrap_err(),
                DependencySourceError::UnsupportedMediaType,
                "{mime}"
            );
        }
        let mut bad_json = original.clone();
        replace_content(&mut bad_json["changes"][1], b"not actually JSON");
        assert_eq!(
            decode(&bad_json, &paths).unwrap_err(),
            DependencySourceError::UnsupportedMediaType
        );
        let mut bad_json_mime = original;
        bad_json_mime["changes"][1]["media_type"] = json!("text/plain");
        assert!(decode(&bad_json_mime, &paths).is_err());
        for path in [
            "picture.png",
            "image.svg",
            "source.bin",
            "unknown",
            "source.py",
        ] {
            let paths = vec![path.to_owned()];
            let mut document = document(vec![change(path, b"ASCII is not sufficient")]);
            document["changes"][0]["media_type"] = json!("text/plain");
            assert!(decode(&document, &paths).is_err());
        }
    }

    #[test]
    fn patch_integrity_and_typed_set_metadata_are_fail_closed() {
        let (original, paths) = fixture();
        for (field, value) in [
            ("patch_sha256", json!("0".repeat(64))),
            ("patch_base64", json!("!!!!")),
            ("patch_base64", Value::Null),
            ("git_bundle_sha256", json!("0".repeat(64))),
            ("git_bundle_base64", json!("")),
            ("head_commit", json!("HEAD")),
            ("head_commit", json!({})),
            ("branch", json!("")),
            ("branch", json!("branch\ninjection")),
            ("branch", json!("a".repeat(513))),
        ] {
            let mut document = original.clone();
            document[field] = value;
            assert!(decode(&document, &paths).is_err(), "{field}");
        }
    }

    #[test]
    fn envelope_and_file_count_caps_precede_unbounded_parsing() {
        let (_, paths) = fixture();
        assert_eq!(
            decode_typed_source(
                &vec![b' '; MAX_TYPED_SOURCE_ENVELOPE_BYTES + 1],
                &contract(&paths)
            )
            .unwrap_err(),
            DependencySourceError::LimitExceeded("envelope byte limit")
        );
        let paths = (0..MAX_TYPED_SOURCE_FILES)
            .map(|index| format!("file-{index}.md"))
            .collect::<Vec<_>>();
        let mut document = document(paths.iter().map(|path| change(path, b"small")).collect());
        assert_eq!(
            decode(&document, &paths).unwrap().len(),
            MAX_TYPED_SOURCE_FILES
        );
        // The ninth value is rejected before its fields are deserialized.
        document["changes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"unexpected": "ninth"}));
        assert_eq!(
            decode(&document, &paths).unwrap_err(),
            DependencySourceError::InvalidEnvelope
        );
        let mut too_many = paths;
        too_many.push("file-extra.md".to_owned());
        assert_eq!(
            decode(&document, &too_many).unwrap_err(),
            DependencySourceError::LimitExceeded("file count limit")
        );
    }

    #[test]
    fn per_file_encoded_decoded_and_declared_sizes_are_bounded() {
        let paths = vec!["handoff.md".to_owned()];
        let document_at_limit = document(vec![change(
            &paths[0],
            &vec![b'a'; MAX_TYPED_SOURCE_FILE_BYTES],
        )]);
        assert_eq!(
            decode(&document_at_limit, &paths).unwrap()[0].content.len(),
            MAX_TYPED_SOURCE_FILE_BYTES
        );
        let over = document(vec![change(
            &paths[0],
            &vec![b'a'; MAX_TYPED_SOURCE_FILE_BYTES + 1],
        )]);
        assert!(matches!(
            decode(&over, &paths),
            Err(DependencySourceError::LimitExceeded(_))
        ));
        let unicode = "é".repeat(MAX_TYPED_SOURCE_FILE_BYTES / 2 + 1);
        assert!(matches!(
            decode(
                &document(vec![change(&paths[0], unicode.as_bytes())]),
                &paths
            ),
            Err(DependencySourceError::LimitExceeded(_))
        ));
        let mut lying = document_at_limit.clone();
        lying["changes"][0]["bytes"] = json!(u64::MAX);
        lying["changes"][0]["content_base64"] = json!("!");
        assert_eq!(
            decode(&lying, &paths).unwrap_err(),
            DependencySourceError::LimitExceeded("per-file byte limit")
        );
        let mut encoded = document_at_limit;
        encoded["changes"][0]["bytes"] = json!(1);
        encoded["changes"][0]["content_base64"] =
            json!("A".repeat(encoded_len(MAX_TYPED_SOURCE_FILE_BYTES) + 4));
        assert!(matches!(
            decode(&encoded, &paths),
            Err(DependencySourceError::LimitExceeded(_))
        ));
    }

    #[test]
    fn total_prompt_cap_includes_paths_hashes_framing_and_utf8_bytes() {
        let paths = vec!["a.md".to_owned(), "b.md".to_owned()];
        let metadata = paths
            .iter()
            .map(|path| path.len() + 64 + FILE_FRAMING_BYTES)
            .sum::<usize>();
        let second_size = MAX_TYPED_SOURCE_PROMPT_BYTES - metadata - MAX_TYPED_SOURCE_FILE_BYTES;
        let mut at_limit = document(vec![
            change(&paths[0], &vec![b'a'; MAX_TYPED_SOURCE_FILE_BYTES]),
            change(&paths[1], &vec![b'b'; second_size]),
        ]);
        assert_eq!(decode(&at_limit, &paths).unwrap().len(), 2);
        replace_content(&mut at_limit["changes"][1], &vec![b'b'; second_size + 1]);
        // Invalid first-file base64 still has the correct encoded length: the
        // aggregate limit must be rejected before trying to decode that file.
        at_limit["changes"][0]["content_base64"] =
            json!("!".repeat(encoded_len(MAX_TYPED_SOURCE_FILE_BYTES)));
        assert_eq!(
            decode(&at_limit, &paths).unwrap_err(),
            DependencySourceError::LimitExceeded("per-parent prompt byte limit")
        );
    }

    #[test]
    fn redundant_patch_has_independent_encoded_and_decoded_caps() {
        let (mut document, paths) = fixture();
        let patch = vec![b'x'; MAX_PATCH_BYTES + 1];
        document["patch_sha256"] = json!(digest(&patch));
        document["patch_base64"] = json!(BASE64.encode(&patch));
        assert!(serde_json::to_vec(&document).unwrap().len() < MAX_TYPED_SOURCE_ENVELOPE_BYTES);
        assert!(matches!(
            decode(&document, &paths),
            Err(DependencySourceError::LimitExceeded(_))
        ));
        document["patch_base64"] = json!("A".repeat(encoded_len(MAX_PATCH_BYTES) + 4));
        assert!(matches!(
            decode(&document, &paths),
            Err(DependencySourceError::LimitExceeded(_))
        ));
    }

    #[test]
    fn an_invalid_later_file_never_returns_a_partial_handoff() {
        let (mut document, paths) = fixture();
        replace_content(&mut document["changes"][1], b"\0");
        assert_eq!(
            decode(&document, &paths).unwrap_err(),
            DependencySourceError::InvalidText
        );
    }
}
