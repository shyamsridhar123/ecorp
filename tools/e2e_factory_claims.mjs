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

async function snapshot(demo) {
  const result = await request(
    `/api/corps/${demo.corp_id}/snapshot?actor_id=${demo.alice_actor_id}`,
  )
  if (!result.response.ok) {
    throw new Error(`${result.response.status}: ${JSON.stringify(result.body)}`)
  }
  return result.body
}

async function waitForMission(demo, missionId, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const state = await snapshot(demo)
    const mission = state.snapshot.missions.find((item) => item.id === missionId)
    if (mission && ['completed', 'failed', 'cancelled'].includes(mission.status)) {
      return { state, mission }
    }
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error(`timed out waiting for factory mission ${missionId}`)
}

async function restartLocalServer() {
  if (process.env.CRONY_SKIP_SERVER_RESTART === '1') return false
  const pidPath =
    process.env.CRONY_TEST_SERVER_PID_FILE ??
    path.join(root, 'output', 'local-pids.json')
  if (!existsSync(pidPath)) {
    throw new Error(`test-owned server PID file does not exist: ${pidPath}`)
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
  const stdout = openSync(path.join(logDir, 'factory-server-restart.stdout.log'), 'a')
  const stderr = openSync(path.join(logDir, 'factory-server-restart.stderr.log'), 'a')
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
    writeFileSync(pidPath, `${JSON.stringify({ ...pidState, server: child.pid }, null, 2)}\n`)
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
      if (health.status === 'ok' && health.runners >= 1) return true
    } catch {
      // The server is restarting or the runner has not reconnected yet.
    }
    await new Promise((resolve) => setTimeout(resolve, 200))
  }
  throw new Error('server or runner did not recover after factory restart')
}

const demo = await postOk('/api/demo/reset', {})
const nonce = crypto.randomUUID()
const claimPath = `/api/corps/${demo.corp_id}/factory/work-items/claim`
const claimRequest = {
  actor_id: demo.alice_actor_id,
  source_project_owner: 'shyamsridhar123',
  source_project_number: 3,
  source_project_item_id: `PVTI_FACTORY_E2E_${nonce}`,
  source_repository_owner: 'shyamsridhar123',
  source_repository_name: 'ecorp',
  source_issue_number: 59,
  source_issue_node_id: `I_FACTORY_E2E_${nonce}`,
  source_issue_url: 'https://github.com/shyamsridhar123/ecorp/issues/59',
  source_title: 'Persist and fence dark-factory issue claims before mission dispatch',
  source_revision: '2026-09-01T00:00:00Z',
  idempotency_key: `factory-e2e-claim-${nonce}`,
  lease_seconds: 300,
  policy: {
    schema_version: 1,
    source_of_truth: 'github_project',
    project_status: 'Todo',
    repository_allowlist: ['shyamsridhar123/ecorp'],
    source_base_ref: 'HEAD',
    adapter_allowlist: ['fake-process'],
    strategy_allowlist: ['single'],
    model: null,
    reasoning_effort: null,
    write_scope: ['crates/**', 'db/migrations/**', 'tools/**', 'docs/**'],
    allowed_tools: ['filesystem', 'shell'],
    prohibited_actions: [
      'modify files outside the assigned worktree',
      'use undeclared long-lived credentials',
      'merge or deploy without a separate current authorization',
    ],
    secret_ids: [],
    verification_required: true,
    budget_tokens: 20_000,
    budget_cost_microusd: 1_000_000,
    auto_merge: false,
  },
}

const concurrentClaims = await Promise.all([
  postOk(claimPath, claimRequest),
  postOk(claimPath, claimRequest),
])
assert.equal(concurrentClaims[0].work_item.id, concurrentClaims[1].work_item.id)
assert.equal(concurrentClaims[0].claim_token, concurrentClaims[1].claim_token)
assert.deepEqual(
  concurrentClaims.map((claim) => claim.replayed).sort(),
  [false, true],
)
const claim = concurrentClaims[0]
assert.equal(claim.work_item.state, 'claimed')
assert.equal(claim.work_item.version, 1)
assert.ok(claim.claim_token)

const unauthorized = await post(claimPath, {
  ...claimRequest,
  actor_id: demo.eve_actor_id,
  idempotency_key: `factory-e2e-guest-${nonce}`,
})
assert.equal(unauthorized.response.status, 403)
const guestSnapshot = await request(
  `/api/corps/${demo.corp_id}/snapshot?actor_id=${demo.eve_actor_id}`,
)
assert.equal(guestSnapshot.response.status, 200)
assert.equal(guestSnapshot.body.snapshot.factory_work_items.length, 0)
const guestFactoryEvents = guestSnapshot.body.snapshot.events.filter(
  (event) => event.aggregate_id === claim.work_item.id,
)
assert.equal(guestFactoryEvents.length, 1)
assert.equal(
  JSON.stringify(guestFactoryEvents[0].payload).includes('source_'),
  false,
)
assert.equal(
  JSON.stringify(guestFactoryEvents[0].payload).includes(claim.claim_token),
  false,
)
const operatorSnapshot = await request(
  `/api/corps/${demo.corp_id}/snapshot?actor_id=${demo.bob_actor_id}`,
)
assert.equal(operatorSnapshot.response.status, 200)
assert.equal(operatorSnapshot.body.snapshot.factory_work_items.length, 1)
const crossCorp = await post(
  `/api/corps/${crypto.randomUUID()}/factory/work-items/claim`,
  {
    ...claimRequest,
    idempotency_key: `factory-e2e-cross-corp-${nonce}`,
  },
)
assert.equal(crossCorp.response.status, 403)

const sameOwnerReplay = await postOk(claimPath, {
  ...claimRequest,
  idempotency_key: `factory-e2e-same-owner-${nonce}`,
})
assert.equal(sameOwnerReplay.work_item.id, claim.work_item.id)
assert.equal(sameOwnerReplay.claim_token, claim.claim_token)
const duplicateActive = await post(claimPath, {
  ...claimRequest,
  actor_id: demo.bob_actor_id,
  idempotency_key: `factory-e2e-competing-${nonce}`,
})
assert.equal(duplicateActive.response.status, 409)

const restarted = await restartLocalServer()

const renewPath = `/api/corps/${demo.corp_id}/factory/work-items/${claim.work_item.id}/renew`
const renewRequest = {
  actor_id: demo.alice_actor_id,
  claim_token: claim.claim_token,
  expected_version: claim.work_item.version,
  idempotency_key: `factory-e2e-renew-${nonce}`,
  lease_seconds: 300,
}
const concurrentRenewals = await Promise.all([
  postOk(renewPath, renewRequest),
  postOk(renewPath, renewRequest),
])
assert.equal(concurrentRenewals[0].work_item.version, 2)
assert.equal(concurrentRenewals[1].work_item.version, 2)
assert.deepEqual(
  concurrentRenewals.map((renewal) => renewal.replayed).sort(),
  [false, true],
)
assert.equal(concurrentRenewals[0].claim_token, claim.claim_token)
const renewed = concurrentRenewals[0]

const staleRenewal = await post(renewPath, {
  ...renewRequest,
  idempotency_key: `factory-e2e-stale-version-${nonce}`,
})
assert.equal(staleRenewal.response.status, 409)
const wrongToken = await post(renewPath, {
  ...renewRequest,
  claim_token: crypto.randomUUID(),
  expected_version: renewed.work_item.version,
  idempotency_key: `factory-e2e-wrong-token-${nonce}`,
})
assert.equal(wrongToken.response.status, 409)

const materializePath =
  `/api/corps/${demo.corp_id}/factory/work-items/` +
  `${claim.work_item.id}/materialize`
const materializeRequest = {
  actor_id: demo.alice_actor_id,
  claim_token: claim.claim_token,
  expected_version: renewed.work_item.version,
  idempotency_key: `factory-e2e-materialize-${nonce}`,
  title: '[slow] [factory] Implement GitHub issue #59 with durable evidence.',
  preferred_adapter: 'fake-process',
  strategy: 'single',
  budget_tokens: 20_000,
  budget_cost_microusd: 1_000_000,
  contract: {
    objective:
      'Implement the linked GitHub issue exactly as specified and preserve issue-to-mission provenance.',
    expected_output: 'A verified repository change for GitHub issue #59.',
    acceptance_tests: [
      'the durable factory work item links to exactly one mission',
      'duplicate materialization returns the original mission',
    ],
    allowed_tools: ['filesystem', 'shell'],
    prohibited_actions: ['merge or deploy without a separate current authorization'],
    references: ['https://github.com/shyamsridhar123/ecorp/issues/59'],
    write_scope: ['crates/**', 'db/migrations/**', 'tools/**', 'docs/**'],
  },
}
const policyBypass = await post(materializePath, {
  ...materializeRequest,
  idempotency_key: `factory-e2e-policy-bypass-${nonce}`,
  budget_tokens: 25_000,
  contract: {
    ...materializeRequest.contract,
    write_scope: ['**'],
  },
})
assert.equal(policyBypass.response.status, 400)
const toolBypass = await post(materializePath, {
  ...materializeRequest,
  idempotency_key: `factory-e2e-tool-bypass-${nonce}`,
  contract: {
    ...materializeRequest.contract,
    allowed_tools: ['filesystem', 'shell', 'network'],
  },
})
assert.equal(toolBypass.response.status, 400)
const secretBypass = await post(materializePath, {
  ...materializeRequest,
  idempotency_key: `factory-e2e-secret-bypass-${nonce}`,
  secret_refs: [
    {
      secret_id: crypto.randomUUID(),
      env_name: 'FACTORY_UNDECLARED_SECRET',
      tool: 'shell',
      resource: 'github.com/acme',
    },
  ],
})
assert.equal(secretBypass.response.status, 400)
const materialized = await Promise.all([
  postOk(materializePath, materializeRequest),
  postOk(materializePath, materializeRequest),
])
assert.equal(materialized[0].mission_id, materialized[1].mission_id)
assert.equal(materialized[0].task_id, materialized[1].task_id)
assert.deepEqual(
  materialized.map((item) => item.replayed).sort(),
  [false, true],
)
assert.ok(materialized.every((item) => item.work_item.state === 'mission_created'))
assert.ok(materialized.every((item) => item.work_item.version === 3))

const afterMaterialize = await snapshot(demo)
const factoryItems = afterMaterialize.snapshot.factory_work_items
assert.equal(factoryItems.length, 1)
assert.equal(factoryItems[0].mission_id, materialized[0].mission_id)
assert.equal(
  Object.hasOwn(factoryItems[0], 'claim_token'),
  false,
  'factory fencing token leaked into the shared snapshot',
)
assert.equal(
  afterMaterialize.snapshot.missions.filter(
    (mission) => mission.id === materialized[0].mission_id,
  ).length,
  1,
)
const linkedTask = afterMaterialize.snapshot.tasks.find(
  (task) => task.id === materialized[0].task_id,
)
assert.match(linkedTask.contract.objective, /linked GitHub issue/)
assert.equal(linkedTask.contract.source_repository, 'shyamsridhar123/ecorp')
assert.equal(linkedTask.contract.source_base_ref, 'HEAD')
assert.deepEqual(linkedTask.contract.write_scope, materializeRequest.contract.write_scope)
assert.ok(
  linkedTask.contract.references.includes(
    'https://github.com/shyamsridhar123/ecorp/issues/59',
  ),
)

const factoryEvents = afterMaterialize.snapshot.events.filter(
  (event) => event.aggregate_id === claim.work_item.id,
)
assert.deepEqual(
  factoryEvents.map((event) => event.type),
  [
    'factory.work_item_claimed',
    'factory.claim_renewed',
    'factory.mission_linked',
  ],
)
assert.ok(
  factoryEvents.every(
    (event) => !JSON.stringify(event.payload).includes(claim.claim_token),
  ),
)

const sameOwnerAfterMaterialize = await postOk(claimPath, {
  ...claimRequest,
  idempotency_key: `factory-e2e-after-materialize-${nonce}`,
})
assert.equal(
  sameOwnerAfterMaterialize.work_item.mission_id,
  materialized[0].mission_id,
)
const duplicateAfterMaterialize = await post(claimPath, {
  ...claimRequest,
  actor_id: demo.bob_actor_id,
  idempotency_key: `factory-e2e-after-materialize-bob-${nonce}`,
})
assert.equal(duplicateAfterMaterialize.response.status, 409)

const launch = await postOk(
  `/api/corps/${demo.corp_id}/missions/${materialized[0].mission_id}/launch`,
  { requested_by: demo.alice_actor_id },
)
const transitionPath =
  `/api/corps/${demo.corp_id}/factory/work-items/` +
  `${claim.work_item.id}/transition`
const runningTransition = {
  actor_id: demo.alice_actor_id,
  claim_token: claim.claim_token,
  expected_version: materialized[0].work_item.version,
  idempotency_key: `factory-e2e-running-${nonce}`,
  state: 'running',
}
const running = await postOk(transitionPath, runningTransition)
assert.equal(running.work_item.state, 'running')
assert.equal(running.work_item.version, 4)
const runningReplay = await postOk(transitionPath, runningTransition)
assert.equal(runningReplay.replayed, true)
assert.equal(runningReplay.work_item.version, 4)
const forgedVerified = await post(transitionPath, {
  actor_id: demo.alice_actor_id,
  claim_token: claim.claim_token,
  expected_version: running.work_item.version,
  idempotency_key: `factory-e2e-forged-verified-${nonce}`,
  state: 'verified',
})
assert.equal(forgedVerified.response.status, 409)

const completed = await waitForMission(demo, materialized[0].mission_id)
assert.equal(completed.mission.status, 'completed')
const run = completed.state.snapshot.runs.find((item) => item.id === launch.run_id)
assert.equal(run.status, 'completed')
const verified = await postOk(transitionPath, {
  actor_id: demo.alice_actor_id,
  claim_token: claim.claim_token,
  expected_version: running.work_item.version,
  idempotency_key: `factory-e2e-verified-${nonce}`,
  state: 'verified',
})
assert.equal(verified.work_item.state, 'verified')
assert.equal(verified.work_item.version, 5)
const forgedPublished = await post(transitionPath, {
  actor_id: demo.alice_actor_id,
  claim_token: claim.claim_token,
  expected_version: verified.work_item.version,
  idempotency_key: `factory-e2e-forged-published-${nonce}`,
  state: 'published',
})
assert.equal(forgedPublished.response.status, 400)
const invalidRegression = await post(transitionPath, {
  actor_id: demo.alice_actor_id,
  claim_token: claim.claim_token,
  expected_version: verified.work_item.version,
  idempotency_key: `factory-e2e-invalid-regression-${nonce}`,
  state: 'running',
})
assert.equal(invalidRegression.response.status, 409)
const finalState = await snapshot(demo)
const finalFactoryItem = finalState.snapshot.factory_work_items.find(
  (item) => item.id === claim.work_item.id,
)
assert.equal(finalFactoryItem.state, 'verified')
const finalFactoryEvents = finalState.snapshot.events.filter(
  (event) => event.aggregate_id === claim.work_item.id,
)
assert.deepEqual(
  finalFactoryEvents.map((event) => event.type),
  [
    'factory.work_item_claimed',
    'factory.claim_renewed',
    'factory.mission_linked',
    'factory.state_changed',
    'factory.state_changed',
  ],
)
const finalGuestSnapshot = await request(
  `/api/corps/${demo.corp_id}/snapshot?actor_id=${demo.eve_actor_id}`,
)
assert.equal(finalGuestSnapshot.response.status, 200)
const finalGuestFactoryEvents = finalGuestSnapshot.body.snapshot.events.filter(
  (event) => event.aggregate_id === claim.work_item.id,
)
assert.deepEqual(
  finalGuestFactoryEvents.map((event) => event.type),
  ['factory.work_item_claimed', 'factory.claim_renewed'],
)
assert.ok(
  finalGuestFactoryEvents.every(
    (event) => !JSON.stringify(event.payload).includes('source_'),
  ),
)

const blockedClaimRequest = {
  ...claimRequest,
  source_project_item_id: `PVTI_FACTORY_BLOCKED_${nonce}`,
  source_issue_number: 60,
  source_issue_node_id: `I_FACTORY_BLOCKED_${nonce}`,
  source_issue_url: 'https://github.com/shyamsridhar123/ecorp/issues/60',
  source_title: 'Consume eligible ECorp Build issues into governed missions',
  source_revision: '2026-09-01T00:01:00Z',
  idempotency_key: `factory-e2e-blocked-claim-${nonce}`,
  lease_seconds: 30,
}
const blockedClaim = await postOk(claimPath, blockedClaimRequest)
const blockedTransitionPath =
  `/api/corps/${demo.corp_id}/factory/work-items/` +
  `${blockedClaim.work_item.id}/transition`
const blocked = await postOk(blockedTransitionPath, {
  actor_id: demo.alice_actor_id,
  claim_token: blockedClaim.claim_token,
  expected_version: blockedClaim.work_item.version,
  idempotency_key: `factory-e2e-blocked-${nonce}`,
  state: 'blocked',
  failure_detail: 'pre-materialization dependency check failed',
})
assert.equal(blocked.work_item.state, 'blocked')
await new Promise((resolve) => setTimeout(resolve, 31_000))
const failoverClaim = await postOk(claimPath, {
  ...blockedClaimRequest,
  actor_id: demo.bob_actor_id,
  idempotency_key: `factory-e2e-bob-takeover-${nonce}`,
  lease_seconds: 300,
})
assert.equal(failoverClaim.work_item.id, blockedClaim.work_item.id)
assert.equal(failoverClaim.work_item.state, 'claimed')
assert.equal(failoverClaim.work_item.claim_owner_id, demo.bob_actor_id)
assert.equal(failoverClaim.work_item.failure_detail, null)
const failoverMaterialize = await postOk(
  `/api/corps/${demo.corp_id}/factory/work-items/` +
    `${blockedClaim.work_item.id}/materialize`,
  {
    actor_id: demo.bob_actor_id,
    claim_token: failoverClaim.claim_token,
    expected_version: failoverClaim.work_item.version,
    idempotency_key: `factory-e2e-bob-materialize-${nonce}`,
    title: '[factory] Recover an expired pre-materialization claim.',
    preferred_adapter: 'fake-process',
    strategy: 'single',
    budget_tokens: 20_000,
    budget_cost_microusd: 1_000_000,
    contract: {
      objective:
        'Recover the expired blocked claim without creating a second factory work item.',
      expected_output: 'One durable mission linked by the replacement controller.',
      acceptance_tests: ['the original work item is reused'],
      allowed_tools: ['filesystem', 'shell'],
      prohibited_actions: ['merge or deploy without a separate current authorization'],
      references: ['https://github.com/shyamsridhar123/ecorp/issues/60'],
      write_scope: claimRequest.policy.write_scope,
    },
  },
)
assert.equal(failoverMaterialize.work_item.state, 'mission_created')

const report = {
  checked_at: new Date().toISOString(),
  source_issue: claim.work_item.source_issue_url,
  work_item_id: claim.work_item.id,
  mission_id: materialized[0].mission_id,
  task_id: materialized[0].task_id,
  run_id: launch.run_id,
  concurrent_claims_collapsed: true,
  concurrent_renewals_collapsed: true,
  concurrent_materialization_collapsed: true,
  server_restart_recovered: restarted,
  unauthorized_claim_rejected: true,
  guest_snapshot_hides_factory_work: true,
  cross_corp_claim_rejected: true,
  active_duplicate_rejected: true,
  same_owner_recovered_active_token: true,
  stale_version_rejected: true,
  stale_token_rejected: true,
  policy_widening_rejected: true,
  tool_and_secret_widening_rejected: true,
  pre_verification_transition_rejected: true,
  publication_state_requires_dedicated_operation: true,
  invalid_state_regression_rejected: true,
  expired_blocked_claim_recovered: true,
  expired_claim_taken_over_by_second_operator: true,
  fencing_token_absent_from_snapshot_and_events: true,
  issue_contract_persisted: true,
  factory_state: finalFactoryItem.state,
  factory_event_types: finalFactoryEvents.map((event) => event.type),
  mission_status: completed.mission.status,
  run_status: run.status,
}
await writeFile(
  path.join(root, 'output', 'e2e-factory-claims.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
