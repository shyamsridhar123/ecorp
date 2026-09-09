//! Bounded compatibility reader for an already-stopped native Copilot session.
//! No Session is registered, resumed, created, or sent a message. This capability
//! is test-only: the pinned runtime rejected the unloaded-session read in the
//! retained real-session probe. No production command can dispatch this reader.

use super::*;
use github_copilot_sdk::copilot_request_handler::{
    CopilotHttpRequest, CopilotHttpResponse, CopilotRequestContext, CopilotRequestError,
    CopilotRequestHandler, CopilotWebSocketHandler, CopilotWebSocketResponse,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::{AtomicU64, Ordering},
};

#[path = "copilot_evidence_wire_tests.rs"]
mod wire_tests;

const PAGE_SIZE: usize = 200;
const MAX_PAGES: usize = 20;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(20);

fn rejected(reason: &'static str) -> AdapterError {
    AdapterError::Runtime(anyhow!(reason))
}

#[derive(Default)]
struct NoModelTraffic {
    denied: AtomicU64,
}

#[async_trait]
impl CopilotRequestHandler for NoModelTraffic {
    async fn send_request(
        &self,
        _request: CopilotHttpRequest,
        _context: &CopilotRequestContext,
    ) -> Result<CopilotHttpResponse, CopilotRequestError> {
        self.denied.fetch_add(1, Ordering::SeqCst);
        Err(CopilotRequestError::message(
            "Stopped-session evidence collection cannot make model-layer requests",
        ))
    }

    async fn open_websocket(
        &self,
        _context: &CopilotRequestContext,
        _response: CopilotWebSocketResponse,
    ) -> Result<Box<dyn CopilotWebSocketHandler>, CopilotRequestError> {
        self.denied.fetch_add(1, Ordering::SeqCst);
        Err(CopilotRequestError::message(
            "Stopped-session evidence collection cannot open model-layer WebSockets",
        ))
    }
}

struct ClientCleanup(Client);

impl Drop for ClientCleanup {
    fn drop(&mut self) {
        // A fallback is not an exit receipt. Success below requires awaited stop;
        // no collected evidence is returned when shutdown cannot be confirmed.
        self.0.force_stop();
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn account_matches(login: Option<&str>, expected: &str) -> bool {
    valid_digest(expected)
        && login.is_some_and(|login| {
            !login.is_empty()
                && login.len() <= 100
                && login
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && hex::encode(Sha256::digest(login.to_ascii_lowercase().as_bytes())) == expected
        })
}

async fn check_account(client: &Client, expected: &str) -> Result<(), AdapterError> {
    let auth = client
        .get_auth_status()
        .await
        .map_err(|_| rejected("Native reader account could not be checked"))?;
    if !auth.is_authenticated || !account_matches(auth.login.as_deref(), expected) {
        return Err(rejected(
            "Native reader account does not match the authorized source account",
        ));
    }
    Ok(())
}

#[derive(Default)]
struct History {
    pages: usize,
    bytes: usize,
    event_ids: BTreeSet<Uuid>,
    cursors: BTreeSet<String>,
    counts: BTreeMap<&'static str, usize>,
    digests: Vec<String>,
    session_start_matched: bool,
}

impl History {
    fn add(&mut self, page: &Value, session_id: &str) -> Result<(String, bool), AdapterError> {
        let bytes = serde_json::to_vec(page)
            .map_err(|_| rejected("Native event page is not serializable"))?;
        self.bytes = self.bytes.saturating_add(bytes.len());
        self.pages += 1;
        if self.bytes > MAX_BYTES || self.pages > MAX_PAGES {
            return Err(rejected(
                "Native history exceeds the bounded collection allowance",
            ));
        }
        let cursor = page
            .get("cursor")
            .and_then(Value::as_str)
            .filter(|cursor| !cursor.is_empty() && cursor.len() <= 4096)
            .ok_or_else(|| rejected("Native history omitted a bounded continuation cursor"))?;
        if page.get("cursorStatus").and_then(Value::as_str) != Some("ok")
            || !self.cursors.insert(cursor.to_owned())
        {
            return Err(rejected("Native history cursor is expired or repeated"));
        }
        let more = page
            .get("hasMore")
            .and_then(Value::as_bool)
            .ok_or_else(|| rejected("Native history omitted continuation status"))?;
        let events = page
            .get("events")
            .and_then(Value::as_array)
            .filter(|events| !events.is_empty() && events.len() <= PAGE_SIZE)
            .ok_or_else(|| rejected("Native history returned an empty or oversized event page"))?;
        self.add_events(events, session_id)?;
        self.digests.push(hex::encode(Sha256::digest(&bytes)));
        Ok((cursor.to_owned(), more))
    }

    fn add_events(&mut self, events: &[Value], session_id: &str) -> Result<(), AdapterError> {
        for event in events {
            let id = event
                .get("id")
                .and_then(Value::as_str)
                .and_then(|id| Uuid::parse_str(id).ok())
                .filter(|id| !id.is_nil())
                .ok_or_else(|| rejected("Native history has an invalid durable event identity"))?;
            if !self.event_ids.insert(id) {
                return Err(rejected("Native history repeated a durable event"));
            }
            let kind = event
                .get("type")
                .and_then(Value::as_str)
                .ok_or_else(|| rejected("Native history has no event type"))?;
            if kind == "session.start" {
                if event.pointer("/data/sessionId").and_then(Value::as_str) != Some(session_id) {
                    return Err(rejected("Native history belongs to a different session"));
                }
                self.session_start_matched = true;
            }
            // Never export text, tool arguments/results, arbitrary event labels,
            // local paths, credentials, or private native-profile metadata.
            let count_key = match kind {
                "session.start" => "session.start",
                "session.resume" => "session.resume",
                "session.shutdown" => "session.shutdown",
                "assistant.message" => "assistant.message",
                "assistant.turn_start" => "assistant.turn_start",
                "assistant.turn_end" => "assistant.turn_end",
                "user.message" => "user.message",
                "tool.execution_start" => "tool.execution_start",
                "tool.execution_complete" => "tool.execution_complete",
                "abort" => "abort",
                _ => "other",
            };
            *self.counts.entry(count_key).or_default() += 1;
        }
        Ok(())
    }

    fn receipt(self, session_id: &str, account: &str, pid: u32) -> Result<Value, AdapterError> {
        if !self.session_start_matched {
            return Err(rejected(
                "Native history has no matching persisted session start",
            ));
        }
        Ok(json!({
            "schema_version":1,
            "provider":"github-copilot",
            "sdk_version":"1.0.11",
            "runtime_version":SUPPORTED_COPILOT_RUNTIME,
            "method":"session.eventLog.read",
            "kind":"stopped_session_history_collection",
            "session_id":session_id,
            "account_observation_sha256":account,
            "coverage":"complete persisted event history returned by the native API",
            "pages":self.pages,
            "events":self.event_ids.len(),
            "canonical_json_bytes":self.bytes,
            "canonical_page_sha256":self.digests,
            "historical_event_counts":self.counts,
            "provider_completion_claimed":false,
            "new_prompt_sends":0,
            "new_sessions":0,
            "resumed_sessions":0,
            "registered_tools":0,
            "model_requests_forwarded":0,
            "reader_process_id":pid,
            "reader_shutdown_confirmed":true
        }))
    }
}

async fn read_history(
    client: &Client,
    session_id: &str,
    expected_account_sha256: &str,
    blocker: &NoModelTraffic,
) -> Result<History, AdapterError> {
    let status = client
        .get_status()
        .await
        .map_err(|_| rejected("Native reader runtime could not be verified"))?;
    if !runtime_version_is_supported(&status.version) {
        return Err(rejected(
            "Native history reader does not match the pinned runtime",
        ));
    }
    check_account(client, expected_account_sha256).await?;
    let mut history = History::default();
    let mut cursor = None;
    loop {
        let mut params = json!({
            "sessionId":session_id,"direction":"backward",
            "max":PAGE_SIZE,"includeEphemeral":false,"waitMs":0
        });
        if let Some(cursor) = &cursor {
            params["cursor"] = json!(cursor);
        }
        let page = client
                .call("session.eventLog.read", Some(params))
                .await
                .map_err(|error| {
                    if matches!(
                        error.kind(),
                        github_copilot_sdk::ErrorKind::Session(
                            github_copilot_sdk::SessionErrorKind::NotFound(_)
                        )
                    ) {
                        rejected("Pinned native runtime cannot read this unloaded persisted session; no resume or send was attempted")
                    } else {
                        rejected("Native persisted-history read failed; no resume or send was attempted")
                    }
                })?;
        let (next, more) = history.add(&page, session_id)?;
        if !more {
            break;
        }
        if history.pages == MAX_PAGES {
            return Err(rejected(
                "Native history is incomplete within the bounded page allowance",
            ));
        }
        cursor = Some(next);
    }
    check_account(client, expected_account_sha256).await?;
    if blocker.denied.load(Ordering::SeqCst) != 0 {
        return Err(rejected(
            "Native history read succeeded but model-layer traffic was attempted and blocked; evidence withheld",
        ));
    }
    Ok(history)
}

/// A separately selected diagnostic for the other public SDK timeline API.
/// This is never a fallback from eventLog.read, and never loads a Session.
async fn read_timeline(
    client: &Client,
    session_id: &str,
    expected_account_sha256: &str,
    blocker: &NoModelTraffic,
) -> Result<History, AdapterError> {
    let status = client
        .get_status()
        .await
        .map_err(|_| rejected("Native timeline runtime could not be verified"))?;
    if !runtime_version_is_supported(&status.version) {
        return Err(rejected(
            "Native timeline reader does not match the pinned runtime",
        ));
    }
    check_account(client, expected_account_sha256).await?;
    let response = client
        .call("session.getMessages", Some(json!({"sessionId":session_id})))
        .await
        .map_err(|error| {
            if matches!(
                error.kind(),
                github_copilot_sdk::ErrorKind::Session(
                    github_copilot_sdk::SessionErrorKind::NotFound(_)
                )
            ) {
                rejected("Pinned native timeline API cannot read this unloaded persisted session; no resume or send was attempted")
            } else {
                rejected("Native timeline read failed; no resume or send was attempted")
            }
        })?;
    let bytes = serde_json::to_vec(&response)
        .map_err(|_| rejected("Native timeline response is not serializable"))?;
    let events = response
        .get("events")
        .and_then(Value::as_array)
        .filter(|events| !events.is_empty() && events.len() <= PAGE_SIZE * MAX_PAGES)
        .ok_or_else(|| rejected("Native timeline has no bounded nonempty event array"))?;
    if bytes.len() > MAX_BYTES {
        return Err(rejected(
            "Native timeline exceeds the diagnostic byte bound",
        ));
    }
    let mut history = History {
        pages: 1,
        bytes: bytes.len(),
        digests: vec![hex::encode(Sha256::digest(&bytes))],
        ..History::default()
    };
    history.add_events(events, session_id)?;
    check_account(client, expected_account_sha256).await?;
    if blocker.denied.load(Ordering::SeqCst) != 0 {
        return Err(rejected(
            "Native timeline read succeeded but model-layer traffic was attempted and blocked; evidence withheld",
        ));
    }
    Ok(history)
}

pub(super) async fn collect(
    adapter: &CopilotSdkAdapter,
    workspace: &Path,
    session_id: &str,
    expected_account_sha256: &str,
    read: crate::adapter::StoppedSessionRead,
) -> Result<Value, AdapterError> {
    if adapter.config.fixture
        || adapter.config.external_host.is_some()
        || adapter.profile.is_none()
        || !workspace.is_absolute()
        || !workspace.is_dir()
        || !valid_digest(expected_account_sha256)
        || !Uuid::parse_str(session_id).is_ok_and(|id| !id.is_nil())
    {
        return Err(rejected(
            "Native history requires an exact local saved-account scope",
        ));
    }
    let blocker = Arc::new(NoModelTraffic::default());
    let options = adapter
        .client_options(workspace)
        .await?
        .with_transport(Transport::Stdio)
        .with_request_handler(blocker.clone())
        .with_log_level(LogLevel::Error);
    // No Session API is called, so the SDK has no registered tool, permission,
    // or filesystem handlers. Native model traffic is rejected by both hooks.
    let client =
        ClientCleanup(Client::start(options).await.map_err(|_| {
            rejected("Native history reader startup failed; no evidence collected")
        })?);
    let pid = client
        .0
        .pid()
        .ok_or_else(|| rejected("Native reader has no owned local process"))?;
    let result = tokio::time::timeout(READ_TIMEOUT, async {
        match read {
            crate::adapter::StoppedSessionRead::EventLog => {
                read_history(&client.0, session_id, expected_account_sha256, &blocker).await
            }
            crate::adapter::StoppedSessionRead::Timeline => {
                read_timeline(&client.0, session_id, expected_account_sha256, &blocker).await
            }
        }
    })
    .await
    .unwrap_or_else(|_| Err(rejected("Native history read exceeded its time allowance")));
    let stopped = tokio::time::timeout(Duration::from_secs(15), client.0.stop()).await;
    if !matches!(stopped, Ok(Ok(()))) {
        return Err(rejected(
            "Native reader shutdown was not confirmed; evidence withheld",
        ));
    }
    let mut receipt = result?.receipt(session_id, expected_account_sha256, pid)?;
    if matches!(read, crate::adapter::StoppedSessionRead::Timeline) {
        receipt["method"] = json!("session.getMessages");
        receipt["coverage"] =
            json!("timeline returned by the native API; not a full event-log claim");
        receipt["canonical_response_sha256"] = receipt
            .as_object_mut()
            .unwrap()
            .remove("canonical_page_sha256")
            .unwrap();
    }
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(session: Uuid) -> Value {
        json!({"cursor":"native-opaque-cursor","cursorStatus":"ok","hasMore":false,"events":[
            {"id":Uuid::new_v4(),"type":"session.start","data":{"sessionId":session}},
            {"id":Uuid::new_v4(),"type":"assistant.message","data":{"content":"PRIVATE-PROMPT-AND-SECRET"}},
            {"id":Uuid::new_v4(),"type":"tool.execution_complete","data":{"result":"PRIVATE-TOOL-RESULT"}}
        ]})
    }

    #[test]
    fn issue211_native_history_receipt_is_redacted_and_never_claims_provider_completion() {
        let session = Uuid::new_v4();
        let mut history = History::default();
        assert!(!history.add(&page(session), &session.to_string()).unwrap().1);
        let receipt = history
            .receipt(&session.to_string(), &"a".repeat(64), 123)
            .unwrap();
        let serialized = serde_json::to_string(&receipt).unwrap();
        assert!(!serialized.contains("PRIVATE"));
        assert_eq!(receipt["events"], 3);
        assert_eq!(receipt["provider_completion_claimed"], false);
        assert_eq!(receipt["new_prompt_sends"], 0);
        assert_eq!(receipt["resumed_sessions"], 0);
        assert_eq!(receipt["model_requests_forwarded"], 0);
    }

    #[test]
    fn issue211_native_history_rejects_foreign_missing_or_repeated_identity() {
        let session = Uuid::new_v4();
        let valid = page(session);
        assert!(
            History::default()
                .add(&valid, &Uuid::new_v4().to_string())
                .is_err()
        );
        let mut history = History::default();
        history.add(&valid, &session.to_string()).unwrap();
        assert!(history.add(&valid, &session.to_string()).is_err());
        for field in ["cursor", "cursorStatus", "hasMore", "events"] {
            let mut damaged = valid.clone();
            damaged.as_object_mut().unwrap().remove(field);
            assert!(
                History::default()
                    .add(&damaged, &session.to_string())
                    .is_err()
            );
        }
        let mut expired = valid;
        expired["cursorStatus"] = json!("expired");
        assert!(
            History::default()
                .add(&expired, &session.to_string())
                .is_err()
        );
    }

    #[test]
    fn issue211_native_history_and_account_bounds_fail_closed() {
        let session = Uuid::new_v4();
        let mut history = History {
            pages: MAX_PAGES,
            ..History::default()
        };
        assert!(history.add(&page(session), &session.to_string()).is_err());
        let mut history = History {
            bytes: MAX_BYTES,
            ..History::default()
        };
        assert!(history.add(&page(session), &session.to_string()).is_err());
        let account = hex::encode(Sha256::digest(b"alice"));
        assert!(account_matches(Some("Alice"), &account));
        assert!(!account_matches(Some("bob"), &account));
        assert!(!account_matches(None, &account));
        assert!(!account_matches(Some("alice"), "invalid"));
        assert!(
            History::default()
                .receipt(&session.to_string(), &account, 1)
                .is_err()
        );
    }
}
