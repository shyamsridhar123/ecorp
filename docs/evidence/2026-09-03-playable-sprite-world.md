# Playable sprite-world validation

Date: September 3, 2026

Issue: #107

## Outcome

The web client now presents ECorp as one late-1980s arcade game shell rather than a permanent wall
of framed operational panels.

- Control Floor is the dominant sprite-art world.
- Agent state is represented by position, animation, color, and semantic status signals derived
  from authoritative server state.
- Agent controls open in a dismissible command HUD.
- The five existing cabinets remain Control floor, Factory, Missions, Comms, and Audit.
- A fixed bottom control dock switches cabinets.
- The score rail keeps live runs, decisions, factory work, verified outcomes, runner health, and
  connection health visible.
- Factory, Missions, Comms, and Audit retain their original authority-bearing controls and focused
  drill-down behavior.

The redesign preserved form field order, section IDs, hash navigation, structured deep links,
test IDs, role checks, mission contracts, approval controls, evidence records, and runner
boundaries.

## Design contract

Reading this as a real-time collaborative agent operations room for engineering teams, with a
tactile late-1980s arcade-command-center language: one playable sprite world, focused contextual
HUDs, and progressive disclosure instead of dashboard walls.

- Design variance: 7
- Motion intensity: 6 with a reduced-motion fallback
- Visual density: 4
- Theme: one dark arcade palette
- Geometry: square pixel edges
- Typography: locally bundled Press Start 2P for display text and VT323 for compact terminal text
- Redesign mode: visual overhaul with information architecture and behavior preserved

## Local repository gates

`pnpm check` passed in the isolated worktree.

- 30 immutable migrations validated
- `cargo fmt --check` passed
- `cargo clippy --workspace --all-targets -- -D warnings` passed
- 101 Rust unit and documentation tests passed
- production web build passed
- web lint passed

The production web bundle included the two local font assets and emitted no external font request.

## Browser validation

Chromium exercised the live web client at `1440x1000` and `390x844`.

- exactly one active workspace surface appeared for each of the five cabinets
- desktop document width was 1440 of 1440 pixels
- mobile document width was 390 of 390 pixels
- bottom dock targets measured at least 52 pixels high
- agent HUD close targets measured 44 pixels high
- the primary button contrast ratio measured 14.27:1
- Escape closed the agent HUD
- reduced-motion mode removed sprite animation and transitions
- Factory and Missions remained reachable and visible
- no browser console errors or page errors occurred

## Complete server-runner-browser path

A separate temporary Postgres database, Rust server on port 8792, enrolled runner, isolated runner
workspace, and Vite client on port 5188 were used so the active dark-factory runtime was not
modified.

`tools/e2e_smoke.ps1` completed a real deterministic child-process run:

- mission `8e46943d-f821-4a13-9d0a-92dedd8cdcf9`
- run `bb9c899f-cd0c-4efe-aa06-5ebf59e33bac`
- runner `runner-local`
- run status `completed`
- durable artifact SHA-256 `bea2157905239965de8a4051fac9c8a63b9d95dc57a16919b9a3da4f2d575022`
- 20 persisted events
- Alice acquired the lease
- Bob could not replace the unexpired lease
- the live message reached and was acknowledged by the child process
- the isolated worktree was preserved after producing source evidence

The browser then observed one online runner, opened Wally's command HUD, closed it with Escape,
switched to Missions, and rendered the completed mission and verification evidence without errors.
The temporary processes and database were removed afterward.

## Visual evidence

`docs/assets/ecorp-control-room.png` was recaptured from a live temporary stack with the start guide
closed. It shows the final desktop Control Floor with a live connection and one enrolled runner.

Hosted GitHub Actions were not used. No auto-merge, merge, or deployment was enabled.
