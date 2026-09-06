export interface OfficeAgent {
  id: string
  name: string
  role: string
  adapter: string
  status: 'idle' | 'starting' | 'working' | 'blocked' | 'reviewing' | 'offline'
  station: string | null
  current_run_id: string | null
}

export type OfficeState =
  | 'idle'
  | 'starting'
  | 'working'
  | 'reading'
  | 'reviewing'
  | 'approval'
  | 'blocked'
  | 'offline'

export interface OfficePoint {
  x: number
  y: number
}

export interface OfficeRect extends OfficePoint {
  width: number
  height: number
}

export interface OfficePage {
  /** Zero-based studio index; desk indexes restart within each studio. */
  index: number
  agents: OfficeAgent[]
}

export const OFFICE_WIDTH = 640
export const OFFICE_HEIGHT = 320
export const DESKS_PER_STUDIO = 6

const DESKS: readonly OfficePoint[] = [
  { x: 129, y: 162 },
  { x: 225, y: 162 },
  { x: 331, y: 162 },
  { x: 104, y: 270 },
  { x: 209, y: 270 },
  { x: 319, y: 270 },
]

const STATE_LABELS: Readonly<Record<OfficeState, string>> = {
  idle: 'Idle',
  starting: 'Starting',
  working: 'Working',
  reading: 'Reading',
  reviewing: 'Reviewing',
  approval: 'Awaiting approval',
  blocked: 'Blocked',
  offline: 'Offline',
}

const STATE_DESCRIPTIONS: Readonly<Record<OfficeState, string>> = {
  idle: 'The agent reports no active work.',
  starting: 'The agent reports that a run is starting.',
  working: 'The agent reports active work.',
  reading: 'The agent reports a read or search operation.',
  reviewing: 'The agent reports review activity or its current run has a pending review.',
  approval: 'The current run has a pending approval request.',
  blocked: 'The agent is blocked without a matching pending approval or review.',
  offline: 'The agent is reported offline.',
}

/**
 * Project reported state only. Offline wins; otherwise exact current-run
 * approval takes precedence over review. Stale stations never animate an idle,
 * starting, blocked or offline agent, and blocked alone never implies approval.
 */
export function resolveOfficeState(
  agent: OfficeAgent,
  pendingApprovalRunIds: ReadonlySet<string> = new Set<string>(),
  pendingReviewRunIds: ReadonlySet<string> = new Set<string>(),
): OfficeState {
  if (agent.status === 'offline') return 'offline'

  const runId = agent.current_run_id
  if (runId && pendingApprovalRunIds.has(runId)) return 'approval'
  if (runId && pendingReviewRunIds.has(runId)) return 'reviewing'

  if (agent.status !== 'working') return agent.status

  const station = agent.station?.trim().toLowerCase()
  return station === 'read' || station === 'search' ? 'reading' : 'working'
}

export function stateLabel(state: OfficeState): string {
  return STATE_LABELS[state]
}

export function stateDescription(state: OfficeState): string {
  return STATE_DESCRIPTIONS[state]
}

/**
 * Sort a copy by persistent identity, not name, status, provider or input order.
 * For the same set of unique agent IDs, snapshot reordering cannot move seats.
 */
export function sortOfficeAgents(agents: readonly OfficeAgent[]): OfficeAgent[] {
  return [...agents].sort((left, right) =>
    left.id < right.id ? -1 : left.id > right.id ? 1 : 0,
  )
}

/** Empty input yields no pages. Every agent occupies one seat in one studio. */
export function paginateOfficeAgents(agents: readonly OfficeAgent[]): OfficePage[] {
  const roster = sortOfficeAgents(agents)
  const pages: OfficePage[] = []
  for (let offset = 0; offset < roster.length; offset += DESKS_PER_STUDIO) {
    pages.push({
      index: pages.length,
      agents: roster.slice(offset, offset + DESKS_PER_STUDIO),
    })
  }
  return pages
}

/** Returns a feet-center/chair anchor, never the desk's top-left corner. */
export function getDesk(index: number): OfficePoint {
  if (!Number.isInteger(index) || index < 0 || index >= DESKS_PER_STUDIO) {
    throw new RangeError('Desk index must be an integer from 0 through 5; page the roster first.')
  }
  return { ...DESKS[index] }
}

/** Conservative footprints traced from the Gemini scene, above each chair. */
export function getDeskBounds(index: number): OfficeRect {
  const seat = getDesk(index)
  return { x: seat.x - 45, y: seat.y - 72, width: 90, height: 62 }
}

/**
 * One finite arrival, ending at the feet anchor. A renderer may follow these
 * waypoints once on an actual arrival/seat change, or snap there for reduced
 * motion. Do not restart this route on every snapshot or loop it as wandering.
 * All waypoints avoid the decor-only right zone (x=448..624).
 */
export function getArrivalRoute(index: number): readonly OfficePoint[] {
  const seat = getDesk(index)
  const approachY = index < 3 ? 183 : 290
  return [
    { x: 420, y: 316 },
    { x: 397, y: 316 },
    { x: 397, y: approachY },
    { x: seat.x, y: approachY },
    seat,
  ]
}
