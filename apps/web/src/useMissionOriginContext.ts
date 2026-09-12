import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  currentMissionOrigin, missionOriginScope, startMissionOriginRead,
} from './missionOriginContext'
import type { MissionOriginApi, MissionOriginLoad } from './missionOriginContext'

export type MissionOriginRequest = {
  corpId: string | null | undefined
  actorId: string | null | undefined
  missionId: string | null | undefined
  roomId: string | null | undefined
  actorRole: string
  api: MissionOriginApi
}

/** One exact origin read shared by the work heading and its result. */
export function useMissionOriginContext({
  corpId, actorId, missionId, roomId, actorRole, api,
}: MissionOriginRequest) {
  const [reload, setReload] = useState(0)
  const request = useMemo(() => {
    // Presentation guard only; current actor/room authorization stays server-side.
    if (!['owner', 'admin', 'manager', 'member'].includes(actorRole)) return null
    try {
      return { scope: missionOriginScope(corpId, actorId, missionId, roomId), api, reload }
    } catch {
      return null
    }
  }, [corpId, actorId, missionId, roomId, actorRole, api, reload])
  const [load, setLoad] = useState<MissionOriginLoad | null>(null)
  useEffect(() => {
    if (!request) return
    return startMissionOriginRead(request.scope, request.api, setLoad)
  }, [request])
  const current = currentMissionOrigin(request?.scope ?? null, load)
  const pending = Boolean(request && (!current || current.status === 'pending'))
  const refresh = useCallback(() => setReload((value) => value + 1), [])
  return { current, pending, refresh }
}
