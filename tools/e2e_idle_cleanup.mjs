import assert from 'node:assert/strict'
import { writeFile } from 'node:fs/promises'
import path from 'node:path'

const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const root = path.resolve(import.meta.dirname, '..')
const activeStatuses = new Set([
  'provisioning',
  'starting',
  'running',
  'waiting_for_input',
  'waiting_for_approval',
  'verifying',
])

async function request(url, init) {
  const response = await fetch(`${server}${url}`, init)
  const body = await response.json()
  if (!response.ok) {
    throw new Error(`${init?.method ?? 'GET'} ${url}: ${JSON.stringify(body)}`)
  }
  return body
}

function post(url, body) {
  return request(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
}

const demo = await post('/api/demo/bootstrap', {})
const snapshot = () =>
  request(`/api/corps/${demo.corp_id}/snapshot?actor_id=${demo.alice_actor_id}`)

const mission = await post(`/api/corps/${demo.corp_id}/missions`, {
  requested_by: demo.alice_actor_id,
  preferred_adapter: 'fake-process',
  strategy: 'single',
  title: '[slow] Verify terminal provider cleanup and off-shift agent state.',
})
const launch = await post(
  `/api/corps/${demo.corp_id}/missions/${mission.mission_id}/launch`,
  { requested_by: demo.alice_actor_id },
)

let sawLiveAgent = false
let finalState
const deadline = Date.now() + 90_000
while (Date.now() < deadline) {
  const state = await snapshot()
  const run = state.snapshot.runs.find((candidate) => candidate.id === launch.run_id)
  const agent = run
    ? state.snapshot.agents.find((candidate) => candidate.id === run.agent_id)
    : undefined
  if (run && agent?.current_run_id === run.id && activeStatuses.has(run.status)) {
    sawLiveAgent = true
  }
  if (
    run?.status === 'completed' &&
    ['preserved', 'removed'].includes(run.workspace_disposition)
  ) {
    finalState = state
    break
  }
  await new Promise((resolve) => setTimeout(resolve, 250))
}

assert.ok(finalState, 'cleanup scenario did not complete')
assert.equal(sawLiveAgent, true, 'agent never exposed a live run instance')
const run = finalState.snapshot.runs.find(
  (candidate) => candidate.id === launch.run_id,
)
const agent = finalState.snapshot.agents.find(
  (candidate) => candidate.id === run.agent_id,
)
assert.equal(agent.status, 'idle')
assert.equal(agent.current_run_id, null)

const runEvents = finalState.snapshot.events.filter(
  (event) => event.aggregate_id === run.id,
)
const terminated = runEvents.find(
  (event) => event.type === 'run.session_terminated',
)
const verification = runEvents.find(
  (event) => event.type === 'run.verification_started',
)
const completed = runEvents.find((event) => event.type === 'run.completed')
assert.ok(terminated, 'runner omitted run.session_terminated')
assert.equal(terminated.payload.provider_process_alive, false)
assert.ok(verification, 'runner omitted verification start')
assert.ok(completed, 'runner omitted completion')
assert.ok(
  terminated.seq < verification.seq && verification.seq < completed.seq,
  'provider must terminate before verification and accepted completion',
)

const report = {
  checked_at: new Date().toISOString(),
  mission_id: mission.mission_id,
  run_id: run.id,
  agent_id: agent.id,
  saw_live_agent: sawLiveAgent,
  terminal_event_seq: terminated.seq,
  verification_event_seq: verification.seq,
  completion_event_seq: completed.seq,
  provider_process_alive: terminated.payload.provider_process_alive,
  final_agent_status: agent.status,
  final_agent_current_run_id: agent.current_run_id,
  workspace_disposition: run.workspace_disposition,
}
await writeFile(
  path.join(root, 'output', 'e2e-idle-cleanup.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
