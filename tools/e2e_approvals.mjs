import assert from 'node:assert/strict'
import { restartOwnedTestServer } from './owned_test_stack.mjs'
import { writeFile } from 'node:fs/promises'
import path from 'node:path'

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

async function waitFor(demo, predicate, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const state = await snapshot(demo)
    const value = predicate(state)
    if (value) return { state, value }
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error('timed out waiting for approval scenario')
}

async function restartServer() {
  if (process.env.CRONY_SKIP_SERVER_RESTART === '1') return false
  const pidPath = process.env.CRONY_TEST_SERVER_PID_FILE
  if (!pidPath) return false
  const databaseUrl = process.env.DATABASE_URL
  if (!databaseUrl) {
    throw new Error('DATABASE_URL is required for the approval restart test')
  }
  await restartOwnedTestServer({ root, server, databaseUrl, logPrefix: 'approval-restart' })
  return true
}

const demo = await post('/api/demo/reset', {})
const mission = await post(`/api/corps/${demo.corp_id}/missions`, {
  requested_by: demo.alice_actor_id,
  preferred_adapter: 'fake-process',
  title: '[approval-action] suspend before a risky external side effect',
})
const launch = await post(
  `/api/corps/${demo.corp_id}/missions/${mission.mission_id}/launch`,
  { requested_by: demo.alice_actor_id },
)
const pending = await waitFor(demo, (state) =>
  state.snapshot.action_approvals.find(
    (approval) => approval.run_id === launch.run_id && approval.status === 'pending',
  ),
)
assert.equal(
  pending.state.snapshot.runs.find((run) => run.id === launch.run_id)?.status,
  'waiting_for_approval',
)

const restarted = await restartServer()
const restartRecovery = restarted
  ? await waitFor(demo, (state) => {
      const events = state.snapshot.events.filter(
        (event) => event.aggregate_id === launch.run_id,
      )
      const recovered =
        events.some((event) => event.type === 'runner.grace_started') &&
        events.some((event) => event.type === 'run.reconciled')
      const run = state.snapshot.runs.find(
        (candidate) => candidate.id === launch.run_id,
      )
      return recovered && run?.status === 'waiting_for_approval'
        ? run
        : null
    })
  : null
const decisionKey = crypto.randomUUID()
const firstDecision = await post(
  `/api/corps/${demo.corp_id}/approvals/${pending.value.id}/decision`,
  {
    actor_id: demo.bob_actor_id,
    approved: true,
    note: 'Second-device reviewer approved the bounded release action.',
    decision_key: decisionKey,
  },
)
assert.equal(firstDecision.effect_queued, true)
const duplicateDecision = await post(
  `/api/corps/${demo.corp_id}/approvals/${pending.value.id}/decision`,
  {
    actor_id: demo.bob_actor_id,
    approved: true,
    note: 'Second-device reviewer approved the bounded release action.',
    decision_key: decisionKey,
  },
)
assert.equal(duplicateDecision.effect_queued, false)

const completed = await waitFor(demo, (state) => {
  const run = state.snapshot.runs.find((candidate) => candidate.id === launch.run_id)
  const acknowledged = state.snapshot.events.some(
    (event) =>
      event.type === 'runner.command_acknowledged' &&
      event.aggregate_id === launch.run_id,
  )
  return run?.status === 'completed' && acknowledged ? run : null
})
assert.equal(completed.value.status, 'completed')

const rejectedMission = await post(`/api/corps/${demo.corp_id}/missions`, {
  requested_by: demo.alice_actor_id,
  preferred_adapter: 'fake-process',
  title: '[approval-action] reject a risky external side effect',
})
const rejectedLaunch = await post(
  `/api/corps/${demo.corp_id}/missions/${rejectedMission.mission_id}/launch`,
  { requested_by: demo.alice_actor_id },
)
const rejectionPending = await waitFor(demo, (state) =>
  state.snapshot.action_approvals.find(
    (approval) =>
      approval.run_id === rejectedLaunch.run_id && approval.status === 'pending',
  ),
)
await post(
  `/api/corps/${demo.corp_id}/approvals/${rejectionPending.value.id}/decision`,
  {
    actor_id: demo.alice_actor_id,
    approved: false,
    note: 'The requested side effect is not authorized.',
    decision_key: crypto.randomUUID(),
  },
)
const rejected = await waitFor(demo, (state) => {
  const run = state.snapshot.runs.find(
    (candidate) => candidate.id === rejectedLaunch.run_id,
  )
  return run?.status === 'cancelled' ? run : null
})
assert.equal(rejected.value.status, 'cancelled')

const expiryMission = await post(`/api/corps/${demo.corp_id}/missions`, {
  requested_by: demo.alice_actor_id,
  preferred_adapter: 'fake-process',
  title: '[approval-expiry] expire a risky external side effect',
})
const expiryLaunch = await post(
  `/api/corps/${demo.corp_id}/missions/${expiryMission.mission_id}/launch`,
  { requested_by: demo.alice_actor_id },
)
await waitFor(demo, (state) =>
  state.snapshot.action_approvals.find(
    (approval) =>
      approval.run_id === expiryLaunch.run_id && approval.status === 'pending',
  ),
)
const expired = await waitFor(demo, (state) => {
  const approval = state.snapshot.action_approvals.find(
    (candidate) => candidate.run_id === expiryLaunch.run_id,
  )
  const run = state.snapshot.runs.find(
    (candidate) => candidate.id === expiryLaunch.run_id,
  )
  const task = state.snapshot.tasks.find(
    (candidate) => candidate.id === run?.task_id,
  )
  const agent = state.snapshot.agents.find(
    (candidate) => candidate.id === run?.agent_id,
  )
  const event = state.snapshot.events.find(
    (candidate) =>
      candidate.type === 'run.approval_expired' &&
      candidate.aggregate_id === expiryLaunch.run_id,
  )
  const acknowledged = state.snapshot.events.find(
    (candidate) =>
      candidate.type === 'runner.command_acknowledged' &&
      candidate.aggregate_id === expiryLaunch.run_id,
  )
  return approval?.status === 'expired' &&
    run?.status === 'cancelled' &&
    task?.status === 'cancelled' &&
    agent?.status === 'idle' &&
    event &&
    acknowledged
    ? { approval, run, task, agent }
    : null
})

const report = {
  checked_at: new Date().toISOString(),
  server_restarted_while_suspended: restarted,
  startup_runner_recovery_verified: Boolean(restartRecovery),
  second_actor_approved: true,
  first_effect_queued: firstDecision.effect_queued,
  duplicate_effect_queued: duplicateDecision.effect_queued,
  approved_command_acknowledged: true,
  approved_run_status: completed.value.status,
  rejected_run_status: rejected.value.status,
  expired_approval_status: expired.value.approval.status,
  expired_run_status: expired.value.run.status,
  expired_task_status: expired.value.task.status,
  expired_agent_status: expired.value.agent.status,
  expired_command_acknowledged: true,
}
await writeFile(
  path.join(root, 'output', 'e2e-approvals.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
