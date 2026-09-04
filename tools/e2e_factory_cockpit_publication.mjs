import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import { execFile as execFileCallback, spawn } from 'node:child_process'
import {
  existsSync,
  openSync,
  readFileSync,
  writeFileSync,
} from 'node:fs'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { promisify } from 'node:util'

const execFile = promisify(execFileCallback)
const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const socketBase = server.replace(/^http/, 'ws')
const databaseUrl =
  process.env.DATABASE_URL ?? 'postgres://crony:crony@127.0.0.1:54329/crony'
const root = path.resolve(import.meta.dirname, '..')
const sourceRepository = process.env.ECORP_TEST_SOURCE_REPOSITORY
if (!sourceRepository) {
  throw new Error('ECORP_TEST_SOURCE_REPOSITORY must identify the isolated source clone')
}
const cli =
  process.env.CRONY_CLI_BINARY ??
  path.join(
    root,
    'target',
    'debug',
    process.platform === 'win32' ? 'crony-cli.exe' : 'crony-cli',
  )
const serverBinary =
  process.env.CRONY_TEST_SERVER_BINARY ??
  path.join(
    root,
    'target',
    'debug',
    process.platform === 'win32' ? 'crony-server.exe' : 'crony-server',
  )
const fakeGithub = path.join(root, 'tools', 'fake_github_cli.mjs')
const nonce = crypto.randomUUID()
const authorizationId = crypto.randomUUID()
const artifactRoot = path.resolve(
  process.env.ECORP_TEST_ARTIFACT_ROOT ??
    path.join(os.tmpdir(), `ecorp-cockpit-publication-${nonce}`),
)
await mkdir(artifactRoot, { recursive: true })
const statePath = path.join(artifactRoot, 'fake-github-state.json')
const remotePath = path.join(artifactRoot, 'target-remote.git')
const credentialPath = path.join(artifactRoot, 'publisher.credential')

async function request(url, init) {
  const response = await fetch(`${server}${url}`, init)
  const body = response.status === 204 ? null : await response.json()
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

async function snapshot(demo, actorId = demo.alice_actor_id) {
  return requestOk(
    `/api/corps/${demo.corp_id}/snapshot?actor_id=${actorId}`,
  )
}

async function waitFor(demo, predicate, description, timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const state = await snapshot(demo)
    const value = predicate(state)
    if (value) return { state, value }
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error(`timed out waiting for ${description}`)
}

async function openBrowserClient(demo, actorId, afterSeq = 0) {
  const events = []
  let cursor = afterSeq
  let readyResolve
  let readyReject
  let closedResolve
  const ready = new Promise((resolve, reject) => {
    readyResolve = resolve
    readyReject = reject
  })
  const closed = new Promise((resolve) => {
    closedResolve = resolve
  })
  const socket = new WebSocket(
    `${socketBase}/ws/corps/${demo.corp_id}?actor_id=${actorId}&after_seq=${afterSeq}`,
  )
  const timeout = setTimeout(() => {
    socket.close()
    readyReject(new Error(`browser ${actorId} replay timed out`))
  }, 10_000)
  socket.onerror = () => {
    clearTimeout(timeout)
    readyReject(new Error(`browser ${actorId} websocket failed`))
  }
  socket.onclose = () => closedResolve()
  socket.onmessage = (message) => {
    const payload = JSON.parse(message.data)
    if (payload.type === 'event') {
      events.push(payload.event)
      cursor = Math.max(cursor, payload.event.seq)
    } else if (payload.type === 'ready') {
      clearTimeout(timeout)
      cursor = Math.max(cursor, payload.replayed_through)
      readyResolve()
    }
  }
  await ready
  return { actorId, events, socket, closed, cursor: () => cursor }
}

function assertUniqueReplay(events, afterSeq) {
  const sequences = events.map((event) => event.seq)
  assert.equal(new Set(sequences).size, sequences.length)
  assert.ok(sequences.every((sequence) => sequence > afterSeq))
  assert.ok(
    sequences.every(
      (sequence, index) => index === 0 || sequence > sequences[index - 1],
    ),
  )
}

async function runController(demo, issueNumber) {
  const { stdout } = await execFile(
    cli,
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
      sourceRepository,
      '--source-base-ref',
      'HEAD',
      '--publication-base-ref',
      'HEAD',
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
      '--write-scope',
      '**',
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

async function enrollPublisher(demo, publisherId) {
  const enrollment = await postOk(
    `/api/corps/${demo.corp_id}/factory/publication-publishers/credentials`,
    {
      actor_id: demo.alice_actor_id,
      publisher_id: publisherId,
      expires_in_seconds: 3600,
    },
  )
  await writeFile(credentialPath, enrollment.credential)
  return enrollment
}

async function runPublisher(
  demo,
  workItemId,
  sourceDeliverableId,
  branch,
  idempotencyKey,
  publisherId,
  githubToken,
  crashAfter,
) {
  const args = [
    'factory-publish',
    demo.corp_id,
    demo.alice_actor_id,
    workItemId,
    '--authorization-id',
    authorizationId,
    '--authorization-reason',
    'Focused dark-factory acceptance authorizes one review-only branch and pull request.',
    '--publisher-id',
    publisherId,
    '--publisher-credential-file',
    credentialPath,
    '--source-deliverable-id',
    sourceDeliverableId,
    '--repository',
    'shyamsridhar123/ecorp',
    '--branch',
    branch,
    '--idempotency-key',
    idempotencyKey,
    '--lease-seconds',
    '5',
    '--wait-seconds',
    '60',
    '--github-cli',
    process.execPath,
  ]
  try {
    const { stdout } = await execFile(cli, args, {
      cwd: root,
      env: {
        ...process.env,
        GH_TOKEN: githubToken,
        ECORP_FAKE_GITHUB_EXPECT_TOKEN: githubToken,
        ECORP_GITHUB_CLI_PREFIX_ARGS_JSON: JSON.stringify([fakeGithub]),
        ECORP_FAKE_GITHUB_STATE: statePath,
        ECORP_PUBLICATION_TEST_REMOTE_URL: remotePath,
        ECORP_PUBLICATION_EFFECT_LEASE_SECONDS: '30',
        ECORP_GITHUB_COMMAND_TIMEOUT_MS: '2000',
        ECORP_SOURCE_GIT_COMMAND_TIMEOUT_MS: '5000',
        ...(crashAfter
          ? { ECORP_PUBLICATION_TEST_CRASH_AFTER: crashAfter }
          : {}),
      },
      maxBuffer: 8 * 1024 * 1024,
      windowsHide: true,
    })
    if (crashAfter) {
      throw new Error(`publisher did not crash at ${crashAfter}`)
    }
    return JSON.parse(stdout)
  } catch (error) {
    if (crashAfter && error.code === 86) {
      return { crashed: true, stage: crashAfter }
    }
    throw error
  }
}

async function restartLocalServer() {
  const pidPath = process.env.CRONY_TEST_SERVER_PID_FILE
  if (!pidPath || !existsSync(pidPath)) {
    throw new Error('CRONY_TEST_SERVER_PID_FILE must identify the test-owned server')
  }
  const pidState = JSON.parse(readFileSync(pidPath, 'utf8'))
  const serverPid = Number(pidState.server)
  if (!Number.isSafeInteger(serverPid) || serverPid <= 0) {
    throw new Error(`test-owned server PID is invalid: ${serverPid}`)
  }
  process.kill(serverPid, 0)
  process.kill(serverPid)
  await new Promise((resolve) => setTimeout(resolve, 500))

  const serverUrl = new URL(server)
  const logDir = path.dirname(path.resolve(pidPath))
  const stdout = openSync(path.join(logDir, 'publication-restart.stdout.log'), 'a')
  const stderr = openSync(path.join(logDir, 'publication-restart.stderr.log'), 'a')
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
  writeFileSync(
    pidPath,
    `${JSON.stringify({ ...pidState, server: child.pid }, null, 2)}\n`,
  )
  child.unref()

  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    try {
      const health = await fetch(`${server}/health`).then((response) =>
        response.json(),
      )
      if (health.status === 'ok' && health.runners >= 1) return child.pid
    } catch {
      // Server and runner are reconnecting.
    }
    await new Promise((resolve) => setTimeout(resolve, 200))
  }
  throw new Error('server and runner did not recover during publication')
}

const sourceBaseCommit = (
  await execFile('git', ['rev-parse', 'HEAD'], {
    cwd: sourceRepository,
    windowsHide: true,
  })
).stdout.trim()
await execFile('git', ['clone', '--quiet', '--bare', sourceRepository, remotePath], {
  cwd: artifactRoot,
  windowsHide: true,
})
await execFile(
  'git',
  ['--git-dir', remotePath, 'update-ref', 'refs/heads/main', sourceBaseCommit],
  { cwd: artifactRoot, windowsHide: true },
)
await execFile(
  'git',
  ['--git-dir', remotePath, 'symbolic-ref', 'HEAD', 'refs/heads/main'],
  { cwd: artifactRoot, windowsHide: true },
)

const issueNumber = 9201
const issue = {
  id: `I_COCKPIT_PUBLICATION_${nonce}`,
  number: issueNumber,
  title: 'Publish one dark-factory cockpit result exactly once',
  body: `## Outcome

Create one verified commit and publish one reviewable pull request.

## Acceptance criteria

- [ ] one work item, mission, task, and run
- [ ] one verified commit branch
- [ ] one pull request with auto-merge disabled
- [ ] Project moves to In Review after the pull request exists

## Dependencies

No blockers.
`,
  url: `https://github.com/shyamsridhar123/ecorp/issues/${issueNumber}`,
  state: 'OPEN',
  createdAt: '2026-09-04T20:00:00Z',
  updatedAt: '2026-09-04T20:00:00Z',
  labels: [{ name: 'factory:ready' }],
}
await writeFile(
  statePath,
  `${JSON.stringify(
    {
      repository: 'shyamsridhar123/ecorp',
      canonical_repository: 'ShyamSridhar123/ECorp',
      project: {
        id: 'PVT_COCKPIT_PUBLICATION',
        number: 7,
        owner: 'acme',
        title: 'Cockpit Publication Test',
        status_field_id: 'PVTSSF_COCKPIT_PUBLICATION',
        status_options: [
          { id: 'todo', name: 'Todo' },
          { id: 'in-progress', name: 'In Progress' },
          { id: 'in-review', name: 'In Review' },
          { id: 'done', name: 'Done' },
        ],
      },
      items: [
        {
          id: `PVTI_COCKPIT_PUBLICATION_${nonce}`,
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
      next_pr_number: 51,
      item_edits: 0,
      pr_create_calls: 0,
      effect_log: [],
      branch_heads: {},
    },
    null,
    2,
  )}\n`,
)

const demo = await postOk('/api/demo/reset', {})
const aliceBrowser = await openBrowserClient(demo, demo.alice_actor_id)
const bobBrowser = await openBrowserClient(demo, demo.bob_actor_id)

const firstController = await runController(demo, issueNumber)
assert.equal(firstController.factory_state, 'running')
const completed = await waitFor(
  demo,
  (state) => {
    const mission = state.snapshot.missions.find(
      (candidate) => candidate.id === firstController.mission_id,
    )
    return mission?.status === 'completed' ? mission : null
  },
  'factory mission completion',
)
const verifiedController = await runController(demo, issueNumber)
assert.equal(verifiedController.factory_state, 'verified')
assert.equal(
  verifiedController.factory_work_item_id,
  firstController.factory_work_item_id,
)

const verified = await snapshot(demo)
const workItem = verified.snapshot.factory_work_items.find(
  (item) => item.id === firstController.factory_work_item_id,
)
assert.ok(workItem)
const tasks = verified.snapshot.tasks.filter(
  (task) => task.mission_id === firstController.mission_id,
)
assert.equal(tasks.length, 1)
const runs = verified.snapshot.runs.filter((run) => run.task_id === tasks[0].id)
assert.equal(runs.length, 1)
const source = verified.snapshot.source_deliverables.find(
  (deliverable) =>
    deliverable.task_id === tasks[0].id &&
    deliverable.form === 'commit_branch' &&
    deliverable.integration_state === 'ready_for_review',
)
assert.ok(source?.head_commit)

const commentBody = `Alice linked publication acceptance ${nonce}`
const comment = await postOk(
  `/api/corps/${demo.corp_id}/rooms/${demo.room_id}/messages`,
  {
    actor_id: demo.alice_actor_id,
    body: commentBody,
    reply_to_id: null,
    mentions: [demo.bob_actor_id],
    link: { kind: 'mission', id: firstController.mission_id },
    idempotency_key: crypto.randomUUID(),
  },
)
assert.equal(comment.replayed, false)

const branch = `ecorp/cockpit-publication-${source.head_commit.slice(0, 12)}`
const fakeState = JSON.parse(await readFile(statePath, 'utf8'))
fakeState.branch_heads = { [branch]: source.head_commit }
await writeFile(statePath, `${JSON.stringify(fakeState, null, 2)}\n`)

const publisherId = 'trusted-cockpit-publication'
await enrollPublisher(demo, publisherId)
const githubToken = `fake-github-${crypto.randomUUID()}`
const publicationKey =
  `cockpit-publication:${workItem.id}:${source.id}:${branch}`
const firstPublish = await runPublisher(
  demo,
  workItem.id,
  source.id,
  branch,
  publicationKey,
  publisherId,
  githubToken,
  'after_pull_request_remote',
)
assert.equal(firstPublish.crashed, true)

const aliceCursor = aliceBrowser.cursor()
const bobCursor = bobBrowser.cursor()
const restartedServerPid = await restartLocalServer()
await Promise.all([aliceBrowser.closed, bobBrowser.closed])
const aliceReconnected = await openBrowserClient(
  demo,
  demo.alice_actor_id,
  aliceCursor,
)
const bobReconnected = await openBrowserClient(
  demo,
  demo.bob_actor_id,
  bobCursor,
)
assertUniqueReplay(aliceReconnected.events, aliceCursor)
assertUniqueReplay(bobReconnected.events, bobCursor)

await new Promise((resolve) => setTimeout(resolve, 5_500))
const recovered = await runPublisher(
  demo,
  workItem.id,
  source.id,
  branch,
  publicationKey,
  publisherId,
  githubToken,
)
assert.equal(recovered.publication.state, 'published')
const duplicateResults = await Promise.all([
  runPublisher(
    demo,
    workItem.id,
    source.id,
    branch,
    `${publicationKey}:duplicate-a`,
    publisherId,
    githubToken,
  ),
  runPublisher(
    demo,
    workItem.id,
    source.id,
    branch,
    `${publicationKey}:duplicate-b`,
    publisherId,
    githubToken,
  ),
])
assert.ok(
  duplicateResults.every(
    (result) => result.publication.id === recovered.publication.id,
  ),
)

const final = await snapshot(demo)
const publication = final.snapshot.pull_request_publications.find(
  (candidate) => candidate.factory_work_item_id === workItem.id,
)
assert.equal(publication.state, 'published')
assert.equal(publication.pull_request_number, 51)
assert.equal(publication.auto_merge_enabled, false)
assert.equal(publication.merge_authorized, false)
assert.equal(publication.deployment_authorized, false)
assert.equal(
  final.snapshot.factory_work_items.filter(
    (item) => item.source_project_item_id === workItem.source_project_item_id,
  ).length,
  1,
)
assert.equal(
  final.snapshot.missions.filter(
    (mission) => mission.id === firstController.mission_id,
  ).length,
  1,
)
assert.equal(
  final.snapshot.runs.filter((run) => run.task_id === tasks[0].id).length,
  1,
)
assert.equal(
  final.snapshot.room_messages.filter(
    (message) => message.body === commentBody,
  ).length,
  1,
)
assert.equal(
  final.snapshot.pull_request_publications.filter(
    (candidate) => candidate.factory_work_item_id === workItem.id,
  ).length,
  1,
)

const finalFakeState = JSON.parse(await readFile(statePath, 'utf8'))
assert.equal(finalFakeState.pr_create_calls, 1)
assert.equal(finalFakeState.pull_requests.length, 1)
assert.equal(finalFakeState.pull_requests[0].autoMergeRequest, null)
assert.equal(
  finalFakeState.items.find(
    (item) => item.id === workItem.source_project_item_id,
  ).status,
  'In Review',
)
const pullRequestEffect = finalFakeState.effect_log.findIndex(
  (effect) => effect.kind === 'pull_request_created',
)
const projectEffect = finalFakeState.effect_log.findIndex(
  (effect) =>
    effect.kind === 'project_status' && effect.status === 'In Review',
)
assert.ok(pullRequestEffect >= 0)
assert.ok(projectEffect > pullRequestEffect)
const remoteMain = (
  await execFile(
    'git',
    ['--git-dir', remotePath, 'rev-parse', 'refs/heads/main'],
    { cwd: artifactRoot, windowsHide: true },
  )
).stdout.trim()
const remoteBranch = (
  await execFile(
    'git',
    ['--git-dir', remotePath, 'rev-parse', `refs/heads/${branch}`],
    { cwd: artifactRoot, windowsHide: true },
  )
).stdout.trim()
assert.equal(remoteMain, sourceBaseCommit)
assert.equal(remoteBranch, source.head_commit)

for (const client of [aliceReconnected, bobReconnected]) {
  const completions = client.events.filter(
    (event) =>
      event.type === 'factory.publication_completed' &&
      event.aggregate_id === publication.id,
  )
  assert.ok(completions.length <= 1)
  client.socket.close()
}

const report = {
  checked_at: new Date().toISOString(),
  server_restart_pid: restartedServerPid,
  source_repository: 'shyamsridhar123/ecorp',
  source_base_commit: sourceBaseCommit,
  work_item_id: workItem.id,
  mission_id: firstController.mission_id,
  run_id: runs[0].id,
  source_deliverable_id: source.id,
  publication_id: publication.id,
  pull_request_number: publication.pull_request_number,
  pull_request_url: publication.pull_request_url,
  pull_request_create_calls: finalFakeState.pr_create_calls,
  project_status: publication.project_status_after,
  project_after_pull_request: projectEffect > pullRequestEffect,
  duplicate_work_items: 0,
  duplicate_missions: 0,
  duplicate_runs: 0,
  duplicate_publications: 0,
  auto_merge_enabled: publication.auto_merge_enabled,
  merge_authorized: publication.merge_authorized,
  deployment_authorized: publication.deployment_authorized,
  alice_replay_events: aliceReconnected.events.length,
  bob_replay_events: bobReconnected.events.length,
  final_state: publication.state,
}
await writeFile(
  path.join(root, 'output', 'e2e-factory-cockpit-publication.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
