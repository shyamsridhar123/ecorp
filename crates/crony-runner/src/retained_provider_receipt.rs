//! Collect an existing historical adapter receipt, never a provider session.
//! The current command/grant and sealed bytes authorize collection now; they
//! do not attest a previous artifact acceptance or a new provider completion.

use std::path::Path;

use cap_fs_ext::{FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::{ambient_authority, fs::Dir};
use crony_domain::{
    MAX_RETAINED_PROVIDER_RECEIPT_BYTES, RETAINED_COPILOT_RECEIPT_FILE,
    retained_provider_receipt_metadata, validate_retained_copilot_receipt,
};

use super::*;

pub(super) const COLLECTION_SUMMARY: &str = "Historical native provider receipt collected now from the sealed workspace; current verification ran without starting a provider.";

#[derive(Debug, thiserror::Error)]
#[error("retained provider receipt source integrity failed: {0}")]
struct SourceIntegrity(String);

fn integrity(error: anyhow::Error) -> anyhow::Error {
    SourceIntegrity(format!("{error:#}")).into()
}

pub(super) fn is_integrity_failure(error: &anyhow::Error) -> bool {
    error.is::<SourceIntegrity>()
}

pub(super) fn validate_assignment(assignment: &Assignment) -> Result<()> {
    let Some(grant) = assignment.retained_provider_receipt.as_ref() else {
        return Ok(());
    };
    grant.validate().map_err(anyhow::Error::msg)?;
    if !assignment.checkpoint_verification
        || assignment.provider_artifact.is_some()
        || assignment.adapter != "verification-only"
        || assignment.model.is_some()
        || assignment.reasoning_effort.is_some()
        || !assignment.secrets.is_empty()
        || assignment.verification_command_id != Some(grant.collection_id)
        || grant.corp_id != assignment.corp_id
        || grant.task_id != assignment.task_id
        || grant.run_id != assignment.run_id
        || grant.workspace_run_id != assignment.workspace_run_id
        || assignment.expected_workspace_fingerprint.as_deref()
            != Some(grant.expected_workspace_fingerprint.as_str())
        || assignment.expected_head_commit.as_deref() != Some(grant.expected_head_commit.as_str())
    {
        return Err(anyhow!(
            "retained provider receipt grant does not match this checkpoint verification command"
        ));
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
    bytes: u64,
}

impl FileIdentity {
    fn read(file: &cap_std::fs::File) -> Result<Self> {
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.nlink() != 1
            || metadata.len() == 0
            || metadata.len() > MAX_RETAINED_PROVIDER_RECEIPT_BYTES as u64
        {
            return Err(anyhow!(
                "retained native receipt must be a bounded, nonempty, singly-linked regular file"
            ));
        }
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            bytes: metadata.len(),
        })
    }
}

struct GuardedReceipt {
    root: PathBuf,
    dir: Dir,
    root_identity: (u64, u64),
    file: cap_std::fs::File,
    identity: FileIdentity,
}

fn plain_root(root: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(root)?;
    if !metadata.is_dir() || workspace::snapshot_entry_is_link(&metadata) {
        return Err(anyhow!("retained receipt root is not a plain directory"));
    }
    Ok(())
}

fn open_fixed_receipt(dir: &Dir, root: &Path) -> Result<cap_std::fs::File> {
    // This is deliberately not a caller-supplied path or legacy-name search.
    let metadata = std::fs::symlink_metadata(root.join(RETAINED_COPILOT_RECEIPT_FILE))?;
    if !metadata.is_file() || workspace::snapshot_entry_is_link(&metadata) {
        return Err(anyhow!(
            "retained native receipt cannot be a link, control path, or special file"
        ));
    }
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        // A concurrent regular-file -> FIFO replacement must not block opening.
        options.custom_flags(libc::O_NONBLOCK);
    }
    dir.open_with(RETAINED_COPILOT_RECEIPT_FILE, &options)
        .context("open fixed retained provider receipt without following links")
}

impl GuardedReceipt {
    fn open(root: &Path) -> Result<Self> {
        plain_root(root)?;
        let dir = Dir::open_ambient_dir(root, ambient_authority())?;
        let root_metadata = dir.dir_metadata()?;
        let root_identity = (root_metadata.dev(), root_metadata.ino());
        let file = open_fixed_receipt(&dir, root)?;
        let identity = FileIdentity::read(&file)?;
        let guarded = Self {
            root: root.to_owned(),
            dir,
            root_identity,
            file,
            identity,
        };
        guarded.ensure_stable()?;
        Ok(guarded)
    }

    fn ensure_stable(&self) -> Result<()> {
        plain_root(&self.root)?;
        let current_dir = Dir::open_ambient_dir(&self.root, ambient_authority())?;
        let metadata = current_dir.dir_metadata()?;
        if self.root_identity != (metadata.dev(), metadata.ino())
            || self.identity != FileIdentity::read(&self.file)?
            || self.identity != FileIdentity::read(&open_fixed_receipt(&self.dir, &self.root)?)?
        {
            return Err(anyhow!(
                "retained receipt directory or file was replaced during collection"
            ));
        }
        Ok(())
    }

    async fn read(&self) -> Result<Vec<u8>> {
        let file = tokio::fs::File::from_std(self.file.try_clone()?.into_std());
        let mut bytes = Vec::new();
        file.take(MAX_RETAINED_PROVIDER_RECEIPT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .await?;
        self.ensure_stable()?;
        if bytes.len() != self.identity.bytes as usize
            || bytes.len() > MAX_RETAINED_PROVIDER_RECEIPT_BYTES
        {
            return Err(anyhow!("retained receipt changed size while being read"));
        }
        Ok(bytes)
    }
}

async fn cancellable<T>(
    cancellation: &mut watch::Receiver<bool>,
    operation: impl std::future::Future<Output = Result<T>>,
) -> Result<Option<T>> {
    if *cancellation.borrow() {
        return Ok(None);
    }
    tokio::select! {
        biased;
        () = verifier::wait_for_verifier_cancellation(cancellation) => Ok(None),
        result = operation => result.map(Some),
    }
}

async fn current_source(
    workspaces: &WorkspaceManager,
    workspace: &WorkspaceLease,
    baseline: &VerificationSnapshot,
    assignment: &Assignment,
    source_receipt: &GuardedReceipt,
    baseline_receipt: &GuardedReceipt,
) -> Result<()> {
    source_receipt.ensure_stable().map_err(integrity)?;
    baseline_receipt.ensure_stable().map_err(integrity)?;
    verify_prepared_recovery_workspace(workspaces, workspace, assignment)
        .await
        .map_err(integrity)?;
    let fingerprint = workspace::fingerprint_path(baseline.path())
        .await
        .map_err(integrity)?;
    if assignment.expected_workspace_fingerprint.as_deref() != Some(fingerprint.as_str()) {
        return Err(integrity(anyhow!("retained receipt baseline changed")));
    }
    Ok(())
}

pub(super) fn acknowledgment_matches(ack: &ArtifactAck, run_id: Uuid, sha256: &str) -> bool {
    ack.run_id == run_id
        && !ack.artifact_id.is_nil()
        && ack.artifact_role == "provider_evidence"
        && ack.sha256 == sha256
}

/// None is cancellation, never a successful empty collection.
#[allow(clippy::too_many_arguments)]
pub(super) async fn collect(
    outbound: &OutboundBus,
    runner_id: &str,
    assignment: &Assignment,
    workspace: &WorkspaceLease,
    workspaces: &WorkspaceManager,
    baseline: &VerificationSnapshot,
    artifact_snapshot: &VerificationSnapshot,
    artifact_acks: &mut mpsc::UnboundedReceiver<ArtifactAck>,
    cancellation: &mut watch::Receiver<bool>,
) -> Result<Option<PreparedVerificationArtifact>> {
    validate_assignment(assignment)?;
    let grant = assignment
        .retained_provider_receipt
        .as_ref()
        .context("retained receipt collection requires an explicit grant")?;
    if *cancellation.borrow() {
        return Ok(None);
    }
    let source_receipt = GuardedReceipt::open(&workspace.path)?;
    let baseline_receipt = GuardedReceipt::open(baseline.path())?;
    let Some(bytes) = cancellable(cancellation, baseline_receipt.read()).await? else {
        return Ok(None);
    };
    validate_retained_copilot_receipt(&bytes, grant).map_err(anyhow::Error::msg)?;
    if source_receipt.identity.bytes != bytes.len() as u64 {
        return Err(integrity(anyhow!("retained source receipt changed size")));
    }
    if cancellable(
        cancellation,
        current_source(
            workspaces,
            workspace,
            baseline,
            assignment,
            &source_receipt,
            &baseline_receipt,
        ),
    )
    .await?
    .is_none()
    {
        return Ok(None);
    }
    let sha256 = hex::encode(sha2::Sha256::digest(&bytes));
    let artifact_path = artifact_snapshot.path().join(RETAINED_COPILOT_RECEIPT_FILE);
    let staging = async {
        let root = tokio::fs::canonicalize(artifact_snapshot.path()).await?;
        let source = tokio::fs::canonicalize(&workspace.path).await?;
        let sealed = tokio::fs::canonicalize(baseline.path()).await?;
        if root.starts_with(&source)
            || source.starts_with(&root)
            || root.starts_with(&sealed)
            || sealed.starts_with(&root)
        {
            return Err(anyhow!("retained receipt staging overlaps sealed source"));
        }
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&artifact_path)
            .await?;
        file.write_all(&bytes).await?;
        file.flush().await?;
        Ok(())
    };
    if cancellable(cancellation, staging).await?.is_none() {
        return Ok(None);
    }
    let metadata = retained_provider_receipt_metadata(grant);
    let upload = json!({
        "sha256": sha256,
        "bytes": bytes.len(),
        "media_type": "application/json",
        "artifact_role": "provider_evidence",
        "file_name": RETAINED_COPILOT_RECEIPT_FILE,
        "workspace_relative_path": RETAINED_COPILOT_RECEIPT_FILE,
        "content_base64": BASE64.encode(&bytes),
        "retained_provider_receipt": metadata["retained_provider_receipt"],
    });
    // Same durable upload/ACK policy as the existing deliverable path. The outer
    // verifier select continues consuming controls throughout collection and ACKs.
    for _ in 0..6 {
        if cancellable(
            cancellation,
            current_source(
                workspaces,
                workspace,
                baseline,
                assignment,
                &source_receipt,
                &baseline_receipt,
            ),
        )
        .await?
        .is_none()
        {
            return Ok(None);
        }
        send_run_event(
            outbound,
            runner_id,
            assignment,
            "run.artifact_upload",
            upload.clone(),
        );
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let attempt = tokio::select! {
                biased;
                () = verifier::wait_for_verifier_cancellation(cancellation) => return Ok(None),
                attempt = tokio::time::timeout_at(deadline, artifact_acks.recv()) => attempt,
            };
            if cancellable(
                cancellation,
                current_source(
                    workspaces,
                    workspace,
                    baseline,
                    assignment,
                    &source_receipt,
                    &baseline_receipt,
                ),
            )
            .await?
            .is_none()
            {
                return Ok(None);
            }
            match attempt {
                Ok(Some(ack)) if acknowledgment_matches(&ack, assignment.run_id, &sha256) => {
                    return Ok(Some(PreparedVerificationArtifact {
                        artifact: AdapterArtifact {
                            path: artifact_path,
                            sha256: sha256.clone(),
                            bytes: bytes.len(),
                            media_type: "application/json".to_owned(),
                        },
                        source_artifact: Some(AdapterArtifact {
                            path: workspace.path.join(RETAINED_COPILOT_RECEIPT_FILE),
                            sha256,
                            bytes: bytes.len(),
                            media_type: "application/json".to_owned(),
                        }),
                    }));
                }
                Ok(Some(_)) => {}
                Ok(None) => {
                    return Err(anyhow!(
                        "retained provider receipt acknowledgment channel closed"
                    ));
                }
                Err(_) => break,
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
        }
    }
    Err(anyhow!(
        "retained provider receipt storage acknowledgment timed out"
    ))
}
