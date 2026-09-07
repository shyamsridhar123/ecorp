// Pure in-memory API/checkpoint simulations. No network, filesystem writes,
// server/runner/DB processes, real providers, or browser actions.
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import path from 'node:path'
import test from 'node:test'
import { Api, parseArgs, runFixture, validateFixture } from './e2e_evidence_selection.mjs'

const id = (n) => `00000000-0000-4000-8000-${String(n).padStart(12, '0')}`
const sha = (bytes) => createHash('sha256').update(bytes).digest('hex')
const copy = (value) => structuredClone(value)
const env = { CRONY_EVIDENCE_TEST: '1' }
const fixture = () => ({
  schema_version: 1, test_owned: true, purpose: 'evidence-selection-163', fixture_id: id(1),
  server_url: 'http://127.0.0.1:18968', web_url: 'http://127.0.0.1:15498',
  database: { host: '127.0.0.1', port: 15497, name: 'owned_evidence_qa' },
  auth_mode: 'development', corp_id: id(2), room_id: id(3), requester_actor_id: id(4), reviewer_actor_id: id(5),
  runner_id: 'owned-evidence-runner', source_checkout: path.resolve('unit-fixture-source'),
  runner_workspace: path.resolve('unit-fixture-runner'),
  source: { repository: 'local/owned-fixture-abc', base_ref: 'HEAD', base_commit: 'a'.repeat(40) },
})
const automatic = { checks: [{ type: 'artifact', min_bytes: 1 }], manual_gate: null }
const started = '2026-09-07T10:00:00.000Z'

function harness() {
  const f = validateFixture(fixture(), env)
  const s = { corp: { id: f.corp_id }, rooms: [{ id: f.room_id, corp_id: f.corp_id, created_at: started }],
    actors: [
      { id: f.requester_actor_id, corp_id: f.corp_id, name: 'Alice', kind: 'human', role: 'owner', created_at: started },
      { id: f.reviewer_actor_id, corp_id: f.corp_id, name: 'Bob', kind: 'human', role: 'member', created_at: started },
    ], agents: [], missions: [], tasks: [], runs: [], events: [], verification_evidence: [],
    verification_requests: [], mission_contract_revisions: [], source_deliverables: [],
    action_approvals: [], factory_work_items: [], factory_controllers: [] }
  const state = { snapshot: s, runners: [{ id: f.runner_id, corp_id: f.corp_id, connected: true,
    status: 'connected', capabilities: [{ name: 'fake-process', available: true },
      { name: 'workspace-isolation', available: true, source_repository: f.source.repository,
        source_base_ref: f.source.base_ref, source_base_commit: f.source.base_commit }] }] }
  const store = { saved: null, writes: [],
    read: async () => copy(store.saved),
    write: async (value) => { store.saved = copy(value); store.writes.push(copy(value)) },
  }
  let seq = 0
  const append = (type, aggregateId, payload = {}, actorId = null, correlationId = id(100)) => {
    const e = { id: id(10_000 + ++seq), seq, type, aggregate_id: aggregateId,
      corp_id: f.corp_id, room_id: f.room_id, actor_id: actorId, correlation_id: correlationId,
      payload, idempotency_key: `unit:${seq}`, created_at: started }
    s.events.push(e)
    return e
  }
  append('fixture.exists', f.corp_id, {}, null, null)
  const objects = new Map()
  const revisions = new Map()
  const requests = []
  let fault = null
  let corruptDownload = false
  const create = (body) => {
    s.missions.push({ id: id(100), corp_id: f.corp_id, room_id: f.room_id, requested_by: body.requested_by,
      title: body.title, description: body.description, strategy: body.strategy, status: 'ready', specification_version: 1 })
    for (const [i, key] of ['specialist-a', 'specialist-b', 'synthesis'].entries()) {
      s.agents.push({ id: id(110 + i), actor_id: id(120 + i), corp_id: f.corp_id, mission_id: id(100),
        name: key, adapter: 'fake-process', pinned: false })
      s.tasks.push({ id: id(200 + i), corp_id: f.corp_id, mission_id: id(100), plan_key: key,
        assigned_agent_id: id(110 + i), depth: i === 2 ? 1 : 0, depends_on: i === 2 ? [id(200), id(201)] : [],
        contract_version: 1, attempt_count: 0, required_adapter: 'fake-process', verification_policy: copy(automatic),
        verification_status: 'pending', status: i === 2 ? 'pending' : 'ready', contract: {
          objective: `${body.description}\n\nTASK-SPECIFIC OBJECTIVE:\n${key}`,
          expected_output: 'result.md', source_repository: f.source.repository,
          source_base_ref: f.source.base_ref, source_base_commit: f.source.base_commit,
          allowed_tools: ['filesystem', 'shell'], write_scope: ['**'], prohibited_actions: ['no external writes'],
          acceptance_tests: ['artifact exists'], references: i === 2 ? ['task:specialist-a', 'task:specialist-b'] : [],
          budget_tokens: 20_000, secret_refs: [], model: null, reasoning_effort: null, deliverable: null,
        } })
    }
    append('mission.created', id(100))
    return { mission_id: id(100), task_id: id(200), task_ids: [id(200), id(201), id(202)], strategy: 'parallel-specialists' }
  }
  const revise = (body) => {
    if (revisions.has(body.idempotency_key)) return { revision: revisions.get(body.idempotency_key), replayed: true }
    const t = s.tasks.find((t) => t.id === body.task_id)
    assert.equal(t.contract_version, body.expected_contract_version)
    const mission = s.missions.find((m) => m.id === id(100))
    const nextVersion = Math.max(mission.specification_version, t.contract_version) + 1
    const r = { id: id(300 + revisions.size), corp_id: f.corp_id, mission_id: id(100), task_id: t.id,
      version: nextVersion, revised_by: body.actor_id, next_action: body.next_action, source_run_id: null,
      reason: body.reason, previous_description: body.description, replacement_description: body.description,
      previous_contract: copy(t.contract), replacement_contract: copy(body.contract),
      previous_verification_policy: copy(t.verification_policy), replacement_verification_policy: copy(body.verification_policy) }
    t.contract_version = nextVersion
    mission.specification_version = nextVersion
    t.verification_policy = copy(body.verification_policy)
    s.mission_contract_revisions.push(r)
    revisions.set(body.idempotency_key, r)
    append('mission.contract_revised', id(100), { task_id: t.id }, body.actor_id)
    return { revision: r, replayed: false }
  }
  const allocate = (t, i) => {
    const r = { id: id(400 + i), corp_id: f.corp_id, task_id: t.id, agent_id: t.assigned_agent_id,
      runner_id: f.runner_id, resumed_from_run_id: null, model: null, reasoning_effort: null,
      source_repository: f.source.repository, source_base_ref: f.source.base_ref, source_base_commit: f.source.base_commit,
      workspace_path: path.join(f.runner_workspace, `run-${i}`), workspace_base_commit: f.source.base_commit,
      workspace_disposition: 'preserved', status: 'running', verification_status: 'pending',
      created_at: `2026-09-07T10:00:0${i}.000Z` }
    t.attempt_count = 1
    s.runs.push(r)
    append('run.requested', r.id, { task_id: t.id }, f.requester_actor_id)
    append('run.started', r.id)
    return r
  }
  const finish = (r, text = `Deterministic unit artifact ${r.id}`) => {
    const t = s.tasks.find((t) => t.id === r.task_id)
    const bytes = Buffer.from(text)
    r.artifact_id = id(500 + s.verification_evidence.length)
    r.artifact_uri = `/api/corps/${f.corp_id}/artifacts/${r.artifact_id}`
    r.artifact_sha256 = sha(bytes)
    r.artifact_signature = `unit-signature-${r.id}`
    r.artifact_media_type = 'text/markdown'
    objects.set(r.artifact_uri, { bytes, run: r })
    s.verification_evidence.push({ id: id(600 + s.verification_evidence.length), corp_id: f.corp_id,
      task_id: t.id, run_id: r.id, check_index: 0, kind: 'artifact', status: 'passed' })
    if (t.depth === 0) {
      r.status = r.verification_status = t.verification_status = 'waiting_for_approval'
      t.status = 'awaiting_approval'
      s.verification_requests.push({ run_id: r.id, task_id: t.id, corp_id: f.corp_id, gate_type: 'independent_review',
        gate: copy(t.verification_policy.manual_gate), status: 'pending', requested_at: started,
        decided_by: null, decision_note: null, decided_at: null })
      append('run.verification_waiting', r.id, { task_id: t.id })
    } else {
      r.status = t.status = 'completed'
      r.verification_status = t.verification_status = 'passed'
      s.missions.find((m) => m.id === id(100)).status = 'completed'
      append('run.completed', r.id)
    }
  }
  const launch = () => {
    const runs = s.tasks.slice(0, 2).map(allocate)
    runs.forEach((r) => finish(r))
    s.missions.find((m) => m.id === id(100)).status = 'running'
    return { run_ids: runs.map((r) => r.id), run_id: runs[0].id, runner_ids: [f.runner_id, f.runner_id], replayed: false }
  }
  const fetcher = async (url, init) => {
    const u = new URL(url)
    const route = u.pathname
    const body = init.body ? JSON.parse(init.body) : undefined
    requests.push({ route, method: init.method })
    let result
    if (init.method === 'GET') {
      if (route.endsWith('/snapshot')) return Response.json(copy(state))
      const object = objects.get(route)
      assert.ok(object, `Unexpected GET ${route}`)
      return new Response(corruptDownload ? 'wrong bytes' : object.bytes, { headers: {
        'content-type': object.run.artifact_media_type, 'x-content-type-options': 'nosniff',
        'x-crony-artifact-signature': object.run.artifact_signature, 'x-crony-artifact-role': 'provider_evidence',
      } })
    }
    if (route.endsWith('/preview')) return Response.json({ strategy: 'parallel-specialists',
      tasks: ['specialist-a', 'specialist-b', 'synthesis'].map((key) => ({ key })) })
    // Every simulated remote mutation must already have a durable exact intent.
    const operation = store.saved.operations.find((o) => o.route === `${u.pathname}${u.search}` && JSON.stringify(o.body) === JSON.stringify(body))
    assert.ok(operation && operation.sends.length > 0, 'Mutation preceded its checkpoint')
    const before = operation.sends.at(-1)
    assert.ok(before.snapshot_id && before.sha256 && before.event_ids.length)
    assert.deepEqual(before.run_ids, s.runs.map((r) => r.id))
    assert.deepEqual(before.artifact_ids, s.runs.map((r) => r.artifact_id).filter(Boolean))
    const matchingFault = fault && route.endsWith(fault.suffix) && --fault.remaining === 0
    if (matchingFault && !fault.after) { fault = null; throw new Error('Simulated transport failure before commit') }
    if (route.endsWith('/contract-revisions')) result = revise(body)
    else if (route.endsWith('/launch')) result = launch()
    else { assert.ok(route.endsWith('/missions')); result = create(body) }
    if (matchingFault) { fault = null; throw new Error('Simulated response loss after commit') }
    return Response.json(copy(result))
  }
  const run = (phase = 'prepare', afterReplay = false) => runFixture({
    fixture: f, phase, afterReplay, api: new Api(f, env, phase, fetcher), store, sleep: async () => {},
  })
  // This models an EXTERNAL decision solely in test memory. The product helper
  // cannot call a decision endpoint and never gets this function.
  const decide = (runId, actor = f.reviewer_actor_id) => {
    const r = s.runs.find((r) => r.id === runId)
    const t = s.tasks.find((t) => t.id === r.task_id)
    const request = s.verification_requests.find((v) => v.run_id === r.id)
    assert.equal(request.status, 'pending')
    request.status = 'approved'
    request.decided_by = actor
    request.decision_note = 'Bob accepted the recorded verification evidence.'
    request.decided_at = '2026-09-07T10:01:00.000Z'
    r.status = t.status = 'completed'
    r.verification_status = t.verification_status = 'passed'
    append('verification.approved', r.id, { task_id: t.id, status: 'approved', note: request.decision_note }, actor)
    if (s.tasks.slice(0, 2).every((t) => t.status === 'completed')) {
      const synthesis = allocate(s.tasks[2], 2)
      finish(synthesis, `VERIFIED DEPENDENCY OUTPUTS\n${s.runs[0].id}\n${s.runs[1].id}`)
    }
  }
  return { f, s, state, store, requests, run, decide, append,
    fail: (suffix, after = true, remaining = 1) => { fault = { suffix, after, remaining } },
    corrupt: (value) => { corruptDownload = value } }
}

test('strict phase parsing and explicit opt-in', () => {
  assert.deepEqual(parseArgs(['--phase', 'verify-first', '--after-replay']), { phase: 'verify-first', afterReplay: true })
  for (const args of [[], ['prepare'], ['--phase', 'release'], ['--phase', 'prepare', '--after-replay']]) {
    assert.throws(() => parseArgs(args))
  }
  assert.throws(() => validateFixture(fixture(), {}), /CRONY_EVIDENCE_TEST/u)
})

test('reject manual/default/aliased ports, credentials, ambiguous URLs and mismatched environment before API access', () => {
  for (const p of [18962, 15491, 8791, 5432, 0, 9999, 65536]) {
    for (const field of ['server_url', 'web_url', 'database']) {
      const f = fixture()
      if (field === 'database') f.database.port = p
      else f[field] = `http://127.0.0.1:${p}`
      assert.throws(() => validateFixture(f, env), field)
    }
  }
  for (const server of ['http://localhost:18968', 'http://127.0.0.2:18968', 'http://127.1:18968',
    'http://2130706433:18968', 'https://127.0.0.1:18968',
    'http://user:password@127.0.0.1:18968', 'http://127.0.0.1:18968/?x=1']) {
    assert.throws(() => validateFixture({ ...fixture(), server_url: server }, env))
  }
  assert.throws(() => validateFixture(fixture(), { ...env, CRONY_SERVER_HTTP: 'http://127.0.0.1:18962' }))
  assert.throws(() => validateFixture(fixture(), { ...env, DATABASE_URL: 'postgres://x:y@127.0.0.1:15491/shared' }))
  assert.throws(() => validateFixture({ ...fixture(), unexpected_token: 'do-not-persist' }, env), /missing\/unknown/u)
  assert.throws(() => validateFixture({ ...fixture(), auth_mode: 'oidc' }, env), /Both human/u)
  const f = fixture()
  f.reviewer_actor_id = f.requester_actor_id
  assert.throws(() => validateFixture(f, env), /Independent/u)
})

test('verification is GET-only and no phase can call decision/reset/bootstrap/foreign-Corp routes', async () => {
  let calls = 0
  const f = validateFixture(fixture(), env)
  for (const phase of ['prepare', 'verify-first', 'verify']) {
    const api = new Api(f, env, phase, async () => { calls++; throw new Error('Must not fetch') })
    for (const route of [`${api.prefix}/runs/${id(400)}/verification-decision`,
      '/api/demo/reset', '/api/demo/bootstrap', `/api/corps/${id(99)}/missions`]) {
      await assert.rejects(api.request(route, {}), /Only scoped/u)
    }
    if (phase !== 'prepare') await assert.rejects(api.request(`${api.prefix}/missions/preview`, {}), /Only scoped/u)
  }
  assert.equal(calls, 0)
})

test('prepare → newer approved → isolated replay observation → explicit older approval → automatic synthesis', async () => {
  const h = harness()
  const p = await h.run()
  assert.equal(p.reviews_newer_first.length, 2)
  assert.equal(p.reviews_newer_first[0].run_id, id(401))
  assert.equal(h.store.saved.operations.length, 4)
  assert.deepEqual(h.s.tasks.map((t) => t.contract_version), [2, 3, 1], 'Native mission-wide revision sequencing')
  assert.deepEqual(h.store.saved.operations.map((o) => o.sends.length), [1, 1, 1, 1])
  const mutations = h.requests.filter((r) => r.method === 'POST').length
  assert.equal((await h.run()).mission_id, p.mission_id, 'Prepare replay must return the same mission')
  assert.equal(h.requests.filter((r) => r.method === 'POST').length, mutations)
  h.decide(id(401))
  await h.run('verify-first')
  await h.run('verify-first', true)
  assert.equal(h.store.saved.replay.second_still_pending, id(400))
  h.decide(id(400))
  const final = await h.run('verify')
  assert.equal(final.api_passed, true)
  assert.equal(final.synthesis.run_id, id(402))
  assert.equal(final.second_decision.operator.id, h.f.reviewer_actor_id)
  assert.equal(final.synthesis.download.checked_as, h.f.reviewer_actor_id)
  assert.equal(h.requests.filter((r) => r.method === 'POST').length, mutations, 'All verification calls must be reads')
  assert.equal((await h.run('verify')).api_passed, true, 'Final verification is resumable and non-mutating')
})

test('lost create response adopts exact marker without sending a second create', async () => {
  const h = harness()
  h.fail('/missions')
  await assert.rejects(h.run(), /transport failed/u)
  assert.equal(h.s.missions.length, 1)
  assert.equal((await h.run()).mission_id, id(100))
  assert.equal(h.requests.filter((r) => r.method === 'POST' && r.route.endsWith('/missions')).length, 1)
})

test('unknown create that never arrived fails closed, never creates a replacement', async () => {
  const h = harness()
  h.fail('/missions', false)
  await assert.rejects(h.run(), /transport failed/u)
  await assert.rejects(h.run(), /Uncertain create/u)
  assert.equal(h.s.missions.length, 0)
  assert.equal(h.requests.filter((r) => r.method === 'POST' && r.route.endsWith('/missions')).length, 1)
})

test('lost revision response replays the exact native UUID and preserves only two revisions', async () => {
  for (const occurrence of [1, 2]) {
    const h = harness()
    h.fail('/contract-revisions', true, occurrence)
    await assert.rejects(h.run(), /transport failed/u)
    const key = h.store.saved.operations[occurrence].body.idempotency_key
    await h.run()
    assert.equal(h.store.saved.operations[occurrence].body.idempotency_key, key)
    assert.equal(h.store.saved.operations[occurrence].response.replayed, true)
    assert.equal(h.s.mission_contract_revisions.length, 2)
    assert.equal(h.store.saved.operations[occurrence].sends.length, 2)
    assert.deepEqual(h.s.tasks.map((t) => t.contract_version), [2, 3, 1])
  }
})

test('lost launch response recovers exact root IDs by readback, never dispatches again', async () => {
  const h = harness()
  h.fail('/launch')
  await assert.rejects(h.run(), /transport failed/u)
  const p = await h.run()
  assert.deepEqual(p.reviews_newer_first.map((r) => r.run_id), [id(401), id(400)])
  assert.equal(h.requests.filter((r) => r.method === 'POST' && r.route.endsWith('/launch')).length, 1)
})

test('wrong source/pinned staffing/non-quiescent fixture rejected before any POST', async () => {
  for (const change of [
    (h) => { h.state.runners[0].capabilities[1].source_base_commit = 'b'.repeat(40) },
    (h) => { h.s.agents.push({ id: id(800), pinned: true, adapter: 'fake-process' }) },
    (h) => { h.s.runs.push({ id: id(801), task_id: id(802), status: 'running' }) },
  ]) {
    const h = harness()
    change(h)
    await assert.rejects(h.run())
    assert.equal(h.requests.filter((r) => r.method === 'POST').length, 0)
  }
})

test('cannot skip intermediate observation or approve the older run first', async () => {
  const h = harness()
  await h.run()
  h.decide(id(400))
  await assert.rejects(h.run('verify-first'), /Unexpected browser decision/u)
  await assert.rejects(h.run('verify'), /requires verify-first/u)
  const other = harness()
  await other.run()
  other.decide(id(401))
  await assert.rejects(other.run('verify-first', true), /BEFORE/u)
})

test('exact reviewer attribution, artifact binding and no second decision are enforced', async () => {
  const wrongActor = harness()
  await wrongActor.run()
  wrongActor.decide(id(401), wrongActor.f.requester_actor_id)
  await assert.rejects(wrongActor.run('verify-first'), /reviewer attribution/u)
  const swapped = harness()
  await swapped.run()
  swapped.s.runs[1].artifact_id = swapped.s.runs[0].artifact_id
  swapped.s.runs[1].artifact_uri = swapped.s.runs[0].artifact_uri
  await assert.rejects(swapped.run('verify-first'), /IDs changed/u)
  const both = harness()
  await both.run()
  both.decide(id(401))
  await both.run('verify-first')
  both.decide(id(400))
  await assert.rejects(both.run('verify-first', true), /running|Synthesis/u)
})

test('duplicate decision event and journal rollover cannot pass as replay isolation', async () => {
  const h = harness()
  await h.run()
  h.decide(id(401))
  await h.run('verify-first')
  const original = h.s.events.find((e) => e.type === 'verification.approved')
  h.append(original.type, original.aggregate_id, original.payload, original.actor_id)
  await assert.rejects(h.run('verify-first', true), /Duplicate\/cross-run/u)
  const rolled = harness()
  await rolled.run()
  rolled.s.events = []
  await assert.rejects(rolled.run(), /journal anchor/u)
})

test('checkpoint fixture mismatch and modified pre-existing rows are rejected', async () => {
  const h = harness()
  h.s.missions.push({ id: id(900), title: 'Existing preserved mission', status: 'completed' })
  await h.run()
  h.s.missions.find((m) => m.id === id(900)).title = 'Unexpected modification'
  await assert.rejects(h.run(), /Pre-existing missions/u)
  const other = harness()
  await other.run()
  other.store.saved.fixture.fixture_id = id(901)
  const reads = other.requests.length
  await assert.rejects(other.run(), /another fixture/u)
  assert.equal(other.requests.length, reads)
})

test('failed artifact download is rechecked on prepare resume without more POSTs', async () => {
  const h = harness()
  h.corrupt(true)
  await assert.rejects(h.run(), /digest mismatch/u)
  const posts = h.requests.filter((r) => r.method === 'POST').length
  await assert.rejects(h.run(), /digest mismatch/u)
  h.corrupt(false)
  assert.equal((await h.run()).reviews_newer_first[0].download.checked_as, h.f.reviewer_actor_id)
  assert.equal(h.requests.filter((r) => r.method === 'POST').length, posts)
})
