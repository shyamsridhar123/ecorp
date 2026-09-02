use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use clap::Args;
use crony_domain::{PullRequestPublication, PullRequestPublicationState};
use crony_protocol::PullRequestPublicationResponse;
use reqwest::{Client, Method};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::factory::{
    gh_json, gh_output, gh_run, sanitize_failure_detail, server_json, source_git_output,
};

#[derive(Debug, Args)]
pub struct FactoryPublishArgs {
    pub corp_id: Uuid,
    pub actor_id: Uuid,
    pub work_item_id: Uuid,

    #[arg(long)]
    pub source_deliverable_id: Option<Uuid>,

    #[arg(long)]
    pub repository: Option<String>,

    #[arg(long)]
    pub base_ref: Option<String>,

    #[arg(long)]
    pub branch: Option<String>,

    #[arg(long)]
    pub title: Option<String>,

    #[arg(long)]
    pub body_file: Option<PathBuf>,

    #[arg(long)]
    pub authorization_id: Option<Uuid>,

    #[arg(
        long,
        default_value = "Publish this verified factory deliverable for human review."
    )]
    pub authorization_reason: String,

    #[arg(long)]
    pub effect_key: Option<String>,

    #[arg(long)]
    pub idempotency_key: Option<String>,

    #[arg(long, env = "ECORP_PUBLICATION_PUBLISHER_ID")]
    pub publisher_id: Option<String>,

    #[arg(long, env = "ECORP_GITHUB_CLI", default_value = "gh")]
    pub github_cli: PathBuf,

    #[arg(long, default_value_t = 300)]
    pub lease_seconds: i64,

    #[arg(long, default_value_t = 120)]
    pub wait_seconds: u64,

    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug)]
struct PublicationPlan {
    source_deliverable_id: Uuid,
    artifact_id: Uuid,
    target_repository: String,
    base_ref: String,
    branch: String,
    commit_sha: String,
    title: String,
    body: String,
    authorization_id: Uuid,
    authorization_reason: String,
    effect_key: String,
    idempotency_key: String,
    publisher_id: String,
    issue_number: i64,
    issue_url: String,
    project_owner: String,
    project_number: i64,
    project_item_id: String,
    project_status_before: String,
    project_review_status: String,
}

#[derive(Debug, Deserialize)]
struct CommitBranchDocument {
    schema_version: i64,
    form: String,
    base_commit: String,
    head_commit: String,
    branch: String,
    verification_sha256: String,
    git_bundle_sha256: String,
    git_bundle_base64: String,
}

#[derive(Debug, Clone, Deserialize)]
struct PullRequestView {
    number: i64,
    id: String,
    url: String,
    state: String,
    #[serde(rename = "isDraft")]
    is_draft: bool,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "baseRefName")]
    base_ref_name: String,
    #[serde(rename = "autoMergeRequest")]
    auto_merge_request: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ProjectView {
    id: String,
}

#[derive(Debug, Deserialize)]
struct ProjectFields {
    #[serde(default)]
    fields: Vec<ProjectField>,
}

#[derive(Debug, Deserialize)]
struct ProjectField {
    id: String,
    name: String,
    #[serde(default)]
    options: Vec<ProjectFieldOption>,
}

#[derive(Debug, Deserialize)]
struct ProjectFieldOption {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct ProjectItemsEnvelope {
    #[serde(default)]
    items: Vec<ProjectItem>,
}

#[derive(Debug, Deserialize)]
struct ProjectItem {
    id: String,
    #[serde(default)]
    status: String,
}

struct TemporaryPublisherWorkspace {
    root: PathBuf,
    repository: PathBuf,
    bundle: PathBuf,
    body: PathBuf,
}

impl TemporaryPublisherWorkspace {
    fn create() -> Result<Self> {
        let root = env::temp_dir().join(format!("ecorp-publisher-{}", Uuid::new_v4().simple()));
        let repository = root.join("repository.git");
        let bundle = root.join("deliverable.bundle");
        let body = root.join("pull-request-body.md");
        fs::create_dir_all(&repository).context("create trusted publisher workspace")?;
        Ok(Self {
            root,
            repository,
            bundle,
            body,
        })
    }
}

impl Drop for TemporaryPublisherWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub async fn run(client: &Client, server: &str, args: FactoryPublishArgs) -> Result<Value> {
    validate_args(&args)?;
    let snapshot = server_json(
        client,
        Method::GET,
        format!(
            "{server}/api/corps/{}/snapshot?actor_id={}",
            args.corp_id, args.actor_id
        ),
        None,
    )
    .await?;
    let plan = publication_plan(&args, &snapshot)?;
    if args.dry_run {
        return Ok(plan_json(&args, &plan, "dry_run", None));
    }

    let mut response =
        start_publication(client, server, &args, &plan, &plan.idempotency_key).await?;
    if response.publication.state == PullRequestPublicationState::Published {
        return Ok(plan_json(
            &args,
            &plan,
            "recovered",
            Some(&response.publication),
        ));
    }
    if response.publisher_token.is_none() {
        response = wait_or_recover_publication(client, server, &args, &plan, response).await?;
    }
    if response.publication.state == PullRequestPublicationState::Published {
        return Ok(plan_json(
            &args,
            &plan,
            "recovered",
            Some(&response.publication),
        ));
    }
    let publisher_token = response
        .publisher_token
        .context("publication has no active trusted-publisher token")?;
    test_crash("after_start");

    match execute_publication(client, server, &args, &plan, &mut response, publisher_token).await {
        Ok(()) => Ok(plan_json(
            &args,
            &plan,
            "published",
            Some(&response.publication),
        )),
        Err(error) => {
            let detail = sanitize_failure_detail(&format!("{error:#}"));
            let _ = record_failure(
                client,
                server,
                &args,
                &plan,
                &mut response,
                publisher_token,
                &detail,
            )
            .await;
            Err(error)
        }
    }
}

async fn start_publication(
    client: &Client,
    server: &str,
    args: &FactoryPublishArgs,
    plan: &PublicationPlan,
    idempotency_key: &str,
) -> Result<PullRequestPublicationResponse> {
    let response = server_json(
        client,
        Method::POST,
        format!(
            "{server}/api/corps/{}/factory/work-items/{}/publication",
            args.corp_id, args.work_item_id
        ),
        Some(json!({
            "actor_id": args.actor_id,
            "source_deliverable_id": plan.source_deliverable_id,
            "target_repository": plan.target_repository,
            "base_ref": plan.base_ref,
            "branch": plan.branch,
            "title": plan.title,
            "body": plan.body,
            "authorization_id": plan.authorization_id,
            "authorization_reason": plan.authorization_reason,
            "effect_key": plan.effect_key,
            "idempotency_key": idempotency_key,
            "publisher_id": plan.publisher_id,
            "lease_seconds": args.lease_seconds,
        })),
    )
    .await?;
    serde_json::from_value(response).context("decode publication start response")
}

async fn wait_or_recover_publication(
    client: &Client,
    server: &str,
    args: &FactoryPublishArgs,
    plan: &PublicationPlan,
    mut response: PullRequestPublicationResponse,
) -> Result<PullRequestPublicationResponse> {
    let deadline = Instant::now() + Duration::from_secs(args.wait_seconds);
    while response.busy && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(250));
        response = get_publication(client, server, args).await?;
        if response.publication.state == PullRequestPublicationState::Published {
            return Ok(response);
        }
    }
    if response.busy {
        bail!(
            "publication {} remains leased to another trusted publisher until {}",
            response.publication.id,
            response
                .publication
                .publisher_lease_expires_at
                .map(|value| value.to_rfc3339())
                .unwrap_or_else(|| "an unknown time".to_owned())
        );
    }
    let recovery_key = format!(
        "{}:recover:{}:{}",
        plan.effect_key, response.publication.version, response.publication.attempt_count
    );
    start_publication(client, server, args, plan, &recovery_key).await
}

async fn get_publication(
    client: &Client,
    server: &str,
    args: &FactoryPublishArgs,
) -> Result<PullRequestPublicationResponse> {
    let response = server_json(
        client,
        Method::GET,
        format!(
            "{server}/api/corps/{}/factory/work-items/{}/publication?actor_id={}",
            args.corp_id, args.work_item_id, args.actor_id
        ),
        None,
    )
    .await?;
    serde_json::from_value(response).context("decode publication status response")
}

async fn execute_publication(
    client: &Client,
    server: &str,
    args: &FactoryPublishArgs,
    plan: &PublicationPlan,
    response: &mut PullRequestPublicationResponse,
    publisher_token: Uuid,
) -> Result<()> {
    ensure_publication_matches_plan(&response.publication, plan)?;
    let bytes = download_deliverable(client, server, args, plan.artifact_id).await?;
    let document: CommitBranchDocument =
        serde_json::from_slice(&bytes).context("decode commit/branch deliverable")?;
    let bundle = validate_deliverable_document(&document, &response.publication)?;
    let workspace = TemporaryPublisherWorkspace::create()?;
    fs::write(&workspace.bundle, &bundle).context("write portable Git bundle")?;
    fs::write(&workspace.body, plan.body.as_bytes()).context("write pull request body")?;
    prepare_repository(&workspace, plan, &document)?;

    if publication_rank(response.publication.state)
        < publication_rank(PullRequestPublicationState::BranchPushed)
    {
        renew_publication(
            client,
            server,
            args,
            plan,
            response,
            publisher_token,
            "branch-push",
        )
        .await?;
        ensure_remote_base(&workspace.repository, plan, &document.base_commit)?;
        push_or_adopt_branch(&workspace.repository, plan)?;
        test_crash("after_branch_remote");
        checkpoint(
            client,
            server,
            args,
            plan,
            response,
            publisher_token,
            "branch-pushed",
            json!({
                "kind": "branch_pushed",
                "commit_sha": plan.commit_sha
            }),
        )
        .await?;
        test_crash("after_branch_checkpoint");
    } else {
        ensure_remote_branch(&workspace.repository, plan)?;
    }

    if publication_rank(response.publication.state)
        < publication_rank(PullRequestPublicationState::PullRequestCreated)
    {
        renew_publication(
            client,
            server,
            args,
            plan,
            response,
            publisher_token,
            "pull-request",
        )
        .await?;
        let pull_request = create_or_adopt_pull_request(args, plan, &workspace.body)?;
        test_crash("after_pull_request_remote");
        checkpoint(
            client,
            server,
            args,
            plan,
            response,
            publisher_token,
            "pull-request-created",
            json!({
                "kind": "pull_request_created",
                "number": pull_request.number,
                "node_id": pull_request.id,
                "url": pull_request.url,
                "state": pull_request.state,
                "draft": pull_request.is_draft,
                "head_ref": pull_request.head_ref_name,
                "base_ref": pull_request.base_ref_name,
                "auto_merge_enabled": pull_request.auto_merge_request.is_some()
            }),
        )
        .await?;
        test_crash("after_pull_request_checkpoint");
    } else {
        let pull_request = find_pull_request(args, plan)?
            .context("persisted publication pull request no longer exists")?;
        ensure_remote_pull_request_matches(&pull_request, plan)?;
    }

    if response.publication.state != PullRequestPublicationState::Published {
        let (project_id, _, _, status) = project_status(args, plan)?;
        if status != plan.project_status_before && status != plan.project_review_status {
            bail!(
                "GitHub Project item status is {status}, expected {} or {}",
                plan.project_status_before,
                plan.project_review_status
            );
        }
        renew_publication(
            client,
            server,
            args,
            plan,
            response,
            publisher_token,
            "project-review",
        )
        .await?;
        let (_, refreshed_field_id, refreshed_option_id, refreshed_status) =
            project_status(args, plan)?;
        if refreshed_status == plan.project_status_before {
            let edit_result = gh_run(
                &args.github_cli,
                &[
                    "project",
                    "item-edit",
                    "--id",
                    &plan.project_item_id,
                    "--project-id",
                    &project_id,
                    "--field-id",
                    &refreshed_field_id,
                    "--single-select-option-id",
                    &refreshed_option_id,
                ],
            );
            if let Err(error) = edit_result {
                let (_, _, _, recovered_status) = project_status(args, plan)?;
                if recovered_status != plan.project_review_status {
                    return Err(error).context("move GitHub Project item into review");
                }
            }
        }
        let (_, _, _, final_status) = project_status(args, plan)?;
        if final_status != plan.project_review_status {
            bail!(
                "GitHub Project item status is {final_status}, not {}",
                plan.project_review_status
            );
        }
        test_crash("after_project_remote");
        checkpoint(
            client,
            server,
            args,
            plan,
            response,
            publisher_token,
            "published",
            json!({
                "kind": "published",
                "project_status": final_status,
                "project_field_id": refreshed_field_id,
                "project_option_id": refreshed_option_id
            }),
        )
        .await?;
    }
    Ok(())
}

async fn renew_publication(
    client: &Client,
    server: &str,
    args: &FactoryPublishArgs,
    plan: &PublicationPlan,
    response: &mut PullRequestPublicationResponse,
    publisher_token: Uuid,
    stage: &str,
) -> Result<()> {
    let version = response.publication.version;
    let value = server_json(
        client,
        Method::POST,
        format!(
            "{server}/api/corps/{}/factory/publications/{}/renew",
            args.corp_id, response.publication.id
        ),
        Some(json!({
            "actor_id": args.actor_id,
            "publisher_token": publisher_token,
            "expected_version": version,
            "idempotency_key": format!("{}:renew:{stage}:{version}", plan.effect_key),
            "lease_seconds": args.lease_seconds,
        })),
    )
    .await?;
    *response = serde_json::from_value(value).context("decode publication renewal response")?;
    if response.publisher_token != Some(publisher_token) {
        bail!("publication renewal did not preserve the trusted-publisher token");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn checkpoint(
    client: &Client,
    server: &str,
    args: &FactoryPublishArgs,
    plan: &PublicationPlan,
    response: &mut PullRequestPublicationResponse,
    publisher_token: Uuid,
    stage: &str,
    checkpoint: Value,
) -> Result<()> {
    let version = response.publication.version;
    let value = server_json(
        client,
        Method::POST,
        format!(
            "{server}/api/corps/{}/factory/publications/{}/checkpoint",
            args.corp_id, response.publication.id
        ),
        Some(json!({
            "actor_id": args.actor_id,
            "publisher_token": publisher_token,
            "expected_version": version,
            "idempotency_key": format!("{}:checkpoint:{stage}:{version}", plan.effect_key),
            "checkpoint": checkpoint
        })),
    )
    .await?;
    *response = serde_json::from_value(value).context("decode publication checkpoint response")?;
    Ok(())
}

async fn record_failure(
    client: &Client,
    server: &str,
    args: &FactoryPublishArgs,
    plan: &PublicationPlan,
    response: &mut PullRequestPublicationResponse,
    publisher_token: Uuid,
    failure_detail: &str,
) -> Result<()> {
    if response.publication.state == PullRequestPublicationState::Published
        || response.publisher_token != Some(publisher_token)
    {
        return Ok(());
    }
    checkpoint(
        client,
        server,
        args,
        plan,
        response,
        publisher_token,
        "failed",
        json!({
            "kind": "failed",
            "failure_detail": failure_detail
        }),
    )
    .await
}

async fn download_deliverable(
    client: &Client,
    server: &str,
    args: &FactoryPublishArgs,
    artifact_id: Uuid,
) -> Result<Vec<u8>> {
    let response = client
        .get(format!(
            "{server}/api/corps/{}/artifacts/{artifact_id}?actor_id={}",
            args.corp_id, args.actor_id
        ))
        .send()
        .await
        .context("download source deliverable")?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .context("read source deliverable response")?;
    if !status.is_success() {
        bail!(
            "source deliverable download failed with HTTP {status}: {}",
            sanitize_failure_detail(&String::from_utf8_lossy(&bytes))
        );
    }
    Ok(bytes.to_vec())
}

fn validate_deliverable_document(
    document: &CommitBranchDocument,
    publication: &PullRequestPublication,
) -> Result<Vec<u8>> {
    if document.schema_version != 1
        || document.form != "commit_branch"
        || !document.base_commit.eq_ignore_ascii_case(
            publication
                .provenance
                .pointer("/deliverable/base_commit")
                .and_then(Value::as_str)
                .context("publication provenance omitted deliverable base commit")?,
        )
        || !document
            .head_commit
            .eq_ignore_ascii_case(&publication.commit_sha)
        || document.verification_sha256
            != publication
                .provenance
                .pointer("/verification_sha256")
                .and_then(Value::as_str)
                .context("publication provenance omitted verification digest")?
    {
        bail!("downloaded commit/branch deliverable does not match publication provenance");
    }
    let bundle = BASE64
        .decode(&document.git_bundle_base64)
        .context("decode portable Git bundle")?;
    let digest = format!("{:x}", Sha256::digest(&bundle));
    if digest != document.git_bundle_sha256 {
        bail!("portable Git bundle digest does not match the deliverable");
    }
    Ok(bundle)
}

fn prepare_repository(
    workspace: &TemporaryPublisherWorkspace,
    plan: &PublicationPlan,
    document: &CommitBranchDocument,
) -> Result<()> {
    source_git_output(&workspace.repository, &["init", "--bare"])?;
    let remote_url = env::var("ECORP_PUBLICATION_TEST_REMOTE_URL")
        .unwrap_or_else(|_| format!("https://github.com/{}.git", plan.target_repository));
    source_git_output(
        &workspace.repository,
        &["remote", "add", "origin", &remote_url],
    )?;
    source_git_output(
        &workspace.repository,
        &["fetch", "--no-tags", "origin", &plan.base_ref],
    )
    .context("fetch authorized pull request base")?;
    let fetched_base = git_text(&workspace.repository, &["rev-parse", "FETCH_HEAD^{commit}"])?;
    if !fetched_base.eq_ignore_ascii_case(&document.base_commit) {
        bail!(
            "remote base {} resolved to {}, not verified commit {}",
            plan.base_ref,
            fetched_base,
            document.base_commit
        );
    }
    source_git_output(
        &workspace.repository,
        &["bundle", "verify", path_text(&workspace.bundle)?],
    )
    .context("verify portable Git bundle prerequisites")?;
    let source_ref = format!("refs/heads/{}", document.branch);
    let import_ref = "refs/heads/ecorp-import";
    let refspec = format!("{source_ref}:{import_ref}");
    source_git_output(
        &workspace.repository,
        &[
            "fetch",
            "--no-tags",
            path_text(&workspace.bundle)?,
            &refspec,
        ],
    )
    .context("import portable Git bundle")?;
    let imported = git_text(
        &workspace.repository,
        &["rev-parse", "refs/heads/ecorp-import^{commit}"],
    )?;
    if !imported.eq_ignore_ascii_case(&plan.commit_sha) {
        bail!(
            "portable Git bundle imported commit {imported}, expected {}",
            plan.commit_sha
        );
    }
    source_git_output(
        &workspace.repository,
        &[
            "merge-base",
            "--is-ancestor",
            &document.base_commit,
            &plan.commit_sha,
        ],
    )
    .context("verify publication commit descends from the authorized base")?;
    Ok(())
}

fn ensure_remote_base(
    repository: &Path,
    plan: &PublicationPlan,
    expected_commit: &str,
) -> Result<()> {
    let reference = remote_base_reference(&plan.base_ref);
    let actual = remote_reference_commit(repository, &reference)?
        .with_context(|| format!("remote base reference {reference} does not exist"))?;
    if !actual.eq_ignore_ascii_case(expected_commit) {
        bail!(
            "remote base reference {reference} moved from verified commit {expected_commit} to {actual}"
        );
    }
    Ok(())
}

fn push_or_adopt_branch(repository: &Path, plan: &PublicationPlan) -> Result<()> {
    let reference = format!("refs/heads/{}", plan.branch);
    match remote_reference_commit(repository, &reference)? {
        Some(existing) if existing.eq_ignore_ascii_case(&plan.commit_sha) => return Ok(()),
        Some(existing) => {
            bail!(
                "remote branch {} already points to {}, not verified commit {}",
                plan.branch,
                existing,
                plan.commit_sha
            )
        }
        None => {}
    }
    let refspec = format!("{}:{reference}", plan.commit_sha);
    if let Err(error) = source_git_output(repository, &["push", "origin", &refspec]) {
        match remote_reference_commit(repository, &reference)? {
            Some(existing) if existing.eq_ignore_ascii_case(&plan.commit_sha) => return Ok(()),
            _ => return Err(error).context("push verified publication branch"),
        }
    }
    ensure_remote_branch(repository, plan)
}

fn ensure_remote_branch(repository: &Path, plan: &PublicationPlan) -> Result<()> {
    let reference = format!("refs/heads/{}", plan.branch);
    let actual = remote_reference_commit(repository, &reference)?
        .with_context(|| format!("remote publication branch {} does not exist", plan.branch))?;
    if !actual.eq_ignore_ascii_case(&plan.commit_sha) {
        bail!(
            "remote publication branch {} points to {}, not {}",
            plan.branch,
            actual,
            plan.commit_sha
        );
    }
    Ok(())
}

fn remote_reference_commit(repository: &Path, reference: &str) -> Result<Option<String>> {
    let output = source_git_output(repository, &["ls-remote", "--refs", "origin", reference])?;
    let text = String::from_utf8(output).context("remote Git reference is not UTF-8")?;
    let mut matches = text
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_owned);
    let first = matches.next();
    if matches.next().is_some() {
        bail!("remote Git reference query returned multiple matches for {reference}");
    }
    Ok(first)
}

fn remote_base_reference(base_ref: &str) -> String {
    if base_ref == "HEAD" || base_ref.starts_with("refs/") {
        base_ref.to_owned()
    } else {
        format!("refs/heads/{base_ref}")
    }
}

fn create_or_adopt_pull_request(
    args: &FactoryPublishArgs,
    plan: &PublicationPlan,
    body_file: &Path,
) -> Result<PullRequestView> {
    if let Some(existing) = find_pull_request(args, plan)? {
        ensure_remote_pull_request_matches(&existing, plan)?;
        return Ok(existing);
    }
    let create = gh_output(
        &args.github_cli,
        &[
            "pr",
            "create",
            "--repo",
            &plan.target_repository,
            "--base",
            &plan.base_ref,
            "--head",
            &plan.branch,
            "--title",
            &plan.title,
            "--body-file",
            path_text(body_file)?,
        ],
    );
    if let Err(error) = create {
        if let Some(recovered) = find_pull_request(args, plan)? {
            ensure_remote_pull_request_matches(&recovered, plan)?;
            return Ok(recovered);
        }
        return Err(error).context("create GitHub pull request");
    }
    let created = find_pull_request(args, plan)?
        .context("GitHub pull request creation returned success but no pull request exists")?;
    ensure_remote_pull_request_matches(&created, plan)?;
    Ok(created)
}

fn find_pull_request(
    args: &FactoryPublishArgs,
    plan: &PublicationPlan,
) -> Result<Option<PullRequestView>> {
    let value = gh_json(
        &args.github_cli,
        &[
            "pr",
            "list",
            "--repo",
            &plan.target_repository,
            "--state",
            "all",
            "--head",
            &plan.branch,
            "--base",
            &plan.base_ref,
            "--limit",
            "100",
            "--json",
            "number,id,url,state,isDraft,headRefName,baseRefName,autoMergeRequest",
        ],
    )?;
    let mut pull_requests: Vec<PullRequestView> =
        serde_json::from_value(value).context("decode GitHub pull request list")?;
    pull_requests.retain(|pull_request| {
        pull_request.head_ref_name == plan.branch && pull_request.base_ref_name == plan.base_ref
    });
    if pull_requests.len() > 1 {
        bail!(
            "multiple pull requests exist for {} -> {}",
            plan.branch,
            plan.base_ref
        );
    }
    Ok(pull_requests.pop())
}

fn ensure_remote_pull_request_matches(
    pull_request: &PullRequestView,
    plan: &PublicationPlan,
) -> Result<()> {
    if pull_request.state != "OPEN"
        || pull_request.head_ref_name != plan.branch
        || pull_request.base_ref_name != plan.base_ref
        || pull_request.auto_merge_request.is_some()
    {
        bail!("GitHub pull request does not match the authorized non-merging publication");
    }
    Ok(())
}

fn project_status(
    args: &FactoryPublishArgs,
    plan: &PublicationPlan,
) -> Result<(String, String, String, String)> {
    let project: ProjectView = serde_json::from_value(gh_json(
        &args.github_cli,
        &[
            "project",
            "view",
            &plan.project_number.to_string(),
            "--owner",
            &plan.project_owner,
            "--format",
            "json",
        ],
    )?)
    .context("decode GitHub Project")?;
    let fields: ProjectFields = serde_json::from_value(gh_json(
        &args.github_cli,
        &[
            "project",
            "field-list",
            &plan.project_number.to_string(),
            "--owner",
            &plan.project_owner,
            "--format",
            "json",
        ],
    )?)
    .context("decode GitHub Project fields")?;
    let status_field = fields
        .fields
        .iter()
        .find(|field| field.name == "Status")
        .context("GitHub Project has no Status field")?;
    let review_option = status_field
        .options
        .iter()
        .find(|option| option.name == plan.project_review_status)
        .with_context(|| {
            format!(
                "GitHub Project Status has no {} option",
                plan.project_review_status
            )
        })?;
    let items: ProjectItemsEnvelope = serde_json::from_value(gh_json(
        &args.github_cli,
        &[
            "project",
            "item-list",
            &plan.project_number.to_string(),
            "--owner",
            &plan.project_owner,
            "--format",
            "json",
            "--limit",
            "1000",
        ],
    )?)
    .context("decode GitHub Project items")?;
    let status = items
        .items
        .iter()
        .find(|item| item.id == plan.project_item_id)
        .map(|item| item.status.clone())
        .context("GitHub Project item disappeared during publication")?;
    Ok((
        project.id,
        status_field.id.clone(),
        review_option.id.clone(),
        status,
    ))
}

fn publication_plan(args: &FactoryPublishArgs, snapshot: &Value) -> Result<PublicationPlan> {
    let work_item = snapshot
        .pointer("/snapshot/factory_work_items")
        .and_then(Value::as_array)
        .context("ECorp snapshot omitted factory work items")?
        .iter()
        .find(|item| value_uuid(item, "/id").ok() == Some(args.work_item_id))
        .context("factory work item was not found in the authorized snapshot")?;
    let existing_publication = snapshot
        .pointer("/snapshot/pull_request_publications")
        .and_then(Value::as_array)
        .context("ECorp snapshot omitted pull-request publications")?
        .iter()
        .find(|publication| {
            value_uuid(publication, "/factory_work_item_id").ok() == Some(args.work_item_id)
        });
    let mission_id =
        value_uuid(work_item, "/mission_id").context("factory work item has no mission")?;
    let task_ids = snapshot
        .pointer("/snapshot/tasks")
        .and_then(Value::as_array)
        .context("ECorp snapshot omitted tasks")?
        .iter()
        .filter(|task| value_uuid(task, "/mission_id").ok() == Some(mission_id))
        .filter_map(|task| value_uuid(task, "/id").ok())
        .collect::<Vec<_>>();
    let deliverables = snapshot
        .pointer("/snapshot/source_deliverables")
        .and_then(Value::as_array)
        .context("ECorp snapshot omitted source deliverables")?
        .iter()
        .filter(|deliverable| {
            value_uuid(deliverable, "/task_id")
                .ok()
                .is_some_and(|task_id| task_ids.contains(&task_id))
                && deliverable.get("form").and_then(Value::as_str) == Some("commit_branch")
                && match deliverable.get("integration_state").and_then(Value::as_str) {
                    Some("ready_for_review") => true,
                    Some("published") => existing_publication.is_some(),
                    _ => false,
                }
                && deliverable.get("head_commit").is_some_and(Value::is_string)
        })
        .filter(|deliverable| {
            args.source_deliverable_id
                .is_none_or(|expected| value_uuid(deliverable, "/id").ok() == Some(expected))
        })
        .collect::<Vec<_>>();
    let deliverable = match deliverables.as_slice() {
        [deliverable] => *deliverable,
        [] => bail!("factory mission has no merge-ready commit/branch deliverable"),
        _ => bail!("factory mission has multiple merge-ready deliverables; select one explicitly"),
    };
    let policy = work_item
        .get("policy")
        .and_then(Value::as_object)
        .context("factory work item omitted its policy snapshot")?;
    let publication_policy = policy
        .get("publication")
        .and_then(Value::as_object)
        .context("factory policy does not authorize publication")?;
    let source_owner = value_string(work_item, "/source_repository_owner")?;
    let source_name = value_string(work_item, "/source_repository_name")?;
    let target_repository = existing_publication
        .and_then(|publication| publication.get("target_repository"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| args.repository.clone())
        .unwrap_or_else(|| format!("{source_owner}/{source_name}"));
    let base_ref = existing_publication
        .and_then(|publication| publication.get("base_ref"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| args.base_ref.clone())
        .or_else(|| {
            publication_policy
                .get("base_ref")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .context("publication base ref was not provided or authorized by policy")?;
    let commit_sha = value_string(deliverable, "/head_commit")?;
    let issue_number = value_i64(work_item, "/source_issue_number")?;
    let branch = existing_publication
        .and_then(|publication| publication.get("branch"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| args.branch.clone())
        .unwrap_or_else(|| {
            format!(
                "ecorp/issue-{issue_number}-{}",
                commit_sha.chars().take(12).collect::<String>()
            )
        });
    let title = existing_publication
        .and_then(|publication| publication.get("title"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| args.title.clone())
        .unwrap_or_else(|| value_string(work_item, "/source_title").unwrap_or_default());
    let issue_url = value_string(work_item, "/source_issue_url")?;
    let body = if let Some(body) = existing_publication
        .and_then(|publication| publication.get("body"))
        .and_then(Value::as_str)
    {
        body.to_owned()
    } else if let Some(path) = args.body_file.as_ref() {
        fs::read_to_string(path).context("read pull request body file")?
    } else {
        format!(
            "## ECorp verified factory deliverable\n\nImplements {issue_url}\n\n- Factory work item: `{}`\n- Mission: `{mission_id}`\n- Verified commit: `{commit_sha}`\n- Deliverable digest: `{}`\n\nCloses #{issue_number}\n\nAuto-merge, merge, and deployment are not authorized by this publication.",
            args.work_item_id,
            value_string(deliverable, "/sha256").unwrap_or_default()
        )
    };
    let authorization_id = args.authorization_id.unwrap_or_else(Uuid::new_v4);
    let effect_key = existing_publication
        .and_then(|publication| publication.get("effect_key"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| args.effect_key.clone())
        .unwrap_or_else(|| {
            format!(
                "github-pr:{}:{}:{}:{}",
                args.work_item_id,
                value_uuid(deliverable, "/id").unwrap_or_default(),
                target_repository,
                branch
            )
        });
    let idempotency_key = args
        .idempotency_key
        .clone()
        .unwrap_or_else(|| format!("{effect_key}:start:{}", args.actor_id));
    let publisher_id = args
        .publisher_id
        .clone()
        .unwrap_or_else(default_publisher_id);
    Ok(PublicationPlan {
        source_deliverable_id: value_uuid(deliverable, "/id")?,
        artifact_id: value_uuid(deliverable, "/artifact_id")?,
        target_repository,
        base_ref,
        branch,
        commit_sha,
        title,
        body,
        authorization_id,
        authorization_reason: args.authorization_reason.trim().to_owned(),
        effect_key,
        idempotency_key,
        publisher_id,
        issue_number,
        issue_url,
        project_owner: value_string(work_item, "/source_project_owner")?,
        project_number: value_i64(work_item, "/source_project_number")?,
        project_item_id: value_string(work_item, "/source_project_item_id")?,
        project_status_before: policy
            .get("project_status")
            .and_then(Value::as_str)
            .unwrap_or("In Progress")
            .to_owned(),
        project_review_status: publication_policy
            .get("review_status")
            .and_then(Value::as_str)
            .context("publication policy omitted review_status")?
            .to_owned(),
    })
}

fn ensure_publication_matches_plan(
    publication: &PullRequestPublication,
    plan: &PublicationPlan,
) -> Result<()> {
    if publication.source_deliverable_id != plan.source_deliverable_id
        || publication.artifact_id != plan.artifact_id
        || publication.target_repository != plan.target_repository
        || publication.base_ref != plan.base_ref
        || publication.branch != plan.branch
        || publication.commit_sha != plan.commit_sha
        || publication.title != plan.title
        || publication.body != plan.body
        || publication.effect_key != plan.effect_key
    {
        bail!("server publication does not match the trusted publisher plan");
    }
    Ok(())
}

fn plan_json(
    args: &FactoryPublishArgs,
    plan: &PublicationPlan,
    mode: &str,
    publication: Option<&PullRequestPublication>,
) -> Value {
    json!({
        "mode": mode,
        "corp_id": args.corp_id,
        "actor_id": args.actor_id,
        "factory_work_item_id": args.work_item_id,
        "source_deliverable_id": plan.source_deliverable_id,
        "artifact_id": plan.artifact_id,
        "target_repository": plan.target_repository,
        "base_ref": plan.base_ref,
        "branch": plan.branch,
        "commit_sha": plan.commit_sha,
        "title": plan.title,
        "issue_number": plan.issue_number,
        "issue_url": plan.issue_url,
        "authorization_id": plan.authorization_id,
        "effect_key": plan.effect_key,
        "idempotency_key": plan.idempotency_key,
        "publisher_id": plan.publisher_id,
        "project_owner": plan.project_owner,
        "project_number": plan.project_number,
        "project_item_id": plan.project_item_id,
        "project_status_before": plan.project_status_before,
        "project_review_status": plan.project_review_status,
        "publication": publication,
        "auto_merge": false,
        "merge": false,
        "deploy": false
    })
}

fn validate_args(args: &FactoryPublishArgs) -> Result<()> {
    if !(5..=3_600).contains(&args.lease_seconds) {
        bail!("publication lease must be between 5 and 3600 seconds");
    }
    if args.wait_seconds > 3_600 {
        bail!("publication wait cannot exceed 3600 seconds");
    }
    if args.authorization_reason.trim().is_empty() {
        bail!("publication authorization reason cannot be empty");
    }
    if args.body_file.as_ref().is_some_and(|path| !path.is_file()) {
        bail!("pull request body file does not exist");
    }
    Ok(())
}

fn default_publisher_id() -> String {
    let host = env::var("COMPUTERNAME")
        .or_else(|_| env::var("HOSTNAME"))
        .unwrap_or_else(|_| "local".to_owned());
    format!("crony-cli:{host}")
}

fn publication_rank(state: PullRequestPublicationState) -> u8 {
    match state {
        PullRequestPublicationState::Requested => 0,
        PullRequestPublicationState::Publishing => 1,
        PullRequestPublicationState::BranchPushed => 2,
        PullRequestPublicationState::PullRequestCreated => 3,
        PullRequestPublicationState::Published => 4,
    }
}

fn git_text(repository: &Path, args: &[&str]) -> Result<String> {
    String::from_utf8(source_git_output(repository, args)?)
        .context("Git output is not UTF-8")
        .map(|value| value.trim().to_owned())
}

fn path_text(path: &Path) -> Result<&str> {
    path.to_str().context("publisher path is not valid UTF-8")
}

fn value_uuid(value: &Value, pointer: &str) -> Result<Uuid> {
    let raw = value
        .pointer(pointer)
        .and_then(Value::as_str)
        .with_context(|| format!("value omitted {pointer}"))?;
    Uuid::parse_str(raw).with_context(|| format!("value {pointer} is not a UUID"))
}

fn value_string(value: &Value, pointer: &str) -> Result<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .with_context(|| format!("value omitted {pointer}"))
}

fn value_i64(value: &Value, pointer: &str) -> Result<i64> {
    value
        .pointer(pointer)
        .and_then(Value::as_i64)
        .with_context(|| format!("value omitted {pointer}"))
}

fn test_crash(stage: &str) {
    if env::var("ECORP_PUBLICATION_TEST_CRASH_AFTER").as_deref() == Ok(stage) {
        std::process::exit(86);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_publication_branch_is_stable() {
        let commit = "a".repeat(40);
        assert_eq!(
            format!(
                "ecorp/issue-61-{}",
                commit.chars().take(12).collect::<String>()
            ),
            "ecorp/issue-61-aaaaaaaaaaaa"
        );
    }

    #[test]
    fn remote_base_refs_are_explicit() {
        assert_eq!(remote_base_reference("main"), "refs/heads/main");
        assert_eq!(remote_base_reference("HEAD"), "HEAD");
        assert_eq!(
            remote_base_reference("refs/heads/release"),
            "refs/heads/release"
        );
    }
}
