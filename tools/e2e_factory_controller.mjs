import assert from 'node:assert/strict'
import { execFile as execFileCallback } from 'node:child_process'
import { promisify } from 'node:util'
import { rm, readFile, writeFile } from 'node:fs/promises'
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
const sourceBaseCommit = (
  await execFile('git', ['rev-parse', 'HEAD'], { cwd: root, windowsHide: true })
).stdout.trim()

async function createSourceFixture(repository) {
  const fixture = path.join(root, 'output', `factory-source-${repository.replace('/', '-')}`)
  await rm(fixture, { recursive: true, force: true })
  await execFile('git', ['clone', '--quiet', '--no-hardlinks', root, fixture], {
    cwd: root,
    windowsHide: true,
  })
  await execFile(
    'git',
    ['remote', 'set-url', 'origin', `https://github.com/${repository}.git`],
    { cwd: fixture, windowsHide: true },
  )
  return fixture
}

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

async function lookupFactoryItems(
  demo,
  sourceProjectItemIds,
  {
    actorId = demo.alice_actor_id,
    sourceProjectOwner = 'acme',
    sourceProjectNumber = 7,
  } = {},
) {
  return post(`/api/corps/${demo.corp_id}/factory/work-items/lookup`, {
    actor_id: actorId,
    source_project_owner: sourceProjectOwner,
    source_project_number: sourceProjectNumber,
    source_project_item_ids: sourceProjectItemIds,
  })
}

function factoryPolicy({
  budgetTokens = 20_000,
  budgetCostMicrousd = 1_000_000,
  writeScope = ['**'],
  projectOwner = 'acme',
  projectNumber = 7,
} = {}) {
  return {
    schema_version: 1,
    source_of_truth: 'github_project',
    project_owner: projectOwner,
    project_number: projectNumber,
    project_status: 'Todo',
    required_label: 'factory:ready',
    dependencies: [],
    repository_allowlist: ['shyamsridhar123/ecorp'],
    source_base_ref: 'HEAD',
    source_base_commit: sourceBaseCommit,
    source_commit_upgrade_required: false,
    adapter_allowlist: ['fake-process'],
    strategy_allowlist: ['single'],
    model: null,
    reasoning_effort: null,
    write_scope: writeScope,
    allowed_tools: ['filesystem', 'shell'],
    prohibited_actions: [
      'modify files outside the assigned worktree',
      'use undeclared long-lived credentials',
      'merge or deploy without a separate current authorization',
    ],
    secret_ids: [],
    verification_required: true,
    budget_tokens: budgetTokens,
    budget_cost_microusd: budgetCostMicrousd,
    auto_merge: false,
  }
}

async function createHistoricalFactoryItems(demo, count) {
  const claimPath = `/api/corps/${demo.corp_id}/factory/work-items/claim`
  const batchSize = 32
  for (let start = 0; start < count; start += batchSize) {
    await Promise.all(
      Array.from({ length: Math.min(batchSize, count - start) }, (_, offset) => {
        const index = start + offset
        const suffix = String(index).padStart(4, '0')
        return post(claimPath, {
          actor_id: demo.alice_actor_id,
          source_project_owner: 'acme',
          source_project_number: 7,
          source_project_item_id: `PVTI_FAKE_FACTORY_HISTORY_${suffix}`,
          source_repository_owner: 'shyamsridhar123',
          source_repository_name: 'ecorp',
          source_issue_number: 20_000 + index,
          source_issue_node_id: `I_FAKE_FACTORY_HISTORY_${suffix}`,
          source_issue_url: `https://github.com/shyamsridhar123/ecorp/issues/${20_000 + index}`,
          source_title: `Historical factory work item ${suffix}`,
          source_revision: `2026-08-31T${String(Math.floor(index / 60) % 24).padStart(2, '0')}:${String(index % 60).padStart(2, '0')}:00Z`,
          idempotency_key: `factory-history-${suffix}`,
          lease_seconds: 300,
          policy: factoryPolicy(),
        })
      }),
    )
  }
}

async function runController(
  demo,
  issueNumber,
  dryRun = false,
  {
    actorId = demo.alice_actor_id,
    leaseSeconds = 300,
    repository = 'ShyamSridhar123/ECorp',
    strategy = 'single',
    githubTimeoutMs,
    sourceRepositoryPath = root,
    publicationBaseRef = 'main',
    budgetTokens = 20_000,
    budgetCostMicrousd = 1_000_000,
    writeScope = ['**'],
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
    '--source-repository-path',
    sourceRepositoryPath,
    '--publication-base-ref',
    publicationBaseRef,
    '--adapter',
    'fake-process',
    '--strategy',
    strategy,
    '--budget-tokens',
    String(budgetTokens),
    '--budget-cost-microusd',
    String(budgetCostMicrousd),
    '--lease-seconds',
    String(leaseSeconds),
    '--github-cli',
    process.execPath,
  ]
  for (const scope of writeScope) args.push('--write-scope', scope)
  if (issueNumber !== null) args.push('--issue', String(issueNumber))
  if (dryRun) args.push('--dry-run')
  const { stdout } = await execFile(binary, args, {
    cwd: root,
    env: {
      ...process.env,
      ECORP_GITHUB_CLI_PREFIX_ARGS_JSON: JSON.stringify([fakeGithub]),
      ECORP_FAKE_GITHUB_STATE: statePath,
      ...(githubTimeoutMs
        ? { ECORP_GITHUB_COMMAND_TIMEOUT_MS: String(githubTimeoutMs) }
        : {}),
    },
    maxBuffer: 4 * 1024 * 1024,
    windowsHide: true,
  })
  return JSON.parse(stdout)
}

async function waitForMission(demo, missionId, timeoutMs = 180_000) {
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

async function waitForApproval(demo, missionId, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const state = await snapshot(demo)
    const taskIds = new Set(
      state.snapshot.tasks
        .filter((task) => task.mission_id === missionId)
        .map((task) => task.id),
    )
    const run = state.snapshot.runs.find((item) => taskIds.has(item.task_id))
    const request = run
      ? state.snapshot.verification_requests.find(
          (item) => item.run_id === run.id && item.status === 'pending',
        )
      : undefined
    if (run?.status === 'waiting_for_approval' && request) {
      return { state, run, request }
    }
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error(`timed out waiting for factory approval ${missionId}`)
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

const controlCharacterPolicyResponse = await fetch(
  `${server}/api/corps/${demo.corp_id}/factory/work-items/claim`,
  {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
    actor_id: demo.alice_actor_id,
    source_project_owner: 'acme',
    source_project_number: 7,
    source_project_item_id: 'PVTI_FAKE_FACTORY_CONTROL_REF',
    source_repository_owner: 'shyamsridhar123',
    source_repository_name: 'ecorp',
    source_issue_number: 9098,
    source_issue_node_id: 'I_FAKE_FACTORY_CONTROL_REF',
    source_issue_url: 'https://github.com/shyamsridhar123/ecorp/issues/9098',
    source_title: 'Reject control characters in publication base policy',
    source_revision: '2026-09-02T00:00:00Z',
    idempotency_key: 'factory-control-ref-rejected',
    lease_seconds: 300,
    policy: {
      ...factoryPolicy(),
      publication: {
        allowed: true,
        repository_allowlist: ['shyamsridhar123/ecorp'],
        base_ref: 'main\tbad',
        branch_prefix: 'ecorp/',
        status_before: 'In Progress',
        review_status: 'In Review',
        auto_merge: false,
        merge: false,
        deploy: false,
      },
    },
    }),
  },
)
const controlCharacterPolicyRejected =
  await controlCharacterPolicyResponse.json()
assert.equal(controlCharacterPolicyResponse.status, 400)
assert.match(controlCharacterPolicyRejected.error, /safe Git ref/)

let invalidPublicationBaseRejected = false
try {
  await runController(demo, 9001, true, {
    publicationBaseRef: 'refs/tags/v1',
  })
  assert.fail('non-branch publication base unexpectedly passed validation')
} catch (error) {
  assert.match(
    String(error.stderr ?? error),
    /publication base ref must be HEAD or a branch ref/,
  )
  invalidPublicationBaseRejected = true
}
const preValidationSnapshot = await snapshot(demo)
assert.equal(preValidationSnapshot.snapshot.factory_work_items.length, 0)
assert.equal(JSON.parse(await readFile(statePath, 'utf8')).item_edits, 0)

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
assert.equal(fencedTask.contract.source_base_commit, sourceBaseCommit)
assert.deepEqual(fencedTask.contract.deliverable, {
  form: 'commit_branch',
  commit_after_verification: true,
  paths: [],
})

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
assert.equal(runs[0].source_repository, 'shyamsridhar123/ecorp')
assert.equal(runs[0].source_base_ref, 'HEAD')
assert.equal(runs[0].source_base_commit, sourceBaseCommit)
const sourceDeliverables = finalState.snapshot.source_deliverables.filter(
  (deliverable) => deliverable.run_id === runs[0].id,
)
assert.equal(sourceDeliverables.length, 1)
assert.equal(sourceDeliverables[0].form, 'commit_branch')
assert.equal(sourceDeliverables[0].integration_state, 'ready_for_review')
assert.equal(runs[0].workspace_disposition, 'preserved')
const factoryWorktreeHead = (
  await execFile('git', ['rev-parse', 'HEAD'], {
    cwd: runs[0].workspace_path,
    windowsHide: true,
  })
).stdout.trim()
assert.equal(sourceDeliverables[0].head_commit, factoryWorktreeHead)
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
fakeState.fail_next_item_edit_message =
  'injected GitHub Project status update failure\nrequest details withheld'
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
assert.equal(/[\u0000-\u001f\u007f]/u.test(blockedItems[0].failure_detail), false)
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

const paginationDemo = await post('/api/demo/reset', {})
const paginationIssue = {
  ...issue,
  id: 'I_FAKE_FACTORY_9020',
  number: 9020,
  title: 'Recover an authoritative factory item beyond the snapshot limit',
  url: 'https://github.com/shyamsridhar123/ecorp/issues/9020',
  createdAt: '2026-09-01T14:20:00Z',
  updatedAt: '2026-09-01T14:20:00Z',
}
const paginationExpectedMissionTitle =
  `GitHub #${paginationIssue.number}: ${paginationIssue.title}`
const paginationSourceMarker = [
  `ISSUE: #${paginationIssue.number} — ${paginationIssue.title}`,
  `URL: ${paginationIssue.url}`,
  `SOURCE REVISION: ${paginationIssue.updatedAt}`,
].join('\n')
const paginationState = {
  repository: 'shyamsridhar123/ecorp',
  project: recoveredFakeState.project,
  items: [
    {
      id: 'PVTI_FAKE_FACTORY_9020',
      status: 'Todo',
      content: {
        body: paginationIssue.body,
        number: paginationIssue.number,
        repository: 'shyamsridhar123/ecorp',
        title: paginationIssue.title,
        type: 'Issue',
        url: paginationIssue.url,
      },
    },
  ],
  issues: { '9020': paginationIssue },
  item_edits: 0,
  fail_next_item_edit: true,
  fail_next_item_edit_message:
    'injected pagination recovery status failure',
}
await writeFile(statePath, `${JSON.stringify(paginationState, null, 2)}\n`)
const crossProjectCollisionPolicy = factoryPolicy({
  budgetTokens: 10_000,
  budgetCostMicrousd: 500_000,
  writeScope: ['other-project/**'],
  projectOwner: 'other',
  projectNumber: 8,
})
const crossProjectCollision = await post(
  `/api/corps/${paginationDemo.corp_id}/factory/work-items/claim`,
  {
    actor_id: paginationDemo.alice_actor_id,
    source_project_owner: 'other',
    source_project_number: 8,
    source_project_item_id: 'PVTI_FAKE_FACTORY_9020',
    source_repository_owner: 'shyamsridhar123',
    source_repository_name: 'ecorp',
    source_issue_number: paginationIssue.number,
    source_issue_node_id: paginationIssue.id,
    source_issue_url: paginationIssue.url,
    source_title: paginationIssue.title,
    source_revision: paginationIssue.updatedAt,
    idempotency_key: 'factory-cross-project-collision-9020',
    lease_seconds: 300,
    policy: crossProjectCollisionPolicy,
  },
)
assert.equal(crossProjectCollision.work_item.source_project_owner, 'other')
assert.equal(crossProjectCollision.work_item.source_project_number, 8)
let paginationInitialFailure
try {
  await runController(paginationDemo, 9020)
} catch (error) {
  paginationInitialFailure = error
}
assert.ok(paginationInitialFailure, 'pagination recovery fixture did not block initially')
const paginationInitialSnapshot = await snapshot(paginationDemo)
const paginationInitialItem =
  paginationInitialSnapshot.snapshot.factory_work_items.find(
    (item) =>
      item.source_project_owner === 'acme' &&
      item.source_project_number === 7 &&
      item.source_project_item_id === 'PVTI_FAKE_FACTORY_9020',
  )
assert.equal(paginationInitialItem.state, 'blocked')
assert.ok(paginationInitialItem.mission_id)
const paginationPersistedPolicy = paginationInitialItem.policy
const configuredProjectLookup = await lookupFactoryItems(paginationDemo, [
  'PVTI_FAKE_FACTORY_9020',
])
assert.equal(configuredProjectLookup.total_count, 1)
assert.equal(configuredProjectLookup.items[0].id, paginationInitialItem.id)
assert.deepEqual(configuredProjectLookup.items[0].policy, paginationPersistedPolicy)
const otherProjectLookup = await lookupFactoryItems(
  paginationDemo,
  ['PVTI_FAKE_FACTORY_9020'],
  {
    sourceProjectOwner: 'other',
    sourceProjectNumber: 8,
  },
)
assert.equal(otherProjectLookup.total_count, 1)
assert.equal(otherProjectLookup.items[0].id, crossProjectCollision.work_item.id)
assert.deepEqual(otherProjectLookup.items[0].policy, crossProjectCollisionPolicy)

const historicalWorkItemCount = 501
await createHistoricalFactoryItems(paginationDemo, historicalWorkItemCount)
const truncatedPaginationSnapshot = await snapshot(paginationDemo)
assert.equal(truncatedPaginationSnapshot.snapshot.factory_work_items.length, 500)
assert.equal(
  truncatedPaginationSnapshot.snapshot.factory_work_items.some(
    (item) => item.id === paginationInitialItem.id,
  ),
  false,
)
const paginationLookup = await lookupFactoryItems(paginationDemo, [
  'PVTI_FAKE_FACTORY_9020',
  'PVTI_FAKE_FACTORY_MISSING',
  'PVTI_FAKE_FACTORY_9020',
])
assert.equal(paginationLookup.total_count, 1)
assert.equal(paginationLookup.items.length, 1)
assert.equal(paginationLookup.items[0].id, paginationInitialItem.id)
assert.deepEqual(paginationLookup.items[0].policy, paginationPersistedPolicy)
const guestPaginationLookup = await fetch(
  `${server}/api/corps/${paginationDemo.corp_id}/factory/work-items/lookup`,
  {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      actor_id: paginationDemo.eve_actor_id,
      source_project_owner: 'acme',
      source_project_number: 7,
      source_project_item_ids: ['PVTI_FAKE_FACTORY_9020'],
    }),
  },
)
const guestPaginationLookupBody = await guestPaginationLookup.text()
assert.equal(guestPaginationLookup.status, 403)
assert.equal(guestPaginationLookupBody.includes(paginationIssue.title), false)
assert.equal(guestPaginationLookupBody.includes(paginationInitialItem.id), false)
const oversizedPaginationLookup = await fetch(
  `${server}/api/corps/${paginationDemo.corp_id}/factory/work-items/lookup`,
  {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      actor_id: paginationDemo.alice_actor_id,
      source_project_owner: 'acme',
      source_project_number: 7,
      source_project_item_ids: Array.from(
        { length: 1001 },
        (_, index) => `PVTI_LOOKUP_BOUND_${index}`,
      ),
    }),
  },
)
assert.equal(oversizedPaginationLookup.status, 400)
const overlongPaginationLookup = await fetch(
  `${server}/api/corps/${paginationDemo.corp_id}/factory/work-items/lookup`,
  {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      actor_id: paginationDemo.alice_actor_id,
      source_project_owner: 'acme',
      source_project_number: 7,
      source_project_item_ids: [`PVTI_${'X'.repeat(236)}`],
    }),
  },
)
assert.equal(overlongPaginationLookup.status, 400)

const paginationRecovered = await runController(paginationDemo, 9020, false, {
  leaseSeconds: 600,
  budgetTokens: 30_000,
  budgetCostMicrousd: 2_000_000,
  writeScope: ['crates/**'],
})
assert.equal(paginationRecovered.factory_work_item_id, paginationInitialItem.id)
assert.equal(paginationRecovered.mission_id, paginationInitialItem.mission_id)
assert.equal(paginationRecovered.materialized_now, false)
assert.equal(paginationRecovered.factory_state, 'running')
const paginationCompleted = await waitForMission(
  paginationDemo,
  paginationRecovered.mission_id,
)
assert.equal(paginationCompleted.mission.status, 'completed')
const paginationVerified = await runController(paginationDemo, 9020, false, {
  leaseSeconds: 600,
  budgetTokens: 30_000,
  budgetCostMicrousd: 2_000_000,
  writeScope: ['crates/**'],
})
assert.equal(paginationVerified.factory_work_item_id, paginationInitialItem.id)
assert.equal(paginationVerified.mission_id, paginationInitialItem.mission_id)
assert.equal(paginationVerified.factory_state, 'verified')
const paginationFinalLookup = await lookupFactoryItems(paginationDemo, [
  'PVTI_FAKE_FACTORY_9020',
])
assert.equal(paginationFinalLookup.total_count, 1)
const paginationFinalItem = paginationFinalLookup.items[0]
assert.equal(paginationFinalItem.id, paginationInitialItem.id)
assert.equal(paginationFinalItem.state, 'verified')
assert.deepEqual(paginationFinalItem.policy, paginationPersistedPolicy)
const paginationPostReplaySnapshot = await snapshot(paginationDemo)
const paginationPostReplayLookup = await lookupFactoryItems(paginationDemo, [
  'PVTI_FAKE_FACTORY_9020',
])
const paginationMatchingWorkItems = paginationPostReplayLookup.items.filter(
  (item) =>
    item.source_project_owner === 'acme' &&
    item.source_project_number === 7 &&
    item.source_project_item_id === 'PVTI_FAKE_FACTORY_9020' &&
    item.source_issue_number === paginationIssue.number,
)
assert.equal(paginationMatchingWorkItems.length, 1)
const paginationSourceMarkedMissionIds = new Set(
  paginationPostReplaySnapshot.snapshot.tasks
    .filter((task) => task.objective.includes(paginationSourceMarker))
    .map((task) => task.mission_id),
)
const paginationMissions = paginationPostReplaySnapshot.snapshot.missions.filter(
  (mission) =>
    mission.title === paginationExpectedMissionTitle ||
    paginationSourceMarkedMissionIds.has(mission.id),
)
assert.equal(paginationMissions.length, 1)
const paginationMissionIds = new Set(
  paginationMissions.map((mission) => mission.id),
)
const paginationTasks = paginationPostReplaySnapshot.snapshot.tasks.filter((task) =>
  paginationMissionIds.has(task.mission_id),
)
assert.equal(paginationTasks.length, 1)
assert.ok(
  paginationTasks.every((task) => task.objective.includes(paginationSourceMarker)),
)
const paginationTaskIds = new Set(paginationTasks.map((task) => task.id))
const paginationRuns = paginationPostReplaySnapshot.snapshot.runs.filter((run) =>
  paginationTaskIds.has(run.task_id),
)
assert.equal(paginationRuns.length, 1)

const timeoutDemo = await post('/api/demo/reset', {})
const timeoutIssue = {
  ...issue,
  id: 'I_FAKE_FACTORY_9011',
  number: 9011,
  title: 'Bound a stalled GitHub Project mutation',
  url: 'https://github.com/shyamsridhar123/ecorp/issues/9011',
  createdAt: '2026-09-01T14:11:00Z',
  updatedAt: '2026-09-01T14:11:00Z',
}
const timeoutState = {
  repository: 'shyamsridhar123/ecorp',
  project: recoveredFakeState.project,
  items: [
    {
      id: 'PVTI_FAKE_FACTORY_9011',
      status: 'Todo',
      content: {
        body: timeoutIssue.body,
        number: timeoutIssue.number,
        repository: 'shyamsridhar123/ecorp',
        title: timeoutIssue.title,
        type: 'Issue',
        url: timeoutIssue.url,
      },
    },
  ],
  issues: { '9011': timeoutIssue },
  item_edits: 0,
  item_edit_delay_ms: 3_000,
}
await writeFile(statePath, `${JSON.stringify(timeoutState, null, 2)}\n`)
let timeoutFailure
try {
  await runController(timeoutDemo, 9011, false, { githubTimeoutMs: 1_500 })
} catch (error) {
  timeoutFailure = error
}
assert.ok(timeoutFailure, 'stalled Project mutation did not time out')
const timeoutSnapshot = await snapshot(timeoutDemo)
const timeoutItem = timeoutSnapshot.snapshot.factory_work_items.find(
  (item) => item.source_issue_number === 9011,
)
assert.equal(timeoutItem.state, 'blocked')
assert.match(timeoutItem.failure_detail, /timed out after 1500 ms/i)
const timeoutTaskIds = new Set(
  timeoutSnapshot.snapshot.tasks
    .filter((task) => task.mission_id === timeoutItem.mission_id)
    .map((task) => task.id),
)
assert.equal(
  timeoutSnapshot.snapshot.runs.filter((run) => timeoutTaskIds.has(run.task_id)).length,
  0,
)
const timeoutFakeState = JSON.parse(await readFile(statePath, 'utf8'))
assert.equal(timeoutFakeState.items[0].status, 'Todo')
assert.equal(timeoutFakeState.item_edits, 0)

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

const verificationFailureDemo = await post('/api/demo/reset', {})
const verificationFailureIssue = {
  ...issue,
  id: 'I_FAKE_FACTORY_9005',
  number: 9005,
  title: 'Persist a factory verification failure',
  url: 'https://github.com/shyamsridhar123/ecorp/issues/9005',
  createdAt: '2026-09-01T14:04:00Z',
  updatedAt: '2026-09-01T14:04:00Z',
}
recoveredFakeState.items = [
  {
    id: 'PVTI_FAKE_FACTORY_9005',
    status: 'Todo',
    content: {
      body: verificationFailureIssue.body,
      number: verificationFailureIssue.number,
      repository: 'shyamsridhar123/ecorp',
      title: verificationFailureIssue.title,
      type: 'Issue',
      url: verificationFailureIssue.url,
    },
  },
]
recoveredFakeState.issues = { '9005': verificationFailureIssue }
recoveredFakeState.item_edits = 0
await writeFile(statePath, `${JSON.stringify(recoveredFakeState, null, 2)}\n`)
const verificationStarted = await runController(
  verificationFailureDemo,
  9005,
  false,
  { strategy: 'verification-failure' },
)
const verificationMission = await waitForMission(
  verificationFailureDemo,
  verificationStarted.mission_id,
)
assert.equal(verificationMission.mission.status, 'failed')
let verificationFailure
try {
  await runController(verificationFailureDemo, 9005, false, {
    strategy: 'verification-failure',
  })
} catch (error) {
  verificationFailure = error
}
assert.ok(verificationFailure, 'verification failure did not fail the controller command')
const verificationFailureState = await snapshot(verificationFailureDemo)
const verificationFailureItem =
  verificationFailureState.snapshot.factory_work_items.find(
    (item) => item.id === verificationStarted.factory_work_item_id,
  )
assert.equal(verificationFailureItem.state, 'verification_failed')
assert.match(
  verificationFailureItem.failure_detail,
  /verifier|verification|evidence|artifact/i,
)
const verificationFailureTasks = verificationFailureState.snapshot.tasks.filter(
  (task) => task.mission_id === verificationStarted.mission_id,
)
assert.ok(verificationFailureTasks.length > 0)
assert.ok(
  verificationFailureTasks.every(
    (task) =>
      task.status === 'verification_failed' && task.verification_status === 'failed',
  ),
)
const verificationFailureTaskIds = new Set(
  verificationFailureTasks.map((task) => task.id),
)
const verificationFailureRuns = verificationFailureState.snapshot.runs.filter((run) =>
  verificationFailureTaskIds.has(run.task_id),
)
assert.equal(verificationFailureRuns.length, 1)
assert.equal(verificationFailureRuns[0].verification_status, 'failed')

const approvalDemo = await post('/api/demo/reset', {})
const approvalIssue = {
  ...issue,
  id: 'I_FAKE_FACTORY_9012',
  number: 9012,
  title: 'Require substantive independent factory verification',
  url: 'https://github.com/shyamsridhar123/ecorp/issues/9012',
  createdAt: '2026-09-01T14:12:00Z',
  updatedAt: '2026-09-01T14:12:00Z',
}
const approvalState = {
  repository: 'shyamsridhar123/ecorp',
  project: recoveredFakeState.project,
  items: [
    {
      id: 'PVTI_FAKE_FACTORY_9012',
      status: 'Todo',
      content: {
        body: approvalIssue.body,
        number: approvalIssue.number,
        repository: 'shyamsridhar123/ecorp',
        title: approvalIssue.title,
        type: 'Issue',
        url: approvalIssue.url,
      },
    },
  ],
  issues: { '9012': approvalIssue },
  item_edits: 0,
}
await writeFile(statePath, `${JSON.stringify(approvalState, null, 2)}\n`)
const approvalStarted = await runController(approvalDemo, 9012, false, {
  strategy: 'independent-review',
})
const waitingApproval = await waitForApproval(approvalDemo, approvalStarted.mission_id)
const awaitingFactory = await runController(approvalDemo, 9012, false, {
  strategy: 'independent-review',
})
assert.equal(awaitingFactory.factory_state, 'awaiting_approval')
const requesterDecision = await fetch(
  `${server}/api/corps/${approvalDemo.corp_id}/runs/` +
    `${waitingApproval.run.id}/verification-decision`,
  {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      actor_id: approvalDemo.alice_actor_id,
      approved: true,
      note: 'Requester must not self-review factory evidence.',
    }),
  },
)
assert.equal(requesterDecision.status, 403)
await post(
  `/api/corps/${approvalDemo.corp_id}/runs/` +
    `${waitingApproval.run.id}/verification-decision`,
  {
    actor_id: approvalDemo.bob_actor_id,
    approved: true,
    note: 'Independent reviewer approved the factory evidence.',
  },
)
await waitForMission(approvalDemo, approvalStarted.mission_id)
const approvedFactory = await runController(approvalDemo, 9012, false, {
  strategy: 'independent-review',
})
assert.equal(approvedFactory.factory_state, 'verified')

const mismatchDemo = await post('/api/demo/reset', {})
const mismatchSourceRepository = await createSourceFixture('acme/widget')
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
    sourceRepositoryPath: mismatchSourceRepository,
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
      task.contract.source_base_ref === 'HEAD' &&
      task.contract.source_base_commit === sourceBaseCommit,
  ),
)
const mismatchTaskIds = new Set(mismatchTasks.map((task) => task.id))
assert.equal(
  mismatchState.snapshot.runs.filter((run) => mismatchTaskIds.has(run.task_id)).length,
  0,
)

const sourceChangeDemo = await post('/api/demo/reset', {})
const sourceChangedIssue = {
  ...issue,
  id: 'I_FAKE_FACTORY_9007',
  number: 9007,
  title: 'Block dispatch when the claimed issue changes',
  url: 'https://github.com/shyamsridhar123/ecorp/issues/9007',
  createdAt: '2026-09-01T14:06:00Z',
  updatedAt: '2026-09-01T14:06:00Z',
}
const sourceChangeState = {
  repository: 'shyamsridhar123/ecorp',
  project: recoveredFakeState.project,
  items: [
    {
      id: 'PVTI_FAKE_FACTORY_9007',
      status: 'Todo',
      content: {
        body: sourceChangedIssue.body,
        number: sourceChangedIssue.number,
        repository: 'shyamsridhar123/ecorp',
        title: sourceChangedIssue.title,
        type: 'Issue',
        url: sourceChangedIssue.url,
      },
    },
  ],
  issues: { '9007': sourceChangedIssue },
  item_edits: 0,
  item_list_calls: 0,
  item_list_mutation: {
    call: 3,
    issue_number: 9007,
    patch: { updatedAt: '2026-09-01T14:06:30Z' },
    remove_label: 'factory:ready',
  },
}
await writeFile(statePath, `${JSON.stringify(sourceChangeState, null, 2)}\n`)
let sourceChangeFailure
try {
  await runController(sourceChangeDemo, 9007)
} catch (error) {
  sourceChangeFailure = error
}
assert.ok(sourceChangeFailure, 'source revision change did not stop the controller')
const sourceChangeSnapshot = await snapshot(sourceChangeDemo)
const sourceChangeItem = sourceChangeSnapshot.snapshot.factory_work_items.find(
  (item) => item.source_issue_number === 9007,
)
assert.equal(sourceChangeItem.state, 'blocked')
assert.match(sourceChangeItem.failure_detail, /source revalidation failed before Project status/i)
const sourceChangeTaskIds = new Set(
  sourceChangeSnapshot.snapshot.tasks
    .filter((task) => task.mission_id === sourceChangeItem.mission_id)
    .map((task) => task.id),
)
assert.equal(
  sourceChangeSnapshot.snapshot.runs.filter((run) =>
    sourceChangeTaskIds.has(run.task_id),
  ).length,
  0,
)
const sourceChangeFakeState = JSON.parse(await readFile(statePath, 'utf8'))
assert.equal(sourceChangeFakeState.items[0].status, 'Todo')
assert.equal(sourceChangeFakeState.item_edits, 0)
assert.equal(sourceChangeFakeState.item_list_mutations_applied, 1)

const dependencyChangeDemo = await post('/api/demo/reset', {})
const dependencyChangedIssue = {
  ...issue,
  id: 'I_FAKE_FACTORY_9008',
  number: 9008,
  title: 'Block launch when a dependency reopens',
  body: `${issue.body}

## Dependencies

Blocked by #9009.
`,
  url: 'https://github.com/shyamsridhar123/ecorp/issues/9008',
  createdAt: '2026-09-01T14:07:00Z',
  updatedAt: '2026-09-01T14:07:00Z',
}
const reopenedDependency = {
  ...issue,
  id: 'I_FAKE_FACTORY_9009',
  number: 9009,
  title: 'Dependency that reopens before launch',
  body: 'Dependency fixture.',
  url: 'https://github.com/shyamsridhar123/ecorp/issues/9009',
  state: 'CLOSED',
  createdAt: '2026-09-01T14:08:00Z',
  updatedAt: '2026-09-01T14:08:00Z',
  labels: [],
}
const dependencyChangeState = {
  repository: 'shyamsridhar123/ecorp',
  project: recoveredFakeState.project,
  items: [
    {
      id: 'PVTI_FAKE_FACTORY_9008',
      status: 'Todo',
      content: {
        body: dependencyChangedIssue.body,
        number: dependencyChangedIssue.number,
        repository: 'shyamsridhar123/ecorp',
        title: dependencyChangedIssue.title,
        type: 'Issue',
        url: dependencyChangedIssue.url,
      },
    },
  ],
  issues: {
    '9008': dependencyChangedIssue,
    '9009': reopenedDependency,
  },
  item_edits: 0,
  item_list_calls: 0,
  item_list_mutation: {
    call: 5,
    issue_number: 9009,
    patch: {
      state: 'OPEN',
      updatedAt: '2026-09-01T14:08:30Z',
    },
  },
}
await writeFile(statePath, `${JSON.stringify(dependencyChangeState, null, 2)}\n`)
let dependencyChangeFailure
try {
  await runController(dependencyChangeDemo, 9008)
} catch (error) {
  dependencyChangeFailure = error
}
assert.ok(dependencyChangeFailure, 'reopened dependency did not stop mission launch')
const dependencyChangeSnapshot = await snapshot(dependencyChangeDemo)
const dependencyChangeItem = dependencyChangeSnapshot.snapshot.factory_work_items.find(
  (item) => item.source_issue_number === 9008,
)
assert.equal(dependencyChangeItem.state, 'blocked')
assert.match(dependencyChangeItem.failure_detail, /source revalidation failed before mission launch/i)
assert.match(dependencyChangeItem.failure_detail, /blocked by open issue #9009/i)
const dependencyChangeTaskIds = new Set(
  dependencyChangeSnapshot.snapshot.tasks
    .filter((task) => task.mission_id === dependencyChangeItem.mission_id)
    .map((task) => task.id),
)
assert.equal(
  dependencyChangeSnapshot.snapshot.runs.filter((run) =>
    dependencyChangeTaskIds.has(run.task_id),
  ).length,
  0,
)
const dependencyChangeFakeState = JSON.parse(await readFile(statePath, 'utf8'))
assert.equal(dependencyChangeFakeState.items[0].status, 'In Progress')
assert.equal(dependencyChangeFakeState.item_edits, 1)
assert.equal(dependencyChangeFakeState.item_list_mutations_applied, 1)

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
  claimed_commit_persisted_to_task_and_run: true,
  portable_deliverable_materialized: sourceDeliverables.length === 1,
  portable_deliverable_form: sourceDeliverables[0].form,
  factory_state: factoryItems[0].state,
  claim_token_absent_from_snapshot: true,
  mission_status: missions[0].status,
  run_status: runs[0].status,
  auto_merge: false,
  control_character_publication_base_policy_rejected:
    controlCharacterPolicyResponse.status,
  non_branch_publication_base_rejected_before_claim:
    invalidPublicationBaseRejected,
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
  pagination_safe_recovery: {
    issue_number: 9020,
    project_item_id: 'PVTI_FAKE_FACTORY_9020',
    historical_work_item_count: historicalWorkItemCount,
    snapshot_limit: truncatedPaginationSnapshot.snapshot.factory_work_items.length,
    target_absent_from_snapshot: true,
    selected_item_lookup_count: paginationLookup.total_count,
    lookup_input_count_bounded: oversizedPaginationLookup.status === 400,
    lookup_identifier_size_bounded: overlongPaginationLookup.status === 400,
    guest_lookup_rejected: guestPaginationLookup.status === 403,
    project_scoped_lookup: {
      configured_project_count: configuredProjectLookup.total_count,
      other_project_count: otherProjectLookup.total_count,
      conflicting_policy_excluded: true,
    },
    changed_lease_seconds: {
      initial: 300,
      recovery: 600,
      recovered_without_idempotency_conflict: true,
    },
    factory_work_item_id: paginationFinalItem.id,
    mission_id: paginationInitialItem.mission_id,
    run_ids: paginationRuns.map((run) => run.id),
    post_replay_counts: {
      work_items: paginationMatchingWorkItems.length,
      missions: paginationMissions.length,
      tasks: paginationTasks.length,
      runs: paginationRuns.length,
    },
    mission_identity: {
      expected_title: paginationExpectedMissionTitle,
      source_marker_matched: paginationSourceMarkedMissionIds.size === 1,
      counted_independently_of_work_item_link: true,
    },
    exactly_one_work_item: paginationMatchingWorkItems.length === 1,
    exactly_one_mission: paginationMissions.length === 1,
    exactly_one_task: paginationTasks.length === 1,
    exactly_one_run: paginationRuns.length === 1,
    persisted_policy_reused: true,
    recovered_without_policy_mismatch: true,
    factory_state: paginationFinalItem.state,
  },
  project_status_timeout: {
    issue_number: 9011,
    factory_work_item_id: timeoutItem.id,
    mission_id: timeoutItem.mission_id,
    factory_state: timeoutItem.state,
    project_status: timeoutFakeState.items[0].status,
    project_mutations: timeoutFakeState.item_edits,
    run_count: 0,
  },
  terminal_mission_failure: {
    issue_number: 9004,
    factory_work_item_id: failedItem.id,
    mission_id: failing.mission_id,
    mission_status: failedMission.mission.status,
    factory_state: failedItem.state,
    controller_returned_error: true,
  },
  verification_failure: {
    issue_number: 9005,
    factory_work_item_id: verificationFailureItem.id,
    mission_id: verificationStarted.mission_id,
    run_ids: verificationFailureRuns.map((run) => run.id),
    mission_status: verificationMission.mission.status,
    factory_state: verificationFailureItem.state,
    task_statuses: verificationFailureTasks.map((task) => task.status),
    controller_returned_error: true,
  },
  independent_verification: {
    issue_number: 9012,
    factory_work_item_id: approvalStarted.factory_work_item_id,
    mission_id: approvalStarted.mission_id,
    run_id: waitingApproval.run.id,
    requester_decision_status: requesterDecision.status,
    reviewer_actor_id: approvalDemo.bob_actor_id,
    waiting_factory_state: awaitingFactory.factory_state,
    final_factory_state: approvedFactory.factory_state,
  },
  repository_routing: {
    issue_number: 9006,
    factory_work_item_id: mismatchItem.id,
    mission_id: mismatchItem.mission_id,
    required_repository: 'acme/widget',
    required_base_ref: 'HEAD',
    required_base_commit: sourceBaseCommit,
    mismatched_runner_rejected: true,
    run_count: 0,
    factory_state: mismatchItem.state,
  },
  source_revalidation: {
    source_change_before_project_status: {
      issue_number: 9007,
      factory_work_item_id: sourceChangeItem.id,
      factory_state: sourceChangeItem.state,
      project_status: sourceChangeFakeState.items[0].status,
      project_mutations: sourceChangeFakeState.item_edits,
      run_count: 0,
    },
    dependency_reopened_before_launch: {
      issue_number: 9008,
      dependency_issue_number: 9009,
      factory_work_item_id: dependencyChangeItem.id,
      factory_state: dependencyChangeItem.state,
      project_status: dependencyChangeFakeState.items[0].status,
      project_mutations: dependencyChangeFakeState.item_edits,
      run_count: 0,
    },
  },
}
await writeFile(
  path.join(root, 'output', 'e2e-factory-controller.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
