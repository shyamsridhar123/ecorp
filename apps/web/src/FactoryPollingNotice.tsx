import { useEffect, useState } from 'react'
import { presentFactoryPolling } from './factoryPolling'
import type { FactoryPolling } from './factoryPolling'

export function FactoryPollingNotice({ controller }: {
  controller: { desired_state: string; status: string; polling?: FactoryPolling | null }
}) {
  const [clockTick, setClockTick] = useState(0)
  const polling = presentFactoryPolling(controller)
  const refreshAt = polling?.refreshAt
  useEffect(() => {
    if (refreshAt == null) return
    const timeout = window.setTimeout(
      () => setClockTick((tick) => tick + 1),
      Math.min(Math.max(0, refreshAt - Date.now()), 2_147_483_647),
    )
    return () => window.clearTimeout(timeout)
  }, [refreshAt, clockTick])

  if (!polling) return null
  return (
    <section className="factory-polling-notice" aria-label="GitHub intake polling">
      <p role="status">
        <strong>{polling.heading}.</strong> {polling.retryMessage}
      </p>
      <p>Retry reason: {polling.reason}</p>
      <dl>
        {polling.fields.map((field) => (
          <div key={field.label}>
            <dt>{field.label}</dt>
            <dd>
              {field.dateTime
                ? <time dateTime={field.dateTime} title={field.dateTime}>{field.value}</time>
                : field.value}
            </dd>
          </div>
        ))}
      </dl>
      <p>{polling.continuity} {polling.backoffRule}</p>
    </section>
  )
}
