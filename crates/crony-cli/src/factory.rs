use std::{
    collections::{HashMap, HashSet},
    env,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use clap::Args;
use reqwest::{Client, Method, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Debug, Args)]
pub struct FactoryArgs {
    pub corp_id: Uuid,
    pub actor_id: Uuid,

    #[arg(
        long,
        env = "ECORP_GITHUB_PROJECT_OWNER",
        default_value = "shyamsridhar123"
    )]
    pub owner: String,

    #[arg(long, env = "ECORP_GITHUB_PROJECT_NUMBER", default_value_t = 3)]
    pub project_number: u32,

    #[arg(
        long,
        env = "ECORP_FACTORY_REPOSITORY",
        default_value = "shyamsridhar123/ecorp"
    )]
    pub repository: String,

    #[arg(long, env = "ECORP_FACTORY_SOURCE_BASE_REF", default_value = "HEAD")]
    pub source_base_ref: String,

    #[arg(long)]
    pub adapter: String,

    #[arg(long = "allow-adapter")]
    pub allowed_adapters: Vec<String>,

    #[arg(long, default_value = "single")]
    pub strategy: String,

    #[arg(long)]
    pub model: Option<String>,

    #[arg(long)]
    pub reasoning_effort: Option<String>,

    #[arg(long)]
    pub budget_tokens: i64,

    #[arg(long)]
    pub budget_cost_microusd: i64,

    #[arg(long, default_value_t = 300)]
    pub lease_seconds: i64,

    #[arg(long, default_value = "**")]
    pub write_scope: Vec<String>,

    #[arg(long)]
    pub issue: Option<i64>,

    #[arg(long)]
    pub dry_run: bool,

    #[arg(long, env = "ECORP_GITHUB_CLI", default_value = "gh")]
    pub github_cli: PathBuf,
}

#[derive(Debug, Deserialize)]
struct ProjectItemsEnvelope {
    #[serde(default)]
    items: Vec<ProjectItem>,
    #[serde(rename = "totalCount")]
    total_count: usize,
}

#[derive(Debug, Clone, Deserialize)]
struct ProjectItem {
    id: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    content: ProjectContent,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ProjectContent {
    #[serde(default)]
    body: String,
    #[serde(default)]
    number: i64,
    #[serde(default)]
    repository: String,
    #[serde(default)]
    title: String,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct IssueLabel {
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct IssueView {
    id: String,
    number: i64,
    title: String,
    body: String,
    url: String,
    state: String,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(rename = "updatedAt")]
    updated_at: String,
    #[serde(default)]
    labels: Vec<IssueLabel>,
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

#[derive(Debug, Clone)]
struct ExistingFactoryItem {
    source_revision: String,
    claim_owner_id: Uuid,
    state: String,
    mission_id: Option<Uuid>,
    policy: Value,
    lease_expires_at: DateTime<Utc>,
}

#[derive(Debug)]
struct EvaluatedItem {
    project_item: ProjectItem,
    issue: IssueView,
    dependencies: Vec<i64>,
    reasons: Vec<String>,
    recovery: bool,
}

impl EvaluatedItem {
    fn eligible(&self) -> bool {
        self.reasons.is_empty()
    }

    fn as_json(&self) -> Value {
        json!({
            "project_item_id": self.project_item.id,
            "project_status": self.project_item.status,
            "issue_number": self.issue.number,
            "title": self.issue.title,
            "url": self.issue.url,
            "source_revision": self.issue.updated_at,
            "dependencies": self.dependencies,
            "recovery": self.recovery,
            "eligible": self.eligible(),
            "reasons": self.reasons,
        })
    }
}

pub async fn run(client: &Client, server: &str, mut args: FactoryArgs) -> Result<Value> {
    normalize_args(&mut args)?;
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
    let existing = existing_factory_items(&snapshot)?;
    let project_items = load_project_items(&args.github_cli, &args.owner, args.project_number)?;
    if project_items.items.len() < project_items.total_count {
        bail!(
            "GitHub Project returned {} of {} items; increase the controller query limit",
            project_items.items.len(),
            project_items.total_count
        );
    }
    let mut issue_cache = HashMap::new();
    let mut evaluated = evaluate_items(&args, project_items.items, &existing, &mut issue_cache)?;
    evaluated.sort_by(|left, right| {
        right
            .recovery
            .cmp(&left.recovery)
            .then_with(|| left.issue.created_at.cmp(&right.issue.created_at))
            .then_with(|| left.issue.number.cmp(&right.issue.number))
    });
    let selected_index = evaluated.iter().position(EvaluatedItem::eligible);
    let evaluated_json = evaluated
        .iter()
        .map(EvaluatedItem::as_json)
        .collect::<Vec<_>>();

    if args.dry_run {
        return Ok(json!({
            "mode": "dry_run",
            "source_of_truth": "github_project",
            "project_owner": args.owner,
            "project_number": args.project_number,
            "repository": args.repository,
            "selected": selected_index.map(|index| evaluated[index].as_json()),
            "evaluated": evaluated_json,
            "mutations": [],
        }));
    }

    let selected_index = selected_index.ok_or_else(|| {
        anyhow!(
            "no eligible factory issue found; evaluated candidates: {}",
            serde_json::to_string(&evaluated_json).unwrap_or_else(|_| "[]".to_owned())
        )
    })?;
    let selected = evaluated.swap_remove(selected_index);
    let refreshed = refresh_selected(&args, &existing, &selected)?;
    let adapter_allowlist = factory_adapter_allowlist(&args)?;
    let stable_prefix = format!(
        "github-project:{}:{}:{}:{}",
        args.owner, args.project_number, refreshed.project_item.id, refreshed.issue.updated_at
    );
    let claim_path = format!(
        "{server}/api/corps/{}/factory/work-items/claim",
        args.corp_id
    );
    let policy = if refreshed.recovery {
        existing
            .get(&refreshed.project_item.id)
            .map(|item| item.policy.clone())
            .context("recoverable factory item disappeared from the ECorp snapshot")?
    } else {
        json!({
            "schema_version": 1,
            "source_of_truth": "github_project",
            "project_owner": args.owner,
            "project_number": args.project_number,
            "project_status": refreshed.project_item.status,
            "required_label": "factory:ready",
            "dependencies": refreshed.dependencies,
            "repository_allowlist": [args.repository],
            "source_base_ref": args.source_base_ref,
            "adapter_allowlist": adapter_allowlist,
            "strategy_allowlist": [args.strategy],
            "model": args.model,
            "reasoning_effort": args.reasoning_effort,
            "write_scope": args.write_scope,
            "allowed_tools": ["filesystem", "shell"],
            "prohibited_actions": [
                "modify files outside the assigned worktree",
                "use undeclared long-lived credentials",
                "merge or deploy without a separate current authorization"
            ],
            "secret_ids": [],
            "verification_required": true,
            "budget_tokens": args.budget_tokens,
            "budget_cost_microusd": args.budget_cost_microusd,
            "auto_merge": false,
        })
    };
    let claim_body = json!({
        "actor_id": args.actor_id,
        "source_project_owner": args.owner,
        "source_project_number": args.project_number,
        "source_project_item_id": refreshed.project_item.id,
        "source_repository_owner": repository_parts(&args.repository)?.0,
        "source_repository_name": repository_parts(&args.repository)?.1,
        "source_issue_number": refreshed.issue.number,
        "source_issue_node_id": refreshed.issue.id,
        "source_issue_url": refreshed.issue.url,
        "source_title": refreshed.issue.title,
        "source_revision": refreshed.issue.updated_at,
        "idempotency_key": format!("{stable_prefix}:claim:{}", args.actor_id),
        "lease_seconds": args.lease_seconds,
        "policy": policy,
    });
    let mut claim = server_json(
        client,
        Method::POST,
        claim_path.clone(),
        Some(claim_body.clone()),
    )
    .await?;
    if claim.get("claim_token").is_none_or(Value::is_null) {
        let replay_version = value_i64(&claim, "/work_item/version")?;
        let mut reclaim_body = claim_body;
        reclaim_body["idempotency_key"] = Value::String(format!(
            "{stable_prefix}:reclaim:{}:{replay_version}",
            args.actor_id
        ));
        claim = server_json(client, Method::POST, claim_path, Some(reclaim_body)).await?;
    }

    let work_item_id = value_uuid(&claim, "/work_item/id")?;
    let mut mission_id = value_optional_uuid(&claim, "/work_item/mission_id")?;
    let control_token = value_uuid(&claim, "/claim_token")
        .context("factory work item has no usable controller fencing token")?;
    let mut work_item_version = value_i64(&claim, "/work_item/version")?;
    let mut materialized = false;
    if mission_id.is_none() {
        let materialize_body = json!({
            "actor_id": args.actor_id,
            "claim_token": control_token,
            "expected_version": work_item_version,
            "idempotency_key": format!("{stable_prefix}:materialize"),
            "title": bounded_title(refreshed.issue.number, &refreshed.issue.title),
            "preferred_adapter": args.adapter,
            "preferred_model": args.model,
            "reasoning_effort": args.reasoning_effort,
            "strategy": args.strategy,
            "budget_tokens": args.budget_tokens,
            "budget_cost_microusd": args.budget_cost_microusd,
            "contract": {
                "objective": issue_objective(&refreshed.issue),
                "expected_output": format!(
                    "A complete, verified repository implementation for GitHub issue #{}.",
                    refreshed.issue.number
                ),
                "acceptance_tests": acceptance_tests(&refreshed.issue.body),
                "allowed_tools": ["filesystem", "shell"],
                "prohibited_actions": [
                    "modify files outside the assigned worktree",
                    "use undeclared long-lived credentials",
                    "merge or deploy without a separate current authorization"
                ],
                "references": [
                    refreshed.issue.url,
                    format!(
                        "https://github.com/users/{}/projects/{}",
                        args.owner, args.project_number
                    )
                ],
                "write_scope": args.write_scope,
            }
        });
        let materialize = server_json(
            client,
            Method::POST,
            format!(
                "{server}/api/corps/{}/factory/work-items/{work_item_id}/materialize",
                args.corp_id
            ),
            Some(materialize_body),
        )
        .await?;
        mission_id = Some(value_uuid(&materialize, "/mission_id")?);
        work_item_version = value_i64(&materialize, "/work_item/version")?;
        materialized = !materialize
            .get("replayed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    }
    let mission_id = mission_id.context("factory work item did not produce a mission")?;
    let effect_lease_seconds = args.lease_seconds.max(300);
    let renewed = renew_factory_control(
        client,
        server,
        &args,
        work_item_id,
        control_token,
        work_item_version,
        effect_lease_seconds,
        "project-status",
        &stable_prefix,
    )
    .await?;
    let mut work_item_state = renewed.0;
    work_item_version = renewed.1;

    if let Err(error) = revalidate_selected_for_effect(
        &args,
        &refreshed,
        &refreshed.project_item.status,
        "GitHub Project status update",
    ) {
        if work_item_state != "blocked" {
            transition_factory_state(
                client,
                server,
                &args,
                work_item_id,
                control_token,
                work_item_version,
                "blocked",
                Some(format!(
                    "GitHub source revalidation failed before Project status update: {error}"
                )),
                &stable_prefix,
            )
            .await
            .context("persist blocked state after GitHub source revalidation failure")?;
        }
        return Err(error).context("revalidate GitHub source before Project status update");
    }

    let project_update = set_project_in_progress(
        &args.github_cli,
        &args.owner,
        args.project_number,
        &refreshed.project_item.id,
    )
    .and_then(|_| {
        verify_project_status(
            &args.github_cli,
            &args.owner,
            args.project_number,
            &refreshed.project_item.id,
            "In Progress",
        )
    });
    if let Err(error) = project_update {
        let _ = transition_factory_state(
            client,
            server,
            &args,
            work_item_id,
            control_token,
            work_item_version,
            "blocked",
            Some(format!(
                "GitHub Project status synchronization failed: {error}"
            )),
            &stable_prefix,
        )
        .await;
        return Err(error).context("update GitHub Project status after mission linkage");
    }
    let renewed = renew_factory_control(
        client,
        server,
        &args,
        work_item_id,
        control_token,
        work_item_version,
        effect_lease_seconds,
        "mission-launch",
        &stable_prefix,
    )
    .await
    .context("revalidate factory lease after GitHub Project update")?;
    work_item_state = renewed.0;
    work_item_version = renewed.1;

    if let Err(error) =
        revalidate_selected_for_effect(&args, &refreshed, "In Progress", "mission launch")
    {
        if work_item_state != "blocked" {
            transition_factory_state(
                client,
                server,
                &args,
                work_item_id,
                control_token,
                work_item_version,
                "blocked",
                Some(format!(
                    "GitHub source revalidation failed before mission launch: {error}"
                )),
                &stable_prefix,
            )
            .await
            .context("persist blocked state after GitHub source revalidation failure")?;
        }
        return Err(error).context("revalidate GitHub source before mission launch");
    }

    let launch = match launch_or_recover(client, server, &args, mission_id).await {
        Ok(launch) => launch,
        Err(error) => {
            if work_item_state != "blocked" {
                let _ = transition_factory_state(
                    client,
                    server,
                    &args,
                    work_item_id,
                    control_token,
                    work_item_version,
                    "blocked",
                    Some(format!("Mission launch or recovery failed: {error}")),
                    &stable_prefix,
                )
                .await;
            }
            return Err(error);
        }
    };
    let mission_status = launch.get("mission_status").and_then(Value::as_str);
    if matches!(mission_status, Some("failed" | "cancelled")) {
        let mission_terminal_state = mission_status.expect("matched terminal mission state");
        let terminal_state = if mission_terminal_state == "failed"
            && launch
                .get("verification_failed")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        {
            "verification_failed"
        } else {
            mission_terminal_state
        };
        let failure_detail =
            matches!(terminal_state, "failed" | "verification_failed").then(|| {
                launch
                    .get("run_summary")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("Factory mission {mission_id} failed"))
            });
        if work_item_state != terminal_state {
            let transitioned = transition_factory_state(
                client,
                server,
                &args,
                work_item_id,
                control_token,
                work_item_version,
                terminal_state,
                failure_detail,
                &stable_prefix,
            )
            .await?;
            work_item_state = transitioned.0;
        }
        bail!(
            "factory mission {mission_id} ended as {mission_terminal_state}; factory work item is {}",
            work_item_state
        );
    }
    let mission_completed = mission_status == Some("completed");
    if matches!(work_item_state.as_str(), "mission_created" | "blocked") {
        let transitioned = transition_factory_state(
            client,
            server,
            &args,
            work_item_id,
            control_token,
            work_item_version,
            "running",
            None,
            &stable_prefix,
        )
        .await?;
        work_item_version = transitioned.1;
        work_item_state = transitioned.0;
    }
    if mission_completed && work_item_state == "running" {
        let transitioned = transition_factory_state(
            client,
            server,
            &args,
            work_item_id,
            control_token,
            work_item_version,
            "verified",
            None,
            &stable_prefix,
        )
        .await?;
        work_item_version = transitioned.1;
        work_item_state = transitioned.0;
    }
    Ok(json!({
        "mode": "executed",
        "source_of_truth": "github_project",
        "project_owner": args.owner,
        "project_number": args.project_number,
        "project_item_id": refreshed.project_item.id,
        "project_status": "In Progress",
        "repository": args.repository,
        "issue_number": refreshed.issue.number,
        "issue_url": refreshed.issue.url,
        "source_revision": refreshed.issue.updated_at,
        "factory_work_item_id": work_item_id,
        "factory_state": work_item_state,
        "factory_version": work_item_version,
        "mission_id": mission_id,
        "materialized_now": materialized,
        "launch": launch,
        "auto_merge": false,
    }))
}

fn validate_args(args: &FactoryArgs) -> Result<()> {
    repository_parts(&args.repository)?;
    validate_source_base_ref(&args.source_base_ref)?;
    if args.owner.trim().is_empty() || args.owner.chars().any(char::is_whitespace) {
        bail!("GitHub Project owner is invalid");
    }
    if args.project_number == 0 {
        bail!("GitHub Project number must be positive");
    }
    if args.adapter.trim().is_empty() || args.strategy.trim().is_empty() {
        bail!("factory adapter and strategy are required");
    }
    factory_adapter_allowlist(args)?;
    if args.budget_tokens <= 0 || args.budget_cost_microusd <= 0 {
        bail!("factory token and cost budgets must be positive");
    }
    if !(30..=3_600).contains(&args.lease_seconds) {
        bail!("factory lease must be between 30 and 3600 seconds");
    }
    if args.write_scope.is_empty()
        || args
            .write_scope
            .iter()
            .any(|scope| scope.trim().is_empty() || scope.len() > 500)
    {
        bail!("factory write scope is invalid");
    }
    Ok(())
}

fn normalize_args(args: &mut FactoryArgs) -> Result<()> {
    args.owner = normalize_github_component(&args.owner, "GitHub Project owner")?;
    let repository = args.repository.trim();
    let (repository_owner, repository_name) = repository_parts(repository)?;
    args.repository = format!(
        "{}/{}",
        normalize_github_component(repository_owner, "repository owner")?,
        normalize_github_component(repository_name, "repository name")?
    );
    args.source_base_ref = args.source_base_ref.trim().to_owned();
    args.adapter = args.adapter.trim().to_owned();
    args.strategy = args.strategy.trim().to_owned();
    args.allowed_adapters = args
        .allowed_adapters
        .iter()
        .map(|adapter| adapter.trim().to_owned())
        .collect();
    args.write_scope = args
        .write_scope
        .iter()
        .map(|scope| scope.trim().to_owned())
        .collect();
    Ok(())
}

fn normalize_github_component(value: &str, field: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 100
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        bail!("{field} is invalid");
    }
    Ok(value.to_ascii_lowercase())
}

fn validate_source_base_ref(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 240
        || value.starts_with('-')
        || value.starts_with('/')
        || value.ends_with('/')
        || value.ends_with('.')
        || value.contains("..")
        || value.contains("@{")
        || value
            .chars()
            .any(|character| matches!(character, '\\' | ' ' | '~' | '^' | ':' | '?' | '*' | '['))
    {
        bail!("factory source base ref is invalid");
    }
    Ok(())
}

fn factory_adapter_allowlist(args: &FactoryArgs) -> Result<Vec<String>> {
    let mut adapters = if args.allowed_adapters.is_empty() {
        vec![args.adapter.trim().to_owned()]
    } else {
        args.allowed_adapters
            .iter()
            .map(|adapter| adapter.trim().to_owned())
            .collect::<Vec<_>>()
    };
    if adapters
        .iter()
        .any(|adapter| adapter.is_empty() || adapter.len() > 128)
    {
        bail!("factory adapter allowlist contains an invalid adapter");
    }
    adapters.sort();
    adapters.dedup();
    if !adapters
        .iter()
        .any(|adapter| adapter == args.adapter.trim())
    {
        bail!("factory adapter allowlist must include the preferred adapter");
    }
    Ok(adapters)
}

fn repository_parts(repository: &str) -> Result<(&str, &str)> {
    let mut parts = repository.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    if owner.is_empty()
        || name.is_empty()
        || parts.next().is_some()
        || owner.chars().any(char::is_whitespace)
        || name.chars().any(char::is_whitespace)
    {
        bail!("repository must use the owner/name form");
    }
    Ok((owner, name))
}

fn load_project_items(
    github_cli: &Path,
    owner: &str,
    project_number: u32,
) -> Result<ProjectItemsEnvelope> {
    gh_json(
        github_cli,
        &[
            "project",
            "item-list",
            &project_number.to_string(),
            "--owner",
            owner,
            "--limit",
            "1000",
            "--format",
            "json",
        ],
    )
    .and_then(|value| serde_json::from_value(value).context("decode GitHub Project items"))
}

fn load_issue(github_cli: &Path, repository: &str, number: i64) -> Result<IssueView> {
    gh_json(
        github_cli,
        &[
            "issue",
            "view",
            &number.to_string(),
            "--repo",
            repository,
            "--json",
            "id,state,createdAt,updatedAt,title,body,url,number,labels",
        ],
    )
    .and_then(|value| serde_json::from_value(value).context("decode GitHub issue"))
}

fn evaluate_items(
    args: &FactoryArgs,
    items: Vec<ProjectItem>,
    existing: &HashMap<String, ExistingFactoryItem>,
    issue_cache: &mut HashMap<i64, IssueView>,
) -> Result<Vec<EvaluatedItem>> {
    let mut evaluated = Vec::new();
    let now = Utc::now();
    for item in items {
        if item.content.kind != "Issue"
            || !item
                .content
                .repository
                .eq_ignore_ascii_case(&args.repository)
            || (args.issue.is_none() && !matches!(item.status.as_str(), "Todo" | "In Progress"))
            || args
                .issue
                .is_some_and(|number| number != item.content.number)
        {
            continue;
        }
        let issue = cached_issue(
            &args.github_cli,
            &args.repository,
            item.content.number,
            issue_cache,
        )?;
        let dependencies = blocked_dependency_numbers(&issue.body);
        let mut reasons = Vec::new();
        let mut recovery = false;
        if issue.state != "OPEN" {
            reasons.push("issue is not open".to_owned());
        }
        if issue.title != item.content.title
            || issue.url != item.content.url
            || issue.body != item.content.body
        {
            reasons.push("Project item content is stale relative to the issue".to_owned());
        }
        if let Some(factory_item) = existing.get(&item.id) {
            let recoverable_state = factory_state_is_recoverable(&factory_item.state);
            if factory_item_recoverable_by(factory_item, args.actor_id, &issue.updated_at, now) {
                recovery = true;
            } else {
                let detail = if recoverable_state
                    && factory_item.source_revision == issue.updated_at
                    && factory_item.claim_owner_id != args.actor_id
                {
                    format!(
                        "claimed by another controller until {}",
                        factory_item.lease_expires_at.to_rfc3339()
                    )
                } else {
                    format!(
                        "already linked to factory state {}{}",
                        factory_item.state,
                        factory_item
                            .mission_id
                            .map(|mission_id| format!(" for mission {mission_id}"))
                            .unwrap_or_default()
                    )
                };
                reasons.push(detail);
            }
        }
        if recovery {
            if !matches!(item.status.as_str(), "Todo" | "In Progress") {
                reasons.push(format!(
                    "recoverable factory item has incompatible Project status {}",
                    item.status
                ));
            }
        } else {
            if item.status != "Todo" {
                reasons.push(format!("Project status is {}, not Todo", item.status));
            }
            if !issue
                .labels
                .iter()
                .any(|label| label.name == "factory:ready")
            {
                reasons.push("missing factory:ready label".to_owned());
            }
        }
        for dependency in &dependencies {
            let dependency_issue =
                cached_issue(&args.github_cli, &args.repository, *dependency, issue_cache)?;
            if dependency_issue.state == "OPEN" {
                reasons.push(format!("blocked by open issue #{dependency}"));
            }
        }
        evaluated.push(EvaluatedItem {
            project_item: item,
            issue,
            dependencies,
            reasons,
            recovery,
        });
    }
    if let Some(number) = args.issue
        && !evaluated.iter().any(|item| item.issue.number == number)
    {
        bail!(
            "issue #{number} is not an issue item in Project #{} for {}",
            args.project_number,
            args.repository
        );
    }
    Ok(evaluated)
}

fn factory_state_is_recoverable(state: &str) -> bool {
    matches!(
        state,
        "claimed"
            | "mission_created"
            | "running"
            | "blocked"
            | "awaiting_approval"
            | "verification_failed"
    )
}

fn factory_item_recoverable_by(
    item: &ExistingFactoryItem,
    actor_id: Uuid,
    source_revision: &str,
    now: DateTime<Utc>,
) -> bool {
    item.source_revision == source_revision
        && factory_state_is_recoverable(&item.state)
        && (item.claim_owner_id == actor_id || item.lease_expires_at <= now)
}

fn refresh_selected(
    args: &FactoryArgs,
    existing: &HashMap<String, ExistingFactoryItem>,
    selected: &EvaluatedItem,
) -> Result<EvaluatedItem> {
    let project = load_project_items(&args.github_cli, &args.owner, args.project_number)?;
    if project.items.len() < project.total_count {
        bail!(
            "GitHub Project returned {} of {} items while refreshing the selected issue",
            project.items.len(),
            project.total_count
        );
    }
    let item = project
        .items
        .into_iter()
        .find(|item| item.id == selected.project_item.id)
        .context("selected GitHub Project item disappeared before claim")?;
    let mut cache = HashMap::new();
    let refreshed = evaluate_items(args, vec![item], existing, &mut cache)?
        .into_iter()
        .next()
        .context("selected GitHub Project item is no longer in the repository allowlist")?;
    if !refreshed.eligible() {
        bail!(
            "selected issue became ineligible before claim: {}",
            refreshed.reasons.join("; ")
        );
    }
    Ok(refreshed)
}

fn revalidate_selected_for_effect(
    args: &FactoryArgs,
    selected: &EvaluatedItem,
    expected_project_status: &str,
    stage: &str,
) -> Result<()> {
    let project = load_project_items(&args.github_cli, &args.owner, args.project_number)?;
    if project.items.len() < project.total_count {
        bail!(
            "GitHub Project returned {} of {} items while revalidating before {stage}",
            project.items.len(),
            project.total_count
        );
    }
    let item = project
        .items
        .into_iter()
        .find(|item| item.id == selected.project_item.id)
        .with_context(|| format!("selected GitHub Project item disappeared before {stage}"))?;
    let mut reasons = Vec::new();
    if item.content.kind != "Issue" {
        reasons.push(format!(
            "Project item type is {}, not Issue",
            item.content.kind
        ));
    }
    if !item
        .content
        .repository
        .eq_ignore_ascii_case(&args.repository)
    {
        reasons.push(format!(
            "Project item repository is {}, not {}",
            item.content.repository, args.repository
        ));
    }
    if item.content.number != selected.issue.number {
        reasons.push(format!(
            "Project item issue number changed from {} to {}",
            selected.issue.number, item.content.number
        ));
    }
    if item.status != expected_project_status {
        reasons.push(format!(
            "Project status is {}, expected {expected_project_status}",
            item.status
        ));
    }

    let issue = load_issue(&args.github_cli, &args.repository, selected.issue.number)?;
    if issue.id != selected.issue.id {
        reasons.push("GitHub issue identity changed".to_owned());
    }
    if issue.updated_at != selected.issue.updated_at {
        reasons.push(format!(
            "source revision changed from {} to {}",
            selected.issue.updated_at, issue.updated_at
        ));
    }
    if issue.state != "OPEN" {
        reasons.push("issue is not open".to_owned());
    }
    if issue.title != selected.issue.title
        || issue.body != selected.issue.body
        || issue.url != selected.issue.url
    {
        reasons.push("GitHub issue content changed after claim".to_owned());
    }
    if issue.title != item.content.title
        || issue.url != item.content.url
        || issue.body != item.content.body
    {
        reasons.push("Project item content is stale relative to the issue".to_owned());
    }
    if !issue
        .labels
        .iter()
        .any(|label| label.name == "factory:ready")
    {
        reasons.push("missing factory:ready label".to_owned());
    }

    let dependencies = blocked_dependency_numbers(&issue.body);
    let mut issue_cache = HashMap::new();
    issue_cache.insert(issue.number, issue);
    for dependency in dependencies {
        let dependency_issue = cached_issue(
            &args.github_cli,
            &args.repository,
            dependency,
            &mut issue_cache,
        )?;
        if dependency_issue.state == "OPEN" {
            reasons.push(format!("blocked by open issue #{dependency}"));
        }
    }
    if !reasons.is_empty() {
        bail!(
            "selected issue became ineligible before {stage}: {}",
            reasons.join("; ")
        );
    }
    Ok(())
}

fn cached_issue(
    github_cli: &Path,
    repository: &str,
    number: i64,
    cache: &mut HashMap<i64, IssueView>,
) -> Result<IssueView> {
    if let Some(issue) = cache.get(&number) {
        return Ok(issue.clone());
    }
    let issue = load_issue(github_cli, repository, number)?;
    cache.insert(number, issue.clone());
    Ok(issue)
}

fn existing_factory_items(snapshot: &Value) -> Result<HashMap<String, ExistingFactoryItem>> {
    snapshot
        .pointer("/snapshot/factory_work_items")
        .and_then(Value::as_array)
        .context("ECorp snapshot omitted factory_work_items")?
        .iter()
        .map(|item| {
            let project_item_id = value_string(item, "/source_project_item_id")?;
            Ok((
                project_item_id,
                ExistingFactoryItem {
                    source_revision: value_string(item, "/source_revision")?,
                    claim_owner_id: value_uuid(item, "/claim_owner_id")?,
                    state: value_string(item, "/state")?,
                    mission_id: value_optional_uuid(item, "/mission_id")?,
                    policy: item
                        .get("policy")
                        .cloned()
                        .context("factory work item omitted its policy snapshot")?,
                    lease_expires_at: value_datetime(item, "/lease_expires_at")?,
                },
            ))
        })
        .collect()
}

fn set_project_in_progress(
    github_cli: &Path,
    owner: &str,
    project_number: u32,
    item_id: &str,
) -> Result<()> {
    let project: ProjectView = serde_json::from_value(gh_json(
        github_cli,
        &[
            "project",
            "view",
            &project_number.to_string(),
            "--owner",
            owner,
            "--format",
            "json",
        ],
    )?)
    .context("decode GitHub Project")?;
    let fields: ProjectFields = serde_json::from_value(gh_json(
        github_cli,
        &[
            "project",
            "field-list",
            &project_number.to_string(),
            "--owner",
            owner,
            "--format",
            "json",
        ],
    )?)
    .context("decode GitHub Project fields")?;
    let status = fields
        .fields
        .iter()
        .find(|field| field.name == "Status")
        .context("GitHub Project has no Status field")?;
    let in_progress = status
        .options
        .iter()
        .find(|option| option.name == "In Progress")
        .context("GitHub Project Status has no In Progress option")?;
    gh_run(
        github_cli,
        &[
            "project",
            "item-edit",
            "--id",
            item_id,
            "--project-id",
            &project.id,
            "--field-id",
            &status.id,
            "--single-select-option-id",
            &in_progress.id,
        ],
    )
}

fn verify_project_status(
    github_cli: &Path,
    owner: &str,
    project_number: u32,
    item_id: &str,
    expected: &str,
) -> Result<()> {
    let items = load_project_items(github_cli, owner, project_number)?;
    let status = items
        .items
        .iter()
        .find(|item| item.id == item_id)
        .map(|item| item.status.as_str())
        .context("updated GitHub Project item disappeared")?;
    if status != expected {
        bail!("GitHub Project item status is {status}, not {expected}");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn renew_factory_control(
    client: &Client,
    server: &str,
    args: &FactoryArgs,
    work_item_id: Uuid,
    claim_token: Uuid,
    expected_version: i64,
    lease_seconds: i64,
    stage: &str,
    stable_prefix: &str,
) -> Result<(String, i64)> {
    let response = server_json(
        client,
        Method::POST,
        format!(
            "{server}/api/corps/{}/factory/work-items/{work_item_id}/renew",
            args.corp_id
        ),
        Some(json!({
            "actor_id": args.actor_id,
            "claim_token": claim_token,
            "expected_version": expected_version,
            "idempotency_key": format!(
                "{stable_prefix}:renew:{stage}:{expected_version}"
            ),
            "lease_seconds": lease_seconds,
        })),
    )
    .await?;
    let returned_token = value_uuid(&response, "/claim_token")
        .context("factory renewal did not return its fencing token")?;
    if returned_token != claim_token {
        bail!("factory renewal rotated the controller token unexpectedly");
    }
    Ok((
        value_string(&response, "/work_item/state")?,
        value_i64(&response, "/work_item/version")?,
    ))
}

#[allow(clippy::too_many_arguments)]
async fn transition_factory_state(
    client: &Client,
    server: &str,
    args: &FactoryArgs,
    work_item_id: Uuid,
    claim_token: Uuid,
    expected_version: i64,
    state: &str,
    failure_detail: Option<String>,
    stable_prefix: &str,
) -> Result<(String, i64)> {
    let failure_detail = failure_detail.map(|detail| sanitize_failure_detail(&detail));
    let response = server_json(
        client,
        Method::POST,
        format!(
            "{server}/api/corps/{}/factory/work-items/{work_item_id}/transition",
            args.corp_id
        ),
        Some(json!({
            "actor_id": args.actor_id,
            "claim_token": claim_token,
            "expected_version": expected_version,
            "idempotency_key": format!(
                "{stable_prefix}:transition:{state}:{expected_version}"
            ),
            "state": state,
            "failure_detail": failure_detail,
        })),
    )
    .await?;
    Ok((
        value_string(&response, "/work_item/state")?,
        value_i64(&response, "/work_item/version")?,
    ))
}

async fn launch_or_recover(
    client: &Client,
    server: &str,
    args: &FactoryArgs,
    mission_id: Uuid,
) -> Result<Value> {
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
    let (mission_status, task_ids, verification_failed) =
        factory_mission_snapshot(&snapshot, mission_id)?;
    let existing_runs = snapshot
        .pointer("/snapshot/runs")
        .and_then(Value::as_array)
        .context("ECorp snapshot omitted runs")?
        .iter()
        .filter(|run| {
            run.get("task_id")
                .and_then(Value::as_str)
                .is_some_and(|task_id| task_ids.contains(task_id))
        })
        .collect::<Vec<_>>();
    if !existing_runs.is_empty() {
        return Ok(json!({
            "recovered": true,
            "mission_status": mission_status,
            "verification_failed": verification_failed,
            "run_summary": existing_runs
                .first()
                .and_then(|run| run.get("summary"))
                .and_then(Value::as_str),
            "run_ids": existing_runs
                .iter()
                .filter_map(|run| run.get("id").and_then(Value::as_str))
                .collect::<Vec<_>>(),
        }));
    }
    if mission_status != "ready" {
        bail!("factory mission is {mission_status} without a recoverable run");
    }
    let url = format!(
        "{server}/api/corps/{}/missions/{mission_id}/launch",
        args.corp_id
    );
    let (status, body) = server_json_raw(
        client,
        Method::POST,
        url,
        Some(json!({"requested_by": args.actor_id})),
    )
    .await?;
    if status.is_success() {
        return Ok(json!({
            "recovered": false,
            "run_id": value_uuid(&body, "/run_id")?,
            "run_ids": body.get("run_ids").cloned().unwrap_or_else(|| json!([])),
        }));
    }
    if status == StatusCode::CONFLICT {
        let refreshed = server_json(
            client,
            Method::GET,
            format!(
                "{server}/api/corps/{}/snapshot?actor_id={}",
                args.corp_id, args.actor_id
            ),
            None,
        )
        .await?;
        let (mission_status, task_ids, verification_failed) =
            factory_mission_snapshot(&refreshed, mission_id)?;
        let existing_runs = refreshed
            .pointer("/snapshot/runs")
            .and_then(Value::as_array)
            .context("ECorp snapshot omitted runs after launch conflict")?
            .iter()
            .filter(|run| {
                run.get("task_id")
                    .and_then(Value::as_str)
                    .is_some_and(|task_id| task_ids.contains(task_id))
            })
            .collect::<Vec<_>>();
        if !existing_runs.is_empty() {
            return Ok(json!({
                "recovered": true,
                "mission_status": mission_status,
                "verification_failed": verification_failed,
                "run_summary": existing_runs
                    .first()
                    .and_then(|run| run.get("summary"))
                    .and_then(Value::as_str),
                "run_ids": existing_runs
                    .iter()
                    .filter_map(|run| run.get("id").and_then(Value::as_str))
                    .collect::<Vec<_>>(),
            }));
        }
    }
    bail!("factory mission launch failed with {status}: {body}")
}

fn factory_mission_snapshot(
    snapshot: &Value,
    mission_id: Uuid,
) -> Result<(String, HashSet<String>, bool)> {
    let mission_id = mission_id.to_string();
    let mission_status = snapshot
        .pointer("/snapshot/missions")
        .and_then(Value::as_array)
        .and_then(|missions| {
            missions.iter().find(|mission| {
                mission.get("id").and_then(Value::as_str) == Some(mission_id.as_str())
            })
        })
        .and_then(|mission| mission.get("status"))
        .and_then(Value::as_str)
        .context("factory mission is absent from the authorized snapshot")?
        .to_owned();
    let tasks = snapshot
        .pointer("/snapshot/tasks")
        .and_then(Value::as_array)
        .context("ECorp snapshot omitted tasks")?;
    let mission_tasks = tasks
        .iter()
        .filter(|task| task.get("mission_id").and_then(Value::as_str) == Some(mission_id.as_str()))
        .collect::<Vec<_>>();
    let task_ids = mission_tasks
        .iter()
        .filter_map(|task| task.get("id").and_then(Value::as_str))
        .map(str::to_owned)
        .collect::<HashSet<_>>();
    let verification_failed = mission_tasks.iter().any(|task| {
        task.get("verification_status").and_then(Value::as_str) == Some("failed")
            || task.get("status").and_then(Value::as_str) == Some("verification_failed")
    });
    Ok((mission_status, task_ids, verification_failed))
}

fn blocked_dependency_numbers(body: &str) -> Vec<i64> {
    let mut in_dependencies = false;
    let mut dependencies = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some((level, heading)) = markdown_heading(trimmed) {
            in_dependencies = level == 2 && heading.trim().eq_ignore_ascii_case("Dependencies");
            continue;
        }
        if !in_dependencies {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if let Some(offset) = lower.find("blocked by") {
            let clause = &trimmed[offset..];
            let clause_lower = clause.to_ascii_lowercase();
            let end = [
                clause.find('.'),
                clause.find(';'),
                clause_lower.find(" aligned with"),
                clause_lower.find(" must respect"),
                clause_lower.find(" contributes"),
            ]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(clause.len());
            dependencies.extend(issue_numbers(&clause[..end]));
        } else if trimmed.starts_with("- [ ]")
            && trimmed.contains('#')
            && (lower.contains("blocked") || lower.contains("must"))
        {
            dependencies.extend(issue_numbers(trimmed));
        }
    }
    dependencies.sort_unstable();
    dependencies.dedup();
    dependencies
}

fn acceptance_tests(body: &str) -> Vec<String> {
    let mut in_acceptance = false;
    let mut tests = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some((level, heading)) = markdown_heading(trimmed) {
            in_acceptance =
                level == 2 && heading.trim().eq_ignore_ascii_case("Acceptance criteria");
            continue;
        }
        if !in_acceptance {
            continue;
        }
        let value = trimmed
            .strip_prefix("- [ ] ")
            .or_else(|| trimmed.strip_prefix("- [x] "))
            .or_else(|| trimmed.strip_prefix("- [X] "));
        if let Some(value) = value {
            tests.push(truncate_utf8(value.trim(), 500));
        }
    }
    if tests.is_empty() {
        tests.push("the linked GitHub issue acceptance criteria are satisfied".to_owned());
    }
    tests
}

fn issue_numbers(line: &str) -> Vec<i64> {
    let bytes = line.as_bytes();
    let mut numbers = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'#' {
            index += 1;
            continue;
        }
        index += 1;
        let start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if start < index
            && let Ok(number) = line[start..index].parse()
        {
            numbers.push(number);
        }
    }
    numbers
}

fn markdown_heading(line: &str) -> Option<(usize, &str)> {
    let level = line.bytes().take_while(|byte| *byte == b'#').count();
    if level == 0 || line.as_bytes().get(level) != Some(&b' ') {
        return None;
    }
    Some((level, &line[level + 1..]))
}

fn bounded_title(number: i64, title: &str) -> String {
    truncate_utf8(&format!("GitHub #{number}: {}", title.trim()), 240)
}

fn issue_objective(issue: &IssueView) -> String {
    truncate_utf8(
        &format!(
            "Implement the authoritative GitHub issue below inside the assigned worktree.\n\n\
             ISSUE: #{} — {}\n\
             URL: {}\n\
             SOURCE REVISION: {}\n\n{}",
            issue.number,
            issue.title.trim(),
            issue.url,
            issue.updated_at,
            issue.body.trim()
        ),
        100_000,
    )
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

fn sanitize_failure_detail(value: &str) -> String {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let collapsed = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    let collapsed = if collapsed.is_empty() {
        "factory operation failed"
    } else {
        collapsed.as_str()
    };
    truncate_utf8(collapsed, 2_000)
}

fn gh_json(github_cli: &Path, args: &[&str]) -> Result<Value> {
    let output = gh_output(github_cli, args)?;
    serde_json::from_slice(&output).with_context(|| {
        format!(
            "decode JSON from {} {}",
            github_cli.display(),
            args.join(" ")
        )
    })
}

fn gh_run(github_cli: &Path, args: &[&str]) -> Result<()> {
    gh_output(github_cli, args).map(|_| ())
}

fn gh_output(github_cli: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let prefix_args = env::var("ECORP_GITHUB_CLI_PREFIX_ARGS_JSON")
        .ok()
        .map(|value| {
            serde_json::from_str::<Vec<String>>(&value)
                .context("decode ECORP_GITHUB_CLI_PREFIX_ARGS_JSON")
        })
        .transpose()?
        .unwrap_or_default();
    let output = Command::new(github_cli)
        .args(prefix_args)
        .args(args)
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_PAGER", "cat")
        .output()
        .with_context(|| format!("run {} {}", github_cli.display(), args.join(" ")))?;
    if !output.status.success() {
        let detail = sanitize_failure_detail(&String::from_utf8_lossy(&output.stderr));
        bail!(
            "{} {} failed: {}",
            github_cli.display(),
            args.join(" "),
            detail
        );
    }
    Ok(output.stdout)
}

async fn server_json(
    client: &Client,
    method: Method,
    url: String,
    body: Option<Value>,
) -> Result<Value> {
    let (status, body) = server_json_raw(client, method, url, body).await?;
    if !status.is_success() {
        bail!("ECorp API returned {status}: {body}");
    }
    Ok(body)
}

async fn server_json_raw(
    client: &Client,
    method: Method,
    url: String,
    body: Option<Value>,
) -> Result<(StatusCode, Value)> {
    let mut request = client.request(method, &url);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .with_context(|| format!("request {url}"))?;
    let status = response.status();
    let text = response.text().await?;
    let body = serde_json::from_str(&text)
        .with_context(|| format!("decode response from {url}: {text}"))?;
    Ok((status, body))
}

fn value_string(value: &Value, pointer: &str) -> Result<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .with_context(|| format!("response omitted string {pointer}"))
}

fn value_i64(value: &Value, pointer: &str) -> Result<i64> {
    value
        .pointer(pointer)
        .and_then(Value::as_i64)
        .with_context(|| format!("response omitted integer {pointer}"))
}

fn value_datetime(value: &Value, pointer: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value_string(value, pointer)?)
        .map(|value| value.with_timezone(&Utc))
        .with_context(|| format!("response contained invalid timestamp {pointer}"))
}

fn value_uuid(value: &Value, pointer: &str) -> Result<Uuid> {
    Uuid::parse_str(&value_string(value, pointer)?)
        .with_context(|| format!("response contained invalid UUID {pointer}"))
}

fn value_optional_uuid(value: &Value, pointer: &str) -> Result<Option<Uuid>> {
    match value.pointer(pointer) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            Ok(Some(Uuid::parse_str(value).with_context(|| {
                format!("response contained invalid UUID {pointer}")
            })?))
        }
        Some(_) => Err(anyhow!(
            "response contained invalid optional UUID {pointer}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use serde_json::json;
    use uuid::Uuid;

    use super::{
        ExistingFactoryItem, acceptance_tests, blocked_dependency_numbers,
        factory_item_recoverable_by, issue_numbers, normalize_github_component,
        sanitize_failure_detail, truncate_utf8,
    };

    #[test]
    fn dependency_parser_uses_only_explicit_blockers() {
        let body = r#"
## Dependencies

Blocked by #59 and #53.
Aligned with #48 and #52.
- [ ] #61 must land.
- [ ] Aligned with #62.

### Notes

Blocked by #77.

## Notes

Blocked by #999 outside the section.
"#;
        assert_eq!(blocked_dependency_numbers(body), vec![53, 59, 61]);
        assert_eq!(issue_numbers("Refs #1, #22 and text"), vec![1, 22]);
    }

    #[test]
    fn acceptance_parser_preserves_issue_checklist_order() {
        let body = r#"
## Acceptance criteria

- [ ] first outcome
- [x] already proven

### Notes

- [ ] not an acceptance item

## Boundaries

- [ ] not an acceptance item
"#;
        assert_eq!(
            acceptance_tests(body),
            vec!["first outcome".to_owned(), "already proven".to_owned()]
        );
    }

    #[test]
    fn utf8_truncation_never_splits_a_character() {
        assert_eq!(truncate_utf8("ab🙂cd", 5), "ab");
        assert_eq!(truncate_utf8("ab🙂cd", 6), "ab🙂");
    }

    #[test]
    fn github_identity_and_failure_details_are_canonical_and_single_line() {
        assert_eq!(
            normalize_github_component(" ShYaM.Repo ", "repository").unwrap(),
            "shyam.repo"
        );
        assert_eq!(
            sanitize_failure_detail("GitHub failed:\r\nAuthorization: redacted\trequest"),
            "GitHub failed: Authorization: redacted request"
        );
    }

    #[test]
    fn verified_items_leave_the_queue_and_expired_claims_allow_failover() {
        let now = Utc::now();
        let owner = Uuid::new_v4();
        let replacement = Uuid::new_v4();
        let mut item = ExistingFactoryItem {
            source_revision: "2026-09-01T14:00:00Z".to_owned(),
            claim_owner_id: owner,
            state: "running".to_owned(),
            mission_id: Some(Uuid::new_v4()),
            policy: json!({}),
            lease_expires_at: now + Duration::minutes(5),
        };
        assert!(!factory_item_recoverable_by(
            &item,
            replacement,
            &item.source_revision,
            now,
        ));
        item.lease_expires_at = now - Duration::seconds(1);
        assert!(factory_item_recoverable_by(
            &item,
            replacement,
            &item.source_revision,
            now,
        ));
        item.state = "verified".to_owned();
        assert!(!factory_item_recoverable_by(
            &item,
            owner,
            &item.source_revision,
            now,
        ));
    }
}
