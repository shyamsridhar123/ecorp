import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import vm from 'node:vm'
import ts from 'typescript'
import { createSnapshotRefresher } from './snapshotRefresh.ts'

function fixture() {
  const timers = new Map()
  const calls = []
  const errors = []
  let next = 0
  const coordinator = createSnapshotRefresher({
    refresh: (signal) => new Promise((resolve, reject) => calls.push({ signal, resolve, reject })),
    onError: (error) => errors.push(error),
    schedule: (callback, delay) => {
      assert.equal(delay, 250)
      timers.set(++next, callback)
      return next
    },
    cancel: (timer) => timers.delete(timer),
  })
  const tick = () => {
    const pending = Array.from(timers.values())
    timers.clear()
    pending.forEach((callback) => callback())
  }
  return { coordinator, calls, errors, timers, tick }
}

const settle = () => new Promise((resolve) => setImmediate(resolve))

test('a burst of 1000 streamed events schedules one snapshot instead of 1000 requests', () => {
  const f = fixture()
  for (let i = 0; i < 1000; i++) f.coordinator.request()
  assert.equal(f.calls.length, 0)
  assert.equal(f.timers.size, 1)
  f.tick()
  assert.equal(f.calls.length, 1)
  f.coordinator.dispose()
})

test('events arriving during a request produce exactly one trailing snapshot', async () => {
  const f = fixture()
  f.coordinator.request()
  f.tick()
  for (let i = 0; i < 1000; i++) f.coordinator.request()
  f.tick()
  assert.equal(f.calls.length, 1, 'No overlapping full snapshot')
  f.calls[0].resolve({})
  await settle()
  assert.equal(f.timers.size, 1)
  f.tick()
  assert.equal(f.calls.length, 2)
  f.calls[1].resolve({})
  await settle()
  assert.equal(f.timers.size, 0)
  f.coordinator.dispose()
})

test('the final event during the trailing request is not dropped', async () => {
  const f = fixture()
  f.coordinator.request()
  f.tick()
  f.coordinator.request()
  f.calls[0].resolve({})
  await settle()
  f.tick()
  f.coordinator.request()
  f.calls[1].resolve({})
  await settle()
  f.tick()
  assert.equal(f.calls.length, 3)
  f.coordinator.dispose()
})

test('disposing before a scheduled refresh prevents any fetch', () => {
  const f = fixture()
  f.coordinator.request()
  f.coordinator.dispose()
  f.tick()
  f.coordinator.request()
  assert.equal(f.calls.length, 0)
  assert.equal(f.timers.size, 0)
})

test('disposing an actor/Corp scope aborts its request and drops trailing work', async () => {
  const f = fixture()
  f.coordinator.request()
  f.tick()
  f.coordinator.request()
  f.coordinator.dispose()
  assert.equal(f.calls[0].signal.aborted, true)
  f.calls[0].resolve({})
  await settle()
  f.tick()
  assert.equal(f.calls.length, 1)
  assert.deepEqual(f.errors, [])
})

test('refresh failures are handled and a later event can still refresh', async () => {
  const f = fixture()
  f.coordinator.request()
  f.tick()
  const failure = new Error('temporary read failure')
  f.calls[0].reject(failure)
  await settle()
  assert.deepEqual(f.errors, [failure])
  assert.equal(f.timers.size, 0, 'No unbounded error-retry loop')
  f.coordinator.request()
  f.tick()
  assert.equal(f.calls.length, 2)
  f.coordinator.dispose()
})

test('a disposed scope does not report an obsolete aborted read as a new UI error', async () => {
  const f = fixture()
  f.coordinator.request()
  f.tick()
  f.coordinator.dispose()
  f.calls[0].reject(new Error('aborted obsolete actor request'))
  await settle()
  assert.deepEqual(f.errors, [])
})

test('websocket events use the coalescer and cleanup cancels its read scope', async () => {
  const app = (await readFile(new URL('./App.tsx', import.meta.url), 'utf8')).replaceAll('\r\n', '\n')
  const effect = app.slice(app.indexOf('    let disposed = false\n    let socket: WebSocket'),
    app.indexOf('  const humans = useMemo'))
  assert.match(effect, /createSnapshotRefresher/)
  assert.match(effect, /snapshotRefresh\.request\(\)/)
  assert.match(effect, /snapshotRefresh\.dispose\(\)/)
  assert.doesNotMatch(effect, /void refresh\(bootstrap\.corp_id, selectedActorId\)/)
  assert.match(app, /if \(signal\?\.aborted\) throw new DOMException/)
})

test('initial connection is bounded, cancellable and offers retry rather than mission restart', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8')
  assert.match(app, /fetch\(`\$\{API_URL\}\/health`, \{ signal: controller\.signal \}\)/)
  assert.match(app, /await refresh\(result\.corp_id, initialActor, controller\.signal\)/)
  assert.match(app, /30_000/)
  assert.match(app, /Retry connection/)
  assert.match(app, /do not restart the mission/)
})

test('actual App presence polling only runs for open connection-dependent views, without reconnecting the event socket', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8')
  const file = ts.createSourceFile('App.tsx', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
  const effects = []
  const visit = (node) => {
    if (ts.isCallExpression(node) && node.expression.getText(file) === 'useEffect') effects.push(node)
    ts.forEachChild(node, visit)
  }
  visit(file)
  const socket = effects.find((node) => node.getText(file).includes('createSnapshotRefresher'))
  assert.ok(socket)
  const polling = effects.find((node) => node.getText(file).includes('setInterval(refreshVisiblePresence'))
  assert.ok(polling)
  assert.notEqual(polling, socket, 'opening a panel must not recreate the actor-scoped WebSocket')
  assert.doesNotMatch(socket.arguments[1].getText(file), /connectionsOpen|journeyOpen|activeWorkspaceView|missionComposerCollapsed/)
  for (const name of ['bootstrap', 'selectedActorId', 'refresh', 'connectionsOpen', 'journeyOpen',
    'activeWorkspaceView', 'missionComposerCollapsed']) {
    assert.ok(polling.arguments[1].getText(file).includes(name), `polling cleanup must track ${name}`)
  }
  assert.match(socket.getText(file), /snapshotRefreshRef\.current = snapshotRefresh/)
  assert.match(socket.getText(file), /snapshotRefreshRef\.current = null/)
  const code = ts.transpileModule(`globalThis.mountPresence = ${polling.arguments[0].getText(file)};`, {
    compilerOptions: { target: ts.ScriptTarget.ES2023, module: ts.ModuleKind.None },
  }).outputText
  const timers = new Map(), listeners = new Map(), refreshes = []
  let nextTimer = 0
  const shared = { request: () => { refreshes.push('same actor-scoped coalescer') } }
  const globals = {
    bootstrap: { corp_id: 'corp-a' }, selectedActorId: 'alice', refresh: () => {},
    connectionsOpen: false, journeyOpen: false, activeWorkspaceView: 'floor',
    missionComposerCollapsed: true, snapshotRefreshRef: { current: shared },
    window: {
      setInterval(callback, delay) { assert.equal(delay, 5_000); timers.set(++nextTimer, callback); return nextTimer },
      clearInterval(id) { timers.delete(id) },
    },
    document: {
      visibilityState: 'visible',
      addEventListener(name, callback) { assert.equal(name, 'visibilitychange'); listeners.set(name, callback) },
      removeEventListener(name, callback) { assert.equal(listeners.get(name), callback); listeners.delete(name) },
    },
  }
  vm.runInNewContext(code, globals)
  const defaults = {
    connectionsOpen: false, journeyOpen: false, activeWorkspaceView: 'floor', missionComposerCollapsed: true,
  }
  for (const state of [
    {}, { activeWorkspaceView: 'room' }, { activeWorkspaceView: 'activity' },
    { activeWorkspaceView: 'factory' }, { activeWorkspaceView: 'missions' },
    { activeWorkspaceView: 'floor', missionComposerCollapsed: false },
  ]) {
    Object.assign(globals, defaults, state)
    assert.equal(globals.mountPresence(), undefined)
    assert.equal(timers.size, 0, 'closed views schedule no periodic complete Corp snapshot')
    assert.equal(listeners.size, 0)
  }
  for (const state of [
    { connectionsOpen: true }, { journeyOpen: true },
    { activeWorkspaceView: 'missions', missionComposerCollapsed: false },
  ]) {
    Object.assign(globals, defaults, state)
    globals.document.visibilityState = 'visible'
    const cleanup = globals.mountPresence()
    assert.equal(typeof cleanup, 'function')
    assert.equal(timers.size, 1)
    const before = refreshes.length
    for (const callback of timers.values()) callback()
    assert.equal(refreshes.length, before + 1)
    globals.document.visibilityState = 'hidden'
    for (const callback of timers.values()) callback()
    listeners.get('visibilitychange')()
    assert.equal(refreshes.length, before + 1, 'background tabs do not poll even with a panel open')
    globals.document.visibilityState = 'visible'
    listeners.get('visibilitychange')()
    assert.equal(refreshes.length, before + 2)
    cleanup()
    assert.equal(timers.size, 0)
    assert.equal(listeners.size, 0)
  }
})
