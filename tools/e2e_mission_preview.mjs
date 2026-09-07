// Read-only control-plane regression against an EXISTING, parent-owned QA stack.
// Required: CRONY_MISSION_PREVIEW_TEST=1, CRONY_SERVER_HTTP,
// CRONY_CORP_ID, CRONY_ACTOR_ID. Optional: CRONY_ACCESS_TOKEN,
// CRONY_PREVIEW_RUNNER_ID, CRONY_PREVIEW_ROOMLESS_ACTOR_ID.
// CRONY_PREVIEW_COPILOT_FIXTURE=1 explicitly enables the Studio fixture case.
// No defaults, bootstrap/reset, fixture writes, launch, enrollment, process control,
// or model/verifier execution. Uses only GET snapshots/health and POST preview.
// The parent supplies existing identities/source and may redirect this JSON report.
// Development-principal role/room checks are not production OIDC evidence.

import assert from 'node:assert/strict'
import { createHash, randomUUID } from 'node:crypto'

const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i
const operators = new Set(['owner', 'admin', 'manager', 'member'])
const activeRuns = ['provisioning', 'starting', 'running', 'waiting_for_input',
  'waiting_for_approval', 'verifying']
const tables = ['actors', 'agents', 'rooms', 'missions', 'tasks', 'runs',
  'mission_contract_revisions', 'mission_budget_revisions', 'verification_evidence',
  'verification_requests', 'source_deliverables', 'factory_work_items',
  'factory_verification_recoveries', 'action_approvals', 'events']
const manualPorts = new Set(['8791', '8793', '8991', '15191', '15193', '15491', '15493', '18962'])
const canonical = (value) => JSON.stringify(value, (_, item) =>
  item && typeof item === 'object' && !Array.isArray(item)
    ? Object.fromEntries(Object.entries(item).sort(([a], [b]) => a.localeCompare(b)))
    : item)
const digest = (value) => createHash('sha256').update(canonical(value)).digest('hex')

function required(name) {
  const value = process.env[name]
  assert.ok(value?.trim(), `${name} must identify the existing owned fixture`)
  return value
}

function identifier(value, name) {
  assert.ok(uuid.test(value), `${name} must be an existing fixture UUID`)
  return value.toLowerCase()
}

assert.equal(process.env.CRONY_MISSION_PREVIEW_TEST, '1', 'Requires CRONY_MISSION_PREVIEW_TEST=1')
assert.equal(process.argv.length, 2, 'This regression takes no arguments')
const endpoint = new URL(required('CRONY_SERVER_HTTP'))
assert.equal(endpoint.protocol, 'http:', 'Use the owned loopback HTTP QA stack')
assert.ok(['127.0.0.1', 'localhost', '[::1]'].includes(endpoint.hostname), 'Loopback only')
assert.ok(endpoint.port && Number(endpoint.port) >= 10_000 && !manualPorts.has(endpoint.port),
  'Default/shared/manual ports are forbidden')
assert.equal(endpoint.pathname, '/', 'CRONY_SERVER_HTTP must be an origin')
assert.ok(!endpoint.username && !endpoint.password && !endpoint.search && !endpoint.hash)
const corpId = identifier(required('CRONY_CORP_ID'), 'CRONY_CORP_ID')
const actorId = identifier(required('CRONY_ACTOR_ID'), 'CRONY_ACTOR_ID')
const token = process.env.CRONY_ACCESS_TOKEN
const server = endpoint.origin
const deadline = Date.now() + 60_000
const report = {
  suite: 'mission-preview-read-only',
  status: 'in_progress',
  server,
  corp_id: corpId,
  actor_id: actorId,
  started_at: new Date().toISOString(),
  checks: [],
  coverage_limits: [],
  requests: 0,
  execution_requested: false,
  receipt_scope: 'Authenticated Corp/room snapshot rows and append-only event watermark',
}
let phase = 'read-only fixture admission'
let baseline

async function http(route, method = 'GET', body, accessToken = token) {
  assert.ok(++report.requests <= 100, 'Preview regression exceeded its request bound')
  const remaining = deadline - Date.now()
  assert.ok(remaining > 0, 'Preview regression exceeded its 60-second deadline')
  if (method === 'POST') {
    assert.match(route, /^\/api\/corps\/[0-9a-f-]{36}\/missions\/preview$/)
  } else {
    assert.ok(route === '/health' ||
      route.startsWith(`/api/corps/${corpId}/snapshot?actor_id=`), 'Unexpected read route')
  }
  const response = await fetch(`${server}${route}`, {
    method,
    redirect: 'error',
    headers: {
      ...(accessToken ? { authorization: `Bearer ${accessToken}` } : {}),
      ...(body === undefined ? {} : { 'content-type': 'application/json' }),
    },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    signal: AbortSignal.timeout(Math.min(5_000, remaining)),
  })
  const reader = response.body?.getReader()
  assert.ok(reader, 'API response omitted its body')
  const chunks = []
  let size = 0
  try {
    while (true) {
      const { done, value } = await reader.read()
      if (done) break
      size += value.byteLength
      assert.ok(size <= 8 * 1024 * 1024, 'API response exceeded the byte bound')
      chunks.push(value)
    }
  } finally {
    await reader.cancel()
  }
  let parsed
  try { parsed = JSON.parse(Buffer.concat(chunks).toString('utf8')) } catch {
    throw new Error('API returned invalid JSON; body omitted')
  }
  return { status: response.status, body: parsed }
}

async function snapshot(viewer = actorId) {
  const response = await http(`/api/corps/${corpId}/snapshot?actor_id=${viewer}`)
  assert.equal(response.status, 200, 'Existing fixture snapshot must be readable')
  assert.equal(response.body.snapshot?.corp?.id, corpId, 'Snapshot Corp mismatch')
  return response.body
}

async function inventory() {
  const state = await snapshot()
  // Actors/agents/missions/tasks/runs are unpaginated in this API. Compare exact
  // visible rows, not only counts. A new visible journal event advances the
  // append-only watermark even when the 200-event snapshot window is full.
  const visible = {}
  for (const table of tables) {
    assert.ok(Array.isArray(state.snapshot[table]), `Snapshot omitted ${table}`)
    assert.ok(state.snapshot[table].length <= 5_000, 'Use the small owned acceptance Corp')
    visible[table] = [...state.snapshot[table]].sort((a, b) =>
      String(a.id).localeCompare(String(b.id)))
  }
  assert.ok(state.snapshot.events.every((event) => Number.isSafeInteger(event.seq)),
    'Event watermark must be represented exactly')
  const receipt = {
    rows: Object.fromEntries(tables.map((table) => [table, visible[table].length])),
    active_runs: state.snapshot.runs.filter((run) => activeRuns.includes(run.status)).length,
    event_watermark: Math.max(0, ...state.snapshot.events.map((event) => event.seq)),
    visible_state_sha256: digest(visible),
  }
  assert.equal(receipt.active_runs, 0, 'Existing visible QA runs must be terminal')
  assert.ok(!state.snapshot.missions.some((mission) => mission.status === 'running'),
    'Existing QA missions must not be awaiting automatic scheduling')
  return { receipt, state }
}

async function unchanged(label) {
  const after = await inventory()
  assert.equal(digest(after.receipt), digest(baseline.receipt),
    `${label}: preview changed rows, events, agents, or visible state`)
  return after
}

function assertShape(preview, request, taskCount) {
  assert.equal(canonical(Object.keys(preview).sort()),
    canonical(['strategy', 'budget_tokens', 'budget_cost_microusd', 'tasks'].sort()),
    'Preview must expose exactly the public response fields')
  assert.equal(preview.strategy, request.strategy ?? 'single')
  assert.equal(preview.budget_tokens, request.budget_tokens)
  assert.equal(preview.budget_cost_microusd, request.budget_cost_microusd)
  assert.equal(preview.tasks.length, taskCount)
  const keys = new Set(preview.tasks.map((task) => task.key))
  assert.equal(keys.size, taskCount, 'Task keys must be unique')
  for (const task of preview.tasks) {
    assert.equal(canonical(Object.keys(task).sort()), canonical([
      'key', 'title', 'budget_tokens', 'budget_cost_microusd', 'depends_on', 'max_attempts',
    ].sort()), 'Task preview must not expose identities, contracts, secrets, or prompts')
    assert.match(task.key, /^[a-zA-Z0-9._-]{1,64}$/)
    assert.ok(typeof task.title === 'string' && task.title.length > 0)
    assert.ok(Number.isSafeInteger(task.budget_tokens) && task.budget_tokens > 0)
    assert.ok(Number.isSafeInteger(task.budget_cost_microusd) && task.budget_cost_microusd > 0)
    assert.ok(Number.isSafeInteger(task.max_attempts) && task.max_attempts >= 1 && task.max_attempts <= 3)
    assert.ok(Array.isArray(task.depends_on) &&
      task.depends_on.every((key) => typeof key === 'string' && keys.has(key) && key !== task.key))
  }
  assert.ok(preview.tasks.reduce((sum, task) => sum + task.budget_tokens, 0) <= preview.budget_tokens)
  assert.ok(preview.tasks.reduce((sum, task) => sum + task.budget_cost_microusd, 0) <=
    preview.budget_cost_microusd)
  assert.ok(!JSON.stringify(preview).includes('PRIVATE_PREVIEW_'), 'Private request data leaked')
}

async function preview(label, request, expectedStatus = 200, {
  taskCount = 1, targetCorp = corpId, accessToken = token, errorPattern,
} = {}) {
  phase = label
  const response = await http(`/api/corps/${targetCorp}/missions/preview`, 'POST', request, accessToken)
  // Check state even when a response unexpectedly succeeds or fails.
  await unchanged(label)
  assert.equal(response.status, expectedStatus, `${label}: unexpected HTTP status`)
  if (expectedStatus === 200) assertShape(response.body, request, taskCount)
  else {
    assert.ok(typeof response.body.error === 'string', `${label}: expected an API error`)
    if (errorPattern) assert.ok(errorPattern.test(response.body.error), `${label}: wrong rejection`)
  }
  report.checks.push({ name: label, status: response.status, unchanged: true })
  return response.body
}

try {
  const health = await http('/health')
  assert.equal(health.status, 200)
  assert.equal(health.body.service, 'crony-server')
  assert.ok(['development', 'production'].includes(health.body.mode))
  report.authentication_mode = health.body.mode
  if (health.body.mode === 'production') assert.ok(token, 'Production QA requires CRONY_ACCESS_TOKEN')
  baseline = await inventory()
  report.before = baseline.receipt
  const humans = baseline.state.snapshot.actors.filter((actor) => actor.kind === 'human')
  assert.ok(humans.some((actor) => actor.id === actorId && operators.has(actor.role)) &&
    baseline.state.snapshot.rooms.length > 0,
    'Use an existing operator who belongs to a room in this Corp')
  const runners = baseline.state.runners.filter((runner) => runner.connected &&
    (!process.env.CRONY_PREVIEW_RUNNER_ID || runner.id === process.env.CRONY_PREVIEW_RUNNER_ID) &&
    runner.capabilities.some((cap) => cap.name === 'fake-process' && cap.available) &&
    runner.capabilities.some((cap) => cap.name === 'workspace-isolation' && cap.available &&
      cap.source_repository && cap.source_base_ref && cap.source_base_commit))
  assert.equal(runners.length, 1, 'Select one existing QA runner with CRONY_PREVIEW_RUNNER_ID')
  const runner = runners[0]
  assert.equal(runner.corp_id, corpId, 'Selected QA runner Corp mismatch')
  const workspace = runner.capabilities.find((cap) => cap.name === 'workspace-isolation' && cap.available &&
    cap.source_repository && cap.source_base_ref && cap.source_base_commit)
  const request = {
    title: 'ECorp read-only mission preview',
    description: `PRIVATE_PREVIEW_SPECIFICATION ${'bounded specification '.repeat(100)}`,
    requested_by: actorId,
    preferred_adapter: 'fake-process',
    strategy: 'single',
    source: {
      repository: workspace.source_repository,
      base_ref: workspace.source_base_ref,
      base_commit: workspace.source_base_commit,
    },
    budget_tokens: 100_003,
    budget_cost_microusd: 300_007,
    contract: {
      objective: 'PRIVATE_PREVIEW_OBJECTIVE',
      expected_output: 'PRIVATE_PREVIEW_OUTPUT',
      acceptance_tests: ['The declared result is verified'],
      allowed_tools: ['filesystem'],
      prohibited_actions: ['Do not publish'],
      references: ['PRIVATE_PREVIEW_REFERENCE'],
      write_scope: ['result.md'],
    },
    verification_policy: {
      checks: [{ type: 'file', path: 'result.md', min_bytes: 1 }],
      manual_gate: null,
    },
    deliverable: { form: 'review_only_report', paths: ['result.md'], commit_after_verification: false },
  }
  const first = await preview('valid source-selected preview', request)
  for (let index = 1; index <= 2; index++) {
    const repeated = await preview(`repeated preview ${index}`, request)
    assert.equal(digest(repeated), digest(first), 'Provisional staffing IDs must not affect preview')
  }
  const normalized = await preview('padded title and CRLF description', {
    ...request, title: `  ${request.title}  `, description: 'PRIVATE_PREVIEW_SPECIFICATION\r\nsecond line',
  })
  assert.equal(digest(normalized), digest(first), 'Labels and budgets must not depend on private prose')
  await preview('omitted strategy uses the create default', { ...request, strategy: undefined })
  const parallel = await preview('parallel graph dependencies and budgets', {
    ...request, strategy: 'parallel-specialists',
  }, 200, { taskCount: 3 })
  assert.deepEqual(parallel.tasks.find((task) => task.key === 'synthesis')?.depends_on,
    ['specialist-a', 'specialist-b'])

  if (process.env.CRONY_PREVIEW_COPILOT_FIXTURE === '1') {
    assert.ok(runner.capabilities.some((cap) => cap.name === 'github-copilot' && cap.available),
      'The explicitly selected Copilot fixture must already be available')
    const studioRequest = {
      ...request,
      preferred_adapter: 'github-copilot',
      strategy: 'studio-swarm',
      budget_tokens: 1_000_000,
      budget_cost_microusd: 6_000_000,
      contract: { ...request.contract, write_scope: ['handoffs/**', 'result.md'] },
    }
    const studio = await preview('Studio exact manual-budget allocation', studioRequest, 200,
      { taskCount: 4 })
    assert.deepEqual(studio.tasks.map((task) => task.budget_tokens),
      [150_000, 150_000, 150_000, 550_000])
    assert.deepEqual(studio.tasks.map((task) => task.budget_cost_microusd),
      [900_000, 900_000, 900_000, 3_300_000])
    assert.ok(studio.tasks.slice(0, 3).every((task) =>
      task.depends_on.length === 0 && task.max_attempts === 2))
    assert.deepEqual(studio.tasks[3].depends_on, studio.tasks.slice(0, 3).map((task) => task.key))
    assert.equal(studio.tasks[3].key, 'studio-integration')
    const repeated = await preview('repeated Studio preview', studioRequest, 200, { taskCount: 4 })
    assert.equal(digest(repeated), digest(studio),
      'Studio preview must not persist provisional workers or alter the quote')
  } else {
    report.coverage_limits.push('Studio requires an explicitly selected existing Copilot fixture')
  }

  const invalid = [
    ['empty title', { title: ' \t\r\n ' }, /mission title/],
    ['oversized title', { title: 'x'.repeat(241) }, /mission title/],
    ['UTF-8 title byte bound', { title: 'é'.repeat(121) }, /mission title/],
    ['title control character', { title: 'invalid\u0007title' }, /mission title/],
    ['description control character', { description: 'invalid\u0007description' }, /description/],
    ['description byte bound', { description: 'x'.repeat(100_001) }, /description/],
    ['zero token budget', { budget_tokens: 0 }, /budget/],
    ['zero cost budget', { budget_cost_microusd: 0 }, /budget/],
    ['unknown strategy', { strategy: 'no-such-preview-strategy' }, /strategy/],
    ['unavailable model', { preferred_model: 'no-such-preview-model' }, /model/],
    ['unsupported reasoning', { reasoning_effort: 'no-such-effort' }, /reasoning/],
    ['missing source commit', { source: { ...request.source, base_commit: '' } }, /source/],
    ['stale source commit', { source: { ...request.source, base_commit: '0'.repeat(40) } }, /repository/],
    ['wrong source ref', { source: { ...request.source, base_ref: `preview-${randomUUID()}` } }, /repository/],
    ['wrong source repository', {
      source: { ...request.source, repository: `preview/${randomUUID()}` },
    }, /repository/],
    ['unsafe write scope', { contract: { ...request.contract, write_scope: ['../escape'] } }, /write scope/],
    ['empty verifier policy', { verification_policy: { checks: [], manual_gate: null } }, /verifi/],
    ['invalid verifier timeout', { verification_policy: {
      checks: [{ type: 'command', program: 'node', args: ['--version'], timeout_ms: 0 }], manual_gate: null,
    } }, /verifi|timeout/],
  ]
  for (const [label, overrides, errorPattern] of invalid) {
    await preview(label, { ...request, ...overrides }, 400, { errorPattern })
  }
  await preview('unknown Corp', request, 403, { targetCorp: randomUUID() })
  await preview('unknown or spoofed actor', { ...request, requested_by: randomUUID() }, 403)
  if (health.body.mode === 'development') {
    const guest = humans.find((actor) => !operators.has(actor.role))
    if (guest) {
      await preview('existing non-operator role', { ...request, requested_by: guest.id }, 403,
        { errorPattern: /role/ })
    } else report.coverage_limits.push('No existing guest/spectator; role-denial fixture not created')
    let candidates
    if (process.env.CRONY_PREVIEW_ROOMLESS_ACTOR_ID) {
      const id = identifier(process.env.CRONY_PREVIEW_ROOMLESS_ACTOR_ID, 'CRONY_PREVIEW_ROOMLESS_ACTOR_ID')
      const actor = humans.find((actor) => actor.id === id)
      assert.ok(actor && operators.has(actor.role), 'Room-denial fixture must be an existing operator')
      candidates = [actor]
    } else candidates = humans.filter((actor) => operators.has(actor.role) && actor.id !== actorId).slice(0, 8)
    let roomless
    for (const actor of candidates) {
      const view = await snapshot(actor.id)
      if (view.snapshot.rooms.length === 0) {
        roomless = actor
        break
      }
    }
    if (process.env.CRONY_PREVIEW_ROOMLESS_ACTOR_ID) {
      assert.ok(roomless, 'Explicit room-denial fixture must not belong to any Corp room')
    }
    if (roomless) {
      await preview('existing operator without a room', { ...request, requested_by: roomless.id }, 403,
        { errorPattern: /not a member of any room/ })
    } else report.coverage_limits.push('No existing roomless operator; room-denial fixture not created')
    report.coverage_limits.push('Development-principal RBAC only; production OIDC not exercised')
  } else {
    await preview('missing production bearer', request, 401, { accessToken: '' })
    report.coverage_limits.push('Other-principal role/room denials require separately authenticated fixtures')
  }
  phase = 'delayed no-dispatch receipt'
  await new Promise((resolve) => setTimeout(resolve, 1_100))
  const final = await unchanged(phase)
  report.after = final.receipt
  report.status = 'passed'
} catch (error) {
  report.status = 'failed'
  report.failed_check = phase
  // Do not print response bodies, snapshots, provider details, or connection credentials.
  report.error = String(error.message).replaceAll(token || '\u0000', '[ACCESS_TOKEN]').slice(0, 500)
  process.exitCode = 1
} finally {
  report.finished_at = new Date().toISOString()
  console.log(JSON.stringify(report, null, 2))
}
