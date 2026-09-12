// Opt-in, loopback-only QA transport fixture. Never serves workspace or game files.
// Only original source_deliverable ACK frames can be withheld. All provider work,
// artifact uploads, timeout/retry decisions, and state transitions remain native.
import assert from 'node:assert/strict'
import { createServer, Server } from 'node:http'
import { appendFileSync, readFileSync, writeFileSync } from 'node:fs'
import { pathToFileURL } from 'node:url'
import path from 'node:path'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const uuid = /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i
const digest = /^[0-9a-f]{64}$/i
const keys = new Set([
  'visual-direction', 'gameplay-systems', 'quality-verification', 'studio-integration',
])
const recordedEventTypes = new Set([
  'run.failed', 'run.completed', 'run.cancelled', 'run.started', 'run.session',
  'run.session_terminated', 'run.teardown_uncertain', 'run.workspace_preserved',
  'run.workspace_removed', 'run.verification_evidence', 'run.verification_failed',
  'run.verification_passed', 'run.verification_started', 'run.verification_waiting',
])

export class AckFaultGate {
  constructor({ now = Date.now, record = () => {}, delayMs = 8000 } = {}) {
    assert(delayMs > 5000 && delayMs < 25000, 'Delay must cross one native ACK window')
    this.now = now
    this.record = record
    this.delayMs = delayMs
    this.runs = new Map()
    this.held = []
    this.disabled = false
    this.releasedCommands = new Set()
  }

  bind(binding) {
    for (const id of ['run_id', 'task_id', 'mission_id', 'corp_id']) {
      assert(uuid.test(binding[id]), `Invalid ${id}`)
    }
    assert(keys.has(binding.plan_key), 'Unknown Studio task')
    assert(['start_run', 'resume_run'].includes(binding.kind), 'Unknown assignment kind')
    const prior = this.runs.get(binding.run_id)
    if (prior) {
      for (const name of ['task_id', 'mission_id', 'corp_id', 'plan_key', 'kind']) {
        assert.equal(prior[name], binding[name], 'Assignment identity changed')
      }
      return
    }
    assert(this.runs.size < 12, 'Owned probe run bound exceeded')
    if (binding.plan_key === 'visual-direction' && binding.kind === 'start_run') {
      assert(
        [...this.runs.values()].filter((r) =>
          r.plan_key === 'visual-direction' && r.kind === 'start_run').length < 2,
        'More than two native automatic attempts',
      )
    }
    this.runs.set(binding.run_id, { ...binding, hashes: new Set(), delayUntil: null })
    this.record('assignment_bound', binding)
  }

  upload(frame) {
    if (frame.type !== 'run_event' || frame.event_type !== 'run.deliverable_upload') return
    const run = this.runs.get(frame.run_id)
    assert(run, 'Deliverable for unbound run')
    assert.equal(frame.corp_id, run.corp_id, 'Deliverable Corp mismatch')
    assert(digest.test(frame.payload?.sha256), 'Invalid deliverable digest')
    run.hashes.add(frame.payload.sha256)
    this.record('deliverable_upload', {
      run_id: frame.run_id, task_id: run.task_id, plan_key: run.plan_key,
      sha256: frame.payload.sha256,
    })
  }

  serverFrame(frame, bytes, isBinary, send) {
    if (frame.type !== 'artifact_stored' || frame.artifact_role !== 'source_deliverable') {
      send(bytes, isBinary)
      return
    }
    const run = this.runs.get(frame.run_id)
    assert(run && run.hashes.has(frame.sha256), 'ACK lacks exact run/upload correlation')
    assert(uuid.test(frame.artifact_id), 'Invalid artifact identifier')
    const metadata = {
      run_id: frame.run_id, task_id: run.task_id, plan_key: run.plan_key,
      artifact_id: frame.artifact_id, sha256: frame.sha256,
    }
    if (!this.disabled && run.kind === 'start_run' && run.plan_key === 'visual-direction') {
      assert(this.held.length < 80, 'Held ACK bound exceeded')
      this.held.push({ bytes: Buffer.from(bytes), isBinary, metadata, due: null })
      this.record('ack_dropped', metadata)
      return
    }
    if (!this.disabled && run.kind === 'start_run' && run.plan_key === 'quality-verification') {
      run.delayUntil ??= this.now() + this.delayMs
      if (this.now() < run.delayUntil) {
        assert(this.held.length < 80, 'Held ACK bound exceeded')
        this.held.push({
          bytes: Buffer.from(bytes), isBinary, metadata, due: run.delayUntil,
        })
        this.record('ack_delayed', { ...metadata, release_at_ms: run.delayUntil })
        return
      }
    }
    send(bytes, isBinary)
    this.record('ack_forwarded', metadata)
  }

  tick(send) {
    this.held = this.held.filter((ack) => {
      if (ack.due === null || ack.due > this.now()) return true
      send(ack.bytes, ack.isBinary)
      this.record('delayed_ack_released', ack.metadata)
      return false
    })
  }

  control(control, send) {
    assert.equal(control.test_owned, true)
    if (control.disable_faults === true && !this.disabled) {
      this.disabled = true
      this.record('faults_disabled', {})
    }
    if (!control.release_one_old_ack) return
    const { command_id, run_id } = control.release_one_old_ack
    assert(uuid.test(command_id) && uuid.test(run_id), 'Invalid late-ACK command')
    if (this.releasedCommands.has(command_id)) return
    const index = this.held.findIndex((ack) =>
      ack.due === null && ack.metadata.run_id === run_id)
    assert(index >= 0, 'No original withheld ACK for exact requested run')
    const [ack] = this.held.splice(index, 1)
    send(ack.bytes, ack.isBinary)
    this.releasedCommands.add(command_id)
    this.record('old_ack_released', { ...ack.metadata, command_id })
  }
}

// Import-only seam: synthetic tests retain their bound upstream, never probe/reopen
// a free port or opt a JSON/CLI config into arbitrary transport endpoints.
function syntheticOrigin(upstream) {
  assert(upstream instanceof Server && upstream.listening,
    'Synthetic upstream must be an already-listening HTTP server')
  const address = upstream.address()
  assert.equal(address?.address, '127.0.0.1')
  assert.equal(address.family, 'IPv4')
  assert(Number.isInteger(address.port) && address.port >= 1024 && address.port <= 65535 &&
    ![18961, 18963, 18962, 15491, 8791, 5432].includes(address.port),
    'Synthetic upstream must not use QA/manual/default ports')
  return `http://${address.address}:${address.port}`
}

export function validateConfig(config, { syntheticUpstream } = {}) {
  assert.equal(config.test_owned, true)
  assert.equal(config.purpose, 'issue169-ack-acceptance')
  assert(path.isAbsolute(config.output_root))
  const directory = path.basename(config.output_root)
  const fixedCandidate = directory === 'issue169-ack-acceptance-20260907-fixed'
  assert.equal(directory, fixedCandidate
    ? 'issue169-ack-acceptance-20260907-fixed'
    : 'issue169-ack-acceptance-20260907')
  assert.equal(config.server_url, syntheticUpstream === undefined
    ? 'http://127.0.0.1:18961' : syntheticOrigin(syntheticUpstream))
  assert.equal(config.listen_port, syntheticUpstream === undefined ? 18963 : 0)
  assert.equal(config.runner_id, fixedCandidate
    ? 'issue169-ack-real-copilot-fixed' : 'issue169-ack-real-copilot')
  assert.equal(config.repository, 'shyamsridhar123/ecorp-enterprise-lab')
  assert.equal(config.source_base_ref, 'HEAD')
  assert.equal(config.source_base_commit, 'e3dc3d669b1a99832e2e7af9be16f7f39842586d')
  assert.equal(config.project_owner, 'shyamsridhar123')
  assert.equal(config.project_number, 3)
  assert.equal(config.issue_number, fixedCandidate ? 3 : 2)
  assert(uuid.test(config.corp_id) && uuid.test(config.actor_id))
  return config
}

export async function startRelay(input, options) {
  const config = validateConfig(input, options)
  const { WebSocket, WebSocketServer } = require('ws')
  const receipt = path.join(config.output_root, 'evidence', 'ack-transport.jsonl')
  const statusPath = path.join(config.output_root, 'evidence', 'relay-status.json')
  const controlPath = path.join(config.output_root, 'relay-control.json')
  const status = {
    test_owned: true, connected: false, failed: false, counts: {},
    mission_id: null, factory_id: null, runs: [],
  }
  const record = (event, metadata) => {
    status.counts[event] = (status.counts[event] ?? 0) + 1
    if (event === 'assignment_bound') status.runs.push(metadata)
    appendFileSync(receipt, `${JSON.stringify({ at: new Date().toISOString(), event, ...metadata })}\n`)
    writeFileSync(statusPath, JSON.stringify(status, null, 2))
  }
  const gate = new AckFaultGate({ record })
  let active = null
  const connections = new Set()
  const isCurrent = (connection) => Boolean(connection && active === connection && !connection.closed)
  const sendOn = (connection, bytes, binary) => {
    assert(isCurrent(connection) && connection.client.readyState === WebSocket.OPEN,
      'Owned runner is disconnected')
    connection.client.send(bytes, { binary })
  }
  const sendClient = (bytes, binary) => sendOn(active, bytes, binary)
  const closeConnection = (connection) => {
    if (connection.closed) return
    connection.closed = true
    connection.pending = []
    connection.client.close()
    connection.upstream.close()
    if (active === connection) {
      active = null
      status.connected = false
      record('runner_disconnected', {})
    }
  }
  const fail = (connection = active) => {
    // Deliberately do not print Error.message: it can contain a raw frame.
    if (connection && !isCurrent(connection)) {
      closeConnection(connection)
      return
    }
    if (status.failed) return
    status.failed = true
    status.connected = false
    record('fixture_failed_closed', {})
    if (connection) closeConnection(connection)
  }
  const server = createServer((request, response) => {
    if (request.method === 'GET' && request.url === '/health') {
      response.writeHead(status.failed ? 503 : 200, { 'content-type': 'application/json' })
      response.end(JSON.stringify({
        status: status.failed ? 'failed' : 'ok',
        connected: status.connected, test_owned: true,
      }))
    } else {
      response.writeHead(404)
      response.end()
    }
  })
  const sockets = new WebSocketServer({ noServer: true, maxPayload: 64 * 1024 * 1024 })
  server.on('upgrade', (request, socket, head) => {
    if (request.url !== '/ws/runner' ||
        request.socket.remoteAddress !== '127.0.0.1' ||
        status.failed || (active && active.client.readyState < WebSocket.CLOSING)) {
      socket.destroy()
      return
    }
    sockets.handleUpgrade(request, socket, head, (ws) => sockets.emit('connection', ws))
  })
  const bindAssignment = async (connection, frame) => {
    assert.equal(frame.corp_id, config.corp_id)
    assert.equal(frame.adapter, 'github-copilot')
    assert.equal(frame.source_repository, config.repository)
    assert.equal(frame.source_base_ref, config.source_base_ref)
    assert.equal(frame.source_base_commit, config.source_base_commit)
    const response = await fetch(
      `${config.server_url}/api/corps/${config.corp_id}/snapshot?actor_id=${config.actor_id}`,
      { signal: AbortSignal.timeout(5000) },
    )
    assert(response.ok, 'Assignment snapshot unavailable')
    const state = (await response.json()).snapshot
    if (!isCurrent(connection)) return
    const item = state.factory_work_items.find((entry) =>
      entry.mission_id === frame.mission_id &&
      `${entry.source_repository_owner}/${entry.source_repository_name}` === config.repository &&
      entry.source_issue_number === config.issue_number &&
      entry.source_project_owner === config.project_owner &&
      entry.source_project_number === config.project_number)
    assert(item, 'Assignment is not from the exact owned issue/Project')
    const task = state.tasks.find((entry) => entry.id === frame.task_id)
    assert(task && task.corp_id === frame.corp_id && task.mission_id === frame.mission_id)
    assert.equal(task.assigned_agent_id, frame.agent_id)
    assert(keys.has(task.plan_key))
    if (status.mission_id) assert.equal(status.mission_id, frame.mission_id)
    status.mission_id = frame.mission_id
    status.factory_id = item.id
    gate.bind({
      kind: frame.type, corp_id: frame.corp_id, mission_id: frame.mission_id,
      task_id: frame.task_id, run_id: frame.run_id, plan_key: task.plan_key,
    })
  }
  sockets.on('connection', (ws) => {
    if (active) closeConnection(active)
    let registered = false
    const upstream = new WebSocket(`${config.server_url.replace('http:', 'ws:')}/ws/runner`, {
      maxPayload: 64 * 1024 * 1024,
    })
    const connection = {
      client: ws, upstream, pending: [], closed: false, downstream: Promise.resolve(),
    }
    active = connection
    connections.add(connection)
    const sendUpstream = (bytes, binary) => {
      if (!isCurrent(connection)) return
      if (upstream.readyState === WebSocket.OPEN) upstream.send(bytes, { binary })
      else {
        assert(connection.pending.length < 10, 'Registration queue bound exceeded')
        connection.pending.push({ bytes: Buffer.from(bytes), binary })
      }
    }
    ws.on('message', (bytes, binary) => {
      if (!isCurrent(connection)) return
      try {
        const frame = JSON.parse(bytes.toString())
        if (!registered) {
          assert.equal(frame.type, 'register')
          assert.equal(frame.runner_id, config.runner_id)
          assert.equal(frame.corp_id, config.corp_id)
          registered = true
          record('owned_registration_forwarded', {})
        } else {
          assert.equal(frame.runner_id, config.runner_id)
          if (frame.corp_id !== undefined) assert.equal(frame.corp_id, config.corp_id)
        }
        gate.upload(frame)
        if (frame.type === 'run_event' && uuid.test(frame.run_id) &&
            gate.runs.has(frame.run_id) && recordedEventTypes.has(frame.event_type)) {
          record('native_runner_event', { run_id: frame.run_id, event_type: frame.event_type })
        }
        sendUpstream(bytes, binary)
      } catch { fail(connection) }
    })
    upstream.on('open', () => {
      if (!isCurrent(connection)) { upstream.close(); return }
      for (const frame of connection.pending) upstream.send(frame.bytes, { binary: frame.binary })
      connection.pending = []
      status.connected = true
      record('upstream_connected', {})
    })
    upstream.on('message', (bytes, binary) => {
      // Preserve native command order while the low-frequency assignment lookup
      // validates persisted task identity. No timer/fault delay applies here.
      connection.downstream = connection.downstream.then(async () => {
        if (!isCurrent(connection)) return
        try {
          const frame = JSON.parse(bytes.toString())
          if (['start_run', 'resume_run'].includes(frame.type)) await bindAssignment(connection, frame)
          if (!isCurrent(connection)) return
          gate.serverFrame(frame, bytes, binary,
            (original, isBinary) => sendOn(connection, original, isBinary))
        } catch { fail(connection) }
      })
    })
    ws.on('close', () => {
      closeConnection(connection)
      connections.delete(connection)
    })
    upstream.on('close', () => closeConnection(connection))
    ws.on('error', () => fail(connection))
    upstream.on('error', () => fail(connection))
  })
  const timer = setInterval(() => {
    if (status.failed || !status.connected || active?.client.readyState !== WebSocket.OPEN) return
    try {
      gate.tick(sendClient)
      const control = JSON.parse(readFileSync(controlPath, 'utf8').replace(/^\uFEFF/, ''))
      gate.control(control, sendClient)
    } catch { fail() }
  }, 250)
  timer.unref()
  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(config.listen_port, '127.0.0.1', resolve)
  })
  record('relay_listening', {})
  return { server, sockets, gate, status, close: async () => {
    clearInterval(timer)
    for (const connection of connections) closeConnection(connection)
    await Promise.all([
      new Promise((resolve) => sockets.close(resolve)),
      new Promise((resolve) => server.close(resolve)),
    ])
  } }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    assert.equal(process.env.CRONY_ACK_TEST, '1',
      'Explicit CRONY_ACK_TEST=1 required')
    assert.equal(process.argv.length, 3, 'Expected one owned config path')
    const config = JSON.parse(readFileSync(process.argv[2], 'utf8').replace(/^\uFEFF/, ''))
    await startRelay(config)
    console.log('Owned runner ACK relay listening; no file hosting or synthetic lifecycle events.')
  } catch {
    console.error('ACK fixture startup refused; inspect its owned metadata configuration.')
    process.exitCode = 1
  }
}
