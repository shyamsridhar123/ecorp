type Timer = ReturnType<typeof setTimeout>

// Provider streams can emit many events per second. A client must not turn each
// event into another concurrent full-Corp database snapshot. Keep one request in
// flight and one trailing refresh so the last event is never lost.
export function createSnapshotRefresher({
  refresh,
  onError,
  delayMs = 250,
  schedule = setTimeout,
  cancel = clearTimeout,
}: {
  refresh: (signal: AbortSignal) => Promise<unknown>
  onError: (error: unknown) => void
  delayMs?: number
  schedule?: (callback: () => void, delay: number) => Timer
  cancel?: (timer: Timer) => void
}) {
  const controller = new AbortController()
  let disposed = false
  let dirty = false
  let inFlight = false
  let timer: Timer | null = null

  const arm = () => {
    if (disposed || inFlight || timer !== null || !dirty) return
    timer = schedule(() => void flush(), delayMs)
  }
  const flush = async () => {
    timer = null
    if (disposed || !dirty) return
    dirty = false
    inFlight = true
    try {
      await refresh(controller.signal)
    } catch (error) {
      if (!disposed && !controller.signal.aborted) onError(error)
    } finally {
      inFlight = false
      arm()
    }
  }

  return {
    request() {
      if (disposed) return
      dirty = true
      arm()
    },
    dispose() {
      disposed = true
      dirty = false
      if (timer !== null) cancel(timer)
      timer = null
      controller.abort()
    },
  }
}
