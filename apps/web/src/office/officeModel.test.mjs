import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import {
  currentOfficeAgents, officeNextAction, officePageForAgent, paginateOfficeAgents,
  resolveOfficeState, resolveOfficeView, selectOfficeAgent, shouldAnimateArrival,
} from './officeModel.ts'

const agent = (id = 'agent-01', overrides = {}) => ({
  id, name: id, role: 'worker', adapter: 'github-copilot',
  status: 'idle', station: null, current_run_id: null, ...overrides,
})
const ids = (agents) => agents.map(({ id }) => id)

test('omitted and null retirement fields keep legacy and mission identities available', () => {
  const entries = [
    agent('legacy'),
    agent('mission', { mission_id: 'mission-a', pinned: false, retired_at: null }),
    agent('pinned', { mission_id: null, pinned: true, retired_at: null }),
  ]
  assert.deepEqual(currentOfficeAgents(entries), entries)
})

test('every non-null retirement excludes an identity regardless of pin, date, or reported work', () => {
  const retired = [
    agent('retired', { retired_at: '2026-09-06T10:00:00Z' }),
    agent('pinned', { pinned: true, retired_at: '2026-09-06T10:00:00Z' }),
    agent('future', { retired_at: '2099-01-01T00:00:00Z' }),
    agent('working', { status: 'working', current_run_id: 'old-run', retired_at: '2026-09-06T10:00:00Z' }),
  ]
  assert.deepEqual(currentOfficeAgents(retired), [])
  assert.deepEqual(paginateOfficeAgents(retired), [])
  assert.equal(selectOfficeAgent(retired, 'working'), undefined)
})

test('retired identity remains resolvable in the unmodified historical snapshot', () => {
  const retired = Object.freeze(agent('old-worker', {
    mission_id: 'old-mission', pinned: false, retired_at: '2026-09-06T10:00:00Z',
  }))
  const snapshot = Object.freeze([retired, Object.freeze(agent('current-worker'))])
  const current = currentOfficeAgents(snapshot)
  assert.deepEqual(ids(current), ['current-worker'])
  assert.equal(snapshot.find(({ id }) => id === 'old-worker'), retired)
  assert.equal(snapshot[0].status, 'idle')
  assert.equal(snapshot.length, 2)
})

test('retiring the selected agent falls back to current crew, including the empty-roster case', () => {
  const entries = [agent('old', { retired_at: '2026-09-06T10:00:00Z' }), agent('new')]
  assert.equal(selectOfficeAgent(entries, 'old')?.id, 'new')
  assert.equal(selectOfficeAgent(entries, 'new')?.id, 'new')
  assert.equal(selectOfficeAgent([], 'old'), undefined)
  assert.equal(selectOfficeAgent([], null), undefined)
})

test('pagination filters retirement before assigning seats without mutating order', () => {
  const entries = Array.from({ length: 14 }, (_, i) =>
    agent(`agent-${String(i).padStart(2, '0')}`, i % 2 ? {} : { retired_at: '2026-09-06T10:00:00Z' }),
  ).reverse()
  const before = ids(entries)
  const pages = paginateOfficeAgents(entries)
  assert.deepEqual(pages.map(({ agents }) => agents.length), [6, 1])
  assert.deepEqual(ids(pages.flatMap(({ agents }) => agents)), [
    'agent-01', 'agent-03', 'agent-05', 'agent-07', 'agent-09', 'agent-11', 'agent-13',
  ])
  assert.deepEqual(ids(entries), before)
})

test('idle does not become working or reviewing from a stale station or pending run request', () => {
  const idle = agent('idle', { station: 'read', current_run_id: 'old-run' })
  const pending = new Set(['old-run'])
  assert.equal(resolveOfficeState(idle, pending, pending), 'idle')
  assert.equal(resolveOfficeState({ ...idle, station: 'build' }, pending, pending), 'idle')
  assert.equal(idle.status, 'idle')
})

test('working and blocked status still derive from exact server state and current-run requests', () => {
  const entry = agent('active', { status: 'working', station: 'read', current_run_id: 'current-run' })
  assert.equal(resolveOfficeState(entry), 'reading')
  assert.equal(resolveOfficeState(entry, new Set(['prior-run'])), 'reading')
  assert.equal(resolveOfficeState(entry, new Set(['current-run'])), 'approval')
  assert.equal(resolveOfficeState({ ...entry, status: 'blocked' }), 'blocked')
})

test('#154: a selected worker on a later studio is located on initial mount and selection changes', () => {
  const pages = paginateOfficeAgents(Array.from({ length: 15 }, (_, i) =>
    agent(`agent-${String(i).padStart(2, '0')}`),
  ))
  assert.equal(officePageForAgent(pages, 'agent-00'), 0)
  assert.equal(officePageForAgent(pages, 'agent-08'), 1)
  assert.equal(officePageForAgent(pages, 'agent-14'), 2)
  assert.equal(officePageForAgent(pages, null), undefined)
  assert.equal(officePageForAgent(pages, 'missing'), undefined)
})

test('#154: studio lookup follows retirement repagination, not stale seat indexes', () => {
  const entries = Array.from({ length: 7 }, (_, i) => agent(`agent-${i}`))
  assert.equal(officePageForAgent(paginateOfficeAgents(entries), 'agent-6'), 1)
  const updated = entries.map((entry, i) => i === 0
    ? { ...entry, retired_at: '2026-09-06T10:00:00Z' } : entry)
  assert.equal(officePageForAgent(paginateOfficeAgents(updated), 'agent-6'), 0)
})

test('#154: manual paging survives snapshots, but another selected agent resets page and zoom', () => {
  const entries = Array.from({ length: 15 }, (_, i) => agent(`agent-${String(i).padStart(2, '0')}`))
  const pages = paginateOfficeAgents(entries)
  const mounted = resolveOfficeView(pages, 'agent-08', null)
  assert.equal(mounted.page, 1)
  const browsing = { ...mounted, page: 0, zoom: 1.5 }
  assert.deepEqual(resolveOfficeView(paginateOfficeAgents([...entries].reverse()), 'agent-08', browsing), browsing)
  const selected = resolveOfficeView(pages, 'agent-14', browsing)
  assert.equal(selected.page, 2)
  assert.equal(selected.zoom, 1)
  assert.equal(resolveOfficeView(pages, 'agent-08', selected).page, 1)
  assert.equal(resolveOfficeView([], null, browsing).page, 0)
})

test('#154: retirement moving the selected agent to a different page resets the camera', () => {
  const entries = Array.from({ length: 7 }, (_, i) => agent(`agent-${i}`))
  const before = { ...resolveOfficeView(paginateOfficeAgents(entries), 'agent-6', null), zoom: 2 }
  const after = resolveOfficeView(paginateOfficeAgents(entries.slice(1)), 'agent-6', before)
  assert.equal(after.page, 0)
  assert.equal(after.zoom, 1)
})

test('#154: mounting or remounting the same starting run never fabricates an arrival', () => {
  const starting = agent('worker', { status: 'starting', current_run_id: 'run-a' })
  for (let mount = 0; mount < 3; mount++) {
    assert.equal(shouldAnimateArrival(undefined, starting, 'starting', true), false)
    assert.equal(shouldAnimateArrival('run-a', starting, 'starting', true), false)
  }
})

test('#154: only an observed new active run can animate once, and reduced motion wins', () => {
  const starting = agent('worker', { status: 'starting', current_run_id: 'run-new' })
  assert.equal(shouldAnimateArrival(null, starting, 'starting', true), true)
  assert.equal(shouldAnimateArrival('run-old', starting, 'starting', true), true)
  assert.equal(shouldAnimateArrival('run-new', starting, 'starting', true), false)
  assert.equal(shouldAnimateArrival('run-old', starting, 'starting', false), false)
  for (const status of ['idle', 'blocked', 'offline', 'approval']) {
    assert.equal(shouldAnimateArrival('run-old', starting, status, true), false)
  }
  assert.equal(shouldAnimateArrival(null, { ...starting, current_run_id: null }, 'starting', true), false)
  assert.equal(shouldAnimateArrival(null, {
    ...starting, retired_at: '2026-09-06T10:00:00Z',
  }, 'starting', true), false)
})

test('#154: blocked work takes recovery priority over both idle and ordinary active work', () => {
  for (const active of [0, 3]) {
    const action = officeNextAction(0, 2, active)
    assert.equal(action.heading, '2 blocked')
    assert.equal(action.label, 'Review blocked work')
    assert.equal(action.attentionState, 'blocked')
    assert.match(action.description, /recovery controls/)
    assert.doesNotMatch(action.heading, /What are we building/)
  }
})

test('#154: exact approvals keep priority, and only an unblocked empty floor offers new work', () => {
  assert.equal(officeNextAction(1, 2, 3).attentionState, 'approval')
  assert.equal(officeNextAction(0, 0, 3).heading, 'Work is in progress.')
  assert.equal(officeNextAction(0, 0, 0).heading, 'What are we building?')
})

test('history inspectors retain the full snapshot while the floor uses current identities', async () => {
  const app = await readFile(new URL('../App.tsx', import.meta.url), 'utf8')
  assert.match(app, /<OfficeFloor\s+agents=\{currentAgents\}/)
  assert.match(app, /<MissionCard[\s\S]*?agents=\{data\.snapshot\.agents\}/)
  assert.match(app, /<FactoryPanel[\s\S]*?agents=\{data\.snapshot\.agents\}/)
  assert.match(app, /data\.snapshot\.missions\.find\(\(mission\) => mission\.id === selectedMissionId\)/)
})
