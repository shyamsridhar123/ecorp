import assert from 'node:assert/strict'
import path from 'node:path'
import { assertControlledAssignment, waitForControlledRunnerDispatch } from './controlled_runner_fixture.mjs'

const root = path.resolve(import.meta.dirname, '..')
const reserved = new Set(['5432', '54329', '8791', '8793', '5187', '5291', '15191', '15193'])
const uuid = /^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$/u

export function identityFixtureConfig(args, env) {
  assert.ok(args.every(arg => ['--dry-run', '--lifecycle-only'].includes(arg)) &&
    new Set(args).size === args.length, 'Unknown or repeated identity fixture option')
  assert.equal(env.CRONY_IDENTITY_TEST, '1', 'Explicit owned identity fixture opt-in is required')
  let endpoint
  try { endpoint = new URL(env.CRONY_SERVER_HTTP) } catch { throw new Error('An explicit owned identity fixture URL is required; its value was not disclosed') }
  assert.ok(endpoint.protocol === 'http:' && ['127.0.0.1', 'localhost', '[::1]'].includes(endpoint.hostname) &&
    endpoint.port && !reserved.has(endpoint.port) && endpoint.pathname === '/' && !endpoint.search &&
    !endpoint.hash && !endpoint.username && !endpoint.password, 'Expected an owned, non-manual loopback HTTP origin')
  const lifecycleOnly = args.includes('--lifecycle-only')
  if (lifecycleOnly) {
    assert.ok(env.CRONY_IDENTITY_OUTPUT && path.isAbsolute(env.CRONY_IDENTITY_OUTPUT) &&
      path.basename(env.CRONY_IDENTITY_OUTPUT) === 'e2e-identity-lifecycle.json',
    'The lifecycle-only fixture requires an explicit owned evidence file')
  } else {
    assert.ok(env.GITHUB_ACTIONS === 'true' && env.CI === 'true' && env.RUNNER_OS === 'Linux' &&
      env.GITHUB_JOB === 'integration' && /^[1-9][0-9]*$/u.test(env.GITHUB_RUN_ID ?? ''),
    'Full identity/OIDC conformance is restricted to the Actions integration fixture')
    assert.ok(env.GITHUB_WORKSPACE && path.isAbsolute(env.GITHUB_WORKSPACE) &&
      path.resolve(env.GITHUB_WORKSPACE) === root, 'Unexpected Actions identity workspace')
    assert.equal(endpoint.origin, 'http://127.0.0.1:18471', 'Unexpected Actions identity endpoint')
    assert.equal(env.CRONY_TEST_SERVER_PID_FILE, path.join(root, 'output', 'server-ci.json'))
    assert.equal(env.CRONY_TEST_SERVER_BINARY, path.join(root, 'target', 'debug', 'crony-server'))
    assert.ok(env.DATABASE_URL === 'postgres://crony:crony@127.0.0.1:55471/crony',
      'Expected the owned Actions service database; values are not disclosed')
    assert.equal(env.CRONY_AUTH_TEST_PORT, '18473', 'The production-mode fixture port must be explicit')
    assert.equal(env.FAKE_OIDC_PORT, '18472', 'The fake issuer port must be explicit')
  }
  return { server: endpoint.origin, lifecycleOnly, dryRun: args.includes('--dry-run'),
    output: lifecycleOnly ? env.CRONY_IDENTITY_OUTPUT : path.join(root, 'output', 'e2e-identity.json'),
    productionPort: lifecycleOnly ? null : 18473, oidcPort: lifecycleOnly ? null : 18472 }
}

export function identityFixturePreview(config) {
  return { ...config, services_started: false, database_writes: false, credentials_disclosed: false,
    proposed: ['reset only the explicitly owned synthetic fixture',
      'check native enrollment rotation and replay rejection',
      'wait for source-specific read-only readiness, then verify exact probe assignment',
      'check superseded-epoch and assignment-token fencing plus active revocation',
      config.lifecycleOnly ? 'stop here; no OIDC service or production-mode server is started'
        : 'run the existing full OIDC, Corp RBAC and WebSocket authorization assertions'] }
}

export function assertIdentityProbeSelection(state, demo, probe) {
  // Reuse upstream's exact model/source selection. Runner ordering is not
  // authority, and this protocol simulator does not execute a real provider.
  assert.match(probe.runnerId, /^identity-probe-[0-9a-f-]{36}$/u)
  assert.equal(probe.adapter, 'codex')
  assert.match(probe.modelId, /^identity-lifecycle-[0-9a-f-]{36}$/u)
  const source = probe.readinessSource
  assert.ok(source?.repository && source.base_ref && /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/u.test(source.base_commit))
  const candidates = state.runners.filter(runner => runner.connected && runner.corp_id === demo.corp_id &&
    runner.capabilities.some(cap => cap.name === probe.adapter && cap.available && cap.workspace_connection_id == null &&
      cap.models?.some(model => model.id === probe.modelId && model.policy_state !== 'disabled')) &&
    runner.capabilities.some(cap => cap.name === 'workspace-isolation' && cap.available && cap.workspace_connection_id == null &&
      cap.source_repository === source.repository && cap.source_base_ref === source.base_ref && cap.source_base_commit === source.base_commit))
    .map(runner => runner.id)
  assert.deepEqual(candidates, [probe.runnerId], 'The explicit model/source must select only the identity probe')
  for (const collection of ['missions', 'tasks', 'runs']) {
    assert.equal(state.snapshot[collection].length, 0, 'Identity readiness must precede all fixture work')
  }
}

export async function waitForIdentityProbe({ request, demo, probe }) {
  const previews = await waitForControlledRunnerDispatch({ request, demo, runner: probe })
  const { response, body } = await request(`/api/corps/${demo.corp_id}/snapshot?actor_id=${demo.alice_actor_id}`)
  assert.equal(response.status, 200, 'Cannot verify identity probe selection order')
  assertIdentityProbeSelection(body, demo, probe)
  return previews
}

export function assertIdentityAssignment(assignment, launch, { corpId, missionId, probe }) {
  assert.equal(assignment.type, 'start_run')
  assert.equal(assignment.run_id, launch.run_id, 'Identity assignment must belong to the launched run')
  assert.equal(assignment.corp_id, corpId, 'Identity assignment must remain in the expected Corp')
  assert.equal(assignment.mission_id, missionId, 'Identity assignment must remain in the expected mission')
  assert.equal(assignment.adapter, probe.adapter)
  assert.equal(assignment.model, probe.modelId)
  assert.ok(assignment.workspace_connection_id == null, 'Identity fixture uses the legacy synthetic connection')
  assert.deepEqual({ repository: assignment.source_repository, base_ref: assignment.source_base_ref,
    base_commit: assignment.source_base_commit }, probe.readinessSource, 'Identity assignment must retain its exact source tuple')
  for (const field of ['task_id', 'agent_id', 'assignment_token']) {
    assert.ok(uuid.test(assignment[field] ?? ''), 'Identity assignment omitted a native identifier/fence')
  }
  assert.ok(Array.isArray(assignment.secrets) && assignment.secrets.length === 0,
    'Identity fixture must not receive brokered secrets')
}

export async function captureIdentityAssignment({ probe, corpId, missionId, launch }, {
  timeoutMs = 10_000, schedule = setTimeout, unschedule = clearTimeout,
} = {}) {
  let timer, message, closed, failed
  const pending = new Promise((resolve, reject) => {
    message = event => {
      let payload
      try { payload = JSON.parse(event.data) } catch { reject(new Error('Malformed identity probe frame')); return }
      if (payload.type === 'start_run') resolve(payload)
    }
    closed = () => reject(new Error('Identity probe closed before assignment'))
    failed = () => reject(new Error('Identity probe connection failed before assignment'))
    probe.socket.addEventListener('message', message)
    probe.socket.addEventListener('close', closed)
    probe.socket.addEventListener('error', failed)
    timer = schedule(() => reject(new Error('timed out waiting for lifecycle assignment')), timeoutMs)
  })
  // HTTP launch can still be in flight when the assignment deadline fires.
  // Keep its rejection observed, then propagate it through the awaited promise.
  pending.catch(() => {})
  try {
    const result = await launch()
    assertControlledAssignment(result, probe)
    const assignment = await pending
    assertIdentityAssignment(assignment, result, { corpId, missionId, probe })
    return { launch: result, assignment }
  } finally {
    unschedule(timer)
    probe.socket.removeEventListener('message', message)
    probe.socket.removeEventListener('close', closed)
    probe.socket.removeEventListener('error', failed)
  }
}
