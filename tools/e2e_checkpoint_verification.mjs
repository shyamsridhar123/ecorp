#!/usr/bin/env node
/**
 * Additive PR #195 native checkpoint-verification acceptance. IMPORTS ARE INERT.
 *
 * Parent owns preparation, service/process identity, browser observation and
 * execution. This driver never starts/stops/enrolls/resets a service, runs SQL,
 * reads environment credentials, changes default attempts, or contacts GitHub.
 * The only child is the explicitly supplied, hash-pinned candidate crony CLI;
 * its GitHub implementation is tools/fake_github_cli.mjs with NEW local state.
 *
 * Required: --receipt <absolute issue195 runtime.json> --output-dir <absolute
 * existing owned subdirectory of the issue195 runtime directory> --cli <absolute
 * candidate crony-cli.exe> --fixture-sha256 <CURRENT fixture SHA256>
 * --reviewer-actor-id <existing independent human UUID>.
 * Optional: --tokens 5000|6000 (default 5000), --timeout-ms, --poll-ms,
 * --settle-ms, --cli-sha256 <explicit rebuilt candidate digest>,
 * --missing-required-artifact, --review-via-browser, --continue.
 * Review is ALWAYS external/browser-only. The driver returns the exact pending
 * review IDs without a decision POST; --continue observes Bob's native decision.
 *
 * --continue is OBSERVATION ONLY. It never repeats a CLI/decision mutation or
 * creates a replacement. Lost responses are reconciled only to the unique,
 * recorded synthetic issue/item/mission/run lineage. Unknown results fail closed.
 * Intents precede every non-retried mutation. Existing reports, failures, lock
 * files, and runtime artifacts are retained; stale locks require parent action.
 *
 * Evidence is the complete actor-visible journal through each Ready watermark
 * (payloads fingerprinted/redacted), persisted native checks and independent
 * review, exact source bytes, and a verified nonempty deliverable download.
 * This is deterministic native Codex [budget-stream] PROTOCOL acceptance, not
 * real vendor inference, OS-containment proof, or finished #148 enterprise work.
 */

import { execFile } from 'node:child_process'
import { createHash, randomUUID } from 'node:crypto'
import { constants } from 'node:fs'
import * as fs from 'node:fs/promises'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
// This module has a pathToFileURL guarded entry; importing does not run its E2E.
import {
  BASE_SHA256, CheckFailure, FIXTURE, LIMITS, assertHistory, assertRunner,
  canonical, checkContainedFile, configurationFromReceipt, containedPath,
  createApi, digest, expectedWorkspace, historySummary, localAbsolute,
  readBaseFile, receiptIdentity, replaySummary,
} from './e2e_stopped_source_checkpoint.mjs'

export const SUITE = 'checkpoint-verification-195'
export const PIN = Object.freeze({
  server: 'http://127.0.0.1:18574',
  repository: 'shyamsridhar123/ecorp-enterprise-lab',
  source: 'C:\\Users\\shyamsridhar\\.codex\\dogfood\\issue174-startup-20260907\\source',
  runner_root: 'C:\\Users\\shyamsridhar\\.codex\\dogfood\\issue174-startup-20260907\\runner-workspaces',
  runtime: 'C:\\Users\\shyamsridhar\\.codex\\dogfood\\issue195-checkpoint-runtime-20260908',
  workspace: 'C:\\Users\\shyamsridhar\\.codex\\worktrees\\ecorp-issue174-local-start',
  source_commit: 'a8894b5f02d56f10e2da38df47a450ff71e92fbe',
  code_commit: 'e291a6e19c0b5079ce7260a2fe05c4eb451165c4',
  runner: 'issue174-local-start',
  server_sha256: 'b39e4306d1526a674c57c08091a3b3d9599e4c10609c2fd8b7df0d107c0b80c0',
  runner_sha256: 'b20d6ed77ef7355a2f8f62aa3496186bc38f539b50214e841bbc95698525fd56',
  cli_sha256: '045aa20771f603f91026eb4755c4df8198404a6aaeaf274d3ba6ea280bfb8a1c',
  git: 'C:\\Program Files\\Git\\cmd\\git.exe',
})
const UUID = /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/u
const SHA = /^[0-9a-f]{64}$/u
const TERMINAL = new Set(['completed', 'cancelled', 'failed', 'lost'])
const PUBLIC_STATES = new Set([
  'provisioning', 'starting', 'running', 'waiting_for_input', 'waiting_for_approval', 'verifying',
  'completed', 'cancelled', 'failed', 'lost', 'discovered', 'eligible', 'claimed', 'materialized',
  'ready', 'blocked', 'verification_failed', 'awaiting_approval', 'verified', 'published',
])
const EXTRA_HISTORY = [
  'factory_work_items', 'factory_verification_recoveries', 'verification_evidence',
  'verification_requests', 'source_deliverables', 'mission_contract_revisions',
  'mission_budget_revisions', 'action_approvals', 'circuit_breaker_incidents',
]
const hashBytes = (bytes) => createHash('sha256').update(bytes).digest('hex')
const ensure = (condition, code) => { if (!condition) throw new CheckFailure(code) }
const equal = (a, b, code) => ensure(canonical(a) === canonical(b), code)
const uuid = (value) => { ensure(typeof value === 'string' && UUID.test(value), 'invalid_identity'); return value }
const sha = (value) => { ensure(typeof value === 'string' && SHA.test(value), 'invalid_digest'); return value }
const samePath = (a, b, api = path) => api.relative(localAbsolute(a, api), localAbsolute(b, api)) === ''
const safeError = (error) => error instanceof CheckFailure ? error.code : 'operation_failed_details_withheld'

export function parseArgs(argv) {
  if (argv.length === 1 && argv[0] === '--help') return { help: true }
  const options = {}
  const names = {
    '--receipt': 'receiptPath', '--output-dir': 'outputDir', '--cli': 'cli',
    '--cli-sha256': 'cliSha256',
    '--fixture-sha256': 'fixtureSha256', '--reviewer-actor-id': 'reviewer',
    '--tokens': 'tokens', '--timeout-ms': 'timeoutMs', '--poll-ms': 'pollMs',
    '--settle-ms': 'settleMs',
  }
  for (let index = 0; index < argv.length; index += 1) {
    const option = argv[index]
    const flag = option === '--continue' ? 'continuation' :
      option === '--review-via-browser' ? 'reviewViaBrowser' :
      option === '--missing-required-artifact' ? 'missingArtifact' : null
    const name = flag ?? names[option]
    ensure(name && !Object.hasOwn(options, name), 'unknown_or_duplicate_option')
    if (flag) options[name] = true
    else {
      const value = argv[++index]
      ensure(value && !value.startsWith('--'), 'option_value_missing')
      options[name] = name.endsWith('Ms') || name === 'tokens' ? Number(value) : value
    }
  }
  for (const name of ['receiptPath', 'outputDir', 'cli', 'fixtureSha256', 'reviewer']) {
    ensure(options[name] !== undefined, 'explicit_receipt_output_cli_fixture_reviewer_required')
  }
  return options
}

/** Adapt the retained PUBLIC supervisor shape, never edit its issue190 ancestor. */
export function configuration(receipt, options, api = path) {
  ensure(receipt?.issue === 195 && receipt.source_commit === PIN.code_commit,
    'exact_issue195_candidate_receipt_required')
  ensure(receipt.server_url === PIN.server && receipt.runner_id === PIN.runner &&
    receipt.source_base_commit === PIN.source_commit && receipt.source_base_ref === 'HEAD',
  'receipt_target_not_authorized')
  for (const [field, expected] of [
    ['workspace', PIN.workspace], ['source_repository_path', PIN.source],
    ['runner_root', PIN.runner_root],
  ]) ensure(samePath(receipt[field], expected, api), 'receipt_path_not_authorized')
  const receiptPath = localAbsolute(options.receiptPath, api)
  const outputDir = containedPath(PIN.runtime, options.outputDir, api)
  const cli = localAbsolute(options.cli, api)
  ensure(api.basename(cli).toLowerCase().endsWith('.exe'), 'native_candidate_executable_required')
  const cliSha256 = sha((options.cliSha256 ?? PIN.cli_sha256).toLowerCase())
  const fixtureSha256 = sha(options.fixtureSha256?.toLowerCase())
  const reviewer = uuid(options.reviewer)
  ensure(reviewer !== receipt.actor_id, 'reviewer_must_be_independent')
  const tokens = options.tokens ?? 5000
  ensure(tokens === 5000 || tokens === 6000, 'only_native_stop_or_suspend_budget')
  const publicBinaries = {}
  for (const role of ['server', 'runner']) {
    const binary = receipt.binaries?.[role]
    ensure(typeof binary?.sha256 === 'string' && SHA.test(binary.sha256.toLowerCase()) &&
      samePath(binary.path, receipt.processes?.[role]?.executable, api), 'runtime_binary_receipt_mismatch')
    publicBinaries[role] = {
      path: containedPath(api.join(PIN.runtime, 'bin'), binary.path, api),
      sha256: binary.sha256.toLowerCase(),
    }
  }
  // Reuse its guarded, strict public process/UUID/path/timing normalization.
  // The old helper accepts only issue190; admission for issue195 happened ABOVE.
  const config = configurationFromReceipt({ ...receipt, issue: 190 }, {
    sourceRepository: PIN.repository, fixture: FIXTURE, fixtureSha256, outputDir,
    reportPath: api.join(outputDir, 'checkpoint-verification-report.json'),
    timeoutMs: options.timeoutMs ?? 180_000, pollMs: options.pollMs, settleMs: options.settleMs,
  }, api)
  ensure(!samePath(config.report_path, receiptPath, api), 'receipt_is_not_an_output')
  return {
    ...config, receipt_path: receiptPath, cli, cli_sha256: cliSha256,
    reviewer, tokens, missing_artifact: options.missingArtifact === true, review_via_browser: true,
    runtime_binaries: publicBinaries,
    fixture_file: api.join(PIN.workspace, FIXTURE),
    application_fixture_file: api.join(PIN.workspace, 'scripts', 'checkpoint-application-fixture.mjs'),
    fake_github_cli: api.join(PIN.workspace, 'tools', 'fake_github_cli.mjs'),
  }
}

export function captureBaseline(state, replay, config) {
  const baseline = historySummary(state, replay, config)
  baseline.extra = Object.fromEntries(EXTRA_HISTORY.map((table) => {
    const rows = state.snapshot[table]
    ensure(Array.isArray(rows) && rows.length <= LIMITS.snapshot_rows, 'history_table_missing_or_bounded')
    return [table, rows.map((row) => ({ id: uuid(row.id ?? row.run_id), sha256: digest(row) }))]
  }))
  return baseline
}

export function assertBaseline(baseline, state, replay, config) {
  assertHistory(baseline, state, replay, config)
  for (const [table, rows] of Object.entries(baseline.extra)) {
    ensure(Array.isArray(state.snapshot[table]), 'history_table_missing')
    const current = new Map(state.snapshot[table].map((row) => [row.id ?? row.run_id, digest(row)]))
    ensure(rows.every((row) => current.get(row.id) === row.sha256), 'prior_evidence_or_factory_history_changed')
  }
}

/** Every replayed event is retained; opaque text/tokens NEVER cross into output. */
export function journalView(replay, config) {
  const summary = replaySummary(replay.events, replay.through, config.corp_id)
  return { ...summary, events: replay.events.map((event) => ({
    id: uuid(event.id), seq: event.seq, type: event.type,
    aggregate_id: uuid(event.aggregate_id),
    correlation_id: event.correlation_id == null ? null : uuid(event.correlation_id),
    room_id: event.room_id == null ? null : uuid(event.room_id),
    actor_id: event.actor_id == null ? null : uuid(event.actor_id),
    sha256: digest(event), payload_sha256: digest(event.payload), payload: publicEventPayload(event.payload),
  })) }
}

function publicEventPayload(payload) {
  const result = {}
  for (const key of ['run_id', 'task_id', 'mission_id', 'agent_id', 'source_run_id',
    'replacement_run_id', 'workspace_run_id', 'recovery_id', 'command_id', 'factory_work_item_id']) {
    if (UUID.test(payload?.[key])) result[key] = payload[key]
  }
  for (const key of ['attempt', 'max_attempts', 'input_tokens', 'output_tokens', 'cost_microusd',
    'check_index', 'check_count', 'bytes']) {
    if (Number.isSafeInteger(payload?.[key]) && payload[key] >= 0) result[key] = payload[key]
  }
  for (const key of ['provider_process_alive', 'mission_launch', 'branch_deleted', 'approved']) {
    if (typeof payload?.[key] === 'boolean') result[key] = payload[key]
  }
  const values = new Set(['codex', 'provider', 'verification_only', 'checkpoint_verification',
    'factory_verification_recovery', 'circuit_breaker', 'independent_review', 'pending', 'passed',
    'failed', 'approved', 'rejected', 'healthy', 'steer', 'constrain', 'suspend', 'stop',
    'completed', 'cancelled', 'file', 'command', 'artifact', 'commit_branch'])
  for (const key of ['adapter', 'execution_mode', 'mode', 'command_kind', 'gate_type', 'status',
    'stage', 'outcome', 'kind', 'form']) {
    if (values.has(payload?.[key])) result[key] = payload[key]
  }
  if (payload?.runner_id === PIN.runner) result.runner_id = PIN.runner
  const input = payload?.input
  if (['run_tokens', 'mission_tokens'].includes(input?.metric) &&
    Number.isSafeInteger(input.used) && Number.isSafeInteger(input.limit)) {
    result.input = { metric: input.metric, used: input.used, limit: input.limit }
  }
  return result
}

export function verifierPolicy(config) {
  const checks = [
    { type: 'file', path: 'base.txt', min_bytes: 5 },
    { type: 'command', program: 'node', args: ['-e',
      `const b=require('node:fs').readFileSync('base.txt');if(b.length!==5||require('node:crypto').createHash('sha256').update(b).digest('hex')!=='${BASE_SHA256}')process.exit(1)`,
    ], timeout_ms: 10_000 },
    { type: 'file', path: 'qa/issue195/index.html', min_bytes: 1 },
    { type: 'command', program: 'node', args: ['--test', 'qa/issue195/app.test.mjs'], timeout_ms: 10_000 },
  ]
  if (config.missing_artifact) checks.push({ type: 'artifact', min_bytes: 1 })
  return { checks, manual_gate: {
    type: 'independent_review', roles: ['owner', 'admin', 'manager', 'member'], exclude_requester: true,
  } }
}

const DELIVERABLE = Object.freeze({ form: 'commit_branch', commit_after_verification: true, paths: [] })
export function checkpointPolicyDigests(policy) {
  const declared = verifierPolicy({ missing_artifact: policy.checks?.length === 5 })
  equal(policy, declared, 'persisted_verification_policy_changed')
  // Rust serde declaration order, NOT canonical JSON/JSONB alphabetical order.
  return {
    verification_policy_sha256: hashBytes(JSON.stringify(declared)),
    write_scope_sha256: hashBytes(JSON.stringify(['**'])),
    deliverable_policy_sha256: hashBytes(JSON.stringify(DELIVERABLE)),
  }
}

export function buildPlan(config, driverId) {
  const suffix = uuid(driverId).replaceAll('-', '')
  const number = 195_000_000 + Number.parseInt(suffix.slice(-6), 16)
  const issue = {
    id: `I_checkpoint_${suffix}`, number,
    title: `[checkpoint-app] [budget-stream] Incident tracker ${driverId}`,
    body: [
      '[checkpoint-app] [budget-stream]',
      'Deterministic native protocol acceptance, not real vendor inference.',
      'Create the complete incident tracker in qa/issue195/index.html, qa/issue195/app.mjs,',
      'and qa/issue195/app.test.mjs, plus base.txt containing exactly UTF-8 base followed by LF.',
      '## Acceptance criteria',
      '- The incident tracker source and its native Node tests are present.',
      '- node --test qa/issue195/app.test.mjs passes through the persisted verifier.',
      '- Preserve the native budget-stopped physical checkpoint.',
      '- Do not access credentials, publish, merge, deploy, or change budget/verification policy.',
      'Independent review and typed source delivery are separate native gates.',
    ].join('\n'),
    url: `https://github.com/${PIN.repository}/issues/${number}`,
    state: 'OPEN', createdAt: '2026-09-08T00:00:00Z', updatedAt: '2026-09-08T00:00:00Z',
    labels: [{ name: 'factory:ready' }],
  }
  const itemId = `PVTI_checkpoint_${suffix}`
  return {
    state_path: path.join(config.output_dir, 'checkpoint-verification-github.json'),
    policy_path: path.join(config.output_dir, 'checkpoint-verification-policy.json'),
    issue_number: number, project_number: number, issue, item_id: itemId,
    policy: verifierPolicy(config),
    reason: `Explicit provider-free checkpoint verification ${driverId}; unchanged original source and policy.`,
    github_state: {
      repository: PIN.repository,
      project: { id: `PVT_checkpoint_${suffix}`, owner: 'shyamsridhar123', owner_type: 'User',
        number, status_field_id: `PVTSSF_checkpoint_${suffix}`,
        status_options: [{ id: 'todo', name: 'Todo' }, { id: 'progress', name: 'In Progress' },
          { id: 'review', name: 'In Review' }, { id: 'done', name: 'Done' }] },
      issues: { [number]: issue },
      items: [{ id: itemId, status: 'Todo', isArchived: false,
        content: { type: 'Issue', number, repository: PIN.repository, title: issue.title,
          body: issue.body, url: issue.url } }],
      item_edits: 0,
    },
  }
}

export function factoryArgs(config, plan, recovery = false) {
  const args = [
    '--server', PIN.server, 'factory', config.corp_id, config.actor_id,
    '--owner', 'shyamsridhar123', '--project-number', String(plan.project_number),
    '--repository', PIN.repository, '--source-repository-path', PIN.source,
    '--source-base-ref', config.source.base_ref, '--publication-base-ref', 'HEAD',
    '--adapter', 'codex', '--allow-adapter', 'codex', '--strategy', 'single',
    '--budget-tokens', String(config.tokens), '--budget-cost-microusd', '1000000',
    '--write-scope', '**', '--verification-policy-file', plan.policy_path,
    '--github-cli', process.execPath, '--issue', String(plan.issue_number),
  ]
  if (recovery) args.push('--verification-recovery', 'checkpoint-verification',
    '--verification-recovery-reason', plan.reason)
  return args
}

function bind(identity, field, value) {
  uuid(value)
  ensure(identity[field] === null || identity[field] === value, 'recorded_identity_changed')
  identity[field] = value
}

function ownedRows(state, config, plan, identity) {
  const s = state.snapshot
  const matches = s.factory_work_items.filter((item) => item.id === identity.work_item_id ||
    item.source_project_item_id === plan.item_id ||
    (item.source_project_number === plan.project_number && item.source_issue_number === plan.issue_number))
  ensure(matches.length <= 1, 'duplicate_or_colliding_factory_item')
  if (!matches.length) {
    ensure(!identity.work_item_id, 'recorded_factory_item_missing')
    return null
  }
  const item = matches[0]
  ensure(item.corp_id === config.corp_id && item.source_project_owner === 'shyamsridhar123' &&
    item.source_project_number === plan.project_number && item.source_project_item_id === plan.item_id &&
    item.source_repository_owner === 'shyamsridhar123' && item.source_repository_name === 'ecorp-enterprise-lab' &&
    item.source_issue_number === plan.issue_number && item.source_issue_node_id === plan.issue.id &&
    item.source_issue_url === plan.issue.url && item.source_revision === plan.issue.updatedAt &&
    item.source_title === plan.issue.title && item.claim_owner_id === config.actor_id,
  'factory_source_or_owner_changed')
  bind(identity, 'work_item_id', item.id)
  ensure(item.policy.source_base_commit === PIN.source_commit && item.policy.source_base_ref === 'HEAD' &&
    item.policy.budget_tokens === config.tokens && item.policy.budget_cost_microusd === 1_000_000 &&
    item.policy.auto_merge === false, 'factory_authority_changed')
  equal(item.policy.verification_policy, plan.policy, 'factory_verifier_policy_changed')
  equal(item.policy.write_scope, ['**'], 'factory_write_scope_changed')
  if (item.mission_id === null) {
    ensure(!identity.mission_id, 'recorded_mission_missing')
    return { item }
  }
  bind(identity, 'mission_id', item.mission_id)
  const mission = s.missions.find((row) => row.id === identity.mission_id)
  ensure(mission?.corp_id === config.corp_id && mission.requested_by === config.actor_id &&
    mission.title === `GitHub #${plan.issue_number}: ${plan.issue.title}` &&
    mission.description === plan.issue.body && mission.strategy === 'single' &&
    mission.specification_version === 1 && mission.budget_tokens === config.tokens &&
    mission.original_budget_tokens === config.tokens && mission.budget_cost_microusd === 1_000_000 &&
    mission.original_budget_cost_microusd === 1_000_000, 'mission_identity_or_authority_changed')
  const tasks = s.tasks.filter((task) => task.mission_id === mission.id)
  ensure(tasks.length === 1, 'one_original_task_required')
  const task = tasks[0]
  bind(identity, 'task_id', task.id)
  ensure(task.corp_id === config.corp_id && task.required_adapter === 'codex' &&
    task.max_attempts === 2 && [0, 1].includes(task.attempt_count) && task.contract_version === 1 &&
    task.depth === 0 && task.depends_on.length === 0, 'native_default_attempts_or_task_changed')
  equal(task.verification_policy, plan.policy, 'task_verifier_policy_changed')
  const contract = task.contract
  ensure(contract.source_repository === PIN.repository && contract.source_base_ref === 'HEAD' &&
    contract.source_base_commit === PIN.source_commit && contract.budget_tokens === config.tokens &&
    contract.budget_cost_microusd === 1_000_000 && contract.model === null &&
    contract.reasoning_effort === null, 'task_source_or_budget_changed')
  equal(contract.write_scope, ['**'], 'task_write_scope_changed')
  equal(contract.secret_refs, [], 'unexpected_secret_authority')
  equal(contract.deliverable, DELIVERABLE, 'native_factory_deliverable_policy_changed')
  const agent = s.agents.find((row) => row.id === task.assigned_agent_id)
  ensure(agent?.mission_id === mission.id && agent.adapter === 'codex' &&
    agent.actor_id !== config.reviewer, 'producer_or_reviewer_identity_invalid')
  const runs = s.runs.filter((run) => run.task_id === task.id)
  ensure(runs.length <= 2 && runs.every((run) => run.corp_id === config.corp_id &&
    run.runner_id === config.runner_id && run.agent_id === agent.id), 'unowned_or_extra_run')
  const providers = runs.filter((run) => run.execution_mode === 'provider')
  ensure(providers.length <= 1, 'new_provider_attempt_forbidden')
  if (providers[0]) bind(identity, 'original_run_id', providers[0].id)
  else ensure(!identity.original_run_id, 'recorded_original_run_missing')
  for (const table of ['mission_contract_revisions', 'mission_budget_revisions', 'action_approvals']) {
    ensure(!s[table].some((row) => row.mission_id === mission.id ||
      row.task_id === task.id || runs.some((run) => row.run_id === run.id)), 'unexpected_authority_revision')
  }
  return { item, mission, task, agent, runs, run: providers[0] }
}

function runEvents(replay, runId) { return replay.events.filter((event) => event.aggregate_id === runId) }
function oneEvent(events, type) {
  const matches = events.filter((event) => event.type === type)
  ensure(matches.length === 1, 'native_event_missing_or_duplicated')
  return matches[0]
}

export function assessOriginal(state, replay, config, plan, identity) {
  replaySummary(replay.events, replay.through, config.corp_id)
  const rows = ownedRows(state, config, plan, identity)
  ensure(rows?.run && rows.runs.length === 1, 'one_original_provider_run_required')
  const { run, task, mission, agent, item } = rows
  const stage = config.tokens === 5000 ? 'stop' : 'suspend'
  ensure(['failed', 'cancelled'].includes(run.status) && run.breaker_stage === stage &&
    run.workspace_disposition === 'preserved' && run.execution_mode === 'provider' &&
    run.workspace_run_id === run.id && run.resumed_from_run_id === null &&
    task.attempt_count === 1 && task.status === run.status && mission.status === run.status &&
    run.verification_status === 'pending' && task.verification_status === 'pending',
  'native_budget_terminal_checkpoint_required')
  ensure(run.source_repository === PIN.repository && run.source_base_ref === 'HEAD' &&
    run.source_base_commit === PIN.source_commit && run.model === null && run.reasoning_effort === null &&
    run.budget_tokens_limit === config.tokens && run.budget_cost_microusd_limit === 1_000_000 &&
    run.no_progress_events === 0 && run.repeated_tool_count === 0, 'original_authority_or_loop_counters_changed')
  ensure(agent.current_run_id === null && agent.status === 'idle', 'original_assignment_not_terminal')
  for (const field of ['artifact_id', 'artifact_uri', 'artifact_sha256', 'artifact_signature',
    'artifact_media_type', 'verification_sha256', 'deliverable_sha256']) {
    ensure(run[field] === null, 'original_has_unexpected_accepted_evidence')
  }
  for (const table of ['verification_evidence', 'verification_requests', 'source_deliverables']) {
    ensure(!state.snapshot[table].some((row) => row.run_id === run.id), 'original_verified_before_budget_stop')
  }
  const events = runEvents(replay, run.id)
  ensure(events.every((event) => event.correlation_id === mission.id && event.room_id === mission.room_id),
    'original_journal_scope_changed')
  ensure(!events.some((event) => ['run.completed', 'run.workspace_removed', 'run.teardown_uncertain',
    'run.artifact', 'run.artifact_upload', 'run.deliverable', 'run.deliverable_upload'].includes(event.type) ||
    event.type.startsWith('run.verification_')), 'unexpected_original_completion_or_evidence')
  const requested = oneEvent(events, 'run.requested')
  ensure(requested.actor_id === config.actor_id && requested.payload.mission_launch === true &&
    requested.payload.task_id === task.id && requested.payload.agent_id === agent.id &&
    requested.payload.runner_id === config.runner_id && requested.payload.attempt === 1 &&
    requested.payload.max_attempts === 2, 'original_native_launch_mismatch')
  const started = oneEvent(events, 'run.started')
  ensure(started.payload.adapter === 'codex', 'original_not_native_codex')
  const session = oneEvent(events, 'run.session')
  ensure(uuid(session.payload.session_id) === run.provider_session_id, 'original_provider_session_mismatch')
  const usage = events.filter((event) => event.type === 'run.usage')
  ensure(usage.length === 2 && usage.every((event) => event.payload.input_tokens === 3000 &&
    event.payload.output_tokens === 0 && event.payload.cost_microusd === 0) &&
    run.input_tokens === 6000 && run.output_tokens === 0 && run.cost_microusd === 0,
  'native_fixture_usage_not_exact')
  const stages = ['healthy', 'steer', 'constrain', 'suspend', 'stop']
  let rank = 0
  const transitions = events.filter((event) => event.type === 'run.breaker_transition')
  for (const event of transitions) {
    const next = stages.indexOf(event.payload.stage)
    ensure(next > rank, 'breaker_history_not_monotonic')
    rank = next
  }
  const hard = transitions.filter((event) => ['stop', 'suspend'].includes(event.payload.stage))
  ensure(hard.length === 1 && hard[0].payload.stage === stage, 'hard_budget_incident_missing')
  const breaker = hard[0]
  ensure(['run_tokens', 'mission_tokens'].includes(breaker.payload.input?.metric) &&
    breaker.payload.input.used === 6000 && breaker.payload.input.limit === config.tokens, 'wrong_budget_incident')
  const ack = events.filter((event) => event.type === 'runner.command_acknowledged' &&
    event.payload.command_id === breaker.payload.command_id && event.payload.command_kind === 'circuit_breaker')
  ensure(ack.length === 1 && ack[0].payload.runner_id === config.runner_id &&
    ack[0].seq > breaker.seq, 'native_budget_command_not_acknowledged')
  const incidents = state.snapshot.circuit_breaker_incidents.filter((row) => row.run_id === run.id)
  ensure(incidents.filter((row) => row.stage === stage && row.task_id === task.id &&
    row.mission_id === mission.id && canonical(row.input) === canonical(breaker.payload.input)).length === 1,
  'persisted_budget_incident_missing')
  const terminated = oneEvent(events, 'run.session_terminated')
  ensure(terminated.payload.adapter === 'codex' && terminated.payload.provider_process_alive === false &&
    ['completed', 'cancelled', 'failed'].includes(terminated.payload.outcome), 'native_provider_termination_missing')
  const preserved = oneEvent(events, 'run.workspace_preserved')
  const terminal = oneEvent(events, `run.${run.status}`)
  ensure(requested.seq < started.seq && started.seq < usage[0].seq && usage[1].seq < breaker.seq &&
    breaker.seq < terminated.seq && terminated.seq < preserved.seq && preserved.seq < terminal.seq,
  'native_checkpoint_event_order_invalid')
  const branch = `crony/task-${task.id.replaceAll('-', '')}/run-${run.id.replaceAll('-', '')}`
  const checkpoint = {
    schema_version: 1, corp_id: config.corp_id, mission_id: mission.id, task_id: task.id,
    run_id: run.id, workspace_run_id: run.id, agent_id: agent.id, runner_id: config.runner_id,
    source_repository: PIN.repository, source_base_ref: 'HEAD', source_base_commit: PIN.source_commit,
    workspace_base_commit: PIN.source_commit, branch, head_commit: PIN.source_commit,
    workspace_fingerprint: sha(run.workspace_fingerprint), ...checkpointPolicyDigests(task.verification_policy),
  }
  equal(preserved.payload.source_checkpoint, checkpoint, 'physical_checkpoint_source_or_policy_mismatch')
  ensure(run.workspace_branch === branch && run.workspace_base_commit === PIN.source_commit &&
    run.workspace_base_ref === 'HEAD' &&
    samePath(run.workspace_path, expectedWorkspace(config, task.id, run.id)), 'original_workspace_lineage_mismatch')
  for (const event of [started, preserved]) {
    ensure(samePath(event.payload.workspace, run.workspace_path) &&
      event.payload.workspace_branch === branch && event.payload.workspace_base_ref === 'HEAD' &&
      event.payload.workspace_base_commit === PIN.source_commit, 'native_workspace_event_mismatch')
  }
  ensure(preserved.payload.workspace_fingerprint === checkpoint.workspace_fingerprint &&
    preserved.payload.head_commit === PIN.source_commit && preserved.payload.branch_deleted === false,
  'checkpoint_preservation_not_exact')
  return {
    run_sha256: digest(run), run_events_sha256: digest(events), run_event_count: events.length,
    contract_sha256: digest(task.contract), factory_policy_sha256: digest(item.policy),
    incidents_sha256: digest(incidents), source_checkpoint: checkpoint,
    provider_session_sha256: hashBytes(run.provider_session_id), breaker_stage: stage,
    counters: { attempt_count: 1, max_attempts: 2, input_tokens: 6000, output_tokens: 0,
      cost_microusd: 0, no_progress_events: 0, repeated_tool_count: 0 },
    accepted_provider_artifacts: 0, ordering: {
      requested: requested.seq, started: started.seq, usage: usage.map((event) => event.seq),
      breaker: breaker.seq, acknowledged: ack[0].seq, terminated: terminated.seq,
      checkpoint: preserved.seq, terminal: terminal.seq,
    },
  }
}

function assertOriginalUnchanged(rows, replay, state, original) {
  ensure(rows?.run && digest(rows.run) === original.run_sha256 &&
    digest(runEvents(replay, rows.run.id)) === original.run_events_sha256,
  'original_run_or_complete_history_changed')
  ensure(rows.task.attempt_count === 1 && rows.task.max_attempts === 2 &&
    digest(rows.task.contract) === original.contract_sha256 &&
    digest(rows.item.policy) === original.factory_policy_sha256 &&
    digest(state.snapshot.circuit_breaker_incidents.filter((row) => row.run_id === rows.run.id)) ===
      original.incidents_sha256, 'original_policy_attempts_or_incidents_changed')
}

export function assessRecovery(state, replay, context, config, plan, original, identity) {
  replaySummary(replay.events, replay.through, config.corp_id)
  const rows = ownedRows(state, config, plan, identity)
  assertOriginalUnchanged(rows, replay, state, original)
  ensure(context?.checkpoint_verification === true && context.source_run_id === identity.original_run_id &&
    context.task_id === identity.task_id && context.mission_id === identity.mission_id &&
    (context.work_item_id ?? context.work_item?.id) === identity.work_item_id &&
    context.expected_head_commit === PIN.source_commit &&
    context.workspace_fingerprint === original.source_checkpoint.workspace_fingerprint &&
    context.remaining_mission_tokens === 0 && context.remaining_attempts === 1,
  'native_exhausted_checkpoint_admission_missing')
  const recoveries = state.snapshot.factory_verification_recoveries
    .filter((row) => row.factory_work_item_id === identity.work_item_id)
  ensure(recoveries.length === 1, 'one_exact_native_recovery_required')
  const recovery = recoveries[0]
  bind(identity, 'recovery_id', recovery.id)
  ensure(recovery.corp_id === config.corp_id && recovery.source_run_id === identity.original_run_id &&
    recovery.mission_id === identity.mission_id && recovery.task_id === identity.task_id &&
    recovery.mode === 'checkpoint_verification' && recovery.authorized_by === config.actor_id &&
    recovery.reason === plan.reason && recovery.contract_revision_id === null &&
    recovery.observed_source_revision === plan.issue.updatedAt, 'native_recovery_lineage_or_policy_changed')
  equal(recovery.previous_verification_policy, plan.policy, 'recovery_previous_policy_changed')
  equal(recovery.replacement_verification_policy, plan.policy, 'recovery_replacement_policy_changed')
  bind(identity, 'replacement_run_id', recovery.replacement_run_id)
  const run = rows.runs.find((candidate) => candidate.id === identity.replacement_run_id)
  ensure(rows.runs.length === 2 && run && run.id !== rows.run.id, 'replacement_missing_or_extra_run')
    ensure(run.execution_mode === 'verification_only' && run.resumed_from_run_id === rows.run.id &&
    run.workspace_run_id === rows.run.workspace_run_id && run.provider_session_id === null &&
    run.model === null && run.reasoning_effort === null &&
    run.source_repository === PIN.repository && run.source_base_ref === 'HEAD' &&
    run.source_base_commit === PIN.source_commit &&
    run.workspace_base_commit === PIN.source_commit && run.workspace_base_ref === 'HEAD' &&
    run.workspace_branch === rows.run.workspace_branch &&
    samePath(run.workspace_path, rows.run.workspace_path) && run.workspace_disposition === 'preserved' &&
    run.workspace_fingerprint === original.source_checkpoint.workspace_fingerprint,
  'not_exact_provider_free_workspace_lineage')
  for (const field of ['budget_tokens_limit', 'budget_cost_microusd_limit', 'input_tokens',
    'output_tokens', 'cost_microusd', 'no_progress_events', 'repeated_tool_count']) {
    ensure(run[field] === 0, 'verifier_allocated_or_consumed_model_budget')
  }
  ensure(run.breaker_stage === 'healthy' && run.artifact_id === null &&
    run.artifact_sha256 === null, 'replacement_provider_artifact_or_breaker')
  const events = runEvents(replay, run.id)
  ensure(events.every((event) => event.correlation_id === rows.mission.id &&
    event.room_id === rows.mission.room_id), 'replacement_journal_scope_changed')
  ensure(!events.some((event) => ['run.session', 'run.session_terminated', 'run.output', 'run.usage',
    'run.artifact', 'run.artifact_upload', 'run.resume_requested', 'run.workspace_removed',
    'run.teardown_uncertain', 'run.breaker_transition'].includes(event.type)),
  'replacement_provider_events_or_unsafe_cleanup')
  const requested = oneEvent(events, 'run.verification_requested')
  ensure(requested.payload.recovery_id === recovery.id && requested.payload.source_run_id === rows.run.id &&
    requested.payload.workspace_run_id === rows.run.workspace_run_id && requested.payload.task_id === rows.task.id &&
    requested.payload.agent_id === rows.agent.id && requested.payload.runner_id === config.runner_id &&
    requested.payload.attempt === 1 && requested.payload.max_attempts === 2 &&
    requested.payload.execution_mode === 'verification_only', 'native_verification_request_mismatch')
  const started = oneEvent(events, 'run.started')
  const verifying = oneEvent(events, 'run.verification_started')
  ensure(started.payload.execution_mode === 'verification_only' &&
    samePath(started.payload.workspace, rows.run.workspace_path) &&
    verifying.payload.check_count === plan.policy.checks.length &&
    requested.seq < started.seq && started.seq < verifying.seq, 'native_verify_run_not_observed')
  const ack = events.filter((event) => event.type === 'runner.command_acknowledged' &&
    event.payload.command_kind === 'factory_verification_recovery' &&
    event.payload.runner_id === config.runner_id)
  ensure(ack.length === 1 && UUID.test(ack[0].payload.command_id) &&
    ack[0].seq > requested.seq, 'native_verify_run_command_not_acknowledged')
  const evidence = state.snapshot.verification_evidence.filter((row) => row.run_id === run.id)
    .sort((a, b) => a.check_index - b.check_index)
  const evidenceEvents = events.filter((event) => event.type === 'run.verification_evidence')
  ensure(evidence.length === plan.policy.checks.length && evidenceEvents.length === evidence.length &&
    evidence.every((row, index) => row.corp_id === config.corp_id && row.task_id === rows.task.id &&
      row.check_index === index && row.kind === plan.policy.checks[index].type &&
      evidenceEvents.filter((event) => event.payload.check_index === index &&
        event.payload.kind === row.kind && event.payload.status === row.status && event.seq > verifying.seq).length === 1),
  'persisted_native_check_evidence_missing_or_mismatched')
  const common = {
    run_id: run.id, recovery_id: recovery.id, source_run_id: rows.run.id,
    execution_mode: 'verification_only', original_counters_and_history_unchanged: true,
    new_provider_sessions: 0, new_provider_outputs: 0, new_provider_usage_events: 0,
    model_token_allocation: 0, model_cost_allocation: 0, input_tokens: 0, output_tokens: 0, cost_microusd: 0,
    provider_artifact_bytes_claimed: false, command_id: ack[0].payload.command_id,
    policy_sha256: checkpointPolicyDigests(plan.policy).verification_policy_sha256,
    checks: evidence.map((row) => ({ id: uuid(row.id), check_index: row.check_index,
      kind: row.kind, status: row.status, payload_sha256: digest(row.payload) })),
    ordering: { requested: requested.seq, started: started.seq, verifying: verifying.seq,
      acknowledged: ack[0].seq, evidence: evidenceEvents.map((event) => event.seq) },
    application_evidence: 'native File and Node command checks only; browser UI not tested by this driver',
    publication: 'not attempted; no PR, merge, or deployment acceptance',
  }
  const deliverables = state.snapshot.source_deliverables.filter((row) => row.run_id === run.id)
  const gates = state.snapshot.verification_requests.filter((row) => row.run_id === run.id)
  if (config.missing_artifact) {
    ensure(evidence.slice(0, -1).every((row) => row.status === 'passed') &&
      evidence.at(-1).kind === 'artifact' && evidence.at(-1).status === 'failed' &&
      run.status === 'failed' && run.verification_status === 'failed' && recovery.status === 'failed' &&
      rows.item.state === 'verification_failed' && rows.task.status === 'verification_failed' &&
      rows.mission.status === 'failed' && gates.length === 0 && deliverables.length === 0,
    'negative_did_not_fail_only_for_missing_required_artifact')
    oneEvent(events, 'run.verification_failed')
    ensure(!events.some((event) => ['run.verification_passed', 'run.verification_waiting',
      'run.completed', 'run.deliverable'].includes(event.type)), 'negative_accepted_missing_artifact')
    return { ...common, missing_required_provider_artifact_rejected: true, verified: false }
  }
  ensure(evidence.every((row) => row.status === 'passed'), 'native_automated_verification_failed')
  if (['failed', 'cancelled', 'lost'].includes(run.status)) {
    const errors = events.filter((event) => event.type === 'run.failed').map((event) => event.payload.error)
    ensure(false, errors.some((value) => typeof value === 'string' &&
      value.includes('verifier-only deliverable tree changed from preserved head'))
      ? 'native_deliverable_export_rejects_uncommitted_checkpoint_app'
      : 'native_replacement_failed_before_deliverable_and_review')
  }
  const stored = oneEvent(events, 'run.deliverable')
  const passed = oneEvent(events, 'run.verification_passed')
  const waiting = oneEvent(events, 'run.verification_waiting')
  ensure(Math.max(...evidenceEvents.map((event) => event.seq)) < stored.seq &&
    stored.seq < passed.seq && passed.seq < waiting.seq &&
    waiting.payload.gate_type === 'independent_review', 'native_deliverable_verification_gate_order')
  ensure(deliverables.length === 1 && gates.length === 1, 'one_deliverable_and_independent_gate_required')
  const deliverable = deliverables[0]
  const gate = gates[0]
  equal(gate.gate, plan.policy.manual_gate, 'persisted_independent_gate_changed')
  ensure(deliverable.corp_id === config.corp_id && deliverable.task_id === rows.task.id &&
    deliverable.form === 'commit_branch' && deliverable.bytes > 0 &&
    deliverable.sha256 === run.deliverable_sha256 && SHA.test(deliverable.sha256) &&
    deliverable.verification_sha256 === run.verification_sha256 && SHA.test(run.verification_sha256) &&
    deliverable.base_commit === PIN.source_commit && deliverable.branch === rows.run.workspace_branch &&
    /^[0-9a-f]{40,64}$/u.test(deliverable.head_commit) &&
    typeof deliverable.provenance_signature === 'string' && deliverable.provenance_signature.length > 0,
  'native_deliverable_provenance_mismatch')
  const verified = gate.status === 'approved'
  if (verified) {
    const approved = oneEvent(events, 'verification.approved')
    const factoryVerified = replay.events.filter((event) => event.type === 'factory.verified' &&
      event.aggregate_id === rows.item.id && event.payload.run_id === run.id)
    ensure(gate.decided_by === config.reviewer && approved.actor_id === config.reviewer &&
      approved.payload.gate_type === 'independent_review' && approved.seq > waiting.seq &&
      factoryVerified.length === 1 && factoryVerified[0].seq > waiting.seq &&
      run.status === 'completed' && run.verification_status === 'passed' &&
      rows.task.status === 'completed' && rows.mission.status === 'completed' &&
      rows.item.state === 'verified' && recovery.status === 'completed',
    'independent_review_and_factory_completion_not_proven')
    common.ordering.approved = approved.seq
    common.ordering.factory_verified = factoryVerified[0].seq
  } else ensure(gate.status === 'pending' && gate.decided_by === null &&
    run.status === 'waiting_for_approval' && rows.task.status === 'awaiting_approval' &&
    rows.item.state === 'awaiting_approval', 'not_waiting_for_native_independent_review')
  return { ...common, verified, deliverable: {
    id: uuid(deliverable.id), artifact_id: uuid(deliverable.artifact_id), bytes: deliverable.bytes,
    sha256: deliverable.sha256, verification_sha256: deliverable.verification_sha256,
    base_commit: deliverable.base_commit, head_commit: deliverable.head_commit,
    form: deliverable.form, branch: deliverable.branch,
    provenance_signature_sha256: hashBytes(deliverable.provenance_signature),
  } }
}

export function verifyDownload(bytes, deliverable) {
  ensure(Buffer.isBuffer(bytes) && bytes.length > 0 && bytes.length <= LIMITS.response_bytes &&
    bytes.length === deliverable.bytes && hashBytes(bytes) === deliverable.sha256, 'deliverable_download_digest_mismatch')
  let document
  try { document = JSON.parse(bytes.toString('utf8')) } catch { throw new CheckFailure('deliverable_json_invalid') }
  ensure(document.schema_version === 1 && document.form === 'commit_branch' &&
    document.base_commit === PIN.source_commit && document.head_commit === deliverable.head_commit &&
    document.branch === deliverable.branch && document.verification_sha256 === deliverable.verification_sha256 &&
    Array.isArray(document.changes) && document.changes.length > 0 && document.changes.length <= 4,
  'download_not_native_application_deliverable')
  const allowed = ['base.txt', 'qa/issue195/index.html', 'qa/issue195/app.mjs', 'qa/issue195/app.test.mjs']
  const files = document.changes.map((change) => {
    ensure(allowed.includes(change.path) && typeof change.content_base64 === 'string', 'unexpected_download_change')
    const content = Buffer.from(change.content_base64, 'base64')
    ensure(content.length > 0 && content.length === change.bytes && hashBytes(content) === change.sha256,
      'download_application_file_digest_mismatch')
    return { path: change.path, bytes: content.length, sha256: change.sha256 }
  })
  ensure(new Set(files.map((file) => file.path)).size === files.length &&
    allowed.slice(1).every((file) => files.some((row) => row.path === file)), 'complete_application_download_required')
  for (const kind of ['patch', 'git_bundle']) {
    ensure(typeof document[`${kind}_base64`] === 'string', 'native_download_payload_missing')
    const payload = Buffer.from(document[`${kind}_base64`], 'base64')
    ensure(payload.length > 0 && hashBytes(payload) === document[`${kind}_sha256`], 'native_download_payload_invalid')
  }
  return { bytes: bytes.length, sha256: deliverable.sha256, artifact_id: deliverable.artifact_id,
    files, native_node_checks_passed: true, browser_tested: false, published: false }
}

export function newReport(config, driverId = randomUUID()) {
  return {
    schema_version: 1, suite: SUITE, driver_id: uuid(driverId),
    binding_sha256: receiptIdentity(config), status: 'incomplete', passed: false,
    target: { server_url: PIN.server, repository: PIN.repository, source_path: PIN.source,
      source_commit: PIN.source_commit, code_commit: PIN.code_commit, runner_id: PIN.runner },
    inputs: { receipt_id: config.receipt_id, cli_sha256: config.cli_sha256,
      server_sha256: config.runtime_binaries.server.sha256, runner_sha256: config.runtime_binaries.runner.sha256,
      fixture_sha256: config.provider.sha256, application_fixture_sha256: config.application_fixture_sha256 ?? null,
      fake_github_sha256: config.fake_github_sha256 ?? null },
    baseline: null, journal: null, original: null, replacement: null, download: null, pending_review: null,
    identity: { work_item_id: null, mission_id: null, task_id: null, original_run_id: null,
      replacement_run_id: null, recovery_id: null },
    intents: { create: null, recovery: null },
    observations: [], failures: [],
    coverage: {
      principal: 'parent-owned development QA; existing independent human reviewer, not OIDC',
      provider: 'deterministic native codex budget-stream; no real vendor inference',
      history: 'actor-visible rows and complete journal through recorded Ready watermark',
      journal: 'one record per event; raw payload/text/tokens withheld, full payload and event SHA256 retained',
      ownership: 'parent supervisor receipt and local binary digests; no process inspection or service management',
      review: 'parent browser action required; driver never posts a decision. Native events prove actor/gate, not input device.',
      claim: 'checkpoint recovery acceptance only, not finished issue148 enterprise application proof',
    },
  }
}

export function validateSavedReport(report, config) {
  ensure(report?.schema_version === 1 && report.suite === SUITE &&
    report.binding_sha256 === receiptIdentity(config), 'saved_report_binding_mismatch')
  uuid(report.driver_id)
  equal(Object.keys(report).sort(), Object.keys(newReport(config, report.driver_id)).sort(),
    'saved_report_shape_mismatch')
  equal(Object.keys(report.identity).sort(), Object.keys(newReport(config).identity).sort(), 'saved_identity_shape')
  for (const value of Object.values(report.identity)) if (value !== null) uuid(value)
  equal(Object.keys(report.intents).sort(), ['create', 'recovery'], 'saved_intent_shape')
  for (const value of Object.values(report.intents)) {
    if (value !== null) ensure(SHA.test(value.sha256) && Number.isSafeInteger(value.at),
      'saved_intent_invalid')
  }
  ensure(Array.isArray(report.observations) && Array.isArray(report.failures), 'saved_history_invalid')
  ensure(report.baseline || !Object.values(report.intents).some(Boolean), 'mutation_without_saved_baseline')
  ensure(!report.identity.replacement_run_id || report.intents.recovery, 'replacement_without_intent')
  ensure(!report.identity.original_run_id || report.intents.create, 'original_without_intent')
  if (report.pending_review) {
    ensure(report.pending_review.run_id === report.identity.replacement_run_id &&
      report.pending_review.recovery_id === report.identity.recovery_id &&
      report.pending_review.reviewer_actor_id === config.reviewer, 'saved_browser_review_identity_changed')
  }
  return report
}

async function canonicalDirectory(directory) {
  const declared = localAbsolute(directory)
  const info = await fs.lstat(declared)
  ensure(info.isDirectory() && !info.isSymbolicLink() &&
    samePath(declared, await fs.realpath(declared)), 'canonical_owned_directory_required')
  return declared
}

async function exists(file) {
  try { await fs.lstat(file); return true } catch (error) {
    if (error.code === 'ENOENT') return false
    throw error
  }
}

async function checkReadTarget(file, trustedExecutable) {
  if (!trustedExecutable) return checkContainedFile(path.dirname(file), file)
  // Cargo and Git for Windows legitimately hard-link installed executables.
  // These targets are only hashed/read, never edited; source/checkpoint files
  // retain their separate single-link containment rule.
  await canonicalDirectory(path.dirname(file))
  const info = await fs.lstat(file)
  ensure(info.isFile() && !info.isSymbolicLink() && info.nlink >= 1 &&
    samePath(file, await fs.realpath(file)), 'canonical_executable_required')
}

/** Bound reads and compare opened identity and before/after metadata. */
async function readBounded(file, bound, digestOnly = false, trustedExecutable = false) {
  ensure(!trustedExecutable || digestOnly, 'executable_reads_are_digest_only')
  await checkReadTarget(file, trustedExecutable)
  const before = await fs.lstat(file)
  ensure(before.size <= bound, 'file_byte_bound')
  const handle = await fs.open(file, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0))
  try {
    const opened = await handle.stat()
    ensure(opened.isFile() && (trustedExecutable ? opened.nlink >= 1 : opened.nlink === 1) && opened.ino === before.ino &&
      opened.dev === before.dev && opened.size === before.size, 'opened_file_changed')
    const hash = createHash('sha256')
    const chunks = []
    let size = 0
    const buffer = Buffer.alloc(Math.min(bound + 1, 1024 * 1024))
    while (true) {
      const { bytesRead } = await handle.read(buffer, 0, Math.min(buffer.length, bound + 1 - size), size)
      if (!bytesRead) break
      size += bytesRead
      ensure(size <= bound, 'file_byte_bound')
      hash.update(buffer.subarray(0, bytesRead))
      if (!digestOnly) chunks.push(Buffer.from(buffer.subarray(0, bytesRead)))
    }
    const after = await handle.stat()
    ensure(size === opened.size && after.size === size && after.mtimeMs === opened.mtimeMs &&
      after.nlink === opened.nlink, 'file_changed_while_reading')
    await checkReadTarget(file, trustedExecutable)
    return digestOnly ? hash.digest('hex') : Buffer.concat(chunks)
  } finally { await handle.close() }
}

export function readTrustedExecutableDigest(file) {
  ensure(path.extname(file).toLowerCase() === '.exe', 'explicit_executable_required')
  return readBounded(file, 512 * 1024 * 1024, true, true)
}

async function writeNew(file, bytes) {
  await canonicalDirectory(path.dirname(file))
  const handle = await fs.open(file, 'wx', 0o600)
  try { await handle.writeFile(bytes); await handle.sync() } finally { await handle.close() }
}

export async function invokeFactory(config, plan, recovery, timeoutMs, execute = execFile) {
  const args = factoryArgs(config, plan, recovery)
  return new Promise((resolve, reject) => {
    execute(config.cli, args, {
      cwd: config.output_dir, windowsHide: true, shell: false,
      timeout: Math.max(1, Math.min(timeoutMs, 60_000)), maxBuffer: 2 * 1024 * 1024,
      // Do not inherit HOME, GitHub/provider credentials, proxies, NODE_OPTIONS,
      // DB settings, or the parent's environment. Node is needed by the native
      // CLI's .mjs GitHub-bin dispatch, not a provider process.
      env: { PATH: `${path.dirname(process.execPath)};${path.win32.dirname(PIN.git)}`,
        ECORP_FAKE_GITHUB_STATE: plan.state_path,
        ECORP_GITHUB_CLI_PREFIX_ARGS_JSON: JSON.stringify([config.fake_github_cli]) },
    }, (error, stdout, stderr) => {
      if (error) {
        const failure = new CheckFailure('factory_cli_failed_or_uncertain_do_not_retry')
        failure.cli = { stdout_sha256: hashBytes(stdout ?? ''), stderr_sha256: hashBytes(stderr ?? ''),
          exit_code: Number.isInteger(error.code) ? error.code : null }
        return reject(failure)
      }
      try {
        const value = JSON.parse(stdout)
        ensure(value && typeof value === 'object' && !Array.isArray(value), 'factory_json_object_required')
        resolve(value) // In-memory only; call sites project exact validated IDs.
      } catch { reject(new CheckFailure('factory_response_invalid_do_not_retry')) }
    })
  })
}

function assertReviewer(state, config) {
  const reviewer = state.snapshot.actors.find((actor) => actor.id === config.reviewer)
  ensure(reviewer?.corp_id === config.corp_id && reviewer.kind === 'human' &&
    ['owner', 'admin', 'manager', 'member'].includes(reviewer.role) && reviewer.id !== config.actor_id,
  'existing_independent_reviewer_required')
}

function admissionView(context, report) {
  ensure(context.work_item?.id === report.identity.work_item_id &&
    context.mission_id === report.identity.mission_id && context.task_id === report.identity.task_id &&
    context.source_run_id === report.identity.original_run_id && context.checkpoint_verification === true &&
    context.expected_head_commit === PIN.source_commit &&
    context.workspace_fingerprint === report.original.source_checkpoint.workspace_fingerprint &&
    context.remaining_attempts === 1 && context.remaining_mission_tokens === 0 &&
    context.remaining_mission_cost_microusd === 1_000_000 &&
    Array.isArray(context.recoveries) && context.recoveries.length === 0,
  'exact_native_checkpoint_context_required')
  return {
    work_item_id: context.work_item.id, mission_id: context.mission_id, task_id: context.task_id,
    source_run_id: context.source_run_id, checkpoint_verification: true,
    expected_head_commit: context.expected_head_commit, workspace_fingerprint: context.workspace_fingerprint,
    remaining_attempts: context.remaining_attempts, remaining_mission_tokens: context.remaining_mission_tokens,
    remaining_mission_cost_microusd: context.remaining_mission_cost_microusd,
  }
}

/** All effects are injectable. Tests use ONLY memory, never a fake service. */
export async function executeSuite(config, report, io, { continuation = false } = {}) {
  validateSavedReport(report, config)
  const plan = buildPlan(config, report.driver_id)
  const deadline = io.now() + config.timeout_ms
  let phase = 'preflight'
  let latest
  const save = () => io.save(report)
  const remaining = () => {
    ensure(io.now() < deadline, 'acceptance_deadline_no_replacement')
    return deadline - io.now()
  }
  async function observe(label) {
    phase = label
    remaining()
    const state = await io.snapshot()
    const replay = await io.replay()
    report.journal = journalView(replay, config)
    // Persist the complete journal BEFORE a subsequent safety assertion can fail.
    await save()
    if (report.baseline) assertBaseline(report.baseline, state, replay, config)
    const rows = ownedRows(state, config, plan, report.identity)
    if (rows) ensure(report.intents.create, 'unowned_synthetic_issue_collision')
    const recoveries = state.snapshot.factory_verification_recoveries
      .filter((row) => row.factory_work_item_id === report.identity.work_item_id)
    ensure(recoveries.length <= 1, 'duplicate_recovery')
    if (recoveries[0]) {
      ensure(report.intents.recovery, 'recovery_without_recorded_intent')
      bind(report.identity, 'recovery_id', recoveries[0].id)
      if (recoveries[0].replacement_run_id) bind(report.identity, 'replacement_run_id', recoveries[0].replacement_run_id)
    }
    if (rows?.runs?.length > 1) ensure(report.intents.recovery, 'unrequested_replacement_run')
    if (report.original) assertOriginalUnchanged(rows, replay, state, report.original)
    const replacement = rows?.runs?.find((row) => row.id === report.identity.replacement_run_id)
    const status = {
      phase: label, work_item_id: report.identity.work_item_id, mission_id: report.identity.mission_id,
      original_run_id: report.identity.original_run_id, replacement_run_id: report.identity.replacement_run_id,
      original_status: rows?.run ? (PUBLIC_STATES.has(rows.run.status) ? rows.run.status : 'unrecognized') : null,
      replacement_status: replacement ? (PUBLIC_STATES.has(replacement.status) ? replacement.status : 'unrecognized') : null,
      factory_state: rows?.item ? (PUBLIC_STATES.has(rows.item.state) ? rows.item.state : 'unrecognized') : null,
      native_failure_payload_sha256: replay.events.filter((event) => event.type === 'run.failed' &&
        event.aggregate_id === report.identity.replacement_run_id).map((event) => digest(event.payload)),
    }
    // Hash entire native rows rather than copying summaries, text or tokens.
    if (canonical(report.observations.at(-1)?.state) !== canonical(status)) {
      ensure(report.observations.length < 2000, 'observation_count_bound')
      report.observations.push({ at: io.now(), through: replay.through, state: status })
    }
    latest = { state, replay, rows, replacement }
    await save()
    return latest
  }
  async function poll(label, predicate) {
    while (true) {
      const current = await observe(label)
      if (predicate(current)) return current
      remaining()
      await io.sleep(Math.min(config.poll_ms, remaining()))
    }
  }
  async function quiet(label) {
    ensure(remaining() > config.settle_ms, 'insufficient_quiet_observation_budget')
    await io.sleep(config.settle_ms)
    return observe(label)
  }
  async function mutate(name, request, effect) {
    ensure(!continuation, 'continue_is_observation_only')
    ensure(report.intents[name] === null, 'recorded_mutation_is_never_retried')
    await io.checkInputs?.()
    report.intents[name] = { sha256: digest(request), at: io.now(),
      journal_through: report.journal.through }
    phase = `${name}_intent`
    await save()
    // An error here is ambiguous. No catch-and-retry, fallback, or new identity.
    return effect()
  }
  function recordCli(value, recovery) {
    bind(report.identity, 'work_item_id', value.factory_work_item_id)
    bind(report.identity, 'mission_id', value.mission_id)
    bind(report.identity, recovery ? 'replacement_run_id' : 'original_run_id', value.launch?.run_id)
    if (recovery) {
      ensure(value.launch.verification_recovery === true &&
        value.launch.recovery_mode === 'checkpoint_verification', 'wrong_native_recovery_response')
      bind(report.identity, 'recovery_id', value.launch.recovery_id)
    }
  }
  report.status = 'incomplete'
  report.passed = false
  try {
    const initial = await observe('preflight')
    assertRunner(initial.state, config)
    assertReviewer(initial.state, config)
    if (!report.baseline) {
      ensure(!continuation && !initial.rows, 'new_baseline_or_fresh_item_required')
      report.baseline = captureBaseline(initial.state, initial.replay, config)
      await save()
    }
    if (!report.intents.create) {
      const result = await mutate('create', factoryArgs(config, plan, false),
        () => io.factory(false, remaining()))
      recordCli(result, false)
      await save()
    }
    if (!report.original) {
      await poll('original_checkpoint', ({ rows }) => rows?.run &&
        ['failed', 'cancelled'].includes(rows.run.status) && rows.run.workspace_disposition === 'preserved')
      const current = await quiet('original_quiet')
      report.original = assessOriginal(current.state, current.replay, config, plan, report.identity)
      report.original.base_file = await io.readBase({
        task_id: report.identity.task_id, run_id: report.identity.original_run_id,
      })
      ensure(report.original.base_file.bytes === 5 && report.original.base_file.sha256 === BASE_SHA256,
        'physical_base_bytes_not_proven')
      await save()
    }
    if (!report.original.admission) {
      // A completed recovery has no failed-task context endpoint; never query
      // that route after completion or replace the already retained admission.
      ensure(!report.intents.recovery, 'recorded_admission_missing_do_not_reconstruct_authority')
      report.original.admission = admissionView(await io.context(report.identity.work_item_id), report)
      await save()
    }
    if (!report.intents.recovery) {
      const result = await mutate('recovery', {
        args: factoryArgs(config, plan, true), original_run_id: report.identity.original_run_id,
        source_checkpoint: report.original.source_checkpoint,
      }, () => io.factory(true, remaining()))
      recordCli(result, true)
      await save()
    }
    await poll('native_verifier', ({ replacement }) => replacement &&
      (TERMINAL.has(replacement.status) || replacement.status === 'waiting_for_approval'))
    let current = await quiet('native_verifier_quiet')
    let result = assessRecovery(current.state, current.replay, report.original.admission,
      config, plan, report.original, report.identity)
    report.replacement = result
    await save()
    if (!config.missing_artifact && !result.verified) {
      assertReviewer(current.state, config)
      report.pending_review = {
        corp_id: config.corp_id, work_item_id: report.identity.work_item_id,
        mission_id: report.identity.mission_id, task_id: report.identity.task_id,
        run_id: report.identity.replacement_run_id, recovery_id: report.identity.recovery_id,
        reviewer_actor_id: config.reviewer, gate_type: 'independent_review', status: 'pending',
        evidence_ids: result.checks.map((check) => check.id),
        verification_sha256: result.deliverable.verification_sha256,
        instruction: 'Parent: use Bob Accept evidence for this exact run in the native browser. Then invoke the same driver arguments with --continue.',
      }
      report.status = 'awaiting_browser_review'
      report.passed = false
      await save()
      return report
    }
    if (report.pending_review && result.verified) report.pending_review.status = 'approved'
    if (!config.missing_artifact) {
      ensure(result.verified, 'review_not_verified')
      const bytes = await io.download(result.deliverable)
      const download = verifyDownload(bytes, result.deliverable)
      await io.saveDownload(bytes)
      report.download = download
      await save()
    }
    const final = await observe('final')
    report.replacement = assessRecovery(final.state, final.replay, report.original.admission,
      config, plan, report.original, report.identity)
    const base = await io.readBase({ task_id: report.identity.task_id, run_id: report.identity.original_run_id })
    equal(base, report.original.base_file, 'original_physical_base_file_changed')
    report.status = config.missing_artifact ? 'expected_missing_artifact_rejection' : 'verified_and_downloaded'
    report.passed = true
    await save()
    return report
  } catch (error) {
    const failedPhase = phase
    // Best effort, READ ONLY reconciliation after a lost response. It records
    // exact discovered IDs and journal without issuing another native operation.
    if (io.now() < deadline) {
      try { await observe('failure_reconciliation') } catch { /* Preserve the prior bounded journal. */ }
    }
    report.passed = false
    report.status = 'failed'
    report.failures.push({ at: io.now(), phase: failedPhase, code: safeError(error),
      journal_through: report.journal?.through ?? null,
      ...(error instanceof CheckFailure && error.cli ? { cli: error.cli } : {}) })
    await save()
    throw new CheckFailure(safeError(error))
  }
}

/** Read-only pinned local API. There is deliberately NO decision/approval POST. */
export function createRuntimeIo(config, plan, {
  fetchImpl = globalThis.fetch, WebSocketImpl = globalThis.WebSocket, now = Date.now,
  execute = execFile,
} = {}) {
  ensure(config.server_url === PIN.server, 'runtime_origin_not_authorized')
  const native = createApi(config, { fetchImpl, WebSocketImpl, now })
  const deadline = now() + config.timeout_ms
  let requests = 0
  const invocations = new Set()
  const base = `/api/corps/${uuid(config.corp_id)}`
  async function request(route, binary = false) {
    ensure(++requests <= LIMITS.requests && now() < deadline, 'runtime_request_bound')
    const response = await fetchImpl(`${PIN.server}${route}`, {
      method: 'GET', redirect: 'error',
      headers: { 'content-type': 'application/json' },
      signal: AbortSignal.timeout(Math.max(1, Math.min(LIMITS.request_ms, deadline - now()))),
    })
    if (response.status !== 200 || !response.body) {
      await response.body?.cancel().catch(() => {})
      throw new CheckFailure('native_api_not_ok_body_withheld')
    }
    const reader = response.body.getReader()
    const chunks = []
    let size = 0
    try {
      while (true) {
        const { done, value } = await reader.read()
        if (done) break
        size += value.length
        ensure(size <= LIMITS.response_bytes, 'native_response_byte_bound')
        chunks.push(value)
      }
    } finally { await reader.cancel().catch(() => {}) }
    const bytes = Buffer.concat(chunks)
    if (binary) return bytes
    try { return JSON.parse(bytes.toString('utf8')) } catch { throw new CheckFailure('native_response_json_invalid') }
  }
  return {
    snapshot: native.snapshot, replay: native.replay, now,
    sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
    readBase: (checkpoint) => readBaseFile(config, checkpoint),
    context: (workItemId) => request(
      `${base}/factory/work-items/${uuid(workItemId)}/verification-recoveries?actor_id=${config.actor_id}`),
    download: (deliverable) => request(
      `${base}/artifacts/${uuid(deliverable.artifact_id)}?actor_id=${config.actor_id}`, true),
    factory: (recovery, timeoutMs) => {
      ensure(typeof recovery === 'boolean' && !invocations.has(recovery), 'native_cli_not_retried')
      invocations.add(recovery)
      return invokeFactory(config, plan, recovery, timeoutMs, execute)
    },
  }
}

async function verifyInputs(config, plan) {
  await canonicalDirectory(config.output_dir)
  await canonicalDirectory(config.runner_root)
  ensure(await readTrustedExecutableDigest(config.cli) === config.cli_sha256, 'candidate_cli_digest_mismatch')
  ensure(await readBounded(config.fixture_file, 4 * 1024 * 1024, true) === config.provider.sha256,
    'current_fixture_digest_mismatch')
  const appHash = await readBounded(config.application_fixture_file, 4 * 1024 * 1024, true)
  if (config.application_fixture_sha256) ensure(appHash === config.application_fixture_sha256, 'application_fixture_changed')
  else config.application_fixture_sha256 = appHash
  for (const binary of Object.values(config.runtime_binaries)) {
    ensure(await readBounded(binary.path, 512 * 1024 * 1024, true) === binary.sha256,
      'public_runtime_binary_digest_mismatch')
  }
  const gitHash = await readTrustedExecutableDigest(PIN.git)
  if (config.git_sha256) ensure(gitHash === config.git_sha256, 'installed_git_changed')
  else config.git_sha256 = gitHash
  const fakeHash = await readBounded(config.fake_github_cli, 4 * 1024 * 1024, true)
  if (config.fake_github_sha256) ensure(fakeHash === config.fake_github_sha256, 'fake_github_program_changed')
  else config.fake_github_sha256 = fakeHash
  if (plan) {
    equal(JSON.parse((await readBounded(plan.policy_path, LIMITS.receipt_bytes)).toString('utf8')),
      plan.policy, 'owned_policy_file_changed')
    const state = JSON.parse((await readBounded(plan.state_path, LIMITS.response_bytes)).toString('utf8'))
    ensure(state.repository === PIN.repository && state.project.id === plan.github_state.project.id &&
      state.project.owner === 'shyamsridhar123' && state.project.number === plan.project_number &&
      state.items.length === 1 && state.items[0].id === plan.item_id, 'new_fake_github_state_identity_changed')
    equal(state.issues, plan.github_state.issues, 'new_fake_issue_revision_changed')
    equal(state.items[0].content, plan.github_state.items[0].content, 'new_fake_item_content_changed')
  }
}

export async function main(argv = process.argv.slice(2)) {
  let reportPath = null
  let lock = null
  try {
    const options = parseArgs(argv)
    if (options.help) {
      console.log('Required: --receipt <absolute issue195 runtime.json> --output-dir <existing owned runtime subdirectory> --cli <absolute candidate.exe> --fixture-sha256 <current SHA256> --reviewer-actor-id <independent human UUID>. Optional: --cli-sha256 <explicit rebuilt candidate SHA256>, --tokens 5000|6000, --timeout-ms, --poll-ms, --settle-ms, --missing-required-artifact, --review-via-browser, --continue (observation only). Review always pauses for the parent browser, never a decision POST. Never start or reset the runtime with this driver.')
      return 0
    }
    ensure(process.platform === 'win32', 'this_driver_targets_the_explicit_windows_qa_only')
    const config = configuration(JSON.parse((await readBounded(localAbsolute(options.receiptPath),
      LIMITS.receipt_bytes)).toString('utf8')), options)
    await verifyInputs(config)
    reportPath = config.report_path
    const lockPath = `${reportPath}.lock`
    lock = { path: lockPath, handle: await fs.open(lockPath, 'wx', 0o600) }
    await lock.handle.writeFile(`${JSON.stringify({ suite: SUITE, receipt_id: config.receipt_id })}\n`)
    await lock.handle.sync()
    const priorExists = await exists(reportPath)
    ensure(priorExists === (options.continuation === true),
      priorExists ? 'existing_report_requires_explicit_continue' : 'continuation_report_missing')
    let report
    if (priorExists) {
      const bytes = await readBounded(reportPath, LIMITS.report_bytes)
      report = validateSavedReport(JSON.parse(bytes.toString('utf8')), config)
      // Preserve failed/successful prior reports before ANY observation refresh.
      await writeNew(`${reportPath}.previous-${randomUUID()}.json`, bytes)
    } else report = newReport(config)
    async function save(value) {
      const bytes = Buffer.from(`${JSON.stringify(value, null, 2)}\n`)
      ensure(bytes.length <= LIMITS.report_bytes, 'report_byte_bound')
      if (await exists(reportPath)) await readBounded(reportPath, LIMITS.report_bytes)
      const temp = `${reportPath}.${randomUUID()}.tmp`
      await writeNew(temp, bytes)
      await canonicalDirectory(config.output_dir)
      await fs.rename(temp, reportPath)
    }
    const plan = buildPlan(config, report.driver_id)
    await save(report)
    if (!priorExists) {
      // Never reuse another driver/fixture's fake GitHub state or policy file.
      ensure(!(await exists(plan.state_path)) && !(await exists(plan.policy_path)), 'fixture_output_collision')
      await writeNew(plan.policy_path, `${JSON.stringify(plan.policy, null, 2)}\n`)
      await writeNew(plan.state_path, `${JSON.stringify(plan.github_state, null, 2)}\n`)
    }
    await verifyInputs(config, plan)
    const io = createRuntimeIo(config, plan)
    await executeSuite(config, report, {
      ...io, save, checkInputs: () => verifyInputs(config, plan),
      saveDownload: async (bytes) => {
        const target = path.join(config.output_dir, 'checkpoint-verification-deliverable.json')
        if (await exists(target)) {
          equal(hashBytes(await readBounded(target, LIMITS.response_bytes)), hashBytes(bytes),
            'retained_download_changed_do_not_overwrite')
        } else await writeNew(target, bytes)
      },
    }, { continuation: options.continuation === true })
    console.log(JSON.stringify({ suite: SUITE, passed: report.passed, report_path: reportPath,
      status: report.status, identity: report.identity, pending_review: report.pending_review }))
    return 0
  } catch (error) {
    console.error(JSON.stringify({ suite: SUITE, passed: false, report_path: reportPath,
      error_code: safeError(error), instruction: 'Preserve all work and failure reports. No retries or replacement creation. --continue observes exact recorded work only.' }))
    return 1
  } finally {
    if (lock) {
      await lock.handle.close()
      await fs.unlink(lock.path)
    }
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  process.exitCode = await main()
}
