// Issue #167: one fresh, deterministic parallel-specialists factory graph.
// Existing Windows QA stack only; no bootstrap/reset, initial service/container
// startup, real providers, publication, cleanup, or changes to earlier fixtures.
// Required env (no defaults):
//   CRONY_PREDISPATCH_TEST=1, CRONY_PREDISPATCH_OUTPUT (existing output directory)
//   CRONY_SERVER_HTTP, DATABASE_URL, CRONY_TEST_POSTGRES_CONTAINER
//   CRONY_TEST_SERVER_BINARY, CRONY_TEST_SERVER_PID_FILE
//   CRONY_CORP_ID, CRONY_ACTOR_ID, CRONY_REVIEWER_ACTOR_ID (existing human UUIDs)
//   CRONY_RUNNER_ID, CRONY_RUNNER_WORKSPACE, CRONY_SOURCE_REPOSITORY,
//   CRONY_SOURCE_BASE_REF
// The server PID manifest has the existing owned_test_stack.mjs contract:
// test_owned:true, workspace, server_url, server, server_creation.
// Inherit the ORIGINAL server's auth/artifact/secret environment for its restart.
// Source must be an independent, clean Git fixture with a matching GitHub remote;
// requester and independent reviewer must already share its destination room.
// Only new actor/agent rows and one exact new artifact's availability use SQL.
// Everything is retained, including an interrupted fixture; never rerun by
// adopting its IDs. This is development-principal evidence, not OIDC coverage.

import assert from 'node:assert/strict'
import { execFile as execFileCallback, spawn } from 'node:child_process'
import { createHash, randomUUID } from 'node:crypto'
import { existsSync, lstatSync } from 'node:fs'
import { readFile, realpath, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { promisify } from 'node:util'
import { assertOwnedRestart, restartOwnedTestServer } from './owned_test_stack.mjs'

const execFile = promisify(execFileCallback)
const root = path.resolve(import.meta.dirname, '..')
const required = (name) => {
  assert.ok(process.env[name]?.trim(), `${name} must identify the existing owned QA fixture`)
  return process.env[name]
}
const uuid = (name) => {
  const value = required(name).toLowerCase()
  assert.match(value, /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/u, `${name} must be a UUID`)
  return value
}
const url = (name) => {
  try { return new URL(required(name)) } catch { throw new Error(`${name} must be a valid URL; value withheld`) }
}
const canonical = (value) => JSON.stringify(value, (_, item) =>
  item && typeof item === 'object' && !Array.isArray(item)
    ? Object.fromEntries(Object.entries(item).sort(([a], [b]) => a.localeCompare(b))) : item)
const digest = (value) => createHash('sha256').update(canonical(value)).digest('hex')
const equal = (actual, expected, label) => assert.equal(digest(actual), digest(expected), label)
const identityFields = (value) => Object.fromEntries(Object.entries(value).filter(([key]) => key !== 'created_at'))
const literal = (value) => value == null ? 'NULL' : `'${String(value).replaceAll("'", "''")}'`
const ids = (values) => `ARRAY[${values.map(literal).join(',')}]::uuid[]`
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const within = (parent, child) => {
  const relative = path.relative(parent, child)
  return !relative || (!relative.startsWith('..') && !path.isAbsolute(relative))
}
const active = "('provisioning','starting','running','waiting_for_input','waiting_for_approval','verifying')"

assert.equal(process.env.CRONY_PREDISPATCH_TEST, '1', 'Requires CRONY_PREDISPATCH_TEST=1')
assert.equal(process.argv.length, 2, 'No arguments; configure the existing owned stack through env')
assert.equal(process.platform, 'win32', 'Owned server restart requires Windows identity receipts')
const endpoint = url('CRONY_SERVER_HTTP')
assert.equal(endpoint.protocol, 'http:', 'Owned loopback HTTP only')
assert.equal(endpoint.hostname, '127.0.0.1', 'Restart requires a literal IPv4 SocketAddr, not localhost')
const forbiddenPorts = ['5187', '5291', '8791', '8793', '8991',
  '15191', '15193', '15491', '15493', '18962']
assert.ok(Number(endpoint.port) >= 10_000 && !forbiddenPorts.includes(endpoint.port),
  'Default/shared/manual ports, including 18962, are forbidden')
assert.ok(!endpoint.username && !endpoint.password && !endpoint.search && !endpoint.hash)
assert.equal(endpoint.pathname, '/', 'CRONY_SERVER_HTTP must be an origin')
const server = endpoint.origin
const databaseUrl = required('DATABASE_URL')
const database = url('DATABASE_URL')
assert.ok(['postgres:', 'postgresql:'].includes(database.protocol), 'Postgres URL required')
assert.ok(['127.0.0.1', 'localhost'].includes(database.hostname), 'Owned local database only')
assert.ok(!database.search && !database.hash, 'Use a plain owned local DATABASE_URL')
const databaseName = decodeURIComponent(database.pathname.slice(1))
const databaseUser = decodeURIComponent(database.username)
for (const value of [databaseName, databaseUser]) {
  assert.match(value, /^[A-Za-z_][A-Za-z0-9_$-]{0,62}$/u, 'Explicit database/user required')
}
const container = required('CRONY_TEST_POSTGRES_CONTAINER')
assert.match(container, /^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/u, 'Explicit existing container name required')
const corpId = uuid('CRONY_CORP_ID')
const actorId = uuid('CRONY_ACTOR_ID')
const reviewerId = uuid('CRONY_REVIEWER_ACTOR_ID')
assert.notEqual(actorId, reviewerId, 'Independent review requires a different human')
const runnerId = required('CRONY_RUNNER_ID')
assert.match(runnerId, /^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/u, 'Explicit runner ID required')
assert.notEqual(runnerId, 'runner-local', 'Default/manual runner is forbidden')
const baseRef = required('CRONY_SOURCE_BASE_REF')
assert.ok(baseRef.length <= 200 && !/^-/u.test(baseRef) && !/[\s\u0000-\u001f]/u.test(baseRef))
const nonce = randomUUID()
const crew = ['specialist-a', 'specialist-b', 'manager'].map((role) => ({
  id: randomUUID(), actor_id: randomUUID(), role, name: `!167 ${nonce} ${role}`,
}))
const fixture = { missionId: null, workItemId: null, taskIds: [], runIds: [] }
const report = { suite: 'predispatch-failure-167', passed: false, fixture_key: nonce,
  started_at: new Date().toISOString(), exact_ids: {
    corp_id: corpId, actor_ids: crew.map((agent) => agent.actor_id), agent_ids: crew.map((agent) => agent.id),
  }, checks: [], limitations: [
    'Deterministic fake-process and development principals only; no real-provider/browser/OIDC evidence.',
    'Native approval replay is covered; direct duplicate/late/foreign store callbacks need store tests.',
    'Existing stack and all fixture evidence are retained; no automatic rollback or cleanup.',
  ] }
let phase = 'read-only ownership admission'
let reportPath
let sourceRoot, runnerRoot, binary, source, originalSource, originalRows
const deadline = Date.now() + 240_000
let requests = 0
function bounded() {
  assert.ok(Date.now() < deadline && ++requests <= 250, 'Regression exceeded its time/request bound')
}

async function command(program, args, options = {}) {
  bounded()
  try {
    return (await execFile(program, args, {
      windowsHide: true, timeout: 15_000, maxBuffer: 1024 * 1024, ...options,
    })).stdout.trim()
  } catch {
    throw new Error(`${program} check failed; arguments/output withheld`)
  }
}
async function existingPath(name, directory = true) {
  const value = required(name)
  assert.ok(path.isAbsolute(value), `${name} must be absolute`)
  const resolved = await realpath(value)
  assert.equal((await stat(resolved)).isDirectory(), directory, `${name} has the wrong file type`)
  return resolved
}
const git = (args, cwd = sourceRoot) => command('git', args, { cwd })
const api = (suffix) => `/api/corps/${corpId}${suffix}`
async function request(route, body) {
  bounded()
  assert.ok(route.startsWith(api('/')), 'Only this owned Corp API is permitted')
  const response = await fetch(`${server}${route}`, {
    method: body === undefined ? 'GET' : 'POST', redirect: 'error',
    headers: { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(10_000),
  })
  assert.equal(response.status, 200, `${route}: HTTP ${response.status}; body withheld`)
  const reader = response.body.getReader()
  const chunks = []
  let bytes = 0
  try {
    while (true) {
      const { done, value } = await reader.read()
      if (done) break
      bytes += value.length
      assert.ok(bytes <= 8 * 1024 * 1024, 'API response exceeded the byte bound')
      chunks.push(value)
    }
  } finally { await reader.cancel() }
  return JSON.parse(Buffer.concat(chunks).toString('utf8'))
}
const snapshot = () => request(api(`/snapshot?actor_id=${actorId}`))

// Native Docker psql only. SQL uses stdin; only URL-derived user/database are
// arguments. Docker forwards PGPASSWORD by NAME, never as an argument value.
async function sql(statement, write = false) {
  bounded()
  return new Promise((resolve, reject) => {
    const child = spawn('docker', ['exec', '-i', '--env', 'PGPASSWORD', container,
      'psql', '-X', '-q', '-A', '-t', '-w', '-v', 'ON_ERROR_STOP=1',
      '-h', '127.0.0.1', '-p', '5432', '-U', databaseUser, '-d', databaseName, '-f', '-'], {
      windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'],
      env: { ...process.env, PGPASSWORD: decodeURIComponent(database.password) },
    })
    let output = '', size = 0
    const fail = () => { child.kill(); reject(new Error('Scoped psql failed; SQL/stderr withheld')) }
    const timer = setTimeout(fail, 15_000)
    child.on('error', fail)
    child.stdin.on('error', fail)
    child.stdout.on('data', (chunk) => {
      size += chunk.length
      if (size > 1024 * 1024) fail()
      else output += chunk.toString('utf8')
    })
    child.stderr.on('data', (chunk) => { size += chunk.length; if (size > 1024 * 1024) fail() })
    child.on('close', (code) => {
      clearTimeout(timer)
      if (code !== 0) return reject(new Error('Scoped psql failed; SQL/stderr withheld'))
      try { resolve(JSON.parse(output.trim())) } catch { reject(new Error('Invalid bounded psql JSON result')) }
    })
    child.stdin.end(`BEGIN ISOLATION LEVEL REPEATABLE READ${write ? '' : ' READ ONLY'};
      SET LOCAL statement_timeout = '10s'; SET LOCAL lock_timeout = '3s';
      ${statement}
      COMMIT;\n`)
  })
}
async function processOwnership() {
  assert.ok((await stat(required('CRONY_TEST_SERVER_PID_FILE'))).size <= 65_536, 'Owned process manifest is oversized')
  const manifest = JSON.parse(await readFile(required('CRONY_TEST_SERVER_PID_FILE'), 'utf8'))
  assert.ok(typeof manifest.workspace === 'string' && path.isAbsolute(manifest.workspace),
    'Owned manifest must explicitly name its absolute workspace')
  assert.ok(Number.isSafeInteger(manifest.server) && manifest.server > 0, 'Invalid owned server PID')
  // Use the same PowerShell 7 runtime as the owned stack launcher. The legacy
  // Windows PowerShell CIM import can exceed the unchanged probe deadline.
  const identity = JSON.parse(await command('pwsh.exe', [
    '-NoProfile', '-NonInteractive', '-Command', [
      "$ErrorActionPreference='Stop'",
      '$p=Get-CimInstance Win32_Process -Filter "ProcessId = $env:ECORP_QA_PROCESS_ID"',
      "if (!$p) { throw 'Owned server absent' }",
      '$listeners=@(Get-NetTCPConnection -State Listen -LocalPort ([int]$env:ECORP_QA_PROCESS_PORT))',
      "@{executable=$p.ExecutablePath; creation=$p.CreationDate.ToUniversalTime().ToString('o');",
      'port_owned=[bool]($listeners | Where-Object OwningProcess -eq $p.ProcessId)} | ConvertTo-Json -Compress',
    ].join('\n'),
  ], { env: { ...process.env, ECORP_QA_PROCESS_ID: String(manifest.server),
    ECORP_QA_PROCESS_PORT: endpoint.port } }))
  assertOwnedRestart(manifest, identity, { root, server, binary })
}
function connectedSource(state) {
  assert.equal(state.snapshot.corp.id, corpId, 'API Corp identity mismatch')
  const connected = state.runners.filter((runner) => runner.connected)
  equal(connected.map((runner) => runner.id), [runnerId], 'Use exactly the existing owned runner')
  const runner = connected[0]
  assert.equal(runner.corp_id, corpId, 'Runner Corp mismatch')
  assert.ok(runner.capabilities.some((cap) => cap.name === 'fake-process' && cap.available))
  const cap = runner.capabilities.find((cap) => cap.name === 'workspace-isolation' && cap.available)
  assert.ok(cap?.source_repository && cap.source_base_ref && cap.source_base_commit,
    'Runner must advertise complete immutable workspace identity')
  return { repository: cap.source_repository, base_ref: cap.source_base_ref, base_commit: cap.source_base_commit }
}
async function sourceReceipt() {
  return { head: await git(['rev-parse', 'HEAD']),
    status: await git(['status', '--porcelain=v1', '--untracked-files=all']),
    worktrees: (await git(['worktree', 'list', '--porcelain'])).split(/\r?\n\r?\n/u).filter(Boolean) }
}
async function preservedRows() {
  const tables = [
    ['missions', `id IS DISTINCT FROM ${literal(fixture.missionId)}::uuid`],
    ['tasks', `mission_id IS DISTINCT FROM ${literal(fixture.missionId)}::uuid`],
    ['runs', `task_id <> ALL(${ids(fixture.taskIds)})`],
    ['artifacts', `task_id <> ALL(${ids(fixture.taskIds)})`],
    ['factory_work_items', `id IS DISTINCT FROM ${literal(fixture.workItemId)}::uuid`],
    ['actors', `id <> ALL(${ids(crew.map((agent) => agent.actor_id))})`],
    ['agents', `id <> ALL(${ids(crew.map((agent) => agent.id))})`],
  ]
  return sql(`SELECT jsonb_build_object(${tables.map(([table, condition]) => `
    '${table}', (SELECT jsonb_build_array(count(*),
      md5(coalesce(string_agg(md5(to_jsonb(t)::text), '' ORDER BY t.id), '')))
      FROM ${table} t WHERE corp_id=${literal(corpId)}::uuid AND ${condition})`).join(',')});`)
}
async function admission() {
  sourceRoot = await existingPath('CRONY_SOURCE_REPOSITORY')
  runnerRoot = await existingPath('CRONY_RUNNER_WORKSPACE')
  binary = await existingPath('CRONY_TEST_SERVER_BINARY', false)
  const output = await existingPath('CRONY_PREDISPATCH_OUTPUT')
  const pidFile = await existingPath('CRONY_TEST_SERVER_PID_FILE', false)
  assert.ok(!within(root, sourceRoot) && !within(sourceRoot, root), 'Source must be outside the ECorp checkout')
  assert.ok(!within(sourceRoot, runnerRoot) && !within(runnerRoot, sourceRoot), 'Separate source and runner roots required')
  assert.ok(!within(sourceRoot, output) && !within(runnerRoot, output), 'Reports must not modify source/workspaces')
  assert.ok(!within(sourceRoot, pidFile) && !within(runnerRoot, pidFile), 'Restart manifest/logs must not modify source/workspaces')
  equal(path.resolve(await git(['rev-parse', '--show-toplevel'])), sourceRoot, 'Source must be its Git root')
  assert.notEqual(await git(['rev-parse', '--path-format=absolute', '--git-common-dir']),
    await git(['rev-parse', '--path-format=absolute', '--git-common-dir'], root),
    'Do not use the ECorp checkout or one of its linked worktrees as the source fixture')
  await processOwnership()
  const ports = JSON.parse(await command('docker', ['inspect', '--format', '{{json .NetworkSettings.Ports}}', container]))
  assert.ok(ports['5432/tcp']?.some((binding) =>
    binding.HostPort === (database.port || '5432') &&
    ['127.0.0.1', '0.0.0.0', '::'].includes(binding.HostIp)), 'DATABASE_URL must target this existing container')
  const state = await snapshot()
  source = connectedSource(state)
  assert.equal(source.base_ref, baseRef, 'Source ref mismatch')
  assert.equal(source.base_commit, await git(['rev-parse', '--verify', `${baseRef}^{commit}`]), 'Source commit mismatch')
  const remote = await git(['remote', 'get-url', 'origin'])
  const match = remote.match(/github\.com[:/]([A-Za-z0-9_.-]+)\/([A-Za-z0-9_.-]+?)(?:\.git)?$/iu)
  assert.ok(match, 'Independent source requires an existing matching GitHub remote; no remote calls are made')
  assert.equal(`${match[1]}/${match[2]}`.toLowerCase(), source.repository, 'Source repository mismatch')
  const identity = await sql(`SELECT jsonb_build_object(
    'database', current_database(), 'user', current_user,
    'corp', (SELECT to_jsonb(c) FROM corps c WHERE id=${literal(corpId)}::uuid),
    'people', (SELECT jsonb_agg(to_jsonb(a) ORDER BY id) FROM actors a
      WHERE corp_id=${literal(corpId)}::uuid AND id=ANY(${ids([actorId, reviewerId])})),
    'room', (SELECT r.id FROM rooms r JOIN room_memberships m ON m.room_id=r.id
      WHERE r.corp_id=${literal(corpId)}::uuid AND m.actor_id=${literal(actorId)}::uuid
      ORDER BY r.created_at,r.id LIMIT 1),
    'reviewer_rooms', (SELECT jsonb_agg(room_id) FROM room_memberships WHERE actor_id=${literal(reviewerId)}::uuid),
    'runner', (SELECT jsonb_build_object('id',id,'corp_id',corp_id,'capabilities',capabilities,'status',status)
      FROM runner_nodes WHERE id=${literal(runnerId)}),
    'crew_names', (SELECT jsonb_agg(name) FROM agents WHERE corp_id=${literal(corpId)}::uuid
      AND adapter='fake-process' AND status='idle' AND retired_at IS NULL AND (mission_id IS NULL OR pinned)),
    'busy', EXISTS(SELECT 1 FROM runs WHERE status IN ${active})
      OR EXISTS(SELECT 1 FROM runner_commands WHERE status='pending'));`)
  assert.equal(identity.database, databaseName, 'Database name mismatch')
  assert.equal(identity.user, databaseUser, 'Database user mismatch')
  equal(identityFields(identity.corp), identityFields(state.snapshot.corp), 'Database/API Corp mismatch')
  assert.equal(identity.busy, false, 'Owned database must have no active runs or pending commands')
  assert.ok(identity.room && identity.reviewer_rooms?.includes(identity.room), 'Existing reviewers must share the selected room')
  for (const person of identity.people ?? []) {
    assert.equal(person.kind, 'human', 'Existing human principals required')
    assert.ok(['owner', 'admin', 'manager', 'member'].includes(person.role), 'Human lacks required role')
    equal(identityFields(person), identityFields(state.snapshot.actors.find((actor) => actor.id === person.id)),
      'Database/API actor mismatch')
  }
  assert.equal(identity.people?.length, 2, 'Both existing human identities are required')
  assert.ok(['owner', 'admin', 'manager'].includes(identity.people.find((person) => person.id === actorId).role))
  const apiRunner = state.runners.find((runner) => runner.id === runnerId)
  equal(identity.runner, { id: runnerId, corp_id: corpId, capabilities: apiRunner.capabilities, status: 'connected' },
    'Database/API runner mismatch')
  // Retained earlier copies remain unchanged. Rank only the new deterministic
  // crew first, with a bounded name prefix rather than changing old identities.
  const names = identity.crew_names ?? []
  assert.ok(names.length <= 1000, 'Use a bounded owned QA roster')
  const bangs = Math.max(0, ...names.map((name) => name.match(/^!*/u)[0].length)) + 1
  assert.ok(bangs <= 32, 'Fixture name-priority bound exhausted; do not relabel prior agents')
  for (const agent of crew) {
    agent.name = `${'!'.repeat(bangs)}167 ${nonce} ${agent.role}`
    assert.ok(names.every((name) => agent.name < name), 'Cannot safely prioritize the new crew in this roster')
  }
  fixture.roomId = identity.room
  originalSource = await sourceReceipt()
  assert.equal(originalSource.status, '', 'Use a clean independent source; never reset it')
  originalRows = await preservedRows()
  const reservedPath = path.join(output, `e2e-predispatch-failure-${nonce}.json`)
  await writeFile(reservedPath, `${JSON.stringify(report, null, 2)}\n`, { flag: 'wx' })
  reportPath = reservedPath
}

async function save() {
  Object.assign(report.exact_ids, { factory_work_item_id: fixture.workItemId,
    mission_id: fixture.missionId, task_ids: fixture.taskIds, run_ids: fixture.runIds })
  report.phase = phase
  const text = `${JSON.stringify(report, null, 2)}\n`
  assert.ok(Buffer.byteLength(text) <= 16_384, 'Report exceeded its byte bound')
  if (reportPath) await writeFile(reportPath, text)
}
async function createFixture() {
  // New identities only: no cloning/relabeling an existing provider or fixture.
  await sql(`${crew.map((agent) => `
    INSERT INTO actors(id,corp_id,name,kind,role) VALUES
      (${literal(agent.actor_id)}::uuid,${literal(corpId)}::uuid,${literal(agent.name)},'agent',${literal(agent.role)});
    INSERT INTO agents(id,corp_id,actor_id,name,role,adapter,status,accent) VALUES
      (${literal(agent.id)}::uuid,${literal(corpId)}::uuid,${literal(agent.actor_id)}::uuid,
       ${literal(agent.name)},${literal(agent.role)},'fake-process','idle','cobalt');
    INSERT INTO room_memberships(room_id,actor_id,role) VALUES
      (${literal(fixture.roomId)}::uuid,${literal(agent.actor_id)}::uuid,'member');`).join('\n')}
    SELECT jsonb_build_object('inserted_agents',3);`, true)
  const [owner, repository] = source.repository.split('/')
  const title = `Predispatch failure regression ${nonce}`
  const description = 'Produce deterministic parent artifacts; test only dependency admission, not a model.'
  const policy = { schema_version: 1, source_of_truth: 'github_project',
    project_owner: owner, project_number: 991167, project_status: 'Todo',
    required_label: 'factory:ready', dependencies: [], repository_allowlist: [source.repository],
    source_base_ref: source.base_ref, source_base_commit: source.base_commit,
    source_commit_upgrade_required: false, adapter_allowlist: ['fake-process'],
    strategy_allowlist: ['parallel-specialists'], model: null, reasoning_effort: null,
    write_scope: ['**'], allowed_tools: ['filesystem', 'shell'], prohibited_actions: [
      'modify files outside the assigned worktree', 'use undeclared long-lived credentials',
      'merge or deploy without a separate current authorization',
    ], secret_ids: [], verification_required: true, budget_tokens: 100_000,
    budget_cost_microusd: 1_000_000, auto_merge: false }
  const verification = { checks: [{ type: 'artifact', min_bytes: 1 }],
    manual_gate: { type: 'independent_review', roles: ['owner', 'admin', 'manager', 'member'], exclude_requester: true } }
  const materialization = { actor_id: actorId, title, description, preferred_adapter: 'fake-process',
    strategy: 'parallel-specialists', budget_tokens: policy.budget_tokens,
    budget_cost_microusd: policy.budget_cost_microusd, verification_policy: verification,
    contract: { objective: description, expected_output: 'A verified deterministic parent artifact.',
      acceptance_tests: ['The persisted artifact check and independent review pass.'],
      allowed_tools: policy.allowed_tools, prohibited_actions: policy.prohibited_actions,
      references: [`https://github.com/${source.repository}/issues/991167`], write_scope: policy.write_scope } }
  const preflight = await request(api('/factory/preflight'), { ...materialization,
    source_repository_owner: owner, source_repository_name: repository, policy })
  assert.ok(preflight.valid && preflight.task_count === 3, 'Native factory preflight must accept exactly three tasks')
  const claim = await request(api('/factory/work-items/claim'), {
    actor_id: actorId, source_project_owner: owner, source_project_number: policy.project_number,
    source_project_item_id: `PVTI_PREDISPATCH_${nonce}`, source_repository_owner: owner,
    source_repository_name: repository, source_issue_number: 991167,
    source_issue_node_id: `I_PREDISPATCH_${nonce}`,
    source_issue_url: `https://github.com/${source.repository}/issues/991167`,
    source_title: title, source_revision: report.started_at, policy,
    idempotency_key: `${nonce}-claim`, lease_seconds: 300,
  })
  assert.equal(claim.replayed, false, 'Never adopt an earlier fixture')
  fixture.workItemId = claim.work_item.id
  await save()
  const materialized = await request(api(`/factory/work-items/${fixture.workItemId}/materialize`), {
    ...materialization, claim_token: claim.claim_token, expected_version: claim.work_item.version,
    idempotency_key: `${nonce}-materialize`,
  })
  assert.equal(materialized.replayed, false, 'Materialization must create a fresh held graph')
  fixture.missionId = materialized.mission_id
  const held = (await snapshot()).snapshot
  const tasks = held.tasks.filter((task) => task.mission_id === fixture.missionId)
  fixture.taskIds = tasks.map((task) => task.id).sort()
  await save()
  assert.equal(tasks.length, 3, 'Exactly three tasks required')
  assert.equal(held.missions.find((mission) => mission.id === fixture.missionId)?.status, 'ready')
  assert.equal(held.missions.find((mission) => mission.id === fixture.missionId)?.room_id, fixture.roomId)
  assert.equal(held.runs.filter((run) => fixture.taskIds.includes(run.task_id)).length, 0)
  equal(tasks.map((task) => task.assigned_agent_id).sort(), crew.map((agent) => agent.id).sort(),
    'Planner must use only the three new fake-process identities; refuse to launch otherwise')
  equal(tasks.map((task) => task.plan_key).sort(), ['specialist-a', 'specialist-b', 'synthesis'], 'Unexpected graph')
  for (const task of tasks) {
    assert.equal(task.required_adapter, 'fake-process', 'Real providers are forbidden')
    assert.equal(task.attempt_count, 0, 'Held tasks must never have started')
    await request(api(`/missions/${fixture.missionId}/contract-revisions`), {
      actor_id: actorId, task_id: task.id, expected_contract_version: task.contract_version,
      next_action: 'redispatch', source_run_id: null, reason: 'Gate this new predispatch fixture before its first run.',
      idempotency_key: randomUUID(), description,
      contract: task.contract, verification_policy: verification,
    })
  }
  const parents = ['specialist-a', 'specialist-b'].map((key) => tasks.find((task) => task.plan_key === key))
  fixture.childId = tasks.find((task) => task.plan_key === 'synthesis').id
  equal([...tasks.find((task) => task.id === fixture.childId).depends_on].sort(),
    parents.map((task) => task.id).sort(), 'Synthesis must depend on both parents')
  return parents
}

// Complete exact-mission journal, not the snapshot's latest-200-event window.
// Dynamic run selection catches extra retries; a single read-only MVCC snapshot
// observes run/task/mission/factory state together. Oversized output fails closed.
async function observe() {
  const mission = literal(fixture.missionId), item = literal(fixture.workItemId)
  const tables = [
    ['missions', `id=${mission}::uuid`, 'id'],
    ['tasks', `mission_id=${mission}::uuid`, 'id'],
    ['runs', 'task_id IN (SELECT id FROM ft)', 'id', " - 'assignment_token'"],
    ['factory_work_items', `id=${item}::uuid OR mission_id=${mission}::uuid`, 'id', " - 'claim_token'"],
    ['factory_verification_recoveries', `mission_id=${mission}::uuid`, 'id'],
    ...['artifacts', 'source_deliverables', 'verification_evidence', 'action_approvals', 'runner_commands']
      .map((table) => [table, 'run_id IN (SELECT id FROM fr)', 'id']),
    ['verification_requests', 'run_id IN (SELECT id FROM fr)', 'run_id'],
    ['factory_operations', `work_item_id=${item}::uuid`, 'idempotency_key', " - 'claim_token' - 'request'"],
    ['events', `correlation_id=${mission}::uuid OR aggregate_id=${item}::uuid
      OR aggregate_id=${mission}::uuid OR aggregate_id IN (SELECT id FROM ft)
      OR aggregate_id IN (SELECT id FROM fr)`, 'seq'],
  ]
  const state = await sql(`WITH ft AS (SELECT id FROM tasks WHERE corp_id=${literal(corpId)}::uuid
      AND mission_id=${mission}::uuid), fr AS (SELECT id FROM runs WHERE corp_id=${literal(corpId)}::uuid
      AND task_id IN (SELECT id FROM ft))
    SELECT jsonb_build_object(${tables.map(([table, where, order, omit = '']) => `
      '${table}', (SELECT coalesce(jsonb_agg(to_jsonb(t)${omit} ORDER BY ${order}),'[]'::jsonb)
      FROM ${table} t WHERE corp_id=${literal(corpId)}::uuid AND (${where}))`).join(',')},
      'dependencies', (SELECT coalesce(jsonb_agg(to_jsonb(d) ORDER BY task_id,depends_on_task_id),'[]'::jsonb)
        FROM task_dependencies d WHERE task_id IN (SELECT id FROM ft)));`)
  assert.ok(state.events.length <= 200 && state.runs.length <= 6, 'Fixture exceeded its bounded journal/run allowance')
  return state
}
async function waitFor(predicate, label) {
  const until = Math.min(deadline, Date.now() + 90_000)
  while (Date.now() < until) {
    const state = await observe()
    if (predicate(state)) return state
    await sleep(500)
  }
  throw new Error(`Timed out: ${label}; preserve this fixture and inspect its exact IDs`)
}
const taskRun = (state, taskId) => state.runs.find((run) => run.task_id === taskId)
const reviewReady = (run) => run?.status === 'waiting_for_approval' &&
  run.workspace_disposition === 'preserved' && run.workspace_fingerprint
const workspacePath = (taskId, runId) => path.join(runnerRoot, 'worktrees',
  taskId.replaceAll('-', ''), runId.replaceAll('-', ''))
async function decide(runId, body, replayed) {
  const result = await request(api(`/runs/${runId}/verification-decision`), body)
  equal(result, { run_id: runId, status: 'approved', replayed }, 'Native parent decision/replay mismatch')
}
function assertFailed(state, beforeVersion, parentIds) {
  equal(state.tasks.map((task) => task.id).sort(), fixture.taskIds, 'Original task IDs changed')
  equal(state.runs.map((run) => run.id).sort(), fixture.runIds, 'Extra/replaced fixture run')
  assert.equal(state.missions.length, 1)
  assert.equal(state.missions[0].status, 'failed', 'Mission must fail')
  assert.equal(state.factory_work_items.length, 1)
  const item = state.factory_work_items[0], child = taskRun(state, fixture.childId)
  const task = state.tasks.find((entry) => entry.id === fixture.childId)
  const reason = 'dependency context could not be verified: dependency handoff is incomplete or unverified: expected 2, found 1'
  assert.equal(child.status, 'failed')
  assert.equal(task.status, 'failed', 'Never synthesize a verification_failed task')
  assert.equal(task.attempt_count, 1)
  for (const entry of [child, task]) assert.equal(entry.verification_status, 'pending')
  assert.equal(child.summary, reason, 'Must fail dependency loading, not another admission stage')
  assert.equal(child.workspace_detail, 'dispatch_not_started')
  assert.equal(child.workspace_run_id, child.id)
  assert.equal(child.execution_mode, 'provider')
  assert.equal(child.runner_id, runnerId)
  for (const [field, expected] of [['source_repository', source.repository],
    ['source_base_ref', source.base_ref], ['source_base_commit', source.base_commit]]) assert.equal(child[field], expected, field)
  for (const field of ['input_tokens', 'output_tokens', 'cost_microusd']) assert.equal(child[field], 0, field)
  for (const field of ['provider_session_id', 'resumed_from_run_id', 'workspace_path', 'workspace_branch',
    'workspace_base_ref', 'workspace_base_commit', 'workspace_disposition', 'workspace_fingerprint',
    'verification_summary', 'verification_sha256', 'deliverable_sha256', 'artifact_id', 'artifact_uri',
    'artifact_media_type', 'artifact_signature', 'artifact_path', 'artifact_sha256']) assert.equal(child[field], null, field)
  assert.equal(lstatSync(workspacePath(fixture.childId, child.id), { throwIfNoEntry: false }), undefined,
    'Never-started child must have no physical workspace, including dangling links')
  for (const table of ['artifacts', 'source_deliverables', 'verification_evidence',
    'verification_requests', 'action_approvals', 'runner_commands']) {
    assert.equal(state[table].filter((entry) => entry.run_id === child.id).length, 0, `Unexpected child ${table}`)
  }
  assert.equal(state.factory_verification_recoveries.length, 0, 'No fabricated verification recovery')
  for (const id of parentIds) {
    const run = state.runs.find((entry) => entry.id === id)
    const parent = state.tasks.find((entry) => entry.id === run.task_id)
    for (const entry of [run, parent]) {
      assert.equal(entry.status, 'completed', 'Approved parent changed')
      assert.equal(entry.verification_status, 'passed')
    }
    assert.equal(state.verification_requests.find((entry) => entry.run_id === id)?.status, 'approved')
  }
  assert.equal(item.state, 'blocked', 'Regression: factory item stranded awaiting_approval')
  assert.equal(item.id, fixture.workItemId)
  assert.equal(item.mission_id, fixture.missionId)
  assert.equal(item.version, beforeVersion + 1, 'Factory must transition exactly once')
  assert.equal(item.failure_detail, reason)
  assert.ok(Buffer.byteLength(item.failure_detail, 'utf8') <= 2000 &&
    !/[\u0000-\u001f\u007f-\u009f]/u.test(item.failure_detail), 'Failure detail must be bounded and printable')
  const childEvents = state.events.filter((event) => event.aggregate_id === child.id)
  equal(childEvents.map((event) => event.type), ['run.requested', 'run.failed'], 'Child provider/verifier must never start')
  const failed = childEvents[1]
  const blocked = state.events.filter((event) => event.aggregate_id === item.id && event.type === 'factory.blocked')
  assert.equal(blocked.length, 1, 'Exactly one correlated factory.blocked event required')
  for (const event of [failed, blocked[0]]) {
    assert.equal(event.corp_id, corpId)
    assert.equal(event.room_id, fixture.roomId)
    assert.equal(event.correlation_id, fixture.missionId)
  }
  assert.equal(failed.aggregate_type, 'run')
  assert.equal(failed.payload.error, reason)
  assert.equal(failed.payload.dispatch_not_started, true)
  assert.equal(failed.idempotency_key, `run:${child.id}:dispatch-failed`)
  assert.equal(blocked[0].aggregate_type, 'factory_work_item')
  assert.equal(blocked[0].aggregate_version, item.version)
  equal(blocked[0].payload, { previous_state: 'awaiting_approval', state: 'blocked',
    mission_id: fixture.missionId, run_id: child.id, failure_detail: reason, dispatch_not_started: true },
  'Factory failure event must retain exact correlation and bounded reason')
  assert.ok(failed.seq < blocked[0].seq, 'run.failed must precede factory.blocked')
  return { run_failed_seq: failed.seq, factory_blocked_seq: blocked[0].seq, reason }
}

try {
  await admission() // Every operation above this checkpoint is read-only.
  phase = 'new held factory fixture'
  const parents = await createFixture()
  const launched = await request(api(`/missions/${fixture.missionId}/launch`), { requested_by: actorId })
  assert.equal(launched.run_ids.length, 2, 'Only the two roots may launch')
  const held = await waitFor((state) => parents.every((parent) => reviewReady(taskRun(state, parent.id))), 'both parent review gates')
  assert.equal(held.runs.length, 2, 'Child must remain undispatched')
  const parentRuns = parents.map((parent) => taskRun(held, parent.id))
  fixture.runIds = parentRuns.map((run) => run.id).sort()
  for (const run of parentRuns) {
    assert.equal(run.runner_id, runnerId)
    equal(path.resolve(run.workspace_path), workspacePath(run.task_id, run.id), 'Runner workspace env does not match the real fixture')
    assert.ok(existsSync(run.workspace_path), 'Parent workspace must be preserved')
  }
  const decisions = parentRuns.map(() => ({ actor_id: reviewerId, approved: true,
    note: 'Accept this exact new deterministic parent artifact.', decision_key: randomUUID() }))
  await save()
  phase = 'reject only the accepted new parent artifact'
  await decide(parentRuns[0].id, decisions[0], false)
  const first = await observe()
  assert.equal(taskRun(first, parents[0].id).status, 'completed')
  assert.ok(reviewReady(taskRun(first, parents[1].id)), 'Second parent must remain held')
  assert.equal(first.runs.length, 2)
  assert.equal(first.tasks.find((task) => task.id === fixture.childId).attempt_count, 0)
  assert.equal(first.factory_work_items[0].state, 'awaiting_approval')
  const version = first.factory_work_items[0].version
  const artifactId = parentRuns[0].artifact_id
  assert.ok(artifactId, 'New first parent must have a real artifact')
  equal(first.artifacts.map((artifact) => artifact.id).sort(), parentRuns.map((run) => run.artifact_id).sort(),
    'The two parents must own exactly their two new real provider artifacts')
  report.exact_ids.unavailable_artifact_id = artifactId
  const changed = await sql(`WITH changed AS (
    UPDATE artifacts a SET status='rejected', rejection_reason='e2e_predispatch_failure: parent A unavailable'
    FROM runs r WHERE a.id=${literal(artifactId)}::uuid AND a.corp_id=${literal(corpId)}::uuid
      AND a.task_id=${literal(parents[0].id)}::uuid AND a.run_id=${literal(parentRuns[0].id)}::uuid
      AND a.status='ready' AND a.artifact_role='provider_evidence'
      AND r.id=a.run_id AND r.corp_id=a.corp_id AND r.task_id=a.task_id AND r.artifact_id=a.id
      AND r.status='completed' AND r.verification_status='passed'
      AND EXISTS(SELECT 1 FROM runs WHERE id=${literal(parentRuns[1].id)}::uuid
        AND corp_id=a.corp_id AND status='waiting_for_approval')
      AND NOT EXISTS(SELECT 1 FROM runs WHERE task_id=${literal(fixture.childId)}::uuid)
    RETURNING a.id) SELECT coalesce(jsonb_agg(id),'[]'::jsonb) FROM changed;`, true)
  equal(changed, [artifactId], 'Fixture SQL must change exactly the new first parent artifact')
  const unavailable = await observe()
  equal(unavailable.artifacts, first.artifacts.map((artifact) => artifact.id === artifactId
    ? { ...artifact, status: 'rejected', rejection_reason: 'e2e_predispatch_failure: parent A unavailable' } : artifact),
  'Availability injection must preserve every hash, signature, byte reference, and other artifact')
  await save()
  phase = 'dependency admission fails atomically'
  await decide(parentRuns[1].id, decisions[1], false)
  const failed = await waitFor((state) => taskRun(state, fixture.childId)?.status === 'failed', 'child dependency failure')
  fixture.runIds = failed.runs.map((run) => run.id).sort()
  assert.equal(fixture.runIds.length, 3, 'Exactly one child attempt required')
  report.observed = { mission_status: failed.missions[0]?.status,
    child_task_status: failed.tasks.find((task) => task.id === fixture.childId)?.status,
    child_run_status: taskRun(failed, fixture.childId)?.status,
    factory_state: failed.factory_work_items[0]?.state, run_count: failed.runs.length }
  const evidence = assertFailed(failed, version, parentRuns.map((run) => run.id))
  report.checks.push('atomic_predispatch_failure', 'zero_child_execution_or_recovery')
  report.failure = evidence
  const baseline = digest(failed)
  async function stable() {
    await sleep(1000)
    const current = await observe()
    assertFailed(current, version, parentRuns.map((run) => run.id))
    assert.equal(digest(current), baseline, 'Replay/restart changed exact fixture state, IDs, or journal')
  }
  async function replayParents() {
    for (let index = 0; index < parentRuns.length; index++) {
      await decide(parentRuns[index].id, decisions[index], true)
      await stable()
    }
  }
  phase = 'native decision replay'
  await replayParents()
  report.checks.push('native_parent_replay_no_effects')
  await save()
  phase = 'restart only the receipt-owned server'
  await processOwnership()
  const quiet = await sql(`SELECT to_jsonb(NOT EXISTS(SELECT 1 FROM runs WHERE status IN ${active}));`)
  assert.equal(quiet, true, 'Refuse restart while any run is active')
  await restartOwnedTestServer({ root, server, databaseUrl, binary, logPrefix: `predispatch-${nonce}` })
  equal(connectedSource(await snapshot()), source, 'Restart changed owned runner/source identity')
  await stable()
  await replayParents()
  report.checks.push('owned_restart_and_replay_no_effects')
  equal(await preservedRows(), originalRows, 'An original fixture row changed')
  const finalSource = await sourceReceipt()
  assert.equal(finalSource.head, originalSource.head, 'Original source HEAD changed')
  assert.equal(finalSource.status, originalSource.status, 'Original source files changed')
  for (const worktree of originalSource.worktrees) {
    assert.ok(finalSource.worktrees.includes(worktree), 'An original worktree was altered or removed')
  }
  report.checks.push('original_fixtures_source_worktrees_preserved')
  report.fixture_state_sha256 = baseline
  report.passed = true
  phase = 'complete'
} catch (error) {
  // Never serialize errors' actual/expected values, SQL stderr, response bodies,
  // claim/assignment tokens, credentials, or the full persisted projections.
  report.error = { stage: phase, kind: error?.code === 'ERR_ASSERTION' ? 'assertion_failed' : 'operation_failed' }
  process.exitCode = 1
} finally {
  report.finished_at = new Date().toISOString()
  await save()
  console.log(JSON.stringify(report, null, 2))
}
