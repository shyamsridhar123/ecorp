use std::{
    collections::{HashMap, HashSet},
    env,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use clap::{Args, ValueEnum};
use crony_domain::{
    FactoryVerificationRecoveryMode, TaskContract, VerificationPolicy, write_scope_is_valid,
};
use reqwest::{Client, Method, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

mod project;
mod quota;

const DEFAULT_GITHUB_COMMAND_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_SOURCE_GIT_COMMAND_TIMEOUT_MS: u64 = 30_000;
const RECOVERY_SOURCE_REFERENCE_PREFIX: &str = "urn:ecorp:factory-reviewed-source:sha256:";

#[derive(Debug, Args, Clone)]
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

    #[arg(long, env = "ECORP_FACTORY_PUBLICATION_BASE_REF")]
    pub publication_base_ref: Option<String>,

    #[arg(
        long,
        env = "ECORP_FACTORY_SOURCE_REPOSITORY_PATH",
        default_value = "."
    )]
    pub source_repository_path: PathBuf,

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

    #[arg(long, env = "ECORP_FACTORY_VERIFICATION_POLICY_FILE")]
    pub verification_policy_file: Option<PathBuf>,

    #[arg(long)]
    pub issue: Option<i64>,

    #[arg(
        long,
        value_enum,
        requires_all = ["issue", "verification_recovery_reason"]
    )]
    pub verification_recovery: Option<VerificationRecoveryModeArg>,

    #[arg(long, requires = "verification_recovery")]
    pub verification_recovery_reason: Option<String>,

    #[arg(long)]
    pub dry_run: bool,

    #[arg(long, env = "ECORP_GITHUB_CLI", default_value = "gh")]
    pub github_cli: PathBuf,

    #[arg(skip)]
    github_budget: quota::BudgetState,

    /// Discovery hint only; source content/eligibility are always re-read.
    #[arg(skip)]
    active_issue: Arc<AtomicI64>,
}

#[derive(Debug, Args)]
pub struct FactoryWatchArgs {
    #[command(flatten)]
    pub factory: FactoryArgs,

    #[arg(
        long,
        env = "ECORP_FACTORY_CONTROLLER_ID",
        default_value = "00000000-0000-4000-8000-000000000051"
    )]
    pub controller_id: Uuid,

    #[arg(
        long,
        env = "ECORP_FACTORY_WATCH_INTERVAL_SECONDS",
        default_value_t = 30
    )]
    pub interval_seconds: u64,

    #[arg(long, env = "ECORP_FACTORY_HEARTBEAT_SECONDS", default_value_t = 10)]
    pub heartbeat_seconds: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum VerificationRecoveryModeArg {
    SourceCorrection,
    VerifierOnly,
}

impl From<VerificationRecoveryModeArg> for FactoryVerificationRecoveryMode {
    fn from(value: VerificationRecoveryModeArg) -> Self {
        match value {
            VerificationRecoveryModeArg::SourceCorrection => Self::SourceCorrection,
            VerificationRecoveryModeArg::VerifierOnly => Self::VerifierOnly,
        }
    }
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
    version: i64,
    source_project_owner: String,
    source_project_number: i64,
    source_project_item_id: String,
    source_repository_owner: String,
    source_repository_name: String,
    source_issue_number: i64,
    source_issue_node_id: String,
    source_issue_url: String,
    source_title: String,
    source_revision: String,
    claim_owner_id: Uuid,
    state: String,
    mission_id: Option<Uuid>,
    policy: Value,
    lease_expires_at: DateTime<Utc>,
}

struct ResolvedSourceCommit {
    commit: String,
    legacy_upgrade_required: bool,
}

struct FactoryRecoverySnapshot {
    task_id: Uuid,
    source_run_id: Uuid,
    contract_version: i64,
    contract: TaskContract,
    verification_policy: VerificationPolicy,
    workspace_fingerprint: Option<String>,
    expected_head_commit: Option<String>,
    contract_revisions: Vec<Value>,
    active_recovery: Option<Value>,
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
    args.github_budget.check(Utc::now())?;
    if args
        .github_budget
        .snapshot()
        .graphql
        .as_ref()
        .is_none_or(|quota| {
            quota.reset_at <= Utc::now()
                || quota.observed_at + chrono::Duration::seconds(60) <= Utc::now()
        })
    {
        graphql_json(
            &args,
            "query { rateLimit { limit remaining cost resetAt } }",
            json!({}),
        )?;
        if args
            .github_budget
            .snapshot()
            .graphql
            .as_ref()
            .is_none_or(|quota| quota.reset_at <= Utc::now())
        {
            bail!("GitHub did not provide a current GraphQL quota observation");
        }
        args.github_budget.check(Utc::now())?;
    }
    let adapter_allowlist = factory_adapter_allowlist(&args)?;
    let requested_verification_policy = load_verification_policy(&args)?;
    let project_items = load_requested_project_items(&args)?;
    if project_items.items.len() < project_items.total_count {
        bail!(
            "GitHub Project returned {} of {} items; increase the controller query limit",
            project_items.items.len(),
            project_items.total_count
        );
    }
    let candidate_ids = project_items
        .items
        .iter()
        .filter(|item| project_item_is_candidate(&args, item))
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    let existing = lookup_factory_work_items(client, server, &args, &candidate_ids).await?;
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
    if let Some(index) = selected_index {
        args.active_issue
            .store(evaluated[index].issue.number, Ordering::Relaxed);
    }
    let selected_source_base_commit = selected_index
        .map(|index| {
            if evaluated[index].recovery {
                resolve_recovery_source_base_commit(&args, &evaluated[index], &existing)
            } else {
                resolve_source_base_commit(&args).map(|commit| ResolvedSourceCommit {
                    commit,
                    legacy_upgrade_required: false,
                })
            }
        })
        .transpose()?;
    let preview_publication_base_ref = selected_index
        .map(|index| {
            if evaluated[index].recovery {
                resolve_recovery_publication_base_ref(
                    &args,
                    existing.get(&evaluated[index].project_item.id).context(
                        "recoverable factory item disappeared from the selected-item lookup",
                    )?,
                )
            } else {
                Ok(selected_publication_base_ref(&args).to_owned())
            }
        })
        .transpose()?
        .unwrap_or_else(|| selected_publication_base_ref(&args).to_owned());
    let preview_verification_policy = selected_index
        .map(|index| {
            if evaluated[index].recovery {
                let item = existing.get(&evaluated[index].project_item.id).context(
                    "recoverable factory item disappeared from the selected-item lookup",
                )?;
                if args.verification_recovery.is_some() {
                    resolve_explicit_recovery_verification_policy(
                        item,
                        requested_verification_policy.as_ref(),
                    )
                } else {
                    resolve_recovery_verification_policy(
                        item,
                        requested_verification_policy.as_ref(),
                    )
                }
            } else {
                Ok(requested_verification_policy.clone())
            }
        })
        .transpose()?
        .flatten()
        .or(requested_verification_policy.clone());
    let evaluated_json = evaluated
        .iter()
        .map(EvaluatedItem::as_json)
        .collect::<Vec<_>>();

    if args.dry_run {
        let preflight = if let Some(index) = selected_index {
            let selected = &evaluated[index];
            let persisted = existing.get(&selected.project_item.id);
            if persisted.and_then(|item| item.mission_id).is_some() {
                None
            } else {
                let resolved = selected_source_base_commit
                    .as_ref()
                    .context("selected factory issue has no resolved source commit")?;
                let policy = if selected.recovery {
                    preflight_policy(
                        &persisted
                            .context(
                                "recoverable factory item disappeared from the selected-item lookup",
                            )?
                            .policy,
                        &resolved.commit,
                    )?
                } else {
                    new_factory_policy(
                        &args,
                        selected,
                        &resolved.commit,
                        &preview_publication_base_ref,
                        preview_verification_policy.as_ref(),
                        &adapter_allowlist,
                    )
                };
                let mission_body = factory_mission_body(
                    &args,
                    &selected.issue,
                    preview_verification_policy.as_ref(),
                );
                Some(preflight_factory_mission(client, server, &args, policy, mission_body).await?)
            }
        } else {
            None
        };
        return Ok(json!({
            "mode": "dry_run",
            "source_of_truth": "github_project",
            "project_owner": args.owner,
            "project_number": args.project_number,
            "repository": args.repository,
            "source_base_ref": args.source_base_ref,
            "publication_base_ref": preview_publication_base_ref,
            "verification_policy": preview_verification_policy,
            "source_base_commit": selected_source_base_commit
                .as_ref()
                .map(|resolved| resolved.commit.as_str()),
            "legacy_source_upgrade_required": selected_source_base_commit
                .as_ref()
                .is_some_and(|resolved| resolved.legacy_upgrade_required),
            "preflight": preflight,
            "selected": selected_index.map(|index| evaluated[index].as_json()),
            "evaluated": evaluated_json,
            "github_polling": args.github_budget.snapshot(),
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
    let (refreshed, persisted) = refresh_selected(client, server, &args, &selected).await?;
    let source_resolution = if refreshed.recovery {
        resolve_recovery_source_base_commit_from_item(
            &args,
            persisted
                .as_ref()
                .context("recoverable factory item disappeared from the selected-item lookup")?,
        )?
    } else {
        ResolvedSourceCommit {
            commit: resolve_source_base_commit(&args)?,
            legacy_upgrade_required: false,
        }
    };
    let ResolvedSourceCommit {
        commit: source_base_commit,
        legacy_upgrade_required,
    } = source_resolution;
    let publication_base_ref = if refreshed.recovery {
        resolve_recovery_publication_base_ref(
            &args,
            persisted
                .as_ref()
                .context("recoverable factory item disappeared from the selected-item lookup")?,
        )?
    } else {
        selected_publication_base_ref(&args).to_owned()
    };
    let verification_policy = if refreshed.recovery {
        let item = persisted
            .as_ref()
            .context("recoverable factory item disappeared from the selected-item lookup")?;
        if args.verification_recovery.is_some() {
            resolve_explicit_recovery_verification_policy(
                item,
                requested_verification_policy.as_ref(),
            )?
        } else {
            resolve_recovery_verification_policy(item, requested_verification_policy.as_ref())?
        }
    } else {
        requested_verification_policy.clone()
    };
    let stable_prefix = format!(
        "github-project:{}:{}:{}:{}",
        args.owner, args.project_number, refreshed.project_item.id, refreshed.issue.updated_at
    );
    let claim_path = format!(
        "{server}/api/corps/{}/factory/work-items/claim",
        args.corp_id
    );
    let policy = if refreshed.recovery {
        persisted
            .as_ref()
            .map(|item| item.policy.clone())
            .context("recoverable factory item disappeared from the selected-item lookup")?
    } else {
        new_factory_policy(
            &args,
            &refreshed,
            &source_base_commit,
            &publication_base_ref,
            verification_policy.as_ref(),
            &adapter_allowlist,
        )
    };
    let mission_body = factory_mission_body(&args, &refreshed.issue, verification_policy.as_ref());
    let preflight = if persisted
        .as_ref()
        .and_then(|item| item.mission_id)
        .is_none()
    {
        let preflight_policy = if legacy_upgrade_required {
            preflight_policy(&policy, &source_base_commit)?
        } else {
            policy.clone()
        };
        Some(
            preflight_factory_mission(
                client,
                server,
                &args,
                preflight_policy,
                mission_body.clone(),
            )
            .await?,
        )
    } else {
        None
    };
    let claim_generation = persisted.as_ref().map(|item| item.version).unwrap_or(0);
    let persisted_source = refreshed.recovery.then_some(persisted.as_ref()).flatten();
    let (repository_owner, repository_name) = repository_parts(&args.repository)?;
    let claim_body = json!({
        "actor_id": args.actor_id,
        "source_project_owner": persisted_source
            .map(|item| item.source_project_owner.as_str())
            .unwrap_or(args.owner.as_str()),
        "source_project_number": persisted_source
            .map(|item| item.source_project_number)
            .unwrap_or(i64::from(args.project_number)),
        "source_project_item_id": persisted_source
            .map(|item| item.source_project_item_id.as_str())
            .unwrap_or(refreshed.project_item.id.as_str()),
        "source_repository_owner": persisted_source
            .map(|item| item.source_repository_owner.as_str())
            .unwrap_or(repository_owner),
        "source_repository_name": persisted_source
            .map(|item| item.source_repository_name.as_str())
            .unwrap_or(repository_name),
        "source_issue_number": persisted_source
            .map(|item| item.source_issue_number)
            .unwrap_or(refreshed.issue.number),
        "source_issue_node_id": persisted_source
            .map(|item| item.source_issue_node_id.as_str())
            .unwrap_or(refreshed.issue.id.as_str()),
        "source_issue_url": persisted_source
            .map(|item| item.source_issue_url.as_str())
            .unwrap_or(refreshed.issue.url.as_str()),
        "source_title": persisted_source
            .map(|item| item.source_title.as_str())
            .unwrap_or(refreshed.issue.title.as_str()),
        "source_revision": persisted_source
            .map(|item| item.source_revision.as_str())
            .unwrap_or(refreshed.issue.updated_at.as_str()),
        "source_base_ref": args.source_base_ref,
        "source_base_commit": source_base_commit,
        "publication_base_ref": publication_base_ref,
        "idempotency_key": format!(
            "{stable_prefix}:claim:{}:{claim_generation}:lease:{}",
            args.actor_id, args.lease_seconds
        ),
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
            "{stable_prefix}:reclaim:{}:{replay_version}:lease:{}",
            args.actor_id, args.lease_seconds
        ));
        claim = server_json(client, Method::POST, claim_path, Some(reclaim_body)).await?;
    }

    let work_item_id = value_uuid(&claim, "/work_item/id")?;
    let mut mission_id = value_optional_uuid(&claim, "/work_item/mission_id")?;
    let control_token = value_uuid(&claim, "/claim_token")
        .context("factory work item has no usable controller fencing token")?;
    let mut work_item_version = value_i64(&claim, "/work_item/version")?;
    let mut source_commit_upgraded = false;
    if legacy_upgrade_required {
        let upgrade = server_json(
            client,
            Method::POST,
            format!(
                "{server}/api/corps/{}/factory/work-items/{work_item_id}/upgrade-source-commit",
                args.corp_id
            ),
            Some(json!({
                "actor_id": args.actor_id,
                "claim_token": control_token,
                "expected_version": work_item_version,
                "idempotency_key": format!("{stable_prefix}:upgrade-source-commit"),
                "source_base_commit": source_base_commit,
            })),
        )
        .await?;
        work_item_version = value_i64(&upgrade, "/work_item/version")?;
        mission_id = value_optional_uuid(&upgrade, "/work_item/mission_id")?;
        source_commit_upgraded = !upgrade
            .get("replayed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    }
    let mut materialized = false;
    if mission_id.is_none() {
        let mut materialize_body = mission_body;
        let materialize_fields = materialize_body
            .as_object_mut()
            .context("factory mission request must be a JSON object")?;
        materialize_fields.insert(
            "claim_token".to_owned(),
            Value::String(control_token.to_string()),
        );
        materialize_fields.insert(
            "expected_version".to_owned(),
            Value::Number(work_item_version.into()),
        );
        materialize_fields.insert(
            "idempotency_key".to_owned(),
            Value::String(format!("{stable_prefix}:materialize")),
        );
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
    let renewed = renew_factory_control(
        client,
        server,
        &args,
        work_item_id,
        control_token,
        work_item_version,
        effect_lease_seconds,
        "project-status-ready",
        &stable_prefix,
    )
    .await
    .context("renew factory lease immediately before GitHub Project update")?;
    work_item_version = renewed.1;

    let project_update = if refreshed.project_item.status == "In Progress" {
        // The exact item was just revalidated. No external write is needed.
        Ok(())
    } else {
        set_project_in_progress(
            &args.github_cli,
            &args.owner,
            args.project_number,
            &refreshed.project_item.id,
            &args.github_budget,
        )
        .and_then(|_| verify_project_status(&args, &refreshed.project_item.id, "In Progress"))
    };
    if let Err(error) = project_update {
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
                    "GitHub Project status synchronization failed: {error}"
                )),
                &stable_prefix,
            )
            .await
            .context("persist blocked state after GitHub Project status failure")?;
        }
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
    let renewed = renew_factory_control(
        client,
        server,
        &args,
        work_item_id,
        control_token,
        work_item_version,
        effect_lease_seconds,
        "mission-launch-ready",
        &stable_prefix,
    )
    .await
    .context("renew factory lease immediately before mission launch")?;
    work_item_state = renewed.0;
    work_item_version = renewed.1;

    let launch_result = if args.verification_recovery.is_some() {
        recover_factory_verification(
            client,
            server,
            &args,
            &refreshed,
            persisted
                .as_ref()
                .context("verification recovery lost its persisted factory item")?,
            work_item_id,
            control_token,
            work_item_version,
            mission_id,
            &stable_prefix,
            verification_policy.as_ref(),
        )
        .await
    } else {
        launch_or_recover(client, server, &args, mission_id).await
    };
    let launch = match launch_result {
        Ok(launch) => launch,
        Err(error) => {
            if work_item_state != "blocked" {
                transition_factory_state(
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
                .await
                .context("persist blocked state after mission launch or recovery failure")?;
            }
            return Err(error);
        }
    };
    let mission_status = launch.get("mission_status").and_then(Value::as_str);
    if let Some(recovery_state) = launch.get("factory_state").and_then(Value::as_str) {
        work_item_state = recovery_state.to_owned();
    }
    if let Some(recovery_version) = launch.get("factory_version").and_then(Value::as_i64) {
        work_item_version = recovery_version;
    }
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
    let mission_awaiting_approval = launch
        .get("awaiting_approval")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let workspace_cleanup_pending = launch
        .get("workspace_cleanup_pending")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if mission_awaiting_approval && work_item_state != "awaiting_approval" {
        let transitioned = transition_factory_state(
            client,
            server,
            &args,
            work_item_id,
            control_token,
            work_item_version,
            "awaiting_approval",
            None,
            &stable_prefix,
        )
        .await?;
        work_item_version = transitioned.1;
        work_item_state = transitioned.0;
    }
    let mission_completed = mission_status == Some("completed");
    if !mission_awaiting_approval
        && !mission_completed
        && matches!(
            work_item_state.as_str(),
            "mission_created" | "blocked" | "awaiting_approval"
        )
    {
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
    if factory_completion_is_ready(
        mission_completed,
        workspace_cleanup_pending,
        &work_item_state,
    ) {
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
        "github_polling": args.github_budget.snapshot(),
        "source_of_truth": "github_project",
        "project_owner": args.owner,
        "project_number": args.project_number,
        "project_item_id": refreshed.project_item.id,
        "project_status": "In Progress",
        "repository": args.repository,
        "issue_number": refreshed.issue.number,
        "issue_url": refreshed.issue.url,
        "source_revision": refreshed.issue.updated_at,
        "source_base_ref": args.source_base_ref,
        "source_base_commit": source_base_commit,
        "legacy_source_commit_upgraded": source_commit_upgraded,
        "factory_work_item_id": work_item_id,
        "factory_state": work_item_state,
        "factory_version": work_item_version,
        "mission_id": mission_id,
        "materialized_now": materialized,
        "preflight": preflight,
        "launch": launch,
        "auto_merge": false,
    }))
}

pub async fn watch(client: &Client, server: &str, mut args: FactoryWatchArgs) -> Result<Value> {
    normalize_args(&mut args.factory)?;
    validate_args(&args.factory)?;
    if args.factory.dry_run {
        bail!("factory watch cannot run in dry-run mode");
    }
    if !(5..=300).contains(&args.heartbeat_seconds) {
        bail!("factory heartbeat interval must be between 5 and 300 seconds");
    }
    if !(5..=3600).contains(&args.interval_seconds) {
        bail!("factory watch interval must be between 5 and 3600 seconds");
    }
    let (repository_owner, repository_name) = repository_parts(&args.factory.repository)?;
    let connection_epoch = Uuid::new_v4();
    let controller_url = format!(
        "{server}/api/corps/{}/factory/controllers",
        args.factory.corp_id
    );
    let configured = server_json(
        client,
        Method::POST,
        controller_url,
        Some(json!({
            "actor_id": args.factory.actor_id,
            "controller_id": args.controller_id,
            "source_project_owner": args.factory.owner,
            "source_project_number": args.factory.project_number,
            "source_repository_owner": repository_owner,
            "source_repository_name": repository_name,
            "connection_epoch": connection_epoch,
            "lease_seconds": (args.heartbeat_seconds * 3).clamp(10, 300),
            "idempotency_key": format!(
                "factory-controller:{}:connect:{}",
                args.controller_id, connection_epoch
            )
        })),
    )
    .await?;
    let mut controller = configured["controller"].clone();
    args.factory.github_budget.restore(
        serde_json::from_value(
            controller
                .get("polling")
                .cloned()
                .unwrap_or_else(|| json!({})),
        )
        .context("decode persisted factory polling state")?,
        Utc::now(),
    );
    let mut cycle: Option<tokio::task::JoinHandle<Result<Value>>> = None;
    let mut cycle_generation: Option<i64> = None;
    let mut next_periodic = Instant::now();
    let mut last_error: Option<String> = None;
    let mut last_result: Option<&'static str> = None;
    let mut heartbeat = tokio::time::interval(Duration::from_secs(args.heartbeat_seconds));

    loop {
        heartbeat.tick().await;
        if cycle.as_ref().is_some_and(|task| task.is_finished()) {
            let result = cycle
                .take()
                .context("factory cycle disappeared")?
                .await
                .context("factory cycle task failed")?;
            match result {
                Ok(value) => {
                    last_error = None;
                    last_result = Some("succeeded");
                    args.factory.github_budget.success(Utc::now());
                    if matches!(
                        value["factory_state"].as_str(),
                        Some("verified" | "published" | "failed" | "cancelled")
                    ) {
                        args.factory.active_issue.store(0, Ordering::Relaxed);
                    }
                }
                Err(error)
                    if error
                        .to_string()
                        .starts_with("no eligible factory issue found") =>
                {
                    last_error = None;
                    last_result = Some("succeeded");
                    args.factory.github_budget.success(Utc::now());
                    args.factory.active_issue.store(0, Ordering::Relaxed);
                }
                Err(error) => {
                    // Request wrappers record throttling immediately, even if
                    // later persistence/compensation fails. Do not count the
                    // same request twice when its error reaches the watcher.
                    if args
                        .factory
                        .github_budget
                        .snapshot()
                        .next_retry_at
                        .is_none_or(|retry| retry <= Utc::now())
                    {
                        args.factory.github_budget.failure(&error, Utc::now());
                    }
                    last_error = Some(bounded_status_error(&error.to_string()));
                    last_result = Some("failed");
                }
            }
            next_periodic = Instant::now() + Duration::from_secs(args.interval_seconds);
        }

        let snapshot = match server_json(
            client,
            Method::GET,
            format!(
                "{server}/api/corps/{}/snapshot?actor_id={}",
                args.factory.corp_id, args.factory.actor_id
            ),
            None,
        )
        .await
        {
            Ok(snapshot) => snapshot,
            Err(error) => {
                eprintln!(
                    "factory watch snapshot unavailable: {}",
                    bounded_status_error(&error.to_string())
                );
                continue;
            }
        };
        if let Some(current) = snapshot["snapshot"]["factory_controllers"]
            .as_array()
            .and_then(|controllers| {
                controllers
                    .iter()
                    .find(|candidate| candidate["id"] == args.controller_id.to_string())
            })
        {
            controller = current.clone();
        }
        let active_work_item_id = snapshot["snapshot"]["factory_work_items"]
            .as_array()
            .and_then(|items| {
                items.iter().find(|item| {
                    item["source_project_owner"] == args.factory.owner
                        && item["source_project_number"] == i64::from(args.factory.project_number)
                        && item["source_repository_owner"] == repository_owner
                        && item["source_repository_name"] == repository_name
                        && !matches!(
                            item["state"].as_str(),
                            Some("verified" | "published" | "failed" | "cancelled")
                        )
                })
            })
            .and_then(|item| item["id"].as_str());
        let pending_generation = controller["reconcile_generation"].as_i64().unwrap_or(0);
        let completed_generation = controller["completed_reconcile_generation"]
            .as_i64()
            .unwrap_or(0);
        let finished_generation = (cycle.is_none() && last_result.is_some())
            .then_some(cycle_generation.unwrap_or(completed_generation));
        let heartbeat_result = server_json(
            client,
            Method::POST,
            format!(
                "{server}/api/corps/{}/factory/controllers/{}/heartbeat",
                args.factory.corp_id, args.controller_id
            ),
            Some(json!({
                "actor_id": args.factory.actor_id,
                "connection_epoch": connection_epoch,
                "lease_seconds": (args.heartbeat_seconds * 3).clamp(10, 300),
                "active_work_item_id": active_work_item_id,
                "completed_reconcile_generation": finished_generation,
                "reconcile_result": finished_generation.and(last_result),
                "error": last_error,
                "polling": args.factory.github_budget.snapshot(),
            })),
        )
        .await;
        if let Ok(response) = heartbeat_result {
            controller = response["controller"].clone();
            if finished_generation.is_some() {
                cycle_generation = None;
                last_result = None;
            }
        } else if let Err(error) = heartbeat_result {
            eprintln!(
                "factory watch heartbeat failed: {}",
                bounded_status_error(&error.to_string())
            );
            continue;
        }

        let paused = controller["desired_state"].as_str() == Some("paused");
        let forced = pending_generation > completed_generation;
        if cycle.is_none()
            && !paused
            && args.factory.github_budget.check(Utc::now()).is_ok()
            && (forced || Instant::now() >= next_periodic)
        {
            cycle_generation = forced.then_some(pending_generation);
            let cycle_client = client.clone();
            let cycle_server = server.to_owned();
            let mut cycle_args = args.factory.clone();
            let selected = cycle_args.active_issue.load(Ordering::Relaxed);
            if cycle_args.issue.is_none() && selected > 0 {
                // Cache identity only while a selected lineage is active. Each
                // cycle still reads live source/Project state before effects.
                cycle_args.issue = Some(selected);
            }
            cycle = Some(tokio::spawn(async move {
                run(&cycle_client, &cycle_server, cycle_args).await
            }));
        }
    }
}

fn bounded_status_error(value: &str) -> String {
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    value.chars().take(1_000).collect()
}

fn new_factory_policy(
    args: &FactoryArgs,
    selected: &EvaluatedItem,
    source_base_commit: &str,
    publication_base_ref: &str,
    verification_policy: Option<&VerificationPolicy>,
    adapter_allowlist: &[String],
) -> Value {
    json!({
        "schema_version": 1,
        "source_of_truth": "github_project",
        "project_owner": args.owner,
        "project_number": args.project_number,
        "project_status": selected.project_item.status,
        "required_label": "factory:ready",
        "dependencies": selected.dependencies,
        "repository_allowlist": [args.repository],
        "source_base_ref": args.source_base_ref,
        "source_base_commit": source_base_commit,
        "source_commit_upgrade_required": false,
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
        "deliverable_form": "commit_branch",
        "budget_tokens": args.budget_tokens,
        "budget_cost_microusd": args.budget_cost_microusd,
        "auto_merge": false,
        "publication": {
            "allowed": true,
            "repository_allowlist": [args.repository],
            "base_ref": publication_base_ref,
            "branch_prefix": "ecorp/",
            "status_before": "In Progress",
            "review_status": "In Review",
            "auto_merge": false,
            "merge": false,
            "deploy": false
        },
        "verification_policy": verification_policy,
    })
}

fn preflight_policy(policy: &Value, source_base_commit: &str) -> Result<Value> {
    let mut policy = policy.clone();
    let policy = policy
        .as_object_mut()
        .context("recoverable factory policy must be a JSON object")?;
    policy.insert(
        "source_base_commit".to_owned(),
        Value::String(source_base_commit.to_ascii_lowercase()),
    );
    policy.insert(
        "source_commit_upgrade_required".to_owned(),
        Value::Bool(false),
    );
    Ok(Value::Object(policy.clone()))
}

fn factory_mission_body(
    args: &FactoryArgs,
    issue: &IssueView,
    verification_policy: Option<&VerificationPolicy>,
) -> Value {
    json!({
        "actor_id": args.actor_id,
        "title": bounded_title(issue.number, &issue.title),
        "description": issue.body,
        "preferred_adapter": args.adapter,
        "preferred_model": args.model,
        "reasoning_effort": args.reasoning_effort,
        "strategy": args.strategy,
        "budget_tokens": args.budget_tokens,
        "budget_cost_microusd": args.budget_cost_microusd,
        "deliverable": {
            "form": "commit_branch",
            "commit_after_verification": true,
            "paths": [],
        },
        "contract": {
            "objective": issue_objective(issue),
            "expected_output": format!(
                "A complete, verified repository implementation for GitHub issue #{}.",
                issue.number
            ),
            "acceptance_tests": acceptance_tests(&issue.body),
            "allowed_tools": ["filesystem", "shell"],
            "prohibited_actions": [
                "modify files outside the assigned worktree",
                "use undeclared long-lived credentials",
                "merge or deploy without a separate current authorization"
            ],
            "references": [
                issue.url,
                format!(
                    "https://github.com/users/{}/projects/{}",
                    args.owner, args.project_number
                )
            ],
            "write_scope": args.write_scope,
        },
        "verification_policy": verification_policy,
    })
}

fn factory_preflight_body(
    args: &FactoryArgs,
    policy: Value,
    mut mission_body: Value,
) -> Result<Value> {
    let (source_repository_owner, source_repository_name) = repository_parts(&args.repository)?;
    let fields = mission_body
        .as_object_mut()
        .context("factory mission request must be a JSON object")?;
    fields.insert(
        "source_repository_owner".to_owned(),
        Value::String(source_repository_owner.to_owned()),
    );
    fields.insert(
        "source_repository_name".to_owned(),
        Value::String(source_repository_name.to_owned()),
    );
    fields.insert("policy".to_owned(), policy);
    Ok(mission_body)
}

async fn preflight_factory_mission(
    client: &Client,
    server: &str,
    args: &FactoryArgs,
    policy: Value,
    mission_body: Value,
) -> Result<Value> {
    server_json(
        client,
        Method::POST,
        format!("{server}/api/corps/{}/factory/preflight", args.corp_id),
        Some(factory_preflight_body(args, policy, mission_body)?),
    )
    .await
    .context("factory plan preflight rejected before claim")
}

fn validate_args(args: &FactoryArgs) -> Result<()> {
    repository_parts(&args.repository)?;
    validate_source_base_ref(&args.source_base_ref)?;
    validate_publication_base_ref(args)?;
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
            .any(|scope| !write_scope_is_valid(scope))
    {
        bail!("factory write scope is invalid");
    }
    if args
        .verification_policy_file
        .as_ref()
        .is_some_and(|path| !path.is_file())
    {
        bail!("factory verification policy file does not exist");
    }
    match (
        args.verification_recovery,
        args.verification_recovery_reason.as_deref(),
    ) {
        (None, None) => {}
        (Some(_), Some(reason)) => {
            if args.issue.is_none() {
                bail!("factory verification recovery requires an explicit --issue");
            }
            if reason.trim().is_empty()
                || reason.len() > 4_000
                || reason.chars().any(char::is_control)
            {
                bail!(
                    "factory verification recovery reason must contain 1 to 4,000 printable bytes"
                );
            }
        }
        _ => bail!("factory verification recovery mode and reason must be supplied together"),
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
    args.publication_base_ref = args
        .publication_base_ref
        .take()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
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
    args.verification_recovery_reason = args
        .verification_recovery_reason
        .take()
        .map(|reason| reason.trim().to_owned());
    Ok(())
}

fn load_verification_policy(args: &FactoryArgs) -> Result<Option<VerificationPolicy>> {
    let Some(path) = args.verification_policy_file.as_ref() else {
        return Ok(None);
    };
    let bytes = std::fs::read(path).context("read factory verification policy file")?;
    if bytes.is_empty() || bytes.len() > 100_000 {
        bail!("factory verification policy file is empty or too large");
    }
    serde_json::from_slice(&bytes)
        .context("decode factory verification policy file")
        .map(Some)
}

fn resolve_recovery_verification_policy(
    item: &ExistingFactoryItem,
    requested: Option<&VerificationPolicy>,
) -> Result<Option<VerificationPolicy>> {
    let persisted_value = item.policy.get("verification_policy");
    let persisted = persisted_value
        .filter(|value| !value.is_null())
        .map(|value| {
            serde_json::from_value::<VerificationPolicy>(value.clone())
                .context("decode persisted factory verification policy")
        })
        .transpose()?;
    match (persisted, requested) {
        (Some(persisted), Some(requested)) if &persisted != requested => {
            bail!("factory recovery verification policy does not match the persisted policy")
        }
        (Some(persisted), _) => Ok(Some(persisted)),
        (None, Some(_)) => bail!(
            "factory recovery cannot add a verification policy that was not persisted at claim"
        ),
        (None, None) => Ok(None),
    }
}

fn resolve_explicit_recovery_verification_policy(
    item: &ExistingFactoryItem,
    requested: Option<&VerificationPolicy>,
) -> Result<Option<VerificationPolicy>> {
    if let Some(requested) = requested {
        return Ok(Some(requested.clone()));
    }
    item.policy
        .get("verification_policy")
        .filter(|value| !value.is_null())
        .map(|value| {
            serde_json::from_value::<VerificationPolicy>(value.clone())
                .context("decode persisted factory verification policy")
        })
        .transpose()
}

pub(crate) fn normalize_github_component(value: &str, field: &str) -> Result<String> {
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
        || value.chars().any(char::is_control)
        || value
            .chars()
            .any(|character| matches!(character, '\\' | ' ' | '~' | '^' | ':' | '?' | '*' | '['))
    {
        bail!("factory source base ref is invalid");
    }
    Ok(())
}

fn validate_publication_base_ref(args: &FactoryArgs) -> Result<()> {
    let publication_base_ref = selected_publication_base_ref(args);
    let Some(branch) = publication_base_branch(publication_base_ref)? else {
        return Ok(());
    };
    source_git_output(
        &args.source_repository_path,
        &["check-ref-format", "--branch", branch],
    )
    .map(|_| ())
    .with_context(|| {
        format!(
            "factory publication base ref {} is not a valid Git branch",
            publication_base_ref
        )
    })
}

fn selected_publication_base_ref(args: &FactoryArgs) -> &str {
    args.publication_base_ref
        .as_deref()
        .unwrap_or(&args.source_base_ref)
}

fn resolve_recovery_publication_base_ref(
    args: &FactoryArgs,
    item: &ExistingFactoryItem,
) -> Result<String> {
    let publication = item
        .policy
        .get("publication")
        .and_then(Value::as_object)
        .context("persisted factory policy has no publication object")?;
    let persisted = publication
        .get("base_ref")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .context("persisted factory publication policy has no base_ref")?;
    publication_base_branch(persisted)?;
    if let Some(requested) = args.publication_base_ref.as_deref()
        && requested != persisted
    {
        bail!(
            "factory recovery publication base ref mismatch: persisted policy requires {persisted}, controller requested {requested}"
        );
    }
    Ok(persisted.to_owned())
}

fn publication_base_branch(value: &str) -> Result<Option<&str>> {
    validate_source_base_ref(value)?;
    if value == "HEAD" {
        return Ok(None);
    }
    if let Some(branch) = value.strip_prefix("refs/heads/") {
        if branch.is_empty() {
            bail!("factory publication base ref must be HEAD or a branch ref");
        }
        return Ok(Some(branch));
    }
    if value.starts_with("refs/") {
        bail!("factory publication base ref must be HEAD or a branch ref");
    }
    Ok(Some(value))
}

fn resolve_recovery_source_base_commit(
    args: &FactoryArgs,
    selected: &EvaluatedItem,
    existing: &HashMap<String, ExistingFactoryItem>,
) -> Result<ResolvedSourceCommit> {
    let item = existing
        .get(&selected.project_item.id)
        .context("recoverable factory item disappeared from the selected-item lookup")?;
    resolve_recovery_source_base_commit_from_item(args, item)
}

fn resolve_recovery_source_base_commit_from_item(
    args: &FactoryArgs,
    item: &ExistingFactoryItem,
) -> Result<ResolvedSourceCommit> {
    let policy = item
        .policy
        .as_object()
        .context("persisted factory policy is not a JSON object")?;
    let source_base_ref = policy
        .get("source_base_ref")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .context("persisted factory policy has no source_base_ref")?;
    validate_source_base_ref(source_base_ref)?;
    if source_base_ref != args.source_base_ref {
        bail!(
            "factory recovery source base ref mismatch: persisted policy requires {source_base_ref}, controller requested {}",
            args.source_base_ref
        );
    }
    if let Some(commit) = policy.get("source_base_commit").and_then(Value::as_str) {
        let commit = commit.to_ascii_lowercase();
        validate_source_base_commit(&commit)?;
        return Ok(ResolvedSourceCommit {
            commit,
            legacy_upgrade_required: false,
        });
    }
    if policy
        .get("source_commit_upgrade_required")
        .and_then(Value::as_bool)
        != Some(true)
    {
        bail!("persisted factory policy has no immutable source_base_commit");
    }
    Ok(ResolvedSourceCommit {
        commit: resolve_source_base_commit_at_ref(args, source_base_ref)?,
        legacy_upgrade_required: true,
    })
}

fn resolve_source_base_commit(args: &FactoryArgs) -> Result<String> {
    resolve_source_base_commit_at_ref(args, &args.source_base_ref)
}

fn resolve_source_base_commit_at_ref(args: &FactoryArgs, source_base_ref: &str) -> Result<String> {
    let remote = source_git_output(
        &args.source_repository_path,
        &["config", "--get", "remote.origin.url"],
    )
    .context("resolve factory source repository origin")?;
    let remote =
        String::from_utf8(remote).context("factory source repository origin is not UTF-8")?;
    let identity = parse_github_repository_identity(&remote)
        .context("factory source repository origin is not a supported GitHub remote")?;
    if !identity.eq_ignore_ascii_case(&args.repository) {
        bail!(
            "factory source repository identity mismatch: --repository is {}, local origin is {identity}",
            args.repository
        );
    }
    let revision = format!("{source_base_ref}^{{commit}}");
    let commit = source_git_output(
        &args.source_repository_path,
        &["rev-parse", "--verify", &revision],
    )
    .with_context(|| {
        format!(
            "resolve factory source base ref {} in {}",
            source_base_ref,
            args.source_repository_path.display()
        )
    })?;
    let commit = String::from_utf8(commit)
        .context("resolved factory source commit is not UTF-8")?
        .trim()
        .to_ascii_lowercase();
    validate_source_base_commit(&commit)?;
    Ok(commit)
}

fn validate_source_base_commit(value: &str) -> Result<()> {
    if !matches!(value.len(), 40 | 64) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!(
            "factory source base commit must be a full 40- or 64-character hexadecimal Git object id"
        );
    }
    Ok(())
}

fn parse_github_repository_identity(remote: &str) -> Option<String> {
    let remote = remote.trim();
    let path = if let Some(path) = remote.strip_prefix("git@github.com:") {
        path.to_owned()
    } else {
        let url = Url::parse(remote).ok()?;
        if !url
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case("github.com"))
        {
            return None;
        }
        url.path().trim_start_matches('/').to_owned()
    };
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    let mut parts = path.split('/');
    let owner = parts.next()?;
    let repository = parts.next()?;
    if parts.next().is_some()
        || owner.is_empty()
        || repository.is_empty()
        || !owner.chars().chain(repository.chars()).all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        return None;
    }
    Some(format!("{owner}/{repository}"))
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

// Avoid full-Project rescans for a selected issue or an already-claimed item.
fn load_requested_project_items(args: &FactoryArgs) -> Result<ProjectItemsEnvelope> {
    let Some(number) = args.issue else {
        return project::discover(&args.owner, args.project_number, |query, variables| {
            graphql_json(args, query, variables)
        });
    };
    let (owner, repository) = repository_parts(&args.repository)?;
    let query = r#"query($owner:String!,$repo:String!,$number:Int!){
      repository(owner:$owner,name:$repo){issue(number:$number){
        projectItems(first:100){pageInfo{hasNextPage} nodes{
          id project{number owner{... on User{login} ... on Organization{login}}}
        }}
      }} rateLimit{limit remaining cost resetAt}
    }"#;
    let result = graphql_json(
        args,
        query,
        json!({"owner":owner,"repo":repository,"number":number}),
    )?;
    let connection = result
        .pointer("/data/repository/issue/projectItems")
        .context("selected GitHub issue or Project memberships are unavailable")?;
    if connection
        .pointer("/pageInfo/hasNextPage")
        .and_then(Value::as_bool)
        != Some(false)
    {
        bail!("selected issue Project memberships exceeded the bounded exact lookup");
    }
    let nodes = connection["nodes"]
        .as_array()
        .context("missing issue Project memberships")?;
    let matches = nodes
        .iter()
        .filter(|node| project_identity_matches(node, &args.owner, args.project_number))
        .collect::<Vec<_>>();
    if matches.len() > 1 {
        bail!("selected issue has ambiguous membership in the requested Project");
    }
    let items = matches
        .into_iter()
        .map(|node| {
            let id = node["id"]
                .as_str()
                .context("Project membership omitted its identity")?;
            load_project_item_exact(args, id)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(ProjectItemsEnvelope {
        total_count: items.len(),
        items,
    })
}

fn project_identity_matches(node: &Value, owner: &str, number: u32) -> bool {
    node.pointer("/project/number").and_then(Value::as_u64) == Some(u64::from(number))
        && node
            .pointer("/project/owner/login")
            .and_then(Value::as_str)
            .is_some_and(|login| login.eq_ignore_ascii_case(owner))
}

fn load_project_item_exact(args: &FactoryArgs, item_id: &str) -> Result<ProjectItem> {
    let query = r#"query($id:ID!){node(id:$id){
      __typename ... on ProjectV2Item{
        id isArchived project{number owner{... on User{login} ... on Organization{login}}}
        fieldValueByName(name:"Status"){... on ProjectV2ItemFieldSingleSelectValue{name}}
        content{__typename ... on Issue{id number title body url repository{nameWithOwner}}}
      }
    } rateLimit{limit remaining cost resetAt} }"#;
    let result = graphql_json(args, query, json!({"id":item_id}))?;
    parse_exact_project_item(
        &result["data"]["node"],
        &args.owner,
        args.project_number,
        item_id,
    )
}

fn parse_exact_project_item(
    node: &Value,
    owner: &str,
    number: u32,
    item_id: &str,
) -> Result<ProjectItem> {
    if node["__typename"] != "ProjectV2Item"
        || node["isArchived"] != false
        || node["id"] != item_id
        || !project_identity_matches(node, owner, number)
        || node["content"]["__typename"] != "Issue"
    {
        bail!("selected GitHub Project item disappeared or changed Project/source identity");
    }
    let content = &node["content"];
    Ok(ProjectItem {
        id: item_id.to_owned(),
        status: node
            .pointer("/fieldValueByName/name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        content: ProjectContent {
            kind: "Issue".to_owned(),
            number: content["number"]
                .as_i64()
                .context("Project issue number missing")?,
            title: content["title"]
                .as_str()
                .context("Project issue title missing")?
                .to_owned(),
            body: content["body"]
                .as_str()
                .context("Project issue body missing")?
                .to_owned(),
            url: content["url"]
                .as_str()
                .context("Project issue URL missing")?
                .to_owned(),
            repository: content
                .pointer("/repository/nameWithOwner")
                .and_then(Value::as_str)
                .context("Project issue repository missing")?
                .to_ascii_lowercase(),
        },
    })
}

fn load_issue(
    github_cli: &Path,
    repository: &str,
    number: i64,
    budget: &quota::BudgetState,
) -> Result<IssueView> {
    gh_json_guarded(
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
        budget,
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
        if !project_item_is_candidate(args, &item) {
            continue;
        }
        let issue = cached_issue(
            &args.github_cli,
            &args.repository,
            item.content.number,
            issue_cache,
            &args.github_budget,
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
            let explicit_verified_replay = args.verification_recovery.is_none()
                && args.issue == Some(issue.number)
                && factory_item.state == "verified"
                && factory_item.source_revision == issue.updated_at
                && (factory_item.claim_owner_id == args.actor_id
                    || factory_item.lease_expires_at <= now);
            let explicit_verification_recovery = args.verification_recovery.is_some()
                && matches!(
                    factory_item.state.as_str(),
                    "verification_failed" | "running" | "blocked" | "awaiting_approval"
                )
                && (factory_item.claim_owner_id == args.actor_id
                    || factory_item.lease_expires_at <= now);
            if factory_item.state == "verification_failed" && args.verification_recovery.is_none() {
                reasons.push(
                    "verification-failed work requires explicit --verification-recovery and --verification-recovery-reason"
                        .to_owned(),
                );
            } else if args.verification_recovery.is_some()
                && !matches!(
                    factory_item.state.as_str(),
                    "verification_failed" | "running" | "blocked" | "awaiting_approval"
                )
            {
                reasons.push(format!(
                    "verification recovery is incompatible with factory state {}",
                    factory_item.state
                ));
            } else if explicit_verified_replay
                || explicit_verification_recovery
                || factory_item_recoverable_by(factory_item, args.actor_id, &issue.updated_at, now)
            {
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
        if args.verification_recovery.is_some() {
            match existing.get(&item.id) {
                Some(factory_item) if factory_item.mission_id.is_some() => {
                    if !factory_recovery_source_matches(args, &item, &issue, factory_item) {
                        reasons.push(
                            "verification recovery must retain the original Project item and issue identity"
                                .to_owned(),
                        );
                    }
                }
                _ => reasons.push(
                    "verification recovery requires an existing factory item and mission; replacement work is not authorized"
                        .to_owned(),
                ),
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
            let dependency_issue = cached_issue(
                &args.github_cli,
                &args.repository,
                *dependency,
                issue_cache,
                &args.github_budget,
            )?;
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

fn project_item_is_candidate(args: &FactoryArgs, item: &ProjectItem) -> bool {
    item.content.kind == "Issue"
        && item
            .content
            .repository
            .eq_ignore_ascii_case(&args.repository)
        && (args.issue.is_some() || matches!(item.status.as_str(), "Todo" | "In Progress"))
        && args
            .issue
            .is_none_or(|number| number == item.content.number)
}

fn factory_recovery_source_matches(
    args: &FactoryArgs,
    item: &ProjectItem,
    issue: &IssueView,
    persisted: &ExistingFactoryItem,
) -> bool {
    persisted
        .source_project_owner
        .eq_ignore_ascii_case(&args.owner)
        && persisted.source_project_number == i64::from(args.project_number)
        && persisted.source_project_item_id == item.id
        && format!(
            "{}/{}",
            persisted.source_repository_owner, persisted.source_repository_name
        )
        .eq_ignore_ascii_case(&args.repository)
        && persisted.source_issue_number == issue.number
        && persisted.source_issue_node_id == issue.id
        && persisted.source_issue_url == issue.url
}

fn factory_completion_is_ready(
    mission_completed: bool,
    workspace_cleanup_pending: bool,
    state: &str,
) -> bool {
    mission_completed
        && !workspace_cleanup_pending
        && matches!(
            state,
            "mission_created" | "running" | "awaiting_approval" | "blocked"
        )
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

async fn refresh_selected(
    client: &Client,
    server: &str,
    args: &FactoryArgs,
    selected: &EvaluatedItem,
) -> Result<(EvaluatedItem, Option<ExistingFactoryItem>)> {
    let item = load_project_item_exact(args, &selected.project_item.id)?;
    let existing =
        lookup_factory_work_items(client, server, args, std::slice::from_ref(&item.id)).await?;
    let mut cache = HashMap::new();
    let refreshed = evaluate_items(args, vec![item], &existing, &mut cache)?
        .into_iter()
        .next()
        .context("selected GitHub Project item is no longer in the repository allowlist")?;
    if !refreshed.eligible() {
        bail!(
            "selected issue became ineligible before claim: {}",
            refreshed.reasons.join("; ")
        );
    }
    let persisted = existing.get(&refreshed.project_item.id).cloned();
    Ok((refreshed, persisted))
}

fn revalidate_selected_for_effect(
    args: &FactoryArgs,
    selected: &EvaluatedItem,
    expected_project_status: &str,
    stage: &str,
) -> Result<()> {
    let item = load_project_item_exact(args, &selected.project_item.id)
        .with_context(|| format!("read exact Project item before {stage}"))?;
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

    let issue = load_issue(
        &args.github_cli,
        &args.repository,
        selected.issue.number,
        &args.github_budget,
    )?;
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
    if args.verification_recovery.is_none()
        && !issue
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
            &args.github_budget,
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
    budget: &quota::BudgetState,
) -> Result<IssueView> {
    if let Some(issue) = cache.get(&number) {
        return Ok(issue.clone());
    }
    let issue = load_issue(github_cli, repository, number, budget)?;
    cache.insert(number, issue.clone());
    Ok(issue)
}

async fn lookup_factory_work_items(
    client: &Client,
    server: &str,
    args: &FactoryArgs,
    source_project_item_ids: &[String],
) -> Result<HashMap<String, ExistingFactoryItem>> {
    if source_project_item_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut combined = HashMap::new();
    for chunk in source_project_item_ids.chunks(1000) {
        let response = server_json(
            client,
            Method::POST,
            format!(
                "{server}/api/corps/{}/factory/work-items/lookup",
                args.corp_id
            ),
            Some(json!({
                "actor_id": args.actor_id,
                "source_project_owner": args.owner,
                "source_project_number": args.project_number,
                "source_project_item_ids": chunk,
            })),
        )
        .await?;
        let items = response
            .get("items")
            .and_then(Value::as_array)
            .context("factory work-item lookup omitted items")?;
        let total_count = response
            .get("total_count")
            .and_then(Value::as_u64)
            .context("factory work-item lookup omitted total_count")?;
        if total_count != items.len() as u64 {
            bail!(
                "factory work-item lookup returned {} of {} items",
                items.len(),
                total_count
            );
        }
        for item in items {
            if value_string(item, "/source_project_owner")? != args.owner
                || value_i64(item, "/source_project_number")? != i64::from(args.project_number)
            {
                bail!("factory work-item lookup returned an item from another GitHub Project");
            }
            if !chunk.contains(&value_string(item, "/source_project_item_id")?) {
                bail!("factory lookup returned an unrequested Project item");
            }
        }
        for (id, item) in existing_factory_items(items)? {
            if combined.insert(id, item).is_some() {
                bail!("factory lookup returned a duplicate Project item");
            }
        }
    }
    Ok(combined)
}

fn existing_factory_items(items: &[Value]) -> Result<HashMap<String, ExistingFactoryItem>> {
    items
        .iter()
        .map(|item| {
            let project_item_id = value_string(item, "/source_project_item_id")?;
            Ok((
                project_item_id,
                ExistingFactoryItem {
                    version: value_i64(item, "/version")?,
                    source_project_owner: value_string(item, "/source_project_owner")?,
                    source_project_number: value_i64(item, "/source_project_number")?,
                    source_project_item_id: value_string(item, "/source_project_item_id")?,
                    source_repository_owner: value_string(item, "/source_repository_owner")?,
                    source_repository_name: value_string(item, "/source_repository_name")?,
                    source_issue_number: value_i64(item, "/source_issue_number")?,
                    source_issue_node_id: value_string(item, "/source_issue_node_id")?,
                    source_issue_url: value_string(item, "/source_issue_url")?,
                    source_title: value_string(item, "/source_title")?,
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
    budget: &quota::BudgetState,
) -> Result<()> {
    let project: ProjectView = serde_json::from_value(gh_json_guarded(
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
        budget,
    )?)
    .context("decode GitHub Project")?;
    let fields: ProjectFields = serde_json::from_value(gh_json_guarded(
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
        budget,
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
    gh_run_guarded(
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
        budget,
    )
}

fn verify_project_status(args: &FactoryArgs, item_id: &str, expected: &str) -> Result<()> {
    let item = load_project_item_exact(args, item_id)?;
    let status = item.status.as_str();
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

async fn load_factory_recovery_snapshot(
    client: &Client,
    server: &str,
    args: &FactoryArgs,
    mission_id: Uuid,
    work_item_id: Uuid,
    mode: FactoryVerificationRecoveryMode,
    reason: &str,
) -> Result<FactoryRecoverySnapshot> {
    let context = server_json(
        client,
        Method::GET,
        format!(
            "{server}/api/corps/{}/factory/work-items/{work_item_id}/verification-recoveries?actor_id={}",
            args.corp_id, args.actor_id
        ),
        None,
    )
    .await?;
    if value_uuid(&context, "/work_item/id")? != work_item_id
        || value_uuid(&context, "/mission_id")? != mission_id
    {
        bail!("factory recovery context does not match the selected work item");
    }
    let recoveries = context
        .get("recoveries")
        .and_then(Value::as_array)
        .context("factory recovery context omitted recoveries")?;
    let active_recoveries = recoveries
        .iter()
        .filter(|recovery| {
            recovery
                .get("status")
                .and_then(Value::as_str)
                .is_some_and(|status| matches!(status, "authorized" | "running"))
        })
        .collect::<Vec<_>>();
    if active_recoveries.len() > 1 {
        bail!("factory recovery context returned multiple active recoveries");
    }
    if let Some(active) = active_recoveries.first()
        && active.get("mode").and_then(Value::as_str) != Some(mode.as_str())
    {
        bail!(
            "factory work item already has an active {} recovery",
            active
                .get("mode")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        );
    }
    if let Some(active) = active_recoveries.first() {
        if value_uuid(active, "/authorized_by")? != args.actor_id {
            bail!("active factory recovery belongs to another authorizing actor");
        }
        if active.get("reason").and_then(Value::as_str) != Some(reason) {
            bail!("active factory recovery reason does not match this replay");
        }
    }
    let task_id = value_uuid(&context, "/task_id")?;
    let source_run_id = value_uuid(&context, "/source_run_id")?;
    if let Some(active) = active_recoveries.first()
        && (value_uuid(active, "/corp_id")? != args.corp_id
            || value_uuid(active, "/factory_work_item_id")? != work_item_id
            || value_uuid(active, "/mission_id")? != mission_id
            || value_uuid(active, "/task_id")? != task_id
            || value_uuid(active, "/source_run_id")? != source_run_id)
    {
        bail!("active factory recovery does not match the selected source lineage");
    }
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
    let tasks = snapshot
        .pointer("/snapshot/tasks")
        .and_then(Value::as_array)
        .context("factory recovery snapshot omitted tasks")?;
    let runs = snapshot
        .pointer("/snapshot/runs")
        .and_then(Value::as_array)
        .context("factory recovery snapshot omitted runs")?;
    let source_run = runs
        .iter()
        .find(|run| {
            run.get("id").and_then(Value::as_str) == Some(source_run_id.to_string().as_str())
        })
        .context("factory verification recovery source run is absent from the snapshot")?;
    if value_uuid(source_run, "/task_id")? != task_id
        || value_uuid(source_run, "/corp_id")? != args.corp_id
    {
        bail!("factory recovery context source run does not match its task");
    }
    let task = tasks
        .iter()
        .find(|task| task.get("id").and_then(Value::as_str) == Some(task_id.to_string().as_str()))
        .context("factory verification recovery task is absent from the snapshot")?;
    if value_uuid(task, "/mission_id")? != mission_id
        || value_uuid(task, "/corp_id")? != args.corp_id
    {
        bail!("factory recovery task does not match the selected mission");
    }
    let contract: TaskContract = serde_json::from_value(
        task.get("contract")
            .cloned()
            .context("factory recovery task omitted contract")?,
    )
    .context("decode factory recovery task contract")?;
    let verification_policy: VerificationPolicy = serde_json::from_value(
        task.get("verification_policy")
            .cloned()
            .context("factory recovery task omitted verification policy")?,
    )
    .context("decode factory recovery verification policy")?;
    let workspace_fingerprint = context
        .get("workspace_fingerprint")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let expected_head_commit = context
        .get("expected_head_commit")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let contract_revisions = snapshot
        .pointer("/snapshot/mission_contract_revisions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(FactoryRecoverySnapshot {
        task_id,
        source_run_id,
        contract_version: value_i64(task, "/contract_version")?,
        contract,
        verification_policy,
        workspace_fingerprint,
        expected_head_commit,
        contract_revisions,
        active_recovery: active_recoveries
            .first()
            .map(|recovery| (*recovery).clone()),
    })
}

fn reviewed_recovery_source(args: &FactoryArgs, selected: &EvaluatedItem) -> Value {
    json!({
        "repository": args.repository,
        "project_item_id": selected.project_item.id,
        "issue_number": selected.issue.number,
        "issue_node_id": selected.issue.id,
        "issue_url": selected.issue.url,
        "title": selected.issue.title,
        "source_revision": selected.issue.updated_at,
        "body_sha256": format!("{:x}", Sha256::digest(selected.issue.body.as_bytes())),
    })
}

#[allow(clippy::too_many_arguments)]
fn recovery_contract_request(
    args: &FactoryArgs,
    snapshot: &FactoryRecoverySnapshot,
    issue: &IssueView,
    mode: FactoryVerificationRecoveryMode,
    source_changed: bool,
    reason: &str,
    replacement_policy: &VerificationPolicy,
    reviewed_source_snapshot: &Value,
) -> Result<Value> {
    let mut contract = snapshot.contract.clone();
    if mode == FactoryVerificationRecoveryMode::SourceCorrection || source_changed {
        contract.objective = issue_objective(issue);
        contract.expected_output = format!(
            "A corrected, verified implementation for GitHub issue #{}.",
            issue.number
        );
        let acceptance = acceptance_tests(&issue.body);
        if !acceptance.is_empty() {
            contract.acceptance_tests = acceptance;
        }
        if !contract.references.contains(&issue.url) {
            contract.references.push(issue.url.clone());
        }
    }
    // The revision schema has no reviewed-source field. Persist an inert provenance
    // reference so source identity survives the store's objective normalization and
    // size cap. This is not a URL or new source/tool authority. Keep operator refs.
    contract
        .references
        .retain(|reference| !reference.starts_with(RECOVERY_SOURCE_REFERENCE_PREFIX));
    contract.references.push(format!(
        "{RECOVERY_SOURCE_REFERENCE_PREFIX}{:x}",
        Sha256::digest(serde_json::to_vec(reviewed_source_snapshot)?)
    ));
    if contract.references.len() > 64 {
        bail!("factory recovery contract has no room for its reviewed-source provenance reference");
    }
    let description = issue.body.replace("\r\n", "\n").replace('\r', "\n");
    Ok(json!({
        "actor_id": args.actor_id,
        "task_id": snapshot.task_id,
        "expected_contract_version": snapshot.contract_version,
        "next_action": "resume",
        "source_run_id": snapshot.source_run_id,
        "reason": reason,
        "description": description.trim(),
        "contract": contract,
        "verification_policy": replacement_policy,
    }))
}

fn recovery_replacement_contract(request: &Value) -> Result<Value> {
    const SEPARATOR: &str = "\n\nTASK-SPECIFIC OBJECTIVE:\n";
    let description = value_string(request, "/description")?;
    let mut contract = request
        .get("contract")
        .cloned()
        .context("factory recovery revision request omitted its contract")?;
    let objective = value_string(&contract, "/objective")?;
    let objective = objective.trim();
    // On replay, the task projection already includes the replacement description.
    // Match the store's composed objective, not just a source substring or the reason.
    let task_objective = if !description.is_empty() && objective == description {
        ""
    } else if !description.is_empty() {
        objective
            .strip_prefix(&description)
            .and_then(|remainder| remainder.strip_prefix(SEPARATOR))
            .unwrap_or(objective)
    } else {
        objective
    };
    let objective = if description.is_empty() {
        task_objective.to_owned()
    } else if task_objective.is_empty()
        || description.len() + SEPARATOR.len() + task_objective.len() > 100_000
    {
        description
    } else {
        format!("{description}{SEPARATOR}{task_objective}")
    };
    contract["objective"] = json!(objective);
    Ok(contract)
}

fn cached_recovery_contract_revision(
    args: &FactoryArgs,
    mission_id: Uuid,
    snapshot: &FactoryRecoverySnapshot,
    request: &Value,
) -> Result<Option<Uuid>> {
    let replacement_contract = recovery_replacement_contract(request)?;
    let replacement_policy = &request["verification_policy"];
    // A matching historical revision is not authority if another revision superseded
    // it. Both its version and complete replacement must still be the task projection.
    if replacement_contract != serde_json::to_value(&snapshot.contract)?
        || replacement_policy != &serde_json::to_value(&snapshot.verification_policy)?
    {
        return Ok(None);
    }
    snapshot
        .contract_revisions
        .iter()
        .find(|revision| {
            revision["corp_id"] == json!(args.corp_id)
                && revision["mission_id"] == json!(mission_id)
                && revision["task_id"] == request["task_id"]
                && revision["source_run_id"] == request["source_run_id"]
                && revision["revised_by"] == request["actor_id"]
                && revision["next_action"] == request["next_action"]
                && revision["reason"] == request["reason"]
                && revision["version"] == json!(snapshot.contract_version)
                && revision["replacement_description"] == request["description"]
                && revision["replacement_contract"] == replacement_contract
                && &revision["replacement_verification_policy"] == replacement_policy
        })
        .map(|revision| value_uuid(revision, "/id"))
        .transpose()
}

fn recovery_revision_required(
    mode: FactoryVerificationRecoveryMode,
    source_changed: bool,
    policy_changed: bool,
    snapshot: &FactoryRecoverySnapshot,
) -> bool {
    mode == FactoryVerificationRecoveryMode::SourceCorrection
        || source_changed
        || policy_changed
        || snapshot.contract_revisions.iter().any(|revision| {
            revision["task_id"] == json!(snapshot.task_id)
                && revision["source_run_id"] == json!(snapshot.source_run_id)
                && revision["version"] == json!(snapshot.contract_version)
        })
}

fn recovery_revision_key(
    corp_id: Uuid,
    mission_id: Uuid,
    work_item_id: Uuid,
    mode: FactoryVerificationRecoveryMode,
    reviewed_source_snapshot: &Value,
    request: &Value,
) -> Uuid {
    // Include the complete request: changed policy or actor with the same reason
    // must not collide with a revision that persisted before authorization failed.
    stable_uuid(
        &json!({
            "operation": "factory-verification-recovery-contract-v2",
            "corp_id": corp_id,
            "mission_id": mission_id,
            "factory_work_item_id": work_item_id,
            "mode": mode,
            "reviewed_source_snapshot": reviewed_source_snapshot,
            "request": request,
        })
        .to_string(),
    )
}

fn active_recovery_contract_revision(
    active: &Value,
    mode: FactoryVerificationRecoveryMode,
    reviewed_source_snapshot: &Value,
    replacement_policy: &VerificationPolicy,
) -> Result<Option<Uuid>> {
    if active["observed_source_revision"] != reviewed_source_snapshot["source_revision"]
        || &active["reviewed_source_snapshot"] != reviewed_source_snapshot
    {
        bail!("active factory recovery reviewed source does not match this replay");
    }
    if active["replacement_verification_policy"] != serde_json::to_value(replacement_policy)? {
        bail!("active factory recovery replacement verification policy does not match this replay");
    }
    let revision_id = match active.get("contract_revision_id") {
        Some(Value::Null) => None,
        Some(Value::String(id)) => Some(
            Uuid::parse_str(id).context("active factory recovery contract revision is invalid")?,
        ),
        _ => bail!("active factory recovery omitted its contract revision identity"),
    };
    if mode == FactoryVerificationRecoveryMode::SourceCorrection && revision_id.is_none() {
        bail!("active source-correction recovery has no contract revision");
    }
    Ok(revision_id)
}

#[allow(clippy::too_many_arguments)]
async fn recover_factory_verification(
    client: &Client,
    server: &str,
    args: &FactoryArgs,
    selected: &EvaluatedItem,
    persisted: &ExistingFactoryItem,
    work_item_id: Uuid,
    claim_token: Uuid,
    expected_factory_version: i64,
    mission_id: Uuid,
    stable_prefix: &str,
    requested_verification_policy: Option<&VerificationPolicy>,
) -> Result<Value> {
    let mode_arg = args
        .verification_recovery
        .context("factory verification recovery mode is missing")?;
    let mode: FactoryVerificationRecoveryMode = mode_arg.into();
    let reason = args
        .verification_recovery_reason
        .as_deref()
        .context("factory verification recovery reason is missing")?;
    let mut snapshot = load_factory_recovery_snapshot(
        client,
        server,
        args,
        mission_id,
        work_item_id,
        mode,
        reason,
    )
    .await?;
    let reviewed_source_snapshot = reviewed_recovery_source(args, selected);
    if let Some(active) = snapshot.active_recovery.as_ref() {
        active_recovery_contract_revision(
            active,
            mode,
            &reviewed_source_snapshot,
            requested_verification_policy.unwrap_or(&snapshot.verification_policy),
        )?;
    }
    let legacy_workspace_checkpointed = snapshot.workspace_fingerprint.is_none();
    if legacy_workspace_checkpointed {
        let expected_head_commit = snapshot
            .expected_head_commit
            .as_deref()
            .context(
                "legacy factory recovery requires a verification-linked head commit before workspace checkpointing",
            )?;
        let checkpoint = server_json(
            client,
            Method::POST,
            format!(
                "{server}/api/corps/{}/factory/work-items/{work_item_id}/workspace-checkpoint",
                args.corp_id
            ),
            Some(json!({
                "actor_id": args.actor_id,
                "claim_token": claim_token,
                "expected_version": expected_factory_version,
                "idempotency_key": format!(
                    "{stable_prefix}:workspace-checkpoint:{}",
                    snapshot.source_run_id
                ),
                "source_run_id": snapshot.source_run_id,
                "expected_head_commit": expected_head_commit,
            })),
        )
        .await
        .context("request preserved legacy workspace checkpoint")?;
        if let Some(returned_token) = checkpoint.get("claim_token").and_then(Value::as_str) {
            let returned_token = Uuid::parse_str(returned_token)
                .context("workspace checkpoint claim token is invalid")?;
            if returned_token != claim_token {
                bail!("workspace checkpoint returned a different factory fencing token");
            }
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        while snapshot.workspace_fingerprint.is_none() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
            snapshot = load_factory_recovery_snapshot(
                client,
                server,
                args,
                mission_id,
                work_item_id,
                mode,
                reason,
            )
            .await?;
        }
    }
    let workspace_fingerprint = snapshot
        .workspace_fingerprint
        .clone()
        .context("factory recovery source workspace checkpoint did not complete")?;
    let replacement_policy = requested_verification_policy
        .cloned()
        .unwrap_or_else(|| snapshot.verification_policy.clone());
    let source_changed = selected.issue.updated_at != persisted.source_revision;
    let policy_changed = replacement_policy != snapshot.verification_policy;
    let reason_digest = format!("{:x}", Sha256::digest(reason.as_bytes()));
    let contract_revision_id = if let Some(active) = snapshot.active_recovery.as_ref() {
        active_recovery_contract_revision(
            active,
            mode,
            &reviewed_source_snapshot,
            &replacement_policy,
        )?
    } else if recovery_revision_required(mode, source_changed, policy_changed, &snapshot) {
        let mut request = recovery_contract_request(
            args,
            &snapshot,
            &selected.issue,
            mode,
            source_changed,
            reason,
            &replacement_policy,
            &reviewed_source_snapshot,
        )?;
        if let Some(revision_id) =
            cached_recovery_contract_revision(args, mission_id, &snapshot, &request)?
        {
            Some(revision_id)
        } else {
            request["idempotency_key"] = json!(recovery_revision_key(
                args.corp_id,
                mission_id,
                work_item_id,
                mode,
                &reviewed_source_snapshot,
                &request,
            ));
            let revision = server_json(
                client,
                Method::POST,
                format!(
                    "{server}/api/corps/{}/missions/{mission_id}/contract-revisions",
                    args.corp_id
                ),
                Some(request),
            )
            .await
            .context("persist factory verification recovery contract revision")?;
            Some(value_uuid(&revision, "/revision/id")?)
        }
    } else {
        None
    };
    let recovery_key = stable_uuid(&format!(
        "{stable_prefix}:verification-recovery:{}:{}:{}",
        snapshot.source_run_id,
        mode.as_str(),
        reason_digest,
    ));
    let recovery = server_json(
        client,
        Method::POST,
        format!(
            "{server}/api/corps/{}/factory/work-items/{work_item_id}/verification-recoveries",
            args.corp_id
        ),
        Some(json!({
            "actor_id": args.actor_id,
            "claim_token": claim_token,
            "expected_factory_version": expected_factory_version,
            "idempotency_key": recovery_key,
            "source_run_id": snapshot.source_run_id,
            "mode": mode,
            "reason": reason,
            "observed_source_revision": selected.issue.updated_at,
            "reviewed_source_snapshot": reviewed_source_snapshot,
            "contract_revision_id": contract_revision_id,
            "expected_workspace_fingerprint": workspace_fingerprint,
            "expected_head_commit": snapshot.expected_head_commit,
        })),
    )
    .await
    .context("authorize factory verification recovery")?;
    let recovery_status = value_string(&recovery, "/recovery/status")?;
    let factory_state = value_string(&recovery, "/work_item/state")?;
    let dispatch_failed = recovery_status == "failed" || factory_state == "verification_failed";
    Ok(json!({
        "recovered": recovery
            .get("replayed")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        "verification_recovery": true,
        "recovery_id": value_uuid(&recovery, "/recovery/id")?,
        "recovery_mode": mode,
        "legacy_workspace_checkpointed": legacy_workspace_checkpointed,
        "run_id": value_uuid(&recovery, "/run_id")?,
        "run_ids": [value_uuid(&recovery, "/run_id")?],
        "mission_status": if dispatch_failed { "failed" } else { "running" },
        "verification_failed": dispatch_failed,
        "awaiting_approval": false,
        "run_summary": recovery
            .pointer("/work_item/failure_detail")
            .and_then(Value::as_str),
        "contract_revision_id": contract_revision_id,
        "factory_state": factory_state,
        "factory_version": value_i64(&recovery, "/work_item/version")?,
    }))
}

fn stable_uuid(value: &str) -> Uuid {
    let digest = Sha256::digest(value.as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

fn existing_factory_run_summary(snapshot: &Value, mission_id: Uuid) -> Result<Option<Value>> {
    let (mission_status, task_ids, verification_failed, awaiting_approval) =
        factory_mission_snapshot(snapshot, mission_id)?;
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
    if existing_runs.is_empty() {
        return Ok(None);
    }
    let terminal_mission = matches!(
        mission_status.as_str(),
        "completed" | "failed" | "cancelled"
    );
    let workspace_cleanup_pending = terminal_mission
        && existing_runs.iter().any(|run| {
            run.get("workspace_disposition")
                .and_then(Value::as_str)
                .is_none_or(|disposition| disposition == "active")
        });
    Ok(Some(json!({
        "recovered": true,
        "mission_status": mission_status,
        "verification_failed": verification_failed,
        "awaiting_approval": awaiting_approval,
        "workspace_cleanup_pending": workspace_cleanup_pending,
        "run_summary": existing_runs
            .first()
            .and_then(|run| run.get("summary"))
            .and_then(Value::as_str),
        "run_ids": existing_runs
            .iter()
            .filter_map(|run| run.get("id").and_then(Value::as_str))
            .collect::<Vec<_>>(),
    })))
}

async fn existing_factory_run_summary_after_workspace_finalization(
    client: &Client,
    server: &str,
    args: &FactoryArgs,
    snapshot: &Value,
    mission_id: Uuid,
) -> Result<Option<Value>> {
    let Some(mut recovered) = existing_factory_run_summary(snapshot, mission_id)? else {
        return Ok(None);
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    while recovered
        .get("workspace_cleanup_pending")
        .and_then(Value::as_bool)
        == Some(true)
        && Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(100)).await;
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
        recovered = existing_factory_run_summary(&refreshed, mission_id)?
            .context("factory run disappeared while awaiting workspace finalization")?;
    }
    Ok(Some(recovered))
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
    if let Some(recovered) = existing_factory_run_summary_after_workspace_finalization(
        client, server, args, &snapshot, mission_id,
    )
    .await?
    {
        return Ok(recovered);
    }
    let (mission_status, _, _, _) = factory_mission_snapshot(&snapshot, mission_id)?;
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
        if let Some(recovered) = existing_factory_run_summary_after_workspace_finalization(
            client, server, args, &refreshed, mission_id,
        )
        .await?
        {
            return Ok(recovered);
        }
    }
    bail!("factory mission launch failed with {status}: {body}")
}

fn factory_mission_snapshot(
    snapshot: &Value,
    mission_id: Uuid,
) -> Result<(String, HashSet<String>, bool, bool)> {
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
    let awaiting_approval = mission_tasks.iter().any(|task| {
        task.get("verification_status").and_then(Value::as_str) == Some("waiting_for_approval")
            || task.get("status").and_then(Value::as_str) == Some("awaiting_approval")
    });
    Ok((
        mission_status,
        task_ids,
        verification_failed,
        awaiting_approval,
    ))
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

pub(crate) fn sanitize_failure_detail(value: &str) -> String {
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

pub(crate) fn gh_json(github_cli: &Path, args: &[&str]) -> Result<Value> {
    let output = gh_output(github_cli, args)?;
    serde_json::from_slice(quota::json_body(&output)?).with_context(|| {
        format!(
            "decode JSON from {} {}",
            github_cli.display(),
            args.join(" ")
        )
    })
}

fn graphql_json(args: &FactoryArgs, query: &str, variables: Value) -> Result<Value> {
    args.github_budget.check(Utc::now())?;
    let variables = variables
        .as_object()
        .context("GraphQL variables must be an object")?;
    let mut command = vec![
        "api".to_owned(),
        "graphql".to_owned(),
        "--include".to_owned(),
        "-f".to_owned(),
        format!("query={query}"),
    ];
    for (key, value) in variables {
        match value {
            Value::String(value) => command.extend(["-f".to_owned(), format!("{key}={value}")]),
            Value::Number(_) | Value::Bool(_) | Value::Null => {
                command.extend(["-F".to_owned(), format!("{key}={value}")])
            }
            _ => bail!("unsupported GraphQL variable shape"),
        }
    }
    let refs = command.iter().map(String::as_str).collect::<Vec<_>>();
    let output = match gh_output(&args.github_cli, &refs) {
        Ok(output) => output,
        Err(error) => {
            args.github_budget.failure(&error, Utc::now());
            return Err(error);
        }
    };
    let value: Value = serde_json::from_slice(quota::json_body(&output)?)
        .context("decode bounded GitHub GraphQL response")?;
    if graphql_envelope_has_errors(&value) {
        if let Some(wait) = quota::classify_failure(&output, &[], Utc::now()) {
            let error = anyhow!(wait);
            args.github_budget.failure(&error, Utc::now());
            return Err(error);
        }
        bail!("GitHub GraphQL returned partial errors; refusing incomplete source data");
    }
    if value.pointer("/data/rateLimit").is_none() {
        bail!("GitHub GraphQL response omitted rateLimit; refusing unbudgeted discovery");
    }
    args.github_budget.observe_json(&value, Utc::now())?;
    Ok(value)
}

fn graphql_envelope_has_errors(value: &Value) -> bool {
    match value.get("errors") {
        None => false,
        Some(Value::Array(errors)) => !errors.is_empty(),
        Some(_) => true,
    }
}

fn gh_json_guarded(github_cli: &Path, args: &[&str], budget: &quota::BudgetState) -> Result<Value> {
    budget.check(Utc::now())?;
    let result = gh_json(github_cli, args);
    if let Err(error) = &result {
        budget.failure(error, Utc::now());
    }
    result
}

fn gh_run_guarded(github_cli: &Path, args: &[&str], budget: &quota::BudgetState) -> Result<()> {
    budget.check(Utc::now())?;
    let result = gh_run(github_cli, args);
    if let Err(error) = &result {
        budget.failure(error, Utc::now());
    }
    result
}

pub(crate) fn gh_run(github_cli: &Path, args: &[&str]) -> Result<()> {
    gh_output(github_cli, args).map(|_| ())
}

pub(crate) fn gh_output(github_cli: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let prefix_args = env::var("ECORP_GITHUB_CLI_PREFIX_ARGS_JSON")
        .ok()
        .map(|value| {
            serde_json::from_str::<Vec<String>>(&value)
                .context("decode ECORP_GITHUB_CLI_PREFIX_ARGS_JSON")
        })
        .transpose()?
        .unwrap_or_default();
    let mut child = Command::new(github_cli)
        .args(prefix_args)
        .args(args)
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_PAGER", "cat")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("run {} {}", github_cli.display(), args.join(" ")))?;
    let stdout = child
        .stdout
        .take()
        .context("capture GitHub CLI standard output")?;
    let stderr = child
        .stderr
        .take()
        .context("capture GitHub CLI standard error")?;
    let stdout_reader = thread::spawn(move || read_process_output(stdout));
    let stderr_reader = thread::spawn(move || read_process_output(stderr));
    let timeout = github_command_timeout();
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().context("poll GitHub CLI process")? {
            break status;
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = join_process_output(stdout_reader, "GitHub CLI standard output");
            let _ = join_process_output(stderr_reader, "GitHub CLI standard error");
            bail!(
                "{} {} timed out after {} ms",
                github_cli.display(),
                args.join(" "),
                timeout.as_millis()
            );
        }
        thread::sleep(Duration::from_millis(25));
    };
    let stdout = join_process_output(stdout_reader, "GitHub CLI standard output")?;
    let stderr = join_process_output(stderr_reader, "GitHub CLI standard error")?;
    if !status.success() {
        if let Some(wait) = quota::classify_failure(&stdout, &stderr, Utc::now()) {
            return Err(wait.into());
        }
        let detail = sanitize_failure_detail(&String::from_utf8_lossy(&stderr));
        bail!(
            "{} {} failed: {}",
            github_cli.display(),
            args.join(" "),
            detail
        );
    }
    Ok(stdout)
}

pub(crate) fn source_git_output(repository: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("run git -C {} {}", repository.display(), args.join(" ")))?;
    let stdout = child
        .stdout
        .take()
        .context("capture source Git standard output")?;
    let stderr = child
        .stderr
        .take()
        .context("capture source Git standard error")?;
    let stdout_reader = thread::spawn(move || read_process_output(stdout));
    let stderr_reader = thread::spawn(move || read_process_output(stderr));
    let timeout = source_git_command_timeout();
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().context("poll source Git process")? {
            break status;
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = join_process_output(stdout_reader, "source Git standard output");
            let _ = join_process_output(stderr_reader, "source Git standard error");
            bail!(
                "git -C {} {} timed out after {} ms",
                repository.display(),
                args.join(" "),
                timeout.as_millis()
            );
        }
        thread::sleep(Duration::from_millis(25));
    };
    let stdout = join_process_output(stdout_reader, "source Git standard output")?;
    let stderr = join_process_output(stderr_reader, "source Git standard error")?;
    if !status.success() {
        bail!(
            "git -C {} {} failed: {}",
            repository.display(),
            args.join(" "),
            sanitize_failure_detail(&String::from_utf8_lossy(&stderr))
        );
    }
    Ok(stdout)
}

fn github_command_timeout() -> Duration {
    let timeout_ms = env::var("ECORP_GITHUB_COMMAND_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_GITHUB_COMMAND_TIMEOUT_MS)
        .clamp(100, DEFAULT_GITHUB_COMMAND_TIMEOUT_MS);
    Duration::from_millis(timeout_ms)
}

fn source_git_command_timeout() -> Duration {
    let timeout_ms = env::var("ECORP_SOURCE_GIT_COMMAND_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_SOURCE_GIT_COMMAND_TIMEOUT_MS)
        .clamp(100, DEFAULT_SOURCE_GIT_COMMAND_TIMEOUT_MS);
    Duration::from_millis(timeout_ms)
}

fn read_process_output(mut pipe: impl Read) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn join_process_output(
    reader: thread::JoinHandle<std::io::Result<Vec<u8>>>,
    stream: &str,
) -> Result<Vec<u8>> {
    reader
        .join()
        .map_err(|_| anyhow!("{stream} reader panicked"))?
        .with_context(|| format!("read {stream}"))
}

pub(crate) async fn server_json(
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
    use std::collections::HashMap;

    use chrono::{Duration, Utc};
    use serde_json::{Value, json};
    use uuid::Uuid;

    use super::{
        EvaluatedItem, ExistingFactoryItem, FactoryArgs, FactoryRecoverySnapshot,
        FactoryVerificationRecoveryMode, IssueLabel, IssueView, ProjectContent, ProjectItem,
        RECOVERY_SOURCE_REFERENCE_PREFIX, VerificationRecoveryModeArg, acceptance_tests,
        active_recovery_contract_revision, blocked_dependency_numbers,
        cached_recovery_contract_revision, evaluate_items, factory_completion_is_ready,
        factory_item_recoverable_by, factory_recovery_source_matches, graphql_envelope_has_errors,
        issue_numbers, normalize_github_component, parse_exact_project_item,
        parse_github_repository_identity, publication_base_branch, recovery_contract_request,
        recovery_replacement_contract, recovery_revision_key, recovery_revision_required,
        resolve_recovery_publication_base_ref, reviewed_recovery_source, sanitize_failure_detail,
        selected_publication_base_ref, truncate_utf8, validate_source_base_commit,
    };

    struct RecoveryFixture {
        args: FactoryArgs,
        selected: EvaluatedItem,
        snapshot: FactoryRecoverySnapshot,
        mission_id: Uuid,
        work_item_id: Uuid,
        revision_id: Uuid,
    }

    impl RecoveryFixture {
        fn new() -> Self {
            let args = FactoryArgs {
                corp_id: Uuid::from_u128(1),
                actor_id: Uuid::from_u128(2),
                owner: "owner".to_owned(),
                project_number: 3,
                repository: "owner/repo".to_owned(),
                source_base_ref: "main".to_owned(),
                publication_base_ref: None,
                source_repository_path: ".".into(),
                adapter: "github-copilot".to_owned(),
                allowed_adapters: Vec::new(),
                strategy: "single".to_owned(),
                model: Some("reviewed-model".to_owned()),
                reasoning_effort: Some("high".to_owned()),
                budget_tokens: 10_000,
                budget_cost_microusd: 100_000,
                lease_seconds: 300,
                write_scope: vec!["src/**".to_owned()],
                verification_policy_file: None,
                issue: Some(113),
                verification_recovery: Some(VerificationRecoveryModeArg::SourceCorrection),
                verification_recovery_reason: Some("Correct the reviewed check".to_owned()),
                dry_run: false,
                github_cli: "must-not-run-github".into(),
                github_budget: Default::default(),
                active_issue: Default::default(),
            };
            let issue = IssueView {
                id: "issue-node".to_owned(),
                number: 113,
                title: "Governed correction".to_owned(),
                body: "## Acceptance criteria\n- [ ] Preserve the original work".to_owned(),
                url: "https://github.com/owner/repo/issues/113".to_owned(),
                state: "OPEN".to_owned(),
                created_at: "2026-09-01T00:00:00Z".to_owned(),
                updated_at: "2026-09-06T00:00:00Z".to_owned(),
                labels: vec![IssueLabel {
                    name: "factory:ready".to_owned(),
                }],
            };
            let project_item = ProjectItem {
                id: "project-item".to_owned(),
                status: "Todo".to_owned(),
                content: ProjectContent {
                    body: issue.body.clone(),
                    number: issue.number,
                    repository: args.repository.clone(),
                    title: issue.title.clone(),
                    kind: "Issue".to_owned(),
                    url: issue.url.clone(),
                },
            };
            Self {
                args,
                selected: EvaluatedItem {
                    project_item,
                    issue,
                    dependencies: Vec::new(),
                    reasons: Vec::new(),
                    recovery: true,
                },
                snapshot: FactoryRecoverySnapshot {
                    task_id: Uuid::from_u128(3),
                    source_run_id: Uuid::from_u128(4),
                    contract_version: 1,
                    contract: serde_json::from_value(json!({
                        "objective": "Original task objective",
                        "expected_output": "Verified source",
                        "source_repository": "owner/repo",
                        "source_base_ref": "main",
                        "source_base_commit": "a".repeat(40),
                        "acceptance_tests": ["Keep source identity"],
                        "allowed_tools": ["filesystem"],
                        "prohibited_actions": ["No replacement mission"],
                        "references": ["docs/SECURITY.md"],
                        "write_scope": ["src/**"],
                        "budget_tokens": 10_000,
                        "budget_cost_microusd": 100_000,
                        "escalation": "Ask the owner",
                        "model": "reviewed-model",
                        "reasoning_effort": "high",
                        "deliverable": {
                            "form": "commit_branch", "commit_after_verification": true,
                            "paths": ["src/**"]
                        }
                    }))
                    .unwrap(),
                    verification_policy: serde_json::from_value(json!({
                        "checks": [{"type": "file", "path": "src/result.rs", "min_bytes": 1}],
                        "manual_gate": {
                            "type": "independent_review", "roles": ["manager"],
                            "exclude_requester": true
                        }
                    }))
                    .unwrap(),
                    workspace_fingerprint: Some("b".repeat(64)),
                    expected_head_commit: Some("c".repeat(40)),
                    contract_revisions: Vec::new(),
                    active_recovery: None,
                },
                mission_id: Uuid::from_u128(5),
                work_item_id: Uuid::from_u128(6),
                revision_id: Uuid::from_u128(7),
            }
        }

        fn reviewed_source(&self) -> Value {
            reviewed_recovery_source(&self.args, &self.selected)
        }

        fn request(&self) -> Value {
            recovery_contract_request(
                &self.args,
                &self.snapshot,
                &self.selected.issue,
                FactoryVerificationRecoveryMode::SourceCorrection,
                true,
                self.args.verification_recovery_reason.as_deref().unwrap(),
                &self.snapshot.verification_policy,
                &self.reviewed_source(),
            )
            .unwrap()
        }

        fn cache_revision(&mut self) {
            let request = self.request();
            self.cache_request(request);
        }

        fn cache_request(&mut self, request: Value) {
            let contract = recovery_replacement_contract(&request).unwrap();
            self.snapshot.contract = serde_json::from_value(contract.clone()).unwrap();
            self.snapshot.verification_policy =
                serde_json::from_value(request["verification_policy"].clone()).unwrap();
            self.snapshot.contract_version += 1;
            self.snapshot.contract_revisions.push(json!({
                "id": self.revision_id,
                "corp_id": self.args.corp_id,
                "mission_id": self.mission_id,
                "task_id": self.snapshot.task_id,
                "source_run_id": self.snapshot.source_run_id,
                "revised_by": self.args.actor_id,
                "next_action": "resume",
                "reason": request["reason"],
                "version": self.snapshot.contract_version,
                "replacement_description": request["description"],
                "replacement_contract": contract,
                "replacement_verification_policy": request["verification_policy"],
            }));
        }

        fn cached_id(&self, request: &Value) -> Option<Uuid> {
            cached_recovery_contract_revision(&self.args, self.mission_id, &self.snapshot, request)
                .unwrap()
        }

        fn persisted_item(&self) -> ExistingFactoryItem {
            ExistingFactoryItem {
                version: 1,
                source_project_owner: self.args.owner.clone(),
                source_project_number: i64::from(self.args.project_number),
                source_project_item_id: self.selected.project_item.id.clone(),
                source_repository_owner: "owner".to_owned(),
                source_repository_name: "repo".to_owned(),
                source_issue_number: self.selected.issue.number,
                source_issue_node_id: self.selected.issue.id.clone(),
                source_issue_url: self.selected.issue.url.clone(),
                source_title: self.selected.issue.title.clone(),
                source_revision: "2026-09-01T00:00:00Z".to_owned(),
                claim_owner_id: self.args.actor_id,
                state: "verification_failed".to_owned(),
                mission_id: Some(self.mission_id),
                policy: json!({}),
                lease_expires_at: Utc::now() + Duration::minutes(5),
            }
        }
    }

    #[test]
    fn recovery_completion_preserves_catch_up_states_and_cleanup_gate() {
        for state in ["mission_created", "running", "awaiting_approval", "blocked"] {
            assert!(factory_completion_is_ready(true, false, state));
            assert!(!factory_completion_is_ready(true, true, state));
            assert!(!factory_completion_is_ready(false, false, state));
        }
        for state in [
            "claimed",
            "verification_failed",
            "failed",
            "cancelled",
            "verified",
            "published",
        ] {
            assert!(!factory_completion_is_ready(true, false, state));
        }
    }

    #[test]
    fn recovery_cached_revision_replays_the_exact_current_replacement() {
        let mut fixture = RecoveryFixture::new();
        assert_eq!(fixture.cached_id(&fixture.request()), None);
        fixture.cache_revision();
        assert_eq!(
            fixture.cached_id(&fixture.request()),
            Some(fixture.revision_id)
        );
        assert_eq!(
            fixture.cached_id(&fixture.request()),
            Some(fixture.revision_id)
        );
    }

    #[test]
    fn recovery_legacy_unbound_revision_requires_a_new_explicit_binding() {
        let mut fixture = RecoveryFixture::new();
        assert!(!recovery_revision_required(
            FactoryVerificationRecoveryMode::VerifierOnly,
            false,
            false,
            &fixture.snapshot,
        ));
        fixture.cache_revision();
        fixture
            .snapshot
            .contract
            .references
            .retain(|reference| !reference.starts_with(RECOVERY_SOURCE_REFERENCE_PREFIX));
        fixture.snapshot.contract_revisions[0]["replacement_contract"] =
            serde_json::to_value(&fixture.snapshot.contract).unwrap();
        assert_eq!(fixture.cached_id(&fixture.request()), None);
        assert!(recovery_revision_required(
            FactoryVerificationRecoveryMode::VerifierOnly,
            false,
            false,
            &fixture.snapshot,
        ));
    }

    #[test]
    fn recovery_verifier_only_policy_revision_replays_after_a_lost_authorization_response() {
        let mut fixture = RecoveryFixture::new();
        let mut policy = serde_json::to_value(&fixture.snapshot.verification_policy).unwrap();
        policy["checks"][0]["path"] = json!("src/corrected.rs");
        let policy = serde_json::from_value(policy).unwrap();
        let build_request = |fixture: &RecoveryFixture| {
            recovery_contract_request(
                &fixture.args,
                &fixture.snapshot,
                &fixture.selected.issue,
                FactoryVerificationRecoveryMode::VerifierOnly,
                false,
                fixture
                    .args
                    .verification_recovery_reason
                    .as_deref()
                    .unwrap(),
                &policy,
                &fixture.reviewed_source(),
            )
            .unwrap()
        };
        let request = build_request(&fixture);
        fixture.cache_request(request);
        assert!(recovery_revision_required(
            FactoryVerificationRecoveryMode::VerifierOnly,
            false,
            false,
            &fixture.snapshot,
        ));
        assert_eq!(
            fixture.cached_id(&build_request(&fixture)),
            Some(fixture.revision_id)
        );
    }

    #[test]
    fn recovery_cached_revision_rejects_changed_reviewed_source() {
        for change in ["revision", "body", "title", "node", "item"] {
            let mut fixture = RecoveryFixture::new();
            fixture.cache_revision();
            match change {
                "revision" => fixture.selected.issue.updated_at.push_str("-new"),
                "body" => fixture
                    .selected
                    .issue
                    .body
                    .push_str("\nReviewed new acceptance."),
                "title" => fixture.selected.issue.title.push_str(" changed"),
                "node" => fixture.selected.issue.id.push_str("-new"),
                "item" => fixture.selected.project_item.id.push_str("-new"),
                _ => unreachable!(),
            }
            assert_eq!(
                fixture.cached_id(&fixture.request()),
                None,
                "reused stale {change}"
            );
        }
    }

    #[test]
    fn recovery_cached_revision_rejects_changed_policy_with_same_source_and_reason() {
        let mut fixture = RecoveryFixture::new();
        fixture.cache_revision();
        let mut request = fixture.request();
        request["verification_policy"]["checks"][0]["path"] = json!("src/corrected.rs");
        assert_eq!(fixture.cached_id(&request), None);
        fixture.snapshot.verification_policy =
            serde_json::from_value(request["verification_policy"].clone()).unwrap();
        assert_eq!(fixture.cached_id(&request), None);
    }

    #[test]
    fn recovery_cached_revision_rejects_wrong_attribution_or_superseded_contract() {
        for (field, value) in [
            ("corp_id", json!(Uuid::from_u128(99))),
            ("mission_id", json!(Uuid::from_u128(99))),
            ("task_id", json!(Uuid::from_u128(99))),
            ("source_run_id", json!(Uuid::from_u128(99))),
            ("revised_by", json!(Uuid::from_u128(99))),
            ("next_action", json!("redispatch")),
            ("reason", json!("Different authority")),
            ("version", json!(1)),
            ("replacement_description", json!("Older source")),
        ] {
            let mut fixture = RecoveryFixture::new();
            fixture.cache_revision();
            fixture.snapshot.contract_revisions[0][field] = value;
            assert_eq!(
                fixture.cached_id(&fixture.request()),
                None,
                "reused wrong {field}"
            );
        }
        let mut fixture = RecoveryFixture::new();
        fixture.cache_revision();
        fixture.snapshot.contract.write_scope = vec!["src/narrow/**".to_owned()];
        assert_eq!(fixture.cached_id(&fixture.request()), None);
    }

    #[test]
    fn recovery_source_binding_survives_objective_size_cap_and_description_normalization() {
        let mut fixture = RecoveryFixture::new();
        fixture.selected.issue.body = "x".repeat(70_000);
        fixture.cache_revision();
        assert_eq!(
            fixture.snapshot.contract.objective,
            fixture.selected.issue.body
        );
        assert_eq!(
            fixture.cached_id(&fixture.request()),
            Some(fixture.revision_id)
        );
        fixture.selected.issue.updated_at.push_str("-new");
        assert_eq!(
            recovery_replacement_contract(&fixture.request()).unwrap()["objective"],
            json!(fixture.snapshot.contract.objective),
        );
        assert_eq!(fixture.cached_id(&fixture.request()), None);

        let mut fixture = RecoveryFixture::new();
        fixture.selected.issue.body = "  Reviewed\r\nsource  ".to_owned();
        fixture.cache_revision();
        assert_eq!(
            fixture.cached_id(&fixture.request()),
            Some(fixture.revision_id)
        );
        fixture.selected.issue.body = "Reviewed\nsource".to_owned();
        assert_eq!(fixture.cached_id(&fixture.request()), None);
    }

    #[test]
    fn recovery_revision_keys_bind_policy_actor_source_and_exact_request() {
        let fixture = RecoveryFixture::new();
        let request = fixture.request();
        let key = |request: &Value, source: &Value| {
            recovery_revision_key(
                fixture.args.corp_id,
                fixture.mission_id,
                fixture.work_item_id,
                FactoryVerificationRecoveryMode::SourceCorrection,
                source,
                request,
            )
        };
        let source = fixture.reviewed_source();
        let original = key(&request, &source);
        assert_eq!(original, key(&request, &source));
        for (pointer, value) in [
            ("/actor_id", json!(Uuid::from_u128(99))),
            ("/task_id", json!(Uuid::from_u128(99))),
            ("/source_run_id", json!(Uuid::from_u128(99))),
            ("/expected_contract_version", json!(2)),
            ("/reason", json!("Another authorization")),
            ("/contract/write_scope", json!(["src/narrow/**"])),
            (
                "/verification_policy/checks/0/path",
                json!("src/corrected.rs"),
            ),
        ] {
            let mut changed = request.clone();
            *changed.pointer_mut(pointer).unwrap() = value;
            assert_ne!(original, key(&changed, &source), "unbound {pointer}");
        }
        let mut changed_source = source.clone();
        changed_source["source_revision"] = json!("2026-09-06T01:00:00Z");
        assert_ne!(original, key(&request, &changed_source));
    }

    #[test]
    fn recovery_contract_preserves_authority_and_replaces_only_its_provenance_marker() {
        let mut fixture = RecoveryFixture::new();
        let original = serde_json::to_value(&fixture.snapshot.contract).unwrap();
        fixture.cache_revision();
        fixture.selected.issue.updated_at.push_str("-new");
        let request = fixture.request();
        for field in [
            "source_repository",
            "source_base_ref",
            "source_base_commit",
            "allowed_tools",
            "prohibited_actions",
            "write_scope",
            "budget_tokens",
            "budget_cost_microusd",
            "deadline_at",
            "escalation",
            "secret_refs",
            "model",
            "reasoning_effort",
            "deliverable",
        ] {
            assert_eq!(
                request["contract"][field], original[field],
                "changed {field}"
            );
        }
        let references = request["contract"]["references"].as_array().unwrap();
        assert!(references.contains(&json!("docs/SECURITY.md")));
        assert_eq!(
            references
                .iter()
                .filter(|reference| reference
                    .as_str()
                    .unwrap()
                    .starts_with(RECOVERY_SOURCE_REFERENCE_PREFIX))
                .count(),
            1
        );
        assert_eq!(request["actor_id"], json!(fixture.args.actor_id));
        assert_eq!(
            request["verification_policy"],
            serde_json::to_value(&fixture.snapshot.verification_policy).unwrap()
        );
    }

    #[test]
    fn recovery_active_replay_requires_the_same_reviewed_source_and_policy() {
        let fixture = RecoveryFixture::new();
        let source = fixture.reviewed_source();
        let policy = &fixture.snapshot.verification_policy;
        let active = json!({
            "observed_source_revision": source["source_revision"],
            "reviewed_source_snapshot": source,
            "replacement_verification_policy": policy,
            "contract_revision_id": fixture.revision_id,
        });
        assert_eq!(
            active_recovery_contract_revision(
                &active,
                FactoryVerificationRecoveryMode::SourceCorrection,
                &source,
                policy,
            )
            .unwrap(),
            Some(fixture.revision_id)
        );
        let mut changed_source = source.clone();
        changed_source["body_sha256"] = json!("changed");
        assert!(
            active_recovery_contract_revision(
                &active,
                FactoryVerificationRecoveryMode::SourceCorrection,
                &changed_source,
                policy,
            )
            .is_err()
        );
        let mut changed_active = active.clone();
        changed_active["replacement_verification_policy"]["checks"][0]["path"] =
            json!("src/old.rs");
        assert!(
            active_recovery_contract_revision(
                &changed_active,
                FactoryVerificationRecoveryMode::SourceCorrection,
                &source,
                policy,
            )
            .is_err()
        );
        changed_active = active;
        changed_active["contract_revision_id"] = Value::Null;
        assert!(
            active_recovery_contract_revision(
                &changed_active,
                FactoryVerificationRecoveryMode::SourceCorrection,
                &source,
                policy,
            )
            .is_err()
        );
        assert_eq!(
            active_recovery_contract_revision(
                &changed_active,
                FactoryVerificationRecoveryMode::VerifierOnly,
                &source,
                policy,
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn recovery_never_falls_back_to_new_factory_work_or_another_issue() {
        let fixture = RecoveryFixture::new();
        for persisted in [
            None,
            Some({
                let mut item = fixture.persisted_item();
                item.mission_id = None;
                item
            }),
        ] {
            let existing = persisted
                .into_iter()
                .map(|item| (fixture.selected.project_item.id.clone(), item))
                .collect();
            let mut issue_cache = HashMap::from([(
                fixture.selected.issue.number,
                fixture.selected.issue.clone(),
            )]);
            let evaluated = evaluate_items(
                &fixture.args,
                vec![fixture.selected.project_item.clone()],
                &existing,
                &mut issue_cache,
            )
            .unwrap();
            assert!(!evaluated[0].eligible());
            assert!(
                evaluated[0]
                    .reasons
                    .iter()
                    .any(|reason| reason.contains("replacement work is not authorized"))
            );
        }
        let persisted = fixture.persisted_item();
        assert!(factory_recovery_source_matches(
            &fixture.args,
            &fixture.selected.project_item,
            &fixture.selected.issue,
            &persisted,
        ));
        let mut changed = persisted;
        changed.source_issue_node_id = "replacement-issue".to_owned();
        assert!(!factory_recovery_source_matches(
            &fixture.args,
            &fixture.selected.project_item,
            &fixture.selected.issue,
            &changed,
        ));
    }

    #[test]
    fn graphql_error_envelope_must_be_absent_or_an_empty_array() {
        assert!(!graphql_envelope_has_errors(&json!({"data": {}})));
        assert!(!graphql_envelope_has_errors(
            &json!({"data": {}, "errors": []})
        ));
        for errors in [
            json!({}),
            json!("bad"),
            json!(null),
            json!([{"message":"bad"}]),
        ] {
            assert!(graphql_envelope_has_errors(
                &json!({"data": {"node":{}}, "errors": errors})
            ));
        }
    }

    #[test]
    fn exact_project_reads_preserve_source_content_and_project_identity() {
        let node = json!({
            "__typename": "ProjectV2Item", "id": "opaque-item", "isArchived": false,
            "project": {"number": 3, "owner": {"login": "Owner"}},
            "fieldValueByName": {"name": "In Progress"},
            "content": {"__typename": "Issue", "number": 7, "title": "Work",
                "body": "Exact source", "url": "https://github.com/owner/repo/issues/7",
                "repository": {"nameWithOwner": "Owner/Repo"}}
        });
        let item = parse_exact_project_item(&node, "owner", 3, "opaque-item").unwrap();
        assert_eq!(item.status, "In Progress");
        assert_eq!(item.content.repository, "owner/repo");
        assert_eq!(item.content.body, "Exact source");
        assert_eq!(item.content.number, 7);
        assert!(parse_exact_project_item(&node, "other", 3, "opaque-item").is_err());
        assert!(parse_exact_project_item(&node, "owner", 4, "opaque-item").is_err());
        assert!(parse_exact_project_item(&node, "owner", 3, "different-item").is_err());
        let mut archived = node.clone();
        archived["isArchived"] = json!(true);
        assert!(parse_exact_project_item(&archived, "owner", 3, "opaque-item").is_err());
        archived.as_object_mut().unwrap().remove("isArchived");
        assert!(parse_exact_project_item(&archived, "owner", 3, "opaque-item").is_err());
        assert!(
            parse_exact_project_item(&serde_json::Value::Null, "owner", 3, "opaque-item").is_err()
        );
    }

    #[test]
    fn exact_project_reads_reject_deleted_or_non_issue_content() {
        let mut node = json!({
            "__typename": "ProjectV2Item", "id": "item", "isArchived": false,
            "project": {"number": 3, "owner": {"login": "owner"}},
            "fieldValueByName": null, "content": null
        });
        assert!(parse_exact_project_item(&node, "owner", 3, "item").is_err());
        node["content"] = json!({"__typename": "DraftIssue", "number": 7});
        assert!(parse_exact_project_item(&node, "owner", 3, "item").is_err());
    }

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
        assert_eq!(
            parse_github_repository_identity("git@github.com:ShyamSridhar123/ECorp.git").as_deref(),
            Some("ShyamSridhar123/ECorp")
        );
    }

    #[test]
    fn source_commit_requires_a_full_git_object_id() {
        assert!(validate_source_base_commit(&"a".repeat(40)).is_ok());
        assert!(validate_source_base_commit(&"B".repeat(64)).is_ok());
        assert!(validate_source_base_commit(&"a".repeat(39)).is_err());
        assert!(validate_source_base_commit(&"g".repeat(40)).is_err());
    }

    #[test]
    fn publication_base_requires_head_or_a_branch_ref() {
        assert_eq!(publication_base_branch("HEAD").unwrap(), None);
        assert_eq!(publication_base_branch("main").unwrap(), Some("main"));
        assert_eq!(
            publication_base_branch("refs/heads/release").unwrap(),
            Some("release")
        );
        assert!(publication_base_branch("refs/tags/v1").is_err());
        assert!(publication_base_branch("refs/remotes/origin/main").is_err());
        assert!(publication_base_branch("refs/heads/").is_err());
        assert!(publication_base_branch("main\tbad").is_err());
    }

    #[test]
    fn publication_base_defaults_to_the_selected_source_ref() {
        let mut args = FactoryArgs {
            corp_id: Uuid::new_v4(),
            actor_id: Uuid::new_v4(),
            owner: "owner".to_owned(),
            project_number: 1,
            repository: "owner/repo".to_owned(),
            source_base_ref: "release".to_owned(),
            publication_base_ref: None,
            source_repository_path: ".".into(),
            adapter: "fake-process".to_owned(),
            allowed_adapters: Vec::new(),
            strategy: "single".to_owned(),
            model: None,
            reasoning_effort: None,
            budget_tokens: 1,
            budget_cost_microusd: 1,
            lease_seconds: 30,
            write_scope: vec!["**".to_owned()],
            verification_policy_file: None,
            issue: None,
            verification_recovery: None,
            verification_recovery_reason: None,
            dry_run: true,
            github_cli: "gh".into(),
            github_budget: Default::default(),
            active_issue: Default::default(),
        };
        assert_eq!(selected_publication_base_ref(&args), "release");
        args.publication_base_ref = Some("main".to_owned());
        assert_eq!(selected_publication_base_ref(&args), "main");
    }

    #[test]
    fn recovery_publication_base_reuses_persisted_policy() {
        let now = Utc::now();
        let mut item = ExistingFactoryItem {
            version: 1,
            source_project_owner: "owner".to_owned(),
            source_project_number: 1,
            source_project_item_id: "item".to_owned(),
            source_repository_owner: "owner".to_owned(),
            source_repository_name: "repo".to_owned(),
            source_issue_number: 1,
            source_issue_node_id: "issue".to_owned(),
            source_issue_url: "https://github.com/owner/repo/issues/1".to_owned(),
            source_title: "Issue".to_owned(),
            source_revision: "2026-09-02T00:00:00Z".to_owned(),
            claim_owner_id: Uuid::new_v4(),
            state: "running".to_owned(),
            mission_id: Some(Uuid::new_v4()),
            policy: json!({
                "publication": {
                    "base_ref": "main"
                }
            }),
            lease_expires_at: now + Duration::minutes(5),
        };
        let mut args = FactoryArgs {
            corp_id: Uuid::new_v4(),
            actor_id: item.claim_owner_id,
            owner: "owner".to_owned(),
            project_number: 1,
            repository: "owner/repo".to_owned(),
            source_base_ref: "release".to_owned(),
            publication_base_ref: None,
            source_repository_path: ".".into(),
            adapter: "fake-process".to_owned(),
            allowed_adapters: Vec::new(),
            strategy: "single".to_owned(),
            model: None,
            reasoning_effort: None,
            budget_tokens: 1,
            budget_cost_microusd: 1,
            lease_seconds: 30,
            write_scope: vec!["**".to_owned()],
            verification_policy_file: None,
            issue: None,
            verification_recovery: None,
            verification_recovery_reason: None,
            dry_run: true,
            github_cli: "gh".into(),
            github_budget: Default::default(),
            active_issue: Default::default(),
        };
        assert_eq!(
            resolve_recovery_publication_base_ref(&args, &item).unwrap(),
            "main"
        );
        args.publication_base_ref = Some("release".to_owned());
        assert!(resolve_recovery_publication_base_ref(&args, &item).is_err());
        item.policy = json!({});
        assert!(resolve_recovery_publication_base_ref(&args, &item).is_err());
    }

    #[test]
    fn verified_items_leave_the_queue_and_expired_claims_allow_failover() {
        let now = Utc::now();
        let owner = Uuid::new_v4();
        let replacement = Uuid::new_v4();
        let mut item = ExistingFactoryItem {
            version: 1,
            source_project_owner: "owner".to_owned(),
            source_project_number: 1,
            source_project_item_id: "item".to_owned(),
            source_repository_owner: "owner".to_owned(),
            source_repository_name: "repo".to_owned(),
            source_issue_number: 1,
            source_issue_node_id: "issue".to_owned(),
            source_issue_url: "https://github.com/owner/repo/issues/1".to_owned(),
            source_title: "Issue".to_owned(),
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
