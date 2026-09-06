import assert from 'node:assert/strict'
import test from 'node:test'
import { inspectNativeReads, nativeReadRole } from './copilot_probe_native_read.mjs'

const workspace = String.raw`C:\fixture\worktree`
const expectedText = 'ECorp native read marker: test-only-indigo-owl\n'
const options = { sessionId: 'session-1', workspace, expectedText }
const session = { type: 'session.start', data: { sessionId: 'session-1' } }
function call(id, file, success = true, content = expectedText) {
  return [
    { type: 'tool.execution_start', data: { toolCallId: id, toolName: 'view', arguments: { path: file } } },
    { type: 'tool.execution_complete', data: { toolCallId: id, success, result: { content } } },
  ]
}
const complete = () => [
  session,
  ...call('one', 'native-read-seed.txt'),
  ...call('two', `${workspace}\\native-read-seed.txt`),
  ...call('three', 'native-readback.txt'),
]

test('actual successful relative, absolute and round-trip view results are required', () => {
  assert.equal(inspectNativeReads(complete(), options).complete, true)
})

test('provider self-report and final file existence are not native-read evidence', () => {
  const result = inspectNativeReads([session,
    { type: 'assistant.message', data: { content: expectedText } },
  ], options)
  assert.equal(result.complete, false)
  assert.equal(result.missing_roles.length, 3)
})

test('a missing absolute read or readback cannot pass', () => {
  const result = inspectNativeReads([session, ...call('one', 'native-read-seed.txt')], options)
  assert.equal(result.complete, false)
  assert.deepEqual(result.missing_roles, ['seed_absolute', 'readback'])
})

test('missing-file errors and successful-but-wrong content remain failures', () => {
  for (const [success, content] of [[false, expectedText], [true, 'Path does not exist']]) {
    const result = inspectNativeReads([
      ...complete(),
      ...call('failed', 'native-readback.txt', success, content),
    ], options)
    assert.equal(result.complete, false)
  }
})

test('wrong-session or conflicting tool results cannot manufacture evidence', () => {
  assert.throws(() => inspectNativeReads(complete(), { ...options, sessionId: 'other' }), /session/u)
  assert.throws(() => inspectNativeReads([
    ...complete(), ...call('one', 'native-readback.txt'),
  ], options), /conflicting/u)
})

test('an external path with the same filename is not the assigned seed', () => {
  assert.equal(nativeReadRole(String.raw`C:\outside\native-read-seed.txt`, workspace), null)
  assert.equal(nativeReadRole('../native-read-seed.txt', workspace), null)
  assert.equal(nativeReadRole(String.raw`\\?\C:\fixture\worktree\native-read-seed.txt`, workspace),
    'seed_absolute')
})

test('an incomplete stream is explicitly incomplete, not successful empty coverage', () => {
  assert.equal(inspectNativeReads([], options).complete, false)
  assert.equal(inspectNativeReads([session], options).complete, false)
})
