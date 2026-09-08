// PURE tests only. All API, replay, clock, checkpoint and workspace I/O below is
// in memory. This file never starts a server/provider, calls an actual endpoint,
// reads runtime/credentials/environment, or writes filesystem fixtures.
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import path from 'node:path'
import test from 'node:test'
import {
  BASE_SHA256, CASES, FIXTURE, LIMITS, PROTECTED_PORTS,
  assessCase, assertHistory, assertRunner, canonical, caseRequest,
  configurationFromReceipt, containedPath, createApi, digest, executeSuite,
  expectedWorkspace, historySummary, localAbsolute, newReport, parseArgs,
  policyDigests, readBaseFile, receiptIdentity, replaySummary, rustVerificationJson,
  validateReceipt, validateSavedReport,
} from './e2e_stopped_source_checkpoint.mjs'

const id = (number) => `00000000-0000-4000-8000-${number.toString(16).padStart(12, '0')}`
const clone = (value) => structuredClone(value)
const ROOT = path.sep === '\\' ? 'C:\\owned-qa' : '/owned-qa'
const SENTINEL = 'PRIVATE_PROVIDER_CONTENT_OR_ASSIGNMENT_TOKEN_MUST_NOT_BE_LOGGED'
const FIXTURE_SHA256 = '4'.repeat(64) // Synthetic receipt metadata, no disk read.

function configuration(overrides = {}) {
  return validateReceipt({
    schema_version: 1,
    test_owned: true,
    receipt_id: id(900),
    auth_mode: 'development',
    server_url: 'http://127.0.0.1:18574',
    corp_id: id(1),
    actor_id: id(17),
    runner_id: 'issue174-local-start',
    source: { repository: 'shyamsridhar123/ecorp-enterprise-lab', base_ref: 'HEAD',
      base_commit: 'a8894b5f02d56f10e2da38df47a450ff71e92fbe' },
    runner_root: path.join(ROOT, 'runner-workspaces'),
    output_dir: path.join(ROOT, 'evidence'),
    report_path: path.join(ROOT, 'evidence', 'e2e-stopped-source-checkpoint.json'),
    provider: { adapter: 'codex', fixture: FIXTURE, sha256: FIXTURE_SHA256 },
    timeout_ms: 15_000, poll_ms: 250, settle_ms: 2_000,
    ...overrides,
  })
}

function emptyState(config = configuration()) {
  const tables = ['missions', 'tasks', 'runs', 'actors', 'rooms', 'agents',
    'verification_evidence', 'verification_requests', 'source_deliverables',
    'mission_contract_revisions', 'mission_budget_revisions', 'action_approvals',
    'circuit_breaker_incidents', 'events']
  return {
    snapshot: {
      ...Object.fromEntries(tables.map((table) => [table, []])),
      corp: { id: config.corp_id },
      actors: [{ id: config.actor_id, corp_id: config.corp_id, kind: 'human', role: 'owner' }],
      rooms: [{ id: id(20), corp_id: config.corp_id }],
    },
    runners: [{
      id: config.runner_id, corp_id: config.corp_id, connected: true, status: 'connected',
      capabilities: [
        { name: 'codex', available: true },
        { name: 'workspace-isolation', available: true, source_repository: config.source.repository,
          source_base_ref: config.source.base_ref, source_base_commit: config.source.base_commit },
      ],
    }],
  }
}

function event(config, type, aggregateId, seq, payload, missionId = null, roomId = null) {
  return {
    id: id(10_000 + seq), seq, schema_version: 1, corp_id: config.corp_id,
    room_id: roomId, actor_id: type === 'run.requested' ? config.actor_id : null,
    type, aggregate_type: missionId ? 'run' : 'corp', aggregate_id: aggregateId,
    aggregate_version: 1, correlation_id: missionId, causation_id: null,
    idempotency_key: `test-event-${seq}`, visibility: roomId ? 'room' : 'corp',
    payload, created_at: '2026-09-08T12:00:00Z',
  }
}

function caseFixture(config = configuration(), name = 'suspend', firstSeq = 1) {
  const offset = name === 'suspend' ? 100 : 200
  const request = caseRequest(config, name)
  const mission = {
    id: id(offset), corp_id: config.corp_id, room_id: id(20), requested_by: config.actor_id,
    title: request.title, description: request.description, strategy: 'single',
    specification_version: 1, max_nodes: 1, max_depth: 0,
    budget_tokens: CASES[name], original_budget_tokens: CASES[name],
    budget_cost_microusd: 10_000_000, original_budget_cost_microusd: 10_000_000,
    status: name === 'suspend' ? 'cancelled' : 'failed',
  }
  const task = {
    id: id(offset + 1), corp_id: config.corp_id, mission_id: mission.id,
    contract_version: 1, required_adapter: 'codex', max_attempts: 2, attempt_count: 1,
    depth: 0, depends_on: [], assigned_agent_id: id(offset + 2),
    status: mission.status, verification_status: 'pending',
    contract: {
      ...request.contract,
      objective: `${request.description}\n\nTASK-SPECIFIC OBJECTIVE:\n${request.contract.objective}`,
      source_repository: config.source.repository, source_base_ref: config.source.base_ref,
      source_base_commit: config.source.base_commit,
      budget_tokens: CASES[name], budget_cost_microusd: 10_000_000,
      secret_refs: [], model: null, reasoning_effort: null, deliverable: null,
    },
    // Deliberately emulate JSONB key ordering, not Rust declaration ordering.
    verification_policy: JSON.parse(canonical(request.verification_policy)),
  }
  const agent = { id: id(offset + 2), corp_id: config.corp_id, actor_id: id(offset + 3),
    mission_id: mission.id, adapter: 'codex', status: 'idle', current_run_id: null, retired_at: null }
  const runId = id(offset + 4)
  const branch = `crony/task-${task.id.replaceAll('-', '')}/run-${runId.replaceAll('-', '')}`
  const run = {
    id: runId, corp_id: config.corp_id, task_id: task.id, agent_id: agent.id,
    runner_id: config.runner_id, workspace_run_id: runId, resumed_from_run_id: null,
    provider_session_id: id(offset + 5), source_repository: config.source.repository,
    source_base_ref: config.source.base_ref, source_base_commit: config.source.base_commit,
    workspace_path: expectedWorkspace(config, task.id, runId),
    workspace_branch: branch, workspace_base_ref: config.source.base_ref,
    workspace_base_commit: config.source.base_commit,
    workspace_fingerprint: 'b'.repeat(64), workspace_disposition: 'preserved',
    execution_mode: 'provider', model: null, reasoning_effort: null,
    budget_tokens_limit: CASES[name], budget_cost_microusd_limit: 10_000_000,
    input_tokens: 6_000, output_tokens: 0, cost_microusd: 0,
    breaker_stage: name, status: mission.status, verification_status: 'pending',
    artifact_id: null, artifact_uri: null, artifact_sha256: null, artifact_signature: null,
    artifact_media_type: null, verification_sha256: null, verification_summary: null,
    deliverable_sha256: null, summary: SENTINEL,
  }
  const proof = {
    schema_version: 1, corp_id: config.corp_id, mission_id: mission.id,
    task_id: task.id, run_id: run.id, workspace_run_id: run.id,
    agent_id: agent.id, runner_id: config.runner_id,
    source_repository: config.source.repository, source_base_ref: config.source.base_ref,
    source_base_commit: config.source.base_commit, workspace_base_commit: config.source.base_commit,
    branch, head_commit: config.source.base_commit, workspace_fingerprint: run.workspace_fingerprint,
    ...policyDigests(task.verification_policy, ['base.txt'], null),
  }
  const commonWorkspace = { workspace: run.workspace_path, workspace_branch: branch,
    workspace_base_ref: config.source.base_ref, workspace_base_commit: config.source.base_commit }
  const commandId = id(offset + 6)
  const breakerInput = { metric: 'run_tokens', used: 6_000, limit: CASES[name] }
  const entries = [
    ['run.requested', { mission_launch: true, task_id: task.id, agent_id: agent.id,
      runner_id: config.runner_id, attempt: 1, max_attempts: 2 }],
    ['run.session', { session_id: run.provider_session_id }],
    ['run.started', { ...commonWorkspace, adapter: 'codex', mission_id: mission.id,
      task_id: task.id, room_id: mission.room_id }],
    ['run.output', { text: SENTINEL }],
    ['run.usage', { input_tokens: 3_000, output_tokens: 0, cost_microusd: 0 }],
    ['run.usage', { input_tokens: 3_000, output_tokens: 0, cost_microusd: 0 }],
    ['run.breaker_transition', { stage: name, command_id: commandId, input: breakerInput }],
    ['runner.command_acknowledged', { command_id: commandId, runner_id: config.runner_id,
      command_kind: 'circuit_breaker', message_id: null }],
    ['run.session_terminated', { adapter: 'codex', provider_process_alive: false, outcome: run.status }],
    ['run.workspace_preserved', { ...commonWorkspace, source_checkpoint: proof,
      workspace_fingerprint: proof.workspace_fingerprint, head_commit: proof.head_commit,
      branch_deleted: false, detail: SENTINEL }],
    [`run.${run.status}`, { reason: SENTINEL }],
  ]
  const events = entries.map(([type, payload], index) =>
    event(config, type, run.id, firstSeq + index, payload, mission.id, mission.room_id))
  const incident = { id: id(offset + 7), corp_id: config.corp_id, mission_id: mission.id,
    task_id: task.id, run_id: run.id, stage: name, input: breakerInput, reason: SENTINEL }
  const state = emptyState(config)
  Object.assign(state.snapshot, {
    missions: [mission], tasks: [task], runs: [run], agents: [agent],
    circuit_breaker_incidents: [incident],
    // Bounded snapshot events intentionally omit all important early evidence.
    events: events.slice(-2),
  })
  const checkpoint = { ...newReport(config).cases[name], create_attempted: true, launch_attempted: true,
    mission_id: mission.id, task_id: task.id, agent_id: agent.id, run_id: run.id,
    contract_sha256: digest(task.contract), phase: 'launched' }
  return { state, mission, task, agent, run, events, proof, incident, checkpoint,
    replay: { events, through: events.at(-1).seq } }
}

function withOldHistory(config) {
  const state = emptyState(config)
  state.snapshot.missions.push({ id: id(40), corp_id: config.corp_id, status: 'failed', description: SENTINEL })
  state.snapshot.tasks.push({ id: id(41), corp_id: config.corp_id, mission_id: id(40), status: 'failed' })
  state.snapshot.runs.push({ id: id(42), corp_id: config.corp_id, task_id: id(41), status: 'failed', summary: SENTINEL })
  state.snapshot.verification_requests.push({ run_id: id(42), corp_id: config.corp_id, task_id: id(41), status: 'rejected' })
  const events = [event(config, 'corp.demo_bootstrapped', config.corp_id, 1, { text: SENTINEL })]
  return { state, events }
}

function memoryHarness(config, options = {}) {
  const { state, events } = withOldHistory(config)
  const saves = []
  const posts = []
  let clock = Date.parse('2026-09-08T12:00:00Z')
  let createFailureUsed = false
  let launchFailureUsed = false
  const lastSave = () => saves.at(-1)
  const io = {
    now: () => clock,
    sleep: async (ms) => {
      clock += ms
      options.onSleep?.(state, events, clock)
    },
    save: async (report) => {
      options.onSave?.(report)
      saves.push(clone(report))
    },
    snapshot: async () => {
      options.onSnapshot?.(state)
      return clone(state)
    },
    replay: async () => ({ events: clone(events), through: events.at(-1)?.seq ?? 0 }),
    readBase: async (checkpoint) => ({
      path: path.join(expectedWorkspace(config, checkpoint.task_id, checkpoint.run_id), 'base.txt'),
      bytes: 5, sha256: BASE_SHA256,
    }),
    create: async (name) => {
      assert.equal(lastSave().cases[name].create_attempted, true, 'POST must follow persisted intent')
      assert.equal(lastSave().cases[name].phase, 'create_intent')
      posts.push({ operation: 'create', name })
      if (options.createUnknown && !createFailureUsed) {
        createFailureUsed = true
        throw new Error(SENTINEL)
      }
      const fixture = caseFixture(config, name)
      fixture.mission.status = 'ready'
      fixture.task.status = 'ready'
      fixture.task.attempt_count = 0
      for (const [table, row] of [['missions', fixture.mission], ['tasks', fixture.task], ['agents', fixture.agent]]) {
        state.snapshot[table].push(row)
      }
      events.push(event(config, 'mission.created', fixture.mission.id, events.at(-1).seq + 1, {}))
      if (options.createLost && !createFailureUsed) {
        createFailureUsed = true
        throw new Error(SENTINEL)
      }
      return { mission_id: fixture.mission.id, task_id: fixture.task.id, task_ids: [fixture.task.id], strategy: 'single' }
    },
    launch: async (missionId) => {
      const name = Object.keys(CASES).find((candidate) => caseFixture(config, candidate).mission.id === missionId)
      assert.equal(lastSave().cases[name].launch_attempted, true, 'launch POST must follow persisted intent')
      assert.equal(lastSave().cases[name].phase, 'launch_intent')
      posts.push({ operation: 'launch', name })
      if (options.launchUnknown && !launchFailureUsed) {
        launchFailureUsed = true
        throw new Error(SENTINEL)
      }
      const fixture = caseFixture(config, name, events.at(-1).seq + 1)
      for (const [table, row] of [['missions', fixture.mission], ['tasks', fixture.task], ['agents', fixture.agent]]) {
        Object.assign(state.snapshot[table].find((item) => item.id === row.id), row)
      }
      if (options.live && name === 'suspend') {
        fixture.run.status = 'running'
        fixture.run.breaker_stage = 'none'
        fixture.run.workspace_disposition = 'active'
        fixture.run.input_tokens = 0
        state.snapshot.missions.find((item) => item.id === missionId).status = 'running'
        state.snapshot.tasks.find((item) => item.id === fixture.task.id).status = 'running'
      }
      state.snapshot.runs.push(fixture.run)
      state.snapshot.circuit_breaker_incidents.push(fixture.incident)
      events.push(...fixture.events)
      options.afterLaunch?.(fixture, state, events)
      if (options.launchLost && !launchFailureUsed) {
        launchFailureUsed = true
        throw new Error(SENTINEL)
      }
      return { run_id: fixture.run.id, run_ids: [fixture.run.id], runner_id: config.runner_id,
        runner_ids: [config.runner_id], replayed: false }
    },
  }
  return { io, state, events, saves, posts, lastSave }
}

test('CLI is explicit, non-ambient, duplicate/unknown flags fail', () => {
  const receipt = path.join(ROOT, 'runtime.json')
  assert.deepEqual(parseArgs(['--receipt', receipt, '--continue']), { receiptPath: receipt, continuation: true })
  assert.deepEqual(parseArgs(['--schema']), { schema: true })
  for (const args of [[], ['--continue'], ['--receipt', 'relative.json'],
    ['--receipt', receipt, '--receipt', receipt], ['--receipt', receipt, '--reset'],
    ['--receipt', receipt, '--continue', '--continue'],
    ['--receipt', receipt, '--source-repository', '--fixture']]) {
    assert.throws(() => parseArgs(args))
  }
})

test('explicit parent runtime metadata aligns without opening any named process/log/source', () => {
  const config = configuration()
  const workspace = path.join(ROOT, 'code')
  const metadata = {
    schema_version: 1, test_owned: true, issue: 190, owner_task: config.receipt_id,
    phase: 'running', server_url: config.server_url, corp_id: config.corp_id, actor_id: config.actor_id,
    runner_id: config.runner_id, runner_root: config.runner_root, workspace,
    source_commit: '7e541e738321da06b61f2ebb06297d4c4399c17f',
    source_repository_path: path.join(ROOT, 'source'),
    source_base_ref: config.source.base_ref, source_base_commit: config.source.base_commit,
    processes: Object.fromEntries(['server', 'runner'].map((role, index) => [role, {
      role, pid: 50_000 + index, executable: path.join(ROOT, `${role}.exe`), workspace,
      started_utc: '2026-09-08T11:45:15.4217382Z', stdout: SENTINEL, stderr: SENTINEL,
    }])),
    observation_failures: [{ cause: SENTINEL }],
    role_lease_renewal: { unused_metadata: SENTINEL },
  }
  const options = { sourceRepository: config.source.repository, fixture: FIXTURE, fixtureSha256: FIXTURE_SHA256,
    outputDir: config.output_dir, reportPath: config.report_path }
  const result = configurationFromReceipt(metadata, options)
  assert.equal(result.receipt_id, config.receipt_id)
  assert.equal(result.server_url, config.server_url)
  assert.deepEqual(result.source, config.source)
  assert.match(result.runtime_binding_sha256, /^[0-9a-f]{64}$/u)
  assert.ok(!JSON.stringify(result).includes(SENTINEL))
  assert.throws(() => configurationFromReceipt(metadata, { ...options, fixture: undefined }),
    { code: 'explicit_native_fixture_attestation_required' })
  assert.throws(() => configurationFromReceipt(metadata, { ...options, fixtureSha256: undefined }),
    { code: 'explicit_current_fixture_digest_required' })
  assert.throws(() => configurationFromReceipt({ ...metadata, phase: 'starting' }, options))
  assert.throws(() => configurationFromReceipt(metadata, { ...options, sourceRepository: undefined }))
  const changed = clone(metadata)
  changed.processes.runner.pid += 1
  assert.notEqual(receiptIdentity(result), receiptIdentity(configurationFromReceipt(changed, options)))
})

for (const port of PROTECTED_PORTS) {
  test(`rejects protected/manual API port ${port}`, () => {
    assert.throws(() => configuration({ server_url: `http://127.0.0.1:${port}` }),
      { code: 'protected_or_ambiguous_endpoint' })
  })
}
for (const server of ['http://localhost:18574', 'http://127.1:18574',
  'http://[::1]:18574', 'https://127.0.0.1:18574', 'http://127.0.0.1:18574/',
  'http://127.0.0.1:18574/api', 'http://127.0.0.1:18574?x=1',
  'http://secret:secret@127.0.0.1:18574', 'http://example.com:18574']) {
  test(`rejects ambiguous or non-owned origin ${server.replace(/secret/gu, 'redacted')}`, () => {
    assert.throws(() => configuration({ server_url: server }), { code: 'protected_or_ambiguous_endpoint' })
  })
}

test('receipt requires complete immutable source, no unknown authority or provider substitution', () => {
  for (const overrides of [
    { test_owned: false }, { auth_mode: 'production' }, { runner_id: 'runner-local' },
    { source: { repository: 'owner/repo', base_ref: 'HEAD' } },
    { source: { repository: 'C:\\credentials', base_ref: 'HEAD', base_commit: 'a'.repeat(40) } },
    { source: { repository: 'owner/repo', base_ref: '../main', base_commit: 'a'.repeat(40) } },
    { source: { repository: 'owner/repo', base_ref: 'HEAD', base_commit: 'a8894b5' } },
    { provider: { adapter: 'fake-process', fixture: FIXTURE, sha256: FIXTURE_SHA256 } },
    { provider: { adapter: 'codex', fixture: FIXTURE, sha256: 'not-a-digest' } },
    { access_token: SENTINEL }, { policy_override: true }, { timeout_ms: 900_000 },
    { settle_ms: 100 }, { report_path: path.join(ROOT, 'outside.json') },
    { output_dir: path.join(ROOT, 'runner-workspaces', 'worktrees'),
      report_path: path.join(ROOT, 'runner-workspaces', 'worktrees', 'report.json') },
  ]) assert.throws(() => configuration(overrides))
  assert.equal(configuration({ source: { repository: 'local/test-abc', base_ref: 'HEAD', base_commit: 'c'.repeat(64) } })
    .source.base_commit.length, 64)
  const currentFixture = configuration({ provider: { adapter: 'codex', fixture: FIXTURE, sha256: 'a'.repeat(64) } })
  assert.notEqual(receiptIdentity(currentFixture), receiptIdentity(configuration()),
    'parent fixture extensions are allowed but cannot silently replace an existing report identity')
})

test('native requests preserve single strategy, exact source, budget, File verifier, no deliverable', () => {
  const config = configuration()
  for (const [name, tokens] of Object.entries(CASES)) {
    const request = caseRequest(config, name)
    assert.equal(request.budget_tokens, tokens)
    assert.equal(request.strategy, 'single')
    assert.equal(request.preferred_adapter, 'codex')
    assert.deepEqual(request.source, config.source)
    assert.deepEqual(request.contract.write_scope, ['base.txt'])
    assert.deepEqual(request.verification_policy.checks, [{ type: 'file', path: 'base.txt', min_bytes: 5 }])
    assert.deepEqual(request.verification_policy.manual_gate, { type: 'human_approval', roles: ['owner', 'admin'] })
    assert.equal(request.deliverable, null)
    assert.equal(request.max_attempts, undefined, 'must not invent a create API override')
    assert.equal(request.idempotency_key, undefined, 'create API has no idempotency field')
  }
})

test('Rust policy digest uses explicit declaration order, not JS/JSONB property order', () => {
  const policy = { manual_gate: { roles: ['owner', 'admin'], type: 'human_approval' },
    checks: [{ min_bytes: 5, path: 'base.txt', type: 'file' }] }
  const rust = '{"checks":[{"type":"file","path":"base.txt","min_bytes":5}],"manual_gate":{"type":"human_approval","roles":["owner","admin"]}}'
  assert.equal(rustVerificationJson(policy), rust)
  const digests = policyDigests(policy, ['base.txt'], null)
  assert.equal(digests.verification_policy_sha256, createHash('sha256').update(rust).digest('hex'))
  assert.notEqual(digests.verification_policy_sha256, digest(policy))
  assert.equal(digests.deliverable_policy_sha256, createHash('sha256').update('null').digest('hex'))
  assert.equal(digests.write_scope_sha256, createHash('sha256').update('["base.txt"]').digest('hex'))
  assert.throws(() => rustVerificationJson({ ...policy, bypass: true }))
  assert.throws(() => rustVerificationJson({ ...policy, checks: [{ type: 'artifact', min_bytes: 5 }] }))
  assert.throws(() => policyDigests(policy, ['**'], null))
  assert.throws(() => policyDigests(policy, ['base.txt'], { form: 'patch' }))
})

test('canonical containment handles Windows drive prefixes but rejects aliases/escapes', () => {
  const win = path.win32
  const root = 'C:\\owned\\runner'
  assert.equal(containedPath(root, '\\\\?\\C:\\owned\\runner\\worktrees\\base.txt', win),
    'C:\\owned\\runner\\worktrees\\base.txt')
  assert.equal(containedPath(root, 'c:\\OWNED\\runner\\worktrees\\base.txt', win),
    'c:\\OWNED\\runner\\worktrees\\base.txt')
  for (const candidate of [
    'C:\\owned\\runner-escape\\base.txt', root, 'C:\\owned\\runner\\..\\base.txt',
    'C:\\owned\\runner\\.. \\base.txt', 'C:\\owned\\runner\\folder.\\base.txt',
    'D:\\owned\\runner\\base.txt', 'C:base.txt', '\\\\server\\share\\base.txt',
    '\\\\?\\UNC\\server\\share\\base.txt', '\\\\.\\C:\\owned\\runner\\base.txt',
    '\\\\?\\GLOBALROOT\\Device\\x', 'C:\\owned\\runner\\base.txt:stream',
    'C:\\owned\\runner\\NUL.txt', 'C:\\owned\\runner\\folder\\CON',
  ]) assert.throws(() => containedPath(root, candidate, win))
  assert.throws(() => localAbsolute('C:\\', win))
})

test('POSIX containment rejects traversal, equality and sibling prefixes', () => {
  const posix = path.posix
  assert.equal(containedPath('/owned/runner', '/owned/runner/task/base.txt', posix), '/owned/runner/task/base.txt')
  for (const candidate of ['/owned/runner', '/owned/runner-other/base.txt',
    '/owned/runner/../base.txt', '/etc/passwd', 'relative', '//server/share']) {
    assert.throws(() => containedPath('/owned/runner', candidate, posix))
  }
})

function memoryFiles(config, checkpoint, { bytes = 'base\n', link = false, hardlink = false, escape = false } = {}) {
  const workspace = expectedWorkspace(config, checkpoint.task_id, checkpoint.run_id)
  const file = path.join(workspace, 'base.txt')
  const all = [config.runner_root, path.join(config.runner_root, 'worktrees'),
    path.dirname(workspace), workspace, file]
  let opened = 0
  const info = (target) => {
    assert.ok(all.includes(target), 'test file access must stay in declared paths')
    return {
      isDirectory: () => target !== file, isFile: () => target === file,
      isSymbolicLink: () => link && target === path.dirname(workspace),
      nlink: hardlink && target === file ? 2 : 1,
      size: target === file ? Buffer.byteLength(bytes) : 0, dev: 1, ino: all.indexOf(target) + 1, mtimeMs: 1,
    }
  }
  return {
    file, opened: () => opened,
    fs: {
      lstat: async (target) => info(target),
      realpath: async (target) => escape && target === path.dirname(workspace)
        ? path.join(ROOT, 'outside') : target,
      open: async (target) => {
        assert.equal(target, file)
        opened += 1
        const buffer = Buffer.from(bytes)
        return {
          stat: async () => info(target),
          read: async (destination, offset, length, position) => ({
            bytesRead: buffer.copy(destination, offset, position, Math.min(buffer.length, position + length)),
          }),
          close: async () => {},
        }
      },
    },
  }
}

test('base.txt reads exactly five bytes, and all link/reparse/hardlink failures occur before open', async () => {
  const config = configuration()
  const { checkpoint } = caseFixture(config)
  const good = memoryFiles(config, checkpoint)
  assert.deepEqual(await readBaseFile(config, checkpoint, good.fs), { path: good.file, bytes: 5, sha256: BASE_SHA256 })
  assert.equal(good.opened(), 1)
  for (const flags of [{ link: true }, { hardlink: true }, { escape: true }]) {
    const bad = memoryFiles(config, checkpoint, flags)
    await assert.rejects(readBaseFile(config, checkpoint, bad.fs))
    assert.equal(bad.opened(), 0)
  }
  for (const bytes of ['base\r\n', 'base', 'BASE\n', 'base\nextra']) {
    const bad = memoryFiles(config, checkpoint, { bytes })
    await assert.rejects(readBaseFile(config, checkpoint, bad.fs))
  }
})

test('opened-file identity swaps fail closed without reading bytes', async () => {
  const config = configuration()
  const { checkpoint } = caseFixture(config)
  const files = memoryFiles(config, checkpoint)
  const open = files.fs.open
  files.fs.open = async (...args) => {
    const handle = await open(...args)
    const stat = handle.stat
    handle.stat = async () => ({ ...await stat(), ino: 999 })
    handle.read = async () => assert.fail('must not read a replaced file')
    return handle
  }
  await assert.rejects(readBaseFile(config, checkpoint, files.fs), { code: 'opened_file_identity_changed' })
})

test('runners are top-level; nested-only, unavailable, foreign, ambiguous or different-source runners fail', () => {
  const config = configuration()
  assertRunner(emptyState(config), config)
  for (const change of [
    (state) => { state.snapshot.runners = state.runners; delete state.runners },
    (state) => { state.runners[0].connected = false },
    (state) => { state.runners[0].id = 'another-runner' },
    (state) => { state.runners[0].capabilities[0].available = false },
    (state) => { state.runners[0].capabilities[1].source_base_commit = 'f'.repeat(40) },
    (state) => { state.runners.push({ ...state.runners[0], id: 'competing-runner' }) },
  ]) {
    const state = emptyState(config)
    change(state)
    assert.throws(() => assertRunner(state, config))
  }
})

for (const name of Object.keys(CASES)) {
  test(`${name}: actual usage -> breaker -> native teardown -> exact checkpoint -> terminal`, () => {
    const config = configuration()
    const fixture = caseFixture(config, name)
    const evidence = assessCase(fixture.state, fixture.replay, config, name, fixture.checkpoint)
    assert.equal(evidence.breaker_stage, name)
    assert.equal(evidence.attempt_count, 1)
    assert.equal(evidence.task_run_count, 1)
    assert.equal(evidence.verification_started, false)
    assert.equal(evidence.accepted_provider_artifacts, 0)
    assert.equal(evidence.source_checkpoint.head_commit, config.source.base_commit)
    assert.ok(!JSON.stringify(evidence).includes(SENTINEL))
    assert.ok(evidence.ordering.usage[1] < evidence.ordering.breaker)
    assert.ok(evidence.ordering.session_terminated < evidence.ordering.workspace_preserved)
    assert.ok(evidence.ordering.workspace_preserved < evidence.ordering.terminal)
  })
}

const evidenceFailures = [
  ['missing checkpoint', (f) => { delete f.events.find((e) => e.type === 'run.workspace_preserved').payload.source_checkpoint }],
  ['foreign Corp proof', (f) => { f.proof.corp_id = id(999) }],
  ['foreign mission proof', (f) => { f.proof.mission_id = id(999) }],
  ['foreign task proof', (f) => { f.proof.task_id = id(999) }],
  ['foreign run proof', (f) => { f.proof.run_id = id(999) }],
  ['foreign agent proof', (f) => { f.proof.agent_id = id(999) }],
  ['foreign runner proof', (f) => { f.proof.runner_id = 'other' }],
  ['wrong original base', (f) => { f.proof.source_base_commit = 'f'.repeat(40) }],
  ['changed HEAD', (f) => { f.proof.head_commit = 'f'.repeat(40) }],
  ['changed fingerprint', (f) => { f.proof.workspace_fingerprint = 'f'.repeat(64) }],
  ['arbitrary JS policy ordering', (f) => { f.proof.verification_policy_sha256 = digest(f.task.verification_policy) }],
  ['changed write digest', (f) => { f.proof.write_scope_sha256 = 'f'.repeat(64) }],
  ['changed deliverable digest', (f) => { f.proof.deliverable_policy_sha256 = 'f'.repeat(64) }],
  ['token in proof', (f) => { f.proof.assignment_token = SENTINEL }],
  ['live provider', (f) => { f.events.find((e) => e.type === 'run.session_terminated').payload.provider_process_alive = true }],
  ['wrong adapter', (f) => { f.events.find((e) => e.type === 'run.session_terminated').payload.adapter = 'fake-process' }],
  ['foreign event lineage', (f) => { f.events[0].correlation_id = id(999) }],
  ['late checkpoint', (f) => {
    const a = f.events.find((e) => e.type === 'run.workspace_preserved')
    const b = f.events.at(-1)
    ;[a.seq, b.seq] = [b.seq, a.seq]
    f.events.sort((left, right) => left.seq - right.seq)
  }],
  ['missing usage', (f) => { f.events.splice(f.events.findIndex((e) => e.type === 'run.usage'), 1) }],
  ['fake cumulative usage', (f) => { f.run.input_tokens = 5_999 }],
  ['wrong breaker metric', (f) => { f.events.find((e) => e.type === 'run.breaker_transition').payload.input.metric = 'corp_tokens_24h' }],
  ['wrong command acknowledgement', (f) => { f.events.find((e) => e.type === 'runner.command_acknowledged').payload.command_id = id(999) }],
  ['accepted artifact', (f) => { f.run.artifact_id = id(999) }],
  ['verification started', (f) => { f.run.verification_status = 'running' }],
  ['lost is not completion', (f) => { f.run.status = 'lost' }],
  ['agent still assigned', (f) => { f.agent.current_run_id = f.run.id }],
  ['automatic retry', (f) => { f.state.snapshot.runs.push({ ...f.run, id: id(999) }) }],
  ['replacement task', (f) => { f.state.snapshot.tasks.push({ ...f.task, id: id(999) }) }],
  ['resume lineage', (f) => { f.run.resumed_from_run_id = id(999) }],
  ['budget override', (f) => { f.task.contract.budget_tokens += 1 }],
  ['write scope widening', (f) => { f.task.contract.write_scope = ['**'] }],
  ['policy weakening', (f) => { f.task.verification_policy.manual_gate = null }],
  ['path outside runner', (f) => { f.run.workspace_path = path.join(ROOT, 'outside') }],
  ['unknown projection strings are not logged', (f) => { f.run.breaker_stage = SENTINEL }],
  ['accepted evidence row', (f) => {
    f.state.snapshot.verification_evidence.push({ id: id(999), corp_id: f.run.corp_id, run_id: f.run.id, task_id: f.task.id })
  }],
]
for (const [label, mutate] of evidenceFailures) {
  test(`rejects ${label}`, () => {
    const config = configuration()
    const fixture = caseFixture(config)
    mutate(fixture)
    assert.throws(() => assessCase(fixture.state, fixture.replay, config, 'suspend', fixture.checkpoint))
    assert.ok(!JSON.stringify(fixture.checkpoint).includes(SENTINEL))
  })
}

for (const type of ['run.completed', 'run.verification_started', 'run.verification_evidence',
  'run.artifact', 'run.deliverable', 'run.workspace_removed', 'run.teardown_uncertain']) {
  test(`full replay, not the bounded snapshot, rejects earlier ${type}`, () => {
    const config = configuration()
    const fixture = caseFixture(config)
    fixture.events.push(event(config, type, fixture.run.id, fixture.replay.through + 1, {},
      fixture.mission.id, fixture.mission.room_id))
    fixture.replay.through += 1
    fixture.state.snapshot.events = []
    assert.throws(() => assessCase(fixture.state, fixture.replay, config, 'suspend', fixture.checkpoint),
      { code: 'forbidden_accepted_progress_or_uncertain_teardown' })
  })
}

test('visible history and original journal prefix are immutable; no whole-database inference', () => {
  const config = configuration()
  const { state, events } = withOldHistory(config)
  const replay = { events, through: 1 }
  const baseline = historySummary(state, replay, config)
  assertHistory(baseline, state, replay, config)
  assert.ok(!JSON.stringify(baseline).includes(SENTINEL))
  const changed = clone(state)
  changed.snapshot.runs[0].summary = 'changed'
  assert.throws(() => assertHistory(baseline, changed, replay, config), { code: 'prior_visible_history_changed' })
  changed.snapshot.runs = []
  assert.throws(() => assertHistory(baseline, changed, replay, config), { code: 'prior_visible_row_missing_coverage_not_proven' })
  const corrupted = clone(replay)
  corrupted.events[0].payload = {}
  assert.throws(() => assertHistory(baseline, state, corrupted, config), { code: 'prior_visible_journal_prefix_changed' })
  assert.throws(() => assertHistory(baseline, state, { events: [], through: 0 }, config), { code: 'journal_watermark_regressed' })
  const busy = clone(state)
  busy.snapshot.runs[0].status = 'running'
  assert.throws(() => historySummary(busy, replay, config), { code: 'existing_live_runs_block_new_suite' })
})

test('journal bounds, strict ordering, non-contiguous visible sequences and Ready watermark', () => {
  const config = configuration()
  const a = event(config, 'corp.updated', config.corp_id, 12, {})
  const b = event(config, 'corp.updated', config.corp_id, 99, {})
  assert.equal(replaySummary([a, b], 99, config.corp_id).event_count, 2)
  assert.throws(() => replaySummary([a, b], 100, config.corp_id), { code: 'journal_ready_watermark_mismatch' })
  assert.throws(() => replaySummary([b, a], 12, config.corp_id))
  assert.throws(() => replaySummary([a, a], 12, config.corp_id))
  assert.throws(() => replaySummary([{ ...a, corp_id: id(999) }], 12, config.corp_id))
  assert.throws(() => replaySummary(Array(LIMITS.replay_events + 1).fill(a), 12, config.corp_id),
    { code: 'journal_event_bound' })
})

test('API requests allow only exact scoped snapshot/create/launch, never reset/policy/resume', async () => {
  const config = configuration()
  const calls = []
  const api = createApi(config, {
    fetchImpl: async (url, init) => { calls.push({ url, init }); return new Response('{}', { status: 200 }) },
  })
  await api.snapshot()
  await api.create('suspend')
  await api.launch(id(100))
  assert.equal(calls.length, 3)
  assert.ok(calls.every((call) => call.init.redirect === 'error'))
  assert.ok(calls.every((call) => Object.keys(call.init.headers).join() === 'content-type'))
  for (const [route, body] of [
    ['/api/demo/reset', {}], ['/api/demo/bootstrap', {}],
    [`/api/corps/${config.corp_id}/budget-policy`, {}],
    [`/api/corps/${config.corp_id}/runs/${id(100)}/resume`, {}],
    [`/api/corps/${id(999)}/snapshot?actor_id=${config.actor_id}`, undefined],
    [`/api/corps/${config.corp_id}/runners/enroll`, {}],
  ]) await assert.rejects(api.request(route, body), { code: 'api_route_outside_additive_allowlist' })
  assert.equal(calls.length, 3)
})

test('HTTP errors with sensitive bodies are not retried or exposed', async () => {
  let calls = 0
  const api = createApi(configuration(), {
    fetchImpl: async () => { calls += 1; return new Response(SENTINEL, { status: 503 }) },
  })
  await assert.rejects(api.create('suspend'), (error) => {
    assert.equal(error.code, 'api_response_not_ok_body_withheld')
    assert.ok(!String(error).includes(SENTINEL))
    return true
  })
  assert.equal(calls, 1)
})

test('oversized API responses fail at the byte bound, with no response body in the error', async () => {
  const api = createApi(configuration(), {
    fetchImpl: async () => new Response('x'.repeat(LIMITS.response_bytes + 1), { status: 200 }),
  })
  await assert.rejects(api.snapshot(), { code: 'api_response_byte_bound' })
})

function socketClass(frames, { earlyClose = false } = {}) {
  return class {
    constructor(url) {
      assert.match(url, /^ws:\/\/127\.0\.0\.1:18574\/ws\/corps\/[^?]+\?actor_id=[^&]+&after_seq=0$/u)
      queueMicrotask(() => {
        for (const frame of frames) this.onmessage?.({ data: JSON.stringify(frame) })
        if (earlyClose) this.onclose?.()
      })
    }
    close() {}
  }
}
test('native journal API requires a real Ready watermark and refuses partial replay', async () => {
  const config = configuration()
  const e = event(config, 'corp.updated', config.corp_id, 2, {})
  const api = createApi(config, { WebSocketImpl: socketClass([
    { type: 'event', event: e }, { type: 'ready', corp_id: config.corp_id, replayed_through: 2 },
  ]) })
  assert.deepEqual(await api.replay(), { events: [e], through: 2 })
  await assert.rejects(createApi(config, { WebSocketImpl: socketClass([
    { type: 'event', event: e },
  ], { earlyClose: true }) }).replay(), { code: 'journal_closed_before_ready' })
  await assert.rejects(createApi(config, { WebSocketImpl: socketClass([
    { type: 'event', event: e }, { type: 'ready', corp_id: config.corp_id, replayed_through: 3 },
  ]) }).replay(), { code: 'journal_ready_watermark_mismatch' })
})

test('two additive cases checkpoint intent/IDs, retain old history, and pass only after quiet observation', async () => {
  const config = configuration()
  const harness = memoryHarness(config)
  const report = newReport(config, harness.io.now())
  await executeSuite(config, report, harness.io)
  assert.equal(report.passed, true)
  assert.deepEqual(harness.posts.map(({ operation, name }) => `${operation}:${name}`),
    ['create:suspend', 'launch:suspend', 'create:stop', 'launch:stop'])
  assert.equal(harness.state.snapshot.missions.length, 3)
  assert.equal(harness.state.snapshot.runs.length, 3)
  assert.equal(harness.state.snapshot.runs[0].summary, SENTINEL)
  assert.ok(harness.saves.length > 12)
  assert.ok(!JSON.stringify(harness.saves).includes(SENTINEL))
  assert.ok(report.cases.suspend.evidence.quiet_observation_ms >= config.settle_ms)
  assert.equal(report.cases.stop.evidence.task_run_count, 1)
})

test('baseline counts are dynamic, including completed prior browser-created missions', async () => {
  const config = configuration()
  const harness = memoryHarness(config)
  for (let index = 0; index < 3; index += 1) {
    const missionId = id(500 + index * 3)
    const taskId = id(501 + index * 3)
    const runId = id(502 + index * 3)
    harness.state.snapshot.missions.push({ id: missionId, corp_id: config.corp_id,
      status: 'failed', title: '[budget-stream-ui] parent-owned browser case' })
    harness.state.snapshot.tasks.push({ id: taskId, corp_id: config.corp_id,
      mission_id: missionId, status: 'failed' })
    harness.state.snapshot.runs.push({ id: runId, corp_id: config.corp_id,
      task_id: taskId, status: 'failed', input_tokens: 600_000, summary: SENTINEL })
    harness.events.push(event(config, 'run.failed', runId, harness.events.at(-1).seq + 1, {}))
  }
  const report = newReport(config, harness.io.now())
  await executeSuite(config, report, harness.io)
  assert.equal(report.baseline.rows.missions.length, 4)
  assert.equal(report.baseline.rows.runs.length, 4)
  assert.equal(harness.state.snapshot.missions.length, 6)
  assert.equal(harness.state.snapshot.runs.length, 6)
  assert.equal(report.cases.suspend.evidence.persisted_usage.input_tokens, 6_000)
  assert.equal(report.cases.stop.evidence.persisted_usage.input_tokens, 6_000)
  assert.equal(report.passed, true)
})

test('completed checkpoint continuation is read-only and never re-launches either case', async () => {
  const config = configuration()
  const harness = memoryHarness(config)
  const report = newReport(config, harness.io.now())
  await executeSuite(config, report, harness.io)
  const runIds = Object.values(report.cases).map((item) => item.run_id)
  const continued = validateSavedReport(clone(report), config)
  await executeSuite(config, continued, harness.io)
  assert.equal(harness.posts.length, 4)
  assert.deepEqual(Object.values(continued.cases).map((item) => item.run_id), runIds)
  assert.equal(continued.passed, true)
})

test('timeout leaves a live run LIVE with its original ID and never starts a replacement/second case', async () => {
  const config = configuration({ timeout_ms: 10_000 })
  const harness = memoryHarness(config, { live: true })
  const report = newReport(config, harness.io.now())
  await assert.rejects(executeSuite(config, report, harness.io), { code: 'observation_timeout_run_not_reclassified' })
  assert.equal(harness.lastSave().status, 'incomplete_live')
  assert.equal(harness.lastSave().cases.suspend.latest.status, 'running')
  assert.equal(harness.lastSave().cases.suspend.run_id, caseFixture(config).run.id)
  assert.equal(harness.lastSave().cases.stop.create_attempted, false)
  const continued = validateSavedReport(clone(report), config)
  await assert.rejects(executeSuite(config, continued, harness.io))
  assert.equal(harness.posts.length, 2)
  assert.equal(continued.cases.suspend.latest.status, 'running')
})

for (const operation of ['create', 'launch']) {
  test(`lost ${operation} response discovers exact original IDs; intentional continuation sends no duplicate`, async () => {
    const config = configuration()
    const harness = memoryHarness(config, { [`${operation}Lost`]: true })
    const report = newReport(config, harness.io.now())
    await assert.rejects(executeSuite(config, report, harness.io), { code: 'operation_failed_details_withheld' })
    assert.equal(report.cases.suspend.mission_id, caseFixture(config).mission.id)
    assert.equal(report.cases.suspend.task_id, caseFixture(config).task.id)
    if (operation === 'launch') assert.equal(report.cases.suspend.run_id, caseFixture(config).run.id)
    assert.ok(!JSON.stringify(harness.saves).includes(SENTINEL))
    const continued = validateSavedReport(clone(report), config)
    await executeSuite(config, continued, harness.io)
    assert.equal(harness.posts.filter((post) => post.operation === operation && post.name === 'suspend').length, 1)
    assert.equal(continued.passed, true)
  })
  test(`uncertain ${operation} without an observable result blocks continuation, never replacement`, async () => {
    const config = configuration()
    const harness = memoryHarness(config, { [`${operation}Unknown`]: true })
    const report = newReport(config, harness.io.now())
    await assert.rejects(executeSuite(config, report, harness.io))
    const continued = validateSavedReport(clone(report), config)
    await assert.rejects(executeSuite(config, continued, harness.io), {
      code: operation === 'create' ? 'create_intent_ambiguous_no_replacement' : 'launch_intent_ambiguous_do_not_relaunch',
    })
    assert.equal(harness.posts.filter((post) => post.operation === operation).length, 1)
    assert.equal(continued.cases.stop.create_attempted, false)
  })
}

test('driver failure keeps IDs and failure report; late accepted verification cannot pass', async () => {
  const config = configuration()
  const harness = memoryHarness(config, {
    afterLaunch: (fixture, _state, events) => {
      events.push(event(config, 'run.verification_started', fixture.run.id, events.at(-1).seq + 1, {},
        fixture.mission.id, fixture.mission.room_id))
    },
  })
  const report = newReport(config, harness.io.now())
  await assert.rejects(executeSuite(config, report, harness.io),
    { code: 'forbidden_accepted_progress_or_uncertain_teardown' })
  assert.equal(harness.lastSave().passed, false)
  assert.equal(harness.lastSave().cases.suspend.run_id, caseFixture(config).run.id)
  assert.equal(harness.lastSave().cases.stop.create_attempted, false)
  assert.ok(harness.lastSave().cases.suspend.journal_observation.events
    .some((item) => item.type === 'run.verification_started'))
  assert.ok(!JSON.stringify(harness.lastSave()).includes(SENTINEL))
})

test('saved report cannot change receipt/source/runner/process scope or invent successful authority', () => {
  const config = configuration()
  const report = newReport(config)
  const changed = configuration({ source: { ...config.source, base_commit: 'c'.repeat(40) } })
  assert.throws(() => validateSavedReport(clone(report), changed), { code: 'continuation_receipt_mismatch' })
  const unsafe = clone(report)
  unsafe.cases.suspend.assignment_token = SENTINEL
  assert.throws(() => validateSavedReport(unsafe, config), { code: 'invalid_saved_case_fields' })
  const scope = clone(report)
  scope.cases.suspend.run_id = id(100)
  assert.throws(() => validateSavedReport(scope, config), { code: 'saved_run_without_launch_intent' })
  const timingsOnly = configuration({ timeout_ms: 30_000 })
  assert.equal(validateSavedReport(clone(report), timingsOnly).receipt_sha256, report.receipt_sha256)
})
