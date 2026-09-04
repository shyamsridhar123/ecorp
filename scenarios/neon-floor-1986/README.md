# Neon Floor 1986

An original, compact 1980s-style office infiltration game built with plain HTML, CSS, Canvas, and
JavaScript. It uses no external assets, fonts, analytics, network calls, packages, or runtime
dependencies.

## Run

Open `index.html` directly in a modern browser. No server or build step is required.

## Objective

Collect all three access keys, avoid both deterministic security drones, then enter the cyan
uplink terminal. A collision or tile swap with a drone ends the run.

## Controls

- **Move:** Arrow keys or WASD
- **Start:** Start button, Enter, Space, or any movement key
- **Pause / resume:** P or the Pause button
- **Restart:** R or the Restart button

The HUD always reports state, keys, score, uplink readiness, and controls. Decorative CRT motion is
disabled when `prefers-reduced-motion: reduce` is active; gameplay timing is unchanged.

## Test

From the repository root:

```powershell
node --test scenarios/neon-floor-1986/test/game.test.mjs
```

The deterministic engine lives in `game.js` with an ES-module bridge in `game.mjs`; browser
rendering and input handling live in `app.js`.
