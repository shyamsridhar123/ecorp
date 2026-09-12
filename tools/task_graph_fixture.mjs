import assert from 'node:assert/strict'
import path from 'node:path'
import { assertTestEndpoint } from './owned_test_stack.mjs'

export function taskGraphFixtureConfig(args, env) {
  assert.ok(args.length <= 1 && args.every(arg => arg === '--dry-run'), 'Unknown or repeated task-graph fixture option')
  assert.ok(env.CRONY_TASK_GRAPH_TEST === '1', 'Explicit owned task-graph fixture opt-in is required')
  let endpoint
  try { endpoint = assertTestEndpoint(env.CRONY_SERVER_HTTP) } catch {
    throw new Error('An explicit non-manual loopback fixture URL is required; its value was not disclosed')
  }
  assert.ok(env.ECORP_CI_UNIX_DEMO_ROSTER === undefined || env.ECORP_CI_UNIX_DEMO_ROSTER === '0',
    'Legacy roster SQL is obsolete; use native source-selected mission staffing')
  return { server: endpoint.origin,
    output: path.resolve(env.CRONY_TASK_GRAPH_OUTPUT ?? path.join(import.meta.dirname, '..', 'output', 'e2e-task-graph.json')),
    dryRun: args.includes('--dry-run') }
}

export function graphFixtureSource(state, corpId) {
  const runners = state.runners.filter(runner => runner.connected)
  assert.equal(runners.length, 1, 'Task-graph fixture requires exactly one connected runner')
  const runner = runners[0]
  assert.equal(runner.corp_id, corpId)
  assert.ok(runner.capabilities.some(cap => cap.name === 'fake-process' && cap.available && cap.workspace_connection_id == null),
    'Native deterministic staffing must be available')
  const sources = runner.capabilities.filter(cap => cap.name === 'workspace-isolation' && cap.available && cap.workspace_connection_id == null)
  assert.equal(sources.length, 1, 'Task-graph fixture requires one unambiguous legacy source')
  const source = sources[0]
  assert.ok(source.source_repository && source.source_base_ref)
  assert.match(source.source_base_commit, /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/iu)
  return { runner, source: { repository: source.source_repository, base_ref: source.source_base_ref, base_commit: source.source_base_commit } }
}
