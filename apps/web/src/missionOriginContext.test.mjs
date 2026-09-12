import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import * as jsxRuntime from 'react/jsx-runtime'
import { renderToStaticMarkup } from 'react-dom/server'
import ts from 'typescript'
import * as reader from './missionOriginContext.ts'

const { currentMissionOrigin, missionOriginScope, startMissionOriginRead } = reader
const scope = missionOriginScope('corp-a', 'alice', 'mission-a', 'room-a')
const fallback = {
  kind: 'unknown', label: 'Mission origin unavailable',
  detail: 'This view does not establish how this mission entered ECorp.',
}
const factoryOrigin = {
  kind: 'factory', work_item_id: 'work-item-a', source_repository: 'owner/repo',
  source_issue_number: 199, source_issue_url: 'https://github.com/owner/repo/issues/199',
}
const contextFor = (selectedScope = scope, origin = { kind: 'direct' }) => ({
  corp_id: selectedScope.corpId, actor_id: selectedScope.actorId,
  mission_id: selectedScope.missionId, room_id: selectedScope.roomId, origin,
})
const direct = contextFor()
const factory = contextFor(scope, factoryOrigin)
const flush = () => new Promise((resolve) => setImmediate(resolve))

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

function fixture(selectedScope = scope, transport) {
  const response = deferred(), calls = [], loads = [], clock = fakeClock()
  const stop = startMissionOriginRead(selectedScope, (path, init) => {
    calls.push({ path, init })
    return transport ? transport(path, init) : response.promise
  }, (load) => loads.push(load), clock)
  return { response, calls, loads, clock, stop }
}

async function readResult(value) {
  const f = fixture()
  try {
    await flush()
    f.response.resolve(value)
    await flush()
    assert.equal(f.clock.timers.size, 0)
    assert.equal(f.calls.length, 1)
    return f.loads.at(-1)
  } finally {
    f.stop()
  }
}

function assertUnavailable(load, message) {
  assert.deepEqual(load, { scope, status: 'unavailable', context: null }, message)
}

test('scope requires four explicit bounded identities and cannot mutate after submission', () => {
  const ids = ['corp-a', 'alice', 'mission-a', 'room-a']
  for (let index = 0; index < ids.length; index++) {
    for (const invalid of [null, undefined, '', ' ', ' alice', 'alice ', 'a\nb', '\0', '.', '..', 'a'.repeat(129), 42, {}, []]) {
      const input = [...ids]
      input[index] = invalid
      assert.throws(() => missionOriginScope(...input), /explicit Corp, operator, mission and room/)
    }
  }
  assert.equal(missionOriginScope('c'.repeat(128), ...ids.slice(1)).corpId.length, 128)
  assert.ok(Object.isFrozen(scope))
  assert.throws(() => { scope.actorId = 'bob' }, TypeError)
  assert.equal(scope.key, JSON.stringify(ids))
})

test('current origin binds the exact view instance, not just equal IDs', () => {
  const ready = { scope, status: 'ready', context: direct }
  assert.equal(currentMissionOrigin(scope, ready), ready)
  assert.equal(currentMissionOrigin(scope, null), null)
  assert.equal(currentMissionOrigin(null, ready), null)
  for (const change of [
    { corpId: 'corp-b' }, { actorId: 'bob' }, { missionId: 'mission-b' }, { roomId: 'room-b' }, {},
  ]) {
    const ids = { ...scope, ...change }
    const next = missionOriginScope(ids.corpId, ids.actorId, ids.missionId, ids.roomId)
    for (const status of ['pending', 'ready', 'unavailable']) {
      assert.equal(currentMissionOrigin(next, { ...ready, status }), null)
    }
  }
})

test('read issues exactly one encoded, uncached GET with cancellation and no body', async () => {
  const selected = missionOriginScope('corp/a', 'actor+?&=/', 'mission?#/', 'room/a')
  const f = fixture(selected)
  assert.deepEqual(f.loads, [{ scope: selected, status: 'pending', context: null }])
  await flush()
  assert.equal(f.calls.length, 1)
  const { path, init } = f.calls[0]
  assert.equal(path, '/api/corps/corp%2Fa/missions/mission%3F%23%2F/context?actor_id=actor%2B%3F%26%3D%2F')
  assert.deepEqual(Object.keys(init).sort(), ['cache', 'method', 'signal'])
  assert.equal(init.method, 'GET')
  assert.equal(init.cache, 'no-store')
  assert.ok(init.signal instanceof AbortSignal)
  assert.equal(init.signal.aborted, false)
  f.response.resolve(contextFor(selected))
  await flush()
  assert.deepEqual(f.loads.at(-1), { scope: selected, status: 'ready', context: contextFor(selected) })
  assert.equal(f.calls.length, 1)
  assert.equal(f.clock.timers.size, 0)
  f.stop()
})

test('Direct accepts only exact returned linkage and drops unexpected hidden fields', async () => {
  const input = {
    ...direct, claim_token: 'hidden-token',
    origin: { ...factoryOrigin, kind: 'direct', secret: { token: 'hidden-secret' } },
  }
  const load = await readResult(input)
  assert.deepEqual(load, { scope, status: 'ready', context: direct })
  assert.notEqual(load.context, input)
  assert.notEqual(load.context.origin, input.origin)
  assert.equal(input.origin.source_repository, 'owner/repo', 'the reader does not mutate API data')
})

test('Factory retains only the validated work item, repository, issue number and exact URL', async () => {
  const input = {
    ...factory, unrelated_mission: direct, authorization: 'hidden',
    origin: { ...factoryOrigin, policy: { token: 'hidden' }, arbitrary_url: 'https://untrusted.test/' },
  }
  const load = await readResult(input)
  assert.deepEqual(load, { scope, status: 'ready', context: factory })
  assert.notEqual(load.context.origin, input.origin)
})

test('all four response identities must echo the bound request exactly', async () => {
  for (const field of ['corp_id', 'actor_id', 'mission_id', 'room_id']) {
    for (const invalid of [undefined, null, '', 'foreign-id', 1, [direct[field]]]) {
      for (const context of [direct, factory]) {
        assertUnavailable(await readResult({ ...context, [field]: invalid }), field)
      }
    }
  }
})

test('missing, malformed and older origin responses never become Direct', async () => {
  for (const value of [
    null, undefined, false, 'direct', [], {}, '<html>Not found</html>',
    { ...direct, origin: null }, { ...direct, origin: [] },
    { ...direct, origin: 'direct' }, { ...direct, origin: {} },
    { ...direct, origin: { kind: 'unknown' } }, { ...direct, origin: { kind: 'future' } },
    { ...direct, origin: { kind: null } }, { ...direct, origin: { kind: 'Direct' } },
    { mission: direct }, { ...direct, origin: { kind: 'factory' } },
  ]) {
    assertUnavailable(await readResult(value))
  }
})

test('Factory fields are required and null or missing metadata is never partly accepted', async () => {
  for (const field of ['work_item_id', 'source_repository', 'source_issue_number', 'source_issue_url']) {
    for (const invalid of [null, undefined, '', {}, []]) {
      assertUnavailable(await readResult(contextFor(scope, { ...factoryOrigin, [field]: invalid })), field)
    }
  }
  assertUnavailable(await readResult(contextFor(scope, {
    kind: 'factory', work_item_id: null, source_repository: null,
    source_issue_number: null, source_issue_url: null,
  })))
})

test('Factory rejects invalid work-item identities, repository paths and issue numbers', async () => {
  for (const work_item_id of [' ', 'hidden item', '\n', '..', 'a'.repeat(129), 199]) {
    assertUnavailable(await readResult(contextFor(scope, { ...factoryOrigin, work_item_id })))
  }
  for (const source_repository of [
    'owner', '/owner/repo', 'owner/repo/extra', 'https://github.com/owner/repo',
    'owner/repo?query', 'owner/..', 'owner/.', '../repo', 'owner/%2e%2e',
    'owner\\repo', 'owner/repo\n', '-owner/repo', 'owner-/repo', `owner/${'r'.repeat(241)}`,
  ]) {
    assertUnavailable(await readResult(contextFor(scope, {
      ...factoryOrigin, source_repository,
      source_issue_url: `https://github.com/${source_repository}/issues/199`,
    })), source_repository)
  }
  for (const source_issue_number of [0, -1, 1.5, NaN, Infinity, '199', Number.MAX_SAFE_INTEGER + 1]) {
    assertUnavailable(await readResult(contextFor(scope, {
      ...factoryOrigin, source_issue_number,
      source_issue_url: `https://github.com/owner/repo/issues/${source_issue_number}`,
    })))
  }
})

test('only the raw HTTPS GitHub issue URL matching returned repository and number is accepted', async () => {
  for (const source_issue_url of [
    'http://github.com/owner/repo/issues/199',
    'javascript:alert(1)', 'data:text/html,secret', '//github.com/owner/repo/issues/199',
    'https://github.com.evil.test/owner/repo/issues/199',
    'https://github.com@evil.test/owner/repo/issues/199',
    'https://evil@github.com/owner/repo/issues/199',
    'https://user:password@github.com/owner/repo/issues/199',
    'https://www.github.com/owner/repo/issues/199',
    'https://api.github.com/owner/repo/issues/199',
    'https://github.com:443/owner/repo/issues/199',
    'https://github.com:444/owner/repo/issues/199',
    'https://github.com/owner/repo/issues/199?token=hidden',
    'https://github.com/owner/repo/issues/199#comment',
    'https://github.com/owner/repo/issues/199/',
    'https://github.com/other/repo/issues/199',
    'https://github.com/owner/other/issues/199',
    'https://github.com/owner/repo/issues/200',
    'https://github.com/owner/repo/pull/199',
    'https://github.com/owner/repo/issues/0199',
    'https://github.com/owner/repo/issues/+199',
    'https://github.com/discard/../owner/repo/issues/199',
    'https://github.com/owner/r%65po/issues/199',
    'https://github.com/owner/repo/issues/%31%39%39',
    'HTTPS://github.com/Owner/Repo/issues/199',
    'https://GitHub.com/Owner/Repo/issues/199',
    'https://github.com/Owner/Repo/ISSUES/199',
    'https://github.com/Other/Repo/issues/199',
    'https://github.com/Owner/Repo/issues/200',
    ' https://github.com/owner/repo/issues/199',
    'https://github.com/owner/repo/issues/199\n',
    'https://git\thub.com/owner/repo/issues/199',
    'https:\\\\github.com\\owner\\repo\\issues\\199',
  ]) {
    assertUnavailable(await readResult(contextFor(scope, { ...factoryOrigin, source_issue_url })), source_issue_url)
  }
})

test('mixed-case owner/repository URLs match normalized identity and retain display case', async () => {
  for (const source_issue_url of [
    'https://github.com/Owner/repo/issues/199',
    'https://github.com/owner/Repo/issues/199',
    'https://github.com/OWNER/RePo/issues/199',
  ]) {
    const context = contextFor(scope, { ...factoryOrigin, source_issue_url })
    assert.deepEqual(await readResult(context), { scope, status: 'ready', context })
  }
})

test('valid case-preserving repositories and positive safe issue numbers remain verbatim', async () => {
  const origin = {
    ...factoryOrigin, source_repository: 'Owner/Repo.name-1',
    source_issue_number: Number.MAX_SAFE_INTEGER,
    source_issue_url: `https://github.com/Owner/Repo.name-1/issues/${Number.MAX_SAFE_INTEGER}`,
  }
  assert.deepEqual((await readResult(contextFor(scope, origin))).context.origin, origin)
})

test('old endpoints, denied authorization and transport failures stay unavailable without raw errors', async () => {
  for (const error of [
    ...[401, 403, 404, 405, 500, 501].map((status) => Object.assign(new Error('hidden source metadata'), { status })),
    new SyntaxError('Unexpected HTML instead of JSON'), new Error('Network unavailable'),
  ]) {
    const f = fixture()
    await flush()
    f.response.reject(error)
    await flush()
    assertUnavailable(f.loads.at(-1))
    assert.equal(f.calls.length, 1, 'there is no retry')
    assert.equal(f.clock.timers.size, 0)
    assert.doesNotMatch(JSON.stringify(f.loads), /hidden source metadata/)
    f.stop()
  }
})

test('a synchronous API failure is contained and clears the read timer', async () => {
  const f = fixture(scope, () => { throw new Error('Unavailable client') })
  await flush()
  assertUnavailable(f.loads.at(-1))
  assert.equal(f.calls.length, 1)
  assert.equal(f.clock.timers.size, 0)
  f.stop()
})

test('timeout aborts an unsettled read and suppresses both late success and late failure', async () => {
  for (const outcome of ['resolve', 'reject']) {
    const f = fixture()
    await flush()
    f.clock.fire()
    assert.equal(f.calls[0].init.signal.aborted, true)
    assertUnavailable(f.loads.at(-1))
    const count = f.loads.length
    f.response[outcome](outcome === 'resolve' ? factory : new Error('late failure'))
    await flush()
    assert.equal(f.loads.length, count)
    assert.equal(f.clock.timers.size, 0)
    f.stop()
  }
})

test('a settled success or failure cannot be overwritten by an already queued timeout', async () => {
  for (const outcome of ['resolve', 'reject']) {
    const f = fixture()
    const expiredCallback = [...f.clock.timers.values()][0].callback
    await flush()
    f.response[outcome](outcome === 'resolve' ? direct : new Error('denied'))
    await flush()
    const count = f.loads.length
    expiredCallback()
    assert.equal(f.loads.length, count)
    assert.equal(f.clock.timers.size, 0)
    f.stop()
  }
})

test('cancellation before the scheduled read prevents even the GET', async () => {
  const f = fixture()
  f.stop()
  f.stop()
  await flush()
  assert.equal(f.calls.length, 0)
  assert.deepEqual(f.loads, [{ scope, status: 'pending', context: null }])
  assert.equal(f.clock.timers.size, 0)
})

test('in-flight cancellation ignores transports that resolve or reject after abort', async () => {
  for (const outcome of ['resolve', 'reject']) {
    const f = fixture()
    await flush()
    f.stop()
    assert.equal(f.calls[0].init.signal.aborted, true)
    const count = f.loads.length
    f.response[outcome](outcome === 'resolve' ? factory : new Error('late denial'))
    await flush()
    assert.equal(f.loads.length, count, 'navigation must not publish a failure or old privileged metadata')
    assert.equal(f.clock.timers.size, 0)
  }
})

// Exercise the actual TSX with deterministic hook lifetimes and the real reader.
// Local transpilation + server markup rendering are not browser/full-stack evidence.
const componentSource = await readFile(new URL('./MissionOriginDetails.tsx', import.meta.url), 'utf8')
const compiledComponent = ts.transpileModule(componentSource, {
  fileName: 'MissionOriginDetails.tsx', reportDiagnostics: true,
  compilerOptions: {
    target: ts.ScriptTarget.ES2023, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX,
  },
})
assert.deepEqual(compiledComponent.diagnostics, [])

function componentFixture(overrides = {}) {
  const hooks = [], effects = [], calls = [], scopes = [], clock = fakeClock()
  let cursor = 0, dirty = false, writes = 0, tree
  const different = (previous, next) =>
    !previous || previous.length !== next.length || next.some((value, index) => !Object.is(value, previous[index]))
  const react = {
    useMemo(create, deps) {
      const index = cursor++
      if (different(hooks[index]?.deps, deps)) hooks[index] = { deps, value: create() }
      return hooks[index].value
    },
    useState(initial) {
      const index = cursor++
      if (!hooks[index]) {
        const state = { value: typeof initial === 'function' ? initial() : initial }
        state.set = (value) => {
          state.value = typeof value === 'function' ? value(state.value) : value
          dirty = true
          writes++
        }
        hooks[index] = state
      }
      return [hooks[index].value, hooks[index].set]
    },
    useEffect(create, deps) {
      const index = cursor++
      if (different(hooks[index]?.deps, deps)) {
        const effect = { deps, create, cleanup: hooks[index]?.cleanup }
        hooks[index] = effect
        effects.push(effect)
      }
    },
  }
  const api = (path, init) => {
    const response = deferred()
    calls.push({ path, init, response })
    return response.promise
  }
  const exports = {}
  const require = (name) => {
    if (name === 'react') return react
    if (name === 'react/jsx-runtime') return jsxRuntime
    assert.equal(name, './missionOriginContext')
    return {
      ...reader,
      startMissionOriginRead(selectedScope, get, publish) {
        scopes.push(selectedScope)
        return startMissionOriginRead(selectedScope, get, publish, clock)
      },
    }
  }
  new Function('require', 'exports', compiledComponent.outputText)(require, exports)
  let props = {
    corpId: 'corp-a', actorId: 'alice', missionId: 'mission-a', roomId: 'room-a',
    actorRole: 'member', api, fallback, ...overrides,
  }
  const html = () => renderToStaticMarkup(tree)
  const render = (next = props) => {
    props = next
    cursor = 0
    dirty = false
    tree = exports.MissionOriginDetails(props)
    return html()
  }
  const commit = () => {
    for (let pass = 0; ; pass++) {
      assert.ok(pass < 10, 'render must not cause repeated requests')
      for (const effect of effects.splice(0)) {
        effect.cleanup?.()
        effect.cleanup = effect.create()
      }
      if (!dirty) return html()
      render()
    }
  }
  const update = (next = props) => { render(next); return commit() }
  return {
    calls, scopes, clock, render, commit, update,
    get props() { return props },
    get tree() { return tree },
    get writes() { return writes },
    get html() { return html() },
    async flush() { await flush(); return update() },
    unmount() { hooks.forEach((hook) => hook.cleanup?.()); effects.length = 0 },
  }
}

test('component reads once for a valid view and renders truthful Direct copy in one styled paragraph', async () => {
  const view = componentFixture()
  view.update()
  await view.flush()
  assert.equal(view.calls.length, 1)
  assert.match(view.html, /Checking mission context/)
  view.calls[0].response.resolve(direct)
  await view.flush()
  assert.equal(view.tree.type, 'p', 'the parent retains its existing mission-work-context container')
  assert.match(view.html, /<strong>Direct mission<\/strong>/)
  assert.match(view.html, /not linked to Factory intake/)
  assert.doesNotMatch(view.html, /manual|browser|started here|<a |<div|<button|<style|Checking|could not be loaded/i)
  assert.match(view.html, /aria-live="polite" aria-busy="false"/)
  for (let index = 0; index < 3; index++) {
    view.update({ ...view.props, fallback: { ...fallback } })
    await view.flush()
  }
  assert.equal(view.calls.length, 1, 'snapshot/fallback object churn is not a retry')
  view.unmount()
})

test('component links only a fully validated Factory response, not extra returned fields', async () => {
  const view = componentFixture()
  view.update()
  await view.flush()
  view.calls[0].response.resolve({
    ...factory, secret: 'hidden-token',
    origin: { ...factoryOrigin, other_url: 'https://hidden.test/' },
  })
  await view.flush()
  assert.match(view.html, /<a href="https:\/\/github\.com\/owner\/repo\/issues\/199" target="_blank" rel="noopener noreferrer">From GitHub issue #199<\/a>/)
  assert.doesNotMatch(view.html, /Direct mission|hidden-token|hidden\.test|work-item-a/)
  view.unmount()
})

test('all four operating roles may read; guests, spectators and unknown roles never request context', async () => {
  for (const actorRole of ['owner', 'admin', 'manager', 'member', 'guest', 'spectator', 'agent', 'service', '', 'OWNER', undefined]) {
    const allowed = ['owner', 'admin', 'manager', 'member'].includes(actorRole)
    const view = componentFixture({ actorRole })
    view.update()
    await view.flush()
    assert.equal(view.calls.length, allowed ? 1 : 0, String(actorRole))
    assert.match(view.html, /Mission origin unavailable/)
    assert.doesNotMatch(view.html, /Direct mission|<a /)
    if (!allowed) assert.doesNotMatch(view.html, /Checking mission context/)
    view.unmount()
    assert.equal(view.clock.timers.size, 0)
  }
})

test('invalid or absent scope props render the conservative fallback without requesting', async () => {
  for (const field of ['corpId', 'actorId', 'missionId', 'roomId']) {
    for (const invalid of [null, undefined, '', ' ', 'x'.repeat(129)]) {
      const view = componentFixture({ [field]: invalid })
      view.update()
      await view.flush()
      assert.equal(view.calls.length, 0, field)
      assert.match(view.html, /Mission origin unavailable/)
      assert.doesNotMatch(view.html, /Direct mission|<a |Checking mission context/)
      view.unmount()
    }
  }
})

test('failed exact reads preserve #185 positive or unknown fallback without inventing links or hiding failure', async () => {
  for (const selectedFallback of [
    fallback,
    { kind: 'factory', label: 'Factory-linked mission', detail: 'Visible records link this mission to Factory.' },
    { kind: 'factory', label: 'From GitHub issue #185', detail: 'Intake details are unavailable in this view.' },
  ]) {
    for (const outcome of ['denial', 'malformed', 'timeout']) {
      const view = componentFixture({ fallback: selectedFallback })
      view.update()
      await view.flush()
      if (outcome === 'timeout') view.clock.fire()
      else if (outcome === 'malformed') view.calls[0].response.resolve({
        ...factory, origin: { ...factoryOrigin, source_issue_url: 'https://hidden.test/' },
      })
      else view.calls[0].response.reject(Object.assign(new Error('private source and token'), { status: 403 }))
      await view.flush()
      assert.ok(view.html.includes(selectedFallback.label))
      assert.ok(view.html.includes(selectedFallback.detail))
      assert.match(view.html, /Exact mission context could not be loaded/)
      assert.doesNotMatch(view.html, /Direct mission|<a |hidden\.test|private source|#199|Checking mission context/)
      view.unmount()
    }
  }
})

test('Corp, actor, room and mission changes hide old success and errors before effect cleanup', async () => {
  for (const change of [
    { corpId: 'corp-b' }, { actorId: 'bob' }, { roomId: 'room-b' }, { missionId: 'mission-b' },
  ]) {
    for (const firstOutcome of ['ready', 'unavailable']) {
      const view = componentFixture()
      view.update()
      await view.flush()
      if (firstOutcome === 'ready') view.calls[0].response.resolve(factory)
      else view.calls[0].response.reject(new Error('old failure'))
      await view.flush()
      view.render({ ...view.props, ...change, fallback: { ...fallback } })
      assert.match(view.html, /Mission origin unavailable/)
      assert.doesNotMatch(view.html, /<a |#199|could not be loaded/)
      view.commit()
      await view.flush()
      assert.equal(view.calls.length, 2)
      assert.equal(view.calls[0].init.signal.aborted, true)
      assert.notEqual(view.scopes[0], view.scopes[1])
      view.calls[1].response.reject(new Error('current failure'))
      await view.flush()
      assert.match(view.html, /Exact mission context could not be loaded/)
      assert.doesNotMatch(view.html, /<a |#199|Direct mission/)
      view.unmount()
    }
  }
})

test('late success and errors from a replaced request cannot update the current component', async () => {
  for (const change of [
    { corpId: 'corp-b' }, { actorId: 'bob' }, { roomId: 'room-b' }, { missionId: 'mission-b' },
  ]) {
    for (const outcome of ['resolve', 'reject']) {
      const view = componentFixture()
      view.update()
      await view.flush()
      view.update({ ...view.props, ...change })
      await view.flush()
      view.calls[1].response.resolve(contextFor(view.scopes[1]))
      await view.flush()
      const html = view.html, writes = view.writes
      view.calls[0].response[outcome](outcome === 'resolve' ? factory : new Error('old failure'))
      await view.flush()
      assert.equal(view.html, html)
      assert.equal(view.writes, writes)
      assert.match(view.html, /Direct mission/)
      assert.doesNotMatch(view.html, /<a |#199|could not be loaded/)
      view.unmount()
    }
  }
})

test('leaving and returning to identical IDs cannot revive an earlier success or failure', async () => {
  for (const firstOutcome of ['ready', 'unavailable']) {
    const view = componentFixture()
    view.update()
    await view.flush()
    if (firstOutcome === 'ready') view.calls[0].response.resolve(factory)
    else view.calls[0].response.reject(new Error('old failure'))
    await view.flush()
    const originalProps = view.props
    view.update({ ...originalProps, missionId: null })
    assert.doesNotMatch(view.html, /<a |#199|could not be loaded/)
    view.render(originalProps)
    assert.doesNotMatch(view.html, /<a |#199|could not be loaded/)
    view.commit()
    await view.flush()
    assert.equal(view.calls.length, 2)
    assert.equal(view.scopes[0].key, view.scopes[1].key)
    assert.notEqual(view.scopes[0], view.scopes[1])
    view.calls[1].response.reject(new Error('new denial'))
    await view.flush()
    assert.match(view.html, /Mission origin unavailable/)
    assert.match(view.html, /Exact mission context could not be loaded/)
    assert.doesNotMatch(view.html, /<a |#199|Direct mission/)
    view.unmount()
  }
})

test('same-ID reopened pending requests ignore the original response and error after current denial', async () => {
  for (const outcome of ['resolve', 'reject']) {
    const view = componentFixture()
    view.update()
    await view.flush()
    const originalProps = view.props
    view.update({ ...originalProps, missionId: 'mission-b' })
    await view.flush()
    view.update(originalProps)
    await view.flush()
    assert.equal(view.calls.length, 3)
    assert.equal(view.scopes[0].key, view.scopes[2].key)
    assert.notEqual(view.scopes[0], view.scopes[2])
    view.calls[2].response.reject(new Error('current denial'))
    await view.flush()
    const html = view.html, writes = view.writes
    view.calls[0].response[outcome](outcome === 'resolve' ? factory : new Error('late old failure'))
    view.calls[1].response.resolve(contextFor(view.scopes[1]))
    await view.flush()
    assert.equal(view.html, html)
    assert.equal(view.writes, writes)
    assert.match(view.html, /Exact mission context could not be loaded/)
    assert.doesNotMatch(view.html, /<a |#199|Direct mission/)
    view.unmount()
  }
})

test('a guest transition revokes displayed source data and returning operators must read again', async () => {
  const view = componentFixture({ actorRole: 'owner' })
  view.update()
  await view.flush()
  view.calls[0].response.resolve(factory)
  await view.flush()
  view.render({ ...view.props, actorRole: 'guest' })
  assert.doesNotMatch(view.html, /<a |#199|Direct mission/)
  view.commit()
  await view.flush()
  assert.equal(view.calls.length, 1)
  view.update({ ...view.props, actorRole: 'owner' })
  await view.flush()
  assert.equal(view.calls.length, 2)
  assert.notEqual(view.scopes[0], view.scopes[1])
  view.calls[1].response.reject(new Error('current room membership denied'))
  await view.flush()
  assert.match(view.html, /Exact mission context could not be loaded/)
  assert.doesNotMatch(view.html, /<a |#199|Direct mission/)
  view.unmount()
})

test('operating-role and API changes also invalidate old privileged context', async () => {
  for (const kind of ['role', 'api']) {
    const view = componentFixture({ actorRole: 'owner' })
    view.update()
    await view.flush()
    view.calls[0].response.resolve(factory)
    await view.flush()
    const previousApi = view.props.api
    const change = kind === 'role'
      ? { actorRole: 'member' }
      : { api: (path, init) => previousApi(path, init) }
    view.render({ ...view.props, ...change })
    assert.doesNotMatch(view.html, /<a |#199|Direct mission/)
    view.commit()
    await view.flush()
    assert.equal(view.calls.length, 2)
    view.calls[1].response.reject(new Error('current failure'))
    await view.flush()
    assert.match(view.html, /Exact mission context could not be loaded/)
    assert.doesNotMatch(view.html, /<a |#199|Direct mission/)
    view.unmount()
  }
})

test('unmount cancels the read and a fresh mount with identical IDs cannot inherit its outcome', async () => {
  for (const outcome of ['resolve', 'reject']) {
    const oldView = componentFixture()
    oldView.update()
    await oldView.flush()
    oldView.unmount()
    const writes = oldView.writes
    assert.equal(oldView.calls[0].init.signal.aborted, true)
    assert.equal(oldView.clock.timers.size, 0)
    const newView = componentFixture()
    newView.update()
    await newView.flush()
    oldView.calls[0].response[outcome](outcome === 'resolve' ? factory : new Error('old failure'))
    await newView.flush()
    assert.equal(oldView.writes, writes)
    assert.match(newView.html, /Checking mission context/)
    assert.doesNotMatch(newView.html, /<a |#199|could not be loaded|Direct mission/)
    newView.calls[0].response.resolve(direct)
    await newView.flush()
    assert.match(newView.html, /Direct mission/)
    newView.unmount()
  }
})
