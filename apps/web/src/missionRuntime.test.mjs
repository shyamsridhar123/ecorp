import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import {
  availableRunnerAdapters, missionRuntimeError, runnerMatchesRepository,
  selectMissionAdapter, STUDIO_STRATEGY, STUDIO_STRATEGY_LABEL, usesDeterministicHarness,
} from './missionRuntime.ts'

const source = {
  key: 'source-a',
  repository: 'owner/product',
  baseRef: 'main',
  baseCommit: 'a'.repeat(40),
  runnerIds: ['runner-a'],
  runnerLabels: ['local'],
}
const model = (id = 'copilot-model', overrides = {}) => ({
  id, name: id, policy_state: 'enabled', policy_terms: null,
  supports_vision: false, supports_reasoning_effort: true,
  max_prompt_tokens: null, max_context_window_tokens: 100_000,
  supported_reasoning_efforts: ['low', 'high'], default_reasoning_effort: 'high',
  billing_multiplier: null, ...overrides,
})
const capability = (name = 'github-copilot', overrides = {}) => ({
  name, available: true, detail: null, models: [model()], ...overrides,
})
const runner = (overrides = {}, workspace = {}) => ({
  id: 'runner-a', corp_id: 'corp-a', hostname: 'local', os: 'windows',
  connected: true, status: 'connected', last_seen_at: '2026-09-06T12:00:00Z',
  grace_expires_at: null,
  capabilities: [
    capability('workspace-isolation', {
      models: [], source_repository: source.repository, source_base_ref: source.baseRef,
      source_base_commit: source.baseCommit, ...workspace,
    }),
    capability(),
  ],
  ...overrides,
})

test('fresh human-only bootstrap can select a runner runtime before any crew exists', () => {
  const data = { runners: [runner()], snapshot: { agents: [] } }
  const adapters = availableRunnerAdapters(data, source)
  assert.deepEqual(adapters.map(({ name }) => name), ['github-copilot'])
  assert.equal(selectMissionAdapter(STUDIO_STRATEGY, adapters, '')?.name, 'github-copilot')
  assert.equal(data.snapshot.agents.length, 0)
})

test('retired and legacy identities neither enable nor suppress advertised runtimes', () => {
  const runners = [runner()]
  for (const agents of [
    [],
    [{ adapter: 'github-copilot', retired_at: '2026-09-06T12:00:00Z' }],
    [{ adapter: 'fake-process' }],
  ]) {
    assert.deepEqual(
      availableRunnerAdapters({ runners, snapshot: { agents } }, source).map(({ name }) => name),
      ['github-copilot'],
    )
  }
})

test('studio requires the exact selected source on a connected runner, not another repository', () => {
  for (const wrong of [
    runner({ connected: false, status: 'grace' }),
    runner({}, { available: false }),
    runner({}, { source_repository: 'owner/other' }),
    runner({}, { source_base_ref: 'release' }),
    runner({}, { source_base_commit: 'b'.repeat(40) }),
    runner({}, { source_base_commit: null }),
  ]) {
    const adapters = availableRunnerAdapters({ runners: [wrong] }, source)
    assert.deepEqual(adapters, [])
    assert.equal(selectMissionAdapter(STUDIO_STRATEGY, adapters, ''), undefined)
  }
})

test('source repository and commit matching are case-insensitive but ref matching is exact', () => {
  assert.equal(runnerMatchesRepository(runner({}, {
    source_repository: 'OWNER/Product', source_base_commit: 'A'.repeat(40),
  }), source), true)
  assert.equal(runnerMatchesRepository(runner({}, { source_base_ref: 'MAIN' }), source), false)
})

test('unavailable runtimes and non-runtime capabilities are not mission options', () => {
  const data = { runners: [runner({ capabilities: [
    ...runner().capabilities.slice(0, 1),
    capability('github-copilot', { available: false }),
    capability('secret-broker'),
    capability('fake-process', { models: [] }),
  ] })] }
  assert.deepEqual(availableRunnerAdapters(data, source).map(({ name }) => name), ['fake-process'])
})

test('catalog merging prefers an enabled model without changing the source snapshot', () => {
  const data = { runners: [
    runner({ capabilities: [
      runner().capabilities[0],
      capability('github-copilot', { models: [model('shared', { policy_state: 'disabled' })] }),
    ] }),
    runner({ id: 'runner-b', capabilities: [
      runner().capabilities[0],
      capability('github-copilot', { models: [model('shared'), model('other')] }),
    ] }),
    runner({ id: 'unrelated', capabilities: [
      runner({}, { source_repository: 'owner/other' }).capabilities[0],
      capability('github-copilot', { models: [model('wrong-source')] }),
    ] }),
  ] }
  const before = structuredClone(data)
  const [adapter] = availableRunnerAdapters(data, source)
  assert.deepEqual(adapter.models.map(({ id }) => id), ['shared', 'other'])
  assert.equal(adapter.models[0].policy_state, 'enabled')
  assert.deepEqual(data, before)
})

test('studio always selects Copilot even when another runtime was requested', () => {
  const adapters = [capability('codex'), capability('fake-process'), capability()]
  assert.equal(selectMissionAdapter(STUDIO_STRATEGY, adapters, 'codex')?.name, 'github-copilot')
  assert.equal(STUDIO_STRATEGY_LABEL, 'Studio team · 3 Copilot agents')
  assert.equal(usesDeterministicHarness(STUDIO_STRATEGY), false)
})

test('studio does not fall back to another provider or an unavailable Copilot runtime', () => {
  for (const adapters of [
    [],
    [capability('codex')],
    [capability('codex'), capability('github-copilot', { available: false })],
  ]) {
    const selected = selectMissionAdapter(STUDIO_STRATEGY, adapters, 'codex')
    assert.equal(selected, undefined)
    assert.match(missionRuntimeError(STUDIO_STRATEGY, selected, ''), /requires GitHub Copilot/)
  }
  assert.match(missionRuntimeError(STUDIO_STRATEGY, capability('codex'), ''), /requires GitHub Copilot/)
})

test('solo selection and deterministic fixtures retain their runtime contracts', () => {
  const adapters = [capability('codex'), capability('fake-process'), capability()]
  assert.equal(selectMissionAdapter('single', adapters, 'codex')?.name, 'codex')
  assert.equal(selectMissionAdapter('single', adapters, '')?.name, 'github-copilot')
  assert.equal(selectMissionAdapter('verification-matrix', adapters, 'codex')?.name, 'fake-process')
  assert.equal(selectMissionAdapter('verification-matrix', [capability()], ''), undefined)
})

test('a disappearing explicit runtime cannot silently switch provider with stale model settings', () => {
  assert.equal(selectMissionAdapter('single', [capability()], 'codex'), undefined)
  assert.match(missionRuntimeError('single', undefined, 'old-model'), /connected runner/)
})

test('studio uses enabled Copilot model settings or an explicit provider default', () => {
  const adapter = capability()
  assert.equal(missionRuntimeError(STUDIO_STRATEGY, adapter, 'copilot-model'), null)
  assert.equal(missionRuntimeError(STUDIO_STRATEGY, adapter, ''), null)
  assert.deepEqual(adapter.models[0].supported_reasoning_efforts, ['low', 'high'])
  assert.equal(adapter.models[0].default_reasoning_effort, 'high')
  assert.match(missionRuntimeError(STUDIO_STRATEGY, adapter, 'codex-only-model'), /unavailable/)
  assert.match(missionRuntimeError(STUDIO_STRATEGY, capability('github-copilot', {
    models: [model('disabled', { policy_state: 'disabled' })],
  }), 'disabled'), /unavailable/)
})

test('fixtures ignore hidden provider model settings', () => {
  assert.equal(missionRuntimeError('verification-matrix', capability('fake-process', {
    models: [],
  }), 'old-provider-model'), null)
})

test('normal UI bootstrap opts out of seeded crew without changing fixture tooling', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8')
  assert.match(app, /api<BootstrapResponse>\('\/api\/demo\/bootstrap\?seed_crew=false'/)
  assert.doesNotMatch(app, /api<BootstrapResponse>\('\/api\/demo\/bootstrap'/)
})
