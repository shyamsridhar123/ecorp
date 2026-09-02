import assert from 'node:assert/strict'
import { execFile as execFileCallback, spawn } from 'node:child_process'
import { chmod, mkdir, readdir, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { promisify } from 'node:util'

const execFile = promisify(execFileCallback)
const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const root = path.resolve(import.meta.dirname, '..')
const outputRoot = path.join(root, 'output', 'factory-commit-routing')
const wrongRepository = path.join(outputRoot, 'wrong-source')
const wrongWorkspace = path.join(outputRoot, 'wrong-runner')
const statePath = path.join(outputRoot, 'fake-github-state.json')
const fakeGithub = path.join(root, 'tools', 'fake_github_cli.mjs')
const cliBinary =
  process.env.CRONY_CLI_BINARY ??
  path.join(
    root,
    'target',
    'debug',
    process.platform === 'win32' ? 'crony-cli.exe' : 'crony-cli',
  )
const runnerBinary =
  process.env.CRONY_RUNNER_BINARY ??
  path.join(
    root,
    'target',
    'debug',
    process.platform === 'win32' ? 'crony-runner.exe' : 'crony-runner',
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

async function git(args, cwd = root) {
  return (
    await execFile('git', args, {
      cwd,
      encoding: 'utf8',
      windowsHide: true,
    })
  ).stdout.trim()
}

async function waitFor(predicate, message, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const value = await predicate()
    if (value) return value
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error(`timed out waiting for ${message}`)
}

function workspaceCapability(runner) {
  return runner.capabilities.find(
    (capability) => capability.name === 'workspace-isolation',
  )
}

async function runController(demo, issueNumber) {
  const args = [
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
  ]
  const { stdout } = await execFile(cliBinary, args, {
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

async function countFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true }).catch(() => [])
  let count = 0
  for (const entry of entries) {
    if (entry.isDirectory()) {
      count += await countFiles(path.join(directory, entry.name))
    } else {
      count += 1
    }
  }
  return count
}

await rm(outputRoot, { recursive: true, force: true })
await mkdir(outputRoot, { recursive: true })

const sourceBaseCommit = await git(['rev-parse', 'HEAD'])
const wrongBaseCommit = await git(['rev-parse', 'HEAD^'])
assert.notEqual(sourceBaseCommit, wrongBaseCommit)

await execFile('git', ['clone', '--quiet', '--no-hardlinks', root, wrongRepository], {
  cwd: root,
  windowsHide: true,
})
await execFile('git', ['checkout', '--quiet', '--detach', wrongBaseCommit], {
  cwd: wrongRepository,
  windowsHide: true,
})
await execFile(
  'git',
  [
    'remote',
    'set-url',
    'origin',
    'https://github.com/shyamsridhar123/ecorp.git',
  ],
  { cwd: wrongRepository, windowsHide: true },
)

const demo = await post('/api/demo/reset', {})
const enrollment = await post(
  `/api/corps/${demo.corp_id}/runners/enroll`,
  {
    actor_id: demo.alice_actor_id,
    runner_id: 'aaa-wrong-commit',
    expires_in_seconds: 600,
  },
)
const enrollmentFile = path.join(outputRoot, 'wrong-runner-enrollment.token')
const credentialFile = path.join(outputRoot, 'wrong-runner-credential.json')
await writeFile(enrollmentFile, enrollment.enrollment_token)
if (process.platform !== 'win32') await chmod(enrollmentFile, 0o600)

const wrongRunner = spawn(
  runnerBinary,
  [
    '--server-ws',
    new URL('/ws/runner', server).toString().replace(/^http/, 'ws'),
    '--runner-id',
    'aaa-wrong-commit',
    '--corp-id',
    demo.corp_id,
    '--credential-file',
    credentialFile,
    '--enrollment-token-file',
    enrollmentFile,
    '--workspace',
    wrongWorkspace,
    '--source-repository',
    wrongRepository,
    '--source-base-ref',
    'HEAD',
    '--fake-agent-script',
    path.join(root, 'scripts', 'fake-agent.mjs'),
  ],
  {
    cwd: root,
    windowsHide: true,
    stdio: ['ignore', 'pipe', 'pipe'],
  },
)

let wrongStderr = ''
wrongRunner.stderr.setEncoding('utf8')
wrongRunner.stderr.on('data', (chunk) => {
  wrongStderr += chunk
})

try {
  const runners = await waitFor(async () => {
    const state = await snapshot(demo)
    const connected = state.runners.filter((runner) => runner.connected)
    return connected.some((runner) => runner.id === 'aaa-wrong-commit') &&
      connected.some((runner) => runner.id === 'runner-local')
      ? connected
      : null
  }, 'both commit-distinct runners to connect')

  const rightRunner = runners.find((runner) => runner.id === 'runner-local')
  const wrongRunnerSummary = runners.find(
    (runner) => runner.id === 'aaa-wrong-commit',
  )
  const rightWorkspaceCapability = workspaceCapability(rightRunner)
  const wrongWorkspaceCapability = workspaceCapability(wrongRunnerSummary)
  assert.equal(rightWorkspaceCapability.source_repository, 'shyamsridhar123/ecorp')
  assert.equal(wrongWorkspaceCapability.source_repository, 'shyamsridhar123/ecorp')
  assert.equal(rightWorkspaceCapability.source_base_ref, 'HEAD')
  assert.equal(wrongWorkspaceCapability.source_base_ref, 'HEAD')
  assert.equal(rightWorkspaceCapability.source_base_commit, sourceBaseCommit)
  assert.equal(wrongWorkspaceCapability.source_base_commit, wrongBaseCommit)

  const issueNumber = 9066
  const issue = {
    id: 'I_FAKE_FACTORY_9066',
    number: issueNumber,
    title: 'Route factory work only to the immutable source commit',
    body: `## Outcome

Produce one verified write-capable result from the authorized source commit.

## Acceptance criteria

- [ ] the wrong commit receives no assignment
- [ ] the task and run retain the immutable source commit

## Dependencies

No blockers.
`,
    url: `https://github.com/shyamsridhar123/ecorp/issues/${issueNumber}`,
    state: 'OPEN',
    createdAt: '2026-09-01T20:05:00Z',
    updatedAt: '2026-09-01T20:05:00Z',
    labels: [{ name: 'factory:ready' }],
  }
  await writeFile(
    statePath,
    `${JSON.stringify(
      {
        repository: 'shyamsridhar123/ecorp',
        project: {
          id: 'PVT_FAKE_FACTORY_COMMIT',
          number: 7,
          owner: 'acme',
          title: 'Factory Commit Routing',
          status_field_id: 'PVTSSF_FAKE_STATUS',
          status_options: [
            { id: 'todo', name: 'Todo' },
            { id: 'in-progress', name: 'In Progress' },
            { id: 'done', name: 'Done' },
          ],
        },
        items: [
          {
            id: 'PVTI_FAKE_FACTORY_9066',
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
        item_edits: 0,
      },
      null,
      2,
    )}\n`,
  )

  const started = await runController(demo, issueNumber)
  const terminal = await waitFor(async () => {
    const state = await snapshot(demo)
    const mission = state.snapshot.missions.find(
      (candidate) => candidate.id === started.mission_id,
    )
    return mission && ['completed', 'failed', 'cancelled'].includes(mission.status)
      ? { state, mission }
      : null
  }, 'factory mission completion')
  assert.equal(terminal.mission.status, 'completed')
  const replay = await runController(demo, issueNumber)
  assert.equal(replay.factory_state, 'verified')

  const tasks = terminal.state.snapshot.tasks.filter(
    (task) => task.mission_id === started.mission_id,
  )
  assert.ok(tasks.length > 0)
  assert.ok(
    tasks.every(
      (task) =>
        task.contract.source_repository === 'shyamsridhar123/ecorp' &&
        task.contract.source_base_ref === 'HEAD' &&
        task.contract.source_base_commit === sourceBaseCommit,
    ),
  )
  const taskIds = new Set(tasks.map((task) => task.id))
  const runs = terminal.state.snapshot.runs.filter((run) =>
    taskIds.has(run.task_id),
  )
  assert.equal(runs.length, 1)
  assert.equal(runs[0].runner_id, 'runner-local')
  assert.equal(runs[0].source_base_commit, sourceBaseCommit)
  assert.notEqual(runs[0].runner_id, 'aaa-wrong-commit')
  assert.equal(await countFiles(path.join(wrongWorkspace, 'worktrees')), 0)

  const report = {
    checked_at: new Date().toISOString(),
    symbolic_ref: 'HEAD',
    repository: 'shyamsridhar123/ecorp',
    authorized_commit: sourceBaseCommit,
    wrong_commit: wrongBaseCommit,
    commits_differ: true,
    selected_runner: runs[0].runner_id,
    wrong_runner: 'aaa-wrong-commit',
    wrong_runner_received_run: false,
    wrong_runner_worktree_file_count: 0,
    task_source_commit: tasks[0].contract.source_base_commit,
    run_source_commit: runs[0].source_base_commit,
    workspace_base_commit: runs[0].workspace_base_commit,
    mission_status: terminal.mission.status,
    factory_state: replay.factory_state,
  }
  await writeFile(
    path.join(root, 'output', 'e2e-factory-commit-routing.json'),
    `${JSON.stringify(report, null, 2)}\n`,
  )
  console.log(JSON.stringify(report, null, 2))
} finally {
  wrongRunner.kill()
  await waitFor(
    async () => wrongRunner.exitCode !== null,
    'wrong runner process cleanup',
    5_000,
  ).catch(() => {})
  if (wrongRunner.exitCode === null && wrongRunner.signalCode === null) {
    console.error(`wrong runner did not exit cleanly: ${wrongStderr}`)
  }
  await rm(outputRoot, { recursive: true, force: true })
}
