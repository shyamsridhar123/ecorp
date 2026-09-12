import { useEffect, useMemo, useState } from 'react'
import {
  currentMissionOrigin, missionOriginScope, startMissionOriginRead,
} from './missionOriginContext'
import type {
  MissionOriginApi, MissionOriginFallback, MissionOriginLoad,
} from './missionOriginContext'

export type MissionOriginDetailsProps = {
  corpId: string | null | undefined
  actorId: string | null | undefined
  missionId: string | null | undefined
  roomId: string | null | undefined
  actorRole: string
  api: MissionOriginApi
  fallback: MissionOriginFallback
}

/** Replaces the origin paragraph inside the existing .mission-work-context container. */
export function MissionOriginDetails({
  corpId, actorId, missionId, roomId, actorRole, api, fallback,
}: MissionOriginDetailsProps) {
  const request = useMemo(() => {
    // This is a UI guard only. The endpoint enforces human identity and current
    // mission-room membership through the existing Operate authorization.
    if (!['owner', 'admin', 'manager', 'member'].includes(actorRole)) return null
    try {
      return { scope: missionOriginScope(corpId, actorId, missionId, roomId), api }
    } catch {
      return null
    }
  }, [corpId, actorId, missionId, roomId, actorRole, api])
  const [load, setLoad] = useState<MissionOriginLoad | null>(null)
  useEffect(() => {
    if (!request) return
    return startMissionOriginRead(request.scope, request.api, setLoad)
  }, [request])

  // Object identity fences even the render before effect cleanup, and a reopened
  // view with identical IDs. Snapshot refreshes alone neither cache nor retry a read.
  const current = currentMissionOrigin(request?.scope ?? null, load)
  const origin = current?.status === 'ready' ? current.context.origin : null
  const pending = Boolean(request && (!current || current.status === 'pending'))
  const label = origin?.kind === 'direct'
    ? 'Direct mission'
    : origin?.kind === 'factory' ? `From GitHub issue #${origin.source_issue_number}` : fallback.label
  const detail = origin?.kind === 'direct'
    ? 'This mission is not linked to Factory intake.'
    : origin?.kind === 'factory'
      ? 'Factory owns issue intake; this mission owns the tasks and results.'
      : fallback.detail

  return (
    <p aria-live="polite" aria-busy={pending}>
      <strong>
        {origin?.kind === 'factory' ? (
          <a href={origin.source_issue_url} target="_blank" rel="noopener noreferrer">
            {label}
          </a>
        ) : label}
      </strong>
      {' · '}
      {detail}
      {pending ? ' Checking mission context.' : null}
      {current?.status === 'unavailable' ? ' Exact mission context could not be loaded.' : null}
    </p>
  )
}
