import { readFixtureSourceIdentity } from './fixture_source_identity.mjs'
import { restartOwnedTestServer } from './owned_test_stack.mjs'
import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { writeFile } from 'node:fs/promises'
import path from 'node:path'

const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const root = path.resolve(import.meta.dirname, '..')
const sourceRoot = path.resolve(process.env.ECORP_TEST_SOURCE_REPOSITORY ?? root)
const fixtureSource = readFixtureSourceIdentity(sourceRoot)
const databaseUrl =
  process.env.DATABASE_URL ?? 'postgres://crony:crony@127.0.0.1:54329/crony'
const sourceBaseCommit = execFileSync('git', ['rev-parse', 'HEAD'], {
  cwd: sourceRoot,
  encoding: 'utf8',
}).trim()

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

async function restartLocalServer(demo) {
  if (process.env.CRONY_SKIP_SERVER_RESTART === '1') return false
  await restartOwnedTestServer({
    root, server, databaseUrl, logPrefix: 'factory-server-restart',
  })

  const deadline = Date.now() + 30_000
  let lastReadinessError = 'server or runner is unavailable'
  while (Date.now() < deadline) {
    try {
      const signal = AbortSignal.timeout(Math.max(1, deadline - Date.now()))
      const health = await fetch(`${server}/health`, { signal }).then((response) =>
        response.json(),
      )
      if (health.status === 'ok' && health.runners >= 1) {
        // Connected runners may still be reconciling. A read-only preview checks
        // dispatch readiness for the same adapter and immutable source tuple.
        const preview = await fetch(
          `${server}/api/corps/${demo.corp_id}/missions/preview`,
          {
            method: 'POST',
            headers: { 'content-type': 'application/json' },
            signal,
            body: JSON.stringify({
              requested_by: demo.alice_actor_id,
              title: 'Factory restart dispatch readiness',
              preferred_adapter: 'fake-process',
              strategy: 'single',
              budget_tokens: 20_000,
              budget_cost_microusd: 1_000_000,
              source: {
                repository: fixtureSource.repository,
                base_ref: 'HEAD',
                base_commit: sourceBaseCommit,
              },
            }),
          },
        )
        const previewBody = await preview.json()
        if (preview.ok) return true
        lastReadinessError = `${preview.status}: ${JSON.stringify(previewBody)}`
      }
    } catch (error) {
      // The server is restarting or the runner has not reconnected yet.
      lastReadinessError = error.message
    }
    await new Promise((resolve) => setTimeout(resolve, 200))
  }
  throw new Error(
    `server or runner did not recover after factory restart: ${lastReadinessError}`,
  )
}

const demo = await postOk('/api/demo/reset', {})
const nonce = crypto.randomUUID()
const claimPath = `/api/corps/${demo.corp_id}/factory/work-items/claim`
const claimRequest = {
  actor_id: demo.alice_actor_id,
  source_project_owner: 'shyamsridhar123',
  source_project_number: 3,
  source_project_item_id: `PVTI_FACTORY_E2E_${nonce}`,
  source_repository_owner: fixtureSource.owner,
  source_repository_name: fixtureSource.name,
  source_issue_number: 59,
  source_issue_node_id: `I_FACTORY_E2E_${nonce}`,
  source_issue_url: `${fixtureSource.url}/issues/59`,
  source_title: 'Persist and fence dark-factory issue claims before mission dispatch',
  source_revision: '2026-09-01T00:00:00Z',
  idempotency_key: `factory-e2e-claim-${nonce}`,
  lease_seconds: 300,
  policy: {
    schema_version: 1,
    source_of_truth: 'github_project',
    project_status: 'Todo',
    repository_allowlist: [fixtureSource.repository],
    source_base_ref: 'HEAD',
    source_base_commit: sourceBaseCommit,
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
const mixedCaseReplay = await postOk(claimPath, {
  ...claimRequest,
  source_project_owner: 'ShYaMsRiDhAr123',
  source_repository_owner: fixtureSource.owner.toUpperCase(),
  source_repository_name: fixtureSource.name.toUpperCase(),
  source_issue_url: `https://github.com/${fixtureSource.repository.toUpperCase()}/issues/59`,
  idempotency_key: `factory-e2e-mixed-case-${nonce}`,
})
assert.equal(mixedCaseReplay.work_item.id, claim.work_item.id)
assert.equal(mixedCaseReplay.claim_token, claim.claim_token)
assert.equal(mixedCaseReplay.work_item.source_project_owner, 'shyamsridhar123')
assert.equal(mixedCaseReplay.work_item.source_repository_owner, fixtureSource.owner)
assert.equal(mixedCaseReplay.work_item.source_repository_name, fixtureSource.name)

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

const restarted = await restartLocalServer(demo)

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
    references: [`${fixtureSource.url}/issues/59`],
    write_scope: ['crates/**', 'db/migrations/**', 'tools/**', 'docs/**'],
  },
}
async function assertRejectedMaterialization(suffix, patch) {
  const issueNumber = 59_000 + suffix
  const rejectedClaim = await postOk(claimPath, {
    ...claimRequest,
    source_project_item_id: `PVTI_FACTORY_E2E_REJECTED_${suffix}_${nonce}`,
    source_issue_number: issueNumber,
    source_issue_node_id: `I_FACTORY_E2E_REJECTED_${suffix}_${nonce}`,
    source_issue_url: `${fixtureSource.url}/issues/${issueNumber}`,
    source_title: `Reject invalid factory materialization ${suffix}`,
    source_revision: `2026-09-03T19:${String(30 + suffix).padStart(2, '0')}:00Z`,
    idempotency_key: `factory-e2e-rejected-claim-${suffix}-${nonce}`,
  })
  const rejectedPath =
    `/api/corps/${demo.corp_id}/factory/work-items/` +
    `${rejectedClaim.work_item.id}/materialize`
  const rejected = await post(rejectedPath, {
    ...materializeRequest,
    ...patch,
    actor_id: demo.alice_actor_id,
    claim_token: rejectedClaim.claim_token,
    expected_version: rejectedClaim.work_item.version,
    idempotency_key: `factory-e2e-rejected-materialize-${suffix}-${nonce}`,
  })
  assert.equal(rejected.response.status, 400)
  const rejectedSnapshot = await snapshot(demo)
  const blocked = rejectedSnapshot.snapshot.factory_work_items.find(
    (item) => item.id === rejectedClaim.work_item.id,
  )
  assert.equal(blocked.state, 'blocked')
  assert.equal(blocked.mission_id, null)
  assert.ok(Date.parse(blocked.lease_expires_at) <= Date.now() + 30_000)
  return rejected
}

const policyBypass = await assertRejectedMaterialization(1, {
  budget_tokens: 25_000,
  contract: {
    ...materializeRequest.contract,
    write_scope: ['**'],
  },
})
const toolBypass = await assertRejectedMaterialization(2, {
  contract: {
    ...materializeRequest.contract,
    allowed_tools: ['filesystem', 'shell', 'network'],
  },
})
const secretBypass = await assertRejectedMaterialization(3, {
  secret_refs: [
    {
      secret_id: crypto.randomUUID(),
      env_name: 'FACTORY_UNDECLARED_SECRET',
      tool: 'shell',
      resource: 'github.com/acme',
    },
  ],
})
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
const factoryItems = afterMaterialize.snapshot.factory_work_items.filter(
  (item) => item.id === claim.work_item.id,
)
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
assert.equal(linkedTask.contract.source_repository, fixtureSource.repository)
assert.equal(linkedTask.contract.source_base_ref, 'HEAD')
assert.equal(linkedTask.contract.source_base_commit, sourceBaseCommit)
assert.deepEqual(linkedTask.contract.write_scope, materializeRequest.contract.write_scope)
assert.ok(
  linkedTask.contract.references.includes(
    `${fixtureSource.url}/issues/59`,
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
assert.match(
  JSON.stringify(forgedVerified.body),
  /cannot enter verified before its mission and task verification pass/,
)

const completed = await waitForMission(demo, materialized[0].mission_id)
assert.equal(completed.mission.status, 'completed')
const run = completed.state.snapshot.runs.find((item) => item.id === launch.run_id)
assert.equal(run.status, 'completed')
assert.equal(run.verification_status, 'passed')
const completedTask = completed.state.snapshot.tasks.find(
  (item) => item.id === materialized[0].task_id,
)
assert.equal(completedTask.status, 'completed')
assert.equal(completedTask.verification_status, 'passed')
// Accepted completion reconciles Factory in the same transaction. Read that
// result before testing stale controller requests; do not verify it a second time.
const verified = completed.state.snapshot.factory_work_items.find(
  (item) => item.id === claim.work_item.id,
)
assert.ok(verified, 'completed mission is missing its factory work item')
assert.equal(verified.state, 'verified')
assert.equal(verified.version, 5)
assert.equal(verified.mission_id, materialized[0].mission_id)
assert.equal(verified.failure_detail, null)
const verifiedEvents = completed.state.snapshot.events.filter(
  (event) => event.aggregate_id === verified.id && event.type === 'factory.verified',
)
assert.equal(verifiedEvents.length, 1)
assert.equal(verifiedEvents[0].aggregate_version, verified.version)
assert.equal(verifiedEvents[0].correlation_id, materialized[0].mission_id)
assert.equal(verifiedEvents[0].causation_id, launch.run_id)
assert.deepEqual(verifiedEvents[0].payload, {
  previous_state: 'running',
  state: 'verified',
  mission_id: materialized[0].mission_id,
  run_id: launch.run_id,
})
const staleVerified = await post(transitionPath, {
  actor_id: demo.alice_actor_id,
  claim_token: claim.claim_token,
  expected_version: running.work_item.version,
  idempotency_key: `factory-e2e-stale-verified-${nonce}`,
  state: 'verified',
})
assert.equal(staleVerified.response.status, 409)
assert.match(JSON.stringify(staleVerified.body), /factory work item version is 5, not 4/)
const forgedPublished = await post(transitionPath, {
  actor_id: demo.alice_actor_id,
  claim_token: claim.claim_token,
  expected_version: verified.version,
  idempotency_key: `factory-e2e-forged-published-${nonce}`,
  state: 'published',
})
assert.equal(forgedPublished.response.status, 400)
const invalidRegression = await post(transitionPath, {
  actor_id: demo.alice_actor_id,
  claim_token: claim.claim_token,
  expected_version: verified.version,
  idempotency_key: `factory-e2e-invalid-regression-${nonce}`,
  state: 'running',
})
assert.equal(invalidRegression.response.status, 409)
const finalState = await snapshot(demo)
const finalFactoryItem = finalState.snapshot.factory_work_items.find(
  (item) => item.id === claim.work_item.id,
)
assert.deepEqual(finalFactoryItem, verified)
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
    'factory.verified',
  ],
)
assert.ok(
  finalFactoryEvents.every(
    (event) => !JSON.stringify(event.payload).includes(claim.claim_token),
  ),
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
  source_issue_url: `${fixtureSource.url}/issues/60`,
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
const expiredClaimRequest = {
  ...claimRequest,
  source_project_item_id: `PVTI_FACTORY_EXPIRED_CLAIMED_${nonce}`,
  source_issue_number: 61,
  source_issue_node_id: `I_FACTORY_EXPIRED_CLAIMED_${nonce}`,
  source_issue_url: `${fixtureSource.url}/issues/61`,
  source_title: 'Recover an expired claimed factory item',
  source_revision: '2026-09-01T00:01:30Z',
  idempotency_key: `factory-e2e-expired-claimed-${nonce}`,
  lease_seconds: 30,
}
const expiredClaim = await postOk(claimPath, expiredClaimRequest)
assert.equal(expiredClaim.work_item.state, 'claimed')
assert.equal(expiredClaim.work_item.mission_id, null)
await new Promise((resolve) => setTimeout(resolve, 31_000))
const expiredClaimWidened = await post(claimPath, {
  ...expiredClaimRequest,
  actor_id: demo.bob_actor_id,
  idempotency_key: `factory-e2e-expired-claimed-widened-${nonce}`,
  lease_seconds: 300,
  policy: {
    ...expiredClaimRequest.policy,
    write_scope: ['**'],
    budget_tokens: 40_000,
  },
})
assert.equal(expiredClaimWidened.response.status, 400)
const expiredClaimRevised = await post(claimPath, {
  ...expiredClaimRequest,
  actor_id: demo.bob_actor_id,
  source_revision: '2026-09-01T00:02:30Z',
  idempotency_key: `factory-e2e-expired-claimed-revised-${nonce}`,
  lease_seconds: 300,
})
assert.equal(expiredClaimRevised.response.status, 400)
const expiredClaimTakeover = await postOk(claimPath, {
  ...expiredClaimRequest,
  actor_id: demo.bob_actor_id,
  idempotency_key: `factory-e2e-expired-claimed-takeover-${nonce}`,
  lease_seconds: 300,
})
assert.equal(expiredClaimTakeover.work_item.id, expiredClaim.work_item.id)
assert.equal(expiredClaimTakeover.work_item.state, 'claimed')
assert.equal(expiredClaimTakeover.work_item.claim_owner_id, demo.bob_actor_id)
assert.equal(expiredClaimTakeover.work_item.source_revision, expiredClaim.work_item.source_revision)
assert.deepEqual(expiredClaimTakeover.work_item.policy, expiredClaim.work_item.policy)
const expiredClaimCleanup = await postOk(
  `/api/corps/${demo.corp_id}/factory/work-items/` +
    `${expiredClaim.work_item.id}/transition`,
  {
    actor_id: demo.bob_actor_id,
    claim_token: expiredClaimTakeover.claim_token,
    expected_version: expiredClaimTakeover.work_item.version,
    idempotency_key: `factory-e2e-expired-claimed-cleanup-${nonce}`,
    state: 'cancelled',
    failure_detail: 'Test cleanup after expired claimed takeover.',
  },
)
assert.equal(expiredClaimCleanup.work_item.state, 'cancelled')
const widenedReclaim = await post(claimPath, {
  ...blockedClaimRequest,
  actor_id: demo.bob_actor_id,
  idempotency_key: `factory-e2e-bob-widened-takeover-${nonce}`,
  lease_seconds: 300,
  policy: {
    ...blockedClaimRequest.policy,
    write_scope: ['**'],
    budget_tokens: 40_000,
  },
})
assert.equal(widenedReclaim.response.status, 400)
const revisedSourceReclaim = await post(claimPath, {
  ...blockedClaimRequest,
  actor_id: demo.bob_actor_id,
  source_revision: '2026-09-01T00:02:00Z',
  idempotency_key: `factory-e2e-bob-revised-source-takeover-${nonce}`,
  lease_seconds: 300,
})
assert.equal(revisedSourceReclaim.response.status, 400)
const afterRejectedReclaims = await snapshot(demo)
const preservedBlockedClaim =
  afterRejectedReclaims.snapshot.factory_work_items.find(
    (item) => item.id === blockedClaim.work_item.id,
  )
assert.equal(preservedBlockedClaim.state, 'blocked')
assert.equal(preservedBlockedClaim.claim_owner_id, demo.alice_actor_id)
assert.equal(
  preservedBlockedClaim.source_revision,
  blockedClaim.work_item.source_revision,
)
assert.deepEqual(preservedBlockedClaim.policy, blockedClaim.work_item.policy)
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
assert.equal(failoverClaim.work_item.source_revision, blockedClaim.work_item.source_revision)
assert.deepEqual(failoverClaim.work_item.policy, blockedClaim.work_item.policy)
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
      references: [`${fixtureSource.url}/issues/60`],
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
  github_identity_case_normalized: true,
  stale_version_rejected: true,
  stale_token_rejected: true,
  policy_widening_rejected: true,
  tool_and_secret_widening_rejected: true,
  pre_verification_transition_rejected: true,
  completion_automatically_verified_factory: true,
  stale_post_completion_transition_rejected: true,
  publication_state_requires_dedicated_operation: true,
  invalid_state_regression_rejected: true,
  expired_reclaim_policy_widening_rejected: true,
  expired_reclaim_source_revision_change_rejected: true,
  expired_reclaim_preserved_source_and_policy: true,
  expired_blocked_claim_recovered: true,
  expired_claim_taken_over_by_second_operator: true,
  expired_claimed_policy_widening_rejected:
    expiredClaimWidened.response.status === 400,
  expired_claimed_source_revision_change_rejected:
    expiredClaimRevised.response.status === 400,
  expired_claimed_preserved_source_and_policy: true,
  expired_claimed_taken_over_by_second_operator: true,
  expired_claimed_cleanup_state: expiredClaimCleanup.work_item.state,
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
