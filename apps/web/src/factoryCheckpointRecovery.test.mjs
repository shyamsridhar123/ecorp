import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import * as jsxRuntime from 'react/jsx-runtime'
import { renderToStaticMarkup } from 'react-dom/server'
import vm from 'node:vm'
import ts from 'typescript'
import {
  factoryContractRevisionSource, factoryRecoveryBlocksProviderResume, factoryRecoveryConnection,
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

const correctionContext = () => {
  const value = context()
  value.work_item.state = 'blocked'
  delete value.checkpoint_cancellation_event_id
  return {
    ...value, checkpoint_source_correction: true,
    remaining_attempts: 1, remaining_mission_tokens: 176_294,
    remaining_mission_cost_microusd: 500_000,
  }
}

test('issue210 only the explicit server flag adds correction to a native checkpoint', () => {
  const value = correctionContext()
  for (const state of ['verification_failed', 'blocked']) {
    value.work_item.state = state
    assert.deepEqual(factoryRecoveryModes(value), ['checkpoint-verification', 'source-correction'])
    for (const checkpoint_source_correction of [false, undefined]) {
      assert.deepEqual(factoryRecoveryModes({ ...value, checkpoint_source_correction }), ['checkpoint-verification'])
    }
  }
  assert.deepEqual(factoryRecoveryModes({ ...context(), checkpoint_source_correction: true }),
    ['checkpoint-verification'], 'cancelled work can never receive provider correction')
  assert.deepEqual(factoryRecoveryModes({ ...value, checkpoint_verification: false }), [],
    'a contradictory flag cannot become a legacy provider recovery')
  for (const changes of [
    { workspace_fingerprint: null }, { expected_head_commit: null },
    { recoveries: [{ status: 'authorized' }] }, { recoveries: [{ status: 'running' }] },
  ]) assert.deepEqual(factoryRecoveryModes({ ...value, ...changes }), [])
  assert.equal(factoryRecoveryBlocksProviderResume(true, value), true,
    'correction is a governed mode, never generic provider resume')
})

test('issue210 verifier availability is independent of correction and missing availability preserves legacy behavior', () => {
  for (const available of [true, false, undefined]) {
    for (const correction of [true, false, undefined]) {
      for (const state of ['blocked', 'verification_failed']) {
        const value = correctionContext()
        value.work_item.state = state
        if (available !== undefined) value.checkpoint_verification_available = available
        if (correction === undefined) delete value.checkpoint_source_correction
        else value.checkpoint_source_correction = correction
        const expected = []
        if (available !== false) expected.push('checkpoint-verification')
        if (correction === true) expected.push('source-correction')
        assert.deepEqual(factoryRecoveryModes(value), expected)
        assert.equal(factoryRecoveryBlocksProviderResume(true, value), true)
        assert.equal(factoryContractRevisionSource(value, scope, id(40)), correction === true ? id(50) : null)
      }
    }
  }
  assert.deepEqual(factoryRecoveryModes({
    ...context(), checkpoint_verification_available: false, checkpoint_source_correction: true,
  }), [], 'even correction authority cannot turn cancelled intent into provider execution')
  for (const checkpoint_verification of [false, undefined]) {
    const ordinary = correctionContext()
    ordinary.work_item.state = 'verification_failed'
    ordinary.checkpoint_verification = checkpoint_verification
    ordinary.checkpoint_verification_available = false
    delete ordinary.checkpoint_source_correction
    assert.deepEqual(factoryRecoveryModes(ordinary), ['verifier-only', 'source-correction'],
      'availability only changes checkpoint-family modes')
  }
})

test('issue210 revision source is the exact server-selected verifier, with no cross-item fallback', () => {
  const value = correctionContext()
  assert.equal(factoryContractRevisionSource(value, scope, id(40)), id(50))
  for (const changes of [
    { itemId: id(999) }, { corpId: id(999) }, { missionId: id(999) }, { version: 61 },
  ]) assert.equal(factoryContractRevisionSource(value, { ...scope, ...changes }, id(40)), null)
  for (const change of [
    (entry) => { entry.work_item.mission_id = id(999) },
    (entry) => { entry.mission_id = id(999) },
    (entry) => { entry.source_run_id = nil },
    (entry) => { entry.task_id = id(999) },
    (entry) => { entry.checkpoint_source_correction = false },
    (entry) => { delete entry.checkpoint_source_correction },
    (entry) => { entry.recoveries = [{ status: 'running' }] },
  ]) {
    const entry = correctionContext()
    change(entry)
    assert.equal(factoryContractRevisionSource(entry, scope, id(40)), null)
  }
  assert.equal(factoryContractRevisionSource(null, scope, id(40)), null)
  assert.equal(factoryContractRevisionSource({ ...context(), checkpoint_source_correction: true }, scope, id(40)), null)
  const legacy = { ...value, checkpoint_verification: false }
  delete legacy.checkpoint_source_correction
  assert.equal(factoryContractRevisionSource(legacy, scope, id(40)), id(50),
    'ordinary recovery keeps its server-selected revision source')
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
function initializer(name, root = card) {
  const found = all(root, (node) => ts.isVariableDeclaration(node)
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

test('issue210 presentation separates provider-free verification from a budgeted same-session fix', () => {
  const result = presentation(correctionContext())
  assert.equal(result.heading, 'Verify or fix saved work')
  assert.match(result.detail, /without starting an agent/)
  assert.match(result.detail, /same saved session/)
  assert.match(result.detail, /remaining budget and attempts/)
  assert.match(result.detail, /required checks stay in place/)
  const legacy = correctionContext()
  delete legacy.checkpoint_source_correction
  assert.equal(presentation(legacy).heading, 'Verify retained checkpoint')
})

test('issue210 source-only and unavailable checkpoint presentations never suggest generic recovery', () => {
  const value = { ...correctionContext(), checkpoint_verification_available: false }
  const sourceOnly = presentation(value)
  assert.equal(sourceOnly.state, 'ready')
  assert.equal(sourceOnly.heading, 'Fix saved work')
  assert.match(sourceOnly.detail, /same saved session/)
  assert.match(sourceOnly.detail, /Checkpoint verification is not available/)
  assert.doesNotMatch(sourceOnly.detail, /without starting an agent|without a model call/)
  for (const checkpoint_source_correction of [false, undefined]) {
    const unavailable = presentation({ ...value, checkpoint_source_correction })
    assert.equal(unavailable.state, 'unavailable')
    assert.equal(unavailable.heading, 'Recovery unavailable')
    assert.match(unavailable.detail, /No recovery mode is currently available/)
    assert.equal(unavailable.checkpoint, null)
    assert.doesNotMatch(unavailable.detail, /recheck saved work|request a focused correction/iu)
  }
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
    (value) => { value.checkpoint_verification_available = 'false' },
    (value) => { value.checkpoint_verification_available = null },
    (value) => { value.checkpoint_source_correction = 'true' },
    (value) => { value.checkpoint_source_correction = null },
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

test('issue210 actual loader accepts boolean correction flags and defaults missing legacy authority closed', async () => {
  for (const flag of [true, false, undefined]) {
    const value = correctionContext()
    if (flag === undefined) delete value.checkpoint_source_correction
    else value.checkpoint_source_correction = flag
    const { requests, published } = await requestContext(value)
    assert.equal(requests[0][1].method, 'GET')
    const load = published.at(-1)
    assert.equal(load.status, 'ready')
    assert.equal(factoryRecoveryModes(load.data).includes('source-correction'), flag === true)
  }
})

test('issue210 exact loader retains true, false and missing checkpoint availability without granting another mode', async () => {
  for (const available of [true, false, undefined]) {
    const value = correctionContext()
    value.checkpoint_source_correction = false
    if (available !== undefined) value.checkpoint_verification_available = available
    const { requests, published } = await requestContext(value)
    assert.equal(requests[0][1].method, 'GET')
    const load = published.at(-1)
    assert.equal(load.status, 'ready')
    assert.deepEqual(factoryRecoveryModes(load.data), available === false ? [] : ['checkpoint-verification'])
    assert.equal(load.data.checkpoint_verification_available, available)
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
    'recoveryCommandAvailable', 'recoveryCommand', 'recoveryCopyKey', 'copyRecoveryCommand']) {
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

test('issue210 actual correction action retains the original CLI route, source and connection', async () => {
  for (const role of ['owner', 'admin', 'manager']) {
    const { globals, writes } = commandHarness(correctionContext(), role)
    await globals.copyRecoveryCommand('source-correction')
    await globals.copyRecoveryCommand('checkpoint-verification')
    assert.equal(writes.length, 2)
    assert.match(writes[0], /--verification-recovery source-correction/)
    assert.match(writes[1], /--verification-recovery checkpoint-verification/)
    for (const command of writes) {
      assert.match(command, /--repository 'owner\/lab'/)
      assert.match(command, /--source-base-ref 'main'/)
      assert.match(command, /--adapter 'github-copilot'/)
      assert.match(command, /--model 'reviewed-model'/)
      assert.match(command, /--issue 4/)
      assert.match(command, /--budget-tokens 1000000/)
      assert.ok(command.includes(`--workspace-connection-id '${id(198)}'`))
      assert.doesNotMatch(command, /source-repository-path|checkpoint-reconciliation|resume-run/)
    }
  }
  for (const flag of [false, undefined]) {
    const denied = commandHarness({ ...correctionContext(), checkpoint_source_correction: flag })
    await denied.globals.copyRecoveryCommand('source-correction')
    assert.equal(denied.writes.length, 0)
  }
  for (const role of ['member', 'guest', 'spectator']) {
    const denied = commandHarness(correctionContext(), role)
    await denied.globals.copyRecoveryCommand('source-correction')
    assert.equal(denied.writes.length, 0)
  }
  const cancelled = commandHarness({ ...context(), checkpoint_source_correction: true })
  await cancelled.globals.copyRecoveryCommand('source-correction')
  assert.equal(cancelled.writes.length, 0)
})

test('issue210 actual card drops correction authority immediately when item, version or actor changes', async () => {
  const { published } = await requestContext({
    ...correctionContext(), checkpoint_verification_available: false,
  })
  const load = published.at(-1)
  for (const changes of [
    { itemId: id(999) }, { corpId: id(999) }, { missionId: id(999) },
    { version: 61 }, { actorId: id(12) }, { reload: 1 },
  ]) {
    const globals = { recoveryScope: { ...scope, ...changes }, recoveryContextLoad: load }
    evaluate(`${functionNode('factoryRecoveryScopeKey').getText(app)}
      ${functionNode('currentFactoryRecoveryLoad').getText(app)}`, globals)
    for (const name of ['scopedRecoveryLoad', 'recoveryContext']) {
      evaluate(`globalThis.${name} = ${initializer(name)};`, globals)
    }
    assert.equal(globals.recoveryContext, null)
    const denied = commandHarness(globals.recoveryContext)
    await denied.globals.copyRecoveryCommand('source-correction')
    assert.equal(denied.writes.length, 0)
  }
})

const revisionPanel = functionNode('ContractRevisionPanel')
function revisionHarness({
  value = correctionContext(), status = 'ready', selectedScope = scope,
  role = 'owner', actorId = scope.actorId, missionStatus = 'failed', runs,
} = {}) {
  const requests = []
  const globals = {
    factoryContractRevisionSource,
    recoveryScope: selectedScope,
    recoveryLoad: status === null ? null : {
      scopeKey: JSON.stringify([scope.corpId, scope.actorId, scope.missionId, scope.itemId, scope.version, scope.reload]),
      status, data: status === 'ready' ? value : null, error: status === 'error' ? 'lookup failed' : null,
    },
    mission: { id: scope.missionId, requested_by: scope.actorId, status: missionStatus },
    task: { id: id(40), mission_id: scope.missionId, status: 'verification_failed', contract_version: 2 },
    // The wrong-first provider is visible; the authoritative verifier has no session.
    runs: runs ?? [
      { id: id(49), task_id: id(40), status: 'cancelled', provider_session_id: id(70),
        workspace_disposition: 'preserved', breaker_stage: 'suspend' },
      { id: id(50), task_id: id(40), status: 'failed', provider_session_id: null,
        workspace_disposition: 'preserved', breaker_stage: null },
    ],
    actorId, actorRole: role, busy: false,
    terminalRun: (state) => ['completed', 'failed', 'cancelled', 'lost'].includes(state),
    parsedContract: { objective: 'Finish the retained app' }, parsedPolicy: { checks: [] },
    reason: 'Fix the retained source without changing its evidence requirements.',
    parseError: null, description: 'Reviewed correction', idempotencyKey: id(80),
    onRevise: async (...args) => { requests.push(args); return false },
  }
  evaluate(`${functionNode('factoryRecoveryScopeKey').getText(app)}
    ${functionNode('currentFactoryRecoveryLoad').getText(app)}`, globals)
  for (const name of ['canRevise', 'activeRun', 'redispatchEligible', 'scopedRecoveryLoad',
    'recoveryContext', 'sourceRunId', 'nextAction', 'recoverySourceNotice', 'submit']) {
    evaluate(`globalThis.${name} = ${initializer(name, revisionPanel)};`, globals)
  }
  return { globals, requests }
}

test('issue210 actual contract submission uses the scoped verifier, not its provider ancestor', async () => {
  const { globals, requests } = revisionHarness()
  assert.equal(globals.sourceRunId, id(50))
  assert.equal(globals.nextAction, 'resume')
  await globals.submit({ preventDefault() {} })
  await globals.submit({ preventDefault() {} })
  assert.equal(requests.length, 2)
  for (const [, , request] of requests) {
    assert.equal(request.source_run_id, id(50))
    assert.equal(request.task_id, id(40))
    assert.equal(request.expected_contract_version, 2)
    assert.equal(request.next_action, 'resume')
    assert.equal(request.idempotency_key, id(80), 'lost-response retry retains its key')
  }
  const withoutVisibleVerifier = revisionHarness({ runs: [] })
  assert.equal(withoutVisibleVerifier.globals.sourceRunId, id(50),
    'the snapshot is not an alternate source selector')
  const usage = all(card, (node) => ts.isJsxSelfClosingElement(node)
    && node.tagName.getText(app) === 'ContractRevisionPanel')
  assert.equal(usage.length, 1)
  for (const [name, expected] of [['recoveryScope', 'recoveryScope'], ['recoveryLoad', 'scopedRecoveryLoad']]) {
    const attribute = usage[0].attributes.properties.find((entry) => entry.name?.getText(app) === name)
    assert.equal(attribute?.initializer?.expression?.getText(app), expected)
  }
})

test('issue210 editor loading, errors, stale context and denied correction cannot fall back or submit', async () => {
  for (const options of [
    { status: null }, { status: 'loading' }, { status: 'error' },
    { selectedScope: { ...scope, version: 61 } },
    { selectedScope: { ...scope, itemId: id(999) } },
    { selectedScope: { ...scope, actorId: id(12) } },
    { value: { ...correctionContext(), checkpoint_source_correction: false } },
    { value: { ...correctionContext(), task_id: id(999) } },
    { value: { ...context(), checkpoint_source_correction: true } },
  ]) {
    const { globals, requests } = revisionHarness(options)
    assert.equal(globals.sourceRunId, null)
    assert.equal(globals.nextAction, null)
    await globals.submit({ preventDefault() {} })
    assert.equal(requests.length, 0)
  }
  assert.match(revisionHarness({ status: 'loading' }).globals.recoverySourceNotice, /Loading/)
  assert.match(revisionHarness({ status: 'error' }).globals.recoverySourceNotice, /unavailable.*Refresh/)
  const notice = all(revisionPanel, (node) => ts.isJsxOpeningElement(node)
    && node.tagName.getText(app) === 'p').find((node) => node.getText(app).includes('contract-revision-source-'))
  assert.ok(notice?.getText(app).includes("'alert' : 'status'"))
})

test('issue210 ordinary contract resume, redispatch and requester/role boundaries stay intact', async () => {
  const ordinary = revisionHarness({ selectedScope: null })
  assert.equal(ordinary.globals.sourceRunId, id(49))
  await ordinary.globals.submit({ preventDefault() {} })
  assert.equal(ordinary.requests[0][2].source_run_id, id(49))
  const ready = revisionHarness({ selectedScope: null, missionStatus: 'ready', runs: [] })
  assert.equal(ready.globals.nextAction, 'redispatch')
  await ready.globals.submit({ preventDefault() {} })
  assert.equal(ready.requests[0][2].source_run_id, null)
  const requester = revisionHarness({ selectedScope: null, role: 'member' })
  await requester.globals.submit({ preventDefault() {} })
  assert.equal(requester.requests.length, 1)
  const denied = revisionHarness({ actorId: id(12), role: 'member' })
  await denied.globals.submit({ preventDefault() {} })
  assert.equal(denied.requests.length, 0)
})

test('issue210 actual source-only context copies correction and revises the exact verifier; no-mode context does neither', async () => {
  const value = { ...correctionContext(), checkpoint_verification_available: false }
  const { published } = await requestContext(value)
  assert.equal(published.at(-1).status, 'ready')
  const current = published.at(-1).data
  const { globals, writes } = commandHarness(current)
  assert.equal(globals.recoveryCommandAvailable, true)
  await globals.copyRecoveryCommand('checkpoint-verification')
  await globals.copyRecoveryCommand('verifier-only')
  assert.equal(writes.length, 0)
  await globals.copyRecoveryCommand('source-correction')
  assert.equal(writes.length, 1)
  assert.match(writes[0], /--verification-recovery source-correction/)
  assert.ok(writes[0].includes(`--workspace-connection-id '${id(198)}'`))
  const revision = revisionHarness({ value: current })
  await revision.globals.submit({ preventDefault() {} })
  assert.equal(revision.requests[0][2].source_run_id, current.source_run_id)
  for (const checkpoint_source_correction of [false, undefined]) {
    const denied = { ...current, checkpoint_source_correction }
    const command = commandHarness(denied)
    assert.equal(command.globals.recoveryCommandAvailable, false)
    for (const mode of ['checkpoint-verification', 'source-correction', 'verifier-only']) {
      await command.globals.copyRecoveryCommand(mode)
    }
    assert.equal(command.writes.length, 0)
    const editor = revisionHarness({ value: denied })
    await editor.globals.submit({ preventDefault() {} })
    assert.equal(editor.requests.length, 0)
  }
  const wrongItem = { ...current, work_item: { ...current.work_item, id: id(999) } }
  const rejected = await requestContext(wrongItem)
  assert.equal(rejected.published.at(-1).status, 'error')
  assert.equal(rejected.published.at(-1).data, null)
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

test('actual App Resume control survives an ineligible ordinary Factory recovery lookup and uses the native actor-bound POST', async () => {
  const loads = [], calls = [], remembered = []
  const ordinaryRun = {
    id: id(50), task_id: id(40), provider_session_id: 'native-session',
    status: 'cancelled', execution_mode: 'provider', breaker_stage: null,
    workspace_disposition: 'preserved', workspace_run_id: id(50),
  }
  const globals = {
    AbortController, setTimeout: () => 1, clearTimeout: () => {},
    api: async (path, init) => {
      calls.push({ path, init })
      if (init.method === 'GET') throw new Error('factory recovery context requires exactly one failed task; found 0')
      return {}
    },
    bootstrap: { corp_id: scope.corpId }, selectedActor: { id: scope.actorId },
    setBusy: () => {}, setError: () => {},
    refresh: async () => ({ snapshot: { runs: [{
      id: id(51), task_id: id(40), resumed_from_run_id: id(50),
    }] } }),
    factoryRecoveryBlocksProviderResume, recoveryScope: scope, recoveryContextLoad: null,
    factoryItem: { ...context().work_item, state: 'running' }, recoveryItemState: 'running',
    mission: { id: scope.missionId },
    tasks: [{ id: id(40), mission_id: scope.missionId, status: 'cancelled', verification_status: 'pending' }],
    runs: [ordinaryRun], factoryRecoveries: [], resumableRun: ordinaryRun, evidenceRun: ordinaryRun,
    hasUnfinishedRuns: false, busy: false, resumeStopBlocked: false, resumeBudgetBlocked: false,
    pendingBudgetRevision: null, rememberEvidenceRun: (runId) => remembered.push(runId),
    formatTokens: String, formatUsd: String,
  }
  evaluate(`${functionNode('factoryRecoveryScopeKey').getText(app)}
    ${functionNode('currentFactoryRecoveryLoad').getText(app)}
    ${functionNode('requestFactoryRecoveryContext').getText(app)}`, globals)
  const cleanup = globals.requestFactoryRecoveryContext(scope, (load) => loads.push(load))
  await new Promise(setImmediate)
  cleanup()
  globals.recoveryContextLoad = loads.at(-1)
  assert.equal(globals.recoveryContextLoad.status, 'error')
  assert.equal(globals.recoveryContextLoad.data, null)
  const guard = all(card, (node) => ts.isConditionalExpression(node)
    && node.whenTrue.getText(app).includes('onClick={() => void resumeEvidence()}'))[0]
  assert.ok(guard, 'use the actual MissionCard branch, disabled predicates and click callback')
  const rendered = ts.transpileModule(`globalThis.renderResume = () => (${guard.getText(app)});`, {
    compilerOptions: { target: ts.ScriptTarget.ES2023, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX },
  })
  globals.require = (name) => {
    assert.equal(name, 'react/jsx-runtime')
    return jsxRuntime
  }
  globals.exports = {}
  vm.runInNewContext(rendered.outputText, globals)
  const refreshControls = () => {
    for (const name of ['scopedRecoveryLoad', 'recoveryContext', 'resumeLineageRuns', 'requiresFactoryRecovery', 'resumeRecoveryBlocked']) {
      // The pre-fix App has no separate mandatory-recovery predicate.
      const declaration = all(card, (node) => ts.isVariableDeclaration(node)
        && ts.isIdentifier(node.name) && node.name.text === name)
      if (declaration.length) evaluate(`globalThis.${name} = ${initializer(name)};`, globals)
    }
    evaluate(`globalThis.resumeEvidence = ${initializer('resumeEvidence')};`, globals)
    return globals.renderResume()
  }
  evaluate(`globalThis.onResume = ${initializer('resumeAgentRun', functionNode('App'))};`, globals)
  const tree = refreshControls()
  const html = renderToStaticMarkup(tree)
  assert.match(html, /Resume agent session/)
  assert.doesNotMatch(html, /disabled=/)
  const resumeButton = tree.props.children.find((node) => node?.type === 'button')
  assert.ok(resumeButton)
  resumeButton.props.onClick()
  await new Promise(setImmediate)
  assert.equal(calls.length, 2, 'one failed read and one explicit native resume, not a recovery mutation')
  assert.equal(calls[1].path, `/api/corps/${scope.corpId}/runs/${ordinaryRun.id}/resume`)
  assert.equal(calls[1].init.method, 'POST')
  const body = JSON.parse(calls[1].init.body)
  assert.equal(body.requested_by, scope.actorId)
  assert.deepEqual(Object.keys(body).sort(), ['prompt', 'requested_by'])
  assert.deepEqual(remembered, [id(51)])
  // A sibling task's recovered history, or another workspace for this same
  // task, must not hide the selected ordinary session's native resume.
  for (const historical of [
    { id: id(70), task_id: id(41), workspace_run_id: id(70) },
    { id: id(71), task_id: ordinaryRun.task_id, workspace_run_id: id(71) },
  ]) {
    globals.runs = [ordinaryRun, { ...historical, execution_mode: 'verification_only', breaker_stage: 'stop' }]
    globals.tasks.push({ id: id(41), mission_id: scope.missionId, status: 'verification_failed', verification_status: 'failed' })
    globals.factoryRecoveries = [{
      factory_work_item_id: scope.itemId, mission_id: scope.missionId,
      task_id: historical.task_id, source_run_id: historical.id,
      mode: 'checkpoint_verification', status: 'completed',
    }]
    assert.match(renderToStaticMarkup(refreshControls()), /Resume agent session/)
    globals.tasks.pop()
  }
  globals.runs = [ordinaryRun]
  globals.factoryRecoveries = []
  for (const changes of [
    { recoveryItemState: 'verification_failed', factoryItem: { ...globals.factoryItem, state: 'verification_failed' } },
    { recoveryItemState: 'cancelled', factoryItem: { ...globals.factoryItem, state: 'cancelled' } },
    { tasks: [{ ...globals.tasks[0], status: 'verification_failed' }] },
    { runs: [{ ...ordinaryRun, breaker_stage: 'suspend' }] },
    { runs: [{ ...ordinaryRun, breaker_stage: 'stop' }] },
    { runs: [{ ...ordinaryRun, execution_mode: 'verification_only' }] },
    { factoryRecoveries: [{ factory_work_item_id: scope.itemId, mission_id: scope.missionId,
      task_id: ordinaryRun.task_id, source_run_id: ordinaryRun.id,
      mode: 'checkpoint_verification', status: 'completed' }] },
    { factoryRecoveries: [{ factory_work_item_id: scope.itemId, mission_id: scope.missionId,
      task_id: ordinaryRun.task_id, source_run_id: id(80), mode: 'source_correction', status: 'authorized' }] },
  ]) {
    const original = Object.fromEntries(Object.keys(changes).map((key) => [key, globals[key]]))
    Object.assign(globals, changes)
    assert.equal(refreshControls(), null, 'unavailable governed recovery must remain fail-closed')
    await globals.resumeEvidence()
    assert.equal(calls.length, 2)
    Object.assign(globals, original)
  }
  globals.recoveryContextLoad = {
    scopeKey: JSON.stringify([scope.corpId, scope.actorId, scope.missionId, scope.itemId, scope.version, scope.reload]),
    status: 'ready', error: null, data: { ...context(),
      work_item: { ...globals.factoryItem }, checkpoint_cancellation_event_id: null },
  }
  globals.recoveryContextLoad.data.task_id = id(41)
  assert.match(renderToStaticMarkup(refreshControls()), /Resume agent session/,
    'another task native checkpoint context is not a block on this session')
  globals.recoveryContextLoad.data.task_id = ordinaryRun.task_id
  assert.equal(refreshControls(), null, 'positive native checkpoint context is never generic-resume authority')
  await globals.resumeEvidence()
  assert.equal(calls.length, 2)
})

function failedProviderCorrectionContext() {
  return {
    ...correctionContext(),
    source_run_id: id(51),
    workspace_fingerprint: 'c'.repeat(64),
    expected_head_commit: null,
    checkpoint_verification_available: false,
    checkpoint_source_correction: true,
    recoveries: [{
      status: 'failed', mode: 'source_correction', task_id: id(40),
      source_run_id: id(50), replacement_run_id: id(51),
    }],
  }
}

test('issue221 explicit provider-correction permission permits null head without granting checkpoint or generic resume', () => {
  const value = failedProviderCorrectionContext()
  const original = structuredClone(value)
  assert.deepEqual(factoryRecoveryModes(value), ['source-correction'])
  assert.equal(factoryContractRevisionSource(value, scope, id(40)), id(51),
    'the latest failed provider, not its earlier verifier, remains the exact revision source')
  assert.equal(factoryRecoveryBlocksProviderResume(true, value), true)
  for (const changes of [
    { expected_head_commit: undefined }, { expected_head_commit: '' },
    { expected_head_commit: 'short' }, { expected_head_commit: 'g'.repeat(40) },
    { expected_head_commit: 40 }, { workspace_fingerprint: null },
    { workspace_fingerprint: 'c'.repeat(63) }, { source_run_id: nil },
    { task_id: nil }, { checkpoint_verification: false },
    { checkpoint_source_correction: false }, { checkpoint_source_correction: undefined },
    { checkpoint_verification_available: true }, { checkpoint_verification_available: undefined },
    { recoveries: [{ status: 'authorized' }] }, { recoveries: [{ status: 'running' }] },
  ]) {
    const denied = { ...value, ...changes }
    assert.deepEqual(factoryRecoveryModes(denied), [])
    assert.equal(factoryContractRevisionSource(denied, scope, id(40)), null)
  }
  assert.deepEqual(factoryRecoveryModes({
    ...value, work_item: { ...value.work_item, state: 'cancelled' },
    checkpoint_cancellation_event_id: id(60),
  }), [], 'source-correction permission cannot override cancelled Factory intent')
  for (const expected_head_commit of ['a'.repeat(40), 'd'.repeat(64)]) {
    assert.deepEqual(factoryRecoveryModes({ ...value, expected_head_commit }), ['source-correction'])
    assert.deepEqual(factoryRecoveryModes({
      ...value, expected_head_commit, checkpoint_verification_available: true,
    }), ['checkpoint-verification', 'source-correction'])
    assert.deepEqual(factoryRecoveryModes({
      ...value, expected_head_commit, checkpoint_source_correction: false,
    }), [], 'a head by itself does not override an explicit checkpoint denial')
  }
  assert.deepEqual(value, original, 'mode selection cannot rewrite native history or current source proof')
})

test('issue221 actual context reader retains null head/current provider proof and malformed heads remain non-actionable', async () => {
  const value = failedProviderCorrectionContext()
  const { requests, published } = await requestContext(value)
  assert.equal(requests.length, 1)
  assert.equal(requests[0][1].method, 'GET')
  const current = published.at(-1)
  assert.equal(current.status, 'ready')
  assert.equal(current.data.source_run_id, id(51))
  assert.equal(current.data.workspace_fingerprint, 'c'.repeat(64))
  assert.equal(current.data.expected_head_commit, null)
  assert.deepEqual(current.data.recoveries, value.recoveries)
  assert.deepEqual(factoryRecoveryModes(current.data), ['source-correction'])
  assert.equal(presentation(current.data).heading, 'Fix saved work')
  for (const expected_head_commit of [undefined, false, {}, '', 'short', 'g'.repeat(40)]) {
    const { published: rejected } = await requestContext({ ...value, expected_head_commit })
    const load = rejected.at(-1)
    // The App DTO reader rejects missing/non-string fields. String shape is
    // checked at the mode boundary; neither path can expose a copy action.
    assert.equal(load.status, typeof expected_head_commit === 'string' ? 'ready' : 'error')
    assert.deepEqual(factoryRecoveryModes(load.data), [])
    const denied = commandHarness(load.data)
    assert.equal(denied.globals.recoveryCommandAvailable, false)
    for (const mode of ['checkpoint-verification', 'verifier-only', 'source-correction']) {
      await denied.globals.copyRecoveryCommand(mode)
    }
    assert.equal(denied.writes.length, 0)
  }
  for (const changes of [
    { work_item: { ...value.work_item, version: value.work_item.version + 1 } },
    { source_run_id: '' }, { checkpoint_source_correction: 'true' },
  ]) {
    const { published: rejected } = await requestContext({ ...value, ...changes })
    assert.equal(rejected.at(-1).status, 'error')
    assert.equal(rejected.at(-1).data, null)
  }
})

test('issue221 actual copy/revision actions use only source correction for the latest failed provider with null head', async () => {
  const { published } = await requestContext(failedProviderCorrectionContext())
  const current = published.at(-1).data
  assert.ok(current)
  for (const role of ['owner', 'admin', 'manager']) {
    const { globals, writes } = commandHarness(current, role)
    assert.equal(globals.recoveryContext.source_run_id, id(51))
    assert.equal(globals.recoveryCommandAvailable, true)
    assert.equal(writes.length, 0, 'reading context does not copy or execute anything')
    await globals.copyRecoveryCommand('checkpoint-verification')
    await globals.copyRecoveryCommand('verifier-only')
    assert.equal(writes.length, 0)
    await globals.copyRecoveryCommand('source-correction')
    assert.equal(writes.length, 1)
    assert.match(writes[0], /--verification-recovery source-correction/)
    assert.match(writes[0], /--repository 'owner\/lab'/)
    assert.match(writes[0], /--source-base-ref 'main'/)
    assert.ok(writes[0].includes(`--workspace-connection-id '${id(198)}'`))
    assert.ok(globals.recoveryCopyKey('source-correction').includes(`:${id(51)}:`))
    assert.doesNotMatch(writes[0], /--expected-head-commit|--max-attempts|resume-run|source-repository-path/)
  }
  const revision = revisionHarness({ value: current })
  assert.equal(revision.globals.sourceRunId, id(51), 'no fallback to visible provider/verifier ancestors')
  await revision.globals.submit({ preventDefault() {} })
  assert.equal(revision.requests.length, 1)
  assert.equal(revision.requests[0][2].source_run_id, id(51))
  assert.equal(revision.requests[0][2].next_action, 'resume')
  for (const role of ['member', 'guest', 'spectator']) {
    const denied = commandHarness(current, role)
    await denied.globals.copyRecoveryCommand('source-correction')
    assert.equal(denied.writes.length, 0)
  }
  for (const checkpoint_source_correction of [false, undefined]) {
    const denied = commandHarness({ ...current, checkpoint_source_correction })
    await denied.globals.copyRecoveryCommand('source-correction')
    assert.equal(denied.writes.length, 0)
  }
})
