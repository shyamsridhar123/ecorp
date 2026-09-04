# Neon Floor 1986 — Visual & Interaction Handoff

Specialist lane: **Visual and interaction**
Mission: GitHub #146 — Dark-factory dogfood: build Neon Floor 1986
Scope: `scenarios/neon-floor-1986/**` (this document only; no game code produced by this lane)

This document is the authoritative handoff for the integration pass. It defines the original
visual identity, HUD, controls, layout, accessibility, and motion requirements for Neon Floor
1986. All art direction described here is original (no copyrighted characters, sprites, music,
or level designs are referenced or reused).

## 1. Art direction

- **Setting**: a small top-down "neon office floor" — a fictional 1986 corporate data floor with
  cubicle partitions, server racks, and an "uplink terminal" exit.
- **Palette** (CRT/hacker-thriller-adjacent mood, restrained, all original hex values):
  - Background/void: `#0a0e14` (near-black navy)
  - Floor tile: `#12181f` with `#1c2733` grid seams
  - Player ("Operator"): `#39ff9d` (neon green) core with `#0a4d33` outline
  - Access keys: `#ffd23f` (amber) with soft pulsing glow (respect reduced motion)
  - Security drones: `#ff3b5c` (neon red/magenta) with `#4a0f1c` outline
  - Uplink terminal (goal): `#38b6ff` (neon cyan), pulses faster once all keys collected
  - HUD panel: `#0d1117` background, `#39ff9d` text accents, `#ff3b5c` for warnings
- **Rendering approach**: HTML5 `<canvas>` with simple filled rectangles/pixel blocks drawn at a
  fixed low-resolution grid (e.g., 16x16 logical pixels scaled up with `image-rendering: pixelated`)
  to achieve an authentic chunky pixel-art look without any external image assets. All visuals are
  procedurally drawn shapes (rects, small pixel sprites built from small on/off bitmaps in code) —
  no downloaded or copied art files.
- **CRT accent layer**: a thin CSS overlay (scanline gradient + subtle vignette) applied via
  `::after` pseudo-element with `background: repeating-linear-gradient(...)` at low opacity
  (≤6%) so it reads as flavor, not noise, and never reduces text contrast below AA.

## 2. HUD (heads-up display)

Persistent panel (semantic, not canvas-drawn) above or beside the play field, built from real DOM
elements so it's screen-reader accessible:

- **Keys**: `Keys: X / 3` (text, updates live via `aria-live="polite"` region)
- **Score**: `Score: N`
- **State**: one of `Idle`, `Playing`, `Paused`, `Won`, `Lost` — shown as a visible text badge,
  not color-only, with distinct wording per state.
- **Controls reminder**: always-visible text line: `Arrows/WASD move · P pause · R restart`
- Implemented as `<div role="status">` elements or a `<dl>` for label/value pairs — no reliance on
  color alone to convey meaning (state also has distinct text).

## 3. Controls

- **Movement**: Arrow keys (Up/Down/Left/Right) and WASD, both always active simultaneously.
- **Pause/Resume**: `P` key toggles Paused ⇄ Playing. Also exposed as a real `<button>` for
  mouse/touch/AT users (keyboard-focusable, labelled "Pause").
- **Restart**: `R` key restarts from Idle/Won/Lost. Also exposed as a real `<button>` labelled
  "Restart".
- **Start**: Any movement key or an explicit `<button>` labelled "Start" transitions Idle → Playing.
- All interactive controls are real `<button>` elements (not `<div onclick>`), naturally reachable
  via Tab, with visible focus rings (see §5).
- Key handling must call `preventDefault()` only for the specific keys used, so page scroll via
  arrow keys is suppressed during play but nothing else is broken.

## 4. Layout & responsiveness

- Root layout: a single-column flex/grid container: HUD panel on top (or left on wide screens),
  canvas play field below/beside, instructions text beneath.
- **Desktop**: play field canvas sized generously (e.g., up to 640×480 logical, scaled), HUD can
  sit beside the field in a row layout (`flex-direction: row` above a breakpoint, e.g. 700px).
- **Narrow / 390px width** (required breakpoint): layout switches to a single column
  (`flex-direction: column`), canvas scales down via CSS (`max-width: 100%; height: auto;`) while
  preserving aspect ratio, HUD text wraps normally. No element may exceed viewport width — use
  `box-sizing: border-box`, avoid fixed pixel widths larger than 390px on any container, and set
  `overflow-x: hidden` only as a safety net (primary fix is correct fluid sizing, not clipping).
- Font sizes use `rem`/`clamp()` so text remains legible without overflowing at 390px.

## 5. Accessibility

- **Focus visibility**: all buttons and any focusable control get a clearly visible outline
  (e.g., `outline: 3px solid #38b6ff; outline-offset: 2px;` on `:focus-visible`), never
  `outline: none` without replacement.
- **Semantic controls**: real `<button>` elements for Start/Pause/Restart; canvas is supplemented
  by an `aria-hidden` state or a text alternative describing current state for screen readers via
  the HUD live region (canvas itself is decorative/game-visual; the DOM HUD carries the meaning).
- **Instructions**: a visible, non-hidden instructions block (`<section aria-label="How to play">`)
  listing objective, controls, and drone/key mechanics in plain text — present before and during
  play, not just in a tooltip.
- **Contrast**: body text and HUD text must meet at least WCAG AA (4.5:1) against their
  backgrounds — verify amber/green/cyan text choices above against the `#0a0e14`/`#0d1117`
  backgrounds (all chosen values were picked to comfortably clear 4.5:1; integration should spot
  check with a contrast calculation before shipping).
- **Color independence**: state and hazard information (drones vs keys vs goal) must be
  distinguishable by shape/label, not color alone — drones and keys should have distinct pixel
  silhouettes (e.g., drone = diamond/eye shape, key = key-tooth shape) in addition to differing
  colors.
- **Reduced motion**: wrap all non-essential animation (glow pulsing, CRT scanline drift, any
  continuous idle-state flourish) in a check against
  `window.matchMedia('(prefers-reduced-motion: reduce)')` / the CSS equivalent
  `@media (prefers-reduced-motion: reduce)`. When active: disable pulsing/glow animations and
  scanline drift, but keep drones/player moving at normal deterministic gameplay speed (essential
  motion for play must remain functional — only decorative motion is suppressed).

## 6. State visuals

| State    | Visual treatment                                                             |
|----------|-------------------------------------------------------------------------------|
| Idle     | Dim floor, player centered, "Press any move key or Start" prompt in HUD       |
| Playing  | Full color, drones/keys animated, HUD live counters updating                  |
| Paused   | Canvas dims via semi-transparent overlay + "PAUSED" text badge; input frozen  |
| Won      | Uplink terminal flashes cyan, "ACCESS GRANTED" badge, drones freeze           |
| Lost     | Screen briefly flashes red overlay, "CONNECTION TERMINATED" badge, freeze     |

Each state must be reflected in the HUD's text status (not only in canvas visuals) so it is
perceivable without color vision or without seeing the canvas.

## 7. Handoff notes for integration

- This lane defines direction only; no game code, HTML, or assets were produced here — the
  integration task should implement rendering/CSS/HUD per the above, drawing from the gameplay
  and verification lanes' artifacts for exact grid size, tick rate, and entity coordinates.
- Suggested CSS variable names for integration to adopt for consistency:
  `--nf-bg`, `--nf-floor`, `--nf-player`, `--nf-key`, `--nf-drone`, `--nf-goal`, `--nf-hud-bg`,
  `--nf-focus`.
- No external fonts, images, audio, or network requests are to be introduced; use system font
  stack (e.g., `ui-monospace, "Courier New", monospace`) for a terminal/CRT feel.
