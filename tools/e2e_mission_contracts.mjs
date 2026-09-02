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

async function snapshot(demo, actorId = demo.alice_actor_id) {
  return requestOk(
    `/api/corps/${demo.corp_id}/snapshot?actor_id=${actorId}`,
  )
}

async function waitForMission(demo, missionId, predicate, timeoutMs = 90_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const state = await snapshot(demo)
    const mission = state.snapshot.missions.find((item) => item.id === missionId)
    const tasks = state.snapshot.tasks.filter(
      (task) => task.mission_id === missionId,
    )
    const taskIds = new Set(tasks.map((task) => task.id))
    const runs = state.snapshot.runs.filter((run) => taskIds.has(run.task_id))
    if (mission && predicate({ state, mission, tasks, runs })) {
      return { state, mission, tasks, runs }
    }
    await new Promise((resolve) => setTimeout(resolve, 75))
  }
  throw new Error(`timed out waiting for mission ${missionId}`)
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
  const parsed = new URL(databaseUrl)
  const database = decodeURIComponent(parsed.pathname.replace(/^\/+/, ''))
  const user = decodeURIComponent(parsed.username || 'crony')
  return {
    command: 'docker',
    args: [
      'compose',
      '-f',
      path.join(root, 'deploy', 'compose', 'docker-compose.yml'),
      'exec',
      '-T',
      'postgres',
      'psql',
      '-U',
      user,
      '-d',
      database,
    ],
  }
}

async function psql(sql) {
  const invocation = await psqlInvocation()
  await execFile(
    invocation.command,
    [...invocation.args, '-v', 'ON_ERROR_STOP=1', '-c', sql],
    {
      cwd: root,
      windowsHide: true,
      maxBuffer: 4 * 1024 * 1024,
    },
  )
}

function richContract() {
  return {
    objective:
      'Build the requested enterprise workflow and retain every stated trust boundary.',
    expected_output:
      'A runnable implementation plus repository-native test and browser evidence.',
    acceptance_tests: [
      'all six typed verifier checks pass',
      'the requester cannot approve the independent review',
      'the approved context references remain attached',
    ],
    allowed_tools: ['filesystem', 'shell'],
    prohibited_actions: [
      'modify files outside the assigned worktree',
      'disable or bypass verification',
    ],
    references: [
      'docs/PRODUCT_AND_TECHNICAL_PLAN.md',
      'https://github.com/shyamsridhar123/ecorp/issues/52',
      'approved-context://enterprise-security-review',
    ],
    write_scope: ['**'],
  }
}

function matrixPolicy(manualGate = null) {
  return {
    checks: [
      { type: 'artifact', min_bytes: 1 },
      { type: 'file', path: 'verify.txt', min_bytes: 9 },
      {
        type: 'command',
        program: 'node',
        args: [
          '-e',
          "const fs=require('fs');process.exit(fs.existsSync('verify.txt')?0:1)",
        ],
        timeout_ms: 5_000,
      },
      {
        type: 'test',
        program: 'node',
        args: [
          '-e',
          "const fs=require('fs');process.exit(fs.readFileSync('verify.txt','utf8').trim()==='VERIFIED'?0:1)",
        ],
        timeout_ms: 5_000,
      },
      {
        type: 'json_schema',
        path: 'schema.json',
        required_keys: ['status', 'count'],
      },
      { type: 'screenshot', path: 'screenshot.png', min_bytes: 16 },
    ],
    manual_gate: manualGate,
  }
}

async function richCreationAndTypedVerification() {
  const demo = await post('/api/demo/reset', {})
  const description = [
    '# Vendor onboarding specification',
    '',
    'Implement the complete workflow, preserve tenant isolation, and prove it in the worktree.',
    'The issue URL and approved security review are authoritative references.',
  ].join('\n')
  const contract = richContract()
  const verificationPolicy = matrixPolicy({
    type: 'independent_review',
    roles: ['member', 'manager', 'admin', 'owner'],
    exclude_requester: true,
  })
  const created = await post(`/api/corps/${demo.corp_id}/missions`, {
    requested_by: demo.alice_actor_id,
    preferred_adapter: 'fake-process',
    strategy: 'single',
    title: '[verification-matrix] Verify a rich enterprise mission contract.',
    description,
    contract,
    verification_policy: verificationPolicy,
  })
  const planned = await snapshot(demo)
  const mission = planned.snapshot.missions.find(
    (candidate) => candidate.id === created.mission_id,
  )
  const task = planned.snapshot.tasks.find(
    (candidate) => candidate.id === created.task_id,
  )
  assert.ok(mission)
  assert.ok(task)
  assert.equal(mission.description, description)
  assert.equal(mission.specification_version, 1)
  assert.equal(task.contract_version, 1)
  assert.ok(task.contract.objective.includes(description))
  assert.ok(task.contract.objective.includes(contract.objective))
  assert.equal(task.contract.expected_output, contract.expected_output)
  assert.deepEqual(task.contract.allowed_tools, contract.allowed_tools)
  assert.deepEqual(task.contract.write_scope, contract.write_scope)
  for (const acceptance of contract.acceptance_tests) {
    assert.ok(task.contract.acceptance_tests.includes(acceptance))
  }
  for (const reference of contract.references) {
    assert.ok(task.contract.references.includes(reference))
  }
  for (const prohibition of contract.prohibited_actions) {
    assert.ok(task.contract.prohibited_actions.includes(prohibition))
  }
  assert.deepEqual(task.verification_policy, verificationPolicy)

  const launch = await post(
    `/api/corps/${demo.corp_id}/missions/${created.mission_id}/launch`,
    { requested_by: demo.alice_actor_id },
  )
  const waiting = await waitForMission(
    demo,
    created.mission_id,
    ({ state, runs }) =>
      runs[0]?.status === 'waiting_for_approval' &&
      state.snapshot.verification_evidence.filter(
        (evidence) => evidence.run_id === runs[0].id,
      ).length === 6,
  )
  const run = waiting.runs[0]
  const evidence = waiting.state.snapshot.verification_evidence
    .filter((item) => item.run_id === run.id)
    .toSorted((left, right) => left.check_index - right.check_index)
  assert.deepEqual(
    evidence.map((item) => item.kind),
    ['artifact', 'file', 'command', 'test', 'json_schema', 'screenshot'],
  )
  assert.ok(evidence.every((item) => item.status === 'passed'))

  const requesterDecision = await postRaw(
    `/api/corps/${demo.corp_id}/runs/${run.id}/verification-decision`,
    {
      actor_id: demo.alice_actor_id,
      approved: true,
      note: 'Requester must not self-approve.',
    },
  )
  assert.equal(requesterDecision.response.status, 403)
  await post(
    `/api/corps/${demo.corp_id}/runs/${run.id}/verification-decision`,
    {
      actor_id: demo.bob_actor_id,
      approved: true,
      note: 'Independent reviewer accepted all typed evidence.',
    },
  )
  const completed = await waitForMission(
    demo,
    created.mission_id,
    ({ mission: current }) => current.status === 'completed',
  )
  const decision = completed.state.snapshot.verification_requests.find(
    (request) => request.run_id === run.id,
  )
  assert.equal(decision?.decided_by, demo.bob_actor_id)
  return {
    mission_id: created.mission_id,
    task_id: created.task_id,
    run_id: launch.run_id,
    specification_version: mission.specification_version,
    contract_version: task.contract_version,
    references: contract.references,
    evidence_kinds: evidence.map((item) => item.kind),
    requester_decision_status: requesterDecision.response.status,
    reviewer_actor_id: decision?.decided_by,
    final_status: completed.mission.status,
  }
}

async function failingTestBlocksCompletion() {
  const demo = await post('/api/demo/reset', {})
  const created = await post(`/api/corps/${demo.corp_id}/missions`, {
    requested_by: demo.alice_actor_id,
    preferred_adapter: 'fake-process',
    strategy: 'single',
    title: 'A failing application test must block accepted completion.',
    description:
      'The provider will produce metadata, but the application test intentionally exits non-zero.',
    contract: {
      objective: 'Produce the provider artifact, then submit to the explicit failing test.',
      expected_output: 'A failed verification result with no accepted completion.',
      acceptance_tests: ['a non-zero application test blocks completion'],
      allowed_tools: ['filesystem', 'shell'],
      prohibited_actions: ['disable the failing test'],
      references: ['tools/e2e_mission_contracts.mjs'],
      write_scope: ['**'],
    },
    verification_policy: {
      checks: [
        { type: 'artifact', min_bytes: 1 },
        {
          type: 'test',
          program: 'node',
          args: ['-e', 'process.exit(17)'],
          timeout_ms: 5_000,
        },
      ],
      manual_gate: null,
    },
  })
  const launch = await post(
    `/api/corps/${demo.corp_id}/missions/${created.mission_id}/launch`,
    { requested_by: demo.alice_actor_id },
  )
  const failed = await waitForMission(
    demo,
    created.mission_id,
    ({ mission, runs }) =>
      mission.status === 'failed' &&
      runs[0]?.status === 'failed' &&
      runs[0]?.workspace_disposition === 'preserved',
  )
  const evidence = failed.state.snapshot.verification_evidence.filter(
    (item) => item.run_id === launch.run_id,
  )
  assert.equal(evidence.length, 2)
  assert.equal(
    evidence.find((item) => item.kind === 'test')?.status,
    'failed',
  )
  assert.equal(
    failed.state.snapshot.events.filter(
      (event) =>
        event.aggregate_id === launch.run_id && event.type === 'run.completed',
    ).length,
    0,
  )
  return {
    mission_id: created.mission_id,
    run_id: launch.run_id,
    failed_check: evidence.find((item) => item.status === 'failed')?.kind,
    completion_events: 0,
    final_status: failed.mission.status,
  }
}

async function authoredHumanApproval() {
  const demo = await post('/api/demo/reset', {})
  const created = await post(`/api/corps/${demo.corp_id}/missions`, {
    requested_by: demo.alice_actor_id,
    preferred_adapter: 'fake-process',
    strategy: 'single',
    title: 'Use an operator-authored human approval gate.',
    description:
      'Completion must remain suspended until an owner or admin accepts the evidence.',
    contract: {
      objective: 'Produce a bounded artifact for an owner decision.',
      expected_output: 'A verified artifact plus a durable owner approval.',
      acceptance_tests: ['a member cannot decide the owner gate'],
      allowed_tools: ['filesystem'],
      prohibited_actions: ['self-complete without the manual gate'],
      references: ['docs/adr/0024-versioned-mission-contracts.md'],
      write_scope: ['**'],
    },
    verification_policy: {
      checks: [{ type: 'artifact', min_bytes: 1 }],
      manual_gate: {
        type: 'human_approval',
        roles: ['owner', 'admin'],
      },
    },
  })
  const launch = await post(
    `/api/corps/${demo.corp_id}/missions/${created.mission_id}/launch`,
    { requested_by: demo.alice_actor_id },
  )
  const waiting = await waitForMission(
    demo,
    created.mission_id,
    ({ runs }) => runs[0]?.status === 'waiting_for_approval',
  )
  const run = waiting.runs[0]
  const memberAttempt = await postRaw(
    `/api/corps/${demo.corp_id}/runs/${run.id}/verification-decision`,
    {
      actor_id: demo.bob_actor_id,
      approved: true,
      note: 'A member must not decide an owner-only gate.',
    },
  )
  assert.equal(memberAttempt.response.status, 403)
  await post(
    `/api/corps/${demo.corp_id}/runs/${run.id}/verification-decision`,
    {
      actor_id: demo.alice_actor_id,
      approved: true,
      note: 'Owner accepted the authored evidence gate.',
    },
  )
  const completed = await waitForMission(
    demo,
    created.mission_id,
    ({ mission }) => mission.status === 'completed',
  )
  const decision = completed.state.snapshot.verification_requests.find(
    (request) => request.run_id === launch.run_id,
  )
  assert.equal(decision?.gate_type, 'human_approval')
  assert.equal(decision?.decided_by, demo.alice_actor_id)
  return {
    mission_id: created.mission_id,
    run_id: launch.run_id,
    member_decision_status: memberAttempt.response.status,
    owner_actor_id: decision?.decided_by,
    final_status: completed.mission.status,
  }
}

async function redispatchRevision() {
  const demo = await post('/api/demo/reset', {})
  const created = await post(`/api/corps/${demo.corp_id}/missions`, {
    requested_by: demo.alice_actor_id,
    preferred_adapter: 'fake-process',
    strategy: 'single',
    title: 'Revise this ready mission before its first dispatch.',
    description: 'Initial operator specification.',
    contract: richContract(),
    verification_policy: {
      checks: [{ type: 'artifact', min_bytes: 1 }],
      manual_gate: null,
    },
  })
  const initial = await snapshot(demo)
  const task = initial.snapshot.tasks.find(
    (candidate) => candidate.id === created.task_id,
  )
  assert.ok(task)
  const revisedContract = structuredClone(task.contract)
  revisedContract.objective =
    'Execute the corrected pre-dispatch objective and preserve its exact evidence.'
  revisedContract.expected_output = 'A verified result.md produced after explicit redispatch.'
  revisedContract.acceptance_tests = [
    ...revisedContract.acceptance_tests,
    'the corrected contract version is used by the first run',
  ]
  revisedContract.references = [
    ...revisedContract.references,
    'https://github.com/shyamsridhar123/ecorp/issues/52#redispatch',
  ]
  const revisedPolicy = {
    checks: [
      { type: 'artifact', min_bytes: 1 },
      { type: 'file', path: 'result.md', min_bytes: 50 },
    ],
    manual_gate: null,
  }
  const requestBody = {
    actor_id: demo.alice_actor_id,
    task_id: task.id,
    expected_contract_version: 1,
    next_action: 'redispatch',
    source_run_id: null,
    reason: 'Clarify the expected output and add a repository-file gate before execution.',
    idempotency_key: randomUUID(),
    description: 'Corrected pre-dispatch operator specification.',
    contract: revisedContract,
    verification_policy: revisedPolicy,
  }

  const memberAttempt = await postRaw(
    `/api/corps/${demo.corp_id}/missions/${created.mission_id}/contract-revisions`,
    { ...requestBody, actor_id: demo.bob_actor_id, idempotency_key: randomUUID() },
  )
  assert.equal(memberAttempt.response.status, 403)

  const createdRevision = await post(
    `/api/corps/${demo.corp_id}/missions/${created.mission_id}/contract-revisions`,
    requestBody,
  )
  assert.equal(createdRevision.replayed, false)
  assert.equal(createdRevision.revision.version, 2)
  const replay = await post(
    `/api/corps/${demo.corp_id}/missions/${created.mission_id}/contract-revisions`,
    requestBody,
  )
  assert.equal(replay.replayed, true)
  assert.equal(replay.revision.id, createdRevision.revision.id)

  await psql(
    `DELETE FROM room_memberships WHERE room_id = '${demo.room_id}' AND actor_id = '${demo.alice_actor_id}'`,
  )
  const removedRoomReplay = await postRaw(
    `/api/corps/${demo.corp_id}/missions/${created.mission_id}/contract-revisions`,
    requestBody,
  )
  assert.equal(removedRoomReplay.response.status, 403)
  await psql(
    `INSERT INTO room_memberships (room_id, actor_id, role) VALUES ('${demo.room_id}', '${demo.alice_actor_id}', 'member') ON CONFLICT (room_id, actor_id) DO NOTHING`,
  )

  const stale = await postRaw(
    `/api/corps/${demo.corp_id}/missions/${created.mission_id}/contract-revisions`,
    {
      ...requestBody,
      idempotency_key: randomUUID(),
      reason: 'This stale revision must fail.',
    },
  )
  assert.equal(stale.response.status, 400)

  const revised = await snapshot(demo)
  const revisedMission = revised.snapshot.missions.find(
    (candidate) => candidate.id === created.mission_id,
  )
  const revisedTask = revised.snapshot.tasks.find(
    (candidate) => candidate.id === created.task_id,
  )
  assert.equal(revisedMission?.specification_version, 2)
  assert.equal(revisedTask?.contract_version, 2)
  assert.equal(revisedMission?.description, requestBody.description)
  assert.ok(revisedTask?.contract.objective.startsWith(requestBody.description))
  assert.deepEqual(revisedTask?.verification_policy, revisedPolicy)
  assert.equal(
    revised.snapshot.mission_contract_revisions.filter(
      (revision) => revision.mission_id === created.mission_id,
    ).length,
    1,
  )

  const launch = await post(
    `/api/corps/${demo.corp_id}/missions/${created.mission_id}/launch`,
    { requested_by: demo.alice_actor_id },
  )
  const completed = await waitForMission(
    demo,
    created.mission_id,
    ({ mission }) => mission.status === 'completed',
  )
  const evidence = completed.state.snapshot.verification_evidence.filter(
    (item) => item.run_id === launch.run_id,
  )
  assert.deepEqual(
    evidence.map((item) => item.kind),
    ['artifact', 'file'],
  )
  return {
    mission_id: created.mission_id,
    task_id: task.id,
    revision_id: createdRevision.revision.id,
    member_denial_status: memberAttempt.response.status,
    room_replay_denial_status: removedRoomReplay.response.status,
    stale_version_status: stale.response.status,
    idempotent_replay: replay.replayed,
    contract_version: revisedTask?.contract_version,
    explicit_dispatch_run_id: launch.run_id,
    final_status: completed.mission.status,
  }
}

async function resumeRevision() {
  const demo = await post('/api/demo/reset', {})
  const created = await post(`/api/corps/${demo.corp_id}/missions`, {
    requested_by: demo.alice_actor_id,
    preferred_adapter: 'codex',
    strategy: 'single',
    title: '[fail] Preserve a provider session for a bounded contract revision.',
    description: 'Initial provider attempt should fail and preserve its worktree.',
    contract: richContract(),
    verification_policy: {
      checks: [{ type: 'artifact', min_bytes: 1 }],
      manual_gate: null,
    },
  })
  const launch = await post(
    `/api/corps/${demo.corp_id}/missions/${created.mission_id}/launch`,
    { requested_by: demo.alice_actor_id },
  )
  const failed = await waitForMission(
    demo,
    created.mission_id,
    ({ runs }) =>
      runs.length >= 1 &&
      runs.every(
        (run) =>
          ['completed', 'failed', 'cancelled', 'lost'].includes(run.status) &&
          ['preserved', 'removed'].includes(run.workspace_disposition),
      ) &&
      runs.some(
        (run) =>
          run.status === 'failed' &&
          run.provider_session_id &&
          run.workspace_disposition === 'preserved',
      ),
  )
  const sourceRun = failed.runs.find(
    (run) =>
      run.status === 'failed' &&
      run.provider_session_id &&
      run.workspace_disposition === 'preserved',
  )
  const task = failed.tasks.find((candidate) => candidate.id === created.task_id)
  assert.ok(sourceRun)
  assert.ok(task)
  const widened = structuredClone(task.contract)
  widened.allowed_tools = [...widened.allowed_tools, 'unbounded-network']
  const wideningAttempt = await postRaw(
    `/api/corps/${demo.corp_id}/missions/${created.mission_id}/contract-revisions`,
    {
      actor_id: demo.alice_actor_id,
      task_id: task.id,
      expected_contract_version: task.contract_version,
      next_action: 'resume',
      source_run_id: sourceRun.id,
      reason: 'This attempted authority widening must fail closed.',
      idempotency_key: randomUUID(),
      description: failed.mission.description,
      contract: widened,
      verification_policy: task.verification_policy,
    },
  )
  assert.equal(wideningAttempt.response.status, 400)

  const replacement = structuredClone(task.contract)
  replacement.objective =
    'Resume only to produce resumed.txt and satisfy the corrected bounded verifier.'
  replacement.expected_output = 'A verified resumed.txt in the preserved worktree.'
  replacement.allowed_tools = replacement.allowed_tools.slice(0, 1)
  replacement.prohibited_actions = [
    ...replacement.prohibited_actions,
    'perform any work beyond resumed.txt',
  ]
  replacement.write_scope = ['resumed.txt']
  replacement.acceptance_tests = [
    ...replacement.acceptance_tests,
    'resume reuses the preserved provider session and worktree',
    'resumed.txt passes the corrected verifier',
  ]
  const replacementPolicy = {
    checks: [
      { type: 'file', path: 'resumed.txt', min_bytes: 8 },
      {
        type: 'test',
        program: 'node',
        args: [
          '-e',
          "const fs=require('fs');const p=fs.readFileSync('resume-prompt.txt','utf8');process.exit(p.includes('Resume only the bounded remaining work')&&p.includes('Resume only to produce resumed.txt')?0:1)",
        ],
        timeout_ms: 5_000,
      },
    ],
    manual_gate: null,
  }
  const requestBody = {
    actor_id: demo.alice_actor_id,
    task_id: task.id,
    expected_contract_version: task.contract_version,
    next_action: 'resume',
    source_run_id: sourceRun.id,
    reason: 'Narrow the failed attempt to the exact remaining file and verifier.',
    idempotency_key: randomUUID(),
    description: 'Resume only the bounded remaining work in the preserved provider session.',
    contract: replacement,
    verification_policy: replacementPolicy,
  }
  const revision = await post(
    `/api/corps/${demo.corp_id}/missions/${created.mission_id}/contract-revisions`,
    requestBody,
  )
  assert.equal(revision.replayed, false)
  const replay = await post(
    `/api/corps/${demo.corp_id}/missions/${created.mission_id}/contract-revisions`,
    requestBody,
  )
  assert.equal(replay.replayed, true)
  assert.equal(replay.revision.id, revision.revision.id)

  const resumed = await post(
    `/api/corps/${demo.corp_id}/runs/${sourceRun.id}/resume`,
    {
      requested_by: demo.alice_actor_id,
      prompt: 'Complete only the revised bounded contract.',
    },
  )
  const completed = await waitForMission(
    demo,
    created.mission_id,
    ({ runs }) =>
      runs.some(
        (run) =>
          run.id === resumed.run_id &&
          run.status === 'completed' &&
          run.verification_status === 'passed',
      ),
  )
  const resumedRun = completed.runs.find((run) => run.id === resumed.run_id)
  const revisedTask = completed.tasks.find(
    (candidate) => candidate.id === created.task_id,
  )
  const evidence = completed.state.snapshot.verification_evidence.filter(
    (item) => item.run_id === resumed.run_id,
  )
  assert.equal(revisedTask?.contract_version, 2)
  assert.ok(revisedTask?.contract.objective.startsWith(requestBody.description))
  assert.equal(resumedRun?.resumed_from_run_id, sourceRun.id)
  assert.equal(resumedRun?.provider_session_id, sourceRun.provider_session_id)
  assert.equal(resumedRun?.workspace_path, sourceRun.workspace_path)
  assert.deepEqual(evidence.map((item) => item.kind), ['file', 'test'])
  assert.ok(evidence.every((item) => item.status === 'passed'))
  return {
    mission_id: created.mission_id,
    task_id: task.id,
    source_run_id: sourceRun.id,
    resumed_run_id: resumed.run_id,
    revision_id: revision.revision.id,
    widening_denial_status: wideningAttempt.response.status,
    idempotent_replay: replay.replayed,
    contract_version: revisedTask?.contract_version,
    same_provider_session:
      resumedRun?.provider_session_id === sourceRun.provider_session_id,
    same_worktree: resumedRun?.workspace_path === sourceRun.workspace_path,
    evidence_kinds: evidence.map((item) => item.kind),
    final_status: completed.mission.status,
  }
}

const report = {
  checked_at: new Date().toISOString(),
  rich_creation_and_typed_verification:
    await richCreationAndTypedVerification(),
  failing_test_blocks_completion: await failingTestBlocksCompletion(),
  authored_human_approval: await authoredHumanApproval(),
  redispatch_revision: await redispatchRevision(),
  resume_revision: await resumeRevision(),
}

await writeFile(
  path.join(root, 'output', 'e2e-mission-contracts.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
