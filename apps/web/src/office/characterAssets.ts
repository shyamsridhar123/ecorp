/**
 * Pixel Agents / Metro City character sheets, copied without pixel or palette changes.
 *
 * Original Metro City art: JIK-A-4, CC0-1.0.
 * Pixel Agents distribution/adaptations: MIT, Copyright (c) 2026 Pablo De Lucca.
 * Full notices and byte-level provenance: /assets/office/pixel-agents/ATTRIBUTION.md
 *
 * This module describes appearance only. The parent office projection owns activity/state.
 */

export const OFFICE_PIXEL_ASSET_ROOT = '/assets/office/pixel-agents'

export const OFFICE_PIXEL_ASSET_SOURCE = {
  repository: 'https://github.com/pixel-agents-hq/pixel-agents',
  commit: '3537e140c2094761beae748592aeb92ece8edfdd',
  originalCharacterArtist: 'JIK-A-4',
  originalCharacterPack: 'https://jik-a-4.itch.io/metrocity-free-topdown-character-pack',
  originalCharacterLicense: 'CC0-1.0',
  upstreamDistributionLicense: 'MIT',
  attributionUrl: `${OFFICE_PIXEL_ASSET_ROOT}/ATTRIBUTION.md`,
  manifestUrl: `${OFFICE_PIXEL_ASSET_ROOT}/manifest.json`,
} as const

export type OfficeCharacterAnimation = 'idle' | 'walk' | 'type' | 'read'
export type OfficeCharacterDirection = 'down' | 'up' | 'left' | 'right'

/** Native source pixels, not CSS pixels. Keep Canvas imageSmoothingEnabled=false. */
export const OFFICE_CHARACTER_SHEET = {
  width: 112,
  height: 96,
  frameWidth: 16,
  frameHeight: 32,
  columns: 7,
  rows: 3,
  tileSize: 16,
  anchor: { x: 8, y: 32 },
  sittingOffsetY: 6,
  walkSpeedPixelsPerSecond: 48,
} as const

/** Rows encode direction; animation frames run across columns within that row. */
export const OFFICE_CHARACTER_DIRECTIONS = {
  down: { row: 0, flipX: false },
  up: { row: 1, flipX: false },
  right: { row: 2, flipX: false },
  left: { row: 2, flipX: true },
} as const

/** Walk uses the upstream 0,1,2,1 cycle, not a fabricated fourth walking frame. */
export const OFFICE_CHARACTER_ANIMATIONS = {
  idle: { columns: [1], frameDurationMs: 0 },
  walk: { columns: [0, 1, 2, 1], frameDurationMs: 150 },
  type: { columns: [3, 4], frameDurationMs: 300 },
  read: { columns: [5, 6], frameDurationMs: 300 },
} as const

export interface OfficeCharacterAsset {
  readonly id: string
  readonly label: string
  readonly src: string
  readonly upstreamPath: string
  readonly gitBlobSha1: string
  readonly sha256: string
  readonly bytes: number
}

export const OFFICE_CHARACTER_ASSETS = [
  {
    id: 'char_0',
    label: 'Brown hair · blue jacket',
    src: `${OFFICE_PIXEL_ASSET_ROOT}/characters/char_0.png`,
    upstreamPath: 'webview-ui/public/assets/characters/char_0.png',
    gitBlobSha1: '68a86c5035b77c0a2f752404080b4bfeb023fa2f',
    sha256: '9ece93c20765abb876e3dacfe01e0cd1801323eff41c1779be0c47369886c09a',
    bytes: 3956,
  },
  {
    id: 'char_1',
    label: 'Copper braids · dark jacket',
    src: `${OFFICE_PIXEL_ASSET_ROOT}/characters/char_1.png`,
    upstreamPath: 'webview-ui/public/assets/characters/char_1.png',
    gitBlobSha1: '13676875af4411e1bc85bc732a16679a82c710e3',
    sha256: 'f935f033726b06f73dbbe590db5a23cfb20d2d02c1329fd5d67d57a262e9b05e',
    bytes: 5022,
  },
  {
    id: 'char_2',
    label: 'Dark hair · warm jacket',
    src: `${OFFICE_PIXEL_ASSET_ROOT}/characters/char_2.png`,
    upstreamPath: 'webview-ui/public/assets/characters/char_2.png',
    gitBlobSha1: 'd07293bd08665ec1b3b4cb91a00022601d06a0fe',
    sha256: '9f63ee7be4577e1d60de3b64d198d387d1c09794bed3a608958e77c7b955575d',
    bytes: 5221,
  },
  {
    id: 'char_3',
    label: 'Silver hair · rose jacket',
    src: `${OFFICE_PIXEL_ASSET_ROOT}/characters/char_3.png`,
    upstreamPath: 'webview-ui/public/assets/characters/char_3.png',
    gitBlobSha1: '89f858c63939e8f1637e0fffef987a84e69bacc4',
    sha256: 'ee5ebe0847471e4834e367f8a4108e6caf2f402ed506bbfebd90c64a43a79348',
    bytes: 4930,
  },
  {
    id: 'char_4',
    label: 'Curly hair · white shirt',
    src: `${OFFICE_PIXEL_ASSET_ROOT}/characters/char_4.png`,
    upstreamPath: 'webview-ui/public/assets/characters/char_4.png',
    gitBlobSha1: '32dbc619a3b1f724cbf8777175efe9fab4d93a57',
    sha256: 'fcf72626bbee3045d0f3e10bb87a41dab19a331d1affdfc53b483567f9c083aa',
    bytes: 5488,
  },
  {
    id: 'char_5',
    label: 'Black hair · red jacket',
    src: `${OFFICE_PIXEL_ASSET_ROOT}/characters/char_5.png`,
    upstreamPath: 'webview-ui/public/assets/characters/char_5.png',
    gitBlobSha1: 'b5cfa27e8c4c95be748111bacfabdd1e149be052',
    sha256: '4b2c639a50e089a4e3b87be440ffb52b5c5c8cfd6473a386f325b25f99db7381',
    bytes: 5071,
  },
] as const satisfies readonly OfficeCharacterAsset[]

/** Stable identity selection; never re-randomize people's appearance on every snapshot. */
export function getOfficeCharacterAsset(key: string | number = 0): OfficeCharacterAsset {
  if (typeof key === 'string') {
    const named = OFFICE_CHARACTER_ASSETS.find((asset) => asset.id === key)
    if (named) return named
  }
  let index = 0
  if (typeof key === 'number') {
    index = Number.isFinite(key) ? Math.trunc(key) : 0
  } else {
    let hash = 2166136261
    for (let i = 0; i < key.length; i += 1) {
      hash = Math.imul(hash ^ key.charCodeAt(i), 16777619)
    }
    index = hash >>> 0
  }
  const length = OFFICE_CHARACTER_ASSETS.length
  return OFFICE_CHARACTER_ASSETS[((index % length) + length) % length]
    ?? OFFICE_CHARACTER_ASSETS[0]
}

export interface OfficeCharacterFrame {
  readonly sx: number
  readonly sy: number
  readonly sw: 16
  readonly sh: 32
  readonly column: number
  readonly row: number
  readonly animationFrame: number
  readonly flipX: boolean
}

/** Pure source rectangle lookup; elapsedMs is time in the current animation, not a frame index. */
export function getOfficeCharacterFrame(
  animation: OfficeCharacterAnimation = 'idle',
  direction: OfficeCharacterDirection = 'down',
  elapsedMs = 0,
): OfficeCharacterFrame {
  const clip = OFFICE_CHARACTER_ANIMATIONS[animation]
  const facing = OFFICE_CHARACTER_DIRECTIONS[direction]
  const time = Number.isFinite(elapsedMs) ? Math.max(0, elapsedMs) : 0
  const animationFrame = clip.frameDurationMs === 0
    ? 0
    : Math.floor(time / clip.frameDurationMs) % clip.columns.length
  const column = clip.columns[animationFrame] ?? clip.columns[0]
  return {
    sx: column * OFFICE_CHARACTER_SHEET.frameWidth,
    sy: facing.row * OFFICE_CHARACTER_SHEET.frameHeight,
    sw: OFFICE_CHARACTER_SHEET.frameWidth,
    sh: OFFICE_CHARACTER_SHEET.frameHeight,
    column,
    row: facing.row,
    animationFrame,
    flipX: facing.flipX,
  }
}
