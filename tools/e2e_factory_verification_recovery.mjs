import { restartOwnedTestServer } from './owned_test_stack.mjs'
import assert from 'node:assert/strict'
import { execFile as execFileCallback, spawn } from 'node:child_process'
import { createHash, randomUUID } from 'node:crypto'
import { existsSync } from 'node:fs'
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { promisify } from 'node:util'

const execFile = promisify(execFileCallback)
const root = path.resolve(import.meta.dirname, '..')
if (
  process.env.CRONY_RECOVERY_TEST !== '1' ||
  !process.env.CRONY_SERVER_HTTP ||
  !process.env.DATABASE_URL ||
  !process.env.CRONY_RECOVERY_SOURCE ||
  !process.env.CRONY_RECOVERY_OUTPUT
) {
  throw new Error(
    'Recovery E2E requires CRONY_RECOVERY_TEST=1 and explicitly owned server, database, source, and output paths; never use the manual-test stack.',
  )
}
const server = process.env.CRONY_SERVER_HTTP
const endpoint = new URL(server)
if (
  !['127.0.0.1', 'localhost', '[::1]'].includes(endpoint.hostname) ||
  !endpoint.port ||
  ['8791', '8793', '5187', '5291', '15191', '15193'].includes(endpoint.port) ||
  endpoint.username ||
  endpoint.password
) {
  throw new Error('Recovery E2E requires an isolated loopback port, not a shared/manual endpoint.')
}
const output = path.resolve(process.env.CRONY_RECOVERY_OUTPUT)
const sourceRoot = path.resolve(process.env.CRONY_RECOVERY_SOURCE)
if (sourceRoot === root || sourceRoot.startsWith(`${root}${path.sep}`)) {
  throw new Error('Recovery E2E source must be an independent fixture outside the ECorp checkout.')
}
const databaseUrl = process.env.DATABASE_URL
const binary =
  process.env.CRONY_CLI_BINARY ??
  path.join(
    root,
    'target',
    'debug',
    process.platform === 'win32' ? 'crony-cli.exe' : 'crony-cli',
  )
const fakeGithub = path.join(root, 'tools', 'fake_github_cli.mjs')
const sourceBaseCommit = (
  await execFile('git', ['rev-parse', 'HEAD'], { cwd: sourceRoot, windowsHide: true })
).stdout.trim()
await mkdir(output, { recursive: true })
let psqlMode

async function request(url, init) {
  const response = await fetch(`${server}${url}`, {
    ...init,
    signal: init?.signal ?? AbortSignal.timeout(30_000),
  })
  const body = response.status === 204 ? null : await response.json()
  return { response, body }
}

async function requestOk(url, init) {
  const result = await request(url, init)
  if (!result.response.ok) {
    throw new Error(
      `${init?.method ?? 'GET'} ${url} failed: ${JSON.stringify(result.body)}`,
    )
  }
  return result.body
}

function post(url, body) {
  return requestOk(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
}

async function psql(sql) {
  const invocation = await psqlInvocation()
  if (invocation.mode === 'python') {
    return pythonPsql(sql)
  }
  const { stdout } = await execFile(
    invocation.command,
    [
      ...invocation.args,
      '-v',
      'ON_ERROR_STOP=1',
      '-At',
      '-c',
      sql,
    ],
    {
      cwd: root,
      windowsHide: true,
      maxBuffer: 8 * 1024 * 1024,
    },
  )
  return stdout.trim()
}

function pythonPsql(sql) {
  const script = [
    'import os, sys, psycopg',
    'def render(value):',
    '  if isinstance(value, memoryview): value = value.tobytes()',
    "  if isinstance(value, (bytes, bytearray)): value = value.decode('utf-8')",
    "  return 't' if value is True else 'f' if value is False else '' if value is None else str(value)",
    "with psycopg.connect(os.environ['ECORP_TEST_DATABASE_URL'], autocommit=True) as connection:",
    '  with connection.cursor() as cursor:',
    '    cursor.execute(sys.stdin.read(), prepare=False)',
    '    output = []',
    '    while True:',
    '      if cursor.description:',
    "        output = ['|'.join(render(value) for value in row) for row in cursor.fetchall()]",
    '      if not cursor.nextset(): break',
    "    print('\\n'.join(output))",
  ].join('\n')
  return new Promise((resolve, reject) => {
    const child = spawn(process.env.ECORP_TEST_PYTHON ?? 'python', ['-c', script], {
      cwd: root,
      windowsHide: true,
      env: { ...process.env, ECORP_TEST_DATABASE_URL: databaseUrl },
      stdio: ['pipe', 'pipe', 'pipe'],
    })
    const stdout = []
    const stderr = []
    child.stdout.on('data', (chunk) => stdout.push(chunk))
    child.stderr.on('data', (chunk) => stderr.push(chunk))
    child.on('error', reject)
    child.on('close', (code) => {
      const output = Buffer.concat(stdout).toString('utf8').trim()
      if (code === 0) {
        resolve(output)
      } else {
        reject(
          new Error(
            `python psql failed with exit ${code}: ${Buffer.concat(stderr).toString('utf8').trim()}`,
          ),
        )
      }
    })
    child.stdin.end(sql)
  })
}

async function psqlInvocation() {
  if (!psqlMode && process.env.ECORP_TEST_POSTGRES_CONTAINER) {
    psqlMode = 'docker'
  }
  if (!psqlMode) {
    try {
      await execFile('psql', ['--version'], { cwd: root, windowsHide: true })
      psqlMode = 'direct'
    } catch {
      psqlMode =
        process.env.ECORP_TEST_PYTHON_PSQL === '1' ? 'python' : 'docker'
    }
  }
  if (psqlMode === 'python') {
    return { mode: 'python' }
  }
  if (psqlMode === 'direct') {
    return { command: 'psql', args: [databaseUrl] }
  }
  const container = process.env.ECORP_TEST_POSTGRES_CONTAINER
  if (!container) {
    throw new Error(
      'psql is unavailable and ECORP_TEST_POSTGRES_CONTAINER was not provided',
    )
  }
  const parsedDatabaseUrl = new URL(databaseUrl)
  const databaseName = decodeURIComponent(
    parsedDatabaseUrl.pathname.replace(/^\/+/, ''),
  )
  const databaseUser = decodeURIComponent(parsedDatabaseUrl.username || 'crony')
  if (!databaseName) {
    throw new Error('DATABASE_URL omitted its PostgreSQL database name')
  }
  return {
    command: 'docker',
    args: [
      'exec',
      '-i',
      container,
      'psql',
      '-U',
      databaseUser,
      '-d',
      databaseName,
    ],
  }
}

function sqlLiteral(value) {
  return `'${String(value).replaceAll("'", "''")}'`
}

function snapshot(demo) {
  return requestOk(
    `/api/corps/${demo.corp_id}/snapshot?actor_id=${demo.alice_actor_id}`,
  )
}

async function waitFor(demo, predicate, label, timeoutMs = 90_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const state = await snapshot(demo)
    const value = predicate(state.snapshot)
    if (value) return { state, value }
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error(`timed out waiting for ${label}`)
}

function projectState(issue, itemId, projectId, statusFieldId) {
  return {
    repository: 'shyamsridhar123/ecorp',
    project: {
      id: projectId,
      owner: 'acme',
      number: 7,
      status_field_id: statusFieldId,
      status_options: [
        { id: 'todo', name: 'Todo' },
        { id: 'progress', name: 'In Progress' },
        { id: 'review', name: 'In Review' },
        { id: 'done', name: 'Done' },
      ],
    },
    items: [
      {
        id: itemId,
        status: 'Todo',
        content: {
          body: issue.body,
          number: issue.number,
          repository: 'shyamsridhar123/ecorp',
          title: issue.title,
          type: 'Issue',
          url: issue.url,
        },
      },
    ],
    issues: { [String(issue.number)]: issue },
    item_edits: 0,
  }
}

function controllerInvocation(
  demo,
  issueNumber,
  statePath,
  policyPath,
  {
    adapter,
    strategy = 'single',
    recovery,
    recoveryReason,
  },
) {
  const args = [
    'factory',
    demo.corp_id,
    demo.alice_actor_id,
    '--owner',
    'acme',
    '--project-number',
    '7',
    '--repository',
    'shyamsridhar123/ecorp',
    '--source-repository-path',
    sourceRoot,
    '--source-base-ref',
    'HEAD',
    '--adapter',
    adapter,
    '--strategy',
    strategy,
    '--budget-tokens',
    '100000',
    '--budget-cost-microusd',
    '1000000',
    '--lease-seconds',
    '300',
    '--write-scope',
    '**',
    '--verification-policy-file',
    policyPath,
    '--issue',
    String(issueNumber),
    '--github-cli',
    process.execPath,
  ]
  if (recovery) {
    args.push(
      '--verification-recovery',
      recovery,
      '--verification-recovery-reason',
      recoveryReason,
    )
  }
  return {
    args,
    options: {
      cwd: root,
      env: {
        ...process.env,
        CRONY_SERVER_HTTP: server,
        ECORP_GITHUB_CLI_PREFIX_ARGS_JSON: JSON.stringify([fakeGithub]),
        ECORP_FAKE_GITHUB_STATE: statePath,
      },
      maxBuffer: 4 * 1024 * 1024,
      windowsHide: true,
    },
  }
}

async function runController(demo, issueNumber, statePath, policyPath, options) {
  const invocation = controllerInvocation(
    demo,
    issueNumber,
    statePath,
    policyPath,
    options,
  )
  const { stdout } = await execFile(binary, invocation.args, invocation.options)
  return JSON.parse(stdout)
}

async function controllerFailure(
  demo,
  issueNumber,
  statePath,
  policyPath,
  options,
) {
  try {
    await runController(demo, issueNumber, statePath, policyPath, options)
    assert.fail('factory controller unexpectedly accepted the request')
  } catch (error) {
    return [error.message, error.stdout, error.stderr].filter(Boolean).join('\n')
  }
}

async function restartTestServer() {
  if (!process.env.CRONY_TEST_SERVER_PID_FILE) return false
  await restartOwnedTestServer({ root, server, databaseUrl, logPrefix: 'verification-recovery-restart' })
  return true
}

async function openRecoveryEventClient(demo, afterSeq) {
  const events = []
  let failure = null
  const socket = new WebSocket(
    `${server.replace(/^http/u, 'ws')}/ws/corps/${demo.corp_id}?actor_id=${demo.bob_actor_id}&after_seq=${afterSeq}`,
  )
  const ready = new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      socket.close()
      reject(new Error('Recovery event replay did not become ready'))
    }, 15_000)
    socket.onmessage = ({ data }) => {
      const message = JSON.parse(data)
      if (message.type === 'event') events.push(message.event)
      if (message.type === 'ready') {
        clearTimeout(timer)
        resolve()
      }
    }
    socket.onerror = () => {
      failure = new Error('Recovery event socket failed')
      clearTimeout(timer)
      reject(failure)
    }
    socket.onclose = () => {
      failure ??= new Error('Recovery event socket closed')
      clearTimeout(timer)
      reject(failure)
    }
  })
  try {
    await ready
  } catch (error) {
    socket.close()
    throw error
  }
  return { socket, events, failure: () => failure }
}

function assertIncreasingEvents(events) {
  for (let index = 1; index < events.length; index += 1) {
    assert.ok(events[index - 1].seq < events[index].seq, 'Live/replay sequences must increase')
  }
  assert.equal(new Set(events.map((event) => event.id)).size, events.length)
}

async function decideWithOrderedEvents(demo, workItemId, runId, decision) {
  const before = await snapshot(demo)
  const cursor = Math.max(0, ...before.snapshot.events.map((event) => event.seq))
  const live = await openRecoveryEventClient(demo, cursor)
  let replay
  const decisionType = decision.approved ? 'verification.approved' : 'verification.rejected'
  const factoryType = decision.approved ? 'factory.verified' : 'factory.verification_failed'
  const relevant = (event) =>
    (event.type === decisionType && event.aggregate_id === runId) ||
    (event.type === factoryType && event.aggregate_id === workItemId && event.payload.run_id === runId)
  try {
    const response = await post(
      `/api/corps/${demo.corp_id}/runs/${runId}/verification-decision`,
      decision,
    )
    const deadline = Date.now() + 15_000
    while (live.events.filter(relevant).length < 2) {
      if (live.failure()) throw live.failure()
      assert.ok(Date.now() < deadline, `Missing live ${decisionType}/${factoryType} pair`)
      await new Promise((resolve) => setTimeout(resolve, 25))
    }
    assertIncreasingEvents(live.events)
    replay = await openRecoveryEventClient(demo, cursor)
    assertIncreasingEvents(replay.events)
    const livePair = live.events.filter(relevant).map(({ id, seq, type }) => ({ id, seq, type }))
    const replayPair = replay.events.filter(relevant).map(({ id, seq, type }) => ({ id, seq, type }))
    assert.deepEqual(replayPair, livePair)
    assert.equal(livePair.length, 2)
    return { response, live: livePair, replay: replayPair }
  } finally {
    replay?.socket.close()
    live.socket.close()
  }
}

async function assertMemberCannotResumeFactoryFailure(demo, sourceRun) {
  assert.ok(sourceRun.provider_session_id, 'Exercise a real resumable fixture session, not a missing-session error')
  const before = await snapshot(demo)
  assert.equal(before.snapshot.actors.find((actor) => actor.id === demo.bob_actor_id)?.role, 'member')
  const rejected = await request(`/api/corps/${demo.corp_id}/runs/${sourceRun.id}/resume`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      requested_by: demo.bob_actor_id,
      prompt: 'Attempt to bypass governed verification recovery through native resume.',
    }),
  })
  assert.equal(rejected.response.status, 409)
  assert.match(rejected.body.error, /factory verification-failed lineage requires governed verification recovery/u)
  const after = await snapshot(demo)
  assert.deepEqual(
    after.snapshot.runs.map((run) => run.id).sort(),
    before.snapshot.runs.map((run) => run.id).sort(),
  )
  const previousTask = before.snapshot.tasks.find((task) => task.id === sourceRun.task_id)
  const currentTask = after.snapshot.tasks.find((task) => task.id === sourceRun.task_id)
  assert.equal(currentTask.status, previousTask.status)
  assert.equal(currentTask.attempt_count, previousTask.attempt_count)
  return { status: rejected.response.status, new_runs: 0 }
}

const recoveryManualGate = {
  type: 'independent_review',
  roles: ['owner', 'admin', 'manager', 'member'],
  exclude_requester: true,
}

async function heldRecoveryFixture(demo, number, strategy, verificationPolicy, maxAttempts = 2, adapter = 'fake-process') {
  const issue = {
    id: `I_FACTORY_STORE_RECOVERY_${number}`,
    number,
    title: `Bounded store recovery regression ${number}`,
    body: 'Verify the same issue, task, workspace and policy through governed recovery.',
    url: `https://github.com/shyamsridhar123/ecorp/issues/${number}`,
    updatedAt: '2026-09-04T02:00:00Z',
  }
  const itemId = `PVTI_FACTORY_STORE_RECOVERY_${number}`
  const policy = {
    schema_version: 1,
    source_of_truth: 'github_project',
    project_owner: 'acme',
    project_number: 7,
    project_status: 'Todo',
    required_label: 'factory:ready',
    dependencies: [],
    repository_allowlist: ['shyamsridhar123/ecorp'],
    source_base_ref: 'HEAD',
    source_base_commit: sourceBaseCommit,
    source_commit_upgrade_required: false,
    adapter_allowlist: [adapter],
    strategy_allowlist: [strategy],
    model: null,
    reasoning_effort: null,
    write_scope: ['**'],
    allowed_tools: ['filesystem', 'shell'],
    prohibited_actions: [
      'modify files outside the assigned worktree',
      'use undeclared long-lived credentials',
      'merge or deploy without a separate current authorization',
    ],
    secret_ids: [],
    verification_required: true,
    budget_tokens: 100_000,
    budget_cost_microusd: 1_000_000,
    auto_merge: false,
  }
  const materialization = {
    actor_id: demo.alice_actor_id,
    title: issue.title,
    description: issue.body,
    preferred_adapter: adapter,
    strategy,
    budget_tokens: policy.budget_tokens,
    budget_cost_microusd: policy.budget_cost_microusd,
    deliverable: { form: 'commit_branch', commit_after_verification: true, paths: [] },
    contract: {
      objective: issue.body,
      expected_output: 'A verified bounded fixture in the same source lineage.',
      acceptance_tests: ['All persisted verifier checks pass.'],
      allowed_tools: policy.allowed_tools,
      prohibited_actions: policy.prohibited_actions,
      references: [issue.url],
      write_scope: policy.write_scope,
    },
    verification_policy: verificationPolicy,
  }
  const preflight = await post(`/api/corps/${demo.corp_id}/factory/preflight`, {
    ...materialization,
    source_repository_owner: 'shyamsridhar123',
    source_repository_name: 'ecorp',
    policy,
  })
  assert.equal(preflight.valid, true)
  const claim = await post(`/api/corps/${demo.corp_id}/factory/work-items/claim`, {
    actor_id: demo.alice_actor_id,
    source_project_owner: policy.project_owner,
    source_project_number: policy.project_number,
    source_project_item_id: itemId,
    source_repository_owner: 'shyamsridhar123',
    source_repository_name: 'ecorp',
    source_issue_number: number,
    source_issue_node_id: issue.id,
    source_issue_url: issue.url,
    source_title: issue.title,
    source_revision: issue.updatedAt,
    idempotency_key: `store-recovery-${number}-claim`,
    lease_seconds: 300,
    policy,
  })
  assert.equal(claim.replayed, false, 'Use fresh QA; never adopt an existing completed fixture')
  const materialized = await post(
    `/api/corps/${demo.corp_id}/factory/work-items/${claim.work_item.id}/materialize`,
    {
      ...materialization,
      claim_token: claim.claim_token,
      expected_version: claim.work_item.version,
      idempotency_key: `store-recovery-${number}-materialize`,
    },
  )
  const missionId = materialized.mission_id
  const held = await snapshot(demo)
  const tasks = held.snapshot.tasks.filter((task) => task.mission_id === missionId)
  assert.ok(tasks.length > 0)
  assert.ok(tasks.every((task) => task.required_adapter === adapter))
  assert.equal(held.snapshot.runs.filter((run) => tasks.some((task) => task.id === run.task_id)).length, 0)
  // Reuse the native held-graph contract revision API to give each task the same
  // explicit policy before execution, rather than manufacturing terminal states.
  for (const task of tasks) {
    await post(`/api/corps/${demo.corp_id}/missions/${missionId}/contract-revisions`, {
      actor_id: demo.alice_actor_id,
      task_id: task.id,
      expected_contract_version: task.contract_version,
      next_action: 'redispatch',
      source_run_id: null,
      reason: 'Configure the fresh held recovery regression before its first run.',
      idempotency_key: randomUUID(),
      description: issue.body,
      contract: task.contract,
      verification_policy: verificationPolicy,
    })
  }
  if (maxAttempts !== 2) {
    assert.ok(maxAttempts > 2 && maxAttempts <= 4)
    // Fixture configuration only: never reset an attempt or alter a live task.
    await psql(`
      UPDATE tasks SET max_attempts = ${maxAttempts}
      WHERE mission_id = ${sqlLiteral(missionId)}::uuid AND attempt_count = 0
        AND NOT EXISTS (SELECT 1 FROM runs WHERE runs.task_id = tasks.id);
    `)
  }
  return {
    demo, issue, itemId, missionId, tasks,
    workItemId: claim.work_item.id,
    claimToken: claim.claim_token,
  }
}

async function authorizeFixtureRecovery(fixture, reason, { mode = 'verifier_only', contractRevisionId = null } = {}) {
  const { demo, workItemId } = fixture
  const context = await requestOk(
    `/api/corps/${demo.corp_id}/factory/work-items/${workItemId}/verification-recoveries?actor_id=${demo.alice_actor_id}`,
  )
  const renewed = await post(`/api/corps/${demo.corp_id}/factory/work-items/${workItemId}/renew`, {
    actor_id: demo.alice_actor_id,
    claim_token: fixture.claimToken,
    expected_version: context.work_item.version,
    idempotency_key: randomUUID(),
    lease_seconds: 300,
  })
  const body = {
    actor_id: demo.alice_actor_id,
    claim_token: fixture.claimToken,
    expected_factory_version: renewed.work_item.version,
    idempotency_key: randomUUID(),
    source_run_id: context.source_run_id,
    mode,
    reason,
    observed_source_revision: fixture.issue.updatedAt,
    reviewed_source_snapshot: {
      source_revision: fixture.issue.updatedAt,
      issue_number: fixture.issue.number,
      issue_node_id: fixture.issue.id,
      issue_url: fixture.issue.url,
      title: fixture.issue.title,
      body: fixture.issue.body,
      repository: 'shyamsridhar123/ecorp',
      project_owner: 'acme',
      project_number: 7,
      project_item_id: fixture.itemId,
    },
    contract_revision_id: contractRevisionId,
    expected_workspace_fingerprint: context.workspace_fingerprint,
    expected_head_commit: context.expected_head_commit,
  }
  const route = `/api/corps/${demo.corp_id}/factory/work-items/${workItemId}/verification-recoveries`
  return { result: await post(route, body), body, route, context }
}

async function waitForReviewRun(demo, taskId, runId) {
  return waitFor(demo, (state) => state.runs.find((run) =>
    run.task_id === taskId && (!runId || run.id === runId) &&
    run.status === 'waiting_for_approval' && run.workspace_disposition === 'preserved' &&
    run.workspace_fingerprint,
  ), 'exact review run with completed workspace cleanup')
}

function decideFixtureRun(demo, runId, approved, note) {
  return post(`/api/corps/${demo.corp_id}/runs/${runId}/verification-decision`, {
    actor_id: demo.bob_actor_id,
    approved,
    note,
    decision_key: randomUUID(),
  })
}

async function verifierOnlyRecovery() {
  const demo = await post('/api/demo/bootstrap', {})
  const issue = {
    id: 'I_FACTORY_RECOVERY_9100',
    number: 9100,
    title: 'Recover accepted source through verifier-only execution',
    body: [
      '## Outcome',
      '',
      'Preserve the source and rerun its authoritative checks.',
      '',
      '## Acceptance criteria',
      '',
      '- [ ] Provider runs exactly once.',
      '- [ ] Recovery verifies the preserved workspace.',
      '',
    ].join('\n'),
    url: 'https://github.com/shyamsridhar123/ecorp/issues/9100',
    state: 'OPEN',
    createdAt: '2026-09-04T00:00:00Z',
    updatedAt: '2026-09-04T00:00:00Z',
    labels: [{ name: 'factory:ready' }],
  }
  const statePath = path.join(output, 'factory-verifier-recovery-github.json')
  const policyPath = path.join(output, 'factory-verifier-recovery-policy.json')
  const policy = {
    checks: [
      { type: 'artifact', min_bytes: 1 },
      {
        type: 'command',
        program: 'node',
        args: [
          '-e',
          "const fs=require('node:fs');const p='verifier-command-side-effect.txt';const existed=fs.existsSync(p);const n=existed?Number(fs.readFileSync(p,'utf8'))+1:1;fs.writeFileSync(p,String(n));if(existed)fs.writeFileSync('result.md','snapshot-only corruption')",
        ],
        timeout_ms: 30_000,
      },
      { type: 'file', path: 'result.md', min_bytes: 50 },
    ],
    manual_gate: {
      type: 'independent_review',
      roles: ['owner', 'admin', 'manager', 'member'],
      exclude_requester: true,
    },
  }
  await writeFile(
    statePath,
    `${JSON.stringify(
      projectState(
        issue,
        'PVTI_FACTORY_RECOVERY_9100',
        'PVT_FACTORY_RECOVERY',
        'PVTSSF_FACTORY_RECOVERY',
      ),
      null,
      2,
    )}\n`,
  )
  await writeFile(policyPath, `${JSON.stringify(policy, null, 2)}\n`)
  const first = await runController(demo, issue.number, statePath, policyPath, {
    adapter: 'fake-process',
  })
  const firstWaiting = await waitFor(
    demo,
    (state) => {
      const task = state.tasks.find((item) => item.mission_id === first.mission_id)
      const run = state.runs.find(
        (item) =>
          item.task_id === task?.id &&
          item.status === 'waiting_for_approval' &&
          item.workspace_fingerprint,
      )
      return run ? { task, run } : null
    },
    'initial independent review',
  )
  const sourceRun = firstWaiting.value.run
  const sourceDeliverable = firstWaiting.state.snapshot.source_deliverables.find(
    (deliverable) => deliverable.run_id === sourceRun.id,
  )
  assert.ok(sourceDeliverable?.head_commit)
  const rejectionKey = randomUUID()
  const rejection = {
    actor_id: demo.bob_actor_id,
    approved: false,
    note: 'Independent reviewer requests a provider-free recheck.',
    decision_key: rejectionKey,
  }
  const rejectionEvents = await decideWithOrderedEvents(
    demo,
    first.factory_work_item_id,
    sourceRun.id,
    rejection,
  )
  const rejected = rejectionEvents.response
  assert.equal(rejected.replayed, false)
  const decisionReplay = await post(
    `/api/corps/${demo.corp_id}/runs/${sourceRun.id}/verification-decision`,
    rejection,
  )
  assert.equal(decisionReplay.replayed, true)
  const failed = await waitFor(
    demo,
    (state) => {
      const item = state.factory_work_items.find(
        (candidate) => candidate.id === first.factory_work_item_id,
      )
      const mission = state.missions.find(
        (candidate) => candidate.id === first.mission_id,
      )
      return item?.state === 'verification_failed' && mission?.status === 'failed'
        ? { item, mission }
        : null
    },
    'automatic factory failure reconciliation',
  )
  assert.match(failed.value.item.failure_detail, /provider-free recheck/i)
  assert.ok(sourceRun.artifact_id)
  await psql(`
    UPDATE runs
    SET workspace_fingerprint = NULL
    WHERE id = ${sqlLiteral(sourceRun.id)}::uuid;
  `)
  const legacySnapshot = await snapshot(demo)
  const legacySourceRun = legacySnapshot.snapshot.runs.find(
    (run) => run.id === sourceRun.id,
  )
  assert.equal(legacySourceRun.workspace_fingerprint, null)
  // Only the legacy workspace field is missing. Artifact metadata is signed and
  // must not be edited to synthesize an older row: hydration correctly rejects that.
  const originalArtifact = await fetch(
    `${server}/api/corps/${demo.corp_id}/artifacts/${sourceRun.artifact_id}?actor_id=${demo.alice_actor_id}`,
    { signal: AbortSignal.timeout(30_000) },
  )
  assert.equal(originalArtifact.status, 200)
  await originalArtifact.arrayBuffer()
  const recoveryReason =
    'Re-run the same persisted policy against the exact preserved workspace without starting the provider.'
  const recovered = await runController(
    demo,
    issue.number,
    statePath,
    policyPath,
    {
      adapter: 'fake-process',
      recovery: 'verifier-only',
      recoveryReason,
    },
  )
  assert.equal(recovered.launch.legacy_workspace_checkpointed, true)
  const recoveredWaiting = await waitFor(
    demo,
    (state) => {
      const task = state.tasks.find((item) => item.mission_id === first.mission_id)
      const run = state.runs.find(
        (item) =>
          item.task_id === task?.id &&
          item.id !== sourceRun.id &&
          item.status === 'waiting_for_approval',
      )
      const item = state.factory_work_items.find(
        (candidate) => candidate.id === first.factory_work_item_id,
      )
      return run && item?.state === 'awaiting_approval'
        ? { task, run, item }
        : null
    },
    'verifier-only recovery review',
  )
  const recoveryRun = recoveredWaiting.value.run
  await psql(`
    INSERT INTO factory_verification_recoveries (
      id, corp_id, factory_work_item_id, mission_id, task_id,
      source_run_id, replacement_run_id, mode, status, authorized_by,
      reason, idempotency_key, observed_source_revision,
      reviewed_source_snapshot, contract_revision_id,
      previous_verification_policy, replacement_verification_policy, request,
      created_at, updated_at
    )
    SELECT
      gen_random_uuid(),
      ${sqlLiteral(demo.corp_id)}::uuid,
      ${sqlLiteral(first.factory_work_item_id)}::uuid,
      ${sqlLiteral(first.mission_id)}::uuid,
      ${sqlLiteral(recoveryRun.task_id)}::uuid,
      ${sqlLiteral(sourceRun.id)}::uuid,
      NULL,
      'verifier_only',
      'failed',
      ${sqlLiteral(demo.alice_actor_id)}::uuid,
      'historical bounded-snapshot recovery ' || series,
      gen_random_uuid(),
      ${sqlLiteral(issue.updatedAt)},
      '{}'::jsonb,
      NULL,
      '{"checks":[],"manual_gate":null}'::jsonb,
      '{"checks":[],"manual_gate":null}'::jsonb,
      '{}'::jsonb,
      now() + series * interval '1 millisecond',
      now() + series * interval '1 millisecond'
    FROM generate_series(1, 101) AS series
  `)
  const boundedSnapshot = await snapshot(demo)
  assert.ok(
    !boundedSnapshot.snapshot.factory_verification_recoveries.some(
      (recovery) => recovery.id === recovered.launch.recovery_id,
    ),
  )
  const exactContext = await requestOk(
    `/api/corps/${demo.corp_id}/factory/work-items/${first.factory_work_item_id}/verification-recoveries?actor_id=${demo.alice_actor_id}`,
  )
  assert.ok(
    exactContext.recoveries.some(
      (recovery) => recovery.id === recovered.launch.recovery_id,
    ),
  )
  const mismatchedReplay = await controllerFailure(
    demo,
    issue.number,
    statePath,
    policyPath,
    {
      adapter: 'fake-process',
      recovery: 'verifier-only',
      recoveryReason:
        'A different reason must not reuse or mutate the active recovery.',
    },
  )
  assert.match(mismatchedReplay, /active factory recovery reason/i)
  const replay = await runController(
    demo,
    issue.number,
    statePath,
    policyPath,
    {
      adapter: 'fake-process',
      recovery: 'verifier-only',
      recoveryReason,
    },
  )
  assert.equal(replay.launch.recovery_id, recovered.launch.recovery_id)
  assert.equal(replay.launch.run_id, recovered.launch.run_id)
  const checkpointedSourceRun = recoveredWaiting.state.snapshot.runs.find(
    (run) => run.id === sourceRun.id,
  )
  assert.match(checkpointedSourceRun.workspace_fingerprint, /^[0-9a-f]{64}$/)
  assert.equal(recoveryRun.execution_mode, 'verification_only')
  assert.equal(recoveryRun.resumed_from_run_id, sourceRun.id)
  assert.equal(recoveryRun.workspace_run_id, sourceRun.workspace_run_id)
  assert.equal(recoveryRun.provider_session_id, null)
  const recoveryEvents = recoveredWaiting.state.snapshot.events.filter(
    (event) => event.aggregate_id === recoveryRun.id,
  )
  assert.ok(
    !recoveryEvents.some((event) =>
      ['run.session', 'run.session_terminated', 'run.output', 'run.artifact'].includes(
        event.type,
      ),
    ),
  )
  const recoveryEvidence =
    recoveredWaiting.state.snapshot.verification_evidence.filter(
      (item) => item.run_id === recoveryRun.id,
    )
  assert.equal(recoveryEvidence.length, 3)
  assert.ok(recoveryEvidence.every((item) => item.status === 'passed'))
  assert.equal(
    await readFile(path.join(sourceRun.workspace_path, 'verifier-command-side-effect.txt'), 'utf8'),
    '1',
  )
  assert.ok(
    (
      await readFile(path.join(sourceRun.workspace_path, 'result.md'), 'utf8')
    ).length >= 50,
  )
  const recoveryDeliverable =
    recoveredWaiting.state.snapshot.source_deliverables.find(
      (deliverable) => deliverable.run_id === recoveryRun.id,
    )
  assert.equal(recoveryDeliverable?.head_commit, sourceDeliverable.head_commit)
  const approvalEvents = await decideWithOrderedEvents(
    demo,
    first.factory_work_item_id,
    recoveryRun.id,
    {
      actor_id: demo.bob_actor_id,
      approved: true,
      note: 'Independent reviewer accepts the provider-free recheck.',
      decision_key: randomUUID(),
    },
  )
  const completed = await waitFor(
    demo,
    (state) => {
      const item = state.factory_work_items.find(
        (candidate) => candidate.id === first.factory_work_item_id,
      )
      const mission = state.missions.find(
        (candidate) => candidate.id === first.mission_id,
      )
      return item?.state === 'verified' && mission?.status === 'completed'
        ? { item, mission }
        : null
    },
    'verified verifier-only recovery',
  )
  const taskRuns = completed.state.snapshot.runs.filter(
    (run) => run.task_id === recoveryRun.task_id,
  )
  assert.equal(taskRuns.length, 2)
  return {
    factory_work_item_id: first.factory_work_item_id,
    mission_id: first.mission_id,
    source_run_id: sourceRun.id,
    recovery_run_id: recoveryRun.id,
    decision_replay: decisionReplay.replayed,
    decision_event_order: { rejection: rejectionEvents.live, approval: approvalEvents.live },
    decision_live_replay_exact: true,
    controller_replay: replay.launch.recovered,
    same_workspace_lineage:
      recoveryRun.workspace_run_id === sourceRun.workspace_run_id,
    same_head_commit:
      recoveryDeliverable.head_commit === sourceDeliverable.head_commit,
    legacy_workspace_checkpointed:
      recovered.launch.legacy_workspace_checkpointed,
    original_signed_provider_metadata_preserved: true,
    verifier_side_effect_isolated: true,
    provider_events_on_recovery: recoveryEvents.filter((event) =>
      ['run.session', 'run.session_terminated', 'run.output', 'run.artifact'].includes(
        event.type,
      ),
    ).length,
    final_factory_state: completed.value.item.state,
    run_count: taskRuns.length,
  }
}

async function sourceCorrectionRecovery() {
  const demo = await post('/api/demo/bootstrap', {})
  const issue = {
    id: 'I_FACTORY_RECOVERY_9200',
    number: 9200,
    title: 'Correct rejected source in the same factory lineage',
    body: [
      '## Outcome',
      '',
      'Resume the preserved provider session and create resumed.txt.',
      '',
      '## Acceptance criteria',
      '',
      '- [ ] The corrected run reuses the provider session and worktree.',
      '- [ ] resumed.txt is verified.',
      '',
    ].join('\n'),
    url: 'https://github.com/shyamsridhar123/ecorp/issues/9200',
    state: 'OPEN',
    createdAt: '2026-09-04T01:00:00Z',
    updatedAt: '2026-09-04T01:00:00Z',
    labels: [{ name: 'factory:ready' }],
  }
  const statePath = path.join(output, 'factory-source-recovery-github.json')
  const policyPath = path.join(output, 'factory-source-recovery-policy.json')
  const weakenedPolicyPath = path.join(
    output,
    'factory-source-recovery-weakened-policy.json',
  )
  const github = projectState(
    issue,
    'PVTI_FACTORY_SOURCE_9200',
    'PVT_FACTORY_SOURCE',
    'PVTSSF_FACTORY_SOURCE',
  )
  const policy = {
    checks: [
      { type: 'artifact', min_bytes: 1 },
      { type: 'file', path: 'resumed.txt', min_bytes: 8 },
    ],
    manual_gate: {
      type: 'independent_review',
      roles: ['owner', 'admin', 'manager', 'member'],
      exclude_requester: true,
    },
  }
  await writeFile(statePath, `${JSON.stringify(github, null, 2)}\n`)
  await writeFile(policyPath, `${JSON.stringify(policy, null, 2)}\n`)
  await writeFile(
    weakenedPolicyPath,
    `${JSON.stringify({ ...policy, manual_gate: null }, null, 2)}\n`,
  )
  const first = await runController(demo, issue.number, statePath, policyPath, {
    adapter: 'codex',
  })
  const failed = await waitFor(
    demo,
    (state) => {
      const task = state.tasks.find((item) => item.mission_id === first.mission_id)
      const run = state.runs.find(
        (item) =>
          item.task_id === task?.id &&
          item.status === 'failed' &&
          item.workspace_fingerprint,
      )
      const item = state.factory_work_items.find(
        (candidate) => candidate.id === first.factory_work_item_id,
      )
      return run && item?.state === 'verification_failed'
        ? { task, run, item }
        : null
    },
    'initial source verification failure',
  )
  const sourceRun = failed.value.run
  assert.ok(sourceRun.provider_session_id)
  const memberResumeRejection = await assertMemberCannotResumeFactoryFailure(demo, sourceRun)
  await writeFile(
    path.join(output, 'member-generic-resume-rejection.json'),
    `${JSON.stringify({ source_run_id: sourceRun.id, ...memberResumeRejection }, null, 2)}\n`,
  )
  const sourceAgent = failed.state.snapshot.agents.find(
    (agent) => agent.id === sourceRun.agent_id,
  )
  assert.ok(sourceAgent)
  const busyMissionId = randomUUID()
  const busyTaskId = randomUUID()
  const busyRunId = randomUUID()
  // Owned negative-state fixture: a reusable identity is busy in another mission.
  // No provider is started for this row, and the original factory lineage is not altered.
  await psql(`
    INSERT INTO missions
    SELECT (jsonb_populate_record(NULL::missions, to_jsonb(mission) ||
      jsonb_build_object('id', ${sqlLiteral(busyMissionId)}, 'status', 'running',
                         'title', 'Independent busy-agent recovery fence fixture'))).*
    FROM missions mission WHERE id = ${sqlLiteral(first.mission_id)}::uuid;
    INSERT INTO tasks
    SELECT (jsonb_populate_record(NULL::tasks, to_jsonb(task) ||
      jsonb_build_object('id', ${sqlLiteral(busyTaskId)},
                         'mission_id', ${sqlLiteral(busyMissionId)},
                         'status', 'claimed', 'verification_status', 'pending'))).*
    FROM tasks task WHERE id = ${sqlLiteral(sourceRun.task_id)}::uuid;
    INSERT INTO runs (id, corp_id, task_id, agent_id, runner_id,
                      assignment_token, status, workspace_run_id)
    SELECT ${sqlLiteral(busyRunId)}::uuid, corp_id, ${sqlLiteral(busyTaskId)}::uuid,
           agent_id, runner_id, gen_random_uuid(), 'waiting_for_input',
           ${sqlLiteral(busyRunId)}::uuid
    FROM runs WHERE id = ${sqlLiteral(sourceRun.id)}::uuid;
    UPDATE agents SET current_run_id = ${sqlLiteral(busyRunId)}::uuid,
                      status = 'working', pinned = TRUE, retired_at = NULL
    WHERE id = ${sqlLiteral(sourceRun.agent_id)}::uuid;
  `)
  const busyRejected = await controllerFailure(
    demo,
    issue.number,
    statePath,
    policyPath,
    {
      adapter: 'codex',
      recovery: 'verifier-only',
      recoveryReason: 'An active assignment in another mission must fence recovery.',
    },
  )
  assert.match(busyRejected, /task or assigned agent already has an active run/u)
  const busyState = (await snapshot(demo)).snapshot
  assert.equal(
    busyState.agents.find((agent) => agent.id === sourceRun.agent_id)?.current_run_id,
    busyRunId,
  )
  assert.equal(busyState.runs.filter((run) => run.task_id === sourceRun.task_id).length, 1)
  assert.equal(
    busyState.factory_verification_recoveries.filter(
      (recovery) => recovery.factory_work_item_id === first.factory_work_item_id,
    ).length,
    0,
  )
  await psql(`
    UPDATE runs SET status = 'cancelled', updated_at = now()
      WHERE id = ${sqlLiteral(busyRunId)}::uuid;
    UPDATE tasks SET status = 'cancelled', updated_at = now()
      WHERE id = ${sqlLiteral(busyTaskId)}::uuid;
    UPDATE missions SET status = 'cancelled', updated_at = now()
      WHERE id = ${sqlLiteral(busyMissionId)}::uuid;
    UPDATE agents SET current_run_id = NULL, status = 'idle',
                      pinned = ${sourceAgent.pinned ? 'TRUE' : 'FALSE'}
      WHERE id = ${sqlLiteral(sourceRun.agent_id)}::uuid;
  `)
  const missionOwned = sourceAgent.mission_id === first.mission_id && !sourceAgent.pinned
  if (missionOwned) {
    await waitFor(
      demo,
      (state) => state.agents.find((agent) => agent.id === sourceRun.agent_id)?.retired_at,
      'terminal mission worker retirement before explicit recovery',
      30_000,
    )
  }
  const originalSourceEvidence = failed.state.snapshot.verification_evidence.filter(
    (entry) => entry.run_id === sourceRun.id,
  )
  assert.equal(originalSourceEvidence.length, 2)
  assert.equal(originalSourceEvidence.find((entry) => entry.check_index === 0)?.status, 'passed')
  const originalFileCheck = originalSourceEvidence.find((entry) => entry.check_index === 1)
  assert.equal(originalFileCheck?.status, 'failed')
  assert.match(originalFileCheck.summary, /resumed\.txt does not exist/u)
  const providerArtifactName = await psql(`
    SELECT file_name FROM artifacts
    WHERE id = ${sqlLiteral(sourceRun.artifact_id)}::uuid;
  `)
  assert.equal(path.basename(providerArtifactName), providerArtifactName)
  assert.equal(
    existsSync(path.join(sourceRun.workspace_path, providerArtifactName)),
    false,
    'the Codex bridge must exercise evidence stored outside the source worktree',
  )
  const artifactResponse = await fetch(
    `${server}/api/corps/${demo.corp_id}/artifacts/${sourceRun.artifact_id}?actor_id=${demo.alice_actor_id}`,
    { signal: AbortSignal.timeout(30_000) },
  )
  assert.equal(artifactResponse.status, 200)
  const durableArtifactBytes = Buffer.from(await artifactResponse.arrayBuffer())
  assert.equal(
    createHash('sha256').update(durableArtifactBytes).digest('hex'),
    sourceRun.artifact_sha256,
  )
  await psql(`
    UPDATE tasks
    SET max_attempts = 4
    WHERE id = ${sqlLiteral(failed.value.task.id)}::uuid;
  `)
  const verifierOnlyBridge = await runController(
    demo,
    issue.number,
    statePath,
    policyPath,
    {
      adapter: 'codex',
      recovery: 'verifier-only',
      recoveryReason:
        'Re-run the unchanged failing verifier before a later source correction.',
    },
  )
  const verifierOnlyFailed = await waitFor(
    demo,
    (state) => {
      const run = state.runs.find(
        (candidate) => candidate.id === verifierOnlyBridge.launch.run_id,
      )
      const recovery = state.factory_verification_recoveries.find(
        (candidate) => candidate.id === verifierOnlyBridge.launch.recovery_id,
      )
      const item = state.factory_work_items.find(
        (candidate) => candidate.id === first.factory_work_item_id,
      )
      return run?.status === 'failed' &&
        run.execution_mode === 'verification_only' &&
        run.provider_session_id === null &&
        run.workspace_disposition === 'preserved' &&
        recovery?.status === 'failed' &&
        item?.state === 'verification_failed'
        ? { run, recovery, item }
        : null
    },
    'failed verifier-only bridge before source correction',
  )
  const verifierOnlyRun = verifierOnlyFailed.value.run
  if (missionOwned) {
    const requestEvent = verifierOnlyFailed.state.snapshot.events.find(
      (event) =>
        event.type === 'run.verification_requested' &&
        event.aggregate_id === verifierOnlyRun.id,
    )
    assert.equal(requestEvent?.payload.mission_worker_reactivated, true)
  }
  const bridgeEvidence = verifierOnlyFailed.state.snapshot.verification_evidence.filter(
    (entry) => entry.run_id === verifierOnlyRun.id,
  )
  assert.equal(bridgeEvidence.length, 2)
  assert.equal(
    bridgeEvidence.find((entry) => entry.check_index === 0)?.status,
    'passed',
    'the original durable artifact must verify without a local source copy',
  )
  assert.equal(
    bridgeEvidence.find((entry) => entry.check_index === 1)?.status,
    'failed',
    'artifact transfer must not manufacture the originally missing source file',
  )
  assert.equal(verifierOnlyRun.artifact_id, sourceRun.artifact_id)
  assert.equal(verifierOnlyRun.workspace_fingerprint, sourceRun.workspace_fingerprint)
  assert.equal(existsSync(path.join(sourceRun.workspace_path, providerArtifactName)), false)
  assert.equal(
    await psql(`
      SELECT bool_and(NOT ((payload->'provider_artifact') ? 'data_base64'))
      FROM runner_commands
      WHERE run_id = ${sqlLiteral(verifierOnlyRun.id)}::uuid
        AND command_kind = 'factory_verification_recovery';
    `),
    't',
    'durable recovery commands must not contain hydrated artifact bytes',
  )
  const result = await finishSourceCorrectionRecovery({
    demo, issue, statePath, policyPath, weakenedPolicyPath, github, first, failed,
    sourceRun, verifierOnlyRun, missionOwned,
  })
  return { ...result, member_generic_resume_rejection: memberResumeRejection }
}

async function finishSourceCorrectionRecovery({
  demo, issue, statePath, policyPath, weakenedPolicyPath, github, first, failed,
  sourceRun, verifierOnlyRun, missionOwned,
}) {
  const reviewedCorrection = '\n## Reviewed correction\n\nCreate only resumed.txt in the preserved workspace.\n'
  if (!github.issues['9200'].body.includes(reviewedCorrection)) {
    github.issues['9200'].body += reviewedCorrection
  }
  github.issues['9200'].updatedAt = '2026-09-04T01:30:00Z'
  github.items[0].content.body = github.issues['9200'].body
  await writeFile(statePath, `${JSON.stringify(github, null, 2)}\n`)
  const unaudited = await controllerFailure(
    demo,
    issue.number,
    statePath,
    policyPath,
    { adapter: 'codex' },
  )
  assert.match(
    unaudited,
    /explicit --verification-recovery|source revision|no eligible factory issue.*already linked to factory state blocked/is,
  )
  const beforeRestart = await snapshot(demo)
  const runsBeforeRestart = beforeRestart.snapshot.runs.filter(
    (run) => run.task_id === failed.value.task.id,
  ).length
  const weakenedPolicy = await controllerFailure(
    demo,
    issue.number,
    statePath,
    weakenedPolicyPath,
    {
      adapter: 'codex',
      recovery: 'source-correction',
      recoveryReason:
        'This attempted recovery must not remove the independent-review gate.',
    },
  )
  assert.match(weakenedPolicy, /manual verification gate|cannot remove|cannot change/i)
  const afterWeakenedPolicy = await snapshot(demo)
  const unchangedTask = afterWeakenedPolicy.snapshot.tasks.find(
    (task) => task.id === failed.value.task.id,
  )
  assert.equal(unchangedTask.contract_version, 1)
  assert.equal(
    afterWeakenedPolicy.snapshot.runs.filter(
      (run) => run.task_id === failed.value.task.id,
    ).length,
    runsBeforeRestart,
  )
  const restarted = await restartTestServer()
  const secret = await post(`/api/corps/${demo.corp_id}/secrets`, {
    actor_id: demo.alice_actor_id,
    name: `factory-recovery-${randomUUID()}`,
    value: `recovery-secret-${randomUUID()}`,
    allowed_actor_ids: [demo.alice_actor_id],
    allowed_tools: ['filesystem'],
    resource_prefix: 'workspace:',
    max_ttl_seconds: 120,
  })
  const secretRefs = [
    {
      secret_id: secret.secret_id,
      env_name: 'CRONY_RECOVERY_SECRET',
      tool: 'filesystem',
      resource: 'workspace:source-correction',
    },
  ]
  await psql(`
    UPDATE tasks
    SET contract = jsonb_set(
          contract,
          '{secret_refs}',
          ${sqlLiteral(JSON.stringify(secretRefs))}::jsonb,
          true
        ),
        max_attempts = 4
    WHERE id = ${sqlLiteral(failed.value.task.id)}::uuid;

    UPDATE factory_work_items
    SET policy = jsonb_set(
          policy,
          '{secret_ids}',
          ${sqlLiteral(JSON.stringify([secret.secret_id]))}::jsonb,
          true
        )
    WHERE id = ${sqlLiteral(first.factory_work_item_id)}::uuid;
  `)
  await post(`/api/corps/${demo.corp_id}/secrets/${secret.secret_id}/revoke`, {
    actor_id: demo.alice_actor_id,
    reason: 'Inject a deterministic recovery-dispatch secret failure.',
  })
  const failedDispatchReason =
    'This recovery must fail before runner dispatch because its scoped secret is revoked.'
  const failedDispatchError = await controllerFailure(
    demo,
    issue.number,
    statePath,
    policyPath,
    {
      adapter: 'codex',
      recovery: 'source-correction',
      recoveryReason: failedDispatchReason,
    },
  )
  assert.match(failedDispatchError, /factory mission .*failed|verification_failed/i)
  const failedDispatch = await waitFor(
    demo,
    (state) => {
      const task = state.tasks.find((item) => item.id === failed.value.task.id)
      const item = state.factory_work_items.find(
        (candidate) => candidate.id === first.factory_work_item_id,
      )
      const recovery = state.factory_verification_recoveries.find(
        (candidate) =>
          candidate.source_run_id === verifierOnlyRun.id &&
          candidate.status === 'failed',
      )
      const run = state.runs.find(
        (candidate) =>
          candidate.id === recovery?.replacement_run_id &&
          candidate.status === 'failed' &&
          candidate.workspace_detail === 'dispatch_not_started',
      )
      return task?.status === 'verification_failed' &&
        item?.state === 'verification_failed' &&
        recovery &&
        run
        ? { task, item, recovery, run }
        : null
    },
    'terminal recovery dispatch failure',
  )
  assert.equal(
    failedDispatch.state.snapshot.factory_verification_recoveries.filter(
      (candidate) =>
        candidate.factory_work_item_id === first.factory_work_item_id &&
        ['authorized', 'running'].includes(candidate.status),
    ).length,
    0,
  )
  await psql(`
    UPDATE secrets
    SET revoked_at = NULL
    WHERE id = ${sqlLiteral(secret.secret_id)}::uuid;
  `)
  const recoveryReason =
    'Resume the same provider session only to create resumed.txt and satisfy the unchanged verification policy.'
  const recovery = await runController(
    demo,
    issue.number,
    statePath,
    policyPath,
    {
      adapter: 'codex',
      recovery: 'source-correction',
      recoveryReason,
    },
  )
  assert.ok(recovery.launch.contract_revision_id)
  const waiting = await waitFor(
    demo,
    (state) => {
      const task = state.tasks.find((item) => item.mission_id === first.mission_id)
      const runs = state.runs.filter((run) => run.task_id === task?.id)
      const run = runs.find(
        (item) =>
          item.id !== sourceRun.id && item.status === 'waiting_for_approval',
      )
      const item = state.factory_work_items.find(
        (candidate) => candidate.id === first.factory_work_item_id,
      )
      return run && item?.state === 'awaiting_approval'
        ? { task, runs, run, item }
        : null
    },
    'source-correction recovery review',
  )
  const recoveryRun = waiting.value.run
  assert.equal(recoveryRun.execution_mode, 'provider')
  assert.equal(recoveryRun.resumed_from_run_id, verifierOnlyRun.id)
  assert.equal(recoveryRun.workspace_run_id, sourceRun.workspace_run_id)
  assert.equal(recoveryRun.workspace_path, sourceRun.workspace_path)
  assert.equal(recoveryRun.provider_session_id, sourceRun.provider_session_id)
  assert.equal(waiting.value.task.contract_version, 3)
  assert.equal(waiting.value.task.attempt_count, 4)
  assert.equal(waiting.value.runs.length, runsBeforeRestart + 2)
  const recoveryEvidence = waiting.state.snapshot.verification_evidence.filter(
    (item) => item.run_id === recoveryRun.id,
  )
  assert.equal(recoveryEvidence.length, 2)
  assert.ok(recoveryEvidence.every((item) => item.status === 'passed'))
  await post(
    `/api/corps/${demo.corp_id}/runs/${recoveryRun.id}/verification-decision`,
    {
      actor_id: demo.bob_actor_id,
      approved: true,
      note: 'Independent reviewer accepts the same-lineage source correction.',
      decision_key: randomUUID(),
    },
  )
  const completed = await waitFor(
    demo,
    (state) => {
      const item = state.factory_work_items.find(
        (candidate) => candidate.id === first.factory_work_item_id,
      )
      const mission = state.missions.find(
        (candidate) => candidate.id === first.mission_id,
      )
      return item?.state === 'verified' && mission?.status === 'completed'
        ? { item, mission }
        : null
    },
    'verified source correction',
  )
  const staleCommandId = await psql(`
    WITH stale AS (
      UPDATE runner_commands
      SET status = 'pending', dispatched_at = NULL
      WHERE run_id = ${sqlLiteral(verifierOnlyRun.id)}::uuid
        AND command_kind = 'factory_verification_recovery'
      RETURNING id
    )
    SELECT id FROM stale;
  `)
  assert.match(staleCommandId, /^[0-9a-f-]{36}$/u)
  const retired = await waitFor(
    demo,
    (state) => state.events.find(
      (event) =>
        event.type === 'runner.command_failed' &&
        event.payload?.command_id === staleCommandId &&
        event.payload?.detail?.includes('no longer active'),
    ),
    'retirement of a stale terminal recovery command without repeated execution',
  )
  assert.equal(retired.state.snapshot.runs.length, completed.state.snapshot.runs.length)
  assert.equal(
    retired.state.snapshot.factory_work_items.find(
      (item) => item.id === first.factory_work_item_id,
    )?.state,
    'verified',
  )
  assert.equal(
    await psql(`
      SELECT status FROM runner_commands
      WHERE id = ${sqlLiteral(staleCommandId)}::uuid;
    `),
    'failed',
  )
  await psql(`
    UPDATE secrets
    SET revoked_at = now()
    WHERE id = ${sqlLiteral(secret.secret_id)}::uuid;
  `)
  return {
    factory_work_item_id: first.factory_work_item_id,
    mission_id: first.mission_id,
    source_run_id: sourceRun.id,
    verifier_only_bridge_run_id: verifierOnlyRun.id,
    other_mission_active_agent_fenced: true,
    mission_worker_reactivation_exercised: missionOwned,
    recovery_run_id: recoveryRun.id,
    contract_revision_id: recovery.launch.contract_revision_id,
    server_restart_exercised: restarted,
    unaudited_revision_rejected: true,
    weakened_policy_rejected_before_revision: true,
    secret_dispatch_failure_terminalized: true,
    failed_dispatch_run_id: failedDispatch.value.run.id,
    failed_dispatch_recovery_id: failedDispatch.value.recovery.id,
    retry_after_dispatch_failure: true,
    same_provider_session:
      recoveryRun.provider_session_id === sourceRun.provider_session_id,
    provider_session_inherited_across_verifier_only:
      verifierOnlyRun.provider_session_id === null &&
      recoveryRun.provider_session_id === sourceRun.provider_session_id,
    durable_artifact_transferred_without_source_copy: true,
    missing_file_check_not_manufactured: true,
    artifact_transfer_absent_from_durable_commands: true,
    terminal_recovery_command_retired_without_execution: true,
    same_workspace: recoveryRun.workspace_path === sourceRun.workspace_path,
    same_workspace_lineage:
      recoveryRun.workspace_run_id === sourceRun.workspace_run_id,
    contract_version: waiting.value.task.contract_version,
    attempt_count: waiting.value.task.attempt_count,
    final_factory_state: completed.value.item.state,
    run_count: waiting.value.runs.length,
  }
}

async function resumeSourceCorrectionAfterBridge() {
  const demo = await post('/api/demo/bootstrap', {})
  const state = (await snapshot(demo)).snapshot
  const items = state.factory_work_items.filter(
    (item) => item.source_issue_number === 9200 &&
      item.source_project_item_id === 'PVTI_FACTORY_SOURCE_9200',
  )
  assert.equal(items.length, 1, 'resume must identify the existing exact fixture item')
  const item = items[0]
  const task = state.tasks.find((candidate) => candidate.mission_id === item.mission_id)
  assert.equal(task?.attempt_count, 2)
  assert.equal(task.contract_version, 1)
  assert.equal(task.status, 'verification_failed')
  const runs = state.runs.filter((run) => run.task_id === task.id)
  assert.equal(runs.length, 2)
  const sourceRun = runs.find((run) => run.execution_mode === 'provider')
  const verifierOnlyRun = runs.find((run) => run.execution_mode === 'verification_only')
  assert.ok(sourceRun?.provider_session_id)
  assert.equal(verifierOnlyRun?.resumed_from_run_id, sourceRun.id)
  assert.equal(verifierOnlyRun.status, 'failed')
  assert.equal(verifierOnlyRun.provider_session_id, null)
  assert.equal(verifierOnlyRun.workspace_disposition, 'preserved')
  assert.equal(verifierOnlyRun.workspace_run_id, sourceRun.workspace_run_id)
  assert.equal(verifierOnlyRun.workspace_fingerprint, sourceRun.workspace_fingerprint)
  assert.equal(verifierOnlyRun.artifact_id, sourceRun.artifact_id)
  const checks = state.verification_evidence.filter((entry) => entry.run_id === verifierOnlyRun.id)
  assert.equal(checks.length, 2)
  assert.equal(checks.find((entry) => entry.check_index === 0)?.status, 'passed')
  assert.equal(checks.find((entry) => entry.check_index === 1)?.status, 'failed')
  const sourceAgent = state.agents.find((agent) => agent.id === sourceRun.agent_id)
  const missionOwned = sourceAgent?.mission_id === item.mission_id && !sourceAgent.pinned
  const statePath = path.join(output, 'factory-source-recovery-github.json')
  const policyPath = path.join(output, 'factory-source-recovery-policy.json')
  const weakenedPolicyPath = path.join(output, 'factory-source-recovery-weakened-policy.json')
  const github = JSON.parse(await readFile(statePath, 'utf8'))
  assert.equal(github.issues['9200'].number, 9200)
  assert.equal(github.issues['9200'].url, item.source_issue_url)
  const result = await finishSourceCorrectionRecovery({
    demo, issue: github.issues['9200'], statePath, policyPath, weakenedPolicyPath, github,
    first: { factory_work_item_id: item.id, mission_id: item.mission_id },
    failed: { value: { task } }, sourceRun, verifierOnlyRun, missionOwned,
  })
  return { ...result, continued_preserved_database_after_bridge: true }
}

async function cancelledRecoveryTerminalizes() {
  const demo = await post('/api/demo/bootstrap', {})
  const issue = {
    id: 'I_FACTORY_RECOVERY_9250',
    number: 9250,
    title: 'Terminalize an interrupted verifier recovery',
    body: [
      '## Outcome',
      '',
      'An interrupted verifier-only recovery returns to governed recovery state.',
      '',
      '## Acceptance criteria',
      '',
      '- [ ] Cancellation releases the active recovery slot.',
      '- [ ] A later recovery can be authorized from the preserved lineage.',
      '',
    ].join('\n'),
    url: 'https://github.com/shyamsridhar123/ecorp/issues/9250',
    state: 'OPEN',
    createdAt: '2026-09-04T01:30:00Z',
    updatedAt: '2026-09-04T01:30:00Z',
    labels: [{ name: 'factory:ready' }],
  }
  const statePath = path.join(output, 'factory-cancelled-recovery-github.json')
  const policyPath = path.join(output, 'factory-cancelled-recovery-policy.json')
  const policy = {
    checks: [
      { type: 'artifact', min_bytes: 1 },
      { type: 'file', path: 'result.md', min_bytes: 50 },
      {
        type: 'command',
        program: 'node',
        args: [
          '-e',
          "const fs=require('node:fs');if(fs.existsSync('hold-verification.flag'))setTimeout(()=>{},30000)",
        ],
        timeout_ms: 60_000,
      },
    ],
    manual_gate: {
      type: 'independent_review',
      roles: ['owner', 'admin', 'manager', 'member'],
      exclude_requester: true,
    },
  }
  await writeFile(
    statePath,
    `${JSON.stringify(
      projectState(
        issue,
        'PVTI_FACTORY_RECOVERY_9250',
        'PVT_FACTORY_RECOVERY_CANCEL',
        'PVTSSF_FACTORY_RECOVERY_CANCEL',
      ),
      null,
      2,
    )}\n`,
  )
  await writeFile(policyPath, `${JSON.stringify(policy, null, 2)}\n`)
  const first = await runController(demo, issue.number, statePath, policyPath, {
    adapter: 'fake-process',
  })
  const firstWaiting = await waitFor(
    demo,
    (state) => {
      const task = state.tasks.find((item) => item.mission_id === first.mission_id)
      const run = state.runs.find(
        (item) =>
          item.task_id === task?.id &&
          item.status === 'waiting_for_approval' &&
          item.workspace_fingerprint,
      )
      return run ? { task, run } : null
    },
    'cancellation source review',
  )
  const sourceRun = firstWaiting.value.run
  await post(
    `/api/corps/${demo.corp_id}/runs/${sourceRun.id}/verification-decision`,
    {
      actor_id: demo.bob_actor_id,
      approved: false,
      note: 'Exercise verifier-only cancellation and retry.',
      decision_key: randomUUID(),
    },
  )
  await waitFor(
    demo,
    (state) => {
      const item = state.factory_work_items.find(
        (candidate) => candidate.id === first.factory_work_item_id,
      )
      return item?.state === 'verification_failed' ? item : null
    },
    'cancellation source rejection',
  )
  await writeFile(
    path.join(sourceRun.workspace_path, 'hold-verification.flag'),
    'hold\n',
  )
  await psql(`
    UPDATE runs
    SET workspace_fingerprint = NULL
    WHERE id = ${sqlLiteral(sourceRun.id)}::uuid;
    UPDATE tasks
    SET max_attempts = 4
    WHERE id = ${sqlLiteral(firstWaiting.value.task.id)}::uuid;
  `)
  const firstReason =
    'Start a verifier-only recovery and interrupt it while an isolated check is active.'
  const firstRecovery = await runController(
    demo,
    issue.number,
    statePath,
    policyPath,
    {
      adapter: 'fake-process',
      recovery: 'verifier-only',
      recoveryReason: firstReason,
    },
  )
  const firstActive = await waitFor(
    demo,
    (state) => {
      const recovery = state.factory_verification_recoveries.find(
        (candidate) => candidate.id === firstRecovery.launch.recovery_id,
      )
      const run = state.runs.find(
        (candidate) => candidate.id === firstRecovery.launch.run_id,
      )
      return recovery?.status === 'running' && run?.status === 'verifying'
        ? { recovery, run }
        : null
    },
    'active verifier-only recovery before interrupt',
  )
  const firstLease = await post(
    `/api/corps/${demo.corp_id}/agents/${firstActive.value.run.agent_id}/lease`,
    { actor_id: demo.alice_actor_id },
  )
  assert.equal(firstLease.acquired, true)
  assert.ok(firstLease.token)
  await psql(`
    INSERT INTO verification_requests (
      run_id, corp_id, task_id, gate_type, gate, status
    )
    VALUES (
      ${sqlLiteral(firstActive.value.run.id)}::uuid,
      ${sqlLiteral(demo.corp_id)}::uuid,
      ${sqlLiteral(firstActive.value.run.task_id)}::uuid,
      'independent_review',
      '{"type":"independent_review","roles":["owner"],"exclude_requester":true}'::jsonb,
      'pending'
    )
    ON CONFLICT (run_id) DO NOTHING;
  `)
  await post(
    `/api/corps/${demo.corp_id}/agents/${firstActive.value.run.agent_id}/interrupt`,
    {
      actor_id: demo.alice_actor_id,
      lease_token: firstLease.token,
      reason: 'Cancel the verifier-only recovery without cancelling the factory lineage.',
    },
  )
  const firstCancelled = await waitFor(
    demo,
    (state) => {
      const recovery = state.factory_verification_recoveries.find(
        (candidate) => candidate.id === firstRecovery.launch.recovery_id,
      )
      const run = state.runs.find(
        (candidate) => candidate.id === firstRecovery.launch.run_id,
      )
      const task = state.tasks.find((candidate) => candidate.id === run?.task_id)
      const mission = state.missions.find(
        (candidate) => candidate.id === first.mission_id,
      )
      const item = state.factory_work_items.find(
        (candidate) => candidate.id === first.factory_work_item_id,
      )
      const verificationRequest = state.verification_requests.find(
        (candidate) => candidate.run_id === firstRecovery.launch.run_id,
      )
      return recovery?.status === 'failed' &&
        run?.status === 'cancelled' &&
        run.workspace_disposition === 'preserved' &&
        run.workspace_fingerprint &&
        task?.status === 'verification_failed' &&
        task.verification_status === 'failed' &&
        mission?.status === 'failed' &&
        item?.state === 'verification_failed' &&
        verificationRequest?.status === 'rejected'
        ? { recovery, run, task, mission, item, verificationRequest }
        : null
    },
    'terminalized cancelled recovery',
  )
  assert.equal(
    firstCancelled.state.snapshot.factory_verification_recoveries.filter(
      (recovery) =>
        recovery.factory_work_item_id === first.factory_work_item_id &&
        ['authorized', 'running'].includes(recovery.status),
    ).length,
    0,
  )
  const context = await requestOk(
    `/api/corps/${demo.corp_id}/factory/work-items/${first.factory_work_item_id}/verification-recoveries?actor_id=${demo.alice_actor_id}`,
  )
  assert.equal(context.source_run_id, firstCancelled.value.run.id)
  const secondReason =
    'Authorize a fresh verifier-only recovery after the interrupted attempt released its slot.'
  const secondRecovery = await runController(
    demo,
    issue.number,
    statePath,
    policyPath,
    {
      adapter: 'fake-process',
      recovery: 'verifier-only',
      recoveryReason: secondReason,
    },
  )
  assert.notEqual(
    secondRecovery.launch.recovery_id,
    firstRecovery.launch.recovery_id,
  )
  assert.notEqual(secondRecovery.launch.run_id, firstRecovery.launch.run_id)
  const secondActive = await waitFor(
    demo,
    (state) => {
      const recovery = state.factory_verification_recoveries.find(
        (candidate) => candidate.id === secondRecovery.launch.recovery_id,
      )
      const run = state.runs.find(
        (candidate) => candidate.id === secondRecovery.launch.run_id,
      )
      return recovery?.status === 'running' && run?.status === 'verifying'
        ? { recovery, run }
        : null
    },
    'fresh recovery after cancellation',
  )
  const secondLease = await post(
    `/api/corps/${demo.corp_id}/agents/${secondActive.value.run.agent_id}/lease`,
    { actor_id: demo.alice_actor_id },
  )
  assert.equal(secondLease.acquired, true)
  assert.ok(secondLease.token)
  await post(
    `/api/corps/${demo.corp_id}/agents/${secondActive.value.run.agent_id}/interrupt`,
    {
      actor_id: demo.alice_actor_id,
      lease_token: secondLease.token,
      reason: 'Clean up the second cancellation regression run.',
    },
  )
  await waitFor(
    demo,
    (state) => {
      const recovery = state.factory_verification_recoveries.find(
        (candidate) => candidate.id === secondRecovery.launch.recovery_id,
      )
      const run = state.runs.find(
        (candidate) => candidate.id === secondRecovery.launch.run_id,
      )
      return recovery?.status === 'failed' && run?.status === 'cancelled'
        ? { recovery, run }
        : null
    },
    'second cancelled recovery cleanup',
  )
  return {
    factory_work_item_id: first.factory_work_item_id,
    mission_id: first.mission_id,
    first_recovery_id: firstRecovery.launch.recovery_id,
    first_cancelled_run_id: firstRecovery.launch.run_id,
    second_recovery_id: secondRecovery.launch.recovery_id,
    second_cancelled_run_id: secondRecovery.launch.run_id,
    active_slot_released: true,
    stale_manual_request_closed: true,
    exact_context_selected_cancelled_source: true,
    fresh_recovery_authorized: true,
  }
}

async function exhaustedRecoveryIsRejected() {
  const demo = await post('/api/demo/bootstrap', {})
  const issue = {
    id: 'I_FACTORY_RECOVERY_9300',
    number: 9300,
    title: 'Keep verifier recovery attempts bounded',
    body: [
      '## Outcome',
      '',
      'Prove repeated verifier failure cannot bypass the task attempt ceiling.',
      '',
      '## Acceptance criteria',
      '',
      '- [ ] Two verifier-only generations retain the original evidence; a fourth run is never created.',
      '',
    ].join('\n'),
    url: 'https://github.com/shyamsridhar123/ecorp/issues/9300',
    state: 'OPEN',
    createdAt: '2026-09-04T02:00:00Z',
    updatedAt: '2026-09-04T02:00:00Z',
    labels: [{ name: 'factory:ready' }],
  }
  const statePath = path.join(output, 'factory-exhausted-recovery-github.json')
  const policyPath = path.join(output, 'factory-exhausted-recovery-policy.json')
  const policy = {
    checks: [
      { type: 'artifact', min_bytes: 1 },
      { type: 'file', path: 'never-created.txt', min_bytes: 1 },
    ],
    manual_gate: {
      type: 'independent_review',
      roles: ['owner', 'admin', 'manager', 'member'],
      exclude_requester: true,
    },
  }
  await writeFile(
    statePath,
    `${JSON.stringify(
      projectState(
        issue,
        'PVTI_FACTORY_RECOVERY_9300',
        'PVT_FACTORY_EXHAUSTED',
        'PVTSSF_FACTORY_EXHAUSTED',
      ),
      null,
      2,
    )}\n`,
  )
  await writeFile(policyPath, `${JSON.stringify(policy, null, 2)}\n`)
  const first = await runController(demo, issue.number, statePath, policyPath, {
    adapter: 'fake-process',
  })
  const firstFailed = await waitFor(
    demo,
    (state) => {
      const task = state.tasks.find((item) => item.mission_id === first.mission_id)
      const runs = state.runs.filter((run) => run.task_id === task?.id)
      const item = state.factory_work_items.find(
        (candidate) => candidate.id === first.factory_work_item_id,
      )
      return task?.status === 'verification_failed' &&
        item?.state === 'verification_failed' &&
        runs.length === 1 &&
        runs[0]?.workspace_fingerprint
        ? { task, runs, item }
        : null
    },
    'first bounded verifier failure',
  )
  await psql(`
    UPDATE tasks SET max_attempts = 3
    WHERE id = ${sqlLiteral(firstFailed.value.task.id)}::uuid;
  `)
  const firstRecoveryReason =
    'Re-run the unchanged failing verifier once to prove recovery generations remain bounded.'
  await runController(demo, issue.number, statePath, policyPath, {
    adapter: 'fake-process',
    recovery: 'verifier-only',
    recoveryReason: firstRecoveryReason,
  })
  const secondFailed = await waitFor(
    demo,
    (state) => {
      const task = state.tasks.find((item) => item.id === firstFailed.value.task.id)
      const runs = state.runs.filter((run) => run.task_id === task?.id)
      const item = state.factory_work_items.find(
        (candidate) => candidate.id === first.factory_work_item_id,
      )
      return task?.status === 'verification_failed' &&
        item?.state === 'verification_failed' &&
        runs.length === 2 &&
        runs.every((run) => run.workspace_fingerprint)
        ? { task, runs, item }
        : null
    },
    'second bounded verifier failure',
  )
  assert.equal(secondFailed.value.task.attempt_count, 2)
  assert.equal(secondFailed.value.task.max_attempts, 3)
  await runController(demo, issue.number, statePath, policyPath, {
    adapter: 'fake-process',
    recovery: 'verifier-only',
    recoveryReason: 'A second verifier-only generation must verify the original inherited artifact.',
  })
  const thirdFailed = await waitFor(
    demo,
    (state) => {
      const task = state.tasks.find((item) => item.id === firstFailed.value.task.id)
      const runs = state.runs.filter((run) => run.task_id === task?.id)
      return task?.status === 'verification_failed' && runs.length === 3 &&
        runs.every((run) => run.workspace_fingerprint)
        ? { task, runs }
        : null
    },
    'third bounded run with second-generation inherited artifact',
  )
  const runIds = new Set(thirdFailed.value.runs.map((run) => run.id))
  const artifactChecks = thirdFailed.state.snapshot.verification_evidence.filter(
    (entry) => runIds.has(entry.run_id) && entry.check_index === 0,
  )
  assert.equal(artifactChecks.length, 3)
  assert.ok(artifactChecks.every((entry) => entry.status === 'passed'))
  const exhausted = await controllerFailure(
    demo,
    issue.number,
    statePath,
    policyPath,
    {
      adapter: 'fake-process',
      recovery: 'verifier-only',
      recoveryReason:
        'A fourth attempt must be rejected because the task attempt ceiling is exhausted.',
    },
  )
  assert.match(exhausted, /attempt limit|exhausted/i)
  const final = await snapshot(demo)
  const finalRuns = final.snapshot.runs.filter(
    (run) => run.task_id === secondFailed.value.task.id,
  )
  assert.equal(finalRuns.length, 3)
  return {
    factory_work_item_id: first.factory_work_item_id,
    mission_id: first.mission_id,
    attempt_count: thirdFailed.value.task.attempt_count,
    max_attempts: thirdFailed.value.task.max_attempts,
    second_generation_inherited_artifact_verified: true,
    rejected_fourth_run: true,
    run_count: finalRuns.length,
  }
}

async function upstreamThenLaterTaskRecovery() {
  const demo = await post('/api/demo/bootstrap', {})
  const before = await snapshot(demo)
  assert.ok(!before.snapshot.factory_work_items.some((item) =>
    item.source_project_item_id === 'PVTI_FACTORY_STORE_RECOVERY_9400',
  ), 'Graph recovery requires a fresh fixture')
  // The existing planner needs two workers. Seed only new deterministic identities;
  // keep every existing adapter/agent unchanged and use the native graph planner.
  for (let index = 0; index < 2; index += 1) {
    const actorId = randomUUID()
    const agentId = randomUUID()
    await psql(`
      INSERT INTO actors
      SELECT (jsonb_populate_record(NULL::actors, to_jsonb(actor) ||
        jsonb_build_object('id', ${sqlLiteral(actorId)},
                           'name', ${sqlLiteral(`Store recovery fixture actor ${index}`)},
                           'created_at', now()))).*
      FROM actors actor JOIN agents agent ON agent.actor_id = actor.id
      WHERE agent.id = ${sqlLiteral(demo.worker_agent_id)}::uuid;
      INSERT INTO agents
      SELECT (jsonb_populate_record(NULL::agents, to_jsonb(agent) ||
        jsonb_build_object('id', ${sqlLiteral(agentId)},
                           'actor_id', ${sqlLiteral(actorId)},
                           'name', ${sqlLiteral(`000 Store recovery specialist ${index}`)},
                           'adapter', 'fake-process', 'status', 'idle',
                           'current_run_id', NULL, 'station', NULL,
                           'mission_id', NULL, 'retired_at', NULL,
                           'created_at', now()))).*
      FROM agents agent WHERE id = ${sqlLiteral(demo.worker_agent_id)}::uuid;
    `)
  }
  const fixture = await heldRecoveryFixture(demo, 9400, 'parallel-specialists', {
    checks: [{ type: 'artifact', min_bytes: 1 }],
    manual_gate: recoveryManualGate,
  })
  assert.equal(fixture.tasks.length, 3)
  const upstream = fixture.tasks.find((task) => task.plan_key === 'specialist-a')
  const peer = fixture.tasks.find((task) => task.plan_key === 'specialist-b')
  const later = fixture.tasks.find((task) => task.plan_key === 'synthesis')
  assert.ok(upstream && peer && later)
  await post(`/api/corps/${demo.corp_id}/missions/${fixture.missionId}/launch`, {
    requested_by: demo.alice_actor_id,
  })
  const firstUpstream = await waitForReviewRun(demo, upstream.id)
  const firstPeer = await waitForReviewRun(demo, peer.id)
  await decideFixtureRun(demo, firstPeer.value.id, true, 'Accept the independent peer task.')
  await decideFixtureRun(demo, firstUpstream.value.id, false, 'Recheck the upstream task in place.')
  const recoveredUpstream = await authorizeFixtureRecovery(fixture, 'Authorize the same upstream checkpoint.')
  assert.equal(recoveredUpstream.context.task_id, upstream.id)
  const upstreamReview = await waitForReviewRun(demo, upstream.id, recoveredUpstream.result.run_id)
  await decideFixtureRun(demo, upstreamReview.value.id, true, 'Accept the recovered upstream task.')
  const upstreamSettled = await waitFor(demo, (state) => {
    const recovery = state.factory_verification_recoveries.find((entry) => entry.id === recoveredUpstream.result.recovery.id)
    const task = state.tasks.find((entry) => entry.id === later.id)
    const mission = state.missions.find((entry) => entry.id === fixture.missionId)
    return recovery?.status === 'completed' && task?.status !== 'completed' &&
      mission?.status === 'running' ? recovery : null
  }, 'upstream recovery settled before its mission completes')
  assert.equal(upstreamSettled.state.snapshot.factory_verification_recoveries.filter((entry) =>
    entry.factory_work_item_id === fixture.workItemId && ['authorized', 'running'].includes(entry.status),
  ).length, 0)
  const firstLater = await waitForReviewRun(demo, later.id)
  await decideFixtureRun(demo, firstLater.value.id, false, 'Recheck the later task without replacing the graph.')
  const recoveredLater = await authorizeFixtureRecovery(fixture, 'Authorize the same later-task checkpoint.')
  assert.equal(recoveredLater.context.task_id, later.id)
  assert.notEqual(recoveredLater.result.recovery.id, recoveredUpstream.result.recovery.id)
  const laterReview = await waitForReviewRun(demo, later.id, recoveredLater.result.run_id)
  await decideFixtureRun(demo, laterReview.value.id, true, 'Accept the recovered final task.')
  const finished = await waitFor(demo, (state) => {
    const item = state.factory_work_items.find((entry) => entry.id === fixture.workItemId)
    const recoveries = state.factory_verification_recoveries.filter((entry) =>
      entry.factory_work_item_id === fixture.workItemId,
    )
    return item?.state === 'verified' && recoveries.length === 2 &&
      recoveries.every((entry) => entry.status === 'completed') ? recoveries : null
  }, 'both task recoveries completed in the original graph')
  const runs = finished.state.snapshot.runs.filter((run) => fixture.tasks.some((task) => task.id === run.task_id))
  assert.equal(runs.length, 5)
  assert.equal(finished.state.snapshot.tasks.filter((task) => task.mission_id === fixture.missionId).length, 3)
  return {
    factory_work_item_id: fixture.workItemId,
    mission_id: fixture.missionId,
    upstream_recovery_id: recoveredUpstream.result.recovery.id,
    later_recovery_id: recoveredLater.result.recovery.id,
    upstream_settled_before_mission: true,
    later_recovery_admitted: true,
    original_graph_task_count: 3,
    run_count: runs.length,
  }
}

async function gateFreeRecoverySettles() {
  const demo = await post('/api/demo/bootstrap', {})
  const allowFile = path.join(output, 'gate-free-verifier-allowed.flag')
  assert.ok(!existsSync(allowFile), 'Gate-free verifier input must be fresh')
  const fixture = await heldRecoveryFixture(demo, 9450, 'single', {
    checks: [
      { type: 'artifact', min_bytes: 1 },
      {
        type: 'command',
        program: 'node',
        args: ['-e', `process.exit(require('node:fs').existsSync(${JSON.stringify(allowFile)})?0:1)`],
        timeout_ms: 30_000,
      },
    ],
    manual_gate: null,
  })
  await post(`/api/corps/${demo.corp_id}/missions/${fixture.missionId}/launch`, {
    requested_by: demo.alice_actor_id,
  })
  const failed = await waitFor(demo, (state) => state.runs.find((run) =>
    run.task_id === fixture.tasks[0].id && run.verification_status === 'failed' &&
    run.workspace_disposition === 'preserved' && run.workspace_fingerprint,
  ), 'gate-free source failure with retained checkpoint')
  // Change only the external verifier fixture input, never the retained source.
  await writeFile(allowFile, 'allowed\n', { flag: 'wx' })
  const recovered = await authorizeFixtureRecovery(fixture, 'Retry the same source after the verifier fixture becomes available.')
  const finished = await waitFor(demo, (state) => {
    const recovery = state.factory_verification_recoveries.find((entry) => entry.id === recovered.result.recovery.id)
    const run = state.runs.find((entry) => entry.id === recovered.result.run_id)
    const item = state.factory_work_items.find((entry) => entry.id === fixture.workItemId)
    return recovery?.status === 'completed' && run?.status === 'completed' &&
      run.workspace_fingerprint && item?.state === 'verified' ? { recovery, run } : null
  }, 'gate-free recovery and factory completion')
  assert.equal(finished.value.run.workspace_run_id, failed.value.workspace_run_id)
  assert.equal(finished.value.run.workspace_fingerprint, failed.value.workspace_fingerprint)
  assert.equal(finished.value.run.provider_session_id, null)
  assert.equal(finished.state.snapshot.verification_requests.filter((entry) =>
    entry.run_id === recovered.result.run_id,
  ).length, 0)
  return {
    factory_work_item_id: fixture.workItemId,
    mission_id: fixture.missionId,
    recovery_id: recovered.result.recovery.id,
    run_id: recovered.result.run_id,
    manual_decisions: 0,
    same_source_fingerprint: true,
    recovery_status: finished.value.recovery.status,
  }
}

async function acknowledgedVerifierLossRecovery(reviseAfterLoss = false) {
  const demo = await post('/api/demo/bootstrap', {})
  const number = reviseAfterLoss ? 9501 : 9500
  const adapter = reviseAfterLoss ? 'codex' : 'fake-process'
  const holdFile = path.join(output, `lost-verifier-${number}-hold.flag`)
  assert.ok(!existsSync(holdFile), 'Runner-loss verifier input must be fresh')
  const policy = {
    checks: [
      { type: 'artifact', min_bytes: 1 },
      { type: 'file', path: reviseAfterLoss ? 'base.txt' : 'result.md', min_bytes: reviseAfterLoss ? 5 : 50 },
      {
        type: 'command',
        program: 'node',
        args: ['-e', `if(require('node:fs').existsSync(${JSON.stringify(holdFile)}))setTimeout(()=>{},55000)`],
        timeout_ms: 60_000,
      },
    ],
    manual_gate: recoveryManualGate,
  }
  const fixture = await heldRecoveryFixture(demo, number, 'single', policy, 3, adapter)
  await post(`/api/corps/${demo.corp_id}/missions/${fixture.missionId}/launch`, {
    requested_by: demo.alice_actor_id,
  })
  const original = await waitForReviewRun(demo, fixture.tasks[0].id)
  const sourceRun = original.value
  if (reviseAfterLoss) {
    // Same scripts/fake-codex-app-server.mjs protocol fixture as the existing
    // source-correction lane; its native thread/resume writes resumed.txt.
    assert.ok(sourceRun.provider_session_id)
    assert.equal(await readFile(path.join(sourceRun.workspace_path, 'base.txt'), 'utf8'), 'base\n')
  }
  await decideFixtureRun(demo, sourceRun.id, false, 'Exercise loss of one acknowledged verifier-only attempt.')
  await writeFile(holdFile, 'hold\n', { flag: 'wx' })
  const first = await authorizeFixtureRecovery(fixture, 'Authorize the bounded verifier attempt that will lose its connection.')
  assert.match(first.context.expected_head_commit, /^(?:[a-f0-9]{40}|[a-f0-9]{64})$/u)
  const active = await waitFor(demo, (state) => state.runs.find((run) =>
    run.id === first.result.run_id && run.status === 'verifying',
  ), 'verifier-only run before transport loss')
  const commandQuery = `
    SELECT status FROM runner_commands
    WHERE corp_id = ${sqlLiteral(demo.corp_id)}::uuid
      AND run_id = ${sqlLiteral(first.result.run_id)}::uuid
      AND command_kind = 'factory_verification_recovery';
  `
  const ackDeadline = Date.now() + 15_000
  while ((await psql(commandQuery)) !== 'dispatched') {
    assert.ok(Date.now() < ackDeadline, 'Recovery command was not acknowledged before the loss test')
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  const beforeDisconnect = await snapshot(demo)
  const activeStates = ['provisioning', 'starting', 'running', 'waiting_for_input', 'waiting_for_approval', 'verifying']
  assert.deepEqual(beforeDisconnect.snapshot.runs.filter((run) =>
    run.runner_id === active.value.runner_id && activeStates.includes(run.status),
  ).map((run) => run.id), [active.value.id], 'Never disconnect a runner carrying unrelated work')
  assert.equal(await psql(`
    SELECT id FROM runs
    WHERE runner_id = ${sqlLiteral(active.value.runner_id)}
      AND status IN ('provisioning', 'starting', 'running', 'waiting_for_input',
                     'waiting_for_approval', 'verifying')
    ORDER BY id;
  `), active.value.id, 'Room-filtered snapshots must not hide unrelated active runner work')
  const epochQuery = `
    SELECT connection_epoch FROM runner_nodes
    WHERE id = ${sqlLiteral(active.value.runner_id)}
      AND corp_id = ${sqlLiteral(demo.corp_id)}::uuid;
  `
  const priorEpoch = await psql(epochQuery)
  // Reuse the native disconnect primitive. Leave a bounded interval after the
  // five-second QA grace expires to test admission before any cleanup arrives.
  const disconnectedAt = new Date().toISOString()
  await post(`/api/demo/runners/${encodeURIComponent(active.value.runner_id)}/disconnect`, {
    reconnect_delay_ms: 30_000,
  })
  const lost = await waitFor(demo, (state) => {
    const run = state.runs.find((entry) => entry.id === first.result.run_id)
    const recovery = state.factory_verification_recoveries.find((entry) => entry.id === first.result.recovery.id)
    const task = state.tasks.find((entry) => entry.id === run?.task_id)
    const item = state.factory_work_items.find((entry) => entry.id === fixture.workItemId)
    return run?.status === 'lost' && recovery?.status === 'failed' &&
      task?.status === 'verification_failed' && item?.state === 'verification_failed'
      ? { run, recovery, task, item } : null
  }, 'acknowledged verifier-only loss terminalized after grace', 15_000)
  assert.equal(lost.value.run.workspace_fingerprint, null, 'Loss must not mint a trusted fingerprint')
  assert.notEqual(lost.value.run.workspace_disposition, 'preserved', 'Loss alone cannot prove workspace cleanup')
  assert.equal(await psql(commandQuery), 'dispatched')
  const revisionRoute = `/api/corps/${demo.corp_id}/missions/${fixture.missionId}/contract-revisions`
  const revisionBody = (sourceId, overrides = {}) => ({
    actor_id: demo.alice_actor_id,
    task_id: sourceRun.task_id,
    expected_contract_version: lost.value.task.contract_version,
    next_action: 'resume',
    source_run_id: sourceId,
    reason: 'Review the same latest preserved checkpoint after verifier loss.',
    idempotency_key: randomUUID(),
    description: fixture.issue.body,
    contract: lost.value.task.contract,
    verification_policy: policy,
    ...overrides,
  })
  const rejection = async (route, body, pattern) => {
    const response = await request(route, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
    })
    assert.equal(response.response.status, 400)
    assert.match(response.body.error, pattern)
  }
  const unconfirmedContext = await request(`${first.route}?actor_id=${demo.alice_actor_id}`)
  assert.equal(unconfirmedContext.response.status, 400)
  assert.match(unconfirmedContext.body.error, /preserved|confirmed/u)
  for (const [sourceId, pattern] of [
    [lost.value.run.id, /preserved|confirmed/u],
    [sourceRun.id, /not the latest/u],
  ]) {
    await rejection(first.route, {
      ...first.body,
      expected_factory_version: lost.value.item.version,
      idempotency_key: randomUUID(),
      source_run_id: sourceId,
    }, pattern)
  }
  if (reviseAfterLoss) {
    await rejection(revisionRoute, revisionBody(lost.value.run.id), /preserved|confirmed/u)
    await rejection(revisionRoute, revisionBody(sourceRun.id), /not the latest/u)
  }
  // The native reconnect sends StopRun for the stale claim. Wait for its bounded
  // cleanup before requesting another run; do not pretend a DB terminal state
  // alone proves the old verifier has stopped.
  const cleanup = await waitFor(demo, (state) => state.runs.find((run) =>
    run.id === first.result.run_id && run.status === 'lost' &&
    run.workspace_disposition === 'preserved' && run.workspace_fingerprint,
  ), 'native stale-claim stop and workspace cleanup', 45_000)
  assert.equal(cleanup.value.workspace_fingerprint, sourceRun.workspace_fingerprint)
  const reconnectedEpoch = await psql(epochQuery)
  assert.notEqual(reconnectedEpoch, priorEpoch)
  const beforeReplay = await snapshot(demo)
  assert.equal(beforeReplay.snapshot.tasks.find((task) => task.id === sourceRun.task_id).contract_version,
    lost.value.task.contract_version, 'Rejected revisions must leave the contract unchanged')
  assert.equal(beforeReplay.snapshot.runs.filter((run) => run.task_id === sourceRun.task_id).length, 2)
  const confirmedContext = await requestOk(`${first.route}?actor_id=${demo.alice_actor_id}`)
  assert.equal(confirmedContext.source_run_id, lost.value.run.id)
  assert.equal(confirmedContext.workspace_fingerprint, sourceRun.workspace_fingerprint)
  assert.equal(confirmedContext.expected_head_commit, first.context.expected_head_commit)
  const currentRequest = {
    ...first.body,
    expected_factory_version: confirmedContext.work_item.version,
    source_run_id: lost.value.run.id,
  }
  await rejection(first.route, {
    ...currentRequest, idempotency_key: randomUUID(), source_run_id: sourceRun.id,
  }, /not the latest/u)
  const wrongFingerprint = `${sourceRun.workspace_fingerprint[0] === '0' ? '1' : '0'}${sourceRun.workspace_fingerprint.slice(1)}`
  await rejection(first.route, {
    ...currentRequest, idempotency_key: randomUUID(), expected_workspace_fingerprint: wrongFingerprint,
  }, /workspace fingerprint mismatch/u)
  await rejection(first.route, {
    ...currentRequest, idempotency_key: randomUUID(), expected_head_commit: null,
  }, /head commit mismatch/u)
  const replay = await post(first.route, first.body)
  assert.equal(replay.replayed, true)
  assert.equal(replay.run_id, first.result.run_id)
  assert.equal(replay.recovery.status, 'failed')
  const afterReplay = await snapshot(demo)
  assert.deepEqual(
    afterReplay.snapshot.runs.map((run) => run.id).sort(),
    beforeReplay.snapshot.runs.map((run) => run.id).sort(),
  )
  assert.equal(await psql(commandQuery), 'dispatched')
  await rm(holdFile)
  let revision = null
  let replacementPolicy = policy
  if (reviseAfterLoss) {
    await rejection(revisionRoute, revisionBody(sourceRun.id), /not the latest/u)
    const staleContract = { ...lost.value.task.contract, source_base_commit: '0'.repeat(sourceBaseCommit.length) }
    await rejection(revisionRoute, revisionBody(lost.value.run.id, { contract: staleContract }),
      /source|authority|cannot change/u)
    const correction = 'Create resumed.txt in the same preserved workspace using the existing provider session.'
    const description = `${fixture.issue.body}\n\nReviewed correction: ${correction}`
    replacementPolicy = {
      ...policy,
      checks: [
        ...policy.checks.slice(0, 2),
        {
          type: 'command',
          program: 'node',
          args: ['-e', "const fs=require('node:fs');process.exit(fs.existsSync('resumed.txt')&&fs.readFileSync('resumed.txt','utf8')==='resumed\\n'?0:1)"],
          timeout_ms: 60_000,
        },
      ],
    }
    const body = revisionBody(lost.value.run.id, {
      description,
      contract: { ...lost.value.task.contract, objective: correction },
      verification_policy: replacementPolicy,
    })
    revision = await post(revisionRoute, body)
    const revisionReplay = await post(revisionRoute, body)
    assert.equal(revisionReplay.replayed, true)
    assert.equal(revisionReplay.revision.id, revision.revision.id)
    assert.equal(revision.revision.source_run_id, lost.value.run.id)
    assert.equal(revision.revision.revised_by, demo.alice_actor_id)
    fixture.issue.body = description
    fixture.issue.updatedAt = '2026-09-04T02:30:00Z'
  }
  const second = await authorizeFixtureRecovery(
    fixture,
    reviseAfterLoss
      ? 'Authorize the reviewed source correction and replacement policy against the latest preserved lost run.'
      : 'Authorize an unchanged-policy recheck against the latest preserved lost run.',
    {
      mode: reviseAfterLoss ? 'source_correction' : 'verifier_only',
      contractRevisionId: revision?.revision.id ?? null,
    },
  )
  assert.equal(second.context.source_run_id, lost.value.run.id)
  assert.equal(second.context.workspace_fingerprint, sourceRun.workspace_fingerprint)
  assert.equal(second.context.expected_head_commit, first.context.expected_head_commit)
  assert.notEqual(second.result.run_id, first.result.run_id)
  const secondReview = await waitForReviewRun(demo, sourceRun.task_id, second.result.run_id)
  assert.equal(secondReview.value.workspace_run_id, sourceRun.workspace_run_id)
  assert.equal(secondReview.value.resumed_from_run_id, lost.value.run.id)
  assert.equal(secondReview.value.workspace_path, sourceRun.workspace_path)
  assert.equal(secondReview.value.workspace_branch, sourceRun.workspace_branch)
  assert.equal(secondReview.value.source_base_commit, sourceRun.source_base_commit)
  assert.deepEqual(second.result.recovery.replacement_verification_policy, replacementPolicy)
  if (reviseAfterLoss) {
    assert.equal(secondReview.value.execution_mode, 'provider')
    assert.equal(secondReview.value.provider_session_id, sourceRun.provider_session_id)
    assert.equal(second.result.recovery.contract_revision_id, revision.revision.id)
    assert.equal(second.result.recovery.observed_source_revision, fixture.issue.updatedAt)
    assert.equal(await readFile(path.join(sourceRun.workspace_path, 'resumed.txt'), 'utf8'), 'resumed\n')
    assert.match(await readFile(path.join(sourceRun.workspace_path, 'resume-prompt.txt'), 'utf8'), /Reviewed correction/u)
  } else {
    assert.equal(secondReview.value.workspace_fingerprint, sourceRun.workspace_fingerprint)
    assert.equal(secondReview.value.artifact_id, sourceRun.artifact_id)
    assert.equal(secondReview.value.provider_session_id, null)
    assert.equal(secondReview.state.snapshot.source_deliverables.find((entry) =>
      entry.run_id === second.result.run_id,
    )?.head_commit, first.context.expected_head_commit)
    assert.equal(secondReview.state.snapshot.events.filter((event) =>
      event.aggregate_id === second.result.run_id &&
      ['run.session', 'run.session_terminated', 'run.output', 'run.artifact'].includes(event.type),
    ).length, 0)
  }
  await decideFixtureRun(demo, secondReview.value.id, true, 'Accept the fresh explicitly authorized recheck.')
  const completed = await waitFor(demo, (state) => {
    const recovery = state.factory_verification_recoveries.find((entry) => entry.id === second.result.recovery.id)
    const task = state.tasks.find((entry) => entry.id === sourceRun.task_id)
    return recovery?.status === 'completed' && task?.status === 'completed' ? task : null
  }, 'loss recovery retry completion')
  assert.equal(completed.value.attempt_count, 3)
  assert.equal(completed.state.snapshot.runs.filter((run) => run.task_id === sourceRun.task_id).length, 3)
  assert.equal(completed.state.snapshot.factory_verification_recoveries.filter((entry) =>
    entry.factory_work_item_id === fixture.workItemId && ['authorized', 'running'].includes(entry.status),
  ).length, 0)
  return {
    factory_work_item_id: fixture.workItemId,
    mission_id: fixture.missionId,
    runner_id: sourceRun.runner_id,
    prior_epoch: priorEpoch,
    reconnected_epoch: reconnectedEpoch,
    source_run_id: sourceRun.id,
    lost_run_id: first.result.run_id,
    retry_run_id: second.result.run_id,
    disconnected_at: disconnectedAt,
    acknowledged_before_loss: true,
    loss_did_not_mint_trust: true,
    mismatched_checkpoint_rejected: true,
    consumed_command_replay_created_runs: 0,
    unconfirmed_context_and_admissions_rejected: true,
    stale_ancestor_rejected: true,
    missing_head_rejected: true,
    latest_preserved_run_reauthorized: true,
    recovery_source_run_id: second.context.source_run_id,
    contract_revision_id: revision?.revision.id ?? null,
    revised_policy_source_correction: reviseAfterLoss,
    native_provider_session_retained: reviseAfterLoss ? secondReview.value.provider_session_id === sourceRun.provider_session_id : null,
    attempt_count: completed.value.attempt_count,
  }
}

const reportPath = path.join(output, 'e2e-factory-verification-recovery.json')
const resumeAfterBridge = process.argv.includes('--resume-after-bridge')
if (!resumeAfterBridge && existsSync(reportPath)) {
  throw new Error('Recovery evidence already exists; use a fresh owned QA output, never reset/replay a completed fixture.')
}
let report = { passed: false, started_at: new Date().toISOString() }
if (resumeAfterBridge) {
  const previous = JSON.parse(await readFile(reportPath, 'utf8'))
  assert.equal(previous.passed, false)
  assert.equal(previous.failure?.case, 'source_correction')
  assert.equal(previous.verifier_only?.final_factory_state, 'verified')
  await writeFile(
    path.join(output, `recovery-before-resume-${Date.now()}.json`),
    `${JSON.stringify(previous, null, 2)}\n`,
  )
  report = { ...previous, resumed_at: new Date().toISOString() }
  delete report.failure
}
for (const [name, exercise] of [
  ...(!resumeAfterBridge ? [['verifier_only', verifierOnlyRecovery]] : []),
  ['source_correction', resumeAfterBridge ? resumeSourceCorrectionAfterBridge : sourceCorrectionRecovery],
  ['cancelled_recovery', cancelledRecoveryTerminalizes],
  ['exhausted_attempts', exhaustedRecoveryIsRejected],
  ['upstream_then_later_recovery', upstreamThenLaterTaskRecovery],
  ['gate_free_recovery', gateFreeRecoverySettles],
  ['acknowledged_verifier_loss', acknowledgedVerifierLossRecovery],
  ['lost_verifier_revision', () => acknowledgedVerifierLossRecovery(true)],
]) {
  try {
    report[name] = await exercise()
  } catch (error) {
    report.failure = { case: name, message: error.message }
    await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`)
    throw error
  }
  await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`)
}
report.passed = true
report.completed_at = new Date().toISOString()
await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`)
console.log(JSON.stringify(report, null, 2))
