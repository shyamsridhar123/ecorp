import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import test from 'node:test'
import { controlledReadinessCapability, controlledReadinessSource } from './controlled_runner_fixture.mjs'
import { assertIdentityAssignment, assertIdentityProbeSelection, captureIdentityAssignment,
  identityFixtureConfig, identityFixturePreview, waitForIdentityProbe } from './identity_fixture.mjs'

const root = path.resolve(import.meta.dirname, '..')
const id = n => `00000000-0000-4000-8000-${String(n).padStart(12, '0')}`
const demo = { corp_id: id(1), alice_actor_id: id(2) }
const runnerId = 'identity-probe-' + id(3)
const source = { repository: 'all-the-vibes/ecorp', base_ref: 'HEAD', base_commit: 'a'.repeat(40) }
const modelId = 'identity-lifecycle-' + id(4)
const fake = { name: 'fake-process', available: true }
const probe = { runnerId, readinessSource: source, adapter: 'codex', modelId }
const probeCapabilities = [{ name: 'codex', available: true, models: [{ id: modelId, policy_state: 'enabled' }] },
  { name: 'workspace-isolation', available: true, source_repository: source.repository,
    source_base_ref: source.base_ref, source_base_commit: source.base_commit }]
const localEnv = { CRONY_IDENTITY_TEST: '1', CRONY_SERVER_HTTP: 'http://127.0.0.1:18449',
  CRONY_IDENTITY_OUTPUT: path.join(root, 'output', 'e2e-identity-lifecycle.json') }
const actionsEnv = { CRONY_IDENTITY_TEST: '1', CRONY_SERVER_HTTP: 'http://127.0.0.1:18471',
  GITHUB_ACTIONS: 'true', CI: 'true', RUNNER_OS: 'Linux', GITHUB_JOB: 'integration', GITHUB_RUN_ID: '12345',
  GITHUB_WORKSPACE: root, CRONY_TEST_SERVER_PID_FILE: path.join(root, 'output', 'server-ci.json'),
  CRONY_TEST_SERVER_BINARY: path.join(root, 'target', 'debug', 'crony-server'),
  DATABASE_URL: 'postgres://crony:crony@127.0.0.1:55471/crony', CRONY_AUTH_TEST_PORT: '18473', FAKE_OIDC_PORT: '18472' }

function snapshot() {
  return { runners: [
    { id: 'runner-fixture', connected: true, corp_id: demo.corp_id, capabilities: [fake] },
    { id: runnerId, connected: true, corp_id: demo.corp_id, capabilities: structuredClone(probeCapabilities) },
  ], snapshot: { missions: [], tasks: [], runs: [] } }
}

test('lifecycle-only preview requires an explicit owned scope and cannot claim OIDC coverage', () => {
  const config = identityFixtureConfig(['--lifecycle-only', '--dry-run'], localEnv)
  const plan = identityFixturePreview(config)
  assert.equal(config.lifecycleOnly, true)
  assert.equal(config.productionPort, null)
  assert.equal(config.oidcPort, null)
  assert.equal(plan.services_started, false)
  assert.equal(plan.database_writes, false)
  assert.equal(plan.credentials_disclosed, false)
  assert.throws(() => identityFixtureConfig([], localEnv), /restricted/u)
  for (const args of [['--execute'], ['--dry-run', '--dry-run'], ['--lifecycle-only', '--unknown']]) {
    assert.throws(() => identityFixtureConfig(args, localEnv), /option/u)
  }
})

test('identity fixture rejects manual, foreign and credential-bearing endpoints before effects', () => {
  for (const [key, value] of [
    ['CRONY_IDENTITY_TEST', undefined], ['CRONY_SERVER_HTTP', undefined],
    ['CRONY_SERVER_HTTP', 'http://127.0.0.1:8791'], ['CRONY_SERVER_HTTP', 'http://127.0.0.1:54329'],
    ['CRONY_SERVER_HTTP', 'https://127.0.0.1:18449'], ['CRONY_SERVER_HTTP', 'http://example.com:18449'],
    ['CRONY_SERVER_HTTP', 'http://user:secret@127.0.0.1:18449'], ['CRONY_SERVER_HTTP', 'http://127.0.0.1:18449/path'],
    ['CRONY_SERVER_HTTP', 'http://127.0.0.1:18449/?query'], ['CRONY_SERVER_HTTP', 'http://127.0.0.1:18449/#fragment'],
    ['CRONY_IDENTITY_OUTPUT', undefined], ['CRONY_IDENTITY_OUTPUT', 'relative/e2e-identity-lifecycle.json'],
    ['CRONY_IDENTITY_OUTPUT', path.join(root, 'other.json')],
  ]) assert.throws(() => identityFixtureConfig(['--lifecycle-only'], { ...localEnv, [key]: value }))
})

test('full identity/OIDC conformance stays confined to the exact hosted integration fixture', () => {
  const config = identityFixtureConfig(['--dry-run'], actionsEnv)
  assert.equal(config.lifecycleOnly, false)
  assert.equal(config.output, path.join(root, 'output', 'e2e-identity.json'))
  assert.equal(config.productionPort, 18473)
  assert.equal(config.oidcPort, 18472)
  assert.equal(JSON.stringify(identityFixturePreview(config)).includes(actionsEnv.DATABASE_URL), false)
  for (const [key, value] of [
    ['GITHUB_ACTIONS', undefined], ['CI', undefined], ['RUNNER_OS', 'Windows'], ['GITHUB_JOB', 'quality'],
    ['GITHUB_RUN_ID', '0'], ['GITHUB_WORKSPACE', undefined], ['GITHUB_WORKSPACE', path.dirname(root)],
    ['CRONY_TEST_SERVER_PID_FILE', path.join(root, 'output', 'local-pids.json')],
    ['CRONY_TEST_SERVER_BINARY', undefined], ['CRONY_SERVER_HTTP', localEnv.CRONY_SERVER_HTTP],
    ['DATABASE_URL', undefined], ['DATABASE_URL', 'private-value-not-to-be-printed'],
    ['CRONY_AUTH_TEST_PORT', '8793'], ['FAKE_OIDC_PORT', '8792'],
  ]) assert.throws(() => identityFixtureConfig([], { ...actionsEnv, [key]: value }))
})

test('identity markers are per-connection preview-only data, without changing artifact markers', () => {
  const legacyId = 'aaa-identity-probe-' + id(3)
  const marker = controlledReadinessSource(legacyId, id(4))
  assert.notDeepEqual(marker, controlledReadinessSource(legacyId, id(5)))
  assert.equal(marker.base_ref, 'readiness-only')
  assert.equal(controlledReadinessSource('aaa-artifact-staging-' + id(3), id(4)).base_commit, marker.base_commit)
  assert.match(controlledReadinessCapability(marker).detail, /preview only, not checkout evidence/u)
  assert.throws(() => controlledReadinessSource('runner-local', id(4)))
})

test('model/source selection rejects ambiguity, wrong scope, missing capabilities and prior work', () => {
  assert.doesNotThrow(() => assertIdentityProbeSelection(snapshot(), demo, probe))
  for (const change of [
    state => { state.runners[1].connected = false },
    state => { state.runners[1].corp_id = id(99) },
    state => { state.runners[1].capabilities = [] },
    state => { state.runners[0].capabilities = structuredClone(probeCapabilities) },
    state => { state.runners[1].capabilities[0].models[0].id = 'different-model' },
    state => { state.runners[1].capabilities[0].models[0].policy_state = 'disabled' },
    state => { state.runners[1].capabilities[1].source_base_commit = 'b'.repeat(40) },
    state => { state.runners.push(structuredClone(state.runners[1])) },
    state => { state.snapshot.missions.push({ id: id(6) }) },
    state => { state.snapshot.tasks.push({ id: id(6) }) },
    state => { state.snapshot.runs.push({ id: id(6) }) },
  ]) { const state = snapshot(); change(state); assert.throws(() => assertIdentityProbeSelection(state, demo, probe)) }
  const state = snapshot()
  state.runners.unshift({ id: 'aaa-foreign', connected: true, corp_id: id(99), capabilities: [fake] })
  state.runners.unshift({ id: 'aaa-connection', connected: true, corp_id: demo.corp_id,
    capabilities: [{ ...fake, workspace_connection_id: id(7) }] })
  assert.doesNotThrow(() => assertIdentityProbeSelection(state, demo, probe))
})

test('identity readiness retries only native read-only previews before any mission or launch', async () => {
  let previews = 0
  const request = async (route, init) => {
    if (!init) return { response: { status: 200 }, body: snapshot() }
    assert.ok(route.endsWith('/missions/preview'))
    assert.equal(init.method, 'POST')
    assert.deepEqual(JSON.parse(init.body).source, source)
    assert.equal(JSON.parse(init.body).preferred_model, modelId)
    assert.equal(JSON.parse(init.body).preferred_adapter, 'codex')
    previews++
    return previews === 1 ? { response: { status: 400 }, body: { error: 'no matching runner was selectable' } }
      : { response: { status: 200 }, body: {} }
  }
  assert.equal(await waitForIdentityProbe({ request, demo, probe }), 2)
})

const launch = { runner_id: runnerId, run_id: id(10) }
const expected = { corpId: demo.corp_id, missionId: id(11), probe }
function assignment() {
  return { type: 'start_run', run_id: launch.run_id, corp_id: expected.corpId, mission_id: expected.missionId,
    task_id: id(12), agent_id: id(13), assignment_token: id(14), adapter: 'codex', model: modelId, secrets: [],
    source_repository: source.repository, source_base_ref: source.base_ref, source_base_commit: source.base_commit }
}

test('assignment binds exact source/model, scope and fence without leaking secrets', () => {
  assert.doesNotThrow(() => assertIdentityAssignment(assignment(), launch, expected))
  for (const changed of [
    { run_id: id(99) }, { corp_id: id(99) }, { mission_id: id(99) }, { adapter: 'fake-process' }, { model: 'other-model' },
    { workspace_connection_id: id(99) }, { source_repository: 'other/source' },
    { source_base_ref: 'other-ref' }, { source_base_commit: 'b'.repeat(40) },
    { task_id: null }, { agent_id: null }, { assignment_token: null },
    { secrets: [{ value: 'must-not-be-reflected' }] },
  ]) {
    assert.throws(() => assertIdentityAssignment({ ...assignment(), ...changed }, launch, expected), error => {
      assert.equal(error.message.includes('must-not-be-reflected'), false)
      return true
    })
  }
})

class Socket extends EventTarget {
  active = new Set()
  addEventListener(type, callback) { this.active.add(callback); super.addEventListener(type, callback) }
  removeEventListener(type, callback) { this.active.delete(callback); super.removeEventListener(type, callback) }
  frame(data) { this.dispatchEvent(new MessageEvent('message', { data: JSON.stringify(data) })) }
}
function clock() {
  return { tick: null, cleared: false,
    schedule(callback) { this.tick = callback; return 123 },
    unschedule(token) { assert.equal(token, 123); this.cleared = true } }
}
async function capture(launchRequest) {
  const socket = new Socket(), timer = clock()
  const options = { schedule: timer.schedule.bind(timer), unschedule: timer.unschedule.bind(timer) }
  const result = captureIdentityAssignment({ ...expected, probe: { ...probe, socket },
    launch: () => launchRequest(socket, timer) }, options)
  return { result, socket, timer }
}

test('assignment listener exists before launch and is removed with its timer after success', async () => {
  const { result, socket, timer } = await capture(async socket => {
    socket.frame({ type: 'heartbeat' })
    socket.frame(assignment())
    return launch
  })
  assert.deepEqual((await result).assignment, assignment())
  assert.equal(socket.active.size, 0)
  assert.equal(timer.cleared, true)
})

test('wrong-runner launch fails immediately without a dangling assignment deadline', async () => {
  let calls = 0
  const { result, socket, timer } = await capture(async () => { calls++; return { ...launch, runner_id: 'runner-other' } })
  await assert.rejects(result, /different runner/u)
  assert.equal(calls, 1)
  assert.equal(socket.active.size, 0)
  assert.equal(timer.cleared, true)
})

test('failed launch cancels assignment listening without masking the HTTP error', async () => {
  const { result, socket, timer } = await capture(async () => { throw new Error('HTTP fixture rejection') })
  await assert.rejects(result, /HTTP fixture rejection/u)
  assert.equal(socket.active.size, 0)
  assert.equal(timer.cleared, true)
})

test('assignment timeout stays observed during launch and cleans up deterministically', async () => {
  const { result, socket, timer } = await capture(async (_socket, timer) => {
    timer.tick()
    await new Promise(resolve => setImmediate(resolve))
    return launch
  })
  await assert.rejects(result, /timed out waiting for lifecycle assignment/u)
  assert.equal(socket.active.size, 0)
  assert.equal(timer.cleared, true)
})

test('closed and malformed probe streams fail without waiting for the full deadline', async () => {
  for (const event of ['close', 'error', 'malformed']) {
    const { result, socket, timer } = await capture(async socket => {
      socket.dispatchEvent(event === 'malformed' ? new MessageEvent('message', { data: 'invalid-json' }) : new Event(event))
      return launch
    })
    await assert.rejects(result, /closed|failed|Malformed/u)
    assert.equal(socket.active.size, 0)
    assert.equal(timer.cleared, true)
  }
})

test('Linux retains full auth coverage and Windows lifecycle evidence cannot claim OIDC', () => {
  const workflow = readFileSync(path.join(root, '.github', 'workflows', 'ci.yml'), 'utf8').replace(/\r\n/gu, '\n')
  const integration = workflow.split('  integration:')[1].split('  external-adapters-windows:')[0]
  assert.match(integration, /CRONY_IDENTITY_TEST: '1'/u)
  assert.match(integration, /CRONY_AUTH_TEST_PORT: '18473'/u)
  assert.match(integration, /FAKE_OIDC_PORT: '18472'/u)
  assert.match(integration, /node tools\/e2e_identity\.mjs --dry-run\n\s+node tools\/e2e_identity\.mjs\n/u)
  assert.doesNotMatch(integration, /e2e_identity\.mjs --lifecycle-only/u)
  const script = readFileSync(path.join(root, 'tools', 'e2e_identity.mjs'), 'utf8')
  assert.ok(script.indexOf('const readinessPreviews = await') < script.indexOf('const mission = await'))
  assert.ok(script.indexOf('if (config.lifecycleOnly)') < script.indexOf('const link = await json'))
  assert.match(script, /coverage: 'runner_identity_lifecycle_only', oidc_executed: false/u)
  for (const check of ['unauthenticated.response.status, 401', 'authenticated.response.status, 200',
    'spoofedActor.response.status, 403', 'unknownIdentity.response.status, 403', 'otherCorp.response.status, 403',
    "revoked.payload.type, 'registration_rejected'", 'revokedRun.status', 'wrong_assignment_token_rejected: true']) {
    assert.ok(script.includes(check), 'Existing identity assertion/evidence must remain: ' + check)
  }
  const wrapper = readFileSync(path.join(root, 'tools', 'ci_external_adapters_windows.ps1'), 'utf8')
  assert.match(wrapper, /identity-lifecycle.*e2e_identity\.mjs.*--lifecycle-only/u)
  assert.match(wrapper, /identityReport\.oidc_executed/u)
})
