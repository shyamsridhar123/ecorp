import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import {
  existsSync,
  openSync,
  readFileSync,
  writeFileSync,
} from 'node:fs'
import { writeFile } from 'node:fs/promises'
import path from 'node:path'

const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const socketBase = server.replace(/^http/, 'ws')
const root = path.resolve(import.meta.dirname, '..')
const databaseUrl =
  process.env.DATABASE_URL ?? 'postgres://crony:crony@127.0.0.1:54329/crony'

async function request(url, init) {
  const response = await fetch(`${server}${url}`, init)
  const body = response.status === 204 ? null : await response.json()
  return { response, body }
}

function post(url, body) {
  return request(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
}

async function postOk(url, body) {
  const result = await post(url, body)
  if (!result.response.ok) {
    throw new Error(`${result.response.status}: ${JSON.stringify(result.body)}`)
  }
  return result.body
}

async function snapshot(corpId, actorId) {
  const result = await request(
    `/api/corps/${corpId}/snapshot?actor_id=${actorId}`,
  )
  if (!result.response.ok) {
    throw new Error(`${result.response.status}: ${JSON.stringify(result.body)}`)
  }
  return result.body
}

async function waitFor(corpId, actorId, predicate, description, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const state = await snapshot(corpId, actorId)
    const value = predicate(state)
    if (value) return { state, value }
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error(`timed out waiting for ${description}`)
}

async function openBrowserClient(corpId, actorId, afterSeq = 0) {
  const events = []
  let replayedThrough = afterSeq
  let closeReason = null
  let readyResolve
  let readyReject
  let closeResolve
  const ready = new Promise((resolve, reject) => {
    readyResolve = resolve
    readyReject = reject
  })
  const closed = new Promise((resolve) => {
    closeResolve = resolve
  })
  const socket = new WebSocket(
    `${socketBase}/ws/corps/${corpId}?actor_id=${actorId}&after_seq=${afterSeq}`,
  )
  const timeout = setTimeout(() => {
    socket.close()
    readyReject(new Error(`browser ${actorId} timed out waiting for replay`))
  }, 10_000)
  socket.onerror = () => {
    clearTimeout(timeout)
    if (replayedThrough === afterSeq && events.length === 0) {
      readyReject(new Error(`browser ${actorId} websocket failed`))
    }
  }
  socket.onclose = (event) => {
    closeReason = `${event.code}:${event.reason}`
    closeResolve(closeReason)
  }
  socket.onmessage = (message) => {
    const payload = JSON.parse(message.data)
    if (payload.type === 'event') {
      events.push(payload.event)
      replayedThrough = Math.max(replayedThrough, payload.event.seq)
      return
    }
    if (payload.type === 'ready') {
      clearTimeout(timeout)
      replayedThrough = Math.max(replayedThrough, payload.replayed_through)
      readyResolve()
    }
  }
  await ready
  return {
    actorId,
    events,
    socket,
    closed,
    closeReason: () => closeReason,
    cursor: () => replayedThrough,
  }
}

async function waitForBrowserEvent(client, predicate, description, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const value = client.events.find(predicate)
    if (value) return value
    await new Promise((resolve) => setTimeout(resolve, 50))
  }
  throw new Error(`timed out waiting for browser event: ${description}`)
}

async function restartLocalServer() {
  const pidPath = process.env.CRONY_TEST_SERVER_PID_FILE
  if (!pidPath || !existsSync(pidPath)) {
    throw new Error('CRONY_TEST_SERVER_PID_FILE must identify the test-owned server')
  }
  const jsonPidFile = pidPath.endsWith('.json')
  const pidState = jsonPidFile
    ? JSON.parse(readFileSync(pidPath, 'utf8'))
    : { server: Number(readFileSync(pidPath, 'utf8').trim()) }
  const serverPid = Number(pidState.server)
  if (!Number.isSafeInteger(serverPid) || serverPid <= 0) {
    throw new Error(`test-owned server PID is invalid: ${serverPid}`)
  }

  process.kill(serverPid, 0)
  process.kill(serverPid)
  await new Promise((resolve) => setTimeout(resolve, 500))

  const serverUrl = new URL(server)
  const binary =
    process.env.CRONY_TEST_SERVER_BINARY ??
    path.join(
      root,
      'target',
      'debug',
      process.platform === 'win32' ? 'crony-server.exe' : 'crony-server',
    )
  const logDir = path.dirname(path.resolve(pidPath))
  const stdout = openSync(path.join(logDir, 'cockpit-server-restart.stdout.log'), 'a')
  const stderr = openSync(path.join(logDir, 'cockpit-server-restart.stderr.log'), 'a')
  const child = spawn(
    binary,
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
  if (jsonPidFile) {
    writeFileSync(
      pidPath,
      `${JSON.stringify({ ...pidState, server: child.pid }, null, 2)}\n`,
    )
  } else {
    writeFileSync(pidPath, `${child.pid}\n`)
  }
  child.unref()

  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    try {
      const health = await fetch(`${server}/health`).then((response) =>
        response.json(),
      )
      if (health.status === 'ok' && health.runners >= 1) return child.pid
    } catch {
      // The server is restarting or the runner is reconnecting.
    }
    await new Promise((resolve) => setTimeout(resolve, 200))
  }
  throw new Error('server and runner did not recover after cockpit restart')
}

function assertUniqueSequences(events, minimumExclusive) {
  const sequences = events.map((event) => event.seq)
  assert.equal(new Set(sequences).size, sequences.length)
  assert.ok(sequences.every((sequence) => sequence > minimumExclusive))
  assert.ok(
    sequences.every(
      (sequence, index) => index === 0 || sequence > sequences[index - 1],
    ),
  )
}

const demo = await postOk('/api/demo/reset', {})
const initial = await snapshot(demo.corp_id, demo.alice_actor_id)
const connectedRunner = initial.runners.find(
  (runner) =>
    runner.connected &&
    runner.capabilities.some(
      (capability) =>
        capability.name === 'workspace-isolation' &&
        capability.available &&
        capability.source_repository &&
        capability.source_base_ref &&
        capability.source_base_commit,
    ),
)
assert.ok(connectedRunner, 'a connected runner must advertise an immutable source checkout')
const workspace = connectedRunner.capabilities.find(
  (capability) =>
    capability.name === 'workspace-isolation' &&
    capability.available &&
    capability.source_repository &&
    capability.source_base_ref &&
    capability.source_base_commit,
)
assert.ok(workspace, 'a connected runner must advertise an immutable source checkout')
const repositoryParts = workspace.source_repository.split('/')
assert.equal(repositoryParts.length, 2)
const [repositoryOwner, repositoryName] = repositoryParts

const aliceBrowser = await openBrowserClient(
  demo.corp_id,
  demo.alice_actor_id,
)
const bobBrowser = await openBrowserClient(
  demo.corp_id,
  demo.bob_actor_id,
)

const nonce = crypto.randomUUID()
const controllerId = crypto.randomUUID()
const firstEpoch = crypto.randomUUID()
const controllerPath = `/api/corps/${demo.corp_id}/factory/controllers`
const configured = await postOk(controllerPath, {
  actor_id: demo.alice_actor_id,
  controller_id: controllerId,
  source_project_owner: repositoryOwner,
  source_project_number: 145,
  source_repository_owner: repositoryOwner,
  source_repository_name: repositoryName,
  connection_epoch: firstEpoch,
  lease_seconds: 60,
  idempotency_key: `cockpit-controller-${nonce}`,
})
assert.equal(configured.controller.status, 'watching')

const claimPath = `/api/corps/${demo.corp_id}/factory/work-items/claim`
const claimRequest = {
  actor_id: demo.alice_actor_id,
  source_project_owner: repositoryOwner,
  source_project_number: 145,
  source_project_item_id: `PVTI_COCKPIT_${nonce}`,
  source_repository_owner: repositoryOwner,
  source_repository_name: repositoryName,
  source_issue_number: 145,
  source_issue_node_id: `I_COCKPIT_${nonce}`,
  source_issue_url: `https://github.com/${workspace.source_repository}/issues/145`,
  source_title: 'Prove the multiplayer dark-factory cockpit survives reconnects',
  source_revision: '2026-09-04T20:00:00Z',
  idempotency_key: `cockpit-claim-${nonce}`,
  lease_seconds: 300,
  policy: {
    schema_version: 1,
    source_of_truth: 'github_project',
    project_status: 'Todo',
    repository_allowlist: [workspace.source_repository],
    source_base_ref: workspace.source_base_ref,
    source_base_commit: workspace.source_base_commit,
    adapter_allowlist: ['fake-process'],
    strategy_allowlist: ['single'],
    model: null,
    reasoning_effort: null,
    write_scope: ['**'],
    allowed_tools: ['filesystem', 'shell'],
    prohibited_actions: [
      'modify files outside the assigned worktree',
      'merge, auto-merge, publish, or deploy without separate authority',
    ],
    secret_ids: [],
    verification_required: true,
    budget_tokens: 20_000,
    budget_cost_microusd: 1_000_000,
    auto_merge: false,
  },
}
const claim = await postOk(claimPath, claimRequest)
const claimReplay = await postOk(claimPath, claimRequest)
assert.equal(claimReplay.replayed, true)
assert.equal(claimReplay.work_item.id, claim.work_item.id)

const materializePath =
  `/api/corps/${demo.corp_id}/factory/work-items/` +
  `${claim.work_item.id}/materialize`
const materializeRequest = {
  actor_id: demo.alice_actor_id,
  claim_token: claim.claim_token,
  expected_version: claim.work_item.version,
  idempotency_key: `cockpit-materialize-${nonce}`,
  title: '[slow] [approval-action] Operate one work item through the shared cockpit.',
  preferred_adapter: 'fake-process',
  strategy: 'single',
  budget_tokens: 20_000,
  budget_cost_microusd: 1_000_000,
  contract: {
    objective:
      'Prove comments, control, decisions, and execution survive client and server reconnects.',
    expected_output: 'One verified result with preserved actor attribution.',
    acceptance_tests: [
      'Alice and Bob observe the same work item',
      'the same run survives server restart',
      'duplicate requests do not duplicate effects',
    ],
    allowed_tools: ['filesystem', 'shell'],
    prohibited_actions: [
      'merge, auto-merge, publish, or deploy without separate authority',
    ],
    references: [claimRequest.source_issue_url],
    write_scope: ['**'],
  },
}
const materialized = await postOk(materializePath, materializeRequest)
const materializedReplay = await postOk(materializePath, materializeRequest)
assert.equal(materializedReplay.replayed, true)
assert.equal(materializedReplay.mission_id, materialized.mission_id)

const launch = await postOk(
  `/api/corps/${demo.corp_id}/missions/${materialized.mission_id}/launch`,
  { requested_by: demo.alice_actor_id },
)
const transitionPath =
  `/api/corps/${demo.corp_id}/factory/work-items/` +
  `${claim.work_item.id}/transition`
const running = await postOk(transitionPath, {
  actor_id: demo.alice_actor_id,
  claim_token: claim.claim_token,
  expected_version: materialized.work_item.version,
  idempotency_key: `cockpit-running-${nonce}`,
  state: 'running',
})
await postOk(
  `/api/corps/${demo.corp_id}/factory/controllers/${controllerId}/heartbeat`,
  {
    actor_id: demo.alice_actor_id,
    connection_epoch: firstEpoch,
    lease_seconds: 60,
    active_work_item_id: claim.work_item.id,
    completed_reconcile_generation: null,
    reconcile_result: null,
    error: null,
  },
)

const pending = await waitFor(
  demo.corp_id,
  demo.alice_actor_id,
  (state) => {
    const approval = state.snapshot.action_approvals.find(
      (candidate) =>
        candidate.run_id === launch.run_id && candidate.status === 'pending',
    )
    const run = state.snapshot.runs.find(
      (candidate) => candidate.id === launch.run_id,
    )
    return approval && run?.status === 'waiting_for_approval'
      ? { approval, run }
      : null
  },
  'the inline approval stage',
)

const aliceComment = `Alice preserved cockpit context ${nonce}`
const aliceCommentKey = crypto.randomUUID()
const aliceCommentRequest = {
  actor_id: demo.alice_actor_id,
  body: aliceComment,
  reply_to_id: null,
  mentions: [demo.bob_actor_id],
  link: { kind: 'mission', id: materialized.mission_id },
  idempotency_key: aliceCommentKey,
}
const aliceCommentAttempts = await Promise.all([
  postOk(
    `/api/corps/${demo.corp_id}/rooms/${demo.room_id}/messages`,
    aliceCommentRequest,
  ),
  postOk(
    `/api/corps/${demo.corp_id}/rooms/${demo.room_id}/messages`,
    aliceCommentRequest,
  ),
])
assert.deepEqual(
  aliceCommentAttempts.map((attempt) => attempt.replayed).sort(),
  [false, true],
)
assert.equal(
  aliceCommentAttempts[0].message_id,
  aliceCommentAttempts[1].message_id,
)
const aliceCommentOutcome = aliceCommentAttempts[0]
const leasePath =
  `/api/corps/${demo.corp_id}/agents/${pending.value.run.agent_id}/lease`
const firstBobLease = await postOk(leasePath, {
  actor_id: demo.bob_actor_id,
})
assert.equal(firstBobLease.acquired, true)
assert.ok(firstBobLease.token)
const firstSteer = `Bob steering before restart ${nonce}`
const firstSteerKey = crypto.randomUUID()
const firstSteerRequest = {
  actor_id: demo.bob_actor_id,
  lease_token: firstBobLease.token,
  text: firstSteer,
  idempotency_key: firstSteerKey,
}
const firstSteerAttempts = await Promise.all([
  postOk(
    `/api/corps/${demo.corp_id}/agents/${pending.value.run.agent_id}/messages`,
    firstSteerRequest,
  ),
  postOk(
    `/api/corps/${demo.corp_id}/agents/${pending.value.run.agent_id}/messages`,
    firstSteerRequest,
  ),
])
assert.deepEqual(
  firstSteerAttempts.map((attempt) => attempt.replayed).sort(),
  [false, true],
)
assert.equal(firstSteerAttempts[0].message_id, firstSteerAttempts[1].message_id)
const firstSteerOutcome = firstSteerAttempts[0]
assert.equal(firstSteerOutcome.delivery, 'immediate')

for (const client of [aliceBrowser, bobBrowser]) {
  await waitForBrowserEvent(
    client,
    (event) =>
      event.type === 'room.message_posted' &&
      event.actor_id === demo.alice_actor_id &&
      event.payload.body === aliceComment,
    'Alice contextual comment',
  )
  await waitForBrowserEvent(
    client,
    (event) =>
      event.type === 'control.message_accepted' &&
      event.actor_id === demo.bob_actor_id &&
      event.payload.text === firstSteer,
    'Bob pre-restart steer',
  )
}
await waitFor(
  demo.corp_id,
  demo.alice_actor_id,
  (state) =>
    state.snapshot.events.find(
      (event) =>
        event.aggregate_id === launch.run_id &&
        event.type === 'run.output' &&
        String(event.payload.text).includes(firstSteer),
    ),
  'provider acknowledgement of pre-restart steering',
)

await postOk(`/api/demo/runners/${connectedRunner.id}/disconnect`, {
  reconnect_delay_ms: 2_000,
})
await waitFor(
  demo.corp_id,
  demo.alice_actor_id,
  (state) =>
    state.runners.find((runner) => runner.id === connectedRunner.id)?.status ===
    'grace',
  'runner disconnect grace before stale steering',
)
const stalePendingText = `This pending steer must be fenced before reconnect ${nonce}`
const stalePending = await postOk(
  `/api/corps/${demo.corp_id}/agents/${pending.value.run.agent_id}/messages`,
  {
    actor_id: demo.bob_actor_id,
    lease_token: firstBobLease.token,
    text: stalePendingText,
    idempotency_key: crypto.randomUUID(),
  },
)
assert.equal(stalePending.delivery, 'immediate')
const rotatedBeforeReconnect = await postOk(leasePath, {
  actor_id: demo.bob_actor_id,
})
assert.notEqual(rotatedBeforeReconnect.token, firstBobLease.token)
await waitFor(
  demo.corp_id,
  demo.alice_actor_id,
  (state) => {
    const runner = state.runners.find(
      (candidate) => candidate.id === connectedRunner.id,
    )
    const failed = state.snapshot.events.find(
      (event) =>
        event.type === 'runner.command_failed' &&
        event.payload.message_id === stalePending.message_id,
    )
    const message = state.snapshot.queued_messages.find(
      (candidate) => candidate.id === stalePending.message_id,
    )
    const providerApplied = state.snapshot.events.some(
      (event) =>
        event.type === 'run.output' &&
        event.aggregate_id === launch.run_id &&
        String(event.payload.text).includes(stalePendingText),
    )
    return runner?.status === 'connected' &&
      failed &&
      message?.status === 'cancelled' &&
      !providerApplied
      ? { failed, message }
      : null
  },
  'stale pending steering to fail closed after runner reconnect',
)

const aliceCursor = aliceBrowser.cursor()
const bobCursor = bobBrowser.cursor()
await restartLocalServer()
await Promise.all([aliceBrowser.closed, bobBrowser.closed])

const secondEpoch = crypto.randomUUID()
const reconnectedController = await postOk(controllerPath, {
  actor_id: demo.alice_actor_id,
  controller_id: controllerId,
  source_project_owner: repositoryOwner,
  source_project_number: 145,
  source_repository_owner: repositoryOwner,
  source_repository_name: repositoryName,
  connection_epoch: secondEpoch,
  lease_seconds: 60,
  idempotency_key: `cockpit-controller-reconnect-${nonce}`,
})
assert.equal(reconnectedController.controller.id, controllerId)
assert.equal(reconnectedController.controller.active_work_item_id, claim.work_item.id)

const aliceReconnected = await openBrowserClient(
  demo.corp_id,
  demo.alice_actor_id,
  aliceCursor,
)
const bobReconnected = await openBrowserClient(
  demo.corp_id,
  demo.bob_actor_id,
  bobCursor,
)
assertUniqueSequences(aliceReconnected.events, aliceCursor)
assertUniqueSequences(bobReconnected.events, bobCursor)

const afterRestart = await snapshot(demo.corp_id, demo.bob_actor_id)
assert.equal(
  afterRestart.snapshot.room_messages.filter(
    (message) => message.body === aliceComment,
  ).length,
  1,
)
assert.equal(
  afterRestart.snapshot.room_messages.find(
    (message) => message.body === aliceComment,
  )?.actor_id,
  demo.alice_actor_id,
)
assert.equal(
  afterRestart.snapshot.runs.find((run) => run.id === launch.run_id)?.status,
  'waiting_for_approval',
)
const aliceCommentReplay = await postOk(
  `/api/corps/${demo.corp_id}/rooms/${demo.room_id}/messages`,
  {
    actor_id: demo.alice_actor_id,
    body: aliceComment,
    reply_to_id: null,
    mentions: [demo.bob_actor_id],
    link: { kind: 'mission', id: materialized.mission_id },
    idempotency_key: aliceCommentKey,
  },
)
assert.equal(aliceCommentReplay.replayed, true)
assert.equal(aliceCommentReplay.message_id, aliceCommentOutcome.message_id)
const changedCommentReplay = await post(
  `/api/corps/${demo.corp_id}/rooms/${demo.room_id}/messages`,
  {
    actor_id: demo.alice_actor_id,
    body: `${aliceComment} changed`,
    reply_to_id: null,
    mentions: [demo.bob_actor_id],
    link: { kind: 'mission', id: materialized.mission_id },
    idempotency_key: aliceCommentKey,
  },
)
assert.equal(changedCommentReplay.response.status, 409)
const firstSteerReplay = await postOk(
  `/api/corps/${demo.corp_id}/agents/${pending.value.run.agent_id}/messages`,
  {
    actor_id: demo.bob_actor_id,
    lease_token: firstBobLease.token,
    text: firstSteer,
    idempotency_key: firstSteerKey,
  },
)
assert.equal(firstSteerReplay.replayed, true)
assert.equal(firstSteerReplay.message_id, firstSteerOutcome.message_id)
const changedSteerReplay = await post(
  `/api/corps/${demo.corp_id}/agents/${pending.value.run.agent_id}/messages`,
  {
    actor_id: demo.bob_actor_id,
    lease_token: firstBobLease.token,
    text: `${firstSteer} changed`,
    idempotency_key: firstSteerKey,
  },
)
assert.equal(changedSteerReplay.response.status, 409)

const secondBobLease = await postOk(leasePath, {
  actor_id: demo.bob_actor_id,
})
assert.equal(secondBobLease.acquired, true)
assert.notEqual(secondBobLease.token, firstBobLease.token)
const staleSteer = await post(
  `/api/corps/${demo.corp_id}/agents/${pending.value.run.agent_id}/messages`,
  {
    actor_id: demo.bob_actor_id,
    lease_token: firstBobLease.token,
    text: 'A stale browser token must not steer.',
    idempotency_key: crypto.randomUUID(),
  },
)
assert.equal(staleSteer.response.status, 409)

const secondSteer = `Bob steering after reconnect ${nonce}`
const secondSteerKey = crypto.randomUUID()
const secondSteerOutcome = await postOk(
  `/api/corps/${demo.corp_id}/agents/${pending.value.run.agent_id}/messages`,
  {
    actor_id: demo.bob_actor_id,
    lease_token: secondBobLease.token,
    text: secondSteer,
    idempotency_key: secondSteerKey,
  },
)
assert.equal(secondSteerOutcome.delivery, 'immediate')
assert.equal(secondSteerOutcome.replayed, false)
const bobComment = `Bob confirmed the recovered decision stage ${nonce}`
const bobCommentKey = crypto.randomUUID()
const bobCommentOutcome = await postOk(
  `/api/corps/${demo.corp_id}/rooms/${demo.room_id}/messages`,
  {
    actor_id: demo.bob_actor_id,
    body: bobComment,
    reply_to_id: null,
    mentions: [demo.alice_actor_id],
    link: { kind: 'run', id: launch.run_id },
    idempotency_key: bobCommentKey,
  },
)
assert.equal(bobCommentOutcome.replayed, false)
const secondSteerReplay = await postOk(
  `/api/corps/${demo.corp_id}/agents/${pending.value.run.agent_id}/messages`,
  {
    actor_id: demo.bob_actor_id,
    lease_token: secondBobLease.token,
    text: secondSteer,
    idempotency_key: secondSteerKey,
  },
)
assert.equal(secondSteerReplay.replayed, true)
assert.equal(secondSteerReplay.message_id, secondSteerOutcome.message_id)
const bobCommentReplay = await postOk(
  `/api/corps/${demo.corp_id}/rooms/${demo.room_id}/messages`,
  {
    actor_id: demo.bob_actor_id,
    body: bobComment,
    reply_to_id: null,
    mentions: [demo.alice_actor_id],
    link: { kind: 'run', id: launch.run_id },
    idempotency_key: bobCommentKey,
  },
)
assert.equal(bobCommentReplay.replayed, true)
assert.equal(bobCommentReplay.message_id, bobCommentOutcome.message_id)

const decisionKey = crypto.randomUUID()
const decision = await postOk(
  `/api/corps/${demo.corp_id}/approvals/${pending.value.approval.id}/decision`,
  {
    actor_id: demo.bob_actor_id,
    approved: true,
    note: 'Bob approved the persisted boundary crossing after reconnect.',
    decision_key: decisionKey,
  },
)
assert.equal(decision.effect_queued, true)
const decisionReplay = await postOk(
  `/api/corps/${demo.corp_id}/approvals/${pending.value.approval.id}/decision`,
  {
    actor_id: demo.bob_actor_id,
    approved: true,
    note: 'Bob approved the persisted boundary crossing after reconnect.',
    decision_key: decisionKey,
  },
)
assert.equal(decisionReplay.effect_queued, false)

for (const client of [aliceReconnected, bobReconnected]) {
  await waitForBrowserEvent(
    client,
    (event) =>
      event.type === 'control.message_accepted' &&
      event.actor_id === demo.bob_actor_id &&
      event.payload.text === secondSteer,
    'Bob post-reconnect steer',
  )
  await waitForBrowserEvent(
    client,
    (event) =>
      event.type === 'room.message_posted' &&
      event.actor_id === demo.bob_actor_id &&
      event.payload.body === bobComment,
    'Bob post-reconnect comment',
  )
  await waitForBrowserEvent(
    client,
    (event) =>
      event.type === 'approval.decided' &&
      event.actor_id === demo.bob_actor_id &&
      event.aggregate_id === pending.value.approval.id,
    'Bob inline decision',
  )
}

const completed = await waitFor(
  demo.corp_id,
  demo.alice_actor_id,
  (state) => {
    const mission = state.snapshot.missions.find(
      (candidate) => candidate.id === materialized.mission_id,
    )
    const run = state.snapshot.runs.find(
      (candidate) => candidate.id === launch.run_id,
    )
    return mission?.status === 'completed' && run?.status === 'completed'
      ? { mission, run }
      : null
  },
  'the preserved run to complete',
)
const currentItem = completed.state.snapshot.factory_work_items.find(
  (item) => item.id === claim.work_item.id,
)
const verified = await postOk(transitionPath, {
  actor_id: demo.alice_actor_id,
  claim_token: claim.claim_token,
  expected_version: currentItem.version,
  idempotency_key: `cockpit-verified-${nonce}`,
  state: 'verified',
})
assert.equal(verified.work_item.state, 'verified')

const finalController = await postOk(
  `/api/corps/${demo.corp_id}/factory/controllers/${controllerId}/heartbeat`,
  {
    actor_id: demo.alice_actor_id,
    connection_epoch: secondEpoch,
    lease_seconds: 60,
    active_work_item_id: null,
    completed_reconcile_generation: null,
    reconcile_result: null,
    error: null,
  },
)
assert.equal(finalController.controller.status, 'watching')

const [aliceFinal, bobFinal] = await Promise.all([
  snapshot(demo.corp_id, demo.alice_actor_id),
  snapshot(demo.corp_id, demo.bob_actor_id),
])
for (const state of [aliceFinal, bobFinal]) {
  const workItems = state.snapshot.factory_work_items.filter(
    (item) => item.source_project_item_id === claimRequest.source_project_item_id,
  )
  assert.equal(workItems.length, 1)
  assert.equal(workItems[0].id, claim.work_item.id)
  assert.equal(workItems[0].state, 'verified')
  assert.equal(
    state.snapshot.missions.filter(
      (mission) => mission.id === materialized.mission_id,
    ).length,
    1,
  )
  const missionTasks = state.snapshot.tasks.filter(
    (task) => task.mission_id === materialized.mission_id,
  )
  assert.equal(missionTasks.length, 1)
  assert.equal(
    state.snapshot.runs.filter((run) => run.task_id === missionTasks[0].id).length,
    1,
  )
  assert.equal(
    state.snapshot.room_messages.filter(
      (message) => [aliceComment, bobComment].includes(message.body),
    ).length,
    2,
  )
  assert.equal(
    state.snapshot.queued_messages.filter(
      (message) => [firstSteer, secondSteer].includes(message.text),
    ).length,
    2,
  )
  const approval = state.snapshot.action_approvals.find(
    (candidate) => candidate.id === pending.value.approval.id,
  )
  assert.equal(approval.status, 'approved')
  assert.equal(approval.decided_by, demo.bob_actor_id)
  assert.equal(
    state.snapshot.events.filter(
      (event) =>
        event.type === 'approval.decided' &&
        event.aggregate_id === pending.value.approval.id,
    ).length,
    1,
  )
  assert.equal(
    state.snapshot.events.filter(
      (event) =>
        event.type === 'factory.mission_linked' &&
        event.aggregate_id === claim.work_item.id,
    ).length,
    1,
  )
  for (const direction of [firstSteer, secondSteer]) {
    assert.equal(
      state.snapshot.events.filter(
        (event) =>
          event.type === 'run.output' &&
          event.aggregate_id === launch.run_id &&
          String(event.payload.text).includes(direction),
      ).length,
      1,
    )
  }
  assert.equal(
    state.snapshot.events.filter(
      (event) =>
        event.type === 'runner.command_acknowledged' &&
        event.aggregate_id === launch.run_id &&
        event.payload.command_kind === 'control_message',
    ).length,
    2,
  )
  assert.equal(
    state.snapshot.factory_controllers.filter(
      (controller) => controller.id === controllerId,
    ).length,
    1,
  )
}

aliceReconnected.socket.close()
bobReconnected.socket.close()
const report = {
  checked_at: new Date().toISOString(),
  corp_id: demo.corp_id,
  controller_id: controllerId,
  work_item_id: claim.work_item.id,
  mission_id: materialized.mission_id,
  run_id: launch.run_id,
  source_repository: workspace.source_repository,
  source_base_commit: workspace.source_base_commit,
  browser_actors: [demo.alice_actor_id, demo.bob_actor_id],
  alice_replay_events: aliceReconnected.events.length,
  bob_replay_events: bobReconnected.events.length,
  comment_attribution_preserved: true,
  comment_retry_idempotent: true,
  lease_rotated_after_browser_reload: true,
  stale_lease_token_rejected: true,
  stale_pending_steer_cancelled: true,
  pre_restart_steer_delivered: true,
  post_restart_steer_delivered: true,
  steer_retry_idempotent: true,
  durable_steer_acks: 2,
  decision_attribution_preserved: true,
  duplicate_decision_effects: 0,
  duplicate_work_items: 0,
  duplicate_missions: 0,
  duplicate_runs: 0,
  controller_status: finalController.controller.status,
  final_state: verified.work_item.state,
}
await writeFile(
  path.join(root, 'output', 'e2e-factory-cockpit-reconnect.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
