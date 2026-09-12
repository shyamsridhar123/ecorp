import assert from 'node:assert/strict'
import path from 'node:path'
import { assertTestEndpoint } from './owned_test_stack.mjs'

const terminalMissions = new Set(['completed', 'cancelled', 'failed'])
const terminalRuns = new Set(['completed', 'cancelled', 'failed', 'lost'])

export function budgetFixtureConfig(args, env) {
  assert.ok(args.length <= 1 && args.every(arg => arg === '--dry-run'), 'Unknown or repeated budget fixture option')
  assert.ok(env.CRONY_BUDGET_TEST === '1', 'Explicit owned budget fixture opt-in is required')
  let endpoint, database
  try {
    endpoint = assertTestEndpoint(env.CRONY_SERVER_HTTP)
    database = new URL(env.DATABASE_URL)
  } catch {
    throw new Error('Explicit non-manual loopback fixture API and PostgreSQL URLs are required; values are not disclosed')
  }
  assert.ok(Number(endpoint.port) >= 1024 && ['postgres:', 'postgresql:'].includes(database.protocol) &&
    ['127.0.0.1', 'localhost', '[::1]'].includes(database.hostname) && Number(database.port) >= 1024 &&
    !['5432', '54329'].includes(database.port) && database.port !== endpoint.port &&
    database.username && /^\/[a-zA-Z0-9_]{1,63}$/u.test(database.pathname) && !database.search && !database.hash,
  'Expected a separate explicitly owned loopback PostgreSQL fixture; values are not disclosed')
  assert.ok(typeof env.CRONY_BUDGET_OUTPUT === 'string' && path.isAbsolute(env.CRONY_BUDGET_OUTPUT) &&
    path.basename(env.CRONY_BUDGET_OUTPUT) === 'e2e-budgets.json', 'An explicit owned e2e-budgets.json output path is required')
  assert.ok(/^[a-z][a-z0-9-]{0,99}$/u.test(env.CRONY_BUDGET_RUNNER_ID ?? ''), 'An explicit fixture runner ID is required')
  // These are accidental-target guards, not proof of ownership. The existing
  // CI service/Windows supervisor must verify its database and process receipts.
  // Credentials remain only in the trusted environment, never in this result.
  return { server: endpoint.origin, output: path.resolve(env.CRONY_BUDGET_OUTPUT),
    runnerId: env.CRONY_BUDGET_RUNNER_ID, dryRun: args.includes('--dry-run'),
    databaseTarget: { host: database.hostname, port: database.port, database: database.pathname } }
}

export function budgetFixturePreview(config) {
  return { ...config, services_started: false, database_writes: false, network_requests: 0,
    credentials_disclosed: false, real_provider_calls: 0,
    prerequisite: 'The invoking supervisor owns the disposable database, server and deterministic runner.',
    proposed: ['verify the exact quiescent deterministic runner before resetting the owned demo fixture',
      'exercise the existing spend, stop, late-completion, approval-race, loop and rolling-budget scenarios',
      'require native late-provider completion to remain cancelled, unverified and without an accepted artifact',
      'preserve one run, the stop incident, termination/retention/cancellation ordering and no accepted completion',
      'write a new evidence report without overwriting an earlier result'] }
}

export function assertBudgetFixtureReady(state, demo, runnerId) {
  assert.equal(state.snapshot.corp.id, demo.corp_id, 'Unexpected fixture Corp')
  const connected = state.runners.filter(runner => runner.connected)
  assert.equal(connected.length, 1, 'The disposable budget fixture must have exactly one connected runner')
  const runner = connected[0]
  assert.equal(runner.id, runnerId, 'The connected runner does not match the explicit fixture')
  assert.equal(runner.corp_id, demo.corp_id)
  assert.ok(runner.capabilities.some(capability => capability.name === 'fake-process' && capability.available &&
    capability.workspace_connection_id == null), 'The fixture requires the native deterministic adapter')
  assert.ok(state.snapshot.missions.every(mission => terminalMissions.has(mission.status)),
    'Refusing to reset a fixture with held or active missions')
  assert.ok(state.snapshot.runs.every(run => terminalRuns.has(run.status)), 'Refusing to reset active fixture runs')
  assert.equal(state.snapshot.factory_controllers.length, 0, 'An active factory controller is outside this fixture')
}
