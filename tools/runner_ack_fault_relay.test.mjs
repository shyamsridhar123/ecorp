import test from 'node:test'
import assert from 'node:assert/strict'
import { randomUUID } from 'node:crypto'
import { createServer } from 'node:http'
import { once } from 'node:events'
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { createRequire } from 'node:module'
import { setTimeout as delay } from 'node:timers/promises'
import { AckFaultGate, validateConfig, startRelay } from './runner_ack_fault_relay.mjs'

function fixture(plan_key = 'visual-direction', kind = 'start_run') {
  let time = 0
  const events = []
  const sent = []
  const gate = new AckFaultGate({ now: () => time, record: (event, data) => events.push({ event, ...data }) })
  const binding = {
    kind, plan_key, corp_id: randomUUID(), mission_id: randomUUID(),
    task_id: randomUUID(), run_id: randomUUID(),
  }
  gate.bind(binding)
  const sha256 = 'ab'.repeat(32)
  gate.upload({
    type: 'run_event', event_type: 'run.deliverable_upload', run_id: binding.run_id,
    corp_id: binding.corp_id, payload: { sha256, content_base64: 'SENSITIVE' },
  })
  const ack = {
    type: 'artifact_stored', run_id: binding.run_id, artifact_id: randomUUID(),
    artifact_role: 'source_deliverable', sha256,
  }
  // Whitespace proves byte forwarding, not parse/serialize reconstruction.
  const bytes = Buffer.from(`  ${JSON.stringify(ack)}\n`)
  const send = (data, binary) => sent.push({ data, binary })
  const deliver = (frame = ack, data = bytes) => gate.serverFrame(frame, data, false, send)
  return { gate, binding, ack, bytes, send, deliver, sent, events, setTime: (v) => { time = v } }
}

test('withholds all six source ACKs for each of two native automatic attempts', () => {
  const f = fixture()
  for (let attempt = 0; attempt < 2; attempt++) {
    if (attempt) {
      f.binding.run_id = randomUUID()
      f.gate.bind({ ...f.binding })
      f.gate.upload({
        type: 'run_event', event_type: 'run.deliverable_upload',
        run_id: f.binding.run_id, corp_id: f.binding.corp_id,
        payload: { sha256: f.ack.sha256 },
      })
    }
    for (let retry = 0; retry < 6; retry++) {
      f.deliver({ ...f.ack, run_id: f.binding.run_id })
      f.setTime((attempt * 6 + retry) * 5000)
      f.gate.tick(f.send)
    }
  }
  assert.equal(f.sent.length, 0)
  assert.equal(f.gate.held.length, 12)
  assert.throws(() => f.gate.bind({ ...f.binding, run_id: randomUUID() }), /two native/)
})

test('holds all quality ACK duplicates to the same 8-second deadline, preserving bytes', () => {
  const f = fixture('quality-verification')
  f.deliver()
  f.setTime(5001)
  f.deliver()
  f.gate.tick(f.send)
  assert.equal(f.sent.length, 0)
  f.setTime(8000)
  f.gate.tick(f.send)
  assert.equal(f.sent.length, 2)
  assert.deepEqual(f.sent[0].data, f.bytes)
  assert.deepEqual(f.sent[1].data, f.bytes)
  assert.equal(f.sent[0].binary, false)
  f.deliver()
  assert.equal(f.sent.length, 3)
})

test('gameplay and integration pass source ACKs unchanged', () => {
  for (const key of ['gameplay-systems', 'studio-integration']) {
    const f = fixture(key)
    f.deliver()
    assert.deepEqual(f.sent[0].data, f.bytes)
    assert.equal(f.gate.held.length, 0)
  }
})

test('native resume passes ACKs and does not consume a third automatic attempt', () => {
  const f = fixture('visual-direction', 'resume_run')
  f.deliver()
  assert.deepEqual(f.sent[0].data, f.bytes)
})

test('non-source ACK, heartbeat, credential rotation, approval and lifecycle are opaque passthrough', () => {
  const f = fixture()
  for (const type of ['registered', 'heartbeat', 'stop_run', 'approval_decision', 'run.completed']) {
    const bytes = Buffer.from(JSON.stringify({ type, credential: 'DO_NOT_LOG' }))
    f.deliver({ type }, bytes)
    assert.deepEqual(f.sent.at(-1).data, bytes)
  }
  f.deliver({ ...f.ack, artifact_role: 'provider_evidence' })
  assert.equal(f.sent.length, 6)
  assert(!JSON.stringify(f.events).includes('DO_NOT_LOG'))
  assert(!JSON.stringify(f.events).includes('SENSITIVE'))
})

test('rejects foreign or unproven run, Corp and hash before fault decisions', () => {
  const f = fixture()
  assert.throws(() => f.deliver({ ...f.ack, run_id: randomUUID() }), /correlation/)
  assert.throws(() => f.deliver({ ...f.ack, sha256: 'cd'.repeat(32) }), /correlation/)
  assert.throws(() => f.gate.upload({
    type: 'run_event', event_type: 'run.deliverable_upload',
    run_id: f.binding.run_id, corp_id: randomUUID(), payload: { sha256: f.ack.sha256 },
  }), /Corp mismatch/)
  assert.equal(f.sent.length, 0)
})

test('assignment rebinding cannot swap a different task, scope or command kind', () => {
  const f = fixture()
  f.gate.bind(f.binding)
  assert.equal(f.gate.runs.size, 1)
  assert.throws(() => f.gate.bind({ ...f.binding, task_id: randomUUID() }), /identity changed/)
  assert.throws(() => f.gate.bind({ ...f.binding, kind: 'resume_run' }), /identity changed/)
})

test('late-ACK release sends one exact original old-run frame once without retagging', () => {
  const f = fixture()
  f.deliver()
  f.deliver()
  const control = {
    test_owned: true, disable_faults: true,
    release_one_old_ack: { command_id: randomUUID(), run_id: f.binding.run_id },
  }
  f.gate.control(control, f.send)
  f.gate.control(control, f.send)
  assert.equal(f.sent.length, 1)
  assert.deepEqual(f.sent[0].data, f.bytes)
  assert.equal(JSON.parse(f.sent[0].data).run_id, f.binding.run_id)
  assert.equal(f.gate.held.length, 1)
  f.deliver()
  assert.equal(f.sent.length, 2)
})

test('refuses manual-stack, wrong-repository and missing opt-in config', () => {
  const config = {
    test_owned: true, purpose: 'issue169-ack-acceptance',
    server_url: 'http://127.0.0.1:18961', listen_port: 18963,
    runner_id: 'issue169-ack-real-copilot',
    repository: 'shyamsridhar123/ecorp-enterprise-lab', source_base_ref: 'HEAD',
    source_base_commit: 'e3dc3d669b1a99832e2e7af9be16f7f39842586d',
    project_owner: 'shyamsridhar123', project_number: 3, issue_number: 2,
    corp_id: randomUUID(), actor_id: randomUUID(),
    output_root: pathForPlatform(),
  }
  assert.equal(validateConfig(config), config)
  for (const changes of [
    { test_owned: false }, { server_url: 'http://127.0.0.1:18962' },
    { repository: 'shyamsridhar123/ecorp' }, { listen_port: 15491 },
    { issue_number: 3 }, { listen_port: 0 },
    { server_url: 'http://127.0.0.1:0', synthetic: true },
  ]) assert.throws(() => validateConfig({ ...config, ...changes }))
  const fixedCandidate = {
    ...config, output_root: `${config.output_root}-fixed`,
    issue_number: 3, runner_id: 'issue169-ack-real-copilot-fixed',
  }
  assert.equal(validateConfig(fixedCandidate), fixedCandidate)
  assert.throws(() => validateConfig({ ...fixedCandidate, issue_number: 2 }))
  assert.throws(() => validateConfig({ ...fixedCandidate, runner_id: config.runner_id }))
})

function pathForPlatform() {
  return process.platform === 'win32'
    ? 'C:\\owned\\issue169-ack-acceptance-20260907'
    : '/owned/issue169-ack-acceptance-20260907'
}

function boundOrigin(server) {
  const address = server.address()
  assert.equal(server.listening, true)
  assert.equal(address?.address, '127.0.0.1')
  assert.equal(address.family, 'IPv4')
  assert(Number.isInteger(address.port) && address.port > 0)
  assert(![18961, 18963, 18962, 15491, 8791, 5432].includes(address.port))
  return `http://${address.address}:${address.port}`
}

test('real loopback WebSocket wiring: native schema, ordered forwarding, reconnection and redaction',
  { timeout: 15_000, concurrency: 2 }, async (t) => {
    const fixtures = []
    let allBound
    const ready = new Promise((resolve) => { allBound = resolve })
    const register = (fixture) => {
      fixtures.push(fixture)
      if (fixtures.length === 2) allBound()
      return ready
    }
    await Promise.all([1, 2].map((index) => t.test(`isolated concurrent fixture ${index}`,
      (child) => verifyLoopbackTransport(child, fixtures, register))))
    assert.equal(fixtures.length, 2)
    assert.equal(new Set(fixtures.flatMap((f) => [f.serverUrl, f.relayUrl])).size, 4)
    assert.equal(new Set(fixtures.map((f) => f.output)).size, 2)
    for (const { stub, relay } of fixtures) {
      assert.equal(stub.listening, false)
      assert.equal(relay.server.listening, false)
      assert.equal(stub.address(), null)
      assert.equal(relay.server.address(), null)
    }
  })

async function verifyLoopbackTransport(t, fixtures, register) {
  // This is synthetic transport verification only, not server/store/provider acceptance.
  // Both listeners bind port 0 and remain owned until teardown; no port reservation race.
  const { WebSocket, WebSocketServer } = createRequire(import.meta.url)('ws')
  const temporary = await mkdtemp(path.join(tmpdir(), 'ecorp-ack-relay-unit-'))
  let stub, stubSockets, relay
  let releaseSnapshot = () => {}
  const clients = []
  t.after(async () => {
    releaseSnapshot()
    for (const client of clients) client.ws.terminate()
    if (stubSockets) for (const peer of stubSockets.clients) peer.terminate()
    if (relay) await relay.close()
    if (stubSockets) await new Promise((resolve) => stubSockets.close(resolve))
    if (stub) await new Promise((resolve) => stub.close(resolve))
    const absolute = path.resolve(temporary)
    assert(absolute.startsWith(`${path.resolve(tmpdir())}${path.sep}`))
    assert(path.basename(absolute).startsWith('ecorp-ack-relay-unit-'))
    await rm(absolute, { recursive: true })
  })
  const output = path.join(temporary, 'issue169-ack-acceptance-20260907')
  await mkdir(path.join(output, 'evidence'), { recursive: true })
  await writeFile(path.join(output, 'relay-control.json'), '{"test_owned":true}')
  const config = {
    test_owned: true, purpose: 'issue169-ack-acceptance',
    listen_port: 0,
    runner_id: 'issue169-ack-real-copilot', repository: 'shyamsridhar123/ecorp-enterprise-lab',
    source_base_ref: 'HEAD', source_base_commit: 'e3dc3d669b1a99832e2e7af9be16f7f39842586d',
    project_owner: 'shyamsridhar123', project_number: 3, issue_number: 2,
    corp_id: randomUUID(), actor_id: randomUUID(), output_root: output,
  }
  const mission = randomUUID()
  const task = randomUUID()
  const agent = randomUUID()
  const snapshot = {
    snapshot: {
      factory_work_items: [{
        id: randomUUID(), mission_id: mission, source_repository_owner: 'shyamsridhar123',
        source_repository_name: 'ecorp-enterprise-lab', source_issue_number: 2,
        source_project_owner: 'shyamsridhar123', source_project_number: 3,
      }],
      tasks: [{
        id: task, corp_id: config.corp_id, mission_id: mission,
        assigned_agent_id: agent, plan_key: 'visual-direction',
      }],
    },
  }
  let snapshotGate = Promise.resolve()
  let snapshotRequests = 0
  stub = createServer(async (request, response) => {
    assert.equal(request.url,
      `/api/corps/${config.corp_id}/snapshot?actor_id=${config.actor_id}`)
    snapshotRequests++
    await snapshotGate
    response.writeHead(200, { 'content-type': 'application/json' })
    response.end(JSON.stringify(snapshot))
  })
  stubSockets = new WebSocketServer({ server: stub, path: '/ws/runner' })
  const peers = []
  const forwarded = []
  stubSockets.on('connection', (peer) => {
    peers.push(peer)
    peer.on('message', (bytes) => {
      forwarded.push(Buffer.from(bytes))
      if (JSON.parse(bytes).type === 'register') {
        peer.send(' {"type":"registered","credential":"SYNTHETIC_DO_NOT_LOG"} ')
      }
    })
  })
  await new Promise((resolve, reject) => {
    stub.once('error', reject)
    stub.listen(0, '127.0.0.1', resolve)
  })
  config.server_url = boundOrigin(stub)
  const options = { syntheticUpstream: stub }
  relay = await startRelay(config, options)
  const relayUrl = boundOrigin(relay.server)
  const until = async (predicate) => {
    const deadline = Date.now() + 3000
    while (!predicate()) {
      assert(Date.now() < deadline, 'Transport assertion deadline')
      await delay(10)
    }
  }
  // Keep both pairs live together: serial reuse of an ephemeral port cannot pass this check.
  await register({ serverUrl: config.server_url, relayUrl, output, corpId: config.corp_id, stub, relay })
  const other = fixtures.find((f) => f.relay !== relay)
  assert.equal(other.stub.listening, true)
  assert.equal(other.relay.server.listening, true)
  assert.notEqual(relay.gate, other.relay.gate)
  await t.test('synthetic endpoints require live ownership without relaxing config guards', async () => {
    assert.equal(validateConfig(config, options), config)
    assert.throws(() => validateConfig(config))
    assert.throws(() => validateConfig({ ...config, synthetic: true }))
    for (const syntheticUpstream of [null, {}, createServer()]) {
      assert.throws(() => validateConfig(config, { syntheticUpstream }), /already-listening/)
    }
    for (const port of [18961, 18963, 18962, 15491, 8791, 5432]) {
      assert.throws(() => validateConfig({ ...config, server_url: `http://127.0.0.1:${port}` }, options))
      assert.throws(() => validateConfig({ ...config, listen_port: port }, options))
    }
    const port = stub.address().port
    for (const server_url of [
      other.serverUrl, relayUrl, `http://localhost:${port}`, `http://127.0.0.2:${port}`,
      `https://127.0.0.1:${port}`, `http://user:secret@127.0.0.1:${port}`,
      `${config.server_url}/`, `${config.server_url}?unexpected=1`,
    ]) assert.throws(() => validateConfig({ ...config, server_url }, options))
    for (const changes of [
      { test_owned: false }, { purpose: 'synthetic' }, { repository: 'shyamsridhar123/ecorp' },
      { source_base_ref: 'main' }, { source_base_commit: '0'.repeat(40) },
      { project_owner: 'other' }, { project_number: 4 }, { issue_number: 3 },
      { runner_id: 'other-runner' }, { corp_id: 'invalid' }, { actor_id: 'invalid' },
      { output_root: temporary },
    ]) await assert.rejects(startRelay({ ...config, ...changes }, options))
  })
  const connect = async () => {
    const ws = new WebSocket(`${relayUrl.replace('http:', 'ws:')}/ws/runner`)
    const received = []
    ws.on('message', (data) => received.push(Buffer.from(data)))
    const client = { ws, received }
    clients.push(client)
    await once(ws, 'open')
    ws.send(JSON.stringify({
      type: 'register', runner_id: config.runner_id, corp_id: config.corp_id,
      credential: 'SYNTHETIC_DO_NOT_LOG',
    }))
    await until(() => received.length > 0)
    return client
  }
  const assignment = (runId) => ({
    type: 'start_run', run_id: runId, task_id: task, mission_id: mission,
    corp_id: config.corp_id, agent_id: agent, adapter: 'github-copilot',
    source_repository: config.repository, source_base_ref: 'HEAD',
    source_base_commit: config.source_base_commit,
    assignment_token: 'SYNTHETIC_ASSIGNMENT_DO_NOT_LOG',
  })
  const first = await connect()
  await t.test('persisted owner/name schema binds and stop cannot overtake a pending assignment', async () => {
    snapshotGate = new Promise((resolve) => { releaseSnapshot = resolve })
    const start = Buffer.from(` ${JSON.stringify(assignment(randomUUID()))}\n`)
    const stop = Buffer.from('  {"type":"stop_run","opaque":"DO_NOT_LOG_STOP"}\n')
    peers[0].send(start, { binary: false })
    peers[0].send(stop, { binary: false })
    await until(() => snapshotRequests === 1)
    assert.equal(first.received.length, 1)
    releaseSnapshot()
    await until(() => first.received.length === 3)
    assert.deepEqual(first.received[1], start)
    assert.deepEqual(first.received[2], stop)
    assert.equal(relay.status.failed, false)
    assert.equal(relay.status.runs.length, 1)
  })
  await t.test('upload byte transparency, ACK withholding and metadata non-disclosure', async () => {
    const runId = relay.status.runs[0].run_id
    const upload = Buffer.from(JSON.stringify({
      type: 'run_event', event_type: 'run.deliverable_upload',
      runner_id: config.runner_id, corp_id: config.corp_id, run_id: runId,
      assignment_token: 'SYNTHETIC_ASSIGNMENT_DO_NOT_LOG',
      payload: { sha256: 'ab'.repeat(32), content_base64: 'SYNTHETIC_PAYLOAD_DO_NOT_LOG' },
    }))
    first.ws.send(upload, { binary: false })
    await until(() => forwarded.some((bytes) => bytes.equals(upload)))
    const ack = Buffer.from(JSON.stringify({
      type: 'artifact_stored', run_id: runId, artifact_id: randomUUID(),
      artifact_role: 'source_deliverable', sha256: 'ab'.repeat(32),
    }))
    peers[0].send(ack, { binary: false })
    await until(() => relay.gate.held.length === 1)
    assert.equal(first.received.length, 3)
    first.ws.send(JSON.stringify({
      type: 'run_event', runner_id: config.runner_id, corp_id: config.corp_id,
      run_id: 'MALFORMED_IDENTIFIER_DO_NOT_LOG', event_type: 'MALFORMED_EVENT_DO_NOT_LOG',
    }))
    await until(() => forwarded.length === 3)
    const log = await readFile(path.join(output, 'evidence', 'ack-transport.jsonl'), 'utf8')
    assert(!log.includes('DO_NOT_LOG'))
    assert.equal((await fetch(`${relayUrl}/index.html`)).status, 404)
  })
  await t.test('a stale snapshot/close callback cannot bind, send or close the replacement connection', async () => {
    snapshotGate = new Promise((resolve) => { releaseSnapshot = resolve })
    peers[0].send(JSON.stringify(assignment(randomUUID())))
    await until(() => snapshotRequests === 2)
    const closed = once(first.ws, 'close')
    first.ws.close()
    await closed
    await until(() => !relay.status.connected)
    const replacement = await connect()
    releaseSnapshot()
    const marker = Buffer.from(' {"type":"heartbeat","opaque":"replacement-alive"}\n')
    peers[1].send(marker, { binary: false })
    await until(() => replacement.received.length === 2)
    await delay(50)
    assert.equal(replacement.ws.readyState, WebSocket.OPEN)
    assert.deepEqual(replacement.received[1], marker)
    assert.equal(relay.status.runs.length, 1)
    assert.equal(relay.status.failed, false)
  })
  assert.deepEqual(relay.status.runs.map((run) => run.corp_id), [config.corp_id])
  const log = await readFile(path.join(output, 'evidence', 'ack-transport.jsonl'), 'utf8')
  assert(log.includes(config.corp_id))
  assert(!log.includes(other.corpId), 'Concurrent fixture metadata must stay isolated')
}
