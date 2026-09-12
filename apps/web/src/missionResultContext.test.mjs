import assert from 'node:assert/strict'
import test from 'node:test'
import {
  currentMissionResult, missionResultPresentation, missionResultScope, startMissionResultRead,
} from './missionResultContext.ts'

const ids = {
  corpId: 'corp-a', actorId: 'alice', missionId: 'mission-a', roomId: 'room-a',
  workItemId: 'item-a', sourceRepository: 'owner/repo',
}
const scope = missionResultScope(ids)
const digest = 'a'.repeat(64)
const verification = 'b'.repeat(64)
const baseCommit = 'c'.repeat(40)
const headCommit = 'd'.repeat(40)
const flush = () => new Promise((resolve) => setImmediate(resolve))

// Owned synthetic DTOs matching the public store/protocol shape, not native evidence.
function contextFor(selected = scope, phase = 'published') {
  const artifactId = 'artifact-a', runId = 'run-a', taskId = 'task-a', deliverableId = 'deliverable-a'
  const [owner, name] = selected.sourceRepository.split('/')
  const hasPr = ['pull_request_created', 'published'].includes(phase)
  const pr = hasPr ? {
    number: 17, node_id: 'PR_fixture', url: `https://github.com/${owner}/${name}/pull/17`,
    state: 'OPEN', draft: true, base_ref: 'main', head_sha: headCommit,
    head_repository_owner: owner, is_cross_repository: false, auto_merge: false,
    title: 'Private title is not result metadata', body: 'Private body is not result metadata',
  } : null
  return {
    work_item: {
      id: selected.workItemId, corp_id: selected.corpId, mission_id: selected.missionId,
      version: 9, source_repository_owner: owner, source_repository_name: name,
      source_issue_number: 71, source_issue_url: `https://github.com/${owner}/${name}/issues/71`,
      policy: { secret: 'not-for-result-state' },
    },
    publication: {
      id: 'publication-a', corp_id: selected.corpId, mission_id: selected.missionId,
      factory_work_item_id: selected.workItemId, source_deliverable_id: deliverableId,
      artifact_id: artifactId, task_id: taskId, run_id: runId,
      source_issue_number: 71, source_issue_url: `https://github.com/${owner}/${name}/issues/71`,
      state: phase, version: 4, target_repository: `${owner}/${name}`,
      base_ref: 'HEAD', branch: 'ecorp/result', commit_sha: headCommit,
      failure_detail: null, pull_request_number: pr?.number ?? null,
      pull_request_url: pr?.url ?? null, pull_request_state: pr?.state ?? null,
      pull_request_draft: pr?.draft ?? null, pull_request_base_ref: pr?.base_ref ?? null,
      pull_request_head_sha: pr?.head_sha ?? null,
      pull_request_head_repository_owner: pr?.head_repository_owner ?? null,
      pull_request_is_cross_repository: pr?.is_cross_repository ?? null,
      provenance: {
        schema_version: 3, factory_work_item_id: selected.workItemId, mission_id: selected.missionId,
        task_ids: ['task-handoff', taskId], run_ids: ['run-history', runId],
        verification_sha256: verification,
        deliverable: {
          id: deliverableId, artifact_id: artifactId, sha256: digest,
          base_commit: baseCommit, head_commit: headCommit, source_branch: 'crony/source',
        },
        target: { repository: `${owner}/${name}`, base_ref: 'HEAD', branch: 'ecorp/result', commit: headCommit },
        pull_request: pr,
        authorization_snapshot: { token: 'never-retain' },
      },
      publisher_token: 'never-retain', authorization_snapshot: { reason: 'Private authority' },
    },
    source_deliverables: [{
      id: deliverableId, corp_id: selected.corpId, task_id: taskId, run_id: runId,
      artifact_id: artifactId, form: 'commit_branch', file_name: 'result.bundle',
      uri: `/api/corps/${encodeURIComponent(selected.corpId)}/artifacts/${artifactId}`,
      sha256: digest, media_type: 'application/octet-stream', bytes: 1234,
      provenance_signature: 'e'.repeat(64), verification_sha256: verification,
      base_commit: baseCommit, head_commit: headCommit, branch: 'crony/source',
      integration_state: phase === 'published' ? 'published' : 'ready_for_review',
      retention_until: '2030-01-01T00:00:00Z',
      workspace_path: 'private-native-path', arbitrary_url: 'https://untrusted.invalid/',
    }],
  }
}

function deferred() {
  let resolve, reject
  const promise = new Promise((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}

function fakeClock() {
  let nextId = 0
  const timers = new Map()
  return {
    timers,
    setTimeout(callback, delay) {
      const id = ++nextId
      timers.set(id, { callback, delay })
      return id
    },
    clearTimeout(id) { timers.delete(id) },
    fire() {
      assert.equal(timers.size, 1)
      const [id, timer] = [...timers][0]
      assert.equal(timer.delay, 15_000)
      timers.delete(id)
      timer.callback()
    },
  }
}

function fixture(selected = scope, transport) {
  const response = deferred(), calls = [], loads = [], clock = fakeClock()
  const stop = startMissionResultRead(selected, (path, init) => {
    calls.push({ path, init })
    return transport ? transport(path, init) : response.promise
  }, (load) => loads.push(load), clock)
  return { response, calls, loads, clock, stop }
}

async function readResult(value, selected = scope) {
  const f = fixture(selected)
  try {
    await flush()
    f.response.resolve(value)
    await flush()
    assert.equal(f.calls.length, 1)
    assert.equal(f.clock.timers.size, 0)
    return f.loads.at(-1)
  } finally {
    f.stop()
  }
}

function unavailable(load, selected = scope) {
  assert.deepEqual(load, { scope: selected, status: 'unavailable', context: null })
}

function setAt(value, path, replacement) {
  const keys = path.split('.')
  const field = keys.pop()
  let parent = value
  for (const key of keys) parent = parent[key]
  if (replacement === undefined) delete parent[field]
  else parent[field] = replacement
}

test('scope requires every explicit identity and repository and freezes the view generation', () => {
  for (const key of Object.keys(ids)) {
    for (const invalid of [null, undefined, '', ' ', ' bad', 'bad ', 'bad\n', 'bad\0', 42, {}, []]) {
      assert.throws(() => missionResultScope({ ...ids, [key]: invalid }), key)
    }
  }
  for (const invalid of ['owner/repo/extra', 'https://github.com/owner/repo', 'owner/.', 'owner/..', 'owner/a%2fb']) {
    assert.throws(() => missionResultScope({ ...ids, sourceRepository: invalid }))
  }
  assert.ok(Object.isFrozen(scope))
  assert.throws(() => { scope.workItemId = 'another-item' }, TypeError)
  assert.notEqual(missionResultScope(ids), scope)
  assert.equal(missionResultScope(ids).key, scope.key)
})

test('the exact scope object, not equal IDs or a prior A -> B -> A view, selects the load', () => {
  const load = { scope, status: 'pending', context: null }
  assert.equal(currentMissionResult(scope, load), load)
  assert.equal(currentMissionResult(scope, null), null)
  assert.equal(currentMissionResult(null, load), null)
  for (const change of [{}, { actorId: 'bob' }, { corpId: 'corp-b' }, { roomId: 'room-b' },
    { missionId: 'mission-b' }, { workItemId: 'item-b' }, { sourceRepository: 'owner/other' }]) {
    const next = missionResultScope({ ...ids, ...change })
    assert.equal(currentMissionResult(next, load), null)
    assert.equal(missionResultPresentation(next, load).state, 'unavailable')
  }
})

test('one encoded GET is uncached, abortable and body/header/credential free', async () => {
  const selected = missionResultScope({
    ...ids, corpId: 'corp/a', actorId: 'actor+?&=/', workItemId: 'item?#/',
  })
  const f = fixture(selected)
  assert.deepEqual(f.loads, [{ scope: selected, status: 'pending', context: null }])
  await flush()
  assert.equal(f.calls.length, 1)
  const { path, init } = f.calls[0]
  assert.equal(path, '/api/corps/corp%2Fa/factory/work-items/item%3F%23%2F/publication-context?actor_id=actor%2B%3F%26%3D%2F')
  assert.deepEqual(Object.keys(init).sort(), ['cache', 'method', 'signal'])
  assert.equal(init.method, 'GET')
  assert.equal(init.cache, 'no-store')
  assert.ok(init.signal instanceof AbortSignal)
  f.response.resolve(contextFor(selected))
  await flush()
  assert.equal(f.loads.at(-1).status, 'ready')
  assert.equal(f.clock.timers.size, 0)
  f.stop()
})

test('published DTO retains only bound result metadata and leaves raw input untouched', async () => {
  const input = contextFor()
  const original = structuredClone(input)
  const load = await readResult(input)
  assert.equal(load.status, 'ready')
  assert.deepEqual(input, original)
  const { publication: p, deliverable: d } = load.context
  assert.equal(p.run_id, d.run_id)
  assert.equal(p.source_deliverable_id, d.id)
  assert.equal(p.artifact_id, d.artifact_id)
  assert.equal(p.commit_sha, d.head_commit)
  assert.equal(d.sha256, digest)
  assert.equal(d.verification_sha256, verification)
  assert.notEqual(p.branch, d.branch, 'publication branch is not the source worktree branch')
  assert.equal(p.base_ref, 'HEAD', 'symbolic base may resolve to PR base main')
  assert.doesNotMatch(JSON.stringify(load.context),
    /never-retain|Private|authorization|publisher_token|policy|workspace_path|private-native-path|arbitrary_url/)
  assert.equal(Object.hasOwn(p, 'provenance'), false)
  assert.equal(Object.hasOwn(load.context, 'source_deliverables'), false)
  input.publication.pull_request_url = 'https://untrusted.invalid/'
  input.source_deliverables[0].uri = '//untrusted.invalid/'
  assert.equal(p.pull_request_url, 'https://github.com/owner/repo/pull/17')
  assert.equal(d.uri, '/api/corps/corp-a/artifacts/artifact-a')
})

test('only explicit null publication in a correctly scoped response is known absence', async () => {
  const input = contextFor()
  input.publication = null
  const load = await readResult(input)
  assert.equal(load.status, 'ready')
  assert.equal(load.context.publication, null)
  assert.equal(load.context.deliverable, null)
  assert.equal(missionResultPresentation(scope, load).state, 'none')
  for (const invalid of [undefined, {}, [], false, 'null']) {
    const bad = contextFor()
    setAt(bad, 'publication', invalid)
    unavailable(await readResult(bad))
  }
  for (const bad of [null, {}, [], { ...input, source_deliverables: null }]) unavailable(await readResult(bad))
})

test('item and publication scope mismatches never expose links or known absence', async () => {
  for (const path of ['work_item.id', 'work_item.corp_id', 'work_item.mission_id',
    'work_item.source_repository_owner', 'work_item.source_repository_name',
    'publication.corp_id', 'publication.mission_id', 'publication.factory_work_item_id',
    'publication.target_repository', 'work_item.source_issue_number', 'work_item.source_issue_url',
    'publication.source_issue_number', 'publication.source_issue_url']) {
    for (const invalid of [undefined, null, 'foreign']) {
      const input = contextFor()
      setAt(input, path, invalid)
      unavailable(await readResult(input))
      if (path.startsWith('work_item.')) {
        input.publication = null
        unavailable(await readResult(input))
      }
    }
  }
})

test('publication source issue matches the work item without equating authorizer, revisions or checkpoint generations', async () => {
  const input = contextFor()
  input.publication.actor_id = 'bob'
  input.publication.provenance.source_issue = {
    number: 71, url: input.work_item.source_issue_url,
    revision: 'reviewed-correction', claimed_revision: 'original-intake',
  }
  input.publication.provenance.checkpoint = {
    origin_run_id: 'original-provider', original_head_commit: baseCommit, verified_head_commit: headCommit,
  }
  input.source_deliverables[0].retention_until = '2020-01-01T00:00:00Z'
  assert.equal((await readResult(input)).status, 'ready', 'expired artifact retention does not remove a historical PR link')
  for (const [field, value] of [
    ['source_issue_number', 72], ['source_issue_url', 'https://github.com/owner/repo/issues/72'],
    ['source_issue_url', 'https://github.com/owner/other/issues/71'],
    ['source_issue_url', 'https://github.com/owner/repo/issues/71?token=hidden'],
  ]) {
    const bad = contextFor()
    bad.publication[field] = value
    unavailable(await readResult(bad))
  }
})

test('publication and artifact identity tuples plus full digests must all match', async () => {
  const cases = [
    ['publication.id', ''], ['publication.run_id', 'another-run'], ['publication.task_id', 'another-task'],
    ['publication.source_deliverable_id', 'missing'], ['publication.artifact_id', 'other-artifact'],
    ['publication.commit_sha', 'f'.repeat(40)], ['publication.commit_sha', headCommit.slice(0, 12)],
    ['source_deliverables.0.corp_id', 'other-corp'], ['source_deliverables.0.run_id', 'other-run'],
    ['source_deliverables.0.task_id', 'other-task'], ['source_deliverables.0.artifact_id', 'other-artifact'],
    ['source_deliverables.0.head_commit', 'f'.repeat(40)],
    ['source_deliverables.0.sha256', 'f'.repeat(64)],
    ['source_deliverables.0.verification_sha256', 'f'.repeat(64)],
    ['source_deliverables.0.base_commit', 'f'.repeat(40)],
    ['source_deliverables.0.branch', 'crony/other'],
    ['source_deliverables.0.form', 'archive'],
  ]
  for (const [path, value] of cases) {
    const input = contextFor()
    setAt(input, path, value)
    unavailable(await readResult(input))
  }
})

test('missing, duplicate and unrelated-first deliverables cannot substitute a result', async () => {
  const empty = contextFor()
  empty.source_deliverables = []
  unavailable(await readResult(empty))
  const duplicate = contextFor()
  duplicate.source_deliverables.push(structuredClone(duplicate.source_deliverables[0]))
  unavailable(await readResult(duplicate))
  const reordered = contextFor()
  reordered.source_deliverables.unshift({ id: 'unrelated', uri: 'https://untrusted.invalid/' })
  const load = await readResult(reordered)
  assert.equal(load.status, 'ready')
  assert.equal(load.context.deliverable.id, 'deliverable-a')
  assert.doesNotMatch(JSON.stringify(load.context), /unrelated|untrusted/)
})

test('native provenance identity, source/target branches and digests are checked without retaining authority', async () => {
  const cases = [
    ['schema_version', 9], ['schema_version', '3'], ['factory_work_item_id', 'other-item'],
    ['mission_id', 'other-mission'], ['task_ids', ['other-task']], ['run_ids', ['other-run']],
    ['source_issue', { number: 72, url: 'https://github.com/owner/repo/issues/72' }],
    ['source_issue', { number: 71, url: 'https://github.com/other/repo/issues/71' }],
    ['run_ids', ['run-a', 'run-a']], ['verification_sha256', 'f'.repeat(64)],
    ['deliverable.id', 'other-deliverable'], ['deliverable.artifact_id', 'other-artifact'],
    ['deliverable.sha256', 'f'.repeat(64)], ['deliverable.base_commit', 'f'.repeat(40)],
    ['deliverable.head_commit', 'f'.repeat(40)], ['deliverable.source_branch', 'ecorp/result'],
    ['target.repository', 'other/repo'], ['target.branch', 'crony/source'],
    ['target.base_ref', 'different'], ['target.commit', 'f'.repeat(40)],
  ]
  for (const [path, value] of cases) {
    const input = contextFor()
    setAt(input.publication.provenance, path, value)
    unavailable(await readResult(input))
  }
  for (const version of [1, 2, 3]) {
    const input = contextFor()
    input.publication.provenance.schema_version = version
    assert.equal((await readResult(input)).status, 'ready')
  }
})

test('raw GitHub PR URLs allow native slash suffixes but retain exact repository and number checks', async () => {
  for (const url of [
    'http://github.com/owner/repo/pull/17', 'https://github.com.evil.invalid/owner/repo/pull/17',
    'https://user:password@github.com/owner/repo/pull/17', '//github.com/owner/repo/pull/17',
    'javascript:alert(1)', 'https://github.com/owner/other/pull/17', 'https://github.com/owner/repo/issues/17',
    'https://github.com/owner/repo/pull/18', 'https://github.com/owner/repo/pull/017',
    'https://github.com/owner/repo/pull/17?token=hidden',
    'https://github.com/owner/repo/pull/17#comment', 'https://github.com/owner/repo/pull/17\n',
    'https://github.com/owner/repo/pull/17///?token=hidden', 'https://github.com/owner/repo/pull/17///#comment',
    'https://github.com/owner/other/../repo/pull/17', 'https://github.com/owner/%72epo/pull/17',
    'https://github.com:443/owner/repo/pull/17', ' https://github.com/owner/repo/pull/17',
  ]) {
    const input = contextFor()
    input.publication.pull_request_url = url
    input.publication.provenance.pull_request.url = url
    unavailable(await readResult(input))
  }
  for (const number of [null, 0, -1, 1.5, Number.MAX_SAFE_INTEGER + 1, '17']) {
    const input = contextFor()
    input.publication.pull_request_number = number
    unavailable(await readResult(input))
  }
  for (const suffix of ['/', '////']) {
    const input = contextFor()
    const url = `https://github.com/owner/repo/pull/17${suffix}`
    input.publication.pull_request_url = url
    input.publication.provenance.pull_request.url = url
    const load = await readResult(input)
    assert.equal(load.status, 'ready')
    assert.equal(missionResultPresentation(scope, load).pullRequestUrl, url, 'preserve the raw native URL')
    input.publication.provenance.pull_request.url = 'https://github.com/owner/repo/pull/17'
    unavailable(await readResult(input), scope)

    const issue = contextFor()
    issue.work_item.source_issue_url += suffix
    issue.publication.source_issue_url = issue.work_item.source_issue_url
    unavailable(await readResult(issue), scope)
  }
})

test('owner/repository display case is preserved while full hex values compare by bytes', async () => {
  const input = contextFor()
  input.work_item.source_repository_owner = 'OWNER'
  input.publication.target_repository = 'Owner/Repo'
  input.publication.pull_request_url = 'https://github.com/OWNER/Repo/pull/17'
  input.publication.provenance.pull_request.url = input.publication.pull_request_url
  input.publication.commit_sha = headCommit.toUpperCase()
  input.source_deliverables[0].sha256 = digest.toUpperCase()
  const load = await readResult(input)
  assert.equal(load.status, 'ready')
  assert.equal(load.context.publication.pull_request_url, 'https://github.com/OWNER/Repo/pull/17')
  assert.equal(load.context.deliverable.sha256, digest)
  assert.equal(load.context.publication.commit_sha, headCommit)
})

test('PR state, exact head, owner, recorded metadata and phase cannot contradict each other', async () => {
  for (const [path, value] of [
    ['pull_request_head_sha', 'f'.repeat(40)], ['pull_request_head_repository_owner', 'foreign'],
    ['pull_request_is_cross_repository', true], ['pull_request_draft', 'true'],
    ['pull_request_state', 'UNKNOWN'], ['pull_request_base_ref', 'another'],
    ['provenance.pull_request.url', 'https://github.com/owner/repo/pull/18'],
    ['provenance.pull_request.number', 18], ['provenance.pull_request.head_sha', 'f'.repeat(40)],
    ['provenance.pull_request.is_cross_repository', true],
  ]) {
    const input = contextFor()
    setAt(input.publication, path, value)
    unavailable(await readResult(input))
  }
  const premature = contextFor()
  premature.publication.state = 'branch_pushed'
  unavailable(await readResult(premature))
  const missing = contextFor(scope, 'publishing')
  missing.publication.state = 'published'
  unavailable(await readResult(missing))
})

test('download metadata rejects controls, external/native paths, malformed hashes and invalid sizes', async () => {
  for (const [field, value] of [
    ['uri', 'https://untrusted.invalid/file'], ['uri', '//untrusted.invalid/file'],
    ['uri', '/api/corps/other/artifacts/artifact-a'], ['uri', '/api/corps/corp-a/artifacts/other'],
    ['uri', '/api/corps/corp-a/artifacts/artifact-a?token=hidden'],
    ['file_name', '../result.bundle'], ['file_name', 'C:\\private\\result.bundle'], ['file_name', 'bad\n'],
    ['sha256', `${digest}\n`], ['sha256', 'z'.repeat(64)], ['sha256', digest.slice(1)],
    ['sha256', `${digest.slice(1)}\n`],
    ['provenance_signature', null], ['media_type', 'text/html\ninjected'],
    ['bytes', 0], ['bytes', -1], ['bytes', 1.5], ['bytes', Number.MAX_SAFE_INTEGER + 1],
    ['integration_state', 'deployed'], ['retention_until', 'not-a-date'],
    ['integration_state', ['published']],
  ]) {
    const input = contextFor()
    input.source_deliverables[0][field] = value
    unavailable(await readResult(input))
  }
  for (let code = 0; code <= 0x9f; code++) {
    if (code > 0x1f && code < 0x7f) continue
    const input = contextFor()
    input.source_deliverables[0].file_name = `re${String.fromCharCode(code)}sult.bundle`
    unavailable(await readResult(input))
  }
  for (const code of [0x20, 0x7e, 0xa0, 0x1f600]) {
    const input = contextFor()
    input.source_deliverables[0].file_name = `re${String.fromCodePoint(code)}sult.bundle`
    assert.equal((await readResult(input)).status, 'ready', `non-control code point ${code}`)
  }
})

test('pending, PR-created, published and failed publication are distinct without claiming a live app', async () => {
  for (const phase of ['requested', 'publishing', 'branch_pushed', 'pull_request_created', 'published']) {
    const input = contextFor(scope, phase)
    const load = await readResult(input)
    assert.equal(load.status, 'ready')
    const view = missionResultPresentation(scope, load)
    assert.equal(view.state, phase === 'published' ? 'published' : phase === 'pull_request_created' ? 'pr_created' : 'pending')
    assert.equal(Boolean(view.pullRequestUrl), ['published', 'pull_request_created'].includes(phase))
    assert.equal(Object.hasOwn(view, 'appUrl'), false)
    input.publication.failure_detail = 'Private command failure with credential-shaped text'
    const failed = missionResultPresentation(scope, await readResult(input))
    assert.equal(failed.state, 'failed')
    assert.equal(failed.pullRequestUrl, view.pullRequestUrl, 'an existing PR can still be viewed after a later failure')
    assert.doesNotMatch(JSON.stringify(failed), /Private command|credential-shaped/)
  }
})

test('only a different valid pinned run is a mismatch; same-run contradictions stay unavailable', async () => {
  const load = await readResult(contextFor())
  const historical = Object.freeze({ id: 'old-run', task_id: 'old-task' })
  assert.deepEqual(missionResultPresentation(scope, load, historical), {
    state: 'mismatch', publication: null, deliverable: null, pullRequestUrl: null, resultRunId: 'run-a',
  })
  assert.equal(historical.id, 'old-run', 'only explicit UI navigation may change the reviewed run')
  for (const selected of [
    false, 0, '',
    { id: '' }, { id: 'run-a', task_id: 'other-task' },
    { id: 'run-a', verification_sha256: null }, { id: 'run-a', verification_sha256: 'f'.repeat(64) },
    { id: 'run-a', deliverable_sha256: null }, { id: 'run-a', deliverable_sha256: 'f'.repeat(64) },
  ]) {
    assert.deepEqual(missionResultPresentation(scope, load, selected), {
      state: 'unavailable', publication: null, deliverable: null, pullRequestUrl: null, resultRunId: null,
    })
  }
  for (const selected of [undefined, null, { id: 'run-a' },
    { id: 'run-a', task_id: 'task-a', verification_sha256: verification, deliverable_sha256: digest }]) {
    const view = missionResultPresentation(scope, load, selected)
    assert.equal(view.state, 'published')
    assert.equal(view.deliverable.id, 'deliverable-a')
  }
})

test('loading, denied and stale contexts cannot become a result or known absence', async () => {
  const ready = await readResult(contextFor())
  assert.equal(missionResultPresentation(scope, null).state, 'loading')
  assert.equal(missionResultPresentation(scope, { scope, status: 'pending', context: null }).state, 'loading')
  for (const [currentScope, load] of [
    [null, ready], [missionResultScope(ids), ready], [scope, { scope, status: 'unavailable', context: null }],
  ]) {
    const view = missionResultPresentation(currentScope, load)
    assert.equal(view.state, 'unavailable')
    assert.equal(view.pullRequestUrl, null)
    assert.equal(view.deliverable, null)
  }
})

test('denials, missing endpoints and sync/async transport failures stay unavailable without raw errors', async () => {
  for (const failure of [401, 403, 404, 405, 500, 501, 'network', 'invalid JSON']) {
    const f = fixture()
    await flush()
    f.response.reject(new Error(`private server detail ${failure}`))
    await flush()
    unavailable(f.loads.at(-1))
    assert.equal(f.calls.length, 1)
    assert.equal(f.clock.timers.size, 0)
    assert.doesNotMatch(JSON.stringify(f.loads), /private server detail/)
    f.stop()
  }
  const sync = fixture(scope, () => { throw new Error('private sync failure') })
  await flush()
  unavailable(sync.loads.at(-1))
  assert.equal(sync.clock.timers.size, 0)
  sync.stop()
})

test('timeout aborts and suppresses late successes and failures without retries', async () => {
  for (const outcome of ['resolve', 'reject']) {
    const f = fixture()
    await flush()
    f.clock.fire()
    unavailable(f.loads.at(-1))
    assert.equal(f.calls[0].init.signal.aborted, true)
    const count = f.loads.length
    f.response[outcome](outcome === 'resolve' ? contextFor() : new Error('late secret'))
    await flush()
    assert.equal(f.loads.length, count)
    assert.equal(f.calls.length, 1)
    assert.equal(f.clock.timers.size, 0)
    f.stop()
  }
})

test('queued timeout callbacks cannot overwrite a settled success, absence or denial', async () => {
  for (const result of ['publication', 'absence', 'denied']) {
    const f = fixture()
    const timeout = [...f.clock.timers.values()][0].callback
    await flush()
    const value = contextFor()
    if (result === 'absence') value.publication = null
    if (result === 'denied') f.response.reject(new Error('denied'))
    else f.response.resolve(value)
    await flush()
    const count = f.loads.length
    timeout()
    assert.equal(f.loads.length, count)
    assert.equal(f.clock.timers.size, 0)
    f.stop()
  }
})

test('cancellation before start issues no GET; in-flight cancellation publishes no late data', async () => {
  const early = fixture()
  early.stop()
  early.stop()
  await flush()
  assert.equal(early.calls.length, 0)
  assert.equal(early.loads.length, 1)
  assert.equal(early.clock.timers.size, 0)
  for (const outcome of ['resolve', 'reject']) {
    const f = fixture()
    await flush()
    f.stop()
    assert.equal(f.calls[0].init.signal.aborted, true)
    f.response[outcome](outcome === 'resolve' ? contextFor() : new Error('late error'))
    await flush()
    assert.equal(f.loads.length, 1)
    assert.equal(f.clock.timers.size, 0)
  }
})

test('new same-ID generation remains unavailable after denial despite a prior response completing later', async () => {
  const first = fixture()
  await flush()
  first.stop()
  const nextScope = missionResultScope(ids)
  const next = fixture(nextScope)
  await flush()
  next.response.reject(new Error('current access denied'))
  await flush()
  first.response.resolve(contextFor())
  await flush()
  assert.equal(first.loads.length, 1)
  unavailable(next.loads.at(-1), nextScope)
  assert.equal(missionResultPresentation(nextScope, next.loads.at(-1)).state, 'unavailable')
  next.stop()
})
