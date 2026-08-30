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

function connectRunner({ corpId, runnerId, credential }) {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(`${devServer.replace(/^http/, 'ws')}/ws/runner`)
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
        connection_epoch: crypto.randomUUID(),
        hostname: 'identity-test',
        os: process.platform,
        capabilities: [],
        active_runs: [],
      }))
    }
    socket.onmessage = (message) => {
      const payload = JSON.parse(message.data)
      if (payload.type === 'registered' || payload.type === 'registration_rejected') {
        clearTimeout(timeout)
        resolve({ socket, payload })
      }
    }
  })
}

const demo = (
  await json(`${devServer}/api/demo/bootstrap`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: '{}',
  })
).body

const runnerId = `identity-probe-${crypto.randomUUID()}`
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
})
assert.equal(first.payload.type, 'registered')
const rotatedCredential = first.payload.credential
first.socket.close()

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
})
assert.equal(second.payload.type, 'registered')
const currentCredential = second.payload.credential

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
const serverBinary = path.join(
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
      '--mode',
      'production',
      '--oidc-issuer',
      issuer,
      '--allow-insecure-oidc',
      '--secret-master-key-hex',
      'a5c3f1458279dfb241239378dbefa6b8d2ab32703cba1768343712fd37ac1f04',
    ],
    { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] },
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
