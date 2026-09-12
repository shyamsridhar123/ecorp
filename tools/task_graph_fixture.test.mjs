import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import test from 'node:test'
import { graphFixtureSource, taskGraphFixtureConfig } from './task_graph_fixture.mjs'

const corp = '00000000-0000-4000-8000-000000000001'
const env = { CRONY_TASK_GRAPH_TEST: '1', CRONY_SERVER_HTTP: 'http://127.0.0.1:18437' }
const source = { repository: 'all-the-vibes/ecorp', base_ref: 'HEAD', base_commit: 'a'.repeat(40) }
function snapshot() {
  return { runners: [{ id: 'fixture', connected: true, corp_id: corp, capabilities: [
    { name: 'fake-process', available: true },
    { name: 'workspace-isolation', available: true, source_repository: source.repository,
      source_base_ref: source.base_ref, source_base_commit: source.base_commit },
  ] }] }
}

test('task graph requires explicit ownership and a single dry-run option', () => {
  assert.equal(taskGraphFixtureConfig([], env).server, env.CRONY_SERVER_HTTP)
  assert.equal(taskGraphFixtureConfig(['--dry-run'], env).dryRun, true)
  for (const args of [['--execute'], ['--skip'], ['--dry-run', '--dry-run']]) {
    assert.throws(() => taskGraphFixtureConfig(args, env), /option/u)
  }
  for (const options of [{}, { CRONY_SERVER_HTTP: env.CRONY_SERVER_HTTP }, { CRONY_TASK_GRAPH_TEST: '1' }]) {
    assert.throws(() => taskGraphFixtureConfig([], options), /explicit/iu)
  }
  const output = path.join(import.meta.dirname, '..', 'output', 'owned-graph.json')
  assert.equal(taskGraphFixtureConfig([], { ...env, CRONY_TASK_GRAPH_OUTPUT: output }).output, output)
})

test('task graph refuses manual ports, credentials, remote origins and malformed endpoints even in CI', () => {
  for (const server of [
    ...['5432', '54329', '8791', '8793', '5187', '5291', '15191', '15193'].map(port => 'http://127.0.0.1:' + port),
    'https://127.0.0.1:18437', 'http://example.com:18437', 'http://user:canary@127.0.0.1:18437',
    'http://127.0.0.1:18437/path', 'http://127.0.0.1:18437/?query', 'http://127.0.0.1:18437/#fragment',
    'http://127.0.0.1', 'not a URL',
  ]) assert.throws(() => taskGraphFixtureConfig([], { ...env, CRONY_SERVER_HTTP: server, GITHUB_ACTIONS: 'true', CI: 'true' }),
    error => !error.message.includes('canary'))
  for (const server of ['http://localhost:18437', 'http://[::1]:18437']) {
    assert.equal(taskGraphFixtureConfig([], { ...env, CRONY_SERVER_HTTP: server }).server, server)
  }
})

test('obsolete roster SQL cannot be silently re-enabled', () => {
  assert.throws(() => taskGraphFixtureConfig([], { ...env, ECORP_CI_UNIX_DEMO_ROSTER: '1' }), /obsolete/u)
  const helper = readFileSync(new URL('./task_graph_fixture.mjs', import.meta.url), 'utf8')
  assert.doesNotMatch(helper, /child_process|UPDATE agents|docker.*exec|unixDemoRosterSql/u)
})

test('source-selected staffing reads exact source metadata without mutating the snapshot', () => {
  const state = snapshot(), before = structuredClone(state)
  assert.deepEqual(graphFixtureSource(state, corp).source, source)
  assert.deepEqual(state, before)
  state.runners[0].capabilities[1].source_base_commit = 'b'.repeat(64)
  assert.equal(graphFixtureSource(state, corp).source.base_commit.length, 64)
})

test('source selection rejects wrong Corp, connection, adapter, source, or ambiguous runner', () => {
  for (const change of [
    state => { state.runners = [] },
    state => { state.runners.push(structuredClone(state.runners[0])) },
    state => { state.runners[0].connected = false },
    state => { state.runners[0].corp_id = 'other' },
    state => { state.runners[0].capabilities[0].available = false },
    state => { state.runners[0].capabilities[0].workspace_connection_id = 'other' },
    state => { state.runners[0].capabilities[1].available = false },
    state => { state.runners[0].capabilities[1].workspace_connection_id = 'other' },
    state => { state.runners[0].capabilities[1].source_base_commit = 'HEAD' },
    state => { state.runners[0].capabilities[1].source_repository = null },
    state => { state.runners[0].capabilities.push(structuredClone(state.runners[0].capabilities[1])) },
  ]) { const state = snapshot(); change(state); assert.throws(() => graphFixtureSource(state, corp)) }
})

test('the E2E retains native staffing, source assertions, concurrency, handoff and Windows mixed-provider coverage', () => {
  const script = readFileSync(new URL('./e2e_task_graph.mjs', import.meta.url), 'utf8')
  for (const evidence of ['graphFixtureSource(initial, demo.corp_id)', 'source_selected: sourceSelected',
    'worker?.mission_id, created.mission_id', 'legacy_mixed_provider_graph', "parallelGraph.runner_os === 'windows'",
    'root_adapters:', 'synthesis_consumed_verified_specialists: true', 'task.attempt_count, task.max_attempts']) {
    assert.ok(script.includes(evidence), 'Lost graph coverage: ' + evidence)
  }
  assert.ok(script.indexOf('if (config.dryRun)') < script.indexOf("await post('/api/demo/reset'"))
  assert.doesNotMatch(script, /prepareUnixDemoRoster|roster_fixture/u)
  const workflow = readFileSync(new URL('../.github/workflows/ci.yml', import.meta.url), 'utf8')
  assert.doesNotMatch(workflow, /ECORP_CI_UNIX_DEMO_ROSTER/u)
  const wrapper = readFileSync(new URL('./ci_external_adapters_windows.ps1', import.meta.url), 'utf8')
  assert.ok(wrapper.includes('legacy_mixed_provider_graph.root_adapters'))
})
