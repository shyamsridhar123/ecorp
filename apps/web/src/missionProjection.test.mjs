import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import {
  canPostRoomMessage, discussionScopeKey, missionOrigin, resolveDiscussionRoom,
  roomDiscussionMessages, roomWorkContext,
} from './missionProjection.ts'
import { missionIdForLink, relatedWorkOptions } from './workflowContext.ts'

const rooms = [
  { id: 'room-a', name: 'First room', purpose: 'A' },
  { id: 'room-b', name: 'Owning room', purpose: 'B' },
]
const missions = [
  { id: 'm1', room_id: 'room-a', title: 'First mission' },
  { id: 'm2', room_id: 'room-b', title: 'Selected mission' },
  { id: 'm3', room_id: 'room-b', title: 'Another mission in B' },
]
const tasks = missions.map((mission, index) => ({
  id: `t${index + 1}`, mission_id: mission.id, title: `${mission.title} task`,
}))
const runs = tasks.map((task, index) => ({
  id: `r${index + 1}`, task_id: task.id, status: 'completed', artifact_id: `a${index + 1}`,
}))
const messages = [
  { id: 'nested-b', room_id: 'room-b', link: null, reply_to_id: 'reply-b' },
  { id: 'reply-b', room_id: 'room-b', link: null, thread_root_id: 'root-b' },
  { id: 'root-a', room_id: 'room-a', link: { kind: 'mission', id: 'm1' } },
  { id: 'root-b', room_id: 'room-b', link: { kind: 'mission', id: 'm2' } },
  { id: 'task-b', room_id: 'room-b', link: { kind: 'task', id: 't2' } },
  { id: 'run-b', room_id: 'room-b', link: { kind: 'run', id: 'r2' } },
  { id: 'artifact-b', room_id: 'room-b', link: { kind: 'artifact', id: 'a2' } },
  { id: 'other-b', room_id: 'room-b', link: { kind: 'mission', id: 'm3' } },
  { id: 'general-b', room_id: 'room-b', link: null },
  // Adversarial/stale projections must not borrow another room's ancestry.
  { id: 'wrong-room-link', room_id: 'room-a', link: { kind: 'mission', id: 'm2' } },
  { id: 'wrong-room-reply', room_id: 'room-a', link: null, thread_root_id: 'root-b' },
  { id: 'other-context-reply', room_id: 'room-b', link: { kind: 'mission', id: 'm3' }, reply_to_id: 'root-b' },
  { id: 'missing-ancestor', room_id: 'room-b', link: null, reply_to_id: 'absent' },
]
const snapshot = { corp: { id: 'corp' }, rooms, missions, tasks, runs, room_messages: messages }
const scope = { corpId: 'corp', actorId: 'alice', roomId: 'room-b', missionId: 'm2' }
const input = { roomId: 'room-b', replyToId: null, link: { kind: 'mission', id: 'm2' } }

test('mission selection and Discuss use the authoritative non-first owning room', () => {
  assert.equal(resolveDiscussionRoom(rooms, missions, 'm2'), rooms[1])
  assert.equal(resolveDiscussionRoom(rooms, missions, 'm2', 'room-a'), rooms[1])
  assert.equal(resolveDiscussionRoom(rooms.toReversed(), missions, 'm2'), rooms[1])
  assert.equal(missions.length, 3, 'other-room missions remain selectable')
})

test('mission, task, run and artifact deep links resolve to the same owning room', () => {
  for (const [kind, id] of [['mission', 'm2'], ['task', 't2'], ['run', 'r2'], ['artifact', 'a2']]) {
    const missionId = missionIdForLink({ kind, id }, snapshot)
    assert.equal(missionId, 'm2')
    assert.equal(resolveDiscussionRoom(rooms, missions, missionId, 'room-a'), rooms[1])
  }
})

test('an explicit room deep link and all-discussion view stay in that room', () => {
  assert.equal(resolveDiscussionRoom(rooms, missions, null, 'room-b'), rooms[1])
  assert.equal(resolveDiscussionRoom(rooms, missions, null), rooms[0])
  assert.equal(resolveDiscussionRoom(rooms, missions, null, 'missing-room'), undefined)
})

test('stale, missing and older mission/room projections never fall back to the first room', () => {
  assert.equal(resolveDiscussionRoom(rooms, missions, 'missing'), undefined)
  assert.equal(resolveDiscussionRoom(rooms.slice(0, 1), missions, 'm2'), undefined)
  assert.equal(resolveDiscussionRoom([], missions, 'm2'), undefined)
  assert.equal(resolveDiscussionRoom(rooms, [], 'm2'), undefined)
  for (const room_id of [undefined, null, '']) {
    assert.equal(resolveDiscussionRoom(rooms, [{ id: 'm2', title: 'Older mission', room_id }], 'm2'), undefined)
  }
})

test('related work is limited to the selected room and mission, not a fixed item count', () => {
  const roomContext = roomWorkContext(snapshot, 'room-b', null)
  assert.deepEqual(roomContext.missions.map(({ id }) => id), ['m2', 'm3'])
  assert.deepEqual(relatedWorkOptions(roomContext).map(({ value }) => value), [
    'mission:m2', 'mission:m3', 'task:t2', 'task:t3', 'run:r2', 'artifact:a2', 'run:r3', 'artifact:a3',
  ])
  const missionContext = roomWorkContext(snapshot, 'room-b', 'm2')
  assert.deepEqual(relatedWorkOptions(missionContext).map(({ value }) => value),
    ['mission:m2', 'task:t2', 'run:r2', 'artifact:a2'])
  for (const [roomId, missionId] of [[null, null], ['room-a', 'm2'], ['room-b', 'missing']]) {
    assert.deepEqual(relatedWorkOptions(roomWorkContext(snapshot, roomId, missionId)), [])
  }
})

test('messages are room-filtered before mission links and out-of-order reply ancestry', () => {
  const context = roomWorkContext(snapshot, 'room-b', 'm2')
  assert.deepEqual(roomDiscussionMessages(messages, 'room-b', 'm2', context).map(({ id }) => id),
    ['nested-b', 'reply-b', 'root-b', 'task-b', 'run-b', 'artifact-b'])
  const general = roomDiscussionMessages(messages, 'room-b', null, roomWorkContext(snapshot, 'room-b', null))
  assert.ok(general.some(({ id }) => id === 'general-b'))
  assert.ok(general.every(({ room_id }) => room_id === 'room-b'))
  assert.deepEqual(roomDiscussionMessages(messages, null, 'm2', context), [])
  assert.deepEqual(roomDiscussionMessages(messages, 'room-b', 'missing', context), [])
})

test('valid room, mission and inherited reply submissions retain the same scope', () => {
  for (const [kind, id] of [['mission', 'm2'], ['task', 't2'], ['run', 'r2'], ['artifact', 'a2']]) {
    assert.equal(canPostRoomMessage(snapshot, scope, scope, { ...input, link: { kind, id } }), true)
  }
  assert.equal(canPostRoomMessage(snapshot, scope, scope,
    { ...input, link: null, replyToId: 'nested-b' }), true)
  const generalScope = { ...scope, missionId: null }
  assert.equal(canPostRoomMessage(snapshot, generalScope, generalScope, { ...input, link: null }), true)
  assert.equal(canPostRoomMessage({ ...snapshot, missions: [...missions] }, scope, scope, input), true,
    'a same-view snapshot refresh does not invalidate the draft')
})

test('stale actor, Corp, room and mission scopes cannot submit or complete a newer draft', () => {
  for (const change of [
    { actorId: 'bob' }, { corpId: 'other-corp' }, { roomId: 'room-a' },
    { missionId: 'm3' }, { missionId: null },
  ]) {
    const current = { ...scope, ...change }
    assert.notEqual(discussionScopeKey(current), discussionScopeKey(scope))
    assert.equal(canPostRoomMessage(snapshot, current, scope, input), false)
  }
  assert.equal(canPostRoomMessage(snapshot, { ...scope }, scope, input), false,
    'leaving and reopening the same IDs cannot revive an old asynchronous submission')
  assert.equal(canPostRoomMessage({ ...snapshot, corp: { id: 'other-corp' } }, scope, scope, input), false)
})

test('a disappearing mission or owning room fences previously valid submissions', () => {
  assert.equal(canPostRoomMessage({ ...snapshot, missions: missions.filter(({ id }) => id !== 'm2') },
    scope, scope, input), false)
  assert.equal(canPostRoomMessage({ ...snapshot, rooms: rooms.slice(0, 1) }, scope, scope, input), false)
  assert.equal(canPostRoomMessage({ ...snapshot, missions: missions.map((mission) =>
    mission.id === 'm2' ? { ...mission, room_id: 'room-a' } : mission) }, scope, scope, input), false)
  assert.equal(canPostRoomMessage(snapshot, scope, scope, { ...input, roomId: 'room-a' }), false)
})

test('foreign, missing or stale links and replies cannot silently become general comments', () => {
  for (const [kind, id] of [['mission', 'm1'], ['mission', 'm3'], ['task', 't1'], ['run', 'r3'], ['artifact', 'absent']]) {
    assert.equal(canPostRoomMessage(snapshot, scope, scope, { ...input, link: { kind, id } }), false)
  }
  for (const replyToId of ['root-a', 'other-b', 'wrong-room-reply', 'missing-ancestor', 'gone']) {
    assert.equal(canPostRoomMessage(snapshot, scope, scope, { ...input, replyToId }), false)
  }
  assert.equal(canPostRoomMessage(snapshot, scope, scope, { ...input, link: null }), false)
})

test('guest-shaped and missing Factory projections do not establish Direct origin', () => {
  for (const projection of [{}, { factory_work_items: [] }, { factory_work_items: null }]) {
    const origin = missionOrigin('m2', projection)
    assert.equal(origin.kind, 'unknown')
    assert.equal(origin.label, 'Mission origin unavailable')
    assert.doesNotMatch(`${origin.label} ${origin.detail}`, /direct|started here|without GitHub/i)
  }
})

test('a mission older than the 500-item Factory window remains origin-unknown', () => {
  const factory_work_items = Array.from({ length: 500 }, (_, index) => ({
    mission_id: `newer-${index}`, source_issue_number: index + 1,
  }))
  assert.equal(missionOrigin('m2', { factory_work_items }).kind, 'unknown')
})

test('a positively matched visible Factory item retains its issue attribution', () => {
  const origin = missionOrigin('m2', {
    factory_work_items: [
      { mission_id: 'm1', source_issue_number: 184 },
      { mission_id: 'm2', source_issue_number: 185 },
    ],
  })
  assert.equal(origin.kind, 'factory')
  assert.equal(origin.label, 'From GitHub issue #185')
  assert.doesNotMatch(origin.label, /184/)
})

test('known publication/recovery linkage is generic when intake details are unavailable', () => {
  for (const projection of [
    { pull_request_publications: [{ mission_id: 'm2', factory_work_item_id: 'hidden-item-id' }] },
    { factory_verification_recoveries: [{ mission_id: 'm2', factory_work_item_id: 'hidden-item-id' }] },
    { factory_work_items: [{ mission_id: 'm2' }] },
    { factory_work_items: [{ mission_id: 'm2', source_issue_number: -1 }] },
  ]) {
    const origin = missionOrigin('m2', projection)
    assert.equal(origin.kind, 'factory')
    assert.equal(origin.label, 'Factory-linked mission')
    assert.doesNotMatch(JSON.stringify(origin), /hidden-item-id|issue #/)
    assert.equal(missionOrigin('m3', projection).kind, 'unknown')
  }
})

test('projection helpers preserve the authoritative input and do not retain hidden origin data', () => {
  const frozen = Object.freeze({ ...snapshot,
    missions: Object.freeze(missions.map((mission) => Object.freeze({ ...mission }))),
    tasks: Object.freeze([...tasks]), runs: Object.freeze([...runs]),
  })
  const context = roomWorkContext(frozen, 'room-b', 'm2')
  assert.equal(context.missions[0], frozen.missions[1])
  assert.equal(roomDiscussionMessages(messages, 'room-b', 'm2', context)[0], messages[0])
  missionOrigin('m2', { factory_work_items: [{ mission_id: 'm2', source_issue_number: 999 }] })
  assert.equal(missionOrigin('m2', {}).kind, 'unknown', 'no cross-view origin cache')
})

test('the UI uses the tested projection and submission helpers on both discussion entry paths', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8')
  const panel = app.slice(app.indexOf('function RoomPanel('), app.indexOf('function MissionAllocationPreview('))
  assert.match(panel, /roomWorkContext\(\{ missions, tasks, runs \}, room\?\.id, contextMissionId\)/)
  assert.match(panel, /roomDiscussionMessages\(messages, room\.id, contextMissionId, context\)/)
  assert.match(panel, /scopedMessages\.find\(\(message\) => message\.id === replyToId\)/)
  assert.match(panel, /missions\.map\(\(mission\) =>/)
  assert.match(app, /resolveDiscussionRoom\(data\.snapshot\.rooms, data\.snapshot\.missions, roomMissionId, selectedRoomId\)/)
  assert.match(app, /onDiscuss=\{\(mission\) => \{\s+selectDiscussionMission\(mission\.id\)/)
  assert.match(app, /onDiscussMission=\{\(mission\) => \{\s+selectDiscussionMission\(mission\.id\)/)
  assert.match(app, /onContextChange=\{selectDiscussionMission\}/)
  assert.match(app, /key=\{discussionScopeKey\(roomScope\)\}/)
  assert.match(app, /canPostRoomMessage\(current\.snapshot, current\[input\.source\], input\.scope, input\)/)
  assert.match(app, /if \(!isCurrent\(\)\) return false\s+await refresh\(input\.scope\.corpId, input\.scope\.actorId\)/)
  assert.match(app, /snapshotLoad\.actorId === selectedActorId \? snapshotLoad\.response : null/)
  assert.match(app, /origin=\{missionOrigin\(selectedMission\.id, data\.snapshot\)\}/)
  assert.doesNotMatch(app, /const room = data\.snapshot\.rooms\[0\]|'Direct mission'|Started here, without GitHub intake|Your manually started work/)
})
