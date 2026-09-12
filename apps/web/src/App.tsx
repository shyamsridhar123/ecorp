import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import type { FormEvent } from 'react'
import './App.css'
import './Arcade.css'
import './Cabinet.css'
import './World.css'
import './Accessible.css'
import './OperationsUx.css'
import { OfficeFloor, OfficePortrait } from './OfficeFloor'
import { OfficeInspector } from './OfficeInspector'
import { FactoryPollingNotice } from './FactoryPollingNotice'
import { factoryControllerState } from './factoryPolling'
import { selectFactoryController } from './factoryControllerSelection'
import type { FactoryPolling } from './factoryPolling'
import {
  factoryContractRevisionSource, factoryRecoveryBlocksProviderResume, factoryRecoveryConnection,
  factoryRecoveryModes, needsFactoryRecoveryContext,
} from './factoryCheckpointRecovery'
import type { FactoryRecoveryCommandMode } from './factoryCheckpointRecovery'
import { currentOfficeAgents, operatingOfficeAgents, selectOfficeAgent } from './office/officeModel'
import type { OfficeAgent } from './office/officeModel'
import {
  availableRunnerAdapters, missionRuntimeError, selectMissionAdapter,
  STUDIO_STRATEGY, STUDIO_STRATEGY_LABEL, usesDeterministicHarness, workspaceCapability,
} from './missionRuntime'
import type { RepositoryTarget, RunnerCapability, RunnerNode } from './missionRuntime'
import {
  buildMissionRequest, currentMissionPreview, missionRequestScope, startMissionPreview,
} from './missionPreview'
import type { MissionPreviewLoad, MissionRequestScope } from './missionPreview'
import { isProviderLiveRun, missionIdForLink, pendingReviewForRun, relatedWorkOptions, reviewBlockedReason, selectMissionEvidenceRun, workLinkLabel, workflowTaskLabel } from './workflowContext'
import { canPostRoomMessage, discussionScopeKey, missionOrigin, resolveDiscussionRoom, roomDiscussionMessages, roomWorkContext } from './missionProjection'
import type { DiscussionScope } from './missionProjection'
import { createSnapshotRefresher } from './snapshotRefresh'
import { evidenceSelectionKey, readEvidenceSelection, rememberEvidenceSelection } from './evidenceSelection'
import { ConnectionsPanel } from './ConnectionsPanel'
import { MissionOriginDetails } from './MissionOriginDetails'
import {
  connectionLabel, connectionRunnerRevision, connectionScope, connectionStatusLabel,
  connectionTarget, connectionsNeedPresenceRefresh,
} from './workspaceConnections'
import type { WorkspaceConnection, WorkspaceConnections } from './workspaceConnections'

type Actor = {
  id: string
  name: string
  kind: 'human' | 'agent' | 'service'
  role: string
}

type Agent = OfficeAgent & {
  actor_id: string
  accent: string
}

type Mission = {
  id: string
  room_id: string
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
  workspace_fingerprint: string | null
  execution_mode: 'provider' | 'verification_only'
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
  corp_id: string
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
  corp_id: string
  service_actor_id: string
  configured_by: string
  source_project_owner: string
  source_project_number: number
  source_repository_owner: string
  source_repository_name: string
  desired_state: 'running' | 'paused'
  status: 'offline' | 'watching' | 'working' | 'blocked' | 'needs_decision' | 'backing_off'
  polling?: FactoryPolling | null
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

type FactoryVerificationRecovery = {
  id: string
  factory_work_item_id: string
  mission_id: string
  task_id: string
  source_run_id: string
  replacement_run_id: string | null
  mode: 'source_correction' | 'verifier_only' | 'checkpoint_verification'
  status: 'authorized' | 'running' | 'completed' | 'failed'
  authorized_by: string
  reason: string
  observed_source_revision: string
  contract_revision_id: string | null
  previous_verification_policy: VerificationPolicy
  replacement_verification_policy: VerificationPolicy
  created_at: string
  updated_at: string
}

type FactoryVerificationRecoveryContextResponse = {
  work_item: FactoryWorkItem
  recoveries: FactoryVerificationRecovery[]
  mission_id: string
  task_id: string
  source_run_id: string
  remaining_attempts: number
  remaining_mission_tokens: number
  remaining_mission_cost_microusd: number
  workspace_fingerprint: string | null
  expected_head_commit: string | null
  checkpoint_verification?: boolean
  checkpoint_verification_available?: boolean
  checkpoint_source_correction?: boolean
  checkpoint_cancellation_event_id?: string | null
}

type FactoryRecoveryContextScope = {
  corpId: string
  actorId: string
  missionId: string
  itemId: string
  version: number
  reload: number
}

type FactoryRecoveryContextLoad = {
  scopeKey: string
  status: 'loading' | 'ready' | 'error'
  data: FactoryVerificationRecoveryContextResponse | null
  error: string | null
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

type RoomPostInput = {
  source: 'room' | 'factory'
  scope: DiscussionScope
  roomId: string
  body: string
  replyToId: string | null
  mentions: string[]
  link: EntityLink | null
  idempotencyKey: string
}

type BrowserSocketMessage =
  | { type: 'ready'; corp_id: string; replayed_through: number }
  | { type: 'event'; event: DomainEvent }

type WorkspaceView = 'floor' | 'factory' | 'missions' | 'room' | 'activity'

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
    factory_verification_recoveries: FactoryVerificationRecovery[]
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
  replayed?: boolean
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

function missionStatusLabel(value: string): string {
  return value === 'ready' ? 'Awaiting dispatch' : statusLabel(value)
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

function ReviewEligibilityNotice({ reason, id }: { reason: string; id: string }) {
  const demo = !storedAccessToken()
  return (
    <div className="review-eligibility-notice" id={id} role="status">
      <strong>A different reviewer is needed</strong>
      <p>{reason}</p>
      {demo ? (
        <button type="button" className="button button-secondary" onClick={() => {
          const selector = document.getElementById('operator-actor')
          selector?.scrollIntoView({ block: 'center', behavior: 'auto' })
          selector?.focus()
        }}>
          Choose another demo operator
        </button>
      ) : <p>Ask another authorized room member to review using their own account.</p>}
    </div>
  )
}

function terminalRun(status: string): boolean {
  return ['completed', 'failed', 'cancelled', 'lost'].includes(status)
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

function isEcorpRepository(target: RepositoryTarget | undefined): boolean {
  return target?.repository.toLowerCase().endsWith('/ecorp') ?? false
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
          <strong>Verification checks</strong>
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
  recoveryScope,
  recoveryLoad,
  actorId,
  actorRole,
  busy,
  onRevise,
}: {
  mission: Mission
  task: Task
  runs: Run[]
  recoveryScope: FactoryRecoveryContextScope | null
  recoveryLoad: FactoryRecoveryContextLoad | null
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
  const scopedRecoveryLoad = currentFactoryRecoveryLoad(recoveryScope, recoveryLoad)
  const recoveryContext = scopedRecoveryLoad?.status === 'ready' ? scopedRecoveryLoad.data : null
  const sourceRunId = recoveryScope
    ? factoryContractRevisionSource(recoveryContext, recoveryScope, task.id)
    : runs.find(
      (run) =>
        run.task_id === task.id &&
        terminalRun(run.status) &&
        run.provider_session_id &&
        run.workspace_disposition === 'preserved' &&
        run.breaker_stage !== 'stop',
    )?.id ?? null
  const nextAction: MissionContractRevisionInput['next_action'] | null =
    !recoveryScope && !activeRun && redispatchEligible ? 'redispatch' : !activeRun && sourceRunId ? 'resume' : null
  const recoverySourceNotice = recoveryScope && !activeRun && task.status !== 'completed'
    ? scopedRecoveryLoad?.status === 'error'
      ? 'Recovery source unavailable. Refresh recovery context below before revising.'
      : scopedRecoveryLoad?.status !== 'ready'
        ? 'Loading the current recovery source…'
        : recoveryContext?.task_id === task.id && !sourceRunId
          ? 'Source correction is not available in the current recovery context.'
          : null
    : null
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
    if (!canRevise || busy || !nextAction || !parsedContract || !parsedPolicy || !reason.trim() || parseError) return
    const saved = await onRevise(mission, task, {
      task_id: task.id,
      expected_contract_version: task.contract_version,
      next_action: nextAction,
      source_run_id: nextAction === 'resume' ? sourceRunId : null,
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

  if (!canRevise) return null
  if (!nextAction) return recoverySourceNotice ? (
    <p className="factory-recovery-role-note"
      data-testid={`contract-revision-source-${task.id}`}
      role={scopedRecoveryLoad?.status === 'error' ? 'alert' : 'status'}>
      {recoverySourceNotice}
    </p>
  ) : null
  return (
    <div className="contract-revision-panel" data-testid={`contract-revision-${task.id}`}
      data-source-run-id={sourceRunId}>
      {open ? (
        <form onSubmit={submit}>
          <div className="contract-section-heading">
            <div>
              <strong>
                Revise for {nextAction === 'resume'
                  ? recoveryScope ? 'source correction' : 'preserved-session resume' : 'redispatch'}
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
                `Save revision ${task.contract_version + 1}; then explicitly ${nextAction === 'resume'
                  ? recoveryScope ? 'request source correction' : 'resume the preserved run' : 'dispatch the mission'}.`}
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
          Revise contract for {nextAction === 'resume' && recoveryScope ? 'source correction' : nextAction}
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
  scope,
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
  onOpenMission,
  onDiscussMission,
  onNewMission,
  selectedItemId,
  onSelectItem,
}: {
  items: FactoryWorkItem[]
  missions: Mission[]
  publications: PullRequestPublication[]
  publicationAttempts: PullRequestPublicationAttempt[]
  controllers: FactoryController[]
  tasks: Task[]
  runs: Run[]
  room: { id: string; name: string; purpose: string } | undefined
  scope: DiscussionScope
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
  onPostComment: (input: RoomPostInput) => Promise<boolean>
  onActionDecision: (approval: ActionApproval, approved: boolean) => Promise<void>
  onVerificationDecision: (run: Run, approved: boolean) => Promise<void>
  onClaimLease: (agent: Agent) => Promise<void>
  onSteer: (
    agent: Agent,
    text: string,
    token: string | undefined,
    idempotencyKey: string,
  ) => Promise<boolean>
  onOpenMission: (mission: Mission) => void
  onDiscussMission: (mission: Mission) => void
  onNewMission: () => void
  selectedItemId: string | null
  onSelectItem: (item: FactoryWorkItem) => void
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
  const selected = selectedItemId === null
    ? items[0]
    : items.find((item) => item.id === selectedItemId)
  const selectedMission = missions.find((candidate) => candidate.id === selected?.mission_id)
  const selectedTasks = tasks.filter((task) => task.mission_id === selectedMission?.id)
  const selectedTaskIds = new Set(selectedTasks.map((task) => task.id))
  const selectedRuns = runs.filter((run) => selectedTaskIds.has(run.task_id))
  const activeRun = selectedRuns.find((run) => isProviderLiveRun(run, verificationRequests))
  const activeAgent = agents.find(
    (agent) => agent.id === activeRun?.agent_id && agent.retired_at == null,
  )
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
  const contextualMessages = selectedMission
    ? roomDiscussionMessages(messages, room?.id, selectedMission.id,
      roomWorkContext({ missions, tasks, runs }, room?.id, selectedMission.id)).slice(-8)
    : []
  const otherMissions = missions.filter((mission) =>
    missionOrigin(mission.id, {
      factory_work_items: items, pull_request_publications: publications,
    }).kind === 'unknown',
  )
  const selectedPublication = publications.find(
    (candidate) => candidate.factory_work_item_id === selected?.id,
  )
  const selectedAttempts = publicationAttempts
    .filter((attempt) => attempt.publication_id === selectedPublication?.id)
    .sort((left, right) => right.attempt - left.attempt)
  const controller = selectFactoryController(
    controllers, selected, scope.corpId, selectedItemId,
  )
  const controllerState = factoryControllerState(controller)

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
          <h2>GitHub work, on autopilot</h2>
          <p>
            Factory turns eligible GitHub issues into missions. Agents build, checks verify,
            and results become ready for review. Merging is a separate decision.
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
              : selected
                ? 'No trusted Factory watcher matches the selected Project and repository.'
                : selectedItemId
                  ? 'The selected Factory item is unavailable. Choose an item to view its controller.'
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
          <div className="factory-start-help">
            <p>{selected || selectedItemId
              ? 'No controller is available for the selected Factory context. Existing missions can continue independently.'
              : 'Automatic GitHub intake is off. Your runner is separate and can still execute direct missions.'}</p>
            <button type="button" className="button button-primary"
              disabled={!canOperate(selectedActor.role)} onClick={onNewMission}>
              Start a direct mission
            </button>
            <details>
              <summary>How to enable automatic intake</summary>
              <ol>
                <li>Choose a GitHub Project and repository, then mark reviewed issues eligible for Factory.</li>
                <li>Start the trusted <code>crony factory-watch</code> controller for that source. A connected agent runner alone does not start intake.</li>
                <li>Its status appears here. Use Pause intake, Resume intake and Reconcile now to operate it.</li>
              </ol>
              <p>Comments discuss work. Agent direction and review decisions use their own explicit controls.</p>
            </details>
          </div>
        )}
        {controller ? <FactoryPollingNotice controller={controller} /> : null}
        {controller?.last_error ? (
          <p className="factory-controller-error" role="alert">
            {controller.last_error}
          </p>
        ) : null}
      </section>
      {otherMissions.length ? (
        <section className="manual-work-links" aria-label="Other missions">
          <strong>Other missions</strong>
          <p>These missions are not linked in this Factory view. Their intake origin may be unavailable.</p>
          <div className="work-context-actions">
            {otherMissions.slice(0, 5).map((mission) => (
              <button key={mission.id} type="button" className="button button-secondary"
                onClick={() => onOpenMission(mission)}>
                {mission.title}
              </button>
            ))}
          </div>
        </section>
      ) : null}
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
                    onClick={() => onSelectItem(item)}
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
                  {selectedMission ? (
                    <nav className="work-context-actions" aria-label="This work item">
                      <button type="button" className="button button-secondary"
                        onClick={() => onOpenMission(selectedMission)}>Open mission and results</button>
                      <button type="button" className="button button-secondary"
                        onClick={() => onDiscussMission(selectedMission)}>Open discussion</button>
                    </nav>
                  ) : null}

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
                          const blockedReason = reviewBlockedReason(
                            request.gate, selectedActor, selectedMission.requested_by,
                          )
                          const eligible = !blockedReason
                          const noticeId = `factory-review-eligibility-${request.run_id}`
                          const reviewTask = selectedTasks.find((task) => task.id === request.task_id)
                          return run ? (
                            <article key={request.run_id}>
                              <strong>{statusLabel(request.gate_type)}</strong>
                              <p>{reviewTask ? `${workflowTaskLabel(reviewTask.plan_key)}: ${reviewTask.title}` : 'Review the persisted verification evidence for this run.'}</p>
                              <small>Run {shortId(run.id)}</small>
                              {blockedReason ? <ReviewEligibilityNotice reason={blockedReason} id={noticeId} /> : null}
                              <div>
                                <button
                                  type="button"
                                  disabled={busy || !eligible}
                                  onClick={() => void onVerificationDecision(run, false)}
                                  aria-describedby={blockedReason ? noticeId : undefined}
                                >
                                  Reject
                                </button>
                                <button
                                  type="button"
                                  className="button button-primary"
                                  disabled={busy || !eligible}
                                  onClick={() => void onVerificationDecision(run, true)}
                                  aria-describedby={blockedReason ? noticeId : undefined}
                                >
                                  {busy ? 'Submitting decision…' : 'Accept evidence'}
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
                                `ecorp:factory-comment:${discussionScopeKey(scope)}:${selected.id}`
                              const idempotencyKey = browserOperationKey(
                                operationStorageKey,
                                payload,
                              )
                              void onPostComment({
                                source: 'factory',
                                scope,
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
  const live = Boolean(agent.current_run_id) && agent.status !== 'idle' && agent.status !== 'offline'
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
            {agent.mission_id ? ` · mission ${shortId(agent.mission_id)}` : ''}
            {agent.pinned ? ' · pinned' : ''}
          </div>
          <div className={`lease-label ${ownsLease ? 'lease-owned' : ''}`}>{holderLabel}</div>
        </div>
      </div>
      <p className="agent-inspector-help">
        {live
          ? `${agent.name} is ${agent.status === 'working' ? agent.station ?? agent.status : agent.status}. Claim control to steer the live session.`
          : agent.status === 'reviewing'
            ? `${agent.name}'s provider process has ended. The recorded output is awaiting evidence review.`
          : agent.status === 'offline'
            ? `${agent.name}'s runner is unavailable. Inspect the mission's recorded state and recovery controls.`
          : agent.mission_id && !agent.pinned
            ? `${agent.name} reports ${agent.status}. This identity belongs to mission ${shortId(agent.mission_id)}; any next task is assigned by the server.`
          : agent.status === 'idle'
            ? `${agent.name} is off shift. No provider process is running; the identity remains available for future ${adapterLabel(agent.adapter)} missions.`
          : `${agent.name} reports ${agent.status}. No current provider run is reported.`}
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
  const completedMission = mission.status === 'completed'
  const budgetNeedsAction = budgetExhausted && !completedMission
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
      className={`budget-ledger${budgetNeedsAction ? ' budget-ledger-exhausted' : ''}`}
      data-testid="mission-budget-ledger"
      data-budget-exhausted={budgetExhausted}
    >
      <div className="budget-ledger-heading">
        <div>
          <span>Mission budget authority</span>
          <strong>
            {completedMission
              ? 'Completed · recorded spend'
              : budgetExhausted
              ? 'Recovery authorization required'
              : pendingRevision
                ? 'Revision awaiting decision'
                : 'Authorized capacity'}
          </strong>
        </div>
        <span className={`budget-state budget-state-${budgetExhausted ? 'exhausted' : 'available'}`}>
          {completedMission ? 'Recorded' : budgetExhausted ? 'Exhausted' : 'Available'}
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

      {budgetNeedsAction && !pendingRevision && canManageBudget && canReviseNow ? (
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

      {completedMission ? (
        <p className="budget-guidance">
          This mission is complete. Usage remains recorded; no budget revision or replacement mission is needed for this result.
        </p>
      ) : null}
      {budgetNeedsAction && !canManageBudget ? (
        <p className="budget-guidance">
          {resumableRun?.breaker_stage === 'stop'
            ? 'This provider session cannot restart after a stop. Inspect its preserved work and evidence instead of discarding it.'
            : 'Resume is locked. An owner or admin must approve a higher mission ceiling before another provider session starts.'}
        </p>
      ) : null}
      {budgetNeedsAction && canManageBudget && !canReviseNow && !pendingRevision ? (
        <p className="budget-guidance">
          {resumableRun?.breaker_stage === 'stop'
            ? 'This provider session cannot restart after a stop. Inspect its preserved work and evidence instead of discarding it.'
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

// Only exact run-bound receipts establish a total. Task policies can be revised
// without starting a run, and the absence of older history in a snapshot proves nothing.
function automatedVerificationPresentation(
  run: Run | undefined,
  evidence: VerificationEvidence[],
  events: DomainEvent[],
  recoveries: FactoryVerificationRecovery[],
) {
  const records = run
    ? evidence.filter((item) => item.run_id === run.id && item.task_id === run.task_id)
        .toSorted((left, right) => left.check_index - right.check_index)
    : []
  const receipts = run ? [
    ...events.filter((event) =>
      event.type === 'run.verification_started' &&
      event.aggregate_type === 'run' && event.aggregate_id === run.id,
    ).map((event) => event.payload.check_count),
    ...recoveries.filter((recovery) =>
      recovery.replacement_run_id === run.id && recovery.task_id === run.task_id,
    ).map((recovery) => recovery.replacement_verification_policy?.checks?.length),
  ] : []
  const validReceipts = receipts.filter(
    (count): count is number => typeof count === 'number' && Number.isSafeInteger(count) && count > 0,
  )
  const totals = new Set(validReceipts)
  const conflicting = receipts.length !== validReceipts.length || totals.size > 1
  const total = !conflicting && totals.size === 1 ? [...totals][0] : null
  const inconsistent = new Set(records.map((item) => item.check_index)).size !== records.length ||
    records.some((item) => !Number.isInteger(item.check_index) || item.check_index < 0 ||
      !['passed', 'failed'].includes(item.status) ||
      (total !== null && item.check_index >= total))
  const passed = records.filter((item) => item.status === 'passed').length
  const failed = records.filter((item) => item.status === 'failed').length
  const missing = total !== null && !inconsistent ? total - records.length : null
  const complete = missing === 0 && failed === 0
  const status = failed > 0 ? 'failed' : complete ? 'passed' : 'pending'
  const summary = inconsistent
    ? 'Check records are inconsistent; inspect the recorded evidence.'
    : conflicting
      ? 'Run-bound check count is inconsistent; completeness is unknown.'
      : total === null
        ? 'Run-bound check total unavailable; recorded results only.'
        : records.length === 0
          ? 'No check results recorded'
          : failed > 0
            ? `${failed} check${failed === 1 ? '' : 's'} failed`
            : complete ? 'All recorded checks passed' : 'Recorded checks passed; evidence is incomplete.'
  const score = inconsistent
    ? `${passed} passing records · completeness unknown`
    : total === null ? `${passed} passed · total unknown` : `${passed}/${total} passed`
  return { records, total, passed, failed, missing, status, summary, score }
}

function factoryRecoveryScopeKey(scope: FactoryRecoveryContextScope): string {
  return JSON.stringify([
    scope.corpId, scope.actorId, scope.missionId, scope.itemId, scope.version, scope.reload,
  ])
}

function currentFactoryRecoveryLoad(
  scope: FactoryRecoveryContextScope | null,
  load: FactoryRecoveryContextLoad | null,
) {
  return scope && load?.scopeKey === factoryRecoveryScopeKey(scope) ? load : null
}

// Exact, authenticated, read-only context. A cleanup fences even a transport that
// finishes after abort; the render-time scope check also hides old data immediately.
function requestFactoryRecoveryContext(
  scope: FactoryRecoveryContextScope,
  publish: (load: FactoryRecoveryContextLoad) => void,
) {
  const controller = new AbortController()
  const scopeKey = factoryRecoveryScopeKey(scope)
  let current = true
  publish({ scopeKey, status: 'loading', data: null, error: null })
  const timeout = setTimeout(() => {
    if (current && !controller.signal.aborted) {
      publish({ scopeKey, status: 'error', data: null, error: 'Recovery context request timed out. Retry the read-only lookup.' })
      controller.abort()
    }
  }, 15_000)
  void api<FactoryVerificationRecoveryContextResponse>(
    `/api/corps/${encodeURIComponent(scope.corpId)}/factory/work-items/${encodeURIComponent(scope.itemId)}/verification-recoveries?actor_id=${encodeURIComponent(scope.actorId)}`,
    { method: 'GET', signal: controller.signal },
  ).then((data) => {
    if (!current || controller.signal.aborted) return
    if (data.work_item?.id !== scope.itemId || data.work_item.corp_id !== scope.corpId ||
      data.work_item.version !== scope.version || data.work_item.mission_id !== scope.missionId ||
      data.mission_id !== scope.missionId) {
      throw new Error('Recovery context does not match the current item/version. Wait for refreshed state or retry the lookup.')
    }
    if (typeof data.task_id !== 'string' || !data.task_id ||
      typeof data.source_run_id !== 'string' || !data.source_run_id ||
      !Array.isArray(data.recoveries) ||
       !(data.workspace_fingerprint === null || typeof data.workspace_fingerprint === 'string') ||
       !(data.expected_head_commit === null || typeof data.expected_head_commit === 'string') ||
       !(data.checkpoint_verification === undefined || typeof data.checkpoint_verification === 'boolean') ||
       !(data.checkpoint_verification_available === undefined || typeof data.checkpoint_verification_available === 'boolean') ||
       !(data.checkpoint_source_correction === undefined || typeof data.checkpoint_source_correction === 'boolean') ||
       !(data.checkpoint_cancellation_event_id == null || typeof data.checkpoint_cancellation_event_id === 'string') ||
      ![data.remaining_attempts, data.remaining_mission_tokens, data.remaining_mission_cost_microusd].every(Number.isFinite)) {
      throw new Error('Recovery context is missing required source, checkpoint or remaining-budget fields. No snapshot fallback is used.')
    }
    publish({ scopeKey, status: 'ready', data, error: null })
  }).catch((error: unknown) => {
    if (!current || controller.signal.aborted) return
    publish({
      scopeKey, status: 'error', data: null,
      error: error instanceof Error ? error.message : 'Recovery context could not be loaded.',
    })
  }).finally(() => clearTimeout(timeout))
  return () => {
    current = false
    clearTimeout(timeout)
    controller.abort()
  }
}

// Presentation only: selection, attempts and budgets belong to the exact endpoint.
// Snapshot records add warnings/status labels, never eligibility or fallback sources.
function factoryRecoveryPresentation(
  context: FactoryVerificationRecoveryContextResponse | null,
  runs: Run[],
) {
  if (!context) return null
  const run = runs.find((candidate) =>
    candidate.id === context.source_run_id && candidate.task_id === context.task_id,
  )
  const quarantined = runs.some((candidate) =>
    candidate.workspace_disposition === 'quarantined' &&
    (!run || candidate.workspace_run_id === run.workspace_run_id),
  )
  const active = context.recoveries.find((recovery) =>
    recovery.status === 'authorized' || recovery.status === 'running',
  )
  const recoveryModes = factoryRecoveryModes(context)
  const checkpointVerification = recoveryModes.includes('checkpoint-verification')
  const checkpointCorrection = context.checkpoint_verification === true && recoveryModes.includes('source-correction')
  const checkpointUnavailable = context.checkpoint_verification === true && recoveryModes.length === 0
  const cancelledBlocked = context.work_item.state === 'cancelled' && !checkpointVerification
  const state = quarantined ? 'quarantined' : active ? 'active'
    : cancelledBlocked || checkpointUnavailable ? 'unavailable'
      : checkpointVerification && context.checkpoint_cancellation_event_id ? 'reconciliation_required'
    : context.workspace_fingerprint ? 'ready' : 'checkpoint_required'
  return {
    run,
    state,
    heading: quarantined ? 'Quarantine warning — inspect controller context'
      : active ? 'Recovery already authorized'
        : cancelledBlocked ? 'Cancelled Factory intent remains protected'
          : checkpointUnavailable ? 'Recovery unavailable'
          : checkpointCorrection ? checkpointVerification ? 'Verify or fix saved work' : 'Fix saved work'
            : checkpointVerification ? 'Verify retained checkpoint'
        : context.workspace_fingerprint ? 'Recover preserved work' : 'Checkpoint and recheck',
    detail: quarantined
      ? run
        ? 'A visible record in the selected source lineage is quarantined. Its checkpoint hash is withheld. Inspect the native controller before executing a copied request; this lookup does not authorize recovery.'
        : 'A visible mission workspace is quarantined, but this endpoint does not include source-workspace lineage details. The checkpoint hash is withheld until the native controller can resolve that warning.'
      : active
        ? 'An existing recovery is authorized or running. Inspect that operation through the controller; do not create a duplicate grant. Copied templates are not a new authorization.'
        : cancelledBlocked
          ? 'The current server context does not authorize checkpoint reconciliation for this cancellation. User stops and unrelated cancellations cannot be overridden here.'
          : checkpointUnavailable
            ? 'No recovery mode is currently available for this checkpoint. Refresh recovery context to check again.'
          : checkpointCorrection
            ? checkpointVerification
              ? 'Recheck saved work without starting an agent, or request a focused fix in the same saved session. A fix uses the remaining budget and attempts; required checks stay in place.'
              : 'Request a focused fix in the same saved session. A fix uses the remaining budget and attempts; required checks stay in place. Checkpoint verification is not available for the current contract.'
          : checkpointVerification
            ? context.checkpoint_cancellation_event_id
              ? 'The server validated this retained checkpoint. Explicit checkpoint verification first reconciles the controller cancellation, then rechecks the same source without a provider. Original spend and history remain unchanged.'
              : 'Recheck this server-validated retained checkpoint without starting a provider. Original spend, source and verification requirements remain unchanged.'
        : context.workspace_fingerprint
          ? 'Recheck saved work without a model call, or request a focused correction in the same session. The controller rechecks authorization before running.'
          : 'The controller can ask the owning runner to seal this older workspace before rechecking it. The original work stays in place; copying a command executes nothing.',
    checkpoint: quarantined || active || cancelledBlocked || checkpointUnavailable ? null : context.workspace_fingerprint,
    recoveryCount: context.recoveries.length,
  }
}

function MissionCard({
  corpId,
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
  factoryItem,
  origin,
  factoryRecoveries,
  events,
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
  onDiscuss,
  onViewAgents,
  onOpenFactory,
}: {
  corpId: string
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
  factoryItem: FactoryWorkItem | undefined
  origin: ReturnType<typeof missionOrigin>
  factoryRecoveries: FactoryVerificationRecovery[]
  events: DomainEvent[]
  actorId: string
  actorRole: string
  busy: boolean
  onLaunch: (mission: Mission) => Promise<void>
  onResume: (run: Run) => Promise<string | null>
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
  onDiscuss: (mission: Mission) => void
  onViewAgents: (mission: Mission) => void
  onOpenFactory: () => void
}) {
  const [copiedRecoveryCommand, setCopiedRecoveryCommand] = useState<string | null>(null)
  const evidenceStorageKey = evidenceSelectionKey({ server: API_URL, corpId, actorId, missionId: mission.id })
  const [selectedEvidenceRunId, setSelectedEvidenceRunId] = useState<string | null>(
    () => readEvidenceSelection(() => window.sessionStorage, evidenceStorageKey),
  )
  const rememberEvidenceRun = (runId: string) => {
    rememberEvidenceSelection(() => window.sessionStorage, evidenceStorageKey, runId)
    setSelectedEvidenceRunId(runId)
  }
  const [recoveryContextLoad, setRecoveryContextLoad] = useState<FactoryRecoveryContextLoad | null>(null)
  const [recoveryReload, setRecoveryReload] = useState(0)
  const recoveryItemId = factoryItem?.id
  const recoveryItemVersion = factoryItem?.version
  const recoveryItemState = factoryItem?.state
  const recoveryScope = useMemo<FactoryRecoveryContextScope | null>(() =>
    recoveryItemId && recoveryItemVersion !== undefined && needsFactoryRecoveryContext(recoveryItemState)
      ? { corpId, actorId, missionId: mission.id, itemId: recoveryItemId, version: recoveryItemVersion, reload: recoveryReload }
      : null,
  [corpId, actorId, mission.id, recoveryItemId, recoveryItemVersion, recoveryItemState, recoveryReload])
  useEffect(() => {
    if (!recoveryScope) return
    return requestFactoryRecoveryContext(recoveryScope, setRecoveryContextLoad)
  }, [recoveryScope])
  const scopedRecoveryLoad = currentFactoryRecoveryLoad(recoveryScope, recoveryContextLoad)
  const recoveryContext = scopedRecoveryLoad?.status === 'ready' ? scopedRecoveryLoad.data : null
  const orderedTasks = tasks.toSorted((left, right) =>
    left.depth - right.depth || left.plan_key.localeCompare(right.plan_key),
  )
  const taskById = new Map(tasks.map((task) => [task.id, task]))
  const evidenceRun = selectMissionEvidenceRun(runs, verificationRequests, selectedEvidenceRunId)
  const displayedEvidenceRunId = evidenceRun?.id
  useEffect(() => {
    if (selectedEvidenceRunId !== null || !displayedEvidenceRunId) return
    // Pin the initial viewed run too: another reviewer completing it, a newer
    // worker, navigation or reload must not silently move this review context.
    const remembered = rememberEvidenceSelection(
      () => window.sessionStorage, evidenceStorageKey, displayedEvidenceRunId,
    )
    // One guarded synchronization when the first run arrives, never per streamed event.
    // oxlint-disable-next-line react/set-state-in-effect
    setSelectedEvidenceRunId(remembered ? displayedEvidenceRunId : '')
  }, [displayedEvidenceRunId, evidenceStorageKey, selectedEvidenceRunId])
  const completedTasks = tasks.filter((task) => task.status === 'completed').length
  const activeRuns = runs.filter((run) => isProviderLiveRun(run, verificationRequests)).length
  const hasUnfinishedRuns = runs.some((run) => !terminalRun(run.status))
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
      run.task_id === evidenceRun?.task_id &&
      run.provider_session_id &&
      run.workspace_disposition === 'preserved' &&
      terminalRun(run.status),
  )
  const resumeStopBlocked = resumableRun?.breaker_stage === 'stop'
  const pendingReviewRuns = runs.filter((run) => pendingReviewForRun(run, verificationRequests))
  const pendingRequest = pendingReviewForRun(evidenceRun, verificationRequests)
  const pendingRun = selectedEvidenceRunId !== null && pendingRequest ? evidenceRun : undefined
  const runIds = new Set(runs.map((run) => run.id))
  const pendingActionApprovals = actionApprovals.filter(
    (approval) => runIds.has(approval.run_id) && approval.status === 'pending',
  )
  const automated = automatedVerificationPresentation(evidenceRun, evidence, events, factoryRecoveries)
  const runEvidence = automated.records
  const runVerificationRequest = evidenceRun
    ? verificationRequests.find(
        (request) => request.run_id === evidenceRun.id && request.task_id === evidenceRun.task_id,
      )
    : undefined
  const reviewRejected = runVerificationRequest?.status === 'rejected'
  const reviewDecisionLabel = runVerificationRequest?.gate_type === 'independent_review'
    ? 'Independent review'
    : 'Human approval'
  const reviewer = actors.find((actor) => actor.id === runVerificationRequest?.decided_by)
  const reviewDecisionNote = runVerificationRequest?.decision_note?.trim() ||
    'No reason was recorded for this decision.'
  const compactReviewNote = reviewDecisionNote.replace(/\s+/g, ' ')
  const reviewDecisionSummary = compactReviewNote.length > 240
    ? `${compactReviewNote.slice(0, 237).trimEnd()}…`
    : compactReviewNote
  const runDeliverable = evidenceRun
    ? deliverables.find((deliverable) => deliverable.run_id === evidenceRun.id)
    : undefined
  const terminalSummary =
    evidenceRun && terminalRun(evidenceRun.status)
      ? evidenceRun.summary ?? evidenceRun.verification_summary
      : null
  const recovery = factoryRecoveryPresentation(recoveryContext, runs)
  // The endpoint selects the task. This exact-ID lookup supplies command metadata
  // only; absence is a field gap, never permission to pick a different task/source.
  const recoveryTask = recoveryContext
    ? tasks.find((task) => task.id === recoveryContext.task_id && task.mission_id === recoveryContext.mission_id)
    : undefined
  const recoveryItem = recoveryContext?.work_item
  const recoveryModes = factoryRecoveryModes(recoveryContext)
  // Looking for optional checkpoint recovery is not itself a reason to suppress
  // ordinary interrupted-session resume. Snapshot history may require a governed
  // path, but never supplies that path's source, checkpoint or authorization.
  const resumeLineageRuns = runs.filter((run) => run.task_id === resumableRun?.task_id
    && run.workspace_run_id === resumableRun?.workspace_run_id)
  const requiresFactoryRecovery = Boolean(recoveryScope && (
    ['verification_failed', 'cancelled'].includes(recoveryItemState ?? '') ||
    tasks.some((task) => task.id === resumableRun?.task_id &&
      (task.status === 'verification_failed' || task.verification_status === 'failed')) ||
    resumeLineageRuns.some((run) => run.execution_mode === 'verification_only' ||
      run.breaker_stage === 'suspend' || run.breaker_stage === 'stop') ||
    factoryRecoveries.some((entry) =>
      entry.factory_work_item_id === recoveryScope.itemId && entry.mission_id === mission.id &&
      entry.task_id === resumableRun?.task_id &&
      (['authorized', 'running'].includes(entry.status) || (entry.mode === 'checkpoint_verification' &&
        resumeLineageRuns.some((run) => run.id === entry.source_run_id || run.id === entry.replacement_run_id)))) ||
    recoveryContext?.recoveries.some((entry) => entry.task_id === resumableRun?.task_id &&
      ['authorized', 'running'].includes(entry.status))
  ))
  const resumeRecoveryBlocked = requiresFactoryRecovery ||
    factoryRecoveryBlocksProviderResume(Boolean(recoveryScope && recoveryContext &&
      recoveryContext.task_id === resumableRun?.task_id), recoveryContext)
  const canAuthorizeRecovery = ['owner', 'admin', 'manager'].includes(actorRole)
  const recoveryAgent = recoveryTask?.assigned_agent_id
    ? agents.find((agent) => agent.id === recoveryTask.assigned_agent_id)
    : undefined
  const recoveryAdapter = recoveryTask?.required_adapter ?? recoveryAgent?.adapter ?? ''
  const recoverySourceBase = typeof recoveryItem?.policy.source_base_ref === 'string'
    ? recoveryItem.policy.source_base_ref : recoveryTask?.contract.source_base_ref
  const recoveryConnectionId = recoveryItem ? factoryRecoveryConnection(recoveryItem.policy) : undefined
  const recoveryCommandAvailable = Boolean(recoveryContext && recoveryTask && recoveryAdapter && recoverySourceBase
    && recoveryConnectionId !== undefined && recoveryModes.length
    && recovery?.state !== 'quarantined')
  const recoveryCommand = (mode: FactoryRecoveryCommandMode) => {
    if (!recoveryItem || !recoveryTask || !recoveryAdapter || !recoverySourceBase
      || recoveryConnectionId === undefined || !recoveryModes.includes(mode)
      || recovery?.state === 'quarantined') return ''
    const quote = (value: string) => `'${value.replaceAll("'", "''")}'`
    const command = [
      'crony',
      '--server',
      quote(API_URL),
      'factory',
      quote(corpId),
      quote(actorId),
      '--owner',
      quote(recoveryItem.source_project_owner),
      '--project-number',
      String(recoveryItem.source_project_number),
      '--repository',
      quote(`${recoveryItem.source_repository_owner}/${recoveryItem.source_repository_name}`),
      '--source-base-ref',
      quote(recoverySourceBase),
      '--adapter',
      quote(recoveryAdapter),
      '--budget-tokens',
      String(mission.budget_tokens),
      '--budget-cost-microusd',
      String(mission.budget_cost_microusd),
      '--issue',
      String(recoveryItem.source_issue_number),
      '--verification-recovery',
      mode,
      '--verification-recovery-reason',
      quote('Explain why this bounded recovery is authorized.'),
    ]
    if (recoveryConnectionId) {
      command.push('--workspace-connection-id', quote(recoveryConnectionId))
    }
    if (recoveryTask.contract.model) {
      command.push('--model', quote(recoveryTask.contract.model))
    }
    if (recoveryTask.contract.reasoning_effort) {
      command.push('--reasoning-effort', quote(recoveryTask.contract.reasoning_effort))
    }
    return command.join(' ')
  }
  const recoveryCopyKey = (mode: FactoryRecoveryCommandMode) =>
    `${scopedRecoveryLoad?.scopeKey}:${recoveryContext?.source_run_id}:${recoveryCommand(mode)}`
  const copyRecoveryCommand = async (
    mode: FactoryRecoveryCommandMode,
  ) => {
    const command = recoveryCommand(mode)
    if (!command || !canAuthorizeRecovery) return
    try {
      await navigator.clipboard.writeText(command)
      setCopiedRecoveryCommand(recoveryCopyKey(mode))
    } catch {
      setCopiedRecoveryCommand(null)
    }
  }
  const decideEvidence = (approved: boolean) => {
    if (!pendingRun || !pendingRequest) return
    // Keep the decided run visible across snapshot refresh, reload and navigation.
    // Another review needs an explicit selector/next-review action.
    rememberEvidenceRun(pendingRun.id)
    void onVerificationDecision(pendingRun, approved)
  }
  const resumeEvidence = async () => {
    if (!resumableRun || resumeRecoveryBlocked) return
    const resumedRunId = await onResume(resumableRun)
    if (resumedRunId) rememberEvidenceRun(resumedRunId)
  }
  return (
    <article
      className="mission-card"
      data-testid={`mission-${mission.id}`}
      data-mission-id={mission.id}
      data-run-id={evidenceRun?.id}
      tabIndex={-1}
    >
      <div className="mission-card-top">
        <span className={`status-chip status-chip-${mission.status}`}>{missionStatusLabel(mission.status)}</span>
        <span className="mission-id">#{shortId(mission.id)}</span>
      </div>
      <h3>{mission.title}</h3>
      <div className="mission-work-context">
        <MissionOriginDetails
          corpId={corpId}
          actorId={actorId}
          actorRole={actorRole}
          missionId={mission.id}
          roomId={mission.room_id}
          api={api}
          fallback={origin}
        />
        <nav className="work-context-actions" aria-label="Mission workspace">
          <button type="button" className="button button-secondary" onClick={() => onViewAgents(mission)}>
            {activeRuns ? 'View live agents' : 'View task owners'}
          </button>
          <button type="button" className="button button-secondary" onClick={() => onDiscuss(mission)}>
            Discuss this mission
          </button>
          {factoryItem ? <button type="button" className="button button-secondary" onClick={onOpenFactory}>
            View GitHub intake
          </button> : null}
        </nav>
      </div>
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
        <div className="strategy-chip">{mission.strategy === STUDIO_STRATEGY ? STUDIO_STRATEGY_LABEL : statusLabel(mission.strategy)}</div>
        <div className="contract-version-chip">Specification v{mission.specification_version}</div>
      </div>
      <dl>
        <div>
          <dt>Tasks</dt>
          <dd>{completedTasks}/{tasks.length} complete</dd>
        </div>
        <div>
          <dt>Runs</dt>
          <dd>{activeRuns ? `${activeRuns} live` : `${runs.length} run${runs.length === 1 ? '' : 's'}`}</dd>
        </div>
      </dl>
      <details
        className="mission-dossier"
        open={mission.status !== 'completed' && (resumeBudgetBlocked || Boolean(pendingBudgetRevision))}
      >
        <summary>
          <span>{mission.status === 'completed' ? 'Recorded spend' : 'Budget authority'}</span>
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
                <span>{workflowTaskLabel(task.plan_key)}</span>
                <strong>{statusLabel(task.status)}</strong>
                <small>d{task.depth} · {task.attempt_count}/{task.max_attempts}</small>
              </summary>
              <div className="task-contract">
                <div className="task-contract-heading">
                  <p>{task.objective}</p>
                  <span>Contract v{task.contract_version}</span>
                </div>
                <dl>
                  <div><dt>Agent</dt><dd>{assignedAgent?.name ?? 'Unassigned'}{assignedAgent?.retired_at ? ' · retired' : ''} · {adapterLabel(task.required_adapter ?? assignedAgent?.adapter ?? 'unknown')}</dd></div>
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
                  recoveryScope={recoveryScope}
                  recoveryLoad={scopedRecoveryLoad}
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
      {runs.length ? (
        <section
          className="mission-evidence-context"
          aria-label="Run evidence"
          data-evidence-run-id={evidenceRun?.id}
          data-evidence-task-id={evidenceRun?.task_id}
        >
          <label htmlFor={`mission-evidence-${mission.id}`}>Evidence for</label>
          <select
            id={`mission-evidence-${mission.id}`}
            value={evidenceRun?.id ?? ''}
            disabled={busy}
            onChange={(event) => rememberEvidenceRun(event.target.value)}
          >
            {!evidenceRun ? <option value="" disabled>Choose a run to inspect</option> : null}
            {runs.map((run) => (
              <option key={run.id} value={run.id}>
                {taskById.get(run.task_id)?.title ?? 'Task'} · {shortId(run.id)} · {statusLabel(run.status)}
                {pendingReviewForRun(run, verificationRequests) ? ' · Review needed' : ''}
              </option>
            ))}
          </select>
          {evidenceRun ? (
            <>
              <p>
                Produced by {agents.find((agent) => agent.id === evidenceRun.agent_id)?.name ?? shortId(evidenceRun.agent_id)}
                {' · '}run {shortId(evidenceRun.id)}
              </p>
              <p>Downloads, checks and evidence decisions below apply only to this run.</p>
            </>
          ) : (
            <p role="status">Choose a run to view its evidence. Reviews never transfer between runs.</p>
          )}
          {pendingReviewRuns.length ? (
            <div className="work-context-actions">
              <strong>{pendingReviewRuns.length} evidence review{pendingReviewRuns.length === 1 ? '' : 's'} pending</strong>
              {!pendingRequest || pendingReviewRuns.length > 1 ? (
                <button
                  className="button button-secondary"
                  type="button"
                  disabled={busy}
                  onClick={() => {
                    const current = pendingReviewRuns.findIndex((run) => run.id === evidenceRun?.id)
                    rememberEvidenceRun(pendingReviewRuns[(current + 1) % pendingReviewRuns.length].id)
                  }}
                >
                  Review next pending run
                </button>
              ) : null}
            </div>
          ) : null}
        </section>
      ) : null}
      {evidenceRun?.artifact_sha256 && evidenceRun.artifact_uri ? (
        <div
          className="evidence-box evidence-provider"
          data-testid="provider-evidence"
          data-artifact-id={evidenceRun.artifact_id}
        >
          <strong>Provider evidence</strong>
          <span>{shortId(evidenceRun.artifact_sha256)}…</span>
          <button
            type="button"
            className="artifact-download"
            onClick={() => void onDownloadArtifact(evidenceRun)}
          >
            {evidenceRun.verification_status === 'passed'
              ? 'Download verified artifact'
              : 'Download submitted artifact'}
          </button>
        </div>
      ) : null}
      {runDeliverable ? (
        <div className="evidence-box evidence-source" data-testid="source-deliverable">
          <strong>Source deliverable · {statusLabel(runDeliverable.form)}</strong>
          <span>{runDeliverable.file_name} · {runDeliverable.bytes.toLocaleString()} bytes</span>
          <small>
            Verification {shortId(runDeliverable.verification_sha256)}… · bytes {shortId(runDeliverable.sha256)}…
          </small>
          <button
            type="button"
            className="artifact-download"
            onClick={() => void onDownloadDeliverable(runDeliverable)}
          >
            Download source deliverable
          </button>
        </div>
      ) : null}
      {evidenceRun &&
      (evidenceRun.verification_status !== 'pending' || runEvidence.length > 0) ? (
        <details
          key={`verification-${evidenceRun.id}`}
          className={`verification-box operations-verification verification-${automated.status}`}
          data-testid="verification-evidence"
          data-check-total={automated.total ?? 'unknown'}
        >
          <summary>
            <span className="operations-verification-copy">
              <strong>Automated verification</strong>
              <small>
                {automated.summary}
                {automated.missing !== null && automated.missing > 0
                  ? ` · ${automated.missing} result${automated.missing === 1 ? '' : 's'} not recorded`
                  : ''}
              </small>
            </span>
            <span className="operations-verification-score">
              {automated.score}
            </span>
            <span className="operations-disclosure-mark" aria-hidden="true">+</span>
          </summary>
          {runEvidence.length ? (
            <ol className="evidence-checks">
              {runEvidence.map((item) => (
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
      {reviewRejected ? (
        <section
          className="operations-review-decision"
          role="alert"
          aria-labelledby={`review-decision-${mission.id}`}
          data-testid="review-decision"
        >
          <span className="operations-review-label">{reviewDecisionLabel}</span>
          <strong id={`review-decision-${mission.id}`}>Changes requested</strong>
          <small>
            {reviewer
              ? `Reviewed by ${reviewer.name}`
              : runVerificationRequest?.decided_by
                ? `Reviewer ${shortId(runVerificationRequest.decided_by)} (name unavailable)`
                : 'Reviewer not recorded'}
          </small>
          <p>{reviewDecisionSummary}</p>
          {reviewDecisionSummary !== reviewDecisionNote ? (
            <details className="operations-review-details" key={runVerificationRequest?.run_id}>
              <summary>Read full reviewer findings</summary>
              <div
                className="operations-review-full"
                role="region"
                aria-label="Full reviewer findings"
                tabIndex={0}
              >
                <p>{reviewDecisionNote}</p>
              </div>
            </details>
          ) : null}
          <p className="operations-review-next">
            <strong>Next step: </strong>
            {recoveryScope
              ? 'Inspect the governed recovery controls below with an authorized operator. The controller selects the source and revalidates the request; a new outcome review is still required.'
                : 'Ask an authorized operator to resolve the findings and submit new evidence through the governed verification flow.'}
          </p>
        </section>
      ) : null}
      {factoryItem && recoveryScope && (
        ['verification_failed', 'cancelled'].includes(factoryItem.state)
        || recoveryContext?.checkpoint_verification
        || (resumableRun && !hasUnfinishedRuns)
      ) ? (
        <section
          className="factory-recovery-callout"
          aria-labelledby={`factory-recovery-${factoryItem.id}`}
          data-testid="factory-verification-recovery"
          data-recovery-state={recovery?.state ?? scopedRecoveryLoad?.status ?? 'loading'}
          data-recovery-context-status={scopedRecoveryLoad?.status ?? 'loading'}
          data-recovery-run-id={recoveryContext?.source_run_id}
          data-recovery-task-id={recoveryContext?.task_id}
          data-recovery-item-version={recoveryContext?.work_item.version}
          data-checkpoint-reconciliation-needed={recoveryContext?.checkpoint_cancellation_event_id ? 'true' : 'false'}
          aria-busy={!scopedRecoveryLoad || scopedRecoveryLoad.status === 'loading'}
        >
          <div className="factory-recovery-heading">
            <div>
              <span>Governed recovery</span>
              <strong id={`factory-recovery-${factoryItem.id}`}>
                {recovery?.heading ?? (scopedRecoveryLoad?.status === 'error'
                  ? 'Recovery context unavailable' : 'Loading recovery context…')}
              </strong>
            </div>
            <span className="status-chip status-chip-failed">
              {recovery?.state === 'quarantined' ? 'Quarantine warning'
                : recovery?.run ? `Source ${statusLabel(recovery.run.status)}` : 'Controller context'}
            </span>
          </div>
          <p role={scopedRecoveryLoad?.status === 'error' ? 'alert' : 'status'}>
            {recovery?.detail ?? scopedRecoveryLoad?.error ??
              'Reading the exact work-item recovery context. No cached source or checkpoint is displayed.'}
          </p>
          {!recovery && runs.some((run) => run.workspace_disposition === 'quarantined') ? (
            <p className="factory-recovery-role-note">
              A visible mission workspace is quarantined. Preserve it; only the controller can resolve
              the selected source context. No snapshot hash is substituted.
            </p>
          ) : null}
          <button className="button button-secondary" type="button"
            disabled={!scopedRecoveryLoad || scopedRecoveryLoad.status === 'loading'}
            onClick={() => setRecoveryReload((value) => value + 1)}>
            Refresh recovery context
          </button>
          {recoveryContext && recovery ? <>
          <details className="factory-recovery-details">
          <summary>Recovery details</summary>
          <dl>
            <div>
              <dt>Attempts remaining</dt>
              <dd>{recoveryContext.remaining_attempts}</dd>
            </div>
            <div>
              <dt>Server-selected source</dt>
              <dd title={recoveryContext.source_run_id}>{shortId(recoveryContext.source_run_id)}</dd>
            </div>
            <div>
              <dt>Remaining mission tokens</dt>
              <dd>{recoveryContext.remaining_mission_tokens.toLocaleString()}</dd>
            </div>
            <div>
              <dt>Remaining mission budget</dt>
              <dd>{formatUsd(recoveryContext.remaining_mission_cost_microusd)}</dd>
            </div>
            <div>
              <dt>Recorded checkpoint</dt>
              <dd>{recovery.checkpoint ? `${shortId(recovery.checkpoint)}…`
                : recovery.state === 'checkpoint_required' ? 'Owning runner checkpoints through the controller' : 'Withheld; inspect controller context'}</dd>
            </div>
            <div>
              <dt>Visible recovery records</dt>
              <dd>{recovery.recoveryCount}</dd>
            </div>
          </dl>
          </details>
          <p className="factory-recovery-role-note">
            Copying is not granting and executes nothing.
          </p>
          {recoveryCommandAvailable && canAuthorizeRecovery ? (
            <div className="factory-recovery-actions">
              {recoveryModes.map((mode) => (
              <button
                key={mode}
                className="button button-secondary"
                type="button"
                onClick={() => void copyRecoveryCommand(mode)}
              >
                {copiedRecoveryCommand === recoveryCopyKey(mode)
                  ? mode === 'checkpoint-verification' ? 'Checkpoint command copied'
                    : mode === 'verifier-only' ? 'Verifier command copied' : 'Correction command copied'
                  : mode === 'checkpoint-verification' ? 'Copy checkpoint-verification command'
                    : mode === 'verifier-only' ? 'Copy verifier-only command' : 'Copy source-correction command'}
              </button>
              ))}
            </div>
          ) : (
            <p className="factory-recovery-role-note">
              {canAuthorizeRecovery
                ? recoveryModes.length === 0
                  ? 'No new recovery command is available in this current context. Inspect or refresh the native controller context; no replacement work is inferred.'
                  : 'Exact source/connection command metadata is missing or quarantined; inspect the native controller. No substitute is inferred.'
                : 'An owner, admin, or manager must authorize the recovery.'}
            </p>
          )}
          {recoveryCommandAvailable && canAuthorizeRecovery ? <details>
            <summary>Show trusted controller command</summary>
            <code>{recoveryCommand(recoveryModes[0] ?? 'verifier-only')}</code>
          </details> : null}
          </> : null}
        </section>
      ) : null}
      {evidenceRun && terminalRun(evidenceRun.status) ? (
        reviewRejected ? (
          <details className="operations-run-outcome" key={`run-outcome-${evidenceRun.id}`}>
            <summary>Run record · {statusLabel(evidenceRun.status)}</summary>
            <p>{terminalSummary ?? 'The run ended without a summary.'}</p>
            <small>
              Worktree: {evidenceRun.workspace_disposition
                ? statusLabel(evidenceRun.workspace_disposition)
                : 'cleanup pending'}
              {evidenceRun.workspace_detail ? ` · ${evidenceRun.workspace_detail}` : ''}
            </small>
          </details>
        ) : (
          <div className={`terminal-summary terminal-${evidenceRun.status}`}>
            <strong>{statusLabel(evidenceRun.status)}</strong>
            <p>{terminalSummary ?? 'The run ended without a summary.'}</p>
            <small>
              Worktree: {evidenceRun.workspace_disposition
                ? statusLabel(evidenceRun.workspace_disposition)
                : 'cleanup pending'}
              {evidenceRun.workspace_detail ? ` · ${evidenceRun.workspace_detail}` : ''}
            </small>
          </div>
        )
      ) : null}
      {evidenceRun && (
        evidenceRun.input_tokens > 0 || evidenceRun.output_tokens > 0 ||
        evidenceRun.model || evidenceRun.workspace_branch
      ) ? (
        <dl className="operations-run-metadata" aria-label="Run details">
          {evidenceRun.input_tokens > 0 || evidenceRun.output_tokens > 0 ? (
            <div>
              <dt>Usage</dt>
              <dd>{evidenceRun.input_tokens.toLocaleString()} in · {evidenceRun.output_tokens.toLocaleString()} out</dd>
            </div>
          ) : null}
          {evidenceRun.model ? (
            <div>
              <dt>Model</dt>
              <dd>
                {evidenceRun.model}
                {evidenceRun.reasoning_effort ? ` · ${evidenceRun.reasoning_effort} reasoning` : ''}
              </dd>
            </div>
          ) : null}
          {evidenceRun.workspace_branch ? (
            <div title={evidenceRun.workspace_detail ?? undefined}>
              <dt>{statusLabel(evidenceRun.workspace_disposition ?? 'active')} worktree</dt>
              <dd>{evidenceRun.workspace_branch}</dd>
            </div>
          ) : null}
        </dl>
      ) : null}
      {runDeliverable ? (
        <div className="workspace-box integration-box" data-testid="integration-state">
          <span>Integration · {statusLabel(runDeliverable.integration_state)}</span>
          <strong>
            {runDeliverable.head_commit
              ? `${runDeliverable.branch} @ ${shortId(runDeliverable.head_commit)}`
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
                <p className="operations-approval-note">
                  Allowed tools make an action requestable, not pre-approved. This decision covers
                  only the exact action and scope shown; it grants no blanket access. A current
                  authorization for that same action and scope does not need a duplicate grant.
                </p>
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
      {resumableRun && resumableRun.id === evidenceRun?.id && !hasUnfinishedRuns && !resumeRecoveryBlocked && factoryItem?.state !== 'verification_failed' ? (
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
            onClick={() => void resumeEvidence()}
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
        <div className="verification-actions" data-review-run-id={pendingRun.id} data-review-task-id={pendingRun.task_id}>
          {(() => {
            const requiredRoles = pendingRequest.gate.roles
            const blockedReason = reviewBlockedReason(
              pendingRequest.gate,
              { id: actorId, role: actorRole, name: actors.find((actor) => actor.id === actorId)?.name },
              mission.requested_by,
            )
            const canDecide = !blockedReason
            const noticeId = `mission-review-eligibility-${pendingRun.id}`
            return (
              <>
                <span>
                  {statusLabel(pendingRequest.gate_type)} · eligible: {requiredRoles.join(', ')}
                </span>
                <span>{taskById.get(pendingRun.task_id)?.title ?? 'Task'} · run {shortId(pendingRun.id)}</span>
                {blockedReason ? <ReviewEligibilityNotice reason={blockedReason} id={noticeId} /> : null}
          <button
            className="button button-primary"
            type="button"
                  disabled={busy || !canDecide}
            onClick={() => decideEvidence(true)}
            aria-describedby={blockedReason ? noticeId : undefined}
          >
            {busy ? 'Submitting decision…' : 'Accept evidence'}
          </button>
          <button
            className="button button-danger"
            type="button"
                  disabled={busy || !canDecide}
            onClick={() => decideEvidence(false)}
            aria-describedby={blockedReason ? noticeId : undefined}
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
  scope,
  messages,
  actors,
  selectedActor,
  missions,
  tasks,
  runs,
  onPost,
  onNavigateLink,
  onContextChange,
}: {
  room: { id: string; name: string; purpose: string } | undefined
  scope: DiscussionScope
  messages: RoomMessage[]
  actors: Actor[]
  selectedActor: Actor
  missions: Mission[]
  tasks: Task[]
  runs: Run[]
  onPost: (input: RoomPostInput) => Promise<boolean>
  onNavigateLink: (link: EntityLink) => void
  onContextChange: (missionId: string | null) => void
}) {
  const [body, setBody] = useState('')
  const [replyToId, setReplyToId] = useState<string | null>(null)
  const [linkValue, setLinkValue] = useState('')
  const [posting, setPosting] = useState(false)
  const contextMissionId = scope.missionId
  const context = roomWorkContext({ missions, tasks, runs }, room?.id, contextMissionId)
  const contextMission = missions.find((mission) => mission.id === contextMissionId)
  const contextSelector = (
    <div className="room-context-bar">
      <label htmlFor="room-context">Discussion for</label>
      <select id="room-context" value={contextMissionId ?? ''} disabled={posting}
        onChange={(event) => onContextChange(event.target.value || null)}>
        <option value="">All room discussion</option>
        {contextMissionId && !contextMission ? (
          <option value={contextMissionId} disabled>Unavailable mission</option>
        ) : null}
        {missions.map((mission) => (
          <option key={mission.id} value={mission.id}>{mission.title}</option>
        ))}
      </select>
      {room && contextMission ? (
        <button type="button" className="button button-secondary"
          onClick={() => onNavigateLink({ kind: 'mission', id: contextMission.id })}>
          Open mission and results
        </button>
      ) : null}
    </div>
  )

  if (!room) {
    return (
      <section className="room-panel panel" id="room" tabIndex={-1}>
        <div className="panel-heading">
          <div>
            <span className="section-code">Project room</span>
            <h2>Discussion unavailable</h2>
            <p>The selected mission or room is not available in {selectedActor.name}&apos;s current view.</p>
          </div>
        </div>
        {contextSelector}
        <div className="room-denied">Choose an available mission. No comment can be posted to an unverified room.</div>
      </section>
    )
  }

  const linkedOptions = relatedWorkOptions(context)
  const scopedMessages = roomDiscussionMessages(messages, room.id, contextMissionId, context)
  const visibleMessages = scopedMessages.slice(-40)
  const replyTarget = replyToId
    ? scopedMessages.find((message) => message.id === replyToId)
    : undefined
  const inheritedLink = replyTarget?.link ??
    scopedMessages.find((message) => message.id === replyTarget?.thread_root_id)?.link
  const replyLink = inheritedLink && missionIdForLink(inheritedLink, context) ? inheritedLink : null
  const effectiveLinkValue = linkValue || (replyLink
    ? `${replyLink.kind}:${replyLink.id}`
    : contextMission ? `mission:${contextMission.id}` : '')
  const composerError = replyToId && !replyTarget
    ? 'The reply target is no longer in this discussion.'
    : effectiveLinkValue && !linkedOptions.some((option) => option.value === effectiveLinkValue)
      ? 'The related work is no longer in this discussion.'
      : null

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    if (!body.trim() || posting || composerError) return
    const mentionedNames = Array.from(body.matchAll(/@([A-Za-z0-9_-]+)/g), (match) =>
      match[1].toLowerCase(),
    )
    const mentions = actors
      .filter((actor) => mentionedNames.includes(actor.name.toLowerCase()))
      .map((actor) => actor.id)
    const [kind, id] = effectiveLinkValue.split(':')
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
    const operationStorageKey = `ecorp:room-message:${discussionScopeKey(scope)}`
    const idempotencyKey = browserOperationKey(operationStorageKey, payload)
    setPosting(true)
    let saved = false
    try {
      saved = await onPost({
        source: 'room',
        scope,
        roomId: room.id,
        body: body.trim(),
        replyToId,
        mentions,
        link,
        idempotencyKey,
      })
    } finally {
      setPosting(false)
    }
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
          <h2>{contextMission ? contextMission.title : `${room.name} discussion`}</h2>
          <p>{room.name} · Comments are shared discussion—not agent instructions, new tasks, or approvals.</p>
        </div>
        <div className="room-count">{scopedMessages.length} comments</div>
      </div>
      {contextSelector}
      <div className="room-layout">
        <ol className="room-message-list" data-testid="room-message-list">
          {visibleMessages.length ? (
            visibleMessages.map((message) => {
              const author = actors.find((actor) => actor.id === message.actor_id)
              const linked = message.link && missionIdForLink(message.link, context) ? message.link : null
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
                          {workLinkLabel(linked, context)}
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
              <strong>{contextMission ? 'No discussion for this mission yet' : 'The room is quiet'}</strong>
              <span>Leave a comment for your collaborators. Use the mission's agent controls to send instructions.</span>
            </li>
          )}
        </ol>
        <form className="room-composer" onSubmit={submit} aria-busy={posting}>
          <label htmlFor="room-message">Post as {selectedActor.name}</label>
          {composerError ? (
            <div className="room-denied" role="alert">
              {composerError}
              <button type="button" onClick={() => { setReplyToId(null); setLinkValue('') }}>
                Reset reply and related work
              </button>
            </div>
          ) : null}
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
            disabled={posting}
            aria-describedby="room-comment-purpose"
          />
          <p className="room-comment-purpose" id="room-comment-purpose">
            This records a comment only. To direct a worker, open the mission and choose View live agents.
          </p>
          <label htmlFor="room-link">Related work</label>
          <select id="room-link" value={effectiveLinkValue} disabled={posting}
            onChange={(event) => {
              const value = event.target.value
              setLinkValue(value)
              setReplyToId(null)
            }}>
            <option value="">No linked work item</option>
            {linkedOptions.map((option) => (
              <option key={option.value} value={option.value}>{option.label}</option>
            ))}
          </select>
          <button className="button button-primary" type="submit" disabled={posting || !body.trim() || Boolean(composerError)}>
            {posting ? 'Posting comment…' : 'Post comment'}
          </button>
        </form>
      </div>
    </section>
  )
}

function MissionAllocationPreview({
  scope,
}: {
  scope: MissionRequestScope
}) {
  const [load, setLoad] = useState<MissionPreviewLoad | null>(null)
  const [refresh, setRefresh] = useState(0)
  const { key, corpId, actorId, body, strategy } = scope
  useEffect(() => startMissionPreview({ key, corpId, actorId, body, strategy }, api, setLoad),
    [key, corpId, actorId, body, strategy, refresh])
  const current = currentMissionPreview(scope, load)
  const quote = current?.status === 'ready' ? current.quote : null
  return (
    <div
      data-testid="mission-allocation-preview"
      data-preview-status={current?.status ?? 'pending'}
      aria-busy={!current || current.status === 'pending'}
    >
      {quote ? (
        <>
          <p className="operations-approval-note" role="status">
            Server-checked allocation · <strong>{quote.budget_tokens.toLocaleString()} total tokens</strong>
          </p>
          <ul className="evidence-checks" aria-label="Exact task token allocations">
            {quote.tasks.map((task) => (
              <li className="evidence-check" key={task.key} data-task-key={task.key}>
                <strong title={task.title}>
                  {statusLabel(task.key)}<span className="sr-only">: {task.title}</span>
                </strong>
                <span>{task.budget_tokens.toLocaleString()} tokens</span>
              </li>
            ))}
          </ul>
          <details className="mission-allocation-details">
            <summary>Dependencies, retries and cost policy</summary>
            <ul>
              {quote.tasks.map((task) => (
                <li key={task.key}>
                  <strong>{statusLabel(task.key)}:</strong>{' '}
                  {task.depends_on.length ? `after ${task.depends_on.map(statusLabel).join(', ')}` : 'no dependencies'}
                  {' · '}{task.max_attempts} attempt{task.max_attempts === 1 ? '' : 's'} maximum.
                </li>
              ))}
            </ul>
            <p>Reported-cost limit: {formatUsd(quote.budget_cost_microusd)}. This is a policy on reported usage, not a provider billing estimate.</p>
          </details>
        </>
      ) : (
        <p className="operations-approval-note" role={current?.status === 'error' ? 'alert' : 'status'}>
          {current?.error ?? 'Fetching exact allocation for these settings… No current task allocation is available yet.'}
        </p>
      )}
      <p className="operations-approval-note">
        Preview starts no work and grants no approval. Launch revalidates the current request on the server.
      </p>
      {current?.status === 'error' ? (
        <button className="button button-secondary" type="button" onClick={() => {
          setLoad(null)
          setRefresh((value) => value + 1)
        }}>
          Retry preview
        </button>
      ) : null}
    </div>
  )
}

function App() {
  const [bootstrap, setBootstrap] = useState<BootstrapResponse | null>(null)
  const [connectionAttempt, setConnectionAttempt] = useState(0)
  const [snapshotLoad, setSnapshotLoad] = useState<{
    corpId: string; actorId: string; response: SnapshotResponse
  } | null>(null)
  const [selectedActorId, setSelectedActorId] = useState<string | null>(null)
  const data = snapshotLoad && snapshotLoad.corpId === bootstrap?.corp_id &&
    snapshotLoad.actorId === selectedActorId ? snapshotLoad.response : null
  const [missionTitle, setMissionTitle] = useState('')
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
    'brief' | 'proof'
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
  const [connectionsOpen, setConnectionsOpen] = useState(false)
  const [savedConnectionLoad, setSavedConnectionLoad] = useState<{
    scope: string; data: WorkspaceConnections
  } | null>(null)
  const restoredConnectionScope = useRef('')
  const [missionBudgetTokens, setMissionBudgetTokens] = useState(1_000_000)
  const [missionDeliverable, setMissionDeliverable] =
    useState<NonNullable<TaskContract['deliverable']>['form']>('archive')
  const [commitDeliverable, setCommitDeliverable] = useState(false)
  const [pauseAfterPlanning, setPauseAfterPlanning] = useState(false)
  const [developerMode, setDeveloperMode] = useState(false)
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null)
  const [showRegisteredCrew, setShowRegisteredCrew] = useState(false)
  const [floorInspectorOpen, setFloorInspectorOpen] = useState(false)
  const [selectedMissionId, setSelectedMissionId] = useState<string | null>(null)
  const [roomMissionId, setRoomMissionId] = useState<string | null>(null)
  const [selectedRoomId, setSelectedRoomId] = useState<string | null>(null)
  const [selectedFactoryItemId, setSelectedFactoryItemId] = useState<string | null>(null)
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
  const snapshotRefreshRef = useRef<ReturnType<typeof createSnapshotRefresher> | null>(null)
  const currentViewer = useRef<{ corpId: string; actorId: string } | null>(null)
  const currentComments = useRef<{
    snapshot: SnapshotResponse['snapshot']
    room: DiscussionScope
    factory: DiscussionScope
  } | null>(null)
  const composerInitialized = useRef(false)
  const initialWorkspaceHash = useRef(window.location.hash)
  const missionComposerHeading = useRef<HTMLHeadingElement | null>(null)

  useEffect(() => {
    if (missionComposerCollapsed || activeWorkspaceView !== 'missions') return
    // Step changes should not leave keyboard users halfway down the old screen.
    missionComposerHeading.current?.focus()
  }, [missionComposerStep, missionComposerCollapsed, activeWorkspaceView])

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
        setRoomMissionId(null)
        setSelectedRoomId(targetId)
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
      if (!missionId || !data?.snapshot.missions.some((mission) => mission.id === missionId)) {
        setError('The linked mission is unavailable in the current view.')
        return
      }
      if (missionId) {
        setSelectedMissionId(missionId)
        setMissionComposerCollapsed(true)
        setRoomMissionId(missionId)
        setSelectedRoomId(null)
      }
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

  const refresh = useCallback(async (corpId: string, actorId: string, signal?: AbortSignal) => {
    const snapshot = await api<SnapshotResponse>(
      `/api/corps/${corpId}/snapshot?actor_id=${actorId}`,
      { signal },
    )
    if (signal?.aborted) throw new DOMException('Obsolete snapshot scope', 'AbortError')
    if (snapshot.snapshot.corp.id !== corpId) throw new Error('Snapshot Corp does not match the requested Corp.')
    if (currentViewer.current?.corpId !== corpId || currentViewer.current.actorId !== actorId) {
      return snapshot
    }
    const newest = snapshot.snapshot.events.at(-1)?.seq ?? 0
    lastEventSeq.current[actorId] = Math.max(lastEventSeq.current[actorId] ?? 0, newest)
    setSnapshotLoad({ corpId, actorId, response: snapshot })
    return snapshot
  }, [])

  useEffect(() => {
    let cancelled = false
    let timedOut = false
    const controller = new AbortController()
    const timeout = window.setTimeout(() => {
      timedOut = true
      controller.abort()
    }, 30_000)
    void (async () => {
      const health = await fetch(`${API_URL}/health`, { signal: controller.signal }).then((response) =>
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
        currentViewer.current = { corpId, actorId }
        setBootstrap(result)
        setSelectedActorId(actorId)
        await refresh(corpId, actorId, controller.signal)
        return
      }
      const result = await api<BootstrapResponse>('/api/demo/bootstrap?seed_crew=false', {
        method: 'POST',
        body: '{}',
        signal: controller.signal,
      })
      if (cancelled) return
      setBootstrap(result)
      const actorName = new URLSearchParams(window.location.search).get('actor')
      const initialActor = actorName?.toLowerCase() === 'bob'
        ? result.bob_actor_id
        : actorName?.toLowerCase() === 'eve'
          ? result.eve_actor_id
          : result.alice_actor_id
      currentViewer.current = { corpId: result.corp_id, actorId: initialActor }
      setSelectedActorId(initialActor)
      await refresh(result.corp_id, initialActor, controller.signal)
    })()
      .catch((caught: unknown) => {
        if (cancelled) return
        setError(timedOut
          ? 'ECorp did not finish connecting within 30 seconds. Agent runs are independent of this tab. Retry the connection; do not restart the mission.'
          : caught instanceof Error ? caught.message : String(caught))
      })
      .finally(() => window.clearTimeout(timeout))
    return () => {
      cancelled = true
      window.clearTimeout(timeout)
      controller.abort()
    }
  }, [refresh, connectionAttempt])

  useEffect(() => {
    if (!bootstrap || !selectedActorId) return
    let disposed = false
    let socket: WebSocket | null = null
    const snapshotRefresh = createSnapshotRefresher({
      refresh: (signal) => refresh(bootstrap.corp_id, selectedActorId, signal),
      onError: (caught) => {
        if (!disposed) setError(caught instanceof Error ? caught.message : String(caught))
      },
    })
    snapshotRefreshRef.current = snapshotRefresh

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
        if (disposed) return
        const message = JSON.parse(event.data) as BrowserSocketMessage
        if (message.type === 'ready') {
          lastEventSeq.current[selectedActorId] = Math.max(
            lastEventSeq.current[selectedActorId] ?? 0,
            message.replayed_through,
          )
          replaying = false
          setConnection('live')
          if (replayChanged) snapshotRefresh.request()
          return
        }
        if (message.event.seq <= (lastEventSeq.current[selectedActorId] ?? 0)) return
        lastEventSeq.current[selectedActorId] = message.event.seq
        setAnnouncement(statusLabel(message.event.type))
        if (replaying) {
          replayChanged = true
          return
        }
        snapshotRefresh.request()
      }
      socket.onerror = () => {
        if (!disposed) setConnection('offline')
      }
      socket.onclose = () => {
        if (disposed) return
        setConnection('offline')
        reconnectTimer.current = window.setTimeout(() => void connect(), 1_500)
      }
    }

    void connect()
    return () => {
      disposed = true
      if (snapshotRefreshRef.current === snapshotRefresh) snapshotRefreshRef.current = null
      snapshotRefresh.dispose()
      if (reconnectTimer.current !== null) window.clearTimeout(reconnectTimer.current)
      socket?.close()
    }
  }, [bootstrap, refresh, selectedActorId])

  useEffect(() => {
    if (!bootstrap || !selectedActorId || !(connectionsOpen || journeyOpen ||
      (activeWorkspaceView === 'missions' && !missionComposerCollapsed))) return
    const snapshotRefresh = snapshotRefreshRef.current
    if (!snapshotRefresh) return
    // Heartbeat-only readiness matters while choosing/configuring a connection,
    // not in every idle client. Share the event coalescer; never restart its socket
    // or overlap another full-Corp read when a connection-dependent view opens.
    const refreshVisiblePresence = () => {
      if (document.visibilityState === 'visible') snapshotRefresh.request()
    }
    const presenceTimer = window.setInterval(refreshVisiblePresence, 5_000)
    document.addEventListener('visibilitychange', refreshVisiblePresence)
    return () => {
      window.clearInterval(presenceTimer)
      document.removeEventListener('visibilitychange', refreshVisiblePresence)
    }
  }, [bootstrap, refresh, selectedActorId, connectionsOpen, journeyOpen, activeWorkspaceView, missionComposerCollapsed])

  const humans = useMemo(
    () => data?.snapshot.actors.filter((actor) => actor.kind === 'human') ?? [],
    [data],
  )
  const currentAgents = useMemo(
    () => currentOfficeAgents(data?.snapshot.agents ?? []),
    [data],
  )
  const connectionRoom = data
    ? resolveDiscussionRoom(data.snapshot.rooms, data.snapshot.missions, roomMissionId, selectedRoomId)
    : undefined
  const savedConnectionScope = bootstrap?.corp_id && selectedActorId && connectionRoom
    ? connectionScope(bootstrap.corp_id, connectionRoom.id, selectedActorId) : ''
  const savedConnectionCorpId = bootstrap?.corp_id
  const savedConnectionRoomId = connectionRoom?.id
  const canUseSavedConnections = humans.some((actor) => actor.id === selectedActorId && canOperate(actor.role))
  const savedConnections = savedConnectionLoad?.scope === savedConnectionScope
    ? savedConnectionLoad.data : null
  const savedConnectionRunners = useRef<RunnerNode[]>([])
  useLayoutEffect(() => { savedConnectionRunners.current = data?.runners ?? [] }, [data?.runners])
  const savedRunnerRevision = connectionRunnerRevision(data?.runners ?? [])
  const connectionRevision = data?.snapshot.events.reduce((last, event) =>
    event.type.startsWith('workspace.connection') ||
      ['runner.capabilities_updated', 'runner.credential_rotated', 'runner.enrolled',
        'runner.grace_started', 'runner.revoked'].includes(event.type)
      ? Math.max(last, event.seq) : last, 0) ?? 0
  useEffect(() => {
    if (!savedConnectionScope || !savedConnectionCorpId || !selectedActorId || !savedConnectionRoomId ||
      !canUseSavedConnections) return
    let current = true
    let timer: ReturnType<typeof setTimeout> | undefined
    let attempts = 0
    const load = async () => {
      attempts += 1
      try {
        const response = await api<WorkspaceConnections>(
          `/api/corps/${savedConnectionCorpId}/rooms/${savedConnectionRoomId}/connections?actor_id=${selectedActorId}`,
        )
        if (!current) return
        setSavedConnectionLoad({ scope: savedConnectionScope, data: response })
        if (restoredConnectionScope.current !== savedConnectionScope) {
          restoredConnectionScope.current = savedConnectionScope
          const selected = response.connections.find((connection) => connection.id === response.selected_connection_id)
          if (selected) {
            setMissionSourceKey(`connection:${selected.id}`)
            setMissionAdapter(selected.agent)
            setMissionSourceConfirmed(Boolean(selected.source && !selected.source.repository.toLowerCase().endsWith('/ecorp')))
          }
        }
        // Registration presence can precede live dispatch readiness. Re-read
        // the authoritative endpoint briefly; never manufacture a Ready state.
        if (attempts < 5 && connectionsNeedPresenceRefresh(response.connections, savedConnectionRunners.current)) {
          timer = setTimeout(() => void load(), 1000)
        }
      } catch {
        // Older nodes keep the existing mission path; a missing setup API must
        // not erase a draft or silently replace a saved source.
      }
    }
    void load()
    return () => {
      current = false
      if (timer !== undefined) clearTimeout(timer)
    }
  }, [savedConnectionCorpId, selectedActorId, savedConnectionRoomId, savedConnectionScope, connectionRevision, savedRunnerRevision, canUseSavedConnections])
  const missionRepositoryTargets = useMemo(() => {
    const saved = (savedConnections?.connections ?? [])
      .map((connection) => connectionTarget(connection, data?.runners ?? []))
      .filter((target): target is RepositoryTarget => Boolean(target))
    return [...saved, ...repositoryTargets(data)]
  }, [data, savedConnections])
  const selectedMissionConnection = missionSourceKey.startsWith('connection:')
    ? savedConnections?.connections.find((connection) => `connection:${connection.id}` === missionSourceKey)
    : undefined
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
          .filter((adapter) => developerMode || adapter.name !== 'fake-process')
        : [],
    [data, selectedMissionSource, developerMode],
  )
  const selectedActor =
    humans.find((actor) => actor.id === selectedActorId) ?? null
  const room = data
    ? resolveDiscussionRoom(data.snapshot.rooms, data.snapshot.missions, roomMissionId, selectedRoomId)
    : undefined
  // Keep each scope stable across snapshot refreshes, but replace it on context
  // changes (including A -> B -> A) so an old submission cannot become current again.
  const roomScope = useMemo<DiscussionScope>(() => ({
    corpId: bootstrap?.corp_id ?? '',
    actorId: selectedActorId ?? '',
    roomId: room?.id ?? null,
    missionId: roomMissionId,
  }), [bootstrap?.corp_id, selectedActorId, room?.id, roomMissionId])
  const factorySelection = data?.snapshot.factory_work_items.find((item) =>
    item.id === selectedFactoryItemId,
  ) ?? data?.snapshot.factory_work_items[0]
  const factoryRoom = data && factorySelection?.mission_id
    ? resolveDiscussionRoom(data.snapshot.rooms, data.snapshot.missions, factorySelection.mission_id)
    : undefined
  const factoryScope = useMemo<DiscussionScope>(() => ({
    corpId: bootstrap?.corp_id ?? '',
    actorId: selectedActorId ?? '',
    roomId: factoryRoom?.id ?? null,
    missionId: factorySelection?.mission_id ?? null,
  }), [bootstrap?.corp_id, selectedActorId, factoryRoom?.id, factorySelection?.mission_id])
  useLayoutEffect(() => {
    currentComments.current = data && selectedActor
      ? { snapshot: data.snapshot, room: roomScope, factory: factoryScope }
      : null
    return () => { currentComments.current = null }
  })
  const deterministicHarness = usesDeterministicHarness(missionStrategy)
  const studioTeam = missionStrategy === STUDIO_STRATEGY
  const selectedAdapter = selectMissionAdapter(missionStrategy, availableAdapters, missionAdapter)
  const effectiveMissionAdapter = selectedAdapter?.name ?? ''
  const selectedModel = selectedAdapter?.models.find((model) => model.id === missionModel)
  const runtimeError = missionRuntimeError(missionStrategy, selectedAdapter, missionModel)
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
  const missionRequest = selectedActor && selectedMissionSource ? buildMissionRequest({
    title: missionTitle,
    description: missionDescription,
    actorId: selectedActor.id,
    strategy: missionStrategy,
    adapter: effectiveMissionAdapter,
    model: missionModel,
    selectedModel,
    reasoningEffort: missionReasoningEffort,
    source: selectedMissionSource,
    budgetTokens: missionBudgetTokens,
    deliverableForm: missionDeliverable,
    commitDeliverable,
    contract: missionContractHasInput ? missionContract : null,
    customVerification,
    verificationPolicy: missionVerificationPolicy,
  }) : null
  const missionRequestBody = missionRequest ? JSON.stringify(missionRequest) : null
  const missionCorpId = bootstrap?.corp_id
  const missionActorId = selectedActor?.id
  const currentMissionRequest = missionCorpId && missionActorId && missionRequestBody
    ? missionRequestScope(missionCorpId, missionActorId, missionRequestBody) : null
  const missionPreviewEnabled = !missionComposerCollapsed && activeWorkspaceView === 'missions' &&
    Boolean(currentMissionRequest && selectedMissionSource && selectedActor && canOperate(selectedActor.role)) &&
    Boolean(missionTitle.trim()) && missionSourceConfirmed && !busy &&
    !runtimeError && missionVerifierErrors.length === 0

  const selectActor = (actor: Actor) => {
    currentViewer.current = bootstrap ? { corpId: bootstrap.corp_id, actorId: actor.id } : null
    currentComments.current = null
    setSelectedActorId(actor.id)
    setConnectionsOpen(false)
    setSavedConnectionLoad(null)
    setMissionSourceKey('')
    setMissionSourceConfirmed(false)
    setMissionAdapter('')
    setMissionModel('')
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

  const closeMissionComposer = () => {
    setMissionComposerCollapsed(true)
    setAnnouncement('Setup closed. Your draft stays on this page.')
    window.requestAnimationFrame(() => document.getElementById('new-mission-button')?.focus())
  }

  const createMission = async (event: FormEvent) => {
    event.preventDefault()
    if (
      !bootstrap ||
      !selectedActor ||
      !currentMissionRequest ||
      !missionTitle.trim() ||
      !selectedMissionSource ||
      !missionSourceConfirmed ||
      busy ||
      runtimeError ||
      missionVerifierErrors.length > 0
    ) {
      return
    }
    setBusy(true)
    setError(null)
    try {
      const created = await api<CreateMissionResponse>(`/api/corps/${bootstrap.corp_id}/missions`, {
        method: 'POST',
        body: currentMissionRequest.body,
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
      if (!selectedMissionSource.workspaceConnectionId) setMissionSourceKey('')
      setMissionSourceConfirmed(Boolean(selectedMissionSource.workspaceConnectionId && !isEcorpRepository(selectedMissionSource)))
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
        const launchedAgent = currentOfficeAgents(refreshed.snapshot.agents)
          .find((agent) => agent.id === launchedRun?.agent_id)
        if (launchedAgent) setSelectedAgentId(launchedAgent.id)
      }
      setAnnouncement(
        pauseAfterPlanning
          ? 'Mission plan saved on the server. It stays held until you dispatch.'
          : launched?.replayed
            ? 'This mission was already dispatched. Showing its existing run.'
            : 'Mission dispatched. The control floor reflects its current workers.',
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
      const launchedAgent = currentOfficeAgents(refreshed.snapshot.agents)
        .find((agent) => agent.id === launchedRun?.agent_id)
      if (launchedAgent) setSelectedAgentId(launchedAgent.id)
      setAnnouncement(launched.replayed
        ? 'This mission was already dispatched. No duplicate attempt was created.'
        : 'Mission dispatched. The control floor reflects its current workers.')
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
    } finally {
      setBusy(false)
    }
  }

  const resumeAgentRun = async (run: Run): Promise<string | null> => {
    if (!bootstrap || !selectedActor) return null
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
      const updated = await refresh(bootstrap.corp_id, selectedActor.id)
      return updated.snapshot.runs.find(
        (candidate) => candidate.task_id === run.task_id && candidate.resumed_from_run_id === run.id,
      )?.id ?? null
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
      return null
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

  const postRoomMessage = async (input: RoomPostInput): Promise<boolean> => {
    const isCurrent = () => {
      const current = currentComments.current
      return current !== null &&
        currentViewer.current?.corpId === input.scope.corpId &&
        currentViewer.current.actorId === input.scope.actorId &&
        canPostRoomMessage(current.snapshot, current[input.source], input.scope, input)
    }
    if (!isCurrent()) return false
    setError(null)
    try {
      await api(`/api/corps/${input.scope.corpId}/rooms/${input.roomId}/messages`, {
        method: 'POST',
        body: JSON.stringify({
          actor_id: input.scope.actorId,
          body: input.body,
          reply_to_id: input.replyToId,
          mentions: input.mentions,
          link: input.link,
          idempotency_key: input.idempotencyKey,
        }),
      })
      // A saved comment belongs to its original scope. A late response must not
      // refresh another viewer or clear a newer room/mission's draft.
      if (!isCurrent()) return false
      await refresh(input.scope.corpId, input.scope.actorId)
      return isCurrent()
    } catch (caught) {
      if (isCurrent()) setError(caught instanceof Error ? caught.message : String(caught))
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
    currentViewer.current = { corpId, actorId }
    currentComments.current = null
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
        {error ? (
          <button
            className="button button-primary"
            type="button"
            onClick={() => {
              setError(null)
              setConnectionAttempt((attempt) => attempt + 1)
            }}
          >
            Retry connection
          </button>
        ) : null}
      </main>
    )
  }

  const latestMissions = data.snapshot.missions.slice(0, 8)
  const latestEvents = data.snapshot.events.toReversed().slice(0, 28)
  const connectedRunners = data.runners.filter((runner) => runner.connected)
  const runnerLabel = connectedRunners.length
    ? `${connectedRunners.length} runner${connectedRunners.length === 1 ? '' : 's'} online`
    : data.runners.some((runner) => runner.status === 'grace')
      ? 'Runner reconnecting'
      : 'No runner'
  const floorAgents = operatingOfficeAgents(
    currentAgents,
    new Set(allAvailableAdapters.map((adapter) => adapter.name)),
    showRegisteredCrew,
  )
  const selectedAgent = selectOfficeAgent(floorAgents, selectedAgentId)
  const selectedAgentCapability = connectedRunners
    .flatMap((runner) => runner.capabilities)
    .find(
      (capability) =>
        capability.available && capability.name === selectedAgent?.adapter,
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
    isProviderLiveRun(run, data.snapshot.verification_requests),
  )
  const completedMissions = data.snapshot.missions.filter((mission) => mission.status === 'completed')
  const productionAuthenticated = Boolean(storedAccessToken())
  const activeFactoryItems = data.snapshot.factory_work_items.filter(
    (item) => !['published', 'failed', 'cancelled'].includes(item.state),
  )
  const currentWorkspaceView =
    WORKSPACE_VIEWS.find((view) => view.id === activeWorkspaceView) ??
    WORKSPACE_VIEWS[0]
  const selectedMission =
    data.snapshot.missions.find((mission) => mission.id === selectedMissionId) ??
    latestMissions[0]
  const selectedMissionTasks = selectedMission
    ? data.snapshot.tasks.filter((task) => task.mission_id === selectedMission.id)
    : []
  const selectedMissionTaskIds = new Set(selectedMissionTasks.map((task) => task.id))
  const selectedMissionRuns = selectedMission
    ? data.snapshot.runs.filter((run) => selectedMissionTaskIds.has(run.task_id))
    : []

  const selectDiscussionMission = (missionId: string | null) => {
    const destination = missionId === null ? room : resolveDiscussionRoom(
      data.snapshot.rooms, data.snapshot.missions, missionId,
    )
    setSelectedRoomId(destination?.id ?? selectedRoomId)
    setRoomMissionId(missionId)
    if (missionId) setSelectedMissionId(missionId)
  }

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
          <div className="operations-identity">
            <label htmlFor="operator-actor">
              {productionAuthenticated ? 'Signed in as' : 'Demo operator'}
              <select
                id="operator-actor"
                aria-describedby="operator-identity-help"
                disabled={productionAuthenticated}
                value={selectedActor.id}
                onChange={(event) => {
                  const actor = humans.find((candidate) => candidate.id === event.target.value)
                  if (actor) selectActor(actor)
                }}
              >
                {humans.map((actor) => (
                  <option key={actor.id} value={actor.id}>
                    {actor.name} · {actor.role}
                  </option>
                ))}
              </select>
            </label>
            <small id="operator-identity-help">
              {productionAuthenticated
                ? 'Locked to your authenticated OIDC account. Identity switching is disabled.'
                : 'Local demo permissions—not GitHub sign-in.'}
            </small>
          </div>
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
          <button type="button" onClick={() => {
            if (activeRuns[0]) {
              setSelectedAgentId(activeRuns[0].agent_id)
              setFloorInspectorOpen(true)
            } else {
              setFloorInspectorOpen(false)
            }
            activateWorkspaceView('floor')
          }}>
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
            onClick={() => {
              const runId = pendingApprovals[0]?.run_id ?? pendingVerificationRequests[0]?.run_id
              if (runId) navigateToWorkspaceEntity('run', runId)
              else activateWorkspaceView('missions')
            }}
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
          <button type="button" onClick={() => {
            if (completedMissions[0]) navigateToWorkspaceEntity('mission', completedMissions[0].id)
            else activateWorkspaceView('missions')
          }}>
            <span>Results</span>
            <strong>{completedMissions.length}</strong>
            <small>Completed missions</small>
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
              <li className={pendingApprovals.length + pendingVerificationRequests.length
                ? 'journey-current' : completedMissions.length ? 'journey-complete' : ''}>
                <span>4</span>
                <div>
                  <strong>Operate and review</strong>
                  <small>
                    {pendingApprovals.length + pendingVerificationRequests.length
                      ? `${pendingApprovals.length + pendingVerificationRequests.length} decision${pendingApprovals.length + pendingVerificationRequests.length === 1 ? '' : 's'} waiting`
                      : completedMissions.length
                        ? `${completedMissions.length} completed mission${completedMissions.length === 1 ? '' : 's'} ready`
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
          key={`factory:${discussionScopeKey(factoryScope)}:${factorySelection?.id ?? ''}`}
          items={data.snapshot.factory_work_items}
          missions={data.snapshot.missions}
          publications={data.snapshot.pull_request_publications}
          publicationAttempts={data.snapshot.pull_request_publication_attempts}
          controllers={data.snapshot.factory_controllers ?? []}
          tasks={data.snapshot.tasks}
          runs={data.snapshot.runs}
          room={factoryRoom}
          scope={factoryScope}
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
          selectedItemId={selectedFactoryItemId}
          onSelectItem={(item) => {
            setSelectedFactoryItemId(item.id)
            if (item.mission_id) {
              selectDiscussionMission(item.mission_id)
            }
          }}
          onOpenMission={(mission) => navigateToWorkspaceEntity('mission', mission.id)}
          onDiscussMission={(mission) => {
            selectDiscussionMission(mission.id)
            activateWorkspaceView('room')
          }}
          onNewMission={() => {
            setMissionComposerStep('brief')
            setMissionComposerCollapsed(false)
            activateWorkspaceView('missions')
          }}
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
            <p>Live work comes from missions. Idle workers are not running model processes.</p>
          </div>
          <label className="office-roster-toggle">
            <input type="checkbox" checked={showRegisteredCrew}
              onChange={(event) => setShowRegisteredCrew(event.target.checked)} />
            Show offline and test identities
          </label>
          <div className="floor-plan">
            {activeWorkspaceView === 'floor' ? <OfficeFloor
              agents={floorAgents}
              selectedAgentId={selectedAgent?.id ?? null}
              pendingApprovalRunIds={new Set(pendingApprovals.map((approval) => approval.run_id))}
              pendingReviewRunIds={new Set(pendingVerificationRequests.map((request) => request.run_id))}
              connection={connection}
              runnerCount={connectedRunners.length}
              onSelect={(agentId) => {
                setSelectedAgentId(agentId)
                setFloorInspectorOpen(true)
              }}
              onMissions={(agentId) => {
                const agent = currentAgents.find((candidate) => candidate.id === agentId)
                const run = data.snapshot.runs.find((candidate) => candidate.id === agent?.current_run_id)
                const task = data.snapshot.tasks.find((candidate) => candidate.id === run?.task_id)
                const missionId = task?.mission_id ?? agent?.mission_id
                if (missionId) setSelectedMissionId(missionId)
                setMissionComposerCollapsed(true)
                activateWorkspaceView('missions')
              }}
              onFactory={() => activateWorkspaceView('factory')}
            /> : null}
            {floorInspectorOpen && selectedAgent && activeWorkspaceView === 'floor' ? (
              <OfficeInspector agentName={selectedAgent.name} onClose={() => setFloorInspectorOpen(false)}>
                  <AgentDesk
                    key={selectedAgent.id}
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
          <div className="panel-heading operations-mission-toolbar">
            <div>
              <h2>Mission queue</h2>
              <p>
                {missionComposerCollapsed
                  ? 'Track work, inspect evidence, and decide what happens next.'
                  : 'Give ECorp a goal, then choose its runtime and completion evidence.'}
              </p>
            </div>
            {missionComposerCollapsed && (
              <button
                id="new-mission-button"
                className="button button-primary"
                type="button"
                onClick={() => {
                  setMissionComposerStep('brief')
                  setMissionComposerCollapsed(false)
                }}
              >
                New mission
              </button>
            )}
          </div>
          {!missionComposerCollapsed && (
          <form className="mission-form arcade-mission-form" onSubmit={createMission}>
            <div className="arcade-composer-header">
              <div>
                <span className="arcade-ready">Mission setup</span>
                <strong>New mission</strong>
                <small>Describe the work, choose a repository, then review and build.</small>
              </div>
              <button className="button button-quiet mission-composer-cancel" type="button"
                disabled={busy} onClick={closeMissionComposer}>
                Close setup
              </button>
            </div>

            <nav className="mission-stage-nav" aria-label="Mission setup stages">
              {[
                ['brief', '01', 'Describe & setup'],
                ['proof', '02', 'Review & build'],
              ].map(([step, number, label]) => (
                <button
                  key={step}
                  type="button"
                  className={missionComposerStep === step ? 'stage-active' : ''}
                  aria-current={missionComposerStep === step ? 'step' : undefined}
                  onClick={() =>
                    setMissionComposerStep(step as 'brief' | 'proof')
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
                    <span>Your goal</span>
                    <h3 className="mission-composer-heading" tabIndex={-1} ref={missionComposerHeading}>What should ECorp build?</h3>
                    <p>Describe an application, a feature, or a fix. Add a full specification only if you need one.</p>
                  </div>
                  <label className="arcade-input mission-title-input">
                    Mission outcome
                    <span>{missionTitle.length}/240</span>
                    <textarea
                      id="mission-title"
                      aria-label="Mission outcome"
                      value={missionTitle}
                      onChange={(event) => setMissionTitle(event.target.value)}
                      placeholder="Build a vendor approval portal with roles, an audit trail, and tests."
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
                  <details className="mission-advanced-options">
                    <summary>Additional details or a specification{missionDescription ? ' · added' : ' (optional)'}</summary>
                  <label className="arcade-input mission-description-field">
                    <span>
                      Additional details
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
                  </details>
                </div>
              ) : null}

              {missionComposerStep === 'brief' ? (
                <div className="mission-stage-content stage-loadout">
                  <div className="stage-title">
                    <span>Repository and team</span>
                    <h3>Where should ECorp work?</h3>
                    <p>Choose a repository and coding agent. Optional settings are below.</p>
                  </div>
                  <div className="loadout-grid">
                    <div className="mission-field repository-target-field">
                      <label htmlFor="mission-repository">Target repository</label>
                      <button className="button button-secondary" type="button"
                        onClick={() => setConnectionsOpen(true)}>
                        Connect a repository or coding agent
                      </button>
                      <select
                        id="mission-repository"
                        value={missionSourceKey}
                        aria-describedby="mission-repository-help"
                        onChange={(event) => {
                          setMissionSourceKey(event.target.value)
                          const saved = savedConnections?.connections.find((connection) =>
                            `connection:${connection.id}` === event.target.value)
                          setMissionSourceConfirmed(Boolean(saved?.source && !saved.source.repository.toLowerCase().endsWith('/ecorp')))
                          setMissionAdapter(saved?.agent ?? '')
                          setMissionModel('')
                          setMissionReasoningEffort('')
                        }}
                      >
                        <option value="">Choose a repository</option>
                        {(savedConnections?.connections ?? []).filter((connection) => !connection.source).map((connection) =>
                          <option key={connection.id} value={`connection:${connection.id}`}>
                            {connection.label} · {connectionStatusLabel(connection)}
                          </option>)}
                        {missionRepositoryTargets.map((target) => {
                          const saved = savedConnections?.connections.find(
                            (connection) => connection.id === target.workspaceConnectionId,
                          )
                          return (
                            <option key={target.key} value={target.key}>
                              {saved
                                ? `${saved.label} · ${connectionLabel(saved.agent)} · ${connectionStatusLabel(saved)}`
                                : `${target.repository} · ${target.baseRef} · ${target.baseCommit.slice(0, 12)}`}
                            </option>
                          )
                        })}
                      </select>
                      <small id="mission-repository-help">
                        {missionRepositoryTargets.length
                          ? 'The exact repository, ref, and commit are persisted in every task.'
                          : 'Connect your repository and coding agent above. You will not need to configure them for every mission.'}
                      </small>
                      {selectedMissionConnection && !selectedMissionConnection.runner_connected && (
                        <p className="field-warning" role="status">
                          This saved machine is offline. Your repository choice is kept; reconnect it or choose another connection.
                        </p>
                      )}
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
                          {(!selectedMissionSource.workspaceConnectionId || isEcorpRepository(selectedMissionSource)) ? <label className="repository-confirmation">
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
                          </label> : <p className="connections-help">Using your saved connection. ECorp rechecks this exact source before starting.</p>}
                        </div>
                      ) : null}
                    </div>
                    <div className="mission-field">
                      <label htmlFor="mission-adapter">Coding agent</label>
                      <select
                        id="mission-adapter"
                        value={effectiveMissionAdapter}
                        disabled={deterministicHarness || studioTeam}
                        aria-describedby="mission-adapter-help"
                        onChange={(event) => {
                          setMissionAdapter(event.target.value)
                          setMissionModel('')
                          setMissionReasoningEffort('')
                        }}
                      >
                        {!effectiveMissionAdapter ? (
                          <option value="">{studioTeam ? 'GitHub Copilot unavailable for this source' : 'No available runtime'}</option>
                        ) : null}
                        {availableAdapters.filter((adapter) => !studioTeam || adapter.name === 'github-copilot').map((adapter) => (
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
                        id="mission-adapter-help"
                        className={
                          deterministicHarness || effectiveMissionAdapter === 'fake-process'
                            ? 'field-warning'
                            : ''
                        }
                      >
                        {deterministicHarness
                          ? 'This test fixture owns its runtime settings.'
                          : studioTeam
                            ? 'Studio team uses GitHub Copilot for all three workers and the later integration pass. Model and reasoning settings come from the selected source runner.'
                          : selectedAdapter
                            ? adapterDescription(selectedAdapter.name)
                            : selectedMissionSource
                              ? 'No coding agent is available for this repository. Its runner must be connected.'
                              : 'Choose a repository to see its available coding agents.'}
                      </small>
                    </div>
                    <div className="mission-field">
                      <label htmlFor="mission-strategy">Team</label>
                      <select
                        id="mission-strategy"
                        value={missionStrategy}
                        aria-describedby="mission-strategy-help mission-strategy-policy"
                        onChange={(event) => {
                          const strategy = event.target.value
                          setMissionStrategy(strategy)
                          if (strategy === STUDIO_STRATEGY) {
                            setMissionAdapter('github-copilot')
                            if (effectiveMissionAdapter !== 'github-copilot') {
                              setMissionModel('')
                              setMissionReasoningEffort('')
                            }
                          }
                        }}
                      >
                        <option value="single">Solo run</option>
                        <option value="parallel-specialists">Two specialists and synthesis</option>
                        <option value={STUDIO_STRATEGY}>{STUDIO_STRATEGY_LABEL}</option>
                        {developerMode ? (
                          <optgroup label="Test fixtures">
                            <option value="verification-matrix">Verification matrix</option>
                            <option value="human-approval">Human approval</option>
                            <option value="independent-review">Independent review</option>
                            <option value="verification-failure">Failure path</option>
                          </optgroup>
                        ) : null}
                      </select>
                      <small id="mission-strategy-help">
                        {studioTeam
                          ? 'Three Copilot agents work in parallel, then one integrates their verified handoffs.'
                          : missionStrategy === 'parallel-specialists'
                          ? 'Two agents work in parallel, then a final task combines their results.'
                          : missionStrategy === 'single'
                            ? 'One agent handles the work.'
                            : 'A deterministic product-behavior fixture.'}
                      </small>
                      <small id="mission-strategy-policy" className="operations-approval-note">
                        Routine work uses the agent&apos;s native permissions. New scoped actions and any
                        required review keep their existing authority, without duplicate grants for the same action.
                      </small>
                    </div>
                  </div>
                  <details className="mission-advanced-options">
                    <summary>Model, limits and output{pauseAfterPlanning ? ' · save without starting' : ' (optional)'}</summary>
                    <div className="loadout-grid">
                    {!deterministicHarness && selectedAdapter && (selectedAdapter.models.length > 0 || missionModel) ? (
                      <div className="mission-field">
                        <label htmlFor="mission-model">Model</label>
                        <select
                          id="mission-model"
                          value={missionModel}
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
                          {missionModel && !selectedModel ? (
                            <option value={missionModel} disabled>{missionModel} · unavailable</option>
                          ) : null}
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
                        <strong>Save without starting</strong>
                        <small>Save a plan now and start it later. Leave off to build immediately.</small>
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
                           if (!enabled && effectiveMissionAdapter === 'fake-process') {
                             setMissionAdapter('')
                             setMissionModel('')
                             setMissionReasoningEffort('')
                           }
                        }}
                      />
                      <span>
                        <strong>Developer fixtures</strong>
                        <small>Expose deterministic lifecycle fixtures.</small>
                      </span>
                    </label>
                  </div>
                  </details>
                </div>
              ) : null}

              {missionComposerStep === 'proof' ? (
                <div className="mission-stage-content stage-proof">
                  <div className="stage-title">
                    <span>Ready to build</span>
                    <h3 className="mission-composer-heading" tabIndex={-1} ref={missionComposerHeading}>Review and build</h3>
                    <p>Check the destination and completion checks. Build starts the work; only choose Save if you want to start later.</p>
                  </div>
                  <div className="mission-review-target">
                    <strong>{selectedMissionSource?.repository ?? 'Choose a repository in setup'}</strong>
                    {selectedMissionSource ? (
                      <span>{selectedMissionSource.baseRef} · {selectedMissionSource.baseCommit.slice(0, 12)}</span>
                    ) : null}
                    <span>{selectedAdapter ? adapterLabel(selectedAdapter.name) : 'No coding agent selected'}</span>
                    <span>{pauseAfterPlanning ? 'Save plan — work will not start' : 'Build immediately'}</span>
                  </div>
                  <details className="mission-advanced-options">
                    <summary>Detailed requirements{missionContractHasInput ? ' · configured' : ' (optional)'}</summary>
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
                              id="mission-write-scope"
                              aria-label="Authorized write scope"
                              rows={4}
                              value={missionWriteScope}
                              onChange={(event) => setMissionWriteScope(event.target.value)}
                              aria-describedby={studioTeam ? 'studio-write-scope-help' : undefined}
                              placeholder={'apps/web/**\ncrates/crony-server/**'}
                            />
                            {studioTeam ? (
                              <small id="studio-write-scope-help">
                                Include an approved directory scope, such as src/**. The server derives
                                handoff paths beneath it; integration stays within the mission scope.
                                Use a separate target repository for disposable apps, not ECorp.
                              </small>
                            ) : null}
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

                  </details>

                  <label className="mission-run-toggle custom-verification-toggle">
                    <input
                      type="checkbox"
                      checked={customVerification && !deterministicHarness}
                      disabled={deterministicHarness}
                      onChange={(event) => {
                        setCustomVerification(event.target.checked)
                      }}
                    />
                    <span>
                      <strong>Custom verification</strong>
                      <small>Add exact tests, files or a reviewer. This does not change whether work starts now.</small>
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
                      <strong>{studioTeam ? 'Handoff files and provider artifacts must exist' : 'Provider artifact must exist'}</strong>
                      <small>Turn on custom verification for application-level tests and review gates; these are persisted, not inferred from agent claims.</small>
                    </div>
                  )}
                </div>
              ) : null}
            </section>

            <section className="mission-submit-note mission-plan-summary" aria-label="Mission settings and allocation">
              <strong>Current settings · {missionStrategy === 'single' ? 'Solo run' : statusLabel(missionStrategy)}</strong>
              <p className="operations-approval-note">
                Strategy and budget stay selected between missions.
                {!missionPreviewEnabled ? (deterministicHarness ? ' Fixture-owned budget.' : ` Requested limit: ${missionBudgetTokens.toLocaleString()} tokens.`) : ''}
              </p>
              {studioTeam ? (
                <p className="operations-approval-note">
                  Studio: 3 handoffs, then integration after all three complete.
                </p>
              ) : null}
              {missionPreviewEnabled && currentMissionRequest ? (
                <MissionAllocationPreview key={currentMissionRequest.key} scope={currentMissionRequest} />
              ) : (
                <p className="operations-approval-note" role="status">
                  {busy ? 'Starting your mission…'
                    : !selectedActor || !canOperate(selectedActor.role) ? 'Your role cannot start missions.'
                    : !missionTitle.trim() ? 'Describe the work to start setting up your mission.'
                    : !selectedMissionSource ? 'Choose the repository you want ECorp to work in.'
                    : !missionSourceConfirmed ? 'Confirm the selected repository to see the exact plan.'
                    : runtimeError ?? 'Complete the check settings to see the exact plan.'}
                </p>
              )}
            </section>
            <div className="arcade-form-controls">
              {missionComposerStep === 'proof' ? (
                <button className="button button-quiet" type="button"
                  onClick={() => setMissionComposerStep('brief')}>
                  Back to setup
                </button>
              ) : <span />}
              {missionComposerStep === 'brief' ? (
                <button key="review-setup" className="button button-primary" type="button"
                  onClick={(event) => {
                    // Keep this navigation click from becoming a submit when
                    // React replaces the setup controls with the Build button.
                    event.preventDefault()
                    setMissionComposerStep('proof')
                  }}>
                  Review and build
                </button>
              ) : (
                <button
                  key="submit-mission"
                  className="button button-primary mission-submit"
                  type="submit"
                  disabled={
                    busy ||
                    !missionTitle.trim() ||
                    !selectedMissionSource ||
                    !missionSourceConfirmed ||
                    Boolean(runtimeError) ||
                    missionVerifierErrors.length > 0
                  }
                >
                  {busy ? 'Starting mission' : pauseAfterPlanning ? 'Save plan' : 'Build'}
                </button>
              )}
            </div>
            {missionVerifierErrors.length ? (
              <p className="contract-error">{missionVerifierErrors[0]}</p>
            ) : null}
            {missionComposerStep === 'proof' && selectedMissionSource && runtimeError ? (
              <p className="contract-error" role="status">{runtimeError}</p>
            ) : null}
            {missionComposerStep === 'proof' ? (
              <p className="mission-submit-note">
                {pauseAfterPlanning
                  ? 'This saves the plan without starting. You can start it from the mission when ready.'
                  : 'Build starts the mission in its isolated workspace. Any permission exception or required review appears with the work.'}
              </p>
            ) : null}
          </form>
          )}
          {missionComposerCollapsed && (
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
                      <span>{missionStatusLabel(mission.status)}</span>
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
                  key={`${bootstrap.corp_id}:${selectedActor.id}:${selectedMission.id}`}
                  corpId={bootstrap.corp_id}
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
                  factoryItem={data.snapshot.factory_work_items.find(
                    (item) => item.mission_id === selectedMission.id,
                  )}
                  origin={missionOrigin(selectedMission.id, data.snapshot)}
                  factoryRecoveries={data.snapshot.factory_verification_recoveries.filter(
                    (recovery) => recovery.mission_id === selectedMission.id,
                  )}
                  events={data.snapshot.events}
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
                  onDiscuss={(mission) => {
                    selectDiscussionMission(mission.id)
                    activateWorkspaceView('room')
                  }}
                  onViewAgents={() => {
                    const run = selectedMissionRuns.find((candidate) => !terminalRun(candidate.status))
                    if (run && currentAgents.some((agent) => agent.id === run.agent_id)) {
                      setSelectedAgentId(run.agent_id)
                      setFloorInspectorOpen(true)
                      activateWorkspaceView('floor')
                    } else if (selectedMissionTasks[0]) {
                      navigateToWorkspaceEntity('task', selectedMissionTasks[0].id)
                    }
                  }}
                  onOpenFactory={() => {
                    const item = data.snapshot.factory_work_items.find(
                      (candidate) => candidate.mission_id === selectedMission.id,
                    )
                    if (item) setSelectedFactoryItemId(item.id)
                    activateWorkspaceView('factory')
                  }}
                />
              ) : (
                <div className="empty-state">
                  <strong>No missions yet</strong>
                  <span>Start with a concrete outcome and let ECorp create the task contract.</span>
                </div>
              )}
            </div>
          </div>
          )}
        </aside>
      </section>

      <div className="workspace-surface" hidden={activeWorkspaceView !== 'room'}>
        <RoomPanel
          key={discussionScopeKey(roomScope)}
          room={room}
          scope={roomScope}
          messages={data.snapshot.room_messages}
          actors={data.snapshot.actors}
          selectedActor={selectedActor}
          missions={data.snapshot.missions}
          tasks={data.snapshot.tasks}
          runs={data.snapshot.runs}
          onPost={postRoomMessage}
          onNavigateLink={(link) => navigateToWorkspaceEntity(link.kind, link.id)}
          onContextChange={selectDiscussionMission}
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
      {connectionsOpen && bootstrap && selectedActor && connectionRoom && (
        <ConnectionsPanel
          key={savedConnectionScope}
          corpId={bootstrap.corp_id}
          roomId={connectionRoom.id}
          actorId={selectedActor.id}
          actorRole={selectedActor.role}
          runners={data.runners}
          initialSource={selectedMissionSource}
          api={api}
          refreshRevision={connectionRevision}
          onClose={() => setConnectionsOpen(false)}
          onSelect={(connection: WorkspaceConnection) => {
            const viewer = currentViewer.current
            if (viewer?.corpId !== connection.corp_id || viewer.actorId !== selectedActor.id ||
              connection.room_id !== connectionRoom.id) return
            setSavedConnectionLoad((previous) => ({
              scope: savedConnectionScope,
              data: {
                connections: [
                  ...(previous?.scope === savedConnectionScope ? previous.data.connections : [])
                    .filter((candidate) => candidate.id !== connection.id),
                  connection,
                ],
                operations: previous?.scope === savedConnectionScope ? previous.data.operations : [],
                selected_connection_id: connection.id,
              },
            }))
            setMissionSourceKey(`connection:${connection.id}`)
            setMissionAdapter(connection.agent)
            setMissionModel('')
            setMissionReasoningEffort('')
            setMissionSourceConfirmed(Boolean(connection.source && !connection.source.repository.toLowerCase().endsWith('/ecorp')))
            setConnectionsOpen(false)
            setAnnouncement('Connection selected. Describe what you want ECorp to build.')
          }}
        />
      )}
    </main>
  )
}

export default App
