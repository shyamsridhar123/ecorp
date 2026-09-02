import assert from 'node:assert/strict'
import { execFile as execFileCallback } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { writeFile } from 'node:fs/promises'
import path from 'node:path'
import { promisify } from 'node:util'

const execFile = promisify(execFileCallback)
const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const databaseUrl =
  process.env.DATABASE_URL ?? 'postgres://crony:crony@127.0.0.1:54329/crony'
const root = path.resolve(import.meta.dirname, '..')
const proposalRationale =
  'Authorize a bounded finish after reviewing already consumed usage.'
let psqlMode

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

async function psql(sql) {
  const invocation = await psqlInvocation()
  const { stdout } = await execFile(
    invocation.command,
    [
      ...invocation.args,
      '-v',
      'ON_ERROR_STOP=1',
      '-At',
      '-c',
      sql,
    ],
    {
      cwd: root,
      windowsHide: true,
      maxBuffer: 8 * 1024 * 1024,
    },
  )
  return stdout.trim()
}

async function psqlInvocation() {
  if (!psqlMode) {
    try {
      await execFile('psql', ['--version'], { cwd: root, windowsHide: true })
      psqlMode = 'direct'
    } catch {
      psqlMode = 'docker'
    }
  }
  if (psqlMode === 'direct') {
    return { command: 'psql', args: [databaseUrl] }
  }
  const container = process.env.ECORP_TEST_POSTGRES_CONTAINER
  if (!container) {
    throw new Error(
      'psql is unavailable and ECORP_TEST_POSTGRES_CONTAINER was not provided',
    )
  }
  const parsedDatabaseUrl = new URL(databaseUrl)
  const databaseName = decodeURIComponent(
    parsedDatabaseUrl.pathname.replace(/^\/+/, ''),
  )
  const databaseUser = decodeURIComponent(parsedDatabaseUrl.username || 'crony')
  if (!databaseName) {
    throw new Error('DATABASE_URL omitted its PostgreSQL database name')
  }
  return {
    command: 'docker',
    args: [
      'exec',
      '-i',
      container,
      'psql',
      '-U',
      databaseUser,
      '-d',
      databaseName,
    ],
  }
}

function sqlLiteral(value) {
  return `'${String(value).replaceAll("'", "''")}'`
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

async function setPolicy(demo, overrides = {}) {
  await post(`/api/corps/${demo.corp_id}/budget-policy`, {
    actor_id: demo.alice_actor_id,
    actor_tokens_per_24h:
      overrides.actor_tokens_per_24h ?? 10_000_000,
    actor_cost_microusd_per_24h:
      overrides.actor_cost_microusd_per_24h ?? 1_000_000_000,
    corp_tokens_per_24h:
      overrides.corp_tokens_per_24h ?? 100_000_000,
    corp_cost_microusd_per_24h:
      overrides.corp_cost_microusd_per_24h ?? 10_000_000_000,
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
      rationale: proposalRationale,
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

async function rollingBudgetResumeScenario() {
  const results = {}
  for (const scenario of [
    {
      name: 'actor',
      policy: {
        actor_tokens_per_24h: 6_000,
        corp_tokens_per_24h: 100_000_000,
      },
    },
    {
      name: 'corp',
      policy: {
        actor_tokens_per_24h: 10_000_000,
        corp_tokens_per_24h: 6_000,
      },
    },
  ]) {
    const demo = await post('/api/demo/reset', {})
    await setPolicy(demo, scenario.policy)
    const suspended = await createSuspendedMission(
      demo,
      `${scenario.name}-rolling-budget`,
    )
    const proposal = await propose(
      demo,
      suspended.mission.mission_id,
      demo.alice_actor_id,
      20_000,
      randomUUID(),
      null,
    )
    await decide(
      demo,
      suspended.mission.mission_id,
      proposal.revision,
      demo.alice_actor_id,
      true,
      randomUUID(),
    )
    const before = await snapshot(demo)
    const denied = await postRaw(
      `/api/corps/${demo.corp_id}/runs/${suspended.launch.run_id}/resume`,
      {
        requested_by: demo.alice_actor_id,
        prompt: `resume through exhausted ${scenario.name} rolling budget`,
      },
    )
    assert.equal(denied.response.status, 409)
    assert.match(JSON.stringify(denied.body), /rolling budget/)
    const after = await snapshot(demo)
    assert.equal(after.snapshot.runs.length, before.snapshot.runs.length)
    results[`${scenario.name}_resume_rejected_before_run`] = true
  }
  return results
}

async function preDispatchRetryScenario() {
  const demo = await post('/api/demo/reset', {})
  await setPolicy(demo)
  const suspended = await createSuspendedMission(demo, 'pre-dispatch-retry')
  const task = suspended.terminal.state.snapshot.tasks.find(
    (candidate) => candidate.id === suspended.mission.task_id,
  )
  const proposal = await propose(
    demo,
    suspended.mission.mission_id,
    demo.alice_actor_id,
    20_000,
    randomUUID(),
    null,
  )
  await decide(
    demo,
    suspended.mission.mission_id,
    proposal.revision,
    demo.alice_actor_id,
    true,
    randomUUID(),
  )

  const mismatchedContract = {
    ...task.contract,
    source_repository: 'https://github.com/example/unavailable.git',
    source_base_ref: 'main',
    source_base_commit: 'f'.repeat(40),
  }
  await psql(`
    UPDATE tasks
    SET contract = ${sqlLiteral(JSON.stringify(mismatchedContract))}::jsonb
    WHERE id = ${sqlLiteral(task.id)}::uuid;
  `)
  const denied = await postRaw(
    `/api/corps/${demo.corp_id}/runs/${suspended.launch.run_id}/resume`,
    {
      requested_by: demo.alice_actor_id,
      prompt: 'transient resume with a temporarily unavailable checkout',
    },
  )
  assert.equal(denied.response.status, 409)
  assert.match(JSON.stringify(denied.body), /required repository checkout/)
  const failedState = await snapshot(demo)
  const preDispatchFailure = failedState.snapshot.runs.find(
    (run) => run.resumed_from_run_id === suspended.launch.run_id,
  )
  assert.ok(preDispatchFailure)
  assert.equal(preDispatchFailure.status, 'failed')
  assert.equal(preDispatchFailure.workspace_disposition, null)
  assert.equal(preDispatchFailure.workspace_detail, 'dispatch_not_started')

  await psql(`
    UPDATE tasks
    SET contract = ${sqlLiteral(JSON.stringify(task.contract))}::jsonb
    WHERE id = ${sqlLiteral(task.id)}::uuid;
  `)
  const retry = await post(
    `/api/corps/${demo.corp_id}/runs/${suspended.launch.run_id}/resume`,
    {
      requested_by: demo.alice_actor_id,
      prompt: 'retry after the transient pre-dispatch checkout mismatch',
    },
  )
  const terminal = await waitForRun(demo, retry.run_id)
  assert.equal(terminal.run.status, 'completed')
  assert.equal(
    terminal.run.provider_session_id,
    suspended.terminal.run.provider_session_id,
  )
  assert.equal(terminal.run.workspace_run_id, suspended.launch.run_id)
  assert.equal(terminal.run.resumed_from_run_id, suspended.launch.run_id)

  return {
    pre_dispatch_failure_id: preDispatchFailure.id,
    retry_run_id: retry.run_id,
    same_provider_session: true,
    same_workspace_lineage: true,
    final_status: terminal.run.status,
  }
}

async function authorizationRevalidationScenario() {
  const demo = await post('/api/demo/reset', {})
  await setPolicy(demo)
  const suspended = await createSuspendedMission(
    demo,
    'authorization-revalidation',
  )
  const task = suspended.terminal.state.snapshot.tasks.find(
    (candidate) => candidate.id === suspended.mission.task_id,
  )
  const mission = suspended.terminal.state.snapshot.missions.find(
    (candidate) => candidate.id === suspended.mission.mission_id,
  )
  const scope = finishScope(task)
  const proposalKey = randomUUID()
  const proposal = await propose(
    demo,
    suspended.mission.mission_id,
    demo.alice_actor_id,
    20_000,
    proposalKey,
    scope,
  )

  await psql(`
    UPDATE tasks
    SET contract = jsonb_set(
      contract,
      '{objective}',
      to_jsonb('intervening contract change'::text),
      false
    )
    WHERE id = ${sqlLiteral(task.id)}::uuid;
  `)
  const staleApproval = await postRaw(
    `/api/corps/${demo.corp_id}/missions/${suspended.mission.mission_id}/budget-revisions/${proposal.revision.id}/decision`,
    {
      actor_id: demo.alice_actor_id,
      expected_version: proposal.revision.version,
      approved: true,
      note: 'This stale approval must not overwrite the intervening contract.',
      decision_key: randomUUID(),
    },
  )
  assert.equal(staleApproval.response.status, 409)
  assert.match(JSON.stringify(staleApproval.body), /changed after proposal/)

  await psql(`
    DELETE FROM room_memberships
    WHERE room_id = ${sqlLiteral(mission.room_id)}::uuid
      AND actor_id = ${sqlLiteral(demo.alice_actor_id)}::uuid;
  `)
  const replayWithoutMembership = await postRaw(
    `/api/corps/${demo.corp_id}/missions/${suspended.mission.mission_id}/budget-revisions`,
    {
      actor_id: demo.alice_actor_id,
      expected_budget_tokens: 6_000,
      expected_budget_cost_microusd: 10_000_000,
      proposed_budget_tokens: 20_000,
      proposed_budget_cost_microusd: 10_000_000,
      rationale: proposalRationale,
      idempotency_key: proposalKey,
      finish_scope: scope,
    },
  )
  assert.equal(replayWithoutMembership.response.status, 403)
  assert.match(JSON.stringify(replayWithoutMembership.body), /not a member/)

  const decisionWithoutMembership = await postRaw(
    `/api/corps/${demo.corp_id}/missions/${suspended.mission.mission_id}/budget-revisions/${proposal.revision.id}/decision`,
    {
      actor_id: demo.alice_actor_id,
      expected_version: proposal.revision.version,
      approved: false,
      note: 'Removed room members cannot decide revisions.',
      decision_key: randomUUID(),
    },
  )
  assert.equal(decisionWithoutMembership.response.status, 403)
  assert.match(JSON.stringify(decisionWithoutMembership.body), /not a member/)

  return {
    stale_contract_approval_rejected: true,
    proposal_replay_after_room_removal_rejected: true,
    decision_after_room_removal_rejected: true,
  }
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
  const runCountBeforeAncestorRetry = terminal.state.snapshot.runs.length
  const ancestorRetry = await postRaw(
    `/api/corps/${demo.corp_id}/runs/${suspended.launch.run_id}/resume`,
    {
      requested_by: demo.alice_actor_id,
      prompt: 'retry the suspended ancestor after its descendant stopped',
    },
  )
  assert.equal(ancestorRetry.response.status, 409)
  assert.match(JSON.stringify(ancestorRetry.body), /lineage.*stop-stage/)
  assert.equal(
    (await snapshot(demo)).snapshot.runs.length,
    runCountBeforeAncestorRetry,
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
    stopped_lineage_ancestor_resume_rejected: true,
  }
}

const report = {
  checked_at: new Date().toISOString(),
  pre_dispatch_retry: await preDispatchRetryScenario(),
  rolling_budget_resume: await rollingBudgetResumeScenario(),
  authorization_revalidation: await authorizationRevalidationScenario(),
  successful_recovery: await successfulRecoveryScenario(),
  revised_budget_overrun: await revisedBudgetOverrunScenario(),
}

await writeFile(
  path.join(root, 'output', 'e2e-budget-revision.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
