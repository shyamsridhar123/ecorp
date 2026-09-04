(function attachNeonFloorEngine(root, factory) {
  "use strict";

  const api = factory();
  if (typeof module === "object" && module.exports) {
    module.exports = api;
  }
  if (root) {
    root.NeonFloorGame = api;
  }
})(typeof globalThis === "object" ? globalThis : this, function createNeonFloorEngine() {
  "use strict";

  const TICK_MS = 125;
  const KEY_SCORE = 250;
  const WIN_SCORE = 1000;
  const WIN_TIME_BUDGET = 500;

  const PHASES = Object.freeze({
    IDLE: "idle",
    PLAYING: "playing",
    PAUSED: "paused",
    WON: "won",
    LOST: "lost",
  });

  const DIRECTIONS = Object.freeze({
    up: Object.freeze([0, -1]),
    down: Object.freeze([0, 1]),
    left: Object.freeze([-1, 0]),
    right: Object.freeze([1, 0]),
  });

  const LEVEL_ROWS = Object.freeze([
    "##################",
    "#.......B.......U#",
    "#..K.##...##.....#",
    "#....##...##..K..#",
    "#................#",
    "#.A..............#",
    "#................#",
    "#....##...##.....#",
    "#....##.K.##.....#",
    "#................#",
    "#P...............#",
    "##################",
  ]);

  const KEY_DEFINITIONS = Object.freeze([
    Object.freeze({ id: "key-cyan", x: 3, y: 2 }),
    Object.freeze({ id: "key-amber", x: 14, y: 3 }),
    Object.freeze({ id: "key-magenta", x: 8, y: 8 }),
  ]);

  const DRONE_ROUTES = Object.freeze({
    "scanner-a": Object.freeze([
      [2, 5], [3, 5], [4, 5], [5, 5], [6, 5], [7, 5], [8, 5],
      [9, 5], [10, 5], [11, 5], [12, 5], [13, 5], [14, 5], [15, 5],
      [14, 5], [13, 5], [12, 5], [11, 5], [10, 5], [9, 5], [8, 5],
      [7, 5], [6, 5], [5, 5], [4, 5], [3, 5],
    ].map((point) => Object.freeze(point))),
    "scanner-b": Object.freeze([
      [8, 1], [8, 2], [8, 3], [8, 4], [8, 5], [8, 6], [8, 7],
      [8, 8], [8, 9], [8, 10], [8, 9], [8, 8], [8, 7], [8, 6],
      [8, 5], [8, 4], [8, 3], [8, 2],
    ].map((point) => Object.freeze(point))),
  });

  const LEVEL = Object.freeze({
    width: 18,
    height: 12,
    rows: LEVEL_ROWS,
    playerStart: Object.freeze({ x: 1, y: 10 }),
    keys: KEY_DEFINITIONS,
    uplink: Object.freeze({ x: 16, y: 1 }),
    droneStarts: Object.freeze([
      Object.freeze({ id: "scanner-a", x: 2, y: 5, routeIndex: 0, period: 3 }),
      Object.freeze({ id: "scanner-b", x: 8, y: 1, routeIndex: 0, period: 4 }),
    ]),
  });

  const CONFIG = Object.freeze({
    tickMs: TICK_MS,
    board: Object.freeze({
      width: LEVEL.width,
      height: LEVEL.height,
      minX: 0,
      maxX: LEVEL.width - 1,
      minY: 0,
      maxY: LEVEL.height - 1,
    }),
    playerStart: LEVEL.playerStart,
    keyPositions: KEY_DEFINITIONS,
    uplink: LEVEL.uplink,
    droneRoutes: DRONE_ROUTES,
    scoring: Object.freeze({
      key: KEY_SCORE,
      win: WIN_SCORE,
      timeBudgetTicks: WIN_TIME_BUDGET,
    }),
  });

  const KEY_TO_DIRECTION = Object.freeze({
    ArrowUp: "up",
    ArrowDown: "down",
    ArrowLeft: "left",
    ArrowRight: "right",
    KeyW: "up",
    KeyS: "down",
    KeyA: "left",
    KeyD: "right",
    w: "up",
    W: "up",
    s: "down",
    S: "down",
    a: "left",
    A: "left",
    d: "right",
    D: "right",
  });

  function clonePoint(point) {
    return { x: point.x, y: point.y };
  }

  function countCollected(keys) {
    let count = 0;
    for (const key of keys) {
      if (key.collected) count += 1;
    }
    return count;
  }

  function withDerived(state) {
    const keysCollected = countCollected(state.keys);
    return {
      ...state,
      status: state.phase,
      keysCollected,
      elapsedMs: state.tick * TICK_MS,
    };
  }

  function createInitialState() {
    return withDerived({
      version: 1,
      phase: PHASES.IDLE,
      status: PHASES.IDLE,
      tick: 0,
      elapsedMs: 0,
      stepRemainderMs: 0,
      score: 0,
      player: clonePoint(LEVEL.playerStart),
      keys: KEY_DEFINITIONS.map((key) => ({ ...key, collected: false })),
      keysCollected: 0,
      uplink: clonePoint(LEVEL.uplink),
      drones: LEVEL.droneStarts.map((drone) => ({ ...drone })),
      result: null,
      hint: null,
    });
  }

  function isWalkable(x, y) {
    return (
      Number.isInteger(x) &&
      Number.isInteger(y) &&
      y >= 0 &&
      y < LEVEL.height &&
      x >= 0 &&
      x < LEVEL.width &&
      LEVEL.rows[y][x] !== "#"
    );
  }

  function samePoint(a, b) {
    return a.x === b.x && a.y === b.y;
  }

  function normalizeDirection(input) {
    if (typeof input === "string") {
      if (Object.hasOwn(DIRECTIONS, input)) return input;
      return KEY_TO_DIRECTION[input] ?? null;
    }

    if (Array.isArray(input)) {
      const active = input.map(normalizeDirection).filter(Boolean);
      const hasLeft = active.includes("left");
      const hasRight = active.includes("right");
      const hasUp = active.includes("up");
      const hasDown = active.includes("down");
      const horizontal = hasLeft === hasRight ? null : hasLeft ? "left" : "right";
      const vertical = hasUp === hasDown ? null : hasUp ? "up" : "down";
      if (horizontal && vertical) {
        const preferred = normalizeDirection(input.at(-1));
        return preferred === horizontal || preferred === vertical ? preferred : vertical;
      }
      return horizontal ?? vertical;
    }

    if (input && typeof input === "object") {
      if (input.intent !== undefined) return normalizeDirection(input.intent);
      if (Array.isArray(input.active)) return normalizeDirection(input.active);
      const active = [];
      for (const direction of Object.keys(DIRECTIONS)) {
        if (input[direction]) active.push(direction);
      }
      if (input.lastDirection && input[input.lastDirection]) {
        active.push(input.lastDirection);
      }
      return normalizeDirection(active);
    }

    return null;
  }

  function advanceDrone(drone, nextTick) {
    if (nextTick % drone.period !== 0) return { ...drone };
    const route = DRONE_ROUTES[drone.id];
    const routeIndex = (drone.routeIndex + 1) % route.length;
    const [x, y] = route[routeIndex];
    return { ...drone, routeIndex, x, y };
  }

  function reduceTick(state, rawIntent) {
    const intent = normalizeDirection(rawIntent);
    const nextTick = state.tick + 1;
    const oldPlayer = clonePoint(state.player);
    const nextPlayer = clonePoint(state.player);

    if (intent) {
      const [dx, dy] = DIRECTIONS[intent];
      const targetX = state.player.x + dx;
      const targetY = state.player.y + dy;
      if (isWalkable(targetX, targetY)) {
        nextPlayer.x = targetX;
        nextPlayer.y = targetY;
      }
    }

    const oldDrones = state.drones.map((drone) => ({ ...drone }));
    const nextDrones = state.drones.map((drone) => advanceDrone(drone, nextTick));
    let collision = null;

    for (let index = 0; index < oldDrones.length; index += 1) {
      const oldDrone = oldDrones[index];
      const nextDrone = nextDrones[index];
      const enteredOldTile = samePoint(nextPlayer, oldDrone);
      const swapped =
        enteredOldTile &&
        samePoint(oldPlayer, nextDrone) &&
        !samePoint(oldDrone, nextDrone);

      if (enteredOldTile) {
        collision = {
          droneId: oldDrone.id,
          collision: swapped ? "crossed" : "occupied",
        };
      } else if (samePoint(nextPlayer, nextDrone)) {
        collision = { droneId: oldDrone.id, collision: "landed" };
      }

      if (collision) break;
    }

    if (collision) {
      return withDerived({
        ...state,
        phase: PHASES.LOST,
        tick: nextTick,
        player: nextPlayer,
        drones: nextDrones,
        keys: state.keys.map((key) => ({ ...key })),
        result: {
          kind: "lost",
          tick: nextTick,
          droneId: collision.droneId,
          collision: collision.collision,
        },
        hint: null,
      });
    }

    let score = state.score;
    const keys = state.keys.map((key) => {
      if (!key.collected && samePoint(key, nextPlayer)) {
        score += KEY_SCORE;
        return { ...key, collected: true };
      }
      return { ...key };
    });

    const allKeysCollected = countCollected(keys) === keys.length;
    const atUplink = samePoint(nextPlayer, state.uplink);
    if (allKeysCollected && atUplink) {
      score += WIN_SCORE + Math.max(0, WIN_TIME_BUDGET - nextTick);
      return withDerived({
        ...state,
        phase: PHASES.WON,
        tick: nextTick,
        score,
        player: nextPlayer,
        keys,
        drones: nextDrones,
        result: { kind: "won", tick: nextTick, finalScore: score },
        hint: null,
      });
    }

    return withDerived({
      ...state,
      phase: PHASES.PLAYING,
      tick: nextTick,
      score,
      player: nextPlayer,
      keys,
      drones: nextDrones,
      result: null,
      hint: atUplink && !allKeysCollected ? "uplink-locked" : null,
    });
  }

  function reduceGame(state, action) {
    if (!state || !action || typeof action.type !== "string") return state;

    if (action.type === "RESTART") return createInitialState();

    if (action.type === "START") {
      if (state.phase !== PHASES.IDLE) return state;
      return withDerived({
        ...state,
        phase: PHASES.PLAYING,
        result: null,
        hint: null,
      });
    }

    if (action.type === "TOGGLE_PAUSE") {
      if (state.phase === PHASES.PLAYING) {
        return withDerived({ ...state, phase: PHASES.PAUSED, hint: null });
      }
      if (state.phase === PHASES.PAUSED) {
        return withDerived({ ...state, phase: PHASES.PLAYING, hint: null });
      }
      return state;
    }

    if (action.type === "TICK" && state.phase === PHASES.PLAYING) {
      return reduceTick(state, action.intent);
    }

    return state;
  }

  function step(state, input, dtMs) {
    if (
      !state ||
      state.phase !== PHASES.PLAYING ||
      !Number.isFinite(dtMs) ||
      dtMs <= 0
    ) {
      return state;
    }

    const totalMs = (state.stepRemainderMs || 0) + dtMs;
    const tickCount = Math.min(Math.floor(totalMs / TICK_MS), 80);
    if (tickCount === 0) {
      return withDerived({ ...state, stepRemainderMs: totalMs });
    }

    let next = withDerived({
      ...state,
      stepRemainderMs: totalMs % TICK_MS,
    });
    const intent = normalizeDirection(input);
    for (let index = 0; index < tickCount && next.phase === PHASES.PLAYING; index += 1) {
      next = reduceGame(next, { type: "TICK", intent });
    }
    return next.phase === PHASES.PLAYING
      ? next
      : withDerived({ ...next, stepRemainderMs: 0 });
  }

  function getHudModel(state) {
    const keysCollected = countCollected(state.keys);
    return {
      phase: state.phase,
      score: state.score,
      tick: state.tick,
      keysCollected,
      keysTotal: state.keys.length,
      uplinkReady: keysCollected === state.keys.length,
      hint: state.hint,
    };
  }

  return Object.freeze({
    TICK_MS,
    PHASES,
    DIRECTIONS,
    LEVEL,
    DRONE_ROUTES,
    CONFIG,
    createInitialState,
    createGame: createInitialState,
    reduceGame,
    step,
    normalizeDirection,
    isWalkable,
    getHudModel,
  });
});
