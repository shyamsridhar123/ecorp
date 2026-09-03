# Focused sprite-arcade command center

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

The redesign keeps the dark 1980s arcade language and changes the workspace information
architecture:

- one active cabinet view at a time
- persistent HUD for live runs, pending decisions, active factory items, and verified results
- Control Floor as the default view
- compact Factory cards with explicit dossiers
- one selected mission record with a horizontal mission selector
- mission briefing, budget authority, task graph, and verification evidence disclosed on demand
- stable hashes and section IDs for Control Floor, Factory, Missions, Comms, and Audit

The office remains a projection of authoritative state. No server, runner, task, approval,
publication, or authorization behavior changed.

## Local browser evidence

The live Rust server reported healthy development mode with one connected runner. The redesign ran
through a separate Vite process against that real server.

Desktop `1280x900`:

| View | Visible primary panels | Document height |
| --- | ---: | ---: |
| Control Floor | 1 | 1,063 px |
| Factory | 1 | 1,053 px |
| Missions | 1 | 1,205 px |
| Comms | 1 | 1,043 px |
| Audit | 1 | 1,174 px |

Additional desktop checks:

- Factory displayed 7 compact work items with zero dossiers open by default.
- Missions displayed 7 selector entries and exactly 1 full mission card.
- Opening a Factory dossier and the selected mission briefing worked.
- Hash navigation selected the matching view.
- Every view had `scrollWidth === clientWidth`.
- No unexpected console or page errors were observed.

Mobile `390x844`:

- each of the five views became the only visible primary panel when selected
- every view had `scrollWidth === clientWidth === 390`
- the horizontally scrollable cabinet navigation remained inside the document width
- navigation, guide, and HUD controls measured at least 48 pixels high
- the start guide opened and closed from the mobile navigation
- no unexpected console or page errors were observed

Screenshots:

- `output/arcade-command-center-desktop.png`
- `output/arcade-command-center-factory.png`
- `output/arcade-command-center-missions.png`
- `output/arcade-command-center-mobile-floor.png`
- `output/arcade-command-center-mobile-missions.png`

## Local gates

```text
pnpm build:web
pnpm lint:web
git diff --check
```

All passed locally. Hosted GitHub Actions were not used, and the change does not enable
auto-merge.
