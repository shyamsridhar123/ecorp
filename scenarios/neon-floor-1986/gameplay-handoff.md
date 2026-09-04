# Neon Floor 1986 — gameplay and systems handoff

GitHub issue: #146
Specialist lane: Gameplay and systems
Status: Authoritative handoff for the integration task

## Scope

This artifact defines the deterministic game rules, state machine, collision model, scoring, canonical level, and JavaScript module boundary. It intentionally does not implement the browser game, visual treatment, accessibility layer, or acceptance harness.

The integration task should preserve these rules unless a verified specialist conflict requires an explicit, documented resolution.

## Core play contract

- The game is a compact, grid-based office infiltration game.
- The player must collect exactly three access keys and then enter the uplink tile.
- Two security drones patrol fixed routes while the game is in `playing`.
- Touching or crossing through a drone ends the run immediately.
- There is no randomness, procedural generation, network access, wall-clock dependency, or frame-rate-dependent simulation.
- The five phases are `idle`, `playing`, `paused`, `won`, and `lost`.
- All gameplay coordinates are integer tile coordinates with `(0, 0)` at the upper-left.
- Rendering may interpolate between snapshots, but authoritative positions remain integers.

## Canonical level

The board is 18 columns by 12 rows. The outer boundary and the four desk pods are walls. Every other tile is walkable.

```text
    012345678901234567
 0  ##################
 1  #.......B.......U#
 2  #..K.##...##.....#
 3  #....##...##..K..#
 4  #................#
 5  #.A..............#
 6  #................#
 7  #....##...##.....#
 8  #....##.K.##.....#
 9  #................#
10  #P...............#
11  ##################
```

Legend:

- `#`: wall
- `.`: floor
- `P`: player start
- `K`: uncollected access key
- `A`, `B`: drone starts
- `U`: uplink

Canonical coordinates:

| Object | Coordinate |
| --- | --- |
| Player start | `(1, 10)` |
| Key `key-cyan` | `(3, 2)` |
| Key `key-amber` | `(14, 3)` |
| Key `key-magenta` | `(8, 8)` |
| Uplink | `(16, 1)` |
| Drone `scanner-a` start | `(2, 5)` |
| Drone `scanner-b` start | `(8, 1)` |

The desk-pod wall rectangles are inclusive:

- `x=5..6, y=2..3`
- `x=10..11, y=2..3`
- `x=5..6, y=7..8`
- `x=10..11, y=7..8`

The canonical layout must be exported as data rather than inferred from rendered pixels.

## Fixed-step simulation

- One authoritative logic tick is `125 ms`, or 8 ticks per second.
- The engine receives at most one normalized movement intent per tick: `up`, `down`, `left`, `right`, or `null`.
- A valid intent attempts to move the player by exactly one tile.
- Opposing or simultaneous physical key presses are resolved by the input adapter before the engine is called. The most recently pressed still-held direction should win.
- A blocked move leaves the player in place, but the tick still advances and scheduled drones still move.
- A neutral tick leaves the player in place while scheduled drones continue to move.
- Only `playing` consumes ticks. `idle`, `paused`, `won`, and `lost` are frozen.
- The browser loop may use `requestAnimationFrame`, but it must feed whole fixed ticks into the engine. It must not pass elapsed milliseconds into gameplay math.
- To avoid a catch-up spiral, the browser adapter should process at most four accumulated ticks in one animation frame and discard any further accumulated lag.

Reduced-motion preferences affect rendering only. They must not alter tick rate, routes, collision, score, or outcome.

## Drone routes

Each drone has a fixed route, a route index, and a positive integer movement period. On a scheduled move, its route index advances by one modulo route length and its position becomes the coordinate at that index.

### `scanner-a`

- Period: every 3rd active tick.
- Start index: `0`.
- Route: horizontal ping-pong patrol on row `y=5`.

```js
[
  [2, 5], [3, 5], [4, 5], [5, 5], [6, 5], [7, 5], [8, 5],
  [9, 5], [10, 5], [11, 5], [12, 5], [13, 5], [14, 5], [15, 5],
  [14, 5], [13, 5], [12, 5], [11, 5], [10, 5], [9, 5], [8, 5],
  [7, 5], [6, 5], [5, 5], [4, 5], [3, 5]
]
```

### `scanner-b`

- Period: every 4th active tick.
- Start index: `0`.
- Route: vertical ping-pong patrol on column `x=8`.

```js
[
  [8, 1], [8, 2], [8, 3], [8, 4], [8, 5], [8, 6], [8, 7],
  [8, 8], [8, 9], [8, 10], [8, 9], [8, 8], [8, 7], [8, 6],
  [8, 5], [8, 4], [8, 3], [8, 2]
]
```

For a tick numbered `nextTick`, a drone moves when:

```js
nextTick % drone.period === 0
```

Thus `scanner-a` first moves on tick 3 and `scanner-b` first moves on tick 4.

Drones do not collide with walls because every route coordinate is prevalidated as walkable. Drones may occupy the same tile as each other without affecting their routes.

## State model

The logic module should use plain serializable data. The minimum authoritative state shape is:

```js
{
  version: 1,
  phase: "idle",
  tick: 0,
  score: 0,
  player: { x: 1, y: 10 },
  keys: [
    { id: "key-cyan", x: 3, y: 2, collected: false },
    { id: "key-amber", x: 14, y: 3, collected: false },
    { id: "key-magenta", x: 8, y: 8, collected: false }
  ],
  uplink: { x: 16, y: 1 },
  drones: [
    { id: "scanner-a", routeIndex: 0, period: 3, x: 2, y: 5 },
    { id: "scanner-b", routeIndex: 0, period: 4, x: 8, y: 1 }
  ],
  result: null
}
```

`result` is `null` except in terminal states:

```js
{ kind: "won", tick: number, finalScore: number }
{ kind: "lost", tick: number, droneId: string, collision: string }
```

Allowed `collision` values are:

- `occupied`: the player attempted to enter a drone's pre-move tile
- `landed`: a drone and player ended the tick on the same tile
- `crossed`: the player and a drone swapped tiles during one tick

State invariants:

- `phase` is one of the five declared phases.
- `tick` and `score` are non-negative integers.
- The player, uplink, keys, and every drone route coordinate are walkable.
- There are exactly three uniquely identified keys.
- There are at least two uniquely identified drones.
- A drone's stored `(x, y)` equals the coordinate at its `routeIndex`.
- `result === null` in nonterminal phases.
- `result.kind` agrees with `phase` in terminal phases.
- Key collection and terminal bonuses are applied at most once.

## Actions and transitions

The pure reducer accepts these actions:

```js
{ type: "START" }
{ type: "TOGGLE_PAUSE" }
{ type: "RESTART" }
{ type: "TICK", intent: "up" | "down" | "left" | "right" | null }
```

Transition table:

| Current phase | Action | Next phase and effect |
| --- | --- | --- |
| `idle` | `START` | `playing`; no tick is consumed |
| `idle` | `RESTART` | fresh `idle` state |
| `playing` | `TICK` | run one deterministic tick; may remain `playing`, become `won`, or become `lost` |
| `playing` | `TOGGLE_PAUSE` | `paused`; no tick is consumed |
| `playing` | `RESTART` | fresh `idle` state |
| `paused` | `TOGGLE_PAUSE` | `playing`; no tick is consumed |
| `paused` | `RESTART` | fresh `idle` state |
| `won` or `lost` | `RESTART` | fresh `idle` state |
| Any other combination | any | no state change |

`RESTART` must be equivalent to a new `createInitialState()` call. Starting, pausing, and resuming must not move any actor or change the score.

## Movement and collision rules

Direction deltas are fixed:

```js
{
  up: [0, -1],
  down: [0, 1],
  left: [-1, 0],
  right: [1, 0]
}
```

A player destination is accepted only when it is inside the board and not a wall. There is no diagonal movement, pushing, wrapping, or partial movement.

Each `TICK` must resolve in this exact order:

1. Return unchanged unless the phase is `playing`.
2. Set `nextTick = state.tick + 1`.
3. Save the player's old coordinate and every drone's old coordinate.
4. Apply the normalized player intent to produce a tentative player coordinate. A blocked move produces the old coordinate.
5. Advance each drone scheduled for `nextTick`; leave all other drones in place.
6. Detect collision against each drone, in stable drone-array order:
   - `occupied` if the tentative player coordinate equals that drone's old coordinate.
   - Otherwise `landed` if the tentative player coordinate equals that drone's new coordinate.
   - Otherwise `crossed` if the player's old coordinate equals the drone's new coordinate and the tentative player coordinate equals the drone's old coordinate.
7. If a collision exists, commit the actor positions and `nextTick`, set `phase` to `lost`, set the loss result, and stop. Do not collect a key or award points from that tick.
8. If safe, collect every uncollected key at the tentative player coordinate. The canonical level permits at most one.
9. Add the key award for any newly collected key.
10. If all three keys are collected and the tentative player coordinate is the uplink, apply the one-time win award, set `phase` to `won`, and set the win result.
11. Otherwise commit the tick as `playing`.

Collision always outranks collection and winning. Entering the uplink without all three keys is safe but does not end the game.

The reducer should return a new state object for every effective action and return the original object for ignored actions. It must not mutate its input.

## Scoring

Scoring is deterministic and event-based:

- Start: `0`
- Each newly collected key: `+250`
- Win base award: `+1000`
- Win time bonus: `max(0, 500 - winningTick)`
- Blocked movement, elapsed ticks, pause, and loss: no direct score change

The final winning score is therefore:

```js
750 + 1000 + Math.max(0, 500 - winningTick)
```

No scoring event may execute after a terminal transition, and repeated rendering or ignored actions must never change score.

## Required logic module boundary

Implement gameplay in:

```text
scenarios/neon-floor-1986/game.mjs
```

`game.mjs` must:

- contain all authoritative level data and mechanics;
- be importable by both a browser and Node without shims;
- avoid `window`, `document`, canvas, timers, animation frames, audio, storage, fetch, and environment inspection;
- avoid `Date`, `Math.random`, floating-point positions, and hidden module-level mutable state;
- expose enough pure functions and constants for deterministic tests.

Required exports:

```js
export const TICK_MS = 125;
export const PHASES;
export const DIRECTIONS;
export const LEVEL;
export const DRONE_ROUTES;

export function createInitialState();
export function reduceGame(state, action);
export function isWalkable(x, y);
export function getHudModel(state);
```

`getHudModel(state)` is a pure projection with at least:

```js
{
  phase,
  score,
  tick,
  keysCollected,
  keysTotal,
  uplinkReady
}
```

Suggested integration-only boundaries:

```text
game.mjs    authoritative pure rules and state
render.mjs  canvas/DOM drawing from an immutable state snapshot
app.mjs     keyboard normalization, buttons, fixed-step loop, announcements
index.html  semantic shell and module entry point
```

The renderer must never infer collisions, collect keys, move drones, change score, or decide phase transitions. The app adapter must translate controls into reducer actions rather than mutating state directly.

## Input handoff

The integration layer should map:

- Arrow keys and `W`, `A`, `S`, `D` to directional intent.
- `Enter` or `Space` to `START` while idle.
- `P` and a semantic Pause/Resume button to `TOGGLE_PAUSE`.
- `R` and a semantic Restart button to `RESTART`.

Movement keys should prevent page scrolling only while the game control surface is active. Key-repeat behavior must not directly dispatch extra movement actions; held-key state is sampled once per fixed tick.

Losing focus must clear held directions. If the document becomes hidden during play, the app should dispatch one pause action and announce that the run was paused; it must not simulate a backlog when visibility returns.

## Deterministic test obligations

`scenarios/neon-floor-1986/test/game.test.mjs` should use Node's built-in test runner and strict assertions. At minimum, it must prove:

1. **Initial and start state**
   - Initial phase is `idle`.
   - `START` enters `playing` without changing positions, tick, or score.

2. **Movement and bounds**
   - Each direction changes the player by one walkable tile.
   - Outer walls and desk-pod walls block movement.
   - A blocked move still advances the active tick and scheduled drones.

3. **Determinism**
   - Applying the same initial state and action sequence twice yields deeply equal states.
   - No reducer call mutates the prior state.

4. **Key collection**
   - Entering an uncollected key tile marks only that key collected and adds exactly 250.
   - Remaining on or revisiting the tile cannot award the key again.

5. **Uplink gate and win**
   - Entering the uplink with fewer than three keys does not win.
   - Entering it with all keys wins exactly once.
   - The final score includes all key awards, the win award, and the tick-derived time bonus.
   - Further ticks in `won` are ignored.

6. **Drone schedules**
   - `scanner-a` moves on ticks divisible by 3 and otherwise stays put.
   - `scanner-b` moves on ticks divisible by 4 and otherwise stays put.
   - Each route wraps to index 0.

7. **Collision and loss**
   - Entering a drone's old tile loses even if that drone moves away.
   - A drone landing on a stationary player loses.
   - Swapping tiles with a drone loses.
   - A collision on a key or uplink tick prevents collection and winning.
   - The loss result identifies the first colliding drone in stable array order.
   - Further ticks in `lost` are ignored.

8. **Pause**
   - Pausing changes only the phase.
   - Ticks while paused cannot change tick, positions, keys, score, or result.
   - Resuming returns to `playing` without consuming a tick.

9. **Restart**
   - Restart from `playing`, `paused`, `won`, and `lost` deeply equals `createInitialState()`.

Recommended command:

```powershell
node --test scenarios/neon-floor-1986/test/game.test.mjs
```

## Integration acceptance checklist

Before consuming this handoff, the integration task should confirm:

- The canonical map, keys, uplink, routes, periods, score values, and tick duration match this artifact.
- All five phases are visible through the HUD projection.
- Two drones remain deterministic under long runs and route wraparound.
- The game can be fully controlled without a pointer.
- Pause freezes the authoritative simulation.
- Restart is a true reset, not a partial visual reset.
- Reduced motion changes presentation only.
- Node tests import the same `game.mjs` used by the browser.
- No renderer or event handler contains duplicate gameplay rules.

## Flexible integration choices

The visual specialist and integrator may choose canvas versus DOM/SVG rendering, palette, sprite shapes, interpolation, sound policy, labels, help layout, and responsive composition. They may add non-authoritative presentation fields outside the game state.

They must not change the canonical mechanics above merely to simplify rendering. If a later integration conflict requires a mechanics change, update the tests and record the exact deviation in integration evidence rather than silently creating two rule sets.
