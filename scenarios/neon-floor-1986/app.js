(function startNeonFloorApp() {
  "use strict";

  if (globalThis.NeonFloorApp) return;

  const engine = globalThis.NeonFloorGame;
  if (!engine) {
    throw new Error("Neon Floor engine failed to load.");
  }

  const canvas = document.querySelector("#game-canvas");
  const context = canvas.getContext("2d", { alpha: false });
  const stateValue = document.querySelector("#state-value");
  const keysValue = document.querySelector("#keys-value");
  const scoreValue = document.querySelector("#score-value");
  const uplinkValue = document.querySelector("#uplink-value");
  const liveStatus = document.querySelector("#live-status");
  const startButton = document.querySelector("#start-button");
  const pauseButton = document.querySelector("#pause-button");
  const restartButton = document.querySelector("#restart-button");

  const LOGICAL_WIDTH = 720;
  const LOGICAL_HEIGHT = 480;
  const TILE = 40;
  const MAX_FRAME_TICKS = 4;
  const DIRECTION_CODES = Object.freeze({
    ArrowUp: "up",
    ArrowDown: "down",
    ArrowLeft: "left",
    ArrowRight: "right",
    KeyW: "up",
    KeyS: "down",
    KeyA: "left",
    KeyD: "right",
  });
  const USED_CODES = new Set([
    ...Object.keys(DIRECTION_CODES),
    "Enter",
    "Space",
    "KeyP",
    "KeyR",
  ]);

  let state = engine.createGame();
  let heldDirections = [];
  let queuedDirection = null;
  let accumulator = 0;
  let lastFrameTime = performance.now();
  let needsRender = true;
  let resizeQueued = false;
  let hudSignature = "";
  let lastAnnouncedPhase = state.phase;
  let lastAnnouncedKeys = state.keysCollected;
  let lastHint = state.hint;

  function titleCase(value) {
    return value.charAt(0).toUpperCase() + value.slice(1);
  }

  function currentIntent() {
    return heldDirections.at(-1) ?? queuedDirection;
  }

  function clearInput() {
    heldDirections = [];
    queuedDirection = null;
  }

  function announceChanges(previous, next) {
    if (next.phase !== lastAnnouncedPhase) {
      const messages = {
        idle: "Idle. Press Start, Enter, Space, or a movement key.",
        playing:
          previous.phase === engine.PHASES.PAUSED
            ? "Simulation resumed."
            : "Infiltration active. Recover all three access keys.",
        paused: "Paused. Security drones and the operator are frozen.",
        won: `Access granted. Uplink reached with ${next.score} points.`,
        lost: "Connection terminated. A security drone intercepted the operator.",
      };
      liveStatus.textContent = messages[next.phase];
      lastAnnouncedPhase = next.phase;
    } else if (next.keysCollected !== lastAnnouncedKeys) {
      liveStatus.textContent =
        `Access key recovered. ${next.keysCollected} of ${next.keys.length}. ` +
        `Score ${next.score}.`;
    } else if (next.hint === "uplink-locked" && lastHint !== next.hint) {
      liveStatus.textContent = "Uplink locked. Recover all three access keys before extraction.";
    }

    lastAnnouncedKeys = next.keysCollected;
    lastHint = next.hint;
  }

  function setState(next) {
    if (next === state) return;
    const previous = state;
    state = next;
    announceChanges(previous, next);
    needsRender = true;
  }

  function startGame() {
    if (state.phase === engine.PHASES.IDLE) {
      accumulator = 0;
      setState(engine.reduceGame(state, { type: "START" }));
    }
  }

  function togglePause() {
    if (
      state.phase !== engine.PHASES.PLAYING &&
      state.phase !== engine.PHASES.PAUSED
    ) {
      return;
    }
    clearInput();
    accumulator = 0;
    setState(engine.reduceGame(state, { type: "TOGGLE_PAUSE" }));
  }

  function restartGame() {
    clearInput();
    accumulator = 0;
    setState(engine.reduceGame(state, { type: "RESTART" }));
  }

  function addHeldDirection(direction) {
    heldDirections = heldDirections.filter((item) => item !== direction);
    heldDirections.push(direction);
    queuedDirection = direction;
  }

  function removeHeldDirection(direction) {
    heldDirections = heldDirections.filter((item) => item !== direction);
  }

  function onKeyDown(event) {
    if (!USED_CODES.has(event.code)) return;
    event.preventDefault();

    const direction = DIRECTION_CODES[event.code];
    if (direction) {
      if (state.phase === engine.PHASES.IDLE) startGame();
      if (state.phase === engine.PHASES.PLAYING && !event.repeat) {
        addHeldDirection(direction);
      }
      return;
    }

    if (event.repeat) return;
    if ((event.code === "Enter" || event.code === "Space") && state.phase === engine.PHASES.IDLE) {
      startGame();
    } else if (event.code === "KeyP") {
      togglePause();
    } else if (event.code === "KeyR") {
      restartGame();
    }
  }

  function onKeyUp(event) {
    const direction = DIRECTION_CODES[event.code];
    if (!direction) return;
    event.preventDefault();
    removeHeldDirection(direction);
  }

  function drawFloorTile(x, y) {
    const px = x * TILE;
    const py = y * TILE;
    context.fillStyle = (x + y) % 2 === 0 ? "#131c24" : "#101820";
    context.fillRect(px, py, TILE, TILE);
    context.strokeStyle = "#263747";
    context.lineWidth = 1;
    context.strokeRect(px + 0.5, py + 0.5, TILE - 1, TILE - 1);
    context.fillStyle = "#26313b";
    context.fillRect(px + 5, py + 5, 2, 2);
    context.fillRect(px + TILE - 7, py + TILE - 7, 2, 2);
  }

  function drawWall(x, y) {
    const px = x * TILE;
    const py = y * TILE;
    context.fillStyle = "#26323c";
    context.fillRect(px, py, TILE, TILE);
    context.fillStyle = "#354856";
    context.fillRect(px + 3, py + 4, TILE - 6, 7);
    context.fillStyle = "#172028";
    context.fillRect(px + 3, py + 14, TILE - 6, TILE - 18);
    context.fillStyle = "#577184";
    context.fillRect(px + 7, py + 18, TILE - 14, 3);
  }

  function drawKey(key) {
    if (key.collected) return;
    const px = key.x * TILE;
    const py = key.y * TILE;
    context.fillStyle = "#704f06";
    context.fillRect(px + 9, py + 10, 17, 17);
    context.fillStyle = "#ffd23f";
    context.fillRect(px + 12, py + 13, 11, 11);
    context.fillRect(px + 21, py + 17, 11, 5);
    context.fillRect(px + 27, py + 21, 5, 6);
    context.fillRect(px + 22, py + 21, 4, 4);
    context.fillStyle = "#fff3a8";
    context.fillRect(px + 15, py + 16, 5, 5);
  }

  function drawUplink(ready) {
    const { x, y } = state.uplink;
    const px = x * TILE;
    const py = y * TILE;
    context.fillStyle = ready ? "#0a5b75" : "#243c48";
    context.fillRect(px + 6, py + 4, 28, 32);
    context.fillStyle = ready ? "#59c7ff" : "#78909b";
    context.fillRect(px + 9, py + 7, 22, 20);
    context.fillStyle = "#07131a";
    context.fillRect(px + 12, py + 10, 16, 14);
    context.fillStyle = ready ? "#eefcff" : "#c9d6da";
    context.font = "bold 12px monospace";
    context.textAlign = "center";
    context.textBaseline = "middle";
    context.fillText("U", px + 20, py + 17);
    context.fillStyle = ready ? "#59c7ff" : "#78909b";
    context.fillRect(px + 12, py + 30, 16, 3);
  }

  function drawDrone(drone) {
    const cx = drone.x * TILE + TILE / 2;
    const cy = drone.y * TILE + TILE / 2;
    context.fillStyle = "#5c1321";
    context.beginPath();
    context.moveTo(cx, cy - 15);
    context.lineTo(cx + 16, cy);
    context.lineTo(cx, cy + 15);
    context.lineTo(cx - 16, cy);
    context.closePath();
    context.fill();
    context.fillStyle = "#ff6078";
    context.fillRect(cx - 10, cy - 6, 20, 12);
    context.fillStyle = "#fff3f5";
    context.fillRect(cx - 3, cy - 3, 6, 6);
    context.fillStyle = "#310810";
    context.fillRect(cx - 1, cy - 2, 3, 4);
    context.fillStyle = "#ff6078";
    context.fillRect(cx - 18, cy - 2, 5, 4);
    context.fillRect(cx + 13, cy - 2, 5, 4);
  }

  function drawPlayer() {
    const px = state.player.x * TILE;
    const py = state.player.y * TILE;
    context.fillStyle = "#075031";
    context.fillRect(px + 10, py + 7, 21, 19);
    context.fillStyle = "#39ff9d";
    context.fillRect(px + 13, py + 10, 15, 13);
    context.fillStyle = "#06120d";
    context.fillRect(px + 16, py + 13, 10, 4);
    context.fillStyle = "#baffdc";
    context.fillRect(px + 22, py + 14, 3, 2);
    context.fillStyle = "#39ff9d";
    context.fillRect(px + 14, py + 26, 5, 8);
    context.fillRect(px + 23, py + 26, 5, 8);
    context.fillStyle = "#075031";
    context.fillRect(px + 7, py + 12, 4, 12);
  }

  function drawOverlay() {
    const labels = {
      idle: ["SYSTEM READY", "PRESS START OR A MOVE KEY"],
      paused: ["SIMULATION PAUSED", "PRESS P OR PAUSE TO RESUME"],
      won: ["ACCESS GRANTED", `FINAL SCORE ${state.score}`],
      lost: ["CONNECTION TERMINATED", "PRESS R OR RESTART"],
    };
    const label = labels[state.phase];
    if (!label) return;

    context.fillStyle =
      state.phase === engine.PHASES.LOST ? "rgb(67 5 17 / 70%)" : "rgb(2 7 10 / 72%)";
    context.fillRect(0, 0, LOGICAL_WIDTH, LOGICAL_HEIGHT);
    context.fillStyle =
      state.phase === engine.PHASES.LOST
        ? "#ffb1bd"
        : state.phase === engine.PHASES.WON
          ? "#9dffca"
          : "#eef7f4";
    context.textAlign = "center";
    context.textBaseline = "middle";
    context.font = "bold 32px monospace";
    context.fillText(label[0], LOGICAL_WIDTH / 2, LOGICAL_HEIGHT / 2 - 14);
    context.fillStyle = "#c6d5d9";
    context.font = "bold 14px monospace";
    context.fillText(label[1], LOGICAL_WIDTH / 2, LOGICAL_HEIGHT / 2 + 24);
  }

  function render() {
    context.save();
    context.setTransform(
      canvas.width / LOGICAL_WIDTH,
      0,
      0,
      canvas.height / LOGICAL_HEIGHT,
      0,
      0,
    );
    context.imageSmoothingEnabled = false;
    context.fillStyle = "#070b10";
    context.fillRect(0, 0, LOGICAL_WIDTH, LOGICAL_HEIGHT);

    for (let y = 0; y < engine.LEVEL.height; y += 1) {
      for (let x = 0; x < engine.LEVEL.width; x += 1) {
        if (engine.LEVEL.rows[y][x] === "#") drawWall(x, y);
        else drawFloorTile(x, y);
      }
    }

    drawUplink(state.keysCollected === state.keys.length);
    for (const key of state.keys) drawKey(key);
    for (const drone of state.drones) drawDrone(drone);
    drawPlayer();
    drawOverlay();
    context.restore();
  }

  function updateHud() {
    const hud = engine.getHudModel(state);
    const signature = [
      hud.phase,
      hud.score,
      hud.keysCollected,
      hud.keysTotal,
      hud.uplinkReady,
    ].join("|");
    if (signature === hudSignature) return;

    hudSignature = signature;
    stateValue.textContent = titleCase(hud.phase);
    stateValue.dataset.phase = hud.phase;
    keysValue.textContent = `${hud.keysCollected} / ${hud.keysTotal}`;
    scoreValue.textContent = String(hud.score).padStart(4, "0");
    uplinkValue.textContent = hud.uplinkReady ? "Ready" : "Locked";
    startButton.disabled = hud.phase !== engine.PHASES.IDLE;
    pauseButton.disabled =
      hud.phase !== engine.PHASES.PLAYING && hud.phase !== engine.PHASES.PAUSED;
    pauseButton.textContent = hud.phase === engine.PHASES.PAUSED ? "Resume" : "Pause";
  }

  function frame(now) {
    const frameDelta = Math.max(0, now - lastFrameTime);
    lastFrameTime = now;

    if (state.phase === engine.PHASES.PLAYING) {
      accumulator += Math.min(frameDelta, engine.TICK_MS * MAX_FRAME_TICKS);
      let processed = 0;
      while (accumulator >= engine.TICK_MS && processed < MAX_FRAME_TICKS) {
        const intent = currentIntent();
        setState(engine.reduceGame(state, { type: "TICK", intent }));
        if (!heldDirections.includes(queuedDirection)) queuedDirection = null;
        accumulator -= engine.TICK_MS;
        processed += 1;
      }
      if (processed === MAX_FRAME_TICKS && accumulator >= engine.TICK_MS) {
        accumulator = 0;
      }
    } else {
      accumulator = 0;
    }

    if (needsRender) {
      updateHud();
      render();
      needsRender = false;
    }
    requestAnimationFrame(frame);
  }

  function resizeCanvas() {
    resizeQueued = false;
    const pixelRatio = Math.min(globalThis.devicePixelRatio || 1, 2);
    const width = Math.round(LOGICAL_WIDTH * pixelRatio);
    const height = Math.round(LOGICAL_HEIGHT * pixelRatio);
    if (canvas.width !== width || canvas.height !== height) {
      canvas.width = width;
      canvas.height = height;
    }
    needsRender = true;
  }

  function queueResize() {
    if (resizeQueued) return;
    resizeQueued = true;
    requestAnimationFrame(resizeCanvas);
  }

  function pauseForInterruption() {
    clearInput();
    if (state.phase === engine.PHASES.PLAYING) togglePause();
  }

  startButton.addEventListener("click", startGame);
  pauseButton.addEventListener("click", togglePause);
  restartButton.addEventListener("click", restartGame);
  document.addEventListener("keydown", onKeyDown);
  document.addEventListener("keyup", onKeyUp);
  window.addEventListener("blur", pauseForInterruption);
  window.addEventListener("resize", queueResize, { passive: true });
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) pauseForInterruption();
  });

  globalThis.NeonFloorApp = Object.freeze({
    getState: () => JSON.parse(JSON.stringify(state)),
  });

  resizeCanvas();
  updateHud();
  render();
  needsRender = false;
  requestAnimationFrame(frame);
})();
