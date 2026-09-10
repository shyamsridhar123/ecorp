export type MissionOriginScope = Readonly<{
  key: string
  corpId: string
  actorId: string
  missionId: string
  roomId: string
}>

export type MissionOriginContext = {
  corp_id: string
  actor_id: string
  mission_id: string
  room_id: string
  origin:
    | { kind: 'direct' }
    | {
        kind: 'factory'
        work_item_id: string
        source_repository: string
        source_issue_number: number
        source_issue_url: string
      }
}

export type MissionOriginLoad =
  | { scope: MissionOriginScope; status: 'pending' | 'unavailable'; context: null }
  | { scope: MissionOriginScope; status: 'ready'; context: MissionOriginContext }

export type MissionOriginFallback = {
  kind: 'factory' | 'unknown'
  label: string
  detail: string
}

function isIdentifier(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0 && value.length <= 128 &&
    /^[!-~]+$/.test(value) && value !== '.' && value !== '..'
}

export function missionOriginScope(
  corpId: string | null | undefined,
  actorId: string | null | undefined,
  missionId: string | null | undefined,
  roomId: string | null | undefined,
): MissionOriginScope {
  if (!isIdentifier(corpId) || !isIdentifier(actorId) ||
    !isIdentifier(missionId) || !isIdentifier(roomId)) {
    throw new Error('Mission origin requires an explicit Corp, operator, mission and room.')
  }
  return Object.freeze({
    key: JSON.stringify([corpId, actorId, missionId, roomId]), corpId, actorId, missionId, roomId,
  })
}

export function currentMissionOrigin(scope: MissionOriginScope | null, load: MissionOriginLoad | null) {
  // Leaving and reopening the same IDs does not revive a response from an older view.
  return scope && load?.scope === scope ? load : null
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function readContext(value: unknown, scope: MissionOriginScope): MissionOriginContext {
  if (!isRecord(value)) throw new Error('Missing origin context')
  if (value.corp_id !== scope.corpId || value.actor_id !== scope.actorId ||
    value.mission_id !== scope.missionId || value.room_id !== scope.roomId ||
    !isRecord(value.origin)) {
    throw new Error('Mismatched origin context')
  }
  const identity = {
    corp_id: scope.corpId, actor_id: scope.actorId,
    mission_id: scope.missionId, room_id: scope.roomId,
  }
  const origin = value.origin
  if (origin.kind === 'direct') {
    // Exact stored non-Factory linkage is not evidence of browser/manual creation.
    // Whitelist the response; never retain hidden source fields or other metadata.
    return { ...identity, origin: { kind: 'direct' } }
  }
  if (origin.kind !== 'factory' || !isIdentifier(origin.work_item_id) ||
    typeof origin.source_repository !== 'string' || origin.source_repository.length > 240 ||
    !/^[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?\/[A-Za-z0-9_.-]+$/.test(origin.source_repository) ||
    origin.source_repository.endsWith('/.') || origin.source_repository.endsWith('/..') ||
    typeof origin.source_issue_number !== 'number' ||
    !Number.isSafeInteger(origin.source_issue_number) || origin.source_issue_number <= 0) {
    throw new Error('Invalid Factory origin context')
  }
  // Match raw syntax first; only owner/repository display case may differ. Never
  // normalize away credentials, dot segments, escapes, whitespace, queries or fragments.
  const url = typeof origin.source_issue_url === 'string'
    ? /^https:\/\/github\.com\/([A-Za-z0-9-]+\/[A-Za-z0-9_.-]+)\/issues\/([1-9][0-9]*)$/.exec(origin.source_issue_url)
    : null
  if (!url || url[0] !== origin.source_issue_url ||
    url[1].toLowerCase() !== origin.source_repository.toLowerCase() ||
    url[2] !== String(origin.source_issue_number)) {
    throw new Error('Mismatched source issue link')
  }
  return {
    ...identity,
    origin: {
      kind: 'factory', work_item_id: origin.work_item_id,
      source_repository: origin.source_repository,
      source_issue_number: origin.source_issue_number,
      source_issue_url: url[0],
    },
  }
}

export type MissionOriginApi = (path: string, init: RequestInit) => Promise<unknown>
type Timer = ReturnType<typeof globalThis.setTimeout>
type OriginTimers = {
  setTimeout: (callback: () => void, delay: number) => Timer
  clearTimeout: (timer: Timer) => void
}
const nativeTimers: OriginTimers = {
  setTimeout: (callback, delay) => globalThis.setTimeout(callback, delay),
  clearTimeout: (timer) => globalThis.clearTimeout(timer),
}

/** A scoped, read-only lookup. Missing context never becomes a guessed Direct origin. */
export function startMissionOriginRead(
  scope: MissionOriginScope,
  get: MissionOriginApi,
  publish: (load: MissionOriginLoad) => void,
  timers: OriginTimers = nativeTimers,
) {
  const controller = new AbortController()
  let active = true
  const finish = (load: MissionOriginLoad) => {
    if (!active) return
    active = false
    timers.clearTimeout(timeout)
    publish(load)
  }
  publish({ scope, status: 'pending', context: null })
  const timeout = timers.setTimeout(() => {
    if (!active) return
    finish({ scope, status: 'unavailable', context: null })
    controller.abort()
  }, 15_000)
  void Promise.resolve().then(() => {
    if (!active) return undefined
    return get(
      `/api/corps/${encodeURIComponent(scope.corpId)}/missions/${encodeURIComponent(scope.missionId)}/context?actor_id=${encodeURIComponent(scope.actorId)}`,
      { method: 'GET', cache: 'no-store', signal: controller.signal },
    )
  }).then((value) => {
    if (!active) return
    finish({ scope, status: 'ready', context: readContext(value, scope) })
  }).catch(() => {
    // Denial, old servers and malformed responses all remain inconclusive. Never
    // publish raw server errors, which may contain source metadata or credentials.
    finish({ scope, status: 'unavailable', context: null })
  })
  return () => {
    active = false
    timers.clearTimeout(timeout)
    controller.abort()
  }
}
