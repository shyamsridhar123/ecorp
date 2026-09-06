export type FactoryPolling = {
  next_retry_at?: string | null
  retry_reason?:
    | 'graphql_quota'
    | 'primary_rate_limit'
    | 'secondary_rate_limit'
    | 'github_unavailable'
    | null
  consecutive_failures?: number
  graphql?: {
    limit: number
    remaining: number
    cost: number
    reset_at: string
    observed_at: string
  } | null
}

type PollingField = { label: string; value: string; dateTime?: string }
type PollingTime = { milliseconds: number; dateTime: string; local: string }

const UNAVAILABLE = 'Unavailable'
const CONTROLLER_STATES = [
  'offline', 'watching', 'working', 'blocked', 'needs_decision', 'backing_off',
]

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null
}

function count(value: unknown): string {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0
    ? String(value)
    : UNAVAILABLE
}

function pollingTime(value: unknown): PollingTime | null {
  // Require an explicit offset: legacy numeric/date-only strings are not timestamps.
  if (typeof value !== 'string') return null
  const parts = /^(\d{4})-(\d{2})-(\d{2})T(?:[01]\d|2[0-3]):[0-5]\d:[0-5]\d(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/i.exec(value)
  if (!parts) return null
  const [, yearText, monthText, dayText] = parts
  const year = Number(yearText)
  const month = Number(monthText)
  const day = Number(dayText)
  const leap = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0)
  const days = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
  const milliseconds = Date.parse(value)
  if (!Number.isFinite(milliseconds) || day < 1 || day > (days[month - 1] ?? 0)) {
    return null
  }
  return {
    milliseconds,
    dateTime: value,
    local: new Date(milliseconds).toLocaleString(undefined, {
      year: 'numeric', month: 'short', day: 'numeric',
      hour: 'numeric', minute: '2-digit', second: '2-digit', timeZoneName: 'short',
    }),
  }
}

function timeField(label: string, value: PollingTime | null): PollingField {
  return { label, value: value?.local ?? UNAVAILABLE, dateTime: value?.dateTime }
}

export function factoryControllerState(controller: unknown): string {
  const health = record(controller)
  if (!health) return 'not_configured'
  if (health.desired_state === 'paused') return 'paused'
  return typeof health.status === 'string' && CONTROLLER_STATES.includes(health.status)
    ? health.status
    : 'unavailable'
}

/** Read-only projection. A deadline passing never proves that intake actually retried. */
export function presentFactoryPolling(controller: unknown, now = Date.now()) {
  const health = record(controller)
  if (!health) return null
  const polling = record(health.polling)
  // Older controllers keep their existing strip; a new backoff must still explain missing data.
  if (health.polling == null && health.status !== 'backing_off') return null
  const graphql = record(polling?.graphql)
  const retry = pollingTime(polling?.next_retry_at)
  const state = factoryControllerState(health)
  const paused = state === 'paused'
  const offline = health.status === 'offline'
  const reasons: Record<string, string> = {
    graphql_quota: graphql?.remaining === 0
      ? 'GitHub GraphQL quota is exhausted.'
      : 'GitHub GraphQL quota is low.',
    primary_rate_limit: 'GitHub primary API rate limit reached.',
    secondary_rate_limit: 'GitHub is temporarily slowing requests (secondary rate limit).',
    github_unavailable: 'GitHub is temporarily unavailable.',
  }
  const reason = typeof polling?.retry_reason === 'string'
    && Object.hasOwn(reasons, polling.retry_reason)
    ? reasons[polling.retry_reason]
    : UNAVAILABLE
  const waiting = health.status === 'backing_off' || reason !== UNAVAILABLE || retry !== null
  const retryState = !retry ? 'unavailable' : retry.milliseconds > now ? 'scheduled' : 'elapsed'
  let retryMessage = 'No intake backoff is currently reported.'
  if (paused) {
    retryMessage = offline
      ? 'Intake is paused and the controller is offline. Automatic retries wait for resume and reconnection.'
      : 'Intake is paused. Automatic retries wait until intake is resumed.'
  } else if (offline) {
    retryMessage = 'The controller is offline; retry timing is not live. Automatic retries resume after reconnection while intake is enabled.'
  } else if (waiting) {
    retryMessage = retryState === 'elapsed'
      ? 'The scheduled retry time has passed; waiting for the controller to report its next attempt. Intake retries resume automatically.'
      : retryState === 'scheduled'
        ? 'Intake retries resume automatically at or after the scheduled time.'
        : 'The next retry time is unavailable. Intake retries resume automatically when GitHub allows.'
  }
  return {
    heading: paused ? 'Intake paused' : offline ? 'Controller offline'
      : waiting ? 'GitHub intake waiting' : 'GitHub polling',
    reason,
    retryMessage,
    retryState,
    // A local display refresh is not a server poll or a request to bypass backoff.
    refreshAt: !paused && !offline && retryState === 'scheduled'
      ? retry?.milliseconds ?? null
      : null,
    fields: [
      timeField(paused || offline ? 'Recorded retry (local)' : 'Next retry (local)', retry),
      { label: 'GraphQL remaining / limit', value: `${count(graphql?.remaining)} / ${count(graphql?.limit)}` },
      { label: 'Last query cost', value: count(graphql?.cost) },
      timeField('GraphQL reset (local)', pollingTime(graphql?.reset_at)),
      timeField('Last observed (local)', pollingTime(graphql?.observed_at)),
      { label: 'Consecutive intake failures', value: count(polling?.consecutive_failures) },
    ] satisfies PollingField[],
    continuity: 'Local runs and reviews continue while intake waits.',
    backoffRule: 'Queue changes and Reconcile now cannot bypass GitHub backoff.',
  }
}
