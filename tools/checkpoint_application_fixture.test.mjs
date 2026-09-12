import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, realpath, rm, readFile, writeFile, link } from 'node:fs/promises'
import { createHash } from 'node:crypto'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import { APPLICATION_ROOT, applicationFiles, writeCheckpointApplication } from '../scripts/checkpoint-application-fixture.mjs'
import { readTrustedExecutableDigest } from './e2e_checkpoint_verification.mjs'
import { checkContainedFile } from './e2e_stopped_source_checkpoint.mjs'

const execute = promisify(execFile)

test('checkpoint fixture produces a complete, independently tested local application', async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'ecorp-checkpoint-app-'))
  const canonicalRoot = await realpath(root)
  const canonicalTemp = await realpath(tmpdir())
  assert.equal(path.dirname(canonicalRoot), canonicalTemp)
  assert.ok(path.basename(canonicalRoot).startsWith('ecorp-checkpoint-app-'))
  try {
    writeCheckpointApplication(root)
    const app = path.join(root, APPLICATION_ROOT)
    for (const [file, expected] of Object.entries(applicationFiles)) {
      assert.equal(await readFile(path.join(app, file), 'utf8'), expected)
    }
    const result = await execute(process.execPath, ['--test', '--test-reporter=tap', 'app.test.mjs'], {
      cwd: app, windowsHide: true, timeout: 30_000,
      env: { ...process.env, NODE_TEST_CONTEXT: undefined },
    })
    assert.match(result.stdout, /# pass 4/)
    assert.match(result.stdout, /# fail 0/)
    assert.match(applicationFiles['index.html'], /from '\.\/app\.mjs'/)
    assert.doesNotMatch(applicationFiles['index.html'], /https?:\/\//)
    assert.match(applicationFiles['index.html'], /aria-live="polite"/)
  } finally {
    // Only the canonical direct child created by this test is removed.
    assert.equal(await realpath(root), canonicalRoot)
    await rm(canonicalRoot, { recursive: true })
  }
})

test('read-only executable hashing accepts Cargo/Git hardlinks without relaxing source isolation', async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'ecorp-checkpoint-app-'))
  const canonicalRoot = await realpath(root)
  assert.equal(path.dirname(canonicalRoot), await realpath(tmpdir()))
  assert.ok(path.basename(canonicalRoot).startsWith('ecorp-checkpoint-app-'))
  try {
    const binary = path.join(canonicalRoot, 'fixture.exe')
    const alias = path.join(canonicalRoot, 'build-output.exe')
    const bytes = Buffer.from('hash-only fixture; never executed')
    await writeFile(binary, bytes)
    await link(binary, alias)
    assert.equal(await readTrustedExecutableDigest(alias), createHash('sha256').update(bytes).digest('hex'))
    await assert.rejects(checkContainedFile(canonicalRoot, alias), /regular_single_link_file_required/)
    assert.throws(() => readTrustedExecutableDigest(path.join(root, 'source.md')), /explicit_executable_required/)
    assert.deepEqual(await readFile(binary), bytes)
  } finally {
    assert.equal(await realpath(root), canonicalRoot)
    await rm(canonicalRoot, { recursive: true })
  }
})
