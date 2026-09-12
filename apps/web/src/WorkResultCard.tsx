import type { ReactNode } from 'react'
import './WorkResultCard.css'

export type WorkResultCardProps = {
  heading: string
  status: string
  tone?: 'neutral' | 'working' | 'attention' | 'success'
  description: string
  pending?: boolean
  actions?: ReactNode
  facts?: { label: string; value: ReactNode }[]
  children?: ReactNode
  details?: ReactNode
}

/** Presentation only: callers retain the exact scoped data and native actions. */
export function WorkResultCard({
  heading, status, tone = 'neutral', description, pending = false,
  actions, facts, children, details,
}: WorkResultCardProps) {
  return (
    <section
      className={`work-result-card work-result-${tone}`}
      aria-label="Result and next step"
      aria-busy={pending}
      data-testid="work-result-card"
    >
      <div className="work-result-heading">
        <div>
          <span className="work-result-eyebrow">Result / next step</span>
          <h4>{heading}</h4>
        </div>
        <span className="work-result-status">{status}</span>
      </div>
      <p className="work-result-description" aria-live="polite">{description}</p>
      {actions ? <div className="work-result-actions">{actions}</div> : null}
      {facts?.length ? (
        <dl className="work-result-facts">
          {facts.map((fact) => (
            <div key={fact.label}>
              <dt>{fact.label}</dt>
              <dd>{fact.value}</dd>
            </div>
          ))}
        </dl>
      ) : null}
      {children}
      {details ? (
        <details className="work-result-details">
          <summary>Delivery details</summary>
          <div>{details}</div>
        </details>
      ) : null}
    </section>
  )
}
