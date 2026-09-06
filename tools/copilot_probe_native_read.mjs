import assert from 'node:assert/strict'
import path from 'node:path'

export const NATIVE_READ_SEED = 'native-read-seed.txt'
export const NATIVE_READBACK = 'native-readback.txt'
const requiredRoles = ['seed_relative', 'seed_absolute', 'readback']

function normalize(value, windows) {
  const normalized = (windows ? path.win32 : path.posix).normalize(value)
  return windows
    ? normalized.replace(/^\\\\\?\\(?=[a-z]:\\)/iu, '').toLowerCase()
    : normalized
}

export function nativeReadRole(raw, workspace) {
  if (typeof raw !== 'string' || raw.split(/[\\/]/u).includes('..')) return null
  const windows = /^[a-z]:[\\/]|^\\\\/iu.test(workspace)
  const paths = windows ? path.win32 : path.posix
  const normalized = normalize(raw, windows)
  if (normalized === NATIVE_READ_SEED) return 'seed_relative'
  if (normalized === normalize(paths.join(workspace, NATIVE_READ_SEED), windows)) {
    return 'seed_absolute'
  }
  if (normalized === NATIVE_READBACK ||
      normalized === normalize(paths.join(workspace, NATIVE_READBACK), windows)) {
    return 'readback'
  }
  return null
}

function resultText(result) {
  if (typeof result === 'string') return result
  if (typeof result?.content === 'string') return result.content
  if (Array.isArray(result?.content)) {
    return result.content.map((entry) => typeof entry?.text === 'string' ? entry.text : '').join('\n')
  }
  return ''
}

export function inspectNativeReads(events, { sessionId, workspace, expectedText }) {
  assert.ok(typeof expectedText === 'string' && expectedText.trim().length > 0,
    'a non-empty source marker is required')
  const sessions = events.filter((event) => event.type === 'session.start')
  if (sessions.length === 0) {
    return { session_matched: false, calls: [], missing_roles: [...requiredRoles], complete: false }
  }
  assert.ok(sessions.every((event) => event.data?.sessionId === sessionId),
    'native read events must belong to the exact provider session')
  const starts = new Map()
  const completed = new Map()
  for (const event of events) {
    if (event.type === 'tool.execution_start' && event.data?.toolName === 'view') {
      const role = nativeReadRole(event.data.arguments?.path, workspace)
      if (!role) continue
      const previous = starts.get(event.data.toolCallId)
      assert.ok(!previous || previous === role, 'conflicting native tool-call identity')
      starts.set(event.data.toolCallId, role)
    } else if (event.type === 'tool.execution_complete') {
      const role = starts.get(event.data?.toolCallId)
      if (!role) continue
      const result = {
        role,
        tool_call_id: event.data.toolCallId,
        success: event.data.success === true,
        content_matches_expected: resultText(event.data.result).includes(expectedText.trimEnd()),
        error_code: event.data.error?.code ?? null,
      }
      const previous = completed.get(result.tool_call_id)
      assert.ok(!previous || JSON.stringify(previous) === JSON.stringify(result),
        'conflicting native tool completion')
      completed.set(result.tool_call_id, result)
    }
  }
  const calls = [...completed.values()]
  const successful = new Set(calls.filter((call) => call.success && call.content_matches_expected)
    .map((call) => call.role))
  const missing = requiredRoles.filter((role) => !successful.has(role))
  return {
    session_matched: true,
    calls,
    missing_roles: missing,
    complete: missing.length === 0 && calls.every((call) => call.success && call.content_matches_expected),
  }
}
