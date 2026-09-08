import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
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
