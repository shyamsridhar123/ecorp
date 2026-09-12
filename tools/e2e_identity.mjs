import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import fs from 'node:fs/promises'
import path from 'node:path'

const devServer = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const databaseUrl =
  process.env.DATABASE_URL ?? 'postgres://crony:crony@127.0.0.1:54329/crony'
const root = path.resolve(import.meta.dirname, '..')
const productionPort = Number(process.env.CRONY_AUTH_TEST_PORT ?? 8793)
const oidcPort = Number(process.env.FAKE_OIDC_PORT ?? 8792)
const productionServer = `http://127.0.0.1:${productionPort}`
const productionSocket = `ws://127.0.0.1:${productionPort}`
const issuer = `http://127.0.0.1:${oidcPort}`

async function json(url, init) {
  const response = await fetch(url, init)
  const body = response.status === 204 ? null : await response.json()
  return { response, body }
}

async function waitFor(url, predicate, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    try {
      const result = await json(url)
      if (result.response.ok && predicate(result.body)) return result.body
    } catch {
      // Service is still starting.
    }
    await new Promise((resolve) => setTimeout(resolve, 200))
  }
  throw new Error(`timed out waiting for ${url}`)
}

function connectRunner({
  corpId,
  runnerId,
  credential,
  capabilities = [],
  activeRuns = [],
}) {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(`${devServer.replace(/^http/, 'ws')}/ws/runner`)
    const connectionEpoch = crypto.randomUUID()
    const timeout = setTimeout(() => {
      socket.close()
      reject(new Error('timed out waiting for runner registration'))
    }, 10_000)
    socket.onerror = () => {
      clearTimeout(timeout)
      reject(new Error('runner websocket failed'))
    }
    socket.onopen = () => {
      socket.send(JSON.stringify({
        type: 'register',
        runner_id: runnerId,
        corp_id: corpId,
        credential,
        connection_epoch: connectionEpoch,
        hostname: 'identity-test',
        os: process.platform,
        capabilities,
        active_runs: activeRuns,
      }))
    }
    socket.onmessage = (message) => {
      const payload = JSON.parse(message.data)
      if (payload.type === 'registered' || payload.type === 'registration_rejected') {
        clearTimeout(timeout)
        resolve({ socket, payload, connectionEpoch })
      }
    }
  })
}

async function devPost(pathname, body) {
  const result = await json(`${devServer}${pathname}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
  if (!result.response.ok) {
    throw new Error(`${result.response.status}: ${JSON.stringify(result.body)}`)
  }
  return result.body
}

async function devSnapshot(demo) {
  const result = await json(
    `${devServer}/api/corps/${demo.corp_id}/snapshot?actor_id=${demo.alice_actor_id}`,
  )
  if (!result.response.ok) {
    throw new Error(`${result.response.status}: ${JSON.stringify(result.body)}`)
  }
  return result.body
}

async function waitForSnapshot(demo, predicate, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const state = await devSnapshot(demo)
    const value = predicate(state)
    if (value) return { state, value }
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error('timed out waiting for runner identity state')
}

const demo = (
  await json(`${devServer}/api/demo/reset`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: '{}',
  })
).body

const runnerId = `identity-probe-${crypto.randomUUID()}`
const lifecycleModel = `identity-lifecycle-${crypto.randomUUID()}`
const initial = await devSnapshot(demo)
const workspace = initial.runners.filter((runner) => runner.connected)
  .flatMap((runner) => runner.capabilities)
  .find((capability) => capability.name === 'workspace-isolation' &&
    capability.available && capability.workspace_connection_id == null)
assert.ok(workspace?.source_repository && workspace.source_base_ref)
assert.match(workspace.source_base_commit, /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/i)
const lifecycleSource = {
  repository: workspace.source_repository,
  base_ref: workspace.source_base_ref,
  base_commit: workspace.source_base_commit,
}
const fakeCapability = {
  name: 'codex',
  available: true,
  detail: 'controlled identity protocol probe; no provider execution',
  models: [{
    id: lifecycleModel,
    name: 'Controlled identity lifecycle fixture',
    policy_state: 'enabled',
    supports_vision: false,
    supports_reasoning_effort: false,
  }],
}
const lifecycleCapabilities = [fakeCapability, {
  name: 'workspace-isolation',
  available: true,
  detail: 'controlled identity fixture source',
  models: [],
  source_repository: lifecycleSource.repository,
  source_base_ref: lifecycleSource.base_ref,
  source_base_commit: lifecycleSource.base_commit,
}]
const enrollment = await json(
  `${devServer}/api/corps/${demo.corp_id}/runners/enroll`,
  {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      actor_id: demo.alice_actor_id,
      runner_id: runnerId,
      expires_in_seconds: 600,
    }),
  },
)
assert.equal(enrollment.response.status, 200)

const first = await connectRunner({
  corpId: demo.corp_id,
  runnerId,
  credential: enrollment.body.enrollment_token,
  capabilities: lifecycleCapabilities,
})
assert.equal(first.payload.type, 'registered')
const rotatedCredential = first.payload.credential

const replay = await connectRunner({
  corpId: demo.corp_id,
  runnerId,
  credential: enrollment.body.enrollment_token,
})
assert.equal(replay.payload.type, 'registration_rejected')
replay.socket.close()

const second = await connectRunner({
  corpId: demo.corp_id,
  runnerId,
  credential: rotatedCredential,
  capabilities: lifecycleCapabilities,
})
assert.equal(second.payload.type, 'registered')
const currentCredential = second.payload.credential

// Registration/credential rotation precedes native dispatch readiness. Select
// this protocol simulator by its unique model/source, never by runner ordering.
const readyDeadline = Date.now() + 10_000
let dispatchReady = false
while (Date.now() < readyDeadline) {
  const preview = await json(`${devServer}/api/corps/${demo.corp_id}/missions/preview`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      requested_by: demo.alice_actor_id,
      title: 'Identity fixture dispatch readiness',
      preferred_adapter: 'codex',
      preferred_model: lifecycleModel,
      source: lifecycleSource,
    }),
    signal: AbortSignal.timeout(Math.max(1, readyDeadline - Date.now())),
  })
  if (preview.response.ok) {
    dispatchReady = true
    break
  }
  assert.equal(preview.response.status, 400)
  assert.equal(preview.body?.error,
    'no connected runner can staff the selected mission runtime, model, and source')
  await new Promise((resolve) => setTimeout(resolve, 50))
}
assert.ok(dispatchReady, 'current identity fixture did not become dispatch-ready')

const assignmentPromise = new Promise((resolve, reject) => {
  const timeout = setTimeout(
    () => reject(new Error('timed out waiting for lifecycle assignment')),
    10_000,
  )
  const listener = (message) => {
    const payload = JSON.parse(message.data)
    if (payload.type === 'start_run') {
      clearTimeout(timeout)
      second.socket.removeEventListener('message', listener)
      resolve(payload)
    }
  }
  second.socket.addEventListener('message', listener)
})
const mission = await devPost(`/api/corps/${demo.corp_id}/missions`, {
  requested_by: demo.alice_actor_id,
  preferred_adapter: 'codex',
  preferred_model: lifecycleModel,
  source: lifecycleSource,
  title: 'Verify superseded runner fencing and active revocation.',
})
const launch = await devPost(
  `/api/corps/${demo.corp_id}/missions/${mission.mission_id}/launch`,
  { requested_by: demo.alice_actor_id },
)
const assignment = await assignmentPromise
assert.equal(assignment.run_id, launch.run_id)
assert.equal(
  (await devSnapshot(demo)).snapshot.runs.find((run) => run.id === assignment.run_id)?.runner_id,
  runnerId,
)

first.socket.send(JSON.stringify({
  type: 'run_event',
  event_id: crypto.randomUUID(),
  runner_id: runnerId,
  corp_id: demo.corp_id,
  connection_epoch: first.connectionEpoch,
  run_id: assignment.run_id,
  agent_id: assignment.agent_id,
  assignment_token: assignment.assignment_token,
  event_type: 'run.started',
  payload: { station: 'terminal' },
}))
await new Promise((resolve) => setTimeout(resolve, 300))
const supersededView = await devSnapshot(demo)
assert.equal(
  supersededView.snapshot.runs.find((run) => run.id === assignment.run_id)?.status,
  'starting',
)
assert.equal(
  supersededView.snapshot.events.some(
    (event) =>
      event.aggregate_id === assignment.run_id && event.type === 'run.started',
  ),
  false,
)

const wrongTokenMarker = `wrong-assignment-${crypto.randomUUID()}`
second.socket.send(JSON.stringify({
  type: 'run_event',
  event_id: crypto.randomUUID(),
  runner_id: runnerId,
  corp_id: demo.corp_id,
  connection_epoch: second.connectionEpoch,
  run_id: assignment.run_id,
  agent_id: assignment.agent_id,
  assignment_token: crypto.randomUUID(),
  event_type: 'run.output',
  payload: { stream: 'control', text: wrongTokenMarker },
}))
await new Promise((resolve) => setTimeout(resolve, 300))
const wrongTokenView = await devSnapshot(demo)
assert.equal(
  wrongTokenView.snapshot.events.some(
    (event) => event.payload.text === wrongTokenMarker,
  ),
  false,
)

second.socket.send(JSON.stringify({
  type: 'run_event',
  event_id: crypto.randomUUID(),
  runner_id: runnerId,
  corp_id: demo.corp_id,
  connection_epoch: second.connectionEpoch,
  run_id: assignment.run_id,
  agent_id: assignment.agent_id,
  assignment_token: assignment.assignment_token,
  event_type: 'run.started',
  payload: { station: 'terminal' },
}))
await waitForSnapshot(
  demo,
  (state) =>
    state.snapshot.runs.find((run) => run.id === assignment.run_id)?.status ===
    'running',
)

const revoke = await json(
  `${devServer}/api/corps/${demo.corp_id}/runners/${runnerId}/revoke`,
  {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      actor_id: demo.alice_actor_id,
      reason: 'identity conformance test',
    }),
  },
)
assert.equal(revoke.response.status, 200)
assert.equal(revoke.body.revoked, true)
const revokedState = await waitForSnapshot(demo, (state) => {
  const run = state.snapshot.runs.find(
    (candidate) => candidate.id === assignment.run_id,
  )
  return run?.status === 'lost' ? state : null
})
const revokedRun = revokedState.value.snapshot.runs.find(
  (run) => run.id === assignment.run_id,
)
const revokedTask = revokedState.value.snapshot.tasks.find(
  (task) => task.id === revokedRun.task_id,
)
const revokedMission = revokedState.value.snapshot.missions.find(
  (candidate) => candidate.id === mission.mission_id,
)
const revokedAgent = revokedState.value.snapshot.agents.find(
  (agent) => agent.id === revokedRun.agent_id,
)
assert.equal(revokedTask.status, 'blocked')
assert.equal(revokedMission.status, 'failed')
assert.equal(revokedAgent.status, 'idle')
assert.equal(revokedAgent.current_run_id, null)
first.socket.close()
second.socket.close()

const revoked = await connectRunner({
  corpId: demo.corp_id,
  runnerId,
  credential: currentCredential,
})
assert.equal(revoked.payload.type, 'registration_rejected')
revoked.socket.close()

const link = await json(`${devServer}/api/demo/oidc-link`, {
  method: 'POST',
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify({
    actor_id: demo.alice_actor_id,
    issuer,
    subject: 'alice-subject',
    email: 'alice@example.test',
  }),
})
assert.equal(link.response.status, 204)

await fs.mkdir(path.join(root, 'output'), { recursive: true })
const oidc = spawn(process.execPath, [path.join(root, 'scripts', 'fake-oidc.mjs')], {
  cwd: root,
  env: { ...process.env, FAKE_OIDC_PORT: String(oidcPort) },
  stdio: ['ignore', 'pipe', 'pipe'],
})
const serverBinary =
  process.env.CRONY_TEST_SERVER_BINARY ??
  path.join(
    root,
    'target',
    'debug',
    process.platform === 'win32' ? 'crony-server.exe' : 'crony-server',
  )
let production
try {
  await waitFor(`${issuer}/.well-known/openid-configuration`, () => true)
  production = spawn(
    serverBinary,
    [
      '--bind',
      `127.0.0.1:${productionPort}`,
      '--database-url',
      databaseUrl,
      '--runner-startup-recovery',
      'false',
      '--mode',
      'production',
      '--oidc-issuer',
      issuer,
      '--allow-insecure-oidc',
      '--secret-master-key-hex',
      'a5c3f1458279dfb241239378dbefa6b8d2ab32703cba1768343712fd37ac1f04',
    ],
    {
      cwd: root,
      env: {
        ...process.env,
        CRONY_OBJECT_STORE_BACKEND: 's3',
        CRONY_OBJECT_STORE_ENDPOINT: 'https://s3.invalid',
        CRONY_OBJECT_STORE_BUCKET: 'crony-identity-test',
        CRONY_OBJECT_STORE_REGION: 'us-east-1',
        CRONY_OBJECT_STORE_ACCESS_KEY: 'identity-test',
        CRONY_OBJECT_STORE_SECRET_KEY: 'identity-test-secret',
        CRONY_OBJECT_STORE_ALLOW_HTTP: 'false',
        CRONY_ARTIFACT_SIGNING_KEY_HEX: '4c'.repeat(32),
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    },
  )
  await waitFor(`${productionServer}/health`, (body) => body.mode === 'production')

  const unauthenticated = await json(
    `${productionServer}/api/corps/${demo.corp_id}/snapshot?actor_id=${demo.alice_actor_id}`,
  )
  assert.equal(unauthenticated.response.status, 401)

  const authenticated = await json(
    `${productionServer}/api/corps/${demo.corp_id}/snapshot?actor_id=${demo.alice_actor_id}`,
    { headers: { authorization: 'Bearer alice-token' } },
  )
  assert.equal(authenticated.response.status, 200)

  const spoofedActor = await json(
    `${productionServer}/api/corps/${demo.corp_id}/snapshot?actor_id=${demo.bob_actor_id}`,
    { headers: { authorization: 'Bearer alice-token' } },
  )
  assert.equal(spoofedActor.response.status, 403)

  const unknownIdentity = await json(
    `${productionServer}/api/corps/${demo.corp_id}/snapshot?actor_id=${demo.alice_actor_id}`,
    { headers: { authorization: 'Bearer unknown-token' } },
  )
  assert.equal(unknownIdentity.response.status, 403)

  const otherCorp = await json(
    `${productionServer}/api/corps/${crypto.randomUUID()}/snapshot?actor_id=${demo.alice_actor_id}`,
    { headers: { authorization: 'Bearer alice-token' } },
  )
  assert.equal(otherCorp.response.status, 403)

  const ticket = await json(
    `${productionServer}/api/corps/${demo.corp_id}/ws-ticket`,
    {
      method: 'POST',
      headers: {
        authorization: 'Bearer alice-token',
        'content-type': 'application/json',
      },
      body: JSON.stringify({ actor_id: demo.alice_actor_id }),
    },
  )
  assert.equal(ticket.response.status, 200)

  const replayReady = await new Promise((resolve, reject) => {
    const socket = new WebSocket(
      `${productionSocket}/ws/corps/${demo.corp_id}?ticket=${ticket.body.ticket}&after_seq=0`,
    )
    const timeout = setTimeout(() => reject(new Error('OIDC websocket timed out')), 10_000)
    socket.onerror = () => {
      clearTimeout(timeout)
      reject(new Error('OIDC websocket failed'))
    }
    socket.onmessage = (message) => {
      const payload = JSON.parse(message.data)
      if (payload.type === 'ready') {
        clearTimeout(timeout)
        socket.close()
        resolve(payload)
      }
    }
  })
  assert.equal(replayReady.corp_id, demo.corp_id)

  const evidence = {
    checked_at: new Date().toISOString(),
    oidc_missing_token_status: unauthenticated.response.status,
    oidc_authenticated_status: authenticated.response.status,
    actor_spoof_status: spoofedActor.response.status,
    unmapped_identity_status: unknownIdentity.response.status,
    cross_corp_status: otherCorp.response.status,
    websocket_authorized_before_replay: true,
    enrollment_rotated: rotatedCredential !== enrollment.body.enrollment_token,
    enrollment_replay_rejected: true,
    workload_credential_rotated: currentCredential !== rotatedCredential,
    revocation_rejected: true,
    superseded_runner_event_rejected: true,
    wrong_assignment_token_rejected: true,
    active_revocation_run_status: revokedRun.status,
    active_revocation_task_status: revokedTask.status,
    active_revocation_mission_status: revokedMission.status,
    active_revocation_agent_status: revokedAgent.status,
  }
  await fs.writeFile(
    path.join(root, 'output', 'e2e-identity.json'),
    `${JSON.stringify(evidence, null, 2)}\n`,
  )
  console.log(JSON.stringify(evidence, null, 2))
} finally {
  production?.kill()
  oidc.kill()
}
