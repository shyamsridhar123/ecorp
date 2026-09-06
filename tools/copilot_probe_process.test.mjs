import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'

const observer = path.join(import.meta.dirname, 'copilot_probe_process.mjs')
const canary = 'ECORP_CREDENTIAL_CANARY_MUST_NOT_REACH_COPILOT'

async function observe(role, seeded) {
  const root = await mkdtemp(path.join(os.tmpdir(), 'ecorp-env-test-'))
  const observations = path.join(root, 'observations.jsonl')
  const env = {
    ...process.env,
    ECORP_COPILOT_PROBE_ID: 'deterministic-environment-test',
    ECORP_COPILOT_ENV_OBSERVATIONS: observations,
  }
  delete env.GH_TOKEN
  delete env.GITHUB_TOKEN
  if (seeded) env.GITHUB_TOKEN = canary
  const child = spawn(process.execPath, [observer, role, process.execPath, '-e', 'process.exit(0)'], {
    windowsHide: true, env, stdio: ['ignore', 'pipe', 'pipe'],
  })
  let stderr = ''
  child.stderr.on('data', (chunk) => { stderr += chunk })
  child.stdout.resume()
  const [code] = await once(child, 'close')
  const records = (await readFile(observations, 'utf8')).trim().split(/\r?\n/u).map(JSON.parse)
  await rm(root, { recursive: true })
  return { code, stderr, records }
}

test('runner observation proves the synthetic credential was inherited', async () => {
  const result = await observe('runner', true)
  assert.equal(result.code, 0, result.stderr)
  assert.equal(result.records[0].canary_present, true)
  assert.equal(result.records[0].github_token_present, true)
  assert.ok(result.records.some((record) => record.kind === 'runner_spawn'))
  assert.ok(result.records.some((record) => record.kind === 'runner_exit'))
})

test('a missing seed fails before starting a runner', async () => {
  const result = await observe('runner', false)
  assert.notEqual(result.code, 0)
  assert.equal(result.records.length, 1)
  assert.equal(result.records[0].canary_present, false)
})

test('provider observation confirms removal before invoking the real binary', async () => {
  const result = await observe('copilot', false)
  assert.equal(result.code, 0, result.stderr)
  assert.equal(result.records[0].canary_present, false)
  assert.equal(result.records[0].github_token_present, false)
  assert.ok(result.records.some((record) => record.kind === 'copilot_exit'))
})

test('an inherited provider credential fails before any provider execution', async () => {
  const result = await observe('copilot', true)
  assert.notEqual(result.code, 0)
  assert.equal(result.records.length, 1)
  assert.equal(result.records[0].canary_present, true)
  assert.ok(!result.stderr.includes(canary), 'failures must not print credential values')
})
