import assert from 'node:assert/strict'
import { readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { downloadVerifiedArtifact } from './artifact_client.mjs'

const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const root = path.resolve(import.meta.dirname, '..')

async function request(url, init) {
  const response = await fetch(`${server}${url}`, init)
  const body = response.status === 204 ? null : await response.json()
  return { response, body }
}

async function post(url, body) {
  const result = await request(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
  if (!result.response.ok) {
    throw new Error(`${result.response.status}: ${JSON.stringify(result.body)}`)
  }
  return result.body
}

async function snapshot(demo) {
  return (
    await request(
      `/api/corps/${demo.corp_id}/snapshot?actor_id=${demo.alice_actor_id}`,
    )
  ).body
}

async function waitForRun(demo, runId, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const state = await snapshot(demo)
    const run = state.snapshot.runs.find((candidate) => candidate.id === runId)
    if (
      run &&
      ['completed', 'failed', 'cancelled', 'lost'].includes(run.status) &&
      ['preserved', 'removed'].includes(run.workspace_disposition)
    ) {
      return { state, run }
    }
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error(`timed out waiting for secret run ${runId}`)
}

const demo = await post('/api/demo/reset', {})
const canary = `never-log-${crypto.randomUUID()}`

const guestCreate = await request(`/api/corps/${demo.corp_id}/secrets`, {
  method: 'POST',
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify({
    actor_id: demo.eve_actor_id,
    name: 'guest-secret',
    value: 'denied',
    allowed_tools: ['github'],
    resource_prefix: 'repo:',
  }),
})
assert.equal(guestCreate.response.status, 403)

const created = await post(`/api/corps/${demo.corp_id}/secrets`, {
  actor_id: demo.alice_actor_id,
  name: `test-${crypto.randomUUID()}`,
  value: canary,
  allowed_actor_ids: [demo.alice_actor_id],
  allowed_tools: ['github'],
  resource_prefix: 'repo:shyamsridhar123/',
  max_ttl_seconds: 120,
})

const mission = await post(`/api/corps/${demo.corp_id}/missions`, {
  requested_by: demo.alice_actor_id,
  preferred_adapter: 'fake-process',
  strategy: 'single',
  title: '[secret-probe] prove scoped secret delivery without disclosure',
  secret_refs: [
    {
      secret_id: created.secret_id,
      env_name: 'CRONY_TEST_SECRET',
      tool: 'github',
      resource: 'repo:shyamsridhar123/ecorp',
    },
  ],
})
const launch = await post(
  `/api/corps/${demo.corp_id}/missions/${mission.mission_id}/launch`,
  { requested_by: demo.alice_actor_id },
)
const completed = await waitForRun(demo, launch.run_id)
assert.equal(completed.run.status, 'completed')
const artifact = (
  await downloadVerifiedArtifact(server, demo, completed.run)
).toString('utf8')
assert.ok(artifact.includes('Task-scoped secret available: yes.'))
assert.ok(!artifact.includes(canary))
assert.ok(!JSON.stringify(completed.state).includes(canary))

const deniedMission = await post(`/api/corps/${demo.corp_id}/missions`, {
  requested_by: demo.bob_actor_id,
  preferred_adapter: 'fake-process',
  strategy: 'single',
  title: '[secret-probe] unauthorized actor must not receive the secret',
  secret_refs: [
    {
      secret_id: created.secret_id,
      env_name: 'CRONY_TEST_SECRET',
      tool: 'github',
      resource: 'repo:shyamsridhar123/ecorp',
    },
  ],
})
const deniedLaunch = await request(
  `/api/corps/${demo.corp_id}/missions/${deniedMission.mission_id}/launch`,
  {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ requested_by: demo.bob_actor_id }),
  },
)
assert.equal(deniedLaunch.response.status, 409)

await post(`/api/corps/${demo.corp_id}/secrets/${created.secret_id}/revoke`, {
  actor_id: demo.alice_actor_id,
  reason: 'secret broker conformance test complete',
})

for (const logName of ['server.stdout.log', 'server.stderr.log', 'runner.stdout.log', 'runner.stderr.log']) {
  const log = await readFile(path.join(root, 'output', logName), 'utf8').catch(() => '')
  assert.ok(!log.includes(canary), `${logName} leaked the secret canary`)
}

const report = {
  checked_at: new Date().toISOString(),
  owner_created_secret: true,
  guest_create_status: guestCreate.response.status,
  task_scoped_delivery: true,
  unauthorized_actor_denied: deniedLaunch.response.status,
  plaintext_absent_from_snapshot_events_and_logs: true,
  revoked: true,
  assurance: 'environment_reduced_assurance',
}
await writeFile(
  path.join(root, 'output', 'e2e-secrets.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
