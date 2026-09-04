# Neon Floor 1986 — Verification & Quality Handoff

Specialist lane: **Verification and quality**. This document is a *handoff contract*, not an
implementation. The integration task must satisfy every MUST below; SHOULD items are strong
recommendations that may be waived with a recorded reason.

Related mission: GitHub #146 (dark-factory dogfood, three-agent studio swarm).

---

## 1. Testability contract (binding on the gameplay lane)

For any of this to be executable under `node --test`, the integration must respect these
boundaries. These are the minimum surface requirements the tests will bind against.

- MUST expose pure game logic from an ES module with **no DOM, Canvas, timer, or
  `window` references at import time**: `scenarios/neon-floor-1986/game.js`.
- MUST export at least:
  - `createGame(options?)` → fresh state object. `options` MAY carry `{ seed, config }`.
  - `step(state, input, dtMs)` → returns the next state (or mutates and returns `state`;
    pick one and document it — tests will assert whichever is declared here: **mutate-and-return
    is acceptable, but `createGame()` must never share nested objects between instances**).
  - `handleKey(state, code, phase)` where `phase` is `'down' | 'up'`, or an equivalent
    `setInput(state, inputSnapshot)`.
  - `CONFIG` (or `createGame().config`) exposing board bounds, drone paths, key positions,
    uplink position, tick size, and scoring constants.
- MUST make all motion a function of `(state, dtMs)` only — **no `Math.random()`, no
  `Date.now()`, no `performance.now()` inside `step`**. Any randomness must be a seeded PRNG
  stored in state. This is the determinism requirement; a test will run the same input script
  twice and assert deep equality of the resulting states.
- MUST keep rendering (`render.js` / inline canvas code) importing `game.js` and never the
  reverse.
- SHOULD express positions in abstract grid/world units so tests never depend on pixel scale.

State shape expectations used by tests:

| Field | Type | Notes |
| --- | --- | --- |
| `state.status` | `'idle' \| 'playing' \| 'paused' \| 'won' \| 'lost'` | exact string literals |
| `state.player` | `{ x, y }` | world units |
| `state.keys` | array of `{ x, y, collected }` | length 3 |
| `state.keysCollected` | number | `0..3` |
| `state.drones` | array, length ≥ 2 | each with `{ x, y }` and deterministic path data |
| `state.score` | number | integer, never negative |
| `state.uplink` | `{ x, y }` | goal |
| `state.elapsedMs` | number | monotonic while `playing` |

---

## 2. Executable Node tests — required coverage

File: `scenarios/neon-floor-1986/test/game.test.mjs`
Command: `node --test scenarios/neon-floor-1986/test/game.test.mjs`
Constraints: MUST use only `node:test` and `node:assert/strict`. **No devDependencies, no
`package.json` install step, no network.** MUST pass on Node 18+.

Each numbered case below is a required `test(...)`. Names are suggestions; coverage is not.

### 2.1 Movement bounds
1. **Moves in all four directions.** From a known start, one step of `ArrowRight`/`d` increases
   `x`; `ArrowLeft`/`a` decreases; `ArrowUp`/`w` and `ArrowDown`/`s` change `y` in the declared
   screen-space direction (assert the direction you document, and document it).
2. **Clamped at every wall.** Drive 500 steps toward each of the four bounds; assert the player
   is inside `[minX, maxX] × [minY, maxY]` and never NaN. Regression target: off-by-one at the
   far edge where `x + speed*dt` overshoots.
3. **WASD and arrows are equivalent.** Same script via `KeyW` and via `ArrowUp` produce equal
   player positions.
4. **Diagonal input does not exceed max speed.** Holding right+up for one tick must not move
   the player farther (Euclidean) than `speed * dt * (1 + 1e-9)`. If the design intends
   un-normalized diagonals, state that here instead — but then assert the intended value.
5. **Zero and huge `dtMs`.** `step(s, input, 0)` is a no-op for position. A large `dtMs`
   (e.g. 5000) must not tunnel the player through a wall or a drone (see 2.2.3).

### 2.2 Collision
1. **Drone contact loses.** Place the player on a drone's position (via a config/seeded start or
   by stepping into a known path point); after `step`, `status === 'lost'`.
2. **Near-miss does not lose.** Player at collision radius + epsilon stays `'playing'`.
3. **No tunneling.** With a single large `dtMs` that would sweep the player across a drone,
   the result MUST be `'lost'` — or, if the design accepts a fixed internal substep, assert
   `step` subdivides. Pick one and encode it.
4. **Collision is inert when not `playing`.** Overlap a drone while `paused`, `idle`, `won`, or
   `lost`; status must not change to `'lost'` from `'won'`/`'idle'`.

### 2.3 Key collection
1. **Collecting one key** sets that key's `collected`, increments `keysCollected` to 1, and
   increases `score`.
2. **Idempotent.** Stepping again on a collected key does not increment `keysCollected` or
   `score` a second time.
3. **All three** can be collected; `keysCollected === 3` and every `keys[i].collected` is true.
4. **Order independence.** Collecting in order `[2,0,1]` yields the same `keysCollected` and
   the same total key score as `[0,1,2]`.

### 2.4 Win
1. **Uplink with 3 keys wins.** With `keysCollected === 3`, reaching `uplink` sets
   `status === 'won'`.
2. **Uplink with <3 keys does NOT win.** Status stays `'playing'`; the game SHOULD surface a
   "keys required" hint flag (e.g. `state.hint`) rather than silently doing nothing.
3. **Terminal.** After winning, further `step` calls leave `status === 'won'`, and the player
   position and score are frozen.

### 2.5 Loss
1. **Drone contact ⇒ `'lost'`** (also covered by 2.2.1; keep an explicit loss-state test).
2. **Terminal.** After losing, further `step` calls keep `'lost'`; player does not move.
3. If a timer/energy loss condition exists, assert it fires at the boundary and *not* one tick
   early.

### 2.6 Pause
1. **Toggle.** From `'playing'`, pause input ⇒ `'paused'`; again ⇒ `'playing'`.
2. **Frozen world.** While `'paused'`, 100 steps of `dtMs=16` leave player, drones, `score`,
   and `elapsedMs` deep-equal to the pre-pause snapshot.
3. **Pause is a no-op from `'idle'`, `'won'`, `'lost'`** (or explicitly documented otherwise).
4. **Resume continues, not restarts** — key collection and score survive the pause.

### 2.7 Restart
1. **From every status** (`idle`, `playing`, `paused`, `won`, `lost`), restart yields a state
   deep-equal to a fresh `createGame()` with the same seed — except fields documented as
   carried over (e.g. `bestScore`), which MUST be listed here by the integrator.
2. **No shared references.** Mutating the restarted state's `keys[0]` must not affect a
   separately created game (guards against a module-level config object leaking into state).

### 2.8 Determinism
1. Run a fixed 600-tick input script twice from the same seed; assert
   `deepStrictEqual(runA, runB)`.
2. Drone positions at tick N are a pure function of N — assert two independent games agree at
   tick 300.

### 2.9 Edge cases (must not crash)
- Unknown key codes (`'F13'`, `''`, `undefined`) are ignored, no throw.
- `keyup` without a preceding `keydown` is ignored.
- Simultaneous opposite inputs (left+right) resolve to a documented outcome (net zero is
  preferred) rather than jitter.
- Negative or `NaN` `dtMs` is rejected/clamped, never producing `NaN` positions. Add an
  assertion that every numeric state field is `Number.isFinite`.
- Two keys occupying the same tile, or a key on the uplink, does not double-count.
- Losing on the exact tick a key is collected has one documented winner (recommended: loss
  takes precedence).

**Bar:** all tests pass with zero failures and zero `skip`. Test runtime SHOULD stay under
2 seconds total.

---

## 3. Browser acceptance script

Run: open `scenarios/neon-floor-1986/index.html` directly via `file://`. **A local server must
not be required**; if ES modules over `file://` are blocked, inline the module or ship a
`<script type="module">` fallback — this is the integrator's problem to solve, and the
acceptance step is "double-click the file and it plays."

Perform each step with **the keyboard only** (never touch the mouse):

| # | Action | Expected |
| --- | --- | --- |
| B1 | Load page, open DevTools console | Zero errors and zero unhandled rejections. Warnings noted. |
| B2 | Press `Tab` from page load | Focus lands on a visible, clearly outlined interactive control. |
| B3 | Press the documented start key (`Enter`/`Space`) | Status region announces `playing`; drones begin moving. |
| B4 | Move with arrows, then with WASD | Both move the operator; no page scroll (arrows/space must be `preventDefault`-ed while playing). |
| B5 | Collect one key | HUD key counter goes `0/3 → 1/3`; score increases. |
| B6 | Press pause | Status shows `paused`; world visibly frozen; a paused overlay is present. |
| B7 | Press pause again | Play resumes with progress intact. |
| B8 | Walk into a drone | Status `lost`; a lose message is shown and is focusable/announced. |
| B9 | Press restart | Status returns to `idle`/`playing`; keys `0/3`; score reset. |
| B10 | Collect all 3 keys, reach uplink | Status `won` with a win message. |
| B11 | Reach uplink with 2 keys | No win; a hint explains three keys are required. |
| B12 | Reload with `prefers-reduced-motion: reduce` | No CRT flicker/scanline animation/parallax; the game remains fully playable and all state changes still visible. |
| B13 | Resize to 390 × 844 | No horizontal scrollbar; HUD and canvas fully visible; controls reachable. |
| B14 | Leave idle 60s, then play 3 minutes | No memory growth trend, no runaway timers, no console noise. |

Evidence to record: desktop screenshot, 390px screenshot, console screenshot (clean), and a
short note per row. Screenshots MUST show original artwork only.

### HUD requirements (verified in B5–B10)
The HUD MUST visibly report, at all times during play: **keys collected (n/3)**, **score**,
**current state**, and **the control legend**. State text MUST be readable without color alone.

---

## 4. Accessibility checks

- **A1 — Semantics.** Start/pause/restart are real `<button>` elements (not `div`s with
  handlers). The canvas has a meaningful `aria-label` and a text alternative describing the
  objective. Page has one `<h1>` and a sensible heading order.
- **A2 — Live region.** Status changes (`playing`/`paused`/`won`/`lost`, key collected) are
  announced via `aria-live="polite"` on a status element with `role="status"`. Win/lose SHOULD
  be `polite` too — avoid `assertive` spam on every key pickup (throttle to state changes).
- **A3 — Focus visibility.** Every focusable element has a focus indicator with ≥3:1 contrast
  against its background and is not removed by `outline: none` without a replacement. Verify
  in both light-surface and dark panels.
- **A4 — Focus order & trap.** Tab order is logical; no keyboard trap; no positive `tabindex`.
- **A5 — Contrast.** Body and HUD text ≥ 4.5:1; large text and UI/graphical boundaries ≥ 3:1.
  Neon-on-dark accents are the risk area — measure the actual computed colors, don't eyeball.
  Record the measured ratios for: HUD text, status text, button labels, button borders,
  player vs. floor, drone vs. floor, key vs. floor.
- **A6 — Color independence.** Player, keys, drones, and win/lose are distinguishable by shape
  or label, not hue alone (protanopia/deuteranopia check).
- **A7 — Reduced motion.** `@media (prefers-reduced-motion: reduce)` disables scanline drift,
  glow pulsing, screen shake, and any decorative transition. Essential position updates remain.
- **A8 — Text scaling.** At 200% browser zoom and at 390px width, no clipped text and no
  horizontal overflow.
- **A9 — No flashing.** Nothing flashes more than 3 times per second, in any mode.
- **A10 — Instructions.** Controls are visible on screen (not only in the README) before and
  during play.
- **A11 — Language & title.** `<html lang="en">`, a descriptive `<title>`.

Tooling: browser DevTools accessibility pane + contrast picker is sufficient; an automated
axe run is a SHOULD, not a MUST, since no dependencies may be installed.

---

## 5. Performance constraints

- **P1** — Steady 60fps on a mid-range laptop; no frame over 33ms during normal play.
- **P2** — Single `requestAnimationFrame` loop. No `setInterval` for game updates. The loop
  MUST stop or idle cheaply when `paused`/`idle`/`won`/`lost`.
- **P3** — No per-frame allocation churn: no object/array literals created per entity per
  frame in the hot path where avoidable; no per-frame string concatenation for the canvas.
- **P4** — DOM writes only on change. HUD text updates when values change, not every frame.
- **P5** — Total payload under ~150 KB across all files; zero network requests after load
  (verify: DevTools Network tab shows only the local documents — no fonts, no analytics,
  no CDN).
- **P6** — Canvas sized once per resize (debounced), respecting `devicePixelRatio`; not
  resized inside the render loop.
- **P7** — Listeners are registered once; restart must not stack duplicate handlers. Test by
  restarting 20 times and confirming input still moves the player exactly one step's worth.

---

## 6. Edge cases the integration must handle

1. Window blur / tab hidden mid-play → auto-pause (or clamp `dt`), never a giant catch-up
   frame that teleports the player into a drone.
2. `dtMs` spike after a long stall → clamp to a max step (e.g. 50ms) and/or substep.
3. Rapid restart spam (held key) → no duplicate loops, no state corruption.
4. Holding a movement key across a restart → the stale key is not still "down" in the new game,
   or is re-read cleanly from the live keyboard state.
5. Pause pressed on the same tick as a collision → document precedence (recommended: the
   collision resolves first).
6. Extremely narrow (320px) or very wide (2560px) viewports → still playable, no overflow.
7. Zoomed-out/HiDPI displays → crisp pixels via `image-rendering: pixelated`, no blur.
8. Running the module twice in one page (double `<script>` include) → no global collisions.

---

## 7. Definition of done (verification gate)

The integration task is not complete until **all** of these are recorded with evidence:

- [ ] `node --test scenarios/neon-floor-1986/test/game.test.mjs` passes — paste the summary line.
- [ ] Every numbered case in §2 exists as a test.
- [ ] All B1–B14 rows in §3 pass, with notes.
- [ ] All A1–A11 checks in §4 pass, with measured contrast ratios recorded.
- [ ] P1–P7 in §5 verified; Network tab shows zero external requests.
- [ ] §6 edge cases handled or explicitly waived with a reason.
- [ ] Desktop and 390px screenshots attached.
- [ ] `README.md` documents run steps and controls, and matches the on-screen legend.
- [ ] Confirmed no copyrighted characters, names, sprites, audio, or level designs; all art
      generated in-repo from code.

Any waiver MUST be written down in the PR description with a justification. Silent omission
fails this gate.
