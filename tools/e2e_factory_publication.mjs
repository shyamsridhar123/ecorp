import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import {
  execFile as execFileCallback,
  execFileSync,
  spawn,
} from 'node:child_process'
import {
  existsSync,
  openSync,
  readFileSync,
  writeFileSync,
} from 'node:fs'
import { readFile, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { promisify } from 'node:util'

const execFile = promisify(execFileCallback)
const root = path.resolve(import.meta.dirname, '..')
const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const databaseUrl =
  process.env.DATABASE_URL ?? 'postgres://crony:crony@127.0.0.1:54329/crony'
const binary =
  process.env.CRONY_CLI_BINARY ??
  path.join(
    root,
    'target',
    'debug',
    process.platform === 'win32' ? 'crony-cli.exe' : 'crony-cli',
  )
const fakeGithub = path.join(root, 'tools', 'fake_github_cli.mjs')
const nonce = crypto.randomUUID()
const statePath = path.join(root, 'output', `fake-github-publication-${nonce}.json`)
const remotePath = path.join(root, 'output', `fake-publication-remote-${nonce}.git`)
const reportPath = path.join(root, 'output', 'e2e-factory-publication.json')
const publisherToken = `publisher-secret-${crypto.randomUUID()}`
const authorizationId = crypto.randomUUID()
const sourceBaseCommit = execFileSync('git', ['rev-parse', 'HEAD'], {
  cwd: root,
  encoding: 'utf8',
  windowsHide: true,
}).trim()
let psqlMode

async function request(url, init) {
  const response = await fetch(`${server}${url}`, init)
  const body = await response
    .clone()
    .json()
    .catch(() => null)
  return { response, body }
}

async function requestOk(url, init) {
  const result = await request(url, init)
  if (!result.response.ok) {
    throw new Error(
      `${init?.method ?? 'GET'} ${url}: ${result.response.status} ${JSON.stringify(result.body)}`,
    )
  }
  return result.body
}

function post(url, body) {
  return request(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
}

function postOk(url, body) {
  return requestOk(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
}

function snapshot(demo, actorId = demo.alice_actor_id) {
  return requestOk(
    `/api/corps/${demo.corp_id}/snapshot?actor_id=${actorId}`,
  )
}

async function waitForMission(demo, missionId, timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const state = await snapshot(demo)
    const mission = state.snapshot.missions.find((item) => item.id === missionId)
    if (mission && ['completed', 'failed', 'cancelled'].includes(mission.status)) {
      return { state, mission }
    }
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error(`timed out waiting for publication mission ${missionId}`)
}

async function runController(demo, issueNumber) {
  const { stdout } = await execFile(
    binary,
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
      'HEAD',
      '--publication-base-ref',
      'main',
      '--adapter',
      'fake-process',
      '--strategy',
      'single',
      '--budget-tokens',
      '20000',
      '--budget-cost-microusd',
      '1000000',
      '--lease-seconds',
      '300',
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

async function runPublisher(
  demo,
  workItemId,
  {
    crashAfter,
    idempotencyKey,
    expectCrash = false,
  } = {},
) {
  const args = [
    'factory-publish',
    demo.corp_id,
    demo.alice_actor_id,
    workItemId,
    '--authorization-id',
    authorizationId,
    '--authorization-reason',
    'Publication E2E authorizes review-only branch and pull request creation.',
    '--publisher-id',
    'trusted-publication-e2e',
    '--lease-seconds',
    '5',
    '--wait-seconds',
    '30',
    '--github-cli',
    process.execPath,
  ]
  if (idempotencyKey) args.push('--idempotency-key', idempotencyKey)
  try {
    const { stdout, stderr } = await execFile(binary, args, {
      cwd: root,
      env: {
        ...process.env,
        GH_TOKEN: publisherToken,
        ECORP_FAKE_GITHUB_EXPECT_TOKEN: publisherToken,
        ECORP_GITHUB_CLI_PREFIX_ARGS_JSON: JSON.stringify([fakeGithub]),
        ECORP_FAKE_GITHUB_STATE: statePath,
        ECORP_PUBLICATION_TEST_REMOTE_URL: remotePath,
        ...(crashAfter
          ? { ECORP_PUBLICATION_TEST_CRASH_AFTER: crashAfter }
          : {}),
      },
      maxBuffer: 8 * 1024 * 1024,
      windowsHide: true,
    })
    if (expectCrash) {
      throw new Error(`publisher did not crash at ${crashAfter}`)
    }
    assert.equal(stdout.includes(publisherToken), false)
    assert.equal(stderr.includes(publisherToken), false)
    return JSON.parse(stdout)
  } catch (error) {
    if (!expectCrash || error.code !== 86) throw error
    assert.equal(String(error.stdout ?? '').includes(publisherToken), false)
    assert.equal(String(error.stderr ?? '').includes(publisherToken), false)
    return { crashed: true, stage: crashAfter }
  }
}

async function restartLocalServer() {
  const pidPath =
    process.env.CRONY_TEST_SERVER_PID_FILE ??
    path.join(root, 'output', 'local-pids.json')
  if (!existsSync(pidPath)) {
    throw new Error(`test-owned server PID file does not exist: ${pidPath}`)
  }
  const jsonPidFile = pidPath.endsWith('.json')
  const pidState = jsonPidFile
    ? JSON.parse(readFileSync(pidPath, 'utf8'))
    : { server: Number(readFileSync(pidPath, 'utf8').trim()) }
  const serverPid = Number(pidState.server)
  if (!Number.isSafeInteger(serverPid) || serverPid <= 0) {
    throw new Error(`test-owned server PID is invalid: ${serverPid}`)
  }
  process.kill(serverPid, 0)
  process.kill(serverPid)
  await new Promise((resolve) => setTimeout(resolve, 500))

  const serverUrl = new URL(server)
  const serverBinary =
    process.env.CRONY_TEST_SERVER_BINARY ??
    path.join(
      root,
      'target',
      'debug',
      process.platform === 'win32' ? 'crony-server.exe' : 'crony-server',
    )
  const logDir =
    process.env.CRONY_TEST_SERVER_LOG_DIR ?? path.dirname(path.resolve(pidPath))
  const stdout = openSync(
    path.join(logDir, 'publication-server-restart.stdout.log'),
    'a',
  )
  const stderr = openSync(
    path.join(logDir, 'publication-server-restart.stderr.log'),
    'a',
  )
  const child = spawn(
    serverBinary,
    [
      '--bind',
      `${serverUrl.hostname}:${serverUrl.port}`,
      '--database-url',
      databaseUrl,
    ],
    {
      cwd: root,
      detached: true,
      windowsHide: true,
      stdio: ['ignore', stdout, stderr],
    },
  )
  if (jsonPidFile) {
    writeFileSync(
      pidPath,
      `${JSON.stringify({ ...pidState, server: child.pid }, null, 2)}\n`,
    )
  } else {
    writeFileSync(pidPath, `${child.pid}\n`)
  }
  child.unref()

  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    try {
      const health = await fetch(`${server}/health`).then((response) =>
        response.json(),
      )
      if (health.status === 'ok' && health.runners >= 1) return child.pid
    } catch {
      // The test-owned server is restarting and the runner is reconnecting.
    }
    await new Promise((resolve) => setTimeout(resolve, 200))
  }
  throw new Error('server or runner did not recover after publication restart')
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
  return {
    command: 'docker',
    args: ['exec', '-i', container, 'psql', '-U', 'crony', '-d', 'crony'],
  }
}

function sqlLiteral(value) {
  return `'${String(value).replaceAll("'", "''")}'`
}

async function setFakeState(patch) {
  const state = JSON.parse(await readFile(statePath, 'utf8'))
  Object.assign(state, patch)
  await writeFile(statePath, `${JSON.stringify(state, null, 2)}\n`)
}

async function publicationState(demo, workItemId) {
  const state = await snapshot(demo)
  return {
    state,
    publication: state.snapshot.pull_request_publications.find(
      (item) => item.factory_work_item_id === workItemId,
    ),
  }
}

await rm(statePath, { force: true })
await rm(remotePath, { recursive: true, force: true })
await execFile('git', ['clone', '--quiet', '--bare', root, remotePath], {
  cwd: root,
  windowsHide: true,
})
await execFile(
  'git',
  ['--git-dir', remotePath, 'update-ref', 'refs/heads/main', sourceBaseCommit],
  { cwd: root, windowsHide: true },
)

const issueNumber = 9101
const issue = {
  id: `I_PUBLICATION_${nonce}`,
  number: issueNumber,
  title: 'Publish a verified factory result exactly once',
  body: `## Outcome

Produce one verified commit/branch deliverable and publish it for review.

## Acceptance criteria

- [ ] verified commit exists
- [ ] pull request exists exactly once
- [ ] Project enters review only after the pull request exists

## Dependencies

No blockers.
`,
  url: `https://github.com/shyamsridhar123/ecorp/issues/${issueNumber}`,
  state: 'OPEN',
  createdAt: '2026-09-02T02:00:00Z',
  updatedAt: '2026-09-02T02:00:00Z',
  labels: [{ name: 'factory:ready' }],
}
await writeFile(
  statePath,
  `${JSON.stringify(
    {
      repository: 'shyamsridhar123/ecorp',
      project: {
        id: 'PVT_PUBLICATION',
        number: 7,
        owner: 'acme',
        title: 'Factory Publication Test',
        status_field_id: 'PVTSSF_PUBLICATION_STATUS',
        status_options: [
          { id: 'todo', name: 'Todo' },
          { id: 'in-progress', name: 'In Progress' },
          { id: 'in-review', name: 'In Review' },
          { id: 'done', name: 'Done' },
        ],
      },
      items: [
        {
          id: `PVTI_PUBLICATION_${nonce}`,
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
      issues: { [String(issueNumber)]: issue },
      pull_requests: [],
      next_pr_number: 41,
      item_edits: 0,
      pr_create_calls: 0,
      pr_create_delay_ms: 300,
      effect_log: [],
    },
    null,
    2,
  )}\n`,
)

const demo = await postOk('/api/demo/reset', {})
const firstController = await runController(demo, issueNumber)
assert.equal(firstController.factory_state, 'running')
const completed = await waitForMission(demo, firstController.mission_id)
assert.equal(completed.mission.status, 'completed')
const verifiedController = await runController(demo, issueNumber)
assert.equal(verifiedController.factory_state, 'verified')
assert.equal(
  verifiedController.factory_work_item_id,
  firstController.factory_work_item_id,
)

const verifiedSnapshot = await snapshot(demo)
const workItem = verifiedSnapshot.snapshot.factory_work_items.find(
  (item) => item.id === firstController.factory_work_item_id,
)
assert.ok(workItem)
const taskIds = new Set(
  verifiedSnapshot.snapshot.tasks
    .filter((task) => task.mission_id === firstController.mission_id)
    .map((task) => task.id),
)
const source = verifiedSnapshot.snapshot.source_deliverables.find(
  (deliverable) =>
    taskIds.has(deliverable.task_id) &&
    deliverable.form === 'commit_branch' &&
    deliverable.integration_state === 'ready_for_review',
)
assert.ok(source)
assert.ok(source.head_commit)
const branch = `ecorp/issue-${issueNumber}-${source.head_commit.slice(0, 12)}`
const body = `## ECorp verified factory deliverable

Implements ${issue.url}

- Factory work item: \`${workItem.id}\`
- Mission: \`${firstController.mission_id}\`
- Verified commit: \`${source.head_commit}\`
- Deliverable digest: \`${source.sha256}\`

Closes #${issueNumber}

Auto-merge, merge, and deployment are not authorized by this publication.`
const effectKey = `github-pr:${workItem.id}:${source.id}:shyamsridhar123/ecorp:${branch}`
const publicationRequest = {
  actor_id: demo.alice_actor_id,
  source_deliverable_id: source.id,
  target_repository: 'shyamsridhar123/ecorp',
  base_ref: 'main',
  branch,
  title: issue.title,
  body,
  authorization_id: authorizationId,
  authorization_reason:
    'Publication E2E authorizes review-only branch and pull request creation.',
  effect_key: effectKey,
  idempotency_key: `${effectKey}:negative`,
  publisher_id: 'trusted-publication-e2e',
  lease_seconds: 5,
}
const publicationPath =
  `/api/corps/${demo.corp_id}/factory/work-items/${workItem.id}/publication`

const roleRejected = await post(publicationPath, {
  ...publicationRequest,
  actor_id: demo.bob_actor_id,
  idempotency_key: `${effectKey}:member-rejected`,
})
assert.equal(roleRejected.response.status, 403)
const corpRejected = await post(
  `/api/corps/${crypto.randomUUID()}/factory/work-items/${workItem.id}/publication`,
  {
    ...publicationRequest,
    idempotency_key: `${effectKey}:corp-rejected`,
  },
)
assert.equal(corpRejected.response.status, 403)

await psql(
  `UPDATE factory_work_items SET policy = jsonb_set(policy, '{publication,allowed}', 'false'::jsonb) WHERE id = ${sqlLiteral(workItem.id)}::uuid;`,
)
const policyRejected = await post(publicationPath, {
  ...publicationRequest,
  idempotency_key: `${effectKey}:policy-rejected`,
})
assert.equal(policyRejected.response.status, 400)
assert.match(policyRejected.body.error, /does not authorize/)
await psql(
  `UPDATE factory_work_items SET policy = jsonb_set(policy, '{publication,allowed}', 'true'::jsonb) WHERE id = ${sqlLiteral(workItem.id)}::uuid;`,
)

const run = verifiedSnapshot.snapshot.runs.find((item) =>
  taskIds.has(item.task_id),
)
assert.ok(run)
const breakerBefore = await psql(
  `SELECT breaker_stage FROM runs WHERE id = ${sqlLiteral(run.id)}::uuid;`,
)
await psql(
  `UPDATE runs SET breaker_stage = 'suspend' WHERE id = ${sqlLiteral(run.id)}::uuid;`,
)
const breakerRejected = await post(publicationPath, {
  ...publicationRequest,
  idempotency_key: `${effectKey}:breaker-rejected`,
})
assert.equal(breakerRejected.response.status, 400)
assert.match(breakerRejected.body.error, /circuit breaker/)
await psql(
  `UPDATE runs SET breaker_stage = ${sqlLiteral(breakerBefore)} WHERE id = ${sqlLiteral(run.id)}::uuid;`,
)

const usageBefore = (
  await psql(
    `SELECT input_tokens || ',' || output_tokens || ',' || budget_tokens_limit FROM runs WHERE id = ${sqlLiteral(run.id)}::uuid;`,
  )
)
  .split(',')
  .map(Number)
await psql(
  `UPDATE runs SET input_tokens = budget_tokens_limit, output_tokens = 0 WHERE id = ${sqlLiteral(run.id)}::uuid;`,
)
const budgetRejected = await post(publicationPath, {
  ...publicationRequest,
  idempotency_key: `${effectKey}:budget-rejected`,
})
assert.equal(budgetRejected.response.status, 400)
assert.match(budgetRejected.body.error, /budget|hard breaker/)
await psql(
  `UPDATE runs SET input_tokens = ${usageBefore[0]}, output_tokens = ${usageBefore[1]} WHERE id = ${sqlLiteral(run.id)}::uuid;`,
)

await runPublisher(demo, workItem.id, {
  crashAfter: 'after_branch_remote',
  expectCrash: true,
})
const remoteBranchAfterCrash = (
  await execFile(
    'git',
    ['--git-dir', remotePath, 'rev-parse', `refs/heads/${branch}`],
    { cwd: root, windowsHide: true },
  )
).stdout.trim()
assert.equal(remoteBranchAfterCrash, source.head_commit)
let publicationSnapshot = await publicationState(demo, workItem.id)
assert.equal(publicationSnapshot.publication.state, 'publishing')
assert.equal(publicationSnapshot.publication.branch_pushed_at, null)

const restartedServerPid = await restartLocalServer()
await new Promise((resolve) => setTimeout(resolve, 5_500))
await setFakeState({ fail_pr_create_after_success: true })
await runPublisher(demo, workItem.id, {
  crashAfter: 'after_pull_request_checkpoint',
  expectCrash: true,
})
publicationSnapshot = await publicationState(demo, workItem.id)
assert.equal(publicationSnapshot.publication.state, 'pull_request_created')
assert.equal(publicationSnapshot.publication.pull_request_number, 41)
let fakeState = JSON.parse(await readFile(statePath, 'utf8'))
assert.equal(fakeState.pull_requests.length, 1)
assert.equal(fakeState.pr_create_calls, 1)
assert.equal(fakeState.pr_create_external_success_failures, 1)
assert.equal(
  fakeState.items.find((item) => item.id === workItem.source_project_item_id)
    .status,
  'In Progress',
)

await new Promise((resolve) => setTimeout(resolve, 5_500))
await runPublisher(demo, workItem.id, {
  crashAfter: 'after_project_remote',
  expectCrash: true,
})
publicationSnapshot = await publicationState(demo, workItem.id)
assert.equal(publicationSnapshot.publication.state, 'pull_request_created')
fakeState = JSON.parse(await readFile(statePath, 'utf8'))
assert.equal(
  fakeState.items.find((item) => item.id === workItem.source_project_item_id)
    .status,
  'In Review',
)

await new Promise((resolve) => setTimeout(resolve, 5_500))
const concurrent = await Promise.all([
  runPublisher(demo, workItem.id, {
    idempotencyKey: `${effectKey}:concurrent-a`,
  }),
  runPublisher(demo, workItem.id, {
    idempotencyKey: `${effectKey}:concurrent-b`,
  }),
])
assert.equal(concurrent[0].publication.pull_request_number, 41)
assert.equal(concurrent[1].publication.pull_request_number, 41)
assert.equal(concurrent[0].publication.id, concurrent[1].publication.id)

const finalSnapshot = await snapshot(demo)
const publication = finalSnapshot.snapshot.pull_request_publications.find(
  (item) => item.factory_work_item_id === workItem.id,
)
assert.equal(publication.state, 'published')
assert.equal(publication.pull_request_number, 41)
assert.equal(publication.pull_request_url, fakeState.pull_requests[0].url)
assert.equal(publication.project_status_after, 'In Review')
assert.equal(publication.auto_merge_enabled, false)
assert.equal(publication.merge_authorized, false)
assert.equal(publication.deployment_authorized, false)
assert.equal(
  finalSnapshot.snapshot.factory_work_items.find(
    (item) => item.id === workItem.id,
  ).state,
  'published',
)
assert.equal(
  finalSnapshot.snapshot.source_deliverables.find(
    (item) => item.id === source.id,
  ).integration_state,
  'published',
)
const attempts = finalSnapshot.snapshot.pull_request_publication_attempts
  .filter((attempt) => attempt.publication_id === publication.id)
  .sort((left, right) => left.attempt - right.attempt)
assert.ok(attempts.length >= 4)
assert.equal(attempts.at(-1).state, 'published')
assert.ok(attempts.some((attempt) => attempt.state === 'abandoned'))
assert.equal(
  finalSnapshot.snapshot.pull_request_publications.filter(
    (item) => item.factory_work_item_id === workItem.id,
  ).length,
  1,
)

fakeState = JSON.parse(await readFile(statePath, 'utf8'))
assert.equal(fakeState.pull_requests.length, 1)
assert.equal(fakeState.pr_create_calls, 1)
const reviewEffectIndex = fakeState.effect_log.findIndex(
  (effect) => effect.kind === 'project_status' && effect.status === 'In Review',
)
const pullRequestEffectIndex = fakeState.effect_log.findIndex(
  (effect) => effect.kind === 'pull_request_created',
)
assert.ok(pullRequestEffectIndex >= 0)
assert.ok(reviewEffectIndex > pullRequestEffectIndex)
assert.ok(
  fakeState.effect_log[reviewEffectIndex].pull_request_count >= 1,
  'Project entered review before the pull request existed',
)

const remoteBranches = (
  await execFile(
    'git',
    [
      '--git-dir',
      remotePath,
      'for-each-ref',
      '--format=%(refname):%(objectname)',
      `refs/heads/${branch}`,
    ],
    { cwd: root, windowsHide: true },
  )
).stdout
  .trim()
  .split(/\r?\n/)
  .filter(Boolean)
assert.deepEqual(remoteBranches, [
  `refs/heads/${branch}:${source.head_commit}`,
])

const durableText = [
  JSON.stringify(finalSnapshot),
  JSON.stringify(fakeState),
  await psql(
    `SELECT jsonb_build_object(
      'publications', COALESCE(jsonb_agg(to_jsonb(publication)), '[]'::jsonb)
    )::text
    FROM pull_request_publications publication
    WHERE publication.corp_id = ${sqlLiteral(demo.corp_id)}::uuid;`,
  ),
].join('\n')
assert.equal(
  durableText.includes(publisherToken),
  false,
  'trusted publisher credential leaked into durable or shared state',
)
assert.equal(
  finalSnapshot.snapshot.events.some((event) =>
    JSON.stringify(event).includes(publisherToken),
  ),
  false,
)
assert.ok(
  finalSnapshot.snapshot.events.some(
    (event) =>
      event.type === 'factory.publication_completed' &&
      event.aggregate_id === publication.id,
  ),
)

const report = {
  checked_at: new Date().toISOString(),
  factory_work_item_id: workItem.id,
  mission_id: firstController.mission_id,
  source_deliverable_id: source.id,
  source_sha256: source.sha256,
  verification_sha256: source.verification_sha256,
  commit_sha: source.head_commit,
  branch,
  publication_id: publication.id,
  publication_attempts: attempts.length,
  pull_request_number: publication.pull_request_number,
  pull_request_url: publication.pull_request_url,
  pull_request_create_calls: fakeState.pr_create_calls,
  remote_branch_count: remoteBranches.length,
  project_status: publication.project_status_after,
  project_after_pull_request: reviewEffectIndex > pullRequestEffectIndex,
  server_restart_pid: restartedServerPid,
  policy_rejection: policyRejected.response.status,
  role_rejection: roleRejected.response.status,
  corp_rejection: corpRejected.response.status,
  budget_rejection: budgetRejected.response.status,
  breaker_rejection: breakerRejected.response.status,
  credential_non_disclosure: !durableText.includes(publisherToken),
  auto_merge: publication.auto_merge_enabled,
  merge_authorized: publication.merge_authorized,
  deployment_authorized: publication.deployment_authorized,
}
await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`)
console.log(JSON.stringify(report, null, 2))
