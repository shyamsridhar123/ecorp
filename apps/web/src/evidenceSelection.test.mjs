import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import { evidenceSelectionKey, readEvidenceSelection, rememberEvidenceSelection } from './evidenceSelection.ts'
import { pendingReviewForRun, selectMissionEvidenceRun } from './workflowContext.ts'

const id = (value) => `00000000-0000-4000-8000-${String(value).padStart(12, '0')}`
const scope = { server: 'http://127.0.0.1:18961', corpId: id(1), actorId: id(12), missionId: id(20) }
const key = evidenceSelectionKey(scope)
const storage = () => {
  const data = new Map()
  return { getItem: (name) => data.get(name) ?? null, setItem: (name, value) => { data.set(name, value) } }
}

test('reload keeps the decided run instead of the other pending review or a newer worker', () => {
  const saved = storage()
  assert.equal(rememberEvidenceSelection(() => saved, key, id(31)), true)
  const entries = [
    { id: id(33), task_id: id(23), status: 'running' },
    { id: id(31), task_id: id(21), status: 'completed' },
    { id: id(32), task_id: id(22), status: 'waiting_for_approval' },
  ]
  const reviews = [{ run_id: id(32), task_id: id(22), status: 'pending' }]
  const afterReload = selectMissionEvidenceRun(entries, reviews, readEvidenceSelection(() => saved, key))
  assert.equal(afterReload, entries[1])
  assert.equal(pendingReviewForRun(afterReload, reviews), undefined)
  assert.equal(rememberEvidenceSelection(() => saved, key, id(32)), true)
  assert.equal(selectMissionEvidenceRun(entries, reviews, readEvidenceSelection(() => saved, key)), entries[2])
})

test('remembered evidence is scoped to the API server, Corp, actor and mission', () => {
  const saved = storage()
  rememberEvidenceSelection(() => saved, key, id(31))
  for (const field of Object.keys(scope)) {
    const otherKey = evidenceSelectionKey({ ...scope, [field]: `${scope[field]}-other` })
    assert.notEqual(otherKey, key)
    assert.equal(readEvidenceSelection(() => saved, otherKey), null)
  }
  assert.notEqual(
    evidenceSelectionKey({ ...scope, actorId: 'a:b', missionId: 'c' }),
    evidenceSelectionKey({ ...scope, actorId: 'a', missionId: 'b:c' }),
  )
})

test('missing remembered work never silently substitutes another visible review', () => {
  const saved = storage()
  rememberEvidenceSelection(() => saved, key, id(31))
  const entries = [{ id: id(32), task_id: id(22), status: 'waiting_for_approval' }]
  const reviews = [{ run_id: id(32), task_id: id(22), status: 'pending' }]
  assert.equal(selectMissionEvidenceRun(entries, reviews, readEvidenceSelection(() => saved, key)), undefined)
  assert.equal(selectMissionEvidenceRun([], reviews, readEvidenceSelection(() => saved, key)), undefined)
  assert.equal(pendingReviewForRun(undefined, reviews), undefined)
})

test('unreadable or malformed stored context requires explicit selection, not the next review', () => {
  const entries = [{ id: id(32), task_id: id(22), status: 'waiting_for_approval' }]
  const reviews = [{ run_id: id(32), task_id: id(22), status: 'pending' }]
  for (const getItem of [
    () => { throw new Error('storage blocked') },
    () => '',
    () => 'not-a-run',
    () => JSON.stringify({ run_id: id(31) }),
    () => 'x'.repeat(4096),
  ]) {
    const selection = readEvidenceSelection(() => ({ getItem }), key)
    assert.equal(selection, '')
    assert.equal(selectMissionEvidenceRun(entries, reviews, selection), undefined)
  }
  assert.equal(readEvidenceSelection(() => { throw new Error('getter blocked') }, key), '')
})

test('a first visit is distinct from unreadable history; failed writes do not throw', () => {
  const saved = storage()
  assert.equal(readEvidenceSelection(() => saved, key), null)
  assert.equal(rememberEvidenceSelection(() => { throw new Error('getter blocked') }, key, id(31)), false)
  assert.equal(rememberEvidenceSelection(() => ({
    setItem: () => { throw new Error('quota exceeded') },
  }), key, id(31)), false)
  assert.equal(rememberEvidenceSelection(() => saved, key, 'not-a-run'), false)
  assert.equal(saved.getItem(key), null)
})

test('only the selected run ID is stored; reviews and authority never enter browser persistence', () => {
  const writes = []
  assert.equal(rememberEvidenceSelection(() => ({ setItem: (...args) => writes.push(args) }), key, id(31)), true)
  assert.deepEqual(writes, [[key, id(31)]])
})

test('MissionCard scopes remounts, pins initial evidence and fails closed when persistence fails', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8')
  const card = app.slice(app.indexOf('function MissionCard('), app.indexOf('function EventRow('))
  assert.match(card, /evidenceSelectionKey\(\{ server: API_URL, corpId, actorId, missionId: mission\.id \}\)/)
  assert.match(card, /readEvidenceSelection\(\(\) => window\.sessionStorage, evidenceStorageKey\)/)
  assert.match(card, /setSelectedEvidenceRunId\(remembered \? displayedEvidenceRunId : ''\)/)
  assert.match(card, /const pendingRun = selectedEvidenceRunId !== null && pendingRequest \? evidenceRun : undefined/)
  assert.match(card, /onChange=\{\(event\) => rememberEvidenceRun\(event\.target\.value\)\}/)
  assert.match(card, /Choose a run to inspect/)
  assert.match(app, /key=\{`\$\{bootstrap\.corp_id\}:\$\{selectedActor\.id\}:\$\{selectedMission\.id\}`\}/)
})
