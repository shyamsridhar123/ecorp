import { useEffect, useRef } from 'react'
import type { ReactNode } from 'react'

export function OfficeInspector({
  agentName, onClose, children,
}: {
  agentName: string
  onClose: () => void
  children: ReactNode
}) {
  const close = useRef<HTMLButtonElement>(null)
  useEffect(() => {
    const previous = document.activeElement
    const overflow = document.body.style.overflow
    document.body.style.overflow = 'hidden'
    close.current?.focus()
    return () => {
      document.body.style.overflow = overflow
      if (previous instanceof HTMLElement && previous.isConnected) previous.focus({ preventScroll: true })
    }
  }, [])

  return (
    <div className="world-inspector-layer">
      <button type="button" className="world-inspector-scrim" aria-label="Close agent inspector" tabIndex={-1} onClick={onClose} />
      <aside
        className="world-inspector"
        role="dialog"
        aria-modal="true"
        aria-label={`${agentName} details and controls`}
        onKeyDown={(event) => {
          if (event.key === 'Escape') {
            event.preventDefault()
            event.stopPropagation()
            onClose()
          } else if (event.key === 'Tab') {
            const controls = Array.from(event.currentTarget.querySelectorAll<HTMLElement>(
              'button:not(:disabled), input:not(:disabled), textarea:not(:disabled), select:not(:disabled), a[href], [tabindex="0"]',
            )).filter((element) => element.getClientRects().length > 0)
            const first = controls[0]
            const last = controls[controls.length - 1]
            if (event.shiftKey && event.target === first) {
              event.preventDefault()
              last?.focus()
            } else if (!event.shiftKey && event.target === last) {
              event.preventDefault()
              first?.focus()
            }
          }
        }}
      >
        <button type="button" className="world-inspector-close" ref={close} onClick={onClose}>Close</button>
        {children}
      </aside>
    </div>
  )
}
