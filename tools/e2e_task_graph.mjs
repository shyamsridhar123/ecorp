import assert from 'node:assert/strict'
import { writeFile } from 'node:fs/promises'
import { downloadVerifiedArtifact } from './artifact_client.mjs'
import { graphFixtureSource, taskGraphFixtureConfig } from './task_graph_fixture.mjs'

const config = taskGraphFixtureConfig(process.argv.slice(2), process.env)
const server = config.server
if (config.dryRun) {
  console.log(JSON.stringify({ ...config, services_started: false, database_writes: false,
    proposed: ['reset only the explicitly owned demo fixture',
      'verify native source-selected staffing, concurrent roots, dependency artifacts and bounded retries',
      'also verify the original mixed Codex/Claude graph on Windows; no roster SQL mutations'],
  }, null, 2))
  process.exit(0)
}
const activeStatuses = new Set([
  'provisioning',
  'starting',
  'running',
  'waiting_for_input',
  'waiting_for_approval',
  'verifying',
])

async function request(url, init) {
  const response = await fetch(`${server}${url}`, init)
  const body = await response.json()
  if (!response.ok) {
    throw new Error(`${init?.method ?? 'GET'} ${url} failed: ${JSON.stringify(body)}`)
  }
  return body
}

function post(url, body) {
  return request(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
}

async function snapshot(demo) {
  return request(
    `/api/corps/${demo.corp_id}/snapshot?actor_id=${demo.alice_actor_id}`,
  )
}

async function waitForMission(demo, missionId, timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs
  let maxActiveRuns = 0
  while (Date.now() < deadline) {
    const state = await snapshot(demo)
    const mission = state.snapshot.missions.find((item) => item.id === missionId)
    const taskIds = new Set(
      state.snapshot.tasks
        .filter((task) => task.mission_id === missionId)
        .map((task) => task.id),
    )
    const runs = state.snapshot.runs.filter((run) => taskIds.has(run.task_id))
    maxActiveRuns = Math.max(
      maxActiveRuns,
      runs.filter((run) => activeStatuses.has(run.status)).length,
    )
    if (mission && ['completed', 'failed', 'cancelled'].includes(mission.status)) {
      const allSettled = runs.every(
        (run) =>
          !activeStatuses.has(run.status) &&
          ['preserved', 'removed'].includes(run.workspace_disposition),
      )
      if (allSettled) return { state, mission, runs, maxActiveRuns }
    }
    await new Promise((resolve) => setTimeout(resolve, 75))
  }
  throw new Error(`timed out waiting for mission ${missionId}`)
}

async function parallelGraphScenario({ sourceSelected = false } = {}) {
  const demo = await post('/api/demo/reset', {})
  const initial = await snapshot(demo)
  const connected = initial.runners.filter((runner) => runner.connected)
  assert.equal(connected.length, 1, 'task-graph E2E needs its single owned fixture runner')
  const [runner] = connected
  const adapter = sourceSelected ? 'fake-process' : 'codex'
  let source
  if (sourceSelected) {
    source = graphFixtureSource(initial, demo.corp_id).source
  }
  const created = await post(`/api/corps/${demo.corp_id}/missions`, {
    requested_by: demo.alice_actor_id,
    preferred_adapter: adapter,
    strategy: 'parallel-specialists',
    title: '[graph-slow] Build a bounded dependency-aware task graph.',
    ...(source ? { source } : {}),
  })
  assert.equal(created.strategy, 'parallel-specialists')
  assert.equal(created.task_ids.length, 3)

  const planned = await snapshot(demo)
  const mission = planned.snapshot.missions.find(
    (candidate) => candidate.id === created.mission_id,
  )
  const tasks = planned.snapshot.tasks.filter(
    (task) => task.mission_id === created.mission_id,
  )
  assert.equal(mission.strategy, 'parallel-specialists')
  assert.equal(mission.max_nodes, 3)
  assert.equal(mission.max_depth, 1)
  assert.equal(tasks.filter((task) => task.status === 'ready').length, 2)
  assert.equal(tasks.filter((task) => task.status === 'pending').length, 1)
  assert.equal(tasks.filter((task) => task.depth === 0).length, 2)
  const synthesis = tasks.find((task) => task.plan_key === 'synthesis')
  assert.equal(synthesis.depends_on.length, 2)
  assert.deepEqual(
    synthesis.contract.references,
    ['task:specialist-a', 'task:specialist-b'],
  )
  for (const task of tasks) {
    assert.ok(task.contract.objective)
    assert.ok(task.contract.expected_output)
    assert.ok(task.contract.allowed_tools.length)
    assert.ok(task.contract.prohibited_actions.length)
    assert.ok(task.contract.acceptance_tests.length)
    assert.ok(task.contract.write_scope.length)
    assert.ok(task.contract.budget_tokens > 0)
    assert.ok(task.max_attempts <= 3)
    assert.ok(planned.runners.some(candidate => candidate.connected && candidate.capabilities.some(capability =>
      capability.name === task.required_adapter && capability.available && capability.workspace_connection_id == null)),
    'Graph fixture planned an unavailable adapter: ' + task.required_adapter)
    if (sourceSelected) {
      assert.equal(task.required_adapter, adapter)
      assert.deepEqual({
        repository: task.contract.source_repository,
        base_ref: task.contract.source_base_ref,
        base_commit: task.contract.source_base_commit,
      }, source, 'Every task must retain the selected immutable source')
      const worker = planned.snapshot.agents.find((agent) => agent.id === task.assigned_agent_id)
      assert.equal(worker?.mission_id, created.mission_id, 'Use native mission-owned staffing')
    }
  }
  assert.equal(new Set(tasks.map((task) => task.assigned_agent_id)).size, 3)

  const launched = await post(
    `/api/corps/${demo.corp_id}/missions/${created.mission_id}/launch`,
    { requested_by: demo.alice_actor_id },
  )
  assert.equal(launched.run_ids.length, 2)
  const result = await waitForMission(demo, created.mission_id)
  assert.equal(result.mission.status, 'completed')
  assert.ok(result.maxActiveRuns >= 2)

  const finalTasks = result.state.snapshot.tasks.filter(
    (task) => task.mission_id === created.mission_id,
  )
  assert.equal(finalTasks.length, 3)
  assert.ok(finalTasks.every((task) => task.status === 'completed'))
  assert.ok(finalTasks.every((task) => task.attempt_count === 1))
  assert.equal(result.runs.length, 3)
  assert.ok(result.runs.every((run) => run.status === 'completed'))
  assert.equal(new Set(result.runs.map((run) => run.workspace_path)).size, 3)

  const taskById = new Map(finalTasks.map((task) => [task.id, task]))
  const synthesisRun = result.runs.find(
    (run) => taskById.get(run.task_id)?.plan_key === 'synthesis',
  )
  assert.ok(synthesisRun)
  const rootRuns = result.runs.filter(
    (run) => taskById.get(run.task_id)?.depth === 0,
  )
  assert.equal(rootRuns.length, 2)
  if (sourceSelected) {
    assert.ok(rootRuns.every((run) => taskById.get(run.task_id)?.required_adapter === adapter))
    assert.equal(new Set(rootRuns.map((run) => run.agent_id)).size, 2)
  } else {
    assert.equal(
      new Set(rootRuns.map((run) => taskById.get(run.task_id)?.required_adapter)).size,
      2,
    )
  }
  const events = result.state.snapshot.events
  const synthesisRequested = events.find(
    (event) =>
      event.type === 'run.requested' &&
      event.aggregate_id === synthesisRun.id,
  )
  const rootCompleted = rootRuns.map((run) =>
    events.find(
      (event) =>
        event.type === 'run.completed' &&
        event.aggregate_id === run.id,
    ),
  )
  assert.ok(rootCompleted.every(Boolean))
  assert.ok(
    synthesisRequested.seq >
      Math.max(...rootCompleted.map((event) => event.seq)),
  )
  const synthesisArtifact = (
    await downloadVerifiedArtifact(server, demo, synthesisRun)
  ).toString('utf8')
  assert.match(synthesisArtifact, /VERIFIED DEPENDENCY OUTPUTS/)
  for (const rootRun of rootRuns) {
    assert.ok(
      synthesisArtifact.includes(rootRun.id),
      `synthesis artifact omitted specialist run ${rootRun.id}`,
    )
  }

  return {
    root_adapters: rootRuns.map(run => taskById.get(run.task_id).required_adapter).sort(),
    mission_id: created.mission_id,
    runner_os: runner.os,
    source_selected: sourceSelected,
    source: source ?? null,
    task_ids: created.task_ids,
    initial_run_ids: launched.run_ids,
    all_run_ids: result.runs.map((run) => run.id),
    max_active_runs: result.maxActiveRuns,
    synthesis_requested_after_roots: true,
    synthesis_consumed_verified_specialists: true,
    final_status: result.mission.status,
  }
}

async function retryBoundScenario() {
  const demo = await post('/api/demo/reset', {})
  const created = await post(`/api/corps/${demo.corp_id}/missions`, {
    requested_by: demo.alice_actor_id,
    preferred_adapter: 'fake-process',
    strategy: 'single',
    title: '[always-fail] Prove bounded task retries.',
  })
  await post(
    `/api/corps/${demo.corp_id}/missions/${created.mission_id}/launch`,
    { requested_by: demo.alice_actor_id },
  )
  const result = await waitForMission(demo, created.mission_id)
  assert.equal(result.mission.status, 'failed')
  const task = result.state.snapshot.tasks.find(
    (candidate) => candidate.mission_id === created.mission_id,
  )
  assert.equal(task.status, 'failed')
  assert.equal(task.attempt_count, task.max_attempts)
  assert.equal(task.max_attempts, 2)
  assert.equal(result.runs.length, 2)
  assert.ok(result.runs.every((run) => run.status === 'failed'))
  const requestedEvents = result.state.snapshot.events.filter(
    (event) =>
      event.type === 'run.requested' &&
      result.runs.some((run) => run.id === event.aggregate_id),
  )
  assert.equal(requestedEvents.length, 2)

  return {
    mission_id: created.mission_id,
    task_id: task.id,
    attempts: task.attempt_count,
    max_attempts: task.max_attempts,
    run_ids: result.runs.map((run) => run.id),
    final_status: result.mission.status,
  }
}

// The normal cross-platform path staffs distinct workers for the selected
// source/runtime. Keep the original heterogeneous demo-roster case on Windows,
// where its external-CLI worker is supported; Unix refusal is tested separately.
const parallelGraph = await parallelGraphScenario({ sourceSelected: true })
const report = {
  schema_version: 2,
  checked_at: new Date().toISOString(),
  parallel_graph: parallelGraph,
  legacy_mixed_provider_graph: parallelGraph.runner_os === 'windows'
    ? await parallelGraphScenario()
    : null,
  retry_bound: await retryBoundScenario(),
}
await writeFile(
  config.output,
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
