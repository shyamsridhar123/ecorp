use std::{
    collections::{HashMap, HashSet},
    path::{Component, Path},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use crony_domain::{
    Agent, AgentStatus, ManualVerificationGate, PlannedTask, TaskContract, TaskGraphPlan,
    VerificationPolicy, VerifierCheck,
};

pub const MAX_GRAPH_NODES: usize = 8;
pub const MAX_GRAPH_DEPTH: i32 = 4;
pub const MAX_TASK_ATTEMPTS: i32 = 3;
pub const MAX_TASK_BUDGET_TOKENS: i64 = 200_000;
pub const MAX_GRAPH_BUDGET_TOKENS: i64 = 500_000;

pub struct PlanningRequest<'a> {
    pub mission_title: &'a str,
    pub preferred_adapter: Option<&'a str>,
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
        let budget_tokens = 100_000;
        Ok(TaskGraphPlan {
            strategy: self.id().to_owned(),
            max_nodes: 1,
            max_depth: 0,
            budget_tokens,
            tasks: vec![PlannedTask {
                key: "deliver".to_owned(),
                title: "Produce the mission outcome".to_owned(),
                contract: contract(
                    format!("Complete the mission outcome: {}", request.mission_title),
                    "A source-backed, verified mission artifact",
                    budget_tokens,
                ),
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

        let specialist_budget = 80_000;
        let synthesis_budget = 120_000;
        let mut synthesis_contract = contract(
            format!(
                "Produce the final bounded outcome for this mission after both specialist tasks finish: {}",
                request.mission_title
            ),
            "A final synthesis artifact that addresses the mission and acceptance tests",
            synthesis_budget,
        );
        synthesis_contract.references = vec![
            "task:specialist-a".to_owned(),
            "task:specialist-b".to_owned(),
        ];
        Ok(TaskGraphPlan {
            strategy: self.id().to_owned(),
            max_nodes: 3,
            max_depth: 1,
            budget_tokens: specialist_budget * 2 + synthesis_budget,
            tasks: vec![
                PlannedTask {
                    key: "specialist-a".to_owned(),
                    title: format!("{} specialist pass", first.name),
                    contract: contract(
                        format!(
                            "Independently produce one concrete approach for this mission: {}",
                            request.mission_title
                        ),
                        "A concrete specialist artifact with assumptions and verification",
                        specialist_budget,
                    ),
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
                    contract: contract(
                        format!(
                            "Independently produce a distinct approach for this mission: {}",
                            request.mission_title
                        ),
                        "A second specialist artifact with tradeoffs and verification",
                        specialist_budget,
                    ),
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
    let budget_tokens = 50_000;
    Ok(TaskGraphPlan {
        strategy: strategy.to_owned(),
        max_nodes: 1,
        max_depth: 0,
        budget_tokens,
        tasks: vec![PlannedTask {
            key: "verify".to_owned(),
            title: "Produce verifier evidence".to_owned(),
            contract: contract(
                format!("{instruction}\nMission: {}", request.mission_title),
                "An artifact and every file required by the verifier policy",
                budget_tokens,
            ),
            assigned_agent_id: agent.id,
            required_adapter: agent.adapter.clone(),
            depends_on: Vec::new(),
            depth: 0,
            max_attempts: 1,
            verification_policy,
        }],
    })
}

fn ordered_candidates(agents: &[Agent]) -> Vec<&Agent> {
    let mut candidates = agents
        .iter()
        .filter(|agent| agent.status != AgentStatus::Offline)
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
        deadline_at: None,
        escalation: "ask the current human controller or mission owner".to_owned(),
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
                .any(|role| role.trim().is_empty() || role.len() > 64)
        {
            return Err(anyhow!(
                "task {task_key} manual verification gate is invalid"
            ));
        }
    }
    Ok(())
}

fn validate_relative_path(task_key: &str, value: &str) -> Result<()> {
    let path = Path::new(value);
    if value.is_empty()
        || value.len() > 500
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
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
        }
    }

    fn agents() -> Vec<Agent> {
        vec![
            agent("Margo", "manager", "fake-process"),
            agent("Wally", "engineer", "fake-process"),
            agent("Cody", "engineer", "codex"),
        ]
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
                "verification-failure".to_owned(),
                "verification-matrix".to_owned(),
            ]
        );
        let request = PlanningRequest {
            mission_title: "ship the bounded graph",
            preferred_adapter: Some("codex"),
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
                },
                &agents,
            )
            .expect("single plan");
        assert_eq!(plan.tasks[0].required_adapter, "codex");
    }

    #[test]
    fn verifier_policies_reject_unsafe_paths_commands_and_gates() {
        let agents = agents();
        let registry = StrategyRegistry::new();
        let request = PlanningRequest {
            mission_title: "verify safely",
            preferred_adapter: Some("fake-process"),
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
            .plan("verification-matrix", &request, &agents)
            .expect("valid matrix");
        plan.tasks[0].verification_policy.checks.clear();
        assert!(validate_plan(&plan, &agents).is_err());
    }
}
