import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import vm from 'node:vm'
import ts from 'typescript'
import { selectFactoryController } from './factoryControllerSelection.ts'
import { factoryControllerState } from './factoryPolling.ts'

const id = (value) => `00000000-0000-4000-8000-${String(value).padStart(12, '0')}`
const corpId = id(1)
const item = {
  id: 'e6182bf4-0e41-420b-ac03-23a11c5619c5',
  corp_id: corpId,
  source_project_owner: 'shyamsridhar123',
  source_project_number: 3,
  source_repository_owner: 'shyamsridhar123',
  source_repository_name: 'ecorp-enterprise-lab',
}
const controller = (changes = {}) => ({
  ...item,
  id: id(204),
  desired_state: 'running',
  status: 'working',
  active_work_item_id: item.id,
  last_heartbeat_at: '2026-09-09T15:30:00Z',
  version: 17,
  ...changes,
})
const legacy = () => controller({
  id: id(174),
  source_project_number: 174,
  source_repository_name: 'ecorp',
  desired_state: 'paused',
  status: 'offline',
  active_work_item_id: null,
  last_heartbeat_at: '2026-09-01T15:30:00Z',
})

test('wrong-first legacy Project 174 cannot displace the selected live lab controller', () => {
  const live = controller()
  assert.equal(selectFactoryController([legacy(), live], item, corpId), live)
  assert.equal(selectFactoryController([live, legacy()], item, corpId), live)
  assert.equal(factoryControllerState(selectFactoryController([legacy(), live], item, corpId)), 'working')
})

test('every Project/repository/Corp component must match, even with an active-item pointer', () => {
  for (const changes of [
    { source_project_owner: 'another-owner' },
    { source_project_number: 174 },
    { source_repository_owner: 'another-owner' },
    { source_repository_name: 'another-repository' },
    { corp_id: id(2) },
  ]) {
    assert.equal(selectFactoryController([controller(changes)], item, corpId), undefined)
  }
  assert.equal(selectFactoryController([controller()], { ...item, corp_id: id(2) }, corpId), undefined)
  assert.equal(selectFactoryController([controller()], item, id(2)), undefined)
})

test('GitHub owner/repository case and surrounding whitespace normalize independently', () => {
  const mixed = controller({
    source_project_owner: ' SHYAMSRIDHAR123 ',
    source_repository_owner: ' ShyamSridhar123 ',
    source_repository_name: ' ECORP-Enterprise-Lab ',
  })
  assert.equal(selectFactoryController([mixed], item, corpId), mixed)
  assert.equal(selectFactoryController([controller()], { ...item, ...mixed, id: item.id }, corpId)?.id, id(204))
})

test('missing/malformed scope is neutral rather than an empty-scope match', () => {
  for (const changes of [
    { source_project_owner: '' },
    { source_repository_name: ' ' },
    { source_repository_owner: undefined },
    { source_project_number: 0 },
    { source_project_number: '3' },
    { source_project_number: NaN },
  ]) {
    assert.equal(selectFactoryController([controller(changes)], item, corpId), undefined)
    assert.equal(selectFactoryController([controller(changes)], { ...item, ...changes }, corpId), undefined)
  }
  assert.equal(selectFactoryController([controller()], item, ''), undefined)
})

test('the exact active item wins among live controllers in the same scope', () => {
  const exact = controller({ id: id(205), status: 'blocked' })
  const another = controller({
    active_work_item_id: id(99),
    last_heartbeat_at: '2026-09-09T15:31:00Z',
  })
  assert.equal(selectFactoryController([another, exact], item, corpId), exact)
})

test('a current live scope match wins over a stale offline active-item pointer', () => {
  const stale = controller({ status: 'offline' })
  const live = controller({ id: id(205), status: 'watching', active_work_item_id: null })
  assert.equal(selectFactoryController([stale, live], item, corpId), live)
})

test('with only offline matches, retain the exact item rather than crossing scope', () => {
  const exact = controller({ status: 'offline' })
  const another = controller({
    id: id(205), status: 'offline', active_work_item_id: id(99),
    last_heartbeat_at: '2026-09-09T15:31:00Z',
  })
  assert.equal(selectFactoryController([another, legacy(), exact], item, corpId), exact)
})

test('without an item, prefer useful live intake in the current Corp', () => {
  const live = controller()
  const paused = controller({
    id: id(206), desired_state: 'paused', status: 'watching',
    last_heartbeat_at: '2026-09-09T15:32:00Z',
  })
  const foreign = controller({ id: id(207), corp_id: id(2) })
  for (const selected of [undefined, null]) {
    assert.equal(selectFactoryController([legacy(), paused, foreign, live], selected, corpId), live)
  }
})

test('empty/no-match and unavailable explicit selections never borrow another controller', () => {
  assert.equal(selectFactoryController([], item, corpId), undefined)
  assert.equal(selectFactoryController([], undefined, corpId), undefined)
  assert.equal(selectFactoryController([legacy()], item, corpId), undefined)
  assert.equal(factoryControllerState(selectFactoryController([legacy()], item, corpId)), 'not_configured')
  assert.equal(selectFactoryController([controller()], undefined, corpId, item.id), undefined)
  assert.equal(selectFactoryController([controller()], item, corpId, id(999)), undefined)
})

test('heartbeat freshness and stable ID break ties without using array order or mutating inputs', () => {
  const stale = controller({ id: id(206), status: 'offline', last_heartbeat_at: 'invalid' })
  const old = controller({ id: id(207), status: 'offline', last_heartbeat_at: '2026-09-08T15:30:00Z' })
  const newest = controller({ id: id(205), status: 'offline' })
  const tied = controller({ id: id(204), status: 'offline' })
  const input = Object.freeze([stale, old, newest, tied].map(Object.freeze))
  const before = structuredClone(input)
  assert.equal(selectFactoryController(input, undefined, corpId), tied)
  assert.equal(selectFactoryController([...input].reverse(), undefined, corpId), tied)
  assert.deepEqual(input, before)
})

test('snapshot refresh and selected scope changes return the exact new ID/version object', () => {
  const original = controller()
  const current = controller({ version: 18, status: 'watching' })
  const other = legacy()
  assert.equal(selectFactoryController([original, other], item, corpId), original)
  assert.equal(selectFactoryController([current, other], item, corpId), current)
  assert.equal(selectFactoryController([current, other], { ...other, id: id(88) }, corpId), other)
})

// Inspect and execute the actual production wiring with local stubs only.
// No App module mount, browser, timer, fetch, or real API call is involved.
const appText = await readFile(new URL('./App.tsx', import.meta.url), 'utf8')
const app = ts.createSourceFile('App.tsx', appText, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
function findAll(root, predicate) {
  const found = []
  function visit(node) {
    if (predicate(node)) found.push(node)
    ts.forEachChild(node, visit)
  }
  visit(root)
  return found
}
function variable(root, name) {
  const found = findAll(root, (node) =>
    ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.name.text === name)
  assert.equal(found.length, 1, `one production ${name} binding`)
  return found[0].initializer
}
const panel = findAll(app, (node) =>
  ts.isFunctionDeclaration(node) && node.name?.text === 'FactoryPanel')[0]
assert.ok(panel)

function panelSelection(controllers, selectedItemId = item.id, items = [item]) {
  const context = { controllers, selectedItemId, items, scope: { corpId }, selectFactoryController }
  context.selected = vm.runInNewContext(variable(panel, 'selected').getText(app), context)
  const expression = variable(panel, 'controller')
  assert.ok(ts.isCallExpression(expression))
  assert.equal(expression.expression.getText(app), 'selectFactoryController')
  return vm.runInNewContext(expression.getText(app), context)
}

test('the actual FactoryPanel resolver feeds displayed state and both action callbacks', () => {
  const importNode = app.statements.find((node) =>
    ts.isImportDeclaration(node) && node.moduleSpecifier.text === './factoryControllerSelection')
  assert.ok(importNode, 'the component imports the production resolver')
  const live = controller()
  const resolved = panelSelection([legacy(), live])
  assert.equal(resolved, live)
  const state = vm.runInNewContext(variable(panel, 'controllerState').getText(app), {
    controller: resolved, factoryControllerState,
  })
  assert.equal(state, 'working')
  const callbacks = findAll(panel, (node) =>
    ts.isJsxAttribute(node) && node.name.getText(app) === 'onClick'
    && findAll(node, (child) => ts.isCallExpression(child)
      && child.expression.getText(app) === 'onControllerControl').length > 0)
  assert.equal(callbacks.length, 2)
  const calls = []
  for (const attribute of callbacks) {
    const onClick = vm.runInNewContext(`(${attribute.initializer.expression.getText(app)})`, {
      controller: resolved, onControllerControl: (...args) => calls.push(args),
    })
    onClick()
  }
  assert.deepEqual(calls, [[live, 'pause'], [live, 'reconcile']])
  const paused = controller({ desired_state: 'paused', version: 18 })
  const resume = vm.runInNewContext(`(${callbacks[0].initializer.expression.getText(app)})`, {
    controller: panelSelection([legacy(), paused]), onControllerControl: (...args) => calls.push(args),
  })
  resume()
  assert.deepEqual(calls.at(-1), [paused, 'resume'])
  assert.equal(panelSelection([legacy()]), undefined)
  assert.equal(panelSelection([live], id(999)), undefined)
  assert.equal(panelSelection([legacy(), live], null, []), live)
})

test('the actual component guards controller actions, retains role gating and passes the same callback', () => {
  const controls = findAll(panel, (node) =>
    ts.isConditionalExpression(node) && node.condition.getText(app) === 'controller'
    && node.whenTrue.getText(app).includes('onControllerControl'))
  assert.equal(controls.length, 1, 'no controller means no pause/resume/reconcile controls')
  const disabled = findAll(controls[0].whenTrue, (node) =>
    ts.isJsxAttribute(node) && node.name.getText(app) === 'disabled')
  assert.equal(disabled.length, 2)
  assert.ok(disabled.every((node) => node.initializer.expression.getText(app) === 'busy || !canControlFactory'))
  const panelCall = findAll(app, (node) =>
    ts.isJsxSelfClosingElement(node) && node.tagName.getText(app) === 'FactoryPanel')[0]
  const attributes = panelCall.attributes.properties
  const callback = attributes.find((node) => node.name?.getText(app) === 'onControllerControl')
  assert.equal(callback.initializer.expression.getText(app), 'controlFactoryController')
  const roleGate = attributes.find((node) => node.name?.getText(app) === 'canControlFactory')
  for (const role of ['owner', 'admin', 'manager', 'member', 'guest', 'spectator']) {
    assert.equal(vm.runInNewContext(roleGate.initializer.expression.getText(app), {
      selectedActor: { role },
    }), ['owner', 'admin', 'manager'].includes(role))
  }
})

function productionActionHarness(failFirst = false) {
  const requests = []
  const storage = new Map()
  let sequence = 0
  const context = {
    bootstrap: { corp_id: corpId },
    selectedActor: { id: id(11), role: 'owner' },
    window: { sessionStorage: {
      getItem: (key) => storage.get(key) ?? null,
      setItem: (key, value) => storage.set(key, value),
      removeItem: (key) => storage.delete(key),
    } },
    crypto: { randomUUID: () => id(900 + ++sequence) },
    api: async (url, options) => {
      requests.push({ url, body: JSON.parse(options.body) })
      if (failFirst && requests.length === 1) throw new Error('synthetic lost response')
    },
    refresh: async () => {},
    setBusy: () => {},
    setError: () => {},
    setAnnouncement: () => {},
    ApiRequestError: class extends Error {},
  }
  const script = ts.transpileModule(
    `globalThis.control = ${variable(app, 'controlFactoryController').getText(app)};`,
    { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None } },
  ).outputText
  vm.runInNewContext(script, context)
  return { control: context.control, requests, storage }
}

test('the actual API action handler targets the resolved controller ID and version for all actions', async () => {
  const live = controller()
  const resolved = panelSelection([legacy(), live])
  const harness = productionActionHarness()
  for (const action of ['pause', 'resume', 'reconcile']) await harness.control(resolved, action)
  assert.equal(harness.requests.length, 3)
  for (const [index, request] of harness.requests.entries()) {
    assert.equal(request.url, `/api/corps/${corpId}/factory/controllers/${live.id}/control`)
    assert.equal(request.body.expected_version, live.version)
    assert.equal(request.body.actor_id, id(11))
    assert.equal(request.body.action, ['pause', 'resume', 'reconcile'][index])
  }
  assert.equal(harness.storage.size, 0)
})

test('the production handler preserves the existing ID-scoped idempotency/version on a lost response', async () => {
  const harness = productionActionHarness(true)
  await harness.control(panelSelection([legacy(), controller()]), 'pause')
  assert.equal(harness.storage.size, 1)
  await harness.control(panelSelection([legacy(), controller({ version: 18 })]), 'pause')
  assert.deepEqual(harness.requests[1], harness.requests[0])
  assert.equal(harness.requests[1].body.expected_version, 17)
  assert.equal(harness.storage.size, 0)
})
