import engine from "./game.js";

export const {
  TICK_MS,
  PHASES,
  DIRECTIONS,
  LEVEL,
  DRONE_ROUTES,
  CONFIG,
  createInitialState,
  createGame,
  reduceGame,
  step,
  normalizeDirection,
  isWalkable,
  getHudModel,
} = engine;

export default engine;
