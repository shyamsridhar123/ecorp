import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { COPILOT_CREDENTIAL_CANARY as canary } from './copilot_probe_environment.mjs'

const observer = path.join(import.meta.dirname, 'copilot_probe_process.mjs')

async function observe(role, seeded, extraEnvironment = {}) {
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
  Object.assign(env, extraEnvironment)
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
  assert.equal(result.records[0].github_token_is_canary, true)
  assert.ok(result.records.some((record) => record.kind === 'runner_spawn'))
  assert.ok(result.records.some((record) => record.kind === 'runner_exit'))
})

test('a missing seed fails before starting a runner', async () => {
  const result = await observe('runner', false)
  assert.notEqual(result.code, 0)
  assert.equal(result.records.length, 1)
  assert.equal(result.records[0].canary_present, false)
  assert.equal(result.records[0].github_token_is_canary, false)
})

test('provider observation confirms removal before invoking the real binary', async () => {
  const result = await observe('copilot', false)
  assert.equal(result.code, 0, result.stderr)
  assert.equal(result.records[0].canary_present, false)
  assert.equal(result.records[0].github_token_present, false)
  assert.equal(result.records[0].github_token_is_canary, false)
  assert.ok(result.records.some((record) => record.kind === 'copilot_exit'))
})

test('an inherited provider credential fails before any provider execution', async () => {
  const result = await observe('copilot', true)
  assert.notEqual(result.code, 0)
  assert.equal(result.records.length, 1)
  assert.equal(result.records[0].canary_present, true)
  assert.ok(!result.stderr.includes(canary), 'failures must not print credential values')
})

test('a marker under another key cannot hide a missing runner GITHUB_TOKEN seed', async () => {
  const result = await observe('runner', false, { ECORP_OTHER_MARKER: canary })
  assert.notEqual(result.code, 0)
  assert.equal(result.records.length, 1, 'failed seed must not spawn the runner')
  assert.equal(result.records[0].canary_present, true)
  assert.equal(result.records[0].github_token_present, false)
  assert.equal(result.records[0].github_token_is_canary, false)
  assert.ok(!result.stderr.includes(canary))
})

test('a different token plus an unrelated marker cannot manufacture an exact runner seed', async () => {
  const differentToken = 'synthetic-wrong-token-not-an-authentication-credential'
  const result = await observe('runner', false, {
    GITHUB_TOKEN: differentToken, ECORP_OTHER_MARKER: canary,
  })
  assert.notEqual(result.code, 0)
  assert.equal(result.records.length, 1, 'failed seed must not spawn the runner')
  assert.equal(result.records[0].canary_present, true)
  assert.equal(result.records[0].github_token_present, true)
  assert.equal(result.records[0].github_token_is_canary, false)
  assert.ok(!result.stderr.includes(differentToken) && !result.stderr.includes(canary))
  assert.ok(!JSON.stringify(result.records).includes(differentToken))
})

test('a provider token other than the canary is still rejected before child execution', async () => {
  const differentToken = 'synthetic-provider-token-not-an-authentication-credential'
  const result = await observe('copilot', false, { GITHUB_TOKEN: differentToken })
  assert.notEqual(result.code, 0)
  assert.equal(result.records.length, 1)
  assert.equal(result.records[0].canary_present, false)
  assert.equal(result.records[0].github_token_present, true)
  assert.equal(result.records[0].github_token_is_canary, false)
  assert.ok(!result.stderr.includes(differentToken))
})

test('a provider canary under another key remains a failing credential boundary', async () => {
  const result = await observe('copilot', false, { ECORP_OTHER_MARKER: canary })
  assert.notEqual(result.code, 0)
  assert.equal(result.records.length, 1)
  assert.equal(result.records[0].canary_present, true)
  assert.equal(result.records[0].github_token_present, false)
  assert.ok(!result.stderr.includes(canary))
})
