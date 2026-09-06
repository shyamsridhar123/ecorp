# ECorp office character assets

This packet contains **six unchanged character PNG sheets only**. It does not contain
furniture, floors, walls, carpets, pets, screenshots, or any paid LimeZu pack.

## Source and licenses

- **Original character art:** JIK-A-4, *MetroCity — Free TopDown Character Pack*.
  The [creator's asset page](https://jik-a-4.itch.io/metrocity-free-topdown-character-pack)
  explicitly labels its asset license **Creative Commons Zero v1.0 Universal**.
  This was checked on **September 6, 2026**. The CC0 legal text is preserved in
  [`licenses/CC0-1.0.txt`](licenses/CC0-1.0.txt).
- **Bundled/adapted sheets:** [Pixel Agents](https://github.com/pixel-agents-hq/pixel-agents),
  distributed by that project under the MIT license,
  **Copyright (c) 2026 Pablo De Lucca**. The project's
  [pinned README](https://github.com/pixel-agents-hq/pixel-agents/blob/3537e140c2094761beae748592aeb92ece8edfdd/README.md)
  explicitly credits JIK-A-4's MetroCity pack for its six characters.
  The complete upstream notice is preserved in
  [`licenses/MIT-Pixel-Agents.txt`](licenses/MIT-Pixel-Agents.txt).

CC0 describes the original art; MIT describes this upstream project's distribution
and adaptations. These are separate provenance layers, not a claim that the original
artist relicensed their CC0 pack exclusively as MIT. Retain both notices when
redistributing this packet.

**Pinned source commit:** `3537e140c2094761beae748592aeb92ece8edfdd`
(`2026-08-15T21:33:45Z`).

**Upstream files:** `webview-ui/public/assets/characters/char_0.png` through
`char_5.png`, copied into this directory's `characters/` folder.
No image generation, recoloring, resampling, or sprite-sheet editing was performed.
`manifest.json` records every PNG's SHA-256, upstream Git blob SHA-1, size, and source path.

The current upstream README documents both a VS Code extension and a standalone CLI
and says that multiple art categories are bundled. This character-only packet does
**not** rely on older paid-furniture handoffs or make a reuse claim for other categories.
ECorp's parent UI work supplies its own furniture.

## Sheet and animation contract

All six transparent RGBA sheets are **112 × 96 native pixels**:
**7 columns × 3 rows**, each frame **16 × 32 pixels**.

| Direction | Row | Transform |
|---|---:|---|
| down | 0 | none |
| up | 1 | none |
| right | 2 | none |
| left | 2 | horizontal mirror of right |

| Animation | Column sequence | Step duration |
|---|---|---:|
| idle | `1` | still |
| walk | `0, 1, 2, 1` | 150 ms |
| type | `3, 4` | 300 ms |
| read | `5, 6` | 300 ms |

Rows are **directions**, not animation names. Reading/typing are actual bundled
frames, and the fourth walking step reuses column 1.

Frame geometry comes from
[`core/src/assets/constants.ts`](https://github.com/pixel-agents-hq/pixel-agents/blob/3537e140c2094761beae748592aeb92ece8edfdd/core/src/assets/constants.ts).
Frame selection and mirroring come from
[`spriteData.ts`](https://github.com/pixel-agents-hq/pixel-agents/blob/3537e140c2094761beae748592aeb92ece8edfdd/webview-ui/src/office/sprites/spriteData.ts)
and [`characters.ts`](https://github.com/pixel-agents-hq/pixel-agents/blob/3537e140c2094761beae748592aeb92ece8edfdd/webview-ui/src/office/engine/characters.ts).
Timing, native walking speed (48 pixels/second), and the sitting offset (6 pixels)
come from
[`webview-ui/src/constants.ts`](https://github.com/pixel-agents-hq/pixel-agents/blob/3537e140c2094761beae748592aeb92ece8edfdd/webview-ui/src/constants.ts).

## ECorp integration

`characterAssets.ts` exports:

- `OFFICE_CHARACTER_ASSETS`: six source URLs, labels, sizes, and checksums.
- `OFFICE_CHARACTER_SHEET`, `OFFICE_CHARACTER_DIRECTIONS`,
  `OFFICE_CHARACTER_ANIMATIONS`: geometry and timing.
- `getOfficeCharacterAsset(agentIdOrIndex)`: deterministic appearance selection.
- `getOfficeCharacterFrame(animation, direction, elapsedMs)`: Canvas source rectangle
  `{ sx, sy, sw, sh, flipX }`, plus row/column/frame indices.

Animation names are `idle | walk | type | read`; directions are
`down | up | left | right`. The time argument is milliseconds **within the current
animation**, not a frame number.

Use integer destination pixels and integer scale factors. Disable Canvas image
smoothing (or use CSS `image-rendering: pixelated`). Mirror only the selected frame,
not the entire sheet. The provided `{ x: 8, y: 32 }` anchor is a bottom-center cell
coordinate for the parent renderer; layout, occlusion, sitting placement, and live
activity interpretation remain parent-owned.

`VALIDATION.json` records local asset/metadata checks, not a browser-to-server
acceptance claim for the office UI.
