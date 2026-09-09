//! Deterministic wire coverage of the pinned SDK's public External transport.
//! Framing/startup follow github-copilot-sdk 1.0.11's
//! tests/builtin_plugin_directories_test.rs; inference callbacks follow its
//! generated api_types/rpc and CopilotRequestDispatcher wire contract.
//! No CLI, credentials, native home, workspace writes, or real provider is used.
//! The only recorded RPC data are SDK-to-fixture method names.
#![cfg(test)]

use super::*;
use std::{collections::VecDeque, net::SocketAddr, sync::Mutex};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};

const WIRE_SESSION: &str = "00000000-0000-4000-8000-000000000211";
const WIRE_LOGIN: &str = "fixture-reader";
const WIRE_TIMEOUT: Duration = Duration::from_secs(10);
const NATIVE_START_ID: u64 = 900_001;
const NATIVE_CHUNK_ID: u64 = 900_002;
const NATIVE_REQUEST_ID: &str = "synthetic-forbidden-inference";

enum WireReply {
    Page(Value),
    MissingSession,
}

#[derive(Clone, Copy)]
enum NativeTransport {
    Http,
    WebSocket,
}

struct NativeAttempt {
    transport: NativeTransport,
    destination: SocketAddr,
}

struct WireScript {
    replies: VecDeque<WireReply>,
    logins: VecDeque<Option<&'static str>>,
    runtime_version: &'static str,
    native_attempt: Option<NativeAttempt>,
}

impl WireScript {
    fn pages(pages: Vec<Value>) -> Self {
        Self {
            replies: pages.into_iter().map(WireReply::Page).collect(),
            logins: VecDeque::from([Some(WIRE_LOGIN), Some(WIRE_LOGIN)]),
            runtime_version: SUPPORTED_COPILOT_RUNTIME,
            native_attempt: None,
        }
    }
}

fn wire_event(id: u128, kind: &str) -> Value {
    let data = if kind == "session.start" {
        json!({"sessionId": WIRE_SESSION})
    } else {
        json!({})
    };
    json!({"id": Uuid::from_u128(id), "type": kind, "data": data})
}

fn wire_page(cursor: &str, more: bool, events: Vec<Value>) -> Value {
    json!({"cursor": cursor, "cursorStatus": "ok", "hasMore": more, "events": events})
}

fn complete_page() -> Value {
    wire_page(
        "opaque/oldest:done",
        false,
        vec![wire_event(1, "session.start")],
    )
}

fn account_digest() -> String {
    hex::encode(Sha256::digest(WIRE_LOGIN.as_bytes()))
}

async fn read_framed(reader: &mut (impl AsyncRead + Unpin)) -> std::io::Result<Option<Value>> {
    let mut header = String::new();
    loop {
        let mut byte = [0_u8; 1];
        match reader.read_exact(&mut byte).await {
            Ok(_) => header.push(char::from(byte[0])),
            Err(error)
                if error.kind() == std::io::ErrorKind::UnexpectedEof && header.is_empty() =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        }
        if header.ends_with("\r\n\r\n") {
            break;
        }
        if header.len() > 1_024 {
            return Err(std::io::Error::other(
                "synthetic RPC header exceeded its bound",
            ));
        }
    }
    let length = header
        .trim()
        .strip_prefix("Content-Length: ")
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|length| *length <= 64 * 1_024)
        .ok_or_else(|| std::io::Error::other("invalid synthetic RPC request length"))?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body).await?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(std::io::Error::other)
}

async fn write_framed(writer: &mut (impl AsyncWrite + Unpin), frame: Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(&frame).map_err(std::io::Error::other)?;
    writer
        .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
        .await?;
    writer.write_all(&body).await?;
    writer.flush().await
}

async fn result(
    writer: &mut (impl AsyncWrite + Unpin),
    id: Value,
    value: Value,
) -> std::io::Result<()> {
    write_framed(writer, json!({"jsonrpc": "2.0", "id": id, "result": value})).await
}

async fn reply(
    writer: &mut (impl AsyncWrite + Unpin),
    id: Value,
    value: WireReply,
) -> std::io::Result<()> {
    match value {
        WireReply::Page(page) => result(writer, id, page).await,
        WireReply::MissingSession => {
            write_framed(
                writer,
                json!({"jsonrpc": "2.0", "id": id,
                    "error": {"code": -32000, "message": "Session not found"}}),
            )
            .await
        }
    }
}

fn assert_fixed_read(params: &Value, cursor: Option<&str>) {
    // Inspect transient synthetic parameters; never retain or print them.
    assert!(params.get("sessionId").and_then(Value::as_str) == Some(WIRE_SESSION));
    assert!(params.get("direction").and_then(Value::as_str) == Some("backward"));
    assert!(params.get("max").and_then(Value::as_u64) == Some(PAGE_SIZE as u64));
    assert!(params.get("includeEphemeral").and_then(Value::as_bool) == Some(false));
    assert!(params.get("waitMs").and_then(Value::as_u64) == Some(0));
    assert!(params.get("cursor").and_then(Value::as_str) == cursor);
    assert!(
        params.as_object().map(|params| params.len()) == Some(if cursor.is_some() { 6 } else { 5 })
    );
}

async fn send_native_attempt(
    writer: &mut (impl AsyncWrite + Unpin),
    attempt: &NativeAttempt,
) -> std::io::Result<()> {
    let (transport, scheme, method) = match attempt.transport {
        NativeTransport::Http => ("http", "http", "POST"),
        NativeTransport::WebSocket => ("websocket", "ws", "GET"),
    };
    write_framed(
        writer,
        json!({"jsonrpc": "2.0", "id": NATIVE_START_ID,
        "method": "llmInference.httpRequestStart",
        "params": {
            "requestId": NATIVE_REQUEST_ID, "sessionId": WIRE_SESSION,
            "method": method, "url": format!("{scheme}://{}/synthetic", attempt.destination),
            "headers": {}, "transport": transport
        }}),
    )
    .await?;
    if matches!(attempt.transport, NativeTransport::Http) {
        // HTTP dispatch awaits an end-marked body before invoking send_request.
        write_framed(
            writer,
            json!({"jsonrpc": "2.0", "id": NATIVE_CHUNK_ID,
                "method": "llmInference.httpRequestChunk",
                "params": {"requestId": NATIVE_REQUEST_ID, "data": "", "end": true}}),
        )
        .await?;
    }
    Ok(())
}

async fn serve_wire(
    listener: TcpListener,
    mut script: WireScript,
    methods: Arc<Mutex<Vec<String>>>,
) -> std::io::Result<()> {
    let (stream, _) = listener.accept().await?;
    stream.set_nodelay(true)?;
    let (mut reader, mut writer) = tokio::io::split(stream);
    let mut cursor = None::<String>;
    let mut pending_read = None::<(Value, WireReply)>;
    let mut native_transport = None;
    let mut response_started = false;
    while let Some(frame) = read_framed(&mut reader).await? {
        let id = frame
            .get("id")
            .cloned()
            .expect("synthetic RPC omitted its ID");
        let Some(method) = frame.get("method").and_then(Value::as_str) else {
            // Native callback acknowledgements are not method calls or evidence.
            assert!(
                id.as_u64()
                    .is_some_and(|id| { id == NATIVE_START_ID || id == NATIVE_CHUNK_ID })
            );
            assert!(frame.get("error").is_none_or(Value::is_null));
            assert!(frame.get("result").is_some());
            continue;
        };
        methods.lock().unwrap().push(method.to_owned());
        match method {
            "connect" => {
                result(
                    &mut writer,
                    id,
                    json!({"ok": true, "protocolVersion": 3, "version": "synthetic"}),
                )
                .await?;
            }
            "llmInference.setProvider" => {
                result(&mut writer, id, json!({"success": true})).await?;
            }
            "status.get" => {
                result(
                    &mut writer,
                    id,
                    json!({"version": script.runtime_version, "protocolVersion": 3}),
                )
                .await?;
            }
            "auth.getStatus" => {
                let login = script
                    .logins
                    .pop_front()
                    .expect("unexpected extra account read");
                result(
                    &mut writer,
                    id,
                    json!({"isAuthenticated": login.is_some(), "login": login}),
                )
                .await?;
            }
            "session.eventLog.read" => {
                assert_fixed_read(&frame["params"], cursor.as_deref());
                let page = script
                    .replies
                    .pop_front()
                    .expect("unexpected extra history read");
                if let WireReply::Page(value) = &page {
                    cursor = value
                        .get("cursor")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                }
                if let Some(attempt) = script.native_attempt.take() {
                    assert!(pending_read.is_none());
                    native_transport = Some(attempt.transport);
                    pending_read = Some((id, page));
                    send_native_attempt(&mut writer, &attempt).await?;
                } else {
                    reply(&mut writer, id, page).await?;
                }
            }
            "session.getMessages" => {
                assert!(
                    frame["params"].get("sessionId").and_then(Value::as_str) == Some(WIRE_SESSION)
                );
                assert!(frame["params"].as_object().map(|params| params.len()) == Some(1));
                let page = script
                    .replies
                    .pop_front()
                    .expect("unexpected extra timeline read");
                reply(&mut writer, id, page).await?;
            }
            "llmInference.httpResponseStart" => {
                assert!(pending_read.is_some());
                assert!(!response_started);
                assert!(
                    frame["params"].get("requestId").and_then(Value::as_str)
                        == Some(NATIVE_REQUEST_ID)
                );
                // SDK 1.0.11 sends its WS upgrade acknowledgement *before*
                // calling open_websocket. A synthetic 101 is not forwarding.
                let expected_status = match native_transport {
                    Some(NativeTransport::Http) => 502,
                    Some(NativeTransport::WebSocket) => 101,
                    None => panic!("unrequested inference response"),
                };
                assert!(
                    frame["params"].get("status").and_then(Value::as_i64) == Some(expected_status)
                );
                response_started = true;
                result(&mut writer, id, json!({"accepted": true})).await?;
            }
            "llmInference.httpResponseChunk" => {
                assert!(response_started);
                assert!(
                    frame["params"].get("requestId").and_then(Value::as_str)
                        == Some(NATIVE_REQUEST_ID)
                );
                assert!(frame["params"].get("end").and_then(Value::as_bool) == Some(true));
                assert!(frame["params"].get("data").and_then(Value::as_str) == Some(""));
                assert!(
                    frame["params"]
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .is_some_and(|message| message
                            .contains("Stopped-session evidence collection cannot"))
                );
                result(&mut writer, id, json!({"accepted": true})).await?;
                // Release the history call only after the real SDK routed the
                // attempted request through a deny hook and reported its error.
                let (read_id, page) = pending_read.take().expect("unrequested response chunk");
                reply(&mut writer, read_id, page).await?;
            }
            _ => {
                write_framed(
                    &mut writer,
                    json!({"jsonrpc": "2.0", "id": id,
                        "error": {"code": -32601, "message": "Forbidden synthetic RPC method"}}),
                )
                .await?;
            }
        }
    }
    Ok(())
}

struct WireFixture {
    client: Client,
    blocker: Arc<NoModelTraffic>,
    methods: Arc<Mutex<Vec<String>>>,
    server: JoinHandle<std::io::Result<()>>,
}

impl WireFixture {
    async fn start(script: WireScript) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let methods = Arc::new(Mutex::new(Vec::new()));
        let server = tokio::spawn(serve_wire(listener, script, methods.clone()));
        let blocker = Arc::new(NoModelTraffic::default());
        // The SDK resolves program even for External. Its own external fixture
        // supplies current_exe to avoid native binary resolution/extraction.
        // External never executes this path; the SDK connects only to our TCP peer.
        let options = ClientOptions::new()
            .with_program(CliProgram::Path(std::env::current_exe().unwrap()))
            .with_transport(Transport::External {
                host: address.ip().to_string(),
                port: address.port(),
                connection_token: None,
            })
            .with_request_handler(blocker.clone());
        let client = match tokio::time::timeout(WIRE_TIMEOUT, Client::start(options)).await {
            Ok(Ok(client)) => client,
            _ => {
                server.abort();
                let _ = server.await;
                panic!("synthetic SDK startup did not complete");
            }
        };
        let fixture = Self {
            client,
            blocker,
            methods,
            server,
        };
        assert!(
            fixture.client.pid().is_none(),
            "External transport must not own a CLI process"
        );
        fixture
    }

    async fn read(&self) -> Result<History, AdapterError> {
        let expected_account = account_digest();
        tokio::time::timeout(
            WIRE_TIMEOUT,
            read_history(&self.client, WIRE_SESSION, &expected_account, &self.blocker),
        )
        .await
        .expect("synthetic history read exceeded its test timeout")
    }

    async fn finish(mut self) -> Vec<String> {
        tokio::time::timeout(WIRE_TIMEOUT, self.client.stop())
            .await
            .expect("synthetic SDK shutdown timed out")
            .expect("synthetic SDK shutdown failed");
        // External has no child to stop. Explicitly close just this synthetic
        // transport so the fixture peer can observe EOF and be joined.
        self.client.force_stop();
        tokio::time::timeout(WIRE_TIMEOUT, &mut self.server)
            .await
            .expect("synthetic RPC peer did not close")
            .expect("synthetic RPC peer panicked")
            .expect("synthetic RPC peer failed");
        self.methods.lock().unwrap().clone()
    }
}

impl Drop for WireFixture {
    fn drop(&mut self) {
        self.client.force_stop();
        self.server.abort();
    }
}

fn expected_reads(page_count: usize, checked_after: bool) -> Vec<String> {
    let mut expected = [
        "connect",
        "llmInference.setProvider",
        "status.get",
        "auth.getStatus",
    ]
    .map(str::to_owned)
    .to_vec();
    expected.extend((0..page_count).map(|_| "session.eventLog.read".to_owned()));
    if checked_after {
        expected.push("auth.getStatus".to_owned());
    }
    expected
}

fn assert_read_only_methods(methods: &[String]) {
    assert!(
        methods.iter().all(|method| matches!(
            method.as_str(),
            "connect"
                | "llmInference.setProvider"
                | "status.get"
                | "auth.getStatus"
                | "session.eventLog.read"
                | "session.getMessages"
                | "llmInference.httpResponseStart"
                | "llmInference.httpResponseChunk"
        )),
        "wire trace contained a session mutation, tool registration, or other unexpected RPC"
    );
}

fn rejected_history(outcome: Result<History, AdapterError>) -> AdapterError {
    match outcome {
        Ok(_) => panic!("untrusted synthetic history was accepted"),
        Err(error) => error,
    }
}

#[tokio::test]
async fn issue211_native_wire_uses_only_fixed_paginated_reads() {
    let fixture = WireFixture::start(WireScript::pages(vec![
        wire_page(
            "opaque/newest:continue",
            true,
            vec![wire_event(2, "assistant.message"), wire_event(3, "abort")],
        ),
        complete_page(),
    ]))
    .await;
    let history = fixture.read().await.unwrap();
    assert_eq!(history.pages, 2);
    assert_eq!(history.event_ids.len(), 3);
    assert!(history.session_start_matched);
    assert_eq!(fixture.blocker.denied.load(Ordering::SeqCst), 0);
    let methods = fixture.finish().await;
    assert_read_only_methods(&methods);
    assert_eq!(methods, expected_reads(2, true));
}

#[tokio::test]
async fn issue211_native_wire_session_not_found_has_no_resume_or_send_fallback() {
    let mut script = WireScript::pages(Vec::new());
    script.replies.push_back(WireReply::MissingSession);
    let fixture = WireFixture::start(script).await;
    let error = rejected_history(fixture.read().await);
    assert!(error.to_string().contains("unloaded persisted session"));
    assert!(
        error
            .to_string()
            .contains("no resume or send was attempted")
    );
    let methods = fixture.finish().await;
    assert_read_only_methods(&methods);
    assert_eq!(methods, expected_reads(1, false));
}

#[tokio::test]
async fn issue211_native_wire_timeline_not_found_has_no_automatic_fallback() {
    let mut script = WireScript::pages(Vec::new());
    script.replies.push_back(WireReply::MissingSession);
    let fixture = WireFixture::start(script).await;
    let expected_account = account_digest();
    let outcome = tokio::time::timeout(
        WIRE_TIMEOUT,
        read_timeline(
            &fixture.client,
            WIRE_SESSION,
            &expected_account,
            &fixture.blocker,
        ),
    )
    .await
    .expect("synthetic explicitly selected timeline read timed out");
    let error = rejected_history(outcome);
    assert!(error.to_string().contains("timeline"));
    assert!(error.to_string().contains("unloaded persisted session"));
    assert!(
        error
            .to_string()
            .contains("no resume or send was attempted")
    );
    let methods = fixture.finish().await;
    assert_read_only_methods(&methods);
    assert_eq!(
        methods,
        [
            "connect",
            "llmInference.setProvider",
            "status.get",
            "auth.getStatus",
            "session.getMessages",
        ]
    );
}

#[tokio::test]
async fn issue211_native_wire_rejects_malformed_expired_duplicate_and_oversize_pages() {
    let mut malformed = complete_page();
    malformed["hasMore"] = json!("not-a-boolean");
    let mut expired = complete_page();
    expired["cursorStatus"] = json!("expired");
    let duplicate_cursor = vec![
        wire_page(
            "same-cursor",
            true,
            vec![wire_event(2, "assistant.message")],
        ),
        wire_page("same-cursor", false, vec![wire_event(1, "session.start")]),
    ];
    let duplicate_event = vec![
        wire_page(
            "first-cursor",
            true,
            vec![wire_event(2, "assistant.message")],
        ),
        wire_page(
            "last-cursor",
            false,
            vec![wire_event(2, "assistant.message")],
        ),
    ];
    let oversized_page = wire_page(
        "too-many-events",
        false,
        (1..=PAGE_SIZE + 1)
            .map(|id| wire_event(id as u128, "assistant.message"))
            .collect(),
    );
    for (pages, expected_error) in [
        (vec![malformed], "continuation status"),
        (vec![expired], "cursor is expired or repeated"),
        (duplicate_cursor, "cursor is expired or repeated"),
        (duplicate_event, "repeated a durable event"),
        (vec![oversized_page], "oversized event page"),
    ] {
        let reads = pages.len();
        let fixture = WireFixture::start(WireScript::pages(pages)).await;
        let error = rejected_history(fixture.read().await);
        assert!(error.to_string().contains(expected_error));
        let methods = fixture.finish().await;
        assert_read_only_methods(&methods);
        assert_eq!(methods, expected_reads(reads, false));
    }
}

#[tokio::test]
async fn issue211_native_wire_enforces_total_history_bytes_and_page_limit() {
    let mut oversized_bytes = complete_page();
    oversized_bytes["events"][0]["data"]["syntheticPadding"] = json!("x".repeat(MAX_BYTES));
    let fixture = WireFixture::start(WireScript::pages(vec![oversized_bytes])).await;
    let error = rejected_history(fixture.read().await);
    assert!(error.to_string().contains("bounded collection allowance"));
    assert_eq!(fixture.finish().await, expected_reads(1, false));

    let pages = (0..MAX_PAGES)
        .map(|index| {
            wire_page(
                &format!("opaque-page-{index}"),
                true,
                vec![wire_event(index as u128 + 1, "assistant.message")],
            )
        })
        .collect();
    let fixture = WireFixture::start(WireScript::pages(pages)).await;
    let error = rejected_history(fixture.read().await);
    assert!(
        error
            .to_string()
            .contains("incomplete within the bounded page allowance")
    );
    let methods = fixture.finish().await;
    assert_read_only_methods(&methods);
    assert_eq!(methods, expected_reads(MAX_PAGES, false));
}

#[tokio::test]
async fn issue211_native_wire_rechecks_account_after_history_without_fallback() {
    let mut script = WireScript::pages(vec![complete_page()]);
    script.logins = VecDeque::from([Some(WIRE_LOGIN), Some("another-fixture-reader")]);
    let fixture = WireFixture::start(script).await;
    let error = rejected_history(fixture.read().await);
    assert!(
        error
            .to_string()
            .contains("does not match the authorized source account")
    );
    let methods = fixture.finish().await;
    assert_read_only_methods(&methods);
    assert_eq!(methods, expected_reads(1, true));
}

#[tokio::test]
async fn issue211_native_wire_rejects_unpinned_runtime_before_history_or_auth_reads() {
    let mut script = WireScript::pages(vec![complete_page()]);
    script.runtime_version = "0.0.0-synthetic-unsupported";
    let fixture = WireFixture::start(script).await;
    let error = rejected_history(fixture.read().await);
    assert!(
        error
            .to_string()
            .contains("does not match the pinned runtime")
    );
    let methods = fixture.finish().await;
    assert_read_only_methods(&methods);
    assert_eq!(
        methods,
        ["connect", "llmInference.setProvider", "status.get"]
    );
}

struct DownstreamTrap {
    address: SocketAddr,
    contacted: Arc<AtomicU64>,
    task: JoinHandle<()>,
}

impl DownstreamTrap {
    async fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let contacted = Arc::new(AtomicU64::new(0));
        let count = contacted.clone();
        let (ready, started) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = ready.send(());
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                count.fetch_add(1, Ordering::SeqCst);
                // Count and close only. Never serve a model or retain bytes.
                drop(stream);
            }
        });
        started.await.unwrap();
        Self {
            address,
            contacted,
            task,
        }
    }

    async fn finish(mut self) -> u64 {
        self.task.abort();
        let _ = (&mut self.task).await;
        self.contacted.load(Ordering::SeqCst)
    }
}

impl Drop for DownstreamTrap {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test]
async fn issue211_native_wire_http_and_websocket_hooks_deny_without_downstream_contact() {
    for transport in [NativeTransport::Http, NativeTransport::WebSocket] {
        let downstream = DownstreamTrap::start().await;
        let mut script = WireScript::pages(vec![complete_page()]);
        script.native_attempt = Some(NativeAttempt {
            transport,
            destination: downstream.address,
        });
        let fixture = WireFixture::start(script).await;
        let error = rejected_history(fixture.read().await);
        assert!(error.to_string().contains("model-layer traffic"));
        assert_eq!(fixture.blocker.denied.load(Ordering::SeqCst), 1);
        let methods = fixture.finish().await;
        assert_read_only_methods(&methods);
        assert_eq!(
            methods,
            [
                "connect",
                "llmInference.setProvider",
                "status.get",
                "auth.getStatus",
                "session.eventLog.read",
                "llmInference.httpResponseStart",
                "llmInference.httpResponseChunk",
                "auth.getStatus",
            ]
        );
        assert_eq!(downstream.finish().await, 0);
    }
}
