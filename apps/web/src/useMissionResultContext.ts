import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  currentMissionResult, missionResultScope, startMissionResultRead,
} from './missionResultContext'
import type { MissionResultLoad } from './missionResultContext'
import type { MissionOriginApi } from './missionOriginContext'

export type MissionResultRequest = {
  corpId: string | null | undefined
  actorId: string | null | undefined
  actorRole: string
  missionId: string | null | undefined
  roomId: string | null | undefined
  workItemId: string | null | undefined
  sourceRepository: string | null | undefined
  /** A positive lifecycle hint, never an index or proof that a result is absent. */
  revision?: string | number
  api: MissionOriginApi
}

export function useMissionResultContext({
  corpId, actorId, actorRole, missionId, roomId, workItemId, sourceRepository, revision, api,
}: MissionResultRequest) {
  const [reload, setReload] = useState(0)
  const [load, setLoad] = useState<MissionResultLoad | null>(null)
  const request = useMemo(() => {
    if (!['owner', 'admin', 'manager', 'member'].includes(actorRole)) return null
    if (!workItemId || !sourceRepository) return null
    try {
      return {
        scope: missionResultScope({ corpId, actorId, missionId, roomId, workItemId, sourceRepository }),
        api, revision, reload,
      }
    } catch {
      return null
    }
    // API/role/lifecycle changes and explicit refresh create a new view instance.
  }, [corpId, actorId, actorRole, missionId, roomId, workItemId, sourceRepository, revision, reload, api])
  useEffect(() => {
    if (!request) return
    return startMissionResultRead(request.scope, request.api, setLoad)
  }, [request])
  const scope = request?.scope ?? null
  const current = currentMissionResult(scope, load)
  const refresh = useCallback(() => setReload((value) => value + 1), [])
  return { scope, current, refresh }
}
