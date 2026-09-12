import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import test from 'node:test'
import {
  artifactStagingFixtureConfig,
  assertControlledAssignment,
  controlledReadinessCapability,
  controlledReadinessSource,
  waitForControlledRunnerDispatch,
} from './controlled_runner_fixture.mjs'

const root = path.resolve(import.meta.dirname, '..')
const smokeEnv = {
  CRONY_ARTIFACT_STAGING_TEST: '1', CRONY_SERVER_HTTP: 'http://127.0.0.1:18437',
  CRONY_ARTIFACT_STAGING_OUTPUT: path.join(root, 'output', 'controlled-readiness-test.json'),
}
const actionsEnv = {
  CRONY_ARTIFACT_STAGING_TEST: '1', CRONY_SERVER_HTTP: 'http://127.0.0.1:8791',
  GITHUB_ACTIONS: 'true', CI: 'true', RUNNER_OS: 'Linux', GITHUB_JOB: 'integration',
  GITHUB_RUN_ID: '12345', GITHUB_WORKSPACE: root,
  CRONY_TEST_SERVER_PID_FILE: path.join(root, 'output', 'server-ci.pid'),
  DATABASE_URL: 'synthetic-fixture-database-not-disclosed',
}
const demo = { corp_id: '00000000-0000-4000-8000-000000000001', alice_actor_id: 'owner-fixture' }
const runnerId = 'aaa-artifact-staging-00000000-0000-4000-8000-000000000001'
const epoch = '00000000-0000-4000-8000-000000000002'
const source = controlledReadinessSource(runnerId, epoch)
const runner = { runnerId, readinessSource: source }

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
