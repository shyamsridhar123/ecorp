import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { readFile, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { spawnSync } from 'node:child_process'

const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const root = path.resolve(import.meta.dirname, '..')
const managedRoot = path.join(root, 'output', 'runner', 'worktrees')

function git(args, cwd = root) {
  const result = spawnSync('git', args, { cwd, encoding: 'utf8' })
  if (result.status !== 0) {
    throw new Error(`git ${args.join(' ')} failed: ${result.stderr}`)
  }
  return result.stdout
}

async function request(url, init) {
  const response = await fetch(`${server}${url}`, init)
  const body = await response.json()
  if (!response.ok) {
    throw new Error(`${init?.method ?? 'GET'} ${url} failed: ${JSON.stringify(body)}`)
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

function settled(run) {
  return (
    ['completed', 'failed', 'cancelled', 'lost'].includes(run.status) &&
    ['preserved', 'removed'].includes(run.workspace_disposition)
  )
}

async function waitForRuns(demo, runIds, timeoutMs = 45_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const state = await snapshot(demo)
    const runs = runIds.map((id) =>
      state.snapshot.runs.find((candidate) => candidate.id === id),
    )
    if (runs.every((run) => run && settled(run))) return { runs, state }
    await new Promise((resolve) => setTimeout(resolve, 50))
  }
  throw new Error(`timed out waiting for runs ${runIds.join(', ')}`)
}

async function verifyArtifact(run) {
  assert.ok(run.artifact_path)
  const bytes = await readFile(run.artifact_path)
  assert.equal(
    createHash('sha256').update(bytes).digest('hex'),
    run.artifact_sha256,
  )
}

function assertManagedPath(workspace) {
  const relative = path.relative(managedRoot, workspace)
  assert.ok(relative && !relative.startsWith('..') && !path.isAbsolute(relative))
}

const sourceHeadBefore = git(['rev-parse', 'HEAD']).trim()
const sourceStatusBefore = git(['status', '--porcelain=v1', '--untracked-files=all'])
const demo = await post('/api/demo/reset', {})

async function createMission(preferredAdapter, title) {
  const mission = await post(`/api/corps/${demo.corp_id}/missions`, {
    requested_by: demo.alice_actor_id,
    preferred_adapter: preferredAdapter,
    title,
  })
  const launch = await post(
    `/api/corps/${demo.corp_id}/missions/${mission.mission_id}/launch`,
    { requested_by: demo.alice_actor_id },
  )
  return { mission, launch }
}

const [fake, codex] = await Promise.all([
  createMission('fake-process', '[slow] parallel fake worktree validation'),
  createMission('codex', '[slow] parallel Codex worktree validation'),
])
const { runs } = await waitForRuns(demo, [
  fake.launch.run_id,
  codex.launch.run_id,
])
const fakeRun = runs.find((run) => run.id === fake.launch.run_id)
const codexRun = runs.find((run) => run.id === codex.launch.run_id)

assert.equal(fakeRun.status, 'completed')
assert.equal(codexRun.status, 'completed')
assert.equal(fakeRun.workspace_disposition, 'preserved')
assert.equal(codexRun.workspace_disposition, 'preserved')
assert.ok(fakeRun.workspace_path)
assert.ok(codexRun.workspace_path)
assert.ok(fakeRun.workspace_branch)
assert.ok(codexRun.workspace_branch)
assert.notEqual(fakeRun.workspace_path, codexRun.workspace_path)
assert.notEqual(fakeRun.workspace_branch, codexRun.workspace_branch)
assertManagedPath(fakeRun.workspace_path)
assertManagedPath(codexRun.workspace_path)
assert.equal((await stat(path.join(fakeRun.workspace_path, '.git'))).isFile(), true)
assert.equal((await stat(path.join(codexRun.workspace_path, '.git'))).isFile(), true)
assert.equal(
  await readFile(path.join(fakeRun.workspace_path, 'result.md'), 'utf8').then(Boolean),
  true,
)
assert.equal(
  await readFile(path.join(codexRun.workspace_path, 'base.txt'), 'utf8'),
  'base\n',
)
assert.equal(
  await stat(path.join(fakeRun.workspace_path, 'base.txt'))
    .then(() => true)
    .catch(() => false),
  false,
)
assert.equal(
  await stat(path.join(codexRun.workspace_path, 'result.md'))
    .then(() => true)
    .catch(() => false),
  false,
)
await Promise.all([verifyArtifact(fakeRun), verifyArtifact(codexRun)])

const worktreeList = git(['worktree', 'list', '--porcelain'])
assert.ok(worktreeList.includes(fakeRun.workspace_path.replaceAll('\\', '/')))
assert.ok(worktreeList.includes(codexRun.workspace_path.replaceAll('\\', '/')))
assert.equal(
  spawnSync(
    'git',
    ['show-ref', '--verify', '--quiet', `refs/heads/${fakeRun.workspace_branch}`],
    { cwd: root },
  ).status,
  0,
)
assert.equal(
  spawnSync(
    'git',
    ['show-ref', '--verify', '--quiet', `refs/heads/${codexRun.workspace_branch}`],
    { cwd: root },
  ).status,
  0,
)

const clean = await createMission(
  'fake-process',
  '[clean-worktree] produce evidence outside the task checkout',
)
const cleanSettled = await waitForRuns(demo, [clean.launch.run_id])
const cleanRun = cleanSettled.runs[0]
assert.equal(cleanRun.status, 'completed')
assert.equal(cleanRun.workspace_disposition, 'removed')
assert.ok(cleanRun.workspace_path)
assert.ok(cleanRun.workspace_branch)
assert.equal(
  await stat(cleanRun.workspace_path)
    .then(() => true)
    .catch(() => false),
  false,
)
assert.equal(
  spawnSync(
    'git',
    ['show-ref', '--verify', '--quiet', `refs/heads/${cleanRun.workspace_branch}`],
    { cwd: root },
  ).status,
  1,
)
await verifyArtifact(cleanRun)

const ignored = await createMission(
  'fake-process',
  '[ignored-worktree] preserve ignored task output',
)
const ignoredSettled = await waitForRuns(demo, [ignored.launch.run_id])
const ignoredRun = ignoredSettled.runs[0]
assert.equal(ignoredRun.status, 'completed')
assert.equal(ignoredRun.workspace_disposition, 'preserved')
assert.ok(ignoredRun.workspace_detail?.includes('ignored files'))
assert.equal(
  await readFile(path.join(ignoredRun.workspace_path, 'valuable.log'), 'utf8'),
  'ignored but valuable\n',
)
await verifyArtifact(ignoredRun)

const sourceHeadAfter = git(['rev-parse', 'HEAD']).trim()
const sourceStatusAfter = git(['status', '--porcelain=v1', '--untracked-files=all'])
assert.equal(sourceHeadAfter, sourceHeadBefore)
assert.equal(sourceStatusAfter, sourceStatusBefore)

const report = {
  checked_at: new Date().toISOString(),
  source_head_unchanged: sourceHeadAfter === sourceHeadBefore,
  source_status_unchanged: sourceStatusAfter === sourceStatusBefore,
  fake: {
    run_id: fakeRun.id,
    task_id: fake.mission.task_id,
    workspace_path: fakeRun.workspace_path,
    workspace_branch: fakeRun.workspace_branch,
    disposition: fakeRun.workspace_disposition,
  },
  codex: {
    run_id: codexRun.id,
    task_id: codex.mission.task_id,
    workspace_path: codexRun.workspace_path,
    workspace_branch: codexRun.workspace_branch,
    disposition: codexRun.workspace_disposition,
  },
  clean_cleanup: {
    run_id: cleanRun.id,
    workspace_path: cleanRun.workspace_path,
    workspace_branch: cleanRun.workspace_branch,
    disposition: cleanRun.workspace_disposition,
    workspace_removed: true,
  },
  ignored_cleanup: {
    run_id: ignoredRun.id,
    workspace_path: ignoredRun.workspace_path,
    workspace_branch: ignoredRun.workspace_branch,
    disposition: ignoredRun.workspace_disposition,
    detail: ignoredRun.workspace_detail,
  },
}
await writeFile(
  path.join(root, 'output', 'e2e-worktrees.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
