# Believable office character sprites — updated Friday, September 4, 2026

## Design decision

The control floor keeps a restrained retro management-sim identity without making the operational
UI look or read like an arcade cabinet. ECorp renders six original, higher-detail SVG pixel
characters with consistent proportions, camera angle, grounding, and explicit status cues.

The cast is deliberately distinct:

- Margo: burgundy manager blazer and lanyard
- Wally: forest operations polo and headset
- Cody: navy engineering overshirt
- Claudia: teal blouse, curls, and headset
- Opal: charcoal utility jacket, undercut, and glasses
- Piper: mustard sweater, tied-back locs, and headset

Known demo identities have stable authored personas. Unknown future agents derive skin, hair,
clothing, silhouette, glasses, and headset traits independently from the immutable agent ID, so
renaming an agent does not change their appearance and the fallback population is not limited to
six repeated combinations.

## Asset research and licensing

Two external directions were evaluated before implementation:

- The CC0 OpenGameArt `2D Top down office characters` set is safely redistributable, but its
  abstract circular top-down figures do not match ECorp's management-sim perspective or desired
  human detail.
- LimeZu's `Modern Interiors` material is visually closer, but its published terms prohibit
  redistributing the asset pack. Committing those sprites to a public source repository would not
  satisfy ECorp's contributor and redistribution needs.

No third-party character artwork was copied into the repository. The shipped SVG geometry and
personas are original project source.

The status treatment was also compared with
[`pixel-agents-hq/pixel-agents`](https://github.com/pixel-agents-hq/pixel-agents). ECorp adopts the
useful behavioral principle, not its assets: idle characters remain still, confirmed work receives
a small activity animation, and waiting states use explicit labels. ECorp intentionally omits
wandering because authoritative operational state should not be obscured by decorative movement.

## Implementation

- `apps/web/src/AgentSprite.tsx` owns the six persona definitions, deterministic identity mapping,
  detailed SVG geometry, outfits, hair, skin tones, glasses, headsets, and accent integration.
- `apps/web/src/App.tsx` reuses the same sprite component on the floor and in the selected-agent
  inspector, keeps every character at a stable desk position, and prints the authoritative state
  directly under each name: `Idle`, `Starting`, `Working`, `Review ready`, `Needs approval`, or
  `Offline`.
- `apps/web/src/Accessible.css` provides the light retro-surveillance product skin, readable system
  typography, semantic status colors, 44-pixel controls, visible focus treatment, responsive
  layouts, and a two-frame arm cue only for `Working`.
- Reduced-motion mode disables the working cue. Idle, starting, reviewing, blocked, and offline
  characters do not bob, wander, or move between stations.

## Local verification

Observed locally against the complete Vite, ECorp server, and runner stack:

- `pnpm check`: passed, including migrations, Rust formatting, clippy, workspace tests, web build,
  and web lint.
- Desktop Chromium: `scrollWidth=clientWidth=1513`; six floor sprites expose visible `Idle` labels;
  the computed idle animation is `none`.
- Mobile Chromium at `390x844`: `scrollWidth=clientWidth=390`; all visible controls are at least
  44 pixels high; navigation forms a complete three-by-two grid.
- A clean in-app browser reported no console warnings or errors.
- Keyboard traversal begins with the skip link, operator selector, five workspace links, and guide
  button; every checked target had a visible solid focus outline.
- A transient browser-only working-state fixture verified the `Working` label, activity bubble, and
  `ecorp-type-left` / `ecorp-type-right` arm animations. Reload restored authoritative server state.
- Sample text contrast ratios were 18.58:1 for primary text, 7.50:1 for secondary text, 9.95:1 for
  the red accent, and 5.41:1 for the working-state green against white.

These are local observations. Hosted GitHub Actions did not run because the account has no
remaining Actions credits for the current month.
