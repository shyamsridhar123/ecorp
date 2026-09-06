import assert from 'node:assert/strict'
import test from 'node:test'
import {
  DESKS_PER_STUDIO,
  OFFICE_HEIGHT,
  OFFICE_WIDTH,
  getArrivalRoute,
  getDesk,
  getDeskBounds,
  paginateOfficeAgents,
  resolveOfficeState,
  sortOfficeAgents,
  stateDescription,
  stateLabel,
} from '../apps/web/src/office/officeModel.ts'

/** @typedef {import('../apps/web/src/office/officeModel.ts').OfficeAgent} OfficeAgent */

/** @param {Partial<OfficeAgent>} overrides */
function agent(overrides = {}) {
  return {
    id: 'agent-01',
    name: 'Alice',
    role: 'Engineer',
    adapter: 'codex',
    status: 'idle',
    station: null,
    current_run_id: null,
    ...overrides,
  }
}

function roster(count) {
  return Array.from({ length: count }, (_, index) =>
    agent({
      id: `agent-${String(index + 1).padStart(2, '0')}`,
      name: `Engineer ${count - index}`,
    }),
  )
}

const ids = (agents) => agents.map((entry) => entry.id)

for (const status of ['idle', 'starting', 'working', 'blocked', 'reviewing', 'offline']) {
  test(`reported ${status} remains ${status} without a pending request`, () => {
    assert.equal(resolveOfficeState(agent({ status })), status)
  })
}

for (const [station, expected] of [
  ['read', 'reading'],
  ['search', 'reading'],
  [' READ ', 'reading'],
  ['Search', 'reading'],
  ['test', 'working'],
  ['build', 'working'],
  ['review', 'working'],
  ['research', 'working'],
  ['bread', 'working'],
  ['break', 'working'],
  ['unknown-tool', 'working'],
  [null, 'working'],
]) {
  test(`working station ${JSON.stringify(station)} projects ${expected}`, () => {
    assert.equal(
      resolveOfficeState(agent({ status: 'working', station, current_run_id: 'run-current' })),
      expected,
    )
  })
}

test('a stale station does not invent activity for a non-working agent', () => {
  for (const status of ['idle', 'starting', 'blocked', 'reviewing', 'offline']) {
    for (const station of ['read', 'search', 'test', 'build', 'approval']) {
      assert.equal(resolveOfficeState(agent({ status, station })), status)
    }
  }
})

test('a blocked agent needs an exact current-run approval match', () => {
  const blocked = agent({
    status: 'blocked',
    station: 'approval',
    current_run_id: 'run-current',
  })
  assert.equal(resolveOfficeState(blocked), 'blocked')
  assert.equal(resolveOfficeState(blocked, new Set(['run-current'])), 'approval')
  assert.equal(resolveOfficeState(blocked, new Set(['agent-01'])), 'blocked')
  assert.equal(resolveOfficeState(blocked, new Set(['run-current-other'])), 'blocked')
})

test('an exact current-run pending review is distinct from an action approval', () => {
  const blocked = agent({ status: 'blocked', current_run_id: 'run-current' })
  assert.equal(resolveOfficeState(blocked, new Set(), new Set(['run-current'])), 'reviewing')
  assert.equal(resolveOfficeState(blocked, new Set(['run-current']), new Set()), 'approval')
})

test('unrelated and prior-run pending requests cannot affect the current agent', () => {
  const unrelated = new Set(['run-prior', 'run-other-agent', 'RUN-CURRENT'])
  for (const [status, station, expected] of [
    ['idle', null, 'idle'],
    ['working', 'build', 'working'],
    ['working', 'read', 'reading'],
    ['blocked', null, 'blocked'],
  ]) {
    assert.equal(
      resolveOfficeState(
        agent({ status, station, current_run_id: 'run-current' }),
        unrelated,
        unrelated,
      ),
      expected,
    )
  }
})

test('an approval stops affecting an agent as soon as its current run changes', () => {
  const blocked = agent({ status: 'blocked', current_run_id: 'run-prior' })
  const pending = new Set(['run-prior'])
  assert.equal(resolveOfficeState(blocked, pending), 'approval')
  assert.equal(
    resolveOfficeState({ ...blocked, current_run_id: 'run-current' }, pending),
    'blocked',
  )
})

test('null and empty current-run IDs cannot acquire pending request state', () => {
  const pending = new Set(['agent-01', 'run-prior', ''])
  for (const current_run_id of [null, '']) {
    assert.equal(
      resolveOfficeState(agent({ status: 'blocked', current_run_id }), pending, pending),
      'blocked',
    )
  }
})

test('offline wins even when the recorded current run has both pending requests', () => {
  const offline = agent({
    status: 'offline',
    station: 'read',
    current_run_id: 'run-current',
  })
  const pending = new Set(['run-current'])
  assert.equal(resolveOfficeState(offline, pending, pending), 'offline')
})

test('exact action approval takes precedence over exact review', () => {
  const pending = new Set(['run-current'])
  assert.equal(
    resolveOfficeState(
      agent({ status: 'reviewing', current_run_id: 'run-current' }),
      pending,
      pending,
    ),
    'approval',
  )
})

test('explicit current-run requests do not depend on a blocked display status', () => {
  const active = agent({ status: 'working', station: 'build', current_run_id: 'run-current' })
  assert.equal(resolveOfficeState(active, new Set(['run-current'])), 'approval')
  assert.equal(resolveOfficeState(active, new Set(), new Set(['run-current'])), 'reviewing')
})

test('projection does not mutate agents or request sets or infer state from identity text', () => {
  const entry = Object.freeze(agent({
    name: 'Awaiting approval',
    role: 'Reviewer',
    adapter: 'fake-process',
    status: 'working',
    station: 'build',
    current_run_id: 'run-current',
  }))
  const approvals = new Set(['run-prior'])
  const reviews = new Set(['run-other-agent'])
  for (let index = 0; index < 5; index++) {
    assert.equal(resolveOfficeState(entry, approvals, reviews), 'working')
  }
  assert.deepEqual([...approvals], ['run-prior'])
  assert.deepEqual([...reviews], ['run-other-agent'])
  assert.equal(entry.status, 'working')
})

test('all eight states have distinct labels and factual descriptions', () => {
  const labels = {
    idle: 'Idle',
    starting: 'Starting',
    working: 'Working',
    reading: 'Reading',
    reviewing: 'Reviewing',
    approval: 'Awaiting approval',
    blocked: 'Blocked',
    offline: 'Offline',
  }
  for (const [state, label] of Object.entries(labels)) {
    assert.equal(stateLabel(state), label)
    assert.ok(stateDescription(state).trim().length > 15)
  }
  assert.equal(new Set(Object.keys(labels).map(stateLabel)).size, 8)
  assert.match(stateDescription('approval'), /current run.*pending approval/)
  assert.match(stateDescription('blocked'), /without a matching/)
  assert.match(stateDescription('reading'), /read or search/)
})

test('roster ordering is stable by ID without mutating the source array', () => {
  const entries = Object.freeze([
    Object.freeze(agent({ id: 'c', name: 'A' })),
    Object.freeze(agent({ id: 'a', name: 'Z' })),
    Object.freeze(agent({ id: 'b', name: 'M' })),
  ])
  const sorted = sortOfficeAgents(entries)
  assert.deepEqual(ids(sorted), ['a', 'b', 'c'])
  assert.deepEqual(ids(entries), ['c', 'a', 'b'])
  assert.notEqual(sorted, entries)
  assert.deepEqual(sortOfficeAgents([...entries].reverse()), sorted)
})

test('renaming or changing activity does not move an existing roster to different seats', () => {
  const entries = roster(7).reverse()
  const renamed = entries.map((entry, index) => ({
    ...entry,
    name: `Renamed ${index}`,
    status: index === 0 ? 'offline' : 'working',
    station: index === 0 ? null : 'read',
  }))
  assert.deepEqual(
    paginateOfficeAgents(renamed).map((page) => ids(page.agents)),
    paginateOfficeAgents(entries).map((page) => ids(page.agents)),
  )
})

for (const [count, sizes] of [
  [0, []],
  [1, [1]],
  [5, [5]],
  [6, [6]],
  [7, [6, 1]],
  [11, [6, 5]],
  [12, [6, 6]],
  [13, [6, 6, 1]],
  [17, [6, 6, 5]],
  [18, [6, 6, 6]],
  [19, [6, 6, 6, 1]],
]) {
  test(`${count} agents occupy exactly ${sizes.length} studios without stacked seats`, () => {
    const entries = Object.freeze(roster(count).reverse())
    const pages = paginateOfficeAgents(entries)
    assert.deepEqual(pages.map((page) => page.agents.length), sizes)
    assert.deepEqual(pages.map((page) => page.index), sizes.map((_, index) => index))
    assert.deepEqual(ids(pages.flatMap((page) => page.agents)), ids(roster(count)))
    const occupied = new Set()
    for (const page of pages) {
      assert.ok(page.agents.length <= DESKS_PER_STUDIO)
      for (let seat = 0; seat < page.agents.length; seat++) {
        const point = getDesk(seat)
        const key = `${page.index}:${point.x}:${point.y}`
        assert.equal(occupied.has(key), false)
        occupied.add(key)
      }
    }
    assert.equal(occupied.size, count)
  })
}

test('snapshot input reordering preserves page and seat assignment for 18 agents', () => {
  const entries = roster(18)
  const reordered = [
    ...entries.filter((_, index) => index % 2 === 1).reverse(),
    ...entries.filter((_, index) => index % 2 === 0).reverse(),
  ]
  assert.deepEqual(paginateOfficeAgents(reordered), paginateOfficeAgents(entries))
})

test('reordering across a partially filled page boundary preserves each agent seat', () => {
  const placements = (entries) => paginateOfficeAgents(entries).flatMap((page) =>
    page.agents.map((entry, seat) => ({
      id: entry.id,
      studio: page.index,
      seat,
      point: getDesk(seat),
    })),
  )
  for (const count of [7, 13, 19]) {
    const entries = roster(count)
    const reordered = [...entries.slice(6), ...entries.slice(0, 6)].reverse()
    assert.deepEqual(placements(reordered), placements(entries))
  }
})

test('studio dimensions and the six feet anchors match the renderer contract exactly', () => {
  assert.equal(OFFICE_WIDTH, 640)
  assert.equal(OFFICE_HEIGHT, 320)
  assert.equal(DESKS_PER_STUDIO, 6)
  assert.deepEqual(Array.from({ length: 6 }, (_, index) => getDesk(index)), [
    { x: 129, y: 162 },
    { x: 225, y: 162 },
    { x: 331, y: 162 },
    { x: 104, y: 270 },
    { x: 209, y: 270 },
    { x: 319, y: 270 },
  ])
})

test('desk access never wraps overflow or invalid indexes onto occupied seats', () => {
  for (const index of [-1, 6, 7, 18, 0.5, NaN, Infinity, -Infinity]) {
    for (const helper of [getDesk, getDeskBounds, getArrivalRoute]) {
      assert.throws(() => helper(index), RangeError)
    }
  }
})

function rectanglesOverlap(left, right) {
  return left.x < right.x + right.width
    && left.x + left.width > right.x
    && left.y < right.y + right.height
    && left.y + left.height > right.y
}

// Independent closed-edge segment/rectangle check, not sampled waypoints.
function segmentTouchesRectangle(from, to, rect) {
  if (from.x === to.x) {
    return from.x >= rect.x && from.x <= rect.x + rect.width
      && Math.max(from.y, to.y) >= rect.y
      && Math.min(from.y, to.y) <= rect.y + rect.height
  }
  assert.equal(from.y, to.y, 'arrival segments must be axis-aligned')
  return from.y >= rect.y && from.y <= rect.y + rect.height
    && Math.max(from.x, to.x) >= rect.x
    && Math.min(from.x, to.x) <= rect.x + rect.width
}

test('all desk footprints stay above their chairs and never overlap another desk or decor', () => {
  const bounds = Array.from({ length: DESKS_PER_STUDIO }, (_, index) => getDeskBounds(index))
  for (let index = 0; index < bounds.length; index++) {
    const seat = getDesk(index)
    const rect = bounds[index]
    assert.deepEqual(rect, { x: seat.x - 45, y: seat.y - 72, width: 90, height: 62 })
    assert.equal(rect.y + rect.height, seat.y - 10)
    assert.ok(rect.x >= 0 && rect.x + rect.width <= 376)
    assert.ok(rect.y >= 0 && rect.y + rect.height < OFFICE_HEIGHT)
    for (let other = index + 1; other < bounds.length; other++) {
      assert.equal(rectanglesOverlap(rect, bounds[other]), false)
    }
  }
})

for (let index = 0; index < DESKS_PER_STUDIO; index++) {
  test(`desk ${index} has one finite arrival route clear of every workstation`, () => {
    const seat = getDesk(index)
    const approachY = index < 3 ? 183 : 290
    const route = getArrivalRoute(index)
    assert.deepEqual(route, [
      { x: 420, y: 316 },
      { x: 397, y: 316 },
      { x: 397, y: approachY },
      { x: seat.x, y: approachY },
      seat,
    ])
    assert.equal(route.length, 5)
    assert.equal(new Set(route.map((point) => `${point.x}:${point.y}`)).size, 5)
    let distance = 0
    for (let step = 0; step < route.length; step++) {
      const point = route[step]
      assert.ok(Number.isFinite(point.x) && Number.isFinite(point.y))
      assert.ok(point.x >= 0 && point.x <= 432 && point.x < 448)
      assert.ok(point.y >= 0 && point.y <= OFFICE_HEIGHT)
      if (step === 0) continue
      const previous = route[step - 1]
      distance += Math.abs(previous.x - point.x) + Math.abs(previous.y - point.y)
      for (let desk = 0; desk < DESKS_PER_STUDIO; desk++) {
        const rect = getDeskBounds(desk)
        // Eight pixels of feet-center clearance, including the destination desk.
        const expanded = {
          x: rect.x - 8,
          y: rect.y - 8,
          width: rect.width + 16,
          height: rect.height + 16,
        }
        assert.equal(
          segmentTouchesRectangle(previous, point, expanded),
          false,
          `route to ${index}, segment ${step}, intersects desk ${desk}`,
        )
      }
    }
    assert.ok(distance > 0 && distance <= 504)
    assert.deepEqual(route.at(-1), seat)
  })
}

test('route and desk results do not expose shared mutable layout state', () => {
  const expectedDesk = { x: 129, y: 162 }
  const firstDesk = getDesk(0)
  firstDesk.x = -500
  assert.deepEqual(getDesk(0), expectedDesk)
  const firstRoute = getArrivalRoute(0)
  firstRoute[0].x = -500
  firstRoute.at(-1).y = -500
  assert.deepEqual(getArrivalRoute(0)[0], { x: 420, y: 316 })
  assert.deepEqual(getArrivalRoute(0).at(-1), expectedDesk)
  assert.deepEqual(getDesk(0), expectedDesk)
})
