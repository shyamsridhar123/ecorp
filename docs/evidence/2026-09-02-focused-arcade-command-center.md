# Focused sprite-arcade command center

**Latest validation:** September 3, 2026

**Issue:** #93

**Base:** `5999a4c808867796671dbafe02ccd8af69a74445`

**Scope:** `apps/web/**` plus the user-journey documentation

## Problem reproduced

At `1280x900`, the merged UI rendered all five primary subsystems at once:

- visible panels: Factory, Control Floor, Missions, Comms, and Audit
- visible full mission cards: 7
- document height: 4,338 pixels

The palette and sprite office were present, but the workspace still behaved as one long wall of
controls and records.

## Design decision

The redesign uses the taste-skill audit discipline: preserve the information architecture and
authority boundaries, retire repeated card walls, keep one dark theme and one primary cyan accent,
use square geometry consistently, and make every motion communicate state or navigation.

The resulting 1980s arcade command center changes the workspace presentation:

- one active cabinet view at a time
- compact desktop cartridge rail, horizontal mobile rail, and persistent operational HUD
- Control Floor as the default full playfield
- truthful idle and active agent sprites inside the office instead of a decorative background
- one Factory issue queue and one selected workbench dossier
- one Missions cartridge list and one selected quest dossier
- mission briefing, budget authority, task graph, and verification evidence disclosed on demand
- stable hashes and section IDs for Control Floor, Factory, Missions, Comms, and Audit
- structured Comms links route to the owning cabinet and mission
- task deep links open both the task-graph dossier and matching task record
- complete factory failure text is rendered in the selected dossier

The office remains a projection of authoritative state. No server, runner, task, approval,
publication, or authorization behavior changed.

## Local browser evidence

The live Rust server reported healthy development mode with one connected runner. The redesign ran
through a separate Vite process against that real server.

Desktop `1280x900`:

| View | Visible primary panels | Document height |
| --- | ---: | ---: |
| Control Floor | 1 | 924 px |
| Factory | 1 | 1,008 px |
| Missions | 1 | 1,086 px |
| Comms | 1 | 947 px |
| Audit | 1 | 1,114 px |

Additional desktop checks:

- Factory displayed 10 queue entries and exactly 1 full workbench dossier.
- Missions displayed 8 cartridge entries and exactly 1 full mission dossier.
- A failed factory item exposed its complete 70-character diagnostic in the workbench.
- A task deep link selected the correct mission and opened both enclosing `<details>` records.
- Hash navigation selected the matching view.
- Every view had `scrollWidth === clientWidth`.
- No unexpected console or page errors were observed.

Mobile `390x844`:

- each of the five views became the only visible primary panel when selected
- every view had `scrollWidth === clientWidth === 390`
- the horizontally scrollable cabinet navigation remained inside the document width
- every cabinet and guide target measured 54 pixels high
- the start guide opened and closed from the mobile navigation
- no unexpected console or page errors were observed

Screenshots:

- `output/arcade-command-center-v2-floor.png`
- `output/arcade-command-center-v2-factory.png`
- `output/arcade-command-center-v2-missions.png`
- `output/arcade-command-center-v2-mobile-floor.png`

## Local gates

```text
pnpm check
git diff --check
```

`pnpm check` passed with 30 immutable migrations, formatting, clippy with warnings denied, 99 Rust
tests, the production web build, and web lint. All validation is local. Hosted GitHub Actions are
not used, and the change does not enable auto-merge.
