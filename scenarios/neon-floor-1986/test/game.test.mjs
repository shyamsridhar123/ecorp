import test from "node:test";
import assert from "node:assert/strict";

import {
  CONFIG,
  PHASES,
  TICK_MS,
  createGame,
  normalizeDirection,
  reduceGame,
  step,
} from "../game.mjs";

function playing(overrides = {}) {
  return {
    ...reduceGame(createGame(), { type: "START" }),
    ...overrides,
  };
}

function tick(state, intent = null) {
  return reduceGame(state, { type: "TICK", intent });
}

function withoutDrones(state) {
  return { ...state, drones: [] };
}

function before(point, state = playing()) {
  return {
    ...state,
    player: { x: point.x - 1, y: point.y },
  };
}

function collect(state, keyIndex) {
  return tick(before(state.keys[keyIndex], state), "right");
}

function terminalState(phase) {
  if (phase === PHASES.IDLE) return createGame();
  if (phase === PHASES.PLAYING) return playing();
  if (phase === PHASES.PAUSED) {
    return reduceGame(playing(), { type: "TOGGLE_PAUSE" });
  }
  if (phase === PHASES.WON) {
    const state = before(CONFIG.uplink, playing());
    state.keys = state.keys.map((key) => ({ ...key, collected: true }));
    state.keysCollected = 3;
    return tick(state, "right");
  }
  const state = playing({
    player: { x: 1, y: 5 },
  });
  return tick(state, "right");
}

test("movement changes one grid tile in all four directions", () => {
  const base = withoutDrones(playing({ player: { x: 4, y: 4 } }));
  assert.deepEqual(tick(base, "right").player, { x: 5, y: 4 });
  assert.deepEqual(tick(base, "left").player, { x: 3, y: 4 });
  assert.deepEqual(tick(base, "up").player, { x: 4, y: 3 });
  assert.deepEqual(tick(base, "down").player, { x: 4, y: 5 });
});

test("movement stays inside walkable bounds and walls", () => {
  for (const direction of ["up", "down", "left", "right"]) {
    let state = withoutDrones(playing());
    for (let index = 0; index < 500; index += 1) state = tick(state, direction);
    assert.ok(state.player.x >= CONFIG.board.minX && state.player.x <= CONFIG.board.maxX);
    assert.ok(state.player.y >= CONFIG.board.minY && state.player.y <= CONFIG.board.maxY);
    assert.ok(Number.isFinite(state.player.x));
    assert.ok(Number.isFinite(state.player.y));
  }

  const againstDesk = withoutDrones(playing({ player: { x: 4, y: 2 } }));
  assert.deepEqual(tick(againstDesk, "right").player, againstDesk.player);
});

test("WASD and arrow input normalize to the same movement", () => {
  const base = withoutDrones(playing({ player: { x: 4, y: 4 } }));
  assert.deepEqual(
    step(base, "KeyW", TICK_MS).player,
    step(base, "ArrowUp", TICK_MS).player,
  );
  assert.equal(normalizeDirection("d"), normalizeDirection("ArrowRight"));
});

test("opposing or diagonal input resolves to at most one grid move", () => {
  const base = withoutDrones(playing({ player: { x: 4, y: 4 } }));
  assert.equal(normalizeDirection(["left", "right"]), null);
  const moved = step(base, ["right", "up"], TICK_MS);
  assert.ok(Math.hypot(moved.player.x - 4, moved.player.y - 4) <= 1);
});

test("zero or invalid elapsed time is inert and huge time is substepped", () => {
  const base = withoutDrones(playing({ player: { x: 4, y: 2 } }));
  assert.equal(step(base, "right", 0), base);
  assert.equal(step(base, "right", Number.NaN), base);
  const afterHugeStep = step(base, "right", 5000);
  assert.ok(afterHugeStep.player.x < 5, "desk wall prevents tunneling");
  assert.ok(Number.isFinite(afterHugeStep.elapsedMs));
});

test("entering an occupied drone tile loses immediately", () => {
  const state = playing({ player: { x: 1, y: 5 } });
  const lost = tick(state, "right");
  assert.equal(lost.phase, PHASES.LOST);
  assert.equal(lost.status, PHASES.LOST);
  assert.equal(lost.result.droneId, "scanner-a");
});

test("an adjacent near-miss remains playable", () => {
  const state = playing({ player: { x: 1, y: 4 } });
  const next = tick(state, null);
  assert.equal(next.phase, PHASES.PLAYING);
});

test("large elapsed time checks each fixed collision substep", () => {
  const state = playing({ player: { x: 1, y: 5 } });
  assert.equal(step(state, "right", 5000).phase, PHASES.LOST);
});

test("collision checks are inert outside playing", () => {
  for (const phase of [PHASES.IDLE, PHASES.PAUSED, PHASES.WON, PHASES.LOST]) {
    const state = terminalState(phase);
    const overlapped = {
      ...state,
      player: { x: state.drones[0].x, y: state.drones[0].y },
    };
    assert.equal(step(overlapped, null, TICK_MS).phase, phase);
  }
});

test("collecting a key increments the counter and score once", () => {
  let state = collect(playing(), 0);
  assert.equal(state.keys[0].collected, true);
  assert.equal(state.keysCollected, 1);
  assert.equal(state.score, CONFIG.scoring.key);

  const score = state.score;
  state = tick(before(state.keys[0], state), "right");
  assert.equal(state.keysCollected, 1);
  assert.equal(state.score, score);
});

test("all three keys are collectible in any order with equal key score", () => {
  function collectOrder(order) {
    let state = playing();
    for (const index of order) state = collect(state, index);
    return state;
  }

  const canonical = collectOrder([0, 1, 2]);
  const shuffled = collectOrder([2, 0, 1]);
  assert.equal(canonical.keysCollected, 3);
  assert.ok(canonical.keys.every((key) => key.collected));
  assert.equal(canonical.score, CONFIG.scoring.key * 3);
  assert.equal(shuffled.keysCollected, canonical.keysCollected);
  assert.equal(shuffled.score, canonical.score);
});

test("uplink wins only after all keys and terminal win stays frozen", () => {
  let state = before(CONFIG.uplink, playing());
  state.keys = state.keys.map((key) => ({ ...key, collected: true }));
  state.keysCollected = 3;
  const won = tick(state, "right");
  assert.equal(won.phase, PHASES.WON);
  assert.equal(won.result.kind, "won");

  const after = step(won, "left", TICK_MS * 10);
  assert.equal(after, won);
  assert.deepEqual(after.player, won.player);
  assert.equal(after.score, won.score);
});

test("uplink remains locked and explains missing keys", () => {
  const state = before(CONFIG.uplink, playing());
  const next = tick(state, "right");
  assert.equal(next.phase, PHASES.PLAYING);
  assert.equal(next.hint, "uplink-locked");
});

test("loss is terminal and freezes the operator", () => {
  const lost = tick(playing({ player: { x: 1, y: 5 } }), "right");
  const after = step(lost, "down", TICK_MS * 10);
  assert.equal(after, lost);
  assert.equal(after.phase, PHASES.LOST);
  assert.deepEqual(after.player, lost.player);
});

test("pause toggles, freezes the world, and resumes progress", () => {
  let state = collect(playing(), 0);
  const score = state.score;
  const keysCollected = state.keysCollected;
  state = reduceGame(state, { type: "TOGGLE_PAUSE" });
  assert.equal(state.phase, PHASES.PAUSED);

  const snapshot = structuredClone(state);
  for (let index = 0; index < 100; index += 1) {
    state = step(state, "right", 16);
  }
  assert.deepEqual(state, snapshot);

  state = reduceGame(state, { type: "TOGGLE_PAUSE" });
  assert.equal(state.phase, PHASES.PLAYING);
  assert.equal(state.score, score);
  assert.equal(state.keysCollected, keysCollected);
});

test("pause is a no-op from idle, won, and lost", () => {
  for (const phase of [PHASES.IDLE, PHASES.WON, PHASES.LOST]) {
    const state = terminalState(phase);
    assert.equal(reduceGame(state, { type: "TOGGLE_PAUSE" }), state);
  }
});

test("restart from every phase returns a fresh independent idle state", () => {
  for (const phase of Object.values(PHASES)) {
    const restarted = reduceGame(terminalState(phase), { type: "RESTART" });
    assert.deepEqual(restarted, createGame());
  }

  const restarted = reduceGame(playing(), { type: "RESTART" });
  const separate = createGame();
  restarted.keys[0].collected = true;
  assert.equal(separate.keys[0].collected, false);
});

test("fixed input scripts and drone routes are deterministic", () => {
  function run(ticks) {
    let state = playing();
    const script = ["right", "up", null, "left", "down", null];
    for (let index = 0; index < ticks && state.phase === PHASES.PLAYING; index += 1) {
      state = tick(state, script[index % script.length]);
    }
    return state;
  }

  assert.deepEqual(run(600), run(600));
  assert.deepEqual(run(300).drones, run(300).drones);
});

test("unknown input and malformed elapsed time never corrupt numeric state", () => {
  const state = playing();
  assert.equal(normalizeDirection("F13"), null);
  assert.equal(normalizeDirection(""), null);
  assert.equal(normalizeDirection(undefined), null);
  assert.doesNotThrow(() => step(state, "F13", TICK_MS));

  const next = step(state, null, -1);
  for (const value of [
    next.tick,
    next.elapsedMs,
    next.score,
    next.player.x,
    next.player.y,
    ...next.drones.flatMap((drone) => [drone.x, drone.y, drone.routeIndex]),
  ]) {
    assert.ok(Number.isFinite(value));
  }
});

test("collision takes precedence over key collection on the same tick", () => {
  const state = playing({
    player: { x: 1, y: 5 },
    keys: [
      { id: "overlap", x: 2, y: 5, collected: false },
      ...createGame().keys.slice(1),
    ],
  });
  const lost = tick(state, "right");
  assert.equal(lost.phase, PHASES.LOST);
  assert.equal(lost.keys[0].collected, false);
  assert.equal(lost.score, 0);
});
