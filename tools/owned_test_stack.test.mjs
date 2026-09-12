import assert from 'node:assert/strict'
import test from 'node:test'
import {
  assertOwnedRestart, linuxListeningSocketInodes, ownedServerEnvironment,
  parseLinuxProcStat, parseOwnedTestServerManifest,
} from './owned_test_stack.mjs'

// Pure fixtures: no process discovery, signals, services, or filesystem mutation.
const root = 'C:\\fixture-root'
const binary = `${root}\\crony-server.exe`
const server = 'http://127.0.0.1:18965'
const manifest = {
  test_owned: true, workspace: root, server_url: server, server: 4242,
  server_creation: '2026-09-06T22:00:00.000Z',
}
const identity = {
  platform: 'win32', pid: manifest.server, port: 18965,
  executable: binary, creation: manifest.server_creation, port_owned: true,
}
const context = { root, server, binary, platform: 'win32' }

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

test('Windows retains legacy timestamp tolerance and case-insensitive paths', () => {
  assert.doesNotThrow(() => assertOwnedRestart(manifest, {
    ...identity, executable: binary.toUpperCase(), creation: '2026-09-06T22:00:00.020Z',
  }, context))
  assert.throws(() => assertOwnedRestart(manifest, {
    ...identity, creation: '2026-09-06T22:00:00.021Z',
  }, context), /refusing/u)
})

test('legacy Windows identity callers remain supported without weakening explicit checks', () => {
  const legacy = { executable: binary, creation: manifest.server_creation, port_owned: true }
  assert.doesNotThrow(() => assertOwnedRestart(manifest, legacy, context))
  assert.deepEqual(legacy, { executable: binary, creation: manifest.server_creation, port_owned: true })
  for (const change of [
    { platform: 'linux' }, { platform: null }, { pid: 4243 }, { pid: null },
    { port: 18966 }, { port: null }, { port_owned: false },
    { creation: '2026-09-06T22:00:01.000Z' }, { executable: 'C:\\other.exe' },
  ]) assert.throws(() => assertOwnedRestart(manifest, { ...legacy, ...change }, context), /refusing/u)
})

const linuxContext = {
  root: '/work/ecorp', binary: '/work/ecorp/target/debug/crony-server',
  server: 'http://127.0.0.1:18791', platform: 'linux',
}
const linuxManifest = {
  test_owned: true, platform: 'linux', workspace: linuxContext.root,
  server_url: linuxContext.server, server: 4242,
  server_executable: linuxContext.binary,
  server_boot_id: 'c355bc68-49d1-4e81-9d28-50e3e416628e',
  server_start_ticks: '9007199254740993',
}
const linuxIdentity = {
  platform: 'linux', pid: 4242, executable: linuxContext.binary, cwd: linuxContext.root,
  boot_id: linuxManifest.server_boot_id, start_ticks: linuxManifest.server_start_ticks,
  executable_matches: true, cwd_matches: true, root_matches: true,
  port: 18791, port_owned: true, socket_inodes: ['12345'],
}

test('Linux requires the exact boot/PID/start-tick, executable, cwd/root, and socket binding', () => {
  assert.doesNotThrow(() => assertOwnedRestart(linuxManifest, linuxIdentity, linuxContext))
  for (const change of [
    { pid: 4243 }, { platform: 'win32' },
    { pid: undefined }, { platform: undefined }, { port: undefined }, { cwd: undefined },
    { boot_id: undefined }, { start_ticks: undefined },
    { boot_id: 'c355bc68-49d1-4e81-9d28-50e3e416628f' },
    { start_ticks: '9007199254740992' }, { start_ticks: 9007199254740992 },
    { executable: '/work/ecorp/target/debug/other' }, { executable_matches: false },
    { executable: linuxContext.binary.toUpperCase() },
    { cwd: '/work/ecorp-other' }, { cwd_matches: false }, { root_matches: false },
    { port: 8791 }, { port_owned: false }, { socket_inodes: [] }, { socket_inodes: undefined },
    { socket_inodes: ['0'] }, { socket_inodes: ['not-an-inode'] },
  ]) assert.throws(() => assertOwnedRestart(linuxManifest, {
    ...linuxIdentity, ...change,
  }, linuxContext), /refusing/u)
})

test('Linux rejects malformed, legacy-Windows, and changed receipts', () => {
  for (const change of [
    { test_owned: false }, { platform: undefined }, { platform: 'win32' },
    { server: 0 }, { server: -1 }, { server: '4242' }, { server: 4242.5 },
    { server: Number.MAX_SAFE_INTEGER + 1 }, { workspace: null },
    { workspace: '/work/ecorp-other' }, { server_executable: '/bin/other' },
    { server_boot_id: '' }, { server_boot_id: ['c355bc68-49d1-4e81-9d28-50e3e416628e'] },
    { server_start_ticks: '01' }, { server_start_ticks: '-1' },
    { server_start_ticks: '18446744073709551616' }, { server_start_ticks: null },
    { server_url: 'http://127.0.0.1:18792' },
  ]) assert.throws(() => assertOwnedRestart({
    ...linuxManifest, ...change,
  }, linuxIdentity, linuxContext), /refusing/u)
  assert.throws(() => assertOwnedRestart(manifest, linuxIdentity, linuxContext), /refusing/u)
})

test('every existing manual port remains denied on Windows and Linux and all loopback hosts', () => {
  for (const [receipt, processIdentity, options] of [
    [manifest, identity, context], [linuxManifest, linuxIdentity, linuxContext],
  ]) {
    for (const port of ['8791', '8793', '5187', '5291', '15191', '15193']) {
      for (const host of ['127.0.0.1', 'localhost', '[::1]']) {
        const endpoint = `http://${host}:${port}`
        assert.throws(() => assertOwnedRestart(
          { ...receipt, server_url: endpoint },
          { ...processIdentity, port: Number(port) },
          { ...options, server: endpoint },
        ), /refusing/u)
      }
    }
  }
})

test('remote, credentialed, missing-port, and malformed-scope endpoints fail closed', () => {
  for (const endpoint of [
    'http://example.com:18791', 'http://0.0.0.0:18791', 'http://[::]:18791',
    'http://user:password@127.0.0.1:18791', 'http://127.0.0.1',
    'http://127.0.0.1:0', 'ftp://127.0.0.1:18791', 'http://127.0.0.1:18791/other',
    'http://127.0.0.1:18791?bypass=true', 'http://127.0.0.1:18791#fragment',
    'http://127.0.0.1:08791', 'not-a-url', 'http://user:password@127.0.0.1:bad-port',
  ]) assert.throws(() => assertOwnedRestart(
    { ...linuxManifest, server_url: endpoint }, linuxIdentity, { ...linuxContext, server: endpoint },
  ), /refusing/u)
  for (const platform of ['darwin', 'freebsd', 'unknown']) {
    assert.throws(() => assertOwnedRestart(linuxManifest, linuxIdentity, {
      ...linuxContext, platform,
    }), /refusing/u)
  }
})

test('only a complete scoped JSON object is a process receipt', () => {
  assert.deepEqual(parseOwnedTestServerManifest(JSON.stringify(linuxManifest), linuxContext), linuxManifest)
  for (const text of [undefined, null, '', '{', '4242', 'null', '[]', '{}', '{"test_owned":true}', ' '.repeat(16_385)]) {
    assert.throws(() => parseOwnedTestServerManifest(text, linuxContext), /refusing/u)
  }
})

function procStat({ pid = 4242, comm = 'crony-server', state = 'S', ticks = '9007199254740993' } = {}) {
  const fields = Array(50).fill('0')
  fields[0] = state // field 3
  fields[19] = ticks // field 22, not a JavaScript Number
  return `${pid} (${comm}) ${fields.join(' ')}\n`
}

test('proc stat parses the final comm parenthesis and preserves 64-bit start ticks', () => {
  for (const comm of ['crony-server', 'worker (a))', 'a) (b)\n c']) {
    assert.deepEqual(parseLinuxProcStat(procStat({ comm }), 4242), {
      pid: 4242, state: 'S', start_ticks: '9007199254740993',
    })
  }
  assert.equal(parseLinuxProcStat(procStat({ ticks: '18446744073709551615' }), 4242).start_ticks,
    '18446744073709551615')
})

test('proc stat rejects wrong PID, truncated fields, malformed state, and invalid ticks', () => {
  for (const text of [
    '', '4242 (broken S 0', '4242 (worker) S 0', procStat({ pid: 4243 }),
    procStat({ state: 'unknown' }), procStat({ ticks: '-1' }), procStat({ ticks: '1x' }),
    procStat({ ticks: '18446744073709551616' }), procStat().replace(') S', ')S'),
    procStat() + 'garbage',
  ]) assert.throws(() => parseLinuxProcStat(text, 4242), /refusing/u)
})

const tcpHeader = ' sl local_address rem_address st tx_queue rx_queue tr tm->when retrnsmt uid timeout inode\n'
function tcpRow({ ipv6 = false, port = 18791, inode = '12345', state = '0A' } = {}) {
  const local = ipv6 ? '00000000000000000000000001000000' : '0100007F'
  return `0: ${local}:${port.toString(16).padStart(4, '0')} ${'0'.repeat(local.length)}:0000 ` +
    `${state} 00000000:00000000 00:00000000 00000000 1000 0 ${inode} 1 0000000000000000\n`
}

test('Linux listener proof intersects the exact port and LISTEN state with PID-owned inodes', () => {
  const sample = {
    tcp: tcpHeader + tcpRow(), tcp6: tcpHeader + tcpRow({ ipv6: true, inode: '54321' }),
    socketLinks: ['socket:[12345]', 'socket:[54321]', 'pipe:[99]'], port: 18791,
  }
  assert.deepEqual(linuxListeningSocketInodes(sample), ['12345', '54321'])
  for (const change of [
    { port: 18792 }, { socketLinks: ['socket:[99999]'] },
    { tcp: tcpHeader + tcpRow({ state: '01' }), tcp6: tcpHeader },
    { tcp: tcpHeader + tcpRow({ port: 8791 }), tcp6: tcpHeader },
  ]) assert.deepEqual(linuxListeningSocketInodes({ ...sample, ...change }), [])
})

test('malformed or unavailable proc network input supplies no ownership authority', () => {
  const sample = { tcp: tcpHeader, tcp6: tcpHeader, socketLinks: [], port: 18791 }
  for (const change of [
    { tcp: undefined }, { tcp6: undefined }, { tcp: '' },
    { tcp: tcpHeader + '0: truncated' }, { tcp: tcpHeader + tcpRow({ inode: 'bad' }) },
    { tcp: tcpHeader + tcpRow({ ipv6: true }) },
    { socketLinks: [123] }, { socketLinks: ['socket:[bad]'] }, { port: 0 },
  ]) assert.throws(() => linuxListeningSocketInodes({ ...sample, ...change }), /refusing/u)
})

test('artifact recovery overrides are child-only and match native numeric bounds', () => {
  const inherited = { PATH: 'fixture-path', DATABASE_URL: 'old-fixture', KEEP: 'unchanged' }
  const environment = {
    CRONY_ARTIFACT_RECOVERY_GRACE_SECS: '0', CRONY_ARTIFACT_RECOVERY_INTERVAL_SECS: '1',
  }
  assert.deepEqual(ownedServerEnvironment(environment, 'new-fixture', inherited), {
    ...inherited, ...environment, DATABASE_URL: 'new-fixture',
  })
  assert.deepEqual(inherited, { PATH: 'fixture-path', DATABASE_URL: 'old-fixture', KEEP: 'unchanged' })
  assert.deepEqual(ownedServerEnvironment(undefined, 'new-fixture', inherited), {
    ...inherited, DATABASE_URL: 'new-fixture',
  })
  for (const options of [
    { PATH: 'other' }, { DATABASE_URL: 'other' }, { NODE_OPTIONS: '--require=other' },
    { CRONY_ARTIFACT_RECOVERY_GRACE_SECS: '-1' },
    { CRONY_ARTIFACT_RECOVERY_GRACE_SECS: '3601' },
    { CRONY_ARTIFACT_RECOVERY_INTERVAL_SECS: '0' },
    { CRONY_ARTIFACT_RECOVERY_INTERVAL_SECS: '3601' },
    { CRONY_ARTIFACT_RECOVERY_INTERVAL_SECS: 1 },
    { CRONY_ARTIFACT_RECOVERY_INTERVAL_SECS: '1; command' },
  ]) assert.throws(() => ownedServerEnvironment(options, 'fixture', inherited), /refusing/u)
})
