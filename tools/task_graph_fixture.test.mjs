import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import test from 'node:test'
import {
  planUnixDemoRoster,
  prepareUnixDemoRoster,
  taskGraphFixtureConfig,
  unixDemoRosterSql,
} from './task_graph_fixture.mjs'

const corp = '00000000-0000-4000-8000-000000000001'
const container = 'a'.repeat(64)
const localEnv = { CRONY_TASK_GRAPH_TEST: '1', CRONY_SERVER_HTTP: 'http://127.0.0.1:18437' }
const actionsEnv = {
  ...localEnv,
  CRONY_SERVER_HTTP: 'http://127.0.0.1:18471',
  ECORP_CI_UNIX_DEMO_ROSTER: '1',
  ECORP_TEST_POSTGRES_CONTAINER: container,
  GITHUB_ACTIONS: 'true', CI: 'true', RUNNER_OS: 'Linux', GITHUB_JOB: 'integration', GITHUB_RUN_ID: '12345',
}
const disabledFeatures = 'spawn=no, stream=no, steer=no, interrupt=no, stop=no, resume=no, usage=no, artifacts=no'
const expectedAgents = [
  { id: '00000000-0000-4000-8000-000000000032', adapter: 'claude-code', status: 'offline' },
  { id: '00000000-0000-4000-8000-000000000033', adapter: 'opencode', status: 'offline' },
]

function snapshot() {
  return {
    runners: [{
      id: 'runner-ci', os: 'linux', corp_id: corp, connected: true,
      capabilities: [
        ...['claude-code', 'opencode'].map(name => ({ name, available: false, detail: disabledFeatures })),
        ...['codex', 'fake-process'].map(name => ({ name, available: true })),
      ],
    }],
    snapshot: {
      missions: [], tasks: [], runs: [],
      agents: [
        { id: '00000000-0000-4000-8000-000000000031', corp_id: corp, adapter: 'fake-process', status: 'idle' },
        ...expectedAgents.map(agent => ({ ...agent, corp_id: corp, status: 'idle', current_run_id: null, retired_at: null })),
        { id: '00000000-0000-4000-8000-000000000034', corp_id: corp, adapter: 'codex', status: 'idle' },
      ],
    },
  }
}

test('task graph fixture requires explicit ownership and accepts only one dry-run option', () => {
  const config = taskGraphFixtureConfig([], localEnv)
  assert.equal(config.server, localEnv.CRONY_SERVER_HTTP)
  assert.equal(config.dryRun, false)
  assert.equal(config.unixRoster, false)
  assert.equal(config.container, null)
  assert.equal(config.githubRunId, null)
  assert.equal(path.isAbsolute(config.output), true)
  assert.equal(taskGraphFixtureConfig(['--dry-run'], localEnv).dryRun, true)
  for (const args of [['--execute'], ['--skip'], ['--dry-run', '--dry-run']]) {
    assert.throws(() => taskGraphFixtureConfig(args, localEnv), /option/)
  }
  for (const env of [{}, { CRONY_SERVER_HTTP: localEnv.CRONY_SERVER_HTTP }, { CRONY_TASK_GRAPH_TEST: '1' }]) {
    assert.throws(() => taskGraphFixtureConfig([], env), /explicit|Explicit/)
  }
  const output = path.join(import.meta.dirname, '..', 'output', 'owned-graph.json')
  assert.equal(taskGraphFixtureConfig([], { ...localEnv, CRONY_TASK_GRAPH_OUTPUT: output }).output, output)
})

test('task graph fixture rejects live ports, remote URLs, credentials and non-origin endpoints', () => {
  for (const server of [
    ...['8791', '8793', '5187', '5291', '15191', '15193'].map(port => 'http://127.0.0.1:' + port),
    'https://127.0.0.1:18437', 'http://example.com:18437', 'http://user:secret@127.0.0.1:18437',
    'http://127.0.0.1:18437/path', 'http://127.0.0.1:18437/?query', 'http://127.0.0.1:18437/#fragment',
    'http://127.0.0.1', 'not a URL',
  ]) {
    assert.throws(() => taskGraphFixtureConfig([], { ...localEnv, CRONY_SERVER_HTTP: server }))
  }
  for (const server of ['http://localhost:18437', 'http://[::1]:18437']) {
    assert.equal(taskGraphFixtureConfig([], { ...localEnv, CRONY_SERVER_HTTP: server }).server, server)
  }
  for (const env of [{ GITHUB_ACTIONS: 'true' }, { CI: 'true' }]) {
    assert.throws(() => taskGraphFixtureConfig([], { ...localEnv, ...env, CRONY_SERVER_HTTP: 'http://127.0.0.1:8791' }))
  }
})

test('Unix roster preparation requires the exact Actions integration service container scope', () => {
  const config = taskGraphFixtureConfig([], actionsEnv)
  assert.equal(config.unixRoster, true)
  assert.equal(config.container, container)
  assert.equal(config.githubRunId, '12345')
  for (const [key, value] of [
    ['GITHUB_ACTIONS', undefined], ['CI', undefined], ['RUNNER_OS', 'Windows'], ['RUNNER_OS', 'macOS'],
    ['GITHUB_JOB', 'quality'], ['GITHUB_RUN_ID', '0'], ['GITHUB_RUN_ID', '123x'],
    ['ECORP_TEST_POSTGRES_CONTAINER', undefined], ['ECORP_TEST_POSTGRES_CONTAINER', 'postgres'],
    ['ECORP_TEST_POSTGRES_CONTAINER', 'a'.repeat(12)], ['ECORP_TEST_POSTGRES_CONTAINER', 'g'.repeat(64)],
    ['CRONY_SERVER_HTTP', 'http://127.0.0.1:18437'], ['CRONY_SERVER_HTTP', 'http://localhost:8791'],
    ['ECORP_CI_UNIX_DEMO_ROSTER', 'true'],
  ]) {
    assert.throws(() => taskGraphFixtureConfig([], { ...actionsEnv, [key]: value }))
  }
  assert.equal(taskGraphFixtureConfig([], { ...localEnv, ECORP_CI_UNIX_DEMO_ROSTER: '0' }).unixRoster, false)
})

test('roster planning is non-mutating and idempotently targets only the two Windows-only agents', () => {
  const state = snapshot()
  const before = structuredClone(state)
  const plan = planUnixDemoRoster(state)
  assert.deepEqual(plan, { corp_id: corp, disabled_agents: expectedAgents, scope: 'pre_mission_ci_fixture_only' })
  assert.deepEqual(state, before)
  state.snapshot.agents.reverse()
  for (const agent of state.snapshot.agents) {
    if (expectedAgents.some(target => target.id === agent.id)) agent.status = 'offline'
  }
  assert.deepEqual(planUnixDemoRoster(state), plan)
})

test('roster setup refuses mission, task or run history before invoking a database command', async () => {
  for (const collection of ['missions', 'tasks', 'runs']) {
    const state = snapshot()
    state.snapshot[collection].push({ id: 'existing-history' })
    let calls = 0
    await assert.rejects(prepareUnixDemoRoster(state, taskGraphFixtureConfig([], actionsEnv), {
      run: async () => { calls++; throw new Error('must not execute') },
    }), /pre-mission fixture setup only/)
    assert.equal(calls, 0)
  }
})

test('roster setup rejects wrong runner identity, OS, platform contract or graph capabilities', () => {
  const changes = [
    state => { state.runners = [] },
    state => { state.runners.push(structuredClone(state.runners[0])) },
    state => { state.runners[0].connected = false },
    state => { state.runners[0].os = 'windows' },
    state => { state.runners[0].os = 'macos' },
    state => { state.runners[0].corp_id = '00000000-0000-4000-8000-000000000002' },
    state => { state.runners[0].capabilities[0].available = true },
    state => { state.runners[0].capabilities[0].detail = 'spawn=yes' },
    state => { state.runners[0].capabilities[1].workspace_connection_id = 'foreign' },
  ]
  for (const adapter of ['codex', 'fake-process']) {
    changes.push(
      state => { state.runners[0].capabilities = state.runners[0].capabilities.filter(cap => cap.name !== adapter) },
      state => { state.runners[0].capabilities.find(cap => cap.name === adapter).available = false },
      state => { state.runners[0].capabilities.find(cap => cap.name === adapter).workspace_connection_id = 'foreign' },
    )
  }
  for (const change of changes) {
    const state = snapshot()
    change(state)
    assert.throws(() => planUnixDemoRoster(state))
  }
})

test('roster setup rejects foreign, duplicate, malformed, busy or retired target agents', () => {
  for (const change of [
    state => { state.snapshot.agents.splice(1, 1) },
    state => { state.snapshot.agents.push({ ...state.snapshot.agents[1] }) },
    state => { state.snapshot.agents[2].id = state.snapshot.agents[1].id },
    state => { state.snapshot.agents[2].adapter = 'claude-code' },
    state => { state.snapshot.agents[1].corp_id = 'foreign' },
    state => { state.snapshot.agents[1].id = "id'; UPDATE agents SET status='offline'; --" },
    state => { state.snapshot.agents[1].status = 'working' },
    state => { state.snapshot.agents[1].current_run_id = 'a-running-task' },
    state => { state.snapshot.agents[1].retired_at = '2026-09-12T00:00:00Z' },
  ]) {
    const state = snapshot()
    change(state)
    assert.throws(() => planUnixDemoRoster(state))
  }
})

test('fixture SQL is bounded, transactional and scopes the update to exact Corp and id/adapter pairs', () => {
  const sql = unixDemoRosterSql(planUnixDemoRoster(snapshot()))
  assert.match(sql, /^BEGIN ISOLATION LEVEL SERIALIZABLE;/)
  assert.match(sql, /SET LOCAL lock_timeout = '3s'/)
  assert.match(sql, /SET LOCAL statement_timeout = '10s'/)
  for (const table of ['missions', 'tasks', 'runs']) {
    assert.ok(sql.includes('SELECT 1 FROM ' + table + " WHERE corp_id = '" + corp + "'::uuid"))
  }
  for (const agent of expectedAgents) {
    assert.ok(sql.includes("(id = '" + agent.id + "'::uuid AND adapter = '" + agent.adapter + "')"))
  }
  assert.match(sql, /status IN \('idle', 'offline'\) AND current_run_id IS NULL AND retired_at IS NULL/)
  assert.match(sql, /IF eligible <> 2 THEN/)
  assert.equal((sql.match(/UPDATE /g) ?? []).length, 1)
  assert.match(sql, /UPDATE agents SET status = 'offline' WHERE corp_id = /)
  assert.match(sql, /AND status = 'idle';/)
  assert.match(sql, /json_agg\(a ORDER BY a\.id\)/)
  assert.match(sql, /COMMIT;$/)
  assert.doesNotMatch(sql, /DELETE|TRUNCATE|ALTER TABLE|UPDATE (missions|tasks|runs)/)
})

test('SQL construction independently rejects unsafe plans', () => {
  for (const change of [
    plan => { plan.corp_id = 'foreign' },
    plan => { plan.disabled_agents.pop() },
    plan => { plan.disabled_agents[1].id = plan.disabled_agents[0].id },
    plan => { plan.disabled_agents[0].id = 'invalid-uuid' },
    plan => { plan.disabled_agents[0].adapter = "claude-code'; DELETE FROM agents; --" },
    plan => { plan.disabled_agents[0].status = 'idle' },
  ]) {
    const plan = planUnixDemoRoster(snapshot())
    change(plan)
    assert.throws(() => unixDemoRosterSql(plan))
  }
})

test('a dry-run validates the snapshot but never inspects Docker or writes to the database', async () => {
  const config = taskGraphFixtureConfig(['--dry-run'], actionsEnv)
  const report = await prepareUnixDemoRoster(snapshot(), config, {
    run: async () => { assert.fail('dry-run must not start a process') },
  })
  assert.equal(report.dry_run, true)
  assert.equal(report.database_writes, false)
  assert.deepEqual(report.disabled_agents, expectedAgents)
})

test('execution verifies the exact running container and the database result without reading environment fields', async () => {
  const calls = []
  const config = taskGraphFixtureConfig([], actionsEnv)
  const report = await prepareUnixDemoRoster(snapshot(), config, {
    run: async (program, args, options) => {
      calls.push({ program, args, options })
      return { stdout: args[0] === 'inspect' ? container + '|true|postgres:17-alpine\r\n' : JSON.stringify(expectedAgents) + '\n' }
    },
  })
  assert.equal(calls.length, 2)
  assert.deepEqual(calls[0], {
    program: 'docker', args: ['inspect', '--format', '{{.Id}}|{{.State.Running}}|{{.Config.Image}}', container],
    options: { timeout: 10_000, maxBuffer: 64 * 1024 },
  })
  assert.equal(calls[1].program, 'docker')
  assert.deepEqual(calls[1].args.slice(0, -1), [
    'exec', container, 'psql', '-X', '-A', '-t', '-q', '-v', 'ON_ERROR_STOP=1', '-U', 'crony', '-d', 'crony', '-c',
  ])
  assert.equal(calls[1].args.at(-1), unixDemoRosterSql(planUnixDemoRoster(snapshot())))
  assert.deepEqual(calls[1].options, { timeout: 15_000, maxBuffer: 64 * 1024 })
  assert.equal(report.database_writes, true)
  assert.equal(report.dry_run, false)
  assert.equal(report.github_run_id, '12345')
})

test('wrong container, image or stopped service cannot proceed to SQL execution', async () => {
  for (const stdout of [
    '', 'b'.repeat(64) + '|true|postgres:17-alpine', container + '|false|postgres:17-alpine',
    container + '|true|postgres:16-alpine', container + '|true|other/image',
  ]) {
    let calls = 0
    await assert.rejects(prepareUnixDemoRoster(snapshot(), taskGraphFixtureConfig([], actionsEnv), {
      run: async () => { calls++; return { stdout } },
    }), /expected image/)
    assert.equal(calls, 1)
  }
  for (const config of [{ unixRoster: false, container }, { unixRoster: true, container: 'postgres' }]) {
    await assert.rejects(prepareUnixDemoRoster(snapshot(), config, {
      run: async () => { assert.fail('invalid configuration must not execute') },
    }))
  }
})

test('Docker and SQL errors propagate without retries or success evidence', async () => {
  for (const failOn of ['inspect', 'exec']) {
    const calls = []
    await assert.rejects(prepareUnixDemoRoster(snapshot(), taskGraphFixtureConfig([], actionsEnv), {
      run: async (_program, args) => {
        calls.push(args[0])
        if (args[0] === failOn) throw new Error('fixture command failed')
        return { stdout: container + '|true|postgres:17-alpine' }
      },
    }), /fixture command failed/)
    assert.deepEqual(calls, failOn === 'inspect' ? ['inspect'] : ['inspect', 'exec'])
  }
})

test('execution rejects malformed, missing, duplicated or unexpected database rows', async () => {
  for (const rows of [
    [], [expectedAgents[0]], [expectedAgents[0], expectedAgents[0]],
    [{ ...expectedAgents[0], status: 'idle' }, expectedAgents[1]],
    [{ ...expectedAgents[0], adapter: 'opencode' }, expectedAgents[1]],
    [{ ...expectedAgents[0], extra: 'unexpected' }, expectedAgents[1]],
    'not JSON',
  ]) {
    await assert.rejects(prepareUnixDemoRoster(snapshot(), taskGraphFixtureConfig([], actionsEnv), {
      run: async (_program, args) => ({ stdout: args[0] === 'inspect'
        ? container + '|true|postgres:17-alpine'
        : (typeof rows === 'string' ? rows : JSON.stringify(rows)) }),
    }))
  }
})

test('CI retains Linux graph assertions and the original mixed Codex/Claude graph on Windows', () => {
  const read = relative => readFileSync(new URL(relative, import.meta.url), 'utf8').replaceAll('\r\n', '\n')
  const workflow = read('../.github/workflows/ci.yml')
  const linux = workflow.split('\n  integration:')[1].split('\n  external-adapters-windows:')[0]
  const windows = read('./ci_external_adapters_windows.ps1')
  const graph = read('./e2e_task_graph.mjs')
  assert.match(workflow, /node --test tools\/e2e_external_adapters\.test\.mjs tools\/task_graph_fixture\.test\.mjs/)
  assert.match(linux, /ECORP_CI_UNIX_DEMO_ROSTER: '1'/)
  assert.match(linux, /ECORP_TEST_POSTGRES_CONTAINER: \$\{\{ job\.services\.postgres\.id \}\}/)
  assert.match(linux, /node tools\/e2e_task_graph\.mjs --dry-run\n\s+node tools\/e2e_task_graph\.mjs\n/)
  assert.doesNotMatch(linux, /continue-on-error/)
  assert.match(windows, /--codex-command-arg', \(Join-Path \$repo 'scripts\/fake-codex-app-server\.mjs'\)/)
  assert.match(windows, /Invoke-FixtureCommand 'task-graph'/)
  assert.match(windows, /Compare-Object @\('claude-code', 'codex'\)/)
  assert.doesNotMatch(windows, /ECORP_CI_UNIX_DEMO_ROSTER/)
  assert.ok(graph.indexOf('prepareUnixDemoRoster(await snapshot(demo), config)') < graph.indexOf('preferred_adapter:'))
  assert.ok(graph.indexOf('if (config.dryRun)') < graph.indexOf("const demo = await post('/api/demo/reset'"))
  for (const assertion of [
    'assert.equal(created.task_ids.length, 3)', 'assert.equal(launched.run_ids.length, 2)',
    'assert.ok(result.maxActiveRuns >= 2)', 'assert.equal(result.runs.length, 3)',
    'assert.equal(new Set(result.runs.map((run) => run.workspace_path)).size, 3)',
    'synthesisRequested.seq >', 'assert.match(synthesisArtifact, /VERIFIED DEPENDENCY OUTPUTS/)',
    'synthesisArtifact.includes(rootRun.id)', 'assert.equal(task.attempt_count, task.max_attempts)',
    'assert.equal(task.max_attempts, 2)', 'assert.equal(result.runs.length, 2)',
  ]) assert.ok(graph.includes(assertion), 'Missing graph coverage: ' + assertion)
})
