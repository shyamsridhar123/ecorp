// Issue #163. API-only fixture driver; NEVER approves, resets, enrolls, runs SQL,
// starts/stops services, creates Docker/Git worktrees, or invokes a provider CLI.
//
// Existing supervisor-owned server/runner/DB/UI and independent Git source only.
// Required env: CRONY_EVIDENCE_TEST=1, CRONY_EVIDENCE_FIXTURE=<absolute JSON file>,
// CRONY_EVIDENCE_CHECKPOINT=<absolute JSON file in an existing evidence directory>.
// Ownership receipt (created by the fixture owner, NOT by this helper):
// {
//   "schema_version": 1, "test_owned": true, "purpose": "evidence-selection-163",
//   "fixture_id": "<UUID>", "server_url": "http://127.0.0.1:<QA port>",
//   "web_url": "http://127.0.0.1:<QA port>",
//   "database": { "host": "127.0.0.1", "port": 15497, "name": "<owned DB>" },
//   "auth_mode": "development", "corp_id": "<UUID>", "room_id": "<UUID>",
//   "requester_actor_id": "<UUID>", "reviewer_actor_id": "<different human UUID>",
//   "runner_id": "<owned runner>", "source_checkout": "<absolute independent Git repo>",
//   "runner_workspace": "<absolute owned runner workspace>",
//   "source": { "repository": "<advertised identity>", "base_ref": "<ref>",
//               "base_commit": "<full immutable Git object ID>" }
// }
// auth_mode=oidc requires CRONY_ACCESS_TOKEN and CRONY_REVIEWER_ACCESS_TOKEN.
// Tokens are used only in headers, never checkpointed. Development actor claims
// are explicitly lower-assurance, not proof of independent human authentication.
// Optional ambient CRONY_SERVER_HTTP / CRONY_EVIDENCE_WEB / DATABASE_URL must
// agree with the receipt. The DB association and native fake-process script are
// owner-attested: the API does not attest server process/database configuration.
//
// node tools/e2e_evidence_selection.mjs --phase prepare
//   Browser: select the reported NEWER run and approve as the recorded reviewer.
// node tools/e2e_evidence_selection.mjs --phase verify-first
//   Browser: exercise stale/repeated first-decision context; do NOT select/approve
//   the older run yet. Retain CUA evidence of the attempt and displayed downloads.
// node tools/e2e_evidence_selection.mjs --phase verify-first --after-replay
//   --after-replay is the operator's attestation of that browser exercise, NOT an
//   API replay made by this helper. It checks no second decision followed it.
//   Browser: explicitly select the recorded older run, then approve.
// node tools/e2e_evidence_selection.mjs --phase verify
//
// Reuse the SAME checkpoint after interruption. Create/launch are sent at most
// once; an uncertain result is recovered by exact identity or fails closed.
// Only contract revisions may replay, using their persisted native UUID key.
// Snapshots are bounded: loss of the journal anchor/evidence fails, never passes.
// All files/state are retained. A crashed .lock requires explicit owner inspection
// and removal after confirming the original helper is no longer running.

import assert from 'node:assert/strict'
import { createHash, randomUUID } from 'node:crypto'
import { lstat, open, readFile, realpath, rename, unlink } from 'node:fs/promises'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const SUITE = 'evidence-selection-163'
const UUID = /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/u
const SHA = /^[0-9a-f]{64}$/u
const FORBIDDEN = new Set([5187, 5291, 5432, 8791, 8793, 8991, 15191, 15193, 15491, 15493, 18962])
const TERMINAL = new Set(['completed', 'failed', 'cancelled', 'lost'])
const GATE = { type: 'independent_review', roles: ['owner', 'admin', 'manager', 'member'], exclude_requester: true }
const AUTOMATIC = { checks: [{ type: 'artifact', min_bytes: 1 }], manual_gate: null }
const PRESERVED = ['missions', 'tasks', 'runs', 'agents', 'actors', 'verification_requests',
  'verification_evidence', 'mission_contract_revisions', 'source_deliverables', 'action_approvals',
  'factory_work_items']
const canonical = (value) => JSON.stringify(value, (_, item) =>
  item && typeof item === 'object' && !Array.isArray(item)
    ? Object.fromEntries(Object.entries(item).sort(([a], [b]) => a.localeCompare(b))) : item)
const hash = (value) => createHash('sha256').update(value).digest('hex')
const digest = (value) => hash(canonical(value))
const same = (a, b, message) => assert.equal(digest(a), digest(b), message)
const rowId = (row) => row.id ?? row.run_id
const nativePath = (value) => process.platform !== 'win32' ? value
  : value.replace(/^\\\\\?\\UNC\\/iu, '\\\\').replace(/^\\\\\?\\(?=[A-Za-z]:\\)/u, '')
const within = (parent, child) => {
  const relative = path.relative(nativePath(parent), nativePath(child))
  return relative === '' || (relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative))
}
const exact = (rows, predicate, label) => {
  const matches = rows.filter(predicate)
  assert.equal(matches.length, 1, `Expected exactly one ${label}`)
  return matches[0]
}
const checkUuid = (value, label) => assert.match(value ?? '', UUID, `${label} must be a lowercase UUID`)

function port(value, label) {
  assert.ok(Number.isInteger(value) && value >= 10_000 && value <= 65_535 && !FORBIDDEN.has(value),
    `${label}: explicit non-default, non-manual QA port required (18962/15491 forbidden)`)
}
function origin(value, label) {
  assert.ok(typeof value === 'string' && /^http:\/\/127\.0\.0\.1:\d+\/?$/u.test(value),
    `${label}: explicit literal loopback origin required`)
  let url
  try { url = new URL(value) } catch { throw new Error(`${label}: invalid URL; value withheld`) }
  assert.equal(url.protocol, 'http:', `${label}: owned loopback HTTP only`)
  assert.equal(url.hostname, '127.0.0.1', `${label}: literal 127.0.0.1 only`)
  assert.ok(!url.username && !url.password && !url.search && !url.hash && url.pathname === '/',
    `${label}: origin only, no credentials/path/query/fragment`)
  port(Number(url.port), label)
  return url.origin
}

export function parseArgs(args) {
  assert.ok(args.length === 2 || args.length === 3, 'Use --phase prepare|verify-first|verify [--after-replay]')
  assert.equal(args[0], '--phase')
  assert.ok(['prepare', 'verify-first', 'verify'].includes(args[1]), 'Unknown phase')
  const afterReplay = args.length === 3
  if (afterReplay) {
    assert.equal(args[2], '--after-replay')
    assert.equal(args[1], 'verify-first', '--after-replay is only valid with verify-first')
  }
  return { phase: args[1], afterReplay }
}

export function validateFixture(input, env) {
  assert.equal(env.CRONY_EVIDENCE_TEST, '1', 'Requires CRONY_EVIDENCE_TEST=1 before any API access')
  // Reject unknown fields, especially credentials accidentally put in the receipt.
  const fields = ['schema_version', 'test_owned', 'purpose', 'fixture_id', 'server_url', 'web_url',
    'database', 'auth_mode', 'corp_id', 'room_id', 'requester_actor_id', 'reviewer_actor_id',
    'runner_id', 'source_checkout', 'runner_workspace', 'source']
  same(Object.keys(input).sort(), fields.sort(), 'Ownership receipt has missing/unknown fields')
  assert.ok(input.schema_version === 1 && input.test_owned === true && input.purpose === SUITE,
    'Explicit issue-163 test-owned receipt required')
  for (const key of ['fixture_id', 'corp_id', 'room_id', 'requester_actor_id', 'reviewer_actor_id']) checkUuid(input[key], key)
  assert.notEqual(input.requester_actor_id, input.reviewer_actor_id, 'Independent reviewer must differ from requester')
  const fixture = structuredClone(input)
  fixture.server_url = origin(input.server_url, 'server_url')
  fixture.web_url = origin(input.web_url, 'web_url')
  same(Object.keys(input.database).sort(), ['host', 'name', 'port'], 'Database receipt must not contain credentials')
  assert.equal(input.database.host, '127.0.0.1', 'Owned loopback database only')
  port(input.database.port, 'database.port')
  assert.match(input.database.name, /^[A-Za-z_][A-Za-z0-9_-]{0,62}$/u, 'Explicit owned database name required')
  assert.ok(!['postgres', 'template0', 'template1'].includes(input.database.name), 'Maintenance databases are not fixtures')
  assert.equal(new Set([Number(new URL(fixture.server_url).port), Number(new URL(fixture.web_url).port),
    input.database.port]).size, 3, 'Server, UI and DB must have distinct ports')
  assert.match(input.runner_id, /^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/u)
  assert.notEqual(input.runner_id, 'runner-local', 'Default/manual runner is forbidden')
  same(Object.keys(input.source).sort(), ['base_commit', 'base_ref', 'repository'], 'Complete immutable source required')
  assert.match(input.source.repository, /^(?:local|[A-Za-z0-9_.-]+)\/[A-Za-z0-9_.-]+$/u)
  assert.match(input.source.base_commit, /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/u)
  assert.ok(typeof input.source.base_ref === 'string' && input.source.base_ref.length <= 200
    && /^[^\s\u0000-\u001f-][^\s\u0000-\u001f]*$/u.test(input.source.base_ref), 'Invalid source ref')
  for (const key of ['source_checkout', 'runner_workspace']) assert.ok(path.isAbsolute(input[key]), `${key} must be absolute`)
  assert.ok(['development', 'oidc'].includes(input.auth_mode), 'Explicit auth_mode required')
  if (input.auth_mode === 'oidc') {
    assert.ok(env.CRONY_ACCESS_TOKEN && env.CRONY_REVIEWER_ACCESS_TOKEN, 'Both human access tokens required in environment')
  } else {
    assert.ok(!env.CRONY_ACCESS_TOKEN && !env.CRONY_REVIEWER_ACCESS_TOKEN, 'Development receipt must not silently use bearer credentials')
  }
  for (const [key, expected] of [['CRONY_SERVER_HTTP', fixture.server_url], ['CRONY_EVIDENCE_WEB', fixture.web_url]]) {
    if (env[key]) assert.equal(origin(env[key], key), expected, `${key} differs from the owned receipt`)
  }
  if (env.DATABASE_URL) {
    let db
    try { db = new URL(env.DATABASE_URL) } catch { throw new Error('Invalid DATABASE_URL; value withheld') }
    assert.ok(['postgres:', 'postgresql:'].includes(db.protocol) && db.hostname === '127.0.0.1'
      && Number(db.port) === input.database.port && db.pathname === `/${input.database.name}`
      && !db.search && !db.hash, 'DATABASE_URL differs from owned receipt; values withheld')
  }
  return fixture
}

export class Api {
  constructor(fixture, env, phase, fetcher = fetch) {
    this.fixture = fixture
    this.env = env
    this.phase = phase
    this.fetcher = fetcher
    this.deadline = Date.now() + 180_000
    this.requests = 0
    this.prefix = `/api/corps/${fixture.corp_id}`
  }
  async request(route, body, reviewer = false, binary = false) {
    assert.ok(Date.now() < this.deadline && ++this.requests <= 240, 'Invocation exceeded time/request bound')
    const suffix = route.slice(this.prefix.length)
    const getAllowed = /^\/snapshot\?actor_id=[0-9a-f-]{36}$/u.test(suffix)
      || /^\/artifacts\/[0-9a-f-]{36}\?actor_id=[0-9a-f-]{36}$/u.test(suffix)
    const postAllowed = /^\/missions(?:\/preview|\/[0-9a-f-]{36}\/(?:contract-revisions|launch))?$/u.test(suffix)
    assert.ok(route.startsWith(this.prefix) && (body === undefined ? getAllowed : postAllowed && this.phase === 'prepare'),
      'Only scoped snapshot/artifact reads and prepare planning/revision/launch endpoints are permitted')
    const token = reviewer ? this.env.CRONY_REVIEWER_ACCESS_TOKEN : this.env.CRONY_ACCESS_TOKEN
    if (body !== undefined) assert.ok(Buffer.byteLength(JSON.stringify(body)) <= 65_536, 'API request exceeded byte bound')
    let response
    try {
      response = await this.fetcher(`${this.fixture.server_url}${route}`, {
        method: body === undefined ? 'GET' : 'POST', redirect: 'error',
        headers: { accept: binary ? 'application/octet-stream' : 'application/json',
          ...(body === undefined ? {} : { 'content-type': 'application/json' }),
          ...(token ? { authorization: `Bearer ${token}` } : {}) },
        body: body === undefined ? undefined : JSON.stringify(body),
        signal: AbortSignal.timeout(10_000),
      })
    } catch { throw new Error('API transport failed; preserve checkpoint (request details withheld)') }
    assert.equal(response.status, 200, `Scoped API returned HTTP ${response.status}; body withheld`)
    const chunks = []
    let length = 0
    const reader = response.body.getReader()
    try {
      for (;;) {
        const { done, value } = await reader.read()
        if (done) break
        length += value.length
        assert.ok(length <= (binary ? 1024 * 1024 : 8 * 1024 * 1024), 'API response exceeded byte bound')
        chunks.push(value)
      }
    } finally { await reader.cancel() }
    const bytes = Buffer.concat(chunks)
    return binary ? { bytes, headers: response.headers } : JSON.parse(bytes.toString('utf8'))
  }
  snapshot(reviewer = false) {
    const actor = reviewer ? this.fixture.reviewer_actor_id : this.fixture.requester_actor_id
    return this.request(`${this.prefix}/snapshot?actor_id=${actor}`, undefined, reviewer)
  }
}

function admit(state, f, missionId) {
  const s = state.snapshot
  assert.equal(s.corp.id, f.corp_id, 'Wrong Corp / database fixture')
  for (const key of PRESERVED) assert.ok(Array.isArray(s[key]), `Snapshot missing ${key}`)
  const requester = exact(s.actors, (a) => a.id === f.requester_actor_id, 'requester')
  const reviewer = exact(s.actors, (a) => a.id === f.reviewer_actor_id, 'reviewer')
  for (const actor of [requester, reviewer]) {
    assert.ok(actor.corp_id === f.corp_id && actor.kind === 'human' && GATE.roles.includes(actor.role), 'Authorized human required')
  }
  exact(s.rooms, (room) => room.id === f.room_id && room.corp_id === f.corp_id, 'visible fixture room')
  const connected = state.runners.filter((runner) => runner.connected)
  assert.equal(connected.length, 1, 'Exactly one connected owned runner required')
  const runner = connected[0]
  assert.ok(runner.id === f.runner_id && runner.corp_id === f.corp_id && runner.status === 'connected', 'Wrong runner')
  assert.ok(runner.capabilities.some((c) => c.name === 'fake-process' && c.available), 'Native fake-process runner required')
  const source = exact(runner.capabilities, (c) => c.name === 'workspace-isolation' && c.available, 'workspace capability')
  same([source.source_repository, source.source_base_ref, source.source_base_commit],
    [f.source.repository, f.source.base_ref, f.source.base_commit], 'Live runner source differs from authorized source')
  const ownedTasks = new Set(s.tasks.filter((t) => t.mission_id === missionId).map((t) => t.id))
  assert.ok(s.runs.every((r) => ownedTasks.has(r.task_id) || TERMINAL.has(r.status)), 'Other fixture runs/reviews must be quiescent')
  assert.ok(s.missions.every((m) => m.id === missionId || ['ready', 'completed', 'failed', 'cancelled'].includes(m.status)),
    'Other missions must be held or terminal')
  assert.ok((s.factory_controllers ?? []).every((c) => c.desired_state !== 'running'), 'Pause fixture intake externally first')
  assert.ok(!s.action_approvals.some((a) => a.status === 'pending'), 'No action approvals permitted in this fixture')
  assert.ok(s.verification_evidence.length < 990 && s.verification_requests.length < 490
    && s.mission_contract_revisions.length < 490, 'Snapshot near a bounded evidence/history limit')
  return { requester, reviewer }
}

function graph(s, cp, f) {
  const mission = exact(s.missions, (m) => m.id === cp.mission_id, 'checkpoint mission')
  assert.ok(mission.corp_id === f.corp_id && mission.room_id === f.room_id
    && mission.requested_by === f.requester_actor_id && mission.title === cp.title
    && mission.description === cp.description && mission.strategy === 'parallel-specialists', 'Mission identity drift')
  assert.ok(Number.isSafeInteger(mission.specification_version)
    && mission.specification_version >= 1 && mission.specification_version <= 3, 'Unexpected mission specification version')
  const tasks = s.tasks.filter((t) => t.mission_id === mission.id)
  same(tasks.map((t) => t.id).sort(), [...cp.task_ids].sort(), 'Exact three-task graph changed')
  same(tasks.map((t) => t.plan_key).sort(), ['specialist-a', 'specialist-b', 'synthesis'], 'Unexpected plan')
  const roots = tasks.filter((t) => t.depth === 0)
  assert.equal(roots.length, 2, 'Two independent roots required')
  for (const t of tasks) {
    assert.ok(t.corp_id === f.corp_id && t.required_adapter === 'fake-process'
      && t.attempt_count <= 1, 'Provider, tenant or retry drift')
    same([t.contract.source_repository, t.contract.source_base_ref, t.contract.source_base_commit],
      [f.source.repository, f.source.base_ref, f.source.base_commit], 'Task source binding changed')
    assert.ok(!t.contract.model && !t.contract.reasoning_effort && !t.contract.deliverable
      && t.contract.secret_refs.length === 0, 'No provider settings, deliverables or secrets allowed')
    const agent = exact(s.agents, (a) => a.id === t.assigned_agent_id, 'mission worker')
    assert.ok(agent.mission_id === mission.id && agent.adapter === 'fake-process' && !agent.pinned,
      'Must use newly mission-owned fake-process staffing, not existing agents')
  }
  assert.equal(new Set(tasks.map((t) => t.assigned_agent_id)).size, 3, 'Three distinct native mission workers required')
  roots.forEach((t) => assert.equal(t.depends_on.length, 0))
  const synthesis = tasks.find((t) => t.plan_key === 'synthesis')
  assert.equal(synthesis.depth, 1)
  same([...synthesis.depends_on].sort(), roots.map((t) => t.id).sort(), 'Synthesis must depend on both roots')
  same(synthesis.verification_policy, AUTOMATIC, 'Synthesis must remain automatically verified')
  const runs = s.runs.filter((r) => cp.task_ids.includes(r.task_id))
  for (const run of runs) {
    assert.ok(run.corp_id === f.corp_id && run.runner_id === f.runner_id
      && !run.resumed_from_run_id && !run.model && !run.reasoning_effort, 'Unexpected run lineage/provider')
    same([run.source_repository, run.source_base_ref, run.source_base_commit],
      [f.source.repository, f.source.base_ref, f.source.base_commit], 'Run source binding changed')
    assert.equal(run.agent_id, tasks.find((t) => t.id === run.task_id).assigned_agent_id)
    if (run.workspace_path) {
      assert.ok(within(f.runner_workspace, run.workspace_path) && !within(f.source_checkout, run.workspace_path),
        'Run must use the owned isolated runner workspace, not source')
      assert.equal(run.workspace_base_commit, f.source.base_commit)
    }
  }
  return { mission, tasks, roots, synthesis, runs }
}

function policies(s, cp) {
  assert.equal(s.missions.find((m) => m.id === cp.mission_id).specification_version, 3,
    'Two native revisions must advance mission specification 1 -> 2 -> 3')
  for (const original of cp.original_tasks) {
    const t = exact(s.tasks, (t) => t.id === original.id, 'original task')
    same(t.contract, original.contract, 'Task contract changed outside this helper')
    const revisions = s.mission_contract_revisions.filter((r) => r.task_id === t.id)
    if (original.depth !== 0) {
      assert.equal(t.contract_version, 1)
      assert.equal(revisions.length, 0)
      continue
    }
    assert.equal(revisions.length, 1, 'Exactly one persisted explicit root revision required')
    const r = revisions[0]
    const operation = cp.operations.find((op) => op.name === `gate:${t.id}`)
    assert.ok(operation?.response?.revision?.id === r.id, 'Revision not linked to checkpointed operation')
    assert.ok(r.corp_id === cp.fixture.corp_id && r.mission_id === cp.mission_id
      && r.revised_by === cp.fixture.requester_actor_id && r.next_action === 'redispatch'
      && r.source_run_id === null && r.version === operation.expected_revision_version
      && t.contract_version === r.version, 'Revision authority/version mismatch')
    same(r.previous_contract, original.contract, 'Revision prior contract mismatch')
    same(r.replacement_contract, original.contract, 'Revision widened contract')
    same(r.previous_verification_policy, original.verification_policy, 'Revision prior policy mismatch')
    same(r.replacement_verification_policy, { ...original.verification_policy, manual_gate: GATE }, 'Revision gate mismatch')
    same(t.verification_policy, r.replacement_verification_policy, 'Persisted task policy differs from revision')
    assert.equal(r.previous_description, cp.description)
    assert.equal(r.replacement_description, cp.description)
  }
}

function event(cp, type, runId) {
  return exact(cp.events, (e) => e.type === type && e.aggregate_id === runId, `${type} event for recorded run`)
}
function boundRun(s, cp, task, run) {
  assert.ok(run && run.task_id === task.id && task.attempt_count === 1, 'Missing or extra task attempt')
  const evidence = s.verification_evidence.filter((e) => e.run_id === run.id)
  assert.equal(evidence.length, task.verification_policy.checks.length, 'Incomplete/extra verifier evidence')
  for (let i = 0; i < evidence.length; i++) {
    const e = exact(evidence, (e) => e.check_index === i, 'verifier check index')
    assert.ok(e.corp_id === cp.fixture.corp_id && e.task_id === task.id && e.status === 'passed'
      && e.kind === task.verification_policy.checks[i].type, 'Evidence identity/status mismatch')
  }
  checkUuid(run.artifact_id, 'artifact_id')
  assert.match(run.artifact_sha256 ?? '', SHA)
  assert.ok(run.artifact_signature && run.artifact_media_type && run.workspace_path, 'Missing signed artifact/workspace evidence')
  assert.equal(run.workspace_disposition, 'preserved', 'Deterministic result must retain its worktree')
  assert.equal(Object.hasOwn(run, 'artifact_path'), false, 'Runner-local artifact path leaked')
  const artifactUri = `/api/corps/${cp.fixture.corp_id}/artifacts/${run.artifact_id}`
  assert.equal(run.artifact_uri, artifactUri, 'Artifact URI must match exact Corp/artifact IDs')
  return { task_id: task.id, run_id: run.id, agent_id: run.agent_id,
    artifact_id: run.artifact_id, artifact_uri: artifactUri, artifact_sha256: run.artifact_sha256,
    artifact_signature: run.artifact_signature, artifact_media_type: run.artifact_media_type,
    evidence_ids: evidence.map((e) => e.id).sort(), created_at: run.created_at,
    requested_event: event(cp, 'run.requested', run.id) }
}
function review(s, cp, binding, approved) {
  const r = exact(s.verification_requests, (r) => r.run_id === binding.run_id, 'recorded review')
  assert.ok(r.corp_id === cp.fixture.corp_id && r.task_id === binding.task_id
    && r.gate_type === 'independent_review', 'Review task/Corp/run/gate mismatch')
  same(r.gate, GATE, 'Persisted independent-review gate changed')
  const run = exact(s.runs, (r) => r.id === binding.run_id, 'recorded run')
  const task = exact(s.tasks, (t) => t.id === binding.task_id, 'recorded task')
  assert.equal(r.status, approved ? 'approved' : 'pending', 'Unexpected browser decision / wrong selected run')
  assert.equal(run.status, approved ? 'completed' : 'waiting_for_approval')
  assert.equal(task.status, approved ? 'completed' : 'awaiting_approval')
  assert.equal(run.verification_status, approved ? 'passed' : 'waiting_for_approval')
  assert.equal(task.verification_status, run.verification_status)
  const decisions = cp.events.filter((e) =>
    ['verification.approved', 'verification.rejected'].includes(e.type) && e.aggregate_id === run.id)
  assert.equal(decisions.length, approved ? 1 : 0, 'Duplicate/cross-run review decision')
  if (!approved) {
    assert.ok(r.decided_by === null && r.decided_at === null && r.decision_note === null, 'Pending review was already decided')
    return r
  }
  const e = decisions[0]
  assert.ok(r.decided_by === cp.fixture.reviewer_actor_id && r.decided_at
    && e.actor_id === r.decided_by && e.corp_id === cp.fixture.corp_id
    && e.room_id === cp.fixture.room_id && e.correlation_id === cp.mission_id
    && e.payload.task_id === binding.task_id && e.payload.status === 'approved', 'Exact reviewer attribution mismatch')
  assert.equal(e.payload.note, r.decision_note ?? '', 'Decision note differs from journal')
  return { request: r, event: e, operator: cp.operators.reviewer }
}

export async function runFixture({ fixture: f, phase, afterReplay = false, api, store,
  sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms)) }) {
  assert.ok(['prepare', 'verify-first', 'verify'].includes(phase) && api.phase === phase, 'Phase/API mode mismatch')
  assert.ok(!afterReplay || phase === 'verify-first', 'Replay observation requires verify-first')
  let cp = await store.read()
  if (cp) same(cp.fixture, f, 'Checkpoint belongs to another fixture; never repoint it')
  else {
    assert.equal(phase, 'prepare', 'Prepare checkpoint required')
    cp = { schema_version: 1, suite: SUITE, fixture: f,
      title: `[graph-slow] ECorp #163 evidence selection ${f.fixture_id}`,
      description: `Owned deterministic two-review regression ${f.fixture_id}. Independent root reviews; automatic synthesis.`,
      mission_id: null, task_ids: [], operations: [], events: [], prepared: null, first: null, replay: null }
  }
  assert.ok(cp.schema_version === 1 && cp.suite === SUITE, 'Unsupported checkpoint')
  assert.equal(cp.title, `[graph-slow] ECorp #163 evidence selection ${f.fixture_id}`, 'Checkpoint mission marker changed')
  const save = () => store.write(cp)
  let state
  async function observe() {
    state = await api.snapshot()
    const operators = admit(state, f, cp.mission_id)
    if (cp.operators) same(operators, cp.operators, 'Human identity/role changed')
    else cp.operators = operators
    const s = state.snapshot
    if (!cp.baseline) {
      const rooms = [...s.rooms].sort((a, b) => a.created_at.localeCompare(b.created_at))
      assert.equal(rooms[0]?.id, f.room_id, 'Configured room must be the requester oldest visible room')
      assert.ok(!s.agents.some((a) => a.pinned && a.adapter === 'fake-process'), 'Pinned fake workers could be reused; use isolated fixture staffing')
      cp.baseline = Object.fromEntries(PRESERVED.map((key) =>
        [key, Object.fromEntries(s[key].map((row) => [rowId(row), digest(row)]))]))
    } else {
      for (const key of PRESERVED) {
        const rows = new Map(s[key].map((row) => [rowId(row), digest(row)]))
        for (const [id, value] of Object.entries(cp.baseline[key])) {
          assert.equal(rows.get(id), value, `Pre-existing ${key} row changed/disappeared: ${id}`)
        }
      }
    }
    const events = s.events
    assert.ok(Array.isArray(events) && events.every((e) => e.corp_id === f.corp_id && Number.isSafeInteger(e.seq)), 'Invalid event journal')
    if (cp.anchor) {
      const anchor = exact(events, (e) => e.id === cp.anchor.id, 'journal anchor (bounded snapshot may have rolled over)')
      same(anchor, cp.anchor, 'Journal history changed')
      cp.events.push(...events.filter((e) => e.seq > cp.anchor.seq))
    }
    assert.ok(cp.events.length <= 600, 'Fixture exceeded bounded event history')
    cp.anchor = events.at(-1) ?? null
    cp.last_snapshot = { snapshot_id: randomUUID(), read_at: new Date().toISOString(), sha256: digest(state),
      event_ids: events.map((e) => ({ id: e.id, seq: e.seq })),
      mission_ids: s.missions.map((m) => m.id), task_ids: s.tasks.map((t) => t.id),
      run_ids: s.runs.map((r) => r.id), artifact_ids: s.runs.map((r) => r.artifact_id).filter(Boolean),
      review_run_ids: s.verification_requests.map((r) => r.run_id) }
    await save()
    return s
  }
  async function mutate(name, route, body, allowReplay = false, expectedRevisionVersion = null) {
    let op = cp.operations.find((o) => o.name === name)
    if (op?.response) return op.response
    if (op) {
      same([op.route, op.body], [route, body], 'Mutation differs from persisted intent')
      assert.ok(allowReplay, 'Uncertain non-idempotent operation must not be retried')
    } else {
      op = { name, route, body: structuredClone(body), sends: [], expected_revision_version: expectedRevisionVersion }
      cp.operations.push(op)
    }
    assert.ok(op.sends.length < (allowReplay ? 3 : 1), 'Native operation replay bound reached')
    // IDs not yet allocated by the server remain absent, never guessed. Every
    // observed snapshot/task/run/artifact identity is durable BEFORE sending.
    op.sends.push(structuredClone(cp.last_snapshot))
    await save()
    op.response = await api.request(route, body)
    await save()
    return op.response
  }
  async function artifacts(bindings) {
    const contents = new Map()
    for (const b of bindings) {
      const { bytes, headers } = await api.request(`${b.artifact_uri}?actor_id=${f.reviewer_actor_id}`,
        undefined, true, true)
      assert.equal(hash(bytes), b.artifact_sha256, 'Downloaded artifact digest mismatch')
      assert.equal(headers.get('x-crony-artifact-signature'), b.artifact_signature)
      assert.equal(headers.get('content-type'), b.artifact_media_type)
      assert.equal(headers.get('x-content-type-options'), 'nosniff')
      assert.equal(headers.get('x-crony-artifact-role'), 'provider_evidence')
      b.download = { bytes: bytes.length, sha256: hash(bytes), checked_as: f.reviewer_actor_id }
      contents.set(b.run_id, bytes)
    }
    return contents
  }
  async function unchangedBindings(g) {
    for (const binding of cp.prepared.reviews) {
      const current = boundRun(state.snapshot, cp, g.tasks.find((t) => t.id === binding.task_id),
        exact(g.runs, (r) => r.id === binding.run_id, 'checkpoint root run'))
      const { download: _download, ...original } = binding
      same(current, original, 'Exact recorded run/task/artifact/evidence IDs changed')
    }
    policies(state.snapshot, cp)
  }

  let s = await observe()
  if (phase === 'prepare' && !cp.prepared) {
    const reviewerState = await api.snapshot(true)
    admit(reviewerState, f, cp.mission_id)
    exact(reviewerState.snapshot.rooms, (r) => r.id === f.room_id, 'reviewer room membership')
    if (!cp.mission_id) {
      const matches = s.missions.filter((m) => m.title === cp.title && m.description === cp.description)
      const create = cp.operations.find((o) => o.name === 'create')
      if (create) {
        assert.equal(matches.length, 1, 'Uncertain create: preserve checkpoint; do not retry or create a replacement mission')
        cp.mission_id = matches[0].id
        cp.task_ids = s.tasks.filter((t) => t.mission_id === cp.mission_id).map((t) => t.id)
        if (create.response) {
          assert.equal(create.response.mission_id, cp.mission_id, 'Create response differs from exact readback')
          same([...create.response.task_ids].sort(), [...cp.task_ids].sort(), 'Create task IDs differ from readback')
        }
        create.recovered_readback = { mission_id: cp.mission_id, task_ids: cp.task_ids }
        assert.ok(!cp.baseline.missions[cp.mission_id], 'Cannot adopt pre-existing fixture history')
        await save()
      } else {
        assert.equal(matches.length, 0, 'Existing mission marker: use its original checkpoint, never create a duplicate')
        const body = { title: cp.title, description: cp.description, requested_by: f.requester_actor_id,
          preferred_adapter: 'fake-process', strategy: 'parallel-specialists', source: f.source,
          budget_tokens: 70_000, budget_cost_microusd: 1_000_000 }
        const preview = await api.request(`${api.prefix}/missions/preview`, body)
        assert.equal(preview.strategy, 'parallel-specialists')
        same(preview.tasks.map((t) => t.key).sort(), ['specialist-a', 'specialist-b', 'synthesis'], 'Unsupported native graph preview')
        s = await observe()
        assert.ok(!s.missions.some((m) => m.title === cp.title), 'Mission appeared during preview; refusing duplicate')
        const created = await mutate('create', `${api.prefix}/missions`, body)
        cp.mission_id = created.mission_id
        cp.task_ids = created.task_ids
        checkUuid(cp.mission_id, 'created mission')
        assert.equal(cp.task_ids.length, 3)
        await save()
        s = await observe()
      }
    }
    let g = graph(s, cp, f)
    if (!cp.original_tasks) {
      assert.ok(g.mission.status === 'ready' && g.runs.length === 0, 'New mission must be held before revisions')
      for (const t of g.tasks) {
        assert.ok(t.contract_version === 1 && t.attempt_count === 0)
        same(t.verification_policy, AUTOMATIC, 'Only newly planned automatic policies may receive explicit gates')
      }
      cp.original_tasks = structuredClone(g.tasks)
      await save()
    }
    for (const t of cp.original_tasks.filter((t) => t.depth === 0)) {
      const name = `gate:${t.id}`
      if (cp.operations.find((o) => o.name === name)?.response) continue
      s = await observe()
      g = graph(s, cp, f)
      assert.ok(g.mission.status === 'ready' && g.runs.length === 0, 'No revision after any dispatch')
      const prior = cp.operations.find((o) => o.name === name)
      const body = prior?.body ?? { actor_id: f.requester_actor_id, task_id: t.id,
        expected_contract_version: 1, next_action: 'redispatch', source_run_id: null,
        reason: 'Explicit independent root review for isolated issue-163 evidence selection fixture.',
        idempotency_key: randomUUID(), description: cp.description, contract: t.contract,
        verification_policy: { ...t.verification_policy, manual_gate: GATE } }
      const current = g.tasks.find((task) => task.id === t.id)
      // Native revisions use max(mission specification, task contract) + 1.
      // The second root is version 3, not a second independent version 2.
      const expectedVersion = prior?.expected_revision_version
        ?? Math.max(g.mission.specification_version, current.contract_version) + 1
      assert.ok(expectedVersion === 2 || expectedVersion === 3, 'Unexpected native revision version')
      same(current.contract, t.contract, 'Do not overwrite an intervening contract change')
      assert.ok(current.contract_version === 1 || (prior && current.contract_version === expectedVersion),
        'Do not overwrite an intervening revision')
      same(current.verification_policy, current.contract_version === 1 ? t.verification_policy : body.verification_policy,
        'Do not replace an intervening persisted policy')
      await mutate(name, `${api.prefix}/missions/${cp.mission_id}/contract-revisions`, body, true, expectedVersion)
    }
    s = await observe()
    g = graph(s, cp, f)
    policies(s, cp)
    if (!cp.operations.some((o) => o.name === 'launch')) {
      assert.ok(g.mission.status === 'ready' && g.runs.length === 0, 'Launch only this newly held graph')
      await mutate('launch', `${api.prefix}/missions/${cp.mission_id}/launch`, { requested_by: f.requester_actor_id })
    }
    for (let poll = 0; poll < 100; poll++) {
      s = await observe()
      g = graph(s, cp, f)
      assert.ok(g.runs.length <= 2 && !g.runs.some((r) => TERMINAL.has(r.status)), 'Unexpected terminal run, retry or early synthesis')
      if (g.runs.length === 2 && g.runs.every((r) => r.status === 'waiting_for_approval' && r.workspace_disposition === 'preserved')) break
      assert.ok(poll < 99, 'Prepare timed out; retain checkpoint and investigate the exact launch')
      await sleep(1000)
    }
    policies(s, cp)
    const reviews = g.roots.map((t) => boundRun(s, cp, t, exact(g.runs, (r) => r.task_id === t.id, 'root run')))
      .sort((a, b) => b.requested_event.seq - a.requested_event.seq)
    const launch = cp.operations.find((o) => o.name === 'launch')
    if (launch.response) {
      same([...launch.response.run_ids].sort(), reviews.map((b) => b.run_id).sort(), 'Launch response run IDs differ from exact readback')
      assert.ok(launch.response.runner_ids.length === 2 && launch.response.runner_ids.every((id) => id === f.runner_id),
        'Launch response runner IDs differ from owned runner')
    }
    launch.recovered_readback = { run_ids: reviews.map((b) => b.run_id), artifact_ids: reviews.map((b) => b.artifact_id) }
    reviews.forEach((b) => review(s, cp, b, false))
    assert.equal(s.verification_requests.filter((r) => cp.task_ids.includes(r.task_id)).length, 2)
    assert.equal(g.synthesis.status, 'pending')
    assert.equal(g.synthesis.attempt_count, 0)
    assert.ok(Math.max(...reviews.map((b) => event(cp, 'run.started', b.run_id).seq))
      < Math.min(...reviews.map((b) => event(cp, 'run.verification_waiting', b.run_id).seq)), 'Both workers must start before either waits for review')
    assert.equal(new Set(reviews.map((b) => b.artifact_id)).size, 2, 'Independent artifacts required')
    cp.prepared = { snapshot_id: cp.last_snapshot.snapshot_id, reviews, synthesis_task_id: g.synthesis.id }
    // Persist exact IDs BEFORE exposing them for the parent's browser mutations.
    await save()
    await artifacts(cp.prepared.reviews)
    await save()
  }
  assert.ok(cp.prepared, 'Complete prepare before browser decisions/verification')
  s = await observe()
  const g = graph(s, cp, f)
  await unchangedBindings(g)
  const [first, second] = cp.prepared.reviews // newer first, older deliberately left pending
  if (phase === 'prepare') {
    cp.prepared.reviews.forEach((b) => review(s, cp, b, false))
    assert.equal(g.runs.length, 2)
    await artifacts(cp.prepared.reviews)
  } else if (phase === 'verify-first') {
    assert.equal(g.mission.status, 'running')
    assert.equal(g.runs.length, 2, 'Synthesis must not dispatch after only one review')
    assert.equal(g.synthesis.status, 'pending')
    assert.equal(g.synthesis.attempt_count, 0)
    const decision = review(s, cp, first, true)
    review(s, cp, second, false)
    if (cp.first) same(cp.first.decision, decision, 'First decision changed/replayed into another decision')
    else {
      assert.ok(!afterReplay, 'Record verify-first BEFORE the parent replay exercise')
      cp.first = { snapshot: cp.last_snapshot, decision }
    }
    if (afterReplay) cp.replay = { snapshot: cp.last_snapshot, decision,
      browser_attempt: 'operator-attested; retain separate CUA evidence', second_still_pending: second.run_id }
    await artifacts(cp.prepared.reviews)
  } else {
    assert.ok(cp.first && cp.replay, 'Final verify requires verify-first, then verify-first --after-replay while older review is still pending')
    let finalGraph = g
    for (let poll = 0; !(finalGraph.mission.status === 'completed'
      && finalGraph.runs.length === 3 && finalGraph.runs.every((r) => r.workspace_disposition === 'preserved')); poll++) {
      assert.ok(poll < 100 && !['failed', 'cancelled'].includes(finalGraph.mission.status), 'Synthesis failed/timed out')
      assert.ok(finalGraph.runs.length <= 3 && !finalGraph.runs.some((r) =>
        ['failed', 'cancelled', 'lost'].includes(r.status)), 'Unexpected failed/retried synthesis')
      await sleep(1000)
      s = await observe()
      finalGraph = graph(s, cp, f)
    }
    await unchangedBindings(finalGraph)
    same(review(s, cp, first, true), cp.first.decision, 'First review changed after explicit second selection')
    const secondDecision = review(s, cp, second, true)
    const replayWatermark = cp.replay.snapshot.event_ids.at(-1)?.seq ?? 0
    assert.ok(secondDecision.event.seq > replayWatermark, 'Second decision preceded recorded replay isolation check')
    assert.equal(finalGraph.runs.length, 3, 'Exactly two roots and one synthesis; no retry/replay runs')
    assert.ok(finalGraph.tasks.every((t) => t.status === 'completed' && t.attempt_count === 1
      && t.verification_status === 'passed'), 'All three tasks must complete once')
    const synthesisRun = exact(finalGraph.runs, (r) => r.task_id === finalGraph.synthesis.id, 'synthesis run')
    assert.equal(synthesisRun.status, 'completed')
    assert.equal(synthesisRun.verification_status, 'passed')
    const synthesis = boundRun(s, cp, finalGraph.synthesis, synthesisRun)
    assert.ok(synthesis.requested_event.seq > secondDecision.event.seq, 'Synthesis dispatched before both root decisions')
    event(cp, 'run.completed', synthesis.run_id)
    assert.equal(s.verification_requests.filter((r) => cp.task_ids.includes(r.task_id)).length, 2, 'Unexpected synthesis review')
    const contents = await artifacts([...cp.prepared.reviews, synthesis])
    const text = contents.get(synthesis.run_id).toString('utf8')
    assert.ok(text.includes('VERIFIED DEPENDENCY OUTPUTS') && cp.prepared.reviews.every((b) => text.includes(b.run_id)),
      'Synthesis did not consume both exact verified root outputs')
    cp.final = { snapshot: cp.last_snapshot, second_decision: secondDecision, synthesis, api_passed: true }
  }
  await save()
  const ui = new URL(f.web_url)
  if (f.auth_mode === 'development') ui.searchParams.set('actor', cp.operators.reviewer.name.toLowerCase())
  ui.hash = 'missions'
  return { suite: SUITE, phase, api_passed: phase === 'verify' ? cp.final.api_passed : undefined,
    mission_id: cp.mission_id, mission_title: cp.title, snapshot_id: cp.last_snapshot.snapshot_id,
    ui_url: ui.href, reviewer: cp.operators.reviewer, reviews_newer_first: cp.prepared.reviews,
    synthesis_task_id: cp.prepared.synthesis_task_id, replay_isolation_recorded: Boolean(cp.replay),
    first_decision: cp.first?.decision, second_decision: cp.final?.second_decision, synthesis: cp.final?.synthesis,
    limitations: ['No browser/display/download/replay-attempt proof; retain parent CUA evidence.',
      'DB/process ownership and native fake-process configuration are supervisor-attested, not API-attested.',
      'No real-provider, production-authentication (in development mode), restart or deployment claim.'] }
}

async function readJson(file, maxBytes) {
  const info = await lstat(file)
  assert.ok(info.isFile() && !info.isSymbolicLink() && info.nlink === 1 && info.size <= maxBytes, 'Unsafe/oversized JSON file')
  const text = await readFile(file, 'utf8')
  try { return JSON.parse(text) } catch { throw new Error('Invalid JSON file; content withheld') }
}

async function main() {
  if (process.argv.slice(2).join(' ') === '--help') {
    console.log((await readFile(new URL(import.meta.url), 'utf8')).split('\n\nimport ')[0])
    return
  }
  const args = parseArgs(process.argv.slice(2))
  assert.equal(process.env.CRONY_EVIDENCE_TEST, '1', 'Requires CRONY_EVIDENCE_TEST=1')
  const manifestPath = process.env.CRONY_EVIDENCE_FIXTURE
  const checkpointPath = process.env.CRONY_EVIDENCE_CHECKPOINT
  assert.ok(manifestPath && path.isAbsolute(manifestPath) && checkpointPath && path.isAbsolute(checkpointPath),
    'Absolute ownership receipt/checkpoint paths required')
  const f = validateFixture(await readJson(manifestPath, 16_384), process.env)
  const root = await realpath(path.resolve(import.meta.dirname, '..'))
  const source = await realpath(f.source_checkout)
  const workspace = await realpath(f.runner_workspace)
  f.source_checkout = source
  f.runner_workspace = workspace
  const output = await realpath(path.dirname(checkpointPath))
  assert.ok((await lstat(path.join(source, '.git'))).isDirectory(), 'Source must be an independent Git repository, not a linked worktree')
  for (const [a, b] of [[root, source], [root, workspace], [root, output], [source, workspace], [source, output], [workspace, output]]) {
    assert.ok(!within(a, b) && !within(b, a), 'Source, runner workspace, evidence directory and product must be disjoint')
  }
  let productGit = path.join(root, '.git')
  if ((await lstat(productGit)).isFile()) {
    const pointer = (await readFile(productGit, 'utf8')).trim()
    assert.ok(pointer.startsWith('gitdir: '), 'Invalid product Git metadata')
    productGit = path.resolve(root, pointer.slice(8))
    productGit = path.resolve(productGit, (await readFile(path.join(productGit, 'commondir'), 'utf8')).trim())
  }
  assert.notEqual(await realpath(path.join(source, '.git')), await realpath(productGit), 'Never use the product checkout as fixture source')
  const target = path.join(output, path.basename(checkpointPath))
  assert.ok(target.endsWith('.json') && target !== await realpath(manifestPath), 'Use a separate .json checkpoint')
  const lockPath = `${target}.lock`
  const lock = await open(lockPath, 'wx', 0o600)
  try {
    await lock.writeFile(JSON.stringify({ pid: process.pid, checkpoint: target, started_at: new Date().toISOString() }))
    await lock.sync()
    const store = {
      read: async () => {
        try { return await readJson(target, 16 * 1024 * 1024) } catch (error) {
          if (error.code === 'ENOENT') return null
          throw error
        }
      },
      write: async (value) => {
        const bytes = `${JSON.stringify(value, null, 2)}\n`
        assert.ok(Buffer.byteLength(bytes) <= 16 * 1024 * 1024, 'Checkpoint exceeded byte bound')
        const temporary = `${target}.${randomUUID()}.tmp`
        const file = await open(temporary, 'wx', 0o600)
        try { await file.writeFile(bytes); await file.sync() } finally { await file.close() }
        // Atomic replacement only of this locked checkpoint; failed temps retained.
        await rename(temporary, target)
      },
    }
    const result = await runFixture({ fixture: f, ...args, api: new Api(f, process.env, args.phase), store })
    console.log(JSON.stringify({ ...result, checkpoint: target }, null, 2))
  } finally {
    await lock.close()
    await unlink(lockPath)
  }
}

if (process.argv[1] && pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url) {
  main().catch((error) => { console.error(`Evidence-selection fixture stopped: ${error.message}`); process.exitCode = 1 })
}
