// Parent owns the fresh QA stack and its restart; this file never starts/resets/stops it.
// CRONY_ADMISSION_TEST=1 CRONY_SERVER_HTTP=http://127.0.0.1:<owned-port>
// CRONY_ADMISSION_OUTPUT=<new-checkpoint-directory> node tools/e2e_launch_admission.mjs --phase prepare
// Restart the same owned server/runner with the same database/source, then --phase release.
// Release may resume saved completed checks; an ambiguous started check fails closed, never recreates work.
// The seeded parallel-specialists crew also selects Claude: configure the existing
// fake-external-agent fixture and set CRONY_ADMISSION_GRAPH_FIXTURES=1 before release.
// Uses the existing development demo actor authorization; this is not an OIDC test.
// No browser, live GitHub intake, real-provider, or server-restart coverage is claimed.
import assert from 'node:assert/strict'
import { createHash, randomUUID } from 'node:crypto'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'

assert.equal(process.env.CRONY_ADMISSION_TEST, '1', 'Requires explicit QA opt-in')
assert.deepEqual(process.argv.slice(2, 3), ['--phase'])
assert.equal(process.argv.length, 4, 'Use exactly --phase prepare|release')
const phase = process.argv[3]
assert.ok(['prepare', 'release'].includes(phase), 'Unknown phase')
assert.ok(process.env.CRONY_SERVER_HTTP, 'CRONY_SERVER_HTTP must be explicit')
const endpoint = new URL(process.env.CRONY_SERVER_HTTP)
assert.equal(endpoint.protocol, 'http:', 'Use the owned loopback HTTP QA stack')
assert.ok(['127.0.0.1', 'localhost', '[::1]'].includes(endpoint.hostname), 'Loopback only')
assert.ok(endpoint.port && !['8991', '8791'].includes(endpoint.port), 'Shared/manual ports forbidden')
assert.ok(!endpoint.username && !endpoint.password && !endpoint.search && !endpoint.hash)
assert.equal(endpoint.pathname, '/', 'CRONY_SERVER_HTTP must be an origin')
assert.ok(process.env.CRONY_ADMISSION_OUTPUT, 'CRONY_ADMISSION_OUTPUT must be explicit')
const server = endpoint.origin
const output = path.resolve(process.env.CRONY_ADMISSION_OUTPUT)
const checkpointPath = path.join(output, 'launch-admission.json')
const active = new Set(['provisioning', 'starting', 'running', 'waiting_for_input', 'waiting_for_approval', 'verifying'])
const terminal = new Set(['completed', 'failed', 'cancelled'])
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const digest = (bytes) => createHash('sha256').update(bytes).digest('hex')
const pick = (value, keys) => Object.fromEntries(keys.map((key) => [key, value[key]]))
const canonical = (value) => JSON.stringify(value, (_, item) =>
  item && typeof item === 'object' && !Array.isArray(item)
    ? Object.fromEntries(Object.entries(item).sort(([a], [b]) => a.localeCompare(b)))
    : item)
let checkpoint
let releasedId
let releasedFactoryId
let observations = 0

async function http(route, body) {
  const url = new URL(route, server)
  assert.equal(url.origin, server, 'Refusing an off-stack request')
  assert.ok(url.pathname.startsWith('/api/'), 'API paths only')
  return fetch(url, {
    method: body === undefined ? 'GET' : 'POST',
    headers: { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
    redirect: 'error',
    signal: AbortSignal.timeout(15_000),
  })
}

async function request(route, body, accepted = [200]) {
  const response = await http(route, body)
  // Never log a response body: factory claim responses contain a bearer claim token.
  assert.ok(accepted.includes(response.status), `${route}: HTTP ${response.status}`)
  return { status: response.status, body: await response.json() }
}

const api = (suffix) => `/api/corps/${checkpoint.corp_id}${suffix}`
const save = () => writeFile(checkpointPath, `${JSON.stringify(checkpoint, null, 2)}\n`)
const launch = (id, accepted = [200], actor = checkpoint.actor_id) =>
  request(api(`/missions/${id}/launch`), { requested_by: actor }, accepted)

async function reserveCheck(idKey) {
  checkpoint.begun_checks ??= {}
  assert.ok(!checkpoint.begun_checks[idKey] && !checkpoint[idKey],
    `${idKey} already began without a saved result; inspect it, do not duplicate its mutations`)
  checkpoint.begun_checks[idKey] = true
  await save() // Reserve before POST, including the uncertain-response/crash window.
}

function missionView(state, id) {
  const mission = state.snapshot.missions.find((item) => item.id === id)
  assert.ok(mission, `Mission ${id} absent; do not reset the QA database`)
  const tasks = state.snapshot.tasks.filter((task) => task.mission_id === id).sort((a, b) => a.id.localeCompare(b.id))
  assert.ok(tasks.length, `Mission ${id} omitted tasks`)
  const taskIds = new Set(tasks.map((task) => task.id))
  return { mission, tasks, runs: state.snapshot.runs.filter((run) => taskIds.has(run.task_id)) }
}

function authority(view) {
  return digest(canonical({
    mission: pick(view.mission, ['id', 'corp_id', 'room_id', 'requested_by', 'title', 'description',
      'specification_version', 'strategy', 'max_nodes', 'max_depth', 'original_budget_tokens',
      'original_budget_cost_microusd', 'budget_tokens', 'budget_cost_microusd']),
    tasks: view.tasks.map((task) => pick(task, ['id', 'mission_id', 'corp_id', 'title', 'objective',
      'plan_key', 'contract', 'contract_version', 'depth', 'max_attempts', 'required_adapter',
      'depends_on', 'verification_policy', 'assigned_agent_id'])),
  }))
}

async function snapshot() {
  const { body } = await request(api(`/snapshot?actor_id=${checkpoint.actor_id}`))
  for (const held of checkpoint.held) {
    const view = missionView(body, held.mission_id)
    assert.equal(authority(view), held.authority_sha256, `${held.kind}: authority changed`)
    if (held.factory_work_item_id) {
      const item = body.snapshot.factory_work_items.find((item) => item.id === held.factory_work_item_id)
      assert.equal(item?.mission_id, held.mission_id)
    }
    if (held.mission_id === releasedId || held.mission_id === releasedFactoryId) continue
    assert.equal(view.mission.status, 'ready', `${held.kind}: hold was released implicitly`)
    assert.equal(view.runs.length, 0, `${held.kind}: an unlaunched mission acquired a run`)
    assert.ok(view.tasks.every((task) => task.attempt_count === 0 && ['pending', 'ready'].includes(task.status)))
    const taskIds = new Set(view.tasks.map((task) => task.id))
    assert.ok(!body.snapshot.events.some((event) => event.type === 'run.requested' && taskIds.has(event.payload?.task_id)))
  }
  observations++
  return body
}

function sourceFromRunner(runner) {
  const workspace = runner.capabilities.find((cap) => cap.name === 'workspace-isolation' && cap.available)
  return workspace && {
    repository: workspace.source_repository, base_ref: workspace.source_base_ref,
    base_commit: workspace.source_base_commit,
  }
}

function requireRunner(state, source, adapter = 'fake-process') {
  assert.ok(state.runners.some((runner) => runner.connected &&
    canonical(sourceFromRunner(runner)) === canonical(source) &&
    runner.capabilities.some((cap) => cap.name === adapter && cap.available)),
  `Parent must provide a connected source-matched ${adapter} QA fixture`)
}

async function quietWindow() {
  const until = Date.now() + 2_000
  do { await snapshot(); await sleep(100) } while (Date.now() < until)
}

async function waitForSnapshot(predicate, label) {
  const until = Date.now() + 60_000
  do {
    const state = await snapshot()
    if (predicate(state)) return state
    await sleep(100)
  } while (Date.now() < until)
  throw new Error(`Timed out waiting for ${label}; only parent owns stack cleanup`)
}

async function waitForMission(id, expected) {
  const until = Date.now() + 90_000
  let maxActiveRuns = 0
  do {
    const state = await snapshot()
    const view = missionView(state, id)
    maxActiveRuns = Math.max(maxActiveRuns, view.runs.filter((run) => active.has(run.status)).length)
    if (terminal.has(view.mission.status)) {
      assert.equal(view.mission.status, expected, `Unexpected terminal mission ${id}`)
      if (view.runs.length && view.runs.every((run) => !active.has(run.status) &&
        ['preserved', 'removed'].includes(run.workspace_disposition))) {
        return { state, ...view, maxActiveRuns }
      }
    }
    await sleep(100)
  } while (Date.now() < until)
  throw new Error(`Timed out waiting for ${id}; parent owns diagnosis/cleanup`)
}

const verification = { checks: [{ type: 'artifact', min_bytes: 1 }, { type: 'file', path: 'result.md', min_bytes: 1 }], manual_gate: null }
const prohibited = ['modify files outside the assigned worktree', 'use undeclared long-lived credentials',
  'merge or deploy without a separate current authorization']
function missionBody(label, strategy = 'single') {
  return {
    requested_by: checkpoint.actor_id, title: `${label} admission ${checkpoint.nonce}`,
    description: 'Deterministic runner-owned admission regression; no network, merge or deployment.',
    preferred_adapter: 'fake-process', preferred_model: null, reasoning_effort: null,
    strategy, source: checkpoint.source, secret_refs: [], budget_tokens: 80_000,
    budget_cost_microusd: 1_000_000,
    verification_policy: verification,
  }
}

async function createOrdinary(label, strategy = 'single', overrides = {}) {
  return (await request(api('/missions'), { ...missionBody(label, strategy), ...overrides })).body
}

async function rememberHeld(created, kind, factoryId) {
  const state = await snapshot()
  const view = missionView(state, created.mission_id)
  assert.equal(view.tasks.length, 1)
  assert.equal(view.tasks[0].required_adapter, 'fake-process')
  assert.equal(view.tasks[0].contract.source_repository, checkpoint.source.repository)
  assert.equal(view.tasks[0].contract.source_base_ref, checkpoint.source.base_ref)
  assert.equal(view.tasks[0].contract.source_base_commit, checkpoint.source.base_commit)
  assert.equal(view.mission.budget_tokens, 80_000)
  assert.equal(view.mission.budget_cost_microusd, 1_000_000)
  assert.deepEqual(view.tasks[0].verification_policy, verification)
  checkpoint.held.push({
    kind, mission_id: created.mission_id, task_ids: view.tasks.map((task) => task.id),
    authority_sha256: authority(view), ...(factoryId ? { factory_work_item_id: factoryId } : {}),
  })
  await save()
  await snapshot()
}

async function createHeldFactory() {
  const [owner, repository] = checkpoint.source.repository.split('/')
  const fixtureId = `ADMISSION_${checkpoint.nonce}`
  const body = missionBody('[slow] held-factory')
  const issueUrl = `https://github.com/${owner}/${repository}/issues/991155`
  const policy = {
    schema_version: 1, source_of_truth: 'github_project', project_owner: owner,
    project_number: 991155, project_status: 'Todo', required_label: 'factory:ready', dependencies: [],
    repository_allowlist: [checkpoint.source.repository], source_base_ref: checkpoint.source.base_ref,
    source_base_commit: checkpoint.source.base_commit, adapter_allowlist: ['fake-process'],
    strategy_allowlist: ['single'], model: null, reasoning_effort: null, write_scope: ['**'],
    allowed_tools: ['filesystem', 'shell'], prohibited_actions: prohibited, secret_ids: [],
    verification_required: true, budget_tokens: body.budget_tokens,
    budget_cost_microusd: body.budget_cost_microusd, auto_merge: false,
  }
  const { body: claim } = await request(api('/factory/work-items/claim'), {
    actor_id: checkpoint.actor_id, source_project_owner: owner, source_project_number: 991155,
    source_project_item_id: `PVTI_${fixtureId}`, source_repository_owner: owner,
    source_repository_name: repository, source_issue_number: 991155,
    source_issue_node_id: `I_${fixtureId}`, source_issue_url: issueUrl,
    source_title: body.title, source_revision: new Date().toISOString(),
    idempotency_key: `claim-${checkpoint.nonce}`, lease_seconds: 300, policy,
  })
  checkpoint.factory_work_item_id = claim.work_item.id // IDs only; never persist claim_token.
  await save()
  const { requested_by, source, ...materialization } = body
  const { body: created } = await request(api(`/factory/work-items/${claim.work_item.id}/materialize`), {
    ...materialization, actor_id: requested_by, claim_token: claim.claim_token,
    expected_version: claim.work_item.version, idempotency_key: `materialize-${checkpoint.nonce}`,
    contract: {
      objective: 'Synthetic factory fixture held until explicit launch; no live GitHub intake.',
      expected_output: 'Runner-verified result.md', acceptance_tests: ['Runner verifier passes'],
      allowed_tools: ['filesystem', 'shell'], prohibited_actions: prohibited,
      references: [issueUrl], write_scope: ['**'],
    },
  })
  await rememberHeld(created, 'synthetic-factory', claim.work_item.id)
}

function checkAttempts(result, count, expected) {
  assert.equal(result.mission.status, expected)
  assert.equal(result.runs.length, result.tasks.length * count)
  assert.ok(result.tasks.every((task) => task.status === expected && task.attempt_count === count))
  assert.ok(result.runs.every((run) => run.status === expected))
  const ids = new Set(result.runs.map((run) => run.id))
  const requested = result.state.snapshot.events.filter((event) => event.type === 'run.requested' && ids.has(event.aggregate_id))
  assert.equal(requested.length, ids.size, 'Duplicate/missing run.requested events')
  const admissions = requested.filter((event) => event.payload?.mission_launch === true)
  assert.equal(admissions.length, 1, 'Exactly one ready-to-running admission must be audited')
  assert.equal(admissions[0].actor_id, checkpoint.actor_id)
  for (const run of result.runs) {
    assert.equal(run.source_repository, checkpoint.source.repository)
    assert.equal(run.source_base_ref, checkpoint.source.base_ref)
    assert.equal(run.source_base_commit, checkpoint.source.base_commit)
    if (expected === 'completed') {
      const task = result.tasks.find((task) => task.id === run.task_id)
      const passed = result.state.snapshot.verification_evidence.filter((item) => item.run_id === run.id && item.status === 'passed')
      assert.deepEqual([...new Set(passed.map((item) => item.check_index))].sort((a, b) => a - b),
        task.verification_policy.checks.map((_, index) => index))
      assert.equal(run.verification_status, 'passed')
      assert.ok(run.artifact_id && run.artifact_sha256)
    }
  }
}

const summary = (result) => ({
  mission_id: result.mission.id, task_ids: result.tasks.map((task) => task.id),
  run_ids: result.runs.map((run) => run.id), status: result.mission.status,
  attempts: result.tasks.map((task) => task.attempt_count), max_active_runs: result.maxActiveRuns,
})

async function recordedResult(record, attempts, expected, fingerprint = record.authority_sha256) {
  const state = await snapshot()
  const result = { state, ...missionView(state, record.mission_id), maxActiveRuns: record.max_active_runs }
  assert.ok(fingerprint, 'Completed checks require their original authority fingerprint')
  assert.equal(authority(result), fingerprint, 'Checkpointed authority changed')
  assert.equal(record.status, expected)
  assert.deepEqual(result.tasks.map((task) => task.id).sort(), [...record.task_ids].sort())
  assert.deepEqual(result.runs.map((run) => run.id).sort(), [...record.run_ids].sort())
  checkAttempts(result, attempts, expected)
  assert.ok(result.runs.every((run) => ['preserved', 'removed'].includes(run.workspace_disposition) &&
    run.workspace_detail !== 'dispatch_not_started' && state.snapshot.events.some((event) =>
      event.type === 'run.started' && event.aggregate_id === run.id)), 'Require real, settled runner starts')
  return result
}

async function unrelated(label, expected, idKey) {
  if (idKey) await reserveCheck(idKey)
  const created = await createOrdinary(label)
  if (idKey) checkpoint[idKey] = created.mission_id
  checkpoint.unrelated_ids.push(created.mission_id)
  await save()
  const planned = missionView(await snapshot(), created.mission_id)
  assert.ok(planned.tasks.every((task) => task.required_adapter === 'fake-process'))
  await launch(created.mission_id)
  const result = await waitForMission(created.mission_id, expected)
  const attempts = expected === 'failed' ? 2 : 1
  checkAttempts(result, attempts, expected)
  if (expected === 'failed') {
    assert.equal(result.tasks[0].max_attempts, 2)
    const failed = result.state.snapshot.events.filter((event) =>
      event.type === 'run.failed' && result.runs.some((run) => run.id === event.aggregate_id))
    assert.equal(failed.length, 2, 'Both retry failures must come from the runner')
  }
  await quietWindow()
  const stable = await snapshot()
  checkAttempts({ state: stable, ...missionView(stable, created.mission_id) }, attempts, expected)
  assert.equal(authority(result), authority(planned))
  return { ...summary(result), authority_sha256: authority(planned) }
}

function checkSecretDenial(state, record) {
  const view = missionView(state, record.mission_id)
  assert.equal(authority(view), record.authority_sha256)
  assert.equal(view.mission.status, 'failed')
  assert.equal(view.runs.length, 1)
  assert.deepEqual(view.tasks.map((task) => task.attempt_count), [1])
  assert.equal(view.tasks[0].contract.secret_refs[0].secret_id, record.missing_secret_id)
  assert.equal(view.runs[0].status, 'failed')
  assert.equal(view.runs[0].workspace_detail, 'dispatch_not_started')
  assert.match(view.runs[0].summary, /secret broker denied assignment/i)
  assert.ok(!state.snapshot.events.some((event) =>
    event.type === 'run.started' && event.aggregate_id === view.runs[0].id))
  if (record.run_id) assert.equal(view.runs[0].id, record.run_id)
  return view.runs[0].id
}

async function missingSecretDenial() {
  // Missing-ID variant of e2e_secrets.mjs: public secret_refs/launch bodies, no secret value or row.
  await reserveCheck('secret_denial_mission_id')
  const missingId = randomUUID()
  const created = await createOrdinary('missing-secret dispatch denial', 'single', {
    secret_refs: [{ secret_id: missingId, env_name: 'CRONY_TEST_SECRET',
      tool: 'github', resource: `repo:${checkpoint.source.repository}` }],
  })
  checkpoint.secret_denial_mission_id = created.mission_id
  await save()
  const record = { mission_id: created.mission_id, missing_secret_id: missingId,
    authority_sha256: authority(missionView(await snapshot(), created.mission_id)) }
  await launch(created.mission_id, [409])
  record.run_id = checkSecretDenial(await snapshot(), record)
  await Promise.all(Array.from({ length: 3 }, () => launch(created.mission_id, [409])))
  await quietWindow()
  checkSecretDenial(await snapshot(), record)
  return { ...record,
    dispatch_status: 409, repeat_statuses: [409, 409, 409], attempts: 1, started_events: 0 }
}

async function artifact(run) {
  assert.ok(run.artifact_uri?.startsWith(api('/artifacts/')), 'Artifact must remain Corp scoped')
  const separator = run.artifact_uri.includes('?') ? '&' : '?'
  const response = await http(`${run.artifact_uri}${separator}actor_id=${checkpoint.actor_id}`)
  assert.equal(response.status, 200, 'Artifact download failed')
  assert.equal(response.headers.get('x-crony-artifact-signature'), run.artifact_signature)
  const bytes = Buffer.from(await response.arrayBuffer())
  assert.equal(digest(bytes), run.artifact_sha256, 'Artifact integrity mismatch')
  return bytes.toString('utf8')
}

async function parallelGraph(delayedRoot = false) {
  const idKey = delayedRoot ? 'delayed_graph_mission_id' :
    checkpoint.failed_overlap ? 'overlap_recheck_mission_id' : 'graph_mission_id'
  await reserveCheck(idKey)
  const created = await createOrdinary('[slow] [graph-slow] dependency graph', 'parallel-specialists')
  checkpoint[idKey] = created.mission_id
  await save()
  const plannedState = await snapshot()
  const planned = missionView(plannedState, created.mission_id)
  assert.equal(planned.tasks.length, 3)
  const roots = planned.tasks.filter((task) => task.depth === 0)
  const join = planned.tasks.find((task) => task.plan_key === 'synthesis')
  assert.equal(roots.length, 2)
  assert.ok(roots.every((task) => task.depends_on.length === 0))
  assert.deepEqual([...join.depends_on].sort(), roots.map((task) => task.id).sort())
  for (const task of planned.tasks) {
    if (task.required_adapter !== 'fake-process') {
      assert.equal(process.env.CRONY_ADMISSION_GRAPH_FIXTURES, '1',
        `Graph selects ${task.required_adapter}; parent must configure deterministic provider fixtures, never live AI`)
      assert.ok(['claude-code', 'codex'].includes(task.required_adapter), 'Unexpected graph provider')
    }
    requireRunner(plannedState, checkpoint.source, task.required_adapter)
  }
  let blockerRunId, newDispatch
  const blockedRoot = roots.find((task) => task.required_adapter === 'fake-process')
  if (delayedRoot) {
    // Plan first: occupying a worker before planning would change ordered_candidates.
    const blocker = await createOrdinary('owned root admission gate', 'human-approval', { verification_policy: null })
    checkpoint.root_blocker_mission_id = blocker.mission_id
    await save()
    const blockerPlan = missionView(await snapshot(), blocker.mission_id)
    assert.equal(blockerPlan.tasks[0].required_adapter, 'fake-process')
    assert.equal(blockerPlan.tasks[0].assigned_agent_id, blockedRoot.assigned_agent_id)
    blockerRunId = (await launch(blocker.mission_id)).body.run_id
    checkpoint.root_blocker_run_id = blockerRunId
    await save()
    await waitForSnapshot((state) => missionView(state, blocker.mission_id).runs.some((run) =>
      run.id === blockerRunId && run.status === 'waiting_for_approval' && run.workspace_disposition === 'preserved') &&
      state.snapshot.verification_requests.some((item) => item.run_id === blockerRunId && item.status === 'pending'),
    'test-owned fake-process verification gate')
    assert.equal(missionView(await snapshot(), created.mission_id).runs.length, 0)
  }
  const { body: launched } = await launch(created.mission_id)
  assert.equal(launched.run_ids.length, delayedRoot ? 1 : 2)
  if (delayedRoot) {
    const state = await waitForSnapshot((state) => missionView(state, created.mission_id).runs.some((run) =>
      run.id === launched.run_id && run.status === 'completed' && ['removed', 'preserved'].includes(run.workspace_disposition)),
    'unblocked graph root to complete')
    const partial = missionView(state, created.mission_id)
    assert.equal(partial.mission.status, 'running')
    assert.equal(partial.runs.length, 1)
    assert.notEqual(partial.runs[0].task_id, blockedRoot.id)
    assert.equal(partial.tasks.find((task) => task.id === blockedRoot.id).attempt_count, 0)
    assert.equal(partial.tasks.find((task) => task.id === join.id).attempt_count, 0)
    // Reject ONLY the run created above. Rejection frees its worker without a Corp scheduling sweep.
    const decision = await request(api(`/runs/${blockerRunId}/verification-decision`), {
      actor_id: checkpoint.actor_id, approved: false, note: 'Release only this test-owned admission blocker',
    })
    assert.equal(decision.body.status, 'rejected')
    assert.equal(missionView(await snapshot(), created.mission_id).runs.length, 1)
    newDispatch = (await launch(created.mission_id)).body
    assert.equal(newDispatch.run_ids.length, 1)
    assert.equal(newDispatch.run_id, newDispatch.run_ids[0])
    assert.ok(!launched.run_ids.includes(newDispatch.run_id), 'Dispatch returned a historical root ID')
    assert.equal(missionView(await snapshot(), created.mission_id).runs.find((run) =>
      run.id === newDispatch.run_id)?.task_id, blockedRoot.id, 'Dispatch must identify the newly released root')
  }
  const result = await waitForMission(created.mission_id, 'completed')
  return verifyGraph(result, { authority_sha256: authority(planned),
    ...(delayedRoot ? { blocker_run_id: blockerRunId, historical_run_ids: launched.run_ids,
      fresh_dispatch_run_id: newDispatch.run_id } : {}) }, delayedRoot)
}

async function verifyGraph(result, record, delayedRoot) {
  checkAttempts(result, 1, 'completed')
  assert.equal(result.tasks.length, 3)
  assert.equal(authority(result), record.authority_sha256)
  const roots = result.tasks.filter((task) => task.depth === 0)
  const join = result.tasks.find((task) => task.plan_key === 'synthesis')
  assert.equal(roots.length, 2)
  assert.deepEqual([...join.depends_on].sort(), roots.map((task) => task.id).sort())
  // Persisted start/completion ordering below proves overlap even between snapshot polls.
  assert.ok(result.runs.every((run) => run.workspace_path))
  assert.equal(new Set(result.runs.map((run) => run.workspace_path)).size, 3)
  const rootRuns = result.runs.filter((run) => roots.some((task) => task.id === run.task_id))
  const joinRun = result.runs.find((run) => run.task_id === join.id)
  const eventSeq = (type, run) => {
    const event = result.state.snapshot.events.find((item) => item.type === type && item.aggregate_id === run.id)
    assert.ok(event, `Missing ${type} for ${run.id}`)
    return Number(event.seq)
  }
  assert.ok(eventSeq('run.requested', joinRun) > Math.max(...rootRuns.map((run) => eventSeq('run.completed', run))))
  if (!delayedRoot) assert.ok(Math.max(...rootRuns.map((run) => eventSeq('run.started', run))) <
    Math.min(...rootRuns.map((run) => eventSeq('run.completed', run))))
  if (delayedRoot) {
    const fresh = rootRuns.find((run) => run.id === record.fresh_dispatch_run_id)
    assert.ok(fresh && !record.historical_run_ids.includes(fresh.id))
    assert.deepEqual(rootRuns.filter((run) => run.id !== fresh.id).map((run) => run.id).sort(),
      [...record.historical_run_ids].sort())
    assert.ok(eventSeq('run.requested', fresh) > Math.max(...rootRuns.filter((run) =>
      run.id !== fresh.id).map((run) => eventSeq('run.completed', run))))
    const blocker = missionView(result.state, checkpoint.root_blocker_mission_id).runs
    assert.equal(blocker.length, 1)
    assert.equal(blocker[0].id, record.blocker_run_id)
    assert.equal(blocker[0].agent_id, fresh.agent_id)
    assert.equal(blocker[0].status, 'failed')
    assert.ok(result.state.snapshot.verification_requests.some((item) =>
      item.run_id === record.blocker_run_id && item.status === 'rejected' && item.decided_by === checkpoint.actor_id))
  }
  const output = await artifact(joinRun)
  assert.ok(output.includes('VERIFIED DEPENDENCY OUTPUTS'))
  for (const run of rootRuns) {
    assert.ok(output.includes(run.id) && output.includes(run.artifact_sha256))
    assert.ok(output.includes((await artifact(run)).trim()), 'Join omitted actual verified dependency bytes')
  }
  return { ...record, ...summary(result), dependency_join_verified: true,
    adapters: result.tasks.map((task) => task.required_adapter) }
}

async function prepare() {
  checkpoint = { version: 1, phase: 'preparing', server, nonce: randomUUID(), held: [], unrelated_ids: [] }
  await mkdir(output, { recursive: true })
  // Refuse repeat prepare before ANY HTTP mutation; preserve all earlier test state.
  await writeFile(checkpointPath, `${JSON.stringify(checkpoint, null, 2)}\n`, { flag: 'wx' })
  const { body: demo } = await request('/api/demo/bootstrap', {})
  Object.assign(checkpoint, { corp_id: demo.corp_id, actor_id: demo.alice_actor_id, guest_actor_id: demo.eve_actor_id })
  await save()
  const state = await snapshot()
  assert.ok(!state.snapshot.runs.some((run) => active.has(run.status)), 'QA stack must be idle; do not reset it')
  const sources = state.runners.filter((runner) => runner.connected &&
    runner.capabilities.some((cap) => cap.name === 'fake-process' && cap.available)).map(sourceFromRunner).filter(Boolean)
  assert.equal(new Set(sources.map(canonical)).size, 1, 'Provide exactly one unambiguous QA source')
  checkpoint.source = sources[0]
  assert.match(checkpoint.source.repository, /^[\w.-]+\/[\w.-]+$/, 'Synthetic factory needs owner/repository identity')
  assert.ok(checkpoint.source.base_ref)
  assert.match(checkpoint.source.base_commit, /^[a-f0-9]{40,64}$/)
  requireRunner(state, checkpoint.source)
  await save()
  await rememberHeld(await createOrdinary('[slow] held-ordinary'), 'ordinary')
  await createHeldFactory()
  checkpoint.prepare_completed_work = await unrelated('unrelated completion', 'completed')
  checkpoint.prepare_failed_work = await unrelated('[always-fail] unrelated failure', 'failed')
  await quietWindow()
  checkpoint.prepare_observations = observations
  checkpoint.prepared_at = new Date().toISOString()
  checkpoint.phase = 'prepared'
  await save()
}

async function preserveFailedOverlap() {
  // One explicit fixture correction, never an automatic "retry until green" policy.
  const failedId = checkpoint.graph_mission_id
  if (!failedId || (!checkpoint.failed_overlap && checkpoint.parallel_graph)) {
    return
  }
  if (!checkpoint.failed_overlap) {
    assert.equal(process.env.CRONY_ADMISSION_ALLOW_OVERLAP_RECHECK, '1',
      'A started graph requires inspection; explicitly authorize one recheck only after correcting the fixture')
  }
  assert.notEqual(checkpoint.parallel_graph?.mission_id, failedId, 'Failed overlap cannot be a passing case')
  const evidencePath = path.join(output, 'graph-overlap-failure.json')
  const bytes = await readFile(evidencePath)
  const evidence = JSON.parse(bytes.toString('utf8').replace(/^\uFEFF/, ''))
  assert.equal(evidence.mission_id, failedId)
  const roots = evidence.runs.filter((run) => run.task.startsWith('specialist-'))
  assert.equal(roots.length, 2)
  const lastStart = Math.max(...roots.map((run) => run.events.find((event) => event.type === 'run.started')?.seq))
  const firstEnd = Math.min(...roots.map((run) => run.events.find((event) => event.type === 'run.completed')?.seq))
  assert.ok(lastStart >= firstEnd, 'Require actual negative overlap evidence, not a guessed fixture failure')
  const live = missionView(await snapshot(), failedId)
  assert.equal(live.mission.status, 'completed') // Mission completion was not parallel-test success.
  assert.ok(live.tasks.every((task) => task.attempt_count === 1))
  assert.deepEqual(live.runs.map((run) => run.id).sort(), evidence.runs.map((run) => run.run_id).sort())
  const fixturePath = path.resolve(import.meta.dirname, '../scripts/fake-external-agent.mjs')
  const original = JSON.parse((await readFile(path.join(output, 'original-external-fixture.json'), 'utf8')).replace(/^\uFEFF/, ''))
  assert.equal(path.resolve(original.path), fixturePath, 'Fixture provenance must identify this source file')
  const correctedFixture = digest(await readFile(fixturePath))
  assert.notEqual(correctedFixture, original.sha256.toLowerCase(), 'Do not repeat an unchanged failed fixture')
  const record = { mission_id: failedId, status: 'failed_overlap', evidence_path: evidencePath,
    evidence_sha256: digest(bytes), run_ids: live.runs.map((run) => run.id).sort(),
    last_root_started_seq: lastStart, first_root_completed_seq: firstEnd,
    original_fixture_sha256: original.sha256.toLowerCase(), corrected_fixture_sha256: correctedFixture }
  if (checkpoint.failed_overlap) assert.deepEqual(checkpoint.failed_overlap, record)
  else { checkpoint.failed_overlap = record; await save() }
  // Keep graph_mission_id and the original evidence JSON untouched; replacement has its own ID slot.
}

async function releaseOrdinary(held) {
  if (checkpoint.release) {
    assert.equal(checkpoint.release.mission_id, held.mission_id)
    await recordedResult(checkpoint.release, 1, 'completed', held.authority_sha256)
    return // No launch POST, not even a duplicate or guest request, on this recovery path.
  }
  assert.equal(checkpoint.phase, 'prepared', 'Incomplete ordinary release requires inspection, not another launch')
  const denied = await launch(held.mission_id, [401, 403], checkpoint.guest_actor_id)
  assert.ok([401, 403].includes(denied.status))
  await snapshot()
  checkpoint.phase = 'releasing'
  await save()
  releasedId = held.mission_id
  // 409 is allowed for duplicate requests; successful responses must share the one run.
  const launches = await Promise.all(Array.from({ length: 4 }, () => launch(releasedId, [200, 409])))
  assert.ok(launches.some((item) => item.status === 200), 'No explicit dispatch succeeded')
  launches.push(await launch(releasedId, [200, 409]))
  const result = await waitForMission(releasedId, 'completed')
  assert.equal(result.tasks.length, 1)
  checkAttempts(result, 1, 'completed')
  launches.push(await launch(releasedId, [200, 409])) // Also reject duplication after completion.
  for (const { status, body } of launches) {
    if (status !== 200) continue
    const ids = body.run_ids ?? [body.run_id]
    assert.equal(ids.length, 1)
    assert.equal(ids[0], result.runs[0].id)
  }
  await quietWindow()
  const stable = missionView(await snapshot(), releasedId)
  assert.equal(stable.runs.length, 1)
  assert.equal(stable.tasks[0].attempt_count, 1)
  checkpoint.release = { ...summary(result), dispatch_statuses: launches.map((item) => item.status),
    unauthorized_dispatch_denied: true, immutable_authority_preserved: true }
  await save()
}

async function executeFactoryPlan(held) {
  if (checkpoint.factory_execution) {
    const record = checkpoint.factory_execution
    assert.equal(record.mission_id, held.mission_id)
    assert.equal(record.factory_work_item_id, held.factory_work_item_id)
    assert.equal(record.scope, 'synthetic_materialized_plan_execution_only')
    await recordedResult(record, 1, 'completed', held.authority_sha256)
    return
  }
  await quietWindow() // This exact plan remained the held negative control through every earlier check.
  await reserveCheck('factory_execution_mission_id')
  checkpoint.factory_execution_mission_id = held.mission_id
  await save()
  const { body: launched } = await launch(held.mission_id)
  releasedFactoryId = held.mission_id // Only after successful explicit dispatch, never a global hold exemption.
  const result = await waitForMission(held.mission_id, 'completed')
  assert.equal(result.tasks.length, 1)
  checkAttempts(result, 1, 'completed')
  assert.equal(authority(result), held.authority_sha256)
  assert.deepEqual(launched.run_ids, [result.runs[0].id])
  checkpoint.factory_execution = { ...summary(result), authority_sha256: held.authority_sha256,
    factory_work_item_id: held.factory_work_item_id, scope: 'synthetic_materialized_plan_execution_only',
    live_github_intake: false, controller_coverage: false, publication_coverage: false }
  await save()
}

async function release() {
  checkpoint = JSON.parse(await readFile(checkpointPath, 'utf8'))
  assert.equal(checkpoint.version, 1)
  assert.ok(['prepared', 'releasing', 'released'].includes(checkpoint.phase), 'Requires a prepared checkpoint')
  assert.equal(checkpoint.server, server, 'Release must use the same owned QA origin/database')
  assert.equal(checkpoint.held.length, 2)
  const held = checkpoint.held.find((item) => item.kind === 'ordinary')
  const factory = checkpoint.held.find((item) => item.kind === 'synthetic-factory')
  assert.ok(held && factory && held.mission_id !== factory.mission_id)
  if (checkpoint.release) {
    assert.equal(checkpoint.release.mission_id, held.mission_id)
    releasedId = held.mission_id // Set before snapshot, then revalidate real completed authority below.
  }
  if (checkpoint.factory_execution) {
    assert.equal(checkpoint.factory_execution.mission_id, factory.mission_id)
    releasedFactoryId = factory.mission_id
  }
  if (checkpoint.phase === 'released') assert.ok(checkpoint.release && checkpoint.parallel_graph &&
    checkpoint.secret_denial && checkpoint.delayed_root_dispatch && checkpoint.release_retry &&
    checkpoint.factory_execution, 'A released checkpoint must contain all completed checks')
  const state = await snapshot() // No bootstrap, enrollment, claim recovery, or DB reset.
  requireRunner(state, checkpoint.source)
  const fakeWorkers = state.snapshot.agents.filter((agent) =>
    agent.role !== 'manager' && agent.adapter === 'fake-process' && agent.status === 'idle')
  assert.ok(fakeWorkers.length >= 2 || process.env.CRONY_ADMISSION_GRAPH_FIXTURES === '1',
    'Seeded graph needs parent-configured deterministic external fixtures and CRONY_ADMISSION_GRAPH_FIXTURES=1')
  await quietWindow()
  await releaseOrdinary(held)
  await preserveFailedOverlap()
  if (checkpoint.parallel_graph) {
    assert.equal(checkpoint.parallel_graph.mission_id, checkpoint.overlap_recheck_mission_id ?? checkpoint.graph_mission_id)
    await verifyGraph(await recordedResult(checkpoint.parallel_graph, 1, 'completed'), checkpoint.parallel_graph, false)
  } else { checkpoint.parallel_graph = await parallelGraph(); await save() }
  if (checkpoint.secret_denial) {
    assert.equal(checkpoint.secret_denial.mission_id, checkpoint.secret_denial_mission_id)
    assert.deepEqual(checkpoint.secret_denial.repeat_statuses, [409, 409, 409])
    checkSecretDenial(await snapshot(), checkpoint.secret_denial)
  } else { checkpoint.secret_denial = await missingSecretDenial(); await save() }
  if (checkpoint.delayed_root_dispatch) {
    assert.equal(checkpoint.delayed_root_dispatch.mission_id, checkpoint.delayed_graph_mission_id)
    await verifyGraph(await recordedResult(checkpoint.delayed_root_dispatch, 1, 'completed'), checkpoint.delayed_root_dispatch, true)
  } else { checkpoint.delayed_root_dispatch = await parallelGraph(true); await save() }
  if (checkpoint.release_retry) {
    assert.equal(checkpoint.release_retry.mission_id, checkpoint.release_retry_mission_id)
    await recordedResult(checkpoint.release_retry, 2, 'failed')
  } else {
    checkpoint.release_retry = await unrelated('[always-fail] post-restart retries', 'failed', 'release_retry_mission_id')
    await save()
  }
  await quietWindow()
  const finalState = await snapshot()
  const finalReleased = missionView(finalState, releasedId) // Admission audit was checked before later event windows.
  assert.equal(finalReleased.runs.length, 1)
  assert.equal(finalReleased.tasks[0].attempt_count, 1)
  await executeFactoryPlan(factory) // Last mutation: execute the original materialized plan, not a new mission.
  checkpoint.release_observations ??= observations
  checkpoint.released_at ??= new Date().toISOString()
  checkpoint.restart_scope = 'Parent performed restart; this phase verifies persisted holds, not process lifecycle'
  checkpoint.authorization_scope = 'Development demo actor RBAC; guest dispatch rejected'
  checkpoint.browser_coverage = false
  checkpoint.phase = 'released'
  await save()
}

await (phase === 'prepare' ? prepare() : release())
console.log(JSON.stringify({ phase: checkpoint.phase, checkpoint: checkpointPath,
  held_mission_ids: checkpoint.held.map((item) => item.mission_id), browser_coverage: false }, null, 2))
