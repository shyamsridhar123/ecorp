import assert from 'node:assert/strict'
import { execFile as execFileCallback, spawn } from 'node:child_process'
import { readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { promisify } from 'node:util'

const execFile = promisify(execFileCallback)
const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const root = path.resolve(import.meta.dirname, '..')
const cliBinary =
  process.env.CRONY_CLI_BINARY ??
  path.join(
    root,
    'target',
    'debug',
    process.platform === 'win32' ? 'crony-cli.exe' : 'crony-cli',
  )
const fakeGithub = path.join(root, 'tools', 'fake_github_cli.mjs')
const statePath = path.join(root, 'output', 'fake-github-legacy-source-state.json')
const migrationPath = path.join(
  root,
  'db',
  'migrations',
  '0022_run_source_identity.sql',
)

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
  throw new Error(`timed out waiting for mission ${missionId}`)
}

async function psql(sql) {
  const child = spawn(
    'docker',
    [
      'compose',
      '-f',
      path.join(root, 'deploy', 'compose', 'docker-compose.yml'),
      'exec',
      '-T',
      'postgres',
      'psql',
      '-v',
      'ON_ERROR_STOP=1',
      '-U',
      'crony',
      '-d',
      'crony',
    ],
    { cwd: root, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] },
  )
  let stdout = ''
  let stderr = ''
  child.stdout.setEncoding('utf8')
  child.stderr.setEncoding('utf8')
  child.stdout.on('data', (chunk) => {
    stdout += chunk
  })
  child.stderr.on('data', (chunk) => {
    stderr += chunk
  })
  child.stdin.end(sql)
  const code = await new Promise((resolve, reject) => {
    child.once('error', reject)
    child.once('exit', resolve)
  })
  if (code !== 0) {
    throw new Error(`psql failed (${code}): ${stderr}`)
  }
  return stdout
}

function policy(sourceBaseCommit) {
  return {
    schema_version: 1,
    source_of_truth: 'github_project',
    project_status: 'Todo',
    required_label: 'factory:ready',
    repository_allowlist: ['shyamsridhar123/ecorp'],
    source_base_ref: 'HEAD',
    ...(sourceBaseCommit
      ? {
          source_base_commit: sourceBaseCommit,
          source_commit_upgrade_required: false,
        }
      : {}),
    adapter_allowlist: ['fake-process'],
    strategy_allowlist: ['single'],
    model: null,
    reasoning_effort: null,
    write_scope: ['**'],
    allowed_tools: ['filesystem', 'shell'],
    prohibited_actions: [
      'modify files outside the assigned worktree',
      'use undeclared long-lived credentials',
      'merge or deploy without a separate current authorization',
    ],
    secret_ids: [],
    verification_required: true,
    budget_tokens: 20_000,
    budget_cost_microusd: 1_000_000,
    auto_merge: false,
  }
}

function claimRequest(demo, issueNumber, projectItemId, revision, sourceBaseCommit) {
  return {
    actor_id: demo.alice_actor_id,
    source_project_owner: 'acme',
    source_project_number: 7,
    source_project_item_id: projectItemId,
    source_repository_owner: 'shyamsridhar123',
    source_repository_name: 'ecorp',
    source_issue_number: issueNumber,
    source_issue_node_id: `I_LEGACY_${issueNumber}`,
    source_issue_url: `https://github.com/shyamsridhar123/ecorp/issues/${issueNumber}`,
    source_title: `Legacy source identity ${issueNumber}`,
    source_revision: revision,
    idempotency_key: `legacy-claim-${issueNumber}`,
    lease_seconds: 300,
    policy: policy(sourceBaseCommit),
  }
}

async function materializeAndRun(demo, claim, issueNumber) {
  const materialized = await post(
    `/api/corps/${demo.corp_id}/factory/work-items/${claim.work_item.id}/materialize`,
    {
      actor_id: demo.alice_actor_id,
      claim_token: claim.claim_token,
      expected_version: claim.work_item.version,
      idempotency_key: `legacy-materialize-${issueNumber}`,
      title: `Legacy source identity ${issueNumber}`,
      preferred_adapter: 'fake-process',
      strategy: 'single',
      budget_tokens: 20_000,
      budget_cost_microusd: 1_000_000,
      contract: {
        objective: 'Create deterministic migration evidence.',
        expected_output: 'A verified artifact.',
        acceptance_tests: ['artifact exists'],
        allowed_tools: ['filesystem', 'shell'],
        prohibited_actions: [
          'modify files outside the assigned worktree',
          'use undeclared long-lived credentials',
          'merge or deploy without a separate current authorization',
        ],
        references: [],
        write_scope: ['**'],
      },
    },
  )
  const launch = await post(
    `/api/corps/${demo.corp_id}/missions/${materialized.mission_id}/launch`,
    { requested_by: demo.alice_actor_id },
  )
  const terminal = await waitForMission(demo, materialized.mission_id)
  assert.equal(terminal.mission.status, 'completed')
  return { materialized, launch, terminal }
}

async function runController(demo, issueNumber, sourceBaseRef = 'HEAD') {
  const { stdout } = await execFile(
    cliBinary,
    [
      'factory',
      demo.corp_id,
      demo.alice_actor_id,
      '--owner',
      'acme',
      '--project-number',
      '7',
      '--repository',
      'shyamsridhar123/ecorp',
      '--source-repository-path',
      root,
      '--source-base-ref',
      sourceBaseRef,
      '--adapter',
      'fake-process',
      '--strategy',
      'single',
      '--budget-tokens',
      '20000',
      '--budget-cost-microusd',
      '1000000',
      '--issue',
      String(issueNumber),
      '--github-cli',
      process.execPath,
    ],
    {
      cwd: root,
      env: {
        ...process.env,
        ECORP_GITHUB_CLI_PREFIX_ARGS_JSON: JSON.stringify([fakeGithub]),
        ECORP_FAKE_GITHUB_STATE: statePath,
      },
      maxBuffer: 4 * 1024 * 1024,
      windowsHide: true,
    },
  )
  return JSON.parse(stdout)
}

const sourceBaseCommit = (
  await execFile('git', ['rev-parse', 'HEAD'], {
    cwd: root,
    encoding: 'utf8',
    windowsHide: true,
  })
).stdout.trim()
const demo = await post('/api/demo/reset', {})
const nonce = crypto.randomUUID().replaceAll('-', '').slice(0, 10)

const derivableIssue = 90_000 + Math.floor(Math.random() * 4_000)
const rejectedLegacyIssue = derivableIssue + 2
const rejectedLegacyProjectItemId = `PVTI_LEGACY_REJECTED_${nonce}`
let rejectedLegacyClaimError
try {
  await post(
    `/api/corps/${demo.corp_id}/factory/work-items/claim`,
    claimRequest(
      demo,
      rejectedLegacyIssue,
      rejectedLegacyProjectItemId,
      '2026-09-02T00:02:00Z',
      null,
    ),
  )
} catch (error) {
  rejectedLegacyClaimError = error
}
assert.match(
  rejectedLegacyClaimError?.message ?? '',
  /source_base_commit/,
  'new commit-less factory claim was accepted',
)
assert.equal(
  (await snapshot(demo)).snapshot.factory_work_items.some(
    (item) => item.source_project_item_id === rejectedLegacyProjectItemId,
  ),
  false,
)

const derivableProjectItemId = `PVTI_LEGACY_DERIVABLE_${nonce}`
const derivableClaim = await post(
  `/api/corps/${demo.corp_id}/factory/work-items/claim`,
  claimRequest(
    demo,
    derivableIssue,
    derivableProjectItemId,
    '2026-09-02T00:00:00Z',
    sourceBaseCommit,
  ),
)
const derivable = await materializeAndRun(demo, derivableClaim, derivableIssue)
const derivableRun = derivable.terminal.state.snapshot.runs.find(
  (run) => run.id === derivable.launch.run_id,
)
assert.equal(derivableRun.workspace_base_commit, sourceBaseCommit)

const upgradeIssue = derivableIssue + 1
const upgradeProjectItemId = `PVTI_LEGACY_UPGRADE_${nonce}`
const upgradeRevision = '2026-09-02T00:01:00Z'
const upgradeClaim = await post(
  `/api/corps/${demo.corp_id}/factory/work-items/claim`,
  claimRequest(
    demo,
    upgradeIssue,
    upgradeProjectItemId,
    upgradeRevision,
    sourceBaseCommit,
  ),
)

await psql(`
UPDATE factory_work_items
SET policy = policy - 'source_base_commit' - 'source_commit_upgrade_required'
WHERE id IN ('${derivableClaim.work_item.id}', '${upgradeClaim.work_item.id}');
UPDATE tasks
SET contract = contract - 'source_base_commit'
WHERE mission_id = '${derivable.materialized.mission_id}';
UPDATE runs
SET source_repository = NULL,
    source_base_ref = NULL,
    source_base_commit = NULL
WHERE id = '${derivable.launch.run_id}';
`)

await psql(await readFile(migrationPath, 'utf8'))

const migrated = await snapshot(demo)
const migratedItem = migrated.snapshot.factory_work_items.find(
  (item) => item.id === derivableClaim.work_item.id,
)
const migratedTask = migrated.snapshot.tasks.find(
  (task) => task.mission_id === derivable.materialized.mission_id,
)
const migratedRun = migrated.snapshot.runs.find(
  (run) => run.id === derivable.launch.run_id,
)
assert.equal(migratedItem.policy.source_base_commit, sourceBaseCommit)
assert.equal(migratedItem.policy.source_commit_upgrade_required, false)
assert.equal(migratedTask.contract.source_base_commit, sourceBaseCommit)
assert.equal(migratedRun.source_base_commit, sourceBaseCommit)

const pendingUpgrade = migrated.snapshot.factory_work_items.find(
  (item) => item.id === upgradeClaim.work_item.id,
)
assert.equal(pendingUpgrade.policy.source_base_commit, undefined)
assert.equal(pendingUpgrade.policy.source_commit_upgrade_required, true)

const issue = {
  id: `I_LEGACY_${upgradeIssue}`,
  number: upgradeIssue,
  title: `Legacy source identity ${upgradeIssue}`,
  body: `## Outcome

Recover an unmaterialized legacy factory claim through an explicit source pin.

## Acceptance criteria

- [ ] the source upgrade is authorized and audited
- [ ] the upgraded task and run use the resolved commit

## Dependencies

No blockers.
`,
  url: `https://github.com/shyamsridhar123/ecorp/issues/${upgradeIssue}`,
  state: 'OPEN',
  createdAt: upgradeRevision,
  updatedAt: upgradeRevision,
  labels: [{ name: 'factory:ready' }],
}
await writeFile(
  statePath,
  `${JSON.stringify(
    {
      repository: 'shyamsridhar123/ecorp',
      project: {
        id: 'PVT_FAKE_FACTORY_LEGACY',
        number: 7,
        owner: 'acme',
        title: 'Factory Legacy Source Upgrade',
        status_field_id: 'PVTSSF_FAKE_STATUS',
        status_options: [
          { id: 'todo', name: 'Todo' },
          { id: 'in-progress', name: 'In Progress' },
          { id: 'done', name: 'Done' },
        ],
      },
      items: [
        {
          id: upgradeProjectItemId,
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
      issues: { [String(upgradeIssue)]: issue },
      item_edits: 0,
    },
    null,
    2,
  )}\n`,
)

let mismatchedRefError
try {
  await runController(demo, upgradeIssue, 'main')
} catch (error) {
  mismatchedRefError = error
}
assert.match(
  mismatchedRefError?.message ?? '',
  /persisted policy requires HEAD, controller requested main/,
)
const afterMismatchedRef = await snapshot(demo)
const stillPendingUpgrade = afterMismatchedRef.snapshot.factory_work_items.find(
  (item) => item.id === upgradeClaim.work_item.id,
)
assert.equal(stillPendingUpgrade.policy.source_base_commit, undefined)
assert.equal(stillPendingUpgrade.policy.source_commit_upgrade_required, true)
assert.equal(
  afterMismatchedRef.snapshot.events.some(
    (event) =>
      event.aggregate_id === upgradeClaim.work_item.id &&
      event.type === 'factory.source_commit_pinned',
  ),
  false,
)

const upgraded = await runController(demo, upgradeIssue)
assert.equal(upgraded.legacy_source_commit_upgraded, true)
const upgradedTerminal = await waitForMission(demo, upgraded.mission_id)
assert.equal(upgradedTerminal.mission.status, 'completed')
const replay = await runController(demo, upgradeIssue)
assert.equal(replay.factory_state, 'verified')

const finalState = await snapshot(demo)
const finalItem = finalState.snapshot.factory_work_items.find(
  (item) => item.id === upgradeClaim.work_item.id,
)
const finalTask = finalState.snapshot.tasks.find(
  (task) => task.mission_id === upgraded.mission_id,
)
const finalRun = finalState.snapshot.runs.find(
  (run) => run.task_id === finalTask.id,
)
const upgradeEvents = finalState.snapshot.events.filter(
  (event) =>
    event.aggregate_id === upgradeClaim.work_item.id &&
    event.type === 'factory.source_commit_pinned',
)
assert.equal(finalItem.policy.source_base_commit, sourceBaseCommit)
assert.equal(finalItem.policy.source_commit_upgrade_required, false)
assert.equal(finalTask.contract.source_base_commit, sourceBaseCommit)
assert.equal(finalRun.source_base_commit, sourceBaseCommit)
assert.equal(upgradeEvents.length, 1)

const report = {
  checked_at: new Date().toISOString(),
  source_base_commit: sourceBaseCommit,
  intake_rejection: {
    commitless_claim_rejected: true,
    work_item_created: false,
  },
  derivable_legacy_run: {
    work_item_id: derivableClaim.work_item.id,
    mission_id: derivable.materialized.mission_id,
    run_id: derivable.launch.run_id,
    policy_backfilled: true,
    task_backfilled: true,
    run_backfilled: true,
  },
  unmaterialized_legacy_claim: {
    work_item_id: upgradeClaim.work_item.id,
    migration_marked_upgrade_required: true,
    mismatched_source_ref_rejected: true,
    persisted_source_ref: 'HEAD',
    explicit_upgrade_applied: upgraded.legacy_source_commit_upgraded,
    audit_event_count: upgradeEvents.length,
    mission_id: upgraded.mission_id,
    run_id: finalRun.id,
    factory_state: replay.factory_state,
  },
}
await writeFile(
  path.join(root, 'output', 'e2e-factory-legacy-source-upgrade.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
