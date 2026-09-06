use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use crony_domain::{
    Agent, AgentStatus, DeliverableForm, DeliverableSpec, ManualVerificationGate, PlannedTask,
    TaskContract, TaskGraphPlan, TaskSecretReference, VerificationPolicy, VerifierCheck,
    repository_relative_path_is_valid, write_scope_is_valid,
};

pub const MAX_GRAPH_NODES: usize = 8;
pub const MAX_GRAPH_DEPTH: i32 = 4;
pub const MAX_TASK_ATTEMPTS: i32 = 3;
pub const MAX_TASK_BUDGET_TOKENS: i64 = 2_000_000;
pub const MAX_GRAPH_BUDGET_TOKENS: i64 = 2_000_000;
const DEFAULT_SINGLE_TASK_BUDGET_TOKENS: i64 = 1_000_000;

pub fn uses_deterministic_harness(strategy: &str) -> bool {
    matches!(
        strategy,
        "verification-matrix" | "verification-failure" | "human-approval" | "independent-review"
    )
}

pub struct PlanningRequest<'a> {
    pub mission_title: &'a str,
    pub preferred_adapter: Option<&'a str>,
    pub preferred_model: Option<&'a str>,
    pub reasoning_effort: Option<&'a str>,
    pub secret_refs: &'a [TaskSecretReference],
    pub budget_tokens: Option<i64>,
    pub budget_cost_microusd: Option<i64>,
    pub deliverable: Option<&'a DeliverableSpec>,
    pub handoff_root: Option<&'a str>,
}

pub trait ManagerStrategy: Send + Sync {
    fn id(&self) -> &'static str;
    fn plan(&self, request: &PlanningRequest<'_>, agents: &[Agent]) -> Result<TaskGraphPlan>;
}

#[derive(Clone)]
pub struct StrategyRegistry {
    strategies: Arc<HashMap<String, Arc<dyn ManagerStrategy>>>,
}

impl StrategyRegistry {
    pub fn new() -> Self {
        let strategies: Vec<Arc<dyn ManagerStrategy>> = vec![
            Arc::new(SingleTaskStrategy),
            Arc::new(ParallelSpecialistsStrategy),
            Arc::new(StudioSwarmStrategy),
            Arc::new(VerificationMatrixStrategy),
            Arc::new(VerificationFailureStrategy),
            Arc::new(HumanApprovalStrategy),
            Arc::new(IndependentReviewStrategy),
        ];
        Self {
            strategies: Arc::new(
                strategies
                    .into_iter()
                    .map(|strategy| (strategy.id().to_owned(), strategy))
                    .collect(),
            ),
        }
    }

    #[cfg(test)]
    pub fn ids(&self) -> Vec<String> {
        let mut ids = self.strategies.keys().cloned().collect::<Vec<_>>();
        ids.sort();
        ids
    }

    pub fn plan(
        &self,
        strategy_id: &str,
        request: &PlanningRequest<'_>,
        agents: &[Agent],
    ) -> Result<TaskGraphPlan> {
        let strategy = self
            .strategies
            .get(strategy_id)
            .with_context(|| format!("unknown manager strategy {strategy_id}"))?;
        let plan = strategy.plan(request, agents)?;
        validate_plan(&plan, agents)?;
        Ok(plan)
    }
}

struct SingleTaskStrategy;

impl ManagerStrategy for SingleTaskStrategy {
    fn id(&self) -> &'static str {
        "single"
    }

    fn plan(&self, request: &PlanningRequest<'_>, agents: &[Agent]) -> Result<TaskGraphPlan> {
        let agent = ordered_candidates(agents)
            .into_iter()
            .find(|agent| {
                agent.role != "manager"
                    && request
                        .preferred_adapter
                        .is_none_or(|adapter| agent.adapter == adapter)
            })
            .context("no worker agent satisfies the requested adapter")?;
        let budget_tokens = request
            .budget_tokens
            .unwrap_or(DEFAULT_SINGLE_TASK_BUDGET_TOKENS);
        let budget_cost_microusd = request.budget_cost_microusd.unwrap_or(1_000_000);
        let mut task_contract = contract(
            format!("Complete the mission outcome: {}", request.mission_title),
            "A source-backed, verified mission artifact",
            budget_tokens,
        );
        task_contract.budget_cost_microusd = budget_cost_microusd;
        task_contract.secret_refs = request.secret_refs.to_vec();
        task_contract.model = request.preferred_model.map(str::to_owned);
        task_contract.reasoning_effort = request.reasoning_effort.map(str::to_owned);
        if let Some(deliverable) = request.deliverable {
            task_contract.deliverable = Some(deliverable.clone());
        }
        Ok(TaskGraphPlan {
            strategy: self.id().to_owned(),
            max_nodes: 1,
            max_depth: 0,
            budget_tokens,
            budget_cost_microusd,
            staffing: Vec::new(),
            tasks: vec![PlannedTask {
                key: "deliver".to_owned(),
                title: "Produce the mission outcome".to_owned(),
                contract: task_contract,
                assigned_agent_id: agent.id,
                required_adapter: agent.adapter.clone(),
                depends_on: Vec::new(),
                depth: 0,
                max_attempts: 2,
                verification_policy: artifact_policy(),
            }],
        })
    }
}

struct ParallelSpecialistsStrategy;

impl ManagerStrategy for ParallelSpecialistsStrategy {
    fn id(&self) -> &'static str {
        "parallel-specialists"
    }

    fn plan(&self, request: &PlanningRequest<'_>, agents: &[Agent]) -> Result<TaskGraphPlan> {
        let mut workers = ordered_candidates(agents)
            .into_iter()
            .filter(|agent| agent.role != "manager")
            .collect::<Vec<_>>();
        if let Some(preferred) = request.preferred_adapter {
            let position = workers
                .iter()
                .position(|agent| agent.adapter == preferred)
                .with_context(|| {
                    format!("parallel-specialists has no worker for preferred adapter {preferred}")
                })?;
            workers.swap(0, position);
        }
        let first = workers
            .first()
            .context("parallel-specialists requires at least two non-manager agents")?;
        let second = workers
            .get(1)
            .context("parallel-specialists requires at least two non-manager agents")?;
        let synthesizer = ordered_candidates(agents)
            .into_iter()
            .find(|agent| agent.role == "manager")
            .unwrap_or(first);
        let (first_model, first_reasoning) = provider_settings(request, &first.adapter);
        let (second_model, second_reasoning) = provider_settings(request, &second.adapter);
        let (synthesis_model, synthesis_reasoning) =
            provider_settings(request, &synthesizer.adapter);

        let total_budget = request.budget_tokens.unwrap_or(280_000);
        let total_cost_budget = request.budget_cost_microusd.unwrap_or(3_000_000);
        let specialist_budget = (total_budget * 2 / 7).max(1);
        let synthesis_budget = (total_budget - specialist_budget * 2).max(1);
        let specialist_cost_budget = (total_cost_budget * 2 / 7).max(1);
        let synthesis_cost_budget = (total_cost_budget - specialist_cost_budget * 2).max(1);
        let mut synthesis_contract = contract(
            format!(
                "Produce the final bounded outcome for this mission after both specialist tasks finish: {}",
                request.mission_title
            ),
            "A final synthesis artifact that addresses the mission and acceptance tests",
            synthesis_budget,
        );
        synthesis_contract.budget_cost_microusd = synthesis_cost_budget;
        synthesis_contract.model = synthesis_model;
        synthesis_contract.reasoning_effort = synthesis_reasoning;
        if let Some(deliverable) = request.deliverable {
            synthesis_contract.deliverable = Some(deliverable.clone());
        }
        synthesis_contract.references = vec![
            "task:specialist-a".to_owned(),
            "task:specialist-b".to_owned(),
        ];
        Ok(TaskGraphPlan {
            strategy: self.id().to_owned(),
            max_nodes: 3,
            max_depth: 1,
            budget_tokens: total_budget,
            budget_cost_microusd: total_cost_budget,
            staffing: Vec::new(),
            tasks: vec![
                PlannedTask {
                    key: "specialist-a".to_owned(),
                    title: format!("{} specialist pass", first.name),
                    contract: {
                        let mut contract = contract(
                            format!(
                                "Independently produce one concrete approach for this mission: {}",
                                request.mission_title
                            ),
                            "A concrete specialist artifact with assumptions and verification",
                            specialist_budget,
                        );
                        contract.budget_cost_microusd = specialist_cost_budget;
                        contract.model = first_model;
                        contract.reasoning_effort = first_reasoning;
                        contract
                    },
                    assigned_agent_id: first.id,
                    required_adapter: first.adapter.clone(),
                    depends_on: Vec::new(),
                    depth: 0,
                    max_attempts: 2,
                    verification_policy: artifact_policy(),
                },
                PlannedTask {
                    key: "specialist-b".to_owned(),
                    title: format!("{} specialist pass", second.name),
                    contract: {
                        let mut contract = contract(
                            format!(
                                "Independently produce a distinct approach for this mission: {}",
                                request.mission_title
                            ),
                            "A second specialist artifact with tradeoffs and verification",
                            specialist_budget,
                        );
                        contract.budget_cost_microusd = specialist_cost_budget;
                        contract.model = second_model;
                        contract.reasoning_effort = second_reasoning;
                        contract
                    },
                    assigned_agent_id: second.id,
                    required_adapter: second.adapter.clone(),
                    depends_on: Vec::new(),
                    depth: 0,
                    max_attempts: 2,
                    verification_policy: artifact_policy(),
                },
                PlannedTask {
                    key: "synthesis".to_owned(),
                    title: "Synthesize the specialist results".to_owned(),
                    contract: synthesis_contract,
                    assigned_agent_id: synthesizer.id,
                    required_adapter: synthesizer.adapter.clone(),
                    depends_on: vec!["specialist-a".to_owned(), "specialist-b".to_owned()],
                    depth: 1,
                    max_attempts: 2,
                    verification_policy: artifact_policy(),
                },
            ],
        })
    }
}

struct StudioSwarmStrategy;

impl ManagerStrategy for StudioSwarmStrategy {
    fn id(&self) -> &'static str {
        "studio-swarm"
    }

    fn plan(&self, request: &PlanningRequest<'_>, agents: &[Agent]) -> Result<TaskGraphPlan> {
        if request
            .preferred_adapter
            .is_some_and(|adapter| adapter != "github-copilot")
        {
            return Err(anyhow!(
                "studio-swarm requires github-copilot or an omitted requested adapter"
            ));
        }
        let mut identities = HashSet::new();
        let candidates = ordered_candidates(agents)
            .into_iter()
            .filter(|agent| agent.role != "manager" && agent.adapter == "github-copilot")
            .filter(|agent| identities.insert(agent.id))
            .collect::<Vec<_>>();
        let roles = [
            (
                "visual-direction",
                "visual and interaction direction",
                "Visual hierarchy, interaction states, accessibility, and responsive layout decisions",
                "Specify concrete visual tokens, interaction states, keyboard/focus behavior, and responsive layouts that integration can implement.",
            ),
            (
                "gameplay-systems",
                "gameplay and systems architecture",
                "Deterministic mechanics, state transitions, data contracts, and module boundaries",
                "Specify deterministic rules, state transitions, module interfaces, and edge cases with enough detail for implementation without further design work.",
            ),
            (
                "quality-verification",
                "quality and verification design",
                "Executable test cases, browser acceptance checks, accessibility, and performance criteria",
                "Specify reproducible test inputs and expected results, browser acceptance steps, accessibility checks, performance limits, and failure cases; distinguish proposed checks from observed evidence.",
            ),
        ];
        // Reserve every explicit staffing role before a generic worker fills an unmatched role.
        let mut workers =
            roles.map(|(key, ..)| candidates.iter().copied().find(|agent| agent.role == key));
        let mut assigned = workers
            .iter()
            .flatten()
            .map(|agent| agent.id)
            .collect::<HashSet<_>>();
        for worker in workers.iter_mut().filter(|worker| worker.is_none()) {
            *worker = candidates
                .iter()
                .copied()
                .find(|agent| assigned.insert(agent.id));
        }
        let [Some(visual), Some(gameplay), Some(quality)] = workers else {
            return Err(anyhow!(
                "studio-swarm requires three distinct non-retired github-copilot workers"
            ));
        };
        let workers = [visual, gameplay, quality];
        let handoff_root = request.handoff_root.unwrap_or("handoffs");
        let total_budget = request.budget_tokens.unwrap_or(2_000_000);
        let total_cost_budget = request.budget_cost_microusd.unwrap_or(6_000_000);
        let (specialist_budget, integration_budget) = studio_budget_split(total_budget)?;
        let (specialist_cost_budget, integration_cost_budget) =
            studio_budget_split(total_cost_budget)?;
        let mut tasks = Vec::with_capacity(4);
        for ((key, focus, expected_output, acceptance), agent) in roles.into_iter().zip(&workers) {
            let path = studio_handoff_path(handoff_root, key)?;
            let mut task_contract = contract(
                format!(
                    "Produce only the {focus} technical handoff for this mission: {}\n\
                     Write exactly one concise UTF-8 Markdown handoff at {path}, at most 12 KiB \
                     (12288 bytes). Use only Copilot native file tools to read relevant files and \
                     create or edit the handoff, including scoped directory creation if needed. \
                     No shell commands are needed or allowed. Do not inspect unrelated repository \
                     files, implement the final product, modify other files, or create commits. \
                     The integration task receives the verified handoff contents, not your \
                     conversational context or access to your worktree.",
                    request.mission_title
                ),
                &format!("{path}: {expected_output}; a self-contained technical Markdown handoff"),
                specialist_budget,
            );
            task_contract.budget_cost_microusd = specialist_cost_budget;
            task_contract.model = request.preferred_model.map(str::to_owned);
            task_contract.reasoning_effort = request.reasoning_effort.map(str::to_owned);
            task_contract.allowed_tools = vec!["filesystem".to_owned()];
            task_contract.prohibited_actions.extend([
                "execute shell commands".to_owned(),
                "modify files outside the exact handoff write scope".to_owned(),
                "implement the final deliverable before integration".to_owned(),
                "create commits, publish, merge, or deploy".to_owned(),
            ]);
            task_contract.write_scope = vec![path.clone()];
            task_contract.acceptance_tests.extend([
                acceptance.to_owned(),
                "The handoff is non-empty UTF-8 Markdown, at most 12 KiB (12288 bytes), with concrete decisions, constraints, and verification guidance rather than a transcript.".to_owned(),
                "Only the exact declared handoff file is changed and exported; no implementation files or commits are produced.".to_owned(),
            ]);
            task_contract.deliverable = Some(DeliverableSpec {
                form: DeliverableForm::TypedArtifactSet,
                commit_after_verification: false,
                paths: vec![path.clone()],
            });
            tasks.push(PlannedTask {
                key: key.to_owned(),
                title: format!("{} {focus} pass", agent.name),
                contract: task_contract,
                assigned_agent_id: agent.id,
                required_adapter: agent.adapter.clone(),
                depends_on: Vec::new(),
                depth: 0,
                max_attempts: 2,
                verification_policy: VerificationPolicy {
                    checks: vec![
                        VerifierCheck::File { path: path.clone(), min_bytes: 1 },
                        VerifierCheck::Artifact { min_bytes: 1 },
                        VerifierCheck::Command {
                            program: "node".to_owned(),
                            args: vec![
                                "-e".to_owned(),
                                "const b=require('node:fs').readFileSync(process.argv[1]);new TextDecoder('utf-8',{fatal:true}).decode(b);if(b.length>12288)process.exit(1)".to_owned(),
                                path,
                            ],
                            timeout_ms: 5_000,
                        },
                    ],
                    manual_gate: None,
                },
            });
        }

        let mut integration_contract = contract(
            format!(
                "After all three studio specialist tasks complete, consume every verified handoff \
                 and integrate the final repository deliverable for this mission: {}\n\
                 Apply the visual-direction, gameplay-systems, and quality-verification decisions \
                 and acceptance checks. If any verified handoff content is missing or unusable, \
                 escalate rather than guessing from task names or reading sibling worktrees.",
                request.mission_title
            ),
            "The requested final repository deliverable, incorporating all three technical handoffs and concrete verification evidence",
            integration_budget,
        );
        integration_contract.budget_cost_microusd = integration_cost_budget;
        integration_contract.model = request.preferred_model.map(str::to_owned);
        integration_contract.reasoning_effort = request.reasoning_effort.map(str::to_owned);
        integration_contract.references = tasks
            .iter()
            .map(|task| format!("task:{}", task.key))
            .collect();
        integration_contract.acceptance_tests.extend([
            "Consume all three verified handoffs and explain how their decisions and acceptance checks are reflected in the final deliverable.".to_owned(),
            "Produce the requested final deliverable and report concrete verification results, including any unresolved limitations.".to_owned(),
        ]);
        integration_contract.deliverable = request.deliverable.cloned();
        let dependencies = tasks.iter().map(|task| task.key.clone()).collect();
        tasks.push(PlannedTask {
            key: "studio-integration".to_owned(),
            title: format!("{} studio integration", gameplay.name),
            contract: integration_contract,
            assigned_agent_id: gameplay.id,
            required_adapter: gameplay.adapter.clone(),
            depends_on: dependencies,
            depth: 1,
            max_attempts: 2,
            verification_policy: artifact_policy(),
        });
        Ok(TaskGraphPlan {
            strategy: self.id().to_owned(),
            max_nodes: 4,
            max_depth: 1,
            budget_tokens: total_budget,
            budget_cost_microusd: total_cost_budget,
            staffing: Vec::new(),
            tasks,
        })
    }
}

fn studio_budget_split(total: i64) -> Result<(i64, i64)> {
    if total < 4 {
        return Err(anyhow!("studio-swarm budget must fund all four tasks"));
    }
    let specialist = (total
        .checked_mul(3)
        .context("studio-swarm budget overflow")?
        / 20)
        .max(1);
    // Assign rounding remainder to integration so the four budgets sum to the request exactly.
    Ok((specialist, total - specialist * 3))
}

fn studio_handoff_path(root: &str, role: &str) -> Result<String> {
    let path = format!("{root}/{role}.md");
    crate::dependency_source::validate_typed_source_paths(std::slice::from_ref(&path))
        .context("studio-swarm handoff root cannot be decoded safely")?;
    if !repository_relative_path_is_valid(root)
        || !write_scope_is_valid(&path)
        || root.split('/').any(|component| {
            let stem = component
                .split('.')
                .next()
                .unwrap_or_default()
                .to_ascii_uppercase();
            component.eq_ignore_ascii_case(".git")
                || component.ends_with([' ', '.'])
                || component
                    .chars()
                    .any(|character| matches!(character, '<' | '>' | '|' | '"'))
                || matches!(
                    stem.as_str(),
                    "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
                )
                || stem
                    .strip_prefix("COM")
                    .or_else(|| stem.strip_prefix("LPT"))
                    .is_some_and(|suffix| {
                        matches!(
                            suffix,
                            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                        )
                    })
        })
    {
        return Err(anyhow!(
            "studio-swarm handoff root must be a safe literal repository-relative directory"
        ));
    }
    Ok(path)
}

struct VerificationMatrixStrategy;

impl ManagerStrategy for VerificationMatrixStrategy {
    fn id(&self) -> &'static str {
        "verification-matrix"
    }

    fn plan(&self, request: &PlanningRequest<'_>, agents: &[Agent]) -> Result<TaskGraphPlan> {
        verification_plan(
            self.id(),
            request,
            agents,
            "[verification-matrix] Produce every automated verification fixture.",
            VerificationPolicy {
                checks: vec![
                    VerifierCheck::Artifact { min_bytes: 1 },
                    VerifierCheck::File {
                        path: "verify.txt".to_owned(),
                        min_bytes: 9,
                    },
                    VerifierCheck::Command {
                        program: "node".to_owned(),
                        args: vec![
                            "-e".to_owned(),
                            "const fs=require('fs');process.exit(fs.existsSync('verify.txt')?0:1)"
                                .to_owned(),
                        ],
                        timeout_ms: 5_000,
                    },
                    VerifierCheck::Test {
                        program: "node".to_owned(),
                        args: vec![
                            "-e".to_owned(),
                            "const fs=require('fs');process.exit(fs.readFileSync('verify.txt','utf8').trim()==='VERIFIED'?0:1)"
                                .to_owned(),
                        ],
                        timeout_ms: 5_000,
                    },
                    VerifierCheck::JsonSchema {
                        path: "schema.json".to_owned(),
                        required_keys: vec!["status".to_owned(), "count".to_owned()],
                    },
                    VerifierCheck::Screenshot {
                        path: "screenshot.png".to_owned(),
                        min_bytes: 16,
                    },
                ],
                manual_gate: None,
            },
        )
    }
}

struct VerificationFailureStrategy;

impl ManagerStrategy for VerificationFailureStrategy {
    fn id(&self) -> &'static str {
        "verification-failure"
    }

    fn plan(&self, request: &PlanningRequest<'_>, agents: &[Agent]) -> Result<TaskGraphPlan> {
        verification_plan(
            self.id(),
            request,
            agents,
            "Produce an artifact that intentionally lacks missing-required.txt.",
            VerificationPolicy {
                checks: vec![
                    VerifierCheck::Artifact { min_bytes: 1 },
                    VerifierCheck::File {
                        path: "missing-required.txt".to_owned(),
                        min_bytes: 1,
                    },
                ],
                manual_gate: None,
            },
        )
    }
}

struct HumanApprovalStrategy;

impl ManagerStrategy for HumanApprovalStrategy {
    fn id(&self) -> &'static str {
        "human-approval"
    }

    fn plan(&self, request: &PlanningRequest<'_>, agents: &[Agent]) -> Result<TaskGraphPlan> {
        verification_plan(
            self.id(),
            request,
            agents,
            "Produce an artifact and wait for an authorized human approval.",
            VerificationPolicy {
                checks: vec![VerifierCheck::Artifact { min_bytes: 1 }],
                manual_gate: Some(ManualVerificationGate::HumanApproval {
                    roles: vec!["owner".to_owned(), "admin".to_owned(), "manager".to_owned()],
                }),
            },
        )
    }
}

struct IndependentReviewStrategy;

impl ManagerStrategy for IndependentReviewStrategy {
    fn id(&self) -> &'static str {
        "independent-review"
    }

    fn plan(&self, request: &PlanningRequest<'_>, agents: &[Agent]) -> Result<TaskGraphPlan> {
        verification_plan(
            self.id(),
            request,
            agents,
            "Produce an artifact and wait for an independent reviewer.",
            VerificationPolicy {
                checks: vec![VerifierCheck::Artifact { min_bytes: 1 }],
                manual_gate: Some(ManualVerificationGate::IndependentReview {
                    roles: vec!["member".to_owned(), "owner".to_owned(), "admin".to_owned()],
                    exclude_requester: true,
                }),
            },
        )
    }
}

fn verification_plan(
    strategy: &str,
    request: &PlanningRequest<'_>,
    agents: &[Agent],
    instruction: &str,
    verification_policy: VerificationPolicy,
) -> Result<TaskGraphPlan> {
    let agent = ordered_candidates(agents)
        .into_iter()
        .find(|agent| agent.role != "manager" && agent.adapter == "fake-process")
        .context("verification strategy requires a fake-process worker")?;
    let budget_tokens = request.budget_tokens.unwrap_or(50_000);
    let budget_cost_microusd = request.budget_cost_microusd.unwrap_or(500_000);
    let mut task_contract = contract(
        format!("{instruction}\nMission: {}", request.mission_title),
        "An artifact and every file required by the verifier policy",
        budget_tokens,
    );
    task_contract.budget_cost_microusd = budget_cost_microusd;
    task_contract.normalize_for_adapter(&agent.adapter);
    task_contract.deliverable = request.deliverable.cloned();
    Ok(TaskGraphPlan {
        strategy: strategy.to_owned(),
        max_nodes: 1,
        max_depth: 0,
        budget_tokens,
        budget_cost_microusd,
        staffing: Vec::new(),
        tasks: vec![PlannedTask {
            key: "verify".to_owned(),
            title: "Produce verifier evidence".to_owned(),
            contract: task_contract,
            assigned_agent_id: agent.id,
            required_adapter: agent.adapter.clone(),
            depends_on: Vec::new(),
            depth: 0,
            max_attempts: 1,
            verification_policy,
        }],
    })
}

fn provider_settings(
    request: &PlanningRequest<'_>,
    adapter: &str,
) -> (Option<String>, Option<String>) {
    if request.preferred_adapter == Some(adapter) {
        (
            request.preferred_model.map(str::to_owned),
            request.reasoning_effort.map(str::to_owned),
        )
    } else {
        (None, None)
    }
}

fn ordered_candidates(agents: &[Agent]) -> Vec<&Agent> {
    let mut candidates = agents
        .iter()
        .filter(|agent| agent.status != AgentStatus::Offline && agent.retired_at.is_none())
        .collect::<Vec<_>>();
    candidates.sort_by_key(|agent| {
        (
            agent.status != AgentStatus::Idle,
            agent.role != "manager",
            agent.adapter != "fake-process",
            agent.adapter.as_str(),
            agent.name.as_str(),
            agent.id,
        )
    });
    candidates
}

fn artifact_policy() -> VerificationPolicy {
    VerificationPolicy {
        checks: vec![VerifierCheck::Artifact { min_bytes: 1 }],
        manual_gate: None,
    }
}

fn contract(objective: String, expected_output: &str, budget_tokens: i64) -> TaskContract {
    TaskContract {
        objective,
        expected_output: expected_output.to_owned(),
        source_repository: None,
        source_base_ref: None,
        source_base_commit: None,
        acceptance_tests: vec![
            "the declared artifact exists".to_owned(),
            "the task reports concrete verification evidence".to_owned(),
        ],
        allowed_tools: vec!["filesystem".to_owned(), "shell".to_owned()],
        prohibited_actions: vec![
            "modify files outside the assigned worktree".to_owned(),
            "use undeclared long-lived credentials".to_owned(),
        ],
        references: Vec::new(),
        write_scope: vec!["**".to_owned()],
        budget_tokens,
        budget_cost_microusd: 1_000_000,
        deadline_at: None,
        escalation: "ask the current human controller or mission owner".to_owned(),
        secret_refs: Vec::new(),
        model: None,
        reasoning_effort: None,
        deliverable: None,
    }
}

pub fn validate_plan(plan: &TaskGraphPlan, agents: &[Agent]) -> Result<()> {
    if plan.tasks.is_empty() || plan.tasks.len() > MAX_GRAPH_NODES {
        return Err(anyhow!(
            "task graph must contain between 1 and {MAX_GRAPH_NODES} nodes"
        ));
    }
    if plan.max_nodes < plan.tasks.len() as i32 || plan.max_nodes > MAX_GRAPH_NODES as i32 {
        return Err(anyhow!("task graph node limit is invalid"));
    }
    if !(0..=MAX_GRAPH_DEPTH).contains(&plan.max_depth) {
        return Err(anyhow!("task graph depth limit is invalid"));
    }
    if !(1..=MAX_GRAPH_BUDGET_TOKENS).contains(&plan.budget_tokens) {
        return Err(anyhow!("task graph budget is invalid"));
    }
    if !(1..=50_000_000).contains(&plan.budget_cost_microusd) {
        return Err(anyhow!("task graph cost budget is invalid"));
    }

    let agents = agents
        .iter()
        .map(|agent| (agent.id, agent))
        .collect::<HashMap<_, _>>();
    let mut keys = HashMap::new();
    for (index, task) in plan.tasks.iter().enumerate() {
        if task.key.is_empty()
            || task.key.len() > 64
            || !task
                .key
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
        {
            return Err(anyhow!("task key is invalid: {}", task.key));
        }
        if keys.insert(task.key.as_str(), index).is_some() {
            return Err(anyhow!("duplicate task key {}", task.key));
        }
        let agent = agents
            .get(&task.assigned_agent_id)
            .with_context(|| format!("task {} references an unknown agent", task.key))?;
        if agent.adapter != task.required_adapter {
            return Err(anyhow!(
                "task {} requires adapter {} but assigned agent uses {}",
                task.key,
                task.required_adapter,
                agent.adapter
            ));
        }
        if task.required_adapter == "fake-process"
            && (task.contract.model.is_some() || task.contract.reasoning_effort.is_some())
        {
            return Err(anyhow!(
                "task {} uses fake-process and cannot require a model or reasoning effort",
                task.key
            ));
        }
        if !(1..=MAX_TASK_ATTEMPTS).contains(&task.max_attempts) {
            return Err(anyhow!("task {} retry limit is invalid", task.key));
        }
        validate_contract(&task.key, &task.contract)?;
        validate_verification_policy(&task.key, &task.verification_policy)?;
    }

    let total_budget = plan.tasks.iter().try_fold(0_i64, |total, task| {
        total
            .checked_add(task.contract.budget_tokens)
            .context("task graph budget overflow")
    })?;
    if total_budget > plan.budget_tokens {
        return Err(anyhow!(
            "task budgets total {total_budget}, above mission budget {}",
            plan.budget_tokens
        ));
    }
    let total_cost_budget = plan.tasks.iter().try_fold(0_i64, |total, task| {
        total
            .checked_add(task.contract.budget_cost_microusd)
            .context("task graph cost budget overflow")
    })?;
    if total_cost_budget > plan.budget_cost_microusd {
        return Err(anyhow!(
            "task cost budgets total {total_cost_budget}, above mission cost budget {}",
            plan.budget_cost_microusd
        ));
    }

    let mut visiting = HashSet::new();
    let mut depths = HashMap::new();
    for task in &plan.tasks {
        let depth = compute_depth(
            task.key.as_str(),
            &plan.tasks,
            &keys,
            &mut visiting,
            &mut depths,
        )?;
        if depth != task.depth {
            return Err(anyhow!(
                "task {} declared depth {} but dependency depth is {depth}",
                task.key,
                task.depth
            ));
        }
        if depth > plan.max_depth || depth > MAX_GRAPH_DEPTH {
            return Err(anyhow!("task {} exceeds graph depth limit", task.key));
        }
    }
    Ok(())
}

fn validate_contract(task_key: &str, contract: &TaskContract) -> Result<()> {
    if contract.objective.trim().is_empty()
        || contract.expected_output.trim().is_empty()
        || contract.escalation.trim().is_empty()
    {
        return Err(anyhow!("task {task_key} has an incomplete contract"));
    }
    if contract.acceptance_tests.is_empty()
        || contract.allowed_tools.is_empty()
        || contract.prohibited_actions.is_empty()
        || contract.write_scope.is_empty()
    {
        return Err(anyhow!("task {task_key} omits required contract lists"));
    }
    if !(1..=MAX_TASK_BUDGET_TOKENS).contains(&contract.budget_tokens) {
        return Err(anyhow!("task {task_key} budget is invalid"));
    }
    if !(1..=10_000_000).contains(&contract.budget_cost_microusd) {
        return Err(anyhow!("task {task_key} cost budget is invalid"));
    }
    validate_source_requirement(
        task_key,
        contract.source_repository.as_deref(),
        contract.source_base_ref.as_deref(),
        contract.source_base_commit.as_deref(),
    )?;
    if let Some(deliverable) = &contract.deliverable {
        validate_deliverable(task_key, deliverable)?;
    }
    if contract
        .model
        .as_ref()
        .is_some_and(|model| model.trim().is_empty() || model.len() > 128)
    {
        return Err(anyhow!("task {task_key} model is invalid"));
    }
    if contract.reasoning_effort.as_ref().is_some_and(|effort| {
        !matches!(
            effort.as_str(),
            "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
        )
    }) {
        return Err(anyhow!("task {task_key} reasoning effort is invalid"));
    }
    for value in contract
        .acceptance_tests
        .iter()
        .chain(&contract.allowed_tools)
        .chain(&contract.prohibited_actions)
        .chain(&contract.references)
        .chain(&contract.write_scope)
    {
        if value.trim().is_empty() || value.len() > 500 {
            return Err(anyhow!(
                "task {task_key} contains an invalid contract entry"
            ));
        }
    }
    if let Some(scope) = contract
        .write_scope
        .iter()
        .find(|scope| !write_scope_is_valid(scope))
    {
        return Err(anyhow!(
            "task {task_key} contains invalid write scope {scope}"
        ));
    }
    let mut environment_names = HashSet::new();
    for secret in &contract.secret_refs {
        if secret.env_name.is_empty()
            || secret.env_name.len() > 128
            || !secret.env_name.chars().all(|character| {
                character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
            })
            || secret
                .env_name
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_digit())
        {
            return Err(anyhow!(
                "task {task_key} secret environment name is invalid"
            ));
        }
        if !environment_names.insert(secret.env_name.as_str()) {
            return Err(anyhow!(
                "task {task_key} repeats secret environment name {}",
                secret.env_name
            ));
        }
        if secret.tool.trim().is_empty()
            || secret.tool.len() > 64
            || secret.resource.trim().is_empty()
            || secret.resource.len() > 512
        {
            return Err(anyhow!("task {task_key} secret scope is invalid"));
        }
    }
    Ok(())
}

fn validate_deliverable(task_key: &str, deliverable: &DeliverableSpec) -> Result<()> {
    if deliverable.paths.len() > 128 {
        return Err(anyhow!(
            "task {task_key} deliverable cannot contain more than 128 paths"
        ));
    }
    for path in &deliverable.paths {
        if !repository_relative_path_is_valid(path) {
            return Err(anyhow!(
                "task {task_key} deliverable path must be a literal repository-relative path"
            ));
        }
    }
    Ok(())
}

fn validate_source_requirement(
    task_key: &str,
    repository: Option<&str>,
    base_ref: Option<&str>,
    base_commit: Option<&str>,
) -> Result<()> {
    let (Some(repository), Some(base_ref), Some(base_commit)) = (repository, base_ref, base_commit)
    else {
        if repository.is_some() || base_ref.is_some() || base_commit.is_some() {
            return Err(anyhow!(
                "task {task_key} must specify source repository, base ref, and immutable base commit together"
            ));
        }
        return Ok(());
    };
    let mut repository_parts = repository.split('/');
    let owner = repository_parts.next().unwrap_or_default();
    let name = repository_parts.next().unwrap_or_default();
    let valid_component = |value: &str| {
        !value.is_empty()
            && value.len() <= 100
            && value.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
            })
    };
    if !valid_component(owner)
        || !valid_component(name)
        || repository_parts.next().is_some()
        || base_ref.is_empty()
        || base_ref.len() > 240
        || base_ref.starts_with('-')
        || base_ref.starts_with('/')
        || base_ref.ends_with('/')
        || base_ref.ends_with('.')
        || base_ref.contains("..")
        || base_ref.contains("@{")
        || base_ref
            .chars()
            .any(|character| matches!(character, '\\' | ' ' | '~' | '^' | ':' | '?' | '*' | '['))
        || !matches!(base_commit.len(), 40 | 64)
        || !base_commit.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(anyhow!(
            "task {task_key} contains an invalid source repository requirement"
        ));
    }
    Ok(())
}

fn validate_verification_policy(task_key: &str, policy: &VerificationPolicy) -> Result<()> {
    if policy.checks.is_empty() || policy.checks.len() > 16 {
        return Err(anyhow!(
            "task {task_key} verifier must contain between 1 and 16 checks"
        ));
    }
    for check in &policy.checks {
        match check {
            VerifierCheck::Artifact { min_bytes } => {
                if *min_bytes == 0 {
                    return Err(anyhow!("task {task_key} artifact check has no byte floor"));
                }
            }
            VerifierCheck::File { path, min_bytes }
            | VerifierCheck::Screenshot { path, min_bytes } => {
                validate_relative_path(task_key, path)?;
                if *min_bytes == 0 {
                    return Err(anyhow!("task {task_key} file check has no byte floor"));
                }
            }
            VerifierCheck::JsonSchema {
                path,
                required_keys,
            } => {
                validate_relative_path(task_key, path)?;
                if required_keys.is_empty()
                    || required_keys.len() > 32
                    || required_keys
                        .iter()
                        .any(|key| key.trim().is_empty() || key.len() > 128)
                {
                    return Err(anyhow!("task {task_key} JSON schema check is invalid"));
                }
            }
            VerifierCheck::Command {
                program,
                args,
                timeout_ms,
            }
            | VerifierCheck::Test {
                program,
                args,
                timeout_ms,
            } => {
                if program.trim().is_empty()
                    || program.len() > 256
                    || program.starts_with('-')
                    || args.len() > 32
                    || args.iter().any(|arg| arg.len() > 2_000)
                    || !(100..=60_000).contains(timeout_ms)
                {
                    return Err(anyhow!("task {task_key} command verifier is invalid"));
                }
            }
        }
    }
    if let Some(gate) = &policy.manual_gate {
        let roles = match gate {
            ManualVerificationGate::HumanApproval { roles }
            | ManualVerificationGate::IndependentReview { roles, .. } => roles,
        };
        if roles.is_empty()
            || roles.len() > 16
            || roles
                .iter()
                .any(|role| !matches!(role.as_str(), "owner" | "admin" | "manager" | "member"))
        {
            return Err(anyhow!(
                "task {task_key} manual verification gate is invalid"
            ));
        }
    }
    Ok(())
}

fn validate_relative_path(task_key: &str, value: &str) -> Result<()> {
    if !repository_relative_path_is_valid(value) {
        return Err(anyhow!(
            "task {task_key} verifier path must stay inside the worktree"
        ));
    }
    Ok(())
}

fn compute_depth<'a>(
    key: &'a str,
    tasks: &'a [PlannedTask],
    keys: &HashMap<&'a str, usize>,
    visiting: &mut HashSet<&'a str>,
    depths: &mut HashMap<&'a str, i32>,
) -> Result<i32> {
    if let Some(depth) = depths.get(key) {
        return Ok(*depth);
    }
    if !visiting.insert(key) {
        return Err(anyhow!("task graph contains a dependency cycle at {key}"));
    }
    let task = keys
        .get(key)
        .and_then(|index| tasks.get(*index))
        .with_context(|| format!("unknown task dependency {key}"))?;
    let mut depth = 0;
    for dependency in &task.depends_on {
        if !keys.contains_key(dependency.as_str()) {
            return Err(anyhow!(
                "task {} depends on unknown task {dependency}",
                task.key
            ));
        }
        depth = depth.max(
            compute_depth(dependency, tasks, keys, visiting, depths)?
                .checked_add(1)
                .context("task depth overflow")?,
        );
    }
    visiting.remove(key);
    depths.insert(key, depth);
    Ok(depth)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use uuid::Uuid;

    use super::*;

    fn agent(name: &str, role: &str, adapter: &str) -> Agent {
        Agent {
            id: Uuid::new_v4(),
            corp_id: Uuid::new_v4(),
            actor_id: Uuid::new_v4(),
            name: name.to_owned(),
            role: role.to_owned(),
            adapter: adapter.to_owned(),
            status: AgentStatus::Idle,
            station: None,
            current_run_id: None,
            accent: "test".to_owned(),
            created_at: Utc::now(),
            mission_id: None,
            pinned: false,
            retired_at: None,
        }
    }

    fn agents() -> Vec<Agent> {
        vec![
            agent("Margo", "manager", "fake-process"),
            agent("Wally", "engineer", "fake-process"),
            agent("Cody", "engineer", "codex"),
        ]
    }

    fn copilot_workers() -> Vec<Agent> {
        (1..=3)
            .map(|index| agent(&format!("Studio {index}"), "engineer", "github-copilot"))
            .collect()
    }

    fn studio_request<'a>() -> PlanningRequest<'a> {
        PlanningRequest {
            mission_title: "deliver the bounded repository outcome",
            preferred_adapter: None,
            preferred_model: None,
            reasoning_effort: None,
            secret_refs: &[],
            budget_tokens: None,
            budget_cost_microusd: None,
            deliverable: None,
            handoff_root: None,
        }
    }

    #[test]
    fn studio_swarm_uses_three_distinct_copilot_roots_and_gameplay_join() {
        let registry = StrategyRegistry::new();
        let workers = copilot_workers();
        let mut roster = agents();
        roster.extend(workers.clone());
        roster.push(agent("00 Manager", "manager", "github-copilot"));
        let mut retired = agent("00 Retired", "engineer", "github-copilot");
        retired.retired_at = Some(Utc::now());
        roster.push(retired);
        let mut offline = agent("00 Offline", "engineer", "github-copilot");
        offline.status = AgentStatus::Offline;
        roster.push(offline);
        let request = studio_request();
        let plan = registry
            .plan("studio-swarm", &request, &roster)
            .expect("three-Copilot studio plan");

        assert_eq!(plan.strategy, "studio-swarm");
        assert!(!uses_deterministic_harness(&plan.strategy));
        assert_eq!(plan.tasks.len(), 4);
        assert_eq!(plan.max_nodes, 4);
        assert_eq!(plan.max_depth, 1);
        assert!(plan.staffing.is_empty());
        let roots = &plan.tasks[..3];
        let keys = [
            "visual-direction",
            "gameplay-systems",
            "quality-verification",
        ];
        assert_eq!(
            roots
                .iter()
                .map(|task| task.key.as_str())
                .collect::<Vec<_>>(),
            keys
        );
        assert_eq!(
            roots
                .iter()
                .map(|task| task.assigned_agent_id)
                .collect::<HashSet<_>>()
                .len(),
            3
        );
        for root in roots {
            assert_eq!(root.depth, 0);
            assert!(root.depends_on.is_empty());
            assert!(
                workers
                    .iter()
                    .any(|worker| worker.id == root.assigned_agent_id)
            );
        }
        assert!(
            plan.tasks
                .iter()
                .all(|task| task.required_adapter == "github-copilot")
        );
        let join = &plan.tasks[3];
        assert_eq!(join.key, "studio-integration");
        assert_eq!(join.depth, 1);
        assert_eq!(join.assigned_agent_id, roots[1].assigned_agent_id);
        assert_eq!(join.depends_on, keys.map(str::to_owned));
        assert_eq!(
            join.contract.references,
            keys.map(|key| format!("task:{key}"))
        );
        assert!(join.contract.deliverable.is_none());

        roster.reverse();
        let reordered = registry
            .plan("studio-swarm", &request, &roster)
            .expect("deterministic reordered plan");
        assert_eq!(
            plan.tasks
                .iter()
                .map(|task| (&task.key, task.assigned_agent_id))
                .collect::<Vec<_>>(),
            reordered
                .tasks
                .iter()
                .map(|task| (&task.key, task.assigned_agent_id))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn studio_swarm_prefers_staffing_roles_and_reserves_them_before_fallback() {
        let registry = StrategyRegistry::new();
        let workers = vec![
            agent("Visual designer", "visual-direction", "github-copilot"),
            agent("Gameplay engineer", "gameplay-systems", "github-copilot"),
            agent("Quality engineer", "quality-verification", "github-copilot"),
        ];
        let mut roster = workers.clone();
        roster.push(agent("00 Generic engineer", "engineer", "github-copilot"));
        let request = studio_request();
        let plan = registry
            .plan("studio-swarm", &request, &roster)
            .expect("staffing-role plan");
        for (task, worker) in plan.tasks[..3].iter().zip(&workers) {
            assert_eq!(task.key, worker.role);
            assert_eq!(task.assigned_agent_id, worker.id);
        }
        assert_eq!(plan.tasks[3].assigned_agent_id, workers[1].id);

        for generic_index in 0..3 {
            let mut roster = workers.clone();
            roster[generic_index].role = "engineer".to_owned();
            roster[generic_index].name = "ZZ Generic engineer".to_owned();
            let plan = registry
                .plan("studio-swarm", &request, &roster)
                .expect("reserve explicit roles before deterministic fallback");
            for (task, worker) in plan.tasks[..3].iter().zip(&workers) {
                assert_eq!(task.assigned_agent_id, worker.id);
            }
            assert_eq!(plan.tasks[3].assigned_agent_id, workers[1].id);
        }
    }

    #[test]
    fn studio_swarm_scopes_role_handoffs_and_preserves_final_deliverable() {
        let registry = StrategyRegistry::new();
        let workers = copilot_workers();
        let requested = DeliverableSpec {
            form: DeliverableForm::Archive,
            commit_after_verification: true,
            paths: vec!["src".to_owned(), "tests".to_owned()],
        };
        let roles = [
            ("visual-direction", "accessibility", "responsive layouts"),
            (
                "gameplay-systems",
                "state transitions",
                "deterministic rules",
            ),
            (
                "quality-verification",
                "browser acceptance",
                "reproducible test inputs",
            ),
        ];
        for handoff_root in [None, Some("docs/team handoffs")] {
            let request = PlanningRequest {
                handoff_root,
                deliverable: Some(&requested),
                ..studio_request()
            };
            let plan = registry
                .plan("studio-swarm", &request, &workers)
                .expect("scoped handoff plan");
            for (task, (key, expected, acceptance)) in plan.tasks[..3].iter().zip(roles) {
                let path = format!("{}/{key}.md", handoff_root.unwrap_or("handoffs"));
                assert_eq!(task.key, key);
                assert_eq!(task.contract.write_scope, vec![path.clone()]);
                assert_eq!(task.contract.allowed_tools, vec!["filesystem"]);
                assert!(task.contract.objective.contains(&path));
                assert!(task.contract.objective.contains("native file tools"));
                assert!(task.contract.expected_output.contains(expected));
                assert!(
                    task.contract
                        .acceptance_tests
                        .iter()
                        .any(|test| test.contains(acceptance))
                );
                assert!(
                    task.contract
                        .acceptance_tests
                        .iter()
                        .any(|test| test.contains("12288 bytes"))
                );
                assert!(
                    task.contract
                        .prohibited_actions
                        .iter()
                        .any(|action| action == "execute shell commands")
                );
                assert_eq!(
                    task.contract.deliverable,
                    Some(DeliverableSpec {
                        form: DeliverableForm::TypedArtifactSet,
                        commit_after_verification: false,
                        paths: vec![path.clone()],
                    })
                );
                assert_eq!(
                    task.verification_policy,
                    VerificationPolicy {
                        checks: vec![
                            VerifierCheck::File { path: path.clone(), min_bytes: 1 },
                            VerifierCheck::Artifact { min_bytes: 1 },
                            VerifierCheck::Command {
                                program: "node".to_owned(),
                                args: vec![
                                    "-e".to_owned(),
                                    "const b=require('node:fs').readFileSync(process.argv[1]);new TextDecoder('utf-8',{fatal:true}).decode(b);if(b.length>12288)process.exit(1)".to_owned(),
                                    path,
                                ],
                                timeout_ms: 5_000,
                            },
                        ],
                        manual_gate: None,
                    }
                );
            }
            assert_eq!(
                plan.tasks[3].contract.deliverable.as_ref(),
                Some(&requested)
            );
        }
    }

    #[test]
    fn studio_swarm_pins_model_and_reasoning_on_every_task() {
        let registry = StrategyRegistry::new();
        let workers = copilot_workers();
        for preferred_adapter in [None, Some("github-copilot")] {
            let request = PlanningRequest {
                preferred_adapter,
                preferred_model: Some("gpt-5.6-sol"),
                reasoning_effort: Some("high"),
                ..studio_request()
            };
            let plan = registry
                .plan("studio-swarm", &request, &workers)
                .expect("model-pinned studio plan");
            for task in &plan.tasks {
                assert_eq!(task.required_adapter, "github-copilot");
                assert_eq!(task.contract.model.as_deref(), Some("gpt-5.6-sol"));
                assert_eq!(task.contract.reasoning_effort.as_deref(), Some("high"));
            }
        }
    }

    #[test]
    fn studio_swarm_splits_budgets_and_bounds_attempts() {
        let registry = StrategyRegistry::new();
        let workers = copilot_workers();
        for (budget_tokens, budget_cost_microusd, expected_tokens, expected_cost) in [
            (
                None,
                None,
                [300_000, 300_000, 300_000, 1_100_000],
                [900_000, 900_000, 900_000, 3_300_000],
            ),
            (
                Some(100_003),
                Some(1_000_003),
                [15_000, 15_000, 15_000, 55_003],
                [150_000, 150_000, 150_000, 550_003],
            ),
            (Some(4), Some(4), [1, 1, 1, 1], [1, 1, 1, 1]),
        ] {
            let request = PlanningRequest {
                budget_tokens,
                budget_cost_microusd,
                ..studio_request()
            };
            let plan = registry
                .plan("studio-swarm", &request, &workers)
                .expect("bounded studio budgets");
            assert_eq!(plan.budget_tokens, expected_tokens.iter().sum::<i64>());
            assert_eq!(plan.budget_cost_microusd, expected_cost.iter().sum::<i64>());
            for (index, task) in plan.tasks.iter().enumerate() {
                assert_eq!(task.contract.budget_tokens, expected_tokens[index]);
                assert_eq!(task.contract.budget_cost_microusd, expected_cost[index]);
                assert_eq!(task.max_attempts, 2);
            }
        }
    }

    #[test]
    fn studio_swarm_rejects_insufficient_or_mixed_provider_workers() {
        let registry = StrategyRegistry::new();
        let workers = copilot_workers();
        for count in 0..3 {
            for preferred_adapter in [None, Some("github-copilot")] {
                let request = PlanningRequest {
                    preferred_adapter,
                    ..studio_request()
                };
                let mut roster = workers[..count].to_vec();
                let error = registry
                    .plan("studio-swarm", &request, &roster)
                    .expect_err("fewer than three Copilot workers must fail");
                assert!(error.to_string().contains("three distinct"));
                roster.extend(agents());
                roster.push(agent("Claude", "engineer", "claude-code"));
                roster.push(agent("Copilot manager", "manager", "github-copilot"));
                assert!(
                    registry.plan("studio-swarm", &request, &roster).is_err(),
                    "must not replace missing Copilot workers with another provider or a manager"
                );
            }
        }
    }

    #[test]
    fn studio_swarm_rejects_duplicate_manager_retired_or_offline_workers() {
        let registry = StrategyRegistry::new();
        let workers = copilot_workers();
        let request = studio_request();
        for replacement in [
            workers[0].clone(),
            Agent {
                role: "manager".to_owned(),
                ..workers[2].clone()
            },
            Agent {
                retired_at: Some(Utc::now()),
                pinned: true,
                ..workers[2].clone()
            },
            Agent {
                status: AgentStatus::Offline,
                ..workers[2].clone()
            },
        ] {
            let mut roster = workers.clone();
            roster[2] = replacement;
            assert!(registry.plan("studio-swarm", &request, &roster).is_err());
        }
    }

    #[test]
    fn studio_swarm_rejects_non_copilot_adapter_requests() {
        let registry = StrategyRegistry::new();
        let mut roster = copilot_workers();
        roster.extend(agents());
        roster.push(agent("Claude", "engineer", "claude-code"));
        for adapter in ["codex", "claude-code", "fake-process", "", "GitHub-Copilot"] {
            let request = PlanningRequest {
                preferred_adapter: Some(adapter),
                ..studio_request()
            };
            let error = registry
                .plan("studio-swarm", &request, &roster)
                .expect_err("explicit adapter must be github-copilot");
            assert!(error.to_string().contains("requires github-copilot"));
        }
    }

    #[test]
    fn studio_swarm_rejects_unsafe_handoff_roots() {
        let registry = StrategyRegistry::new();
        let workers = copilot_workers();
        let too_long = "a".repeat(500);
        for root in [
            "",
            ".",
            "..",
            "../handoffs",
            "handoffs/../outside",
            "/handoffs",
            "C:/handoffs",
            r"C:\handoffs",
            r"\\server\share\handoffs",
            r"handoffs\notes",
            "handoffs//notes",
            "handoffs/./notes",
            "handoffs/",
            "**",
            "handoffs/**",
            "handoffs/*",
            "handoffs/?",
            "handoffs/[notes]",
            ":(exclude)handoffs",
            ".git",
            "nested/.GiT/hooks",
            "nested/.git./hooks",
            "nested /handoffs",
            "handoffs/NUL",
            "con.txt",
            "nested/COM1",
            "lpt9/handoffs",
            "COM¹/notes",
            "handoffs|outside",
            "hand<offs",
            "hand\"offs",
            " handoffs",
            "handoffs\nnotes",
            too_long.as_str(),
        ] {
            let request = PlanningRequest {
                handoff_root: Some(root),
                ..studio_request()
            };
            let error = registry
                .plan("studio-swarm", &request, &workers)
                .expect_err("unsafe handoff root must fail");
            assert!(
                error.to_string().contains("handoff root"),
                "{root:?}: {error}"
            );
        }
    }

    #[test]
    fn studio_swarm_preserves_graph_and_budget_bounds() {
        let registry = StrategyRegistry::new();
        let workers = copilot_workers();
        for (budget_tokens, budget_cost_microusd) in [
            (Some(-1), None),
            (Some(0), None),
            (Some(1), None),
            (Some(2), None),
            (Some(3), None),
            (Some(MAX_GRAPH_BUDGET_TOKENS + 1), None),
            (Some(i64::MAX), None),
            (None, Some(-1)),
            (None, Some(0)),
            (None, Some(1)),
            (None, Some(2)),
            (None, Some(3)),
            (None, Some(20_000_000)),
            (None, Some(50_000_001)),
            (None, Some(i64::MAX)),
        ] {
            let request = PlanningRequest {
                budget_tokens,
                budget_cost_microusd,
                ..studio_request()
            };
            assert!(registry.plan("studio-swarm", &request, &workers).is_err());
        }

        let plan = registry
            .plan("studio-swarm", &studio_request(), &workers)
            .expect("valid studio graph");
        let mut invalid = plan.clone();
        invalid.max_nodes = 3;
        assert!(validate_plan(&invalid, &workers).is_err());
        let mut invalid = plan.clone();
        invalid.max_depth = 0;
        assert!(validate_plan(&invalid, &workers).is_err());
        let mut invalid = plan.clone();
        invalid.tasks[3].max_attempts = MAX_TASK_ATTEMPTS + 1;
        assert!(validate_plan(&invalid, &workers).is_err());
        let mut invalid = plan.clone();
        invalid.tasks[0]
            .depends_on
            .push("studio-integration".to_owned());
        assert!(validate_plan(&invalid, &workers).is_err());
        let mut invalid = plan.clone();
        invalid
            .tasks
            .resize(MAX_GRAPH_NODES + 1, plan.tasks[0].clone());
        assert!(validate_plan(&invalid, &workers).is_err());
    }

    #[test]
    fn ordered_candidates_exclude_retired_agents() {
        let mut roster = agents();
        roster[1].retired_at = Some(Utc::now());
        roster[1].pinned = true;
        let candidates = ordered_candidates(&roster);
        assert_eq!(candidates.len(), 2);
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.id != roster[1].id)
        );
        let request = PlanningRequest {
            preferred_adapter: Some("fake-process"),
            ..studio_request()
        };
        for strategy in ["single", "parallel-specialists", "verification-matrix"] {
            assert!(
                StrategyRegistry::new()
                    .plan(strategy, &request, &roster)
                    .is_err()
            );
        }
    }

    #[test]
    fn strategies_are_replaceable_and_deterministic() {
        let registry = StrategyRegistry::new();
        assert_eq!(
            registry.ids(),
            vec![
                "human-approval".to_owned(),
                "independent-review".to_owned(),
                "parallel-specialists".to_owned(),
                "single".to_owned(),
                "studio-swarm".to_owned(),
                "verification-failure".to_owned(),
                "verification-matrix".to_owned(),
            ]
        );
        let request = PlanningRequest {
            mission_title: "ship the bounded graph",
            preferred_adapter: Some("codex"),
            preferred_model: None,
            reasoning_effort: None,
            secret_refs: &[],
            budget_tokens: None,
            budget_cost_microusd: None,
            deliverable: None,
            handoff_root: None,
        };
        let agents = agents();
        let first = registry
            .plan("parallel-specialists", &request, &agents)
            .expect("first plan");
        let second = registry
            .plan("parallel-specialists", &request, &agents)
            .expect("second plan");
        assert_eq!(
            first
                .tasks
                .iter()
                .map(|task| (&task.key, task.required_adapter.as_str()))
                .collect::<Vec<_>>(),
            second
                .tasks
                .iter()
                .map(|task| (&task.key, task.required_adapter.as_str()))
                .collect::<Vec<_>>()
        );
        assert_eq!(first.tasks[0].required_adapter, "codex");
        assert_eq!(first.tasks[2].depends_on.len(), 2);
    }

    #[test]
    fn rejects_cycles_depth_budget_and_retry_violations() {
        let agents = agents();
        let registry = StrategyRegistry::new();
        let request = PlanningRequest {
            mission_title: "bounded plan",
            preferred_adapter: None,
            preferred_model: None,
            reasoning_effort: None,
            secret_refs: &[],
            budget_tokens: None,
            budget_cost_microusd: None,
            deliverable: None,
            handoff_root: None,
        };
        let mut plan = registry
            .plan("parallel-specialists", &request, &agents)
            .expect("valid plan");
        plan.tasks[0].depends_on.push("synthesis".to_owned());
        assert!(validate_plan(&plan, &agents).is_err());

        let mut plan = registry
            .plan("parallel-specialists", &request, &agents)
            .expect("valid plan");
        plan.tasks[2].max_attempts = MAX_TASK_ATTEMPTS + 1;
        assert!(validate_plan(&plan, &agents).is_err());

        let mut plan = registry
            .plan("parallel-specialists", &request, &agents)
            .expect("valid plan");
        plan.tasks[2].contract.budget_tokens = MAX_TASK_BUDGET_TOKENS + 1;
        assert!(validate_plan(&plan, &agents).is_err());

        let mut plan = registry
            .plan("parallel-specialists", &request, &agents)
            .expect("valid plan");
        plan.tasks[2].depth = MAX_GRAPH_DEPTH + 1;
        assert!(validate_plan(&plan, &agents).is_err());

        let mut plan = registry
            .plan("parallel-specialists", &request, &agents)
            .expect("valid plan");
        plan.tasks[1].required_adapter = "missing-adapter".to_owned();
        assert!(validate_plan(&plan, &agents).is_err());

        let mut plan = registry
            .plan("parallel-specialists", &request, &agents)
            .expect("valid plan");
        let duplicate_key = plan.tasks[0].key.clone();
        plan.tasks[1].key = duplicate_key;
        assert!(validate_plan(&plan, &agents).is_err());

        let mut plan = registry
            .plan("parallel-specialists", &request, &agents)
            .expect("valid plan");
        plan.tasks[2].depends_on.push("missing".to_owned());
        assert!(validate_plan(&plan, &agents).is_err());

        let mut plan = registry
            .plan("parallel-specialists", &request, &agents)
            .expect("valid plan");
        plan.tasks[0].contract.acceptance_tests.clear();
        assert!(validate_plan(&plan, &agents).is_err());

        let mut plan = registry
            .plan("parallel-specialists", &request, &agents)
            .expect("valid plan");
        plan.budget_tokens = 1;
        assert!(validate_plan(&plan, &agents).is_err());

        assert!(registry.plan("missing", &request, &agents).is_err());
        assert!(registry.plan("single", &request, &[]).is_err());
    }

    #[test]
    fn single_strategy_enforces_requested_adapter() {
        let agents = agents();
        let registry = StrategyRegistry::new();
        let plan = registry
            .plan(
                "single",
                &PlanningRequest {
                    mission_title: "use Codex",
                    preferred_adapter: Some("codex"),
                    preferred_model: None,
                    reasoning_effort: None,
                    secret_refs: &[],
                    budget_tokens: None,
                    budget_cost_microusd: None,
                    deliverable: None,
                    handoff_root: None,
                },
                &agents,
            )
            .expect("single plan");
        assert_eq!(plan.tasks[0].required_adapter, "codex");
        assert_eq!(
            plan.tasks[0].contract.budget_tokens,
            DEFAULT_SINGLE_TASK_BUDGET_TOKENS
        );
    }

    #[test]
    fn single_strategy_preserves_typed_deliverable_and_rejects_unsafe_scope() {
        let agents = agents();
        let registry = StrategyRegistry::new();
        let requested = DeliverableSpec {
            form: crony_domain::DeliverableForm::Archive,
            commit_after_verification: true,
            paths: vec!["src".to_owned()],
        };
        let plan = registry
            .plan(
                "single",
                &PlanningRequest {
                    mission_title: "export the verified application",
                    preferred_adapter: Some("fake-process"),
                    preferred_model: None,
                    reasoning_effort: None,
                    secret_refs: &[],
                    budget_tokens: None,
                    budget_cost_microusd: None,
                    deliverable: Some(&requested),
                    handoff_root: None,
                },
                &agents,
            )
            .expect("single plan");
        assert_eq!(plan.tasks[0].contract.deliverable, Some(requested));

        let unsafe_requested = DeliverableSpec {
            paths: vec!["../escape".to_owned()],
            ..DeliverableSpec::default()
        };
        assert!(
            registry
                .plan(
                    "single",
                    &PlanningRequest {
                        mission_title: "reject unsafe export scope",
                        preferred_adapter: Some("fake-process"),
                        preferred_model: None,
                        reasoning_effort: None,
                        secret_refs: &[],
                        budget_tokens: None,
                        budget_cost_microusd: None,
                        deliverable: Some(&unsafe_requested),
                        handoff_root: None,
                    },
                    &agents,
                )
                .is_err()
        );

        let magic_requested = DeliverableSpec {
            paths: vec![":(exclude)secret.txt".to_owned()],
            ..DeliverableSpec::default()
        };
        assert!(
            registry
                .plan(
                    "single",
                    &PlanningRequest {
                        mission_title: "reject Git pathspec magic",
                        preferred_adapter: Some("fake-process"),
                        preferred_model: None,
                        reasoning_effort: None,
                        secret_refs: &[],
                        budget_tokens: None,
                        budget_cost_microusd: None,
                        deliverable: Some(&magic_requested),
                        handoff_root: None,
                    },
                    &agents,
                )
                .is_err()
        );

        let mut invalid_scope_plan = plan;
        invalid_scope_plan.tasks[0].contract.write_scope = vec!["src/*.rs".to_owned()];
        assert!(validate_plan(&invalid_scope_plan, &agents).is_err());
    }

    #[test]
    fn parallel_strategy_scopes_model_settings_to_matching_adapters() {
        let agents = agents();
        let registry = StrategyRegistry::new();
        let plan = registry
            .plan(
                "parallel-specialists",
                &PlanningRequest {
                    mission_title: "compare two bounded approaches",
                    preferred_adapter: Some("codex"),
                    preferred_model: Some("gpt-5.6-sol"),
                    reasoning_effort: Some("high"),
                    secret_refs: &[],
                    budget_tokens: None,
                    budget_cost_microusd: None,
                    deliverable: None,
                    handoff_root: None,
                },
                &agents,
            )
            .expect("parallel plan");
        let codex = plan
            .tasks
            .iter()
            .find(|task| task.required_adapter == "codex")
            .expect("codex specialist");
        assert_eq!(codex.contract.model.as_deref(), Some("gpt-5.6-sol"));
        assert_eq!(codex.contract.reasoning_effort.as_deref(), Some("high"));
        for task in plan
            .tasks
            .iter()
            .filter(|task| task.required_adapter != "codex")
        {
            assert_eq!(task.contract.model, None);
            assert_eq!(task.contract.reasoning_effort, None);
        }
    }

    #[test]
    fn deterministic_verification_strategy_drops_provider_model_settings() {
        let agents = agents();
        let registry = StrategyRegistry::new();
        let plan = registry
            .plan(
                "verification-matrix",
                &PlanningRequest {
                    mission_title: "verify without a provider",
                    preferred_adapter: Some("github-copilot"),
                    preferred_model: Some("gpt-5.6-sol"),
                    reasoning_effort: Some("max"),
                    secret_refs: &[],
                    budget_tokens: None,
                    budget_cost_microusd: None,
                    deliverable: None,
                    handoff_root: None,
                },
                &agents,
            )
            .expect("verification plan");
        let task = &plan.tasks[0];
        assert_eq!(task.required_adapter, "fake-process");
        assert_eq!(task.contract.model, None);
        assert_eq!(task.contract.reasoning_effort, None);
    }

    #[test]
    fn verifier_policies_reject_unsafe_paths_commands_and_gates() {
        let agents = agents();
        let registry = StrategyRegistry::new();
        let request = PlanningRequest {
            mission_title: "verify safely",
            preferred_adapter: Some("fake-process"),
            preferred_model: None,
            reasoning_effort: None,
            secret_refs: &[],
            budget_tokens: None,
            budget_cost_microusd: None,
            deliverable: None,
            handoff_root: None,
        };

        let mut plan = registry
            .plan("verification-matrix", &request, &agents)
            .expect("valid matrix");
        plan.tasks[0].verification_policy.checks[1] = VerifierCheck::File {
            path: "../escape".to_owned(),
            min_bytes: 1,
        };
        assert!(validate_plan(&plan, &agents).is_err());

        let mut plan = registry
            .plan("verification-matrix", &request, &agents)
            .expect("valid matrix");
        plan.tasks[0].verification_policy.checks[2] = VerifierCheck::Command {
            program: "node".to_owned(),
            args: Vec::new(),
            timeout_ms: 1,
        };
        assert!(validate_plan(&plan, &agents).is_err());

        let mut plan = registry
            .plan("human-approval", &request, &agents)
            .expect("valid human gate");
        plan.tasks[0].verification_policy.manual_gate =
            Some(ManualVerificationGate::HumanApproval { roles: Vec::new() });
        assert!(validate_plan(&plan, &agents).is_err());

        let mut plan = registry
            .plan("human-approval", &request, &agents)
            .expect("valid human gate");
        plan.tasks[0].verification_policy.manual_gate =
            Some(ManualVerificationGate::HumanApproval {
                roles: vec!["spectator".to_owned()],
            });
        assert!(validate_plan(&plan, &agents).is_err());

        let mut plan = registry
            .plan("verification-matrix", &request, &agents)
            .expect("valid matrix");
        plan.tasks[0].verification_policy.checks.clear();
        assert!(validate_plan(&plan, &agents).is_err());
    }
}
