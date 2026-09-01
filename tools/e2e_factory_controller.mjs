import assert from 'node:assert/strict'
import { execFile as execFileCallback } from 'node:child_process'
import { promisify } from 'node:util'
import { readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'

const execFile = promisify(execFileCallback)
const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const root = path.resolve(import.meta.dirname, '..')
const binary =
  process.env.CRONY_CLI_BINARY ??
  path.join(
    root,
    'target',
    'debug',
    process.platform === 'win32' ? 'crony-cli.exe' : 'crony-cli',
  )
const statePath = path.join(root, 'output', 'fake-github-factory-state.json')
const fakeGithub = path.join(root, 'tools', 'fake_github_cli.mjs')

async function request(url, init) {
  const response = await fetch(`${server}${url}`, init)
  const body = await response.json()
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

async function runController(
  demo,
  issueNumber,
  dryRun = false,
  {
    actorId = demo.alice_actor_id,
    leaseSeconds = 300,
    repository = 'shyamsridhar123/ecorp',
  } = {},
) {
  const args = [
    'factory',
    demo.corp_id,
    actorId,
    '--owner',
    'acme',
    '--project-number',
    '7',
    '--repository',
    repository,
    '--adapter',
    'fake-process',
    '--budget-tokens',
    '20000',
    '--budget-cost-microusd',
    '1000000',
    '--lease-seconds',
    String(leaseSeconds),
    '--github-cli',
    process.execPath,
  ]
  if (issueNumber !== null) args.push('--issue', String(issueNumber))
  if (dryRun) args.push('--dry-run')
  const { stdout } = await execFile(binary, args, {
    cwd: root,
    env: {
      ...process.env,
      ECORP_GITHUB_CLI_PREFIX_ARGS_JSON: JSON.stringify([fakeGithub]),
      ECORP_FAKE_GITHUB_STATE: statePath,
    },
    maxBuffer: 4 * 1024 * 1024,
    windowsHide: true,
  })
  return JSON.parse(stdout)
}

async function waitForMission(demo, missionId, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const state = await snapshot(demo)
    const mission = state.snapshot.missions.find((item) => item.id === missionId)
    if (mission && ['completed', 'failed', 'cancelled'].includes(mission.status)) {
      return { state, mission }
    }
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error(`timed out waiting for factory controller mission ${missionId}`)
}

const demo = await post('/api/demo/reset', {})
const issue = {
  id: 'I_FAKE_FACTORY_9001',
  number: 9001,
  title: 'Build a deterministic factory controller canary',
  body: `## Outcome

Run one governed issue through ECorp and produce verified evidence.

## Acceptance criteria

- [ ] one durable factory work item exists
- [ ] one mission and one run complete
- [ ] replay creates no duplicate

## Dependencies

No blockers.
`,
  url: 'https://github.com/shyamsridhar123/ecorp/issues/9001',
  state: 'OPEN',
  createdAt: '2026-09-01T14:00:00Z',
  updatedAt: '2026-09-01T14:00:00Z',
  labels: [{ name: 'factory:ready' }],
}
await writeFile(
  statePath,
  `${JSON.stringify(
    {
      repository: 'shyamsridhar123/ecorp',
      project: {
        id: 'PVT_FAKE_FACTORY',
        number: 7,
        owner: 'acme',
        title: 'Factory Test Project',
        status_field_id: 'PVTSSF_FAKE_STATUS',
        status_options: [
          { id: 'todo', name: 'Todo' },
          { id: 'in-progress', name: 'In Progress' },
          { id: 'done', name: 'Done' },
        ],
      },
      items: [
        {
          id: 'PVTI_FAKE_FACTORY_9001',
          status: 'Todo',
          content: {
            body: issue.body,
            number: issue.number,
            repository: 'shyamsridhar123/ecorp',
            title: issue.title,
            type: 'Issue',
            url: issue.url,
          },
        },
      ],
      issues: { '9001': issue },
      item_edits: 0,
    },
    null,
    2,
  )}\n`,
)

const dryRun = await runController(demo, 9001, true)
assert.equal(dryRun.mode, 'dry_run')
assert.equal(dryRun.selected.issue_number, 9001)
assert.equal(dryRun.selected.eligible, true)
assert.deepEqual(dryRun.mutations, [])
assert.equal(JSON.parse(await readFile(statePath, 'utf8')).item_edits, 0)

const first = await runController(demo, 9001)
assert.equal(first.mode, 'executed')
assert.equal(first.issue_number, 9001)
assert.equal(first.project_status, 'In Progress')
assert.equal(first.materialized_now, true)
assert.equal(first.factory_state, 'running')
assert.equal(first.auto_merge, false)
const fencedState = await snapshot(demo)
const fencedItem = fencedState.snapshot.factory_work_items.find(
  (item) => item.id === first.factory_work_item_id,
)
assert.ok(Date.parse(fencedItem.lease_expires_at) > Date.now() + 240_000)
const fencedEvents = fencedState.snapshot.events.filter(
  (event) => event.aggregate_id === first.factory_work_item_id,
)
assert.ok(
  fencedEvents.filter((event) => event.type === 'factory.claim_renewed').length >= 2,
)
const fencedTask = fencedState.snapshot.tasks.find(
  (task) => task.mission_id === first.mission_id,
)
assert.equal(fencedTask.contract.source_repository, 'shyamsridhar123/ecorp')
assert.equal(fencedTask.contract.source_base_ref, 'HEAD')

const completed = await waitForMission(demo, first.mission_id)
assert.equal(completed.mission.status, 'completed')
const replay = await runController(demo, 9001)
assert.equal(replay.factory_work_item_id, first.factory_work_item_id)
assert.equal(replay.mission_id, first.mission_id)
assert.equal(replay.materialized_now, false)
assert.equal(replay.launch.recovered, true)
assert.equal(replay.factory_state, 'verified')

const finalState = await snapshot(demo)
const factoryItems = finalState.snapshot.factory_work_items.filter(
  (item) => item.source_project_item_id === 'PVTI_FAKE_FACTORY_9001',
)
assert.equal(factoryItems.length, 1)
assert.equal(factoryItems[0].state, 'verified')
assert.equal(Object.hasOwn(factoryItems[0], 'claim_token'), false)
const missions = finalState.snapshot.missions.filter(
  (item) => item.id === first.mission_id,
)
assert.equal(missions.length, 1)
const taskIds = new Set(
  finalState.snapshot.tasks
    .filter((task) => task.mission_id === first.mission_id)
    .map((task) => task.id),
)
const runs = finalState.snapshot.runs.filter((run) => taskIds.has(run.task_id))
assert.equal(runs.length, 1)
assert.equal(runs[0].status, 'completed')
const fakeState = JSON.parse(await readFile(statePath, 'utf8'))
assert.equal(fakeState.items[0].status, 'In Progress')
assert.ok(fakeState.item_edits >= 1)
const successfulProjectStatus = fakeState.items[0].status

const queuedIssue = {
  ...issue,
  id: 'I_FAKE_FACTORY_9003',
  number: 9003,
  title: 'Advance to the next eligible issue after verification',
  url: 'https://github.com/shyamsridhar123/ecorp/issues/9003',
  createdAt: '2026-09-01T14:02:00Z',
  updatedAt: '2026-09-01T14:02:00Z',
}
fakeState.items.push({
  id: 'PVTI_FAKE_FACTORY_9003',
  status: 'Todo',
  content: {
    body: queuedIssue.body,
    number: queuedIssue.number,
    repository: 'shyamsridhar123/ecorp',
    title: queuedIssue.title,
    type: 'Issue',
    url: queuedIssue.url,
  },
})
fakeState.issues['9003'] = queuedIssue
await writeFile(statePath, `${JSON.stringify(fakeState, null, 2)}\n`)
const nextIssue = await runController(demo, null)
assert.equal(nextIssue.issue_number, 9003)
assert.equal(nextIssue.factory_state, 'running')
await waitForMission(demo, nextIssue.mission_id)
const nextIssueVerified = await runController(demo, 9003)
assert.equal(nextIssueVerified.factory_state, 'verified')

const recoveryDemo = await post('/api/demo/reset', {})
const recoveryIssue = {
  ...issue,
  id: 'I_FAKE_FACTORY_9002',
  number: 9002,
  title: 'Recover a factory mission after Project status synchronization fails',
  url: 'https://github.com/shyamsridhar123/ecorp/issues/9002',
  createdAt: '2026-09-01T14:01:00Z',
  updatedAt: '2026-09-01T14:01:00Z',
}
fakeState.items = [
  {
    id: 'PVTI_FAKE_FACTORY_9002',
    status: 'Todo',
    content: {
      body: recoveryIssue.body,
      number: recoveryIssue.number,
      repository: 'shyamsridhar123/ecorp',
      title: recoveryIssue.title,
      type: 'Issue',
      url: recoveryIssue.url,
    },
  },
]
fakeState.issues = { '9002': recoveryIssue }
fakeState.item_edits = 0
fakeState.fail_next_item_edit = true
await writeFile(statePath, `${JSON.stringify(fakeState, null, 2)}\n`)

let injectedFailure
try {
  await runController(recoveryDemo, 9002)
} catch (error) {
  injectedFailure = error
}
assert.ok(injectedFailure, 'injected Project status failure was not surfaced')
const blockedState = await snapshot(recoveryDemo)
const blockedItems = blockedState.snapshot.factory_work_items.filter(
  (item) => item.source_issue_number === 9002,
)
assert.equal(blockedItems.length, 1)
assert.equal(blockedItems[0].state, 'blocked')
assert.match(blockedItems[0].failure_detail, /status synchronization failed/)
assert.ok(blockedItems[0].mission_id)
const blockedMissionRuns = blockedState.snapshot.runs.filter((run) =>
  blockedState.snapshot.tasks.some(
    (task) =>
      task.id === run.task_id && task.mission_id === blockedItems[0].mission_id,
  ),
)
assert.equal(blockedMissionRuns.length, 0)

const recovered = await runController(recoveryDemo, 9002)
assert.equal(recovered.factory_work_item_id, blockedItems[0].id)
assert.equal(recovered.mission_id, blockedItems[0].mission_id)
assert.equal(recovered.factory_state, 'running')
const recoveryCompleted = await waitForMission(recoveryDemo, recovered.mission_id)
assert.equal(recoveryCompleted.mission.status, 'completed')
const verifiedRecovery = await runController(recoveryDemo, 9002)
assert.equal(verifiedRecovery.factory_work_item_id, blockedItems[0].id)
assert.equal(verifiedRecovery.factory_state, 'verified')
const recoveredState = await snapshot(recoveryDemo)
const recoveredItem = recoveredState.snapshot.factory_work_items.find(
  (item) => item.id === blockedItems[0].id,
)
assert.equal(recoveredItem.state, 'verified')
const recoveredTaskIds = new Set(
  recoveredState.snapshot.tasks
    .filter((task) => task.mission_id === recovered.mission_id)
    .map((task) => task.id),
)
const recoveredRuns = recoveredState.snapshot.runs.filter((run) =>
  recoveredTaskIds.has(run.task_id),
)
assert.equal(recoveredRuns.length, 1)
const recoveredFakeState = JSON.parse(await readFile(statePath, 'utf8'))
assert.equal(recoveredFakeState.items[0].status, 'In Progress')
assert.equal(recoveredFakeState.item_edit_failures, 1)
const recoveredProjectStatus = recoveredFakeState.items[0].status

const failureDemo = await post('/api/demo/reset', {})
const failureIssue = {
  ...issue,
  id: 'I_FAKE_FACTORY_9004',
  number: 9004,
  title: '[always-fail] Persist a terminal factory failure',
  url: 'https://github.com/shyamsridhar123/ecorp/issues/9004',
  createdAt: '2026-09-01T14:03:00Z',
  updatedAt: '2026-09-01T14:03:00Z',
}
recoveredFakeState.items = [
  {
    id: 'PVTI_FAKE_FACTORY_9004',
    status: 'Todo',
    content: {
      body: failureIssue.body,
      number: failureIssue.number,
      repository: 'shyamsridhar123/ecorp',
      title: failureIssue.title,
      type: 'Issue',
      url: failureIssue.url,
    },
  },
]
recoveredFakeState.issues = { '9004': failureIssue }
recoveredFakeState.item_edits = 0
recoveredFakeState.fail_next_item_edit = false
await writeFile(statePath, `${JSON.stringify(recoveredFakeState, null, 2)}\n`)
const failing = await runController(failureDemo, 9004)
const failedMission = await waitForMission(failureDemo, failing.mission_id)
assert.equal(failedMission.mission.status, 'failed')
let terminalFailure
try {
  await runController(failureDemo, 9004)
} catch (error) {
  terminalFailure = error
}
assert.ok(terminalFailure, 'terminal mission failure did not fail the controller command')
const failureState = await snapshot(failureDemo)
const failedItem = failureState.snapshot.factory_work_items.find(
  (item) => item.id === failing.factory_work_item_id,
)
assert.equal(failedItem.state, 'failed')
assert.match(failedItem.failure_detail, /fail/i)

const mismatchDemo = await post('/api/demo/reset', {})
const mismatchIssue = {
  ...issue,
  id: 'I_FAKE_FACTORY_9006',
  number: 9006,
  title: 'Reject a runner checked out to the wrong repository',
  url: 'https://github.com/acme/widget/issues/9006',
  createdAt: '2026-09-01T14:05:00Z',
  updatedAt: '2026-09-01T14:05:00Z',
}
recoveredFakeState.repository = 'acme/widget'
recoveredFakeState.items = [
  {
    id: 'PVTI_FAKE_FACTORY_9006',
    status: 'Todo',
    content: {
      body: mismatchIssue.body,
      number: mismatchIssue.number,
      repository: 'acme/widget',
      title: mismatchIssue.title,
      type: 'Issue',
      url: mismatchIssue.url,
    },
  },
]
recoveredFakeState.issues = { '9006': mismatchIssue }
recoveredFakeState.item_edits = 0
await writeFile(statePath, `${JSON.stringify(recoveredFakeState, null, 2)}\n`)
let repositoryMismatch
try {
  await runController(mismatchDemo, 9006, false, {
    repository: 'acme/widget',
  })
} catch (error) {
  repositoryMismatch = error
}
assert.ok(repositoryMismatch, 'repository mismatch did not fail the controller')
const mismatchState = await snapshot(mismatchDemo)
const mismatchItem = mismatchState.snapshot.factory_work_items.find(
  (item) => item.source_issue_number === 9006,
)
assert.equal(mismatchItem.state, 'blocked')
assert.match(mismatchItem.failure_detail, /repository|runner|dispatch/i)
const mismatchTasks = mismatchState.snapshot.tasks.filter(
  (task) => task.mission_id === mismatchItem.mission_id,
)
assert.ok(mismatchTasks.length > 0)
assert.ok(
  mismatchTasks.every(
    (task) =>
      task.contract.source_repository === 'acme/widget' &&
      task.contract.source_base_ref === 'HEAD',
  ),
)
const mismatchTaskIds = new Set(mismatchTasks.map((task) => task.id))
assert.equal(
  mismatchState.snapshot.runs.filter((run) => mismatchTaskIds.has(run.task_id)).length,
  0,
)

const report = {
  checked_at: new Date().toISOString(),
  issue_number: 9001,
  project_item_id: 'PVTI_FAKE_FACTORY_9001',
  project_status: successfulProjectStatus,
  factory_work_item_id: first.factory_work_item_id,
  mission_id: first.mission_id,
  run_ids: runs.map((run) => run.id),
  dry_run_mutated_nothing: true,
  exactly_one_work_item: true,
  exactly_one_mission: true,
  exactly_one_run: true,
  replay_recovered_existing_mission: true,
  external_effect_lease_revalidated: true,
  claimed_repository_persisted_to_task: true,
  factory_state: factoryItems[0].state,
  claim_token_absent_from_snapshot: true,
  mission_status: missions[0].status,
  run_status: runs[0].status,
  auto_merge: false,
  queue_progression: {
    next_issue_number: nextIssue.issue_number,
    verified_items_do_not_starve_todo_work: true,
  },
  project_status_failure: {
    issue_number: 9002,
    factory_work_item_id: recoveredItem.id,
    mission_id: recovered.mission_id,
    run_ids: recoveredRuns.map((run) => run.id),
    durable_blocked_state_recorded: true,
    retry_reused_existing_mission: true,
    recovered_factory_state: recoveredItem.state,
    project_status: recoveredProjectStatus,
  },
  terminal_mission_failure: {
    issue_number: 9004,
    factory_work_item_id: failedItem.id,
    mission_id: failing.mission_id,
    mission_status: failedMission.mission.status,
    factory_state: failedItem.state,
    controller_returned_error: true,
  },
  repository_routing: {
    issue_number: 9006,
    factory_work_item_id: mismatchItem.id,
    mission_id: mismatchItem.mission_id,
    required_repository: 'acme/widget',
    required_base_ref: 'HEAD',
    mismatched_runner_rejected: true,
    run_count: 0,
    factory_state: mismatchItem.state,
  },
}
await writeFile(
  path.join(root, 'output', 'e2e-factory-controller.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
