//! Fixed native setup commands. Output is private, bounded and never logged.
use std::{
    collections::BTreeMap, ffi::OsString, path::Path, process::Stdio, sync::Arc, time::Duration,
};

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    task::JoinHandle,
};

use crate::adapter::process_tree::{OwnedProcessTree, OwnedProcessTreeSpawn};

const MAX_OUTPUT: usize = 4 * 1024 * 1024;
const MAX_LINE: usize = 16 * 1024;
pub(super) type LineObserver = Arc<dyn Fn(&str) + Send + Sync>;

pub(super) struct NativeOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
}

struct Reader(JoinHandle<Result<Vec<u8>>>);

impl Drop for Reader {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn read_bounded(
    mut stream: impl AsyncRead + Unpin,
    observer: Option<LineObserver>,
    retain: bool,
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut line = Vec::new();
    let mut total = 0usize;
    let mut buffer = [0; 4096];
    loop {
        let count = stream.read(&mut buffer).await?;
        if count == 0 {
            if let Some(observer) = &observer
                && !line.is_empty()
                && let Ok(text) = std::str::from_utf8(&line)
            {
                observer(text);
            }
            return Ok(output);
        }
        total = total
            .checked_add(count)
            .context("native setup output exceeded its bound")?;
        if total > MAX_OUTPUT {
            return Err(anyhow!("native setup output exceeded its bound"));
        }
        if retain {
            output.extend_from_slice(&buffer[..count]);
        }
        if let Some(observer) = &observer {
            for byte in &buffer[..count] {
                if *byte == b'\n' || *byte == b'\r' {
                    if let Ok(text) = std::str::from_utf8(&line) {
                        observer(text);
                    }
                    line.clear();
                } else if line.len() < MAX_LINE {
                    line.push(*byte);
                } else {
                    return Err(anyhow!("native sign-in output exceeded its line bound"));
                }
            }
        }
    }
}

pub(super) fn bounded_deadline(expires_at: DateTime<Utc>, seconds: i64) -> DateTime<Utc> {
    expires_at.min(Utc::now() + chrono::Duration::seconds(seconds))
}

/// The executable, prefix and environment are runner-owned, never setup payloads.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_owned(
    executable: &Path,
    prefix: &[OsString],
    args: &[OsString],
    cwd: &Path,
    environment: &BTreeMap<String, OsString>,
    remove_environment: &[&str],
    expires_at: DateTime<Utc>,
    observer: Option<LineObserver>,
) -> Result<NativeOutput> {
    let remaining = (expires_at - Utc::now())
        .to_std()
        .context("native setup operation expired")?;
    if remaining.is_zero() {
        return Err(anyhow!("native setup operation expired"));
    }
    let mut command = Command::new(executable);
    command
        .args(prefix)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in [
        "DATABASE_URL",
        "CRONY_DATABASE_URL",
        "CRONY_ACCESS_TOKEN",
        "CRONY_RUNNER_CREDENTIAL",
        "CRONY_RUNNER_ENROLLMENT_TOKEN",
        "OPENAI_API_KEY",
        "CODEX_ACCESS_TOKEN",
        "ANTHROPIC_API_KEY",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "COPILOT_GITHUB_TOKEN",
        "COPILOT_SDK_AUTH_TOKEN",
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "AWS_SESSION_TOKEN",
        "AZURE_CLIENT_SECRET",
        "AZURE_STORAGE_KEY",
    ] {
        command.env_remove(name);
    }
    for name in remove_environment {
        command.env_remove(name);
    }
    command.envs(environment);
    let mut tree = match OwnedProcessTree::spawn(&mut command)
        .context("native setup executable could not be started")?
    {
        OwnedProcessTreeSpawn::Ready(tree) => tree,
        OwnedProcessTreeSpawn::CleanupRequired { mut tree, .. } => {
            tree.terminate_and_wait().await?;
            return Err(anyhow!(
                "native setup process ownership could not be established"
            ));
        }
    };
    let stdout = tree
        .child_mut()
        .stdout
        .take()
        .context("native setup stdout is missing")?;
    let stderr = tree
        .child_mut()
        .stderr
        .take()
        .context("native setup stderr is missing")?;
    let mut out = Reader(tokio::spawn(read_bounded(stdout, observer.clone(), true)));
    let mut err = Reader(tokio::spawn(read_bounded(stderr, observer, false)));
    let deadline = tokio::time::sleep(remaining);
    tokio::pin!(deadline);
    let mut tick = tokio::time::interval(Duration::from_millis(25));
    let mut out_result = None;
    let mut err_result = None;
    let result = loop {
        tokio::select! {
            _ = &mut deadline => break Err(anyhow!("native setup operation timed out")),
            value = &mut out.0, if out_result.is_none() => {
                match value {
                    Ok(Ok(bytes)) => out_result = Some(bytes),
                    _ => break Err(anyhow!("native setup output could not be read safely")),
                }
            }
            value = &mut err.0, if err_result.is_none() => {
                match value {
                    Ok(Ok(bytes)) => err_result = Some(bytes),
                    _ => break Err(anyhow!("native setup diagnostics exceeded their bound")),
                }
            }
            _ = tick.tick() => {
                match tree.try_wait_root() {
                    Ok(Some(status)) => break Ok(status.success()),
                    Ok(None) => {},
                    Err(_) => break Err(anyhow!("native setup process could not be observed")),
                }
            }
        }
    };
    // A root exit alone does not establish that the setup operation is quiescent.
    tree.terminate_and_wait()
        .await
        .context("native setup cleanup could not be verified")?;
    let success = result?;
    let stdout = match out_result {
        Some(bytes) => bytes,
        None => tokio::time::timeout(Duration::from_secs(2), &mut out.0)
            .await
            .context("native setup output did not close")?
            .context("native setup output task failed")??,
    };
    if err_result.is_none() {
        tokio::time::timeout(Duration::from_secs(2), &mut err.0)
            .await
            .context("native setup diagnostics did not close")?
            .context("native setup diagnostic task failed")??;
    }
    Ok(NativeOutput { success, stdout })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn setup_reader_bounds_unterminated_lines_and_never_retains_diagnostics() {
        let data = b"first\rsecond\n";
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let collect = seen.clone();
        let observer: LineObserver = Arc::new(move |line| {
            collect.lock().unwrap().push(line.to_owned());
        });
        assert!(
            read_bounded(&data[..], Some(observer), false)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(*seen.lock().unwrap(), vec!["first", "second"]);
        let huge = vec![b'x'; MAX_LINE + 1];
        assert!(
            read_bounded(&huge[..], Some(Arc::new(|_| {})), false)
                .await
                .is_err()
        );
    }
}
