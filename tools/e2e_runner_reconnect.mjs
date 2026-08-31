import { writeFile } from "node:fs/promises";
import path from "node:path";

const server = process.env.CRONY_SERVER_HTTP ?? "http://127.0.0.1:8791";
const root = path.resolve(import.meta.dirname, "..");
const runnerId = process.env.CRONY_E2E_RUNNER_ID ?? "runner-local";

async function request(path, body) {
  const response = await fetch(`${server}${path}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  const payload = await response.json();
  if (!response.ok) {
    throw new Error(`${response.status}: ${JSON.stringify(payload)}`);
  }
  return payload;
}

async function snapshot(corpId, actorId) {
  const response = await fetch(
    `${server}/api/corps/${corpId}/snapshot?actor_id=${actorId}`,
  );
  const payload = await response.json();
  if (!response.ok) {
    throw new Error(`${response.status}: ${JSON.stringify(payload)}`);
  }
  return payload;
}

async function waitFor(label, fn, timeoutMs = 25_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const result = await fn();
    if (result) return result;
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 75));
  }
  throw new Error(`timed out waiting for ${label}`);
}

async function createSlowRun(demo, title) {
  const mission = await request(`/api/corps/${demo.corp_id}/missions`, {
    requested_by: demo.alice_actor_id,
    title: `[slow] ${title}`,
  });
  const launch = await request(
    `/api/corps/${demo.corp_id}/missions/${mission.mission_id}/launch`,
    { requested_by: demo.alice_actor_id },
  );
  await waitFor("slow run to start", async () => {
    const current = await snapshot(demo.corp_id, demo.alice_actor_id);
    const run = current.snapshot.runs.find((candidate) => candidate.id === launch.run_id);
    return run?.status === "running" ? run : null;
  });
  return launch;
}

const shortDemo = await request("/api/demo/reset", {});
const initial = await snapshot(shortDemo.corp_id, shortDemo.alice_actor_id);
const initialRunner = initial.runners.find((runner) => runner.id === runnerId);
if (!initialRunner || initialRunner.status !== "connected" || !initialRunner.last_seen_at) {
  throw new Error("runner heartbeat record was not persisted");
}
const initialHeartbeat = Date.parse(initialRunner.last_seen_at);
const shortRun = await createSlowRun(
  shortDemo,
  "survive a transient runner transport disconnect.",
);
await request(`/api/demo/runners/${runnerId}/disconnect`, {
  reconnect_delay_ms: 1_000,
});
const graceObserved = await waitFor("runner grace state", async () => {
  const current = await snapshot(shortDemo.corp_id, shortDemo.alice_actor_id);
  return current.runners.find((runner) => runner.id === runnerId)?.status === "grace"
    ? current
    : null;
});
const reconnected = await waitFor("runner reconnect", async () => {
  const current = await snapshot(shortDemo.corp_id, shortDemo.alice_actor_id);
  const runner = current.runners.find((candidate) => candidate.id === runnerId);
  const reconciled = current.snapshot.events.some(
    (event) => event.type === "run.reconciled" && event.aggregate_id === shortRun.run_id,
  );
  return runner?.status === "connected" && reconciled ? current : null;
});
const completed = await waitFor("reconciled run completion", async () => {
  const current = await snapshot(shortDemo.corp_id, shortDemo.alice_actor_id);
  const run = current.snapshot.runs.find((candidate) => candidate.id === shortRun.run_id);
  return run?.status === "completed" ? current : null;
}, 30_000);
if (
  completed.snapshot.events.some(
    (event) => event.type === "run.lost" && event.aggregate_id === shortRun.run_id,
  )
) {
  throw new Error("transient disconnect incorrectly marked the active run lost");
}

const longDemo = await request("/api/demo/reset", {});
const lostRun = await createSlowRun(
  longDemo,
  "become lost after the runner exceeds its grace period.",
);
await request(`/api/demo/runners/${runnerId}/disconnect`, {
  reconnect_delay_ms: 8_000,
});
await waitFor("runner long-disconnect grace state", async () => {
  const current = await snapshot(longDemo.corp_id, longDemo.alice_actor_id);
  return current.runners.find((runner) => runner.id === runnerId)?.status === "grace"
    ? current
    : null;
});
const lost = await waitFor("run loss after grace expiry", async () => {
  const current = await snapshot(longDemo.corp_id, longDemo.alice_actor_id);
  const run = current.snapshot.runs.find((candidate) => candidate.id === lostRun.run_id);
  return run?.status === "lost" ? current : null;
}, 15_000);
const final = await waitFor("runner reconnect after lost run", async () => {
  const current = await snapshot(longDemo.corp_id, longDemo.alice_actor_id);
  const runner = current.runners.find((candidate) => candidate.id === runnerId);
  return runner?.status === "connected" ? current : null;
}, 15_000);
const finalRun = final.snapshot.runs.find((candidate) => candidate.id === lostRun.run_id);
if (finalRun?.status !== "lost") {
  throw new Error(`stale runner claim changed lost run to ${finalRun?.status}`);
}
for (const required of ["runner.grace_started", "run.lost"]) {
  if (
    !lost.snapshot.events.some(
      (event) => event.type === required && event.aggregate_id === lostRun.run_id,
    )
  ) {
    throw new Error(`missing runner lifecycle event ${required}`);
  }
}

await new Promise((resolvePromise) => setTimeout(resolvePromise, 10_500));
const heartbeatView = await snapshot(longDemo.corp_id, longDemo.alice_actor_id);
const finalRunner = heartbeatView.runners.find((runner) => runner.id === runnerId);
if (!finalRunner || Date.parse(finalRunner.last_seen_at) <= initialHeartbeat) {
  throw new Error("runner heartbeat timestamp did not advance after reconnect");
}

const report = {
      initial_runner_status: initialRunner.status,
      grace_observed: Boolean(graceObserved),
      short_reconnect_status: reconnected.runners.find(
        (runner) => runner.id === runnerId,
      )?.status,
      short_run_status: completed.snapshot.runs.find(
        (run) => run.id === shortRun.run_id,
      )?.status,
      reconciliation_event: true,
      lost_run_status: finalRun.status,
      stale_claim_preserved_lost_state: true,
      heartbeat_advanced: true,
};
await writeFile(
  path.join(root, "output", "e2e-runner-reconnect.json"),
  `${JSON.stringify(report, null, 2)}\n`,
);
console.log(JSON.stringify(report, null, 2));

