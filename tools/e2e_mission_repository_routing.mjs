import assert from 'node:assert/strict'
import { execFile as execFileCallback, spawn } from 'node:child_process'
import { chmod, mkdir, readdir, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { promisify } from 'node:util'

const execFile = promisify(execFileCallback)
const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const root = path.resolve(import.meta.dirname, '..')
const outputRoot = path.resolve(
  process.env.CRONY_TEST_OUTPUT_ROOT ??
    path.join(root, 'output', 'mission-repository-routing'),
)
const runnerBinary =
  process.env.CRONY_RUNNER_BINARY ??
  path.join(
    root,
    'target',
    'debug',
    process.platform === 'win32' ? 'crony-runner.exe' : 'crony-runner',
  )

async function requestRaw(url, init) {
  const response = await fetch(`${server}${url}`, init)
  const body = await response.json()
  return { response, body }
}

async function request(url, init) {
  const result = await requestRaw(url, init)
  if (!result.response.ok) {
    throw new Error(`${result.response.status}: ${JSON.stringify(result.body)}`)
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

function postRaw(url, body) {
  return requestRaw(url, {
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

async function waitFor(predicate, label, timeoutMs = 45_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const result = await predicate()
    if (result) return result
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error(`timed out waiting for ${label}`)
}

async function git(args, cwd) {
  return (
    await execFile('git', args, {
      cwd,
      encoding: 'utf8',
      windowsHide: true,
    })
  ).stdout.trim()
}

async function createRepository(directory, identity, marker) {
  await mkdir(directory, { recursive: true })
  await git(['init', '--initial-branch=main'], directory)
  await writeFile(path.join(directory, 'README.md'), `# ${marker}\n`)
  await git(['add', 'README.md'], directory)
  await git(
    [
      '-c',
      'user.name=ECorp Routing Test',
      '-c',
      'user.email=routing@example.invalid',
      'commit',
      '-m',
      'Initialize routing fixture',
    ],
    directory,
  )
  await git(
    ['remote', 'add', 'origin', `https://github.com/${identity}.git`],
    directory,
  )
  return git(['rev-parse', 'HEAD'], directory)
}

async function enrollAndStartRunner(demo, config) {
  const enrollment = await post(
    `/api/corps/${demo.corp_id}/runners/enroll`,
    {
      actor_id: demo.alice_actor_id,
      runner_id: config.id,
      expires_in_seconds: 600,
    },
  )
  const enrollmentFile = path.join(outputRoot, `${config.id}.enrollment.token`)
  const credentialFile = path.join(outputRoot, `${config.id}.credential.json`)
  await writeFile(enrollmentFile, enrollment.enrollment_token)
  if (process.platform !== 'win32') await chmod(enrollmentFile, 0o600)
  const child = spawn(
    runnerBinary,
    [
      '--server-ws',
      new URL('/ws/runner', server).toString().replace(/^http/u, 'ws'),
      '--runner-id',
      config.id,
      '--corp-id',
      demo.corp_id,
      '--credential-file',
      credentialFile,
      '--enrollment-token-file',
      enrollmentFile,
      '--workspace',
      config.workspace,
      '--source-repository',
      config.repository,
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
  let stderr = ''
  child.stderr.setEncoding('utf8')
  child.stderr.on('data', (chunk) => {
    stderr += chunk
  })
  return { child, stderr: () => stderr }
}

function workspaceCapability(runner) {
  return runner.capabilities.find(
    (capability) => capability.name === 'workspace-isolation',
  )
}

async function countFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true }).catch(() => [])
  let count = 0
  for (const entry of entries) {
    count += entry.isDirectory()
      ? await countFiles(path.join(directory, entry.name))
      : 1
  }
  return count
}

function processExists(pid) {
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

async function stopRunner(record, label) {
  if (processExists(record.child.pid)) record.child.kill('SIGKILL')
  await waitFor(
    async () => !processExists(record.child.pid),
    `${label} runner cleanup`,
    10_000,
  )
}

const rootStatusBefore = await git(['status', '--porcelain'], root)
await rm(outputRoot, { recursive: true, force: true })
await mkdir(outputRoot, { recursive: true })

const alphaRepository = path.join(outputRoot, 'alpha-source')
const betaRepository = path.join(outputRoot, 'beta-source')
const alphaWorkspace = path.join(outputRoot, 'alpha-workspace')
const betaWorkspace = path.join(outputRoot, 'beta-workspace')
const alphaIdentity = 'acme/ecorp-dogfood-alpha'
const betaIdentity = 'acme/ecorp-dogfood-beta'
const alphaCommit = await createRepository(
  alphaRepository,
  alphaIdentity,
  'Alpha source',
)
const betaCommit = await createRepository(
  betaRepository,
  betaIdentity,
  'Beta source',
)
assert.notEqual(alphaIdentity, betaIdentity)

const demo = await post('/api/demo/reset', {})
const alpha = await enrollAndStartRunner(demo, {
  id: 'runner-repository-alpha',
  repository: alphaRepository,
  workspace: alphaWorkspace,
})
const beta = await enrollAndStartRunner(demo, {
  id: 'runner-repository-beta',
  repository: betaRepository,
  workspace: betaWorkspace,
})

try {
  const runners = await waitFor(async () => {
    const state = await snapshot(demo)
    const connected = state.runners.filter((runner) => runner.connected)
    return connected.some((runner) => runner.id === 'runner-repository-alpha') &&
      connected.some((runner) => runner.id === 'runner-repository-beta')
      ? connected
      : null
  }, 'both repository-specific runners')
  const alphaSummary = runners.find(
    (runner) => runner.id === 'runner-repository-alpha',
  )
  const betaSummary = runners.find(
    (runner) => runner.id === 'runner-repository-beta',
  )
  const alphaSource = workspaceCapability(alphaSummary)
  const betaSource = workspaceCapability(betaSummary)
  assert.equal(alphaSource.source_repository, alphaIdentity)
  assert.equal(alphaSource.source_base_ref, 'HEAD')
  assert.equal(alphaSource.source_base_commit, alphaCommit)
  assert.equal(betaSource.source_repository, betaIdentity)
  assert.equal(betaSource.source_base_commit, betaCommit)

  const mission = await post(`/api/corps/${demo.corp_id}/missions`, {
    requested_by: demo.alice_actor_id,
    preferred_adapter: 'fake-process',
    strategy: 'single',
    title: 'Produce repository-routing evidence in the selected target.',
    source: {
      repository: alphaSource.source_repository,
      base_ref: alphaSource.source_base_ref,
      base_commit: alphaSource.source_base_commit,
    },
  })
  const launch = await post(
    `/api/corps/${demo.corp_id}/missions/${mission.mission_id}/launch`,
    { requested_by: demo.alice_actor_id },
  )
  const terminal = await waitFor(async () => {
    const state = await snapshot(demo)
    const run = state.snapshot.runs.find(
      (candidate) => candidate.id === launch.run_id,
    )
    return run && ['completed', 'failed', 'cancelled'].includes(run.status)
      ? { state, run }
      : null
  }, 'selected repository mission')
  assert.equal(terminal.run.status, 'completed', terminal.run.summary)
  assert.equal(terminal.run.runner_id, 'runner-repository-alpha')
  assert.equal(terminal.run.source_repository, alphaIdentity)
  assert.equal(terminal.run.source_base_ref, 'HEAD')
  assert.equal(terminal.run.source_base_commit, alphaCommit)
  const tasks = terminal.state.snapshot.tasks.filter(
    (task) => task.mission_id === mission.mission_id,
  )
  assert.equal(tasks.length, 1)
  assert.equal(tasks[0].contract.source_repository, alphaIdentity)
  assert.equal(tasks[0].contract.source_base_ref, 'HEAD')
  assert.equal(tasks[0].contract.source_base_commit, alphaCommit)
  assert.equal(await countFiles(path.join(betaWorkspace, 'worktrees')), 0)

  const missionCount = terminal.state.snapshot.missions.length
  const mismatch = await postRaw(`/api/corps/${demo.corp_id}/missions`, {
    requested_by: demo.alice_actor_id,
    preferred_adapter: 'fake-process',
    strategy: 'single',
    title: 'Reject a stale repository selection.',
    source: {
      repository: alphaIdentity,
      base_ref: 'HEAD',
      base_commit: 'f'.repeat(40),
    },
  })
  assert.equal(mismatch.response.status, 400)
  assert.match(mismatch.body.error, /not available from a connected runner/u)
  const afterMismatch = await snapshot(demo)
  assert.equal(afterMismatch.snapshot.missions.length, missionCount)
  assert.equal(await git(['status', '--porcelain'], alphaRepository), '')
  assert.equal(await git(['status', '--porcelain'], betaRepository), '')
  assert.equal(await git(['status', '--porcelain'], root), rootStatusBefore)

  const report = {
    checked_at: new Date().toISOString(),
    selected_repository: alphaIdentity,
    selected_commit: alphaCommit,
    selected_runner: terminal.run.runner_id,
    unselected_repository: betaIdentity,
    unselected_commit: betaCommit,
    unselected_runner_received_run: false,
    unselected_workspace_file_count: 0,
    task_source_persisted: true,
    run_source_persisted: true,
    stale_selection_status: mismatch.response.status,
    stale_selection_created_mission: false,
    ecorp_checkout_unchanged: true,
  }
  await writeFile(
    path.join(outputRoot, 'result.json'),
    `${JSON.stringify(report, null, 2)}\n`,
  )
  console.log(JSON.stringify(report, null, 2))
} finally {
  await Promise.all([
    stopRunner(alpha, 'alpha').catch((error) => {
      throw new Error(`${error.message}: ${alpha.stderr()}`)
    }),
    stopRunner(beta, 'beta').catch((error) => {
      throw new Error(`${error.message}: ${beta.stderr()}`)
    }),
  ])
}
