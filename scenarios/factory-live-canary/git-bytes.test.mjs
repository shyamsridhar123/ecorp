import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

const repository = fileURLToPath(new URL('../../', import.meta.url))
const nullDevice = process.platform === 'win32' ? 'NUL' : '/dev/null'
const gitEnv = {
  PATH: process.env.PATH,
  SystemRoot: process.env.SystemRoot,
  GIT_CONFIG_NOSYSTEM: '1',
  GIT_CONFIG_GLOBAL: nullDevice,
  GIT_TERMINAL_PROMPT: '0',
  GIT_OPTIONAL_LOCKS: '0',
}
const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex')
const git = (args, input) => {
  const result = spawnSync('git', ['-c', `core.attributesFile=${nullDevice}`, ...args], {
    cwd: repository,
    env: gitEnv,
    input,
    timeout: 5000,
    maxBuffer: 65536,
    windowsHide: true,
  })
  assert.equal(result.error, undefined, result.error?.message)
  assert.equal(result.signal, null)
  assert.equal(result.status, 0, result.stderr?.toString())
  return result.stdout
}

// These are issue #225's original requirements, not hashes of a normalized copy.
// Keep Git-dependent checks separate from the six portable behavior tests.
for (const [name, expected] of [
  ['status.mjs', 'df8af8ce2c756d230c1d303c2121c88a8495c608c3616592504b42a3d47c520b'],
  ['README.md', '123cc52972ec592813f99069b97b79423bff1b04fa9025822327190fd646fa8e'],
]) {
  const path = `scenarios/factory-live-canary/${name}`

  test(`${name}: raw staged Git blob meets the unchanged byte contract`, () => {
    // cat-file returns raw bytes: no checkout, text decoding, or newline filter.
    const blob = git(['cat-file', 'blob', `:${path}`])
    assert.equal(sha256(blob), expected, `${path}: staged Git SHA-256`)
    assert.deepEqual(readFileSync(new URL(name, import.meta.url)), blob,
      `${path}: working copy must equal the raw staged blob`)
  })

  test(`${name}: Git keeps the literal bytes with autocrlf false, true, and input`, () => {
    assert.equal(
      git(['check-attr', '--cached', 'text', '--', path]).toString().trim(),
      `${path}: text: unset`,
      'The staged attributes must disable newline normalization for this exact file',
    )
    const bytes = readFileSync(new URL(name, import.meta.url))
    assert.equal(sha256(bytes), expected)
    const rawObject = git(['hash-object', '--no-filters', '--stdin'], bytes).toString().trim()
    for (const setting of ['false', 'true', 'input']) {
      const cleanObject = git([
        '-c', `core.autocrlf=${setting}`, 'hash-object', `--path=${path}`, '--stdin',
      ], bytes).toString().trim()
      assert.equal(cleanObject, rawObject, `${path}: core.autocrlf=${setting}`)
      const checkoutBytes = git([
        '-c', `core.autocrlf=${setting}`, 'cat-file', '--filters', `--path=${path}`, `:${path}`,
      ])
      assert.deepEqual(checkoutBytes, bytes, `${path}: checkout with core.autocrlf=${setting}`)
    }
  })
}
