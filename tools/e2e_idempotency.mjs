const server = process.env.CRONY_SERVER_HTTP ?? "http://127.0.0.1:8791";

async function post(path, body) {
  const response = await fetch(`${server}${path}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  const payload = await response.json();
  if (!response.ok) throw new Error(`${response.status}: ${JSON.stringify(payload)}`);
  return payload;
}

async function snapshot(corpId, actorId) {
  const response = await fetch(
    `${server}/api/corps/${corpId}/snapshot?actor_id=${actorId}`,
  );
  const payload = await response.json();
  if (!response.ok) throw new Error(`${response.status}: ${JSON.stringify(payload)}`);
  return payload;
}

const demo = await post("/api/demo/reset", {});
const mission = await post(`/api/corps/${demo.corp_id}/missions`, {
  requested_by: demo.alice_actor_id,
  title: "[duplicate-event] Prove duplicate runner delivery is idempotent.",
});
const launch = await post(
  `/api/corps/${demo.corp_id}/missions/${mission.mission_id}/launch`,
  { requested_by: demo.alice_actor_id },
);

const deadline = Date.now() + 15_000;
let final;
while (Date.now() < deadline) {
  final = await snapshot(demo.corp_id, demo.alice_actor_id);
  const run = final.snapshot.runs.find((candidate) => candidate.id === launch.run_id);
  if (run?.status === "completed") break;
  await new Promise((resolvePromise) => setTimeout(resolvePromise, 100));
}
const run = final?.snapshot.runs.find((candidate) => candidate.id === launch.run_id);
if (run?.status !== "completed") {
  throw new Error(`duplicate-delivery run ended as ${run?.status}`);
}

const started = final.snapshot.events.filter(
  (event) => event.type === "run.started" && event.aggregate_id === launch.run_id,
);
if (started.length !== 1) {
  throw new Error(`expected one persisted run.started event, got ${started.length}`);
}
const ids = final.snapshot.events.map((event) => event.id);
if (new Set(ids).size !== ids.length) {
  throw new Error("snapshot contains duplicate event IDs");
}

console.log(
  JSON.stringify(
    {
      run_id: launch.run_id,
      duplicate_deliveries_sent: 2,
      persisted_run_started_events: started.length,
      unique_event_ids: new Set(ids).size,
      run_status: run.status,
    },
    null,
    2,
  ),
);

