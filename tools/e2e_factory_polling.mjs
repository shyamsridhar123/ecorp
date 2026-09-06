// Isolated deterministic GitHub/runner conformance. Never targets the manual UI.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { createHash, randomUUID } from 'node:crypto'
import { readFile, writeFile, mkdir, open } from 'node:fs/promises'
import path from 'node:path'

assert.equal(process.env.CRONY_POLLING_TEST, '1', 'Explicit isolated QA opt-in required')
const server = process.env.CRONY_SERVER_HTTP
const web = process.env.CRONY_POLLING_WEB
const output = process.env.CRONY_POLLING_OUTPUT
const binary = process.env.CRONY_CLI_BINARY
assert.ok(server && output && binary)
assert.ok(['127.0.0.1', 'localhost'].includes(new URL(server).hostname))
assert.ok(!['8791', '8991', '18962'].includes(new URL(server).port), 'Manual/shared servers forbidden')
const root = path.resolve(import.meta.dirname, '..')
await mkdir(output, { recursive: true })
const checkpointPath = path.join(output, 'polling-evidence.json')
const fixturePath = path.join(output, 'github-state.json')
const suite = { scope: 'Deterministic fixture, not real GitHub mutation or AI inference',
  started_at: new Date().toISOString(), server, phase: 'starting', posts: [], processes: [], checks: {} }
await writeFile(checkpointPath, JSON.stringify(suite, null, 2), { flag: 'wx' })
const save = () => writeFile(checkpointPath, `${JSON.stringify(suite, null, 2)}\n`)
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const deadline = Date.now() + 360_000
let demo
let controller
let logHandles = []

async function request(route, body, expected = [200]) {
  if (body) {
    suite.posts.push({ route, intent_at: new Date().toISOString(),
      request_sha256: createHash('sha256').update(JSON.stringify(body)).digest('hex') })
    await save()
  }
  const response = await fetch(`${server}${route}`, {
    ...(body ? { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) } : {}),
    signal: AbortSignal.timeout(8000),
  })
  const result = await response.json()
  assert.ok(expected.includes(response.status), `${route}: ${response.status} ${JSON.stringify(result)}`)
  if (body) { suite.posts.at(-1).status = response.status; await save() }
  return result
}
const api = (route) => `/api/corps/${demo.corp_id}${route}`
const snapshot = () => request(api(`/snapshot?actor_id=${demo.alice_actor_id}`))
async function waitFor(test, label) {
  while (Date.now() < deadline) {
    if (controller && (controller.exitCode !== null || controller.signalCode !== null)) {
      throw Error('Owned controller exited while its result was required; inspect its recorded log')
    }
    const state = await snapshot()
    const controllerError = ctrl(state)?.last_error
    const freshController = Date.parse(ctrl(state)?.last_heartbeat_at) >=
      Date.parse(suite.processes.at(-1)?.started_at)
    if (freshController && controllerError &&
      /unknown repository|fixture configuration|missing issue|unsupported fake/i.test(controllerError)) {
      throw Error(`Fixture setup rejected before work: ${controllerError}`)
    }
    if (await test(state)) return state
    await delay(200)
  }
  throw Error(`Deadline while waiting for ${label}; preserve all recorded IDs`)
}
const fixture = async () => JSON.parse(await readFile(fixturePath, 'utf8'))
const ctrl = (state) => state.snapshot.factory_controllers.find((entry) => entry.id === suite.controller_id)
const activeStatuses = ['provisioning', 'starting', 'running', 'waiting_for_input', 'waiting_for_approval', 'verifying']

async function startController(label) {
  const stdout = await open(path.join(output, `${label}.stdout.log`), 'a')
  const stderr = await open(path.join(output, `${label}.stderr.log`), 'a')
  logHandles.push(stdout, stderr)
  const child = spawn(binary, ['--server', server, 'factory-watch',
    demo.corp_id, demo.alice_actor_id, '--controller-id', suite.controller_id,
    '--owner', 'ecorp-fixtures', '--project-number', '9',
    '--repository', 'ecorp-fixtures/quota-intake',
    '--source-repository-path', process.env.ECORP_TEST_SOURCE_REPOSITORY,
    '--source-base-ref', 'HEAD', '--publication-base-ref', 'main',
    '--adapter', 'fake-process', '--strategy', 'single',
    '--budget-tokens', '50000', '--budget-cost-microusd', '1000000',
    '--interval-seconds', '5', '--heartbeat-seconds', '5',
    '--github-cli', process.execPath,
  ], {
    cwd: root, windowsHide: true, stdio: ['ignore', stdout.fd, stderr.fd],
    env: { ...process.env, ECORP_FAKE_GITHUB_STATE: fixturePath,
      ECORP_GITHUB_CLI_PREFIX_ARGS_JSON: JSON.stringify([path.join(root, 'tools/fake_github_cli.mjs')]),
      GH_TOKEN: '', GITHUB_TOKEN: '', CRONY_ACCESS_TOKEN: '',
    },
  })
  suite.processes.push({ label, pid: child.pid, started_at: new Date().toISOString() })
  await save()
  return child
}

async function stopController() {
  if (!controller) return
  if (controller.exitCode === null && controller.signalCode === null) {
    const exited = once(controller, 'exit')
    controller.kill()
    await Promise.race([exited, delay(10_000).then(() => { throw Error('Owned controller did not exit') })])
  }
  suite.processes.at(-1).ended_at = new Date().toISOString()
  suite.processes.at(-1).exit_code = controller.exitCode
  suite.processes.at(-1).signal = controller.signalCode
  controller = null
  await save()
}

async function browserCapture(reason) {
  if (!web) return
  const directory = path.join(output, `browser-${reason}`)
  await mkdir(directory, { recursive: true })
  const child = spawn(process.execPath, [path.join(root, 'tools/e2e_factory_polling_browser.mjs')], {
    cwd: root, windowsHide: true, stdio: 'pipe',
    env: { ...process.env, CRONY_POLLING_WEB: web, CRONY_POLLING_OUTPUT: directory,
      CRONY_POLLING_REASON: reason },
  })
  const chunks = []
  child.stdout.on('data', (chunk) => chunks.push(chunk))
  child.stderr.on('data', (chunk) => chunks.push(chunk))
  const [code] = await once(child, 'exit')
  await writeFile(path.join(directory, 'process.log'), Buffer.concat(chunks))
  assert.equal(code, 0, 'Read-only browser verification must pass')
}

try {
  assert.ok(process.env.ECORP_TEST_SOURCE_REPOSITORY)
  demo = await request('/api/demo/bootstrap', {})
  const initial = await snapshot()
  assert.equal(initial.snapshot.missions.length, 0, 'Requires a fresh owned test database')
  assert.equal(initial.snapshot.runs.length, 0)
  assert.equal(initial.snapshot.tasks.length, 0)
  assert.equal(initial.snapshot.factory_work_items.length, 0)
  let predecessor
  const continueWork = process.env.CRONY_POLLING_CONTINUE_WORK
  const predecessorPath = continueWork || process.env.CRONY_POLLING_CONTINUE_CHECKPOINT
  if (predecessorPath) {
    predecessor = JSON.parse(await readFile(predecessorPath, 'utf8'))
    assert.equal(predecessor.server, server)
    assert.equal(predecessor.phase, 'failed')
    if (continueWork) {
      assert.equal(predecessor.error, 'Deadline while waiting for original mission to launch; preserve all recorded IDs')
      assert.ok(predecessor.checks.restart_preserved_wait && predecessor.checks.force_did_not_bypass_wait)
      suite.checks = { ...predecessor.checks }
      suite.reused_validation = 'Low-quota/restart/browser evidence from the preserved preceding phase'
    } else {
      assert.equal(predecessor.error, 'Need remaining window to prove restart wait')
    }
    assert.ok(!predecessor.mission_id && !predecessor.run_id && predecessor.controller_id)
    assert.equal(initial.snapshot.factory_controllers.length, 1)
    const existing = initial.snapshot.factory_controllers[0]
    assert.equal(existing.id, predecessor.controller_id)
    assert.equal(existing.source_project_owner, 'ecorp-fixtures')
    assert.equal(existing.source_project_number, 9)
    assert.equal(existing.source_repository_owner, 'ecorp-fixtures')
    assert.equal(existing.source_repository_name, 'quota-intake')
    assert.equal(existing.status, 'offline')
    assert.ok(Date.parse(existing.lease_expires_at) <= Date.now())
    for (const processRecord of predecessor.processes) {
      assert.ok(processRecord.ended_at, 'Prior controller was not observed terminal')
      let missing = false
      try { process.kill(processRecord.pid, 0) } catch (error) {
        if (error.code === 'ESRCH') missing = true
        else throw error
      }
      assert.ok(missing, 'Prior process ID is still present; inspect identity before continuing')
    }
    suite.predecessor = { checkpoint: predecessorPath,
      error: predecessor.error, controller_id: predecessor.controller_id, preserved: true }
  } else {
    assert.equal(initial.snapshot.factory_controllers.length, 0)
  }
  const source = initial.runners.filter((r) => r.connected).flatMap((r) => r.capabilities)
    .find((c) => c.name === 'workspace-isolation' && c.available)
  assert.equal(source?.source_repository, 'ecorp-fixtures/quota-intake')
  suite.source = { repository: source.source_repository, ref: source.source_base_ref, commit: source.source_base_commit }
  suite.controller_id = predecessor?.controller_id ?? randomUUID()
  const now = new Date().toISOString()
  const issue = { id: 'I_quota_2001', number: 2001,
    title: '[slow] Deterministic original factory quota lineage',
    body: 'Produce the bounded portable fixture deliverable in the isolated worktree.\n- [ ] File and artifact verification passes.',
    url: 'https://github.com/ecorp-fixtures/quota-intake/issues/2001',
    state: 'OPEN', createdAt: now, updatedAt: now, labels: [{ name: 'factory:ready' }] }
  const items = Array.from({ length: 1001 }, (_, i) => ({
    id: `PVTI_unrelated_${i}`, status: 'Done',
    content: { type: i % 17 === 0 ? 'DraftIssue' : 'Issue', number: i + 1,
      title: 'Noncandidate', body: '', repository: 'other/fixture', url: `https://github.com/other/fixture/issues/${i + 1}` },
  }))
  items.push({ id: 'PVTI_quota_target', status: 'Todo',
    content: { type: 'Issue', number: issue.number, title: issue.title, body: issue.body,
      repository: 'ecorp-fixtures/quota-intake', url: issue.url } })
  items.push({ id: 'PVTI_archived', status: 'Todo', isArchived: true,
    content: { type: 'Issue', number: 2002, title: 'Archived', body: '', repository: 'ecorp-fixtures/quota-intake', url: '' } })
  let state = {
    repository: 'ecorp-fixtures/quota-intake',
    project: { id: 'PVT_quota_fixture', owner: 'ecorp-fixtures', number: 9,
      status_field_id: 'PVTSSF_quota_status',
      status_options: [{ id: 'todo', name: 'Todo' }, { id: 'progress', name: 'In Progress' }, { id: 'review', name: 'In Review' }] },
    items, issues: { 2001: issue },
    graphql_quota: { limit: 5000, remaining: 1, cost_per_query: 1,
      reset_at: new Date(Date.now() + 120_000).toISOString() },
    // The first cycle performs four exact revalidations. The next active-
    // lineage cycle must fail at its pre-effect read, after reclaim, not in
    // broad discovery. This exercises blocked -> verified catch-up.
    graphql_failures: [{ match: 'item_exact', calls: [7], kind: 'secondary', retry_after: 90 }],
  }
  if (continueWork) {
    const priorFixture = path.join(path.dirname(predecessorPath), 'github-state.json')
    state = JSON.parse(await readFile(priorFixture, 'utf8'))
    assert.equal(state.project.owner, 'ecorp-fixtures')
    assert.equal(state.project.number, 9)
    assert.equal(state.graphql_query_counts.item_exact ?? 0, 0, 'No prior claim/effect boundary may have run')
    assert.ok(state.items.some((item) => item.id === 'PVTI_quota_target' &&
      item.content.repository === 'ecorp-fixtures/quota-intake'))
    // Correct only the omitted fake-CLI repository declaration. Preserve all
    // source issue content, prior counters, timestamps and failure evidence.
    state.repository = 'ecorp-fixtures/quota-intake'
    suite.fixture_correction = { field: 'repository', previous: null, preserved_fixture: priorFixture }
  }
  await writeFile(fixturePath, `${JSON.stringify(state, null, 2)}\n`, { flag: 'wx' })
  suite.phase = continueWork ? 'continue-original-work' : 'low-quota'
  await save()
  controller = await startController('controller-first')
  let observed
  let firstWait
  if (!continueWork) {
  observed = await waitFor((s) =>
    ctrl(s)?.polling?.retry_reason === 'graphql_quota' &&
    Date.parse(ctrl(s).polling.next_retry_at) > Date.now() &&
    Date.parse(ctrl(s).polling.graphql?.observed_at) >= Date.parse(suite.processes.at(-1).started_at),
  'a fresh durable low-quota observation from this controller process')
  firstWait = ctrl(observed).polling
  assert.equal(firstWait.graphql.remaining, 0)
  assert.equal(observed.snapshot.missions.length, 0)
  const firstCalls = (await fixture()).graphql_calls
  assert.equal(firstCalls, 1)
  suite.checks.low_quota = firstWait
  await save()
  const current = ctrl(await snapshot())
  await request(api(`/factory/controllers/${suite.controller_id}/control`), {
    actor_id: demo.alice_actor_id, expected_version: current.version,
    action: 'reconcile', idempotency_key: randomUUID(),
  })
  // Stop/restart only the owned idle controller. Providers, server, data and
  // the persisted wait remain untouched.
  assert.ok(Date.parse(firstWait.next_retry_at) - Date.now() > 10_000, 'Need remaining window to prove restart wait')
  await stopController()
  controller = await startController('controller-restarted')
  observed = await waitFor((s) =>
    ctrl(s)?.polling?.next_retry_at === firstWait.next_retry_at &&
    ctrl(s).version > current.version &&
    Date.parse(ctrl(s).last_heartbeat_at) >= Date.parse(suite.processes.at(-1).started_at),
  'fresh controller heartbeat retaining the same wait after restart')
  const until = Math.min(Date.parse(firstWait.next_retry_at) - 1000, Date.now() + 6500)
  while (Date.now() < until) {
    assert.equal((await fixture()).graphql_calls, firstCalls, 'Restart/reconcile bypassed the upstream wait')
    assert.equal((await snapshot()).snapshot.missions.length, 0)
    await delay(200)
  }
  suite.checks.restart_preserved_wait = true
  suite.checks.force_did_not_bypass_wait = true
  await save()
  await browserCapture('graphql_quota')
  } else {
    firstWait = predecessor.checks.low_quota
  }

  observed = await waitFor((s) => s.snapshot.missions.length === 1 && s.snapshot.runs.length === 1, 'original mission to launch')
  suite.mission_id = observed.snapshot.missions[0].id
  suite.run_id = observed.snapshot.runs[0].id
  suite.factory_item_id = observed.snapshot.factory_work_items[0].id
  suite.phase = 'external-throttle'
  await save()
  observed = await waitFor((s) => ctrl(s)?.polling?.retry_reason === 'secondary_rate_limit', 'durable secondary-limit backoff')
  const secondWait = ctrl(observed).polling
  assert.equal(observed.snapshot.factory_work_items[0].id, suite.factory_item_id)
  assert.equal(observed.snapshot.factory_work_items[0].state, 'blocked')
  const failure = (await fixture()).graphql_events.find((e) => e.failure === 'secondary')
  assert.ok(failure)
  const secondCalls = (await fixture()).graphql_calls
  suite.checks.secondary_wait = secondWait
  suite.checks.failure_event = failure
  await save()
  await browserCapture('secondary_rate_limit')
  observed = await waitFor((s) => s.snapshot.missions[0]?.status === 'completed', 'local original mission to complete during backoff')
  assert.equal(observed.snapshot.runs.length, 1)
  assert.equal(observed.snapshot.runs[0].id, suite.run_id)
  assert.equal(observed.snapshot.runs[0].verification_status, 'passed')
  const completed = observed.snapshot.events.find((event) =>
    event.type === 'run.completed' && event.aggregate_id === suite.run_id)
  assert.ok(completed, 'Require the authoritative completion event, not observation time')
  assert.ok(Date.parse(completed.created_at) > Date.parse(failure.at))
  assert.ok(Date.parse(completed.created_at) < Date.parse(secondWait.next_retry_at),
    'Local completion must precede permitted remote retry')
  const duringWait = (await fixture()).graphql_events.filter((event) =>
    event.call > secondCalls && Date.parse(event.at) < Date.parse(secondWait.next_retry_at))
  assert.deepEqual(duringWait, [], 'GitHub was contacted during the wait')
  suite.checks.local_completed_at = completed.created_at
  suite.checks.local_completion_during_outage = true
  await save()

  observed = await waitFor((s) => s.snapshot.factory_work_items[0]?.state === 'verified', 'same-lineage verified recovery')
  assert.equal(observed.snapshot.factory_work_items.length, 1)
  assert.equal(observed.snapshot.factory_work_items[0].id, suite.factory_item_id)
  assert.equal(observed.snapshot.missions.length, 1)
  assert.equal(observed.snapshot.missions[0].id, suite.mission_id)
  assert.equal(observed.snapshot.runs.length, 1)
  assert.equal(observed.snapshot.runs[0].id, suite.run_id)
  assert.equal(observed.snapshot.tasks.length, 1)
  assert.equal(observed.snapshot.tasks[0].attempt_count, 1)
  const github = await fixture()
  assert.ok(github.graphql_query_counts.project_items >= 11, 'Large Project was not fully paginated')
  assert.equal(github.item_list_calls ?? 0, 0, 'Legacy full-field Project listing was used')
  assert.ok(github.graphql_events.every((event) =>
    event.call === 1 || Date.parse(event.at) >= Date.parse(firstWait.next_retry_at)))
  const eventsAfterFailure = github.graphql_events.filter((e) => e.call > failure.call)
  assert.ok(eventsAfterFailure.every((e) => Date.parse(e.at) >= Date.parse(secondWait.next_retry_at)))
  assert.ok(!observed.snapshot.runs.some((r) => activeStatuses.includes(r.status)))
  suite.checks.same_lineage_recovered = true
  suite.checks.legacy_wide_queries = 0
  suite.checks.project_pages = github.graphql_query_counts.project_items
  suite.checks.query_log = github.graphql_events
  suite.phase = 'passed'
  suite.finished_at = new Date().toISOString()
  await save()
} catch (error) {
  suite.phase = 'failed'
  suite.error = String(error.message).slice(0, 2000)
  await save()
  throw error
} finally {
  await stopController()
  for (const handle of logHandles) await handle.close()
}
console.log(JSON.stringify({ phase: suite.phase, checkpoint: checkpointPath,
  mission_id: suite.mission_id, run_id: suite.run_id, checks: suite.checks }, null, 2))
