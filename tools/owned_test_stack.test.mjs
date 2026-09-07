import assert from 'node:assert/strict'
import test from 'node:test'
import path from 'node:path'
import { assertOwnedRestart } from './owned_test_stack.mjs'

const root = path.resolve('fixture-root')
const binary = path.join(root, 'crony-server.exe')
const server = 'http://127.0.0.1:18965'
const manifest = {
  test_owned: true, workspace: root, server_url: server, server: 4242,
  server_creation: '2026-09-06T22:00:00.000Z',
}
const identity = { executable: binary, creation: manifest.server_creation, port_owned: true }
const context = { root, server, binary }

test('owned server requires matching executable, creation, endpoint, and workspace', () => {
  assert.doesNotThrow(() => assertOwnedRestart(manifest, identity, context))
  for (const changed of [
    { ...identity, executable: path.join(root, 'other-server.exe') },
    { ...identity, creation: '2026-09-06T22:00:01.000Z' },
    { ...identity, creation: 'invalid' },
    { ...identity, port_owned: false },
  ]) {
    assert.throws(() => assertOwnedRestart(manifest, changed, context), /refusing/u)
  }
})

test('manual, remote, stale, or non-owned manifests never authorize restart', () => {
  for (const changed of [
    { ...manifest, test_owned: false },
    { ...manifest, server: 0 },
    { ...manifest, server_creation: null },
    { ...manifest, workspace: path.resolve('other-workspace') },
    { ...manifest, server_url: 'http://127.0.0.1:8791' },
  ]) {
    assert.throws(() => assertOwnedRestart(changed, identity, context), /refusing/u)
  }
  for (const endpoint of ['http://127.0.0.1:8791', 'http://example.com:18965']) {
    assert.throws(
      () => assertOwnedRestart({ ...manifest, server_url: endpoint }, identity, { ...context, server: endpoint }),
      /refusing/u,
    )
  }
})
