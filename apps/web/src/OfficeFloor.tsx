import { useEffect, useMemo, useRef, useState } from 'react'
import type { CSSProperties, RefObject } from 'react'
import {
  ACTIVE_OFFICE_STATES as ACTIVE_STATES, currentOfficeAgents,
  getArrivalRoute, getDesk, OFFICE_HEIGHT, OFFICE_WIDTH, officeNextAction,
  paginateOfficeAgents, resolveOfficeState, resolveOfficeView, shouldAnimateArrival,
  stateDescription, stateLabel,
} from './office/officeModel'
import type { OfficeAgent, OfficePoint, OfficeState, OfficeView } from './office/officeModel'
import {
  getOfficeCharacterAsset, getOfficeCharacterFrame, OFFICE_CHARACTER_ASSETS,
} from './office/characterAssets'
import type { OfficeCharacterAnimation, OfficeCharacterDirection } from './office/characterAssets'
import './OfficeFloor.css'

type Person = { agent: OfficeAgent; state: OfficeState; seat: number }
type Pose = {
  point: OfficePoint
  runId: string | null
  route: readonly OfficePoint[] | null
  beganAt: number
  direction: OfficeCharacterDirection
}

const EMPTY_RUN_IDS: ReadonlySet<string> = new Set()

function characterFor(agentId: string) {
  // Preserve six distinct appearances for the existing development crew.
  const demo = /^00000000-0000-4000-8000-00000000003([1-6])$/.exec(agentId)
  return getOfficeCharacterAsset(demo ? Number(demo[1]) - 1 : agentId)
}

function providerLabel(adapter: string) {
  const labels: Record<string, string> = {
    'github-copilot': 'GitHub Copilot',
    'claude-code': 'Claude Code',
    codex: 'OpenAI Codex',
    opencode: 'OpenCode',
    'fake-process': 'Test harness',
  }
  return labels[adapter] ?? adapter
}

function useMotionPreference() {
  const [reduced, setReduced] = useState(() =>
    window.matchMedia('(prefers-reduced-motion: reduce)').matches,
  )
  const [paused, setPaused] = useState(() => {
    try { return window.localStorage.getItem('ecorp.office.motion') === 'paused' }
    catch { return false }
  })
  useEffect(() => {
    const query = window.matchMedia('(prefers-reduced-motion: reduce)')
    const update = () => setReduced(query.matches)
    query.addEventListener('change', update)
    return () => query.removeEventListener('change', update)
  }, [])
  const toggle = () => {
    const next = !paused
    setPaused(next)
    try { window.localStorage.setItem('ecorp.office.motion', next ? 'paused' : 'on') }
    catch { /* Motion preference still works without browser storage. */ }
  }
  return { reduced, paused, toggle, enabled: !reduced && !paused }
}

function routePoint(route: readonly OfficePoint[], progress: number) {
  const lengths = route.slice(1).map((point, i) =>
    Math.abs(point.x - route[i].x) + Math.abs(point.y - route[i].y),
  )
  let distance = Math.max(0, Math.min(1, progress)) * lengths.reduce((a, b) => a + b, 0)
  for (let i = 0; i < lengths.length; i++) {
    if (distance <= lengths[i] || i === lengths.length - 1) {
      const ratio = lengths[i] ? distance / lengths[i] : 1
      const from = route[i]
      const to = route[i + 1]
      const direction: OfficeCharacterDirection = to.x < from.x
        ? 'left' : to.x > from.x ? 'right' : to.y < from.y ? 'up' : 'down'
      return {
        point: { x: Math.round(from.x + (to.x - from.x) * ratio), y: Math.round(from.y + (to.y - from.y) * ratio) },
        direction,
      }
    }
    distance -= lengths[i]
  }
  return { point: route[route.length - 1], direction: 'up' as const }
}

function OfficeCanvas({
  people, motion, selectedId, buttons,
}: {
  people: Person[]
  motion: boolean
  selectedId: string | null
  buttons: RefObject<Map<string, HTMLButtonElement>>
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const poses = useRef(new Map<string, Pose>())
  const images = useRef(new Map<string, HTMLImageElement>())
  const [artState, setArtState] = useState<'loading' | 'ready' | 'fallback'>('loading')
  const [visible, setVisible] = useState(!document.hidden)

  useEffect(() => {
    const update = () => setVisible(!document.hidden)
    document.addEventListener('visibilitychange', update)
    return () => document.removeEventListener('visibilitychange', update)
  }, [])

  useEffect(() => {
    let current = true
    Promise.all(OFFICE_CHARACTER_ASSETS.map((asset) => new Promise<boolean>((resolve) => {
      const image = new Image()
      image.onload = () => {
        images.current.set(asset.src, image)
        resolve(true)
      }
      image.onerror = () => resolve(false)
      image.src = asset.src
    }))).then((results) => {
      if (current) setArtState(results.every(Boolean) ? 'ready' : 'fallback')
    })
    return () => { current = false }
  }, [])

  useEffect(() => {
    const canvas = canvasRef.current
    const context = canvas?.getContext('2d')
    if (!canvas || !context) return
    const now = performance.now()
    const present = new Set(people.map((person) => person.agent.id))
    for (const id of poses.current.keys()) {
      if (!present.has(id)) poses.current.delete(id)
    }
    for (const { agent, state, seat } of people) {
      const previous = poses.current.get(agent.id)
      // Only an observed run transition causes an arrival. First paint, page
      // changes and floor remounts do not imply a new run.
      const arriving = shouldAnimateArrival(previous?.runId, agent, state, motion)
      const keepRoute = motion && previous?.route && ACTIVE_STATES.includes(state)
      poses.current.set(agent.id, {
        point: getDesk(seat),
        runId: agent.current_run_id,
        route: arriving ? getArrivalRoute(seat) : keepRoute ? previous.route : null,
        beganAt: arriving ? now : previous?.beganAt ?? now,
        direction: previous?.direction ?? 'down',
      })
    }

    let frameId = 0
    let lastPaint = -Infinity
    const paint = (time: number) => {
      if (time - lastPaint < 80) {
        frameId = requestAnimationFrame(paint)
        return
      }
      lastPaint = time
      context.clearRect(0, 0, OFFICE_WIDTH, OFFICE_HEIGHT)
      context.imageSmoothingEnabled = false
      const moving = motion && visible
      const ordered = people.map((person) => {
        const pose = poses.current.get(person.agent.id)!
        if (pose.route && moving) {
          const progress = (time - pose.beganAt) / 2400
          if (progress >= 1) {
            pose.route = null
            pose.point = getDesk(person.seat)
          } else {
            const position = routePoint(pose.route, progress)
            pose.point = position.point
            pose.direction = position.direction
          }
        } else if (!moving) {
          pose.point = getDesk(person.seat)
        }
        return { ...person, pose }
      }).sort((left, right) => left.pose.point.y - right.pose.point.y)
      for (const { agent, state, pose } of ordered) {
        const { x, y } = pose.point
        const button = buttons.current.get(agent.id)
        if (button) {
          button.style.left = `${x / OFFICE_WIDTH * 100}%`
          button.style.top = `${y / OFFICE_HEIGHT * 100}%`
          button.dataset.moving = String(Boolean(moving && pose.route))
        }
        context.fillStyle = agent.id === selectedId ? '#f9d8a8cc' : '#182d3c40'
        context.fillRect(x - 13, y - 3, 26, 5)
        const asset = characterFor(agent.id)
        const image = images.current.get(asset.src)
        const working = state === 'working'
        const reading = state === 'reading' || state === 'reviewing'
        const animation: OfficeCharacterAnimation = pose.route && moving
          ? 'walk' : working ? 'type' : reading ? 'read' : 'idle'
        const direction = pose.route && moving ? pose.direction
          : working ? 'up' : reading ? 'right' : 'down'
        const frame = getOfficeCharacterFrame(animation, direction, moving ? time : 0)
        if (button) {
          button.dataset.pose = animation
          button.dataset.frame = String(frame.column)
          button.dataset.facing = direction
        }
        if (image) {
          context.save()
          context.globalAlpha = state === 'offline' ? 0.58 : 1
          context.translate(x, y - (working || reading ? 6 : 0))
          if (frame.flipX) context.scale(-1, 1)
          context.drawImage(image, frame.sx, frame.sy, frame.sw, frame.sh, -12, -48, 24, 48)
          context.restore()
        } else {
          // The accessible crew controls remain usable if a local asset fails.
          context.fillStyle = '#d9c99f'
          context.fillRect(x - 5, y - 29, 10, 11)
          context.fillStyle = '#475f68'
          context.fillRect(x - 7, y - 18, 14, 17)
        }
      }
      if (moving && people.some(({ agent, state }) =>
        ACTIVE_STATES.includes(state) || poses.current.get(agent.id)?.route,
      )) frameId = requestAnimationFrame(paint)
    }
    paint(now)
    return () => cancelAnimationFrame(frameId)
  }, [people, motion, visible, selectedId, artState, buttons])

  return (
    <canvas
      ref={canvasRef}
      width={OFFICE_WIDTH}
      height={OFFICE_HEIGHT}
      className="pixel-office-canvas"
      aria-hidden="true"
      data-art-status={artState}
      data-motion={motion && visible ? 'enabled' : 'paused'}
    />
  )
}

export function OfficePortrait({ agentId }: { agentId: string }) {
  return (
    <span className="pixel-office-portrait" aria-hidden="true">
      <span style={{ backgroundImage: `url("${characterFor(agentId).src}")` }} />
    </span>
  )
}

export function OfficeFloor({
  agents,
  selectedAgentId,
  pendingApprovalRunIds = EMPTY_RUN_IDS,
  pendingReviewRunIds = EMPTY_RUN_IDS,
  connection,
  runnerCount,
  onSelect,
  onMissions,
  onFactory,
}: {
  agents: readonly OfficeAgent[]
  selectedAgentId: string | null
  pendingApprovalRunIds?: ReadonlySet<string>
  pendingReviewRunIds?: ReadonlySet<string>
  connection: string
  runnerCount: number
  onSelect: (agentId: string) => void
  onMissions: (agentId?: string) => void
  onFactory: () => void
}) {
  const currentAgents = useMemo(() => currentOfficeAgents(agents), [agents])
  const pages = useMemo(() => paginateOfficeAgents(currentAgents), [currentAgents])
  const [requestedView, setRequestedView] = useState<OfficeView | null>(null)
  const view = resolveOfficeView(pages, selectedAgentId, requestedView)
  const { selectedPage, page: pageIndex, zoom } = view
  // Remember each selection transition before paint, including A -> B -> A.
  // Otherwise an old manual page request for A could hide it on the return.
  if (requestedView?.selectedId !== view.selectedId || requestedView?.selectedPage !== view.selectedPage) {
    setRequestedView(view)
  }
  const [backgroundUnavailable, setBackgroundUnavailable] = useState(false)
  const viewportRef = useRef<HTMLDivElement>(null)
  const buttons = useRef(new Map<string, HTMLButtonElement>())
  const preference = useMotionPreference()
  useEffect(() => {
    viewportRef.current?.scrollTo(0, 0)
  }, [selectedAgentId, selectedPage])
  const people = useMemo(() => (pages[pageIndex]?.agents ?? []).map((agent, seat) => ({
    agent, seat, state: resolveOfficeState(agent, pendingApprovalRunIds, pendingReviewRunIds),
  })), [pages, pageIndex, pendingApprovalRunIds, pendingReviewRunIds])
  const activeCount = currentAgents.filter((agent) =>
    ACTIVE_STATES.includes(resolveOfficeState(agent, pendingApprovalRunIds, pendingReviewRunIds)),
  ).length
  const decisionCount = currentAgents.filter((agent) =>
    resolveOfficeState(agent, pendingApprovalRunIds, pendingReviewRunIds) === 'approval',
  ).length
  const blockedCount = currentAgents.filter((agent) =>
    resolveOfficeState(agent, pendingApprovalRunIds, pendingReviewRunIds) === 'blocked',
  ).length
  const nextAction = officeNextAction(decisionCount, blockedCount, activeCount)
  const attentionAgent = nextAction.attentionState
    ? currentAgents.find((agent) =>
        resolveOfficeState(agent, pendingApprovalRunIds, pendingReviewRunIds) === nextAction.attentionState,
      )
    : undefined

  const changePage = (page: number) => {
    setRequestedView({ ...view, page, zoom: 1 })
    viewportRef.current?.scrollTo(0, 0)
  }
  const fit = () => {
    setRequestedView({ ...view, zoom: 1 })
    viewportRef.current?.scrollTo(0, 0)
  }

  return (
    <div className="pixel-office" data-testid="pixel-office">
      <div className="pixel-office-main">
        <div className="pixel-office-toolbar">
          <div className="pixel-office-studio">
            <span className="pixel-office-door-mark" aria-hidden="true">E/</span>
            <strong>{pages.length > 1 ? `Studio ${pageIndex + 1}` : 'The office'}</strong>
            <span>{decisionCount ? `${decisionCount} awaiting approval` : blockedCount ? `${blockedCount} blocked` : activeCount ? `${activeCount} active` : 'No active runs'}</span>
          </div>
          <div className="pixel-office-camera" aria-label="Office view controls">
            <button type="button" onClick={fit} aria-label="Fit office to view">Fit</button>
            <button type="button" disabled={zoom <= 1} onClick={() => setRequestedView({ ...view, zoom: Math.max(1, zoom - 0.25) })} aria-label="Zoom out">−</button>
            <output aria-label="Office zoom">{Math.round(zoom * 100)}%</output>
            <button type="button" disabled={zoom >= 2} onClick={() => setRequestedView({ ...view, zoom: Math.min(2, zoom + 0.25) })} aria-label="Zoom in">+</button>
          </div>
        </div>
        <div className={`pixel-office-viewport${zoom > 1 ? ' pixel-office-viewport-zoomed' : ''}`} ref={viewportRef} tabIndex={0} aria-label="Office floor. Select an agent or use the crew list. Zoomed views can be scrolled.">
          <div className="pixel-office-world" style={{ '--office-zoom': zoom } as CSSProperties}>
            <img
              className="pixel-office-backdrop"
              src="/assets/office/ecorp-studio-gemini.jpg"
              width={2912}
              height={1440}
              alt=""
              aria-hidden="true"
              decoding="async"
              data-state="loading"
              onLoad={(event) => {
                event.currentTarget.dataset.state = 'ready'
                setBackgroundUnavailable(false)
              }}
              onError={(event) => {
                event.currentTarget.dataset.state = 'unavailable'
                setBackgroundUnavailable(true)
              }}
            />
            {backgroundUnavailable ? (
              <p className="pixel-office-background-error" role="status">
                Office artwork could not load. The live crew list and agent controls are still available.
              </p>
            ) : null}
            <OfficeCanvas people={people} motion={preference.enabled} selectedId={selectedAgentId} buttons={buttons} />
            {people.map(({ agent, state, seat }) => {
              const desk = getDesk(seat)
              return (
                <button
                  key={agent.id}
                  type="button"
                  ref={(element) => {
                    if (element) buttons.current.set(agent.id, element)
                    else buttons.current.delete(agent.id)
                  }}
                  className={`pixel-office-agent pixel-office-state-${state}`}
                  style={{ left: `${desk.x / OFFICE_WIDTH * 100}%`, top: `${desk.y / OFFICE_HEIGHT * 100}%` }}
                  onClick={() => onSelect(agent.id)}
                  aria-label={`Inspect ${agent.name}, ${stateLabel(state)}`}
                  aria-pressed={selectedAgentId === agent.id}
                  data-testid={`agent-${agent.name}`}
                  data-state={state}
                  title={`${agent.name} — ${stateLabel(state)}. ${stateDescription(state)}`}
                >
                  {state === 'approval' || state === 'blocked' ? (
                    <span className="pixel-office-alert" aria-hidden="true">{state === 'approval' ? '?' : '!'}</span>
                  ) : null}
                  <span className="pixel-office-nameplate" aria-hidden="true">
                    <strong>{agent.name}</strong>
                    <small><i />{state === 'approval' ? 'Approval' : stateLabel(state)}</small>
                  </span>
                </button>
              )
            })}
            {!currentAgents.length ? (
              <div className="pixel-office-empty">
                <strong>Your office is ready.</strong>
                <span>Create a mission to staff the office.</span>
                <button type="button" onClick={() => onMissions()}>Set up a mission</button>
              </div>
            ) : null}
          </div>
        </div>
        <div className="pixel-office-bottom">
          <div className="pixel-office-legend" aria-label="Agent state legend">
            <span className="pixel-office-state-idle"><i />Idle</span>
            <span className="pixel-office-state-working"><i />Working</span>
            <span className="pixel-office-state-reviewing"><i />Review</span>
            <span className="pixel-office-state-blocked"><i />Blocked</span>
            <span className="pixel-office-state-approval"><i />Approval</span>
          </div>
          <button
            className="pixel-office-motion"
            type="button"
            onClick={preference.toggle}
            aria-pressed={preference.paused || preference.reduced}
            disabled={preference.reduced}
            title={preference.reduced ? 'Motion is off because your system requests reduced motion.' : 'Pauses visual animation only. Agents keep running.'}
          >
            {preference.reduced ? 'Reduced motion' : preference.paused ? 'Resume motion' : 'Pause motion'}
          </button>
        </div>
      </div>

      <aside className="pixel-office-roster" aria-label="Available crew">
        <header>
          <div><h3>Crew</h3><span>{currentAgents.length} {currentAgents.length === 1 ? 'agent' : 'agents'}</span></div>
          <p>Select anyone to inspect their work.</p>
        </header>
        <div className="pixel-office-roster-list">
          {people.map(({ agent, state }) => (
            <button
              type="button"
              key={agent.id}
              className={`pixel-office-crew pixel-office-state-${state}`}
              onClick={() => onSelect(agent.id)}
              aria-label={`Inspect ${agent.name}, ${stateLabel(state)}, ${providerLabel(agent.adapter)}`}
              aria-pressed={selectedAgentId === agent.id}
              data-state={state}
            >
              <OfficePortrait agentId={agent.id} />
              <span className="pixel-office-crew-copy">
                <strong>{agent.name}</strong>
                <small>{providerLabel(agent.adapter)}</small>
              </span>
              <span className="pixel-office-crew-status"><i />{stateLabel(state)}</span>
            </button>
          ))}
        </div>
        {pages.length > 1 ? (
          <nav className="pixel-office-pagination" aria-label="Office studios">
            <button type="button" disabled={pageIndex === 0} onClick={() => changePage(pageIndex - 1)}>Previous</button>
            <span>{pageIndex + 1} / {pages.length}</span>
            <button type="button" disabled={pageIndex === pages.length - 1} onClick={() => changePage(pageIndex + 1)}>Next</button>
          </nav>
        ) : null}
        <div className="pixel-office-next">
          <strong>{nextAction.heading}</strong>
          <p>{nextAction.description}</p>
          <button type="button" className="pixel-office-primary" onClick={() => onMissions(attentionAgent?.id)}>{nextAction.label}<span aria-hidden="true">↗</span></button>
          <button type="button" className="pixel-office-factory-link" onClick={onFactory}>Open Factory <span aria-hidden="true">→</span></button>
        </div>
      </aside>

      <footer className="pixel-office-statusline">
        <span><i className={connection === 'live' ? 'pixel-office-connected' : ''} />{connection === 'live' ? 'Live state' : `Connection: ${connection}`}<b>·</b>{runnerCount ? `${runnerCount} runner${runnerCount === 1 ? '' : 's'} connected` : 'No runner connected'}</span>
        <span>The floor reflects real agent activity. No simulated work.</span>
      </footer>
    </div>
  )
}
