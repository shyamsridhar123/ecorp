import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { spawnSync } from 'node:child_process'

const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const root = path.resolve(import.meta.dirname, '..')

function git(args, cwd = root) {
  const result = spawnSync('git', args, { cwd, encoding: 'utf8' })
  if (result.status !== 0) {
    throw new Error(`git ${args.join(' ')} failed: ${result.stderr}`)
  }
  return result.stdout.trim()
}

async function request(url, init) {
  const response = await fetch(`${server}${url}`, init)
  const body = await response
    .clone()
    .json()
    .catch(() => null)
  if (!response.ok) {
    throw new Error(`${init?.method ?? 'GET'} ${url}: ${response.status} ${JSON.stringify(body)}`)
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

function snapshot(demo) {
  return request(
    `/api/corps/${demo.corp_id}/snapshot?actor_id=${demo.alice_actor_id}`,
  )
}

async function waitForRun(demo, runId, timeoutMs = 60_000) {
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
  throw new Error(`timed out waiting for portable deliverable run ${runId}`)
}

async function createAndRun(demo, title, deliverable) {
  const mission = await post(`/api/corps/${demo.corp_id}/missions`, {
    requested_by: demo.alice_actor_id,
    preferred_adapter: 'fake-process',
    title,
    deliverable,
  })
  const launch = await post(
    `/api/corps/${demo.corp_id}/missions/${mission.mission_id}/launch`,
    { requested_by: demo.alice_actor_id },
  )
  const terminal = await waitForRun(demo, launch.run_id)
  assert.equal(terminal.run.status, 'completed', terminal.run.summary)
  const source = terminal.state.snapshot.source_deliverables.find(
    (candidate) => candidate.run_id === terminal.run.id,
  )
  assert.ok(source, 'completed run omitted its source deliverable')
  return { mission, ...terminal, source }
}

async function download(demo, source, actorId = demo.alice_actor_id) {
  const response = await fetch(
    `${server}${source.uri}?actor_id=${actorId}`,
    { headers: { accept: 'application/octet-stream' } },
  )
  if (actorId === demo.alice_actor_id) {
    assert.equal(response.status, 200)
    assert.equal(response.headers.get('x-crony-artifact-role'), 'source_deliverable')
    assert.equal(
      response.headers.get('x-crony-artifact-signature'),
      source.provenance_signature,
    )
    assert.match(
      response.headers.get('content-disposition') ?? '',
      new RegExp(source.file_name.replaceAll('.', '\\.')),
    )
  }
  return { response, bytes: Buffer.from(await response.arrayBuffer()) }
}

const sourceHeadBefore = git(['rev-parse', 'HEAD'])
const sourceStatusBefore = git(['status', '--porcelain=v1', '--untracked-files=all'])
const demo = await post('/api/demo/reset', {})

const archive = await createAndRun(
  demo,
  '[portable-deliverable] Export tracked and untracked source as a portable archive.',
  { form: 'archive', commit_after_verification: false, paths: [] },
)
const archiveDownload = await download(demo, archive.source)
assert.equal(
  createHash('sha256').update(archiveDownload.bytes).digest('hex'),
  archive.source.sha256,
)
const archiveDocument = JSON.parse(archiveDownload.bytes.toString('utf8'))
assert.equal(archiveDocument.form, 'archive')
assert.equal(archiveDocument.base_commit, archive.run.workspace_base_commit)
assert.equal(archiveDocument.verification_sha256, archive.source.verification_sha256)
assert.equal(archive.run.verification_sha256, archive.source.verification_sha256)
assert.equal(archive.run.deliverable_sha256, archive.source.sha256)
assert.deepEqual(
  archiveDocument.changes.map((change) => change.path),
  ['README.md', 'portable-untracked.txt'],
)
assert.equal(
  archiveDocument.changes.some((change) => change.path === 'result.md'),
  false,
  'provider evidence leaked into the source deliverable',
)
assert.ok(
  archive.state.snapshot.events.some(
    (event) =>
      event.type === 'run.artifact' &&
      event.aggregate_id === archive.run.id &&
      event.payload.artifact_role === 'provider_evidence',
  ),
)
assert.ok(
  archive.state.snapshot.events.some(
    (event) =>
      event.type === 'run.deliverable' &&
      event.aggregate_id === archive.run.id &&
      event.payload.artifact_role === 'source_deliverable',
  ),
)
assert.equal(archive.source.integration_state, 'ready_for_review')
assert.equal(
  archive.state.snapshot.events.some(
    (event) =>
      event.aggregate_id === archive.run.id &&
      (Object.hasOwn(event.payload, 'content_base64') ||
        Object.hasOwn(event.payload, 'object_key')),
  ),
  false,
)
const unauthorized = await download(demo, archive.source, demo.eve_actor_id)
assert.equal(unauthorized.response.status, 404)

const committed = await createAndRun(
  demo,
  '[portable-deliverable] Commit verified tracked and untracked source on the isolated task branch.',
  { form: 'commit_branch', commit_after_verification: true, paths: [] },
)
assert.ok(committed.source.head_commit)
assert.equal(committed.source.branch, committed.run.workspace_branch)
assert.equal(
  git(['rev-parse', 'HEAD'], committed.run.workspace_path),
  committed.source.head_commit,
)
assert.match(
  git(['show', '--format=', '--name-only', committed.source.head_commit], committed.run.workspace_path),
  /portable-untracked\.txt/,
)
const committedDownload = await download(demo, committed.source)
const committedDocument = JSON.parse(committedDownload.bytes.toString('utf8'))
assert.equal(committedDocument.head_commit, committed.source.head_commit)
assert.equal(committedDocument.branch, committed.source.branch)

const reclaim = await createAndRun(
  demo,
  '[clean-worktree] Export a review report, await durable storage, then reclaim the clean worktree.',
  { form: 'review_only_report', commit_after_verification: false, paths: [] },
)
assert.equal(reclaim.run.workspace_disposition, 'removed')
assert.equal(reclaim.source.integration_state, 'not_applicable')
const deliverableEvent = reclaim.state.snapshot.events.find(
  (event) => event.type === 'run.deliverable' && event.aggregate_id === reclaim.run.id,
)
const removedEvent = reclaim.state.snapshot.events.find(
  (event) => event.type === 'run.workspace_removed' && event.aggregate_id === reclaim.run.id,
)
assert.ok(deliverableEvent)
assert.ok(removedEvent)
assert.ok(
  deliverableEvent.seq < removedEvent.seq,
  'clean worktree was reclaimed before durable deliverable retention',
)
await download(demo, reclaim.source)

assert.equal(git(['rev-parse', 'HEAD']), sourceHeadBefore)
assert.equal(
  git(['status', '--porcelain=v1', '--untracked-files=all']),
  sourceStatusBefore,
)

const report = {
  checked_at: new Date().toISOString(),
  archive_run_id: archive.run.id,
  archive_artifact_id: archive.source.artifact_id,
  archive_sha256: archive.source.sha256,
  archive_verification_sha256: archive.source.verification_sha256,
  committed_run_id: committed.run.id,
  committed_artifact_id: committed.source.artifact_id,
  committed_head: committed.source.head_commit,
  committed_branch: committed.source.branch,
  reclaimed_run_id: reclaim.run.id,
  reclaimed_artifact_id: reclaim.source.artifact_id,
  retained_before_cleanup: deliverableEvent.seq < removedEvent.seq,
  unauthorized_download_status: unauthorized.response.status,
}
await writeFile(
  path.join(root, 'output', 'e2e-portable-deliverables.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
