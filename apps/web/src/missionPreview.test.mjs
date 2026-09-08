import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import {
  buildMissionRequest, currentMissionPreview, missionRequestScope, startMissionPreview,
} from './missionPreview.ts'

const clone = (value) => JSON.parse(JSON.stringify(value))
const draft = {
  title: 'A small game', description: 'Keep the exact operator specification.',
  actorId: 'alice', strategy: 'studio-swarm', adapter: 'github-copilot',
  model: 'chosen-model', selectedModel: { id: 'chosen-model', supported_reasoning_efforts: ['high'] },
  reasoningEffort: 'high',
  source: { repository: 'owner/repo', baseRef: 'main', baseCommit: 'a'.repeat(40) },
  budgetTokens: 1_000_000, deliverableForm: 'archive', commitDeliverable: false,
  contract: {
    objective: 'Build the game', expected_output: 'Source and evidence',
    acceptance_tests: ['Keyboard input works'], allowed_tools: ['filesystem'],
    prohibited_actions: ['No deployment'], references: ['SPEC.md'], write_scope: ['src/**'],
  },
  customVerification: true,
  verificationPolicy: {
    checks: [{ type: 'file', path: 'proof.txt', min_bytes: 1 }],
    manual_gate: { type: 'independent_review', roles: ['member'], exclude_requester: true },
  },
}
const request = buildMissionRequest(draft)
const scopeFor = (request, corp = 'corp-a', actor = request.requested_by) =>
  missionRequestScope(corp, actor, JSON.stringify(request))
const scope = scopeFor(request)
const quote = {
  strategy: 'studio-swarm', budget_tokens: 1_000_000, budget_cost_microusd: 6_000_000,
  tasks: [
    { key: 'visual-direction', title: 'Visual handoff', budget_tokens: 150_000, budget_cost_microusd: 900_000, depends_on: [], max_attempts: 2 },
    { key: 'gameplay-systems', title: 'Systems handoff', budget_tokens: 150_000, budget_cost_microusd: 900_000, depends_on: [], max_attempts: 2 },
    { key: 'quality-verification', title: 'Quality handoff', budget_tokens: 150_000, budget_cost_microusd: 900_000, depends_on: [], max_attempts: 2 },
    { key: 'studio-integration', title: 'Integration', budget_tokens: 550_000, budget_cost_microusd: 3_300_000, depends_on: ['visual-direction', 'gameplay-systems', 'quality-verification'], max_attempts: 2 },
  ],
}
const flush = () => new Promise((resolve) => setImmediate(resolve))
function deferred() {
  let resolve, reject
  const promise = new Promise((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}
function fixture(selectedScope = scope, response = deferred()) {
  let nextId = 0
  const timers = new Map(), calls = [], loads = []
  const clock = {
    setTimeout(callback, delay) { const id = ++nextId; timers.set(id, { callback, delay }); return id },
    clearTimeout(id) { timers.delete(id) },
  }
  const stop = startMissionPreview(selectedScope, (path, init) => {
    calls.push({ path, init })
    return response.promise
  }, (load) => loads.push(load), clock)
  const fire = (delay) => {
    const entry = [...timers].find(([, timer]) => timer.delay === delay)
    assert.ok(entry, `Missing ${delay}ms timer`)
    timers.delete(entry[0])
    entry[1].callback()
  }
  return { timers, calls, loads, stop, fire, response }
}

test('one builder preserves the complete current creation payload and retained settings', () => {
  assert.deepEqual(request, {
    title: draft.title, description: draft.description, requested_by: 'alice',
    preferred_adapter: 'github-copilot', preferred_model: 'chosen-model', reasoning_effort: 'high',
    strategy: 'studio-swarm',
    source: { repository: 'owner/repo', base_ref: 'main', base_commit: 'a'.repeat(40) },
    budget_tokens: 1_000_000,
    deliverable: { form: 'archive', commit_after_verification: false, paths: [] },
    contract: draft.contract, verification_policy: draft.verificationPolicy,
  })
  assert.equal('budget_cost_microusd' in request, false, 'Do not invent a previously omitted cost budget')
  assert.equal('secret_refs' in request, false, 'The prior UI did not submit secret-reference fields')
  assert.equal(buildMissionRequest({ ...draft, budgetTokens: 500_000 }).budget_tokens, 500_000)
  assert.equal(buildMissionRequest({ ...draft, strategy: 'single' }).strategy, 'single')
})

test('provider defaults, supported reasoning and commit-branch semantics match submission', () => {
  const defaults = buildMissionRequest({ ...draft, selectedModel: undefined, contract: null, customVerification: false })
  assert.equal(defaults.preferred_model, null)
  assert.equal(defaults.reasoning_effort, null)
  assert.equal(defaults.contract, null)
  assert.equal(defaults.verification_policy, null)
  assert.equal(buildMissionRequest({ ...draft, reasoningEffort: 'unsupported' }).reasoning_effort, null)
  assert.equal(buildMissionRequest({ ...draft, deliverableForm: 'commit_branch' }).deliverable.commit_after_verification, true)
  assert.equal(buildMissionRequest({ ...draft, commitDeliverable: true }).deliverable.commit_after_verification, true)
})

test('existing deterministic fixture semantics are not converted into provider settings', () => {
  const result = buildMissionRequest({ ...draft, strategy: 'verification-matrix', adapter: 'fake-process' })
  assert.equal(result.budget_tokens, null)
  assert.equal(result.preferred_model, null)
  assert.equal(result.reasoning_effort, null)
  assert.equal(result.verification_policy, null)
})

test('scope freezes the exact serialized body and rejects an actor mismatch', () => {
  const mutable = clone(request)
  const snapshot = scopeFor(mutable)
  mutable.contract.allowed_tools.push('changed-after-serialization')
  assert.equal(snapshot.body, JSON.stringify(request))
  assert.equal(snapshot.key, scope.key)
  assert.throws(() => scopeFor(request, 'corp-a', 'bob'), /operator scope/)
})

test('every request field, Corp and actor invalidate an earlier quote', () => {
  const variants = [
    { title: 'Other title' }, { description: 'Other description' },
    { requested_by: 'bob' }, { preferred_adapter: 'codex' }, { preferred_model: null },
    { reasoning_effort: null }, { strategy: 'single' }, { budget_tokens: 2_000_000 },
    { budget_cost_microusd: 1234 }, { secret_refs: [{ secret_id: 'reference-only' }] },
    ...['repository', 'base_ref', 'base_commit'].map((field) => ({ source: { ...request.source, [field]: 'changed' } })),
    ...['form', 'commit_after_verification', 'paths'].map((field) => ({
      deliverable: { ...request.deliverable, [field]: field === 'paths' ? ['src/game.ts'] : field === 'form' ? 'patch' : true },
    })),
    ...Object.keys(request.contract).map((field) => ({
      contract: { ...request.contract, [field]: Array.isArray(request.contract[field]) ? ['changed'] : 'changed' },
    })),
    { verification_policy: { ...request.verification_policy, checks: [{ type: 'artifact', min_bytes: 100 }] } },
    { verification_policy: { ...request.verification_policy, manual_gate: null } },
  ]
  const old = { scopeKey: scope.key, status: 'ready', quote, error: null }
  for (const change of variants) {
    const next = scopeFor({ ...request, ...change })
    assert.notEqual(next.key, scope.key)
    assert.equal(currentMissionPreview(next, old), null)
  }
  assert.equal(currentMissionPreview(scopeFor(request, 'corp-b'), old), null)
  assert.equal(currentMissionPreview(null, old), null)
})

test('preview debounces, uses the exact creation body, and calls only the read-only preview POST', async () => {
  const f = fixture()
  assert.deepEqual(f.loads[0], { scopeKey: scope.key, status: 'pending', quote: null, error: null })
  assert.equal(f.calls.length, 0)
  f.fire(300)
  assert.equal(f.calls.length, 1)
  assert.equal(f.calls[0].path, '/api/corps/corp-a/missions/preview')
  assert.equal(f.calls[0].init.method, 'POST')
  assert.equal(f.calls[0].init.body, scope.body)
  assert.deepEqual(JSON.parse(f.calls[0].init.body), request)
  f.response.resolve(quote)
  await flush()
  assert.deepEqual(f.loads.at(-1).quote, quote)
  assert.equal(f.loads.at(-1).status, 'ready')
  f.stop()
  assert.equal(f.timers.size, 0)
})

test('an edit or closing the composer before debounce starts no request', () => {
  const f = fixture()
  f.stop()
  assert.equal(f.timers.size, 0)
  assert.equal(f.calls.length, 0)
})

test('an invalidated request is aborted and cannot publish a late successful quote', async () => {
  const old = fixture()
  old.fire(300)
  old.stop()
  const nextRequest = { ...request, description: 'A newer specification' }
  const next = fixture(scopeFor(nextRequest))
  next.fire(300)
  next.response.resolve(quote)
  await flush()
  old.response.resolve({ ...quote, budget_tokens: 999 })
  await flush()
  assert.equal(old.calls[0].init.signal.aborted, true)
  assert.equal(old.loads.length, 1)
  assert.equal(next.loads.at(-1).scopeKey, scopeFor(nextRequest).key)
  assert.equal(next.loads.at(-1).quote.budget_tokens, quote.budget_tokens)
  next.stop()
})

test('late errors after actor/Corp change or unmount cannot replace the new preview', async () => {
  const old = fixture()
  old.fire(300)
  old.stop()
  old.response.reject(new Error('Obsolete failure'))
  await flush()
  assert.deepEqual(old.loads.map((load) => load.status), ['pending'])
  assert.equal(old.timers.size, 0)
})

test('editing away and back still requires a fresh request, not a revived quote', async () => {
  const first = fixture()
  first.fire(300)
  first.response.resolve(quote)
  await flush()
  first.stop()
  const next = fixture(scope)
  assert.equal(next.loads[0].quote, null)
  assert.equal(next.loads[0].status, 'pending')
  next.stop()
})

test('allocations are displayed verbatim from the server, not a frontend Studio formula', async () => {
  const authoritative = clone(quote)
  authoritative.tasks.forEach((task, index) => { task.budget_tokens = [17, 23, 41, 919][index] })
  authoritative.budget_tokens = 1000
  const f = fixture()
  f.fire(300)
  f.response.resolve(authoritative)
  await flush()
  assert.deepEqual(f.loads.at(-1).quote, authoritative)
  f.stop()
})

test('missing or mismatched quote fields never become a current allocation', async () => {
  for (const value of [
    null, { ...quote, strategy: 'single' }, { ...quote, tasks: [] },
    { ...quote, budget_tokens: undefined }, { ...quote, budget_cost_microusd: 'unknown' },
    { ...quote, tasks: [{ ...quote.tasks[0], max_attempts: undefined }] },
    { ...quote, tasks: [quote.tasks[0], quote.tasks[0]] },
  ]) {
    const f = fixture()
    f.fire(300)
    f.response.resolve(value)
    await flush()
    assert.equal(f.loads.at(-1).status, 'error')
    assert.equal(f.loads.at(-1).quote, null)
    assert.match(f.loads.at(-1).error, /unknown/)
    f.stop()
  }
})

test('an absent preview endpoint gives a useful failure state without inventing allocations', async () => {
  for (const error of [
    Object.assign(new Error('Missing route'), { status: 404 }),
    Object.assign(new Error('Method not allowed'), { status: 405 }),
    Object.assign(new Error('Not implemented'), { status: 501 }),
    new SyntaxError('Unexpected end of JSON input'),
  ]) {
    const f = fixture()
    f.fire(300)
    f.response.reject(error)
    await flush()
    assert.equal(f.loads.at(-1).status, 'error')
    assert.equal(f.loads.at(-1).quote, null)
    assert.match(f.loads.at(-1).error, /unavailable on this server/)
    assert.match(f.loads.at(-1).error, /Launch still uses the existing server validation/)
    f.stop()
  }
})

test('preview response is bounded to eight tasks without reallocating server budgets', async () => {
  for (const count of [8, 9]) {
    const value = {
      ...quote,
      tasks: Array.from({ length: count }, (_, index) => ({
        ...quote.tasks[0], key: `task-${index}`, depends_on: index ? [`task-${index - 1}`] : [],
      })),
    }
    const f = fixture()
    f.fire(300)
    f.response.resolve(value)
    await flush()
    assert.equal(f.loads.at(-1).status, count === 8 ? 'ready' : 'error')
    assert.deepEqual(f.loads.at(-1).quote, count === 8 ? value : null)
    f.stop()
  }
})

test('unknown, self, duplicate and oversized dependency references remain unknown allocations', async () => {
  for (const depends_on of [['absent-task'], ['visual-direction'], ['gameplay-systems', 'gameplay-systems'], Array(9).fill('gameplay-systems')]) {
    const value = clone(quote)
    value.tasks[0].depends_on = depends_on
    const f = fixture()
    f.fire(300)
    f.response.resolve(value)
    await flush()
    assert.equal(f.loads.at(-1).status, 'error')
    assert.equal(f.loads.at(-1).quote, null)
    assert.match(f.loads.at(-1).error, /unknown/)
    f.stop()
  }
})

test('retry clears an old failure immediately and can accept a fresh quote', async () => {
  const failed = fixture()
  failed.fire(300)
  failed.response.reject(new Error('Temporary failure'))
  await flush()
  failed.stop()
  const retry = fixture()
  assert.equal(retry.loads[0].error, null)
  assert.equal(retry.loads[0].quote, null)
  retry.fire(300)
  retry.response.resolve(quote)
  await flush()
  assert.equal(retry.loads.at(-1).status, 'ready')
  retry.stop()
})

test('timeout clears the quote, aborts the request and ignores a late response', async () => {
  const f = fixture()
  f.fire(300)
  f.fire(15_000)
  assert.equal(f.calls[0].init.signal.aborted, true)
  assert.equal(f.loads.at(-1).quote, null)
  assert.match(f.loads.at(-1).error, /timed out/)
  const count = f.loads.length
  f.response.resolve(quote)
  await flush()
  assert.equal(f.loads.length, count)
  f.stop()
  assert.equal(f.timers.size, 0)
})

test('App shares the request body, invalidates keyed preview instances and retains native Launch', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8')
  assert.equal((app.match(/buildMissionRequest\(/g) ?? []).length, 1)
  const create = app.slice(app.indexOf('const createMission ='), app.indexOf('const launchMission ='))
  assert.match(create, /body: currentMissionRequest\.body/)
  assert.doesNotMatch(create, /JSON\.stringify\(\{\s*title:/)
  assert.match(create, /if \(!pauseAfterPlanning\)/)
  assert.match(create, /missions\/\$\{created\.mission_id\}\/launch/)
  assert.doesNotMatch(create, /preview.*(?:approved|ready)|quote.*(?:approved|ready)/i)
  assert.match(app, /missionPreviewEnabled && currentMissionRequest/)
  assert.match(app, /MissionAllocationPreview key=\{currentMissionRequest\.key\} scope=\{currentMissionRequest\}/)
  assert.match(app, /!missionComposerCollapsed && activeWorkspaceView === 'missions'/)
  assert.match(app, /currentMissionRequest && selectedMissionSource && selectedActor && canOperate\(selectedActor\.role\)/)
  assert.match(app, /Boolean\(missionTitle\.trim\(\)\) && missionSourceConfirmed && !busy/)
  assert.match(app, /!runtimeError && missionVerifierErrors\.length === 0/)
  assert.match(app, /useEffect\(\(\) => startMissionPreview\(\{ key, corpId, actorId, body, strategy \}, api, setLoad\)/)
  assert.match(app, /\[key, corpId, actorId, body, strategy, refresh\]/)
  assert.match(app, /Strategy and budget stay selected between missions/)
  assert.match(app, /3 handoffs, then integration after all three complete/)
  assert.match(app, /<details className="mission-allocation-details">/)
  assert.match(app, /not a provider billing estimate/)
  assert.match(app, /Preview starts no work and grants no approval/)
  assert.match(app, /useState\('single'\)/)
  assert.match(app, /useState\(1_000_000\)/)
  assert.match(app, /quote\.tasks\.map/)
  assert.match(app, /task\.budget_tokens\.toLocaleString\(\)/)
  assert.ok(app.indexOf('aria-label="Mission settings and allocation"') < app.indexOf('className="button button-primary mission-submit"'))
})
