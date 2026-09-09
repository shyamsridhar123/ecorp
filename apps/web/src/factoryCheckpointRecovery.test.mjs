import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import vm from 'node:vm'
import ts from 'typescript'
import {
  factoryRecoveryBlocksProviderResume, factoryRecoveryConnection,
  factoryRecoveryModes, needsFactoryRecoveryContext,
} from './factoryCheckpointRecovery.ts'

const id = (value) => `00000000-0000-4000-8000-${String(value).padStart(12, '0')}`
const nil = '00000000-0000-0000-0000-000000000000'
const scope = { corpId: id(1), actorId: id(11), missionId: id(20), itemId: id(30), version: 60, reload: 0 }
const context = () => ({
  work_item: {
    id: scope.itemId, corp_id: scope.corpId, mission_id: scope.missionId,
    version: 60, state: 'cancelled', source_project_owner: 'owner',
    source_project_number: 3, source_repository_owner: 'owner',
    source_repository_name: 'lab', source_issue_number: 4,
    policy: { source_base_ref: 'main', workspace_connection_id: id(198) },
  },
  mission_id: scope.missionId, task_id: id(40), source_run_id: id(50),
  workspace_fingerprint: 'b'.repeat(64), expected_head_commit: 'c'.repeat(40),
  checkpoint_verification: true, checkpoint_cancellation_event_id: id(60),
  recoveries: [], remaining_attempts: 0, remaining_mission_tokens: 0,
  remaining_mission_cost_microusd: 0,
})

test('only native proof plus its cancellation marker exposes source-only checkpoint verification', () => {
  const value = context()
  assert.deepEqual(factoryRecoveryModes(value), ['checkpoint-verification'])
  for (const checkpoint_verification of [undefined, false]) {
    assert.deepEqual(factoryRecoveryModes({ ...value, checkpoint_verification }), [])
  }
  for (const checkpoint_cancellation_event_id of [undefined, null, '', 'not-a-uuid', nil]) {
    assert.deepEqual(factoryRecoveryModes({ ...value, checkpoint_cancellation_event_id }), [])
  }
  // Model budgets/attempts can be zero; only native stopped-source proof grants
  // this provider-free path. No new spend or completion is inferred here.
  assert.equal(value.remaining_mission_tokens, 0)
  assert.equal(value.remaining_attempts, 0)
})

test('a running checkpoint needs no cancellation repair, and ordinary verification failure stays compatible', () => {
  const running = context()
  running.work_item.state = 'running'
  delete running.checkpoint_cancellation_event_id
  assert.deepEqual(factoryRecoveryModes(running), ['checkpoint-verification'])
  running.checkpoint_verification = false
  assert.deepEqual(factoryRecoveryModes(running), [])
  running.work_item.state = 'verification_failed'
  assert.deepEqual(factoryRecoveryModes(running), ['verifier-only', 'source-correction'])
  assert.deepEqual(factoryRecoveryModes(null), [])
})

test('missing/tampered source, checkpoint, head, active recovery or terminal outcome offers no new recovery', () => {
  const original = context()
  for (const changes of [
    { task_id: nil }, { source_run_id: 'missing' },
    { workspace_fingerprint: null }, { workspace_fingerprint: 'not-a-fingerprint' },
    { expected_head_commit: null }, { expected_head_commit: 'short' },
    { recoveries: [{ status: 'authorized' }] }, { recoveries: [{ status: 'running' }] },
  ]) assert.deepEqual(factoryRecoveryModes({ ...original, ...changes }), [])
  for (const state of ['published', 'verified', 'failed']) {
    assert.deepEqual(factoryRecoveryModes({
      ...original, work_item: { ...original.work_item, state },
    }), [])
  }
  assert.deepEqual(factoryRecoveryModes({
    ...original, work_item: { ...original.work_item, state: 'running' },
  }), [], 'a cancellation marker cannot be transplanted to another state')
})

test('context requests are not recovery grants and never depend on inferred mission spend/status', () => {
  for (const state of ['running', 'blocked', 'awaiting_approval', 'cancelled', 'verification_failed']) {
    assert.equal(needsFactoryRecoveryContext(state), true)
  }
  for (const state of [undefined, 'claimed', 'mission_created', 'verified', 'published', 'failed']) {
    assert.equal(needsFactoryRecoveryContext(state), false)
  }
  assert.equal(factoryRecoveryBlocksProviderResume(true, null), true)
  assert.equal(factoryRecoveryBlocksProviderResume(true, context()), true)
  assert.equal(factoryRecoveryBlocksProviderResume(true, { ...context(), checkpoint_verification: false }), true)
  assert.equal(factoryRecoveryBlocksProviderResume(false, null), false, 'direct missions retain their existing path')
})

test('copied command binding retains the saved connection and never falls back on malformed metadata', () => {
  assert.equal(factoryRecoveryConnection({ workspace_connection_id: id(198) }), id(198))
  assert.equal(factoryRecoveryConnection({}), null)
  assert.equal(factoryRecoveryConnection({ workspace_connection_id: null }), null)
  for (const value of ['', nil, 'other account', 198, {}, []]) {
    assert.equal(factoryRecoveryConnection({ workspace_connection_id: value }), undefined)
  }
})

const appText = await readFile(new URL('./App.tsx', import.meta.url), 'utf8')
const app = ts.createSourceFile('App.tsx', appText, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
function all(root, predicate) {
  const found = []
  function visit(node) {
    if (predicate(node)) found.push(node)
    ts.forEachChild(node, visit)
  }
  visit(root)
  return found
}
function functionNode(name) {
  return all(app, (node) => ts.isFunctionDeclaration(node) && node.name?.text === name)[0]
}
const card = functionNode('MissionCard')
function initializer(name) {
  const found = all(card, (node) => ts.isVariableDeclaration(node)
    && ts.isIdentifier(node.name) && node.name.text === name)
  assert.equal(found.length, 1, name)
  return found[0].initializer.getText(app)
}
function evaluate(source, globals) {
  const output = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None },
  }).outputText
  vm.runInNewContext(output, globals)
}
function presentation(value, runs = []) {
  const globals = { factoryRecoveryModes, value, runs }
  evaluate(`${functionNode('factoryRecoveryPresentation').getText(app)}
    globalThis.result = factoryRecoveryPresentation(value, runs);`, globals)
  return globals.result
}

test('actual presentation selects the endpoint source, not the newest visible run or mission cancellation', () => {
  const value = context()
  const result = presentation(value, [{ id: id(999), task_id: id(998), status: 'running' }])
  assert.equal(result.run, undefined)
  assert.equal(result.state, 'reconciliation_required')
  assert.equal(result.heading, 'Verify retained checkpoint')
  assert.match(result.detail, /without a provider/)
  assert.equal(result.checkpoint, value.workspace_fingerprint)
  delete value.checkpoint_cancellation_event_id
  assert.equal(presentation(value).state, 'unavailable')
  assert.equal(presentation(value).checkpoint, null)
})

async function requestContext(value) {
  const requests = []
  const published = []
  const globals = {
    AbortController, setTimeout: () => 1, clearTimeout: () => {},
    api: async (...args) => { requests.push(args); return value },
  }
  evaluate(`${functionNode('factoryRecoveryScopeKey').getText(app)}
    ${functionNode('requestFactoryRecoveryContext').getText(app)}`, globals)
  const cleanup = globals.requestFactoryRecoveryContext(scope, (load) => published.push(load))
  await new Promise(setImmediate)
  cleanup()
  return { requests, published }
}

test('actual context loader is GET-only and admits the exact item/version/mission checkpoint', async () => {
  const value = context()
  const { requests, published } = await requestContext(value)
  assert.equal(requests.length, 1)
  assert.equal(requests[0][0],
    `/api/corps/${scope.corpId}/factory/work-items/${scope.itemId}/verification-recoveries?actor_id=${scope.actorId}`)
  assert.equal(requests[0][1].method, 'GET')
  assert.equal(published.at(-1).status, 'ready')
  assert.equal(published.at(-1).data.source_run_id, id(50))
  const globals = {}
  evaluate(`${functionNode('factoryRecoveryScopeKey').getText(app)}
    ${functionNode('currentFactoryRecoveryLoad').getText(app)}`, globals)
  assert.equal(globals.currentFactoryRecoveryLoad({ ...scope, actorId: id(12) }, published.at(-1)), null)
  assert.equal(globals.currentFactoryRecoveryLoad({ ...scope, version: 61 }, published.at(-1)), null)
})

test('actual context loader rejects stale/cross-scope and malformed flags without snapshot fallback', async () => {
  for (const change of [
    (value) => { value.work_item.version++ },
    (value) => { value.work_item.id = id(999) },
    (value) => { value.work_item.corp_id = id(999) },
    (value) => { value.mission_id = id(999) },
    (value) => { value.checkpoint_verification = 'true' },
    (value) => { value.checkpoint_cancellation_event_id = 60 },
  ]) {
    const value = context()
    change(value)
    const { requests, published } = await requestContext(value)
    assert.equal(requests.length, 1)
    assert.equal(published.at(-1).status, 'error')
    assert.equal(published.at(-1).data, null)
  }
})

function commandHarness(value = context(), role = 'owner') {
  const writes = []
  const globals = {
    API_URL: 'http://127.0.0.1:18961',
    recoveryContext: value, corpId: scope.corpId, actorId: scope.actorId, actorRole: role,
    mission: { budget_tokens: 1_000_000, budget_cost_microusd: 1_000_000 },
    tasks: [
      { id: id(999), mission_id: scope.missionId, required_adapter: 'wrong-provider', contract: {} },
      { id: id(40), mission_id: scope.missionId, required_adapter: 'github-copilot',
        contract: { model: 'reviewed-model', reasoning_effort: 'high', source_base_ref: 'main' } },
    ],
    agents: [], recovery: presentation(value), scopedRecoveryLoad: { scopeKey: 'exact-context' },
    factoryRecoveryModes, factoryRecoveryConnection,
    navigator: { clipboard: { writeText: async (text) => { writes.push(text) } } },
    setCopiedRecoveryCommand: () => {},
  }
  for (const name of ['recoveryTask', 'recoveryItem', 'recoveryModes', 'recoveryConnectionId',
    'canAuthorizeRecovery', 'recoveryAgent', 'recoveryAdapter', 'recoverySourceBase',
    'recoveryCommand', 'recoveryCopyKey', 'copyRecoveryCommand']) {
    evaluate(`globalThis.${name} = ${initializer(name)};`, globals)
  }
  return { globals, writes }
}

test('actual UI action copies only explicit checkpoint verification with the original scope/connection', async () => {
  const { globals, writes } = commandHarness()
  await globals.copyRecoveryCommand('checkpoint-verification')
  assert.equal(writes.length, 1)
  assert.match(writes[0], /^crony --server 'http:\/\/127\.0\.0\.1:18961' factory /)
  assert.match(writes[0], /--verification-recovery checkpoint-verification/)
  assert.match(writes[0], /--project-number 3/)
  assert.match(writes[0], /--repository 'owner\/lab'/)
  assert.match(writes[0], /--source-base-ref 'main'/)
  assert.match(writes[0], /--adapter 'github-copilot'/)
  assert.match(writes[0], /--issue 4/)
  assert.ok(writes[0].includes(`--workspace-connection-id '${id(198)}'`))
  assert.doesNotMatch(writes[0], /source-repository-path|wrong-provider/)
  await globals.copyRecoveryCommand('source-correction')
  await globals.copyRecoveryCommand('verifier-only')
  assert.equal(writes.length, 1, 'checkpoint authority is not a generic provider recovery grant')
  for (const role of ['member', 'guest', 'spectator']) {
    const denied = commandHarness(context(), role)
    await denied.globals.copyRecoveryCommand('checkpoint-verification')
    assert.equal(denied.writes.length, 0)
  }
  const stopped = context()
  delete stopped.checkpoint_cancellation_event_id
  const denied = commandHarness(stopped)
  await denied.globals.copyRecoveryCommand('checkpoint-verification')
  assert.equal(denied.writes.length, 0)
})

test('actual resume handler and rendered provider control respect native recovery blocking', async () => {
  let resumed = 0
  const globals = {
    resumableRun: { id: id(50) }, resumeRecoveryBlocked: true,
    onResume: async () => { resumed++; return null }, rememberEvidenceRun: () => {},
  }
  evaluate(`globalThis.resume = ${initializer('resumeEvidence')};`, globals)
  await globals.resume()
  assert.equal(resumed, 0)
  const guard = all(card, (node) => ts.isConditionalExpression(node)
    && node.whenTrue.getText(app).includes('onClick={() => void resumeEvidence()}'))[0]
  assert.ok(guard.condition.getText(app).includes('!resumeRecoveryBlocked'))
  assert.ok(initializer('resumeRecoveryBlocked').includes('factoryRecoveryBlocksProviderResume'))
  assert.ok(initializer('recoveryScope').includes('needsFactoryRecoveryContext(recoveryItemState)'))
  const copy = all(card, (node) => ts.isCallExpression(node)
    && node.expression.getText(app) === 'copyRecoveryCommand')
  assert.equal(copy.length, 1)
  assert.equal(copy[0].arguments[0].getText(app), 'mode')
})
