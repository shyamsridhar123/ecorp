import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { existsSync, readFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import test from 'node:test'
import { assertBudgetFixtureReady, budgetFixtureConfig, budgetFixturePreview } from './budget_fixture.mjs'

const root = path.resolve(import.meta.dirname, '..')
const output = path.join(os.tmpdir(), 'ecorp-budget-preview-' + randomUUID(), 'e2e-budgets.json')
const env = { CRONY_BUDGET_TEST: '1', CRONY_SERVER_HTTP: 'http://127.0.0.1:18453',
  DATABASE_URL: 'postgres://fixture:private-canary@127.0.0.1:55453/fixture',
  CRONY_BUDGET_RUNNER_ID: 'runner-fixture', CRONY_BUDGET_OUTPUT: output }

test('budget fixture requires explicit scope and previews no network, services or writes', () => {
  const plan = budgetFixturePreview(budgetFixtureConfig(['--dry-run'], env))
  assert.equal(plan.network_requests, 0)
  assert.equal(plan.services_started, false)
  assert.equal(plan.database_writes, false)
  assert.equal(plan.credentials_disclosed, false)
  assert.equal(JSON.stringify(plan).includes('private-canary'), false)
  for (const args of [['--execute'], ['--dry-run', '--dry-run'], ['--skip']]) {
    assert.throws(() => budgetFixtureConfig(args, env))
  }
})

test('invalid or manual API/database targets are rejected without reflecting credentials', () => {
  for (const [key, value] of [
    ['CRONY_BUDGET_TEST', undefined], ['CRONY_SERVER_HTTP', undefined],
    ['CRONY_SERVER_HTTP', 'http://127.0.0.1:8791'], ['CRONY_SERVER_HTTP', 'http://example.com:18453'],
    ['CRONY_SERVER_HTTP', 'http://user:private-canary@127.0.0.1:18453'],
    ['CRONY_SERVER_HTTP', 'http://127.0.0.1:18453/path'], ['CRONY_SERVER_HTTP', 'https://127.0.0.1:18453'],
    ['DATABASE_URL', undefined], ['DATABASE_URL', 'private-canary'],
    ['DATABASE_URL', 'postgres://user:private-canary@127.0.0.1:54329/crony'],
    ['DATABASE_URL', 'postgres://user:private-canary@example.com:55453/fixture'],
    ['DATABASE_URL', 'postgres://user@127.0.0.1:55453/fixture?host=example.com'],
    ['DATABASE_URL', 'postgres://user@127.0.0.1:18453/fixture'],
    ['CRONY_BUDGET_OUTPUT', undefined], ['CRONY_BUDGET_OUTPUT', 'relative/e2e-budgets.json'],
    ['CRONY_BUDGET_OUTPUT', path.join(root, 'other.json')], ['CRONY_BUDGET_RUNNER_ID', undefined],
  ]) assert.throws(() => budgetFixtureConfig([], { ...env, [key]: value }), error => !error.message.includes('private-canary'))
})

test('the actual driver dry-run neither calls fetch nor creates an evidence directory', () => {
  const script = path.join(root, 'tools', 'e2e_budgets.mjs')
  const code = 'globalThis.fetch = () => { throw new Error("dry-run attempted network"); };' +
    'process.argv = [process.execPath,' + JSON.stringify(script) + ',"--dry-run"];' +
    'await import(' + JSON.stringify(pathToFileURL(script).href) + ');'
  const result = spawnSync(process.execPath, ['--input-type=module', '-e', code], {
    env: { ...env, ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}) },
    encoding: 'utf8', windowsHide: true, timeout: 10_000,
  })
  assert.equal(result.status, 0, result.stderr)
  assert.equal(JSON.parse(result.stdout).network_requests, 0)
  assert.equal(existsSync(path.dirname(output)), false)
  assert.equal((result.stdout + result.stderr).includes('private-canary'), false)
})

test('budget reset requires the configured deterministic runner and no live or held work', () => {
  const demo = { corp_id: 'fixture' }
  const state = { runners: [{ id: env.CRONY_BUDGET_RUNNER_ID, corp_id: demo.corp_id, connected: true,
    capabilities: [{ name: 'fake-process', available: true }] }],
  snapshot: { corp: { id: demo.corp_id }, missions: [], runs: [], factory_controllers: [] } }
  assertBudgetFixtureReady(state, demo, env.CRONY_BUDGET_RUNNER_ID)
  for (const change of [
    value => { value.runners[0].id = 'runner-local' },
    value => { value.runners.push(structuredClone(value.runners[0])) },
    value => { value.runners[0].capabilities[0].workspace_connection_id = 'other' },
    value => { value.snapshot.missions.push({ status: 'ready' }) },
    value => { value.snapshot.runs.push({ status: 'running' }) },
    value => { value.snapshot.factory_controllers.push({}) },
  ]) { const changed = structuredClone(state); change(changed); assert.throws(() => assertBudgetFixtureReady(changed, demo, env.CRONY_BUDGET_RUNNER_ID)) }
})

test('upstream late-completion proof and the complete budget matrix are retained', () => {
  const script = readFileSync(path.join(root, 'tools', 'e2e_budgets.mjs'), 'utf8')
  for (const evidence of ["assert.equal(late.mission.status, 'cancelled')", 'lateTasks[0].attempt_count, 1',
    'lateRun.input_tokens + lateRun.output_tokens, 6_000', 'lateRun.budget_tokens_limit, 5_000',
    "lateTerminated[0].payload.outcome, 'completed'", 'lateTerminated[0].payload.provider_process_alive, false',
    "path.join(lateRun.workspace_path, 'result.md')", 'latePreserved[0].seq < lateCancelled[0].seq',
    'lateArtifactEvents.length, 0', 'lateAcceptedCompletionEvents.length, 0', "event.type.startsWith('run.verification_')",
    'approvalResponse.status, 400', "reason.includes('repeated_tool')", "reason.includes('actor_tokens_24h')",
    "reason.includes('corp_tokens_24h')", "conversation.run.status, 'completed'"])
    assert.ok(script.includes(evidence), 'Lost upstream budget proof: ' + evidence)
  assert.ok(script.indexOf('if (config.dryRun)') < script.indexOf("await post('/api/demo/reset'"))
  assert.ok(script.includes("flag: 'wx'"))
})
