import assert from 'node:assert/strict'
import { writeFile } from 'node:fs/promises'
import path from 'node:path'

const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const root = path.resolve(import.meta.dirname, '..')

async function fetchJson(url, init) {
  const response = await fetch(`${server}${url}`, init)
  const body = await response.json()
  return { response, body }
}

async function request(url, init) {
  const { response, body } = await fetchJson(url, init)
  if (!response.ok) {
    throw new Error(`${init?.method ?? 'GET'} ${url} failed: ${JSON.stringify(body)}`)
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

async function snapshot(demo, actorId = demo.alice_actor_id) {
  return request(`/api/corps/${demo.corp_id}/snapshot?actor_id=${actorId}`)
}

async function createAndLaunch(demo, strategy, title) {
  const created = await post(`/api/corps/${demo.corp_id}/missions`, {
    requested_by: demo.alice_actor_id,
    preferred_adapter: 'fake-process',
    strategy,
    title,
  })
  const launched = await post(
    `/api/corps/${demo.corp_id}/missions/${created.mission_id}/launch`,
    { requested_by: demo.alice_actor_id },
  )
  return { created, launched }
}

async function waitFor(demo, missionId, predicate, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const state = await snapshot(demo)
    const mission = state.snapshot.missions.find((item) => item.id === missionId)
    const task = state.snapshot.tasks.find((item) => item.mission_id === missionId)
    const runs = state.snapshot.runs.filter((run) => run.task_id === task?.id)
    if (mission && task && predicate({ state, mission, task, runs })) {
      return { state, mission, task, runs }
    }
    await new Promise((resolve) => setTimeout(resolve, 75))
  }
  throw new Error(`timed out waiting for verification mission ${missionId}`)
}

async function automatedMatrix() {
  const demo = await post('/api/demo/reset', {})
  const { created, launched } = await createAndLaunch(
    demo,
    'verification-matrix',
    'Exercise every automated verifier type.',
  )
  const result = await waitFor(
    demo,
    created.mission_id,
    ({ mission, runs }) =>
      mission.status === 'completed' &&
      runs.length === 1 &&
      runs[0].workspace_disposition === 'preserved',
  )
  const run = result.runs[0]
  const evidence = result.state.snapshot.verification_evidence.filter(
    (item) => item.run_id === run.id,
  )
  assert.equal(result.task.verification_status, 'passed')
  assert.equal(run.verification_status, 'passed')
  assert.equal(evidence.length, 6)
  assert.ok(evidence.every((item) => item.status === 'passed'))
  assert.deepEqual(
    evidence.map((item) => item.kind),
    ['artifact', 'file', 'command', 'test', 'json_schema', 'screenshot'],
  )
  assert.ok(evidence.every((item) => item.task_id === result.task.id))
  assert.equal(
    result.state.snapshot.events.filter(
      (event) => event.aggregate_id === run.id && event.type === 'run.completed',
    ).length,
    1,
  )
  return {
    mission_id: created.mission_id,
    task_id: result.task.id,
    run_id: launched.run_id,
    evidence_kinds: evidence.map((item) => item.kind),
    status: run.verification_status,
  }
}

async function failedVerification() {
  const demo = await post('/api/demo/reset', {})
  const { created, launched } = await createAndLaunch(
    demo,
    'verification-failure',
    'Prove missing evidence blocks completion.',
  )
  const result = await waitFor(
    demo,
    created.mission_id,
    ({ mission, runs }) =>
      mission.status === 'failed' &&
      runs.length === 1 &&
      runs[0].workspace_disposition === 'preserved',
  )
  const run = result.runs[0]
  const evidence = result.state.snapshot.verification_evidence.filter(
    (item) => item.run_id === run.id,
  )
  assert.equal(result.task.status, 'verification_failed')
  assert.equal(result.task.verification_status, 'failed')
  assert.equal(run.status, 'failed')
  assert.equal(run.verification_status, 'failed')
  assert.equal(evidence.length, 2)
  assert.equal(evidence.filter((item) => item.status === 'failed').length, 1)
  assert.equal(
    result.state.snapshot.events.filter(
      (event) => event.aggregate_id === run.id && event.type === 'run.completed',
    ).length,
    0,
  )
  return {
    mission_id: created.mission_id,
    task_id: result.task.id,
    run_id: launched.run_id,
    failed_checks: evidence
      .filter((item) => item.status === 'failed')
      .map((item) => item.kind),
    task_status: result.task.status,
  }
}

async function humanApproval() {
  const demo = await post('/api/demo/reset', {})
  const { created, launched } = await createAndLaunch(
    demo,
    'human-approval',
    'Require an authorized human verification decision.',
  )
  const waiting = await waitFor(
    demo,
    created.mission_id,
    ({ runs, state }) =>
      runs[0]?.status === 'waiting_for_approval' &&
      runs[0]?.workspace_disposition === 'preserved' &&
      state.snapshot.verification_requests.some(
        (request) => request.run_id === runs[0].id && request.status === 'pending',
      ),
  )
  const run = waiting.runs[0]
  assert.equal(waiting.task.status, 'awaiting_approval')
  const waitingAgent = waiting.state.snapshot.agents.find(
    (agent) => agent.id === run.agent_id,
  )
  assert.equal(waitingAgent.status, 'reviewing')
  assert.equal(waitingAgent.current_run_id, null)
  const decision = await post(
    `/api/corps/${demo.corp_id}/runs/${run.id}/verification-decision`,
    {
      actor_id: demo.alice_actor_id,
      approved: true,
      note: 'Owner approved the evidence.',
    },
  )
  assert.equal(decision.status, 'approved')
  const completed = await waitFor(
    demo,
    created.mission_id,
    ({ mission, state }) =>
      mission.status === 'completed' &&
      state.snapshot.verification_requests.some(
        (request) => request.run_id === run.id && request.status === 'approved',
      ),
  )
  assert.equal(completed.task.status, 'completed')
  const repeated = await fetchJson(
    `/api/corps/${demo.corp_id}/runs/${run.id}/verification-decision`,
    {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        actor_id: demo.alice_actor_id,
        approved: true,
        note: 'Duplicate decision must fail.',
      }),
    },
  )
  assert.equal(repeated.response.status, 400)
  return {
    mission_id: created.mission_id,
    run_id: launched.run_id,
    gate_type: 'human_approval',
    repeated_decision_status: repeated.response.status,
    final_status: completed.mission.status,
  }
}

async function independentReview() {
  const demo = await post('/api/demo/reset', {})
  const { created, launched } = await createAndLaunch(
    demo,
    'independent-review',
    'Require a reviewer other than the requester.',
  )
  const waiting = await waitFor(
    demo,
    created.mission_id,
    ({ runs, state }) =>
      runs[0]?.status === 'waiting_for_approval' &&
      state.snapshot.verification_requests.some(
        (request) => request.run_id === runs[0].id && request.status === 'pending',
      ),
  )
  const run = waiting.runs[0]
  const requesterAttempt = await fetchJson(
    `/api/corps/${demo.corp_id}/runs/${run.id}/verification-decision`,
    {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        actor_id: demo.alice_actor_id,
        approved: true,
        note: 'Requester must not self-review.',
      }),
    },
  )
  assert.equal(requesterAttempt.response.status, 403)

  const reviewerDecision = await post(
    `/api/corps/${demo.corp_id}/runs/${run.id}/verification-decision`,
    {
      actor_id: demo.bob_actor_id,
      approved: true,
      note: 'Independent reviewer approved the evidence.',
    },
  )
  assert.equal(reviewerDecision.status, 'approved')
  const completed = await waitFor(
    demo,
    created.mission_id,
    ({ mission, state }) =>
      mission.status === 'completed' &&
      state.snapshot.verification_requests.some(
        (request) =>
          request.run_id === run.id &&
          request.status === 'approved' &&
          request.decided_by === demo.bob_actor_id,
      ),
  )
  return {
    mission_id: created.mission_id,
    run_id: launched.run_id,
    requester_status: requesterAttempt.response.status,
    reviewer_actor_id: demo.bob_actor_id,
    final_status: completed.mission.status,
  }
}

const report = {
  checked_at: new Date().toISOString(),
  automated_matrix: await automatedMatrix(),
  failed_verification: await failedVerification(),
  human_approval: await humanApproval(),
  independent_review: await independentReview(),
}
await writeFile(
  path.join(root, 'output', 'e2e-verification.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
