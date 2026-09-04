type AgentSpriteProps = {
  agentId: string
}

type HairStyle = 'bob' | 'crop' | 'side-part' | 'curls' | 'undercut' | 'locs'
type OutfitStyle = 'blazer' | 'polo' | 'overshirt' | 'blouse' | 'utility' | 'sweater'

type Persona = {
  skin: string
  skinShadow: string
  hair: string
  hairLight: string
  jacket: string
  jacketShadow: string
  shirt: string
  trousers: string
  shoes: string
  hairStyle: HairStyle
  outfitStyle: OutfitStyle
  glasses?: boolean
  headset?: boolean
}

const PERSONAS: Persona[] = [
  {
    skin: '#a95f3f',
    skinShadow: '#7a3f31',
    hair: '#21141d',
    hairLight: '#3b2330',
    jacket: '#7f2948',
    jacketShadow: '#4e1830',
    shirt: '#f1dfc7',
    trousers: '#302b42',
    shoes: '#110d18',
    hairStyle: 'bob',
    outfitStyle: 'blazer',
  },
  {
    skin: '#70412f',
    skinShadow: '#4c291f',
    hair: '#17131a',
    hairLight: '#342b32',
    jacket: '#315e4b',
    jacketShadow: '#1d3b31',
    shirt: '#c7d7b2',
    trousers: '#2a3037',
    shoes: '#11151a',
    hairStyle: 'crop',
    outfitStyle: 'polo',
    headset: true,
  },
  {
    skin: '#d79a71',
    skinShadow: '#a9644f',
    hair: '#bd8740',
    hairLight: '#e0b766',
    jacket: '#294760',
    jacketShadow: '#192d43',
    shirt: '#e9e1cf',
    trousers: '#2a3347',
    shoes: '#15151d',
    hairStyle: 'side-part',
    outfitStyle: 'overshirt',
  },
  {
    skin: '#bd704f',
    skinShadow: '#8f4838',
    hair: '#6f273d',
    hairLight: '#a54253',
    jacket: '#147d7a',
    jacketShadow: '#0c4f53',
    shirt: '#d8ede2',
    trousers: '#313543',
    shoes: '#15151d',
    hairStyle: 'curls',
    outfitStyle: 'blouse',
    headset: true,
  },
  {
    skin: '#e0ad82',
    skinShadow: '#aa7459',
    hair: '#15141d',
    hairLight: '#383449',
    jacket: '#424552',
    jacketShadow: '#252833',
    shirt: '#8f61bf',
    trousers: '#252a38',
    shoes: '#101118',
    hairStyle: 'undercut',
    outfitStyle: 'utility',
    glasses: true,
  },
  {
    skin: '#5d382d',
    skinShadow: '#3e251f',
    hair: '#1b1519',
    hairLight: '#3a2931',
    jacket: '#c48a25',
    jacketShadow: '#805719',
    shirt: '#f1d990',
    trousers: '#1e4f72',
    shoes: '#11151b',
    hairStyle: 'locs',
    outfitStyle: 'sweater',
    headset: true,
  },
]

const KNOWN_PERSONAS: Record<string, number> = {
  '00000000-0000-4000-8000-000000000031': 0,
  '00000000-0000-4000-8000-000000000032': 1,
  '00000000-0000-4000-8000-000000000033': 2,
  '00000000-0000-4000-8000-000000000034': 3,
  '00000000-0000-4000-8000-000000000035': 4,
  '00000000-0000-4000-8000-000000000036': 5,
}

function hashText(value: string): number {
  let hash = 2166136261
  for (const character of value) {
    hash ^= character.charCodeAt(0)
    hash = Math.imul(hash, 16777619)
  }
  return hash >>> 0
}

function personaFor(agentId: string): Persona {
  const normalized = agentId.trim().toLowerCase()
  const known = KNOWN_PERSONAS[normalized]
  if (known !== undefined) return PERSONAS[known]

  const skin = PERSONAS[hashText(`${normalized}:skin`) % PERSONAS.length]
  const hair = PERSONAS[hashText(`${normalized}:hair`) % PERSONAS.length]
  const outfit = PERSONAS[hashText(`${normalized}:outfit`) % PERSONAS.length]
  const traits = hashText(`${normalized}:traits`)
  return {
    skin: skin.skin,
    skinShadow: skin.skinShadow,
    hair: hair.hair,
    hairLight: hair.hairLight,
    jacket: outfit.jacket,
    jacketShadow: outfit.jacketShadow,
    shirt: outfit.shirt,
    trousers: outfit.trousers,
    shoes: outfit.shoes,
    hairStyle: PERSONAS[hashText(`${normalized}:hair-style`) % PERSONAS.length].hairStyle,
    outfitStyle: PERSONAS[hashText(`${normalized}:outfit-style`) % PERSONAS.length].outfitStyle,
    glasses: (traits & 1) !== 0,
    headset: (traits & 2) !== 0,
  }
}

function Hair({ persona }: { persona: Persona }) {
  const common = { fill: persona.hair, stroke: '#09070d', strokeWidth: 2 }
  switch (persona.hairStyle) {
    case 'bob':
      return (
        <g>
          <path {...common} d="M17 20V10l5-6h21l5 6v25h-7V18H23v17h-7z" />
          <path fill={persona.hairLight} d="M22 8h16v4H22zM41 12h4v10h-4z" />
        </g>
      )
    case 'crop':
      return (
        <g>
          <path {...common} d="M18 14V9l5-5h19l5 5v8l-6-4H24z" />
          <path fill={persona.hairLight} d="M23 7h18v3H23z" />
        </g>
      )
    case 'side-part':
      return (
        <g>
          <path {...common} d="M17 18V9l6-5h20l5 7-3 8-5-7-22 6z" />
          <path fill={persona.hairLight} d="M24 7h17l3 4-19 2z" />
        </g>
      )
    case 'curls':
      return (
        <g fill={persona.hair} stroke="#09070d" strokeWidth="2">
          <rect x="15" y="10" width="8" height="12" />
          <rect x="20" y="4" width="10" height="11" />
          <rect x="28" y="2" width="11" height="11" />
          <rect x="37" y="5" width="10" height="12" />
          <rect x="42" y="12" width="7" height="15" />
          <path fill={persona.hairLight} stroke="none" d="M23 7h7v4h-7zM36 7h7v4h-7z" />
        </g>
      )
    case 'undercut':
      return (
        <g>
          <path {...common} d="M18 17V9l6-5h20l4 6-12 1-12 7z" />
          <path fill={persona.hairLight} d="M24 7h18v3H30z" />
          <path fill="#51495c" d="M18 15h5v8h-5z" />
        </g>
      )
    case 'locs':
      return (
        <g>
          <path {...common} d="M17 17V9l6-5h18l6 5v10H17z" />
          <path fill={persona.hair} d="M15 13h6v25h-6zM20 15h5v27h-5zM42 13h6v24h-6zM38 16h5v27h-5z" />
          <path fill={persona.hairLight} d="M22 7h18v4H22z" />
        </g>
      )
  }
}

function Outfit({ persona }: { persona: Persona }) {
  const jacket = persona.jacket
  const shadow = persona.jacketShadow
  switch (persona.outfitStyle) {
    case 'blazer':
      return (
        <g>
          <path fill={jacket} stroke="#09070d" strokeWidth="2" d="M16 42l8-8h17l8 8-3 27H18z" />
          <path fill={shadow} d="M24 35l8 13-6 8-5-18zM41 35l-9 13 6 8 6-18z" />
          <path fill={persona.shirt} d="M28 35h9l-5 13z" />
          <path className="sprite-accent-fill" d="M31 39h3l2 18-4 5-4-5z" />
        </g>
      )
    case 'polo':
      return (
        <g>
          <path fill={jacket} stroke="#09070d" strokeWidth="2" d="M16 42l8-7h17l8 7-3 27H18z" />
          <path fill={shadow} d="M18 57h28v12H18z" />
          <path fill={persona.shirt} d="M26 35h12l-3 8h-6z" />
          <path className="sprite-accent-fill" d="M31 43h3v10h-3z" />
        </g>
      )
    case 'overshirt':
      return (
        <g>
          <path fill={jacket} stroke="#09070d" strokeWidth="2" d="M15 42l9-7h17l9 7-4 27H18z" />
          <path fill={persona.shirt} d="M27 35h10v31H27z" />
          <path fill={shadow} d="M18 56h9v13h-9zM37 56h9v13h-9z" />
          <path className="sprite-accent-fill" d="M20 43h5v3h-5zM39 43h5v3h-5z" />
        </g>
      )
    case 'blouse':
      return (
        <g>
          <path fill={jacket} stroke="#09070d" strokeWidth="2" d="M17 42l8-7h15l8 7-2 27H18z" />
          <path fill={persona.shirt} d="M27 35h10l-2 9h-6z" />
          <path fill={shadow} d="M19 58h26v11H19z" />
          <path className="sprite-accent-fill" d="M30 46h4v15h-4z" />
        </g>
      )
    case 'utility':
      return (
        <g>
          <path fill={jacket} stroke="#09070d" strokeWidth="2" d="M15 41l9-7h17l9 7-4 28H18z" />
          <path fill={persona.shirt} d="M27 35h10v31H27z" />
          <path fill={shadow} d="M18 51h8v9h-8zM38 51h8v9h-8z" />
          <path className="sprite-accent-fill" d="M29 42h6v4h-6z" />
        </g>
      )
    case 'sweater':
      return (
        <g>
          <path fill={jacket} stroke="#09070d" strokeWidth="2" d="M16 42l8-7h17l8 7-3 27H18z" />
          <path fill={shadow} d="M18 59h28v10H18z" />
          <path fill={persona.shirt} d="M26 35h12v6H26z" />
          <path className="sprite-accent-fill" d="M20 48h24v4H20z" />
        </g>
      )
  }
}

export function AgentSprite({ agentId }: AgentSpriteProps) {
  const persona = personaFor(agentId)
  return (
    <svg
      className="sprite-person office-character-art"
      viewBox="0 0 64 88"
      role="presentation"
      focusable="false"
      shapeRendering="crispEdges"
    >
      <g className="sprite-character">
        <g className="character-leg character-leg-left">
          <path fill={persona.trousers} stroke="#09070d" strokeWidth="2" d="M19 64h12v17H17l2-17z" />
          <path fill={persona.shoes} stroke="#09070d" strokeWidth="2" d="M15 78h17v7H14z" />
        </g>
        <g className="character-leg character-leg-right">
          <path fill={persona.trousers} stroke="#09070d" strokeWidth="2" d="M33 64h12l2 17H33z" />
          <path fill={persona.shoes} stroke="#09070d" strokeWidth="2" d="M33 78h17l1 7H33z" />
        </g>
        <g className="character-arm character-arm-left">
          <path fill={persona.skinShadow} stroke="#09070d" strokeWidth="2" d="M14 43h8v22l-5 6-5-4z" />
          <path fill={persona.skin} d="M14 45h5v18l-3 3-2-2z" />
        </g>
        <g className="character-arm character-arm-right">
          <path fill={persona.skinShadow} stroke="#09070d" strokeWidth="2" d="M42 43h8l2 24-5 4-5-6z" />
          <path fill={persona.skin} d="M45 45h5v19l-2 2-3-3z" />
        </g>
        <Outfit persona={persona} />
        <path fill={persona.skinShadow} d="M27 29h10v9H27z" />
        <path fill={persona.skin} stroke="#09070d" strokeWidth="2" d="M18 13l5-7h18l6 7v15l-7 8H24l-6-8z" />
        <path fill={persona.skinShadow} d="M39 14h6v13l-6 7z" />
        <path fill={persona.skin} stroke="#09070d" strokeWidth="2" d="M15 18h5v10h-5zM45 18h5v10h-5z" />
        <Hair persona={persona} />
        <path fill="#09070d" d="M24 20h4v3h-4zM37 20h4v3h-4z" />
        <path fill="#fff3ca" d="M25 20h2v1h-2zM38 20h2v1h-2z" />
        <path fill={persona.skinShadow} d="M31 22h3v6h-4z" />
        <path fill="#6b2938" d="M28 30h9v2h-9z" />
        {persona.glasses ? (
          <path fill="none" stroke="#09070d" strokeWidth="2" d="M21 18h10v7H21zM34 18h10v7H34zM31 21h3" />
        ) : null}
        {persona.headset ? (
          <g>
            <path fill="none" stroke="#09070d" strokeWidth="2" d="M18 16c0-9 28-9 28 0" />
            <path className="sprite-accent-fill" stroke="#09070d" strokeWidth="2" d="M14 18h5v10h-5zM45 18h5v10h-5z" />
            <path fill="#09070d" d="M46 27h7v3h-7z" />
          </g>
        ) : null}
        <path className="sprite-accent-fill" stroke="#09070d" strokeWidth="1" d="M39 49h5v7h-5z" />
        <path fill="#fff3ca" d="M40 50h3v2h-3z" />
      </g>
    </svg>
  )
}
