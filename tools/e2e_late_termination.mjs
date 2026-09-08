#!/usr/bin/env node
/**
 * Additive #193 native-runtime probe; importing this module is inert.
 *
 * OFFLINE development: node --test tools/e2e_late_termination.test.mjs
 * Parent-only execution (Node >= 22, native fetch/WebSocket):
 *   node tools/e2e_late_termination.mjs --schema
 *   node tools/e2e_late_termination.mjs --receipt <absolute ownership JSON>
 *   node tools/e2e_late_termination.mjs --receipt <same JSON> --continue
 *
 * Reuses #190's supported receipt, scoped client, complete Ready-watermarked
 * journal, history, policy and contained base.txt helpers. The existing issue190
 * runtime.json is accepted unchanged with its explicit driver-only CLI options:
 *   --source-repository <exact synthetic enterprise-lab routing label>
 *   --fixture scripts/fake-codex-app-server.mjs
 *   --fixture-sha256 <parent-computed CURRENT fixture SHA256>
 *   --output-dir <existing owned evidence directory>
 *   --report-path <new absolute e2e-late-termination.json in that directory>
 * Optional --timeout-ms / --poll-ms / --settle-ms use #190's bounds.
 * Its owner_task remains the receipt identity. Its original source path (the
 * parent's issue174-startup fixture), code commit and process identities remain
 * bound by #190's runtime_binding_sha256; none of those paths is followed here.
 *
 * Only fixture18574 is allowed, with the EXISTING issue174-local-start runner
 * and original a8894b5... source commit. Port 15574/browser is not accessed.
 * Parent owns all services/resources and attests the deterministic executable.
 * No bootstrap/reset, SQL, policy/credential/environment access, enrollment,
 * resume, replacement mission, service/process/browser/provider launch or cleanup.
 * Exactly one new single mission may be created and launched. Neither POST is
 * retried; intent is saved/printed first and returned IDs saved immediately.
 * --continue observes the same case, or launches its never-launched held plan.
 * An ambiguous intent without discoverable IDs fails closed.
 *
 * [budget-queued-completion] writes base.txt then emits 3000 + 3000 usage and
 * completed synchronously. #190 skips uploads only AFTER received hard control;
 * it cannot retract an already-allowed upload. This is a race CANDIDATE, not a
 * guarantee. No controls, ACKs or artifact bytes are intercepted/delayed/retagged.
 * PASS requires actual journal order: usage -> stop -> artifact-rejection
 * run.failed -> valid native run.session_terminated. Actual command ACK sequence
 * is recorded, not prescribed relative to failure/termination. A missed race,
 * missing telemetry or observation bound exits nonzero and retains the case.
 *
 * Terminal statuses, failure summary, authority, attempts and original journal
 * are protected. Native workspace finalization and worker retirement may finish
 * normally; their fields must settle too. Snapshots are NOT atomic with the
 * failure transaction: this probe cannot prove a physical before/after DB diff.
 * Evidence is actor-visible native server/runner behavior with a deterministic
 * Codex protocol, NOT vendor session persistence or full #148 recovery coverage.
 *
 * Report lifecycle matches #190: bounded single-link reads, canonical existing
 * directories, exclusive .lock, fsynced temporary file + atomic replacement,
 * preserved previous report on --continue, no stale-lock stealing or orphan
 * cleanup. Parent must establish driver death before removing its exact lock.
 * Raw snapshots, payloads, provider output and native error details are withheld.
 */

import { randomUUID } from 'node:crypto'
import { constants } from 'node:fs'
import * as fs from 'node:fs/promises'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import {
  BASE_SHA256, FIXTURE, LIMITS, CheckFailure, assertHistory, assertRunner,
  canonical, caseRequest as stoppedRequest, checkContainedFile,
  configurationFromReceipt, createApi, digest, expectedWorkspace, historySummary,
  localAbsolute, parseArgs, policyDigests, readBaseFile, receiptIdentity,
  receiptSchema, replaySummary,
} from './e2e_stopped_source_checkpoint.mjs'

export const SUITE = 'late-termination-193'
export const MARKER = '[budget-queued-completion]'
export const SOURCE_COMMIT = 'a8894b5f02d56f10e2da38df47a450ff71e92fbe'
const UUID = /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/u
const SHA256 = /^[0-9a-f]{64}$/u
const LIVE = new Set(['provisioning', 'starting', 'running', 'waiting_for_input', 'waiting_for_approval', 'verifying'])
const TERMINAL = new Set(['completed', 'failed', 'cancelled', 'lost'])
const STAGES = ['healthy', 'steer', 'constrain', 'suspend', 'stop']
const EMPTY_ROWS = { missions: [], tasks: [], runs: [] }
const ABSENCE_TABLES = ['verification_evidence', 'verification_requests', 'source_deliverables']
const WAITABLE = new Set(['native_termination_not_observed', 'hard_command_ack_not_observed',
  'workspace_preservation_not_observed'])

function requireThat(value, code) { if (!value) throw new CheckFailure(code) }
function exact(actual, expected, code) { requireThat(canonical(actual) === canonical(expected), code) }
function uuid(value) { requireThat(typeof value === 'string' && UUID.test(value), 'invalid_uuid'); return value }
function hash(value) { requireThat(typeof value === 'string' && SHA256.test(value), 'invalid_sha256'); return value }
function keys(value, allowed, code) {
  requireThat(value && typeof value === 'object' && !Array.isArray(value) &&
    Object.keys(value).length === allowed.length && allowed.every(key => Object.hasOwn(value, key)), code)
}
function safeCode(error) { return error instanceof CheckFailure ? error.code : 'operation_failed_details_withheld' }
function samePath(left, right) { return path.relative(localAbsolute(left), localAbsolute(right)) === '' }
function without(value, fields) { return Object.fromEntries(Object.entries(value).filter(([key]) => !fields.includes(key))) }

export function probeConfiguration(receipt, options = {}, pathApi = path) {
  const config = configurationFromReceipt(receipt, options, pathApi)
  requireThat(config.server_url === 'http://127.0.0.1:18574' &&
    config.runner_id === 'issue174-local-start' && config.source.base_commit === SOURCE_COMMIT,
  'original_fixture18574_runner_and_source_required')
  return config
}

export function probeRequest(config) {
  const base = stoppedRequest(config, 'stop')
  return {
    ...base,
    title: `${MARKER} late-termination ${config.receipt_id}`,
    description: 'Deterministic native Codex queued-completion probe. Preserve the original failed run and source while recording later native provider termination; no recovery or accepted completion.',
    contract: {
      ...base.contract,
      objective: `${MARKER} Write only base.txt containing exactly base followed by LF, then emit the queued-completion fixture sequence.`,
      expected_output: 'Original failed source and native late-termination accounting, not an accepted deliverable.',
      acceptance_tests: ['base.txt contains exactly five bytes: base followed by LF',
        'native artifact-rejection run.failed precedes native run.session_terminated without changing terminal state'],
    },
  }
}

export function createProbeApi(config, dependencies) {
  const api = createApi(config, dependencies)
  return { ...api, create: () => api.request(`/api/corps/${config.corp_id}/missions`, probeRequest(config)) }
}

export function newReport(config, now = Date.now()) {
  return {
    schema_version: 1, suite: SUITE, receipt_id: config.receipt_id, receipt_sha256: receiptIdentity(config),
    started_at: new Date(now).toISOString(), updated_at: new Date(now).toISOString(),
    status: 'incomplete', passed: false, race_observed: false, baseline: null, error_code: null,
    case: {
      budget_tokens: 5_000, request_sha256: digest(probeRequest(config)),
      create_attempted: false, launch_attempted: false,
      mission_id: null, task_id: null, agent_id: null, run_id: null, contract_sha256: null,
      phase: 'new', latest: null, terminal_baseline: null, journal_prefix: null,
      failure_event: null, journal_observation: null, evidence: null,
    },
    coverage: {
      provider: 'native server/runner with deterministic Codex protocol; not real vendor persistence or full #148',
      ownership: 'explicit parent receipt; no process/executable/source-checkout inspection',
      history: 'original actor-visible mission/task/run/agent rows and complete visible journal prefix only',
      terminal_state: 'failure summary/status/identity/authority and observed terminal projection; not an atomic before/after database diff',
      allowed_finalization: 'native workspace metadata/updated_at and mission-worker retirement; all must settle',
      ordering: 'server journal sequence only; no provider-frame timing or command-ACK timing assumption',
      absence: 'accepted records/events only through recorded Ready watermark and quiet interval',
      local_reads: 'bounded receipt/report and exact contained base.txt only; no artifact or credential bytes',
      resource_lifetime: 'parent-owned; timeout never stops, cancels, relaunches or reclassifies a live run',
    },
  }
}

export function caseContext(state, config, checkpoint) {
  // Reuse #190's bounded snapshot/tenant validation, without asserting a new baseline.
  assertHistory({ rows: EMPTY_ROWS }, state, null, config)
  const snap = state.snapshot
  const request = probeRequest(config)
  const matches = snap.missions.filter(row => row.title === request.title)
  requireThat(matches.length <= 1, 'duplicate_case_mission')
  if (checkpoint.mission_id) requireThat(matches[0]?.id === checkpoint.mission_id, 'original_case_mission_missing')
  if (!matches.length) return null
  requireThat(checkpoint.create_attempted, 'unowned_case_title_collision')
  const mission = matches[0]
  checkpoint.mission_id = uuid(mission.id)
  requireThat(mission.requested_by === config.actor_id && mission.description === request.description &&
    mission.strategy === 'single' && mission.specification_version === 1 &&
    mission.budget_tokens === request.budget_tokens && mission.original_budget_tokens === request.budget_tokens &&
    mission.budget_cost_microusd === request.budget_cost_microusd &&
    mission.original_budget_cost_microusd === request.budget_cost_microusd, 'mission_authority_changed')
  const tasks = snap.tasks.filter(row => row.mission_id === mission.id)
  requireThat(tasks.length === 1, 'one_original_task_required')
  const task = tasks[0]
  requireThat(!checkpoint.task_id || checkpoint.task_id === task.id, 'original_task_replaced')
  checkpoint.task_id = uuid(task.id)
  requireThat(task.required_adapter === 'codex' && task.max_attempts === 2 && task.depth === 0 &&
    task.depends_on.length === 0 && task.contract_version === 1, 'native_single_task_changed')
  const contract = task.contract
  requireThat(contract.source_repository === config.source.repository &&
    contract.source_base_ref === config.source.base_ref && contract.source_base_commit === config.source.base_commit &&
    contract.budget_tokens === request.budget_tokens && contract.budget_cost_microusd === request.budget_cost_microusd &&
    contract.model === null && contract.reasoning_effort === null, 'task_source_or_budget_changed')
  exact(contract.secret_refs, [], 'unexpected_secret_authority')
  exact(contract.allowed_tools, request.contract.allowed_tools, 'unexpected_tool_authority')
  requireThat(contract.objective === `${request.description}\n\nTASK-SPECIFIC OBJECTIVE:\n${request.contract.objective}` &&
    contract.expected_output === request.contract.expected_output &&
    ['acceptance_tests', 'prohibited_actions', 'references'].every(key =>
      request.contract[key].every(item => contract[key].includes(item))), 'native_contract_overlay_changed')
  exact(task.verification_policy, request.verification_policy, 'persisted_verifier_changed')
  policyDigests(task.verification_policy, contract.write_scope, contract.deliverable)
  if (checkpoint.contract_sha256) exact(digest(contract), checkpoint.contract_sha256, 'original_contract_changed')
  else checkpoint.contract_sha256 = digest(contract)
  const agent = snap.agents.find(row => row.id === task.assigned_agent_id)
  requireThat(agent?.adapter === 'codex' && agent.mission_id === mission.id, 'mission_owned_native_agent_required')
  requireThat(!checkpoint.agent_id || checkpoint.agent_id === agent.id, 'original_agent_replaced')
  checkpoint.agent_id = uuid(agent.id)
  const runs = snap.runs.filter(row => row.task_id === task.id)
  requireThat(runs.length <= 1, 'automatic_retry_or_duplicate_run')
  const run = runs[0] ?? null
  if (checkpoint.run_id) requireThat(run?.id === checkpoint.run_id, 'original_run_missing_or_replaced')
  if (run) {
    requireThat(checkpoint.launch_attempted, 'run_without_launch_intent')
    checkpoint.run_id = uuid(run.id)
    requireThat(LIVE.has(run.status) || TERMINAL.has(run.status), 'unknown_run_state')
    requireThat(STAGES.includes(run.breaker_stage), 'unknown_breaker_stage')
    checkpoint.latest = {
      status: run.status, breaker_stage: run.breaker_stage,
      task_failed: task.status === 'failed', mission_failed: mission.status === 'failed',
      agent_idle: agent.status === 'idle', agent_has_current_run: agent.current_run_id !== null,
      summary_sha256: digest(run.summary),
    }
    requireThat(run.agent_id === agent.id && run.runner_id === config.runner_id &&
      run.workspace_run_id === run.id && run.resumed_from_run_id === null &&
      run.source_repository === config.source.repository && run.source_base_ref === config.source.base_ref &&
      run.source_base_commit === config.source.base_commit && run.execution_mode === 'provider' &&
      run.model === null && run.reasoning_effort === null &&
      run.budget_tokens_limit === request.budget_tokens &&
      run.budget_cost_microusd_limit === request.budget_cost_microusd && task.attempt_count === 1,
    'original_run_identity_or_authority_changed')
  }
  return { snap, mission, task, agent, run }
}

function terminalProjection({ run, task, mission, agent }) {
  return {
    // Workspace bookkeeping may legitimately arrive after logical failure.
    run: digest(without(run, ['workspace_disposition', 'workspace_detail', 'workspace_fingerprint', 'updated_at'])),
    task: digest(task), mission: digest(mission),
    agent: digest(without(agent, ['retired_at', 'updated_at'])),
  }
}

function protectTerminal(context, checkpoint, now) {
  if (!TERMINAL.has(context.run.status)) {
    requireThat(checkpoint.terminal_baseline === null, 'terminal_run_became_live_again')
    return
  }
  const projection = terminalProjection(context)
  if (checkpoint.terminal_baseline) exact(projection, checkpoint.terminal_baseline.digests, 'terminal_projection_changed')
  else checkpoint.terminal_baseline = { first_observed_at: new Date(now).toISOString(), digests: projection }
}

function assertOriginalHistory(baseline, state, replay, config) {
  assertHistory(baseline, state, replay, config)
  for (const old of baseline.agents) {
    const current = state.snapshot.agents.find(row => row.id === old.id)
    requireThat(current && digest(current) === old.sha256, 'original_agent_history_changed')
  }
}

export function observeJournal(replay, checkpoint, config) {
  replaySummary(replay.events, replay.through, config.corp_id)
  const events = replay.events.filter(event => event.aggregate_id === checkpoint.run_id)
  const failed = events.find(event => event.type === 'run.failed')
  const terminated = events.find(event => event.type === 'run.session_terminated')
  checkpoint.journal_observation = {
    through: replay.through,
    events: events.map(event => ({ id: event.id, seq: event.seq, type: event.type, payload_sha256: digest(event.payload) })),
    command_acknowledgments: events.filter(event => event.type === 'runner.command_acknowledged').map(event => ({
      id: event.id, seq: event.seq,
      command_id: UUID.test(event.payload?.command_id ?? '') ? event.payload.command_id : null,
      native_breaker_ack: event.payload?.command_kind === 'circuit_breaker' && event.payload?.runner_id === config.runner_id,
      before_failed: failed ? event.seq < failed.seq : null,
      before_session_terminated: terminated ? event.seq < terminated.seq : null,
    })),
  }
  if (failed) {
    const identity = { id: failed.id, seq: failed.seq, sha256: digest(failed) }
    if (checkpoint.failure_event) exact(identity, checkpoint.failure_event, 'original_failure_event_changed')
    else checkpoint.failure_event = identity
    if (!checkpoint.journal_prefix) checkpoint.journal_prefix = replaySummary(replay.events, replay.through, config.corp_id)
  }
}

export function assessCase(state, replay, config, checkpoint) {
  const context = caseContext(state, config, checkpoint)
  requireThat(context?.run, 'original_run_not_visible')
  const { snap, mission, task, agent, run } = context
  replaySummary(replay.events, replay.through, config.corp_id)
  const events = replay.events.filter(event => event.aggregate_id === run.id)
  requireThat(events.every(event => event.aggregate_type === 'run' && event.correlation_id === mission.id &&
    event.room_id === mission.room_id), 'run_event_lineage_mismatch')
  requireThat(run.status !== 'completed', 'accepted_completion_present')
  const forbidden = new Set(['run.completed', 'run.artifact', 'run.artifact_upload', 'run.deliverable',
    'run.deliverable_upload', 'run.workspace_removed', 'run.teardown_uncertain'])
  requireThat(!events.some(event => forbidden.has(event.type) || event.type.startsWith('run.verification_')),
    'accepted_progress_or_uncertain_teardown')
  requireThat(run.verification_status === 'pending' && task.verification_status === 'pending', 'verification_state_changed')
  for (const key of ['artifact_id', 'artifact_uri', 'artifact_sha256', 'artifact_signature', 'artifact_media_type',
    'verification_sha256', 'verification_summary', 'deliverable_sha256']) {
    requireThat(run[key] === null, 'accepted_artifact_or_verification_present')
  }
  for (const table of [...ABSENCE_TABLES, 'mission_contract_revisions', 'mission_budget_revisions', 'action_approvals']) {
    requireThat(!snap[table].some(row => row.run_id === run.id || row.task_id === task.id ||
      row.mission_id === mission.id), 'accepted_evidence_or_changed_authority')
  }
  const one = (type, missing = 'required_native_event_missing') => {
    const matches = events.filter(event => event.type === type)
    requireThat(matches.length > 0, missing)
    requireThat(matches.length === 1, 'duplicate_native_event')
    return matches[0]
  }
  const failed = one('run.failed', 'race_not_observed_artifact_rejection_missing')
  requireThat(typeof failed.payload?.error === 'string' &&
    failed.payload.error.startsWith('artifact upload rejected: '), 'race_not_observed_failure_not_artifact_rejection')
  requireThat(run.status === 'failed' && task.status === 'failed' && mission.status === 'failed' &&
    agent.status === 'idle' && agent.current_run_id === null && agent.station === null &&
    run.summary === failed.payload.error, 'original_failed_projection_not_preserved')
  requireThat(events.filter(event => ['run.failed', 'run.cancelled', 'run.lost'].includes(event.type)).length === 1,
    'terminal_history_changed')
  const requested = one('run.requested')
  requireThat(requested.actor_id === config.actor_id && requested.payload.mission_launch === true &&
    requested.payload.task_id === task.id && requested.payload.agent_id === agent.id &&
    requested.payload.runner_id === config.runner_id && requested.payload.attempt === 1 &&
    requested.payload.max_attempts === 2, 'native_launch_receipt_mismatch')
  const started = one('run.started')
  requireThat(started.payload.adapter === 'codex' && started.payload.mission_id === mission.id &&
    started.payload.task_id === task.id && started.payload.room_id === mission.room_id, 'native_start_receipt_mismatch')
  const session = one('run.session')
  uuid(session.payload.session_id)
  requireThat(session.payload.session_id === run.provider_session_id, 'provider_session_mismatch')
  const usage = events.filter(event => event.type === 'run.usage')
  requireThat(usage.length === 2 && usage.every(event => event.payload.input_tokens === 3_000 &&
    event.payload.output_tokens === 0 && event.payload.cost_microusd === 0) &&
    run.input_tokens === 6_000 && run.output_tokens === 0 && run.cost_microusd === 0, 'fixture_usage_not_exact')
  const transitions = events.filter(event => event.type === 'run.breaker_transition')
  let stage = 0
  for (const event of transitions) {
    const next = STAGES.indexOf(event.payload.stage)
    requireThat(next > stage, 'nonmonotonic_breaker_history')
    stage = next
  }
  const hard = transitions.filter(event => ['suspend', 'stop'].includes(event.payload.stage))
  requireThat(hard.length === 1 && hard[0].payload.stage === 'stop' && run.breaker_stage === 'stop', 'native_hard_stop_missing')
  const breaker = hard[0]
  uuid(breaker.payload.command_id)
  requireThat(['run_tokens', 'mission_tokens'].includes(breaker.payload.input?.metric) &&
    breaker.payload.input.used === 6_000 && breaker.payload.input.limit === 5_000, 'wrong_breaker_authority')
  const incident = snap.circuit_breaker_incidents.filter(row => row.run_id === run.id && row.stage === 'stop')
  requireThat(incident.length === 1 && incident[0].mission_id === mission.id &&
    incident[0].task_id === task.id, 'native_stop_incident_missing')
  exact(incident[0].input, breaker.payload.input, 'native_stop_incident_changed')
  const terminated = one('run.session_terminated', 'native_termination_not_observed')
  const payload = terminated.payload
  requireThat(payload && typeof payload === 'object' && !Array.isArray(payload) &&
    Object.keys(payload).every(key => ['adapter', 'outcome', 'provider_process_alive', 'message'].includes(key)) &&
    payload.adapter === 'codex' && payload.provider_process_alive === false &&
    ['completed', 'cancelled', 'failed', 'runtime_error'].includes(payload.outcome) &&
    (!Object.hasOwn(payload, 'message') || (typeof payload.message === 'string' && payload.message.trim() &&
      Buffer.byteLength(payload.message) <= 1024 && !/[\u0000-\u001f\u007f-\u009f]/u.test(payload.message))),
  'invalid_native_termination_payload')
  for (const event of [started, session, ...usage, failed, terminated]) {
    requireThat(event.actor_id === null &&
      event.idempotency_key === `runner:${config.runner_id}:event:${event.id}`, 'native_runner_event_binding_mismatch')
  }
  requireThat(requested.seq < started.seq && started.seq < usage[0].seq && session.seq < usage[0].seq &&
    usage[1].seq < breaker.seq && breaker.seq < failed.seq, 'native_usage_stop_failure_order_not_proven')
  requireThat(failed.seq < terminated.seq, 'race_not_observed_termination_preceded_failure')
  const acks = events.filter(event => event.type === 'runner.command_acknowledged' &&
    event.payload.command_id === breaker.payload.command_id)
  requireThat(acks.length > 0, 'hard_command_ack_not_observed')
  requireThat(acks.length === 1 && acks[0].payload.command_kind === 'circuit_breaker' &&
    acks[0].payload.runner_id === config.runner_id && acks[0].seq > breaker.seq, 'native_hard_command_ack_invalid')
  const preserved = one('run.workspace_preserved', 'workspace_preservation_not_observed')
  requireThat(terminated.seq < preserved.seq && run.workspace_disposition === 'preserved' &&
    preserved.payload.workspace_fingerprint === hash(run.workspace_fingerprint) &&
    preserved.payload.branch_deleted === false, 'original_workspace_not_preserved')
  const branch = `crony/task-${task.id.replaceAll('-', '')}/run-${run.id.replaceAll('-', '')}`
  requireThat(samePath(run.workspace_path, expectedWorkspace(config, task.id, run.id)) &&
    run.workspace_branch === branch && run.workspace_base_ref === config.source.base_ref &&
    run.workspace_base_commit === config.source.base_commit, 'original_workspace_identity_changed')
  for (const receipt of [started.payload, preserved.payload]) {
    requireThat(samePath(receipt.workspace, run.workspace_path) && receipt.workspace_branch === branch &&
      receipt.workspace_base_ref === config.source.base_ref &&
      receipt.workspace_base_commit === config.source.base_commit, 'native_workspace_receipt_mismatch')
  }
  return {
    ordering: { requested: requested.seq, started: started.seq, usage: usage.map(event => event.seq),
      stop: breaker.seq, failed: failed.seq, session_terminated: terminated.seq,
      command_acknowledged: acks[0].seq, workspace_preserved: preserved.seq },
    command_id: breaker.payload.command_id, failure_event_id: failed.id, failure_payload_sha256: digest(failed.payload),
    termination_event_id: terminated.id, termination_payload_sha256: digest(payload), provider_outcome: payload.outcome,
    journal_through: replay.through, terminal_projection: terminalProjection(context),
    persisted_usage: { input_tokens: 6_000, output_tokens: 0, cost_microusd: 0 },
    terminal_state: { run: 'failed', task: 'failed', mission: 'failed', agent: 'idle', current_run_id: null },
    task_run_count: 1, attempt_count: 1, accepted_artifacts: 0, accepted_completion: false,
    base_file: null, quiet_observation_ms: 0,
  }
}

export async function executeProbe(config, report, io) {
  const checkpoint = report.case
  const deadline = io.now() + config.timeout_ms
  const save = async () => { report.updated_at = new Date(io.now()).toISOString(); await io.save(report) }
  const inspect = async () => {
    const state = await io.snapshot()
    assertOriginalHistory(report.baseline, state, null, config)
    const context = caseContext(state, config, checkpoint)
    if (context?.run) protectTerminal(context, checkpoint, io.now())
    await save()
    return { state, context }
  }
  const journal = async state => {
    const replay = await io.replay()
    assertOriginalHistory(report.baseline, state, replay, config)
    if (checkpoint.journal_prefix) assertHistory({ rows: EMPTY_ROWS, journal: checkpoint.journal_prefix }, state, replay, config)
    observeJournal(replay, checkpoint, config)
    await save()
    return replay
  }
  const intent = async operation => {
    checkpoint[`${operation}_attempted`] = true
    checkpoint.phase = `${operation}_intent`
    await save()
    const request = operation === 'create' ? probeRequest(config) : { requested_by: config.actor_id }
    const route = `/api/corps/${config.corp_id}/missions${operation === 'launch' ? `/${checkpoint.mission_id}/launch` : ''}`
    await io.announce?.({ suite: SUITE, operation, method: 'POST', route, non_retried: true, receipt_id: config.receipt_id,
      request_sha256: digest(request), title: probeRequest(config).title, source: config.source, budget_tokens: checkpoint.budget_tokens,
      mission_id: checkpoint.mission_id, task_id: checkpoint.task_id, agent_id: checkpoint.agent_id,
      run_id: checkpoint.run_id, runner_id: config.runner_id })
  }
  const reconcileFailure = async () => {
    try { await inspect() } catch { /* Keep original ambiguous intent; never send another POST. */ }
    await save()
  }
  try {
    report.passed = false
    report.race_observed = false
    report.status = 'incomplete'
    report.error_code = null
    checkpoint.evidence = null
    if (!report.baseline) {
      requireThat(!checkpoint.create_attempted, 'missing_original_history_baseline')
      const state = await io.snapshot()
      assertRunner(state, config)
      caseContext(state, config, checkpoint)
      report.baseline = { ...historySummary(state, await io.replay(), config),
        agents: state.snapshot.agents.map(row => ({ id: row.id, sha256: digest(row) })) }
      await save()
    } else {
      const state = await io.snapshot()
      assertOriginalHistory(report.baseline, state, await io.replay(), config)
    }
    let { state, context } = await inspect()
    if (!context) {
      requireThat(!checkpoint.create_attempted, 'create_intent_ambiguous_no_replacement')
      assertRunner(state, config)
      const replay = await io.replay()
      assertOriginalHistory(report.baseline, state, replay, config)
      historySummary(state, replay, config) // No other live work before a new effect.
      await intent('create')
      try {
        const created = await io.create()
        checkpoint.mission_id = uuid(created.mission_id)
        checkpoint.task_id = uuid(created.task_id)
        await save()
        exact(created.task_ids, [checkpoint.task_id], 'create_not_single_task')
        requireThat(created.strategy === 'single', 'create_strategy_mismatch')
      } catch (error) { await reconcileFailure(); throw error }
      checkpoint.phase = 'created'
      ;({ state, context } = await inspect())
    }
    requireThat(context, 'created_mission_not_visible')
    if (!context.run) {
      requireThat(!checkpoint.launch_attempted, 'launch_intent_ambiguous_no_relaunch')
      requireThat(context.mission.status === 'ready' && context.task.status === 'ready' &&
        context.task.attempt_count === 0 && context.agent.status === 'idle' &&
        context.agent.current_run_id === null, 'original_plan_not_never_launched_ready')
      assertRunner(state, config)
      const replay = await io.replay()
      assertOriginalHistory(report.baseline, state, replay, config)
      historySummary(state, replay, config)
      await intent('launch')
      try {
        const launched = await io.launch(checkpoint.mission_id)
        checkpoint.run_id = uuid(launched.run_id)
        await save()
        exact(launched.run_ids, [checkpoint.run_id], 'launch_not_single_run')
        exact(launched.runner_ids, [config.runner_id], 'launch_runner_mismatch')
        requireThat(launched.runner_id === config.runner_id && launched.replayed === false, 'unexpected_launch_replay')
      } catch (error) { await reconcileFailure(); throw error }
      checkpoint.phase = 'launched'
      await save()
    }
    let stableSince = null
    let settledDigest = null
    let lastLiveReplay = -Infinity
    let pending = 'observation_timeout_run_not_reclassified'
    while (io.now() < deadline) {
      ;({ state, context } = await inspect())
      requireThat(context?.run, 'original_run_not_visible')
      if (TERMINAL.has(context.run.status)) {
        checkpoint.phase = 'terminal'
        const replay = await journal(state)
        ;({ state, context } = await inspect()) // State is read AFTER the Ready watermark.
        const current = digest([context.run, context.task, context.mission, context.agent])
        if (current !== settledDigest) { settledDigest = current; stableSince = io.now() }
        try {
          const evidence = assessCase(state, replay, config, checkpoint)
          if (io.now() - stableSince >= config.settle_ms) {
            evidence.base_file = await io.readBase(checkpoint)
            exact({ bytes: evidence.base_file.bytes, sha256: evidence.base_file.sha256 },
              { bytes: 5, sha256: BASE_SHA256 }, 'local_base_evidence_mismatch')
            const finalReplay = await journal(state)
            ;({ state, context } = await inspect())
            requireThat(digest([context.run, context.task, context.mission, context.agent]) === settledDigest,
              'terminal_state_changed_after_settle')
            const final = assessCase(state, finalReplay, config, checkpoint)
            checkpoint.evidence = { ...final, base_file: evidence.base_file, quiet_observation_ms: io.now() - stableSince }
            checkpoint.phase = 'passed'
            report.status = 'passed'
            report.passed = true
            report.race_observed = true
            await save()
            return report
          }
        } catch (error) {
          if (!(error instanceof CheckFailure) || !WAITABLE.has(error.code)) throw error
          pending = error.code
        }
      } else {
        checkpoint.phase = 'live'
        // Retain real ACK/journal observations even if a live run hits the bound.
        if (io.now() - lastLiveReplay >= Math.max(1000, config.settle_ms)) {
          await journal(state)
          lastLiveReplay = io.now()
        }
      }
      await save()
      // Full replay is bounded and at most once per second while terminal.
      await io.sleep(Math.min(TERMINAL.has(context.run.status) ? Math.max(1000, config.poll_ms) : config.poll_ms,
        Math.max(0, deadline - io.now())))
    }
    throw new CheckFailure(pending)
  } catch (error) {
    report.passed = false
    report.race_observed = false
    report.status = LIVE.has(checkpoint.latest?.status) ? 'incomplete_live' : 'failed'
    report.error_code = safeCode(error)
    await save()
    throw new CheckFailure(report.error_code)
  }
}

function validateJournalSummary(value) {
  keys(value, ['through', 'event_count', 'sha256'], 'invalid_saved_journal')
  requireThat(Number.isSafeInteger(value.through) && value.through >= 0 &&
    Number.isSafeInteger(value.event_count) && value.event_count >= 0 &&
    value.event_count <= LIMITS.replay_events, 'invalid_saved_journal_bound')
  hash(value.sha256)
}

export function validateSavedReport(saved, config) {
  const fresh = newReport(config)
  keys(saved, Object.keys(fresh), 'invalid_report_schema')
  requireThat(saved.schema_version === 1 && saved.suite === SUITE && saved.receipt_id === config.receipt_id &&
    saved.receipt_sha256 === receiptIdentity(config), 'continuation_receipt_mismatch')
  requireThat(typeof saved.started_at === 'string' && Number.isFinite(Date.parse(saved.started_at)), 'invalid_saved_time')
  const item = saved.case
  keys(item, Object.keys(fresh.case), 'invalid_saved_case')
  requireThat(item.budget_tokens === 5_000 && item.request_sha256 === fresh.case.request_sha256 &&
    typeof item.create_attempted === 'boolean' && typeof item.launch_attempted === 'boolean' &&
    (!item.launch_attempted || item.create_attempted), 'invalid_saved_intent')
  for (const key of ['mission_id', 'task_id', 'agent_id', 'run_id']) if (item[key] !== null) uuid(item[key])
  requireThat((!item.mission_id || item.create_attempted) && (!item.task_id || item.mission_id) &&
    (!item.agent_id || item.task_id) && (!item.run_id || (item.launch_attempted && item.agent_id)),
  'saved_identity_without_intent')
  if (item.contract_sha256 !== null) hash(item.contract_sha256)
  requireThat(['new', 'create_intent', 'created', 'launch_intent', 'launched', 'live', 'terminal', 'passed'].includes(item.phase),
    'invalid_saved_phase')
  if (item.terminal_baseline !== null) {
    requireThat(item.run_id !== null, 'terminal_baseline_without_run')
    keys(item.terminal_baseline, ['first_observed_at', 'digests'], 'invalid_saved_terminal_baseline')
    requireThat(Number.isFinite(Date.parse(item.terminal_baseline.first_observed_at)), 'invalid_saved_time')
    keys(item.terminal_baseline.digests, ['run', 'task', 'mission', 'agent'], 'invalid_saved_terminal_digests')
    Object.values(item.terminal_baseline.digests).forEach(hash)
  }
  if (item.journal_prefix !== null) validateJournalSummary(item.journal_prefix)
  if (item.failure_event !== null) {
    keys(item.failure_event, ['id', 'seq', 'sha256'], 'invalid_saved_failure')
    uuid(item.failure_event.id)
    hash(item.failure_event.sha256)
    requireThat(item.run_id && item.journal_prefix && Number.isSafeInteger(item.failure_event.seq) &&
      item.failure_event.seq > 0 && item.failure_event.seq <= item.journal_prefix.through, 'invalid_saved_failure_sequence')
  }
  if (saved.baseline !== null) {
    keys(saved.baseline, ['rows', 'journal', 'agents'], 'invalid_saved_baseline')
    keys(saved.baseline.rows, Object.keys(EMPTY_ROWS), 'invalid_saved_rows')
    for (const rows of [...Object.values(saved.baseline.rows), saved.baseline.agents]) {
      requireThat(Array.isArray(rows) && rows.length <= LIMITS.snapshot_rows, 'invalid_saved_history_bound')
      const seen = new Set()
      for (const row of rows) {
        keys(row, ['id', 'sha256'], 'invalid_saved_history_row')
        uuid(row.id); hash(row.sha256)
        requireThat(!seen.has(row.id), 'duplicate_saved_history_row')
        seen.add(row.id)
      }
    }
    validateJournalSummary(saved.baseline.journal)
  } else requireThat(!item.create_attempted, 'missing_original_history_baseline')
  // Preserve original identity/intent/history anchors; recompute every success claim.
  return { ...fresh, started_at: saved.started_at, baseline: saved.baseline,
    case: { ...item, phase: item.phase === 'passed' ? 'launched' : item.phase,
      latest: null, journal_observation: null, evidence: null } }
}

async function canonicalDirectory(directory, io) {
  const exactPath = localAbsolute(directory)
  const info = await io.lstat(exactPath)
  requireThat(info.isDirectory() && !info.isSymbolicLink() &&
    samePath(exactPath, await io.realpath(exactPath)), 'canonical_existing_directory_required')
  return exactPath
}

export async function readBounded(file, maximum, io = fs) {
  await checkContainedFile(path.dirname(file), file, io)
  const before = await io.lstat(file)
  requireThat(before.size <= maximum, 'local_file_byte_bound')
  const handle = await io.open(file, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0))
  try {
    const opened = await handle.stat()
    requireThat(opened.isFile() && opened.nlink === 1 && opened.dev === before.dev &&
      opened.ino === before.ino && opened.size === before.size, 'opened_file_identity_changed')
    const bytes = Buffer.alloc(opened.size + 1)
    let used = 0
    while (used < bytes.length) {
      const read = await handle.read(bytes, used, bytes.length - used, used)
      if (!read.bytesRead) break
      used += read.bytesRead
    }
    const after = await handle.stat()
    requireThat(used === opened.size && after.size === opened.size && after.mtimeMs === opened.mtimeMs &&
      after.nlink === 1, 'local_file_changed_during_read')
    await checkContainedFile(path.dirname(file), file, io)
    return bytes.subarray(0, used)
  } finally { await handle.close() }
}

async function exists(file, io) {
  try { await io.lstat(file); return true } catch (error) { if (error.code === 'ENOENT') return false; throw error }
}

export async function openReport(config, options, io = fs) {
  await canonicalDirectory(config.runner_root, io)
  const output = await canonicalDirectory(config.output_dir, io)
  requireThat(!samePath(config.report_path, options.receiptPath), 'report_cannot_replace_receipt')
  const target = config.report_path
  const lockPath = `${target}.lock`
  const lock = await io.open(lockPath, 'wx', 0o600)
  const close = async () => { await lock.close(); await io.unlink(lockPath) }
  try {
    await lock.writeFile(`${JSON.stringify({ suite: SUITE, receipt_id: config.receipt_id })}\n`)
    await lock.sync()
    const present = await exists(target, io)
    requireThat(present === options.continuation, present ? 'report_exists_use_explicit_continue' : 'continuation_report_missing')
    let report
    if (present) {
      const bytes = await readBounded(target, LIMITS.report_bytes, io)
      report = validateSavedReport(JSON.parse(bytes.toString('utf8')), config)
      const previous = await io.open(`${target}.previous-${randomUUID()}.json`, 'wx', 0o600)
      try { await previous.writeFile(bytes); await previous.sync() } finally { await previous.close() }
    } else report = newReport(config)
    const save = async value => {
      await canonicalDirectory(output, io)
      if (await exists(target, io)) await readBounded(target, LIMITS.report_bytes, io)
      const encoded = `${JSON.stringify(value, null, 2)}\n`
      requireThat(Buffer.byteLength(encoded) <= LIMITS.report_bytes, 'report_byte_bound')
      const tempPath = `${target}.${randomUUID()}.tmp`
      const temporary = await io.open(tempPath, 'wx', 0o600)
      try { await temporary.writeFile(encoded); await temporary.sync() } finally { await temporary.close() }
      await canonicalDirectory(output, io)
      await io.rename(tempPath, target)
    }
    return { report, save, close }
  } catch (error) { await close(); throw error }
}

export async function main(argv = process.argv.slice(2)) {
  let storage
  let reportPath = null
  try {
    const options = parseArgs(argv)
    if (options.schema) {
      console.log(JSON.stringify({ ...receiptSchema(),
        source: { repository: '<exact advertised synthetic enterprise-lab label>', base_ref: 'HEAD', base_commit: SOURCE_COMMIT },
        report_path: '<new absolute e2e-late-termination.json directly in output_dir>' }, null, 2))
      return 0
    }
    const receipt = JSON.parse((await readBounded(options.receiptPath, LIMITS.receipt_bytes)).toString('utf8'))
    const config = probeConfiguration(receipt, options)
    reportPath = config.report_path
    storage = await openReport(config, options)
    await storage.save(storage.report)
    await executeProbe(config, storage.report, {
      ...createProbeApi(config), now: Date.now, sleep: ms => new Promise(resolve => setTimeout(resolve, ms)),
      save: storage.save, readBase: checkpoint => readBaseFile(config, checkpoint),
      announce: value => console.log(JSON.stringify(value)),
    })
    console.log(JSON.stringify({ suite: SUITE, passed: true, race_observed: true,
      report_path: reportPath, run_id: storage.report.case.run_id, ordering: storage.report.case.evidence.ordering }))
    return 0
  } catch (error) {
    console.error(JSON.stringify({ suite: SUITE, passed: false, race_observed: false,
      error_code: safeCode(error), report_path: reportPath,
      instruction: 'Retain original case and resources. Parent may intentionally --continue the same receipt/report; never replace uncertain work.' }))
    return 1
  } finally {
    try { await storage?.close() } catch {
      console.error(JSON.stringify({ suite: SUITE, error_code: 'report_lock_release_failed',
        instruction: 'Parent must verify driver death before removing the exact report lock.' }))
      return 1
    }
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  process.exitCode = await main()
}
