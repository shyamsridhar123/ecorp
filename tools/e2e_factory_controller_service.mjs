import assert from 'node:assert/strict'
import { writeFile } from 'node:fs/promises'
import path from 'node:path'
import { randomUUID } from 'node:crypto'

const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const root = path.resolve(import.meta.dirname, '..')

async function raw(url, init) {
  const response = await fetch(`${server}${url}`, init)
  const body = await response.json()
  return { response, body }
}

async function post(url, body) {
  const result = await raw(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
  if (!result.response.ok) {
    throw new Error(`${result.response.status}: ${JSON.stringify(result.body)}`)
  }
  return result.body
}

async function postRaw(url, body) {
  return raw(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
}

async function snapshot(demo) {
  const result = await raw(
    `/api/corps/${demo.corp_id}/snapshot?actor_id=${demo.alice_actor_id}`,
  )
  assert.equal(result.response.status, 200)
  return result.body
}

const demo = await post('/api/demo/reset', {})
const controllerId = randomUUID()
const firstEpoch = randomUUID()
const configureKey = randomUUID()
const controllerPath = `/api/corps/${demo.corp_id}/factory/controllers`
const controlPath = `/api/corps/${demo.corp_id}/factory/controllers/${controllerId}/control`
const heartbeatPath = `/api/corps/${demo.corp_id}/factory/controllers/${controllerId}/heartbeat`
const configuration = {
  actor_id: demo.alice_actor_id,
  controller_id: controllerId,
  source_project_owner: 'shyamsridhar123',
  source_project_number: 3,
  source_repository_owner: 'shyamsridhar123',
  source_repository_name: 'ecorp',
  connection_epoch: firstEpoch,
  lease_seconds: 10,
  idempotency_key: configureKey,
}

const configured = await post(controllerPath, configuration)
assert.equal(configured.replayed, false)
assert.equal(configured.controller.status, 'watching')
assert.equal(configured.controller.desired_state, 'running')
const replayed = await post(controllerPath, configuration)
assert.equal(replayed.replayed, true)
assert.equal(replayed.controller.id, controllerId)

const memberDenied = await postRaw(controlPath, {
  actor_id: demo.bob_actor_id,
  expected_version: configured.controller.version,
  action: 'pause',
  idempotency_key: randomUUID(),
})
assert.equal(memberDenied.response.status, 403)

const pauseRequest = {
  actor_id: demo.alice_actor_id,
  expected_version: configured.controller.version,
  action: 'pause',
  idempotency_key: randomUUID(),
}
const paused = await post(controlPath, pauseRequest)
assert.equal(paused.controller.desired_state, 'paused')
assert.equal(paused.controller.version, configured.controller.version + 1)
const pausedReplay = await post(controlPath, pauseRequest)
assert.equal(pausedReplay.replayed, true)
assert.equal(pausedReplay.controller.version, paused.controller.version)

const resumed = await post(controlPath, {
  actor_id: demo.alice_actor_id,
  expected_version: paused.controller.version,
  action: 'resume',
  idempotency_key: randomUUID(),
})
assert.equal(resumed.controller.desired_state, 'running')
assert.equal(resumed.controller.reconcile_generation, 1)

const reconciled = await post(heartbeatPath, {
  actor_id: demo.alice_actor_id,
  connection_epoch: firstEpoch,
  lease_seconds: 10,
  active_work_item_id: null,
  completed_reconcile_generation: 1,
  reconcile_result: 'succeeded',
  error: null,
})
assert.equal(reconciled.controller.completed_reconcile_generation, 1)
assert.equal(reconciled.controller.last_reconcile_result, 'succeeded')
assert.equal(reconciled.controller.status, 'watching')

const requested = await post(controlPath, {
  actor_id: demo.alice_actor_id,
  expected_version: resumed.controller.version,
  action: 'reconcile',
  idempotency_key: randomUUID(),
})
assert.equal(requested.controller.reconcile_generation, 2)

const failed = await post(heartbeatPath, {
  actor_id: demo.alice_actor_id,
  connection_epoch: firstEpoch,
  lease_seconds: 10,
  active_work_item_id: null,
  completed_reconcile_generation: 2,
  reconcile_result: 'failed',
  error: 'fixture reconciliation failed\nwithout leaking credentials',
})
assert.equal(failed.controller.status, 'blocked')
assert.equal(
  failed.controller.last_error,
  'fixture reconciliation failed without leaking credentials',
)

await new Promise((resolve) => setTimeout(resolve, 10_500))
const offlineSnapshot = await snapshot(demo)
const offline = offlineSnapshot.snapshot.factory_controllers.find(
  (controller) => controller.id === controllerId,
)
assert.equal(offline.status, 'offline')

const secondEpoch = randomUUID()
const reconnected = await post(controllerPath, {
  ...configuration,
  connection_epoch: secondEpoch,
  lease_seconds: 30,
  idempotency_key: randomUUID(),
})
assert.equal(reconnected.controller.status, 'blocked')
const stale = await postRaw(heartbeatPath, {
  actor_id: demo.alice_actor_id,
  connection_epoch: firstEpoch,
  lease_seconds: 30,
  active_work_item_id: null,
  completed_reconcile_generation: null,
  reconcile_result: null,
  error: null,
})
assert.equal(stale.response.status, 400)
const recovered = await post(heartbeatPath, {
  actor_id: demo.alice_actor_id,
  connection_epoch: secondEpoch,
  lease_seconds: 30,
  active_work_item_id: null,
  completed_reconcile_generation: 2,
  reconcile_result: 'succeeded',
  error: null,
})
assert.equal(recovered.controller.status, 'watching')

const finalSnapshot = await snapshot(demo)
assert.equal(finalSnapshot.snapshot.factory_controllers.length, 1)
const report = {
  checked_at: new Date().toISOString(),
  controller_id: controllerId,
  configure_replay: true,
  member_control_denied: true,
  pause_resume: true,
  reconcile_generation: recovered.controller.reconcile_generation,
  completed_reconcile_generation:
    recovered.controller.completed_reconcile_generation,
  failed_reconcile_blocked: true,
  lease_expiry_offline: true,
  stale_epoch_rejected: true,
  reconnect_recovered: true,
}
await writeFile(
  path.join(root, 'output', 'e2e-factory-controller-service.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
