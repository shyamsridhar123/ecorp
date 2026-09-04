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
      'def render(value):',
      '  if isinstance(value, memoryview): value = value.tobytes()',
      "  if isinstance(value, (bytes, bytearray)): value = value.decode('utf-8')",
      "  return 't' if value is True else 'f' if value is False else '' if value is None else str(value)",
      'with psycopg.connect(dsn, autocommit=True) as connection:',
      '  with connection.cursor() as cursor:',
      "    statements = [item.strip() for item in sql.split(';') if item.strip()]",
      '    for statement in statements:',
      '      cursor.execute(statement)',
      '    if cursor.description:',
      '      for row in cursor.fetchall():',
      "        print('|'.join(render(value) for value in row))",
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
        max_attempts = 3
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
          candidate.source_run_id === sourceRun.id &&
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
  assert.equal(recoveryRun.resumed_from_run_id, sourceRun.id)
  assert.equal(recoveryRun.workspace_run_id, sourceRun.workspace_run_id)
  assert.equal(recoveryRun.workspace_path, sourceRun.workspace_path)
  assert.equal(recoveryRun.provider_session_id, sourceRun.provider_session_id)
  assert.equal(waiting.value.task.contract_version, 3)
  assert.equal(waiting.value.task.attempt_count, 3)
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
  await psql(`
    UPDATE secrets
    SET revoked_at = now()
    WHERE id = ${sqlLiteral(secret.secret_id)}::uuid;
  `)
  return {
    factory_work_item_id: first.factory_work_item_id,
    mission_id: first.mission_id,
    source_run_id: sourceRun.id,
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
    same_workspace: recoveryRun.workspace_path === sourceRun.workspace_path,
    same_workspace_lineage:
      recoveryRun.workspace_run_id === sourceRun.workspace_run_id,
    contract_version: waiting.value.task.contract_version,
    attempt_count: waiting.value.task.attempt_count,
    final_factory_state: completed.value.item.state,
    run_count: waiting.value.runs.length,
  }
}

async function cancelledRecoveryTerminalizes() {
  const demo = await post('/api/demo/reset', {})
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
      return recovery?.status === 'failed' &&
        run?.status === 'cancelled' &&
        run.workspace_disposition === 'preserved' &&
        run.workspace_fingerprint &&
        task?.status === 'verification_failed' &&
        task.verification_status === 'failed' &&
        mission?.status === 'failed' &&
        item?.state === 'verification_failed'
        ? { recovery, run, task, mission, item }
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
    exact_context_selected_cancelled_source: true,
    fresh_recovery_authorized: true,
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
  cancelled_recovery: await cancelledRecoveryTerminalizes(),
  exhausted_attempts: await exhaustedRecoveryIsRejected(),
}
await writeFile(
  path.join(output, 'e2e-factory-verification-recovery.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
