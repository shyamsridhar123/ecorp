#!/usr/bin/env node
/**
 * #224 public planned-attempt acceptance. Integrate as tools/e2e_planned_attempts.mjs.
 *
 * ONE Factory lifecycle: native provider attempt 1 -> provider-free checkpoint
 * failure -> provider correction 2 with genuine verification_failed -> provider
 * correction 3 with the existing [checkpoint-app] fixture -> Bob's independent
 * decision. This is Codex protocol-fixture execution, NOT vendor inference,
 * vendor session-persistence proof, or a run.failed transport-failure scenario.
 *
 * The parent provisions and verifies the owned server/runner/database. This
 * script never inspects a DB, resets/bootstrap state, changes an allowance after
 * creation, starts/stops a service, opens a browser, invokes Git/GitHub, or reads
 * provider homes/worktrees. All filesystem writes stay in a NEW evidence child.
 *
 * Required:
 *   CRONY_SERVER_HTTP = http://127.0.0.1:19084 (exactly)
 *   CRONY_CASE_DIR = an existing parent-owned directory outside this checkout
 *   CRONY_TEST_CONTEXT = public JSON, or a context-file path inside CRONY_CASE_DIR
 * The context contains test_owned, workspace, server, corp_id, alice_actor_id,
 * bob_actor_id, runner_id, source { repository, base_ref, base_commit }.
 * An optional guest_actor_id selects an existing guest; otherwise discover one
 * from the authorized public snapshot. A missing guest fails before creation.
 * Optional allow_fixture_publication: true records review-only publication policy
 * before claim so a separate owned publisher drill can continue the accepted case.
 * This driver never publishes; omitted/false retains the original policy.
 *
 * No automatic POST retries and no cleanup/reset on failure. Intent, observations,
 * failed HTTP responses and the final result are retained for parent inspection.
 * A failed/uncertain mutation must be investigated before another invocation.
 */
import { createHash, randomUUID } from 'node:crypto'
import { lstat, mkdir, readFile, realpath, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const SERVER = 'http://127.0.0.1:19084'
const WORKSPACE = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const REPOSITORY = 'ecorp-fixture/planned-attempts-fixture'
const APPLICATION = 'qa/issue195'
const MISSION_TOKENS = 10_000
const TASK_TOKENS = 5_700 // 6,000 / 5,000 is STOP, not the permitted suspension.
const COST_LIMIT = 1_000_000
const ATTEMPTS = 3
const JSON_LIMIT = 8 * 1024 * 1024
const ARTIFACT_LIMIT = 16 * 1024 * 1024
const ACTIVE = new Set(['provisioning', 'starting', 'running', 'waiting_for_input', 'verifying', 'waiting_for_approval'])
const ROWS = [
  'actors', 'rooms', 'agents', 'missions', 'tasks', 'runs', 'room_messages',
  'queued_messages', 'mission_contract_revisions', 'mission_budget_revisions',
  'factory_work_items', 'factory_verification_recoveries', 'verification_evidence',
  'verification_requests', 'source_deliverables', 'action_approvals',
  'circuit_breaker_incidents', 'pull_request_publications', 'pull_request_publication_attempts',
]
const PRIVATE_KEY = /^(claim_token|assignment_token|publisher_token|lease_token|enrollment_token|credential|credential_hash|authorization)$/u
const privateValues = new Set()

class LaneError extends Error {
  constructor(code, details = {}) {
    super(code)
    this.name = 'LaneError'
    this.code = code
    this.details = details
  }
}
function requireThat(condition, code, details = {}) {
  if (!condition) throw new LaneError(code, details)
}
function clone(value) { return structuredClone(value) }
function canonical(value) {
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`
  if (value !== null && typeof value === 'object') {
    return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`).join(',')}}`
  }
  return JSON.stringify(value)
}
function digest(value) { return createHash('sha256').update(canonical(value)).digest('hex') }
function byteDigest(value) { return createHash('sha256').update(value).digest('hex') }
function equal(actual, expected, code) {
  requireThat(canonical(actual) === canonical(expected), code, { actual, expected })
}
function uuid(value, code = 'uuid_required') {
  requireThat(typeof value === 'string' &&
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/iu.test(value) &&
    value !== '00000000-0000-0000-0000-000000000000', code)
  return value
}
function sha(value, length, code) {
  requireThat(typeof value === 'string' && new RegExp(`^[0-9a-f]{${length}}$`, 'u').test(value), code)
  return value
}
function pathKey(value) {
  return path.resolve(value).replace(/^\\\\\?\\/u, '').replace(/[\\/]+$/u, '').toLowerCase()
}
function within(root, candidate) {
  const relative = path.relative(root, candidate)
  return relative === '' || (!path.isAbsolute(relative) && relative !== '..' &&
    !relative.startsWith(`..${path.sep}`))
}
function rememberPrivate(value) {
  if (!value || typeof value !== 'object') return
  for (const [key, item] of Object.entries(value)) {
    if (PRIVATE_KEY.test(key) && typeof item === 'string' && item.length >= 16) privateValues.add(item)
    else rememberPrivate(item)
  }
}
function redact(value) {
  if (typeof value === 'string') {
    for (const secret of privateValues) value = value.split(secret).join('[REDACTED]')
    return value
  }
  if (Array.isArray(value)) return value.map(redact)
  if (value && typeof value === 'object') {
    return Object.fromEntries(Object.entries(value).map(([key, item]) =>
      [key, PRIVATE_KEY.test(key) ? '[REDACTED]' : redact(item)]))
  }
  return value
}
function rowKey(row) { return row.id ?? row.run_id ?? row.agent_id ?? canonical(row) }
function rows(snapshot, name) {
  requireThat(Array.isArray(snapshot[name]), 'public_snapshot_collection_missing', { name })
  return snapshot[name]
}
function one(items, predicate, code) {
  const selected = items.filter(predicate)
  requireThat(selected.length === 1, code, { matches: selected.length })
  return selected[0]
}
function stable(snapshot) {
  // Runner heartbeats/presence are outside these authoritative collections.
  // Include journal events and all application rows; do not ignore task counters.
  return Object.fromEntries([...ROWS, 'events'].map((name) =>
    [name, clone(rows(snapshot, name)).sort((a, b) => String(rowKey(a)).localeCompare(String(rowKey(b))))]))
}
function usage(run) { return run.input_tokens + run.output_tokens }
const pause = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds))
function stripBudgetMarkers(value) {
  if (typeof value === 'string') return value.replace(/\[budget-stream(?:-ui)?\]/gu, '').trim()
  if (Array.isArray(value)) return value.map(stripBudgetMarkers)
  return value
}
function hasBudgetMarker(value) {
  return /\[budget-stream(?:-ui)?\]/u.test(canonical(value))
}

async function configuration() {
  requireThat(process.platform === 'win32', 'this_lane_requires_the_assigned_windows_host')
  requireThat(process.env.CRONY_SERVER_HTTP === SERVER, 'exact_owned_server_required')
  const suppliedCase = process.env.CRONY_CASE_DIR
  requireThat(typeof suppliedCase === 'string' && path.isAbsolute(suppliedCase), 'explicit_absolute_case_directory_required')
  requireThat(!within(WORKSPACE, path.resolve(suppliedCase)), 'evidence_must_stay_outside_source_checkout')
  const caseStat = await lstat(suppliedCase)
  requireThat(caseStat.isDirectory() && !caseStat.isSymbolicLink(), 'case_directory_must_be_existing_non_link_directory')
  const caseDir = await realpath(suppliedCase)
  requireThat(pathKey(caseDir) === pathKey(suppliedCase) && !within(WORKSPACE, caseDir),
    'case_directory_canonical_identity_changed')

  const suppliedContext = process.env.CRONY_TEST_CONTEXT
  requireThat(typeof suppliedContext === 'string' && suppliedContext.trim(), 'explicit_public_context_required')
  let encoded = suppliedContext.trim()
  let contextPath = null
  if (!encoded.startsWith('{')) {
    contextPath = path.resolve(caseDir, encoded)
    requireThat(within(caseDir, contextPath), 'context_file_outside_owned_case_directory')
    const info = await lstat(contextPath)
    requireThat(info.isFile() && !info.isSymbolicLink() && info.nlink === 1 && info.size <= 65_536,
      'context_file_must_be_bounded_regular_file')
    requireThat(pathKey(await realpath(contextPath)) === pathKey(contextPath), 'context_file_identity_changed')
    encoded = await readFile(contextPath, 'utf8')
  }
  requireThat(Buffer.byteLength(encoded) <= 65_536, 'context_size_bound')
  const context = JSON.parse(encoded.replace(/^\uFEFF/u, ''))
  requireThat(context && typeof context === 'object' && !Array.isArray(context), 'context_object_required')
  const allowed = new Set(['test_owned', 'workspace', 'server', 'corp_id', 'alice_actor_id', 'bob_actor_id', 'guest_actor_id', 'runner_id', 'source', 'allow_fixture_publication'])
  requireThat(Object.keys(context).every((key) => allowed.has(key)), 'unexpected_context_fields')
  requireThat(context.test_owned === true && context.server === SERVER, 'owned_context_server_mismatch')
  requireThat(context.allow_fixture_publication === undefined || typeof context.allow_fixture_publication === 'boolean',
    'fixture_publication_option_must_be_boolean')
  requireThat(typeof context.workspace === 'string' && pathKey(context.workspace) === pathKey(WORKSPACE),
    'assigned_worktree_context_required')
  // Do not resolve/read an arbitrary context-supplied workspace before checking it.
  requireThat(pathKey(await realpath(WORKSPACE)) === pathKey(WORKSPACE), 'assigned_worktree_identity_changed')
  requireThat(pathKey(fileURLToPath(import.meta.url)) === pathKey(path.join(WORKSPACE, 'tools', 'e2e_planned_attempts.mjs')),
    'integrate_draft_into_assigned_worktree_before_execution')
  for (const field of ['corp_id', 'alice_actor_id', 'bob_actor_id']) uuid(context[field], `context_${field}_invalid`)
  requireThat(context.alice_actor_id !== context.bob_actor_id, 'independent_reviewer_required')
  if (context.guest_actor_id !== undefined) uuid(context.guest_actor_id, 'guest_actor_id_invalid')
  requireThat(typeof context.runner_id === 'string' && /^[a-zA-Z0-9_.:-]{1,200}$/u.test(context.runner_id),
    'explicit_runner_id_required')
  const source = context.source
  requireThat(source && Object.keys(source).sort().join(',') === 'base_commit,base_ref,repository',
    'exact_source_tuple_required')
  requireThat(source.repository === REPOSITORY && source.base_ref === 'main', 'owned_source_tuple_mismatch')
  sha(source.base_commit, 40, 'fresh_source_commit_required')
  return { context, contextPath, caseDir }
}

class Harness {
  constructor(config, output, tag) {
    this.context = config.context
    this.output = output
    this.tag = tag
    this.sequence = 0
    this.phase = 'initialization'
    this.ids = {}
    this.frozen = new Map()
    this.frozenEvents = new Map()
    this.events = new Map()
    this.checks = []
    this.observeGuard = () => {}
    this.lastState = null
    this.report = {
      suite: 'issue224-public-planned-attempts',
      status: 'running',
      started_at: new Date().toISOString(),
      workspace: WORKSPACE,
      server: SERVER,
      context: clone(config.context),
      context_path: config.contextPath,
      evidence_directory: output,
      scope: {
        native_server_runner: true,
        provider_mode: 'codex_protocol_fixture',
        real_vendor_inference: false,
        vendor_session_persistence_proven: false,
        correction_failure_type: 'verification_failed_after_provider_completion',
        ordinary_run_failed_transport_failure: false,
        browser_exercised: false,
        database_ownership: 'parent_attested_context_no_database_access',
        history: 'public_rows_native_journal_and_source_bindings_not_private_DB_row_audit',
        independent_signature_verification_by_client: false,
        SQL_or_counter_repairs: false,
        publication_policy_declared_before_claim: config.context.allow_fixture_publication === true,
      },
      checks: this.checks,
      ids: this.ids,
    }
  }
  async save(label, value) {
    requireThat(/^[a-z0-9-]{1,90}$/u.test(label), 'internal_evidence_label_invalid')
    const file = `${String(++this.sequence).padStart(4, '0')}-${label}.json`
    await writeFile(path.join(this.output, file), `${JSON.stringify(redact(value), null, 2)}\n`,
      { flag: 'wx', mode: 0o600 })
    return file
  }
  url(route) {
    requireThat(typeof route === 'string' && route.startsWith('/') && !route.startsWith('//'), 'native_relative_route_required')
    const url = new URL(route, SERVER)
    const prefix = `/api/corps/${this.context.corp_id}`
    requireThat(url.origin === SERVER && url.username === '' && url.password === '' && url.hash === '',
      'foreign_or_redirected_server_forbidden')
    const tail = url.pathname.slice(prefix.length)
    const id = '[0-9a-fA-F-]{36}'
    const permitted = new RegExp(
      `^(?:/snapshot|/missions/preview|/factory/preflight|/factory/work-items/claim|` +
      `/factory/work-items/${id}/(?:renew|materialize|transition|verification-recoveries)|` +
      `/missions/${id}/(?:launch|contract-revisions)|/runs/${id}/verification-decision|/artifacts/${id})$`, 'u')
    requireThat(url.pathname.startsWith(prefix) && permitted.test(tail), 'route_outside_public_acceptance_scope', { route })
    return url
  }
  async bodyBytes(response, limit) {
    const declared = response.headers.get('content-length')
    requireThat(declared === null || (Number.isSafeInteger(Number(declared)) && Number(declared) <= limit),
      'response_size_bound')
    const reader = response.body?.getReader()
    requireThat(reader, 'response_body_missing')
    let length = 0
    const chunks = []
    try {
      for (;;) {
        const { value, done } = await reader.read()
        if (done) break
        length += value.byteLength
        requireThat(length <= limit, 'response_size_bound')
        chunks.push(Buffer.from(value))
      }
    } catch (error) {
      await reader.cancel().catch(() => {})
      throw error
    }
    return Buffer.concat(chunks, length)
  }
  async http(route, body, label) {
    const method = body === undefined ? 'GET' : 'POST'
    if (body !== undefined) {
      rememberPrivate(body)
      await this.save(`${label}-intent`, {
        phase: this.phase, method, route, request: body,
        automatic_retry: false, sent_at: new Date().toISOString(),
      })
    }
    const response = await fetch(this.url(route), {
      method, redirect: 'error', signal: AbortSignal.timeout(30_000),
      headers: { accept: 'application/json', ...(body === undefined ? {} : { 'content-type': 'application/json' }) },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    })
    const bytes = await this.bodyBytes(response, JSON_LIMIT)
    const text = bytes.toString('utf8')
    let decoded
    try { decoded = JSON.parse(text) } catch { decoded = { error: text.slice(0, 4_000) } }
    rememberPrivate(decoded)
    const result = { status: response.status, ok: response.ok, body: decoded }
    if (method === 'POST' || !response.ok) await this.save(`${label}-response`, result)
    return result
  }
  async post(route, body, label) {
    const response = await this.http(route, body, label)
    requireThat(response.ok, 'public_mutation_rejected', { label, route, ...response })
    return response.body
  }
  async observe() {
    const response = await this.http(
      `/api/corps/${this.context.corp_id}/snapshot?actor_id=${this.context.alice_actor_id}`,
      undefined, 'snapshot')
    requireThat(response.ok, 'authorized_snapshot_failed', response)
    const state = response.body
    requireThat(state.snapshot?.corp?.id === this.context.corp_id, 'snapshot_corp_mismatch')
    for (const name of [...ROWS, 'events']) rows(state.snapshot, name)
    requireThat(Array.isArray(state.runners), 'runner_projection_missing')
    const runner = one(state.runners, (item) => item.id === this.context.runner_id, 'owned_runner_missing_or_duplicated')
    requireThat(runner.corp_id === this.context.corp_id && runner.connected === true &&
      runner.status === 'connected' && runner.capabilities?.some((cap) => cap.name === 'codex' && cap.available === true) &&
      runner.capabilities?.some((cap) => cap.name === 'workspace-isolation' && cap.available === true &&
        cap.source_repository === this.context.source.repository && cap.source_base_ref === 'main' &&
        cap.source_base_commit === this.context.source.base_commit), 'owned_runner_source_or_capability_changed')
    for (const event of state.snapshot.events) {
      uuid(event.id, 'journal_event_id_invalid')
      requireThat(event.corp_id === this.context.corp_id && Number.isSafeInteger(event.seq) && event.seq > 0,
        'journal_scope_or_sequence_invalid')
      if (this.events.has(event.id)) equal(event, this.events.get(event.id), 'observed_journal_event_rewritten')
      this.events.set(event.id, clone(event))
    }
    requireThat(this.events.size <= 5_000, 'bounded_lane_journal_exceeded')
    for (const { collection, key, value } of this.frozen.values()) {
      const current = one(rows(state.snapshot, collection), (row) => rowKey(row) === key, 'immutable_public_row_disappeared')
      equal(current, value, 'immutable_public_history_changed')
    }
    const visibleEvents = new Map(state.snapshot.events.map((event) => [event.id, event]))
    for (const [id, event] of this.frozenEvents) {
      requireThat(visibleEvents.has(id), 'original_journal_left_public_window_coverage_unproven', { id })
      equal(visibleEvents.get(id), event, 'original_journal_changed')
    }
    this.observeGuard(state.snapshot)
    this.lastState = state
    return state.snapshot
  }
  async checkpoint(label, snapshot = null) {
    const current = snapshot ?? await this.observe()
    await this.save(label, { phase: this.phase, ids: this.ids, snapshot: current })
    return current
  }
  async unchanged(label, action) {
    const before = await this.observe()
    const result = await action()
    const after = await this.observe()
    await this.save(`${label}-nonmutation`, { before_sha256: digest(stable(before)), after_sha256: digest(stable(after)) })
    equal(stable(after), stable(before), `${label}_changed_authoritative_state`)
    this.checks.push({ name: label, status: 'passed', no_mutation: true })
    return result
  }
  async denied(route, body, label, statuses = [400, 409, 422]) {
    return this.unchanged(label, async () => {
      const response = await this.http(route, body, label)
      if (!statuses.includes(response.status)) {
        // Preserve the response and observation if an ignored field unexpectedly
        // starts work. Never compensate with a status/counter repair or reset.
        await this.checkpoint(`${label}-unexpected-state`)
        throw new LaneError('expected_public_denial_not_observed', { label, expected_statuses: statuses, response })
      }
      return response.body
    })
  }
  async wait(label, predicate, milliseconds = 90_000) {
    const until = Date.now() + milliseconds
    while (Date.now() < until) {
      const snapshot = await this.observe()
      if (predicate(snapshot)) return this.settled(label)
      await pause(200)
    }
    await this.checkpoint(`${label}-timeout`)
    throw new LaneError('native_product_state_gate_unproven', { label, timeout_ms: milliseconds })
  }
  async settled(label) {
    let before = await this.observe()
    for (let attempt = 0; attempt < 20; attempt += 1) {
      await pause(200)
      const after = await this.observe()
      if (digest(stable(before)) === digest(stable(after))) return this.checkpoint(label, after)
      before = after
    }
    await this.checkpoint(`${label}-unsettled`, before)
    throw new LaneError('native_state_did_not_settle', { label })
  }
  freeze(snapshot, runIds, recoveryIds = []) {
    const capture = (collection, predicate) => {
      for (const row of rows(snapshot, collection).filter(predicate)) {
        const key = rowKey(row)
        this.frozen.set(`${collection}:${key}`, { collection, key, value: clone(row) })
      }
    }
    capture('runs', (row) => runIds.includes(row.id))
    capture('factory_verification_recoveries', (row) => recoveryIds.includes(row.id))
    capture('verification_evidence', (row) => runIds.includes(row.run_id))
    capture('circuit_breaker_incidents', (row) => runIds.includes(row.run_id))
    capture('mission_contract_revisions', (row) => row.mission_id === this.ids.mission)
    for (const event of snapshot.events) {
      if (runIds.includes(event.aggregate_id) || recoveryIds.includes(event.aggregate_id) ||
        event.correlation_id === this.ids.mission) this.frozenEvents.set(event.id, clone(event))
    }
  }
  journal(runId, type) {
    return [...this.events.values()].filter((event) => event.aggregate_id === runId &&
      (type === undefined || event.type === type)).sort((a, b) => a.seq - b.seq)
  }
  async recoveryContext(label) {
    const response = await this.http(
      `/api/corps/${this.context.corp_id}/factory/work-items/${this.ids.item}/verification-recoveries?actor_id=${this.context.alice_actor_id}`,
      undefined, label)
    requireThat(response.ok, 'current_recovery_context_unproven', { label, response })
    await this.save(label, response.body)
    return response.body
  }
  async artifact(row, label, source = false) {
    const id = uuid(row.artifact_id, 'ready_artifact_id_missing')
    const uri = source ? row.uri : row.artifact_uri
    const expectedSha = sha(source ? row.sha256 : row.artifact_sha256, 64, 'artifact_sha256_missing')
    const signature = source ? row.provenance_signature : row.artifact_signature
    const media = source ? row.media_type : row.artifact_media_type
    requireThat(typeof signature === 'string' && signature.length > 0, 'native_artifact_signature_missing')
    requireThat(typeof uri === 'string', 'native_artifact_uri_missing')
    const url = this.url(uri)
    requireThat(url.pathname === `/api/corps/${this.context.corp_id}/artifacts/${id}`, 'artifact_uri_binding_mismatch')
    url.searchParams.set('actor_id', this.context.alice_actor_id)
    const response = await fetch(url, {
      headers: { accept: 'application/octet-stream' },
      redirect: 'error', signal: AbortSignal.timeout(30_000),
    })
    requireThat(response.status === 200, 'native_signed_artifact_download_failed', { status: response.status, label })
    const bytes = await this.bodyBytes(response, ARTIFACT_LIMIT)
    equal(byteDigest(bytes), expectedSha, 'downloaded_bytes_digest_mismatch')
    equal(response.headers.get('content-type'), media, 'artifact_media_type_mismatch')
    equal(response.headers.get('x-content-type-options'), 'nosniff', 'artifact_nosniff_missing')
    equal(response.headers.get('x-crony-artifact-signature'), signature, 'artifact_signature_header_mismatch')
    if (source) equal(bytes.length, row.bytes, 'source_bundle_byte_count_mismatch')
    const file = `${label}.${source ? 'bundle' : 'bin'}`
    await writeFile(path.join(this.output, file), bytes, { flag: 'wx', mode: 0o600 })
    await this.save(`${label}-download`, {
      artifact_id: id, file, bytes: bytes.length, sha256: expectedSha, media_type: media,
      validation: 'server_authorized_signature_boundary_plus_client_byte_hash_and_header_binding',
    })
  }
}

function checkTaskPreview(body, attempts, factory = false) {
  if (factory) requireThat(body.valid === true && body.task_count === 1, 'factory_preflight_not_single_valid_task')
  requireThat(body.strategy === 'single' && Array.isArray(body.tasks) && body.tasks.length === 1,
    'single_preview_tasks_required')
  equal(body.tasks[0].max_attempts, attempts, 'public_planned_attempts_not_propagated')
  equal(body.budget_tokens, MISSION_TOKENS, 'preview_mission_budget_changed')
}
function checkProvider(run, root, context, parentId) {
  equal(run.execution_mode, 'provider', 'correction_not_provider_mode')
  equal(run.corp_id, context.corp_id, 'run_corp_changed')
  equal(run.resumed_from_run_id, parentId, 'native_parent_binding_changed')
  for (const field of ['agent_id', 'runner_id', 'workspace_run_id', 'workspace_path', 'workspace_branch',
    'workspace_base_ref', 'workspace_base_commit', 'source_repository', 'source_base_ref',
    'source_base_commit', 'provider_session_id', 'model', 'reasoning_effort']) {
    equal(run[field], root[field], `native_${field}_changed`)
  }
  equal(run.workspace_connection_id ?? null, root.workspace_connection_id ?? null, 'saved_connection_changed')
  requireThat(typeof run.provider_session_id === 'string' && run.provider_session_id.length > 0,
    'native_provider_session_missing')
  sha(run.workspace_fingerprint, 64, 'native_preserved_fingerprint_missing')
  equal(run.workspace_disposition, 'preserved', 'native_source_not_preserved')
}
function checkProviderContext(context, source, remaining, correctionAvailable) {
  equal(context.source_run_id, source.id, 'context_selected_obsolete_source')
  equal(context.workspace_fingerprint, source.workspace_fingerprint, 'context_fingerprint_not_current')
  requireThat(Object.hasOwn(context, 'expected_head_commit') && context.expected_head_commit === null,
    'failed_provider_without_export_must_keep_null_head')
  equal(context.checkpoint_verification, true, 'checkpoint_family_history_missing')
  equal(context.checkpoint_verification_available, false, 'old_checkpoint_inappropriately_available')
  equal(context.checkpoint_source_correction, correctionAvailable, 'current_correction_availability_wrong')
  equal(context.remaining_attempts, remaining, 'remaining_attempts_wrong')
}

async function lifecycle(h) {
  const c = h.context
  const prefix = `/api/corps/${c.corp_id}`
  const title = `ECorp planned attempts ${h.tag}`
  const initialDescription = '[budget-stream] Prepare the retained source for the planned-attempt acceptance.'
  const cleanDescription = 'Complete the first bounded source correction under the unchanged application checks.'
  const finalDescription = '[checkpoint-app] Complete the retained application under the unchanged verification policy.'
  const sourceRevision = `issue224-${h.tag}`
  const issueUrl = `https://github.com/${REPOSITORY}/issues/224` // Metadata only; never fetched.
  const writeScope = ['base.txt', 'resumed.txt', 'resume-prompt.txt', `${APPLICATION}/**`]
  const verification = {
    checks: [
      { type: 'file', path: 'base.txt', min_bytes: 5 },
      { type: 'artifact', min_bytes: 1 },
      { type: 'file', path: `${APPLICATION}/index.html`, min_bytes: 100 },
      { type: 'test', program: process.execPath, args: ['--test', `${APPLICATION}/app.test.mjs`], timeout_ms: 10_000 },
    ],
    manual_gate: { type: 'independent_review', roles: ['member', 'owner', 'admin'], exclude_requester: true },
  }
  const deliverable = { form: 'commit_branch', commit_after_verification: true, paths: [] }
  const contractOverlay = {
    objective: 'Produce the retained application under its fixed evidence contract.',
    expected_output: 'The existing deterministic incident application with passing artifact, file and test evidence.',
    acceptance_tests: ['All original automated checks pass before independent outcome review.'],
    allowed_tools: ['filesystem', 'shell'],
    prohibited_actions: [
      'modify files outside the assigned worktree',
      'use undeclared long-lived credentials',
      'merge or deploy without a separate current authorization',
    ],
    references: [issueUrl],
    write_scope: writeScope,
  }
  const missionBody = {
    title, description: initialDescription, preferred_adapter: 'codex', strategy: 'single',
    budget_tokens: MISSION_TOKENS, budget_cost_microusd: COST_LIMIT,
    contract: contractOverlay, verification_policy: verification, deliverable,
  }
  const policy = {
    schema_version: 1, source_of_truth: 'github_project',
    project_owner: 'ecorp-fixture', project_number: 224, project_status: 'Todo',
    required_label: 'factory:ready', dependencies: [],
    repository_allowlist: [REPOSITORY],
    source_base_ref: c.source.base_ref, source_base_commit: c.source.base_commit,
    source_commit_upgrade_required: false,
    adapter_allowlist: ['codex'], strategy_allowlist: ['single'],
    model: null, reasoning_effort: null, write_scope: writeScope,
    allowed_tools: contractOverlay.allowed_tools, prohibited_actions: contractOverlay.prohibited_actions,
    secret_ids: [], verification_required: true, verification_policy: verification,
    budget_tokens: MISSION_TOKENS, budget_cost_microusd: COST_LIMIT,
    max_task_attempts: ATTEMPTS, auto_merge: false,
    ...(c.allow_fixture_publication === true ? {
      deliverable_form: 'commit_branch',
      publication: {
        allowed: true, repository_allowlist: [REPOSITORY], base_ref: c.source.base_ref,
        branch_prefix: 'ecorp/', status_before: 'Todo', review_status: 'In Review',
        auto_merge: false, merge: false, deploy: false,
      },
    } : {}),
  }
  const task = (s) => one(s.tasks, (row) => row.id === h.ids.task, 'owned_task_not_unique')
  const mission = (s) => one(s.missions, (row) => row.id === h.ids.mission, 'owned_mission_not_unique')
  const item = (s) => one(s.factory_work_items, (row) => row.id === h.ids.item, 'owned_factory_item_not_unique')
  const run = (s, id) => one(s.runs, (row) => row.id === id, 'owned_run_not_unique')
  const missionRuns = (s) => s.runs.filter((row) => row.task_id === h.ids.task)
  const recoveries = (s) => s.factory_verification_recoveries.filter((row) => row.factory_work_item_id === h.ids.item)
  const spent = (s) => missionRuns(s).reduce((sum, row) => sum + usage(row), 0)

  h.phase = 'read-only-preflight'
  const baseline = await h.settled('baseline')
  requireThat(!baseline.runs.some((row) => ACTIVE.has(row.status)), 'existing_live_work_forbids_this_lane')
  const alice = one(baseline.actors, (row) => row.id === c.alice_actor_id, 'alice_not_visible')
  const bob = one(baseline.actors, (row) => row.id === c.bob_actor_id, 'bob_not_visible')
  requireThat(alice.kind === 'human' && ['owner', 'admin', 'manager'].includes(alice.role), 'alice_not_operator')
  requireThat(bob.kind === 'human' && verification.manual_gate.roles.includes(bob.role), 'bob_not_independent_reviewer')
  const guests = baseline.actors.filter((row) => row.kind === 'human' &&
    ['guest', 'spectator'].includes(row.role) && (!c.guest_actor_id || row.id === c.guest_actor_id))
  requireThat(guests.length > 0, 'guest_actor_missing_parent_must_provision_before_case')
  const guest = guests.sort((a, b) => a.id.localeCompare(b.id))[0]
  const preview = { ...clone(missionBody), requested_by: c.alice_actor_id, source: clone(c.source) }
  for (const attempts of [undefined, 1, 3]) {
    const body = clone(preview)
    if (attempts !== undefined) body.max_task_attempts = attempts
    const label = `preview-${attempts ?? 'default'}`
    const result = await h.unchanged(label, () => h.post(`${prefix}/missions/preview`, body, label))
    checkTaskPreview(result, attempts ?? 2)
  }
  for (const attempts of [0, 4]) {
    await h.denied(`${prefix}/missions/preview`, { ...clone(preview), max_task_attempts: attempts },
      `preview-invalid-${attempts}`, [400, 422])
  }
  await h.denied(`${prefix}/missions/preview`, { ...clone(preview), requested_by: guest.id },
    'preview-guest-denied', [403])
  const factoryPreview = {
    ...clone(missionBody), actor_id: c.alice_actor_id, max_task_attempts: ATTEMPTS,
    source_repository_owner: 'ecorp-fixture', source_repository_name: 'planned-attempts-fixture',
    policy: clone(policy),
  }
  const valid = await h.unchanged('factory-preflight-valid',
    () => h.post(`${prefix}/factory/preflight`, factoryPreview, 'factory-preflight-valid'))
  checkTaskPreview(valid, ATTEMPTS, true)
  const invalidFactory = [
    ['zero', { ...clone(factoryPreview), max_task_attempts: 0 }],
    ['four', { ...clone(factoryPreview), max_task_attempts: 4 }],
    ['mismatch', { ...clone(factoryPreview), max_task_attempts: 2 }],
    ['omitted-request', (() => { const body = clone(factoryPreview); delete body.max_task_attempts; return body })()],
    ['omitted-policy', (() => { const body = clone(factoryPreview); delete body.policy.max_task_attempts; return body })()],
    ['invalid-policy', (() => { const body = clone(factoryPreview); body.policy.max_task_attempts = 4; return body })()],
  ]
  for (const [label, body] of invalidFactory) {
    await h.denied(`${prefix}/factory/preflight`, body, `factory-preflight-${label}`, [400, 422])
  }
  await h.checkpoint('previews-complete')

  h.phase = 'one-held-factory-mission'
  const claimBody = {
    actor_id: c.alice_actor_id,
    source_project_owner: policy.project_owner, source_project_number: policy.project_number,
    source_project_item_id: `PVTI_ISSUE224_${h.tag}`,
    source_repository_owner: 'ecorp-fixture', source_repository_name: 'planned-attempts-fixture',
    source_issue_number: 224, source_issue_node_id: `I_ISSUE224_${h.tag}`,
    source_issue_url: issueUrl, source_title: title, source_revision: sourceRevision,
    idempotency_key: `issue224-${h.tag}-claim`, lease_seconds: 600, policy: clone(policy),
  }
  const claim = await h.post(`${prefix}/factory/work-items/claim`, claimBody, 'factory-claim')
  requireThat(claim.replayed === false, 'fresh_case_must_not_adopt_existing_factory')
  h.ids.item = uuid(claim.work_item?.id)
  let claimToken = uuid(claim.claim_token, 'factory_claim_capability_missing')
  const claimedPolicy = clone(claim.work_item.policy)
  equal(claimedPolicy.max_task_attempts, ATTEMPTS, 'claimed_attempt_policy_missing')
  const claimReplay = await h.unchanged('claim-exact-replay',
    () => h.post(`${prefix}/factory/work-items/claim`, claimBody, 'claim-exact-replay'))
  requireThat(claimReplay.replayed === true && claimReplay.work_item.id === h.ids.item, 'claim_replay_not_exact')
  const changedClaim = clone(claimBody)
  changedClaim.policy.max_task_attempts = 2
  await h.denied(`${prefix}/factory/work-items/claim`, changedClaim, 'claim-changed-policy')

  const materializeRoute = `${prefix}/factory/work-items/${h.ids.item}/materialize`
  const materializeBody = {
    ...clone(missionBody), actor_id: c.alice_actor_id, max_task_attempts: ATTEMPTS,
    claim_token: claimToken, expected_version: claim.work_item.version,
    idempotency_key: `issue224-${h.tag}-materialize`,
  }
  const materialized = await h.post(materializeRoute, materializeBody, 'materialize')
  requireThat(materialized.replayed === false && materialized.task_ids?.length === 1, 'single_fresh_materialization_required')
  h.ids.mission = uuid(materialized.mission_id)
  h.ids.task = uuid(materialized.task_ids[0])
  let expectedTaskBudget = MISSION_TOKENS
  let promptMustBeClean = false
  let lastAttemptCount = 0
  h.observeGuard = (s) => {
    const currentTask = task(s)
    const currentMission = mission(s)
    const currentItem = item(s)
    equal(currentTask.max_attempts, ATTEMPTS, 'persisted_attempt_allowance_changed')
    requireThat(Number.isInteger(currentTask.attempt_count) && currentTask.attempt_count >= 0 &&
      currentTask.attempt_count <= ATTEMPTS, 'native_attempt_count_outside_original_ceiling')
    requireThat(currentTask.attempt_count >= lastAttemptCount, 'native_attempt_count_regressed')
    lastAttemptCount = currentTask.attempt_count
    equal(currentTask.contract.budget_tokens, expectedTaskBudget, 'task_budget_changed_outside_preexecution_revision')
    equal(currentTask.verification_policy, verification, 'fixed_verification_policy_changed')
    equal(currentTask.contract.write_scope, writeScope, 'original_write_scope_changed')
    if (promptMustBeClean) {
      requireThat(!hasBudgetMarker(currentTask.contract) && !hasBudgetMarker(currentMission.description),
        'native_revision_reintroduced_initial_budget_marker')
    }
    equal(currentMission.budget_tokens, MISSION_TOKENS, 'mission_token_budget_changed')
    equal(currentMission.original_budget_tokens, MISSION_TOKENS, 'original_mission_token_budget_changed')
    equal(currentMission.budget_cost_microusd, COST_LIMIT, 'mission_cost_budget_changed')
    equal(currentMission.original_budget_cost_microusd, COST_LIMIT, 'original_mission_cost_budget_changed')
    equal(currentItem.policy, claimedPolicy, 'immutable_factory_policy_changed')
    equal(currentItem.source_title, title, 'immutable_factory_title_changed')
    equal(currentItem.source_revision, sourceRevision, 'immutable_factory_source_revision_changed')
    equal(s.pull_request_publications, baseline.pull_request_publications, 'publication_outside_scope')
    equal(s.pull_request_publication_attempts, baseline.pull_request_publication_attempts, 'publication_attempt_outside_scope')
    equal(s.mission_budget_revisions, baseline.mission_budget_revisions, 'budget_recovery_outside_scope')
  }
  let held = await h.settled('held-plan')
  equal(task(held).attempt_count, 0, 'held_plan_consumed_attempt')
  equal(missionRuns(held).length, 0, 'held_plan_started_without_explicit_launch')
  equal(mission(held).status, 'ready', 'materialized_plan_not_held')
  requireThat(!title.includes('[budget-stream]'), 'immutable_title_contains_stream_marker')
  const replayedMaterialization = await h.unchanged('materialize-exact-replay',
    () => h.post(materializeRoute, materializeBody, 'materialize-exact-replay'))
  requireThat(replayedMaterialization.replayed === true &&
    replayedMaterialization.mission_id === h.ids.mission, 'materialization_replay_not_exact')
  await h.denied(materializeRoute, { ...clone(materializeBody), max_task_attempts: 2 }, 'materialize-changed-attempts')
  await h.denied(materializeRoute, { ...clone(materializeBody), description: 'Different same-key intent.' },
    'materialize-changed-description')

  const revisionRoute = `${prefix}/missions/${h.ids.mission}/contract-revisions`
  const revisionBody = (s, action, sourceId, description, objective) => {
    const contract = { ...clone(task(s).contract), objective }
    if (action === 'resume') {
      // apply_mission_description also embeds the old description in objective.
      // Scrub every carried mutable prompt field, not only mission.description.
      description = stripBudgetMarkers(description)
      for (const field of ['objective', 'expected_output', 'acceptance_tests', 'references']) {
        if (Object.hasOwn(contract, field)) contract[field] = stripBudgetMarkers(contract[field])
      }
      // Do not silently alter authority-bearing prohibitions/tools/source to
      // remove a marker. Such an unexpected fixture must stop before dispatch.
      requireThat(!hasBudgetMarker(contract) && !hasBudgetMarker(description),
        'initial_budget_marker_survived_in_resume_prompt_or_authority')
      equal(task(s).verification_policy, verification, 'resume_must_keep_original_verifier_policy')
    }
    return {
      actor_id: c.alice_actor_id, task_id: h.ids.task, expected_contract_version: task(s).contract_version,
      next_action: action, source_run_id: sourceId,
      reason: `Explicit ${action} for the bounded public planned-attempt fixture.`,
      idempotency_key: randomUUID(), description, contract,
      verification_policy: clone(verification),
    }
  }
  held = await h.observe()
  const narrow = revisionBody(held, 'redispatch', null, initialDescription, contractOverlay.objective)
  narrow.contract.budget_tokens = TASK_TOKENS
  const narrowResult = await h.post(revisionRoute, narrow, 'preexecution-budget-narrowing')
  requireThat(narrowResult.replayed === false, 'preexecution_revision_not_fresh')
  expectedTaskBudget = TASK_TOKENS
  held = await h.checkpoint('narrowed-held-plan')
  equal(task(held).attempt_count, 0, 'narrowing_consumed_attempt')
  equal(missionRuns(held).length, 0, 'narrowing_created_a_fake_source')
  equal(narrowResult.revision.source_run_id, null, 'preexecution_revision_invented_source')
  h.ids.preexecution_revision = uuid(narrowResult.revision.id)
  const launchRoute = `${prefix}/missions/${h.ids.mission}/launch`
  await h.denied(launchRoute, { requested_by: c.alice_actor_id, max_task_attempts: ATTEMPTS },
    'launch-unknown-attempt-option', [400, 422])

  h.phase = 'native-initial-provider-attempt'
  equal(item(held).state, 'mission_created', 'initial_factory_not_ready_for_native_launch')
  const launched = await h.post(launchRoute, { requested_by: c.alice_actor_id }, 'launch-original')
  h.ids.original = uuid(launched.run_id)
  // Match the native Factory controller: mission launch does not itself advance
  // the linked Factory item from mission_created to running. Do not synthesize
  // checkpoint eligibility or loosen the recovery guard to cover a skipped step.
  const running = await h.post(`${prefix}/factory/work-items/${h.ids.item}/transition`, {
    actor_id: c.alice_actor_id, claim_token: claimToken,
    expected_version: item(held).version,
    idempotency_key: `issue224-${h.tag}:transition:running:${item(held).version}`,
    state: 'running', failure_detail: null,
  }, 'factory-native-running')
  equal(running.work_item.id, h.ids.item, 'native_running_transition_item_changed')
  equal(running.work_item.mission_id, h.ids.mission, 'native_running_transition_mission_changed')
  equal(running.work_item.state, 'running', 'native_factory_launch_transition_missing')
  equal(running.work_item.version, item(held).version + 1, 'native_factory_launch_version_wrong')
  let sourceState = await h.wait('original-suspended-and-preserved', (s) => {
    const current = s.runs.find((row) => row.id === h.ids.original)
    return current?.status === 'cancelled' && current.workspace_disposition === 'preserved' &&
      typeof current.workspace_fingerprint === 'string'
  })
  equal(item(sourceState).state, 'running', 'suspended_source_lost_native_factory_context')
  const original = clone(run(sourceState, h.ids.original))
  equal(task(sourceState).attempt_count, 1, 'original_provider_did_not_consume_attempt_one')
  equal(missionRuns(sourceState).length, 1, 'original_launch_created_multiple_runs')
  equal(original.execution_mode, 'provider', 'original_is_not_native_provider_mode')
  equal(original.workspace_run_id, original.id, 'original_workspace_identity_wrong')
  equal(original.resumed_from_run_id, null, 'original_has_synthetic_parent')
  equal(original.runner_id, c.runner_id, 'original_runner_wrong')
  equal(original.source_repository, c.source.repository, 'original_repository_wrong')
  equal(original.source_base_ref, c.source.base_ref, 'original_ref_wrong')
  equal(original.source_base_commit, c.source.base_commit, 'original_commit_wrong')
  equal(original.budget_tokens_limit, TASK_TOKENS, 'original_task_budget_not_narrowed')
  equal(usage(original), 6_000, 'native_fixture_usage_not_observed')
  equal(original.breaker_stage, 'suspend', 'original_hard_stopped_instead_of_suspended')
  equal(original.artifact_id, null, 'budget_exit_created_unverified_provider_artifact')
  requireThat(typeof original.workspace_path === 'string' && path.isAbsolute(original.workspace_path) &&
    typeof original.workspace_branch === 'string' && original.workspace_branch.startsWith('crony/'),
  'native_isolated_workspace_metadata_missing')
  equal(original.workspace_base_commit, c.source.base_commit, 'native_workspace_base_not_pinned')
  requireThat(typeof original.provider_session_id === 'string' && original.provider_session_id.length > 0,
    'original_native_session_missing')
  const started = one(h.journal(original.id, 'run.started'), () => true, 'original_native_start_missing')
  const requested = one(h.journal(original.id, 'run.requested'), () => true, 'original_native_launch_event_missing')
  equal(requested.payload.attempt, 1, 'original_launch_event_not_attempt_one')
  equal(requested.payload.max_attempts, ATTEMPTS, 'original_launch_event_not_public_three_attempt_policy')
  const breaker = one(h.journal(original.id, 'run.breaker_transition'),
    (event) => event.payload.stage === 'suspend', 'native_suspension_event_missing')
  const termination = one(h.journal(original.id, 'run.session_terminated'), () => true, 'original_native_termination_missing')
  const preservation = one(h.journal(original.id, 'run.workspace_preserved'),
    (event) => event.payload.source_checkpoint, 'native_source_checkpoint_missing')
  const cancelled = one(h.journal(original.id, 'run.cancelled'), () => true, 'original_native_cancel_missing')
  requireThat(started.seq < breaker.seq && breaker.seq < termination.seq &&
    termination.seq < preservation.seq && preservation.seq < cancelled.seq, 'original_native_lifecycle_order_wrong')
  equal(termination.payload.provider_process_alive, false, 'original_provider_still_alive')
  requireThat([...h.events.values()].some((event) => event.type === 'runner.command_acknowledged' &&
    event.payload?.command_id === breaker.payload.command_id && event.payload.command_kind === 'circuit_breaker'),
  'native_suspension_command_not_acknowledged')
  const proof = preservation.payload.source_checkpoint
  for (const [field, expected] of Object.entries({
    schema_version: 1, corp_id: c.corp_id, mission_id: h.ids.mission, task_id: h.ids.task,
    run_id: original.id, workspace_run_id: original.id, agent_id: original.agent_id,
    runner_id: c.runner_id, source_repository: c.source.repository, source_base_ref: c.source.base_ref,
    source_base_commit: c.source.base_commit, workspace_base_commit: c.source.base_commit,
    head_commit: c.source.base_commit, branch: original.workspace_branch,
    workspace_fingerprint: original.workspace_fingerprint,
  })) equal(proof[field], expected, `native_checkpoint_${field}_mismatch`)
  for (const field of ['verification_policy_sha256', 'write_scope_sha256', 'deliverable_policy_sha256']) {
    sha(proof[field], 64, `native_checkpoint_${field}_missing`)
  }
  h.freeze(sourceState, [original.id])

  async function renew(context, label) {
    const renewed = await h.post(`${prefix}/factory/work-items/${h.ids.item}/renew`, {
      actor_id: c.alice_actor_id, claim_token: claimToken, expected_version: context.work_item.version,
      idempotency_key: `issue224-${h.tag}-${label}-${randomUUID()}`, lease_seconds: 600,
    }, `${label}-renew`)
    if (renewed.claim_token) claimToken = uuid(renewed.claim_token)
    return renewed.work_item.version
  }
  async function recoveryBody(mode, revisionId, label) {
    const context = await h.recoveryContext(`${label}-current-context`)
    const version = await renew(context, label)
    return {
      context,
      body: {
        actor_id: c.alice_actor_id, claim_token: claimToken, expected_factory_version: version,
        idempotency_key: randomUUID(), source_run_id: context.source_run_id, mode,
        reason: `Explicit ${label} under the original three-attempt policy.`,
        observed_source_revision: sourceRevision,
        reviewed_source_snapshot: {
          source_revision: sourceRevision, issue_number: 224, issue_url: issueUrl,
          issue_node_id: claimBody.source_issue_node_id, title,
          body: contractOverlay.objective, repository: REPOSITORY,
          project_owner: policy.project_owner, project_number: policy.project_number,
          project_item_id: claimBody.source_project_item_id,
        },
        contract_revision_id: revisionId,
        expected_workspace_fingerprint: context.workspace_fingerprint,
        expected_head_commit: context.expected_head_commit,
      },
    }
  }
  const recoveryRoute = `${prefix}/factory/work-items/${h.ids.item}/verification-recoveries`
  async function recoveryReplay(body, id, label) {
    const response = await h.unchanged(label, () => h.post(recoveryRoute, body, label))
    requireThat(response.replayed === true && response.run_id === id, 'recovery_replay_not_exact', { label })
    return response
  }
  function failedVerification(s, id) {
    const current = s.runs.find((row) => row.id === id)
    return current?.status === 'failed' && current.verification_status === 'failed' &&
      current.workspace_disposition === 'preserved' && typeof current.workspace_fingerprint === 'string'
  }
  function requireVerificationFailure(child, label) {
    requireThat(h.journal(child.id, 'run.verification_failed').length === 1, 'expected_verification_failed_missing', { label })
    equal(h.journal(child.id, 'run.failed').length, 0, 'transport_failure_is_not_this_acceptance', { label })
    requireThat(h.journal(child.id, 'run.verification_started').length === 1, 'native_verification_never_started', { label })
  }

  h.phase = 'provider-free-checkpoint-verifier'
  const checkpointInput = await recoveryBody('checkpoint_verification', null, 'checkpoint')
  equal(checkpointInput.context.source_run_id, original.id, 'checkpoint_not_bound_to_original')
  equal(checkpointInput.context.checkpoint_verification_available, true, 'native_checkpoint_not_available')
  equal(checkpointInput.context.remaining_attempts, 2, 'checkpoint_context_attempt_count_wrong')
  const checkpointResult = await h.post(recoveryRoute, checkpointInput.body, 'checkpoint-authorize')
  h.ids.verifier = uuid(checkpointResult.run_id)
  h.ids.checkpoint_recovery = uuid(checkpointResult.recovery.id)
  const verifierState = await h.wait('checkpoint-failed-preserved', (s) => failedVerification(s, h.ids.verifier))
  const verifier = clone(run(verifierState, h.ids.verifier))
  requireVerificationFailure(verifier, 'checkpoint')
  equal(verifier.execution_mode, 'verification_only', 'checkpoint_started_provider')
  equal(verifier.provider_session_id, null, 'checkpoint_has_provider_session')
  equal(verifier.resumed_from_run_id, original.id, 'checkpoint_parent_wrong')
  equal(verifier.workspace_run_id, original.id, 'checkpoint_workspace_changed')
  equal(verifier.workspace_fingerprint, original.workspace_fingerprint, 'source_only_verifier_changed_source_bytes')
  equal(usage(verifier), 0, 'checkpoint_reported_model_usage')
  equal(verifier.cost_microusd, 0, 'checkpoint_reported_model_cost')
  equal(verifier.budget_tokens_limit, 0, 'checkpoint_allocated_model_tokens')
  equal(verifier.budget_cost_microusd_limit, 0, 'checkpoint_allocated_model_cost')
  equal(task(verifierState).attempt_count, 1, 'checkpoint_consumed_provider_attempt')
  h.freeze(verifierState, [original.id, verifier.id], [h.ids.checkpoint_recovery])

  h.phase = 'first-source-correction-verification-failure'
  const firstRevisionBody = revisionBody(verifierState, 'resume', verifier.id, cleanDescription,
    'Complete the first bounded source correction while retaining every original check.')
  await h.denied(revisionRoute, { ...clone(firstRevisionBody), max_task_attempts: ATTEMPTS },
    'revision-unknown-attempt-option', [400, 422])
  const revisionFence = await h.observe()
  for (const value of [ATTEMPTS, null]) {
    const label = `revision-nested-attempt-${value === null ? 'null' : 'three'}`
    const nested = clone(firstRevisionBody)
    nested.contract.max_task_attempts = value
    await h.denied(revisionRoute, nested, label, [400, 422])
    const after = await h.observe()
    equal(task(after).contract_version, task(revisionFence).contract_version,
      'rejected_nested_attempt_option_changed_contract_version')
    equal(mission(after).specification_version, mission(revisionFence).specification_version,
      'rejected_nested_attempt_option_changed_specification_version')
    equal(after.mission_contract_revisions, revisionFence.mission_contract_revisions,
      'rejected_nested_attempt_option_changed_revision_history')
    await h.save(`${label}-version-fence`, {
      contract_version: task(after).contract_version,
      specification_version: mission(after).specification_version,
      contract_sha256: digest(task(after).contract),
      revision_history_sha256: digest(after.mission_contract_revisions),
      unchanged: true,
    })
  }
  const firstRevision = await h.post(revisionRoute, firstRevisionBody, 'first-current-revision')
  h.ids.first_revision = uuid(firstRevision.revision.id)
  promptMustBeClean = true
  await h.checkpoint('first-native-revision-prompt-clear')
  const firstInput = await recoveryBody('source_correction', h.ids.first_revision, 'first-correction')
  equal(firstInput.context.source_run_id, verifier.id, 'first_correction_source_wrong')
  await h.denied(recoveryRoute, { ...clone(firstInput.body), max_task_attempts: ATTEMPTS },
    'recovery-unknown-attempt-option', [400, 422])
  const firstResult = await h.post(recoveryRoute, firstInput.body, 'first-correction-authorize')
  h.ids.first_correction = uuid(firstResult.run_id)
  h.ids.first_recovery = uuid(firstResult.recovery.id)
  const firstState = await h.wait('first-correction-verification-failed',
    (s) => failedVerification(s, h.ids.first_correction))
  const first = clone(run(firstState, h.ids.first_correction))
  checkProvider(first, original, c, verifier.id)
  requireVerificationFailure(first, 'first-correction')
  equal(first.breaker_stage, 'healthy', 'first_correction_did_not_remain_healthy')
  equal(task(firstState).attempt_count, 2, 'first_correction_not_attempt_two')
  equal(first.budget_tokens_limit, MISSION_TOKENS - usage(original), 'first_correction_did_not_use_remaining_allocation')
  requireThat(usage(first) > 0 && first.artifact_id, 'first_provider_did_not_finish_and_upload_real_fixture_artifact')
  requireThat(first.workspace_fingerprint !== verifier.workspace_fingerprint, 'first_correction_did_not_change_source')
  const firstTermination = one(h.journal(first.id, 'run.session_terminated'), () => true, 'first_native_termination_missing')
  equal(firstTermination.payload.outcome, 'completed', 'first_failure_was_not_post_provider_verification')
  equal(firstTermination.payload.provider_process_alive, false, 'first_provider_still_alive')
  requireThat(firstState.verification_evidence.some((evidence) => evidence.run_id === first.id &&
    evidence.check_index === 2 && evidence.status === 'failed'), 'fixed_missing_application_check_did_not_fail')
  equal(firstState.source_deliverables.filter((row) => row.run_id === first.id).length, 0,
    'failed_correction_exported_verified_source')
  h.freeze(firstState, [original.id, verifier.id, first.id], [h.ids.checkpoint_recovery, h.ids.first_recovery])
  await h.artifact(first, 'first-provider-artifact')
  await recoveryReplay(firstInput.body, first.id, 'first-exact-replay')
  await h.denied(recoveryRoute, { ...clone(firstInput.body), reason: 'Different same-key correction authorization.' },
    'first-changed-payload')
  // Availability advertises that a new latest-source revision can authorize a
  // correction; it does not replace that separately required revision/admission.
  const unrevisedContext = await h.unchanged('latest-failed-provider-context-read',
    () => h.recoveryContext('latest-failed-provider-context'))
  checkProviderContext(unrevisedContext, first, 1, true)

  h.phase = 'second-current-revision-and-correction'
  const beforeSecond = await h.observe()
  const secondRevisionBody = revisionBody(beforeSecond, 'resume', first.id, finalDescription,
    'Create the required retained application using this existing provider session and unchanged checks.')
  const secondRevision = await h.post(revisionRoute, secondRevisionBody, 'second-current-revision')
  h.ids.second_revision = uuid(secondRevision.revision.id)
  equal(secondRevision.revision.source_run_id, first.id, 'second_revision_selected_old_verifier')
  const revisionReplay = await h.unchanged('second-revision-exact-replay',
    () => h.post(revisionRoute, secondRevisionBody, 'second-revision-exact-replay'))
  requireThat(revisionReplay.replayed === true && revisionReplay.revision.id === h.ids.second_revision,
    'current_revision_replay_not_exact')
  await h.denied(recoveryRoute, firstInput.body, 'superseded-original-replay')
  const secondInput = await recoveryBody('source_correction', h.ids.second_revision, 'second-correction')
  checkProviderContext(secondInput.context, first, 1, true)
  const staleSource = {
    ...clone(secondInput.body), idempotency_key: randomUUID(), source_run_id: verifier.id,
    expected_workspace_fingerprint: verifier.workspace_fingerprint,
    expected_head_commit: checkpointInput.context.expected_head_commit,
  }
  await h.denied(recoveryRoute, staleSource, 'fresh-obsolete-verifier-source')
  const beforeAdmission = await h.observe()
  const secondResult = await h.post(recoveryRoute, secondInput.body, 'second-correction-authorize')
  h.ids.second_correction = uuid(secondResult.run_id)
  h.ids.second_recovery = uuid(secondResult.recovery.id)
  const activeContext = await h.recoveryContext('second-active-context-zero-remaining')
  checkProviderContext(activeContext, first, 0, false)
  const waiting = await h.wait('second-correction-awaiting-bob', (s) => {
    const current = s.runs.find((row) => row.id === h.ids.second_correction)
    return current?.status === 'waiting_for_approval' && current.workspace_disposition === 'preserved' &&
      typeof current.workspace_fingerprint === 'string' &&
      s.source_deliverables.some((row) => row.run_id === current.id)
  })
  const secondWaiting = run(waiting, h.ids.second_correction)
  checkProvider(secondWaiting, original, c, first.id)
  equal(task(waiting).attempt_count, 3, 'second_correction_not_attempt_three')
  equal(secondWaiting.budget_tokens_limit, Math.min(TASK_TOKENS, MISSION_TOKENS - spent(beforeAdmission)),
    'second_correction_remaining_allocation_wrong')
  requireThat(secondWaiting.workspace_fingerprint !== first.workspace_fingerprint, 'second_correction_source_did_not_change')
  for (let index = 0; index < verification.checks.length; index += 1) {
    requireThat(waiting.verification_evidence.some((evidence) => evidence.run_id === secondWaiting.id &&
      evidence.check_index === index && evidence.status === 'passed'), 'unchanged_native_check_not_passed', { index })
  }
  requireThat(waiting.verification_requests.some((request) => request.run_id === secondWaiting.id &&
    request.status === 'pending'), 'independent_review_not_pending')
  equal(item(waiting).state, 'awaiting_approval', 'factory_skipped_independent_review')
  await recoveryReplay(secondInput.body, secondWaiting.id, 'second-exact-replay')
  await h.denied(recoveryRoute, { ...clone(secondInput.body), reason: 'Changed current second-correction request.' },
    'second-changed-payload')
  await h.artifact(secondWaiting, 'second-provider-artifact')

  h.phase = 'independent-bob-decision'
  await h.post(`${prefix}/runs/${secondWaiting.id}/verification-decision`, {
    actor_id: c.bob_actor_id, approved: true,
    note: 'Independent review of the fixed native protocol-fixture evidence; no vendor, publication or merge claim.',
    decision_key: randomUUID(),
  }, 'bob-independent-approval')
  const final = await h.wait('completed-and-verified', (s) => {
    const current = s.runs.find((row) => row.id === h.ids.second_correction)
    return current?.status === 'completed' && current.verification_status === 'passed' &&
      task(s).status === 'completed' && mission(s).status === 'completed' && item(s).state === 'verified'
  })
  const completed = run(final, h.ids.second_correction)
  equal(missionRuns(final).length, 4, 'expected_exactly_three_providers_and_one_verifier')
  equal(missionRuns(final).filter((row) => row.execution_mode === 'provider').length, 3, 'unexpected_provider_allocation')
  equal(recoveries(final).length, 3, 'unexpected_recovery_allocation')
  equal(task(final).max_attempts - task(final).attempt_count, 0, 'final_remaining_attempts_not_zero')
  equal(task(final).contract.budget_tokens, TASK_TOKENS, 'final_task_budget_was_reset')
  const acceptedSource = one(final.source_deliverables, (row) => row.run_id === completed.id &&
    row.task_id === h.ids.task && row.corp_id === c.corp_id, 'exact_verified_source_export_missing')
  equal(acceptedSource.form, 'commit_branch', 'source_export_form_changed')
  equal(acceptedSource.base_commit, c.source.base_commit, 'source_export_base_changed')
  equal(acceptedSource.branch, original.workspace_branch, 'source_export_branch_changed')
  sha(acceptedSource.head_commit, 40, 'verified_source_head_missing')
  requireThat(acceptedSource.head_commit !== c.source.base_commit, 'no_new_verified_source_commit')
  equal(acceptedSource.verification_sha256, completed.verification_sha256, 'source_verification_binding_changed')
  equal(acceptedSource.sha256, completed.deliverable_sha256, 'source_byte_binding_changed')
  await h.artifact(acceptedSource, 'verified-source', true)
  h.freeze(final, [original.id, verifier.id, first.id], [h.ids.checkpoint_recovery, h.ids.first_recovery])
  await h.checkpoint('final-history-recheck')
  await h.save('observed-native-journal', [...h.events.values()].sort((a, b) => a.seq - b.seq))
  h.report.status = 'passed'
  h.report.summary = {
    mission_count_created: 1, task_count_created: 1, run_count: 4,
    provider_attempts: 3, checkpoint_verifiers: 1, initial_max_attempts: 3,
    attempt_sequence: [1, 1, 2, 3], active_context_remaining_attempts: activeContext.remaining_attempts,
    final_remaining_attempts: task(final).max_attempts - task(final).attempt_count,
    original_mission_tokens: MISSION_TOKENS, preexecution_task_tokens: TASK_TOKENS,
    observed_tokens_spent: spent(final), observed_remaining_mission_tokens: MISSION_TOKENS - spent(final),
    first_correction_failure: 'native_verification_failed_not_transport_run_failed',
    second_correction: 'same_policy_passed_then_independent_bob_approval',
    final_factory_state: item(final).state, original_history_preserved: true,
    source_artifact_id: acceptedSource.artifact_id, source_sha256: acceptedSource.sha256,
    no_vendor_inference_or_browser_acceptance_claim: true,
  }
}

let harness
try {
  const config = await configuration()
  const tag = randomUUID()
  const stamp = new Date().toISOString().replace(/[-:.]/gu, '')
  const output = path.join(config.caseDir, `planned-attempts-${stamp}-${tag}`)
  requireThat(within(config.caseDir, output), 'evidence_output_escape')
  await mkdir(output, { recursive: false, mode: 0o700 })
  requireThat(pathKey(await realpath(output)) === pathKey(output), 'new_evidence_directory_identity_changed')
  harness = new Harness(config, output, tag)
  await harness.save('owned-context', harness.report)
  await lifecycle(harness)
} catch (error) {
  if (harness) {
    harness.report.status = 'failed'
    harness.report.failure = {
      phase: harness.phase,
      code: error.code ?? 'unhandled_lane_error',
      message: String(error.message ?? error),
      details: error.details ?? {},
      note: 'No compensating SQL, reset, allowance change, provider stop, or replacement mission was attempted.',
    }
    if (harness.lastState) await harness.save('last-observed-state-on-failure', harness.lastState)
  } else {
    console.error(JSON.stringify({ status: 'failed', code: error.code ?? 'context_or_output_validation_failed',
      message: String(error.message ?? error) }))
  }
  process.exitCode = 1
} finally {
  if (harness) {
    harness.report.finished_at = new Date().toISOString()
    const result = await harness.save('result', harness.report)
    console.log(JSON.stringify(redact({
      status: harness.report.status, suite: harness.report.suite, result: path.join(harness.output, result),
      ids: harness.ids, summary: harness.report.summary, failure: harness.report.failure,
    }), null, 2))
  }
}
