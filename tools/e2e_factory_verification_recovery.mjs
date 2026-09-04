import assert from 'node:assert/strict'
import { execFile as execFileCallback, spawn } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import {
  existsSync,
  openSync,
  readFileSync,
  writeFileSync,
} from 'node:fs'
import { readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { promisify } from 'node:util'

const execFile = promisify(execFileCallback)
const root = path.resolve(import.meta.dirname, '..')
const output = path.join(root, 'output')
const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const databaseUrl =
  process.env.DATABASE_URL ?? 'postgres://crony:crony@127.0.0.1:54329/crony'
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
  await execFile('git', ['rev-parse', 'HEAD'], { cwd: root, windowsHide: true })
).stdout.trim()
let psqlMode

async function request(url, init) {
  const response = await fetch(`${server}${url}`, init)
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
    const script = [
      'import sys, psycopg',
      'dsn, sql = sys.argv[1], sys.argv[2]',
      'with psycopg.connect(dsn, autocommit=True) as connection:',
      '  with connection.cursor() as cursor:',
      "    statements = [item.strip() for item in sql.split(';') if item.strip()]",
      '    for statement in statements:',
      '      cursor.execute(statement)',
      '    if cursor.description:',
      '      for row in cursor.fetchall():',
      "        print('|'.join('t' if value is True else 'f' if value is False else '' if value is None else str(value) for value in row))",
    ].join('\n')
    const { stdout } = await execFile(
      process.env.ECORP_TEST_PYTHON ?? 'python',
      ['-c', script, databaseUrl, sql],
      {
        cwd: root,
        windowsHide: true,
        maxBuffer: 8 * 1024 * 1024,
      },
    )
    return stdout.trim()
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

async function psqlInvocation() {
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
    root,
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
  const pidPath = process.env.CRONY_TEST_SERVER_PID_FILE
  if (!pidPath) return false
  if (!existsSync(pidPath)) {
    throw new Error(`test-owned server PID file does not exist: ${pidPath}`)
  }
  const pidState = JSON.parse(readFileSync(pidPath, 'utf8'))
  const serverPid = Number(pidState.server)
  if (!Number.isSafeInteger(serverPid) || serverPid <= 0) {
    throw new Error(`test-owned server PID is invalid: ${serverPid}`)
  }
  process.kill(serverPid, 0)
  process.kill(serverPid)
  await new Promise((resolve) => setTimeout(resolve, 500))
  const serverUrl = new URL(server)
  const serverBinary =
    process.env.CRONY_TEST_SERVER_BINARY ??
    path.join(
      root,
      'target',
      'debug',
      process.platform === 'win32' ? 'crony-server.exe' : 'crony-server',
    )
  const logDirectory = path.dirname(path.resolve(pidPath))
  const stdout = openSync(
    path.join(logDirectory, 'verification-recovery-restart.stdout.log'),
    'a',
  )
  const stderr = openSync(
    path.join(logDirectory, 'verification-recovery-restart.stderr.log'),
    'a',
  )
  const child = spawn(
    serverBinary,
    [
      '--bind',
      `${serverUrl.hostname}:${serverUrl.port}`,
      '--database-url',
      databaseUrl,
    ],
    {
      cwd: root,
      detached: true,
      windowsHide: true,
      stdio: ['ignore', stdout, stderr],
    },
  )
  writeFileSync(
    pidPath,
    `${JSON.stringify({ ...pidState, server: child.pid }, null, 2)}\n`,
  )
  child.unref()
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    try {
      const health = await fetch(`${server}/health`).then((response) =>
        response.json(),
      )
      if (health.status === 'ok' && health.runners >= 1) return true
    } catch {
      // The isolated test server is restarting.
    }
    await new Promise((resolve) => setTimeout(resolve, 200))
  }
  throw new Error('test-owned server or runner did not recover after restart')
}

async function verifierOnlyRecovery() {
  const demo = await post('/api/demo/reset', {})
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
  const rejected = await post(
    `/api/corps/${demo.corp_id}/runs/${sourceRun.id}/verification-decision`,
    rejection,
  )
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

    UPDATE artifacts
    SET metadata = metadata - 'workspace_relative_path'
    WHERE id = ${sqlLiteral(sourceRun.artifact_id)}::uuid;
  `)
  const legacySnapshot = await snapshot(demo)
  const legacySourceRun = legacySnapshot.snapshot.runs.find(
    (run) => run.id === sourceRun.id,
  )
  assert.equal(legacySourceRun.workspace_fingerprint, null)
  assert.equal(
    await psql(`
      SELECT metadata ? 'workspace_relative_path'
      FROM artifacts
      WHERE id = ${sqlLiteral(sourceRun.artifact_id)}::uuid
    `),
    'f',
  )
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
  assert.equal(recoveryEvidence.length, 2)
  assert.ok(recoveryEvidence.every((item) => item.status === 'passed'))
  const recoveryDeliverable =
    recoveredWaiting.state.snapshot.source_deliverables.find(
      (deliverable) => deliverable.run_id === recoveryRun.id,
    )
  assert.equal(recoveryDeliverable?.head_commit, sourceDeliverable.head_commit)
  await post(
    `/api/corps/${demo.corp_id}/runs/${recoveryRun.id}/verification-decision`,
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
    controller_replay: replay.launch.recovered,
    same_workspace_lineage:
      recoveryRun.workspace_run_id === sourceRun.workspace_run_id,
    same_head_commit:
      recoveryDeliverable.head_commit === sourceDeliverable.head_commit,
    legacy_workspace_checkpointed:
      recovered.launch.legacy_workspace_checkpointed,
    legacy_provider_artifact_metadata_recovered: true,
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
  const demo = await post('/api/demo/reset', {})
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
  github.issues['9200'].body +=
    '\n## Reviewed correction\n\nCreate only resumed.txt in the preserved workspace.\n'
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
  assert.match(unaudited, /explicit --verification-recovery|source revision/i)
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
  assert.equal(recoveryRun.resumed_from_run_id, sourceRun.id)
  assert.equal(recoveryRun.workspace_run_id, sourceRun.workspace_run_id)
  assert.equal(recoveryRun.workspace_path, sourceRun.workspace_path)
  assert.equal(recoveryRun.provider_session_id, sourceRun.provider_session_id)
  assert.equal(waiting.value.task.contract_version, 2)
  assert.equal(waiting.value.task.attempt_count, 2)
  assert.equal(waiting.value.runs.length, runsBeforeRestart + 1)
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
  return {
    factory_work_item_id: first.factory_work_item_id,
    mission_id: first.mission_id,
    source_run_id: sourceRun.id,
    recovery_run_id: recoveryRun.id,
    contract_revision_id: recovery.launch.contract_revision_id,
    server_restart_exercised: restarted,
    unaudited_revision_rejected: true,
    weakened_policy_rejected_before_revision: true,
    same_provider_session:
      recoveryRun.provider_session_id === sourceRun.provider_session_id,
    same_workspace: recoveryRun.workspace_path === sourceRun.workspace_path,
    same_workspace_lineage:
      recoveryRun.workspace_run_id === sourceRun.workspace_run_id,
    contract_version: waiting.value.task.contract_version,
    attempt_count: waiting.value.task.attempt_count,
    final_factory_state: completed.value.item.state,
    run_count: waiting.value.runs.length,
  }
}

async function exhaustedRecoveryIsRejected() {
  const demo = await post('/api/demo/reset', {})
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
      '- [ ] The third run is never created.',
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
  assert.equal(secondFailed.value.task.max_attempts, 2)
  const exhausted = await controllerFailure(
    demo,
    issue.number,
    statePath,
    policyPath,
    {
      adapter: 'fake-process',
      recovery: 'verifier-only',
      recoveryReason:
        'A third attempt must be rejected because the task attempt ceiling is exhausted.',
    },
  )
  assert.match(exhausted, /attempt limit|exhausted/i)
  const final = await snapshot(demo)
  const finalRuns = final.snapshot.runs.filter(
    (run) => run.task_id === secondFailed.value.task.id,
  )
  assert.equal(finalRuns.length, 2)
  return {
    factory_work_item_id: first.factory_work_item_id,
    mission_id: first.mission_id,
    attempt_count: secondFailed.value.task.attempt_count,
    max_attempts: secondFailed.value.task.max_attempts,
    rejected_third_run: true,
    run_count: finalRuns.length,
  }
}

const report = {
  passed: true,
  verifier_only: await verifierOnlyRecovery(),
  source_correction: await sourceCorrectionRecovery(),
  exhausted_attempts: await exhaustedRecoveryIsRejected(),
}
await writeFile(
  path.join(output, 'e2e-factory-verification-recovery.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
