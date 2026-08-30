import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import {
  appendFileSync,
  existsSync,
  openSync,
  readFileSync,
  writeFileSync,
} from 'node:fs'
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
  const ciPidPath = path.join(root, 'output', 'server-ci.pid')
  const localPidPath = path.join(root, 'output', 'local-pids.json')
  let pid
  let pidWriter
  if (existsSync(ciPidPath)) {
    pid = Number(readFileSync(ciPidPath, 'utf8').trim())
    pidWriter = (nextPid) => writeFileSync(ciPidPath, `${nextPid}\n`)
  } else if (existsSync(localPidPath)) {
    const pids = JSON.parse(readFileSync(localPidPath, 'utf8'))
    pid = Number(pids.server)
    pidWriter = (nextPid) => {
      pids.server = nextPid
      writeFileSync(localPidPath, `${JSON.stringify(pids, null, 2)}\n`)
    }
  } else {
    return false
  }
  process.kill(pid)
  await new Promise((resolve) => setTimeout(resolve, 500))
  const binary = path.join(
    root,
    'target',
    'debug',
    process.platform === 'win32' ? 'crony-server.exe' : 'crony-server',
  )
  const stdout = openSync(path.join(root, 'output', 'server.stdout.log'), 'a')
  const stderr = openSync(path.join(root, 'output', 'server.stderr.log'), 'a')
  appendFileSync(
    path.join(root, 'output', 'server.stdout.log'),
    '\n--- durable approval restart ---\n',
  )
  const child = spawn(
    binary,
    [
      '--bind',
      '127.0.0.1:8791',
      '--database-url',
      process.env.DATABASE_URL ??
        'postgres://crony:crony@127.0.0.1:54329/crony',
    ],
    {
      cwd: root,
      detached: true,
      windowsHide: true,
      stdio: ['ignore', stdout, stderr],
    },
  )
  pidWriter(child.pid)
  child.unref()

  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    try {
      const health = await fetch(`${server}/health`).then((response) =>
        response.json(),
      )
      if (health.status === 'ok' && health.runners >= 1) return true
    } catch {
      // Server or runner is still reconnecting.
    }
    await new Promise((resolve) => setTimeout(resolve, 200))
  }
  throw new Error('server or runner did not recover after approval restart')
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
  return run?.status === 'completed' ? run : null
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

const report = {
  checked_at: new Date().toISOString(),
  server_restarted_while_suspended: restarted,
  second_actor_approved: true,
  first_effect_queued: firstDecision.effect_queued,
  duplicate_effect_queued: duplicateDecision.effect_queued,
  approved_run_status: completed.value.status,
  rejected_run_status: rejected.value.status,
}
await writeFile(
  path.join(root, 'output', 'e2e-approvals.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
