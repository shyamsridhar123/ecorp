import assert from 'node:assert/strict'
import { writeFile } from 'node:fs/promises'
import path from 'node:path'

const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const root = path.resolve(import.meta.dirname, '..')

async function request(url, init) {
  const response = await fetch(`${server}${url}`, init)
  const body = response.status === 204 ? null : await response.json()
  if (!response.ok) {
    throw new Error(`${response.status}: ${JSON.stringify(body)}`)
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

async function snapshot(demo) {
  return request(
    `/api/corps/${demo.corp_id}/snapshot?actor_id=${demo.alice_actor_id}`,
  )
}

async function waitForRun(demo, runId, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const state = await snapshot(demo)
    const run = state.snapshot.runs.find((candidate) => candidate.id === runId)
    if (run && ['completed', 'cancelled', 'failed'].includes(run.status)) {
      return { state, run }
    }
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error(`timed out waiting for budget run ${runId}`)
}

async function setPolicy(demo, overrides = {}) {
  await post(`/api/corps/${demo.corp_id}/budget-policy`, {
    actor_id: demo.alice_actor_id,
    actor_tokens_per_24h: 10_000_000,
    actor_cost_microusd_per_24h: 1_000_000_000,
    corp_tokens_per_24h: 100_000_000,
    corp_cost_microusd_per_24h: 10_000_000_000,
    no_progress_event_limit: 100,
    repeated_tool_limit: 100,
    ...overrides,
  })
}

async function launch(demo, title, budgetTokens) {
  const mission = await post(`/api/corps/${demo.corp_id}/missions`, {
    requested_by: demo.alice_actor_id,
    preferred_adapter: 'fake-process',
    title,
    budget_tokens: budgetTokens,
    budget_cost_microusd: 10_000_000,
  })
  return post(`/api/corps/${demo.corp_id}/missions/${mission.mission_id}/launch`, {
    requested_by: demo.alice_actor_id,
  })
}

const demo = await post('/api/demo/reset', {})
await setPolicy(demo)
const spendLaunch = await launch(
  demo,
  '[budget-loop] deterministic spend breaker progression',
  100_000,
)
const spend = await waitForRun(demo, spendLaunch.run_id)
assert.equal(spend.run.status, 'cancelled')
const spendStages = spend.state.snapshot.circuit_breaker_incidents
  .filter((incident) => incident.run_id === spend.run.id)
  .map((incident) => incident.stage)
for (const required of ['steer', 'constrain', 'suspend']) {
  assert.ok(spendStages.includes(required))
}
assert.match(spend.run.summary, /Circuit breaker suspend checkpoint/)
assert.ok(
  spend.state.snapshot.events.some(
    (event) =>
      event.type === 'runner.command_acknowledged' &&
      event.aggregate_id === spend.run.id,
  ),
)

await post('/api/demo/reset', {})
await setPolicy(demo)
const stopLaunch = await launch(
  demo,
  '[budget-loop] immediate hard-stop budget enforcement',
  5_000,
)
const stopped = await waitForRun(demo, stopLaunch.run_id)
assert.equal(stopped.run.status, 'cancelled')
const stopStages = stopped.state.snapshot.circuit_breaker_incidents
  .filter((incident) => incident.run_id === stopped.run.id)
  .map((incident) => incident.stage)
assert.deepEqual(new Set(stopStages), new Set(['stop']))

await post('/api/demo/reset', {})
await setPolicy(demo, { repeated_tool_limit: 10 })
const loopLaunch = await launch(
  demo,
  '[budget-loop] deterministic repeated-tool breaker progression',
  200_000,
)
const looped = await waitForRun(demo, loopLaunch.run_id)
assert.equal(looped.run.status, 'cancelled')
const loopReasons = looped.state.snapshot.circuit_breaker_incidents
  .filter((incident) => incident.run_id === looped.run.id)
  .map((incident) => incident.reason)
assert.ok(loopReasons.some((reason) => reason.includes('repeated_tool')))

await post('/api/demo/reset', {})
await setPolicy(demo, { actor_tokens_per_24h: 100_000 })
const actorLaunch = await launch(
  demo,
  '[budget-loop] actor rolling budget enforcement',
  200_000,
)
const actorBounded = await waitForRun(demo, actorLaunch.run_id)
const actorReasons = actorBounded.state.snapshot.circuit_breaker_incidents
  .filter((incident) => incident.run_id === actorBounded.run.id)
  .map((incident) => incident.reason)
assert.ok(actorReasons.some((reason) => reason.includes('actor_tokens_24h')))

await post('/api/demo/reset', {})
await setPolicy(demo, { corp_tokens_per_24h: 100_000 })
const corpLaunch = await launch(
  demo,
  '[budget-loop] Corp rolling budget enforcement',
  200_000,
)
const corpBounded = await waitForRun(demo, corpLaunch.run_id)
const corpReasons = corpBounded.state.snapshot.circuit_breaker_incidents
  .filter((incident) => incident.run_id === corpBounded.run.id)
  .map((incident) => incident.reason)
assert.ok(corpReasons.some((reason) => reason.includes('corp_tokens_24h')))

await post('/api/demo/reset', {})
await setPolicy(demo, {
  no_progress_event_limit: 2,
  repeated_tool_limit: 2,
})
const conversationLaunch = await launch(
  demo,
  '[healthy-conversation] human discussion is not no-progress',
  100_000,
)
const conversation = await waitForRun(demo, conversationLaunch.run_id)
assert.equal(conversation.run.status, 'completed')
assert.equal(
  conversation.state.snapshot.circuit_breaker_incidents.filter(
    (incident) => incident.run_id === conversation.run.id,
  ).length,
  0,
)

const report = {
  checked_at: new Date().toISOString(),
  spend_stages: spendStages,
  hard_stop_stages: stopStages,
  repeated_tool_reasons: loopReasons,
  actor_budget_reasons: actorReasons,
  corp_budget_reasons: corpReasons,
  healthy_conversation_status: conversation.run.status,
  healthy_conversation_incidents: 0,
}
await writeFile(
  path.join(root, 'output', 'e2e-budgets.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
