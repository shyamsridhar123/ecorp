import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import path from 'node:path'

const root = path.resolve(import.meta.dirname, '..')
const reservedPorts = ['8791', '8793', '5187', '5291', '15191', '15193']

export function artifactStagingFixtureConfig(args, env) {
  assert.ok(args.every(arg => ['--dry-run', '--readiness-smoke'].includes(arg)) &&
    new Set(args).size === args.length, 'Unknown or repeated artifact fixture option')
  assert.equal(env.CRONY_ARTIFACT_STAGING_TEST, '1', 'Explicit artifact staging fixture opt-in is required')
  assert.ok(env.CRONY_SERVER_HTTP, 'An explicit owned CRONY_SERVER_HTTP is required')
  const endpoint = new URL(env.CRONY_SERVER_HTTP)
  assert.ok(endpoint.protocol === 'http:' && ['127.0.0.1', 'localhost', '[::1]'].includes(endpoint.hostname) &&
    endpoint.port && endpoint.pathname === '/' && !endpoint.search && !endpoint.hash &&
    !endpoint.username && !endpoint.password, 'Expected an explicit loopback HTTP origin')
  const readinessSmoke = args.includes('--readiness-smoke')
  if (readinessSmoke) {
    assert.ok(!reservedPorts.includes(endpoint.port), 'Refusing a manual-stack port for the readiness smoke')
    assert.ok(env.CRONY_ARTIFACT_STAGING_OUTPUT && path.isAbsolute(env.CRONY_ARTIFACT_STAGING_OUTPUT),
      'The readiness smoke requires an explicit owned output file')
  } else {
    // The legacy full suite injects SQL faults and restarts a PID-recorded server.
    // Keep those effects confined to its existing disposable hosted CI stack.
    assert.ok(env.GITHUB_ACTIONS === 'true' && env.CI === 'true' && env.RUNNER_OS === 'Linux' &&
      env.GITHUB_JOB === 'integration' && /^[1-9][0-9]*$/u.test(env.GITHUB_RUN_ID ?? ''),
    'Full artifact fault injection is restricted to the Actions integration fixture')
    assert.equal(path.resolve(env.GITHUB_WORKSPACE ?? ''), root, 'Unexpected Actions workspace')
    assert.equal(endpoint.origin, 'http://127.0.0.1:8791')
    assert.equal(env.CRONY_TEST_SERVER_PID_FILE, path.join(root, 'output', 'server-ci.pid'))
    assert.ok(env.DATABASE_URL, 'The owned Actions service database must be explicit')
  }
  return {
    server: endpoint.origin, readinessSmoke, dryRun: args.includes('--dry-run'),
    output: readinessSmoke ? env.CRONY_ARTIFACT_STAGING_OUTPUT : path.join(root, 'output', 'e2e-artifact-staging.json'),
  }
}

export function controlledReadinessSource(runnerId, connectionEpoch) {
  assert.match(runnerId, /^aaa-artifact-staging-[0-9a-f-]{36}$/u)
  assert.match(connectionEpoch, /^[0-9a-f-]{36}$/u)
  // A synthetic, per-connection marker for preview selection only. No mission
  // is executed against this marker and it is not evidence of a Git checkout.
  return {
    repository: 'fixture/' + runnerId,
    base_ref: 'readiness-only',
    base_commit: createHash('sha256').update(connectionEpoch).digest('hex').slice(0, 40),
  }
}

export function controlledReadinessCapability(source) {
  return {
    name: 'workspace-isolation', available: true, models: [],
    detail: 'synthetic controlled-runner readiness marker; preview only, not checkout evidence',
    source_repository: source.repository, source_base_ref: source.base_ref, source_base_commit: source.base_commit,
  }
}

export async function waitForControlledRunnerDispatch({ request, demo, runner }, {
  now = Date.now, wait = ms => new Promise(resolve => setTimeout(resolve, ms)),
} = {}) {
  const source = runner.readinessSource
  assert.ok(source?.repository && source.base_ref && source.base_commit)
  const prefix = `/api/corps/${demo.corp_id}`
  const snapshotRoute = `${prefix}/snapshot?actor_id=${demo.alice_actor_id}`
  const deadline = now() + 30_000
  let previews = 0
  while (now() < deadline) {
    const { response, body: state } = await request(snapshotRoute)
    assert.equal(response.status, 200, 'Cannot verify controlled runner identity')
    const candidates = state.runners.filter(candidate => candidate.connected && candidate.corp_id === demo.corp_id &&
      candidate.capabilities.some(cap => cap.name === 'workspace-isolation' && cap.available &&
        cap.workspace_connection_id == null && cap.source_repository === source.repository &&
        cap.source_base_ref === source.base_ref && cap.source_base_commit === source.base_commit))
    assert.deepEqual(candidates.map(candidate => candidate.id), [runner.runnerId],
      'The readiness marker must identify exactly the controlled runner')
    assert.ok(candidates[0].capabilities.some(cap => cap.name === 'fake-process' && cap.available &&
      cap.workspace_connection_id == null), 'The controlled fixture adapter must be available')
    for (const collection of ['missions', 'tasks', 'runs']) {
      assert.equal(state.snapshot[collection].length, 0, 'Readiness must precede all fixture work')
    }
    const preview = await request(`${prefix}/missions/preview`, {
      method: 'POST', headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ requested_by: demo.alice_actor_id, preferred_adapter: 'fake-process',
        title: 'Preview controlled artifact runner readiness only.', source }),
    })
    previews++
    if (preview.response.status === 200) return previews
    assert.equal(preview.response.status, 400, 'Unexpected controlled-runner preview failure')
    assert.match(preview.body.error ?? '', /no connected runner can staff|no matching runner was selectable/u)
    await wait(100)
  }
  throw new Error('Timed out waiting for controlled-runner dispatch readiness')
}

export function assertControlledAssignment(launch, runner) {
  assert.equal(launch.runner_id, runner.runnerId, 'Artifact fixture assignment went to a different runner')
  assert.ok(launch.run_id, 'Artifact fixture launch omitted the assigned run')
}

export function selectFixtureRunnerForSource(state, demo, expected) {
  assert.ok(expected.repository && expected.base_ref && /^[0-9a-f]{40}$/u.test(expected.base_commit),
    'The fixture must declare its expected immutable checkout')
  const matches = state.runners.filter(runner => runner.connected && runner.corp_id === demo.corp_id)
    .flatMap(runner => runner.capabilities.filter(cap => cap.name === 'workspace-isolation' && cap.available &&
      cap.workspace_connection_id == null && cap.source_repository?.toLowerCase() === expected.repository.toLowerCase() &&
      cap.source_base_ref === expected.base_ref && cap.source_base_commit === expected.base_commit)
      .map(cap => ({ runnerId: runner.id, readinessSource: { repository: cap.source_repository,
        base_ref: cap.source_base_ref, base_commit: cap.source_base_commit } })))
  assert.equal(matches.length, 1, 'Expected exactly one owned runner advertising the declared immutable checkout')
  return matches[0]
}
