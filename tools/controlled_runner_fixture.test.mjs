import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import test from 'node:test'
import {
  artifactStagingFixtureConfig,
  assertAutomaticFactoryVerification,
  assertControlledAssignment,
  controlledReadinessCapability,
  controlledReadinessSource,
  selectFixtureRunnerForSource,
  waitForControlledRunnerDispatch,
} from './controlled_runner_fixture.mjs'

const root = path.resolve(import.meta.dirname, '..')
const smokeEnv = {
  CRONY_ARTIFACT_STAGING_TEST: '1', CRONY_SERVER_HTTP: 'http://127.0.0.1:18437',
  CRONY_ARTIFACT_STAGING_OUTPUT: path.join(root, 'output', 'controlled-readiness-test.json'),
}
const actionsEnv = {
  CRONY_ARTIFACT_STAGING_TEST: '1', CRONY_SERVER_HTTP: 'http://127.0.0.1:18471',
  GITHUB_ACTIONS: 'true', CI: 'true', RUNNER_OS: 'Linux', GITHUB_JOB: 'integration',
  GITHUB_RUN_ID: '12345', GITHUB_WORKSPACE: root,
  CRONY_TEST_SERVER_PID_FILE: path.join(root, 'output', 'server-ci.json'),
  DATABASE_URL: 'synthetic-fixture-database-not-disclosed',
}
const demo = { corp_id: '00000000-0000-4000-8000-000000000001', alice_actor_id: 'owner-fixture' }
const runnerId = 'aaa-artifact-staging-00000000-0000-4000-8000-000000000001'
const epoch = '00000000-0000-4000-8000-000000000002'
const source = controlledReadinessSource(runnerId, epoch)
const runner = { runnerId, readinessSource: source }

test('model-scoped readiness selects its probe despite another runner with the same source', async () => {
  const actual = { repository: 'all-the-vibes/ecorp', base_ref: 'HEAD', base_commit: 'b'.repeat(40) }
  const probe = { runnerId: 'zz-controlled-probe', readinessSource: actual, adapter: 'codex', modelId: 'controlled-model' }
  const workspace = { name: 'workspace-isolation', available: true, source_repository: actual.repository,
    source_base_ref: actual.base_ref, source_base_commit: actual.base_commit }
  const capability = id => ({ name: 'codex', available: true, models: [{ id, policy_state: 'enabled' }] })
  const state = { runners: [
    { id: 'aaa-normal-runner', connected: true, corp_id: demo.corp_id, capabilities: [workspace, capability('normal-model')] },
    { id: probe.runnerId, connected: true, corp_id: demo.corp_id, capabilities: [workspace, capability(probe.modelId)] },
  ], snapshot: { missions: [], tasks: [], runs: [] } }
  let previews = 0
  const request = async (_route, init) => {
    if (!init) return { response: { status: 200 }, body: state }
    previews++
    const body = JSON.parse(init.body)
    assert.equal(body.preferred_adapter, 'codex')
    assert.equal(body.preferred_model, probe.modelId)
    assert.deepEqual(body.source, actual)
    return { response: { status: 200 }, body: {} }
  }
  assert.equal(await waitForControlledRunnerDispatch({ request, demo, runner: probe }), 1)
  assert.equal(previews, 1)
  state.runners[0].capabilities[1] = capability(probe.modelId)
  await assert.rejects(waitForControlledRunnerDispatch({ request, demo, runner: probe }))
  assert.equal(previews, 1, 'Ambiguity must reject before another preview')
})

test('a missing or disabled probe model cannot borrow another runtime or source', async () => {
  const probe = { runnerId, readinessSource: source, adapter: 'codex', modelId: 'probe-model' }
  for (const models of [[], [{ id: 'other-model' }], [{ id: probe.modelId, policy_state: 'disabled' }]]) {
    let previews = 0
    const request = async (_route, init) => {
      if (init) { previews++; throw new Error('Must reject before a preview') }
      return { response: { status: 200 }, body: { runners: [{ id: runnerId, connected: true, corp_id: demo.corp_id,
        capabilities: [controlledReadinessCapability(source), { name: 'codex', available: true, models }] }],
      snapshot: { missions: [], tasks: [], runs: [] } } }
    }
    await assert.rejects(waitForControlledRunnerDispatch({ request, demo, runner: probe }))
    assert.equal(previews, 0)
  }
})

test('artifact smoke requires explicit owned opt-in, loopback origin and output', () => {
  const config = artifactStagingFixtureConfig(['--readiness-smoke', '--dry-run'], smokeEnv)
  assert.equal(config.readinessSmoke, true)
  assert.equal(config.dryRun, true)
  assert.equal(config.output, smokeEnv.CRONY_ARTIFACT_STAGING_OUTPUT)
  for (const args of [['--skip'], ['--dry-run', '--dry-run'], ['--readiness-smoke', '--unknown']]) {
    assert.throws(() => artifactStagingFixtureConfig(args, smokeEnv), /option/)
  }
  for (const [key, value] of [
    ['CRONY_ARTIFACT_STAGING_TEST', undefined], ['CRONY_SERVER_HTTP', undefined],
    ['CRONY_SERVER_HTTP', 'http://127.0.0.1:8791'], ['CRONY_SERVER_HTTP', 'http://example.com:18437'],
    ['CRONY_SERVER_HTTP', 'http://user:secret@127.0.0.1:18437'], ['CRONY_SERVER_HTTP', 'http://127.0.0.1:18437/path'],
    ['CRONY_SERVER_HTTP', 'http://127.0.0.1:18437/?query'], ['CRONY_SERVER_HTTP', 'http://127.0.0.1:18437/#fragment'],
    ['CRONY_ARTIFACT_STAGING_OUTPUT', undefined], ['CRONY_ARTIFACT_STAGING_OUTPUT', 'relative.json'],
  ]) assert.throws(() => artifactStagingFixtureConfig(['--readiness-smoke'], { ...smokeEnv, [key]: value }))
})

test('full SQL and restart coverage remains restricted to the exact Actions integration fixture', () => {
  const config = artifactStagingFixtureConfig([], actionsEnv)
  assert.equal(config.readinessSmoke, false)
  assert.equal(config.output, path.join(root, 'output', 'e2e-artifact-staging.json'))
  assert.equal(JSON.stringify(config).includes(actionsEnv.DATABASE_URL), false)
  for (const [key, value] of [
    ['GITHUB_ACTIONS', undefined], ['CI', undefined], ['RUNNER_OS', 'Windows'], ['GITHUB_JOB', 'quality'],
    ['GITHUB_RUN_ID', '0'], ['GITHUB_WORKSPACE', path.dirname(root)],
    ['CRONY_TEST_SERVER_PID_FILE', path.join(root, 'output', 'local-pids.json')],
    ['CRONY_SERVER_HTTP', 'http://127.0.0.1:18437'], ['DATABASE_URL', undefined],
  ]) assert.throws(() => artifactStagingFixtureConfig([], { ...actionsEnv, [key]: value }))
  assert.throws(() => artifactStagingFixtureConfig([], smokeEnv), /restricted/)
})

test('the readiness marker is synthetic, unique to the connection and explicitly not checkout evidence', () => {
  assert.deepEqual(source, controlledReadinessSource(runnerId, epoch))
  assert.notDeepEqual(source, controlledReadinessSource(runnerId, '00000000-0000-4000-8000-000000000003'))
  assert.equal(source.repository, 'fixture/' + runnerId)
  assert.equal(source.base_ref, 'readiness-only')
  assert.match(source.base_commit, /^[0-9a-f]{40}$/)
  const cap = controlledReadinessCapability(source)
  assert.equal(cap.source_repository, source.repository)
  assert.equal(cap.source_base_ref, source.base_ref)
  assert.equal(cap.source_base_commit, source.base_commit)
  assert.match(cap.detail, /preview only, not checkout evidence/)
  assert.throws(() => controlledReadinessSource('runner-local', epoch))
})

function mock({ delays = 0, status, error, mutate } = {}) {
  const state = {
    runners: [{ id: runnerId, corp_id: demo.corp_id, connected: true,
      capabilities: [controlledReadinessCapability(source), { name: 'fake-process', available: true }] }],
    snapshot: { missions: [], tasks: [], runs: [] },
  }
  if (mutate) mutate(state)
  const calls = []
  return {
    calls,
    request: async (route, init) => {
      calls.push({ route, init })
      if (!init) return { response: { status: 200 }, body: structuredClone(state) }
      assert.ok(route.endsWith('/missions/preview'), 'Only read-only previews may be retried')
      const observed = status ?? (delays-- > 0 ? 400 : 200)
      return { response: { status: observed }, body: observed === 200 ? { tasks: [] } : {
        error: error ?? 'no connected runner can staff the selected mission runtime, model, and source',
      } }
    },
  }
}

test('registration alone cannot satisfy readiness; only bounded source-specific read-only previews are retried', async () => {
  const api = mock({ delays: 2 })
  const previews = await waitForControlledRunnerDispatch({ request: api.request, demo, runner }, { wait: async () => {} })
  assert.equal(previews, 3)
  assert.equal(api.calls.length, 6)
  for (const call of api.calls.filter(call => call.init)) {
    assert.equal(call.init.method, 'POST')
    const body = JSON.parse(call.init.body)
    assert.deepEqual(body.source, source)
    assert.equal(body.preferred_adapter, 'fake-process')
    assert.equal(body.requested_by, demo.alice_actor_id)
  }
  assert.equal(api.calls.some(call => call.route.endsWith('/launch') || call.route.endsWith('/missions')), false)
})

test('wrong, disconnected, ambiguous or unavailable controlled runner cannot pass the preview gate', async () => {
  for (const mutate of [
    state => { state.runners[0].id = 'runner-local' },
    state => { state.runners[0].connected = false },
    state => { state.runners[0].corp_id = 'foreign' },
    state => { state.runners.push(structuredClone(state.runners[0])) },
    state => { state.runners[0].capabilities[0].source_base_commit = 'b'.repeat(40) },
    state => { state.runners[0].capabilities[0].workspace_connection_id = 'foreign' },
    state => { state.runners[0].capabilities[1].available = false },
    ...['missions', 'tasks', 'runs'].map(collection => state => { state.snapshot[collection].push({ id: 'unexpected' }) }),
  ]) {
    const api = mock({ mutate })
    await assert.rejects(waitForControlledRunnerDispatch({ request: api.request, demo, runner }))
    assert.equal(api.calls.length, 1)
  }
})

test('readiness fails closed on unrelated HTTP errors and has a deterministic deadline', async () => {
  for (const status of [401, 403, 409, 500]) {
    const api = mock({ status })
    await assert.rejects(waitForControlledRunnerDispatch({ request: api.request, demo, runner }), /preview failure/)
    assert.equal(api.calls.length, 2)
  }
  const unrelated = mock({ status: 400, error: 'another validation error' })
  await assert.rejects(waitForControlledRunnerDispatch({ request: unrelated.request, demo, runner }))
  let clock = 0
  const unavailable = mock({ delays: 100 })
  await assert.rejects(waitForControlledRunnerDispatch({ request: unavailable.request, demo, runner }, {
    now: () => { clock += 10_000; return clock }, wait: async () => {},
  }), /Timed out waiting for controlled-runner dispatch readiness/)
  assert.ok(unavailable.calls.length <= 6)
})

test('every artifact launch must belong to the controlled runner before waiting for an assignment', () => {
  assertControlledAssignment({ run_id: 'run-fixture', runner_id: runnerId }, runner)
  assert.throws(() => assertControlledAssignment({ run_id: 'run-fixture', runner_id: 'runner-local' }, runner), /different runner/)
  assert.throws(() => assertControlledAssignment({ runner_id: runnerId }, runner), /omitted/)
})

test('source-bound fixture readiness verifies the expected repository, ref, commit, Corp and unique runner', () => {
  const expected = { repository: 'all-the-vibes/ecorp', base_ref: 'HEAD', base_commit: 'a'.repeat(40) }
  const cap = { ...controlledReadinessCapability(expected), source_repository: 'All-The-Vibes/ecorp' }
  const state = { runners: [{ id: 'runner-local', corp_id: demo.corp_id, connected: true, capabilities: [cap] }] }
  const selected = selectFixtureRunnerForSource(state, demo, expected)
  assert.equal(selected.runnerId, 'runner-local')
  assert.equal(selected.readinessSource.repository, 'All-The-Vibes/ecorp')
  for (const changed of [
    { repository: 'shyamsridhar123/ecorp' }, { base_ref: 'main' }, { base_commit: 'b'.repeat(40) },
    { base_commit: '' },
  ]) assert.throws(() => selectFixtureRunnerForSource(state, demo, { ...expected, ...changed }))
  for (const change of [
    value => { value.runners[0].connected = false },
    value => { value.runners[0].corp_id = 'other-corp' },
    value => { value.runners[0].capabilities[0].available = false },
    value => { value.runners[0].capabilities[0].workspace_connection_id = 'another-connection' },
    value => { value.runners.push(structuredClone(value.runners[0])) },
  ]) {
    const changed = structuredClone(state)
    change(changed)
    assert.throws(() => selectFixtureRunnerForSource(changed, demo, expected))
  }
})

test('automatic factory verification requires exact persisted completion, scope, version and event lineage', () => {
  const expected = { corpId: 'corp', workItemId: 'item', missionId: 'mission', runId: 'run', previousVersion: 4 }
  const state = { snapshot: {
    factory_work_items: [{ id: 'item', corp_id: 'corp', mission_id: 'mission', state: 'verified', version: 5 }],
    missions: [{ id: 'mission', corp_id: 'corp', status: 'completed' }],
    tasks: [{ id: 'task', corp_id: 'corp', mission_id: 'mission', status: 'completed', verification_status: 'passed' }],
    runs: [{ id: 'run', corp_id: 'corp', task_id: 'task', status: 'completed', verification_status: 'passed' }],
    events: [{ type: 'factory.verified', aggregate_id: 'item', aggregate_version: 5, corp_id: 'corp', actor_id: null,
      correlation_id: 'mission', causation_id: 'run', idempotency_key: 'factory:item:verified:run',
      payload: { previous_state: 'running', state: 'verified', mission_id: 'mission', run_id: 'run' } }],
  } }
  const before = structuredClone(state)
  assert.equal(assertAutomaticFactoryVerification(state, expected).version, 5)
  assert.deepEqual(state, before)
  for (const change of [
    snapshot => { snapshot.factory_work_items = [] },
    snapshot => { snapshot.factory_work_items[0].state = 'running' },
    snapshot => { snapshot.factory_work_items[0].corp_id = 'other' },
    snapshot => { snapshot.factory_work_items[0].version = 6 },
    snapshot => { snapshot.missions[0].status = 'running' },
    snapshot => { snapshot.tasks[0].verification_status = 'pending' },
    snapshot => { snapshot.runs[0].verification_status = 'pending' },
    snapshot => { snapshot.runs[0].task_id = 'other-task' },
    snapshot => { snapshot.events = [] },
    snapshot => { snapshot.events.push(structuredClone(snapshot.events[0])) },
    snapshot => { snapshot.events[0].type = 'factory.state_changed' },
    snapshot => { snapshot.events[0].actor_id = 'second-operator' },
    snapshot => { snapshot.events[0].aggregate_version = 4 },
    snapshot => { snapshot.events[0].causation_id = 'other-run' },
    snapshot => { snapshot.events[0].idempotency_key = 'different-key' },
    snapshot => { snapshot.events[0].payload.run_id = 'other-run' },
  ]) {
    const changed = structuredClone(state)
    change(changed.snapshot)
    assert.throws(() => assertAutomaticFactoryVerification(changed, expected))
  }
})

test('Linux keeps the full artifact suite and Windows smoke cannot claim fault or restart coverage', () => {
  const read = file => readFileSync(new URL(file, import.meta.url), 'utf8').replaceAll('\r\n', '\n')
  const script = read('./e2e_artifact_staging.mjs')
  const workflow = read('../.github/workflows/ci.yml')
  assert.match(workflow, /run: node --test .*tools\/controlled_runner_fixture\.test\.mjs/)
  const linux = workflow.split('\n  integration:')[1].split('\n  external-adapters-windows:')[0]
  assert.match(linux, /node tools\/e2e_artifact_staging\.mjs --dry-run\n\s+node tools\/e2e_artifact_staging\.mjs\n/)
  assert.doesNotMatch(linux, /--readiness-smoke|continue-on-error/)
  assert.match(read('./ci_external_adapters_windows.ps1'), /'controlled-runner-readiness'.*'--readiness-smoke'/)
  assert.match(script, /coverage: 'controlled_runner_readiness_only'/)
  assert.match(script, /fault_injection_executed: false, server_restarted: false/)
  assert.ok(script.indexOf('assertControlledAssignment(launch, runner)') < script.indexOf('runner.waitForAssignment(launch.run_id)'))
  const main = script.slice(script.indexOf("const demo = await post('/api/demo/reset'"))
  assert.ok(main.indexOf('await waitForControlledRunnerDispatch') < main.indexOf('await acceptedArtifact('))
  const smoke = main.slice(main.indexOf('if (config.readinessSmoke)'), main.indexOf('const rejected ='))
  assert.ok(smoke.indexOf('process.exit(0)') < smoke.indexOf('await psql('))
  for (const coverage of ['accepted_shared_digest', 'breaker_rejection_before_staging', 'database_reservation_failure',
    'database_failure_after_object_publication', 'restart_recovery', 'periodic_recovery']) {
    assert.ok(script.includes(coverage + ': {'), 'Missing artifact coverage: ' + coverage)
  }
})
