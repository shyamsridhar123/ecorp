import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { CSSProperties, FormEvent } from 'react'
import './App.css'

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
  strategy: string
  max_nodes: number
  max_depth: number
  budget_tokens: number
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
  deadline_at: string | null
  escalation: string
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
  depth: number
  max_attempts: number
  attempt_count: number
  required_adapter: string | null
  depends_on: string[]
  verification_policy: Record<string, unknown>
  verification_status: string
  status: string
  assigned_agent_id: string | null
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

type SnapshotResponse = {
  snapshot: {
    corp: { id: string; name: string }
    actors: Actor[]
    rooms: { id: string; name: string; purpose: string }[]
    agents: Agent[]
    missions: Mission[]
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

function usesDeterministicHarness(strategy: string): boolean {
  return DETERMINISTIC_HARNESS_STRATEGIES.includes(
    strategy as (typeof DETERMINISTIC_HARNESS_STRATEGIES)[number],
  )
}

const OFFICE_POSITIONS = [
  { x: 14, y: 31 },
  { x: 39, y: 31 },
  { x: 64, y: 31 },
  { x: 24, y: 69 },
  { x: 50, y: 69 },
  { x: 76, y: 69 },
] as const

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
  if (agent.status === 'idle' && !agent.current_run_id) return 'available · off shift'
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

function availableRunnerAdapters(data: SnapshotResponse | null): RunnerCapability[] {
  if (!data) return []
  const connectedRunners = data.runners.filter((runner) => runner.connected)
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
    throw new Error(body.error ?? `${response.status} ${response.statusText}`)
  }
  return body as T
}

function shortId(value: string | null | undefined): string {
  return value ? value.slice(0, 8) : '—'
}

function time(value: string): string {
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(value))
}

function leaseTokenKey(actorId: string, agentId: string): string {
  return `${actorId}:${agentId}`
}

function StatusMark({ status }: { status: Agent['status'] }) {
  return <span className={`status-mark status-${status}`} aria-label={status} />
}

function FactoryPanel({
  items,
  missions,
  publications,
  publicationAttempts,
}: {
  items: FactoryWorkItem[]
  missions: Mission[]
  publications: PullRequestPublication[]
  publicationAttempts: PullRequestPublicationAttempt[]
}) {
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
  return (
    <section className="factory-panel panel" id="factory" data-testid="factory-panel">
      <div className="panel-heading factory-heading">
        <div>
          <span className="section-code">DARK FACTORY / 02</span>
          <h2>Governed issue intake</h2>
          <p>
            GitHub Project work is claimed once, fenced, linked to one mission, and advanced by
            durable evidence—not labels alone.
          </p>
        </div>
        <div className="operations-summary">
          <span>{active.length} active</span>
          <span>{items.filter((item) => item.state === 'verified').length} verified</span>
          <span>auto-merge off</span>
        </div>
      </div>
      <div className="factory-list">
        {items.length ? (
          items.map((item) => {
            const mission = missions.find((candidate) => candidate.id === item.mission_id)
            const publication = publications.find(
              (candidate) => candidate.factory_work_item_id === item.id,
            )
            const attempts = publicationAttempts
              .filter((attempt) => attempt.publication_id === publication?.id)
              .sort((left, right) => right.attempt - left.attempt)
            return (
              <article className="factory-card" key={item.id}>
                <div className="factory-card-top">
                  <span className="factory-source">
                    GitHub issue #{item.source_issue_number}
                  </span>
                  <span className={`status-chip status-chip-${stateTone(item.state)}`}>
                    {statusLabel(item.state)}
                  </span>
                </div>
                <h3>
                  <a href={item.source_issue_url} target="_blank" rel="noreferrer">
                    {item.source_title}
                  </a>
                </h3>
                <dl>
                  <div>
                    <dt>Project</dt>
                    <dd>
                      {item.source_project_owner} / #{item.source_project_number}
                    </dd>
                  </div>
                  <div>
                    <dt>Mission</dt>
                    <dd>{mission ? shortId(mission.id) : 'Not materialized'}</dd>
                  </div>
                  <div>
                    <dt>Controller</dt>
                    <dd>{shortId(item.claim_owner_id)}</dd>
                  </div>
                  <div>
                    <dt>Lease</dt>
                    <dd>{time(item.lease_expires_at)}</dd>
                  </div>
                </dl>
                {item.failure_detail ? (
                  <p className="factory-failure">{item.failure_detail}</p>
                ) : null}
                {publication ? (
                  <div className="publication-proof" data-testid="factory-publication">
                    <div className="publication-proof-heading">
                      <strong>Verified pull-request publication</strong>
                      <span
                        className={`status-chip status-chip-${
                          publication.state === 'published'
                            ? 'completed'
                            : publication.failure_detail
                              ? 'failed'
                              : 'running'
                        }`}
                      >
                        {statusLabel(publication.state)}
                      </span>
                    </div>
                    <dl>
                      <div>
                        <dt>Target</dt>
                        <dd>
                          {publication.target_repository} · {publication.base_ref}
                          {publication.pull_request_base_ref &&
                          publication.pull_request_base_ref !== publication.base_ref
                            ? ` → ${publication.pull_request_base_ref}`
                            : ''}
                        </dd>
                      </div>
                      <div>
                        <dt>Branch</dt>
                        <dd>
                          {publication.branch} @ {shortId(publication.commit_sha)}
                        </dd>
                      </div>
                      <div>
                        <dt>Authorization</dt>
                        <dd>
                          {publication.authorization_snapshot.actor_role ?? 'authorized'} ·{' '}
                          {shortId(publication.actor_id)}
                        </dd>
                      </div>
                      <div>
                        <dt>Attempts</dt>
                        <dd>
                          {publication.attempt_count}
                          {attempts[0] ? ` · latest ${statusLabel(attempts[0].state)}` : ''}
                        </dd>
                      </div>
                    </dl>
                    {publication.authorization_snapshot.reason ? (
                      <p>{publication.authorization_snapshot.reason}</p>
                    ) : null}
                    {publication.pull_request_url ? (
                      <>
                        <a
                          className="publication-link"
                          href={publication.pull_request_url}
                          target="_blank"
                          rel="noreferrer"
                        >
                          Pull request #{publication.pull_request_number} ·{' '}
                          {publication.pull_request_draft ? 'draft' : 'open for review'}
                        </a>
                        <span className="publication-pending">
                          Verified head {publication.pull_request_head_repository_owner} @{' '}
                          {publication.pull_request_head_sha
                            ? shortId(publication.pull_request_head_sha)
                            : 'pending'}
                          {publication.pull_request_is_cross_repository === false
                            ? ' · same repository'
                            : ''}
                          {publication.pull_request_base_ref
                            ? ` · PR base ${publication.pull_request_base_ref}`
                            : ''}
                        </span>
                      </>
                    ) : (
                      <span className="publication-pending">Pull request not created yet</span>
                    )}
                    <footer>
                      <span>
                        Project {publication.project_status_before}
                        {publication.project_status_after
                          ? ` → ${publication.project_status_after}`
                          : ''}
                      </span>
                      <span>auto-merge off · merge/deploy unauthorized</span>
                    </footer>
                    {publication.failure_detail ? (
                      <p className="factory-failure">{publication.failure_detail}</p>
                    ) : null}
                  </div>
                ) : null}
                <footer>
                  <span>rev {item.source_revision}</span>
                  <span>v{item.version}</span>
                </footer>
              </article>
            )
          })
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
    <div className={`agent-sprite accent-${agent.accent} sprite-${agent.status}`} aria-hidden="true">
      <span className="sprite-shadow" />
      <span className="sprite-person">
        <span className="sprite-head">
          <span className="sprite-face" />
        </span>
        <span className="sprite-body" />
        <span className="sprite-arm sprite-arm-left" />
        <span className="sprite-arm sprite-arm-right" />
        <span className="sprite-leg sprite-leg-left" />
        <span className="sprite-leg sprite-leg-right" />
      </span>
    </div>
  )
}

function OfficeFloor({
  agents,
  selectedAgentId,
  onSelect,
}: {
  agents: Agent[]
  selectedAgentId: string
  onSelect: (agent: Agent) => void
}) {
  const liveAgents = agents.filter(
    (agent) =>
      agent.current_run_id ||
      ['starting', 'working', 'blocked', 'reviewing'].includes(agent.status),
  )
  return (
    <div className="office-stage" aria-label="Live agent office">
      <div className="office-wall">
        <span className="office-clock" />
        <span className="office-window office-window-left" />
        <span className="office-window office-window-right" />
        <span className="office-sign">ECORP · AUTOMATION CONTROL</span>
      </div>
      <div className="office-zone zone-review">
        <span>Review table</span>
        <i />
      </div>
      <div className="office-zone zone-approval">
        <span>Approval desk</span>
        <i />
      </div>
      <div className="office-zone zone-lounge">
        <span>Operator bay</span>
        <i />
      </div>
      {liveAgents.map((agent) => {
        const index = agents.findIndex((candidate) => candidate.id === agent.id)
        const home = OFFICE_POSITIONS[index % OFFICE_POSITIONS.length]
        const destination =
          agent.status === 'blocked'
            ? { x: 86, y: 25 }
            : agent.status === 'reviewing'
              ? { x: 51 + (index % 2) * 8, y: 23 }
              : agent.status === 'offline'
                ? { x: 6, y: 82 }
                : home
        const style = {
          '--agent-x': `${destination.x}%`,
          '--agent-y': `${destination.y}%`,
          '--agent-delay': `${index * -0.37}s`,
        } as CSSProperties
        return (
          <button
            key={agent.id}
            type="button"
            className={`office-agent office-agent-${agent.status} ${
              selectedAgentId === agent.id ? 'office-agent-selected' : ''
            }`}
            style={style}
            onClick={() => onSelect(agent)}
            aria-label={`Inspect ${agent.name}, ${agent.status}`}
            data-testid={`agent-${agent.name}`}
          >
            {agent.status !== 'idle' && agent.status !== 'offline' ? (
              <span className="sprite-activity">
                {agent.status === 'blocked' ? 'Approval needed' : agent.station ?? agent.status}
              </span>
            ) : null}
            <AgentAvatar agent={agent} />
            <span className="sprite-name">
              <StatusMark status={agent.status} />
              {agent.name}
            </span>
          </button>
        )
      })}
      {liveAgents.length === 0 ? (
        <div className="office-empty" role="status">
          <span>No live agent processes</span>
          <strong>The crew is off shift</strong>
          <small>Start a mission to launch a worker in an isolated worktree.</small>
        </div>
      ) : null}
      {agents.map((agent, index) => {
        const position = OFFICE_POSITIONS[index % OFFICE_POSITIONS.length]
        const style = {
          '--desk-x': `${position.x}%`,
          '--desk-y': `${position.y}%`,
        } as CSSProperties
        return (
          <div className="office-desk-mini" style={style} key={`desk-${agent.id}`} aria-hidden="true">
            <span className="mini-monitor" />
            <span className="mini-desk" />
            <span className="mini-chair" />
          </div>
        )
      })}
      <div className="office-door" aria-hidden="true"><span>Secure runner</span></div>
      <div className="office-carpet" aria-hidden="true" />
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
  onMessage: (agent: Agent, text: string, token: string | undefined) => Promise<void>
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
    await onMessage(agent, text, messageToken)
    setText('')
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

function MissionCard({
  mission,
  tasks,
  runs,
  agents,
  evidence,
  deliverables,
  verificationRequests,
  actionApprovals,
  actorId,
  actorRole,
  busy,
  onLaunch,
  onResume,
  onDownloadArtifact,
  onDownloadDeliverable,
  onVerificationDecision,
  onActionApprovalDecision,
}: {
  mission: Mission
  tasks: Task[]
  runs: Run[]
  agents: Agent[]
  evidence: VerificationEvidence[]
  deliverables: SourceDeliverable[]
  verificationRequests: VerificationRequest[]
  actionApprovals: ActionApproval[]
  actorId: string
  actorRole: string
  busy: boolean
  onLaunch: (mission: Mission) => Promise<void>
  onResume: (run: Run) => Promise<void>
  onDownloadArtifact: (run: Run) => Promise<void>
  onDownloadDeliverable: (deliverable: SourceDeliverable) => Promise<void>
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
  const resumableRun = runs.find(
    (run) =>
      run.provider_session_id &&
      run.workspace_disposition !== 'removed' &&
      terminalRun(run.status),
  )
  const pendingRun = runs.find((run) => run.status === 'waiting_for_approval')
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
      <div className="strategy-chip">{statusLabel(mission.strategy)}</div>
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
      <div className="task-graph-list">
        {orderedTasks.map((task) => {
          const assignedAgent = agents.find((agent) => agent.id === task.assigned_agent_id)
          const dependencies = task.depends_on
            .map((dependencyId) => taskById.get(dependencyId)?.plan_key ?? shortId(dependencyId))
          return (
            <details className="task-graph-item" key={task.id} data-task-id={task.id}>
              <summary className="task-graph-row">
                <span>{task.plan_key}</span>
                <strong>{statusLabel(task.status)}</strong>
                <small>d{task.depth} · {task.attempt_count}/{task.max_attempts}</small>
              </summary>
              <div className="task-contract">
                <p>{task.objective}</p>
                <dl>
                  <div><dt>Agent</dt><dd>{assignedAgent?.name ?? 'Unassigned'} · {adapterLabel(task.required_adapter ?? assignedAgent?.adapter ?? 'unknown')}</dd></div>
                  <div><dt>Depends on</dt><dd>{dependencies.length ? dependencies.join(', ') : 'Nothing — ready independently'}</dd></div>
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
              </div>
            </details>
          )
        })}
      </div>
      {latestRun?.artifact_sha256 && latestRun.artifact_uri ? (
        <div className="evidence-box evidence-provider" data-testid="provider-evidence">
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
        <div
          className={`verification-box verification-${latestRun.verification_status}`}
          data-testid="verification-evidence"
        >
          <strong>{statusLabel(latestRun.verification_status)}</strong>
          <span>
            {latestEvidence.filter((item) => item.status === 'passed').length}/
            {latestEvidence.length} checks passed
          </span>
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
        </div>
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
        <button className="button button-secondary mission-launch" type="button" disabled={busy} onClick={() => onResume(resumableRun)}>
          Resume agent session
        </button>
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
  }) => Promise<void>
}) {
  const [body, setBody] = useState('')
  const [replyToId, setReplyToId] = useState<string | null>(null)
  const [linkValue, setLinkValue] = useState('')

  if (!room) {
    return (
      <section className="room-panel panel" id="room">
        <div className="panel-heading">
          <div>
            <span className="section-code">SECURE COMMS / 03</span>
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
    await onPost({
      roomId: room.id,
      body,
      replyToId,
      mentions,
      link,
    })
    setBody('')
    setReplyToId(null)
    setLinkValue('')
  }

  return (
    <section className="room-panel panel" id="room" data-room-id={room.id}>
      <div className="panel-heading">
        <div>
          <span className="section-code">SECURE COMMS / 03</span>
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
                          onClick={() => {
                            const target = document.querySelector<HTMLElement>(
                              `[data-${linked.kind}-id="${CSS.escape(linked.id)}"]`,
                            )
                            target?.scrollIntoView({ behavior: 'smooth', block: 'center' })
                            target?.focus({ preventScroll: true })
                          }}
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
  const [missionAdapter, setMissionAdapter] = useState('')
  const [missionModel, setMissionModel] = useState('')
  const [missionReasoningEffort, setMissionReasoningEffort] = useState('')
  const [missionStrategy, setMissionStrategy] = useState('single')
  const [missionBudgetTokens, setMissionBudgetTokens] = useState(1_000_000)
  const [missionDeliverable, setMissionDeliverable] =
    useState<NonNullable<TaskContract['deliverable']>['form']>('archive')
  const [commitDeliverable, setCommitDeliverable] = useState(false)
  const [pauseAfterPlanning, setPauseAfterPlanning] = useState(false)
  const [developerMode, setDeveloperMode] = useState(false)
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null)
  const [journeyOpen, setJourneyOpen] = useState(true)
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
        const target = (['run', 'task', 'mission', 'room'] as const)
          .map((kind) => {
            const id = values.get(kind)
            return id ? document.querySelector(`[data-${kind}-id="${CSS.escape(id)}"]`) : null
          })
          .find(Boolean)
        target?.scrollIntoView({ behavior: 'smooth', block: 'center' })
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
  }, [bootstrap])
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
  const availableAdapters = useMemo(() => availableRunnerAdapters(data), [data])
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
    if (!bootstrap || !selectedActor || !missionTitle.trim()) return
    setBusy(true)
    setError(null)
    try {
      const created = await api<CreateMissionResponse>(`/api/corps/${bootstrap.corp_id}/missions`, {
        method: 'POST',
        body: JSON.stringify({
          title: missionTitle,
          requested_by: selectedActor.id,
          preferred_adapter: effectiveMissionAdapter,
          preferred_model: !deterministicHarness && selectedModel ? missionModel : null,
          reasoning_effort:
            !deterministicHarness &&
            selectedModel?.supported_reasoning_efforts.includes(missionReasoningEffort)
              ? missionReasoningEffort
              : null,
          strategy: missionStrategy,
          budget_tokens: deterministicHarness ? null : missionBudgetTokens,
          deliverable: {
            form: missionDeliverable,
            commit_after_verification:
              commitDeliverable || missionDeliverable === 'commit_branch',
            paths: [],
          },
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
            note: approved
              ? `${selectedActor.name} approved the scoped action.`
              : `${selectedActor.name} rejected the scoped action.`,
            decision_key: crypto.randomUUID(),
          }),
        },
      )
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

  const sendMessage = async (agent: Agent, text: string, token: string | undefined) => {
    if (!bootstrap || !selectedActor) return
    setError(null)
    try {
      await api(`/api/corps/${bootstrap.corp_id}/agents/${agent.id}/messages`, {
        method: 'POST',
        body: JSON.stringify({
          actor_id: selectedActor.id,
          lease_token: token ?? null,
          text,
        }),
      })
      await refresh(bootstrap.corp_id, selectedActor.id)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
    }
  }

  const postRoomMessage = async (input: {
    roomId: string
    body: string
    replyToId: string | null
    mentions: string[]
    link: EntityLink | null
  }) => {
    if (!bootstrap || !selectedActor) return
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
        }),
      })
      await refresh(bootstrap.corp_id, selectedActor.id)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
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
  const realAdapters = availableAdapters.filter((adapter) => adapter.name !== 'fake-process')
  const pendingApprovals = data.snapshot.action_approvals.filter(
    (approval) => approval.status === 'pending',
  )
  const activeRuns = data.snapshot.runs.filter((run) =>
    ['provisioning', 'starting', 'running', 'waiting_for_input', 'waiting_for_approval', 'verifying'].includes(run.status),
  )
  const acceptedArtifacts = data.snapshot.runs.filter(
    (run) => run.verification_status === 'passed' && run.artifact_uri,
  )
  const productionAuthenticated = Boolean(storedAccessToken())

  return (
    <main className="app-shell" aria-busy={busy}>
      <div className="sr-only" role="status" aria-live="polite" aria-atomic="true">
        {announcement}
      </div>
      <header className="topbar">
        <div className="brand-lockup">
          <span className="brand-kicker">Distributed intelligence division</span>
          <div className="brand-row">
            <span className="brand-mark" aria-hidden="true">
              <span className="brand-e">E</span>
              <span className="brand-slash" />
            </span>
            <h1><span>e</span>corp</h1>
            <span className="alpha-stamp">OPS NETWORK / NODE 00A</span>
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
          <label>
            Operating as
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
        <a href="#floor">Control floor</a>
        <a href="#factory">Factory</a>
        <a href="#missions">Missions</a>
        <a href="#room">Comms</a>
        <a href="#activity">Audit</a>
        <button type="button" onClick={() => setJourneyOpen((current) => !current)}>
          {journeyOpen ? 'Hide start guide' : 'Show start guide'}
        </button>
      </nav>

      <section className={`journey-panel ${journeyOpen ? '' : 'journey-panel-collapsed'}`}>
        <div className="journey-heading">
          <div>
            <span className="section-code">START HERE</span>
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

      <FactoryPanel
        items={data.snapshot.factory_work_items}
        missions={data.snapshot.missions}
        publications={data.snapshot.pull_request_publications}
        publicationAttempts={data.snapshot.pull_request_publication_attempts}
      />

      <section className="office-grid">
        <div className="floor-panel panel" id="floor">
          <div className="panel-heading">
            <div>
              <span className="section-code">CONTROL FLOOR / 01</span>
              <h2>{room?.name ?? 'Automation division'}</h2>
              <p>Active sessions and work awaiting review appear on the floor. Finished agents return off shift.</p>
            </div>
            <div className="floor-legend">
              <span><StatusMark status="idle" /> off shift</span>
              <span><StatusMark status="working" /> active</span>
              <span><StatusMark status="reviewing" /> review</span>
              <span><StatusMark status="blocked" /> blocked</span>
            </div>
          </div>
          <div className="floor-plan">
            <OfficeFloor
              agents={data.snapshot.agents}
              selectedAgentId={selectedAgent.id}
              onSelect={(agent) => setSelectedAgentId(agent.id)}
            />
            <div className="agent-roster" aria-label="Agent roster">
              {data.snapshot.agents.map((agent) => (
                <button
                  key={agent.id}
                  type="button"
                  className={selectedAgent.id === agent.id ? 'agent-roster-selected' : ''}
                  onClick={() => setSelectedAgentId(agent.id)}
                >
                  <StatusMark status={agent.status} />
                  <span>{agent.name}</span>
                  <small>
                    {adapterLabel(agent.adapter)} · {agentStatusLabel(agent)}
                  </small>
                </button>
              ))}
            </div>
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
          </div>
        </div>

        <aside className="mission-panel panel" id="missions">
          <div className="panel-heading">
            <div>
              <span className="section-code">MISSION CONTROL / 02</span>
              <h2>Authorize work</h2>
              <p>Describe the outcome. ECorp isolates the repo, dispatches agents, and verifies the result.</p>
            </div>
          </div>
          <form className="mission-form" onSubmit={createMission}>
            <div className="mission-form-heading">
              <div>
                <strong>What should the crew deliver?</strong>
                <span>Write the outcome and the proof you expect—not a chat message.</span>
              </div>
              <span>{missionTitle.length}/240</span>
            </div>
            <textarea
              id="mission-title"
              aria-label="Mission outcome"
              value={missionTitle}
              onChange={(event) => setMissionTitle(event.target.value)}
              placeholder="Example: Fix checkout totals, add regression tests, and attach the test evidence."
              rows={4}
              maxLength={240}
            />
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
            <div className="mission-field">
              <label htmlFor="mission-adapter">Who should run it?</label>
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
                      ? ` · ${adapter.models.filter((model) => model.policy_state !== 'disabled').length} models`
                      : adapter.name === 'fake-process'
                        ? ' · no AI'
                        : ''}
                  </option>
                ))}
              </select>
              <small className={deterministicHarness || effectiveMissionAdapter === 'fake-process' ? 'field-warning' : ''}>
                {deterministicHarness
                  ? 'This lifecycle fixture always uses the deterministic test harness. AI models and reasoning settings do not apply.'
                  : selectedAdapter
                  ? adapterDescription(selectedAdapter.name)
                  : 'Connect a runner to make an agent runtime available.'}
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
                      {model.name} · {model.id}
                      {model.policy_state === 'disabled' ? ' · disabled by policy' : ''}
                    </option>
                  ))}
                </select>
                <small>
                  {selectedModel
                    ? `${selectedModel.max_context_window_tokens?.toLocaleString() ?? 'Unknown'} context tokens${selectedModel.supports_vision ? ' · vision' : ''}`
                    : 'Use the provider default or choose an account-enabled model.'}
                </small>
              </div>
            ) : null}
            {!deterministicHarness && selectedModel?.supports_reasoning_effort ? (
              <div className="mission-field">
                <label htmlFor="mission-reasoning">Reasoning effort</label>
                <select
                  id="mission-reasoning"
                  value={
                    selectedModel.supported_reasoning_efforts.includes(missionReasoningEffort)
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
                <label htmlFor="mission-budget">Mission token budget</label>
                <select
                  id="mission-budget"
                  value={missionBudgetTokens}
                  onChange={(event) => setMissionBudgetTokens(Number(event.target.value))}
                >
                  <option value={500_000}>Quick task · 500,000 tokens</option>
                  <option value={1_000_000}>Standard · 1,000,000 tokens</option>
                  <option value={2_000_000}>Large build · 2,000,000 tokens</option>
                </select>
                <small>
                  This is a hard safety ceiling across the run. Use Large build for games or
                  other multi-file work with high reasoning effort.
                </small>
              </div>
            ) : null}
            <div className="mission-field">
              <label htmlFor="mission-deliverable">Portable deliverable</label>
              <select
                id="mission-deliverable"
                value={missionDeliverable}
                onChange={(event) =>
                  setMissionDeliverable(
                    event.target.value as NonNullable<TaskContract['deliverable']>['form'],
                  )
                }
              >
                <option value="archive">Source archive</option>
                <option value="patch">Deterministic Git patch</option>
                <option value="typed_artifact_set">Typed artifact set</option>
                <option value="commit_branch">Verified commit and branch bundle</option>
                <option value="review_only_report">Review-only report</option>
              </select>
              <small>
                Source exports include tracked changes and untracked files, but never ignored
                secret-like paths or runner internals.
              </small>
            </div>
            <label className="mission-run-toggle">
              <input
                type="checkbox"
                checked={commitDeliverable || missionDeliverable === 'commit_branch'}
                disabled={missionDeliverable === 'commit_branch'}
                onChange={(event) => setCommitDeliverable(event.target.checked)}
              />
              <span>
                <strong>Commit after verification</strong>
                <small>
                  Creates a commit only on the isolated task branch. Publication and merge stay
                  separate authorized effects.
                </small>
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
                <strong>Developer mode</strong>
                <small>Expose deterministic lifecycle and verification fixtures.</small>
              </span>
            </label>
            <div className="mission-field">
              <label htmlFor="mission-strategy">Execution pattern</label>
              <select
                id="mission-strategy"
                value={missionStrategy}
                onChange={(event) => setMissionStrategy(event.target.value)}
              >
                <option value="single">One agent delivers the outcome</option>
                <option value="parallel-specialists">Two specialists, then synthesis</option>
                {developerMode ? (
                  <optgroup label="Deterministic verification fixtures">
                    <option value="verification-matrix">Automated verification matrix</option>
                    <option value="human-approval">Pause for human approval</option>
                    <option value="independent-review">Require an independent reviewer</option>
                    <option value="verification-failure">Verification failure demo</option>
                  </optgroup>
                ) : null}
              </select>
              <small>
                {missionStrategy === 'single'
                  ? 'Best for a focused build, fix, or review.'
                  : missionStrategy === 'parallel-specialists'
                    ? 'ECorp runs two independent approaches before a final synthesis task.'
                    : missionStrategy.includes('approval') || missionStrategy.includes('review')
                      ? 'The run pauses until an authorized human records a decision.'
                      : 'This strategy exercises ECorp verification behavior.'}
              </small>
            </div>
            <label className="mission-run-toggle">
              <input
                type="checkbox"
                checked={pauseAfterPlanning}
                onChange={(event) => setPauseAfterPlanning(event.target.checked)}
              />
              <span>
                <strong>Pause after planning</strong>
                <small>Inspect the generated task graph before dispatching agents.</small>
              </span>
            </label>
            <button
              className="button button-primary mission-submit"
              type="submit"
              disabled={busy || !missionTitle.trim() || !effectiveMissionAdapter}
            >
              {busy ? 'Working…' : pauseAfterPlanning ? 'Create mission plan' : 'Plan and run mission'}
            </button>
            <p className="mission-submit-note">
              {pauseAfterPlanning
                ? 'No agent starts until you press “Dispatch mission” on the new plan.'
                : 'The mission is planned and dispatched immediately. Risky actions still require approval.'}
            </p>
          </form>
          <div className="mission-list">
            {latestMissions.length ? (
              latestMissions.map((mission) => {
                const tasks = data.snapshot.tasks.filter((candidate) => candidate.mission_id === mission.id)
                const taskIds = new Set(tasks.map((task) => task.id))
                const runs = data.snapshot.runs.filter((candidate) => taskIds.has(candidate.task_id))
                return (
                  <MissionCard
                    key={mission.id}
                    mission={mission}
                    tasks={tasks}
                    runs={runs}
                    agents={data.snapshot.agents}
                    evidence={data.snapshot.verification_evidence}
                    deliverables={data.snapshot.source_deliverables}
                    verificationRequests={data.snapshot.verification_requests}
                    actionApprovals={data.snapshot.action_approvals}
                    actorId={selectedActor.id}
                    actorRole={selectedActor.role}
                    busy={busy}
                    onLaunch={launchMission}
                    onResume={resumeAgentRun}
                    onDownloadArtifact={downloadArtifact}
                    onDownloadDeliverable={downloadDeliverable}
                    onVerificationDecision={decideVerification}
                    onActionApprovalDecision={decideActionApproval}
                  />
                )
              })
            ) : (
              <div className="empty-state">
                <strong>No missions yet</strong>
                <span>Start with a concrete outcome and let ECorp create the task contract.</span>
              </div>
            )}
          </div>
        </aside>
      </section>

      <RoomPanel
        room={room}
        messages={data.snapshot.room_messages}
        actors={data.snapshot.actors}
        selectedActor={selectedActor}
        missions={data.snapshot.missions}
        tasks={data.snapshot.tasks}
        runs={data.snapshot.runs}
        onPost={postRoomMessage}
      />

      <section className="operations-panel panel" id="activity">
        <div className="panel-heading operations-heading">
          <div>
            <span className="section-code">AUDIT NETWORK / 04</span>
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
