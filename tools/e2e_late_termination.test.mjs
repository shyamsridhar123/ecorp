// PURE driver tests: clocks, API responses, journals, files and reports are all
// in memory. No runtime receipt, service, database, environment, credentials,
// browser, real provider or source checkout is accessed by this test module.
import assert from 'node:assert/strict'
import path from 'node:path'
import test from 'node:test'
import {
  BASE_SHA256, FIXTURE, LIMITS, canonical, digest, expectedWorkspace, parseArgs, receiptIdentity,
} from './e2e_stopped_source_checkpoint.mjs'
import {
  MARKER, SOURCE_COMMIT, SUITE, assessCase, caseContext, createProbeApi, executeProbe,
  newReport, observeJournal, openReport, probeConfiguration, probeRequest, readBounded, validateSavedReport,
} from './e2e_late_termination.mjs'

const id = number => `00000000-0000-4000-8000-${number.toString(16).padStart(12, '0')}`
const clone = value => structuredClone(value)
const ROOT = path.sep === '\\' ? 'C:\\owned-qa' : '/owned-qa'
const PRIVATE = 'PRIVATE_PROVIDER_CONTENT_OR_ASSIGNMENT_TOKEN_MUST_NOT_BE_LOGGED'
const FAILURE = `artifact upload rejected: artifact upload blocked by circuit breaker stop; ${PRIVATE}`

function configuration(overrides = {}) {
  return probeConfiguration({
    schema_version: 1, test_owned: true, auth_mode: 'development', receipt_id: id(900),
    server_url: 'http://127.0.0.1:18574', corp_id: id(1), actor_id: id(17), runner_id: 'issue174-local-start',
    source: { repository: 'shyamsridhar123/ecorp-enterprise-lab', base_ref: 'HEAD', base_commit: SOURCE_COMMIT },
    runner_root: path.join(ROOT, 'runner-workspaces'), output_dir: path.join(ROOT, 'evidence'),
    report_path: path.join(ROOT, 'evidence', 'e2e-late-termination.json'),
    provider: { adapter: 'codex', fixture: FIXTURE, sha256: '4'.repeat(64) },
    timeout_ms: 10_000, poll_ms: 250, settle_ms: 2_000, ...overrides,
  })
}

function emptyState(config) {
  const tables = ['missions', 'tasks', 'runs', 'actors', 'rooms', 'agents', 'verification_evidence',
    'verification_requests', 'source_deliverables', 'mission_contract_revisions', 'mission_budget_revisions',
    'action_approvals', 'circuit_breaker_incidents', 'events']
  return {
    snapshot: {
      ...Object.fromEntries(tables.map(table => [table, []])), corp: { id: config.corp_id },
      actors: [{ id: config.actor_id, corp_id: config.corp_id, kind: 'human', role: 'owner' }],
      rooms: [{ id: id(20), corp_id: config.corp_id }],
    },
    runners: [{ id: config.runner_id, corp_id: config.corp_id, connected: true, status: 'connected',
      capabilities: [{ name: 'codex', available: true }, { name: 'workspace-isolation', available: true,
        source_repository: config.source.repository, source_base_ref: config.source.base_ref,
        source_base_commit: config.source.base_commit }] }],
  }
}

function event(config, type, seq, payload, runId = id(104), missionId = id(100)) {
  return {
    id: id(10_000 + seq), seq, schema_version: 1, corp_id: config.corp_id,
    type, aggregate_type: missionId ? 'run' : 'corp', aggregate_id: runId,
    aggregate_version: 1, actor_id: type === 'run.requested' ? config.actor_id : null,
    room_id: missionId ? id(20) : null, correlation_id: missionId, causation_id: null,
    idempotency_key: `runner:${config.runner_id}:event:${id(10_000 + seq)}`,
    visibility: missionId ? 'room' : 'corp', payload, created_at: '2026-09-08T12:00:00Z',
  }
}

function fixture(config = configuration(), { firstSeq = 1, ackPosition = 'after', omit = [] } = {}) {
  const request = probeRequest(config)
  const mission = {
    id: id(100), corp_id: config.corp_id, room_id: id(20), requested_by: config.actor_id,
    title: request.title, description: request.description, strategy: 'single', specification_version: 1,
    status: 'failed', budget_tokens: 5_000, original_budget_tokens: 5_000,
    budget_cost_microusd: 10_000_000, original_budget_cost_microusd: 10_000_000,
  }
  const task = {
    id: id(101), corp_id: config.corp_id, mission_id: mission.id, assigned_agent_id: id(102),
    required_adapter: 'codex', depth: 0, depends_on: [], max_attempts: 2, attempt_count: 1,
    contract_version: 1, status: 'failed', verification_status: 'pending',
    contract: {
      ...request.contract, objective: `${request.description}\n\nTASK-SPECIFIC OBJECTIVE:\n${request.contract.objective}`,
      source_repository: config.source.repository, source_base_ref: config.source.base_ref,
      source_base_commit: config.source.base_commit, budget_tokens: 5_000, budget_cost_microusd: 10_000_000,
      secret_refs: [], model: null, reasoning_effort: null, deliverable: null,
    },
    verification_policy: JSON.parse(canonical(request.verification_policy)),
  }
  const agent = { id: id(102), corp_id: config.corp_id, actor_id: id(103), mission_id: mission.id,
    adapter: 'codex', status: 'idle', station: null, current_run_id: null, retired_at: null }
  const branch = `crony/task-${task.id.replaceAll('-', '')}/run-${id(104).replaceAll('-', '')}`
  const run = {
    id: id(104), corp_id: config.corp_id, task_id: task.id, agent_id: agent.id, runner_id: config.runner_id,
    workspace_run_id: id(104), resumed_from_run_id: null, provider_session_id: id(105),
    execution_mode: 'provider', source_repository: config.source.repository,
    source_base_ref: config.source.base_ref, source_base_commit: config.source.base_commit,
    model: null, reasoning_effort: null, budget_tokens_limit: 5_000, budget_cost_microusd_limit: 10_000_000,
    input_tokens: 6_000, output_tokens: 0, cost_microusd: 0, breaker_stage: 'stop', status: 'failed',
    workspace_path: expectedWorkspace(config, task.id, id(104)), workspace_branch: branch,
    workspace_base_ref: config.source.base_ref, workspace_base_commit: config.source.base_commit,
    workspace_disposition: 'preserved', workspace_detail: PRIVATE, workspace_fingerprint: 'b'.repeat(64),
    verification_status: 'pending', artifact_id: null, artifact_uri: null, artifact_sha256: null,
    artifact_signature: null, artifact_media_type: null, verification_sha256: null, verification_summary: null,
    deliverable_sha256: null, summary: FAILURE,
  }
  const workspace = { workspace: run.workspace_path, workspace_branch: branch,
    workspace_base_ref: config.source.base_ref, workspace_base_commit: config.source.base_commit }
  const breakerInput = { metric: 'run_tokens', used: 6_000, limit: 5_000 }
  const ack = ['runner.command_acknowledged', { command_id: id(106), runner_id: config.runner_id,
    command_kind: 'circuit_breaker', message_id: null }]
  const entries = [
    ['run.requested', { mission_launch: true, task_id: task.id, agent_id: agent.id,
      runner_id: config.runner_id, attempt: 1, max_attempts: 2 }],
    ['run.session', { session_id: run.provider_session_id }],
    ['run.started', { ...workspace, adapter: 'codex', mission_id: mission.id, task_id: task.id, room_id: mission.room_id }],
    ['run.output', { text: PRIVATE }],
    ['run.usage', { input_tokens: 3_000, output_tokens: 0, cost_microusd: 0 }],
    ['run.usage', { input_tokens: 3_000, output_tokens: 0, cost_microusd: 0 }],
    ['run.breaker_transition', { stage: 'stop', command_id: id(106), input: breakerInput }],
    ...(ackPosition === 'before' ? [ack] : []),
    ['run.failed', { error: FAILURE }],
    ...(ackPosition === 'between' ? [ack] : []),
    ['run.session_terminated', { adapter: 'codex', outcome: 'completed', provider_process_alive: false,
      message: 'Provider session stopped; no idle agent process remains.' }],
    ...(ackPosition === 'after' ? [ack] : []),
    ['run.workspace_preserved', { ...workspace, workspace_fingerprint: run.workspace_fingerprint,
      head_commit: SOURCE_COMMIT, branch_deleted: false, detail: PRIVATE }],
  ].filter(([type]) => !omit.includes(type))
  const events = entries.map(([type, payload], index) => event(config, type, firstSeq + index, payload))
  const incident = { id: id(107), corp_id: config.corp_id, mission_id: mission.id, task_id: task.id,
    run_id: run.id, stage: 'stop', input: breakerInput }
  const state = emptyState(config)
  Object.assign(state.snapshot, { missions: [mission], tasks: [task], agents: [agent], runs: [run],
    circuit_breaker_incidents: [incident], events: events.slice(-1) })
  const checkpoint = { ...newReport(config).case, create_attempted: true, launch_attempted: true,
    mission_id: mission.id, task_id: task.id, agent_id: agent.id, run_id: run.id,
    contract_sha256: digest(task.contract), phase: 'launched' }
  return { state, mission, task, agent, run, events, incident, checkpoint,
    replay: { events, through: events.at(-1).seq } }
}

function memoryHarness(config, options = {}) {
  const state = emptyState(config)
  state.snapshot.missions.push({ id: id(40), corp_id: config.corp_id, status: 'failed', description: PRIVATE })
  state.snapshot.tasks.push({ id: id(41), corp_id: config.corp_id, mission_id: id(40), status: 'failed' })
  state.snapshot.runs.push({ id: id(42), corp_id: config.corp_id, task_id: id(41), status: 'failed', summary: PRIVATE })
  state.snapshot.agents.push({ id: id(43), corp_id: config.corp_id, mission_id: id(40), status: 'idle' })
  const events = [event(config, 'corp.demo_bootstrapped', 1, { secret: PRIVATE }, config.corp_id, null)]
  const saves = []
  const posts = []
  const announcements = []
  let clock = Date.parse('2026-09-08T12:00:00Z')
  let failCreate = !!(options.createUnknown || options.createLost)
  let failLaunch = !!(options.launchUnknown || options.launchLost)
  let liveFixture
  const io = {
    now: () => clock, save: async report => { saves.push(clone(report)); options.onSave?.(report) },
    announce: async intent => { announcements.push(clone(intent)); assert.equal(saves.at(-1).case.phase, `${intent.operation}_intent`) },
    snapshot: async () => { options.onSnapshot?.(state, events); return clone(state) },
    replay: async () => { options.onReplay?.(state, events); return { events: clone(events), through: events.at(-1).seq } },
    sleep: async ms => { clock += ms; options.onSleep?.(state, events, clock, liveFixture) },
    readBase: async checkpoint => ({ path: path.join(expectedWorkspace(config, checkpoint.task_id, checkpoint.run_id), 'base.txt'),
      bytes: 5, sha256: BASE_SHA256 }),
    create: async () => {
      assert.equal(saves.at(-1).case.create_attempted, true)
      assert.equal(saves.at(-1).case.phase, 'create_intent')
      assert.equal(announcements.at(-1).operation, 'create')
      posts.push('create')
      if (failCreate && options.createUnknown) { failCreate = false; throw new Error(PRIVATE) }
      const created = fixture(config)
      created.mission.status = 'ready'
      created.task.status = 'ready'
      created.task.attempt_count = 0
      for (const [table, row] of [['missions', created.mission], ['tasks', created.task], ['agents', created.agent]]) {
        state.snapshot[table].push(row)
      }
      events.push(event(config, 'mission.created', events.at(-1).seq + 1, {}, created.mission.id, null))
      if (failCreate) { failCreate = false; throw new Error(PRIVATE) }
      return { mission_id: created.mission.id, task_id: created.task.id, task_ids: [created.task.id], strategy: 'single' }
    },
    launch: async missionId => {
      assert.equal(saves.at(-1).case.launch_attempted, true)
      assert.equal(saves.at(-1).case.phase, 'launch_intent')
      assert.equal(saves.at(-1).case.mission_id, missionId)
      assert.equal(announcements.at(-1).operation, 'launch')
      posts.push('launch')
      if (failLaunch && options.launchUnknown) { failLaunch = false; throw new Error(PRIVATE) }
      liveFixture = fixture(config, { firstSeq: events.at(-1).seq + 1, omit: options.omit ?? [] })
      if (options.live) {
        liveFixture.run.status = 'running'
        liveFixture.run.workspace_disposition = 'active'
        liveFixture.run.workspace_fingerprint = null
        liveFixture.run.input_tokens = 0
        liveFixture.run.breaker_stage = 'healthy'
        liveFixture.run.summary = ''
        liveFixture.task.status = 'running'
        liveFixture.mission.status = 'running'
        liveFixture.agent.status = 'working'
        liveFixture.agent.station = 'terminal'
        liveFixture.agent.current_run_id = liveFixture.run.id
        liveFixture.events.splice(4)
      }
      for (const [table, row] of [['missions', liveFixture.mission], ['tasks', liveFixture.task], ['agents', liveFixture.agent]]) {
        Object.assign(state.snapshot[table].find(item => item.id === row.id), row)
      }
      state.snapshot.runs.push(liveFixture.run)
      state.snapshot.circuit_breaker_incidents.push(liveFixture.incident)
      events.push(...liveFixture.events)
      options.afterLaunch?.(state, events, liveFixture)
      if (failLaunch) { failLaunch = false; throw new Error(PRIVATE) }
      return { run_id: liveFixture.run.id, run_ids: [liveFixture.run.id],
        runner_id: config.runner_id, runner_ids: [config.runner_id], replayed: false }
    },
  }
  return { io, state, events, saves, posts, announcements, last: () => saves.at(-1) }
}

test('explicit CLI and parent runtime receipt reuse retain owner/source/process binding', () => {
  const config = configuration()
  const receiptPath = path.join(ROOT, 'runtime.json')
  assert.deepEqual(parseArgs(['--receipt', receiptPath, '--continue']), { receiptPath, continuation: true })
  assert.throws(() => parseArgs(['--receipt', receiptPath, '--reset']))
  const workspace = path.join(ROOT, 'code')
  const receipt = {
    schema_version: 1, test_owned: true, issue: 190, owner_task: config.receipt_id, phase: 'running',
    server_url: config.server_url, corp_id: config.corp_id, actor_id: config.actor_id,
    runner_id: config.runner_id, runner_root: config.runner_root, workspace,
    source_commit: '3524cf4956392ebc929032a823f6febb641855ec',
    source_repository_path: path.join(ROOT, 'issue174-startup', 'source'),
    source_base_ref: 'HEAD', source_base_commit: SOURCE_COMMIT,
    processes: Object.fromEntries(['server', 'runner'].map((role, index) => [role, {
      role, pid: 50_000 + index, executable: path.join(ROOT, `${role}.exe`), workspace,
      started_utc: '2026-09-08T11:00:00Z', stderr: PRIVATE,
    }])),
    unconsumed_metadata: PRIVATE,
  }
  const options = { sourceRepository: config.source.repository, fixture: FIXTURE, fixtureSha256: '4'.repeat(64),
    outputDir: config.output_dir, reportPath: config.report_path }
  const result = probeConfiguration(receipt, options)
  assert.equal(result.receipt_id, receipt.owner_task)
  assert.deepEqual(result.source, config.source)
  assert.ok(result.runtime_binding_sha256)
  assert.ok(!JSON.stringify(result).includes(PRIVATE))
  assert.throws(() => probeConfiguration(receipt, { ...options, fixtureSha256: undefined }))
  const other = clone(receipt)
  other.processes.runner.pid++
  assert.notEqual(receiptIdentity(result), receiptIdentity(probeConfiguration(other, options)))
  for (const change of [{ server_url: 'http://127.0.0.1:18575' }, { runner_id: 'replacement' },
    { source: { ...config.source, base_commit: 'a'.repeat(40) } }, { test_owned: false },
    { access_token: PRIVATE }, { provider: { ...config.provider, adapter: 'fake-process' } }]) {
    assert.throws(() => configuration(change))
  }
})

test('additive request changes only the fixture case, keeps native 5000 authority and no delivery/resume', () => {
  const request = probeRequest(configuration())
  assert.ok(request.title.startsWith(MARKER))
  assert.ok(!JSON.stringify(request).includes('[budget-stream]'))
  assert.equal(request.budget_tokens, 5000)
  assert.equal(request.preferred_adapter, 'codex')
  assert.equal(request.strategy, 'single')
  assert.deepEqual(request.secret_refs, [])
  assert.equal(request.deliverable, null)
  assert.deepEqual(request.contract.write_scope, ['base.txt'])
  assert.deepEqual(request.verification_policy, { checks: [{ type: 'file', path: 'base.txt', min_bytes: 5 }],
    manual_gate: { type: 'human_approval', roles: ['owner', 'admin'] } })
  assert.equal(request.idempotency_key, undefined, 'native create has no invented idempotency field')
  assert.equal(request.max_attempts, undefined, 'do not change the native API')
})

test('shared client sends exact additive create/launch bodies without retries or extra routes', async () => {
  const config = configuration()
  const calls = []
  const api = createProbeApi(config, { fetchImpl: async (url, options) => {
    calls.push({ url, options })
    return new Response('{}', { status: 200 })
  } })
  await api.create()
  await api.launch(id(100))
  assert.deepEqual(JSON.parse(calls[0].options.body), probeRequest(config))
  assert.deepEqual(JSON.parse(calls[1].options.body), { requested_by: config.actor_id })
  assert.ok(calls.every(call => call.options.redirect === 'error' &&
    canonical(call.options.headers) === canonical({ 'content-type': 'application/json' })))
  for (const route of ['reset', 'bootstrap', 'runners/enroll', 'budget-policy', `runs/${id(104)}/resume`]) {
    await assert.rejects(api.request(`/api/corps/${config.corp_id}/${route}`, {}), { code: 'api_route_outside_additive_allowlist' })
  }
  assert.equal(calls.length, 2)
  let failedCalls = 0
  const failed = createProbeApi(config, { fetchImpl: async () => {
    failedCalls++
    return new Response(PRIVATE, { status: 500 })
  } })
  await assert.rejects(failed.create(), { code: 'api_response_not_ok_body_withheld' })
  assert.equal(failedCalls, 1)
})

for (const ackPosition of ['before', 'between', 'after']) {
  test(`pure native trace accepts actual ACK ${ackPosition} failure/termination without prescribing that order`, () => {
    const config = configuration()
    const f = fixture(config, { ackPosition })
    const proof = assessCase(f.state, f.replay, config, f.checkpoint)
    observeJournal(f.replay, f.checkpoint, config)
    assert.ok(proof.ordering.failed < proof.ordering.session_terminated)
    const ack = f.checkpoint.journal_observation.command_acknowledgments[0]
    assert.equal(ack.before_failed, ackPosition === 'before')
    assert.equal(ack.before_session_terminated, ackPosition !== 'after')
    assert.equal(ack.seq, proof.ordering.command_acknowledged)
    assert.ok(!JSON.stringify([proof, f.checkpoint]).includes(PRIVATE))
  })
}

const negativeCases = [
  ['missing actual artifact rejection', f => {
    f.events.find(e => e.type === 'run.failed').type = 'run.cancelled'
  }, 'race_not_observed_artifact_rejection_missing'],
  ['unrelated failure', f => {
    f.events.find(e => e.type === 'run.failed').payload.error = 'provider fixture failed'
  }, 'race_not_observed_failure_not_artifact_rejection'],
  ['termination before failure is not the race', f => {
    const a = f.events.findIndex(e => e.type === 'run.failed')
    const b = f.events.findIndex(e => e.type === 'run.session_terminated')
    // Alternate in-memory journal data, never a runtime event rewrite or relay.
    const first = f.events[a]
    const second = f.events[b]
    f.events[a] = { ...second, seq: first.seq }
    f.events[b] = { ...first, seq: second.seq }
  }, 'race_not_observed_termination_preceded_failure'],
  ['missing termination is not success', f => {
    f.events.splice(f.events.findIndex(e => e.type === 'run.session_terminated'), 1)
  }, 'native_termination_not_observed'],
  ['missing ACK remains unproven', f => {
    f.events.splice(f.events.findIndex(e => e.type === 'runner.command_acknowledged'), 1)
  }, 'hard_command_ack_not_observed'],
  ['wrong command correlation', f => {
    f.events.find(e => e.type === 'runner.command_acknowledged').payload.command_id = id(999)
  }, 'hard_command_ack_not_observed'],
  ['wrong ACK runner', f => {
    f.events.find(e => e.type === 'runner.command_acknowledged').payload.runner_id = 'foreign'
  }, 'native_hard_command_ack_invalid'],
  ['foreign session event scope', f => {
    f.events.find(e => e.type === 'run.session_terminated').correlation_id = id(999)
  }, 'run_event_lineage_mismatch'],
  ['non-native event attribution', f => {
    f.events.find(e => e.type === 'run.session_terminated').actor_id = id(17)
  }, 'native_runner_event_binding_mismatch'],
  ['wrong native event identity', f => {
    f.events.find(e => e.type === 'run.session_terminated').idempotency_key = 'invented'
  }, 'native_runner_event_binding_mismatch'],
  ['rewritten failure summary', f => { f.run.summary = 'cancelled' }, 'original_failed_projection_not_preserved'],
  ['agent still assigned', f => { f.agent.current_run_id = f.run.id }, 'original_failed_projection_not_preserved'],
  ['task resurrected', f => { f.task.status = 'ready' }, 'original_failed_projection_not_preserved'],
  ['mission resurrected', f => { f.mission.status = 'running' }, 'original_failed_projection_not_preserved'],
  ['accepted artifact metadata', f => { f.run.artifact_id = id(300) }, 'accepted_artifact_or_verification_present'],
  ['late accepted completion even if snapshot says failed', f => {
    f.events.find(e => e.type === 'run.output').type = 'run.completed'
  }, 'accepted_progress_or_uncertain_teardown'],
  ['accepted artifact even outside bounded snapshot.events', f => {
    f.events.find(e => e.type === 'run.output').type = 'run.artifact'
  }, 'accepted_progress_or_uncertain_teardown'],
  ['verification event is forbidden', f => {
    f.events.find(e => e.type === 'run.output').type = 'run.verification_started'
  }, 'accepted_progress_or_uncertain_teardown'],
  ['uncertain teardown is forbidden', f => {
    f.events.find(e => e.type === 'run.output').type = 'run.teardown_uncertain'
  }, 'accepted_progress_or_uncertain_teardown'],
  ['accepted evidence record', f => {
    f.state.snapshot.verification_evidence.push({ id: id(500), corp_id: f.run.corp_id, run_id: f.run.id })
  }, 'accepted_evidence_or_changed_authority'],
  ['budget revision', f => {
    f.state.snapshot.mission_budget_revisions.push({ id: id(500), corp_id: f.run.corp_id, mission_id: f.mission.id })
  }, 'accepted_evidence_or_changed_authority'],
  ['replacement run', f => {
    f.state.snapshot.runs.push({ ...f.run, id: id(500) })
  }, 'automatic_retry_or_duplicate_run'],
  ['source substitution', f => { f.run.source_base_commit = 'c'.repeat(40) }, 'original_run_identity_or_authority_changed'],
  ['verification-only execution cannot attest a provider', f => {
    f.run.execution_mode = 'verification_only'
  }, 'original_run_identity_or_authority_changed'],
  ['usage must be exactly two 3000-token increments', f => {
    f.events.find(e => e.type === 'run.usage').payload.input_tokens = 300_000
  }, 'fixture_usage_not_exact'],
]
for (const [name, change, code] of negativeCases) {
  test(`pure evidence rejection: ${name}`, () => {
    const config = configuration()
    const f = fixture(config)
    change(f)
    assert.throws(() => assessCase(f.state, f.replay, config, f.checkpoint), { code })
  })
}

for (const invalid of [
  null, [], {}, { adapter: 'other' }, { provider_process_alive: true }, { provider_process_alive: 'false' },
  { outcome: 'verified' }, { message: '' }, { message: 1 }, { message: 'x'.repeat(1025) },
  { message: 'untrusted\ntext' }, { message: '\u0085' }, { credential: PRIVATE },
]) {
  test(`invalid native termination payload cannot pass (${JSON.stringify(invalid).slice(0, 60)})`, () => {
    const config = configuration()
    const f = fixture(config)
    const terminated = f.events.find(e => e.type === 'run.session_terminated')
    terminated.payload = invalid === null || Array.isArray(invalid) || !Object.keys(invalid).length
      ? invalid : { ...terminated.payload, ...invalid }
    assert.throws(() => assessCase(f.state, f.replay, config, f.checkpoint), { code: 'invalid_native_termination_payload' })
  })
}

test('all native terminal outcomes and absent optional message are valid telemetry, not accepted completion', () => {
  for (const outcome of ['completed', 'cancelled', 'failed', 'runtime_error']) {
    const config = configuration()
    const f = fixture(config)
    const terminated = f.events.find(e => e.type === 'run.session_terminated')
    terminated.payload.outcome = outcome
    delete terminated.payload.message
    assert.equal(assessCase(f.state, f.replay, config, f.checkpoint).accepted_completion, false)
  }
})

test('one additive case checkpoints and announces intent/IDs, retains history, and waits for quiet readback', async () => {
  const config = configuration()
  const h = memoryHarness(config)
  const report = newReport(config, h.io.now())
  await executeProbe(config, report, h.io)
  assert.deepEqual(h.posts, ['create', 'launch'])
  assert.equal(h.announcements[0].mission_id, null, 'server has not allocated IDs before create')
  assert.equal(h.announcements[0].request_sha256, digest(probeRequest(config)))
  assert.equal(h.announcements[1].mission_id, id(100))
  assert.equal(h.announcements[1].task_id, id(101))
  assert.equal(h.announcements[1].request_sha256, digest({ requested_by: config.actor_id }))
  assert.ok(h.announcements[1].route.endsWith(`/missions/${id(100)}/launch`))
  assert.equal(report.passed, true)
  assert.equal(report.race_observed, true)
  assert.ok(report.case.evidence.quiet_observation_ms >= config.settle_ms)
  assert.equal(report.case.evidence.base_file.sha256, BASE_SHA256)
  assert.equal(report.baseline.rows.missions.length, 1)
  assert.equal(report.baseline.agents.length, 1)
  assert.ok(!JSON.stringify(h.saves).includes(PRIVATE))
})

test('passed same-case continuation recomputes proof with zero further mutations', async () => {
  const config = configuration()
  const h = memoryHarness(config)
  await executeProbe(config, newReport(config), h.io)
  const continued = validateSavedReport(clone(h.last()), config)
  assert.equal(continued.passed, false)
  assert.equal(continued.case.evidence, null)
  await executeProbe(config, continued, h.io)
  assert.deepEqual(h.posts, ['create', 'launch'])
  assert.equal(continued.case.run_id, id(104))
  assert.equal(continued.passed, true)
})

for (const mode of ['createLost', 'createUnknown', 'launchLost', 'launchUnknown']) {
  test(`${mode}: failure preserves exact intent, continuation never retries an attempted POST`, async () => {
    const config = configuration()
    const h = memoryHarness(config, { [mode]: true })
    await assert.rejects(executeProbe(config, newReport(config), h.io), { code: 'operation_failed_details_withheld' })
    const saved = clone(h.last())
    assert.equal(saved.case.create_attempted, true)
    assert.ok(!JSON.stringify(saved).includes(PRIVATE))
    const continued = validateSavedReport(saved, config)
    if (mode.endsWith('Unknown')) {
      await assert.rejects(executeProbe(config, continued, h.io), {
        code: mode === 'createUnknown' ? 'create_intent_ambiguous_no_replacement' : 'launch_intent_ambiguous_no_relaunch',
      })
      assert.equal(h.posts.filter(post => post === (mode === 'createUnknown' ? 'create' : 'launch')).length, 1)
    } else {
      await executeProbe(config, continued, h.io)
      assert.equal(continued.passed, true)
      assert.deepEqual(h.posts, ['create', 'launch'], 'only never-attempted launch may proceed')
      assert.equal(continued.case.run_id, id(104))
    }
  })
}

test('bounded live failure leaves the original run live, IDs and intent intact, with no replacement', async () => {
  const config = configuration()
  const h = memoryHarness(config, { live: true })
  await assert.rejects(executeProbe(config, newReport(config), h.io), { code: 'observation_timeout_run_not_reclassified' })
  assert.equal(h.last().status, 'incomplete_live')
  assert.equal(h.last().case.latest.status, 'running')
  assert.equal(h.last().case.run_id, id(104))
  assert.ok(h.last().case.journal_observation.events.length > 0)
  assert.equal(h.state.snapshot.runs.find(row => row.id === id(104)).status, 'running')
  assert.equal(h.last().race_observed, false)
  assert.deepEqual(h.posts, ['create', 'launch'])
  await assert.rejects(executeProbe(config, validateSavedReport(clone(h.last()), config), h.io))
  assert.deepEqual(h.posts, ['create', 'launch'])
})

test('missing late telemetry is a bounded failure with actual ACK order, not a race claim', async () => {
  const config = configuration()
  const h = memoryHarness(config, { omit: ['run.session_terminated'] })
  await assert.rejects(executeProbe(config, newReport(config), h.io), { code: 'native_termination_not_observed' })
  const report = h.last()
  assert.equal(report.passed, false)
  assert.equal(report.race_observed, false)
  assert.equal(report.case.latest.status, 'failed')
  assert.ok(report.case.failure_event)
  assert.equal(report.case.journal_observation.command_acknowledgments.length, 1)
  assert.equal(report.case.journal_observation.command_acknowledgments[0].before_session_terminated, null)
  assert.deepEqual(h.posts, ['create', 'launch'])
})

test('natural later telemetry/finalization can settle without rewriting terminal state or historical events', async () => {
  const config = configuration()
  let appended = false
  const h = memoryHarness(config, {
    omit: ['run.session_terminated', 'run.workspace_preserved'],
    afterLaunch: (_state, _events, f) => {
      f.run.workspace_disposition = 'active'
      f.run.workspace_fingerprint = null
    },
    onSleep: (_state, events, _clock, f) => {
      if (appended) return
      appended = true
      // Newly observed records appended to a pure memory trace; no control relay.
      const final = fixture(config)
      for (const type of ['run.session_terminated', 'run.workspace_preserved']) {
        events.push(event(config, type, events.at(-1).seq + 1, final.events.find(e => e.type === type).payload))
      }
      f.run.workspace_disposition = 'preserved'
      f.run.workspace_fingerprint = final.run.workspace_fingerprint
    },
  })
  await executeProbe(config, newReport(config), h.io)
  assert.equal(h.last().passed, true)
  assert.ok(h.last().case.evidence.ordering.failed < h.last().case.evidence.ordering.session_terminated)
  assert.equal(h.last().case.terminal_baseline.digests.run, h.last().case.evidence.terminal_projection.run)
})

for (const [name, mutate, code] of [
  ['terminal run summary', state => { state.snapshot.runs.at(-1).summary = 'rewritten' }, 'terminal_projection_changed'],
  ['terminal task', state => { state.snapshot.tasks.at(-1).status = 'ready' }, 'terminal_projection_changed'],
  ['terminal mission', state => { state.snapshot.missions.at(-1).status = 'running' }, 'terminal_projection_changed'],
  ['terminal agent', state => { state.snapshot.agents.at(-1).current_run_id = id(999) }, 'terminal_projection_changed'],
  ['old run', state => { state.snapshot.runs[0].summary = 'rewritten' }, 'prior_visible_history_changed'],
  ['old agent', state => { state.snapshot.agents[0].status = 'working' }, 'original_agent_history_changed'],
  ['old journal prefix', (_state, events) => { events[0].payload = {} }, 'prior_visible_journal_prefix_changed'],
  ['original failure event', (_state, events) => { events.find(e => e.type === 'run.failed').payload.extra = true },
    'prior_visible_journal_prefix_changed'],
]) {
  test(`settling cannot hide changed ${name}`, async () => {
    const config = configuration()
    let changed = false
    const h = memoryHarness(config, { onSleep: (state, events) => {
      if (!changed) { changed = true; mutate(state, events) }
    } })
    await assert.rejects(executeProbe(config, newReport(config), h.io), { code })
    assert.equal(h.last().race_observed, false)
    assert.deepEqual(h.posts, ['create', 'launch'])
  })
}

test('saved report cannot substitute receipt, native authority, source, or erase baseline after an effect', () => {
  const config = configuration()
  const report = newReport(config)
  for (const changed of [configuration({ receipt_id: id(901) }),
    configuration({ provider: { ...config.provider, sha256: 'c'.repeat(64) } }),
    configuration({ source: { ...config.source, repository: 'other/source' } })]) {
    assert.throws(() => validateSavedReport(clone(report), changed), { code: 'continuation_receipt_mismatch' })
  }
  for (const change of [
    value => { value.case.access_token = PRIVATE },
    value => { value.case.budget_tokens++ },
    value => { value.case.create_attempted = true },
    value => { value.case.run_id = id(104) },
    value => { delete value.case.mission_id },
    value => { value.suite = 'stopped-source-checkpoint-190' },
  ]) {
    const changed = clone(report)
    change(changed)
    assert.throws(() => validateSavedReport(changed, config))
  }
  const longer = configuration({ timeout_ms: 15_000 })
  assert.equal(validateSavedReport(clone(report), longer).receipt_sha256, report.receipt_sha256)
})

// Minimal memory-only fs implementing the APIs used by the report lifecycle.
function reportFiles(config, options = {}) {
  const entries = new Map()
  const operations = []
  let inode = 1
  const add = (file, bytes = '', directory = false) => {
    const entry = { bytes: Buffer.from(bytes), directory, inode: inode++, mtime: 1, link: false, nlink: 1 }
    entries.set(file, entry)
    return entry
  }
  for (const directory of [config.runner_root, config.output_dir]) add(directory, '', true)
  const get = file => {
    const entry = entries.get(file)
    if (!entry) throw Object.assign(new Error(PRIVATE), { code: 'ENOENT' })
    return entry
  }
  const stat = entry => ({
    isDirectory: () => entry.directory, isFile: () => !entry.directory, isSymbolicLink: () => entry.link,
    size: entry.bytes.length, nlink: entry.nlink, ino: entry.inode, dev: 1, mtimeMs: entry.mtime,
  })
  const io = {
    lstat: async file => stat(get(file)),
    realpath: async file => { get(file); return file },
    open: async (file, flags, mode) => {
      operations.push(['open', file, flags, mode])
      if (flags === 'wx') {
        if (entries.has(file)) throw Object.assign(new Error(PRIVATE), { code: 'EEXIST' })
        add(file)
      }
      const entry = get(file)
      options.onOpen?.(entry, file, flags)
      return {
        stat: async () => stat(entry),
        read: async (buffer, offset, length, position) => {
          operations.push(['read', file])
          const bytesRead = entry.bytes.copy(buffer, offset, position, position + length)
          return { bytesRead }
        },
        writeFile: async bytes => { entry.bytes = Buffer.from(bytes); entry.mtime++; operations.push(['write', file]) },
        sync: async () => { operations.push(['sync', file]) },
        close: async () => { operations.push(['close', file]) },
      }
    },
    rename: async (from, to) => {
      operations.push(['rename', from, to])
      if (options.failRename) throw new Error(PRIVATE)
      entries.set(to, get(from)); entries.delete(from)
    },
    unlink: async file => {
      assert.equal(file, `${config.report_path}.lock`, 'only this invocation lock can be cleaned')
      get(file); entries.delete(file); operations.push(['unlink', file])
    },
  }
  return { io, entries, operations, add }
}

test('report save uses exclusive lock, private synced temp and atomic rename; refuses implicit overwrite', async () => {
  const config = configuration()
  const files = reportFiles(config)
  const options = { receiptPath: path.join(ROOT, 'runtime.json'), continuation: false }
  const storage = await openReport(config, options, files.io)
  await storage.save(storage.report)
  const rename = files.operations.findIndex(op => op[0] === 'rename')
  const temporary = files.operations[rename][1]
  assert.ok(files.operations.slice(0, rename).some(op => op[0] === 'sync' && op[1] === temporary))
  assert.ok(files.operations.some(op => op[0] === 'open' && op[1] === temporary && op[2] === 'wx' && op[3] === 0o600))
  await storage.close()
  const original = Buffer.from(files.entries.get(config.report_path).bytes)
  await assert.rejects(openReport(config, options, files.io), { code: 'report_exists_use_explicit_continue' })
  assert.deepEqual(files.entries.get(config.report_path).bytes, original)
  assert.ok(!files.entries.has(`${config.report_path}.lock`))
  const continued = await openReport(config, { ...options, continuation: true }, files.io)
  assert.equal([...files.entries.keys()].filter(file => file.includes('.previous-')).length, 1)
  assert.deepEqual(files.entries.get([...files.entries.keys()].find(file => file.includes('.previous-'))).bytes, original)
  await continued.close()
})

test('stale locks are never stolen and failed saves retain original report and orphaned temp', async () => {
  const config = configuration()
  const options = { receiptPath: path.join(ROOT, 'runtime.json'), continuation: false }
  const stale = reportFiles(config)
  const lock = stale.add(`${config.report_path}.lock`, 'previous driver')
  await assert.rejects(openReport(config, options, stale.io), { code: 'EEXIST' })
  assert.equal(stale.entries.get(`${config.report_path}.lock`), lock)
  assert.ok(!stale.operations.some(op => op[0] === 'unlink'))
  const failed = reportFiles(config, { failRename: true })
  const storage = await openReport(config, options, failed.io)
  await assert.rejects(storage.save(storage.report))
  assert.ok([...failed.entries.keys()].some(file => file.endsWith('.tmp')))
  assert.ok(!failed.entries.has(config.report_path))
  await storage.close()
  assert.ok([...failed.entries.keys()].some(file => file.endsWith('.tmp')))
})

test('report cannot replace receipt; bounded reads reject links, hardlinks and inode swaps before reading', async () => {
  const config = configuration()
  const files = reportFiles(config)
  await assert.rejects(openReport(config, { receiptPath: config.report_path, continuation: false }, files.io),
    { code: 'report_cannot_replace_receipt' })
  for (const shape of ['link', 'hardlink', 'oversized', 'swap']) {
    const f = reportFiles(config, { onOpen: entry => { if (shape === 'swap') entry.inode++ } })
    const entry = f.add(config.report_path, '{}')
    if (shape === 'link') entry.link = true
    if (shape === 'hardlink') entry.nlink = 2
    await assert.rejects(readBounded(config.report_path, shape === 'oversized' ? 1 : LIMITS.report_bytes, f.io))
    assert.ok(!f.operations.some(op => op[0] === 'read'), shape)
  }
})
