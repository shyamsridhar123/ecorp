import assert from 'node:assert/strict'
import { randomUUID } from 'node:crypto'
import { writeFile } from 'node:fs/promises'
import path from 'node:path'

const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const root = path.resolve(import.meta.dirname, '..')

async function request(url, init) {
  const response = await fetch(`${server}${url}`, init)
  const body = response.status === 204 ? null : await response.json()
  return { response, body }
}

async function requestOk(url, init) {
  const result = await request(url, init)
  if (!result.response.ok) {
    throw new Error(
      `${init?.method ?? 'GET'} ${url} failed: ${JSON.stringify(result.body)}`,
    )
  }
  return result.body
}

function post(url, body) {
  return requestOk(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
}

function postRaw(url, body) {
  return request(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
}

async function snapshot(demo) {
  return requestOk(
    `/api/corps/${demo.corp_id}/snapshot?actor_id=${demo.alice_actor_id}`,
  )
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
  throw new Error(`timed out waiting for budget revision run ${runId}`)
}

async function setPolicy(demo) {
  await post(`/api/corps/${demo.corp_id}/budget-policy`, {
    actor_id: demo.alice_actor_id,
    actor_tokens_per_24h: 10_000_000,
    actor_cost_microusd_per_24h: 1_000_000_000,
    corp_tokens_per_24h: 100_000_000,
    corp_cost_microusd_per_24h: 10_000_000_000,
    no_progress_event_limit: 100,
    repeated_tool_limit: 100,
  })
}

async function createSuspendedMission(demo, suffix) {
  const mission = await post(`/api/corps/${demo.corp_id}/missions`, {
    requested_by: demo.alice_actor_id,
    preferred_adapter: 'codex',
    title: `[budget-stream] suspend recoverable mission ${suffix}`,
    budget_tokens: 6_000,
    budget_cost_microusd: 10_000_000,
  })
  const launch = await post(
    `/api/corps/${demo.corp_id}/missions/${mission.mission_id}/launch`,
    { requested_by: demo.alice_actor_id },
  )
  const terminal = await waitForRun(demo, launch.run_id)
  assert.equal(terminal.run.breaker_stage, 'suspend')
  assert.ok(['failed', 'cancelled'].includes(terminal.run.status))
  assert.equal(terminal.run.workspace_disposition, 'preserved')
  assert.ok(terminal.run.provider_session_id)
  assert.equal(terminal.run.input_tokens + terminal.run.output_tokens, 6_000)
  return { mission, launch, terminal }
}

function finishScope(task, budgetTokens = 4_000) {
  return {
    task_id: task.id,
    objective: 'Finish only the bounded verification and evidence work.',
    expected_output: 'A verified completion using the preserved provider session.',
    acceptance_tests: [
      'resume reuses the preserved provider session and worktree',
      'the revised mission budget remains enforced',
    ],
    write_scope: ['**'],
    budget_tokens: budgetTokens,
    budget_cost_microusd: 1_000_000,
    verification_policy: task.verification_policy,
  }
}

async function propose(
  demo,
  missionId,
  actorId,
  proposedTokens,
  idempotencyKey,
  scope,
) {
  return post(
    `/api/corps/${demo.corp_id}/missions/${missionId}/budget-revisions`,
    {
      actor_id: actorId,
      expected_budget_tokens: 6_000,
      expected_budget_cost_microusd: 10_000_000,
      proposed_budget_tokens: proposedTokens,
      proposed_budget_cost_microusd: 10_000_000,
      rationale: 'Authorize a bounded finish after reviewing already consumed usage.',
      idempotency_key: idempotencyKey,
      finish_scope: scope,
    },
  )
}

async function decide(
  demo,
  missionId,
  revision,
  actorId,
  approved,
  decisionKey,
) {
  return post(
    `/api/corps/${demo.corp_id}/missions/${missionId}/budget-revisions/${revision.id}/decision`,
    {
      actor_id: actorId,
      expected_version: revision.version,
      approved,
      note: approved
        ? 'Approved after reviewing spent usage and bounded finish scope.'
        : 'Rejected until the remaining work is explicitly bounded.',
      decision_key: decisionKey,
    },
  )
}

async function successfulRecoveryScenario() {
  const demo = await post('/api/demo/reset', {})
  await setPolicy(demo)
  const suspended = await createSuspendedMission(demo, 'successful-recovery')
  const task = suspended.terminal.state.snapshot.tasks.find(
    (candidate) => candidate.id === suspended.mission.task_id,
  )

  const deniedResume = await postRaw(
    `/api/corps/${demo.corp_id}/runs/${suspended.launch.run_id}/resume`,
    {
      requested_by: demo.alice_actor_id,
      prompt: 'resume without an approved budget revision',
    },
  )
  assert.equal(deniedResume.response.status, 409)
  assert.match(JSON.stringify(deniedResume.body), /budget revision before resume/)
  assert.equal(
    (await snapshot(demo)).snapshot.runs.filter(
      (run) => run.task_id === suspended.mission.task_id,
    ).length,
    1,
  )

  const unauthorizedProposal = await postRaw(
    `/api/corps/${demo.corp_id}/missions/${suspended.mission.mission_id}/budget-revisions`,
    {
      actor_id: demo.bob_actor_id,
      expected_budget_tokens: 6_000,
      expected_budget_cost_microusd: 10_000_000,
      proposed_budget_tokens: 12_000,
      proposed_budget_cost_microusd: 10_000_000,
      rationale: 'Member should not revise mission budget.',
      idempotency_key: randomUUID(),
      finish_scope: null,
    },
  )
  assert.equal(unauthorizedProposal.response.status, 403)

  const rejectedProposalKey = randomUUID()
  const rejectedProposal = await propose(
    demo,
    suspended.mission.mission_id,
    demo.alice_actor_id,
    12_000,
    rejectedProposalKey,
    null,
  )
  const rejectedReplay = await propose(
    demo,
    suspended.mission.mission_id,
    demo.alice_actor_id,
    12_000,
    rejectedProposalKey,
    null,
  )
  assert.equal(rejectedReplay.replayed, true)
  assert.equal(rejectedReplay.revision.id, rejectedProposal.revision.id)

  const rejectedDecisionKey = randomUUID()
  const rejected = await decide(
    demo,
    suspended.mission.mission_id,
    rejectedProposal.revision,
    demo.alice_actor_id,
    false,
    rejectedDecisionKey,
  )
  const rejectedDecisionReplay = await decide(
    demo,
    suspended.mission.mission_id,
    rejectedProposal.revision,
    demo.alice_actor_id,
    false,
    rejectedDecisionKey,
  )
  assert.equal(rejected.revision.status, 'rejected')
  assert.equal(rejectedDecisionReplay.replayed, true)

  const stillDenied = await postRaw(
    `/api/corps/${demo.corp_id}/runs/${suspended.launch.run_id}/resume`,
    {
      requested_by: demo.alice_actor_id,
      prompt: 'resume after a rejected budget revision',
    },
  )
  assert.equal(stillDenied.response.status, 409)

  const approvedProposalKey = randomUUID()
  const approvedProposal = await propose(
    demo,
    suspended.mission.mission_id,
    demo.alice_actor_id,
    20_000,
    approvedProposalKey,
    finishScope(task),
  )
  const pendingConflict = await postRaw(
    `/api/corps/${demo.corp_id}/missions/${suspended.mission.mission_id}/budget-revisions`,
    {
      actor_id: demo.alice_actor_id,
      expected_budget_tokens: 6_000,
      expected_budget_cost_microusd: 10_000_000,
      proposed_budget_tokens: 18_000,
      proposed_budget_cost_microusd: 10_000_000,
      rationale: 'A second pending revision must not race the first.',
      idempotency_key: randomUUID(),
      finish_scope: null,
    },
  )
  assert.equal(pendingConflict.response.status, 400)
  assert.match(JSON.stringify(pendingConflict.body), /pending budget revision/)

  const unauthorizedDecision = await postRaw(
    `/api/corps/${demo.corp_id}/missions/${suspended.mission.mission_id}/budget-revisions/${approvedProposal.revision.id}/decision`,
    {
      actor_id: demo.bob_actor_id,
      expected_version: approvedProposal.revision.version,
      approved: true,
      note: 'Member cannot approve this.',
      decision_key: randomUUID(),
    },
  )
  assert.equal(unauthorizedDecision.response.status, 403)

  const approvalKey = randomUUID()
  const approved = await decide(
    demo,
    suspended.mission.mission_id,
    approvedProposal.revision,
    demo.alice_actor_id,
    true,
    approvalKey,
  )
  const approvedReplay = await decide(
    demo,
    suspended.mission.mission_id,
    approvedProposal.revision,
    demo.alice_actor_id,
    true,
    approvalKey,
  )
  assert.equal(approved.revision.status, 'approved')
  assert.equal(approvedReplay.replayed, true)

  const resume = await post(
    `/api/corps/${demo.corp_id}/runs/${suspended.launch.run_id}/resume`,
    {
      requested_by: demo.alice_actor_id,
      prompt: 'resume the bounded finish and complete verification',
    },
  )
  const resumed = await waitForRun(demo, resume.run_id)
  assert.equal(resumed.run.status, 'completed')
  assert.equal(
    resumed.run.provider_session_id,
    suspended.terminal.run.provider_session_id,
  )
  assert.equal(resumed.run.resumed_from_run_id, suspended.launch.run_id)
  assert.equal(resumed.run.budget_tokens_limit, 4_000)

  const finalMission = resumed.state.snapshot.missions.find(
    (candidate) => candidate.id === suspended.mission.mission_id,
  )
  const finalTask = resumed.state.snapshot.tasks.find(
    (candidate) => candidate.id === suspended.mission.task_id,
  )
  const revisions = resumed.state.snapshot.mission_budget_revisions.filter(
    (revision) => revision.mission_id === suspended.mission.mission_id,
  )
  assert.equal(finalMission.original_budget_tokens, 6_000)
  assert.equal(finalMission.budget_tokens, 20_000)
  assert.equal(finalTask.contract.objective, finishScope(task).objective)
  assert.equal(revisions.length, 2)
  assert.deepEqual(
    new Set(revisions.map((revision) => revision.status)),
    new Set(['approved', 'rejected']),
  )
  assert.equal(
    revisions.find((revision) => revision.status === 'approved').decided_by,
    demo.alice_actor_id,
  )

  return {
    mission_id: suspended.mission.mission_id,
    source_run_id: suspended.launch.run_id,
    resume_run_id: resume.run_id,
    original_budget_tokens: finalMission.original_budget_tokens,
    revised_budget_tokens: finalMission.budget_tokens,
    consumed_tokens: resumed.state.snapshot.runs
      .filter((run) => run.task_id === suspended.mission.task_id)
      .reduce((total, run) => total + run.input_tokens + run.output_tokens, 0),
    resume_budget_tokens: resumed.run.budget_tokens_limit,
    rejected_revision_id: rejected.revision.id,
    approved_revision_id: approved.revision.id,
    approving_actor_id: approved.revision.decided_by,
    final_status: finalMission.status,
  }
}

async function revisedBudgetOverrunScenario() {
  const demo = await post('/api/demo/reset', {})
  await setPolicy(demo)
  const suspended = await createSuspendedMission(demo, 'revised-overrun')
  const task = suspended.terminal.state.snapshot.tasks.find(
    (candidate) => candidate.id === suspended.mission.task_id,
  )
  const proposal = await propose(
    demo,
    suspended.mission.mission_id,
    demo.alice_actor_id,
    10_000,
    randomUUID(),
    finishScope(task),
  )
  await decide(
    demo,
    suspended.mission.mission_id,
    proposal.revision,
    demo.alice_actor_id,
    true,
    randomUUID(),
  )
  const resume = await post(
    `/api/corps/${demo.corp_id}/runs/${suspended.launch.run_id}/resume`,
    {
      requested_by: demo.alice_actor_id,
      prompt: '[budget-stream] exceed the revised bounded finish budget',
    },
  )
  const terminal = await waitForRun(demo, resume.run_id)
  assert.equal(terminal.run.status, 'failed')
  assert.equal(terminal.run.breaker_stage, 'stop')
  assert.equal(terminal.run.artifact_id, null)
  const events = terminal.state.snapshot.events.filter(
    (event) => event.aggregate_id === resume.run_id,
  )
  assert.equal(
    events.filter((event) => event.type === 'run.completed').length,
    0,
  )

  return {
    mission_id: suspended.mission.mission_id,
    source_run_id: suspended.launch.run_id,
    resume_run_id: resume.run_id,
    revised_budget_tokens: 10_000,
    resume_budget_tokens: terminal.run.budget_tokens_limit,
    resume_usage_tokens:
      terminal.run.input_tokens + terminal.run.output_tokens,
    breaker_stage: terminal.run.breaker_stage,
    accepted_artifact: terminal.run.artifact_id,
    completed_events: 0,
  }
}

const report = {
  checked_at: new Date().toISOString(),
  successful_recovery: await successfulRecoveryScenario(),
  revised_budget_overrun: await revisedBudgetOverrunScenario(),
}

await writeFile(
  path.join(root, 'output', 'e2e-budget-revision.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
