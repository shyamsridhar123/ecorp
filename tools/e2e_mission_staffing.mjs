// DETERMINISTIC public-API regression; parent owns the isolated QA stack and fixture repository.
// Run ONLY after the parent declares the stack ready, with the runner's --copilot-fixture enabled.
// CRONY_STAFFING_TEST=1 CRONY_SERVER_HTTP=http://127.0.0.1:18962
// CRONY_STAFFING_OUTPUT=<new-evidence-directory> node tools/e2e_mission_staffing.mjs
//
// Uses development demo-actor RBAC, not OIDC. Existing unpinned static crew is preserved.
// Requires no existing mission/task/run/factory history; never resets or repairs that history.
// Creates six missions / twelve workers / fourteen tasks, but launches ONLY one fake-process task.
// Studio and factory-studio plans remain HELD: the Copilot fixture writes copilot-result.md,
// not the exact handoff paths. This is NOT real Copilot execution, overlap, or source-handoff proof.
// No SQL, process control, enrollment, permission changes, source edits, GitHub calls, or cleanup.
// Every POST is checkpointed before sending. Existing, failed, or ambiguous checkpoints STOP;
// no automatic resume/retry can create replacement missions after an interruption.

import assert from 'node:assert/strict'
import { createHash, randomUUID } from 'node:crypto'
import { mkdir, open, readFile, rename } from 'node:fs/promises'
import path from 'node:path'

assert.equal(process.env.CRONY_STAFFING_TEST, '1', 'Requires CRONY_STAFFING_TEST=1')
assert.equal(process.argv.length, 2, 'This regression takes no arguments')
assert.ok(process.env.CRONY_SERVER_HTTP, 'CRONY_SERVER_HTTP must be explicit')
const endpoint = new URL(process.env.CRONY_SERVER_HTTP)
assert.equal(endpoint.protocol, 'http:', 'Use the parent-owned loopback HTTP QA stack')
assert.ok(['127.0.0.1', 'localhost', '[::1]'].includes(endpoint.hostname), 'Loopback only')
assert.ok(endpoint.port && Number(endpoint.port) >= 10_000 &&
  !['8791', '8991'].includes(endpoint.port), 'Default/shared/manual ports forbidden; use an owned high port')
assert.ok(!endpoint.username && !endpoint.password && !endpoint.search && !endpoint.hash)
assert.equal(endpoint.pathname, '/', 'CRONY_SERVER_HTTP must be an origin')
assert.ok(process.env.CRONY_STAFFING_OUTPUT, 'CRONY_STAFFING_OUTPUT must be explicit')
const output = path.resolve(process.env.CRONY_STAFFING_OUTPUT)
assert.notEqual(output, path.parse(output).root, 'Use a dedicated evidence directory, not a drive root')
const server = endpoint.origin
const checkpointPath = path.join(output, 'deterministic-mission-staffing.json')
const prefix = '[deterministic-staffing-fixture]'
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i
const active = new Set(['provisioning', 'starting', 'running', 'waiting_for_input',
  'waiting_for_approval', 'verifying'])
const tables = ['actors', 'agents', 'missions', 'tasks', 'runs', 'factory_work_items']
const roles = ['visual-direction', 'gameplay-systems', 'quality-verification']
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const hash = (bytes) => createHash('sha256').update(bytes).digest('hex')
const sorted = (items) => [...items].sort()
const ids = (items) => sorted(items.map((item) => item.id))
const pick = (item, keys) => Object.fromEntries(keys.map((key) => [key, item[key]]))
const canonical = (value) => JSON.stringify(value, (_, item) =>
  item && typeof item === 'object' && !Array.isArray(item)
    ? Object.fromEntries(Object.entries(item).sort(([a], [b]) => a.localeCompare(b)))
    : item)
const secrets = new Set()
const redact = (message) => {
  let text = String(message)
  for (const secret of secrets) text = text.replaceAll(secret, '[REDACTED]')
  return text.slice(0, 2_000)
}
const deadline = Date.now() + 180_000
let checkpoint
let lastState
let baseline
let sequence = 0
let currentCheck = 'checkpoint initialization'
const counts = { get: 0, post: 0, snapshots: 0 }

async function durableWrite(file, value, flag) {
  const handle = await open(file, flag, 0o600)
  try {
    await handle.writeFile(`${JSON.stringify(value, null, 2)}\n`, 'utf8')
    await handle.sync()
  } finally {
    await handle.close()
  }
}

async function save() {
  checkpoint.requests = { ...counts }
  checkpoint.updated_at = new Date().toISOString()
  const temporary = path.join(output, `.staffing-${checkpoint.nonce}-${++sequence}.tmp`)
  await durableWrite(temporary, checkpoint, 'wx')
  await rename(temporary, checkpointPath)
}

async function initialize() {
  let previous
  try {
    previous = await readFile(checkpointPath, 'utf8')
  } catch (error) {
    if (error.code !== 'ENOENT') throw error
  }
  if (previous !== undefined) {
    let phase = 'unreadable/incomplete'
    try { phase = JSON.parse(previous).phase ?? phase } catch {}
    throw new Error(`Existing ${phase} checkpoint: ${checkpointPath}. Inspect its saved IDs; no HTTP request or replacement mission will be issued.`)
  }
  await mkdir(output, { recursive: true })
  checkpoint = {
    schema_version: 1,
    suite: 'deterministic-mission-staffing-public-api',
    evidence_scope: 'Deterministic control-plane and fake-process runner regression only',
    real_copilot_execution: false,
    studio_dispatched: false,
    browser_coverage: false,
    live_github_intake: false,
    server,
    nonce: randomUUID(),
    phase: 'in_progress',
    started_at: new Date().toISOString(),
    operations: [],
    missions: {},
    results: {},
    dispatch_mission_id: null,
  }
  // Exclusive creation also fences concurrent invocations against the same evidence directory.
  await durableWrite(checkpointPath, checkpoint, 'wx')
}

const api = (suffix) => `/api/corps/${checkpoint.corp_id}${suffix}`
function inventory(state) {
  return Object.fromEntries(tables.map((table) => [table, ids(state.snapshot[table])]))
}
function fingerprint(state, selected = tables) {
  return hash(canonical(Object.fromEntries(selected.map((table) =>
    [table, [...state.snapshot[table]].sort((a, b) => a.id.localeCompare(b.id))]))))
}
function sameState(before, after, label, selected = tables) {
  assert.equal(fingerprint(after, selected), fingerprint(before, selected),
    `${label}: persisted rows changed unexpectedly`)
}
function added(before, after, table) {
  const old = new Set(ids(before.snapshot[table]))
  assert.ok(before.snapshot[table].every((item) =>
    after.snapshot[table].some((current) => current.id === item.id)), `${table}: history was removed`)
  return after.snapshot[table].filter((item) => !old.has(item.id))
}

async function boundedBody(response, limit) {
  const declared = Number(response.headers.get('content-length'))
  assert.ok(!Number.isFinite(declared) || declared <= limit, 'API response exceeds byte bound')
  const reader = response.body?.getReader()
  assert.ok(reader, 'API response has no body')
  const chunks = []
  let size = 0
  try {
    while (true) {
      const { done, value } = await reader.read()
      if (done) break
      size += value.byteLength
      assert.ok(size <= limit, 'API response exceeds byte bound')
      chunks.push(Buffer.from(value))
    }
  } finally {
    await reader.cancel()
  }
  return Buffer.concat(chunks)
}

async function http(route, method = 'GET', body) {
  const url = new URL(route, server)
  assert.equal(url.origin, server, 'Refusing an off-stack request')
  assert.ok(url.pathname.startsWith('/api/'), 'Public API paths only')
  const remaining = deadline - Date.now()
  assert.ok(remaining > 0, 'Three-minute regression bound exhausted; preserve the checkpoint')
  const key = method === 'POST' ? 'post' : 'get'
  assert.ok(++counts[key] <= (key === 'post' ? 20 : 512), 'Bounded request count exceeded')
  return fetch(url, {
    method,
    headers: { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
    redirect: 'error',
    signal: AbortSignal.timeout(Math.min(15_000, remaining)),
  })
}

async function jsonBody(response) {
  const bytes = await boundedBody(response, 8 * 1024 * 1024)
  try { return JSON.parse(bytes.toString('utf8')) } catch {
    // Never include response text: claim responses contain a bearer fencing token.
    throw new Error('API returned invalid JSON; response body omitted')
  }
}

function responseIds(body) {
  const result = {}
  for (const key of ['corp_id', 'room_id', 'alice_actor_id', 'bob_actor_id', 'eve_actor_id',
    'mission_id', 'task_id', 'run_id']) {
    if (typeof body[key] === 'string' && uuid.test(body[key])) result[key] = body[key]
  }
  for (const key of ['task_ids', 'run_ids']) {
    if (Array.isArray(body[key])) result[key] = body[key].filter((id) => uuid.test(id))
  }
  if (body.work_item && uuid.test(body.work_item.id)) {
    result.work_item_id = body.work_item.id
    result.work_item_version = body.work_item.version
    result.work_item_state = body.work_item.state
  }
  if (typeof body.replayed === 'boolean') result.replayed = body.replayed
  return result
}

async function post(label, route, body, accepted = [200]) {
  currentCheck = label
  const ordinary = checkpoint.corp_id && route === api('/missions')
  const factory = checkpoint.corp_id && [
    api('/factory/preflight'), api('/factory/work-items/claim'),
    checkpoint.factory?.work_item_id &&
      api(`/factory/work-items/${checkpoint.factory.work_item_id}/materialize`),
  ].includes(route)
  const launch = checkpoint.dispatch_mission_id &&
    route === api(`/missions/${checkpoint.dispatch_mission_id}/launch`)
  assert.ok(route === '/api/demo/bootstrap?seed_crew=false' || ordinary || factory || launch,
    'Mutation route is outside the bounded staffing regression')
  if (launch) {
    const record = checkpoint.missions.single_before
    assert.equal(record?.mission_id, checkpoint.dispatch_mission_id, 'Only the first single may launch')
    assert.equal(record.strategy, 'single')
    assert.equal(record.adapter, 'fake-process', 'Never dispatch a Copilot fixture or studio plan')
    assert.ok(!checkpoint.operations.some((operation) => operation.route === route),
      'No automatic launch retry after a possibly accepted request')
  }
  const { claim_token: omitted, ...safeBody } = body
  // Mission IDs are server-allocated: save the unique title, operation ID, and request before
  // creation; save returned IDs immediately, before validation or ANY subsequent POST.
  const operation = {
    operation_id: randomUUID(), label, route, state: 'pending',
    started_at: new Date().toISOString(), request: safeBody,
    ...(omitted ? { claim_token_omitted: true } : {}),
    ids_before: lastState ? inventory(lastState) : {},
  }
  checkpoint.operations.push(operation)
  await save()
  const response = await http(route, 'POST', body) // No retries, including transport uncertainty.
  operation.http_status = response.status
  const parsed = await jsonBody(response)
  if (typeof parsed.claim_token === 'string') secrets.add(parsed.claim_token)
  operation.response_ids = responseIds(parsed)
  operation.state = 'response_received'
  operation.received_at = new Date().toISOString()
  await save() // Preserve IDs even if an expected denial unexpectedly created a mission.
  assert.ok(accepted.includes(response.status), `${label}: unexpected HTTP ${response.status}`)
  return parsed
}

function missionView(state, missionId) {
  const mission = state.snapshot.missions.find((item) => item.id === missionId)
  assert.ok(mission, `Checkpointed mission ${missionId} is missing; never recreate it`)
  const tasks = state.snapshot.tasks.filter((task) => task.mission_id === missionId)
    .sort((a, b) => a.plan_key.localeCompare(b.plan_key))
  assert.ok(tasks.length, `Mission ${missionId} has no retained tasks`)
  const taskIds = new Set(ids(tasks))
  const runs = state.snapshot.runs.filter((run) => taskIds.has(run.task_id))
  return { mission, tasks, runs }
}

function authority(view) {
  return hash(canonical({
    mission: pick(view.mission, ['id', 'corp_id', 'room_id', 'requested_by', 'title', 'description',
      'strategy', 'max_nodes', 'max_depth', 'budget_tokens', 'budget_cost_microusd']),
    tasks: view.tasks.map((task) => pick(task, ['id', 'mission_id', 'plan_key', 'assigned_agent_id',
      'required_adapter', 'depends_on', 'depth', 'max_attempts', 'contract', 'verification_policy'])),
  }))
}

async function snapshot() {
  const response = await http(api(`/snapshot?actor_id=${checkpoint.actor_id}`))
  assert.equal(response.status, 200, 'Owner snapshot request failed')
  const state = await jsonBody(response)
  for (const table of tables) {
    assert.ok(Array.isArray(state.snapshot?.[table]), `Snapshot omits ${table}`)
    assert.ok(state.snapshot[table].length <= 500, 'Use a small owned QA Corp, not a shared snapshot')
  }
  assert.ok(Array.isArray(state.runners))
  for (const record of Object.values(checkpoint.missions)) {
    const view = missionView(state, record.mission_id)
    assert.equal(authority(view), record.authority_sha256, 'Saved task/source authority changed')
    if (record.mission_id === checkpoint.dispatch_mission_id) continue
    assert.equal(view.mission.status, 'ready', `${record.label}: held mission was admitted implicitly`)
    assert.equal(view.runs.length, 0, `${record.label}: held mission acquired a run`)
    assert.ok(view.tasks.every((task) =>
      task.attempt_count === 0 && ['pending', 'ready'].includes(task.status)))
  }
  // There is no studio dispatch exception, even while polling unrelated fake-process completion.
  assert.ok(state.snapshot.runs.every((run) =>
    baseline?.snapshot.runs.some((original) => original.id === run.id) ||
    checkpoint.missions.single_before?.task_ids.includes(run.task_id)),
  'Unexpected run: this regression only authorizes its first single/fake-process task')
  counts.snapshots++
  lastState = state
  return state
}

function normalizeSource(source) {
  return { repository: source.repository.toLowerCase(), base_ref: source.base_ref,
    base_commit: source.base_commit.toLowerCase() }
}
function sourceFromRunner(runner) {
  const cap = runner.capabilities.find((item) => item.name === 'workspace-isolation' && item.available)
  if (!cap || !cap.source_repository || !cap.source_base_ref || !cap.source_base_commit) return null
  return normalizeSource({ repository: cap.source_repository, base_ref: cap.source_base_ref,
    base_commit: cap.source_base_commit })
}
function matchingRunners(state, adapter, source) {
  return state.runners.filter((runner) => runner.connected &&
    canonical(sourceFromRunner(runner)) === canonical(source) &&
    runner.capabilities.some((cap) => cap.name === adapter && cap.available))
}
function checkSource(task) {
  assert.deepEqual(normalizeSource({
    repository: task.contract.source_repository,
    base_ref: task.contract.source_base_ref,
    base_commit: task.contract.source_base_commit,
  }), checkpoint.source, 'Every task must retain the selected immutable source')
}

const artifactPolicy = { checks: [{ type: 'artifact', min_bytes: 1 }], manual_gate: null }
const finalPath = 'arcade/deterministic-final-fixture.md'
const finalDeliverable = { form: 'typed_artifact_set', commit_after_verification: false, paths: [finalPath] }
const finalPolicy = { checks: [{ type: 'artifact', min_bytes: 1 },
  { type: 'file', path: finalPath, min_bytes: 1 }], manual_gate: null }
const prohibited = ['modify files outside the assigned worktree', 'use undeclared long-lived credentials',
  'merge or deploy without a separate current authorization']
function missionBody(label, strategy = 'single') {
  const studio = strategy === 'studio-swarm'
  return {
    title: `${prefix} ${label} ${checkpoint.nonce}`,
    description: 'Deterministic staffing regression only. No live provider inference, publication, or application-build evidence is claimed.',
    requested_by: checkpoint.actor_id,
    strategy,
    preferred_adapter: studio ? 'github-copilot' : 'fake-process',
    preferred_model: null,
    reasoning_effort: null,
    source: checkpoint.source,
    secret_refs: [],
    budget_tokens: studio ? 200_000 : 80_000,
    budget_cost_microusd: 1_000_000,
    verification_policy: studio ? finalPolicy : artifactPolicy,
    ...(studio ? {
      deliverable: finalDeliverable,
      contract: {
        objective: 'Deterministic held-plan fixture; do not dispatch this studio graph.',
        expected_output: 'Deterministic final-deliverable placeholder; not a built application.',
        acceptance_tests: ['Final placeholder is reserved for integration, never a specialist root.'],
        allowed_tools: ['filesystem'],
        prohibited_actions: prohibited,
        references: [],
        write_scope: ['arcade/**'],
      },
    } : {}),
  }
}

function checkStudio(view, workers, factory = false) {
  const roots = view.tasks.filter((task) => task.depth === 0)
  const join = view.tasks.find((task) => task.plan_key === 'studio-integration')
  assert.equal(roots.length, 3)
  assert.ok(join)
  assert.deepEqual(sorted(roots.map((task) => task.plan_key)), sorted(roles))
  assert.equal(new Set(roots.map((task) => task.assigned_agent_id)).size, 3)
  for (const root of roots) {
    const handoff = `arcade/handoffs/${root.plan_key}.md`
    const worker = workers.find((agent) => agent.id === root.assigned_agent_id)
    assert.equal(worker?.role, root.plan_key, 'Staffing roles must win over alphabetical agent names')
    assert.deepEqual(root.depends_on, [])
    assert.deepEqual(root.contract.write_scope, [handoff], 'Root scope must narrow arcade/** to one file')
    assert.deepEqual(root.contract.deliverable,
      { form: 'typed_artifact_set', paths: [handoff], commit_after_verification: false })
    assert.deepEqual(root.contract.allowed_tools, ['filesystem'])
    assert.ok(root.contract.expected_output.includes(handoff))
    assert.notEqual(root.contract.expected_output, join.contract.expected_output)
    assert.ok(root.contract.acceptance_tests.some((test) => test.includes('12288 bytes')))
    assert.ok(root.verification_policy.checks.some((check) =>
      check.type === 'file' && check.path === handoff && check.min_bytes > 0))
    assert.ok(root.verification_policy.checks.some((check) =>
      check.type === 'artifact' && check.min_bytes > 0))
    // This is a persisted runner-owned verifier, not permission for the provider to use shell.
    assert.deepEqual(root.verification_policy.checks.find((check) =>
      check.type === 'command' && check.program === 'node'), {
      type: 'command',
      program: 'node',
      args: ['-e',
        "const b=require('node:fs').readFileSync(process.argv[1]);new TextDecoder('utf-8',{fatal:true}).decode(b);if(b.length>12288)process.exit(1)",
        handoff],
      timeout_ms: 5_000,
    }, 'Every root must persist the exact-path, strict UTF-8, maximum-12-KiB Node verifier')
    assert.ok(!root.verification_policy.checks.some((check) => check.path === finalPath))
  }
  assert.equal(join.depth, 1)
  assert.equal(join.assigned_agent_id,
    roots.find((task) => task.plan_key === 'gameplay-systems').assigned_agent_id)
  assert.deepEqual(sorted(join.depends_on), ids(roots))
  for (const role of roles) assert.ok(join.contract.references.includes(`task:${role}`))
  assert.deepEqual(join.contract.write_scope, ['arcade/**'])
  assert.deepEqual(join.contract.deliverable, finalDeliverable)
  assert.deepEqual(join.verification_policy.checks, finalPolicy.checks)
  if (factory) assert.ok(view.tasks.every((task) =>
    task.verification_policy.manual_gate?.type === 'independent_review'))
}

async function rememberMission(label, created, before, body, factory = false) {
  assert.match(created.mission_id, uuid)
  const after = await snapshot()
  const view = missionView(after, created.mission_id)
  const studio = body.strategy === 'studio-swarm'
  const count = body.strategy === 'single' ? 1 : 3
  const taskCount = studio ? 4 : count
  const workers = added(before, after, 'agents')
  assert.deepEqual(ids(added(before, after, 'missions')), [created.mission_id])
  assert.equal(workers.length, count, 'Provision exactly the required identities, without reuse')
  assert.equal(added(before, after, 'actors').length, count, 'No orphan or extra agent actors')
  assert.deepEqual(ids(added(before, after, 'tasks')), ids(view.tasks))
  assert.deepEqual(ids(view.tasks), sorted(created.task_ids))
  assert.equal(view.tasks.length, taskCount)
  assert.equal(view.mission.strategy, body.strategy)
  assert.equal(view.mission.status, 'ready')
  assert.equal(view.runs.length, 0)
  assert.equal(view.mission.budget_tokens, body.budget_tokens)
  assert.equal(view.mission.budget_cost_microusd, body.budget_cost_microusd)
  assert.equal(new Set(workers.map((worker) => worker.actor_id)).size, count)
  const expectedRoles = body.strategy === 'single' ? ['engineer']
    : studio ? roles : ['specialist-a', 'specialist-b', 'manager']
  assert.deepEqual(sorted(workers.map((worker) => worker.role)), sorted(expectedRoles))
  for (const worker of workers) {
    assert.equal(worker.mission_id, created.mission_id)
    assert.equal(worker.corp_id, checkpoint.corp_id)
    assert.equal(worker.adapter, body.preferred_adapter)
    assert.equal(worker.pinned, false)
    assert.equal(worker.retired_at, null)
    assert.equal(worker.status, 'idle')
    assert.equal(worker.current_run_id, null)
    assert.ok(after.snapshot.actors.some((actor) => actor.id === worker.actor_id && actor.kind === 'agent'))
    assert.ok(view.tasks.some((task) => task.assigned_agent_id === worker.id), 'Unused identity is an orphan')
  }
  assert.deepEqual(sorted(new Set(view.tasks.map((task) => task.assigned_agent_id))), ids(workers))
  for (const task of view.tasks) {
    assert.equal(task.required_adapter, body.preferred_adapter)
    assert.equal(task.attempt_count, 0)
    assert.equal(task.max_attempts, 2)
    assert.equal(task.contract.model, null)
    assert.equal(task.contract.reasoning_effort, null)
    checkSource(task)
  }
  if (studio) checkStudio(view, workers, factory)
  else if (body.strategy === 'parallel-specialists') {
    const roots = view.tasks.filter((task) => task.depth === 0)
    const join = view.tasks.find((task) => task.plan_key === 'synthesis')
    assert.equal(roots.length, 2)
    assert.ok(roots.every((root) => root.depends_on.length === 0))
    assert.deepEqual(sorted(join.depends_on), ids(roots))
    assert.equal(workers.find((worker) => worker.id === join.assigned_agent_id)?.role, 'manager')
  } else {
    assert.deepEqual(view.tasks[0].verification_policy, artifactPolicy)
    assert.equal(view.tasks[0].contract.deliverable, null, 'Fake-process needs only its provider artifact')
  }
  assert.equal(added(before, after, 'runs').length, 0, 'Creation must not dispatch any task')
  const record = {
    label, mission_id: created.mission_id, strategy: body.strategy, adapter: body.preferred_adapter,
    task_ids: ids(view.tasks), agent_ids: ids(workers), actor_ids: sorted(workers.map((agent) => agent.actor_id)),
    authority_sha256: authority(view), evidence_scope: 'deterministic held-plan assertions',
  }
  checkpoint.missions[label] = record
  await save() // Persist assigned worker/task IDs before the next write.
  return record
}

async function createHeld(label, strategy = 'single') {
  const before = await snapshot()
  const body = missionBody(label, strategy)
  const created = await post(`create ${label}`, api('/missions'), body)
  return rememberMission(label, created, before, body)
}

async function immutablePost(label, route, body, accepted = [200], selected = tables) {
  const before = await snapshot()
  const response = await post(label, route, body, accepted)
  const after = await snapshot()
  sameState(before, after, label, selected)
  checkpoint.results[label] = {
    evidence_scope: 'deterministic public-API no-mutation assertion',
    http_status: checkpoint.operations.at(-1).http_status,
    row_counts: Object.fromEntries(selected.map((table) => [table, after.snapshot[table].length])),
    before_sha256: fingerprint(before, selected), after_sha256: fingerprint(after, selected),
  }
  await save()
  return response
}

async function factoryChecks() {
  const [owner, repository] = checkpoint.source.repository.split('/')
  const fixtureNumber = 991048
  const issueUrl = `https://github.com/${owner}/${repository}/issues/${fixtureNumber}`
  const ordinary = missionBody('synthetic-factory-studio', 'studio-swarm')
  const { requested_by, source, ...materialization } = ordinary
  materialization.contract = { ...materialization.contract, references: [issueUrl] }
  const policy = {
    schema_version: 1, source_of_truth: 'github_project', project_owner: owner,
    project_number: fixtureNumber, project_status: 'Todo', required_label: 'factory:ready',
    dependencies: [], repository_allowlist: [source.repository], source_base_ref: source.base_ref,
    source_base_commit: source.base_commit, adapter_allowlist: ['github-copilot'],
    strategy_allowlist: ['studio-swarm'], model: null, reasoning_effort: null,
    write_scope: ['arcade/**'], allowed_tools: ['filesystem'], prohibited_actions: prohibited,
    secret_ids: [], verification_required: true, budget_tokens: ordinary.budget_tokens,
    budget_cost_microusd: ordinary.budget_cost_microusd, auto_merge: false,
  }
  const preflight = { ...materialization, actor_id: requested_by,
    source_repository_owner: owner, source_repository_name: repository, policy }
  for (let index = 1; index <= 3; index++) {
    const result = await immutablePost(`deterministic factory preflight ${index}`,
      api('/factory/preflight'), preflight)
    assert.equal(result.valid, true)
    assert.equal(result.strategy, 'studio-swarm')
    assert.equal(result.task_count, 4)
    assert.equal(result.budget_tokens, ordinary.budget_tokens)
    assert.equal(result.budget_cost_microusd, ordinary.budget_cost_microusd)
  }
  const claimBody = {
    actor_id: requested_by, source_project_owner: owner, source_project_number: fixtureNumber,
    source_project_item_id: `PVTI_DETERMINISTIC_STAFFING_${checkpoint.nonce}`,
    source_repository_owner: owner, source_repository_name: repository,
    source_issue_number: fixtureNumber, source_issue_node_id: `I_DETERMINISTIC_STAFFING_${checkpoint.nonce}`,
    source_issue_url: issueUrl, source_title: ordinary.title,
    source_revision: checkpoint.started_at, idempotency_key: `deterministic-staffing-claim-${checkpoint.nonce}`,
    lease_seconds: 300, policy,
  }
  const beforeClaim = await snapshot()
  const claim = await post('deterministic factory claim', api('/factory/work-items/claim'), claimBody)
  assert.match(claim.work_item?.id, uuid)
  checkpoint.factory = {
    work_item_id: claim.work_item.id,
    claim_idempotency_key: claimBody.idempotency_key,
    materialization_idempotency_key: `deterministic-staffing-materialize-${checkpoint.nonce}`,
    evidence_scope: 'synthetic API fixture; no live GitHub discovery or publication',
  }
  await save()
  const afterClaim = await snapshot()
  sameState(beforeClaim, afterClaim, 'claim must not provision staff', tables.filter((table) => table !== 'factory_work_items'))
  assert.deepEqual(ids(added(beforeClaim, afterClaim, 'factory_work_items')), [claim.work_item.id])
  assert.equal(claim.work_item.state, 'claimed')
  assert.equal(claim.replayed, false)
  assert.match(claim.claim_token, uuid)
  const replayedClaim = await immutablePost('deterministic factory claim replay',
    api('/factory/work-items/claim'), claimBody)
  assert.equal(replayedClaim.replayed, true)
  assert.equal(replayedClaim.work_item.id, claim.work_item.id)
  assert.ok(replayedClaim.claim_token === claim.claim_token, 'Claim replay must retain its fencing token')
  const materializeBody = {
    ...materialization, actor_id: requested_by, claim_token: claim.claim_token,
    expected_version: claim.work_item.version,
    idempotency_key: checkpoint.factory.materialization_idempotency_key,
  }
  const route = api(`/factory/work-items/${claim.work_item.id}/materialize`)
  const beforeMaterialize = await snapshot()
  const created = await post('deterministic factory materialization', route, materializeBody)
  assert.equal(created.replayed, false)
  assert.equal(created.work_item.id, claim.work_item.id)
  assert.equal(created.work_item.state, 'mission_created')
  const record = await rememberMission('factory_studio', created, beforeMaterialize, ordinary, true)
  checkpoint.factory.mission_id = record.mission_id
  await save()
  const replayed = await immutablePost('deterministic factory materialization replay', route, materializeBody)
  assert.equal(replayed.replayed, true)
  assert.equal(replayed.mission_id, record.mission_id)
  assert.deepEqual(sorted(replayed.task_ids), record.task_ids)
  assert.equal(replayed.work_item.id, claim.work_item.id)
  assert.equal(replayed.work_item.mission_id, record.mission_id)
}

async function quietWindow() {
  const until = Date.now() + 2_000
  do { await snapshot(); await sleep(250) } while (Date.now() < until)
}

async function executeSingle(record) {
  const before = await snapshot()
  const held = missionView(before, record.mission_id)
  assert.equal(held.tasks.length, 1)
  assert.equal(held.tasks[0].required_adapter, 'fake-process')
  assert.equal(held.runs.length, 0)
  assert.equal(held.mission.status, 'ready')
  checkpoint.dispatch_mission_id = record.mission_id
  await save()
  const launched = await post('dispatch only deterministic single/fake-process',
    api(`/missions/${record.mission_id}/launch`), { requested_by: checkpoint.actor_id })
  assert.equal(launched.run_ids?.length, 1)
  const until = Date.now() + 90_000
  while (Date.now() < until) {
    const state = await snapshot()
    const view = missionView(state, record.mission_id)
    assert.ok(!['failed', 'cancelled'].includes(view.mission.status),
      'Deterministic fake-process mission failed; do not relaunch it')
    const agent = state.snapshot.agents.find((item) => item.id === record.agent_ids[0])
    assert.ok(agent, 'Retirement must preserve the agent history row')
    if (view.mission.status === 'completed' && agent.retired_at) {
      assert.equal(view.runs.length, 1)
      const run = view.runs[0]
      if (!['preserved', 'removed'].includes(run.workspace_disposition)) {
        await sleep(500)
        continue
      }
      assert.deepEqual(launched.run_ids, [run.id])
      assert.equal(run.agent_id, agent.id)
      assert.equal(run.status, 'completed')
      assert.equal(run.verification_status, 'passed')
      assert.ok(checkpoint.fake_runner_ids.includes(run.runner_id))
      assert.equal(view.tasks[0].status, 'completed')
      assert.equal(view.tasks[0].attempt_count, 1)
      assert.equal(agent.mission_id, record.mission_id)
      assert.equal(agent.pinned, false)
      assert.equal(agent.status, 'idle')
      assert.equal(agent.current_run_id, null)
      assert.ok(Number.isFinite(Date.parse(agent.retired_at)))
      assert.deepEqual(normalizeSource({ repository: run.source_repository,
        base_ref: run.source_base_ref, base_commit: run.source_base_commit }), checkpoint.source)
      const evidence = state.snapshot.verification_evidence.filter((item) => item.run_id === run.id)
      assert.ok(evidence.some((item) => item.check_index === 0 && item.status === 'passed'))
      assert.match(run.artifact_id, uuid)
      assert.match(run.artifact_sha256, /^[0-9a-f]{64}$/i)
      const response = await http(api(`/artifacts/${run.artifact_id}?actor_id=${checkpoint.actor_id}`))
      assert.equal(response.status, 200, 'Verified fixture artifact must remain downloadable')
      assert.equal(response.headers.get('x-crony-artifact-role'), 'provider_evidence')
      const bytes = await boundedBody(response, 64 * 1024)
      assert.ok(bytes.length > 0)
      assert.equal(hash(bytes), run.artifact_sha256.toLowerCase())
      const requested = state.snapshot.events.filter((event) =>
        event.type === 'run.requested' && event.aggregate_id === run.id)
      assert.equal(requested.length, 1)
      assert.equal(requested[0].payload?.mission_launch, true)
      assert.equal(requested[0].actor_id, checkpoint.actor_id)
      for (const type of ['run.started', 'run.completed']) assert.ok(state.snapshot.events.some((event) =>
        event.type === type && event.aggregate_id === run.id), `Missing ${type} fixture evidence`)
      assert.ok(state.snapshot.events.some((event) => event.type === 'agent.retired' &&
        event.aggregate_id === agent.id && event.payload?.mission_id === record.mission_id))
      checkpoint.results.single_fixture_execution = {
        evidence_scope: 'Parent-owned runner executing the deterministic fake-process adapter',
        mission_id: record.mission_id, task_id: view.tasks[0].id, run_id: run.id,
        runner_id: run.runner_id, agent_id: agent.id, retired_at: agent.retired_at,
        artifact_id: run.artifact_id, artifact_sha256: hash(bytes), artifact_bytes: bytes.length,
        workspace_disposition: run.workspace_disposition,
        history_retained: true, real_copilot_execution: false,
      }
      await save()
      return
    }
    await sleep(500)
  }
  throw new Error('Timed out awaiting fake-process completion and automatic retirement; preserve all IDs')
}

function noOrphans(state) {
  const records = Object.values(checkpoint.missions)
  const newAgents = added(baseline, state, 'agents')
  const newActors = added(baseline, state, 'actors')
  assert.deepEqual(ids(newAgents), sorted(records.flatMap((record) => record.agent_ids)))
  assert.deepEqual(ids(newActors), sorted(newAgents.map((agent) => agent.actor_id)))
  assert.deepEqual(ids(added(baseline, state, 'missions')), sorted(records.map((record) => record.mission_id)))
  assert.deepEqual(ids(added(baseline, state, 'tasks')), sorted(records.flatMap((record) => record.task_ids)))
  assert.equal(new Set(records.flatMap((record) => record.agent_ids)).size, newAgents.length,
    'Unpinned identities must never cross mission ownership')
  for (const agent of newAgents) {
    const record = records.find((item) => item.mission_id === agent.mission_id)
    assert.ok(record?.agent_ids.includes(agent.id))
    assert.ok(state.snapshot.tasks.some((task) => task.mission_id === agent.mission_id &&
      task.assigned_agent_id === agent.id), 'New identity has no task in its owning mission')
  }
  for (const old of baseline.snapshot.agents) assert.deepEqual(
    state.snapshot.agents.find((agent) => agent.id === old.id), old, 'Existing crew must remain unchanged')
  assert.deepEqual(ids(added(baseline, state, 'factory_work_items')), [checkpoint.factory.work_item_id])
  const item = state.snapshot.factory_work_items.find((work) => work.id === checkpoint.factory.work_item_id)
  assert.equal(item.mission_id, checkpoint.missions.factory_studio.mission_id)
  assert.equal(item.state, 'mission_created')
  assert.ok(!Object.hasOwn(item, 'claim_token'), 'Shared snapshots must not expose fencing tokens')
}

await initialize() // Existing checkpoints are rejected before even an idempotent bootstrap POST.
try {
  const demo = await post('bootstrap deterministic humans/room without crew',
    '/api/demo/bootstrap?seed_crew=false', {})
  for (const key of ['corp_id', 'room_id', 'alice_actor_id', 'eve_actor_id']) assert.match(demo[key], uuid)
  Object.assign(checkpoint, { corp_id: demo.corp_id, room_id: demo.room_id,
    actor_id: demo.alice_actor_id, guest_actor_id: demo.eve_actor_id })
  await save()
  baseline = await snapshot()
  for (const table of ['missions', 'tasks', 'runs', 'factory_work_items']) assert.equal(
    baseline.snapshot[table].length, 0, `Existing ${table}: inspect prior checkpoints; never reset or recreate`)
  assert.ok(!baseline.snapshot.agents.some((agent) => agent.pinned),
    'Exact provisioning cases require no pinned reusable crew; parent must supply isolated QA')
  assert.ok(!baseline.snapshot.runs.some((run) => active.has(run.status)))
  assert.ok(baseline.snapshot.rooms.some((room) => room.id === demo.room_id))
  assert.ok(baseline.snapshot.actors.some((actor) => actor.id === demo.alice_actor_id && actor.kind === 'human'))
  if (baseline.snapshot.agents.length === 0) assert.ok(baseline.snapshot.actors.every((actor) => actor.kind === 'human'),
    'Fresh seed_crew=false bootstrap must not create agent actors')
  const sources = new Map(baseline.runners.filter((runner) => runner.connected &&
    runner.capabilities.some((cap) => cap.name === 'fake-process' && cap.available))
    .map(sourceFromRunner).filter(Boolean).map((source) => [canonical(source), source]))
  assert.equal(sources.size, 1, 'Parent must provide one unambiguous fake-process fixture source')
  checkpoint.source = [...sources.values()][0]
  assert.match(checkpoint.source.repository, /^[a-z0-9_.-]+\/[a-z0-9_.-]+$/)
  assert.ok(checkpoint.source.base_ref)
  assert.match(checkpoint.source.base_commit, /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/)
  checkpoint.fake_runner_ids = matchingRunners(baseline, 'fake-process', checkpoint.source).map((runner) => runner.id)
  checkpoint.copilot_plan_runner_ids = matchingRunners(baseline, 'github-copilot', checkpoint.source).map((runner) => runner.id)
  checkpoint.copilot_fixture_configuration = 'Parent prerequisite: --copilot-fixture; not inferred from availability or execution'
  assert.ok(checkpoint.copilot_plan_runner_ids.length,
    'Parent must start the source-matched runner with --copilot-fixture; studio will remain held')
  checkpoint.baseline_ids = inventory(baseline)
  checkpoint.results.bootstrap_without_crew = {
    evidence_scope: 'Deterministic seed_crew=false bootstrap',
    initial_agent_count: baseline.snapshot.agents.length,
    fresh_no_crew_observed: baseline.snapshot.agents.length === 0,
    default_fixture_bootstrap_exercised: false,
  }
  await save()

  const single = await createHeld('single_before')
  await createHeld('parallel_held', 'parallel-specialists')
  // Same provider AND same engineer role while the first worker is still idle and unpinned.
  await createHeld('single_while_other_held')
  await createHeld('studio_held', 'studio-swarm')
  const missingModel = `deterministic-unavailable-model-${checkpoint.nonce}`
  assert.ok(!baseline.runners.some((runner) => runner.capabilities.some((cap) =>
    cap.models?.some((model) => model.id === missingModel))))
  await immutablePost('deterministic unavailable explicit model', api('/missions'),
    { ...missionBody('denied-model'), preferred_adapter: 'github-copilot', preferred_model: missingModel }, [400])
  const missingAdapter = 'deterministic-unavailable-provider'
  assert.ok(!baseline.runners.some((runner) => runner.capabilities.some((cap) =>
    cap.name === missingAdapter && cap.available)))
  await immutablePost('deterministic unavailable explicit provider', api('/missions'),
    { ...missionBody('denied-provider'), preferred_adapter: missingAdapter }, [400])
  const staleCommit = (checkpoint.source.base_commit.startsWith('0') ? '1' : '0')
    .repeat(checkpoint.source.base_commit.length)
  await immutablePost('deterministic stale source', api('/missions'),
    { ...missionBody('denied-source'), source: { ...checkpoint.source, base_commit: staleCommit } }, [400])
  await immutablePost('deterministic guest create denied', api('/missions'),
    { ...missionBody('denied-guest'), requested_by: checkpoint.guest_actor_id }, [401, 403])
  await factoryChecks()
  await quietWindow() // All five plans, including factory studio, must remain ready with zero runs.
  await executeSingle(single)
  await createHeld('single_after_retirement')
  const repeatedDemo = await immutablePost('deterministic bootstrap preserves existing and retired identities',
    '/api/demo/bootstrap?seed_crew=false', {})
  assert.equal(repeatedDemo.corp_id, checkpoint.corp_id)
  await quietWindow()
  const finalState = await snapshot()
  noOrphans(finalState)
  const retained = missionView(finalState, single.mission_id)
  assert.equal(retained.mission.status, 'completed')
  assert.deepEqual(ids(retained.runs), [checkpoint.results.single_fixture_execution.run_id])
  assert.equal(finalState.snapshot.agents.find((agent) => agent.id === single.agent_ids[0]).retired_at,
    checkpoint.results.single_fixture_execution.retired_at)
  assert.equal(added(baseline, finalState, 'agents').length, 12)
  assert.equal(added(baseline, finalState, 'missions').length, 6)
  assert.equal(added(baseline, finalState, 'tasks').length, 14)
  assert.equal(added(baseline, finalState, 'runs').length, 1)
  checkpoint.results.summary = {
    evidence_scope: 'DETERMINISTIC fixture regression; not real Copilot or three-provider execution evidence',
    agents_created: 12, missions_created: 6, tasks_created: 14, fake_process_runs: 1,
    held_mission_ids: Object.values(checkpoint.missions).filter((record) =>
      record.mission_id !== checkpoint.dispatch_mission_id).map((record) => record.mission_id),
    all_held_plans_have_zero_runs: true, cross_mission_unpinned_reuse: false,
    orphan_identities: 0, factory_preflight_repetitions: 3, factory_replays_verified: true,
    studio_dispatched: false, real_copilot_execution: false,
  }
  checkpoint.phase = 'passed'
  checkpoint.finished_at = new Date().toISOString()
  await save()
  console.log(JSON.stringify({ phase: checkpoint.phase, checkpoint: checkpointPath,
    ...checkpoint.results.summary }, null, 2))
} catch (error) {
  checkpoint.phase = 'incomplete'
  checkpoint.failure = { check: currentCheck, message: redact(error.message),
    recorded_at: new Date().toISOString(), automatic_retry_allowed: false }
  try { await save() } catch {
    console.error('Could not update the checkpoint; preserve its existing file and any .tmp records.')
  }
  console.error(JSON.stringify({ phase: 'incomplete', checkpoint: checkpointPath,
    check: currentCheck, error: redact(error.message), next_action: 'Parent inspects saved IDs; no resets or automatic rerun.' }, null, 2))
  process.exitCode = 1
}
