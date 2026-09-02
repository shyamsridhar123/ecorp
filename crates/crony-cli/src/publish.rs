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
    gh_json, gh_output, gh_run, normalize_github_component, sanitize_failure_detail, server_json,
    source_git_output,
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
    verified_base_commit: String,
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
    #[serde(rename = "headRefOid")]
    head_ref_oid: String,
    #[serde(rename = "headRepositoryOwner")]
    head_repository_owner: PullRequestRepositoryOwner,
    #[serde(rename = "isCrossRepository")]
    is_cross_repository: bool,
    title: String,
    body: String,
    #[serde(rename = "autoMergeRequest")]
    auto_merge_request: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct PullRequestRepositoryOwner {
    login: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedRemoteBase {
    commit: String,
    pull_request_base_ref: String,
}

#[derive(Debug, Deserialize)]
struct ProjectView {
    id: String,
}

#[derive(Debug, Deserialize)]
struct GraphQlProjectFieldEnvelope {
    data: GraphQlProjectFieldData,
}

#[derive(Debug, Deserialize)]
struct GraphQlProjectFieldData {
    node: Option<GraphQlProjectFieldNode>,
}

#[derive(Debug, Deserialize)]
struct GraphQlProjectFieldNode {
    #[serde(rename = "__typename")]
    kind: String,
    field: Option<GraphQlProjectField>,
}

#[derive(Debug, Deserialize)]
struct GraphQlProjectField {
    #[serde(rename = "__typename")]
    kind: String,
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
struct GraphQlProjectItemEnvelope {
    data: GraphQlProjectItemData,
}

#[derive(Debug, Deserialize)]
struct GraphQlProjectItemData {
    node: Option<GraphQlProjectItem>,
}

#[derive(Debug, Deserialize)]
struct GraphQlProjectItem {
    #[serde(rename = "__typename")]
    kind: String,
    id: String,
    project: GraphQlProject,
    #[serde(rename = "fieldValueByName")]
    status: Option<GraphQlProjectStatus>,
}

#[derive(Debug, Deserialize)]
struct GraphQlProject {
    id: String,
    number: i64,
    owner: GraphQlProjectOwner,
}

#[derive(Debug, Deserialize)]
struct GraphQlProjectOwner {
    login: String,
}

#[derive(Debug, Deserialize)]
struct GraphQlProjectStatus {
    #[serde(rename = "__typename")]
    kind: String,
    name: String,
    field: GraphQlProjectStatusField,
}

#[derive(Debug, Deserialize)]
struct GraphQlProjectStatusField {
    id: String,
    name: String,
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
    let context = server_json(
        client,
        Method::GET,
        format!(
            "{server}/api/corps/{}/factory/work-items/{}/publication-context?actor_id={}",
            args.corp_id, args.work_item_id, args.actor_id
        ),
        None,
    )
    .await?;
    let publication_is_complete = published_publication_exists(&context, args.work_item_id);
    let plan = publication_plan(&args, &context)?;
    validate_publication_branch(&plan.branch)?;
    if args.dry_run {
        return Ok(plan_json(&args, &plan, "dry_run", None));
    }
    if !publication_is_complete {
        preflight_publication_target(&plan)?;
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
    let recovery_key = stable_recovery_idempotency_key(
        args.actor_id,
        &plan.idempotency_key,
        response.publication.version,
        response.publication.attempt_count,
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
    test_crash("after_plan_validation");
    renew_publication(
        client,
        server,
        args,
        plan,
        response,
        publisher_token,
        "prepare",
    )
    .await?;
    let bytes = download_deliverable(client, server, args, plan.artifact_id).await?;
    let document: CommitBranchDocument =
        serde_json::from_slice(&bytes).context("decode commit/branch deliverable")?;
    let bundle = validate_deliverable_document(&document, &response.publication)?;
    let workspace = TemporaryPublisherWorkspace::create()?;
    fs::write(&workspace.bundle, &bundle).context("write portable Git bundle")?;
    fs::write(&workspace.body, plan.body.as_bytes()).context("write pull request body")?;
    let resolved_base = prepare_repository(&workspace, plan, &document)?;
    ensure_distinct_publication_branch(&plan.branch, &resolved_base.pull_request_base_ref)?;
    if let Some(persisted_base) = response.publication.pull_request_base_ref.as_deref()
        && persisted_base != resolved_base.pull_request_base_ref
    {
        bail!(
            "persisted pull request base {persisted_base} no longer matches resolved base {}",
            resolved_base.pull_request_base_ref
        );
    }

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
        let current_base = ensure_remote_base(&workspace.repository, plan, &document.base_commit)?;
        if current_base != resolved_base {
            bail!(
                "remote publication base changed from {} at {} to {} at {}",
                resolved_base.pull_request_base_ref,
                resolved_base.commit,
                current_base.pull_request_base_ref,
                current_base.commit
            );
        }
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
        let pull_request = create_or_adopt_pull_request(
            args,
            plan,
            &resolved_base.pull_request_base_ref,
            &workspace.body,
        )?;
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
                "title": pull_request.title,
                "body": pull_request.body,
                "head_ref": pull_request.head_ref_name,
                "base_ref": pull_request.base_ref_name,
                "head_sha": pull_request.head_ref_oid,
                "head_repository_owner": pull_request.head_repository_owner.login,
                "is_cross_repository": pull_request.is_cross_repository,
                "auto_merge_enabled": pull_request.auto_merge_request.is_some()
            }),
        )
        .await?;
        test_crash("after_pull_request_checkpoint");
    } else {
        let pull_request = find_pull_request(args, plan, &resolved_base.pull_request_base_ref)?
            .context("persisted publication pull request no longer exists")?;
        ensure_remote_pull_request_matches(
            &pull_request,
            plan,
            &resolved_base.pull_request_base_ref,
        )?;
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
        let pull_request = find_pull_request(args, plan, &resolved_base.pull_request_base_ref)?
            .context("persisted publication pull request no longer matches its durable identity")?;
        ensure_remote_pull_request_matches(
            &pull_request,
            plan,
            &resolved_base.pull_request_base_ref,
        )?;
        ensure_pull_request_matches_publication(&pull_request, &response.publication)?;
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
            "lease_seconds": effect_lease_seconds(args),
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
        || document.branch
            != publication
                .provenance
                .pointer("/deliverable/source_branch")
                .and_then(Value::as_str)
                .context("publication provenance omitted source branch")?
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
) -> Result<ResolvedRemoteBase> {
    source_git_output(&workspace.repository, &["init", "--bare"])?;
    add_publication_remote(&workspace.repository, &plan.target_repository)?;
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
    let resolved_base = resolve_remote_base(&workspace.repository, &plan.base_ref)?;
    if !resolved_base
        .commit
        .eq_ignore_ascii_case(&document.base_commit)
    {
        bail!(
            "resolved remote base {} points to {}, not verified commit {}",
            resolved_base.pull_request_base_ref,
            resolved_base.commit,
            document.base_commit
        );
    }
    source_git_output(
        &workspace.repository,
        &["bundle", "verify", path_text(&workspace.bundle)?],
    )
    .context("verify portable Git bundle prerequisites")?;
    let import_ref = "refs/heads/ecorp-import";
    let refspec = format!("HEAD:{import_ref}");
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
    Ok(resolved_base)
}

fn preflight_publication_target(plan: &PublicationPlan) -> Result<()> {
    let workspace = TemporaryPublisherWorkspace::create()?;
    source_git_output(&workspace.repository, &["init", "--bare"])?;
    add_publication_remote(&workspace.repository, &plan.target_repository)?;
    let resolved = resolve_remote_base(&workspace.repository, &plan.base_ref)?;
    if !resolved
        .commit
        .eq_ignore_ascii_case(&plan.verified_base_commit)
    {
        bail!(
            "remote base {} resolved to {}, not verified commit {}",
            resolved.pull_request_base_ref,
            resolved.commit,
            plan.verified_base_commit
        );
    }
    ensure_distinct_publication_branch(&plan.branch, &resolved.pull_request_base_ref)
}

fn add_publication_remote(repository: &Path, target_repository: &str) -> Result<()> {
    let remote_url = env::var("ECORP_PUBLICATION_TEST_REMOTE_URL")
        .unwrap_or_else(|_| format!("https://github.com/{target_repository}.git"));
    source_git_output(repository, &["remote", "add", "origin", &remote_url])?;
    Ok(())
}

fn ensure_remote_base(
    repository: &Path,
    plan: &PublicationPlan,
    expected_commit: &str,
) -> Result<ResolvedRemoteBase> {
    let resolved = resolve_remote_base(repository, &plan.base_ref)?;
    if !resolved.commit.eq_ignore_ascii_case(expected_commit) {
        bail!(
            "remote base {} moved from verified commit {expected_commit} to {}",
            resolved.pull_request_base_ref,
            resolved.commit
        );
    }
    Ok(resolved)
}

fn resolve_remote_base(repository: &Path, base_ref: &str) -> Result<ResolvedRemoteBase> {
    if base_ref == "HEAD" {
        let output = source_git_output(repository, &["ls-remote", "--symref", "origin", "HEAD"])?;
        let text = String::from_utf8(output).context("remote Git HEAD is not UTF-8")?;
        let (symref, commit) = parse_remote_head(&text)?;
        let target_commit = remote_reference_commit(repository, &symref)?
            .context("remote Git HEAD symbolic target does not exist")?;
        if target_commit != commit {
            bail!(
                "remote Git HEAD object {commit} does not match symbolic target {symref} at {target_commit}"
            );
        }
        let pull_request_base_ref = pull_request_base_name(&symref)?;
        return Ok(ResolvedRemoteBase {
            commit,
            pull_request_base_ref,
        });
    }
    let reference = remote_base_reference(base_ref);
    let commit = remote_reference_commit(repository, &reference)?
        .with_context(|| format!("remote base reference {reference} does not exist"))?;
    let pull_request_base_ref = pull_request_base_name(&reference)?;
    Ok(ResolvedRemoteBase {
        commit,
        pull_request_base_ref,
    })
}

fn ensure_distinct_publication_branch(branch: &str, pull_request_base_ref: &str) -> Result<()> {
    if branch == pull_request_base_ref {
        bail!(
            "publication branch {branch} must differ from resolved pull request base {pull_request_base_ref}"
        );
    }
    Ok(())
}

fn pull_request_base_name(reference: &str) -> Result<String> {
    reference
        .strip_prefix("refs/heads/")
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .context("remote publication base did not resolve to a branch")
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
    if reference == "HEAD" {
        let output = source_git_output(repository, &["ls-remote", "--symref", "origin", "HEAD"])?;
        let text = String::from_utf8(output).context("remote Git HEAD is not UTF-8")?;
        let (symref, commit) = parse_remote_head(&text)?;
        let target_commit = remote_reference_commit(repository, &symref)?
            .context("remote Git HEAD symbolic target does not exist")?;
        if target_commit != commit {
            bail!(
                "remote Git HEAD object {commit} does not match symbolic target {symref} at {target_commit}"
            );
        }
        return Ok(Some(commit));
    }
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

fn parse_remote_head(output: &str) -> Result<(String, String)> {
    let mut symref = None;
    let mut commit = None;
    for line in output.lines() {
        let mut fields = line.split_whitespace();
        let first = fields.next().unwrap_or_default();
        let second = fields.next().unwrap_or_default();
        let third = fields.next();
        if first == "ref:" && second.starts_with("refs/heads/") && third == Some("HEAD") {
            if symref.replace(second.to_owned()).is_some() {
                bail!("remote Git HEAD advertised multiple symbolic targets");
            }
        } else if second == "HEAD"
            && matches!(first.len(), 40 | 64)
            && first.bytes().all(|byte| byte.is_ascii_hexdigit())
            && commit.replace(first.to_ascii_lowercase()).is_some()
        {
            bail!("remote Git HEAD advertised multiple object ids");
        }
    }
    let symref = symref.context("remote Git HEAD omitted its symbolic branch target")?;
    let commit = commit.context("remote Git HEAD omitted its object id")?;
    if symref == "refs/heads/HEAD" {
        bail!("remote Git HEAD advertised an invalid symbolic target");
    }
    Ok((symref, commit))
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
    pull_request_base_ref: &str,
    body_file: &Path,
) -> Result<PullRequestView> {
    if let Some(existing) = find_pull_request(args, plan, pull_request_base_ref)? {
        ensure_remote_pull_request_matches(&existing, plan, pull_request_base_ref)?;
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
            pull_request_base_ref,
            "--head",
            &plan.branch,
            "--title",
            &plan.title,
            "--body-file",
            path_text(body_file)?,
        ],
    );
    if let Err(error) = create {
        if let Some(recovered) = find_pull_request(args, plan, pull_request_base_ref)? {
            ensure_remote_pull_request_matches(&recovered, plan, pull_request_base_ref)?;
            return Ok(recovered);
        }
        return Err(error).context("create GitHub pull request");
    }
    let created = find_pull_request(args, plan, pull_request_base_ref)?
        .context("GitHub pull request creation returned success but no pull request exists")?;
    ensure_remote_pull_request_matches(&created, plan, pull_request_base_ref)?;
    Ok(created)
}

fn find_pull_request(
    args: &FactoryPublishArgs,
    plan: &PublicationPlan,
    pull_request_base_ref: &str,
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
            pull_request_base_ref,
            "--limit",
            "100",
            "--json",
            "number,id,url,state,isDraft,title,body,headRefName,baseRefName,headRefOid,headRepositoryOwner,isCrossRepository,autoMergeRequest",
        ],
    )?;
    let pull_requests: Vec<PullRequestView> =
        serde_json::from_value(value).context("decode GitHub pull request list")?;
    let mut authorized = pull_requests
        .into_iter()
        .filter(|pull_request| {
            pull_request.head_ref_name == plan.branch
                && pull_request.base_ref_name == pull_request_base_ref
                && pull_request_matches_verified_head(pull_request, plan)
        })
        .collect::<Vec<_>>();
    if authorized.len() > 1 {
        bail!(
            "multiple verified pull requests exist for {} -> {}",
            plan.branch,
            pull_request_base_ref
        );
    }
    Ok(authorized.pop())
}

fn pull_request_matches_verified_head(
    pull_request: &PullRequestView,
    plan: &PublicationPlan,
) -> bool {
    let target_owner = plan
        .target_repository
        .split_once('/')
        .map(|(owner, _)| owner)
        .unwrap_or_default();
    !pull_request.is_cross_repository
        && pull_request
            .head_repository_owner
            .login
            .eq_ignore_ascii_case(target_owner)
        && pull_request
            .head_ref_oid
            .eq_ignore_ascii_case(&plan.commit_sha)
        && pull_request.title == plan.title
        && pull_request.body == plan.body
}

fn ensure_remote_pull_request_matches(
    pull_request: &PullRequestView,
    plan: &PublicationPlan,
    pull_request_base_ref: &str,
) -> Result<()> {
    if pull_request.state != "OPEN"
        || pull_request.head_ref_name != plan.branch
        || pull_request.base_ref_name != pull_request_base_ref
        || !pull_request_matches_verified_head(pull_request, plan)
        || pull_request.auto_merge_request.is_some()
    {
        bail!("GitHub pull request does not match the authorized non-merging publication");
    }
    Ok(())
}

fn ensure_pull_request_matches_publication(
    pull_request: &PullRequestView,
    publication: &PullRequestPublication,
) -> Result<()> {
    if publication.pull_request_number != Some(pull_request.number)
        || publication.pull_request_node_id.as_deref() != Some(pull_request.id.as_str())
        || publication.pull_request_url.as_deref() != Some(pull_request.url.as_str())
        || publication.pull_request_state.as_deref() != Some(pull_request.state.as_str())
        || publication.pull_request_draft != Some(pull_request.is_draft)
        || publication.pull_request_base_ref.as_deref() != Some(pull_request.base_ref_name.as_str())
        || !publication
            .pull_request_head_sha
            .as_deref()
            .is_some_and(|sha| sha.eq_ignore_ascii_case(&pull_request.head_ref_oid))
        || !publication
            .pull_request_head_repository_owner
            .as_deref()
            .is_some_and(|owner| {
                owner.eq_ignore_ascii_case(&pull_request.head_repository_owner.login)
            })
        || publication.pull_request_is_cross_repository != Some(pull_request.is_cross_repository)
    {
        bail!("GitHub pull request no longer matches the durable publication identity");
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
    let status_field = load_project_status_field(args, &project.id)?;
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
    let item = load_project_item(args, plan)?;
    if item.kind != "ProjectV2Item"
        || item.id != plan.project_item_id
        || item.project.id != project.id
        || item.project.number != plan.project_number
        || !item
            .project
            .owner
            .login
            .eq_ignore_ascii_case(&plan.project_owner)
    {
        bail!("GitHub Project item does not match the authorized publication Project");
    }
    let status = item
        .status
        .context("GitHub Project item has no Status value")?;
    if status.kind != "ProjectV2ItemFieldSingleSelectValue"
        || status.field.id != status_field.id
        || status.field.name != "Status"
    {
        bail!("GitHub Project item Status value does not match the Project Status field");
    }
    Ok((
        project.id,
        status_field.id.clone(),
        review_option.id.clone(),
        status.name,
    ))
}

fn load_project_status_field(
    args: &FactoryPublishArgs,
    project_id: &str,
) -> Result<GraphQlProjectField> {
    const QUERY: &str = r#"query($id:ID!){node(id:$id){__typename ... on ProjectV2{field(name:"Status"){__typename ... on ProjectV2SingleSelectField{id name options{id name}}}}}}"#;
    let id = format!("id={project_id}");
    let query = format!("query={QUERY}");
    let envelope: GraphQlProjectFieldEnvelope = serde_json::from_value(gh_json(
        &args.github_cli,
        &["api", "graphql", "-f", &query, "-F", &id],
    )?)
    .context("decode exact GitHub Project Status field")?;
    let node = envelope
        .data
        .node
        .context("GitHub Project disappeared during publication")?;
    let field = node.field.context("GitHub Project has no Status field")?;
    if node.kind != "ProjectV2"
        || field.kind != "ProjectV2SingleSelectField"
        || field.name != "Status"
    {
        bail!("GitHub Project Status field has an unexpected identity");
    }
    Ok(field)
}

fn load_project_item(
    args: &FactoryPublishArgs,
    plan: &PublicationPlan,
) -> Result<GraphQlProjectItem> {
    const QUERY: &str = r#"query($id:ID!){node(id:$id){__typename ... on ProjectV2Item{id project{id number owner{__typename ... on User{login} ... on Organization{login}}} fieldValueByName(name:"Status"){__typename ... on ProjectV2ItemFieldSingleSelectValue{name field{... on ProjectV2SingleSelectField{id name}}}}}}}"#;
    let id = format!("id={}", plan.project_item_id);
    let envelope: GraphQlProjectItemEnvelope = serde_json::from_value(gh_json(
        &args.github_cli,
        &["api", "graphql", "-f", &format!("query={QUERY}"), "-F", &id],
    )?)
    .context("decode exact GitHub Project item")?;
    envelope
        .data
        .node
        .context("GitHub Project item disappeared during publication")
}

fn publication_plan(args: &FactoryPublishArgs, context: &Value) -> Result<PublicationPlan> {
    let work_item = context
        .get("work_item")
        .context("ECorp publication context omitted the factory work item")?;
    if value_uuid(work_item, "/id")? != args.work_item_id {
        bail!("ECorp publication context returned a different factory work item");
    }
    let existing_publication = context.get("publication").filter(|value| !value.is_null());
    if existing_publication.is_some_and(|publication| {
        value_uuid(publication, "/factory_work_item_id").ok() != Some(args.work_item_id)
    }) {
        bail!("ECorp publication context returned a publication for another work item");
    }
    let mission_id =
        value_uuid(work_item, "/mission_id").context("factory work item has no mission")?;
    let persisted_deliverable_id = existing_publication
        .and_then(|publication| publication.get("source_deliverable_id"))
        .and_then(Value::as_str)
        .map(Uuid::parse_str)
        .transpose()
        .context("persisted publication deliverable id is invalid")?;
    let selected_deliverable_id =
        publication_deliverable_selection(args.source_deliverable_id, persisted_deliverable_id)?;
    let deliverables = context
        .get("source_deliverables")
        .and_then(Value::as_array)
        .context("ECorp publication context omitted source deliverables")?
        .iter()
        .filter(|deliverable| {
            deliverable.get("form").and_then(Value::as_str) == Some("commit_branch")
                && match deliverable.get("integration_state").and_then(Value::as_str) {
                    Some("ready_for_review") => true,
                    Some("published") => existing_publication.is_some(),
                    _ => false,
                }
                && deliverable.get("head_commit").is_some_and(Value::is_string)
        })
        .filter(|deliverable| {
            selected_deliverable_id
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
    let target_repository = normalize_publication_repository(&target_repository)?;
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
    let verified_base_commit = value_string(deliverable, "/base_commit")?;
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
    let title = normalize_publication_title(&title)?;
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
    let body = normalize_publication_body(&body)?;
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
    let persisted_publication_actor_id = existing_publication
        .and_then(|publication| publication.get("actor_id"))
        .and_then(Value::as_str)
        .map(Uuid::parse_str)
        .transpose()
        .context("persisted publication actor id is invalid")?;
    let persisted_authorization_id = existing_publication
        .and_then(|publication| publication.get("authorization_id"))
        .and_then(Value::as_str)
        .map(Uuid::parse_str)
        .transpose()
        .context("persisted publication authorization id is invalid")?;
    let same_authorization_actor = persisted_publication_actor_id == Some(args.actor_id);
    let reusable_authorization_id = same_authorization_actor
        .then_some(persisted_authorization_id)
        .flatten();
    let authorization_id = match (args.authorization_id, reusable_authorization_id) {
        (Some(requested), Some(persisted)) if requested != persisted => {
            bail!(
                "requested authorization id does not match the persisted publication authorization"
            )
        }
        (Some(requested), _) => requested,
        (None, Some(persisted)) => persisted,
        (None, None) => stable_authorization_id(args.actor_id, &effect_key),
    };
    let publisher_id = args
        .publisher_id
        .clone()
        .unwrap_or_else(default_publisher_id);
    let mut plan = PublicationPlan {
        source_deliverable_id: value_uuid(deliverable, "/id")?,
        artifact_id: value_uuid(deliverable, "/artifact_id")?,
        target_repository,
        base_ref,
        verified_base_commit,
        branch,
        commit_sha,
        title,
        body,
        authorization_id,
        authorization_reason: args.authorization_reason.trim().to_owned(),
        effect_key,
        idempotency_key: String::new(),
        publisher_id,
        issue_number,
        issue_url,
        project_owner: value_string(work_item, "/source_project_owner")?,
        project_number: value_i64(work_item, "/source_project_number")?,
        project_item_id: value_string(work_item, "/source_project_item_id")?,
        project_status_before: publication_policy
            .get("status_before")
            .and_then(Value::as_str)
            .context("publication policy omitted status_before")?
            .to_owned(),
        project_review_status: publication_policy
            .get("review_status")
            .and_then(Value::as_str)
            .context("publication policy omitted review_status")?
            .to_owned(),
    };
    plan.idempotency_key = match args.idempotency_key.clone() {
        Some(key) => key,
        None => stable_start_idempotency_key(args, &plan)?,
    };
    Ok(plan)
}

fn publication_deliverable_selection(
    requested: Option<Uuid>,
    persisted: Option<Uuid>,
) -> Result<Option<Uuid>> {
    match (requested, persisted) {
        (Some(requested), Some(persisted)) if requested != persisted => {
            bail!(
                "requested source deliverable does not match the persisted publication deliverable"
            )
        }
        (Some(requested), _) => Ok(Some(requested)),
        (None, Some(persisted)) => Ok(Some(persisted)),
        (None, None) => Ok(None),
    }
}

fn published_publication_exists(context: &Value, work_item_id: Uuid) -> bool {
    context
        .get("publication")
        .filter(|publication| !publication.is_null())
        .is_some_and(|publication| {
            value_uuid(publication, "/factory_work_item_id").ok() == Some(work_item_id)
                && publication.get("state").and_then(Value::as_str) == Some("published")
        })
}

fn normalize_publication_body(value: &str) -> Result<String> {
    let value = value.replace("\r\n", "\n").replace('\r', "\n");
    let value = value.trim();
    if value.is_empty() {
        bail!("pull request body cannot be empty");
    }
    if value.len() > 65_536 {
        bail!("pull request body cannot exceed 65536 bytes");
    }
    if value.contains('\0') {
        bail!("pull request body cannot contain NUL bytes");
    }
    Ok(value.to_owned())
}

fn normalize_publication_repository(value: &str) -> Result<String> {
    let repository = value.trim();
    let mut parts = repository.split('/');
    let owner = parts
        .next()
        .context("publication target repository omitted owner")?;
    let name = parts
        .next()
        .context("publication target repository omitted name")?;
    if parts.next().is_some() {
        bail!("publication target repository must use owner/name form");
    }
    Ok(format!(
        "{}/{}",
        normalize_github_component(owner, "publication repository owner")?,
        normalize_github_component(name, "publication repository name")?
    ))
}

fn normalize_publication_title(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        bail!("pull request title cannot be empty");
    }
    if value.len() > 256 {
        bail!("pull request title cannot exceed 256 bytes");
    }
    if value.chars().any(char::is_control) {
        bail!("pull request title cannot contain control characters");
    }
    Ok(value.to_owned())
}

fn validate_publication_branch(branch: &str) -> Result<()> {
    source_git_output(Path::new("."), &["check-ref-format", "--branch", branch])
        .map(|_| ())
        .with_context(|| format!("publication branch {branch} is not a valid Git branch name"))
}

fn stable_authorization_id(actor_id: Uuid, effect_key: &str) -> Uuid {
    let mut digest = Sha256::new();
    digest.update(b"ecorp-publication-authorization-v1\0");
    digest.update(actor_id.as_bytes());
    digest.update(effect_key.as_bytes());
    let digest = digest.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

fn stable_start_idempotency_key(
    args: &FactoryPublishArgs,
    plan: &PublicationPlan,
) -> Result<String> {
    let request = json!({
        "actor_id": args.actor_id,
        "work_item_id": args.work_item_id,
        "source_deliverable_id": plan.source_deliverable_id,
        "target_repository": plan.target_repository,
        "base_ref": plan.base_ref,
        "branch": plan.branch,
        "title": plan.title,
        "body": plan.body,
        "authorization_id": plan.authorization_id,
        "authorization_reason": plan.authorization_reason,
        "effect_key": plan.effect_key,
        "publisher_id": plan.publisher_id,
        "lease_seconds": args.lease_seconds,
    });
    stable_start_idempotency_key_for_request(args.actor_id, &request)
}

fn stable_start_idempotency_key_for_request(actor_id: Uuid, request: &Value) -> Result<String> {
    let encoded = serde_json::to_vec(&request).context("encode publication start identity")?;
    let digest = format!("{:x}", Sha256::digest(encoded));
    Ok(format!("publication-start:{actor_id}:{}", &digest[..32]))
}

fn stable_recovery_idempotency_key(
    actor_id: Uuid,
    start_idempotency_key: &str,
    version: i64,
    attempt: i32,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"ecorp-publication-recovery-v1\0");
    digest.update(start_idempotency_key.as_bytes());
    let digest = format!("{:x}", digest.finalize());
    format!(
        "publication-recover:{actor_id}:{version}:{attempt}:{}",
        &digest[..16]
    )
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
        "verified_base_commit": plan.verified_base_commit,
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

fn effect_lease_seconds(args: &FactoryPublishArgs) -> i64 {
    env::var("ECORP_PUBLICATION_EFFECT_LEASE_SECONDS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or_else(|| args.lease_seconds.max(120))
        .clamp(args.lease_seconds, 3_600)
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

    #[test]
    fn remote_head_requires_one_symbolic_branch_and_object() {
        let commit = "a".repeat(40);
        let output = format!("ref: refs/heads/main\tHEAD\n{commit}\tHEAD\n");
        assert_eq!(
            parse_remote_head(&output).expect("parse HEAD"),
            ("refs/heads/main".to_owned(), commit.clone())
        );
        assert!(parse_remote_head(&format!("{commit}\tHEAD\n")).is_err());
        assert!(parse_remote_head("ref: refs/heads/main\tHEAD\n").is_err());
        assert_eq!(
            pull_request_base_name("refs/heads/main").expect("branch name"),
            "main"
        );
        assert!(pull_request_base_name("HEAD").is_err());
    }

    #[test]
    fn publication_branch_authorization_and_body_are_retry_stable() {
        assert!(ensure_distinct_publication_branch("feature", "main").is_ok());
        assert!(ensure_distinct_publication_branch("main", "main").is_err());
        assert_eq!(
            normalize_publication_body("line one\r\nline two\r\n").expect("normalize"),
            "line one\nline two"
        );
        assert_eq!(
            normalize_publication_title("  Review title  ").expect("normalize title"),
            "Review title"
        );
        assert_eq!(
            normalize_publication_repository(" ShyamSridhar123 / ECorp ")
                .expect("normalize repository"),
            "shyamsridhar123/ecorp"
        );
        assert!(normalize_publication_repository("owner/repo/extra").is_err());
        assert!(normalize_publication_title("   ").is_err());
        assert!(normalize_publication_title("bad\ntitle").is_err());
        assert!(normalize_publication_title(&"x".repeat(257)).is_err());
        let actor = Uuid::new_v4();
        let first = stable_authorization_id(actor, "effect");
        assert_eq!(first, stable_authorization_id(actor, "effect"));
        assert_ne!(first, stable_authorization_id(actor, "other-effect"));
        assert_ne!(first, stable_authorization_id(Uuid::new_v4(), "effect"));
        let work_item_id = Uuid::new_v4();
        assert!(published_publication_exists(
            &json!({
                "publication": {
                    "factory_work_item_id": work_item_id,
                    "state": "published"
                }
            }),
            work_item_id
        ));
        let persisted_deliverable = Uuid::new_v4();
        assert_eq!(
            publication_deliverable_selection(None, Some(persisted_deliverable))
                .expect("persisted deliverable"),
            Some(persisted_deliverable)
        );
        assert!(
            publication_deliverable_selection(Some(Uuid::new_v4()), Some(persisted_deliverable))
                .is_err()
        );
    }

    #[test]
    fn publication_start_and_recovery_keys_track_the_exact_invocation() {
        let actor = Uuid::new_v4();
        let request = json!({
            "publisher_id": "publisher-a",
            "authorization_reason": "reason-a",
            "lease_seconds": 30
        });
        let first = stable_start_idempotency_key_for_request(actor, &request).expect("start key");
        assert_eq!(
            first,
            stable_start_idempotency_key_for_request(actor, &request).expect("repeat start key")
        );
        for changed in [
            json!({
                "publisher_id": "publisher-b",
                "authorization_reason": "reason-a",
                "lease_seconds": 30
            }),
            json!({
                "publisher_id": "publisher-a",
                "authorization_reason": "reason-b",
                "lease_seconds": 30
            }),
            json!({
                "publisher_id": "publisher-a",
                "authorization_reason": "reason-a",
                "lease_seconds": 60
            }),
        ] {
            assert_ne!(
                first,
                stable_start_idempotency_key_for_request(actor, &changed).expect("changed key")
            );
        }
        assert_ne!(
            first,
            stable_start_idempotency_key_for_request(Uuid::new_v4(), &request)
                .expect("other actor key")
        );

        let recovery = stable_recovery_idempotency_key(actor, &first, 4, 2);
        assert_eq!(
            recovery,
            stable_recovery_idempotency_key(actor, &first, 4, 2)
        );
        assert_ne!(
            recovery,
            stable_recovery_idempotency_key(actor, &first, 5, 2)
        );
        assert_ne!(
            recovery,
            stable_recovery_idempotency_key(Uuid::new_v4(), &first, 4, 2)
        );
    }

    #[test]
    fn publication_branch_uses_git_branch_rules() {
        assert!(validate_publication_branch("ecorp/issue-61-safe").is_ok());
        assert!(validate_publication_branch("ecorp/foo//bar").is_err());
        assert!(validate_publication_branch("ecorp/foo.lock").is_err());
    }
}
