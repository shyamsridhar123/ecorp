import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { normalizeStatus } from './status.mjs'

const moduleUrl = new URL('./status.mjs', import.meta.url)
const scenarioFile = fileURLToPath(moduleUrl)
const run = (args) => {
  const result = spawnSync(process.execPath, args, { encoding: 'utf8', timeout: 5000 })
  assert.equal(result.error, undefined, result.error?.message)
  return result
}

test('normalizes the four display labels', () => {
  for (const [value, expected] of [
    [' todo ', 'Todo'], ['IN   PROGRESS', 'In Progress'],
    ['\tIn\nReview ', 'In Review'], ['DONE', 'Done'],
  ]) assert.equal(normalizeStatus(value), expected)
})

test('rejects non-strings and unknown or prototype-like labels without coercion', () => {
  for (const value of [undefined, null, 1, true, {}, [], Symbol('x'), '', ' ', '__proto__', 'constructor', 'merged']) {
    assert.throws(() => normalizeStatus(value), TypeError)
  }
})

test('CLI prints canonical output and exits 0', () => {
  const result = run([scenarioFile, ' in   progress '])
  assert.equal(result.status, 0, result.stderr)
  assert.equal(result.stdout, 'In Progress\n')
  assert.equal(result.stderr, '')
})

test('CLI rejects missing, extra and invalid arguments', () => {
  for (const args of [[], ['done', 'extra'], ['invalid']]) {
    const result = run([scenarioFile, ...args])
    assert.equal(result.status, 2)
    assert.equal(result.stdout, '')
    assert.ok(result.stderr.trim())
  }
})

test('importing the module has no CLI side effects', () => {
  const result = run(['--input-type=module', '-e', `await import(${JSON.stringify(moduleUrl.href)})`])
  assert.equal(result.status, 0, result.stderr)
  assert.equal(result.stdout, '')
  assert.equal(result.stderr, '')
})

test('output Error and TypeError diagnostics are preserved without intercepting stderr', () => {
  for (const kind of ['Error', 'TypeError']) {
    const marker = `factory-canary-EIO-${kind}`
    const script = `process.argv=${JSON.stringify(['node', scenarioFile, 'done'])}; process.stdout.write=()=>{const error=new ${kind}(${JSON.stringify(marker)}); error.code='EIO'; throw error;}; await import(${JSON.stringify(moduleUrl.href)});`
    const result = run(['--input-type=module', '-e', script])
    assert.equal(result.status, 1, result.stderr)
    assert.equal(result.stdout, '')
    assert.ok(result.stderr.includes(marker))
    assert.doesNotMatch(result.stderr, /invalid status|ERR_UNSUPPORTED_ESM_URL_SCHEME/)
  }
})
