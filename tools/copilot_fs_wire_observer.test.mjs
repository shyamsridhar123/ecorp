import assert from 'node:assert/strict'
import test from 'node:test'
import { createFsWireObserver } from './copilot_fs_wire_observer.mjs'

function frame(value) {
  const body = Buffer.from(JSON.stringify(value))
  return Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`), body])
}

test('framed stat metadata survives arbitrary chunk boundaries without payload disclosure', () => {
  const rows = []
  const observer = createFsWireObserver((row) => rows.push(row))
  const secret = 'DO-NOT-RECORD-this-test-payload'
  const request = frame({ id: 1, method: 'sessionFs.stat', params: { path: secret } })
  for (const byte of request) observer.fromCli(Buffer.from([byte]))
  observer.fromSdk(frame({ id: 1, result: {
    isFile: true, isDirectory: false, mtime: '2026-09-06T00:00:00Z',
    birthtime: '2026-09-06T00:00:00Z', size: 4, privateContent: secret,
  } }))
  assert.equal(rows.length, 2)
  assert.equal(rows[1].is_file_camel, true)
  assert.equal(rows[1].mtime_valid, true)
  assert.ok(!JSON.stringify(rows).includes(secret))
})

test('unrelated RPCs and read-file contents are not recorded', () => {
  const rows = []
  const observer = createFsWireObserver((row) => rows.push(row))
  observer.fromCli(frame({ id: 2, method: 'sessionFs.readFile', params: { path: 'private' } }))
  observer.fromSdk(frame({ id: 2, result: { content: 'private bytes' } }))
  observer.fromCli(frame({ id: 3, method: 'authentication.getToken' }))
  observer.fromSdk(frame({ id: 3, result: { token: 'private token' } }))
  assert.deepEqual(rows, [])
})

test('a response ID type change is visible without recording identifier values', () => {
  const rows = []
  const observer = createFsWireObserver((row) => rows.push(row))
  observer.fromCli(frame({ id: '4', method: 'sessionFs.exists' }))
  observer.fromSdk(frame({ id: 4, result: { exists: true } }))
  assert.equal(rows[1].id_type_matches, false)
  assert.equal(rows[1].exists, true)
  assert.equal(Object.hasOwn(rows[1], 'id'), false)
})

test('oversized frames are skipped without parsing nested payload text as requests', () => {
  const rows = []
  const observer = createFsWireObserver((row) => rows.push(row), 120)
  observer.fromCli(frame({ id: 5, method: 'other', payload: 'x'.repeat(200) }))
  observer.fromCli(frame({ id: 6, method: 'sessionFs.stat' }))
  observer.fromSdk(frame({ id: 6, result: { isFile: true } }))
  assert.equal(rows[0].reason, 'oversized_frame_skipped')
  assert.equal(rows.at(-1).is_file_camel, true)
})
