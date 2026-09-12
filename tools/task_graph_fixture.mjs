import assert from 'node:assert/strict'
import { execFile as execFileCallback } from 'node:child_process'
import path from 'node:path'
import { promisify } from 'node:util'
import { assertExternalRunner } from './e2e_external_adapters.mjs'

const execFile = promisify(execFileCallback)
const demoCorp = '00000000-0000-4000-8000-000000000001'
const windowsOnly = ['claude-code', 'opencode']
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u

export function taskGraphFixtureConfig(args, env) {
  assert.ok(args.length <= 1 && args.every(arg => arg === '--dry-run'), 'Unknown or repeated task-graph fixture option')
  assert.equal(env.CRONY_TASK_GRAPH_TEST, '1', 'Explicit owned task-graph fixture opt-in is required')
  assert.ok(env.CRONY_SERVER_HTTP, 'An explicit owned CRONY_SERVER_HTTP is required')
  const endpoint = new URL(env.CRONY_SERVER_HTTP)
  assert.ok(endpoint.protocol === 'http:' && ['127.0.0.1', 'localhost', '[::1]'].includes(endpoint.hostname) &&
    endpoint.port && endpoint.pathname === '/' && !endpoint.search && !endpoint.hash &&
    !endpoint.username && !endpoint.password, 'Expected an explicit loopback HTTP origin')
  const inActions = env.GITHUB_ACTIONS === 'true' && env.CI === 'true'
  assert.ok(!['8791', '8793', '5187', '5291', '15191', '15193'].includes(endpoint.port) || inActions,
    'Refusing a manual-stack port outside GitHub Actions')
  assert.ok([undefined, '0', '1'].includes(env.ECORP_CI_UNIX_DEMO_ROSTER), 'Invalid roster opt-in')
  const unixRoster = env.ECORP_CI_UNIX_DEMO_ROSTER === '1'
  if (unixRoster) {
    assert.ok(inActions && env.RUNNER_OS === 'Linux' && env.GITHUB_JOB === 'integration' &&
      /^[1-9][0-9]*$/u.test(env.GITHUB_RUN_ID ?? '') &&
      /^[0-9a-f]{64}$/u.test(env.ECORP_TEST_POSTGRES_CONTAINER ?? ''),
    'Unix roster preparation is restricted to the explicit Actions integration service container')
    assert.equal(endpoint.origin, 'http://127.0.0.1:8791')
  }
  return {
    server: endpoint.origin,
    output: path.resolve(env.CRONY_TASK_GRAPH_OUTPUT ?? path.join(import.meta.dirname, '..', 'output', 'e2e-task-graph.json')),
    dryRun: args.includes('--dry-run'),
    unixRoster,
    container: unixRoster ? env.ECORP_TEST_POSTGRES_CONTAINER : null,
    githubRunId: unixRoster ? env.GITHUB_RUN_ID : null,
  }
}

export function planUnixDemoRoster(state) {
  const runner = assertExternalRunner(state, 'unix')
  assert.equal(runner.os, 'linux', 'The Actions roster fixture expects Linux')
  assert.equal(runner.corp_id, demoCorp, 'Only the synthetic demo Corp may be prepared')
  for (const collection of ['missions', 'tasks', 'runs']) {
    assert.equal(state.snapshot[collection].length, 0, 'Roster preparation is pre-mission fixture setup only')
  }
  for (const adapter of ['codex', 'fake-process']) {
    assert.ok(runner.capabilities.some(capability => capability.name === adapter && capability.available &&
      capability.workspace_connection_id == null), 'Missing required graph fixture adapter: ' + adapter)
  }
  const targets = state.snapshot.agents.filter(agent => windowsOnly.includes(agent.adapter))
  assert.equal(targets.length, 2, 'Expected only the two fixed Windows-only demo agents')
  assert.deepEqual(targets.map(agent => agent.adapter).sort(), windowsOnly)
  for (const agent of targets) {
    assert.ok(uuid.test(agent.id) && agent.corp_id === demoCorp, 'Invalid or foreign demo agent')
    assert.ok(['idle', 'offline'].includes(agent.status) && !agent.current_run_id && !agent.retired_at,
      'Busy or retired agent cannot be changed by fixture preparation')
  }
  assert.equal(new Set(targets.map(agent => agent.id)).size, 2)
  return {
    corp_id: demoCorp,
    disabled_agents: targets.map(({ id, adapter }) => ({ id, adapter, status: 'offline' }))
      .sort((left, right) => left.id.localeCompare(right.id)),
    scope: 'pre_mission_ci_fixture_only',
  }
}

export function unixDemoRosterSql(plan) {
  assert.equal(plan.corp_id, demoCorp)
  assert.equal(plan.disabled_agents.length, 2)
  assert.equal(new Set(plan.disabled_agents.map(agent => agent.id)).size, 2)
  assert.deepEqual(plan.disabled_agents.map(agent => agent.adapter).sort(), windowsOnly)
  assert.ok(plan.disabled_agents.every(agent => uuid.test(agent.id) && agent.status === 'offline'))
  const scope = "corp_id = '" + demoCorp + "'::uuid"
  const pairs = plan.disabled_agents.map(agent =>
    "(id = '" + agent.id + "'::uuid AND adapter = '" + agent.adapter + "')").join(' OR ')
  const target = scope + ' AND (' + pairs + ')'
  return [
    'BEGIN ISOLATION LEVEL SERIALIZABLE;',
    "SET LOCAL lock_timeout = '3s';",
    "SET LOCAL statement_timeout = '10s';",
    'SET LOCAL search_path = public;',
    'DO $ci_roster$',
    'DECLARE eligible integer;',
    'BEGIN',
    '  IF EXISTS (SELECT 1 FROM missions WHERE ' + scope + ') OR',
    '     EXISTS (SELECT 1 FROM tasks WHERE ' + scope + ') OR',
    '     EXISTS (SELECT 1 FROM runs WHERE ' + scope + ') THEN',
    "    RAISE EXCEPTION 'CI roster cannot alter a Corp with mission history';",
    '  END IF;',
    '  SELECT count(*) INTO eligible FROM agents WHERE ' + target +
      " AND status IN ('idle', 'offline') AND current_run_id IS NULL AND retired_at IS NULL;",
    '  IF eligible <> 2 THEN',
    "    RAISE EXCEPTION 'CI demo roster changed before fixture setup';",
    '  END IF;',
    "  UPDATE agents SET status = 'offline' WHERE " + target + " AND status = 'idle';",
    'END $ci_roster$;',
    "SELECT COALESCE(json_agg(a ORDER BY a.id), '[]'::json) FROM",
    '(SELECT id, adapter, status FROM agents WHERE ' + target + ') a;',
    'COMMIT;',
  ].join('\n')
}

export async function prepareUnixDemoRoster(state, config, { run = execFile } = {}) {
  assert.equal(config.unixRoster, true)
  assert.ok(/^[0-9a-f]{64}$/u.test(config.container ?? ''), 'An exact service container ID is required')
  const plan = planUnixDemoRoster(state)
  if (config.dryRun) return { ...plan, dry_run: true, database_writes: false }
  // Inspect no environment fields: service credentials must not enter output.
  const inspected = await run('docker', ['inspect', '--format',
    '{{.Id}}|{{.State.Running}}|{{.Config.Image}}', config.container],
  { timeout: 10_000, maxBuffer: 64 * 1024 })
  assert.equal(inspected.stdout.trim(), config.container + '|true|postgres:17-alpine',
    'The explicit CI PostgreSQL service is not running with the expected image')
  const result = await run('docker', ['exec', config.container, 'psql', '-X', '-A', '-t', '-q',
    '-v', 'ON_ERROR_STOP=1', '-U', 'crony', '-d', 'crony', '-c', unixDemoRosterSql(plan)],
  { timeout: 15_000, maxBuffer: 64 * 1024 })
  assert.deepEqual(JSON.parse(result.stdout.trim()), plan.disabled_agents,
    'The fixture changed an unexpected roster')
  return { ...plan, dry_run: false, database_writes: true, github_run_id: config.githubRunId }
}
