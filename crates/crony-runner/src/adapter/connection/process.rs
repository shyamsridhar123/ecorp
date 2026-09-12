use std::{process::Stdio, time::Duration};

use serde_json::Value;
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStderr, ChildStdin, ChildStdout, Command},
    time::Instant,
};

use super::{ProbeError, ProbeResult};
use crate::adapter::process_tree::{OwnedProcessTree, OwnedProcessTreeSpawn};

const LINE_LIMIT: usize = 64 * 1024;
const OUTPUT_LIMIT: usize = 1024 * 1024;
const FRAME_LIMIT: usize = 2048;

pub(super) enum OutputLine {
    Stdout(String),
    Stderr(String),
}

pub(super) struct BoundedLines<R> {
    reader: R,
    pending: Vec<u8>,
    eof: bool,
}

impl<R: AsyncBufRead + Unpin> BoundedLines<R> {
    pub(super) fn new(reader: R) -> Self {
        Self {
            reader,
            pending: Vec::new(),
            eof: false,
        }
    }

    pub(super) async fn next(&mut self) -> ProbeResult<Option<String>> {
        if self.eof {
            return Ok(None);
        }
        loop {
            let available = self.reader.fill_buf().await.map_err(ProbeError::io)?;
            if available.is_empty() {
                self.eof = true;
                if self.pending.is_empty() {
                    return Ok(None);
                }
                return self.take_line().map(Some);
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let count = newline.map_or(available.len(), |index| index + 1);
            if self.pending.len() + count > LINE_LIMIT {
                return Err(ProbeError::Incompatible(
                    "Native output exceeded its frame bound.",
                ));
            }
            self.pending.extend_from_slice(&available[..count]);
            self.reader.consume(count);
            if newline.is_some() {
                return self.take_line().map(Some);
            }
        }
    }

    fn take_line(&mut self) -> ProbeResult<String> {
        let bytes = std::mem::take(&mut self.pending);
        String::from_utf8(bytes)
            .map(|line| line.trim_end_matches(['\r', '\n']).to_owned())
            .map_err(|_| ProbeError::Incompatible("Native output was not UTF-8."))
    }

    async fn chunk(&mut self) -> ProbeResult<Option<String>> {
        let available = self.reader.fill_buf().await.map_err(ProbeError::io)?;
        if available.is_empty() {
            self.eof = true;
            return Ok(None);
        }
        let count = available.len().min(LINE_LIMIT);
        // Login parsing only uses ASCII URL/code/prompt fields. Terminal glyphs
        // may span pipe reads and must not prevent recognizing a no-newline prompt.
        let text = String::from_utf8_lossy(&available[..count]).into_owned();
        self.reader.consume(count);
        Ok(Some(text))
    }
}

/// No provider turns are sent by this transport. Cancellation drops the
/// Windows Job Object (kill-on-close); a reported success additionally requires
/// terminate_and_wait to have verified the entire owned process scope.
pub(super) struct SetupProcess {
    tree: OwnedProcessTree,
    stdin: Option<ChildStdin>,
    stdout: BoundedLines<BufReader<ChildStdout>>,
    stderr: BoundedLines<BufReader<ChildStderr>>,
    bytes: usize,
    frames: usize,
}

impl SetupProcess {
    pub(super) async fn spawn(command: Command) -> ProbeResult<Self> {
        let mut tree = spawn_owned(command).await?;
        let (stdin, stdout, stderr) = {
            let child = tree.child_mut();
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        };
        match (stdin, stdout, stderr) {
            (Some(stdin), Some(stdout), Some(stderr)) => Ok(Self {
                tree,
                stdin: Some(stdin),
                stdout: BoundedLines::new(BufReader::new(stdout)),
                stderr: BoundedLines::new(BufReader::new(stderr)),
                bytes: 0,
                frames: 0,
            }),
            _ => {
                let _ = tree.terminate_and_wait().await;
                Err(ProbeError::Failed(
                    "Native standard streams are unavailable.",
                ))
            }
        }
    }

    pub(super) async fn send(&mut self, frame: &Value) -> ProbeResult<()> {
        self.send_before(frame, Instant::now() + Duration::from_secs(2))
            .await
    }

    pub(super) async fn send_before(
        &mut self,
        frame: &Value,
        deadline: Instant,
    ) -> ProbeResult<()> {
        if Instant::now() >= deadline {
            return Err(ProbeError::Timeout);
        }
        let encoded = serde_json::to_vec(frame)
            .map_err(|_| ProbeError::Failed("Cannot encode native control request."))?;
        let stdin = self
            .stdin
            .as_mut()
            .ok_or(ProbeError::Failed("Native input is closed."))?;
        tokio::time::timeout_at(
            deadline.min(Instant::now() + Duration::from_secs(2)),
            async {
                stdin.write_all(&encoded).await?;
                stdin.write_all(b"\n").await?;
                stdin.flush().await
            },
        )
        .await
        .map_err(|_| ProbeError::Timeout)?
        .map_err(ProbeError::io)
    }

    pub(super) async fn next(&mut self, deadline: Instant) -> ProbeResult<Option<OutputLine>> {
        self.next_output(deadline, false).await
    }

    pub(super) async fn next_login(
        &mut self,
        deadline: Instant,
    ) -> ProbeResult<Option<OutputLine>> {
        self.next_output(deadline, true).await
    }

    async fn next_output(
        &mut self,
        deadline: Instant,
        chunks: bool,
    ) -> ProbeResult<Option<OutputLine>> {
        loop {
            if self.stdout.eof && self.stderr.eof {
                return Ok(None);
            }
            let line = tokio::select! {
                _ = tokio::time::sleep_until(deadline) => return Err(ProbeError::Timeout),
                line = async {
                    if chunks { self.stdout.chunk().await } else { self.stdout.next().await }
                }, if !self.stdout.eof => line?.map(OutputLine::Stdout),
                line = async {
                    if chunks { self.stderr.chunk().await } else { self.stderr.next().await }
                }, if !self.stderr.eof => line?.map(OutputLine::Stderr),
            };
            if let Some(line) = line {
                let length = match &line {
                    OutputLine::Stdout(text) | OutputLine::Stderr(text) => text.len(),
                };
                self.bytes += length;
                self.frames += 1;
                if self.bytes > OUTPUT_LIMIT || self.frames > FRAME_LIMIT {
                    return Err(ProbeError::Incompatible(
                        "Native output exceeded its operation bound.",
                    ));
                }
                return Ok(Some(line));
            }
        }
    }

    pub(super) async fn next_json(&mut self, deadline: Instant) -> ProbeResult<Value> {
        loop {
            match self.next(deadline).await? {
                Some(OutputLine::Stdout(line)) if !line.trim().is_empty() => {
                    return serde_json::from_str(&line).map_err(|_| {
                        ProbeError::Incompatible("Native control output was not JSON.")
                    });
                }
                Some(_) => {}
                None => return Err(ProbeError::Offline),
            }
        }
    }

    pub(super) async fn rpc(
        &mut self,
        method: &str,
        id: u64,
        params: Value,
        deadline: Instant,
    ) -> ProbeResult<Value> {
        self.send_before(
            &serde_json::json!({"id":id,"method":method,"params":params}),
            deadline,
        )
        .await?;
        loop {
            let frame = self.next_json(deadline).await?;
            if frame.get("id").and_then(Value::as_u64) == Some(id) && frame.get("method").is_none()
            {
                if frame.get("error").is_some() {
                    // Never propagate native error bodies (which may contain credentials).
                    return Err(ProbeError::Incompatible(
                        "The native control request was rejected.",
                    ));
                }
                return frame.get("result").cloned().ok_or(ProbeError::Incompatible(
                    "Native control response omitted its result.",
                ));
            }
            if frame.get("id").is_some() {
                return Err(ProbeError::Incompatible(
                    "Unexpected native control request or response.",
                ));
            }
        }
    }

    pub(super) async fn wait_success(&mut self, deadline: Instant) -> ProbeResult<bool> {
        loop {
            if let Some(status) = self.tree.try_wait_root().map_err(ProbeError::io)? {
                return Ok(status.success());
            }
            if Instant::now() >= deadline {
                return Err(ProbeError::Timeout);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// This is called ONLY by the native Claude auth-login driver after its
    /// specific paste-code prompt. No SDK/model/control-frame caller uses it.
    pub(super) async fn send_authorization_code(
        &mut self,
        code: String,
        deadline: Instant,
    ) -> ProbeResult<()> {
        if !super::catalog::authorization_code_allowed(&code) {
            return Err(ProbeError::Failed(
                "Native authorization code is empty, oversized, or contains whitespace/control characters.",
            ));
        }
        let stdin = self
            .stdin
            .as_mut()
            .ok_or(ProbeError::Failed("Native authentication input is closed."))?;
        let mut bytes = code.into_bytes();
        let result = tokio::time::timeout_at(
            deadline.min(Instant::now() + Duration::from_secs(2)),
            async {
                stdin.write_all(&bytes).await?;
                stdin.write_all(b"\n").await?;
                stdin.flush().await
            },
        )
        .await
        .map_err(|_| ProbeError::Timeout)
        .and_then(|result| result.map_err(ProbeError::io));
        bytes.fill(0);
        result
    }

    pub(super) async fn finish<T>(mut self, result: ProbeResult<T>) -> ProbeResult<T> {
        if let Some(mut stdin) = self.stdin.take() {
            let _ = tokio::time::timeout(Duration::from_millis(500), stdin.shutdown()).await;
        }
        self.tree
            .terminate_and_wait()
            .await
            .map_err(|_| ProbeError::Failed("Native process cleanup could not be verified."))?;
        result
    }
}

pub(super) async fn spawn_owned(mut command: Command) -> ProbeResult<OwnedProcessTree> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    match OwnedProcessTree::spawn(&mut command).map_err(ProbeError::io)? {
        OwnedProcessTreeSpawn::Ready(tree) => Ok(tree),
        OwnedProcessTreeSpawn::CleanupRequired { mut tree, .. } => {
            let _ = tree.terminate_and_wait().await;
            Err(ProbeError::Failed(
                "Native process ownership could not be established.",
            ))
        }
    }
}
