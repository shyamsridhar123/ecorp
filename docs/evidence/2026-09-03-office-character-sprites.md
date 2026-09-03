# Believable office character sprites — Thursday, September 3, 2026

## Design decision

The control floor keeps its arcade management-sim language, but the agents are no longer assembled
from rectangular CSS head, body, arm, and leg blocks. ECorp now renders six original, higher-detail
SVG pixel characters with consistent proportions, camera angle, grounding, and status motion.

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

## Implementation

- `apps/web/src/AgentSprite.tsx` owns the six persona definitions, deterministic identity mapping,
  detailed SVG geometry, outfits, hair, skin tones, glasses, headsets, and accent integration.
- `apps/web/src/App.tsx` reuses the same sprite component on the floor and in the selected-agent
  inspector.
- `apps/web/src/World.css` removes the obsolete block-doll rules and applies continuous idle,
  walking, working, reviewing, and blocked motion to SVG groups. Reduced-motion mode disables those
  animations.

## Local verification

Observed locally against the live Vite client and ECorp server:

- `pnpm build:web`: passed.
- `pnpm lint:web`: passed.
- Desktop Chromium at `1440x1000`: six floor sprites, no horizontal overflow
  (`scrollWidth=innerWidth=1440`), and no console or page errors.
- Mobile Chromium at `390x844`: all six agent hit areas remain inside the viewport,
  `scrollWidth=innerWidth=390`, and no console or page errors.
- Selecting Cody produced one matching character in the inspector and six on the floor, with no
  identity substitution.

The browser screenshots were retained as local review artifacts under `output/playwright/`; they
are not product source or hosted-environment evidence.
