import assert from 'node:assert/strict'
import test from 'node:test'
import path from 'node:path'
import net from 'node:net'
import os from 'node:os'
import { existsSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import { assertOwnedRestart, assertTestEndpoint, ownedServerEnvironment, restartOwnedTestServer, startOwnedTestServer, stopOwnedTestServer } from './owned_test_stack.mjs'

// Pure fixtures: no process discovery, signals, services, or filesystem mutation.
const root = 'C:\\fixture-root'
const binary = `${root}\\crony-server.exe`
const server = 'http://127.0.0.1:18965'
const manifest = {
  test_owned: true, workspace: root, server_url: server, server: 4242,
  server_creation: '2026-09-06T22:00:00.000Z',
}
const identity = { platform: 'win32', pid: manifest.server, executable: binary, creation: manifest.server_creation, port_owned: true }
const context = { root, server, binary, platform: 'win32' }

test('upstream restart environment remains bounded and cannot override database authority', () => {
  const inherited = { PATH: 'fixture', DATABASE_URL: 'old-private-value' }
  const changed = ownedServerEnvironment({ CRONY_ARTIFACT_RECOVERY_GRACE_SECS: '0',
    CRONY_ARTIFACT_RECOVERY_INTERVAL_SECS: '1' }, 'new-private-value', inherited)
  assert.deepEqual(changed, { PATH: 'fixture', DATABASE_URL: 'new-private-value',
    CRONY_ARTIFACT_RECOVERY_GRACE_SECS: '0', CRONY_ARTIFACT_RECOVERY_INTERVAL_SECS: '1' })
  assert.equal(inherited.DATABASE_URL, 'old-private-value')
  for (const environment of [null, [], { DATABASE_URL: 'canary' }, { NODE_OPTIONS: '--require=canary' },
    { CRONY_ARTIFACT_RECOVERY_INTERVAL_SECS: '0' }, { CRONY_ARTIFACT_RECOVERY_GRACE_SECS: '3601' },
    { CRONY_ARTIFACT_RECOVERY_INTERVAL_SECS: 1 }, { CRONY_ARTIFACT_RECOVERY_GRACE_SECS: '-1' },
    { CRONY_ARTIFACT_RECOVERY_GRACE_SECS: '00' }, { CRONY_ARTIFACT_RECOVERY_GRACE_SECS: '1.5' }]) {
    assert.throws(() => ownedServerEnvironment(environment, 'private-value'), /refusing/u)
  }
})

test('owned server requires matching executable, creation, endpoint, and workspace', () => {
  assert.doesNotThrow(() => assertOwnedRestart(manifest, identity, context))
  for (const changed of [
    { ...identity, executable: `${root}\\other-server.exe` },
    { ...identity, creation: '2026-09-06T22:00:01.000Z' },
    { ...identity, creation: 'invalid' },
    { ...identity, port_owned: false },
    { ...identity, port_owned: 'true' },
    { ...identity, port: 18966 },
    { ...identity, pid: 4243 },
  ]) {
    assert.throws(() => assertOwnedRestart(manifest, changed, context), /refusing/u)
  }
})

test('manual, remote, stale, or non-owned manifests never authorize restart', () => {
  for (const changed of [
    { ...manifest, test_owned: false },
    { ...manifest, server: 0 },
    { ...manifest, server_creation: null },
    { ...manifest, workspace: 'C:\\other-workspace' },
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

test('owned endpoints reject every reserved/manual port and non-origin URL', () => {
  for (const address of [
    ...[5432, 54329, 8791, 8793, 5187, 5291, 15191, 15193].map(port => `http://127.0.0.1:${port}`),
    'https://127.0.0.1:18965', 'http://example.com:18965', 'http://user:secret@127.0.0.1:18965',
    'http://127.0.0.1:18965/path', 'http://127.0.0.1:18965/?query', 'http://127.0.0.1:18965/#fragment',
    'http://127.0.0.1',
  ]) assert.throws(() => assertTestEndpoint(address), /refusing/u)
})

test('Linux receipts require exact kernel identity and case-sensitive executable paths', () => {
  const linux = { platform: 'linux', pid: 4242, executable: '/fixture/server', cwd: '/fixture', uid: 1000,
    creation: manifest.server_creation, port_owned: true, start_ticks: '100001',
    boot_id: '00000000-0000-4000-8000-000000000001', network_namespace: 'net:[12345]' }
  const record = { ...manifest, workspace: '/fixture', server_state: 'running', server_identity: linux }
  const settings = { root: '/fixture', server, binary: '/fixture/server', platform: 'linux' }
  assert.doesNotThrow(() => assertOwnedRestart(record, linux, settings))
  for (const changed of [
    { ...linux, executable: '/fixture/SERVER' }, { ...linux, cwd: '/other' }, { ...linux, pid: 4243 },
    { ...linux, boot_id: '00000000-0000-4000-8000-000000000002' }, { ...linux, start_ticks: '100002' },
    { ...linux, uid: 1001 }, { ...linux, network_namespace: 'net:[12346]' }, { ...linux, port_owned: false },
  ]) assert.throws(() => assertOwnedRestart(record, changed, settings), /refusing/u)
  for (const changed of [
    { ...record, server_identity: undefined }, { ...record, server_state: 'starting' },
    { ...record, server_state: 'stopped' }, { ...record, test_owned: false },
  ]) assert.throws(() => assertOwnedRestart(changed, linux, settings), /refusing/u)
})

test('legacy PID files and pre-existing locks are preserved without signalling or starting a process', async () => {
  const folder = mkdtempSync(path.join(os.tmpdir(), 'ecorp-owned-test-'))
  const manifestPath = path.join(folder, 'server.json')
  const settings = { root: folder, server, binary: process.execPath, manifestPath,
    databaseUrl: 'postgres://fixture:sentinel@127.0.0.1:55471/fixture' }
  try {
    writeFileSync(manifestPath, '4242\n')
    await assert.rejects(restartOwnedTestServer(settings), /refusing/u)
    assert.equal(readFileSync(manifestPath, 'utf8'), '4242\n')
    writeFileSync(manifestPath + '.lock', 'owned-by-another-operation')
    await assert.rejects(restartOwnedTestServer(settings), /EEXIST/u)
    assert.equal(readFileSync(manifestPath + '.lock', 'utf8'), 'owned-by-another-operation')
    await assert.rejects(startOwnedTestServer({ ...settings, manifestPath: path.join(folder, 'other.json'),
      server: 'http://127.0.0.1:8791' }), /refusing/u)
    assert.equal(existsSync(path.join(folder, 'other.json')), false)
  } finally {
    assert.equal(path.dirname(realpathSync(folder)), realpathSync(os.tmpdir()))
    assert.match(path.basename(folder), /^ecorp-owned-test-[a-zA-Z0-9]+$/u)
    rmSync(folder, { recursive: true })
  }
})

test('native owned child startup, two restarts, refusal of database drift and idempotent stop', {
  skip: process.env.ECORP_OWNED_PROCESS_TEST !== '1' || !['linux', 'win32'].includes(process.platform),
  timeout: 240_000,
}, async t => {
  const folder = mkdtempSync(path.join(os.tmpdir(), 'ecorp-owned-test-'))
  const manifestPath = path.join(folder, 'server.json')
  const listener = net.createServer()
  await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve))
  const endpoint = `http://127.0.0.1:${listener.address().port}`
  const settings = { root: folder, server: endpoint, binary: process.execPath, manifestPath,
    args: [path.join(import.meta.dirname, 'fixtures', 'owned_test_server.mjs')],
    databaseUrl: 'postgres://fixture:sentinel@127.0.0.1:55471/fixture' }
  let stopped = false
  try {
    // A pre-existing listener is never adopted or killed, even on an allowed port.
    await assert.rejects(startOwnedTestServer(settings), /EADDRINUSE/u)
    assert.equal(existsSync(manifestPath), false)
    assert.equal(listener.listening, true)
    await new Promise(resolve => listener.close(resolve))
    const first = await startOwnedTestServer(settings)
    const initial = JSON.parse(readFileSync(manifestPath, 'utf8'))
    assert.equal(initial.server, first)
    assert.equal(initial.server_state, 'running')
    assert.equal(initial.test_owned, true)
    assert.equal(initial.server_identity.port_owned, true)
    assert.equal(readFileSync(manifestPath, 'utf8').includes('sentinel'), false)
    await assert.rejects(startOwnedTestServer(settings), /already exists/u)
    await assert.rejects(restartOwnedTestServer({ ...settings,
      databaseUrl: 'postgres://fixture:sentinel@127.0.0.1:55472/other' }), /database target changed/u)
    assert.deepEqual(JSON.parse(readFileSync(manifestPath, 'utf8')), initial)
    const second = await restartOwnedTestServer(settings)
    const third = await restartOwnedTestServer(settings)
    assert.notEqual(first, second)
    assert.notEqual(second, third)
    const current = JSON.parse(readFileSync(manifestPath, 'utf8'))
    assert.equal(current.previous_server_pid, second)
    assert.equal(current.server, third)
    assert.equal(current.server_state, 'running')
    assert.equal(current.server_identity.port_owned, true)
    assert.equal(await stopOwnedTestServer(settings), true)
    stopped = true
    assert.equal(await stopOwnedTestServer(settings), false)
    assert.equal(JSON.parse(readFileSync(manifestPath, 'utf8')).server_state, 'stopped')
  } finally {
    if (listener.listening) await new Promise(resolve => listener.close(resolve))
    if (!stopped && existsSync(manifestPath)) {
      try { stopped = await stopOwnedTestServer(settings) } catch {
        t.diagnostic('Unverifiable test process/receipt preserved at ' + folder)
      }
    }
    if (stopped || !existsSync(manifestPath)) {
      assert.equal(path.dirname(realpathSync(folder)), realpathSync(os.tmpdir()))
      assert.match(path.basename(folder), /^ecorp-owned-test-[a-zA-Z0-9]+$/u)
      rmSync(folder, { recursive: true })
    }
  }
})
