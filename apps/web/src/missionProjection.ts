import { messagesForMission, missionIdForLink } from './workflowContext.ts'
import type { WorkContext, WorkLink } from './workflowContext.ts'

type RoomMission = WorkContext['missions'][number] & { room_id?: string | null }
type RoomWork = Omit<WorkContext, 'missions'> & { missions: readonly RoomMission[] }
type DiscussionMessage = {
  id: string
  room_id: string
  link: WorkLink | null
  reply_to_id?: string | null
  thread_root_id?: string | null
}

export type DiscussionScope = {
  corpId: string
  actorId: string
  roomId: string | null
  missionId: string | null
}

// Only an unselected room view may default to the first visible room. An explicit
// mission/room that disappeared must never silently rebind a draft to another room.
export function resolveDiscussionRoom<R extends { id: string }>(
  rooms: readonly R[],
  missions: readonly RoomMission[],
  missionId: string | null,
  roomId: string | null = null,
): R | undefined {
  if (missionId !== null) {
    const owner = missions.find((mission) => mission.id === missionId)?.room_id
    return owner ? rooms.find((room) => room.id === owner) : undefined
  }
  return roomId !== null ? rooms.find((room) => room.id === roomId) : rooms[0]
}

export function roomWorkContext(
  context: RoomWork,
  roomId: string | null | undefined,
  missionId: string | null,
): WorkContext {
  const missions = context.missions.filter((mission) =>
    Boolean(roomId) && mission.room_id === roomId &&
    (missionId === null || mission.id === missionId),
  )
  const missionIds = new Set(missions.map(({ id }) => id))
  const tasks = context.tasks.filter((task) => missionIds.has(task.mission_id))
  const taskIds = new Set(tasks.map(({ id }) => id))
  return { missions, tasks, runs: context.runs.filter((run) => taskIds.has(run.task_id)) }
}

export function roomDiscussionMessages<T extends DiscussionMessage>(
  messages: readonly T[],
  roomId: string | null | undefined,
  missionId: string | null,
  context: WorkContext,
): T[] {
  if (!roomId) return []
  const roomMessages = messages.filter((message) => message.room_id === roomId)
  if (missionId === null) return roomMessages
  if (!context.missions.some((mission) => mission.id === missionId)) return []
  // Filter the room before following thread ancestry. An explicit link to other
  // work cannot inherit the selected mission merely by replying to its thread.
  return messagesForMission(
    roomMessages.filter((message) =>
      !message.link || missionIdForLink(message.link, context) === missionId,
    ),
    missionId, context.tasks, context.runs,
  )
}

export function discussionScopeKey(scope: DiscussionScope): string {
  return JSON.stringify([scope.corpId, scope.actorId, scope.roomId, scope.missionId])
}

export function canPostRoomMessage(
  snapshot: RoomWork & {
    corp: { id: string }
    rooms: readonly { id: string }[]
    room_messages: readonly DiscussionMessage[]
  },
  current: DiscussionScope,
  submitted: DiscussionScope,
  input: { roomId: string; replyToId: string | null; link: WorkLink | null },
): boolean {
  // App retains the scope object during refreshes and replaces it on selection
  // changes. Equal IDs after leaving and reopening a context do not revive it.
  if (
    !current.corpId || !current.actorId || !current.roomId ||
    snapshot.corp.id !== current.corpId ||
    current !== submitted ||
    discussionScopeKey(current) !== discussionScopeKey(submitted) ||
    input.roomId !== current.roomId
  ) return false
  const room = resolveDiscussionRoom(
    snapshot.rooms, snapshot.missions, current.missionId, current.roomId,
  )
  if (!room || room.id !== current.roomId) return false
  const context = roomWorkContext(snapshot, room.id, current.missionId)
  if (input.link && !missionIdForLink(input.link, context)) return false
  if (input.replyToId !== null) {
    const messages = roomDiscussionMessages(
      snapshot.room_messages, room.id, current.missionId, context,
    )
    if (!messages.some((message) => message.id === input.replyToId)) return false
  }
  return current.missionId === null || input.link !== null || input.replyToId !== null
}

type FactoryOriginProjection = {
  factory_work_items?: readonly {
    mission_id: string | null
    source_issue_number?: number | null
  }[] | null
  pull_request_publications?: readonly {
    mission_id: string
    factory_work_item_id: string
  }[] | null
  factory_verification_recoveries?: readonly {
    mission_id: string
    factory_work_item_id: string
  }[] | null
}

// These are positive observations from the current viewer's projection, not an
// origin index. Guest filtering and bounded history make absence inconclusive.
export function missionOrigin(missionId: string, projection: FactoryOriginProjection) {
  const item = projection.factory_work_items?.find((entry) => entry.mission_id === missionId)
  const knownFactory = Boolean(item) ||
    Boolean(projection.pull_request_publications?.some((entry) =>
      entry.mission_id === missionId && entry.factory_work_item_id,
    )) ||
    Boolean(projection.factory_verification_recoveries?.some((entry) =>
      entry.mission_id === missionId && entry.factory_work_item_id,
    ))
  if (item && typeof item.source_issue_number === 'number' &&
    Number.isSafeInteger(item.source_issue_number) && item.source_issue_number > 0) {
    return {
      kind: 'factory' as const,
      label: `From GitHub issue #${item.source_issue_number}`,
      detail: 'Factory owns issue intake; this mission owns the tasks and results.',
    }
  }
  if (knownFactory) {
    return {
      kind: 'factory' as const,
      label: 'Factory-linked mission',
      detail: 'Visible records link this mission to Factory. Intake details are unavailable in this view.',
    }
  }
  return {
    kind: 'unknown' as const,
    label: 'Mission origin unavailable',
    detail: 'This view does not establish how this mission entered ECorp. Its tasks and verification results remain available.',
  }
}
