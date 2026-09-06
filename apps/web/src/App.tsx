import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { FormEvent } from 'react'
import './App.css'
import './Arcade.css'
import './Cabinet.css'
import './World.css'
import './Accessible.css'
import { OfficeFloor, OfficePortrait } from './OfficeFloor'
import { OfficeInspector } from './OfficeInspector'

type Actor = {
  id: string
  name: string
  kind: 'human' | 'agent' | 'service'
  role: string
}

type Agent = {
  id: string
  actor_id: string
  name: string
  role: string
  adapter: string
  status: 'idle' | 'starting' | 'working' | 'blocked' | 'reviewing' | 'offline'
  station: string | null
  current_run_id: string | null
  accent: string
}

type Mission = {
  id: string
  requested_by: string
  title: string
  description: string
  specification_version: number
  strategy: string
  max_nodes: number
  max_depth: number
  original_budget_tokens: number
  original_budget_cost_microusd: number
  budget_tokens: number
  budget_cost_microusd: number
  status: 'draft' | 'ready' | 'running' | 'completed' | 'failed' | 'cancelled'
  created_at: string
}

type TaskContract = {
  objective: string
  expected_output: string
  source_repository: string | null
  source_base_ref: string | null
  source_base_commit: string | null
  acceptance_tests: string[]
  allowed_tools: string[]
  prohibited_actions: string[]
  references: string[]
  write_scope: string[]
  budget_tokens: number
  budget_cost_microusd: number
  deadline_at: string | null
  escalation: string
  secret_refs: {
    secret_id: string
    env_name: string
    tool: string
    resource: string
  }[]
  model: string | null
  reasoning_effort: string | null
  deliverable: {
    form: 'commit_branch' | 'patch' | 'archive' | 'typed_artifact_set' | 'review_only_report'
    commit_after_verification: boolean
    paths: string[]
  } | null
}

type Task = {
  id: string
  mission_id: string
  title: string
  objective: string
  plan_key: string
  contract: TaskContract
  contract_version: number
  depth: number
  max_attempts: number
  attempt_count: number
  required_adapter: string | null
  depends_on: string[]
  verification_policy: VerificationPolicy
  verification_status: string
  status: string
  assigned_agent_id: string | null
}

type VerifierCheck =
  | { type: 'artifact'; min_bytes: number }
  | { type: 'file'; path: string; min_bytes: number }
  | { type: 'command'; program: string; args: string[]; timeout_ms: number }
  | { type: 'test'; program: string; args: string[]; timeout_ms: number }
  | { type: 'json_schema'; path: string; required_keys: string[] }
  | { type: 'screenshot'; path: string; min_bytes: number }

type ManualVerificationGate =
  | { type: 'human_approval'; roles: string[] }
  | { type: 'independent_review'; roles: string[]; exclude_requester: boolean }

type VerificationPolicy = {
  checks: VerifierCheck[]
  manual_gate: ManualVerificationGate | null
}

type MissionContractRevision = {
  id: string
  mission_id: string
  task_id: string
  version: number
  revised_by: string
  next_action: 'redispatch' | 'resume'
  source_run_id: string | null
  reason: string
  previous_description: string
  replacement_description: string
  previous_contract: TaskContract
  replacement_contract: TaskContract
  previous_verification_policy: VerificationPolicy
  replacement_verification_policy: VerificationPolicy
  created_at: string
}

type MissionContractInput = Pick<
  TaskContract,
  | 'objective'
  | 'expected_output'
  | 'acceptance_tests'
  | 'allowed_tools'
  | 'prohibited_actions'
  | 'references'
  | 'write_scope'
>

type MissionContractRevisionInput = {
  task_id: string
  expected_contract_version: number
  next_action: 'redispatch' | 'resume'
  source_run_id: string | null
  reason: string
  idempotency_key: string
  description: string
  contract: TaskContract
  verification_policy: VerificationPolicy
}

type Run = {
  id: string
  task_id: string
  agent_id: string
  runner_id: string
  provider_session_id: string | null
  resumed_from_run_id: string | null
  workspace_run_id: string
  model: string | null
  reasoning_effort: string | null
  input_tokens: number
  output_tokens: number
  cost_microusd: number
  budget_tokens_limit: number
  budget_cost_microusd_limit: number
  breaker_stage: 'steer' | 'constrain' | 'suspend' | 'stop' | null
  source_repository: string | null
  source_base_ref: string | null
  source_base_commit: string | null
  workspace_path: string | null
  workspace_branch: string | null
  workspace_base_ref: string | null
  workspace_base_commit: string | null
  workspace_disposition: string | null
  workspace_detail: string | null
  verification_status: string
  verification_summary: string | null
  verification_sha256: string | null
  deliverable_sha256: string | null
  status: string
  summary: string | null
  artifact_id: string | null
  artifact_uri: string | null
  artifact_media_type: string | null
  artifact_signature: string | null
  artifact_sha256: string | null
}

type SourceDeliverable = {
  id: string
  task_id: string
  run_id: string
  artifact_id: string
  form: 'commit_branch' | 'patch' | 'archive' | 'typed_artifact_set' | 'review_only_report'
  file_name: string
  uri: string
  sha256: string
  media_type: string
  bytes: number
  provenance_signature: string
  verification_sha256: string
  base_commit: string
  head_commit: string | null
  branch: string
  integration_state: 'not_applicable' | 'ready_for_review' | 'published' | 'integrated'
  retention_until: string
}

type PullRequestPublication = {
  id: string
  factory_work_item_id: string
  mission_id: string
  source_deliverable_id: string
  artifact_id: string
  task_id: string
  run_id: string
  source_issue_number: number
  source_issue_url: string
  target_repository: string
  base_ref: string
  branch: string
  commit_sha: string
  title: string
  body: string
  actor_id: string
  authorization_id: string
  authorization_snapshot: {
    actor_role?: string
    permission?: string
    reason?: string
    authorized_at?: string
  }
  effect_key: string
  idempotency_key: string
  state: 'requested' | 'publishing' | 'branch_pushed' | 'pull_request_created' | 'published'
  version: number
  attempt_count: number
  publisher_id: string | null
  publisher_lease_expires_at: string | null
  failure_detail: string | null
  branch_pushed_at: string | null
  pull_request_number: number | null
  pull_request_node_id: string | null
  pull_request_url: string | null
  pull_request_state: string | null
  pull_request_draft: boolean | null
  pull_request_base_ref: string | null
  pull_request_head_sha: string | null
  pull_request_head_repository_owner: string | null
  pull_request_is_cross_repository: boolean | null
  project_owner: string
  project_number: number
  project_item_id: string
  project_status_before: string
  project_status_after: string | null
  project_status_updated_at: string | null
  auto_merge_enabled: boolean
  merge_authorized: boolean
  deployment_authorized: boolean
}

type PullRequestPublicationAttempt = {
  id: string
  publication_id: string
  attempt: number
  actor_id: string
  authorization_id: string
  authorization_snapshot: {
    actor_role?: string
    permission?: string
    reason?: string
    authorized_at?: string
  }
  publisher_id: string
  state: 'running' | 'failed' | 'abandoned' | 'published'
  failure_detail: string | null
  started_at: string
  finished_at: string | null
}

type Lease = {
  agent_id: string
  actor_id: string
  expires_at: string
}

type QueuedMessage = {
  id: string
  agent_id: string
  actor_id: string
  text: string
  status: string
  created_at: string
}

type VerificationEvidence = {
  id: string
  task_id: string
  run_id: string
  check_index: number
  kind: string
  status: 'passed' | 'failed'
  summary: string
}

type VerificationRequest = {
  run_id: string
  task_id: string
  gate_type: 'human_approval' | 'independent_review'
  gate:
    | { type: 'human_approval'; roles: string[] }
    | { type: 'independent_review'; roles: string[]; exclude_requester: boolean }
  status: 'pending' | 'approved' | 'rejected'
  decided_by: string | null
  decision_note: string | null
}

type ActionApproval = {
  id: string
  run_id: string
  action: string
  risk: 'low' | 'medium' | 'high' | 'critical'
  rationale: string
  required_roles: string[]
  status: 'pending' | 'approved' | 'rejected' | 'expired'
  expires_at: string
}

type CircuitBreakerIncident = {
  id: string
  run_id: string
  stage: 'steer' | 'constrain' | 'suspend' | 'stop'
  reason: string
  created_at: string
}

type MissionBudgetRevision = {
  id: string
  mission_id: string
  proposed_by: string
  status: 'pending' | 'approved' | 'rejected'
  version: number
  current_budget_tokens: number
  current_budget_cost_microusd: number
  proposed_budget_tokens: number
  proposed_budget_cost_microusd: number
  consumed_tokens_at_proposal: number
  consumed_cost_microusd_at_proposal: number
  rationale: string
  replacement_task_id: string | null
  previous_contract: TaskContract | null
  replacement_contract: TaskContract | null
  previous_verification_policy: Record<string, unknown> | null
  replacement_verification_policy: Record<string, unknown> | null
  decided_by: string | null
  decision_note: string | null
  created_at: string
  decided_at: string | null
  updated_at: string
}

type MissionFinishScopeInput = {
  task_id: string
  objective: string
  expected_output: string
  acceptance_tests: string[]
  write_scope: string[]
  budget_tokens: number
  budget_cost_microusd: number
  verification_policy: Record<string, unknown>
}

type MissionBudgetRevisionInput = {
  proposed_budget_tokens: number
  proposed_budget_cost_microusd: number
  rationale: string
  idempotency_key: string
  finish_scope: MissionFinishScopeInput | null
}

type FactoryWorkItem = {
  id: string
  source_project_owner: string
  source_project_number: number
  source_project_item_id: string
  source_repository_owner: string
  source_repository_name: string
  source_issue_number: number
  source_issue_url: string
  source_title: string
  source_revision: string
  state:
    | 'claimed'
    | 'mission_created'
    | 'running'
    | 'blocked'
    | 'awaiting_approval'
    | 'verification_failed'
    | 'verified'
    | 'publishing'
    | 'published'
    | 'failed'
    | 'cancelled'
  version: number
  claim_owner_id: string
  lease_expires_at: string
  policy: Record<string, unknown>
  mission_id: string | null
  failure_detail: string | null
}

type FactoryController = {
  id: string
  service_actor_id: string
  configured_by: string
  source_project_owner: string
  source_project_number: number
  source_repository_owner: string
  source_repository_name: string
  desired_state: 'running' | 'paused'
  status: 'offline' | 'watching' | 'working' | 'blocked' | 'needs_decision'
  version: number
  lease_expires_at: string
  last_heartbeat_at: string
  reconcile_generation: number
  completed_reconcile_generation: number
  active_work_item_id: string | null
  last_reconciled_at: string | null
  last_reconcile_result: 'succeeded' | 'failed' | null
  last_error: string | null
}

type DomainEvent = {
  seq: number
  id: string
  type: string
  actor_id: string | null
  aggregate_type: string
  aggregate_id: string
  payload: Record<string, unknown>
  created_at: string
}

type EntityLink = {
  kind: 'mission' | 'task' | 'run' | 'artifact'
  id: string
}

type RoomMessage = {
  id: string
  room_id: string
  actor_id: string
  thread_root_id: string | null
  reply_to_id: string | null
  body: string
  mentions: string[]
  link: EntityLink | null
  created_at: string
}

type BrowserSocketMessage =
  | { type: 'ready'; corp_id: string; replayed_through: number }
  | { type: 'event'; event: DomainEvent }

type WorkspaceView = 'floor' | 'factory' | 'missions' | 'room' | 'activity'

type RunnerModel = {
  id: string
  name: string
  policy_state: string | null
  policy_terms: string | null
  supports_vision: boolean
  supports_reasoning_effort: boolean
  max_prompt_tokens: number | null
  max_context_window_tokens: number | null
  supported_reasoning_efforts: string[]
  default_reasoning_effort: string | null
  billing_multiplier: number | null
}

type RunnerCapability = {
  name: string
  available: boolean
  detail: string | null
  models: RunnerModel[]
  source_repository?: string | null
  source_base_ref?: string | null
  source_base_commit?: string | null
}

type RunnerNode = {
  id: string
  corp_id: string
  hostname: string
  os: string
  connected: boolean
  status: 'connected' | 'grace' | 'offline'
  last_seen_at: string
  grace_expires_at: string | null
  capabilities: RunnerCapability[]
}

type RepositoryTarget = {
  key: string
  repository: string
  baseRef: string
  baseCommit: string
  runnerIds: string[]
  runnerLabels: string[]
}

type SnapshotResponse = {
  snapshot: {
    corp: { id: string; name: string }
    actors: Actor[]
    rooms: { id: string; name: string; purpose: string }[]
    agents: Agent[]
    missions: Mission[]
    mission_contract_revisions: MissionContractRevision[]
    mission_budget_revisions: MissionBudgetRevision[]
    tasks: Task[]
    runs: Run[]
    room_messages: RoomMessage[]
    leases: Lease[]
    queued_messages: QueuedMessage[]
    verification_evidence: VerificationEvidence[]
    verification_requests: VerificationRequest[]
    source_deliverables: SourceDeliverable[]
    pull_request_publications: PullRequestPublication[]
    pull_request_publication_attempts: PullRequestPublicationAttempt[]
    action_approvals: ActionApproval[]
    circuit_breaker_incidents: CircuitBreakerIncident[]
    factory_work_items: FactoryWorkItem[]
    factory_controllers?: FactoryController[]
    events: DomainEvent[]
  }
  runners: RunnerNode[]
}

type BootstrapResponse = {
  corp_id: string
  room_id: string
  alice_actor_id: string
  bob_actor_id: string
  eve_actor_id: string
  manager_agent_id: string
  worker_agent_id: string
  codex_agent_id: string
}

type CreateMissionResponse = {
  mission_id: string
  task_ids: string[]
  strategy: string
}

type LaunchMissionResponse = {
  run_id: string
  runner_id: string
  run_ids: string[]
  runner_ids: string[]
}

type HealthResponse = {
  status: 'ok'
  service: string
  runners: number
  mode: 'development' | 'production'
}

const API_URL = import.meta.env.VITE_CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const DEFAULT_MISSION = 'Prepare a verified launch-readiness brief for the ECorp alpha.'
const MISSION_EXAMPLES = [
  {
    label: 'Build a feature',
    value: 'Implement the requested feature, test it in the isolated worktree, and produce a reviewable artifact with concrete verification.',
  },
  {
    label: 'Fix a bug',
    value: 'Reproduce the reported bug, identify the root cause, implement the smallest correct fix, and verify the user-visible behavior.',
  },
  {
    label: 'Review a change',
    value: 'Review the current repository changes for correctness, safety, regressions, and missing tests. Produce prioritized findings with evidence.',
  },
] as const

const DETERMINISTIC_HARNESS_STRATEGIES = [
  'verification-matrix',
  'verification-failure',
  'human-approval',
  'independent-review',
] as const

const WORKSPACE_VIEWS: {
  id: WorkspaceView
  code: string
  label: string
  description: string
}[] = [
  {
    id: 'floor',
    code: '01',
    label: 'Control floor',
    description: 'Watch the live crew, take control, and steer active work.',
  },
  {
    id: 'factory',
    code: '02',
    label: 'Factory',
    description: 'Track governed GitHub issue intake and publication state.',
  },
  {
    id: 'missions',
    code: '03',
    label: 'Missions',
    description: 'Authorize one mission, inspect its contract, and decide the next move.',
  },
  {
    id: 'room',
    code: '04',
    label: 'Comms',
    description: 'Coordinate through durable, attributable room messages.',
  },
  {
    id: 'activity',
    code: '05',
    label: 'Audit',
    description: 'Inspect the immutable event trail and pending risk decisions.',
  },
]

function workspaceViewFromHash(hash: string): WorkspaceView {
  const candidate = hash.replace(/^#/, '') as WorkspaceView
  return WORKSPACE_VIEWS.some((view) => view.id === candidate) ? candidate : 'floor'
}

function revealEntityTarget(kind: EntityLink['kind'] | 'room', id: string): boolean {
  const target = document.querySelector<HTMLElement>(
    `[data-${kind}-id="${CSS.escape(id)}"]`,
  )
  if (!target) return false

  if (target instanceof HTMLDetailsElement) target.open = true
  let ancestor = target.parentElement?.closest('details')
  while (ancestor) {
    ancestor.open = true
    ancestor = ancestor.parentElement?.closest('details')
  }

  const focusTarget =
    target instanceof HTMLDetailsElement
      ? target.querySelector<HTMLElement>(':scope > summary') ?? target
      : target
  focusTarget.scrollIntoView({ behavior: 'smooth', block: 'center' })
  focusTarget.focus({ preventScroll: true })
  return true
}

function usesDeterministicHarness(strategy: string): boolean {
  return DETERMINISTIC_HARNESS_STRATEGIES.includes(
    strategy as (typeof DETERMINISTIC_HARNESS_STRATEGIES)[number],
  )
}

function adapterLabel(adapter: string): string {
  if (adapter === 'github-copilot') return 'GitHub Copilot'
  if (adapter === 'codex') return 'OpenAI Codex'
  if (adapter === 'claude-code') return 'Claude Code'
  if (adapter === 'opencode') return 'OpenCode'
  if (adapter === 'fake-process') return 'Test harness'
  return adapter
}

function adapterDescription(adapter: string): string {
  if (adapter === 'github-copilot') return 'Use any model enabled for your Copilot account.'
  if (adapter === 'codex') return 'Run a real Codex coding session in an isolated worktree.'
  if (adapter === 'claude-code') return 'Run the locally authenticated Claude Code CLI.'
  if (adapter === 'opencode') return 'Run the configured OpenCode provider.'
  if (adapter === 'fake-process') return 'Deterministic and quota-free. Use this only to test ECorp itself.'
  return 'Run work through this connected agent adapter.'
}

function agentStatusLabel(agent: Agent): string {
  if (agent.status === 'idle' && !agent.current_run_id) return 'idle'
  if (agent.status === 'offline') return 'runner unavailable'
  return agent.status
}

function statusLabel(value: string): string {
  return value
    .replaceAll('_', ' ')
    .replace(/\b\w/g, (letter) => letter.toUpperCase())
}

function capabilitySupports(
  capability: RunnerCapability | undefined,
  feature: 'steer' | 'interrupt' | 'stop',
): boolean {
  return detailValue(capability?.detail, feature) === 'yes'
}

function canOperate(role: string): boolean {
  return ['owner', 'admin', 'manager', 'member'].includes(role)
}

function terminalRun(status: string): boolean {
  return ['completed', 'failed', 'cancelled', 'lost'].includes(status)
}

function workspaceCapability(runner: RunnerNode): RunnerCapability | undefined {
  return runner.capabilities.find(
    (capability) =>
      capability.name === 'workspace-isolation' &&
      capability.available &&
      capability.source_repository &&
      capability.source_base_ref &&
      capability.source_base_commit,
  )
}

function sourceFingerprint(
  repository: string,
  baseRef: string,
  baseCommit: string,
): string {
  return JSON.stringify([
    repository.toLowerCase(),
    baseRef,
    baseCommit.toLowerCase(),
  ])
}

function repositoryTargets(data: SnapshotResponse | null): RepositoryTarget[] {
  if (!data) return []
  const targets = new Map<string, RepositoryTarget>()
  for (const runner of data.runners.filter((candidate) => candidate.connected)) {
    const capability = workspaceCapability(runner)
    if (
      !capability?.source_repository ||
      !capability.source_base_ref ||
      !capability.source_base_commit
    ) {
      continue
    }
    const key = sourceFingerprint(
      capability.source_repository,
      capability.source_base_ref,
      capability.source_base_commit,
    )
    const existing = targets.get(key)
    if (existing) {
      existing.runnerIds.push(runner.id)
      existing.runnerLabels.push(`${runner.hostname} · ${runner.os}`)
      continue
    }
    targets.set(key, {
      key,
      repository: capability.source_repository,
      baseRef: capability.source_base_ref,
      baseCommit: capability.source_base_commit,
      runnerIds: [runner.id],
      runnerLabels: [`${runner.hostname} · ${runner.os}`],
    })
  }
  return Array.from(targets.values()).sort((left, right) =>
    left.repository.localeCompare(right.repository),
  )
}

function runnerMatchesRepository(
  runner: RunnerNode,
  target: RepositoryTarget,
): boolean {
  const capability = workspaceCapability(runner)
  return Boolean(
    capability?.source_repository?.toLowerCase() ===
      target.repository.toLowerCase() &&
      capability.source_base_ref === target.baseRef &&
      capability.source_base_commit?.toLowerCase() ===
        target.baseCommit.toLowerCase(),
  )
}

function isEcorpRepository(target: RepositoryTarget | undefined): boolean {
  return target?.repository.toLowerCase().endsWith('/ecorp') ?? false
}

function availableRunnerAdapters(
  data: SnapshotResponse | null,
  target?: RepositoryTarget,
): RunnerCapability[] {
  if (!data) return []
  const connectedRunners = data.runners.filter(
    (runner) => runner.connected && (!target || runnerMatchesRepository(runner, target)),
  )
  const agentAdapters = new Set(data.snapshot.agents.map((agent) => agent.adapter))
  return Array.from(
    connectedRunners
      .flatMap((runner) => runner.capabilities)
      .filter(
        (capability) =>
          capability.available && agentAdapters.has(capability.name),
      )
      .reduce((adapters, capability) => {
        const existing = adapters.get(capability.name)
        const modelsById = new Map(
          (existing?.models ?? []).map((model) => [model.id, model]),
        )
        for (const model of capability.models) {
          const prior = modelsById.get(model.id)
          if (
            !prior ||
            (prior.policy_state === 'disabled' &&
              model.policy_state !== 'disabled')
          ) {
            modelsById.set(model.id, model)
          }
        }
        adapters.set(capability.name, {
          ...capability,
          detail: existing?.detail ?? capability.detail,
          models: Array.from(modelsById.values()),
        })
        return adapters
      }, new Map<string, RunnerCapability>())
      .values(),
  )
}

function detailValue(detail: string | null | undefined, key: string): string | null {
  if (!detail) return null
  const match = detail.match(new RegExp(`(?:^|[;,])\\s*${key}=([^;,]+)`))
  return match?.[1]?.trim() ?? null
}

function storedAccessToken(): string | null {
  return window.sessionStorage.getItem('ecorp_access_token')
}

class ApiRequestError extends Error {
  status: number

  constructor(status: number, message: string) {
    super(message)
    this.name = 'ApiRequestError'
    this.status = status
  }
}

function browserOperationKey(storageKey: string, payload: string): string {
  try {
    const stored = window.sessionStorage.getItem(storageKey)
    if (stored) {
      const operation = JSON.parse(stored) as { payload?: unknown; key?: unknown }
      if (
        operation.payload === payload &&
        typeof operation.key === 'string' &&
        operation.key
      ) {
        return operation.key
      }
    }
    const key = crypto.randomUUID()
    window.sessionStorage.setItem(storageKey, JSON.stringify({ payload, key }))
    return key
  } catch {
    return crypto.randomUUID()
  }
}

function clearBrowserOperation(storageKey: string) {
  window.sessionStorage.removeItem(storageKey)
}

async function api<T>(path: string, init?: RequestInit): Promise<T> {
  const token = storedAccessToken()
  const response = await fetch(`${API_URL}${path}`, {
    ...init,
    headers: {
      'content-type': 'application/json',
      ...(token ? { authorization: `Bearer ${token}` } : {}),
      ...init?.headers,
    },
  })
  const body = await response.json()
  if (!response.ok) {
    throw new ApiRequestError(
      response.status,
      body.error ?? `${response.status} ${response.statusText}`,
    )
  }
  return body as T
}

function shortId(value: string | null | undefined): string {
  return value ? value.slice(0, 8) : 'none'
}

function time(value: string): string {
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(value))
}

function formatUsd(microusd: number): string {
  return new Intl.NumberFormat(undefined, {
    style: 'currency',
    currency: 'USD',
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(microusd / 1_000_000)
}

function budgetRemainingLabel(value: number, unit: 'tokens' | 'cost'): string {
  const amount =
    unit === 'tokens'
      ? Math.abs(value).toLocaleString()
      : formatUsd(Math.abs(value))
  return value >= 0 ? `${amount} left` : `${amount} over`
}

function nonEmptyLines(value: string): string[] {
  return value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
}

function commaOrLines(value: string): string[] {
  return value
    .split(/[\r\n,]+/)
    .map((item) => item.trim())
    .filter(Boolean)
}

function defaultVerifierCheck(type: VerifierCheck['type'] = 'artifact'): VerifierCheck {
  if (type === 'artifact') return { type, min_bytes: 1 }
  if (type === 'file') return { type, path: 'README.md', min_bytes: 1 }
  if (type === 'screenshot') {
    return { type, path: 'evidence/browser.png', min_bytes: 1_000 }
  }
  if (type === 'json_schema') {
    return { type, path: 'evidence/result.json', required_keys: ['status'] }
  }
  return {
    type,
    program: type === 'test' ? 'pnpm' : 'git',
    args: type === 'test' ? ['test'] : ['status', '--short'],
    timeout_ms: 60_000,
  }
}

function verifierCheckSummary(check: VerifierCheck): string {
  if (check.type === 'artifact') {
    return `Provider artifact · at least ${check.min_bytes.toLocaleString()} bytes`
  }
  if (check.type === 'file') {
    return `File ${check.path} · at least ${check.min_bytes.toLocaleString()} bytes`
  }
  if (check.type === 'screenshot') {
    return `Screenshot ${check.path} · at least ${check.min_bytes.toLocaleString()} bytes`
  }
  if (check.type === 'json_schema') {
    return `JSON ${check.path} · keys: ${check.required_keys.join(', ')}`
  }
  return `${check.type === 'test' ? 'Test' : 'Command'} · ${[check.program, ...check.args].join(' ')} · ${Math.round(check.timeout_ms / 1_000)}s`
}

function verificationPolicyErrors(policy: VerificationPolicy): string[] {
  const errors: string[] = []
  if (!policy.checks.length) errors.push('Add at least one verifier check.')
  if (policy.checks.length > 16) errors.push('Verifier policies support at most 16 checks.')
  policy.checks.forEach((check, index) => {
    const label = `Check ${index + 1}`
    if ('min_bytes' in check && (!Number.isFinite(check.min_bytes) || check.min_bytes < 1)) {
      errors.push(`${label} needs a positive byte floor.`)
    }
    if ('path' in check && !check.path.trim()) errors.push(`${label} needs a repository path.`)
    if ((check.type === 'command' || check.type === 'test') && !check.program.trim()) {
      errors.push(`${label} needs an executable program.`)
    }
    if (
      (check.type === 'command' || check.type === 'test') &&
      (!Number.isFinite(check.timeout_ms) || check.timeout_ms < 100 || check.timeout_ms > 60_000)
    ) {
      errors.push(`${label} timeout must be between 100 and 60,000 ms.`)
    }
    if (check.type === 'json_schema' && !check.required_keys.length) {
      errors.push(`${label} needs at least one required JSON key.`)
    }
  })
  if (policy.manual_gate && !policy.manual_gate.roles.length) {
    errors.push('The manual gate needs at least one eligible role.')
  } else if (
    policy.manual_gate?.roles.some(
      (role) => !['owner', 'admin', 'manager', 'member'].includes(role),
    )
  ) {
    errors.push('Manual-gate roles must be owner, admin, manager, or member.')
  }
  return errors
}

function leaseTokenKey(actorId: string, agentId: string): string {
  return `${actorId}:${agentId}`
}

function StatusMark({ status }: { status: Agent['status'] }) {
  return <span className={`status-mark status-${status}`} aria-label={status} />
}

function VerificationPolicyPreview({
  policy,
  heading = 'Completion gates',
}: {
  policy: VerificationPolicy
  heading?: string
}) {
  return (
    <div className="verification-policy-preview" data-testid="verification-policy-preview">
      <strong>{heading}</strong>
      <ol>
        {policy.checks.map((check, index) => (
          <li key={`${check.type}-${index}`}>
            <span>{index + 1}</span>
            <p>{verifierCheckSummary(check)}</p>
          </li>
        ))}
      </ol>
      <div className="verification-gate-summary">
        <span>Manual gate</span>
        <strong>
          {policy.manual_gate
            ? `${statusLabel(policy.manual_gate.type)} · ${policy.manual_gate.roles.join(', ')}`
            : 'None'}
        </strong>
        {policy.manual_gate?.type === 'independent_review' ? (
          <small>
            {policy.manual_gate.exclude_requester
              ? 'Mission requester is excluded from the decision.'
              : 'Mission requester may decide if their role is eligible.'}
          </small>
        ) : null}
      </div>
    </div>
  )
}

function VerificationPolicyEditor({
  policy,
  onChange,
  idPrefix,
}: {
  policy: VerificationPolicy
  onChange: (policy: VerificationPolicy) => void
  idPrefix: string
}) {
  const [selectedCheckIndex, setSelectedCheckIndex] = useState(0)
  const selectedIndex = Math.min(
    selectedCheckIndex,
    Math.max(0, policy.checks.length - 1),
  )
  const selectedCheck = policy.checks[selectedIndex]
  const replaceCheck = (index: number, check: VerifierCheck) => {
    const checks = policy.checks.slice()
    checks[index] = check
    onChange({ ...policy, checks })
  }
  const removeCheck = (index: number) => {
    setSelectedCheckIndex(Math.max(0, Math.min(index - 1, policy.checks.length - 2)))
    onChange({
      ...policy,
      checks: policy.checks.filter((_, candidate) => candidate !== index),
    })
  }
  const setGate = (type: 'none' | ManualVerificationGate['type']) => {
    if (type === 'none') {
      onChange({ ...policy, manual_gate: null })
      return
    }
    onChange({
      ...policy,
      manual_gate:
        type === 'human_approval'
          ? { type, roles: ['owner', 'admin'] }
          : {
              type,
              roles: ['member', 'manager', 'admin', 'owner'],
              exclude_requester: true,
            },
    })
  }
  const errors = verificationPolicyErrors(policy)

  return (
    <div className="verification-policy-editor" data-testid={`${idPrefix}-verification-editor`}>
      <div className="contract-section-heading">
        <div>
          <strong>Victory gate editor</strong>
          <span>Each check executes on the runner inside the assigned worktree.</span>
        </div>
        <button
          className="button button-secondary"
          type="button"
          disabled={policy.checks.length >= 16}
          onClick={() => {
            setSelectedCheckIndex(policy.checks.length)
            onChange({
              ...policy,
              checks: [...policy.checks, defaultVerifierCheck('file')],
            })
          }}
        >
          Add check
        </button>
      </div>
      {policy.checks.length ? (
        <>
          <nav className="verification-check-tabs" aria-label="Verifier checks">
            {policy.checks.map((check, index) => (
              <button
                key={`${check.type}-${index}`}
                type="button"
                className={selectedIndex === index ? 'check-tab-active' : ''}
                aria-pressed={selectedIndex === index}
                onClick={() => setSelectedCheckIndex(index)}
              >
                <span>{String(index + 1).padStart(2, '0')}</span>
                <strong>{statusLabel(check.type)}</strong>
              </button>
            ))}
          </nav>
          {selectedCheck ? (
            <fieldset className="verification-check-editor">
              <legend>Check {selectedIndex + 1}</legend>
              <div className="verification-check-toolbar">
                <label>
                  Type
                  <select
                    aria-label={`Verifier check ${selectedIndex + 1} type`}
                    value={selectedCheck.type}
                    onChange={(event) =>
                      replaceCheck(
                        selectedIndex,
                        defaultVerifierCheck(
                          event.target.value as VerifierCheck['type'],
                        ),
                      )
                    }
                  >
                    <option value="artifact">Provider artifact</option>
                    <option value="file">File</option>
                    <option value="command">Command</option>
                    <option value="test">Test</option>
                    <option value="json_schema">JSON schema</option>
                    <option value="screenshot">Screenshot</option>
                  </select>
                </label>
                <button
                  className="button button-quiet"
                  type="button"
                  onClick={() => removeCheck(selectedIndex)}
                >
                  Remove
                </button>
              </div>
              {selectedCheck.type === 'artifact' ? (
                <label>
                  Minimum artifact bytes
                  <input
                    type="number"
                    min={1}
                    value={selectedCheck.min_bytes}
                    onChange={(event) =>
                      replaceCheck(selectedIndex, {
                        ...selectedCheck,
                        min_bytes: Number(event.target.value),
                      })
                    }
                  />
                </label>
              ) : null}
              {selectedCheck.type === 'file' || selectedCheck.type === 'screenshot' ? (
                <div className="verification-check-grid">
                  <label>
                    Worktree-relative path
                    <input
                      value={selectedCheck.path}
                      onChange={(event) =>
                        replaceCheck(selectedIndex, {
                          ...selectedCheck,
                          path: event.target.value,
                        })
                      }
                      placeholder={
                        selectedCheck.type === 'screenshot'
                          ? 'evidence/browser.png'
                          : 'path/to/result.txt'
                      }
                    />
                  </label>
                  <label>
                    Minimum bytes
                    <input
                      type="number"
                      min={1}
                      value={selectedCheck.min_bytes}
                      onChange={(event) =>
                        replaceCheck(selectedIndex, {
                          ...selectedCheck,
                          min_bytes: Number(event.target.value),
                        })
                      }
                    />
                  </label>
                </div>
              ) : null}
              {selectedCheck.type === 'command' || selectedCheck.type === 'test' ? (
                <>
                  <div className="verification-check-grid">
                    <label>
                      Program
                      <input
                        value={selectedCheck.program}
                        onChange={(event) =>
                          replaceCheck(selectedIndex, {
                            ...selectedCheck,
                            program: event.target.value,
                          })
                        }
                        placeholder="pnpm"
                      />
                    </label>
                    <label>
                      Timeout in milliseconds
                      <input
                        type="number"
                        min={100}
                        max={60_000}
                        value={selectedCheck.timeout_ms}
                        onChange={(event) =>
                          replaceCheck(selectedIndex, {
                            ...selectedCheck,
                            timeout_ms: Number(event.target.value),
                          })
                        }
                      />
                    </label>
                  </div>
                  <label>
                    Arguments, one per line
                    <textarea
                      rows={3}
                      value={selectedCheck.args.join('\n')}
                      onChange={(event) =>
                        replaceCheck(selectedIndex, {
                          ...selectedCheck,
                          args: nonEmptyLines(event.target.value),
                        })
                      }
                      placeholder={'--dir\napps/web\ntest'}
                    />
                  </label>
                </>
              ) : null}
              {selectedCheck.type === 'json_schema' ? (
                <>
                  <label>
                    JSON file
                    <input
                      value={selectedCheck.path}
                      onChange={(event) =>
                        replaceCheck(selectedIndex, {
                          ...selectedCheck,
                          path: event.target.value,
                        })
                      }
                      placeholder="evidence/result.json"
                    />
                  </label>
                  <label>
                    Required top-level keys
                    <textarea
                      rows={3}
                      value={selectedCheck.required_keys.join('\n')}
                      onChange={(event) =>
                        replaceCheck(selectedIndex, {
                          ...selectedCheck,
                          required_keys: nonEmptyLines(event.target.value),
                        })
                      }
                    />
                  </label>
                </>
              ) : null}
            </fieldset>
          ) : null}
        </>
      ) : (
        <div className="default-gate-callout">
          <span>NO CHECKS</span>
          <strong>Add a victory gate</strong>
        </div>
      )}
      <div className="manual-gate-editor">
        <label>
          Final reviewer gate
          <select
            value={policy.manual_gate?.type ?? 'none'}
            onChange={(event) =>
              setGate(event.target.value as 'none' | ManualVerificationGate['type'])
            }
          >
            <option value="none">No manual gate</option>
            <option value="human_approval">Human approval</option>
            <option value="independent_review">Independent review</option>
          </select>
        </label>
        {policy.manual_gate ? (
          <label>
            Eligible roles
            <textarea
              rows={2}
              value={policy.manual_gate.roles.join(', ')}
              onChange={(event) =>
                onChange({
                  ...policy,
                  manual_gate: policy.manual_gate
                    ? {
                        ...policy.manual_gate,
                        roles: commaOrLines(event.target.value),
                      }
                    : null,
                })
              }
            />
          </label>
        ) : null}
        {policy.manual_gate?.type === 'independent_review' ? (
          <label className="mission-run-toggle">
            <input
              type="checkbox"
              checked={policy.manual_gate.exclude_requester}
              onChange={(event) =>
                onChange({
                  ...policy,
                  manual_gate:
                    policy.manual_gate?.type === 'independent_review'
                      ? {
                          ...policy.manual_gate,
                          exclude_requester: event.target.checked,
                        }
                      : policy.manual_gate,
                })
              }
            />
            <span>
              <strong>Exclude the mission requester</strong>
              <small>Require another operator to accept the evidence.</small>
            </span>
          </label>
        ) : null}
      </div>
      {errors.length ? <p className="contract-error">{errors[0]}</p> : null}
      <details className="verification-plan-disclosure">
        <summary>Preview exact completion plan</summary>
        <VerificationPolicyPreview policy={policy} heading="Exact completion plan" />
      </details>
    </div>
  )
}

function ContractRevisionPanel({
  mission,
  task,
  runs,
  actorId,
  actorRole,
  busy,
  onRevise,
}: {
  mission: Mission
  task: Task
  runs: Run[]
  actorId: string
  actorRole: string
  busy: boolean
  onRevise: (
    mission: Mission,
    task: Task,
    input: MissionContractRevisionInput,
  ) => Promise<boolean>
}) {
  const [open, setOpen] = useState(false)
  const [description, setDescription] = useState(mission.description)
  const [reason, setReason] = useState('')
  const [contractJson, setContractJson] = useState(() =>
    JSON.stringify(task.contract, null, 2),
  )
  const [policyJson, setPolicyJson] = useState(() =>
    JSON.stringify(task.verification_policy, null, 2),
  )
  const [idempotencyKey, setIdempotencyKey] = useState(() => crypto.randomUUID())
  const canRevise =
    actorId === mission.requested_by || ['owner', 'admin', 'manager'].includes(actorRole)
  const activeRun = runs.some((run) => !terminalRun(run.status))
  const redispatchEligible = mission.status === 'ready' && runs.length === 0
  const sourceRun = runs.find(
    (run) =>
      run.task_id === task.id &&
      terminalRun(run.status) &&
      run.provider_session_id &&
      run.workspace_disposition === 'preserved' &&
      run.breaker_stage !== 'stop',
  )
  const nextAction: MissionContractRevisionInput['next_action'] | null =
    !activeRun && redispatchEligible ? 'redispatch' : !activeRun && sourceRun ? 'resume' : null
  const resetDraft = () => {
    setDescription(mission.description)
    setReason('')
    setContractJson(JSON.stringify(task.contract, null, 2))
    setPolicyJson(JSON.stringify(task.verification_policy, null, 2))
    setIdempotencyKey(crypto.randomUUID())
  }
  useEffect(() => {
    if (!open) resetDraft()
    // Reset only after a committed version arrives or the selected mission changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mission.id, mission.specification_version, task.id, task.contract_version])

  let parsedContract: TaskContract | null = null
  let parsedPolicy: VerificationPolicy | null = null
  let parseError: string | null = null
  try {
    parsedContract = JSON.parse(contractJson) as TaskContract
    parsedPolicy = JSON.parse(policyJson) as VerificationPolicy
    const policyError = verificationPolicyErrors(parsedPolicy)[0]
    if (policyError) parseError = policyError
  } catch (caught) {
    parseError = caught instanceof Error ? caught.message : String(caught)
  }

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    if (!nextAction || !parsedContract || !parsedPolicy || !reason.trim() || parseError) return
    const saved = await onRevise(mission, task, {
      task_id: task.id,
      expected_contract_version: task.contract_version,
      next_action: nextAction,
      source_run_id: nextAction === 'resume' ? sourceRun?.id ?? null : null,
      reason,
      idempotency_key: idempotencyKey,
      description,
      contract: parsedContract,
      verification_policy: parsedPolicy,
    })
    if (saved) {
      setOpen(false)
      setIdempotencyKey(crypto.randomUUID())
    }
  }

  if (!nextAction || !canRevise) return null
  return (
    <div className="contract-revision-panel" data-testid={`contract-revision-${task.id}`}>
      {open ? (
        <form onSubmit={submit}>
          <div className="contract-section-heading">
            <div>
              <strong>
                Revise for {nextAction === 'resume' ? 'preserved-session resume' : 'redispatch'}
              </strong>
              <span>
                Revision {task.contract_version + 1} is durable and never starts work automatically.
              </span>
            </div>
            <button
              className="button button-quiet"
              type="button"
              onClick={() => {
                setOpen(false)
                resetDraft()
              }}
            >
              Cancel
            </button>
          </div>
          <label>
            Revision reason
            <textarea
              rows={2}
              maxLength={4_000}
              value={reason}
              onChange={(event) => setReason(event.target.value)}
              placeholder="What changed, and why is this revision required?"
            />
          </label>
          <label>
            Mission description / specification
            <textarea
              rows={6}
              maxLength={100_000}
              value={description}
              onChange={(event) => setDescription(event.target.value)}
            />
          </label>
          <label>
            Typed task contract · JSON
            <textarea
              className="contract-json-editor"
              rows={18}
              value={contractJson}
              onChange={(event) => setContractJson(event.target.value)}
              spellCheck={false}
            />
          </label>
          <label>
            Typed verifier policy · JSON
            <textarea
              className="contract-json-editor"
              rows={12}
              value={policyJson}
              onChange={(event) => setPolicyJson(event.target.value)}
              spellCheck={false}
            />
          </label>
          {nextAction === 'resume' ? (
            <small>
              Resume revisions cannot widen tools or write scope, remove prohibitions, or change
              source, model, secrets, budget, reasoning, or deliverable authority.
            </small>
          ) : null}
          <div className="contract-revision-footer">
            <span className={parseError ? 'contract-error' : ''}>
              {parseError ??
                `Save revision ${task.contract_version + 1}; then explicitly ${nextAction === 'resume' ? 'resume the preserved run' : 'dispatch the mission'}.`}
            </span>
            <button
              className="button button-primary"
              type="submit"
              disabled={busy || Boolean(parseError) || !reason.trim()}
            >
              Save revision
            </button>
          </div>
        </form>
      ) : (
        <button
          className="button button-secondary contract-revision-open"
          type="button"
          disabled={busy}
          onClick={() => setOpen(true)}
        >
          Revise contract for {nextAction}
        </button>
      )}
    </div>
  )
}

function FactoryPanel({
  items,
  missions,
  publications,
  publicationAttempts,
  controllers,
  tasks,
  runs,
  room,
  messages,
  actors,
  agents,
  leases,
  leaseTokens,
  selectedActor,
  actionApprovals,
  verificationRequests,
  canControlFactory,
  busy,
  onControllerControl,
  onPostComment,
  onActionDecision,
  onVerificationDecision,
  onClaimLease,
  onSteer,
}: {
  items: FactoryWorkItem[]
  missions: Mission[]
  publications: PullRequestPublication[]
  publicationAttempts: PullRequestPublicationAttempt[]
  controllers: FactoryController[]
  tasks: Task[]
  runs: Run[]
  room: { id: string; name: string; purpose: string } | undefined
  messages: RoomMessage[]
  actors: Actor[]
  agents: Agent[]
  leases: Lease[]
  leaseTokens: Record<string, string>
  selectedActor: Actor
  actionApprovals: ActionApproval[]
  verificationRequests: VerificationRequest[]
  canControlFactory: boolean
  busy: boolean
  onControllerControl: (
    controller: FactoryController,
    action: 'pause' | 'resume' | 'reconcile',
  ) => void
  onPostComment: (input: {
    roomId: string
    body: string
    replyToId: string | null
    mentions: string[]
    link: EntityLink | null
    idempotencyKey: string
  }) => Promise<boolean>
  onActionDecision: (approval: ActionApproval, approved: boolean) => Promise<void>
  onVerificationDecision: (run: Run, approved: boolean) => Promise<void>
  onClaimLease: (agent: Agent) => Promise<void>
  onSteer: (
    agent: Agent,
    text: string,
    token: string | undefined,
    idempotencyKey: string,
  ) => Promise<boolean>
}) {
  const [commentBody, setCommentBody] = useState('')
  const [steerText, setSteerText] = useState('')
  const stateTone = (state: FactoryWorkItem['state']) => {
    if (['verified', 'published'].includes(state)) return 'completed'
    if (['blocked', 'verification_failed', 'failed', 'cancelled'].includes(state)) {
      return 'failed'
    }
    if (['running', 'awaiting_approval', 'publishing'].includes(state)) return 'running'
    return 'ready'
  }
  const active = items.filter(
    (item) => !['published', 'failed', 'cancelled'].includes(item.state),
  )
  const [selectedItemId, setSelectedItemId] = useState<string | null>(
    () => items[0]?.id ?? null,
  )
  const selected = items.find((item) => item.id === selectedItemId) ?? items[0]
  const selectedMission = missions.find((candidate) => candidate.id === selected?.mission_id)
  const selectedTasks = tasks.filter((task) => task.mission_id === selectedMission?.id)
  const selectedTaskIds = new Set(selectedTasks.map((task) => task.id))
  const selectedRuns = runs.filter((run) => selectedTaskIds.has(run.task_id))
  const activeRun = selectedRuns.find((run) =>
    ['provisioning', 'starting', 'running', 'waiting_for_input', 'waiting_for_approval'].includes(
      run.status,
    ),
  )
  const activeAgent = agents.find((agent) => agent.id === activeRun?.agent_id)
  const activeLease = leases.find((lease) => lease.agent_id === activeAgent?.id)
  const activeLeaseToken = activeAgent
    ? leaseTokens[leaseTokenKey(selectedActor.id, activeAgent.id)]
    : undefined
  const leaseAttributedToSelectedActor =
    activeLease?.actor_id === selectedActor.id
  const leaseHeldBySelectedActor =
    leaseAttributedToSelectedActor && Boolean(activeLeaseToken)
  const selectedRunIds = new Set(selectedRuns.map((run) => run.id))
  const pendingActions = actionApprovals.filter(
    (approval) => selectedRunIds.has(approval.run_id) && approval.status === 'pending',
  )
  const pendingReviews = verificationRequests.filter(
    (request) => selectedRunIds.has(request.run_id) && request.status === 'pending',
  )
  const linkedIds = new Set([
    ...(selectedMission ? [selectedMission.id] : []),
    ...selectedTasks.map((task) => task.id),
    ...selectedRuns.flatMap((run) => [
      run.id,
      ...(run.artifact_id ? [run.artifact_id] : []),
    ]),
  ])
  const contextualMessages = messages
    .filter((message) => message.link && linkedIds.has(message.link.id))
    .slice(-8)
  const selectedPublication = publications.find(
    (candidate) => candidate.factory_work_item_id === selected?.id,
  )
  const selectedAttempts = publicationAttempts
    .filter((attempt) => attempt.publication_id === selectedPublication?.id)
    .sort((left, right) => right.attempt - left.attempt)
  const controller = controllers[0]
  const controllerState = controller?.desired_state === 'paused'
    ? 'paused'
    : controller?.status ?? 'not_configured'

  return (
    <section
      className="factory-panel panel"
      id="factory"
      data-testid="factory-panel"
      tabIndex={-1}
    >
      <div className="panel-heading factory-heading">
        <div>
          <span className="section-code">Factory</span>
          <h2>Governed issue intake</h2>
          <p>
            GitHub work enters one fenced lane, becomes one mission, and advances only on evidence.
          </p>
        </div>
        <div className="operations-summary">
          <span>{active.length} active</span>
          <span>{items.filter((item) => item.state === 'verified').length} verified</span>
          <span>auto-merge off</span>
        </div>
      </div>
      <section
        className={`factory-controller-strip factory-controller-${controllerState}`}
        aria-label="Factory controller status"
      >
        <div>
          <span className="factory-controller-kicker">Controller</span>
          <strong>{statusLabel(controllerState)}</strong>
          <small>
            {controller
              ? `${controller.source_project_owner} / Project #${controller.source_project_number} · ${controller.source_repository_owner}/${controller.source_repository_name}`
              : 'No trusted factory watcher has registered with this Corp.'}
          </small>
        </div>
        {controller ? (
          <>
            <dl>
              <div>
                <dt>Heartbeat</dt>
                <dd>{time(controller.last_heartbeat_at)}</dd>
              </div>
              <div>
                <dt>Reconciled</dt>
                <dd>
                  {controller.last_reconciled_at
                    ? time(controller.last_reconciled_at)
                    : 'Not yet'}
                </dd>
              </div>
              <div>
                <dt>Generation</dt>
                <dd>
                  {controller.completed_reconcile_generation}/
                  {controller.reconcile_generation}
                </dd>
              </div>
            </dl>
            <div className="factory-controller-actions">
              <button
                type="button"
                className="button button-secondary"
                disabled={busy || !canControlFactory}
                onClick={() =>
                  onControllerControl(
                    controller,
                    controller.desired_state === 'paused' ? 'resume' : 'pause',
                  )
                }
              >
                {controller.desired_state === 'paused' ? 'Resume intake' : 'Pause intake'}
              </button>
              <button
                type="button"
                className="button button-primary"
                disabled={busy || !canControlFactory}
                onClick={() => onControllerControl(controller, 'reconcile')}
              >
                Reconcile now
              </button>
            </div>
          </>
        ) : (
          <p>
            Configure the trusted controller process to make Factory actively watch GitHub
            Project intake.
          </p>
        )}
        {controller?.last_error ? (
          <p className="factory-controller-error" role="alert">
            {controller.last_error}
          </p>
        ) : null}
      </section>
      <div className={`factory-console ${items.length ? '' : 'factory-console-empty'}`}>
        {items.length ? (
          <>
            <nav className="factory-queue" aria-label="Factory work items">
              <div className="factory-queue-label">
                <span>Issue queue</span>
                <strong>{items.length.toString().padStart(2, '0')}</strong>
              </div>
              {items.map((item, index) => {
                const mission = missions.find((candidate) => candidate.id === item.mission_id)
                return (
                  <button
                    key={item.id}
                    type="button"
                    className={selected?.id === item.id ? 'factory-queue-active' : ''}
                    aria-pressed={selected?.id === item.id}
                    aria-controls="factory-workbench"
                    data-status={stateTone(item.state)}
                    onClick={() => setSelectedItemId(item.id)}
                  >
                    <span className="factory-queue-index">
                      {String(index + 1).padStart(2, '0')}
                    </span>
                    <span className="factory-queue-copy">
                      <small>
                        GitHub #{item.source_issue_number} · {statusLabel(item.state)}
                      </small>
                      <strong>{item.source_title}</strong>
                      <span>{mission ? `Mission ${shortId(mission.id)}` : 'Mission pending'}</span>
                    </span>
                  </button>
                )
              })}
            </nav>

            <div className="factory-workbench" id="factory-workbench">
              {selected ? (
                <article
                  className="factory-dossier-stage"
                  data-factory-item-id={selected.id}
                  tabIndex={-1}
                >
                  <div className="factory-dossier-top">
                    <div>
                      <span className="factory-source">
                        GitHub issue #{selected.source_issue_number}
                      </span>
                      <small>
                        {selected.source_repository_owner}/{selected.source_repository_name}
                      </small>
                    </div>
                    <span className={`status-chip status-chip-${stateTone(selected.state)}`}>
                      {statusLabel(selected.state)}
                    </span>
                  </div>
                  <h3>
                    <a href={selected.source_issue_url} target="_blank" rel="noreferrer">
                      {selected.source_title}
                    </a>
                  </h3>

                  {selected.failure_detail ? (
                    <section className="factory-failure-dossier" aria-label="Factory failure detail">
                      <span>Failure record</span>
                      <p>{selected.failure_detail}</p>
                    </section>
                  ) : (
                    <p className="factory-dossier-summary">
                      This work item is fenced to one controller, one source revision, and one
                      evidence-gated mission.
                    </p>
                  )}

                  <dl className="factory-facts">
                    <div>
                      <dt>Project</dt>
                      <dd>
                        {selected.source_project_owner} / #{selected.source_project_number}
                      </dd>
                    </div>
                    <div>
                      <dt>Mission</dt>
                      <dd>{selectedMission ? shortId(selectedMission.id) : 'Not materialized'}</dd>
                    </div>
                    <div>
                      <dt>Controller</dt>
                      <dd>{shortId(selected.claim_owner_id)}</dd>
                    </div>
                    <div>
                      <dt>Lease</dt>
                      <dd>{time(selected.lease_expires_at)}</dd>
                    </div>
                  </dl>

                  {selectedMission ? (
                    <div className="factory-cockpit-grid">
                      <section className="work-item-decisions" aria-label="Work-item decisions">
                        <div className="factory-cockpit-heading">
                          <span>Decisions</span>
                          <strong>{pendingActions.length + pendingReviews.length}</strong>
                        </div>
                        {pendingActions.map((approval) => {
                          const eligible = approval.required_roles.includes(selectedActor.role)
                          return (
                            <article key={approval.id}>
                              <strong>{approval.action}</strong>
                              <p>{approval.rationale}</p>
                              <small>
                                {statusLabel(approval.risk)} risk · expires{' '}
                                {time(approval.expires_at)}
                              </small>
                              <div>
                                <button
                                  type="button"
                                  disabled={busy || !eligible}
                                  onClick={() => void onActionDecision(approval, false)}
                                >
                                  Reject
                                </button>
                                <button
                                  type="button"
                                  className="button button-primary"
                                  disabled={busy || !eligible}
                                  onClick={() => void onActionDecision(approval, true)}
                                >
                                  Approve
                                </button>
                              </div>
                            </article>
                          )
                        })}
                        {pendingReviews.map((request) => {
                          const run = selectedRuns.find(
                            (candidate) => candidate.id === request.run_id,
                          )
                          const eligible =
                            request.gate.roles.includes(selectedActor.role) &&
                            !(
                              request.gate.type === 'independent_review' &&
                              request.gate.exclude_requester &&
                              selectedMission.requested_by === selectedActor.id
                            )
                          return run ? (
                            <article key={request.run_id}>
                              <strong>{statusLabel(request.gate_type)}</strong>
                              <p>Review the persisted verification evidence for this run.</p>
                              <small>Run {shortId(run.id)}</small>
                              <div>
                                <button
                                  type="button"
                                  disabled={busy || !eligible}
                                  onClick={() => void onVerificationDecision(run, false)}
                                >
                                  Reject
                                </button>
                                <button
                                  type="button"
                                  className="button button-primary"
                                  disabled={busy || !eligible}
                                  onClick={() => void onVerificationDecision(run, true)}
                                >
                                  Accept evidence
                                </button>
                              </div>
                            </article>
                          ) : null
                        })}
                        {!pendingActions.length && !pendingReviews.length ? (
                          <p className="factory-cockpit-empty">
                            No policy exception or review decision is waiting.
                          </p>
                        ) : null}
                      </section>
                      <section className="work-item-comms" aria-label="Work-item comments">
                        <div className="factory-cockpit-heading">
                          <span>Contextual Comms</span>
                          <strong>{contextualMessages.length}</strong>
                        </div>
                        <ol>
                          {contextualMessages.map((message) => (
                            <li key={message.id}>
                              <strong>
                                {actors.find((actor) => actor.id === message.actor_id)?.name ??
                                  'Unknown actor'}
                              </strong>
                              <p>{message.body}</p>
                              <small>{time(message.created_at)}</small>
                            </li>
                          ))}
                        </ol>
                        {!contextualMessages.length ? (
                          <p className="factory-cockpit-empty">
                            No comments are linked to this work item yet.
                          </p>
                        ) : null}
                        {room ? (
                          <form
                            onSubmit={(event) => {
                              event.preventDefault()
                              if (!commentBody.trim()) return
                              const mentionedNames = Array.from(
                                commentBody.matchAll(/@([A-Za-z0-9_-]+)/g),
                                (match) => match[1].toLowerCase(),
                              )
                              const body = commentBody.trim()
                              const mentions = actors
                                .filter((actor) =>
                                  mentionedNames.includes(actor.name.toLowerCase()),
                                )
                                .map((actor) => actor.id)
                              const payload = JSON.stringify({
                                roomId: room.id,
                                body,
                                mentions,
                                missionId: selectedMission.id,
                              })
                              const operationStorageKey =
                                `ecorp:factory-comment:${selected.id}:${selectedActor.id}`
                              const idempotencyKey = browserOperationKey(
                                operationStorageKey,
                                payload,
                              )
                              void onPostComment({
                                roomId: room.id,
                                body,
                                replyToId: null,
                                mentions,
                                link: { kind: 'mission', id: selectedMission.id },
                                idempotencyKey,
                              }).then((saved) => {
                                if (saved) {
                                  clearBrowserOperation(operationStorageKey)
                                  setCommentBody('')
                                }
                              })
                            }}
                          >
                            <label htmlFor={`factory-comment-${selected.id}`}>
                              Comment as {selectedActor.name}
                            </label>
                            <textarea
                              id={`factory-comment-${selected.id}`}
                              rows={3}
                              value={commentBody}
                              onChange={(event) => setCommentBody(event.target.value)}
                              placeholder="Comment on this work item. Use agent controls to steer."
                            />
                            <button
                              type="submit"
                              className="button button-secondary"
                              disabled={busy || !commentBody.trim()}
                            >
                              Post comment
                            </button>
                          </form>
                        ) : null}
                      </section>
                      <section className="work-item-steer" aria-label="Live agent direction">
                        <div className="factory-cockpit-heading">
                          <span>Steer</span>
                          <strong>{activeAgent?.name ?? 'No active agent'}</strong>
                        </div>
                        {activeAgent && activeRun ? (
                          <>
                            <p>
                              Run {shortId(activeRun.id)} · {statusLabel(activeRun.status)}
                            </p>
                            {!activeLease ||
                            (leaseAttributedToSelectedActor && !activeLeaseToken) ? (
                              <>
                                {leaseAttributedToSelectedActor ? (
                                  <p className="factory-cockpit-empty">
                                    This browser reconnected without {selectedActor.name}&apos;s
                                    private fencing token. Reclaim control to rotate it; the old
                                    token will stop working.
                                  </p>
                                ) : null}
                                <button
                                  type="button"
                                  className="button button-secondary"
                                  disabled={busy}
                                  onClick={() => void onClaimLease(activeAgent)}
                                >
                                  {leaseAttributedToSelectedActor
                                    ? 'Reclaim control'
                                    : 'Take control'}
                                </button>
                              </>
                            ) : leaseHeldBySelectedActor ? (
                              <form
                                onSubmit={(event) => {
                                  event.preventDefault()
                                  if (!steerText.trim()) return
                                  void onSteer(
                                    activeAgent,
                                    steerText.trim(),
                                    activeLeaseToken,
                                    (() => {
                                      const payload = JSON.stringify({
                                        agentId: activeAgent.id,
                                        actorId: selectedActor.id,
                                        text: steerText.trim(),
                                      })
                                      return browserOperationKey(
                                        `ecorp:factory-steer:${activeRun.id}:${selectedActor.id}`,
                                        payload,
                                      )
                                    })(),
                                  ).then((saved) => {
                                    if (saved) {
                                      clearBrowserOperation(
                                        `ecorp:factory-steer:${activeRun.id}:${selectedActor.id}`,
                                      )
                                      setSteerText('')
                                    }
                                  })
                                }}
                              >
                                <label htmlFor={`factory-steer-${selected.id}`}>
                                  Direction from {selectedActor.name}
                                </label>
                                <textarea
                                  id={`factory-steer-${selected.id}`}
                                  rows={3}
                                  value={steerText}
                                  onChange={(event) => setSteerText(event.target.value)}
                                  placeholder="Send live direction under your control lease."
                                />
                                <button
                                  type="submit"
                                  className="button button-primary"
                                  disabled={busy || !steerText.trim()}
                                >
                                  Send direction
                                </button>
                              </form>
                            ) : (
                              <p className="factory-cockpit-empty">
                                Controlled by{' '}
                                {actors.find((actor) => actor.id === activeLease.actor_id)?.name ??
                                  'another operator'}
                                . Comments remain available without the control lease.
                              </p>
                            )}
                          </>
                        ) : (
                          <p className="factory-cockpit-empty">
                            Live direction appears here while an agent is running.
                          </p>
                        )}
                      </section>
                    </div>
                  ) : null}

                  {selectedPublication ? (
                    <div className="publication-proof" data-testid="factory-publication">
                      <div className="publication-proof-heading">
                        <strong>Verified pull-request publication</strong>
                        <span
                          className={`status-chip status-chip-${
                            selectedPublication.state === 'published'
                              ? 'completed'
                              : selectedPublication.failure_detail
                                ? 'failed'
                                : 'running'
                          }`}
                        >
                          {statusLabel(selectedPublication.state)}
                        </span>
                      </div>
                      <dl>
                        <div>
                          <dt>Target</dt>
                          <dd>
                            {selectedPublication.target_repository} · {selectedPublication.base_ref}
                            {selectedPublication.pull_request_base_ref &&
                            selectedPublication.pull_request_base_ref !== selectedPublication.base_ref
                              ? ` → ${selectedPublication.pull_request_base_ref}`
                              : ''}
                          </dd>
                        </div>
                        <div>
                          <dt>Branch</dt>
                          <dd>
                            {selectedPublication.branch} @ {shortId(selectedPublication.commit_sha)}
                          </dd>
                        </div>
                        <div>
                          <dt>Authorization</dt>
                          <dd>
                            {selectedPublication.authorization_snapshot.actor_role ?? 'authorized'} ·{' '}
                            {shortId(selectedPublication.actor_id)}
                          </dd>
                        </div>
                        <div>
                          <dt>Attempts</dt>
                          <dd>
                            {selectedPublication.attempt_count}
                            {selectedAttempts[0]
                              ? ` · latest ${statusLabel(selectedAttempts[0].state)}`
                              : ''}
                          </dd>
                        </div>
                      </dl>
                      {selectedPublication.authorization_snapshot.reason ? (
                        <p>{selectedPublication.authorization_snapshot.reason}</p>
                      ) : null}
                      {selectedPublication.pull_request_url ? (
                        <>
                          <a
                            className="publication-link"
                            href={selectedPublication.pull_request_url}
                            target="_blank"
                            rel="noreferrer"
                          >
                            Pull request #{selectedPublication.pull_request_number} ·{' '}
                            {selectedPublication.pull_request_draft ? 'draft' : 'open for review'}
                          </a>
                          <span className="publication-pending">
                            Verified head {selectedPublication.pull_request_head_repository_owner} @{' '}
                            {selectedPublication.pull_request_head_sha
                              ? shortId(selectedPublication.pull_request_head_sha)
                              : 'pending'}
                            {selectedPublication.pull_request_is_cross_repository === false
                              ? ' · same repository'
                              : ''}
                            {selectedPublication.pull_request_base_ref
                              ? ` · PR base ${selectedPublication.pull_request_base_ref}`
                              : ''}
                          </span>
                        </>
                      ) : (
                        <span className="publication-pending">Pull request not created yet</span>
                      )}
                      <footer>
                        <span>
                          Project {selectedPublication.project_status_before}
                          {selectedPublication.project_status_after
                            ? ` → ${selectedPublication.project_status_after}`
                            : ''}
                        </span>
                        <span>auto-merge off · merge/deploy unauthorized</span>
                      </footer>
                      {selectedPublication.failure_detail ? (
                        <p className="factory-failure">{selectedPublication.failure_detail}</p>
                      ) : null}
                    </div>
                  ) : (
                    <div className="factory-publication-empty">
                      <span>Publication bay</span>
                      <strong>No pull request yet</strong>
                      <p>Publication unlocks only after the mission and its evidence are verified.</p>
                    </div>
                  )}

                  <footer className="factory-dossier-footer">
                    <span>Source rev {selected.source_revision}</span>
                    <span>Record v{selected.version}</span>
                  </footer>
                </article>
              ) : null}
            </div>
          </>
        ) : (
          <div className="empty-state">
            <strong>No factory work claimed</strong>
            <span>
              Eligible GitHub Project issues appear here after the controller persists its lease.
            </span>
          </div>
        )}
      </div>
    </section>
  )
}

function AgentAvatar({ agent }: { agent: Agent }) {
  return (
    <div
      className={`agent-sprite accent-${agent.accent} sprite-${agent.status}`}
      data-agent={agent.name.toLowerCase()}
      aria-hidden="true"
    >
      <span className="sprite-shadow" />
      <OfficePortrait agentId={agent.id} />
      <span className="sprite-signal" />
    </div>
  )
}

function AgentDesk({
  agent,
  capability,
  lease,
  leaseToken,
  actor,
  humans,
  queuedCount,
  onClaim,
  onRelease,
  onTransfer,
  onInterrupt,
  onEmergencyStop,
  onMessage,
}: {
  agent: Agent
  capability: RunnerCapability | undefined
  lease: Lease | undefined
  leaseToken: string | undefined
  actor: Actor
  humans: Actor[]
  queuedCount: number
  onClaim: (agent: Agent) => Promise<void>
  onRelease: (agent: Agent, token: string) => Promise<void>
  onTransfer: (agent: Agent, token: string, toActor: Actor) => Promise<void>
  onInterrupt: (agent: Agent, token: string) => Promise<void>
  onEmergencyStop: (agent: Agent) => Promise<void>
  onMessage: (
    agent: Agent,
    text: string,
    token: string | undefined,
    idempotencyKey: string,
  ) => Promise<boolean>
}) {
  const [text, setText] = useState('')
  const [transferActorId, setTransferActorId] = useState('')
  const ownsLease = lease?.actor_id === actor.id
  const live = Boolean(agent.current_run_id)
  const operator = canOperate(actor.role)
  const supportsSteer = capabilitySupports(capability, 'steer')
  const supportsInterrupt = capabilitySupports(capability, 'interrupt')
  const supportsStop = capabilitySupports(capability, 'stop')
  const canClaim = live && operator && (supportsSteer || supportsInterrupt)
  const holder = humans.find((human) => human.id === lease?.actor_id)
  const transferCandidates = humans.filter((human) => human.id !== actor.id && canOperate(human.role))
  const transferTarget =
    transferCandidates.find((human) => human.id === transferActorId) ??
    transferCandidates[0]
  const holderLabel = lease
    ? ownsLease
      ? `You hold control until ${time(lease.expires_at)}`
      : `${holder?.name ?? 'Another operator'} holds control until ${time(lease.expires_at)}`
    : live
      ? 'Live session is unclaimed'
      : 'No live session'
  const messageToken = live && supportsSteer && ownsLease ? leaseToken : undefined

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    if (!text.trim()) return
    const normalized = text.trim()
    const payload = JSON.stringify({
      agentId: agent.id,
      actorId: actor.id,
      text: normalized,
    })
    const operationStorageKey = `ecorp:agent-message:${agent.id}:${actor.id}`
    const idempotencyKey = browserOperationKey(operationStorageKey, payload)
    const saved = await onMessage(
      agent,
      normalized,
      messageToken,
      idempotencyKey,
    )
    if (saved) {
      clearBrowserOperation(operationStorageKey)
      setText('')
    }
  }

  return (
    <article className={`agent-inspector desk-${agent.status}`}>
      <div className="agent-inspector-summary">
        <AgentAvatar agent={agent} />
        <div>
          <span className="agent-inspector-kicker">{agent.role} · {agentStatusLabel(agent)}</span>
          <div className="agent-name">
            <StatusMark status={agent.status} />
            {agent.name}
          </div>
          <div className="agent-meta">
            {adapterLabel(agent.adapter)} · run {shortId(agent.current_run_id)}
          </div>
          <div className={`lease-label ${ownsLease ? 'lease-owned' : ''}`}>{holderLabel}</div>
        </div>
      </div>
      <p className="agent-inspector-help">
        {agent.current_run_id
          ? `${agent.name} is ${agent.station ?? agent.status}. Claim control to steer the live session.`
          : agent.status === 'reviewing'
            ? `${agent.name}'s provider process has ended. The recorded output is awaiting evidence review.`
          : `${agent.name} is off shift. No provider process is running; the identity remains available for future ${adapterLabel(agent.adapter)} missions.`}
      </p>
      <div className="desk-actions">
        {canClaim ? (
          <button type="button" className="button button-secondary" onClick={() => onClaim(agent)}>
            {ownsLease && leaseToken ? 'Renew control' : 'Claim live control'}
          </button>
        ) : null}
        {live && ownsLease && leaseToken ? (
          <button
            type="button"
            className="button button-quiet"
            onClick={() => onRelease(agent, leaseToken)}
          >
            Release
          </button>
        ) : null}
        <span className="queue-count">{queuedCount} queued</span>
      </div>
      {live && ownsLease && leaseToken && transferTarget ? (
        <div className="transfer-row">
          <label htmlFor={`transfer-${agent.id}`}>Transfer control</label>
          <select
            id={`transfer-${agent.id}`}
            value={transferTarget.id}
            onChange={(event) => setTransferActorId(event.target.value)}
          >
            {transferCandidates.map((candidate) => (
              <option key={candidate.id} value={candidate.id}>
                {candidate.name} · {candidate.role}
              </option>
            ))}
          </select>
          <button
            type="button"
            className="transfer-control"
            onClick={() => onTransfer(agent, leaseToken, transferTarget)}
          >
            Transfer
          </button>
        </div>
      ) : null}
      {live && supportsStop && ['owner', 'admin', 'manager'].includes(actor.role) ? (
        <button
          type="button"
          className="emergency-stop"
          onClick={() => onEmergencyStop(agent)}
        >
          Emergency stop
        </button>
      ) : null}
      {live && supportsInterrupt && ownsLease && leaseToken ? (
        <button
          type="button"
          className="interrupt-run"
          onClick={() => onInterrupt(agent, leaseToken)}
        >
          Interrupt turn
        </button>
      ) : null}
      <form className="agent-message" onSubmit={submit}>
        <input
          aria-label={`Message ${agent.name}`}
          value={text}
          onChange={(event) => setText(event.target.value)}
          placeholder={messageToken ? 'Send live direction…' : 'Queue a note for the next supported turn…'}
        />
        <button className="button button-ink" type="submit">
          {messageToken ? 'Steer' : 'Queue note'}
        </button>
      </form>
      {live && !supportsSteer ? (
        <small className="control-note">
          {adapterLabel(agent.adapter)} does not support live steering. Notes are queued instead.
        </small>
      ) : null}
    </article>
  )
}

function BudgetRevisionPanel({
  mission,
  tasks,
  runs,
  revisions,
  actors,
  actorRole,
  busy,
  resumableRun,
  onPropose,
  onDecision,
}: {
  mission: Mission
  tasks: Task[]
  runs: Run[]
  revisions: MissionBudgetRevision[]
  actors: Actor[]
  actorRole: string
  busy: boolean
  resumableRun: Run | undefined
  onPropose: (mission: Mission, input: MissionBudgetRevisionInput) => Promise<boolean>
  onDecision: (
    mission: Mission,
    revision: MissionBudgetRevision,
    approved: boolean,
    decisionKey: string,
  ) => Promise<void>
}) {
  const [proposalOpen, setProposalOpen] = useState(false)
  const [proposedTokens, setProposedTokens] = useState(mission.budget_tokens)
  const [proposedCostUsd, setProposedCostUsd] = useState(
    mission.budget_cost_microusd / 1_000_000,
  )
  const [rationale, setRationale] = useState('')
  const [replaceFinishScope, setReplaceFinishScope] = useState(false)
  const [finishTaskId, setFinishTaskId] = useState('')
  const [finishObjective, setFinishObjective] = useState('')
  const [finishExpectedOutput, setFinishExpectedOutput] = useState('')
  const [finishAcceptanceTests, setFinishAcceptanceTests] = useState('')
  const [finishWriteScope, setFinishWriteScope] = useState('')
  const [finishBudgetTokens, setFinishBudgetTokens] = useState(1)
  const [finishBudgetCostUsd, setFinishBudgetCostUsd] = useState(0.01)
  const [proposalKey, setProposalKey] = useState(() => crypto.randomUUID())
  const decisionKeys = useRef<Record<string, string>>({})

  const consumedTokens = runs.reduce(
    (total, run) => total + run.input_tokens + run.output_tokens,
    0,
  )
  const consumedCostMicrousd = runs.reduce(
    (total, run) => total + run.cost_microusd,
    0,
  )
  const remainingTokens = mission.budget_tokens - consumedTokens
  const remainingCostMicrousd =
    mission.budget_cost_microusd - consumedCostMicrousd
  const budgetExhausted = remainingTokens <= 0 || remainingCostMicrousd <= 0
  const activeRun = runs.some((run) => !terminalRun(run.status))
  const orderedRevisions = revisions.toSorted(
    (left, right) =>
      new Date(right.created_at).getTime() - new Date(left.created_at).getTime(),
  )
  const pendingRevision = orderedRevisions.find(
    (revision) => revision.status === 'pending',
  )
  const latestApprovedRevision = orderedRevisions.find(
    (revision) => revision.status === 'approved',
  )
  const latestRun = runs[0]
  const recoverableSuspension =
    latestRun?.id === resumableRun?.id &&
    resumableRun?.breaker_stage === 'suspend'
      ? resumableRun
      : undefined
  const finishTasks = tasks.filter(
    (task) =>
      task.status !== 'completed' && task.id === recoverableSuspension?.task_id,
  )
  const selectedFinishTask = finishTasks.find(
    (task) => task.id === finishTaskId,
  )
  const canManageBudget = ['owner', 'admin'].includes(actorRole)
  const canReviseNow =
    Boolean(recoverableSuspension) &&
    !activeRun &&
    mission.status !== 'completed'
  const proposedCostMicrousd = Math.round(proposedCostUsd * 1_000_000)
  const finishCostMicrousd = Math.round(finishBudgetCostUsd * 1_000_000)
  const proposedRemainingTokens = proposedTokens - consumedTokens
  const proposedRemainingCostMicrousd =
    proposedCostMicrousd - consumedCostMicrousd
  const actorName = (actorId: string | null) =>
    actors.find((actor) => actor.id === actorId)?.name ??
    (actorId ? `Actor ${shortId(actorId)}` : 'Not decided')
  const decisionKey = (
    revision: MissionBudgetRevision,
    approved: boolean,
  ) => {
    const key = `${revision.id}:${approved ? 'approve' : 'reject'}`
    decisionKeys.current[key] ??= crypto.randomUUID()
    return decisionKeys.current[key]
  }

  const seedFinishTask = (
    task: Task | undefined,
    missionTokenCeiling: number,
    missionCostCeiling: number,
  ) => {
    if (!task) {
      setFinishTaskId('')
      setFinishObjective('')
      setFinishExpectedOutput('')
      setFinishAcceptanceTests('')
      setFinishWriteScope('')
      setFinishBudgetTokens(1)
      setFinishBudgetCostUsd(0.01)
      return
    }
    const availableTokens = Math.max(
      1,
      missionTokenCeiling - consumedTokens,
    )
    const availableCostMicrousd = Math.max(
      1,
      missionCostCeiling - consumedCostMicrousd,
    )
    setFinishTaskId(task.id)
    setFinishObjective(task.objective)
    setFinishExpectedOutput(task.contract.expected_output)
    setFinishAcceptanceTests(task.contract.acceptance_tests.join('\n'))
    setFinishWriteScope(task.contract.write_scope.join('\n'))
    setFinishBudgetTokens(
      Math.min(task.contract.budget_tokens, availableTokens, 2_000_000),
    )
    const finishCostMicrousd = Math.min(
      task.contract.budget_cost_microusd,
      availableCostMicrousd,
      10_000_000,
    )
    setFinishBudgetCostUsd(
      Math.max(10_000, Math.floor(finishCostMicrousd / 10_000) * 10_000) /
        1_000_000,
    )
  }

  const openProposal = () => {
    const nextTokens = Math.min(
      20_000_000,
      Math.max(
        mission.budget_tokens +
          Math.max(50_000, Math.ceil(mission.budget_tokens * 0.25)),
        consumedTokens + 50_000,
      ),
    )
    const nextCostMicrousd = Math.min(
      100_000_000,
      Math.ceil(
        Math.max(
        mission.budget_cost_microusd +
          Math.max(500_000, Math.ceil(mission.budget_cost_microusd * 0.25)),
        consumedCostMicrousd + 500_000,
        ) / 10_000,
      ) * 10_000,
    )
    const defaultTask =
      finishTasks.find((task) => task.id === recoverableSuspension?.task_id) ??
      finishTasks[0]
    setProposedTokens(nextTokens)
    setProposedCostUsd(nextCostMicrousd / 1_000_000)
    setRationale('')
    setProposalKey(crypto.randomUUID())
    setReplaceFinishScope(false)
    seedFinishTask(defaultTask, nextTokens, nextCostMicrousd)
    setProposalOpen(true)
  }

  const acceptanceTests = nonEmptyLines(finishAcceptanceTests)
  const writeScope = nonEmptyLines(finishWriteScope)
  const proposalErrors: string[] = []
  if (
    proposedTokens < mission.budget_tokens ||
    proposedCostMicrousd < mission.budget_cost_microusd ||
    (proposedTokens === mission.budget_tokens &&
      proposedCostMicrousd === mission.budget_cost_microusd)
  ) {
    proposalErrors.push(
      'Raise at least one current mission limit without reducing the other.',
    )
  }
  if (proposedTokens <= consumedTokens) {
    proposalErrors.push('The token ceiling must exceed consumed usage.')
  }
  if (proposedCostMicrousd <= consumedCostMicrousd) {
    proposalErrors.push('The cost ceiling must exceed consumed spend.')
  }
  if (
    proposedTokens > 20_000_000 ||
    proposedCostMicrousd > 100_000_000
  ) {
    proposalErrors.push('The proposed mission ceiling exceeds policy bounds.')
  }
  if (!rationale.trim()) {
    proposalErrors.push('Record why this additional budget is authorized.')
  }
  if (replaceFinishScope) {
    if (!selectedFinishTask) {
      proposalErrors.push('Choose the unfinished task to narrow.')
    } else {
      if (!finishObjective.trim() || !finishExpectedOutput.trim()) {
        proposalErrors.push('The bounded finish objective and output are required.')
      }
      if (!acceptanceTests.length || !writeScope.length) {
        proposalErrors.push(
          'Keep at least one acceptance test and one authorized write path.',
        )
      }
      if (
        finishBudgetTokens < 1 ||
        finishBudgetTokens > selectedFinishTask.contract.budget_tokens ||
        finishBudgetTokens > proposedRemainingTokens ||
        finishBudgetTokens > 2_000_000
      ) {
        proposalErrors.push(
          'The finish token budget must fit both the prior task and proposed remaining budget.',
        )
      }
      if (
        finishCostMicrousd < 1 ||
        finishCostMicrousd >
          selectedFinishTask.contract.budget_cost_microusd ||
        finishCostMicrousd > proposedRemainingCostMicrousd ||
        finishCostMicrousd > 10_000_000
      ) {
        proposalErrors.push(
          'The finish cost budget must fit both the prior task and proposed remaining budget.',
        )
      }
    }
  }

  const submitProposal = async (event: FormEvent) => {
    event.preventDefault()
    if (proposalErrors.length) return
    const finishScope =
      replaceFinishScope && selectedFinishTask
        ? {
            task_id: selectedFinishTask.id,
            objective: finishObjective.trim(),
            expected_output: finishExpectedOutput.trim(),
            acceptance_tests: acceptanceTests,
            write_scope: writeScope,
            budget_tokens: finishBudgetTokens,
            budget_cost_microusd: finishCostMicrousd,
            verification_policy: selectedFinishTask.verification_policy,
          }
        : null
    const accepted = await onPropose(mission, {
      proposed_budget_tokens: proposedTokens,
      proposed_budget_cost_microusd: proposedCostMicrousd,
      rationale: rationale.trim(),
      idempotency_key: proposalKey,
      finish_scope: finishScope,
    })
    if (accepted) setProposalOpen(false)
  }

  const tokenPercent =
    mission.budget_tokens > 0
      ? Math.min(100, Math.max(0, (consumedTokens / mission.budget_tokens) * 100))
      : 100
  const costPercent =
    mission.budget_cost_microusd > 0
      ? Math.min(
          100,
          Math.max(
            0,
            (consumedCostMicrousd / mission.budget_cost_microusd) * 100,
          ),
        )
      : 100

  return (
    <section
      className={`budget-ledger${budgetExhausted ? ' budget-ledger-exhausted' : ''}`}
      data-testid="mission-budget-ledger"
      data-budget-exhausted={budgetExhausted}
    >
      <div className="budget-ledger-heading">
        <div>
          <span>Mission budget authority</span>
          <strong>
            {budgetExhausted
              ? 'Recovery authorization required'
              : pendingRevision
                ? 'Revision awaiting decision'
                : 'Authorized capacity'}
          </strong>
        </div>
        <span className={`budget-state budget-state-${budgetExhausted ? 'exhausted' : 'available'}`}>
          {budgetExhausted ? 'Exhausted' : 'Available'}
        </span>
      </div>

      <div className="budget-meters" aria-label="Mission budget use">
        <div>
          <span>Tokens</span>
          <div className="budget-meter" aria-hidden="true">
            <span style={{ width: `${tokenPercent}%` }} />
          </div>
        </div>
        <div>
          <span>Cost</span>
          <div className="budget-meter budget-meter-cost" aria-hidden="true">
            <span style={{ width: `${costPercent}%` }} />
          </div>
        </div>
      </div>

      <div className="budget-metrics">
        <div>
          <span>Consumed</span>
          <strong>{consumedTokens.toLocaleString()} tokens</strong>
          <small>{formatUsd(consumedCostMicrousd)}</small>
        </div>
        <div>
          <span>Original</span>
          <strong>{mission.original_budget_tokens.toLocaleString()} tokens</strong>
          <small>{formatUsd(mission.original_budget_cost_microusd)}</small>
        </div>
        <div>
          <span>Current</span>
          <strong>{mission.budget_tokens.toLocaleString()} tokens</strong>
          <small>{formatUsd(mission.budget_cost_microusd)}</small>
        </div>
        <div>
          <span>Remaining</span>
          <strong>{budgetRemainingLabel(remainingTokens, 'tokens')}</strong>
          <small>{budgetRemainingLabel(remainingCostMicrousd, 'cost')}</small>
        </div>
      </div>

      {latestApprovedRevision ? (
        <div className="budget-approval-proof" data-testid="budget-approval-proof">
          <span>Current ceiling approved by</span>
          <strong>{actorName(latestApprovedRevision.decided_by)}</strong>
          <small>
            {latestApprovedRevision.decided_at
              ? time(latestApprovedRevision.decided_at)
              : 'Decision time unavailable'}
            {' · '}
            {latestApprovedRevision.decision_note}
          </small>
        </div>
      ) : (
        <div className="budget-approval-proof">
          <span>Current ceiling</span>
          <strong>Original mission authorization</strong>
        </div>
      )}

      {pendingRevision ? (
        <div className="budget-pending" data-testid="budget-revision-pending">
          <div>
            <span>Pending change order · v{pendingRevision.version}</span>
            <strong>
              {pendingRevision.current_budget_tokens.toLocaleString()} →{' '}
              {pendingRevision.proposed_budget_tokens.toLocaleString()} tokens
            </strong>
            <small>
              {formatUsd(pendingRevision.current_budget_cost_microusd)} →{' '}
              {formatUsd(pendingRevision.proposed_budget_cost_microusd)}
              {' · proposed by '}
              {actorName(pendingRevision.proposed_by)}
            </small>
            <p>{pendingRevision.rationale}</p>
            {pendingRevision.replacement_task_id ? (
              <small>
                Also narrows task {shortId(pendingRevision.replacement_task_id)} without changing
                its verifier policy.
              </small>
            ) : null}
          </div>
          <div className="budget-decision-actions">
            <button
              className="button button-primary"
              type="button"
              disabled={busy || !canManageBudget}
              onClick={() =>
                void onDecision(
                  mission,
                  pendingRevision,
                  true,
                  decisionKey(pendingRevision, true),
                )
              }
            >
              Approve revision
            </button>
            <button
              className="button button-danger"
              type="button"
              disabled={busy || !canManageBudget}
              onClick={() =>
                void onDecision(
                  mission,
                  pendingRevision,
                  false,
                  decisionKey(pendingRevision, false),
                )
              }
            >
              Reject revision
            </button>
          </div>
          {!canManageBudget ? (
            <small className="budget-guidance">
              Switch to an owner or admin to decide this revision.
            </small>
          ) : null}
        </div>
      ) : null}

      {budgetExhausted && !pendingRevision && canManageBudget && canReviseNow ? (
        proposalOpen ? (
          <form
            className="budget-proposal"
            data-testid="budget-revision-form"
            onSubmit={submitProposal}
          >
            <div className="budget-proposal-heading">
              <div>
                <span>Authorized recovery</span>
                <strong>Propose a new mission ceiling</strong>
              </div>
              <button
                type="button"
                onClick={() => setProposalOpen(false)}
                aria-label="Close budget revision form"
              >
                ×
              </button>
            </div>
            <div className="budget-proposal-grid">
              <label>
                Token ceiling
                <input
                  type="number"
                  min={Math.max(mission.budget_tokens, consumedTokens + 1)}
                  max={20_000_000}
                  step={1}
                  value={proposedTokens}
                  onChange={(event) => setProposedTokens(Number(event.target.value))}
                />
              </label>
              <label>
                Cost ceiling · USD
                <input
                  type="number"
                  min={
                    Math.ceil(
                      Math.max(
                        mission.budget_cost_microusd,
                        consumedCostMicrousd + 1,
                      ) / 10_000,
                    ) / 100
                  }
                  max={100}
                  step={0.01}
                  value={proposedCostUsd}
                  onChange={(event) => setProposedCostUsd(Number(event.target.value))}
                />
              </label>
            </div>
            <label>
              Authorization rationale
              <textarea
                rows={3}
                maxLength={4_000}
                value={rationale}
                onChange={(event) => setRationale(event.target.value)}
                placeholder="Why is more budget justified, and what must finish?"
              />
            </label>
            <label className="mission-run-toggle budget-scope-toggle">
              <input
                type="checkbox"
                checked={replaceFinishScope}
                disabled={!finishTasks.length}
                onChange={(event) => setReplaceFinishScope(event.target.checked)}
              />
              <span>
                <strong>Narrow the remaining task</strong>
                <small>
                  Replace its objective, output, write paths, and budget while retaining the
                  existing verification policy.
                </small>
              </span>
            </label>
            {replaceFinishScope ? (
              <div className="finish-scope-fields" data-testid="finish-scope-fields">
                <label>
                  Task
                  <select
                    value={finishTaskId}
                    onChange={(event) => {
                      const task = finishTasks.find(
                        (candidate) => candidate.id === event.target.value,
                      )
                      seedFinishTask(task, proposedTokens, proposedCostMicrousd)
                    }}
                  >
                    {finishTasks.map((task) => (
                      <option key={task.id} value={task.id}>
                        {task.plan_key} · {statusLabel(task.status)}
                      </option>
                    ))}
                  </select>
                </label>
                <label>
                  Bounded finish objective
                  <textarea
                    rows={3}
                    value={finishObjective}
                    onChange={(event) => setFinishObjective(event.target.value)}
                  />
                </label>
                <label>
                  Expected output
                  <textarea
                    rows={2}
                    maxLength={10_000}
                    value={finishExpectedOutput}
                    onChange={(event) => setFinishExpectedOutput(event.target.value)}
                  />
                </label>
                <label>
                  Acceptance tests · one per line
                  <textarea
                    rows={3}
                    value={finishAcceptanceTests}
                    onChange={(event) => setFinishAcceptanceTests(event.target.value)}
                  />
                </label>
                <label>
                  Authorized write paths · one per line
                  <textarea
                    rows={3}
                    value={finishWriteScope}
                    onChange={(event) => setFinishWriteScope(event.target.value)}
                  />
                </label>
                <div className="budget-proposal-grid">
                  <label>
                    Finish token budget
                    <input
                      type="number"
                      min={1}
                      max={Math.min(
                        selectedFinishTask?.contract.budget_tokens ?? 2_000_000,
                        Math.max(1, proposedRemainingTokens),
                      )}
                      step={1}
                      value={finishBudgetTokens}
                      onChange={(event) =>
                        setFinishBudgetTokens(Number(event.target.value))
                      }
                    />
                  </label>
                  <label>
                    Finish cost budget · USD
                    <input
                      type="number"
                      min={0.01}
                      max={
                        Math.min(
                          selectedFinishTask?.contract.budget_cost_microusd ??
                            10_000_000,
                          Math.max(1, proposedRemainingCostMicrousd),
                        ) / 1_000_000
                      }
                      step={0.01}
                      value={finishBudgetCostUsd}
                      onChange={(event) =>
                        setFinishBudgetCostUsd(Number(event.target.value))
                      }
                    />
                  </label>
                </div>
                <small>
                  The server rejects wider write paths, larger task budgets, or any verifier-policy
                  change.
                </small>
              </div>
            ) : null}
            <div className="budget-proposal-footer">
              <small className={proposalErrors.length ? 'budget-error' : ''}>
                {proposalErrors[0] ??
                  `${proposedRemainingTokens.toLocaleString()} tokens and ${formatUsd(proposedRemainingCostMicrousd)} would remain after recorded usage.`}
              </small>
              <button
                className="button button-primary"
                type="submit"
                disabled={busy || proposalErrors.length > 0}
              >
                Propose revision
              </button>
            </div>
          </form>
        ) : (
          <button
            className="button button-secondary budget-revision-open"
            type="button"
            disabled={busy}
            onClick={openProposal}
          >
            Authorize recovery budget
          </button>
        )
      ) : null}

      {budgetExhausted && !canManageBudget ? (
        <p className="budget-guidance">
          {resumableRun?.breaker_stage === 'stop'
            ? 'A stop-stage breaker is terminal and cannot be overridden. Create a new bounded mission from the preserved evidence.'
            : 'Resume is locked. An owner or admin must approve a higher mission ceiling before another provider session starts.'}
        </p>
      ) : null}
      {budgetExhausted && canManageBudget && !canReviseNow && !pendingRevision ? (
        <p className="budget-guidance">
          {resumableRun?.breaker_stage === 'stop'
            ? 'A stop-stage breaker is terminal and cannot be overridden. Create a new bounded mission from the preserved evidence.'
            : 'Recovery becomes available after the active run stops at a resumable budget suspension.'}
        </p>
      ) : null}

      {orderedRevisions.length ? (
        <details className="budget-history">
          <summary>{orderedRevisions.length} recorded budget revision{orderedRevisions.length === 1 ? '' : 's'}</summary>
          <ol>
            {orderedRevisions.map((revision) => (
              <li key={revision.id}>
                <div>
                  <span className={`budget-history-status budget-history-${revision.status}`}>
                    {statusLabel(revision.status)}
                  </span>
                  <strong>
                    {revision.proposed_budget_tokens.toLocaleString()} tokens ·{' '}
                    {formatUsd(revision.proposed_budget_cost_microusd)}
                  </strong>
                </div>
                <p>{revision.rationale}</p>
                <small>
                  Proposed by {actorName(revision.proposed_by)} at {time(revision.created_at)}
                  {' · consumed '}
                  {revision.consumed_tokens_at_proposal.toLocaleString()} tokens /{' '}
                  {formatUsd(revision.consumed_cost_microusd_at_proposal)}
                </small>
                {revision.decided_by ? (
                  <small>
                    Decided by {actorName(revision.decided_by)}
                    {revision.decided_at ? ` at ${time(revision.decided_at)}` : ''}
                    {revision.decision_note ? ` · ${revision.decision_note}` : ''}
                  </small>
                ) : null}
              </li>
            ))}
          </ol>
        </details>
      ) : null}
    </section>
  )
}

function MissionCard({
  mission,
  tasks,
  runs,
  agents,
  evidence,
  deliverables,
  revisions,
  contractRevisions,
  actors,
  verificationRequests,
  actionApprovals,
  actorId,
  actorRole,
  busy,
  onLaunch,
  onResume,
  onDownloadArtifact,
  onDownloadDeliverable,
  onProposeBudgetRevision,
  onBudgetRevisionDecision,
  onContractRevision,
  onVerificationDecision,
  onActionApprovalDecision,
}: {
  mission: Mission
  tasks: Task[]
  runs: Run[]
  agents: Agent[]
  evidence: VerificationEvidence[]
  deliverables: SourceDeliverable[]
  revisions: MissionBudgetRevision[]
  contractRevisions: MissionContractRevision[]
  actors: Actor[]
  verificationRequests: VerificationRequest[]
  actionApprovals: ActionApproval[]
  actorId: string
  actorRole: string
  busy: boolean
  onLaunch: (mission: Mission) => Promise<void>
  onResume: (run: Run) => Promise<void>
  onDownloadArtifact: (run: Run) => Promise<void>
  onDownloadDeliverable: (deliverable: SourceDeliverable) => Promise<void>
  onProposeBudgetRevision: (
    mission: Mission,
    input: MissionBudgetRevisionInput,
  ) => Promise<boolean>
  onBudgetRevisionDecision: (
    mission: Mission,
    revision: MissionBudgetRevision,
    approved: boolean,
    decisionKey: string,
  ) => Promise<void>
  onContractRevision: (
    mission: Mission,
    task: Task,
    input: MissionContractRevisionInput,
  ) => Promise<boolean>
  onVerificationDecision: (run: Run, approved: boolean) => Promise<void>
  onActionApprovalDecision: (approval: ActionApproval, approved: boolean) => Promise<void>
}) {
  const orderedTasks = tasks.toSorted((left, right) =>
    left.depth - right.depth || left.plan_key.localeCompare(right.plan_key),
  )
  const taskById = new Map(tasks.map((task) => [task.id, task]))
  const latestRun = runs[0]
  const completedTasks = tasks.filter((task) => task.status === 'completed').length
  const activeRuns = runs.filter((run) =>
    ['provisioning', 'starting', 'running', 'waiting_for_input', 'waiting_for_approval', 'verifying'].includes(run.status),
  ).length
  const consumedTokens = runs.reduce(
    (total, run) => total + run.input_tokens + run.output_tokens,
    0,
  )
  const consumedCostMicrousd = runs.reduce(
    (total, run) => total + run.cost_microusd,
    0,
  )
  const resumeBudgetBlocked =
    consumedTokens >= mission.budget_tokens ||
    consumedCostMicrousd >= mission.budget_cost_microusd
  const pendingBudgetRevision = revisions.find(
    (revision) => revision.status === 'pending',
  )
  const resumableRun = runs.find(
    (run) =>
      run.provider_session_id &&
      run.workspace_disposition === 'preserved' &&
      terminalRun(run.status),
  )
  const pendingRun = runs.find((run) => run.status === 'waiting_for_approval')
  const resumeStopBlocked = resumableRun?.breaker_stage === 'stop'
  const pendingRequest = pendingRun
    ? verificationRequests.find(
        (request) => request.run_id === pendingRun.id && request.status === 'pending',
      )
    : undefined
  const runIds = new Set(runs.map((run) => run.id))
  const pendingActionApprovals = actionApprovals.filter(
    (approval) => runIds.has(approval.run_id) && approval.status === 'pending',
  )
  const latestEvidence = latestRun
    ? evidence
        .filter((item) => item.run_id === latestRun.id)
        .toSorted((left, right) => left.check_index - right.check_index)
    : []
  const latestDeliverable = latestRun
    ? deliverables.find((deliverable) => deliverable.run_id === latestRun.id)
    : undefined
  const terminalSummary =
    latestRun && terminalRun(latestRun.status)
      ? latestRun.summary ?? latestRun.verification_summary
      : null
  return (
    <article
      className="mission-card"
      data-testid={`mission-${mission.id}`}
      data-mission-id={mission.id}
      data-run-id={latestRun?.id}
      tabIndex={-1}
    >
      <div className="mission-card-top">
        <span className={`status-chip status-chip-${mission.status}`}>{statusLabel(mission.status)}</span>
        <span className="mission-id">#{shortId(mission.id)}</span>
      </div>
      <h3>{mission.title}</h3>
      {mission.description ? (
        <details className="mission-dossier mission-briefing">
          <summary>
            <span>Mission briefing</span>
            <small>Open full specification</small>
          </summary>
          <p className="mission-description">{mission.description}</p>
        </details>
      ) : null}
      <div className="mission-chip-row">
        <div className="strategy-chip">{statusLabel(mission.strategy)}</div>
        <div className="contract-version-chip">Specification v{mission.specification_version}</div>
      </div>
      <dl>
        <div>
          <dt>Tasks</dt>
          <dd>{completedTasks}/{tasks.length} complete</dd>
        </div>
        <div>
          <dt>Runs</dt>
          <dd>{activeRuns ? `${activeRuns} active` : `${runs.length} attempts`}</dd>
        </div>
      </dl>
      <details
        className="mission-dossier"
        open={resumeBudgetBlocked || Boolean(pendingBudgetRevision)}
      >
        <summary>
          <span>Budget authority</span>
          <small>
            {consumedTokens.toLocaleString()} of {mission.budget_tokens.toLocaleString()} tokens
          </small>
        </summary>
        <BudgetRevisionPanel
          mission={mission}
          tasks={tasks}
          runs={runs}
          revisions={revisions}
          actors={actors}
          actorRole={actorRole}
          busy={busy}
          resumableRun={resumableRun}
          onPropose={onProposeBudgetRevision}
          onDecision={onBudgetRevisionDecision}
        />
      </details>
      <details className="mission-dossier mission-task-dossier">
        <summary>
          <span>Task graph</span>
          <small>{completedTasks}/{tasks.length} complete</small>
        </summary>
        <div className="task-graph-list">
        {orderedTasks.map((task) => {
          const assignedAgent = agents.find((agent) => agent.id === task.assigned_agent_id)
          const dependencies = task.depends_on
            .map((dependencyId) => taskById.get(dependencyId)?.plan_key ?? shortId(dependencyId))
          return (
            <details
              className="task-graph-item"
              key={task.id}
              data-task-id={task.id}
              tabIndex={-1}
            >
              <summary className="task-graph-row">
                <span>{task.plan_key}</span>
                <strong>{statusLabel(task.status)}</strong>
                <small>d{task.depth} · {task.attempt_count}/{task.max_attempts}</small>
              </summary>
              <div className="task-contract">
                <div className="task-contract-heading">
                  <p>{task.objective}</p>
                  <span>Contract v{task.contract_version}</span>
                </div>
                <dl>
                  <div><dt>Agent</dt><dd>{assignedAgent?.name ?? 'Unassigned'} · {adapterLabel(task.required_adapter ?? assignedAgent?.adapter ?? 'unknown')}</dd></div>
                  <div><dt>Depends on</dt><dd>{dependencies.length ? dependencies.join(', ') : 'Nothing; ready independently'}</dd></div>
                  <div><dt>Expected output</dt><dd>{task.contract.expected_output}</dd></div>
                  <div>
                    <dt>Repository</dt>
                    <dd>
                      {task.contract.source_repository ?? 'Runner default'}
                      {task.contract.source_base_ref
                        ? ` @ ${task.contract.source_base_ref}`
                        : ''}
                      {task.contract.source_base_commit
                        ? ` (${task.contract.source_base_commit.slice(0, 12)})`
                        : ''}
                    </dd>
                  </div>
                  <div><dt>Budget</dt><dd>{task.contract.budget_tokens.toLocaleString()} tokens</dd></div>
                  <div><dt>Write scope</dt><dd>{task.contract.write_scope.length ? task.contract.write_scope.join(', ') : 'No repository writes declared'}</dd></div>
                  <div><dt>Allowed tools</dt><dd>{task.contract.allowed_tools.join(', ')}</dd></div>
                  <div><dt>References</dt><dd>{task.contract.references.length ? task.contract.references.join(', ') : 'None attached'}</dd></div>
                  <div>
                    <dt>Deliverable</dt>
                    <dd>
                      {task.contract.deliverable
                        ? statusLabel(task.contract.deliverable.form)
                        : 'Provider evidence only'}
                      {task.contract.deliverable?.commit_after_verification
                        ? ' · commit after verification'
                        : ''}
                    </dd>
                  </div>
                </dl>
                <strong>Acceptance checks</strong>
                <ul>
                  {task.contract.acceptance_tests.map((test) => <li key={test}>{test}</li>)}
                </ul>
                <strong>Prohibited actions</strong>
                <ul>
                  {task.contract.prohibited_actions.map((action) => (
                    <li key={action}>{action}</li>
                  ))}
                </ul>
                <VerificationPolicyPreview
                  policy={task.verification_policy}
                  heading="Exact completion plan"
                />
                <ContractRevisionPanel
                  mission={mission}
                  task={task}
                  runs={runs}
                  actorId={actorId}
                  actorRole={actorRole}
                  busy={busy}
                  onRevise={onContractRevision}
                />
              </div>
            </details>
          )
        })}
      </div>
      {contractRevisions.length ? (
        <details className="contract-revision-history" data-testid="contract-revision-history">
          <summary>
            {contractRevisions.length} contract revision
            {contractRevisions.length === 1 ? '' : 's'}
          </summary>
          <ol>
            {contractRevisions
              .toSorted(
                (left, right) =>
                  right.version - left.version ||
                  right.created_at.localeCompare(left.created_at),
              )
              .map((revision) => {
                const task = taskById.get(revision.task_id)
                const actor = actors.find((candidate) => candidate.id === revision.revised_by)
                return (
                  <li key={revision.id}>
                    <div>
                      <strong>
                        v{revision.version} · {statusLabel(revision.next_action)}
                      </strong>
                      <span>{task?.plan_key ?? shortId(revision.task_id)}</span>
                    </div>
                    <p>{revision.reason}</p>
                    <small>
                      {actor?.name ?? shortId(revision.revised_by)} · {time(revision.created_at)}
                      {revision.source_run_id
                        ? ` · source run ${shortId(revision.source_run_id)}`
                        : ''}
                    </small>
                  </li>
                )
              })}
          </ol>
        </details>
      ) : null}
      </details>
      {latestRun?.artifact_sha256 && latestRun.artifact_uri ? (
        <div
          className="evidence-box evidence-provider"
          data-testid="provider-evidence"
          data-artifact-id={latestRun.artifact_id}
        >
          <strong>Provider evidence</strong>
          <span>{shortId(latestRun.artifact_sha256)}…</span>
          <button
            type="button"
            className="artifact-download"
            onClick={() => void onDownloadArtifact(latestRun)}
          >
            {latestRun.verification_status === 'passed'
              ? 'Download verified artifact'
              : 'Download submitted artifact'}
          </button>
        </div>
      ) : null}
      {latestDeliverable ? (
        <div className="evidence-box evidence-source" data-testid="source-deliverable">
          <strong>Source deliverable · {statusLabel(latestDeliverable.form)}</strong>
          <span>{latestDeliverable.file_name} · {latestDeliverable.bytes.toLocaleString()} bytes</span>
          <small>
            Verification {shortId(latestDeliverable.verification_sha256)}… · bytes {shortId(latestDeliverable.sha256)}…
          </small>
          <button
            type="button"
            className="artifact-download"
            onClick={() => void onDownloadDeliverable(latestDeliverable)}
          >
            Download source deliverable
          </button>
        </div>
      ) : null}
      {latestRun &&
      (latestRun.verification_status !== 'pending' || latestEvidence.length > 0) ? (
        <details
          className={`verification-box verification-${latestRun.verification_status}`}
          data-testid="verification-evidence"
          open={latestRun.verification_status === 'failed'}
        >
          <summary>
            <strong>{statusLabel(latestRun.verification_status)}</strong>
            <span>
              {latestEvidence.filter((item) => item.status === 'passed').length}/
              {latestEvidence.length} checks passed
            </span>
          </summary>
          {latestEvidence.length ? (
            <ol className="evidence-checks">
              {latestEvidence.map((item) => (
                <li className={`evidence-check evidence-check-${item.status}`} key={item.id}>
                  <strong>{statusLabel(item.kind)}</strong>
                  <span>{statusLabel(item.status)}</span>
                  <p>{item.summary}</p>
                </li>
              ))}
            </ol>
          ) : null}
        </details>
      ) : null}
      {latestRun && terminalRun(latestRun.status) ? (
        <div className={`terminal-summary terminal-${latestRun.status}`}>
          <strong>{statusLabel(latestRun.status)}</strong>
          <p>{terminalSummary ?? 'The run ended without a summary.'}</p>
          <small>
            Worktree: {latestRun.workspace_disposition
              ? statusLabel(latestRun.workspace_disposition)
              : 'cleanup pending'}
            {latestRun.workspace_detail ? ` · ${latestRun.workspace_detail}` : ''}
          </small>
        </div>
      ) : null}
      {latestRun && (latestRun.input_tokens > 0 || latestRun.output_tokens > 0) ? (
        <div className="usage-box">
          {latestRun.input_tokens.toLocaleString()} in · {latestRun.output_tokens.toLocaleString()} out
        </div>
      ) : null}
      {latestRun?.model ? (
        <div className="usage-box">
          {latestRun.model}
          {latestRun.reasoning_effort ? ` · ${latestRun.reasoning_effort} reasoning` : ''}
        </div>
      ) : null}
      {latestRun?.workspace_branch ? (
        <div className="workspace-box" title={latestRun.workspace_detail ?? undefined}>
          <span>{latestRun.workspace_disposition ?? 'active'} worktree</span>
          <strong>{latestRun.workspace_branch}</strong>
        </div>
      ) : null}
      {latestDeliverable ? (
        <div className="workspace-box integration-box" data-testid="integration-state">
          <span>Integration · {statusLabel(latestDeliverable.integration_state)}</span>
          <strong>
            {latestDeliverable.head_commit
              ? `${latestDeliverable.branch} @ ${shortId(latestDeliverable.head_commit)}`
              : 'No publication or merge requested'}
          </strong>
          <small>Pull-request publication and merge require separate authorization.</small>
        </div>
      ) : null}
      {mission.status === 'ready' ? (
        <button className="button button-primary mission-launch" type="button" disabled={busy} onClick={() => onLaunch(mission)}>
          Dispatch mission
        </button>
      ) : null}
      {pendingActionApprovals.length ? (
        <div className="mission-approval-list">
          <strong>
            {pendingActionApprovals.length} action
            {pendingActionApprovals.length === 1 ? '' : 's'} need your approval
          </strong>
          {pendingActionApprovals.map((approval) => {
            const canDecide = approval.required_roles.includes(actorRole)
            return (
              <div className="mission-approval-item" key={approval.id}>
                <span>{approval.action}</span>
                <small>{approval.risk} risk · {approval.rationale}</small>
                <div>
                  <button
                    className="button button-primary"
                    type="button"
                    disabled={busy || !canDecide}
                    onClick={() => void onActionApprovalDecision(approval, true)}
                  >
                    Approve action
                  </button>
                  <button
                    className="button"
                    type="button"
                    disabled={busy || !canDecide}
                    onClick={() => void onActionApprovalDecision(approval, false)}
                  >
                    Reject
                  </button>
                </div>
              </div>
            )
          })}
        </div>
      ) : null}
      {resumableRun && activeRuns === 0 ? (
        <>
          <button
            className="button button-secondary mission-launch"
            type="button"
            disabled={
              busy ||
              resumeStopBlocked ||
              resumeBudgetBlocked ||
              Boolean(pendingBudgetRevision)
            }
            onClick={() => onResume(resumableRun)}
          >
            {resumeStopBlocked
              ? 'Stop-stage run cannot resume'
              : pendingBudgetRevision
                ? 'Budget decision required'
                : resumeBudgetBlocked
                  ? 'Budget revision required'
                : 'Resume agent session'}
          </button>
          {resumeStopBlocked ? (
            <p className="budget-resume-guidance" data-testid="budget-resume-guidance">
              This run crossed a stop-stage safety boundary. Start a new bounded mission from its
              preserved evidence instead of resuming the provider session.
            </p>
          ) : pendingBudgetRevision ? (
            <p className="budget-resume-guidance" data-testid="budget-resume-guidance">
              Decide the pending budget revision before starting another provider run.
            </p>
          ) : resumeBudgetBlocked ? (
            <p className="budget-resume-guidance" data-testid="budget-resume-guidance">
              This mission has no authorized budget remaining. Approve a revision above, then resume
              the preserved provider session and worktree.
            </p>
          ) : null}
        </>
      ) : null}
      {pendingRun && pendingRequest ? (
        <div className="verification-actions">
          {(() => {
            const requiredRoles = pendingRequest.gate.roles
            const requesterExcluded =
              pendingRequest.gate.type === 'independent_review' &&
              pendingRequest.gate.exclude_requester &&
              mission.requested_by === actorId
            const canDecide = requiredRoles.includes(actorRole) && !requesterExcluded
            return (
              <>
                <span>
                  {statusLabel(pendingRequest.gate_type)} · eligible: {requiredRoles.join(', ')}
                </span>
                {!canDecide ? (
                  <small>
                    {requesterExcluded
                      ? 'The requester cannot approve an independent review. Switch operator.'
                      : `Your ${actorRole} role is not eligible for this gate.`}
                  </small>
                ) : null}
          <button
            className="button button-primary"
            type="button"
                  disabled={busy || !canDecide}
            onClick={() => onVerificationDecision(pendingRun, true)}
          >
            Approve evidence
          </button>
          <button
            className="button button-danger"
            type="button"
                  disabled={busy || !canDecide}
            onClick={() => onVerificationDecision(pendingRun, false)}
          >
            Reject evidence
          </button>
              </>
            )
          })()}
        </div>
      ) : null}
    </article>
  )
}

function EventRow({ event, actors }: { event: DomainEvent; actors: Actor[] }) {
  const actor = actors.find((candidate) => candidate.id === event.actor_id)
  const label = event.type.replaceAll('.', ' / ')
  const detail =
    typeof event.payload.message === 'string'
      ? event.payload.message
      : typeof event.payload.summary === 'string'
        ? event.payload.summary
        : typeof event.payload.title === 'string'
          ? event.payload.title
          : event.aggregate_type

  return (
    <li className="event-row">
      <span className="event-seq">{String(event.seq).padStart(4, '0')}</span>
      <span className="event-type">{label}</span>
      <span className="event-detail">{detail}</span>
      <span className="event-actor">{actor?.name ?? 'system'}</span>
      <time dateTime={event.created_at}>{time(event.created_at)}</time>
    </li>
  )
}

function RoomPanel({
  room,
  messages,
  actors,
  selectedActor,
  missions,
  tasks,
  runs,
  onPost,
  onNavigateLink,
}: {
  room: { id: string; name: string; purpose: string } | undefined
  messages: RoomMessage[]
  actors: Actor[]
  selectedActor: Actor
  missions: Mission[]
  tasks: Task[]
  runs: Run[]
  onPost: (input: {
    roomId: string
    body: string
    replyToId: string | null
    mentions: string[]
    link: EntityLink | null
    idempotencyKey: string
  }) => Promise<boolean>
  onNavigateLink: (link: EntityLink) => void
}) {
  const [body, setBody] = useState('')
  const [replyToId, setReplyToId] = useState<string | null>(null)
  const [linkValue, setLinkValue] = useState('')

  if (!room) {
    return (
      <section className="room-panel panel" id="room" tabIndex={-1}>
        <div className="panel-heading">
          <div>
            <span className="section-code">Project room</span>
            <h2>No room access</h2>
            <p>{selectedActor.name} is not a member of this project room.</p>
          </div>
        </div>
        <div className="room-denied">Room messages and room-scoped work are hidden.</div>
      </section>
    )
  }

  const linkedOptions = [
    ...missions.slice(0, 2).map((mission) => ({
      value: `mission:${mission.id}`,
      label: `Mission · ${mission.title}`,
    })),
    ...tasks.slice(0, 2).map((task) => ({
      value: `task:${task.id}`,
      label: `Task · ${task.title}`,
    })),
    ...runs.slice(0, 2).flatMap((run) => [
      { value: `run:${run.id}`, label: `Run · ${shortId(run.id)} · ${run.status}` },
      ...(run.artifact_sha256 && run.artifact_id
        ? [{
            value: `artifact:${run.artifact_id}`,
            label: `Artifact · ${shortId(run.artifact_sha256)}`,
          }]
        : []),
    ]),
  ]
  const visibleMessages = messages.slice(-40)
  const replyTarget = replyToId
    ? messages.find((message) => message.id === replyToId)
    : undefined

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    if (!body.trim()) return
    const mentionedNames = Array.from(body.matchAll(/@([A-Za-z0-9_-]+)/g), (match) =>
      match[1].toLowerCase(),
    )
    const mentions = actors
      .filter((actor) => mentionedNames.includes(actor.name.toLowerCase()))
      .map((actor) => actor.id)
    const [kind, id] = linkValue.split(':')
    const link =
      kind && id
        ? ({ kind, id } as EntityLink)
        : null
    const payload = JSON.stringify({
      roomId: room.id,
      body: body.trim(),
      replyToId,
      mentions,
      link,
    })
    const operationStorageKey = `ecorp:room-message:${room.id}:${selectedActor.id}`
    const idempotencyKey = browserOperationKey(operationStorageKey, payload)
    const saved = await onPost({
      roomId: room.id,
      body: body.trim(),
      replyToId,
      mentions,
      link,
      idempotencyKey,
    })
    if (saved) {
      clearBrowserOperation(operationStorageKey)
      setBody('')
      setReplyToId(null)
      setLinkValue('')
    }
  }

  return (
    <section
      className="room-panel panel"
      id="room"
      data-room-id={room.id}
      tabIndex={-1}
    >
      <div className="panel-heading">
        <div>
          <span className="section-code">Project room</span>
          <h2>{room.name} channel</h2>
          <p>Every human and agent message is durable, attributable, and linked to the work.</p>
        </div>
        <div className="room-count">{messages.length} messages</div>
      </div>
      <div className="room-layout">
        <ol className="room-message-list" data-testid="room-message-list">
          {visibleMessages.length ? (
            visibleMessages.map((message) => {
              const author = actors.find((actor) => actor.id === message.actor_id)
              const linked = message.link
              return (
                <li
                  key={message.id}
                  className={`room-message ${message.thread_root_id ? 'room-reply' : ''}`}
                >
                  <div className="room-message-head">
                    <strong>{author?.name ?? 'Unknown actor'}</strong>
                    <span>{author?.role ?? 'member'}</span>
                    <time dateTime={message.created_at}>{time(message.created_at)}</time>
                  </div>
                  <p>{message.body}</p>
                  <div className="room-message-foot">
                    <div>
                      {message.mentions.map((actorId) => {
                        const mentioned = actors.find((actor) => actor.id === actorId)
                        return (
                          <span className="mention-chip" key={actorId}>
                            @{mentioned?.name ?? shortId(actorId)}
                          </span>
                        )
                      })}
                      {linked ? (
                        <button
                          type="button"
                          className="entity-link"
                          onClick={() => onNavigateLink(linked)}
                        >
                          {statusLabel(linked.kind)} · {shortId(linked.id)}
                        </button>
                      ) : null}
                    </div>
                    <button type="button" onClick={() => setReplyToId(message.id)}>
                      Reply
                    </button>
                  </div>
                </li>
              )
            })
          ) : (
            <li className="empty-state">
              <strong>The room is quiet</strong>
              <span>Post the first durable message.</span>
            </li>
          )}
        </ol>
        <form className="room-composer" onSubmit={submit}>
          <label htmlFor="room-message">Post as {selectedActor.name}</label>
          {replyTarget ? (
            <div className="reply-context">
              Replying to {actors.find((actor) => actor.id === replyTarget.actor_id)?.name ?? 'message'}
              <button type="button" onClick={() => setReplyToId(null)}>Cancel</button>
            </div>
          ) : null}
          <textarea
            id="room-message"
            value={body}
            onChange={(event) => setBody(event.target.value)}
            placeholder="Write a message. Mention a colleague with @Name."
            rows={5}
          />
          <label htmlFor="room-link">Structured link</label>
          <select id="room-link" value={linkValue} onChange={(event) => setLinkValue(event.target.value)}>
            <option value="">No linked work item</option>
            {linkedOptions.map((option) => (
              <option key={option.value} value={option.value}>{option.label}</option>
            ))}
          </select>
          <button className="button button-primary" type="submit" disabled={!body.trim()}>
            Post to room
          </button>
        </form>
      </div>
    </section>
  )
}

function App() {
  const [bootstrap, setBootstrap] = useState<BootstrapResponse | null>(null)
  const [data, setData] = useState<SnapshotResponse | null>(null)
  const [selectedActorId, setSelectedActorId] = useState<string | null>(null)
  const [missionTitle, setMissionTitle] = useState(DEFAULT_MISSION)
  const [missionDescription, setMissionDescription] = useState('')
  const [missionObjective, setMissionObjective] = useState('')
  const [missionExpectedOutput, setMissionExpectedOutput] = useState('')
  const [missionAcceptanceTests, setMissionAcceptanceTests] = useState('')
  const [missionAllowedTools, setMissionAllowedTools] = useState('')
  const [missionProhibitedActions, setMissionProhibitedActions] = useState('')
  const [missionReferences, setMissionReferences] = useState('')
  const [missionWriteScope, setMissionWriteScope] = useState('')
  const [customVerification, setCustomVerification] = useState(false)
  const [missionVerificationPolicy, setMissionVerificationPolicy] =
    useState<VerificationPolicy>(() => ({
      checks: [defaultVerifierCheck('artifact')],
      manual_gate: null,
    }))
  const [missionComposerStep, setMissionComposerStep] = useState<
    'brief' | 'loadout' | 'proof'
  >('brief')
  const [missionComposerCollapsed, setMissionComposerCollapsed] = useState(false)
  const [missionContractTab, setMissionContractTab] = useState<
    'outcome' | 'guardrails' | 'context'
  >('outcome')
  const [missionAdapter, setMissionAdapter] = useState('')
  const [missionModel, setMissionModel] = useState('')
  const [missionReasoningEffort, setMissionReasoningEffort] = useState('')
  const [missionStrategy, setMissionStrategy] = useState('single')
  const [missionSourceKey, setMissionSourceKey] = useState('')
  const [missionSourceConfirmed, setMissionSourceConfirmed] = useState(false)
  const [missionBudgetTokens, setMissionBudgetTokens] = useState(1_000_000)
  const [missionDeliverable, setMissionDeliverable] =
    useState<NonNullable<TaskContract['deliverable']>['form']>('archive')
  const [commitDeliverable, setCommitDeliverable] = useState(false)
  const [pauseAfterPlanning, setPauseAfterPlanning] = useState(false)
  const [developerMode, setDeveloperMode] = useState(false)
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null)
  const [floorInspectorOpen, setFloorInspectorOpen] = useState(false)
  const [selectedMissionId, setSelectedMissionId] = useState<string | null>(null)
  const [activeWorkspaceView, setActiveWorkspaceView] = useState<WorkspaceView>(() =>
    workspaceViewFromHash(window.location.hash),
  )
  const [journeyOpen, setJourneyOpen] = useState(false)
  const [connection, setConnection] = useState<'connecting' | 'live' | 'offline'>('connecting')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [announcement, setAnnouncement] = useState('Operations console loading.')
  const [requiresConnection, setRequiresConnection] = useState(false)
  const [connectionCorpId, setConnectionCorpId] = useState(
    () => window.sessionStorage.getItem('ecorp_corp_id') ?? '',
  )
  const [connectionActorId, setConnectionActorId] = useState(
    () => window.sessionStorage.getItem('ecorp_actor_id') ?? '',
  )
  const [connectionToken, setConnectionToken] = useState('')
  const [leaseTokens, setLeaseTokens] = useState<Record<string, string>>({})
  const reconnectTimer = useRef<number | null>(null)
  const composerInitialized = useRef(false)
  const initialWorkspaceHash = useRef(window.location.hash)

  useEffect(() => {
    if (!data || composerInitialized.current) return
    composerInitialized.current = true
    setMissionComposerCollapsed(data.snapshot.missions.length > 0)
    setJourneyOpen(data.snapshot.missions.length === 0)
    if (initialWorkspaceHash.current) {
      window.history.replaceState(
        null,
        '',
        `${window.location.pathname}${window.location.search}${initialWorkspaceHash.current}`,
      )
      initialWorkspaceHash.current = ''
    }
    window.requestAnimationFrame(() => {
      window.scrollTo({ top: 0, left: 0, behavior: 'auto' })
    })
  }, [data])

  useEffect(() => {
    if (!initialWorkspaceHash.current) return
    window.history.replaceState(
      null,
      '',
      `${window.location.pathname}${window.location.search}`,
    )
  }, [])

  useEffect(() => {
    const syncFromHash = () => {
      const view = workspaceViewFromHash(window.location.hash)
      setActiveWorkspaceView(view)
      if (view !== 'floor') setFloorInspectorOpen(false)
    }
    window.addEventListener('hashchange', syncFromHash)
    window.addEventListener('popstate', syncFromHash)
    return () => {
      window.removeEventListener('hashchange', syncFromHash)
      window.removeEventListener('popstate', syncFromHash)
    }
  }, [])

  useEffect(() => {
    if (!floorInspectorOpen) return
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setFloorInspectorOpen(false)
    }
    window.addEventListener('keydown', closeOnEscape)
    return () => window.removeEventListener('keydown', closeOnEscape)
  }, [floorInspectorOpen])

  const navigateToWorkspaceEntity = useCallback(
    (kind: EntityLink['kind'] | 'room', targetId: string) => {
      if (kind === 'room') {
        setActiveWorkspaceView('room')
        window.history.replaceState(null, '', '#room')
        setAnnouncement('Comms opened.')
        window.setTimeout(() => revealEntityTarget(kind, targetId), 80)
        return
      }

      const linkedRun =
        kind === 'run'
          ? data?.snapshot.runs.find((run) => run.id === targetId)
          : kind === 'artifact'
            ? data?.snapshot.runs.find((run) => run.artifact_id === targetId)
            : undefined
      const linkedTask =
        kind === 'task'
          ? data?.snapshot.tasks.find((task) => task.id === targetId)
          : linkedRun
            ? data?.snapshot.tasks.find((task) => task.id === linkedRun.task_id)
            : undefined
      const missionId = kind === 'mission' ? targetId : linkedTask?.mission_id
      if (missionId) setSelectedMissionId(missionId)
      setActiveWorkspaceView('missions')
      window.history.replaceState(null, '', '#missions')
      setAnnouncement(`${statusLabel(kind)} opened in Missions.`)
      window.setTimeout(() => {
        if (revealEntityTarget(kind, targetId)) return
        if (missionId) revealEntityTarget('mission', missionId)
      }, 80)
    },
    [data],
  )

  useEffect(() => {
    const focus = (raw: string) => {
      try {
        const url = new URL(raw)
        const segments = [url.host, ...url.pathname.split('/').filter(Boolean)]
        const keys = ['corp', 'room', 'mission', 'task', 'run']
        const values = new Map<string, string>()
        for (let index = 0; index < segments.length - 1; index += 2) {
          if (keys.includes(segments[index])) values.set(segments[index], segments[index + 1])
        }
        const corp = values.get('corp')
        if (corp && bootstrap && corp !== bootstrap.corp_id) {
          setError(`Deep link targets a different Corp: ${corp}`)
          return
        }
        const targetKind = (['run', 'task', 'mission', 'room'] as const).find(
          (kind) => values.has(kind),
        )
        if (!targetKind) return
        const targetId = values.get(targetKind)
        if (!targetId) return

        navigateToWorkspaceEntity(targetKind, targetId)
      } catch {
        setError('The desktop deep link is invalid.')
      }
    }
    const listener = (event: Event) => {
      focus((event as CustomEvent<string>).detail)
    }
    window.addEventListener('ecorp-deep-link', listener)
    window.addEventListener('crony-deep-link', listener)
    return () => {
      window.removeEventListener('ecorp-deep-link', listener)
      window.removeEventListener('crony-deep-link', listener)
    }
  }, [bootstrap, navigateToWorkspaceEntity])
  const lastEventSeq = useRef<Record<string, number>>({})

  const refresh = useCallback(async (corpId: string, actorId: string) => {
    const snapshot = await api<SnapshotResponse>(
      `/api/corps/${corpId}/snapshot?actor_id=${actorId}`,
    )
    const newest = snapshot.snapshot.events.at(-1)?.seq ?? 0
    lastEventSeq.current[actorId] = Math.max(lastEventSeq.current[actorId] ?? 0, newest)
    setData(snapshot)
    return snapshot
  }, [])

  useEffect(() => {
    let cancelled = false
    void (async () => {
      const health = await fetch(`${API_URL}/health`).then((response) =>
        response.json() as Promise<HealthResponse>,
      )
      if (health.mode === 'production') {
        const corpId = window.sessionStorage.getItem('ecorp_corp_id')
        const actorId = window.sessionStorage.getItem('ecorp_actor_id')
        if (!corpId || !actorId || !storedAccessToken()) {
          if (!cancelled) {
            setRequiresConnection(true)
            setAnnouncement('Production connection details are required.')
          }
          return
        }
        const result: BootstrapResponse = {
          corp_id: corpId,
          room_id: '',
          alice_actor_id: actorId,
          bob_actor_id: actorId,
          eve_actor_id: actorId,
          manager_agent_id: '',
          worker_agent_id: '',
          codex_agent_id: '',
        }
        if (cancelled) return
        setBootstrap(result)
        setSelectedActorId(actorId)
        await refresh(corpId, actorId)
        return
      }
      const result = await api<BootstrapResponse>('/api/demo/bootstrap', {
        method: 'POST',
        body: '{}',
      })
      if (cancelled) return
      setBootstrap(result)
      const actorName = new URLSearchParams(window.location.search).get('actor')
      const initialActor = actorName?.toLowerCase() === 'bob'
        ? result.bob_actor_id
        : actorName?.toLowerCase() === 'eve'
          ? result.eve_actor_id
          : result.alice_actor_id
      setSelectedActorId(initialActor)
      await refresh(result.corp_id, initialActor)
    })()
      .catch((caught: unknown) => setError(caught instanceof Error ? caught.message : String(caught)))
    return () => {
      cancelled = true
    }
  }, [refresh])

  useEffect(() => {
    if (!bootstrap || !selectedActorId) return
    let disposed = false
    let socket: WebSocket | null = null

    const connect = async () => {
      if (disposed) return
      let replaying = true
      let replayChanged = false
      setConnection('connecting')
      const wsUrl = API_URL.replace(/^http/, 'ws')
      let authorization: string
      try {
        const token = storedAccessToken()
        if (token) {
          const ticket = await api<{ ticket: string }>(
            `/api/corps/${bootstrap.corp_id}/ws-ticket`,
            {
              method: 'POST',
              body: JSON.stringify({ actor_id: selectedActorId }),
            },
          )
          authorization = `ticket=${encodeURIComponent(ticket.ticket)}`
        } else {
          authorization = `actor_id=${encodeURIComponent(selectedActorId)}`
        }
      } catch (caught) {
        if (disposed) return
        setConnection('offline')
        setError(caught instanceof Error ? caught.message : String(caught))
        reconnectTimer.current = window.setTimeout(() => void connect(), 1_500)
        return
      }
      if (disposed) return
      socket = new WebSocket(
        `${wsUrl}/ws/corps/${bootstrap.corp_id}?${authorization}&after_seq=${lastEventSeq.current[selectedActorId] ?? 0}`,
      )
      socket.onmessage = (event) => {
        const message = JSON.parse(event.data) as BrowserSocketMessage
        if (message.type === 'ready') {
          lastEventSeq.current[selectedActorId] = Math.max(
            lastEventSeq.current[selectedActorId] ?? 0,
            message.replayed_through,
          )
          replaying = false
          setConnection('live')
          if (replayChanged) void refresh(bootstrap.corp_id, selectedActorId)
          return
        }
        if (message.event.seq <= (lastEventSeq.current[selectedActorId] ?? 0)) return
        lastEventSeq.current[selectedActorId] = message.event.seq
        setAnnouncement(statusLabel(message.event.type))
        if (replaying) {
          replayChanged = true
          return
        }
        void refresh(bootstrap.corp_id, selectedActorId)
      }
      socket.onerror = () => setConnection('offline')
      socket.onclose = () => {
        if (disposed) return
        setConnection('offline')
        reconnectTimer.current = window.setTimeout(() => void connect(), 1_500)
      }
    }

    void connect()
    return () => {
      disposed = true
      if (reconnectTimer.current !== null) window.clearTimeout(reconnectTimer.current)
      socket?.close()
    }
  }, [bootstrap, refresh, selectedActorId])

  const humans = useMemo(
    () => data?.snapshot.actors.filter((actor) => actor.kind === 'human') ?? [],
    [data],
  )
  const missionRepositoryTargets = useMemo(() => repositoryTargets(data), [data])
  const selectedMissionSource = useMemo(
    () =>
      missionRepositoryTargets.find((target) => target.key === missionSourceKey),
    [missionRepositoryTargets, missionSourceKey],
  )
  const allAvailableAdapters = useMemo(
    () => availableRunnerAdapters(data),
    [data],
  )
  const availableAdapters = useMemo(
    () =>
      selectedMissionSource
        ? availableRunnerAdapters(data, selectedMissionSource)
        : [],
    [data, selectedMissionSource],
  )
  const selectedActor =
    humans.find((actor) => actor.id === selectedActorId) ?? humans[0] ?? null
  const preferredAdapter =
    availableAdapters.find((adapter) => adapter.name === 'github-copilot') ??
    availableAdapters.find((adapter) => adapter.name === 'codex') ??
    availableAdapters.find((adapter) => adapter.name === 'claude-code') ??
    availableAdapters[0]
  const deterministicHarness = usesDeterministicHarness(missionStrategy)
  const effectiveMissionAdapter = deterministicHarness
    ? availableAdapters.find((adapter) => adapter.name === 'fake-process')?.name ?? ''
    : availableAdapters.some((adapter) => adapter.name === missionAdapter)
      ? missionAdapter
      : preferredAdapter?.name ?? ''
  const missionContract: MissionContractInput = {
    objective: missionObjective.trim(),
    expected_output: missionExpectedOutput.trim(),
    acceptance_tests: nonEmptyLines(missionAcceptanceTests),
    allowed_tools: nonEmptyLines(missionAllowedTools),
    prohibited_actions: nonEmptyLines(missionProhibitedActions),
    references: nonEmptyLines(missionReferences),
    write_scope: nonEmptyLines(missionWriteScope),
  }
  const missionContractHasInput =
    Boolean(missionContract.objective || missionContract.expected_output) ||
    missionContract.acceptance_tests.length > 0 ||
    missionContract.allowed_tools.length > 0 ||
    missionContract.prohibited_actions.length > 0 ||
    missionContract.references.length > 0 ||
    missionContract.write_scope.length > 0
  const missionVerifierErrors = customVerification && !deterministicHarness
    ? verificationPolicyErrors(missionVerificationPolicy)
    : []

  const selectActor = (actor: Actor) => {
    setSelectedActorId(actor.id)
    setAnnouncement(`Switching operations view to ${actor.name}.`)
    const url = new URL(window.location.href)
    url.searchParams.set('actor', actor.name.toLowerCase())
    window.history.replaceState({}, '', url)
    if (bootstrap) {
      void refresh(bootstrap.corp_id, actor.id).catch((caught: unknown) => {
        setError(caught instanceof Error ? caught.message : String(caught))
      })
    }
  }

  const createMission = async (event: FormEvent) => {
    event.preventDefault()
    if (
      !bootstrap ||
      !selectedActor ||
      !missionTitle.trim() ||
      !selectedMissionSource ||
      !missionSourceConfirmed
    ) {
      return
    }
    setBusy(true)
    setError(null)
    try {
      const created = await api<CreateMissionResponse>(`/api/corps/${bootstrap.corp_id}/missions`, {
        method: 'POST',
        body: JSON.stringify({
          title: missionTitle,
          description: missionDescription,
          requested_by: selectedActor.id,
          preferred_adapter: effectiveMissionAdapter,
          preferred_model: !deterministicHarness && selectedModel ? missionModel : null,
          reasoning_effort:
            !deterministicHarness &&
            selectedModel?.supported_reasoning_efforts.includes(missionReasoningEffort)
              ? missionReasoningEffort
              : null,
          strategy: missionStrategy,
          source: {
            repository: selectedMissionSource.repository,
            base_ref: selectedMissionSource.baseRef,
            base_commit: selectedMissionSource.baseCommit,
          },
          budget_tokens: deterministicHarness ? null : missionBudgetTokens,
          deliverable: {
            form: missionDeliverable,
            commit_after_verification:
              commitDeliverable || missionDeliverable === 'commit_branch',
            paths: [],
          },
          contract: missionContractHasInput ? missionContract : null,
          verification_policy:
            !deterministicHarness && customVerification
              ? missionVerificationPolicy
              : null,
        }),
      })
      let launched: LaunchMissionResponse | null = null
      if (!pauseAfterPlanning) {
        launched = await api<LaunchMissionResponse>(
          `/api/corps/${bootstrap.corp_id}/missions/${created.mission_id}/launch`,
          {
            method: 'POST',
            body: JSON.stringify({ requested_by: selectedActor.id }),
          },
        )
      }
      setMissionTitle('')
      setMissionDescription('')
      setMissionObjective('')
      setMissionExpectedOutput('')
      setMissionAcceptanceTests('')
      setMissionAllowedTools('')
      setMissionProhibitedActions('')
      setMissionReferences('')
      setMissionWriteScope('')
      setMissionSourceKey('')
      setMissionSourceConfirmed(false)
      setCustomVerification(false)
      setMissionVerificationPolicy({
        checks: [defaultVerifierCheck('artifact')],
        manual_gate: null,
      })
      setMissionComposerStep('brief')
      setMissionContractTab('outcome')
      setMissionComposerCollapsed(true)
      setSelectedMissionId(created.mission_id)
      setActiveWorkspaceView('missions')
      window.history.replaceState(null, '', '#missions')
      const refreshed = await refresh(bootstrap.corp_id, selectedActor.id)
      if (launched) {
        const launchedRun = refreshed.snapshot.runs.find(
          (run) => run.id === launched?.run_id,
        )
        if (launchedRun) setSelectedAgentId(launchedRun.agent_id)
      }
      setAnnouncement(
        pauseAfterPlanning
          ? 'Mission plan created. Review the task contracts before dispatch.'
          : 'Mission dispatched. The active worker is selected on the control floor.',
      )
      window.setTimeout(() => {
        const card = document.querySelector<HTMLElement>(
          `[data-mission-id="${CSS.escape(created.mission_id)}"]`,
        )
        card?.scrollIntoView({ behavior: 'smooth', block: 'center' })
        card?.focus({ preventScroll: true })
      }, 80)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
    } finally {
      setBusy(false)
    }
  }

  const launchMission = async (mission: Mission) => {
    if (!bootstrap || !selectedActor) return
    setBusy(true)
    setError(null)
    try {
      const launched = await api<LaunchMissionResponse>(
        `/api/corps/${bootstrap.corp_id}/missions/${mission.id}/launch`,
        {
          method: 'POST',
          body: JSON.stringify({ requested_by: selectedActor.id }),
        },
      )
      const refreshed = await refresh(bootstrap.corp_id, selectedActor.id)
      const launchedRun = refreshed.snapshot.runs.find(
        (run) => run.id === launched.run_id,
      )
      if (launchedRun) setSelectedAgentId(launchedRun.agent_id)
      setAnnouncement('Mission dispatched. The active worker is selected.')
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
    } finally {
      setBusy(false)
    }
  }

  const resumeAgentRun = async (run: Run) => {
    if (!bootstrap || !selectedActor) return
    setBusy(true)
    setError(null)
    try {
      await api(`/api/corps/${bootstrap.corp_id}/runs/${run.id}/resume`, {
        method: 'POST',
        body: JSON.stringify({
          requested_by: selectedActor.id,
          prompt:
            'Continue the prior session in the same repository. Inspect the current state, complete any remaining mission work, and verify the result.',
        }),
      })
      await refresh(bootstrap.corp_id, selectedActor.id)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
    } finally {
      setBusy(false)
    }
  }

  const createContractRevision = async (
    mission: Mission,
    task: Task,
    input: MissionContractRevisionInput,
  ): Promise<boolean> => {
    if (!bootstrap || !selectedActor) return false
    setBusy(true)
    setError(null)
    try {
      await api(
        `/api/corps/${bootstrap.corp_id}/missions/${mission.id}/contract-revisions`,
        {
          method: 'POST',
          body: JSON.stringify({
            actor_id: selectedActor.id,
            ...input,
          }),
        },
      )
      await refresh(bootstrap.corp_id, selectedActor.id)
      setAnnouncement(
        `Contract revision ${task.contract_version + 1} recorded. Explicitly ${
          input.next_action === 'resume' ? 'resume the preserved run' : 'dispatch the mission'
        } when ready.`,
      )
      return true
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
      return false
    } finally {
      setBusy(false)
    }
  }

  const proposeBudgetRevision = async (
    mission: Mission,
    input: MissionBudgetRevisionInput,
  ): Promise<boolean> => {
    if (!bootstrap || !selectedActor) return false
    setBusy(true)
    setError(null)
    try {
      await api(
        `/api/corps/${bootstrap.corp_id}/missions/${mission.id}/budget-revisions`,
        {
          method: 'POST',
          body: JSON.stringify({
            actor_id: selectedActor.id,
            expected_budget_tokens: mission.budget_tokens,
            expected_budget_cost_microusd: mission.budget_cost_microusd,
            proposed_budget_tokens: input.proposed_budget_tokens,
            proposed_budget_cost_microusd:
              input.proposed_budget_cost_microusd,
            rationale: input.rationale,
            idempotency_key: input.idempotency_key,
            finish_scope: input.finish_scope,
          }),
        },
      )
      await refresh(bootstrap.corp_id, selectedActor.id)
      setAnnouncement(
        'Budget revision proposed. An owner or admin must record the decision before resume.',
      )
      return true
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
      return false
    } finally {
      setBusy(false)
    }
  }

  const decideBudgetRevision = async (
    mission: Mission,
    revision: MissionBudgetRevision,
    approved: boolean,
    decisionKey: string,
  ) => {
    if (!bootstrap || !selectedActor) return
    setBusy(true)
    setError(null)
    try {
      await api(
        `/api/corps/${bootstrap.corp_id}/missions/${mission.id}/budget-revisions/${revision.id}/decision`,
        {
          method: 'POST',
          body: JSON.stringify({
            actor_id: selectedActor.id,
            expected_version: revision.version,
            approved,
            note: approved
              ? `${selectedActor.name} authorized the revised mission ceiling.`
              : `${selectedActor.name} rejected the proposed mission ceiling.`,
            decision_key: decisionKey,
          }),
        },
      )
      await refresh(bootstrap.corp_id, selectedActor.id)
      setAnnouncement(
        approved
          ? 'Budget revision approved. The preserved run can resume within the revised ceiling.'
          : 'Budget revision rejected. The prior mission ceiling remains authoritative.',
      )
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
    } finally {
      setBusy(false)
    }
  }

  const decideVerification = async (run: Run, approved: boolean) => {
    if (!bootstrap || !selectedActor) return
    setBusy(true)
    setError(null)
    try {
      await api(`/api/corps/${bootstrap.corp_id}/runs/${run.id}/verification-decision`, {
        method: 'POST',
        body: JSON.stringify({
          actor_id: selectedActor.id,
          approved,
          note: approved
            ? `${selectedActor.name} accepted the recorded verification evidence.`
            : `${selectedActor.name} rejected the recorded verification evidence.`,
        }),
      })
      await refresh(bootstrap.corp_id, selectedActor.id)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
    } finally {
      setBusy(false)
    }
  }

  const decideActionApproval = async (approval: ActionApproval, approved: boolean) => {
    if (!bootstrap || !selectedActor) return
    const note = approved
      ? `${selectedActor.name} approved the scoped action.`
      : `${selectedActor.name} rejected the scoped action.`
    const operationStorageKey =
      `ecorp:approval-decision:${bootstrap.corp_id}:${selectedActor.id}:` +
      `${approval.id}:${approved ? 'approve' : 'reject'}`
    const decisionKey = browserOperationKey(
      operationStorageKey,
      JSON.stringify({ approvalId: approval.id, approved, note }),
    )
    setBusy(true)
    setError(null)
    try {
      await api(
        `/api/corps/${bootstrap.corp_id}/approvals/${approval.id}/decision`,
        {
          method: 'POST',
          body: JSON.stringify({
            actor_id: selectedActor.id,
            approved,
            note,
            decision_key: decisionKey,
          }),
        },
      )
      clearBrowserOperation(operationStorageKey)
      await refresh(bootstrap.corp_id, selectedActor.id)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
    } finally {
      setBusy(false)
    }
  }

  const claimLease = async (agent: Agent) => {
    if (!bootstrap || !selectedActor) return
    setError(null)
    try {
      const result = await api<{
        acquired: boolean
        holder_actor_id: string
        token: string | null
      }>(
        `/api/corps/${bootstrap.corp_id}/agents/${agent.id}/lease`,
        {
          method: 'POST',
          body: JSON.stringify({ actor_id: selectedActor.id }),
        },
      )
      if (!result.acquired) {
        const holder = humans.find((actor) => actor.id === result.holder_actor_id)
        setError(`${holder?.name ?? 'Another operator'} currently controls ${agent.name}.`)
      } else if (result.token) {
        const key = leaseTokenKey(selectedActor.id, agent.id)
        setLeaseTokens((current) => ({ ...current, [key]: result.token as string }))
      }
      await refresh(bootstrap.corp_id, selectedActor.id)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
    }
  }

  const releaseLease = async (agent: Agent, token: string) => {
    if (!bootstrap || !selectedActor) return
    setError(null)
    try {
      await api(`/api/corps/${bootstrap.corp_id}/agents/${agent.id}/lease/release`, {
        method: 'POST',
        body: JSON.stringify({ actor_id: selectedActor.id, token }),
      })
      const key = leaseTokenKey(selectedActor.id, agent.id)
      setLeaseTokens((current) => {
        const next = { ...current }
        delete next[key]
        return next
      })
      await refresh(bootstrap.corp_id, selectedActor.id)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
    }
  }

  const transferLease = async (agent: Agent, token: string, toActor: Actor) => {
    if (!bootstrap || !selectedActor) return
    setError(null)
    try {
      await api<{ token: null; holder_actor_id: string }>(
        `/api/corps/${bootstrap.corp_id}/agents/${agent.id}/lease/transfer`,
        {
          method: 'POST',
          body: JSON.stringify({
            actor_id: selectedActor.id,
            token,
            to_actor_id: toActor.id,
          }),
        },
      )
      const fromKey = leaseTokenKey(selectedActor.id, agent.id)
      setLeaseTokens((current) => {
        const next = { ...current }
        delete next[fromKey]
        return next
      })
      await refresh(bootstrap.corp_id, selectedActor.id)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
    }
  }

  const emergencyStop = async (agent: Agent) => {
    if (!bootstrap || !selectedActor) return
    const defaultReason = `${selectedActor.name} stopped ${agent.name} because the live run required immediate operator intervention.`
    const reason = window.prompt(
      `Emergency stop ${agent.name} on run ${shortId(agent.current_run_id)}. Enter the audited reason:`,
      defaultReason,
    )
    if (!reason?.trim()) return
    setError(null)
    try {
      await api(`/api/corps/${bootstrap.corp_id}/agents/${agent.id}/emergency-stop`, {
        method: 'POST',
        body: JSON.stringify({
          actor_id: selectedActor.id,
          reason: reason.trim(),
        }),
      })
      await refresh(bootstrap.corp_id, selectedActor.id)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
    }
  }

  const interruptRun = async (agent: Agent, token: string) => {
    if (!bootstrap || !selectedActor) return
    setError(null)
    try {
      await api(`/api/corps/${bootstrap.corp_id}/agents/${agent.id}/interrupt`, {
        method: 'POST',
        body: JSON.stringify({
          actor_id: selectedActor.id,
          lease_token: token,
          reason: `${selectedActor.name} interrupted the active turn from the operations floor.`,
        }),
      })
      await refresh(bootstrap.corp_id, selectedActor.id)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
    }
  }

  const sendMessage = async (
    agent: Agent,
    text: string,
    token: string | undefined,
    idempotencyKey: string,
  ): Promise<boolean> => {
    if (!bootstrap || !selectedActor) return false
    setError(null)
    try {
      await api(`/api/corps/${bootstrap.corp_id}/agents/${agent.id}/messages`, {
        method: 'POST',
        body: JSON.stringify({
          actor_id: selectedActor.id,
          lease_token: token ?? null,
          text,
          idempotency_key: idempotencyKey,
        }),
      })
      await refresh(bootstrap.corp_id, selectedActor.id)
      return true
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
      return false
    }
  }

  const postRoomMessage = async (input: {
    roomId: string
    body: string
    replyToId: string | null
    mentions: string[]
    link: EntityLink | null
    idempotencyKey: string
  }): Promise<boolean> => {
    if (!bootstrap || !selectedActor) return false
    setError(null)
    try {
      await api(`/api/corps/${bootstrap.corp_id}/rooms/${input.roomId}/messages`, {
        method: 'POST',
        body: JSON.stringify({
          actor_id: selectedActor.id,
          body: input.body,
          reply_to_id: input.replyToId,
          mentions: input.mentions,
          link: input.link,
          idempotency_key: input.idempotencyKey,
        }),
      })
      await refresh(bootstrap.corp_id, selectedActor.id)
      return true
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
      return false
    }
  }

  const downloadArtifact = async (run: Run) => {
    if (!selectedActor || !run.artifact_uri || !run.artifact_id) return
    setError(null)
    try {
      const token = storedAccessToken()
      const response = await fetch(
        `${API_URL}${run.artifact_uri}?actor_id=${selectedActor.id}`,
        {
          headers: {
            accept: 'application/octet-stream',
            ...(token ? { authorization: `Bearer ${token}` } : {}),
          },
        },
      )
      if (!response.ok) {
        throw new Error(`Artifact download failed with HTTP ${response.status}.`)
      }
      const blob = await response.blob()
      const objectUrl = URL.createObjectURL(blob)
      const link = document.createElement('a')
      const extension = run.artifact_media_type === 'application/json'
        ? 'json'
        : run.artifact_media_type === 'text/markdown'
          ? 'md'
          : 'bin'
      link.href = objectUrl
      link.download = `ecorp-artifact-${run.artifact_id}.${extension}`
      document.body.append(link)
      link.click()
      link.remove()
      URL.revokeObjectURL(objectUrl)
      setAnnouncement('Verified artifact download started.')
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
    }
  }

  const downloadDeliverable = async (deliverable: SourceDeliverable) => {
    if (!selectedActor) return
    setError(null)
    try {
      const token = storedAccessToken()
      const response = await fetch(
        `${API_URL}${deliverable.uri}?actor_id=${selectedActor.id}`,
        {
          headers: {
            accept: 'application/octet-stream',
            ...(token ? { authorization: `Bearer ${token}` } : {}),
          },
        },
      )
      if (!response.ok) {
        throw new Error(`Source deliverable download failed with HTTP ${response.status}.`)
      }
      const blob = await response.blob()
      const objectUrl = URL.createObjectURL(blob)
      const link = document.createElement('a')
      link.href = objectUrl
      link.download = deliverable.file_name
      document.body.append(link)
      link.click()
      link.remove()
      URL.revokeObjectURL(objectUrl)
      setAnnouncement('Source deliverable download started.')
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
    }
  }

  const controlFactoryController = async (
    controller: FactoryController,
    action: 'pause' | 'resume' | 'reconcile',
  ) => {
    if (!bootstrap || !selectedActor) return
    const operationStorageKey =
      `ecorp:factory-control:${bootstrap.corp_id}:${selectedActor.id}:` +
      `${controller.id}:${action}`
    let pendingOperation: { idempotencyKey: string; expectedVersion: number } | null = null
    try {
      const stored = window.sessionStorage.getItem(operationStorageKey)
      if (stored) {
        pendingOperation = JSON.parse(stored) as {
          idempotencyKey: string
          expectedVersion: number
        }
      }
    } catch {
      window.sessionStorage.removeItem(operationStorageKey)
    }
    if (!pendingOperation) {
      pendingOperation = {
        idempotencyKey: crypto.randomUUID(),
        expectedVersion: controller.version,
      }
      window.sessionStorage.setItem(
        operationStorageKey,
        JSON.stringify(pendingOperation),
      )
    }
    setBusy(true)
    setError(null)
    try {
      await api(
        `/api/corps/${bootstrap.corp_id}/factory/controllers/${controller.id}/control`,
        {
          method: 'POST',
          body: JSON.stringify({
            actor_id: selectedActor.id,
            expected_version: pendingOperation.expectedVersion,
            action,
            idempotency_key: pendingOperation.idempotencyKey,
          }),
        },
      )
      window.sessionStorage.removeItem(operationStorageKey)
      await refresh(bootstrap.corp_id, selectedActor.id)
      setAnnouncement(
        action === 'pause'
          ? 'Factory intake paused. Active missions continue.'
          : action === 'resume'
            ? 'Factory intake resumed and reconciliation requested.'
            : 'Factory reconciliation requested.',
      )
    } catch (caught) {
      if (caught instanceof ApiRequestError && caught.status >= 400 && caught.status < 500) {
        window.sessionStorage.removeItem(operationStorageKey)
      }
      setError(caught instanceof Error ? caught.message : String(caught))
    } finally {
      setBusy(false)
    }
  }

  const connectProduction = async (event: FormEvent) => {
    event.preventDefault()
    const corpId = connectionCorpId.trim()
    const actorId = connectionActorId.trim()
    const token = connectionToken.trim()
    if (!corpId || !actorId || !token) return
    setBusy(true)
    setError(null)
    window.sessionStorage.setItem('ecorp_corp_id', corpId)
    window.sessionStorage.setItem('ecorp_actor_id', actorId)
    window.sessionStorage.setItem('ecorp_access_token', token)
    try {
      const result: BootstrapResponse = {
        corp_id: corpId,
        room_id: '',
        alice_actor_id: actorId,
        bob_actor_id: actorId,
        eve_actor_id: actorId,
        manager_agent_id: '',
        worker_agent_id: '',
        codex_agent_id: '',
      }
      await refresh(corpId, actorId)
      setBootstrap(result)
      setSelectedActorId(actorId)
      setConnectionToken('')
      setRequiresConnection(false)
      setAnnouncement('Authenticated production connection established.')
    } catch (caught) {
      window.sessionStorage.removeItem('ecorp_access_token')
      setError(caught instanceof Error ? caught.message : String(caught))
    } finally {
      setBusy(false)
    }
  }

  if (requiresConnection) {
    return (
      <main className="loading-shell production-connect">
        <div className="loading-stamp">ECORP SECURE CONNECTION</div>
        <h1>Connect to your Corp</h1>
        <p>Enter the Corp, actor, and short-lived OIDC access token issued for this session.</p>
        {error ? <p className="error-banner" role="alert">{error}</p> : null}
        <form onSubmit={connectProduction}>
          <label htmlFor="production-corp">Corp ID</label>
          <input
            id="production-corp"
            value={connectionCorpId}
            onChange={(event) => setConnectionCorpId(event.target.value)}
            autoComplete="off"
          />
          <label htmlFor="production-actor">Actor ID</label>
          <input
            id="production-actor"
            value={connectionActorId}
            onChange={(event) => setConnectionActorId(event.target.value)}
            autoComplete="off"
          />
          <label htmlFor="production-token">Access token</label>
          <input
            id="production-token"
            type="password"
            value={connectionToken}
            onChange={(event) => setConnectionToken(event.target.value)}
            autoComplete="off"
          />
          <button
            className="button button-primary"
            type="submit"
            disabled={busy || !connectionCorpId.trim() || !connectionActorId.trim() || !connectionToken.trim()}
          >
            {busy ? 'Connecting…' : 'Connect securely'}
          </button>
        </form>
      </main>
    )
  }

  if (!data || !bootstrap || !selectedActor) {
    return (
      <main className="loading-shell">
        <div className="loading-stamp">ECORP OPERATIONS NETWORK</div>
        <h1>Authorizing the operations console…</h1>
        {error ? <p className="error-banner">{error}</p> : <p>Waiting for the control plane.</p>}
      </main>
    )
  }

  const latestMissions = data.snapshot.missions.slice(0, 8)
  const latestEvents = data.snapshot.events.toReversed().slice(0, 28)
  const room = data.snapshot.rooms[0]
  const connectedRunners = data.runners.filter((runner) => runner.connected)
  const selectedAdapter = availableAdapters.find(
    (adapter) => adapter.name === effectiveMissionAdapter,
  )
  const selectedModel = selectedAdapter?.models.find(
    (model) => model.id === missionModel,
  )
  const runnerLabel = connectedRunners.length
    ? `${connectedRunners.length} runner${connectedRunners.length === 1 ? '' : 's'} online`
    : data.runners.some((runner) => runner.status === 'grace')
      ? 'Runner reconnecting'
      : 'No runner'
  const selectedAgent =
    data.snapshot.agents.find((agent) => agent.id === selectedAgentId) ??
    data.snapshot.agents[0]
  const selectedAgentCapability = connectedRunners
    .flatMap((runner) => runner.capabilities)
    .find(
      (capability) =>
        capability.available && capability.name === selectedAgent.adapter,
    )
  const workspaceCapability = connectedRunners
    .flatMap((runner) => runner.capabilities)
    .find((capability) => capability.name === 'workspace-isolation')
  const sourceRepository =
    detailValue(workspaceCapability?.detail, 'repository') ?? 'No source repository reported'
  const sourceBase = detailValue(workspaceCapability?.detail, 'base') ?? 'HEAD'
  const realAdapters = allAvailableAdapters.filter(
    (adapter) => adapter.name !== 'fake-process',
  )
  const pendingApprovals = data.snapshot.action_approvals.filter(
    (approval) => approval.status === 'pending',
  )
  const pendingVerificationRequests = data.snapshot.verification_requests.filter(
    (request) => request.status === 'pending',
  )
  const activeRuns = data.snapshot.runs.filter((run) =>
    ['provisioning', 'starting', 'running', 'waiting_for_input', 'waiting_for_approval', 'verifying'].includes(run.status),
  )
  const acceptedArtifacts = data.snapshot.runs.filter(
    (run) => run.verification_status === 'passed' && run.artifact_uri,
  )
  const productionAuthenticated = Boolean(storedAccessToken())
  const activeFactoryItems = data.snapshot.factory_work_items.filter(
    (item) => !['published', 'failed', 'cancelled'].includes(item.state),
  )
  const currentWorkspaceView =
    WORKSPACE_VIEWS.find((view) => view.id === activeWorkspaceView) ??
    WORKSPACE_VIEWS[0]
  const selectedMission =
    latestMissions.find((mission) => mission.id === selectedMissionId) ??
    latestMissions[0]
  const selectedMissionTasks = selectedMission
    ? data.snapshot.tasks.filter((task) => task.mission_id === selectedMission.id)
    : []
  const selectedMissionTaskIds = new Set(selectedMissionTasks.map((task) => task.id))
  const selectedMissionRuns = selectedMission
    ? data.snapshot.runs.filter((run) => selectedMissionTaskIds.has(run.task_id))
    : []

  const activateWorkspaceView = (view: WorkspaceView) => {
    const destination = WORKSPACE_VIEWS.find((candidate) => candidate.id === view)
    setActiveWorkspaceView(view)
    if (view !== 'floor') setFloorInspectorOpen(false)
    if (window.location.hash !== `#${view}`) {
      window.history.pushState(null, '', `#${view}`)
    }
    setAnnouncement(`${destination?.label ?? 'Workspace'} opened.`)
    window.scrollTo({ top: 0, left: 0, behavior: 'auto' })
    window.requestAnimationFrame(() => {
      document.getElementById(view)?.focus({ preventScroll: true })
    })
  }

  return (
    <main className={`app-shell view-${activeWorkspaceView}`} aria-busy={busy}>
      <a className="skip-link" href={`#${activeWorkspaceView}`}>
        Skip to workspace
      </a>
      <div className="sr-only" role="status" aria-live="polite" aria-atomic="true">
        {announcement}
      </div>
      <header className="topbar">
        <div className="brand-lockup">
          <span className="brand-kicker">Collaborative agent operations</span>
          <div className="brand-row">
            <span className="brand-mark" aria-hidden="true">
              <span className="brand-e">E</span>
              <span className="brand-slash" />
            </span>
            <h1><span>E</span>CORP</h1>
            <span className="alpha-stamp">Local preview</span>
          </div>
        </div>
        <div className="operator-console">
          <div className={`live-indicator live-${connection}`}>
            <span />
            {connection}
          </div>
          <div className={`runner-indicator ${connectedRunners.length ? 'runner-online' : ''}`}>
            {runnerLabel}
          </div>
          <label title={productionAuthenticated ? 'Your authenticated identity; switching accounts is not allowed here.' : 'Local demo identities only. Alice, Bob and Eve are seeded test users, not GitHub sign-in.'}>
            {productionAuthenticated ? 'Signed in as' : 'Demo operator'}
            <select disabled={productionAuthenticated} value={selectedActor.id} onChange={(event) => {
              const actor = humans.find((candidate) => candidate.id === event.target.value)
              if (actor) selectActor(actor)
            }}>
              {humans.map((actor) => (
                <option key={actor.id} value={actor.id}>
                  {actor.name} · {actor.role}
                </option>
              ))}
            </select>
          </label>
        </div>
      </header>

      <nav className="workspace-nav" aria-label="ECorp workspace sections">
        {WORKSPACE_VIEWS.map((view) => (
          <a
            key={view.id}
            href={`#${view.id}`}
            className={activeWorkspaceView === view.id ? 'workspace-nav-active' : ''}
            aria-current={activeWorkspaceView === view.id ? 'page' : undefined}
            onClick={(event) => {
              event.preventDefault()
              activateWorkspaceView(view.id)
            }}
          >
            <span className="workspace-nav-code">{view.code}</span>
            <strong>{view.label}</strong>
            <small>{activeWorkspaceView === view.id ? 'Active' : 'Load'}</small>
          </a>
        ))}
        <button
          className="workspace-nav-guide"
          type="button"
          onClick={() => setJourneyOpen((current) => !current)}
        >
          <span className="workspace-nav-code">?</span>
          <strong>{journeyOpen ? 'Hide guide' : 'Start guide'}</strong>
          <small>Help</small>
        </button>
      </nav>

      <section className="arcade-command-deck" aria-label="Operations overview">
        <div className="arcade-command-focus">
          <span>Workspace</span>
          <strong>{currentWorkspaceView.label}</strong>
          <small>{currentWorkspaceView.description}</small>
        </div>
        <div className="arcade-command-meters">
          <button type="button" onClick={() => activateWorkspaceView('floor')}>
            <span>Live runs</span>
            <strong>{activeRuns.length}</strong>
            <small>Take control</small>
          </button>
          <button
            type="button"
            className={
              pendingApprovals.length + pendingVerificationRequests.length
                ? 'arcade-meter-alert'
                : ''
            }
            onClick={() => activateWorkspaceView('missions')}
          >
            <span>Decisions</span>
            <strong>{pendingApprovals.length + pendingVerificationRequests.length}</strong>
            <small>Review gates</small>
          </button>
          <button type="button" onClick={() => activateWorkspaceView('factory')}>
            <span>Factory</span>
            <strong>{activeFactoryItems.length}</strong>
            <small>Active items</small>
          </button>
          <button type="button" onClick={() => activateWorkspaceView('activity')}>
            <span>Verified</span>
            <strong>{acceptedArtifacts.length}</strong>
            <small>Evidence ready</small>
          </button>
        </div>
      </section>

      <section className="journey-panel" hidden={!journeyOpen}>
        <div className="journey-heading">
          <div>
            <span className="section-code">Getting started</span>
            <h2>From repository to verified result</h2>
          </div>
          <p>
            ECorp turns repository work into a governed operating process: isolated execution,
            accountable decisions, and evidence that survives the session.
          </p>
        </div>
        {journeyOpen ? (
          <>
            <ol className="journey-steps">
              <li className={connectedRunners.length ? 'journey-complete' : 'journey-current'}>
                <span>1</span>
                <div>
                  <strong>Connect a runner</strong>
                  <small>
                    {connectedRunners.length
                      ? `${connectedRunners.length} runner connected to ${sourceBase}`
                      : 'Start the local stack or enroll a remote runner.'}
                  </small>
                </div>
              </li>
              <li className={realAdapters.length ? 'journey-complete' : 'journey-current'}>
                <span>2</span>
                <div>
                  <strong>Choose the crew</strong>
                  <small>
                    {realAdapters.length
                      ? `${realAdapters.map((adapter) => adapterLabel(adapter.name)).join(', ')} ready`
                      : 'No real AI runtime is available; the test harness still works offline.'}
                  </small>
                </div>
              </li>
              <li className={data.snapshot.missions.length ? 'journey-complete' : 'journey-current'}>
                <span>3</span>
                <div>
                  <strong>Plan and run a mission</strong>
                  <small>
                    {activeRuns.length
                      ? `${activeRuns.length} run active now`
                      : data.snapshot.missions.length
                        ? `${data.snapshot.missions.length} mission records available`
                        : 'Describe the outcome below; ECorp creates the task contract.'}
                  </small>
                </div>
              </li>
              <li className={acceptedArtifacts.length ? 'journey-complete' : pendingApprovals.length ? 'journey-current' : ''}>
                <span>4</span>
                <div>
                  <strong>Operate and review</strong>
                  <small>
                    {pendingApprovals.length
                      ? `${pendingApprovals.length} decision${pendingApprovals.length === 1 ? '' : 's'} waiting`
                      : acceptedArtifacts.length
                        ? `${acceptedArtifacts.length} verified artifact${acceptedArtifacts.length === 1 ? '' : 's'} ready`
                        : 'Watch the floor, steer an agent, approve risk, then download evidence.'}
                  </small>
                </div>
              </li>
            </ol>
            <details className="setup-drawer" open={!connectedRunners.length}>
              <summary>
                <span>Developer setup</span>
                <small>What is running, where agents write, and how to restart it</small>
              </summary>
              <div className="setup-grid">
                <div className="setup-health">
                  <div><span className={`setup-dot setup-${connection}`} />Control plane<strong>{connection}</strong></div>
                  <div><span className={`setup-dot ${connectedRunners.length ? 'setup-live' : 'setup-offline'}`} />Runner<strong>{runnerLabel}</strong></div>
                  <div><span className={`setup-dot ${realAdapters.length ? 'setup-live' : 'setup-offline'}`} />AI runtimes<strong>{realAdapters.length || 'none'}</strong></div>
                </div>
                <div className="setup-path">
                  <span>Source repository</span>
                  <code>{sourceRepository}</code>
                  <small>Every write-capable run receives a linked worktree. The source checkout is never edited directly.</small>
                </div>
                <div className="setup-command">
                  <span>Start or restart locally</span>
                  <code>./tools/start_local.ps1</code>
                  <small>Then open http://127.0.0.1:5187. Use CRONY_SOURCE_REPOSITORY to target another checkout.</small>
                </div>
              </div>
              <div className="runner-list" aria-label="Connected runner readiness">
                {data.runners.map((runner) => {
                  const isolation = runner.capabilities.find(
                    (capability) => capability.name === 'workspace-isolation',
                  )
                  return (
                    <article className={`runner-card runner-card-${runner.status}`} key={runner.id}>
                      <div>
                        <strong>{runner.hostname}</strong>
                        <span>{runner.id} · {runner.os} · {statusLabel(runner.status)}</span>
                      </div>
                      <small>
                        Repository: {detailValue(isolation?.detail, 'repository') ?? 'not reported'}
                      </small>
                      <ul>
                        {runner.capabilities
                          .filter((capability) => capability.name !== 'workspace-isolation')
                          .map((capability) => (
                            <li key={capability.name}>
                              <span>{adapterLabel(capability.name)}</span>
                              <strong>{capability.available ? 'Ready' : 'Unavailable'}</strong>
                              {capability.models.length ? (
                                <small>
                                  {capability.models.filter((model) => model.policy_state !== 'disabled').length} selectable models
                                </small>
                              ) : null}
                            </li>
                          ))}
                      </ul>
                    </article>
                  )
                })}
              </div>
            </details>
          </>
        ) : null}
      </section>

      {error ? (
        <div className="error-banner" role="alert">
          <strong>Operations notice</strong>
          <span>{error}</span>
          <button type="button" onClick={() => setError(null)} aria-label="Dismiss error">
            ×
          </button>
        </div>
      ) : null}

      <div className="workspace-surface" hidden={activeWorkspaceView !== 'factory'}>
        <FactoryPanel
          items={data.snapshot.factory_work_items}
          missions={data.snapshot.missions}
          publications={data.snapshot.pull_request_publications}
          publicationAttempts={data.snapshot.pull_request_publication_attempts}
          controllers={data.snapshot.factory_controllers ?? []}
          tasks={data.snapshot.tasks}
          runs={data.snapshot.runs}
          room={room}
          messages={data.snapshot.room_messages}
          actors={data.snapshot.actors}
          agents={data.snapshot.agents}
          leases={data.snapshot.leases}
          leaseTokens={leaseTokens}
          selectedActor={selectedActor}
          actionApprovals={data.snapshot.action_approvals}
          verificationRequests={data.snapshot.verification_requests}
          canControlFactory={['owner', 'admin', 'manager'].includes(selectedActor.role)}
          busy={busy}
          onControllerControl={controlFactoryController}
          onPostComment={postRoomMessage}
          onActionDecision={decideActionApproval}
          onVerificationDecision={decideVerification}
          onClaimLease={claimLease}
          onSteer={sendMessage}
        />
      </div>

      <section
        className={`office-grid workspace-surface office-grid-${activeWorkspaceView} ${
          activeWorkspaceView === 'missions' && !missionComposerCollapsed
            ? 'office-grid-authoring'
            : 'office-grid-operating'
        }`}
        hidden={!['floor', 'missions'].includes(activeWorkspaceView)}
      >
        <div
          className="floor-panel panel"
          id="floor"
          hidden={activeWorkspaceView !== 'floor'}
          tabIndex={-1}
        >
          <div className="panel-heading world-titleplate">
            <div>
              <span className="section-code">ECorp · Control floor</span>
              <h2>{room?.name ?? 'Automation division'}</h2>
            </div>
            <p>A place for your agents. A clear view of their work.</p>
          </div>
          <div className="floor-plan">
            {activeWorkspaceView === 'floor' ? <OfficeFloor
              agents={data.snapshot.agents}
              selectedAgentId={selectedAgent.id}
              pendingApprovalRunIds={new Set(pendingApprovals.map((approval) => approval.run_id))}
              pendingReviewRunIds={new Set(pendingVerificationRequests.map((request) => request.run_id))}
              connection={connection}
              runnerCount={connectedRunners.length}
              onSelect={(agentId) => {
                setSelectedAgentId(agentId)
                setFloorInspectorOpen(true)
              }}
              onMissions={() => activateWorkspaceView('missions')}
              onFactory={() => activateWorkspaceView('factory')}
            /> : null}
            {floorInspectorOpen && activeWorkspaceView === 'floor' ? (
              <OfficeInspector agentName={selectedAgent.name} onClose={() => setFloorInspectorOpen(false)}>
                  <AgentDesk
                    agent={selectedAgent}
                    capability={selectedAgentCapability}
                    actor={selectedActor}
                    humans={humans}
                    lease={data.snapshot.leases.find((lease) => lease.agent_id === selectedAgent.id)}
                    leaseToken={leaseTokens[leaseTokenKey(selectedActor.id, selectedAgent.id)]}
                    queuedCount={data.snapshot.queued_messages.filter((message) => message.agent_id === selectedAgent.id).length}
                    onClaim={claimLease}
                    onRelease={releaseLease}
                    onTransfer={transferLease}
                    onInterrupt={interruptRun}
                    onEmergencyStop={emergencyStop}
                    onMessage={sendMessage}
                  />
              </OfficeInspector>
            ) : null}
          </div>
        </div>

        <aside
          className="mission-panel panel"
          id="missions"
          hidden={activeWorkspaceView !== 'missions'}
          tabIndex={-1}
        >
          <div className="panel-heading">
            <div>
              <span className="section-code">Missions</span>
              <h2>Authorize work</h2>
              <p>Describe the outcome. ECorp isolates the repo, dispatches agents, and verifies the result.</p>
            </div>
          </div>
          {missionComposerCollapsed ? (
            <div className="arcade-new-mission-bar">
              <div>
                <span>Ready for work</span>
                <strong>Create another mission</strong>
                <small>The active mission log stays below.</small>
              </div>
              <button
                className="button button-primary"
                type="button"
                onClick={() => {
                  setMissionComposerStep('brief')
                  setMissionComposerCollapsed(false)
                }}
              >
                New mission
              </button>
            </div>
          ) : (
          <form className="mission-form arcade-mission-form" onSubmit={createMission}>
            <div className="arcade-composer-header">
              <div>
                <span className="arcade-ready">Mission setup</span>
                <strong>New mission</strong>
                <small>Define the outcome, runtime, and verification.</small>
              </div>
              <div className="arcade-score" aria-label="Mission configuration status">
                <span>SPEC</span>
                <strong>{missionDescription ? 'ON' : 'OFF'}</strong>
                <span>GATES</span>
                <strong>{customVerification ? missionVerificationPolicy.checks.length : 0}</strong>
              </div>
            </div>

            <nav className="mission-stage-nav" aria-label="Mission setup stages">
              {[
                ['brief', '01', 'Mission'],
                ['loadout', '02', 'Run setup'],
                ['proof', '03', 'Verification'],
              ].map(([step, number, label]) => (
                <button
                  key={step}
                  type="button"
                  className={missionComposerStep === step ? 'stage-active' : ''}
                  aria-current={missionComposerStep === step ? 'step' : undefined}
                  onClick={() =>
                    setMissionComposerStep(step as 'brief' | 'loadout' | 'proof')
                  }
                >
                  <span>{number}</span>
                  <strong>{label}</strong>
                </button>
              ))}
            </nav>

            <section className="mission-stage-screen">
              {missionComposerStep === 'brief' ? (
                <div className="mission-stage-content stage-brief">
                  <div className="stage-title">
                    <span>Mission brief</span>
                    <h3>Name the outcome</h3>
                    <p>Give the crew one clear objective, then attach the full specification.</p>
                  </div>
                  <label className="arcade-input mission-title-input">
                    Mission outcome
                    <span>{missionTitle.length}/240</span>
                    <textarea
                      id="mission-title"
                      aria-label="Mission outcome"
                      value={missionTitle}
                      onChange={(event) => setMissionTitle(event.target.value)}
                      placeholder="Fix checkout totals and prove the browser flow."
                      rows={3}
                      maxLength={240}
                    />
                  </label>
                  <div className="mission-examples" aria-label="Mission examples">
                    {MISSION_EXAMPLES.map((example) => (
                      <button
                        key={example.label}
                        type="button"
                        onClick={() => setMissionTitle(example.value)}
                      >
                        {example.label}
                      </button>
                    ))}
                  </div>
                  <label className="arcade-input mission-description-field">
                    <span>
                      Durable specification
                      <small>{missionDescription.length.toLocaleString()}/100,000</small>
                    </span>
                    <textarea
                      id="mission-description"
                      aria-label="Mission specification"
                      value={missionDescription}
                      onChange={(event) => setMissionDescription(event.target.value)}
                      placeholder="Paste the issue, requirements, constraints, edge cases, and approved context."
                      rows={7}
                      maxLength={100_000}
                    />
                  </label>
                </div>
              ) : null}

              {missionComposerStep === 'loadout' ? (
                <div className="mission-stage-content stage-loadout">
                  <div className="stage-title">
                    <span>Run configuration</span>
                    <h3>Choose how the work runs</h3>
                    <p>Pick the runtime, budget, delivery format, and orchestration pattern.</p>
                  </div>
                  <div className="loadout-grid">
                    <div className="mission-field repository-target-field">
                      <label htmlFor="mission-repository">Target repository</label>
                      <select
                        id="mission-repository"
                        value={missionSourceKey}
                        aria-describedby="mission-repository-help"
                        onChange={(event) => {
                          setMissionSourceKey(event.target.value)
                          setMissionSourceConfirmed(false)
                          setMissionAdapter('')
                          setMissionModel('')
                          setMissionReasoningEffort('')
                        }}
                      >
                        <option value="">Select a connected repository</option>
                        {missionRepositoryTargets.map((target) => (
                          <option key={target.key} value={target.key}>
                            {target.repository} · {target.baseRef} ·{' '}
                            {target.baseCommit.slice(0, 12)}
                          </option>
                        ))}
                      </select>
                      <small id="mission-repository-help">
                        {missionRepositoryTargets.length
                          ? 'The exact repository, ref, and commit are persisted in every task.'
                          : 'Connect a runner configured for the repository you want to change.'}
                      </small>
                      {selectedMissionSource ? (
                        <div className="repository-target-summary">
                          <strong>{selectedMissionSource.repository}</strong>
                          <div className="repository-target-metadata">
                            <span>Ref {selectedMissionSource.baseRef}</span>
                            <span>
                              {selectedMissionSource.runnerIds.length} compatible runner
                              {selectedMissionSource.runnerIds.length === 1 ? '' : 's'}
                            </span>
                            <span>
                              {availableAdapters.length} runtime
                              {availableAdapters.length === 1 ? '' : 's'}
                            </span>
                          </div>
                          <small>
                            Runners: {selectedMissionSource.runnerLabels.join(', ')}
                          </small>
                          <code className="repository-commit">
                            {selectedMissionSource.baseCommit}
                          </code>
                          {isEcorpRepository(selectedMissionSource) ? (
                            <p className="repository-target-warning" role="alert">
                              This is the ECorp product repository. Use a separate dogfood
                              repository for disposable applications and acceptance probes.
                            </p>
                          ) : null}
                          <label className="repository-confirmation">
                            <input
                              type="checkbox"
                              checked={missionSourceConfirmed}
                              onChange={(event) =>
                                setMissionSourceConfirmed(event.target.checked)
                              }
                            />
                            <span>
                              <strong>Confirm this target</strong>
                              <small>
                                Agents may modify only isolated worktrees created from this
                                immutable commit.
                              </small>
                            </span>
                          </label>
                        </div>
                      ) : null}
                    </div>
                    <div className="mission-field">
                      <label htmlFor="mission-adapter">Agent runtime</label>
                      <select
                        id="mission-adapter"
                        value={effectiveMissionAdapter}
                        disabled={deterministicHarness}
                        onChange={(event) => {
                          setMissionAdapter(event.target.value)
                          setMissionModel('')
                          setMissionReasoningEffort('')
                        }}
                      >
                        {availableAdapters.map((adapter) => (
                          <option key={adapter.name} value={adapter.name}>
                            {adapterLabel(adapter.name)}
                            {adapter.name === 'github-copilot'
                              ? ` • ${adapter.models.filter((model) => model.policy_state !== 'disabled').length} models`
                              : adapter.name === 'fake-process'
                                ? ' • no AI'
                                : ''}
                          </option>
                        ))}
                      </select>
                      <small
                        className={
                          deterministicHarness || effectiveMissionAdapter === 'fake-process'
                            ? 'field-warning'
                            : ''
                        }
                      >
                        {deterministicHarness
                          ? 'This test fixture owns its runtime settings.'
                          : selectedAdapter
                            ? adapterDescription(selectedAdapter.name)
                            : 'Connect a runner to unlock an agent runtime.'}
                      </small>
                    </div>
                    {!deterministicHarness && selectedAdapter?.models.length ? (
                      <div className="mission-field">
                        <label htmlFor="mission-model">Model</label>
                        <select
                          id="mission-model"
                          value={selectedModel ? missionModel : ''}
                          onChange={(event) => {
                            const nextModel = selectedAdapter.models.find(
                              (model) => model.id === event.target.value,
                            )
                            setMissionModel(event.target.value)
                            setMissionReasoningEffort(
                              nextModel?.default_reasoning_effort ?? '',
                            )
                          }}
                        >
                          <option value="">Provider default</option>
                          {selectedAdapter.models.map((model) => (
                            <option
                              key={model.id}
                              value={model.id}
                              disabled={model.policy_state === 'disabled'}
                            >
                              {model.name} • {model.id}
                              {model.policy_state === 'disabled' ? ' • disabled' : ''}
                            </option>
                          ))}
                        </select>
                        <small>
                          {selectedModel
                            ? `${selectedModel.max_context_window_tokens?.toLocaleString() ?? 'Unknown'} context tokens${selectedModel.supports_vision ? ' • vision' : ''}`
                            : 'Use the provider default or select an enabled model.'}
                        </small>
                      </div>
                    ) : null}
                    {!deterministicHarness && selectedModel?.supports_reasoning_effort ? (
                      <div className="mission-field">
                        <label htmlFor="mission-reasoning">Reasoning</label>
                        <select
                          id="mission-reasoning"
                          value={
                            selectedModel.supported_reasoning_efforts.includes(
                              missionReasoningEffort,
                            )
                              ? missionReasoningEffort
                              : ''
                          }
                          onChange={(event) =>
                            setMissionReasoningEffort(event.target.value)
                          }
                        >
                          <option value="">Provider default</option>
                          {selectedModel.supported_reasoning_efforts.map((effort) => (
                            <option key={effort} value={effort}>
                              {effort}
                            </option>
                          ))}
                        </select>
                      </div>
                    ) : null}
                    {!deterministicHarness ? (
                      <div className="mission-field">
                        <label htmlFor="mission-budget">Token ceiling</label>
                        <select
                          id="mission-budget"
                          value={missionBudgetTokens}
                          onChange={(event) =>
                            setMissionBudgetTokens(Number(event.target.value))
                          }
                        >
                          <option value={500_000}>Quick • 500K</option>
                          <option value={1_000_000}>Standard • 1M</option>
                          <option value={2_000_000}>Large • 2M</option>
                        </select>
                        <small>A hard mission safety ceiling.</small>
                      </div>
                    ) : null}
                    <div className="mission-field">
                      <label htmlFor="mission-deliverable">Deliverable</label>
                      <select
                        id="mission-deliverable"
                        value={missionDeliverable}
                        onChange={(event) =>
                          setMissionDeliverable(
                            event.target.value as NonNullable<
                              TaskContract['deliverable']
                            >['form'],
                          )
                        }
                      >
                        <option value="archive">Source archive</option>
                        <option value="patch">Git patch</option>
                        <option value="typed_artifact_set">Typed artifact set</option>
                        <option value="commit_branch">Commit and branch bundle</option>
                        <option value="review_only_report">Review report</option>
                      </select>
                      <small>Portable source bytes, never a runner-local path.</small>
                    </div>
                    <div className="mission-field">
                      <label htmlFor="mission-strategy">Execution strategy</label>
                      <select
                        id="mission-strategy"
                        value={missionStrategy}
                        onChange={(event) => setMissionStrategy(event.target.value)}
                      >
                        <option value="single">Solo run</option>
                        <option value="parallel-specialists">Two specialists and synthesis</option>
                        {developerMode ? (
                          <optgroup label="Test fixtures">
                            <option value="verification-matrix">Verification matrix</option>
                            <option value="human-approval">Human approval</option>
                            <option value="independent-review">Independent review</option>
                            <option value="verification-failure">Failure path</option>
                          </optgroup>
                        ) : null}
                      </select>
                      <small>
                        {missionStrategy === 'parallel-specialists'
                          ? 'Parallel roots converge on one synthesis task.'
                          : missionStrategy === 'single'
                            ? 'One bounded worker owns the outcome.'
                            : 'A deterministic product-behavior fixture.'}
                      </small>
                    </div>
                  </div>
                  <div className="loadout-switches">
                    <label className="mission-run-toggle">
                      <input
                        type="checkbox"
                        checked={commitDeliverable || missionDeliverable === 'commit_branch'}
                        disabled={missionDeliverable === 'commit_branch'}
                        onChange={(event) => setCommitDeliverable(event.target.checked)}
                      />
                      <span>
                        <strong>Commit verified work</strong>
                        <small>Only inside the isolated task branch.</small>
                      </span>
                    </label>
                    <label className="mission-run-toggle">
                      <input
                        type="checkbox"
                        checked={pauseAfterPlanning}
                        onChange={(event) => setPauseAfterPlanning(event.target.checked)}
                      />
                      <span>
                        <strong>Hold at briefing</strong>
                        <small>Review the generated plan before launch.</small>
                      </span>
                    </label>
                    <label className="developer-mode-toggle">
                      <input
                        type="checkbox"
                        checked={developerMode}
                        onChange={(event) => {
                          const enabled = event.target.checked
                          setDeveloperMode(enabled)
                          if (!enabled && usesDeterministicHarness(missionStrategy)) {
                            setMissionStrategy('single')
                          }
                        }}
                      />
                      <span>
                        <strong>Developer fixtures</strong>
                        <small>Expose deterministic lifecycle fixtures.</small>
                      </span>
                    </label>
                  </div>
                </div>
              ) : null}

              {missionComposerStep === 'proof' ? (
                <div className="mission-stage-content stage-proof">
                  <div className="stage-title">
                    <span>Verification policy</span>
                    <h3>Define completion evidence</h3>
                    <p>Only the active contract panel is shown. The persisted plan stays inspectable.</p>
                  </div>
                  <div className="contract-tab-shell">
                    <nav className="contract-tab-nav" aria-label="Mission contract sections">
                      {[
                        ['outcome', 'Outcome'],
                        ['guardrails', 'Guardrails'],
                        ['context', 'Context'],
                      ].map(([tab, label]) => (
                        <button
                          key={tab}
                          type="button"
                          className={missionContractTab === tab ? 'contract-tab-active' : ''}
                          onClick={() =>
                            setMissionContractTab(
                              tab as 'outcome' | 'guardrails' | 'context',
                            )
                          }
                        >
                          {label}
                        </button>
                      ))}
                    </nav>
                    <div className="contract-tab-panel">
                      {missionContractTab === 'outcome' ? (
                        <div className="mission-contract-grid">
                          <label>
                            Authoritative objective
                            <textarea
                              rows={6}
                              value={missionObjective}
                              onChange={(event) => setMissionObjective(event.target.value)}
                              placeholder="What must be true when the task is complete?"
                            />
                          </label>
                          <label>
                            Expected output
                            <textarea
                              rows={6}
                              value={missionExpectedOutput}
                              onChange={(event) =>
                                setMissionExpectedOutput(event.target.value)
                              }
                              placeholder="Runnable application, report, migration, or commit."
                            />
                          </label>
                        </div>
                      ) : null}
                      {missionContractTab === 'guardrails' ? (
                        <div className="mission-contract-grid">
                          <label>
                            Allowed tools
                            <textarea
                              rows={6}
                              value={missionAllowedTools}
                              onChange={(event) => setMissionAllowedTools(event.target.value)}
                              placeholder={'filesystem\nshell\ntest'}
                            />
                          </label>
                          <label>
                            Prohibited actions
                            <textarea
                              rows={6}
                              value={missionProhibitedActions}
                              onChange={(event) =>
                                setMissionProhibitedActions(event.target.value)
                              }
                              placeholder={'modify files outside the worktree\nforce push\ndisable verification'}
                            />
                          </label>
                          <label className="contract-wide-field">
                            Authorized write scope
                            <textarea
                              rows={4}
                              value={missionWriteScope}
                              onChange={(event) => setMissionWriteScope(event.target.value)}
                              placeholder={'apps/web/**\ncrates/crony-server/**'}
                            />
                          </label>
                        </div>
                      ) : null}
                      {missionContractTab === 'context' ? (
                        <div className="mission-contract-grid">
                          <label>
                            Acceptance criteria
                            <textarea
                              rows={7}
                              value={missionAcceptanceTests}
                              onChange={(event) =>
                                setMissionAcceptanceTests(event.target.value)
                              }
                              placeholder={'All targeted tests pass\nThe browser flow succeeds'}
                            />
                          </label>
                          <label>
                            References and approved context
                            <textarea
                              rows={7}
                              value={missionReferences}
                              onChange={(event) => setMissionReferences(event.target.value)}
                              placeholder={'docs/SPEC.md\nhttps://github.com/owner/repo/issues/123'}
                            />
                          </label>
                        </div>
                      ) : null}
                    </div>
                  </div>

                  <label className="mission-run-toggle custom-verification-toggle">
                    <input
                      type="checkbox"
                      checked={customVerification && !deterministicHarness}
                      disabled={deterministicHarness}
                      onChange={(event) => {
                        setCustomVerification(event.target.checked)
                        if (event.target.checked) setPauseAfterPlanning(true)
                      }}
                    />
                    <span>
                      <strong>Custom verification</strong>
                      <small>Run exact file, test, schema, screenshot, and reviewer checks.</small>
                    </span>
                  </label>
                  {customVerification && !deterministicHarness ? (
                    <VerificationPolicyEditor
                      policy={missionVerificationPolicy}
                      onChange={setMissionVerificationPolicy}
                      idPrefix="mission"
                    />
                  ) : deterministicHarness ? (
                    <small className="field-warning">
                      Debug cartridges own their verifier policy.
                    </small>
                  ) : (
                    <div className="default-gate-callout">
                      <span>Default verification</span>
                      <strong>Provider artifact must exist</strong>
                      <small>Turn on custom verification for application-level proof.</small>
                    </div>
                  )}
                </div>
              ) : null}
            </section>

            <div className="arcade-form-controls">
              {missionComposerStep !== 'brief' ? (
                <button
                  className="button button-quiet"
                  type="button"
                  onClick={() =>
                    setMissionComposerStep(
                      missionComposerStep === 'proof' ? 'loadout' : 'brief',
                    )
                  }
                >
                  Back
                </button>
              ) : <span />}
              {missionComposerStep === 'brief' ? (
                <button
                  className="button button-primary"
                  type="button"
                  disabled={!missionTitle.trim()}
                  onClick={() => setMissionComposerStep('loadout')}
                >
                  Configure run
                </button>
              ) : null}
              {missionComposerStep === 'loadout' ? (
                <button
                  className="button button-primary"
                  type="button"
                  disabled={
                    !selectedMissionSource ||
                    !missionSourceConfirmed ||
                    !effectiveMissionAdapter
                  }
                  onClick={() => setMissionComposerStep('proof')}
                >
                  Set verification
                </button>
              ) : null}
              {missionComposerStep === 'proof' ? (
                <button
                  className="button button-primary mission-submit"
                  type="submit"
                  disabled={
                    busy ||
                    !missionTitle.trim() ||
                    !selectedMissionSource ||
                    !missionSourceConfirmed ||
                    !effectiveMissionAdapter ||
                    missionVerifierErrors.length > 0
                  }
                >
                  {busy
                    ? 'Starting mission'
                    : pauseAfterPlanning
                      ? 'Create mission plan'
                      : 'Launch mission'}
                </button>
              ) : null}
            </div>
            {missionVerifierErrors.length ? (
              <p className="contract-error">{missionVerifierErrors[0]}</p>
            ) : null}
            {missionComposerStep === 'proof' ? (
              <p className="mission-submit-note">
                {pauseAfterPlanning
                  ? 'The crew waits at briefing until you dispatch the reviewed plan.'
                  : 'The mission launches immediately. Risky effects still require approval.'}
              </p>
            ) : null}
          </form>
          )}
          <div
            className={`mission-console ${
              latestMissions.length ? '' : 'mission-console-empty'
            }`}
          >
            {latestMissions.length ? (
              <nav className="mission-selector" aria-label="Mission records">
                {latestMissions.map((mission) => {
                  const missionTasks = data.snapshot.tasks.filter(
                    (task) => task.mission_id === mission.id,
                  )
                  return (
                    <button
                      key={mission.id}
                      type="button"
                      className={
                        selectedMission?.id === mission.id ? 'mission-selector-active' : ''
                      }
                      aria-pressed={selectedMission?.id === mission.id}
                      data-status={mission.status}
                      onClick={() => setSelectedMissionId(mission.id)}
                    >
                      <span>{statusLabel(mission.status)}</span>
                      <strong>{mission.title}</strong>
                      <small>
                        {shortId(mission.id)} · {missionTasks.length} task
                        {missionTasks.length === 1 ? '' : 's'}
                      </small>
                    </button>
                  )
                })}
              </nav>
            ) : null}
            <div className="mission-list">
              {selectedMission ? (
                <MissionCard
                  key={selectedMission.id}
                  mission={selectedMission}
                  tasks={selectedMissionTasks}
                  runs={selectedMissionRuns}
                  agents={data.snapshot.agents}
                  evidence={data.snapshot.verification_evidence}
                  deliverables={data.snapshot.source_deliverables}
                  revisions={data.snapshot.mission_budget_revisions.filter(
                    (revision) => revision.mission_id === selectedMission.id,
                  )}
                  contractRevisions={data.snapshot.mission_contract_revisions.filter(
                    (revision) => revision.mission_id === selectedMission.id,
                  )}
                  actors={data.snapshot.actors}
                  verificationRequests={data.snapshot.verification_requests}
                  actionApprovals={data.snapshot.action_approvals}
                  actorId={selectedActor.id}
                  actorRole={selectedActor.role}
                  busy={busy}
                  onLaunch={launchMission}
                  onResume={resumeAgentRun}
                  onDownloadArtifact={downloadArtifact}
                  onDownloadDeliverable={downloadDeliverable}
                  onProposeBudgetRevision={proposeBudgetRevision}
                  onBudgetRevisionDecision={decideBudgetRevision}
                  onContractRevision={createContractRevision}
                  onVerificationDecision={decideVerification}
                  onActionApprovalDecision={decideActionApproval}
                />
              ) : (
                <div className="empty-state">
                  <strong>No missions yet</strong>
                  <span>Start with a concrete outcome and let ECorp create the task contract.</span>
                </div>
              )}
            </div>
          </div>
        </aside>
      </section>

      <div className="workspace-surface" hidden={activeWorkspaceView !== 'room'}>
        <RoomPanel
          room={room}
          messages={data.snapshot.room_messages}
          actors={data.snapshot.actors}
          selectedActor={selectedActor}
          missions={data.snapshot.missions}
          tasks={data.snapshot.tasks}
          runs={data.snapshot.runs}
          onPost={postRoomMessage}
          onNavigateLink={(link) => navigateToWorkspaceEntity(link.kind, link.id)}
        />
      </div>

      <section
        className="operations-panel panel workspace-surface"
        id="activity"
        hidden={activeWorkspaceView !== 'activity'}
        tabIndex={-1}
      >
        <div className="panel-heading operations-heading">
          <div>
            <span className="section-code">Audit trail</span>
            <h2>Immutable activity</h2>
          </div>
          <div className="operations-summary">
            <span>{data.snapshot.missions.length} missions</span>
            <span>{data.snapshot.runs.length} runs</span>
            <span>{data.snapshot.circuit_breaker_incidents.length} breaker events</span>
            <span>Showing {latestEvents.length} of {data.snapshot.events.length} events</span>
          </div>
        </div>
        {data.snapshot.action_approvals.some((approval) => approval.status === 'pending') ? (
          <div className="approval-queue" data-testid="action-approval-queue">
            {data.snapshot.action_approvals
              .filter((approval) => approval.status === 'pending')
              .map((approval) => (
                <article className="verification-card" key={approval.id}>
                  <div>
                    <strong>{approval.action}</strong>
                    <span>
                      {approval.risk} risk · decision controls are in the owning mission card
                    </span>
                  </div>
                </article>
              ))}
          </div>
        ) : null}
        <ol className="event-list" data-testid="event-list">
          {latestEvents.map((event) => (
            <EventRow key={event.id} event={event} actors={data.snapshot.actors} />
          ))}
        </ol>
      </section>
    </main>
  )
}

export default App
