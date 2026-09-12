import assert from 'node:assert/strict'
import { randomUUID } from 'node:crypto'
import { constants } from 'node:fs'
import { access, lstat, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { assertBudgetFixtureReady, budgetFixtureConfig, budgetFixturePreview } from './budget_fixture.mjs'

const config = budgetFixtureConfig(process.argv.slice(2), process.env)
if (config.dryRun) {
  console.log(JSON.stringify(budgetFixturePreview(config), null, 2))
  process.exit(0)
}
const server = config.server
await access(path.dirname(config.output), constants.W_OK)
try {
  await lstat(config.output)
  throw new Error('Budget evidence already exists; preserve it and choose a new owned evidence directory')
} catch (error) {
  if (error.code !== 'ENOENT') throw error
}

async function request(url, init) {
  const response = await fetch(`${server}${url}`, { ...init, redirect: 'error', signal: AbortSignal.timeout(10_000) })
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

async function waitForMission(demo, missionId, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const state = await snapshot(demo)
    const mission = state.snapshot.missions.find(
      (candidate) => candidate.id === missionId,
    )
    if (mission && ['completed', 'cancelled', 'failed'].includes(mission.status)) {
      return { state, mission }
    }
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error(`timed out waiting for budget mission ${missionId}`)
}

async function waitForApprovalAtHardBreaker(demo, runId, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const state = await snapshot(demo)
    const run = state.snapshot.runs.find((candidate) => candidate.id === runId)
    const approval = state.snapshot.action_approvals.find(
      (candidate) =>
        candidate.run_id === runId && candidate.status === 'pending',
    )
    if (run?.breaker_stage === 'stop' && approval) {
      return { run, approval }
    }
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error(`timed out waiting for hard-breaker approval ${runId}`)
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

const fixtureDemo = { corp_id: '00000000-0000-4000-8000-000000000001',
  alice_actor_id: '00000000-0000-4000-8000-000000000011' }
assertBudgetFixtureReady(await snapshot(fixtureDemo), fixtureDemo, config.runnerId)
const demo = await post('/api/demo/reset', {})
assert.equal(demo.corp_id, fixtureDemo.corp_id)
assert.equal(demo.alice_actor_id, fixtureDemo.alice_actor_id)
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
await setPolicy(demo)
const lateMission = await post(`/api/corps/${demo.corp_id}/missions`, {
  requested_by: demo.alice_actor_id,
  preferred_adapter: 'fake-process',
  title: '[budget-late-completion] ignore stop and attempt late completion',
  budget_tokens: 5_000,
  budget_cost_microusd: 10_000_000,
})
await post(
  `/api/corps/${demo.corp_id}/missions/${lateMission.mission_id}/launch`,
  { requested_by: demo.alice_actor_id },
)
const late = await waitForMission(demo, lateMission.mission_id)
// A delivered hard stop overrides the provider's late success. The source is
// retained, but verification or completion requires separate recovery authority.
assert.equal(late.mission.status, 'cancelled')
const lateTasks = late.state.snapshot.tasks.filter(
  (task) => task.mission_id === lateMission.mission_id,
)
assert.equal(lateTasks.length, 1)
assert.equal(lateTasks[0].status, 'cancelled')
assert.equal(lateTasks[0].attempt_count, 1)
const lateTaskIds = new Set(lateTasks.map((task) => task.id))
const lateRuns = late.state.snapshot.runs.filter((run) =>
  lateTaskIds.has(run.task_id),
)
assert.equal(lateRuns.length, 1)
const [lateRun] = lateRuns
assert.equal(lateRun.status, 'cancelled')
assert.equal(lateRun.breaker_stage, 'stop')
assert.equal(lateRun.resumed_from_run_id, null)
assert.equal(lateRun.workspace_run_id, lateRun.id)
assert.equal(lateRun.input_tokens + lateRun.output_tokens, 6_000)
assert.equal(lateRun.budget_tokens_limit, 5_000)
assert.equal(lateRun.artifact_id, null)
assert.equal(lateRun.verification_status, 'pending')
assert.equal(
  lateRun.summary,
  'Hard circuit-breaker boundary reached; source retained for explicit recovery.',
)
const lateIncidents = late.state.snapshot.circuit_breaker_incidents.filter(
  (incident) => incident.run_id === lateRun.id,
)
assert.equal(lateIncidents.length, 1)
assert.equal(lateIncidents[0].stage, 'stop')
assert.deepEqual(lateIncidents[0].input, {
  metric: 'run_tokens',
  used: 6_000,
  limit: 5_000,
})
const lateEvents = late.state.snapshot.events.filter(
  (event) => event.aggregate_id === lateRun.id,
)
const lateRequested = lateEvents.filter((event) => event.type === 'run.requested')
assert.equal(lateRequested.length, 1)
assert.equal(lateRequested[0].payload.attempt, 1)
const lateTerminated = lateEvents.filter(
  (event) => event.type === 'run.session_terminated',
)
assert.equal(lateTerminated.length, 1)
assert.equal(lateTerminated[0].payload.outcome, 'completed')
assert.equal(lateTerminated[0].payload.provider_process_alive, false)
const latePreserved = lateEvents.filter(
  (event) => event.type === 'run.workspace_preserved',
)
assert.equal(latePreserved.length, 1)
assert.equal(latePreserved[0].payload.branch_deleted, false)
assert.equal(lateRun.workspace_disposition, 'preserved')
assert.ok(lateRun.workspace_path)
const retainedSource = await readFile(
  path.join(lateRun.workspace_path, 'result.md'),
  'utf8',
)
assert.ok(retainedSource.includes(`Run: \`${lateRun.id}\``))
const lateCancelled = lateEvents.filter((event) => event.type === 'run.cancelled')
assert.equal(lateCancelled.length, 1)
assert.equal(lateCancelled[0].payload.reason, lateRun.summary)
assert.ok(lateTerminated[0].seq < latePreserved[0].seq)
assert.ok(latePreserved[0].seq < lateCancelled[0].seq)
const lateAcceptedCompletionEvents = lateEvents.filter(
  (event) => event.type === 'run.completed',
)
assert.equal(lateAcceptedCompletionEvents.length, 0)
const lateArtifactEvents = lateEvents.filter((event) =>
  ['run.artifact', 'run.artifact_upload', 'run.deliverable', 'run.deliverable_upload']
    .includes(event.type),
)
assert.equal(lateArtifactEvents.length, 0)
assert.equal(
  lateEvents.filter(
    (event) => event.type.startsWith('run.verification_') ||
      ['run.failed', 'run.workspace_removed'].includes(event.type),
  ).length,
  0,
)

await post('/api/demo/reset', {})
await setPolicy(demo)
const approvalRaceMission = await post(`/api/corps/${demo.corp_id}/missions`, {
  requested_by: demo.alice_actor_id,
  preferred_adapter: 'fake-process',
  title: '[approval-budget-race] pending action cannot cross a hard breaker',
  budget_tokens: 5_000,
  budget_cost_microusd: 10_000_000,
})
const approvalRaceLaunch = await post(
  `/api/corps/${demo.corp_id}/missions/${approvalRaceMission.mission_id}/launch`,
  { requested_by: demo.alice_actor_id },
)
const approvalRace = await waitForApprovalAtHardBreaker(
  demo,
  approvalRaceLaunch.run_id,
)
const approvalResponse = await fetch(
  `${server}/api/corps/${demo.corp_id}/approvals/${approvalRace.approval.id}/decision`,
  {
    method: 'POST',
    redirect: 'error',
    signal: AbortSignal.timeout(10_000),
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      actor_id: demo.bob_actor_id,
      approved: true,
      note: 'This decision must be rejected after the hard breaker.',
      decision_key: randomUUID(),
    }),
  },
)
assert.equal(approvalResponse.status, 400)
assert.match(
  JSON.stringify(await approvalResponse.json()),
  /blocked.*(?:hard breaker|breaker stage)|(?:hard breaker|breaker stage).*blocked/i,
)
await post(
  `/api/corps/${demo.corp_id}/agents/${approvalRace.run.agent_id}/emergency-stop`,
  {
    actor_id: demo.alice_actor_id,
    reason: 'Clean up the approval-budget race fixture.',
  },
)
await waitForRun(demo, approvalRaceLaunch.run_id)

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
  late_completion_status: late.mission.status,
  late_completion_run_statuses: lateRuns.map((run) => run.status),
  late_completion_attempt_count: lateTasks[0].attempt_count,
  late_completion_breaker_input: lateIncidents[0].input,
  late_completion_provider_outcome: lateTerminated[0].payload.outcome,
  late_completion_workspace_disposition: lateRun.workspace_disposition,
  late_completion_source_retained: retainedSource.includes(`Run: \`${lateRun.id}\``),
  late_completion_artifact_events: lateArtifactEvents.length,
  late_completion_accepted_events: lateAcceptedCompletionEvents.length,
  approval_after_hard_breaker_status: approvalResponse.status,
  repeated_tool_reasons: loopReasons,
  actor_budget_reasons: actorReasons,
  corp_budget_reasons: corpReasons,
  healthy_conversation_status: conversation.run.status,
  healthy_conversation_incidents: 0,
}
await writeFile(
  config.output,
  `${JSON.stringify(report, null, 2)}\n`,
  { encoding: 'utf8', flag: 'wx' },
)
console.log(JSON.stringify(report, null, 2))
