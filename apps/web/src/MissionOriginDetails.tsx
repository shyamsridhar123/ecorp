import { useMissionOriginContext } from './useMissionOriginContext'
import type { MissionOriginRequest } from './useMissionOriginContext'
import type {
  MissionOriginFallback, MissionOriginLoad,
} from './missionOriginContext'

export type MissionOriginDetailsProps = MissionOriginRequest & {
  fallback: MissionOriginFallback
}

export function MissionOriginText({
  current, pending, fallback, onRefresh,
}: {
  current: MissionOriginLoad | null
  pending: boolean
  fallback: MissionOriginFallback
  onRefresh?: () => void
}) {
  const origin = current?.status === 'ready' ? current.context.origin : null
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
      {current?.status === 'unavailable' && onRefresh ? (
        <> <button type="button" className="button button-secondary context-retry" onClick={onRefresh}>Refresh work context</button></>
      ) : null}
    </p>
  )
}

/** Standalone origin paragraph; the cockpit also reuses its scoped hook. */
export function MissionOriginDetails(props: MissionOriginDetailsProps) {
  const { current, pending, refresh } = useMissionOriginContext(props)
  return MissionOriginText({ current, pending, fallback: props.fallback, onRefresh: refresh })
}
