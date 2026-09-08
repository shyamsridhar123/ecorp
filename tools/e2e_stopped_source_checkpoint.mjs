#!/usr/bin/env node
/**
 * Additive #190 / #192 API regression. IMPLEMENTATION/UNIT TESTS ARE OFFLINE.
 *
 * Parent starts and owns the existing development server/runner, using ONLY
 * scripts/fake-codex-app-server.mjs through the native codex adapter. This is a
 * deterministic protocol fixture, NOT a real-vendor inference test.
 *
 * Invocation (Node >= 22, native fetch/WebSocket):
 *   node tools/e2e_stopped_source_checkpoint.mjs --schema
 *   node tools/e2e_stopped_source_checkpoint.mjs --receipt <absolute JSON path>
 *   node tools/e2e_stopped_source_checkpoint.mjs --receipt <same path> --continue
 *
 * The parent's existing issue190 runtime.json is also accepted, WITHOUT editing
 * it. Supply its missing driver-only authority as explicit CLI configuration:
 *   --source-repository shyamsridhar123/ecorp-enterprise-lab
 *   --fixture scripts/fake-codex-app-server.mjs
 *   --fixture-sha256 <parent-computed lowercase SHA256 of the CURRENT fixture>
 *   --output-dir <absolute existing evidence directory>
 *   --report-path <absolute JSON file directly in that directory>
 * Optional --timeout-ms / --poll-ms / --settle-ms retain the bounds below.
 * Public runtime receipt: schema_version=1, test_owned=true, issue=190,
 * owner_task UUID, phase=running, server_url, corp_id, actor_id, runner_id,
 * runner_root, source_repository_path, source_base_ref, source_base_commit,
 * workspace, source_commit, and processes.server/runner with role, pid,
 * executable, workspace, started_utc. Other metadata is ignored, never followed.
 * The explicit --fixture is the parent's attestation, not executable inspection.
 *
 * --schema prints the complete non-secret ownership-receipt/config example.
 * All identity/path/endpoint fields are required; timings alone have defaults.
 * The output directory and runner root must already exist. Never put credentials,
 * environment dumps, provider output, or an enrollment manifest in this receipt.
 *
 * Only mutations: ordinary create + explicit launch for two NEW single missions.
 * No bootstrap/reset, SQL, policy changes, budget revisions, approvals, enrollment,
 * resume API, services, subprocesses, environment reads, provider/browser launch,
 * or cleanup of a mission/run/worktree. Never import the older top-level E2Es.
 *
 * Mutation intent is durably checkpointed BEFORE sending each non-retried POST.
 * A lost response can only be reconciled to the exact existing title/task/run.
 * An uncertain POST with no discoverable result is NOT retried. --continue is
 * explicit suite continuation, not provider resume or permission to replace work.
 * It observes launched/completed cases and may launch only a never-launched plan.
 *
 * Reports use atomic replacement plus an exclusive adjacent .lock file. After
 * process death, the parent must establish that the previous driver is dead before
 * removing that exact stale lock; this driver never steals locks. All other files
 * (including a failed checkpoint and orphaned .tmp file) are retained.
 *
 * Evidence scope: authenticated development-actor-visible snapshots and complete
 * paginated journal replay from zero through each Ready watermark, within explicit
 * byte/event/time bounds. A cap/error fails closed, not "whole history passed".
 * Snapshot.events is only a 200-row window and is never our absence/history proof.
 * The prior visible mission/task/run rows and journal prefix must remain unchanged.
 * No claim is made about other rooms, tenants, database rows, rejected uploads,
 * future retries, OS process ownership, or real provider containment.
 *
 * Only base.txt is read in a returned workspace, after canonical root, generated
 * task/run path, link, opened-file identity and byte-bound checks. HEAD is checked
 * against native persisted checkpoint evidence; no git command runs here. Parent
 * may separately compare HEAD/status/fixture bytes in its EXPLICIT source checkout
 * before/after execution. Such an optional source-checkout check is not invented
 * or implied by this report. Parent receipt attests fixture/process configuration;
 * public runner capabilities do not attest the provider executable.
 */

import { createHash, randomUUID } from 'node:crypto'
import { constants } from 'node:fs'
import * as fs from 'node:fs/promises'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

export const SUITE = 'stopped-source-checkpoint-190'
export const FIXTURE = 'scripts/fake-codex-app-server.mjs'
// The parent may extend the same fixture with a disjoint UI usage marker. Its
// CURRENT byte digest is explicit input, not a stale hard-coded executable claim.
export const BASE_SHA256 = createHash('sha256').update('base\n').digest('hex')
export const CASES = Object.freeze({ suspend: 6_000, stop: 5_000 })
export const LIMITS = Object.freeze({
  response_bytes: 8 * 1024 * 1024,
  replay_bytes: 16 * 1024 * 1024,
  replay_events: 20_000,
  snapshot_rows: 10_000,
  report_bytes: 16 * 1024 * 1024,
  receipt_bytes: 32 * 1024,
  requests: 600,
  request_ms: 10_000,
})
export const PROTECTED_PORTS = Object.freeze([
  5187, 5291, 8791, 8793, 8991, 15191, 15193, 15491, 15493, 15496,
  18961, 18962, 18963,
])
const UUID = /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/u
const SHA256 = /^[0-9a-f]{64}$/u
const COMMIT = /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/u
const LIVE = new Set(['provisioning', 'starting', 'running', 'waiting_for_input', 'waiting_for_approval', 'verifying'])
const TERMINAL = new Set(['completed', 'cancelled', 'failed', 'lost'])
const STAGES = ['none', 'steer', 'constrain', 'suspend', 'stop']
const ROWS = ['missions', 'tasks', 'runs']
const ABSENCE_TABLES = ['verification_evidence', 'verification_requests', 'source_deliverables']
const RECEIPT_KEYS = [
  'schema_version', 'test_owned', 'receipt_id', 'auth_mode', 'server_url', 'corp_id',
  'actor_id', 'runner_id', 'source', 'runner_root', 'output_dir', 'report_path',
  'provider', 'runtime_binding_sha256', 'timeout_ms', 'poll_ms', 'settle_ms',
]

export class CheckFailure extends Error {
  constructor(code) {
    super(code)
    this.name = 'CheckFailure'
    this.code = code
  }
}
function requireThat(condition, code) {
  if (!condition) throw new CheckFailure(code)
}
function object(value, code = 'object_required') {
  requireThat(value !== null && typeof value === 'object' && !Array.isArray(value), code)
  return value
}
function keys(value, allowed, code) {
  object(value, code)
  requireThat(Object.keys(value).every((key) => allowed.includes(key)), code)
}
function exact(actual, expected, code) {
  requireThat(canonical(actual) === canonical(expected), code)
}
function uuid(value) {
  requireThat(typeof value === 'string' && UUID.test(value), 'invalid_uuid')
  return value
}
function integer(value, min, max, code) {
  requireThat(Number.isSafeInteger(value) && value >= min && value <= max, code)
  return value
}
function hashString(value, code = 'invalid_sha256') {
  requireThat(typeof value === 'string' && SHA256.test(value), code)
  return value
}
function safeCode(error) {
  // Never propagate native errors (paths, request bodies, provider output, secrets).
  return error instanceof CheckFailure ? error.code : 'operation_failed_details_withheld'
}
export function canonical(value) {
  return JSON.stringify(value, (_, item) => item && typeof item === 'object' && !Array.isArray(item)
    ? Object.fromEntries(Object.keys(item).sort().map((key) => [key, item[key]])) : item)
}
export function digest(value) {
  return createHash('sha256').update(canonical(value)).digest('hex')
}
const byteDigest = (value) => createHash('sha256').update(value).digest('hex')

export function receiptSchema() {
  return {
    schema_version: 1,
    test_owned: true,
    receipt_id: '<parent-generated UUID, stable across continuation>',
    auth_mode: 'development',
    server_url: 'http://127.0.0.1:18574',
    corp_id: '00000000-0000-4000-8000-000000000001',
    actor_id: '00000000-0000-4000-8000-000000000011',
    runner_id: 'issue174-local-start',
    source: {
      repository: '<exact advertised source_repository, not a filesystem path>',
      base_ref: '<exact advertised source_base_ref>',
      base_commit: 'a8894b5f02d56f10e2da38df47a450ff71e92fbe',
    },
    runner_root: '<absolute existing runner workspace root, ABOVE worktrees>',
    output_dir: '<absolute existing parent-owned evidence directory>',
    report_path: '<absolute .json file directly inside output_dir>',
    provider: { adapter: 'codex', fixture: FIXTURE, sha256: '<parent-computed lowercase SHA256 of current fixture>' },
    timeout_ms: 120_000,
    poll_ms: 250,
    settle_ms: 3_000,
  }
}

export function parseArgs(argv) {
  if (argv.length === 1 && ['--schema', '--help'].includes(argv[0])) return { schema: true }
  let receiptPath
  let continuation = false
  const extra = {}
  const options = {
    '--source-repository': 'sourceRepository', '--fixture': 'fixture', '--fixture-sha256': 'fixtureSha256',
    '--output-dir': 'outputDir', '--report-path': 'reportPath',
    '--timeout-ms': 'timeoutMs', '--poll-ms': 'pollMs', '--settle-ms': 'settleMs',
  }
  for (let index = 0; index < argv.length; index += 1) {
    if (argv[index] === '--receipt' && receiptPath === undefined && argv[index + 1]) {
      receiptPath = argv[++index]
    } else if (argv[index] === '--continue' && !continuation) {
      continuation = true
    } else if (Object.hasOwn(options, argv[index]) && argv[index + 1] &&
      !Object.hasOwn(extra, options[argv[index]])) {
      const key = options[argv[index]]
      const value = argv[++index]
      requireThat(!value.startsWith('--'), 'cli_option_value_missing')
      extra[key] = key.endsWith('Ms') ? Number(value) : value
    } else {
      throw new CheckFailure('usage_requires_receipt_and_optional_continue')
    }
  }
  requireThat(receiptPath !== undefined, 'explicit_receipt_required')
  return { receiptPath: localAbsolute(receiptPath), continuation, ...extra }
}

// Windows canonical paths reported by Rust may use an extended DRIVE prefix.
// Device namespaces, UNC, ADS, drive-relative paths and dot aliases are forbidden.
export function localAbsolute(value, pathApi = path) {
  requireThat(typeof value === 'string' && value.length > 0 && value.length <= 1500 &&
    value === value.trim() && !/[\u0000-\u001f\u007f]/u.test(value), 'unsafe_absolute_path')
  const windows = pathApi.sep === '\\'
  let result = value
  if (windows) {
    result = result.replaceAll('/', '\\')
    if (/^\\\\\?\\[A-Za-z]:\\/u.test(result)) result = result.slice(4)
    requireThat(/^[A-Za-z]:\\/u.test(result) && !result.slice(2).includes(':') &&
      !/[?*<>|"]/u.test(result), 'unsafe_windows_path')
    const components = result.slice(3).split('\\').filter(Boolean)
    requireThat(components.every((component) => component !== '.' && component !== '..' &&
      component === component.trim() && !component.endsWith('.') &&
      !/^(?:con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/iu.test(component)), 'unsafe_windows_component')
  } else {
    requireThat(result.startsWith('/') && !result.startsWith('//') && !result.includes('\\') &&
      result.split('/').every((component) => !['.', '..'].includes(component)), 'unsafe_posix_path')
  }
  result = pathApi.normalize(result)
  requireThat(pathApi.isAbsolute(result) && result !== pathApi.parse(result).root, 'path_root_forbidden')
  return result.replace(/[\\/]$/u, '')
}
function samePath(left, right, pathApi = path) {
  return pathApi.relative(localAbsolute(left, pathApi), localAbsolute(right, pathApi)) === ''
}
export function containedPath(root, candidate, pathApi = path) {
  const base = localAbsolute(root, pathApi)
  const target = localAbsolute(candidate, pathApi)
  const relative = pathApi.relative(base, target)
  requireThat(relative !== '' && relative !== '..' && !relative.startsWith(`..${pathApi.sep}`) &&
    !pathApi.isAbsolute(relative), 'path_outside_owned_root')
  return target
}

export function validateReceipt(input, pathApi = path) {
  keys(input, RECEIPT_KEYS, 'unknown_receipt_field')
  requireThat(input.schema_version === 1 && input.test_owned === true &&
    input.auth_mode === 'development', 'explicit_owned_development_receipt_required')
  uuid(input.receipt_id)
  uuid(input.corp_id)
  uuid(input.actor_id)
  requireThat(typeof input.runner_id === 'string' &&
    /^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/u.test(input.runner_id) &&
    input.runner_id !== 'runner-local', 'explicit_nondefault_runner_required')
  let endpoint
  try { endpoint = new URL(input.server_url) } catch { throw new CheckFailure('invalid_server_origin') }
  requireThat(endpoint.protocol === 'http:' && endpoint.hostname === '127.0.0.1' &&
    Number(endpoint.port) >= 10_000 && Number(endpoint.port) <= 65_535 &&
    !PROTECTED_PORTS.includes(Number(endpoint.port)) &&
    !endpoint.username && !endpoint.password && !endpoint.search && !endpoint.hash &&
    endpoint.pathname === '/' && input.server_url === endpoint.origin, 'protected_or_ambiguous_endpoint')
  keys(input.source, ['repository', 'base_ref', 'base_commit'], 'invalid_source_tuple')
  const { repository, base_ref: ref, base_commit: commit } = input.source
  requireThat(typeof repository === 'string' && repository.length <= 300 &&
    /^(?:local\/[A-Za-z0-9._-]+|https:\/\/github\.com\/[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+(?:\.git)?|[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+)$/u.test(repository),
  'source_repository_must_be_public_routing_identity')
  requireThat(typeof ref === 'string' && ref.length > 0 && ref.length <= 200 &&
    !ref.startsWith('-') && !/[\s\u0000-\u001f\u007f\\:?*[\]~^]/u.test(ref) &&
    !ref.includes('..') && !ref.includes('//') && !ref.includes('@{') &&
    !ref.startsWith('/') && !ref.endsWith('/') && !ref.endsWith('.'), 'invalid_source_ref')
  requireThat(typeof commit === 'string' && COMMIT.test(commit), 'explicit_immutable_commit_required')
  keys(input.provider, ['adapter', 'fixture', 'sha256'], 'exact_native_fixture_receipt_required')
  requireThat(input.provider.adapter === 'codex' && input.provider.fixture === FIXTURE, 'exact_native_fixture_receipt_required')
  hashString(input.provider.sha256, 'explicit_current_fixture_digest_required')
  if (input.runtime_binding_sha256 !== undefined) hashString(input.runtime_binding_sha256)
  const normalized = { ...input }
  for (const field of ['runner_root', 'output_dir', 'report_path']) {
    normalized[field] = localAbsolute(input[field], pathApi)
  }
  containedPath(normalized.output_dir, normalized.report_path, pathApi)
  requireThat(samePath(pathApi.dirname(normalized.report_path), normalized.output_dir, pathApi) &&
    /^[A-Za-z0-9][A-Za-z0-9_.-]{0,119}\.json$/u.test(pathApi.basename(normalized.report_path)),
  'report_must_be_named_json_in_output_directory')
  // Evidence must not be written in an agent-managed task workspace.
  const managed = pathApi.join(normalized.runner_root, 'worktrees')
  const outputRelative = pathApi.relative(managed, normalized.output_dir)
  requireThat(outputRelative.startsWith(`..${pathApi.sep}`) || outputRelative === '..' ||
    pathApi.isAbsolute(outputRelative), 'output_inside_managed_worktrees')
  normalized.timeout_ms = integer(input.timeout_ms ?? 120_000, 10_000, 600_000, 'invalid_timeout_bound')
  normalized.poll_ms = integer(input.poll_ms ?? 250, 100, 2_000, 'invalid_poll_bound')
  normalized.settle_ms = integer(input.settle_ms ?? 3_000, 2_000, 30_000, 'invalid_settle_bound')
  requireThat(normalized.timeout_ms >= 2 * normalized.settle_ms + 5_000, 'insufficient_observation_bound')
  return normalized
}

export function configurationFromReceipt(receipt, options = {}, pathApi = path) {
  if (!Object.hasOwn(object(receipt), 'owner_task')) {
    requireThat(['sourceRepository', 'fixture', 'fixtureSha256', 'outputDir', 'reportPath', 'timeoutMs', 'pollMs', 'settleMs']
      .every((key) => options[key] === undefined), 'self_contained_receipt_cannot_be_overridden')
    return validateReceipt(receipt, pathApi)
  }
  requireThat(receipt.schema_version === 1 && receipt.test_owned === true &&
    receipt.issue === 190 && receipt.phase === 'running', 'owned_running_issue190_receipt_required')
  uuid(receipt.owner_task)
  requireThat(options.fixture === FIXTURE, 'explicit_native_fixture_attestation_required')
  hashString(options.fixtureSha256, 'explicit_current_fixture_digest_required')
  requireThat(typeof receipt.source_commit === 'string' && COMMIT.test(receipt.source_commit),
    'runtime_code_commit_missing')
  const workspace = localAbsolute(receipt.workspace, pathApi)
  const sourcePath = localAbsolute(receipt.source_repository_path, pathApi)
  requireThat(!samePath(sourcePath, workspace, pathApi), 'independent_source_fixture_required')
  const processes = {}
  for (const role of ['server', 'runner']) {
    const process = object(receipt.processes?.[role], 'public_process_identity_required')
    requireThat(process.role === role && samePath(process.workspace, workspace, pathApi) &&
      typeof process.started_utc === 'string' && Number.isFinite(Date.parse(process.started_utc)),
    'public_process_identity_mismatch')
    processes[role] = {
      pid: integer(process.pid, 1, Number.MAX_SAFE_INTEGER, 'invalid_public_process_pid'),
      executable: localAbsolute(process.executable, pathApi),
      started_utc: process.started_utc,
    }
  }
  return validateReceipt({
    schema_version: 1, test_owned: true, receipt_id: receipt.owner_task,
    auth_mode: 'development', server_url: receipt.server_url,
    corp_id: receipt.corp_id, actor_id: receipt.actor_id, runner_id: receipt.runner_id,
    source: { repository: options.sourceRepository, base_ref: receipt.source_base_ref, base_commit: receipt.source_base_commit },
    runner_root: receipt.runner_root, output_dir: options.outputDir, report_path: options.reportPath,
    provider: { adapter: 'codex', fixture: options.fixture, sha256: options.fixtureSha256 },
    runtime_binding_sha256: digest({ workspace, source_path: sourcePath, code_commit: receipt.source_commit, processes }),
    timeout_ms: options.timeoutMs, poll_ms: options.pollMs, settle_ms: options.settleMs,
  }, pathApi)
}
export function receiptIdentity(config) {
  const { timeout_ms, poll_ms, settle_ms, ...identity } = config
  return digest(identity)
}

export function caseRequest(config, name) {
  requireThat(Object.hasOwn(CASES, name), 'unknown_case')
  return {
    requested_by: config.actor_id,
    preferred_adapter: 'codex',
    preferred_model: null,
    reasoning_effort: null,
    strategy: 'single',
    source: { ...config.source },
    title: `[budget-stream] stopped-source ${config.receipt_id} ${name}`,
    description: 'Deterministic native Codex protocol fixture. Retain original source at the hard budget boundary; do not resume, verify, publish, or claim accepted completion.',
    budget_tokens: CASES[name],
    budget_cost_microusd: 10_000_000,
    secret_refs: [],
    deliverable: null,
    contract: {
      objective: '[budget-stream] Write only base.txt containing exactly base followed by LF before the fixture streams usage.',
      expected_output: 'Original source retained with native pre-verification checkpoint evidence, not an accepted deliverable.',
      acceptance_tests: ['base.txt contains exactly five bytes: base followed by LF', 'hard-budget termination precedes checkpoint and terminal state'],
      allowed_tools: ['filesystem'],
      prohibited_actions: ['modify any file except base.txt', 'access credentials or the network', 'publish, commit, resume, or change budget authority'],
      references: [FIXTURE],
      write_scope: ['base.txt'],
    },
    verification_policy: {
      checks: [{ type: 'file', path: 'base.txt', min_bytes: 5 }],
      manual_gate: { type: 'human_approval', roles: ['owner', 'admin'] },
    },
  }
}

/**
 * serde_json::to_vec(VerificationPolicy) follows DECLARATION order, not JSONB's
 * object order. See crony-domain/src/lib.rs: VerifierCheck, ManualVerificationGate,
 * VerificationPolicy. This intentionally supports only this driver's exact File
 * policy. Unknown checks/gates/fields fail, instead of guessing a Rust encoding.
 * canonical()/digest() are ONLY used for order-insensitive row/history comparison.
 */
export function rustVerificationJson(policy) {
  keys(policy, ['checks', 'manual_gate'], 'unexpected_verifier_policy')
  requireThat(Array.isArray(policy.checks) && policy.checks.length === 1, 'unexpected_verifier_policy')
  const check = policy.checks[0]
  exact(check, { type: 'file', path: 'base.txt', min_bytes: 5 }, 'unexpected_file_verifier')
  exact(policy.manual_gate, { type: 'human_approval', roles: ['owner', 'admin'] }, 'unexpected_manual_gate')
  return JSON.stringify({
    checks: [{ type: check.type, path: check.path, min_bytes: check.min_bytes }],
    manual_gate: { type: policy.manual_gate.type, roles: [...policy.manual_gate.roles] },
  })
}
export function policyDigests(policy, writeScope, deliverable) {
  exact(writeScope, ['base.txt'], 'write_scope_not_base_only')
  exact(deliverable, null, 'deliverable_must_remain_null')
  return {
    verification_policy_sha256: byteDigest(rustVerificationJson(policy)),
    write_scope_sha256: byteDigest(JSON.stringify(['base.txt'])),
    deliverable_policy_sha256: byteDigest('null'),
  }
}

function validateSnapshot(state, config) {
  object(state, 'snapshot_required')
  object(state.snapshot, 'snapshot_required')
  const snap = state.snapshot
  requireThat(snap.corp?.id === config.corp_id, 'snapshot_corp_mismatch')
  for (const table of [...ROWS, 'actors', 'rooms', 'agents', ...ABSENCE_TABLES,
    'mission_contract_revisions', 'mission_budget_revisions', 'action_approvals', 'circuit_breaker_incidents']) {
    const values = snap[table]
    requireThat(Array.isArray(values) && values.length <= LIMITS.snapshot_rows, 'snapshot_table_missing_or_bounded_out')
    const seen = new Set()
    for (const row of values) {
      // VerificationRequest is keyed by run_id; unlike evidence it has no id.
      const id = table === 'verification_requests' ? row.run_id : row.id
      uuid(id)
      requireThat(row.corp_id === config.corp_id && !seen.has(id), 'snapshot_identity_mismatch')
      seen.add(id)
    }
  }
  requireThat(Array.isArray(state.runners), 'runner_projection_missing')
  return snap
}
export function assertRunner(state, config) {
  validateSnapshot(state, config)
  const matching = state.runners.filter((runner) => runner.corp_id === config.corp_id &&
    runner.connected === true && runner.status === 'connected' &&
    runner.capabilities?.some((cap) => cap.name === 'codex' && cap.available === true) &&
    runner.capabilities?.some((cap) => cap.name === 'workspace-isolation' && cap.available === true &&
      cap.source_repository === config.source.repository && cap.source_base_ref === config.source.base_ref &&
      cap.source_base_commit === config.source.base_commit))
  requireThat(matching.length === 1 && matching[0].id === config.runner_id, 'exact_unique_native_runner_required')
  const actor = state.snapshot.actors.find((item) => item.id === config.actor_id)
  requireThat(actor?.kind === 'human' && ['owner', 'admin', 'manager'].includes(actor.role), 'requester_not_visible_operator')
}

export function replaySummary(events, through, corpId) {
  requireThat(Array.isArray(events) && events.length <= LIMITS.replay_events, 'journal_event_bound')
  integer(through, 0, Number.MAX_SAFE_INTEGER, 'invalid_replay_watermark')
  let cursor = 0
  const seen = new Set()
  for (const event of events) {
    uuid(event.id)
    requireThat(event.corp_id === corpId && Number.isSafeInteger(event.seq) && event.seq > cursor &&
      !seen.has(event.id), 'journal_identity_or_order_mismatch')
    requireThat(typeof event.type === 'string' && /^[a-z_]+\.[a-z_]+$/u.test(event.type) &&
      event.type.length <= 100, 'journal_event_type_invalid')
    seen.add(event.id)
    cursor = event.seq
  }
  requireThat(cursor === through, 'journal_ready_watermark_mismatch')
  return { through, event_count: events.length, sha256: digest(events) }
}
export function historySummary(state, replay, config) {
  const snap = validateSnapshot(state, config)
  requireThat(snap.runs.every((run) => TERMINAL.has(run.status)), 'existing_live_runs_block_new_suite')
  requireThat(snap.missions.every((mission) => mission.status !== 'running'), 'existing_running_missions_block_new_suite')
  return {
    rows: Object.fromEntries(ROWS.map((table) => [table, snap[table]
      .map((row) => ({ id: row.id, sha256: digest(row) })).sort((a, b) => a.id.localeCompare(b.id))])),
    journal: replaySummary(replay.events, replay.through, config.corp_id),
  }
}
export function assertHistory(baseline, state, replay, config) {
  const snap = validateSnapshot(state, config)
  for (const table of ROWS) {
    const current = new Map(snap[table].map((row) => [row.id, digest(row)]))
    for (const old of baseline.rows[table]) {
      requireThat(current.has(old.id), 'prior_visible_row_missing_coverage_not_proven')
      requireThat(current.get(old.id) === old.sha256, 'prior_visible_history_changed')
    }
  }
  if (replay) {
    replaySummary(replay.events, replay.through, config.corp_id)
    requireThat(replay.through >= baseline.journal.through, 'journal_watermark_regressed')
    const prefix = replay.events.filter((event) => event.seq <= baseline.journal.through)
    requireThat(prefix.length === baseline.journal.event_count &&
      digest(prefix) === baseline.journal.sha256, 'prior_visible_journal_prefix_changed')
  }
}

export function newReport(config, now = Date.now()) {
  return {
    schema_version: 1,
    suite: SUITE,
    receipt_id: config.receipt_id,
    receipt_sha256: receiptIdentity(config),
    started_at: new Date(now).toISOString(),
    updated_at: new Date(now).toISOString(),
    status: 'incomplete',
    passed: false,
    baseline: null,
    cases: Object.fromEntries(Object.entries(CASES).map(([name, budget]) => [name, {
      budget_tokens: budget,
      create_attempted: false,
      launch_attempted: false,
      mission_id: null,
      task_id: null,
      agent_id: null,
      run_id: null,
      contract_sha256: null,
      phase: 'new',
      latest: null,
      journal_observation: null,
      evidence: null,
    }])),
    error_code: null,
    coverage: {
      principal: 'development actor, not OIDC',
      provider: 'deterministic native codex protocol fixture, not real vendor',
      ownership: 'explicit parent receipt; no independent process/executable inspection',
      history: 'only actor-visible mission/task/run rows and replayed journal prefix',
      absence: 'accepted records/events only, through recorded watermark and settle interval',
      local_reads: 'base.txt only within the canonical configured runner root',
      head: 'native persisted source checkpoint compared with original source commit',
      source_checkout_unchanged: 'not checked here; optional explicit-path parent check',
    },
  }
}

function caseContext(state, config, name, checkpoint) {
  const snap = validateSnapshot(state, config)
  const request = caseRequest(config, name)
  const matching = snap.missions.filter((item) => item.title === request.title)
  requireThat(matching.length <= 1, 'duplicate_case_mission')
  if (checkpoint.mission_id) {
    requireThat(matching.length === 1 && matching[0].id === checkpoint.mission_id, 'original_case_mission_missing')
  }
  if (matching.length === 0) return null
  const mission = matching[0]
  requireThat(checkpoint.create_attempted, 'unowned_case_title_collision')
  uuid(mission.id)
  if (!checkpoint.mission_id) checkpoint.mission_id = mission.id
  requireThat(mission.requested_by === config.actor_id && mission.description === request.description &&
    mission.strategy === 'single' && mission.specification_version === 1 &&
    mission.budget_tokens === request.budget_tokens && mission.original_budget_tokens === request.budget_tokens &&
    mission.budget_cost_microusd === request.budget_cost_microusd &&
    mission.original_budget_cost_microusd === request.budget_cost_microusd, 'mission_authority_changed')
  const tasks = snap.tasks.filter((task) => task.mission_id === mission.id)
  requireThat(tasks.length === 1, 'case_must_keep_one_original_task')
  const task = tasks[0]
  requireThat(!checkpoint.task_id || checkpoint.task_id === task.id, 'original_task_replaced')
  checkpoint.task_id = uuid(task.id)
  requireThat(task.required_adapter === 'codex' && task.max_attempts === 2 && task.depth === 0 &&
    task.depends_on.length === 0 && task.contract_version === 1, 'native_single_task_contract_changed')
  const contract = task.contract
  requireThat(contract.source_repository === config.source.repository &&
    contract.source_base_ref === config.source.base_ref && contract.source_base_commit === config.source.base_commit &&
    contract.budget_tokens === request.budget_tokens &&
    contract.budget_cost_microusd === request.budget_cost_microusd &&
    contract.model === null && contract.reasoning_effort === null, 'task_source_or_budget_mismatch')
  exact(contract.secret_refs, [], 'unexpected_secret_authority')
  exact(contract.allowed_tools, request.contract.allowed_tools, 'unexpected_tool_authority')
  requireThat(contract.expected_output === request.contract.expected_output &&
    contract.objective === `${request.description}\n\nTASK-SPECIFIC OBJECTIVE:\n${request.contract.objective}` &&
    request.contract.acceptance_tests.every((item) => contract.acceptance_tests.includes(item)) &&
    request.contract.prohibited_actions.every((item) => contract.prohibited_actions.includes(item)) &&
    request.contract.references.every((item) => contract.references.includes(item)), 'native_contract_overlay_mismatch')
  exact(task.verification_policy, request.verification_policy, 'persisted_verifier_changed')
  const policies = policyDigests(task.verification_policy, contract.write_scope, contract.deliverable)
  if (checkpoint.contract_sha256) {
    requireThat(checkpoint.contract_sha256 === digest(contract), 'original_contract_changed')
  } else checkpoint.contract_sha256 = digest(contract)
  const agent = snap.agents.find((item) => item.id === task.assigned_agent_id)
  requireThat(agent?.adapter === 'codex' && agent.mission_id === mission.id, 'mission_owned_native_agent_required')
  requireThat(!checkpoint.agent_id || checkpoint.agent_id === agent.id, 'original_agent_replaced')
  checkpoint.agent_id = uuid(agent.id)
  const runs = snap.runs.filter((run) => run.task_id === task.id)
  requireThat(runs.length <= 1, 'automatic_retry_or_duplicate_run')
  const run = runs[0] ?? null
  if (checkpoint.run_id) requireThat(run?.id === checkpoint.run_id, 'original_run_missing_or_replaced')
  if (run) {
    requireThat(checkpoint.launch_attempted, 'run_started_without_driver_launch')
    checkpoint.run_id = uuid(run.id)
    // Retain safe live state even if the subsequent authority/proof check fails.
    requireThat(LIVE.has(run.status) || TERMINAL.has(run.status), 'unknown_run_state')
    requireThat(STAGES.includes(run.breaker_stage) &&
      [null, 'active', 'preserved', 'removed', 'quarantined'].includes(run.workspace_disposition),
    'unknown_run_projection_enum')
    checkpoint.latest = {
      status: run.status,
      breaker_stage: run.breaker_stage,
      input_tokens: integer(run.input_tokens, 0, Number.MAX_SAFE_INTEGER, 'invalid_persisted_usage'),
      output_tokens: integer(run.output_tokens, 0, Number.MAX_SAFE_INTEGER, 'invalid_persisted_usage'),
      cost_microusd: integer(run.cost_microusd, 0, Number.MAX_SAFE_INTEGER, 'invalid_persisted_usage'),
      workspace_disposition: run.workspace_disposition,
    }
    requireThat(run.agent_id === agent.id && run.runner_id === config.runner_id &&
      run.workspace_run_id === run.id && run.resumed_from_run_id === null &&
      run.source_repository === config.source.repository && run.source_base_ref === config.source.base_ref &&
      run.source_base_commit === config.source.base_commit && run.execution_mode === 'provider' &&
      run.model === null && run.reasoning_effort === null &&
      run.budget_tokens_limit === request.budget_tokens &&
      run.budget_cost_microusd_limit === request.budget_cost_microusd &&
      task.attempt_count === 1, 'original_run_identity_or_authority_changed')
  }
  return { snap, mission, task, agent, run, policies }
}

export function assessCase(state, replay, config, name, checkpoint) {
  const context = caseContext(state, config, name, checkpoint)
  requireThat(context?.run, 'original_run_not_visible')
  const { snap, mission, task, agent, run, policies } = context
  requireThat(['cancelled', 'failed'].includes(run.status) && run.breaker_stage === name &&
    run.workspace_disposition === 'preserved', 'expected_hard_terminal_checkpoint_missing')
  requireThat(task.status === run.status && mission.status === run.status &&
    agent.status === 'idle' && agent.current_run_id === null, 'terminal_projection_incoherent')
  requireThat(run.verification_status === 'pending' && task.verification_status === 'pending', 'verification_started_or_changed')
  for (const key of ['artifact_id', 'artifact_uri', 'artifact_sha256', 'artifact_signature',
    'artifact_media_type', 'verification_sha256', 'verification_summary', 'deliverable_sha256']) {
    requireThat(run[key] === null, 'accepted_artifact_or_verification_present')
  }
  for (const table of ABSENCE_TABLES) {
    requireThat(!snap[table].some((item) => item.run_id === run.id || item.task_id === task.id), 'accepted_evidence_record_present')
  }
  for (const table of ['mission_contract_revisions', 'mission_budget_revisions', 'action_approvals']) {
    requireThat(!snap[table].some((item) => item.mission_id === mission.id ||
      item.task_id === task.id || item.run_id === run.id), 'unexpected_authority_revision_or_approval')
  }
  const journal = replaySummary(replay.events, replay.through, config.corp_id)
  const events = replay.events.filter((event) => event.aggregate_id === run.id)
  requireThat(events.every((event) => event.aggregate_type === 'run' &&
    event.correlation_id === mission.id && event.room_id === mission.room_id), 'run_event_lineage_mismatch')
  const one = (type) => {
    const matches = events.filter((event) => event.type === type)
    requireThat(matches.length === 1, 'required_run_event_missing_or_duplicated')
    return matches[0]
  }
  const forbidden = new Set(['run.completed', 'run.artifact', 'run.artifact_upload',
    'run.deliverable', 'run.deliverable_upload', 'run.workspace_removed', 'run.teardown_uncertain'])
  requireThat(!events.some((event) => forbidden.has(event.type) || event.type.startsWith('run.verification_')),
    'forbidden_accepted_progress_or_uncertain_teardown')
  const requested = one('run.requested')
  requireThat(requested.actor_id === config.actor_id && requested.payload.mission_launch === true &&
    requested.payload.task_id === task.id && requested.payload.agent_id === agent.id &&
    requested.payload.runner_id === config.runner_id && requested.payload.attempt === 1 &&
    requested.payload.max_attempts === 2, 'native_launch_receipt_mismatch')
  const started = one('run.started')
  requireThat(started.payload.adapter === 'codex' && started.payload.mission_id === mission.id &&
    started.payload.task_id === task.id && started.payload.room_id === mission.room_id, 'native_start_receipt_mismatch')
  const session = one('run.session')
  uuid(session.payload.session_id) // This specific fixture uses a UUID, never a token.
  requireThat(session.payload.session_id === run.provider_session_id, 'provider_session_mismatch')
  const usage = events.filter((event) => event.type === 'run.usage')
  requireThat(usage.length === 2 && usage.every((event) => event.payload.input_tokens === 3_000 &&
    event.payload.output_tokens === 0 && event.payload.cost_microusd === 0), 'actual_fixture_usage_not_persisted')
  requireThat(run.input_tokens === 6_000 && run.output_tokens === 0 && run.cost_microusd === 0, 'usage_projection_not_exact')
  const transitions = events.filter((event) => event.type === 'run.breaker_transition')
  let rank = 0
  for (const event of transitions) {
    const next = STAGES.indexOf(event.payload.stage)
    requireThat(next > rank, 'nonmonotonic_breaker_history')
    rank = next
  }
  const hard = transitions.filter((event) => ['suspend', 'stop'].includes(event.payload.stage))
  requireThat(hard.length === 1 && hard[0].payload.stage === name, 'wrong_hard_breaker_stage')
  const breaker = hard[0]
  requireThat(['run_tokens', 'mission_tokens'].includes(breaker.payload.input?.metric) &&
    breaker.payload.input.used === 6_000 && breaker.payload.input.limit === CASES[name], 'wrong_breaker_metric_or_limit')
  uuid(breaker.payload.command_id)
  const ack = events.filter((event) => event.type === 'runner.command_acknowledged' &&
    event.payload.command_id === breaker.payload.command_id)
  requireThat(ack.length === 1 && ack[0].payload.command_kind === 'circuit_breaker' &&
    ack[0].payload.runner_id === config.runner_id && ack[0].seq > breaker.seq, 'native_breaker_command_not_acknowledged')
  const incident = snap.circuit_breaker_incidents.filter((item) => item.run_id === run.id && item.stage === name)
  requireThat(incident.length === 1 && incident[0].mission_id === mission.id &&
    incident[0].task_id === task.id, 'persisted_hard_breaker_incident_missing')
  exact(incident[0].input, breaker.payload.input, 'breaker_incident_input_mismatch')
  const terminated = one('run.session_terminated')
  requireThat(terminated.payload.adapter === 'codex' && terminated.payload.provider_process_alive === false &&
    ['completed', 'cancelled', 'failed'].includes(terminated.payload.outcome), 'native_provider_termination_unproven')
  const preserved = one('run.workspace_preserved')
  const terminal = events.filter((event) => ['run.failed', 'run.cancelled', 'run.lost'].includes(event.type))
  requireThat(terminal.length === 1 && terminal[0].type === `run.${run.status}`, 'terminal_event_mismatch')
  requireThat(requested.seq < started.seq && started.seq < usage[0].seq &&
    usage[1].seq < breaker.seq && breaker.seq < terminated.seq &&
    terminated.seq < preserved.seq && preserved.seq < terminal[0].seq, 'usage_breaker_termination_checkpoint_terminal_order')
  const proof = preserved.payload.source_checkpoint
  object(proof, 'native_source_checkpoint_missing')
  const expectedBranch = `crony/task-${task.id.replaceAll('-', '')}/run-${run.id.replaceAll('-', '')}`
  const identity = {
    schema_version: 1,
    corp_id: config.corp_id,
    mission_id: mission.id,
    task_id: task.id,
    run_id: run.id,
    workspace_run_id: run.id,
    agent_id: agent.id,
    runner_id: config.runner_id,
    source_repository: config.source.repository,
    source_base_ref: config.source.base_ref,
    source_base_commit: config.source.base_commit,
    workspace_base_commit: config.source.base_commit,
    branch: expectedBranch,
    head_commit: config.source.base_commit,
    workspace_fingerprint: hashString(run.workspace_fingerprint),
    ...policies,
  }
  exact(proof, identity, 'source_checkpoint_identity_head_or_policy_digest_mismatch')
  for (const payload of [started.payload, preserved.payload]) {
    requireThat(payload.workspace_base_ref === config.source.base_ref &&
      payload.workspace_base_commit === config.source.base_commit &&
      payload.workspace_branch === expectedBranch &&
      samePath(payload.workspace, run.workspace_path), 'workspace_receipt_mismatch')
  }
  requireThat(run.workspace_branch === expectedBranch &&
    run.workspace_base_ref === config.source.base_ref && run.workspace_base_commit === config.source.base_commit &&
    preserved.payload.workspace_fingerprint === proof.workspace_fingerprint &&
    preserved.payload.head_commit === config.source.base_commit && preserved.payload.branch_deleted === false,
  'persisted_workspace_checkpoint_mismatch')
  const workspace = expectedWorkspace(config, task.id, run.id)
  requireThat(samePath(workspace, run.workspace_path), 'workspace_not_exact_generated_task_run_path')
  return {
    source_checkpoint: identity,
    provider_session_sha256: byteDigest(run.provider_session_id),
    budget_tokens: CASES[name],
    persisted_usage: { input_tokens: 6_000, output_tokens: 0, cost_microusd: 0 },
    breaker_stage: name,
    terminal_status: run.status,
    provider_outcome: terminated.payload.outcome,
    ordering: {
      requested: requested.seq, started: started.seq, usage: usage.map((event) => event.seq),
      breaker: breaker.seq, acknowledged: ack[0].seq, session_terminated: terminated.seq,
      workspace_preserved: preserved.seq, terminal: terminal[0].seq,
    },
    policy_digests: policies,
    journal_through: journal.through,
    run_event_count: events.length,
    task_run_count: 1,
    attempt_count: 1,
    accepted_provider_artifacts: 0,
    accepted_source_deliverables: 0,
    verification_started: false,
    accepted_completion: false,
    base_file: null,
    quiet_observation_ms: 0,
  }
}

export function expectedWorkspace(config, taskId, runId, pathApi = path) {
  uuid(taskId)
  uuid(runId)
  return containedPath(config.runner_root, pathApi.join(config.runner_root, 'worktrees',
    taskId.replaceAll('-', ''), runId.replaceAll('-', '')), pathApi)
}

async function canonicalDirectory(value, io = fs) {
  const declared = localAbsolute(value)
  const info = await io.lstat(declared)
  requireThat(info.isDirectory() && !info.isSymbolicLink(), 'directory_is_link_or_not_directory')
  const resolved = localAbsolute(await io.realpath(declared))
  requireThat(samePath(declared, resolved), 'directory_alias_not_canonical')
  return resolved
}
export async function checkContainedFile(root, target, io = fs) {
  const canonicalRoot = await canonicalDirectory(root, io)
  const file = containedPath(canonicalRoot, target)
  const parts = path.relative(canonicalRoot, file).split(path.sep)
  let current = canonicalRoot
  for (let index = 0; index < parts.length; index += 1) {
    current = path.join(current, parts[index])
    const info = await io.lstat(current)
    requireThat(!info.isSymbolicLink(), 'workspace_link_rejected')
    const resolved = containedPath(canonicalRoot, await io.realpath(current))
    requireThat(samePath(resolved, current), 'workspace_reparse_or_alias_rejected')
    if (index === parts.length - 1) {
      requireThat(info.isFile() && info.nlink === 1, 'regular_single_link_file_required')
    } else requireThat(info.isDirectory(), 'workspace_parent_not_directory')
  }
  return file
}
async function readBoundedFile(file, maxBytes, io = fs) {
  const before = await io.lstat(file)
  requireThat(before.isFile() && !before.isSymbolicLink() && before.nlink === 1 &&
    before.size <= maxBytes, 'bounded_regular_file_required')
  const handle = await io.open(file, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0))
  try {
    const opened = await handle.stat()
    requireThat(opened.isFile() && opened.nlink === 1 && opened.dev === before.dev &&
      opened.ino === before.ino && opened.size === before.size, 'opened_file_identity_changed')
    const bytes = Buffer.alloc(Math.min(maxBytes, opened.size) + 1)
    let used = 0
    while (used < bytes.length) {
      const result = await handle.read(bytes, used, bytes.length - used, used)
      if (!result.bytesRead) break
      used += result.bytesRead
    }
    const after = await handle.stat()
    requireThat(used <= maxBytes && used === opened.size && after.size === opened.size &&
      after.mtimeMs === opened.mtimeMs && after.nlink === 1, 'file_changed_or_exceeded_bound')
    return bytes.subarray(0, used)
  } finally { await handle.close() }
}
export async function readBaseFile(config, checkpoint, io = fs) {
  const workspace = expectedWorkspace(config, checkpoint.task_id, checkpoint.run_id)
  const file = await checkContainedFile(config.runner_root, path.join(workspace, 'base.txt'), io)
  const bytes = await readBoundedFile(file, 5, io)
  await checkContainedFile(config.runner_root, file, io)
  requireThat(bytes.equals(Buffer.from('base\n')), 'base_file_not_exact_fixture_bytes')
  return { path: file, bytes: 5, sha256: BASE_SHA256 }
}

/** No token headers, redirects, unknown routes, or automatic POST retries. */
export function createApi(config, { fetchImpl = globalThis.fetch, WebSocketImpl = globalThis.WebSocket,
  now = Date.now } = {}) {
  let requests = 0
  const deadline = now() + config.timeout_ms
  function budget() {
    requireThat(++requests <= LIMITS.requests && now() < deadline, 'api_time_or_request_bound')
    return Math.max(1, Math.min(LIMITS.request_ms, deadline - now()))
  }
  const base = `/api/corps/${config.corp_id}`
  const snapshotRoute = `${base}/snapshot?actor_id=${config.actor_id}`
  async function request(route, body) {
    const isRead = body === undefined && route === snapshotRoute
    const isCreate = body !== undefined && route === `${base}/missions`
    const isLaunch = body !== undefined && new RegExp(`^${base}/missions/[0-9a-f-]{36}/launch$`, 'u').test(route)
    requireThat(isRead || isCreate || isLaunch, 'api_route_outside_additive_allowlist')
    const response = await fetchImpl(`${config.server_url}${route}`, {
      method: isRead ? 'GET' : 'POST',
      redirect: 'error',
      headers: { 'content-type': 'application/json' },
      body: isRead ? undefined : JSON.stringify(body),
      signal: AbortSignal.timeout(budget()),
    })
    if (response.status !== 200 || !response.body) {
      await response.body?.cancel().catch(() => {})
      throw new CheckFailure('api_response_not_ok_body_withheld')
    }
    const reader = response.body.getReader()
    let length = 0
    const chunks = []
    try {
      while (true) {
        const { done, value } = await reader.read()
        if (done) break
        length += value.length
        requireThat(length <= LIMITS.response_bytes, 'api_response_byte_bound')
        chunks.push(value)
      }
    } finally { await reader.cancel().catch(() => {}) }
    try { return JSON.parse(Buffer.concat(chunks).toString('utf8')) } catch {
      throw new CheckFailure('api_json_invalid_body_withheld')
    }
  }
  function replay() {
    const timeoutMs = budget()
    return new Promise((resolve, reject) => {
      let socket
      let settled = false
      let size = 0
      const events = []
      const done = (error, value) => {
        if (settled) return
        settled = true
        clearTimeout(timer)
        if (socket) {
          socket.onmessage = null
          socket.onclose = null
          socket.onerror = () => {}
          socket.close()
        }
        if (error) reject(error)
        else resolve(value)
      }
      const timer = setTimeout(() => done(new CheckFailure('journal_replay_timeout')), timeoutMs)
      try {
        socket = new WebSocketImpl(`${config.server_url.replace(/^http:/u, 'ws:')}/ws/corps/${config.corp_id}?actor_id=${config.actor_id}&after_seq=0`)
        socket.onerror = () => done(new CheckFailure('journal_transport_failed'))
        socket.onclose = () => done(new CheckFailure('journal_closed_before_ready'))
        socket.onmessage = (message) => {
          if (settled) return
          try {
            requireThat(typeof message.data === 'string', 'journal_text_frame_required')
            size += Buffer.byteLength(message.data)
            requireThat(size <= LIMITS.replay_bytes, 'journal_byte_bound')
            let frame
            try { frame = JSON.parse(message.data) } catch { throw new CheckFailure('journal_json_invalid') }
            if (frame.type === 'event') {
              requireThat(events.length < LIMITS.replay_events, 'journal_event_bound')
              events.push(frame.event)
            } else {
              requireThat(frame.type === 'ready' && frame.corp_id === config.corp_id, 'journal_ready_scope_mismatch')
              replaySummary(events, frame.replayed_through, config.corp_id)
              done(null, { events, through: frame.replayed_through })
            }
          } catch (error) { done(new CheckFailure(safeCode(error))) }
        }
      } catch { done(new CheckFailure('journal_open_failed')) }
    })
  }
  return {
    request,
    snapshot: () => request(snapshotRoute),
    create: (name) => request(`${base}/missions`, caseRequest(config, name)),
    launch: (missionId) => request(`${base}/missions/${uuid(missionId)}/launch`, { requested_by: config.actor_id }),
    replay,
  }
}

/**
 * Injectable boundary used by PURE unit tests: no fake services, disk fixtures,
 * credentials, environment or provider execution required to test the driver.
 */
export async function executeSuite(config, report, io) {
  const deadline = io.now() + config.timeout_ms
  const save = async () => {
    report.updated_at = new Date(io.now()).toISOString()
    await io.save(report)
  }
  async function observe(name, checkpoint) {
    const state = await io.snapshot()
    assertHistory(report.baseline, state, null, config)
    const context = caseContext(state, config, name, checkpoint)
    await save()
    return { state, context }
  }
  // Even a failed POST may already have committed. One bounded read can discover
  // its original IDs, but this path never sends a second mutation.
  async function reconcileFailure(name, checkpoint) {
    try {
      const state = await io.snapshot()
      caseContext(state, config, name, checkpoint)
    } catch { /* Exact saved intent remains ambiguous; parent must reconcile. */ }
    await save()
  }
  function observeJournal(replay, checkpoint) {
    replaySummary(replay.events, replay.through, config.corp_id)
    checkpoint.journal_observation = {
      through: replay.through,
      events: replay.events.filter((event) => event.aggregate_id === checkpoint.run_id)
        .map((event) => ({ seq: event.seq, id: event.id, type: event.type,
          payload_sha256: digest(event.payload) })),
    }
  }
  try {
    report.passed = false
    report.status = 'incomplete'
    report.error_code = null
    if (!report.baseline) {
      requireThat(Object.values(report.cases).every((item) => !item.create_attempted), 'missing_original_history_baseline')
      const state = await io.snapshot()
      assertRunner(state, config)
      for (const name of Object.keys(CASES)) caseContext(state, config, name, report.cases[name])
      const replay = await io.replay()
      report.baseline = historySummary(state, replay, config)
      const after = await io.snapshot()
      assertHistory(report.baseline, after, null, config)
      await save()
    } else {
      const state = await io.snapshot()
      const replay = await io.replay()
      assertHistory(report.baseline, state, replay, config)
    }
    for (const name of Object.keys(CASES)) {
      const checkpoint = report.cases[name]
      let { state, context } = await observe(name, checkpoint)
      if (!context) {
        requireThat(!checkpoint.create_attempted, 'create_intent_ambiguous_no_replacement')
        assertRunner(state, config)
        requireThat(state.snapshot.runs.every((run) => TERMINAL.has(run.status)), 'live_run_blocks_new_case')
        checkpoint.create_attempted = true
        checkpoint.phase = 'create_intent'
        await save()
        try {
          const created = await io.create(name)
          // Capture valid IDs before checking response coherence.
          checkpoint.mission_id = uuid(created.mission_id)
          checkpoint.task_id = uuid(created.task_id)
          await save()
          exact(created.task_ids, [checkpoint.task_id], 'create_did_not_return_single_task')
          requireThat(created.strategy === 'single', 'create_strategy_mismatch')
        } catch (error) {
          await reconcileFailure(name, checkpoint)
          throw error
        }
        checkpoint.phase = 'created'
        ;({ state, context } = await observe(name, checkpoint))
      }
      requireThat(context, 'created_mission_not_visible')
      if (!context.run) {
        requireThat(!checkpoint.launch_attempted, 'launch_intent_ambiguous_do_not_relaunch')
        requireThat(context.mission.status === 'ready' && context.task.status === 'ready' &&
          context.task.attempt_count === 0 && context.agent.status === 'idle' &&
          context.agent.current_run_id === null, 'original_plan_not_never_launched_ready')
        assertRunner(state, config)
        checkpoint.launch_attempted = true
        checkpoint.phase = 'launch_intent'
        await save()
        try {
          const launched = await io.launch(checkpoint.mission_id)
          checkpoint.run_id = uuid(launched.run_id)
          await save()
          exact(launched.run_ids, [checkpoint.run_id], 'launch_did_not_return_single_run')
          exact(launched.runner_ids, [config.runner_id], 'launch_runner_mismatch')
          requireThat(launched.runner_id === config.runner_id && launched.replayed === false, 'unexpected_launch_replay')
        } catch (error) {
          await reconcileFailure(name, checkpoint)
          throw error
        }
        checkpoint.phase = 'launched'
        await save()
      }
      let stableSince = null
      let terminalDigest = null
      while (io.now() < deadline) {
        ;({ state, context } = await observe(name, checkpoint))
        requireThat(context?.run, 'original_run_not_visible')
        if (TERMINAL.has(context.run.status)) {
          checkpoint.phase = 'terminal'
          const current = digest({ run: context.run, task: context.task, mission: context.mission })
          if (stableSince === null) {
            stableSince = io.now()
            terminalDigest = current
          } else requireThat(current === terminalDigest, 'terminal_case_mutated_during_observation')
          if (io.now() - stableSince >= config.settle_ms) {
            const replay = await io.replay()
            // Snapshot AFTER Ready must retain the exact terminal row and no retry.
            ;({ state, context } = await observe(name, checkpoint))
            requireThat(digest({ run: context.run, task: context.task, mission: context.mission }) === terminalDigest,
              'terminal_case_mutated_after_replay')
            assertHistory(report.baseline, state, replay, config)
            observeJournal(replay, checkpoint)
            await save()
            const evidence = assessCase(state, replay, config, name, checkpoint)
            evidence.base_file = await io.readBase(checkpoint)
            exact({ bytes: evidence.base_file.bytes, sha256: evidence.base_file.sha256 },
              { bytes: 5, sha256: BASE_SHA256 }, 'local_base_evidence_mismatch')
            evidence.quiet_observation_ms = io.now() - stableSince
            checkpoint.evidence = evidence
            checkpoint.phase = 'passed'
            await save()
            break
          }
        } else {
          checkpoint.phase = 'live'
          requireThat(stableSince === null, 'terminal_case_became_live_again')
        }
        await save()
        await io.sleep(Math.min(config.poll_ms, Math.max(0, deadline - io.now())))
      }
      requireThat(checkpoint.phase === 'passed', 'observation_timeout_run_not_reclassified')
    }
    const replay = await io.replay()
    const finalState = await io.snapshot()
    assertHistory(report.baseline, finalState, replay, config)
    for (const name of Object.keys(CASES)) {
      const checkpoint = report.cases[name]
      observeJournal(replay, checkpoint)
      const evidence = assessCase(finalState, replay, config, name, checkpoint)
      evidence.base_file = await io.readBase(checkpoint)
      evidence.quiet_observation_ms = checkpoint.evidence.quiet_observation_ms
      checkpoint.evidence = evidence
    }
    report.passed = true
    report.status = 'passed'
    await save()
    return report
  } catch (error) {
    report.passed = false
    report.status = Object.values(report.cases).some((item) => LIVE.has(item.latest?.status)) ? 'incomplete_live' : 'failed'
    report.error_code = safeCode(error)
    await save()
    throw new CheckFailure(report.error_code)
  }
}

export function validateSavedReport(saved, config) {
  const fresh = newReport(config)
  keys(saved, Object.keys(fresh), 'invalid_report_schema')
  requireThat(saved.schema_version === 1 && saved.suite === SUITE &&
    saved.receipt_id === config.receipt_id && saved.receipt_sha256 === receiptIdentity(config),
  'continuation_receipt_mismatch')
  requireThat(typeof saved.started_at === 'string' && Number.isFinite(Date.parse(saved.started_at)), 'invalid_report_time')
  keys(saved.cases, Object.keys(CASES), 'invalid_saved_cases')
  for (const name of Object.keys(CASES)) {
    const item = saved.cases[name]
    keys(item, Object.keys(fresh.cases[name]), 'invalid_saved_case_fields')
    requireThat(item.budget_tokens === CASES[name] && typeof item.create_attempted === 'boolean' &&
      typeof item.launch_attempted === 'boolean' && (!item.launch_attempted || item.create_attempted), 'invalid_saved_intent')
    for (const field of ['mission_id', 'task_id', 'agent_id', 'run_id']) if (item[field] !== null) uuid(item[field])
    requireThat(!item.run_id || item.launch_attempted, 'saved_run_without_launch_intent')
    if (item.contract_sha256 !== null) hashString(item.contract_sha256)
    requireThat(['new', 'create_intent', 'created', 'launch_intent', 'launched', 'live', 'terminal', 'passed'].includes(item.phase),
      'invalid_saved_phase')
    // Recompute ALL live observations and proof, never trust a saved "passed".
    item.latest = null
    item.journal_observation = null
    item.evidence = null
    if (item.phase === 'passed') item.phase = 'launched'
  }
  if (saved.baseline !== null) {
    keys(saved.baseline, ['rows', 'journal'], 'invalid_saved_baseline')
    keys(saved.baseline.rows, ROWS, 'invalid_saved_baseline_rows')
    for (const table of ROWS) {
      const rows = saved.baseline.rows[table]
      requireThat(Array.isArray(rows) && rows.length <= LIMITS.snapshot_rows, 'invalid_saved_history_bound')
      const seen = new Set()
      for (const row of rows) {
        keys(row, ['id', 'sha256'], 'invalid_saved_history_row')
        uuid(row.id)
        hashString(row.sha256)
        requireThat(!seen.has(row.id), 'duplicate_saved_history_row')
        seen.add(row.id)
      }
    }
    keys(saved.baseline.journal, ['through', 'event_count', 'sha256'], 'invalid_saved_journal')
    integer(saved.baseline.journal.through, 0, Number.MAX_SAFE_INTEGER, 'invalid_saved_watermark')
    integer(saved.baseline.journal.event_count, 0, LIMITS.replay_events, 'invalid_saved_event_count')
    hashString(saved.baseline.journal.sha256)
  }
  return { ...fresh, started_at: new Date(saved.started_at).toISOString(), baseline: saved.baseline, cases: saved.cases }
}

async function exists(file) {
  try { await fs.lstat(file); return true } catch (error) {
    if (error.code === 'ENOENT') return false
    throw error
  }
}

export async function main(argv = process.argv.slice(2)) {
  let reportPath = null
  let lock = null
  let lockPath = null
  try {
    const options = parseArgs(argv)
    if (options.schema) {
      console.log(JSON.stringify(receiptSchema(), null, 2))
      return 0
    }
    const config = configurationFromReceipt(
      JSON.parse((await readBoundedFile(options.receiptPath, LIMITS.receipt_bytes)).toString('utf8')), options)
    const runnerRoot = await canonicalDirectory(config.runner_root)
    const outputDir = await canonicalDirectory(config.output_dir)
    requireThat(samePath(runnerRoot, config.runner_root) && samePath(outputDir, config.output_dir), 'canonical_roots_required')
    reportPath = config.report_path
    requireThat(!samePath(reportPath, options.receiptPath), 'report_cannot_replace_receipt')
    lockPath = `${reportPath}.lock`
    lock = await fs.open(lockPath, 'wx', 0o600)
    await lock.writeFile(`${JSON.stringify({ suite: SUITE, receipt_id: config.receipt_id })}\n`)
    await lock.sync()
    const alreadyExists = await exists(reportPath)
    requireThat(alreadyExists === options.continuation, alreadyExists ? 'report_exists_use_explicit_continue' : 'continuation_report_missing')
    let report
    if (alreadyExists) {
      const bytes = await readBoundedFile(reportPath, LIMITS.report_bytes)
      report = validateSavedReport(JSON.parse(bytes.toString('utf8')), config)
      // Preserve the previous complete JSON before refreshing any saved evidence.
      const priorPath = `${reportPath}.previous-${randomUUID()}.json`
      const prior = await fs.open(priorPath, 'wx', 0o600)
      try { await prior.writeFile(bytes); await prior.sync() } finally { await prior.close() }
    } else report = newReport(config)
    async function save(value) {
      await canonicalDirectory(outputDir)
      if (await exists(reportPath)) await readBoundedFile(reportPath, LIMITS.report_bytes)
      const encoded = `${JSON.stringify(value, null, 2)}\n`
      requireThat(Buffer.byteLength(encoded) <= LIMITS.report_bytes, 'report_byte_bound')
      const tempPath = `${reportPath}.${randomUUID()}.tmp`
      const temp = await fs.open(tempPath, 'wx', 0o600)
      try { await temp.writeFile(encoded); await temp.sync() } finally { await temp.close() }
      await canonicalDirectory(outputDir)
      await fs.rename(tempPath, reportPath)
    }
    await save(report)
    const api = createApi(config)
    await executeSuite(config, report, {
      ...api,
      now: Date.now,
      sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
      readBase: (checkpoint) => readBaseFile(config, checkpoint),
      save,
    })
    console.log(JSON.stringify({ suite: SUITE, passed: true, report_path: reportPath,
      run_ids: Object.values(report.cases).map((item) => item.run_id) }))
    return 0
  } catch (error) {
    console.error(JSON.stringify({ suite: SUITE, passed: false, error_code: safeCode(error),
      report_path: reportPath, instruction: 'Retain all existing work. Inspect the checkpoint; continue intentionally, never replace a live or uncertain case.' }))
    return 1
  } finally {
    if (lock) {
      await lock.close()
      await fs.unlink(lockPath)
    }
  }
}

// Importing helpers is inert: neither filesystem nor network work starts here.
if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  process.exitCode = await main()
}
