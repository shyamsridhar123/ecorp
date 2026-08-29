const server = process.env.CRONY_SERVER_HTTP ?? "http://127.0.0.1:8791";

async function request(path, body) {
  const response = await fetch(`${server}${path}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  const payload = await response.json();
  return { status: response.status, ok: response.ok, payload };
}

async function post(path, body) {
  const result = await request(path, body);
  if (!result.ok) {
    throw new Error(`${result.status}: ${JSON.stringify(result.payload)}`);
  }
  return result.payload;
}

async function snapshot(corpId, actorId) {
  const response = await fetch(
    `${server}/api/corps/${corpId}/snapshot?actor_id=${actorId}`,
  );
  if (!response.ok) throw new Error(`snapshot failed: ${response.status}`);
  return response.json();
}

async function waitForRun(corpId, actorId, runId, terminal = false) {
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    const current = await snapshot(corpId, actorId);
    const run = current.snapshot.runs.find((candidate) => candidate.id === runId);
    if (
      run &&
      (terminal
        ? ["completed", "failed", "cancelled"].includes(run.status)
        : ["running", "waiting_for_input", "waiting_for_approval", "verifying"].includes(
            run.status,
          ))
    ) {
      return { run, current };
    }
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 50));
  }
  throw new Error(`run ${runId} did not reach the expected state`);
}

const demo = await post("/api/demo/reset", {});
const leasePath = `/api/corps/${demo.corp_id}/agents/${demo.worker_agent_id}/lease`;

const [aliceAttempt, bobAttempt] = await Promise.all([
  post(leasePath, { actor_id: demo.alice_actor_id }),
  post(leasePath, { actor_id: demo.bob_actor_id }),
]);
const attempts = [
  { name: "Alice", actorId: demo.alice_actor_id, result: aliceAttempt },
  { name: "Bob", actorId: demo.bob_actor_id, result: bobAttempt },
];
const winners = attempts.filter((attempt) => attempt.result.acquired);
if (winners.length !== 1) {
  throw new Error(`concurrent lease claim produced ${winners.length} winners`);
}
const winner = winners[0];
const loser = attempts.find((attempt) => attempt.actorId !== winner.actorId);

const renewed = await post(leasePath, { actor_id: winner.actorId });
if (!renewed.acquired || renewed.token === winner.result.token) {
  throw new Error("lease renewal did not rotate the fencing token");
}

const stale = await request(
  `/api/corps/${demo.corp_id}/agents/${demo.worker_agent_id}/messages`,
  {
    actor_id: winner.actorId,
    lease_token: winner.result.token,
    text: "This stale command must be rejected.",
  },
);
if (stale.status !== 409) {
  throw new Error(`stale fencing token returned ${stale.status}, expected 409`);
}

await post(`${leasePath}/release`, {
  actor_id: winner.actorId,
  token: renewed.token,
});
const loserClaim = await post(leasePath, { actor_id: loser.actorId });
if (!loserClaim.acquired) {
  throw new Error("lease release did not let the waiting operator claim control");
}

const transferDemo = await post("/api/demo/reset", {});
const transferLeasePath = `/api/corps/${transferDemo.corp_id}/agents/${transferDemo.worker_agent_id}/lease`;
const aliceLease = await post(transferLeasePath, {
  actor_id: transferDemo.alice_actor_id,
});
const leaseSnapshot = await snapshot(
  transferDemo.corp_id,
  transferDemo.alice_actor_id,
);
if (JSON.stringify(leaseSnapshot).includes(aliceLease.token)) {
  throw new Error("lease fencing token leaked into a shared snapshot or event");
}
const transferred = await post(`${transferLeasePath}/transfer`, {
  actor_id: transferDemo.alice_actor_id,
  token: aliceLease.token,
  to_actor_id: transferDemo.bob_actor_id,
});
if (
  transferred.holder_actor_id !== transferDemo.bob_actor_id ||
  transferred.token !== null
) {
  throw new Error("explicit lease transfer exposed a recipient fencing token");
}

const aliceAfterTransfer = await request(
  `/api/corps/${transferDemo.corp_id}/agents/${transferDemo.worker_agent_id}/messages`,
  {
    actor_id: transferDemo.alice_actor_id,
    lease_token: aliceLease.token,
    text: "Alice no longer controls this agent.",
  },
);
if (aliceAfterTransfer.status !== 409) {
  throw new Error("transferred-away controller was not fenced out");
}
const bobLease = await post(transferLeasePath, {
  actor_id: transferDemo.bob_actor_id,
});
if (!bobLease.acquired || !bobLease.token) {
  throw new Error("transferred controller could not claim a fresh private token");
}
const bobQueued = await post(
  `/api/corps/${transferDemo.corp_id}/agents/${transferDemo.worker_agent_id}/messages`,
  {
    actor_id: transferDemo.bob_actor_id,
    lease_token: bobLease.token,
    text: "Bob now holds the valid fencing token.",
  },
);
if (bobQueued.delivery !== "queued") {
  throw new Error(`expected a valid idle message to queue, got ${bobQueued.delivery}`);
}

const stopDemo = await post("/api/demo/reset", {});
await post(
  `/api/corps/${stopDemo.corp_id}/agents/${stopDemo.worker_agent_id}/lease`,
  { actor_id: stopDemo.alice_actor_id },
);
const mission = await post(`/api/corps/${stopDemo.corp_id}/missions`, {
  requested_by: stopDemo.alice_actor_id,
  title: "Run until an authorized operator exercises the emergency stop.",
});
const launch = await post(
  `/api/corps/${stopDemo.corp_id}/missions/${mission.mission_id}/launch`,
  { requested_by: stopDemo.alice_actor_id },
);
await waitForRun(
  stopDemo.corp_id,
  stopDemo.alice_actor_id,
  launch.run_id,
  false,
);

const forbiddenStop = await request(
  `/api/corps/${stopDemo.corp_id}/agents/${stopDemo.worker_agent_id}/emergency-stop`,
  {
    actor_id: stopDemo.bob_actor_id,
    reason: "Reviewer roles must not stop a run.",
  },
);
if (forbiddenStop.status !== 403) {
  throw new Error(`unauthorized emergency stop returned ${forbiddenStop.status}`);
}

await post(
  `/api/corps/${stopDemo.corp_id}/agents/${stopDemo.worker_agent_id}/emergency-stop`,
  {
    actor_id: stopDemo.alice_actor_id,
    reason: "Owner verified emergency-stop behavior.",
  },
);
const stopped = await waitForRun(
  stopDemo.corp_id,
  stopDemo.alice_actor_id,
  launch.run_id,
  true,
);
if (stopped.run.status !== "cancelled") {
  throw new Error(`emergency stop ended as ${stopped.run.status}`);
}
const stopEvents = stopped.current.snapshot.events.map((event) => event.type);
for (const required of ["run.stop_requested", "run.cancelled"]) {
  if (!stopEvents.includes(required)) {
    throw new Error(`missing emergency-stop event ${required}`);
  }
}

console.log(
  JSON.stringify(
    {
      concurrent_winner: winner.name,
      concurrent_loser: loser.name,
      renewal_rotated_token: renewed.token !== winner.result.token,
      stale_token_status: stale.status,
      release_then_claim: loserClaim.acquired,
      transfer_holder: transferred.holder_actor_id,
      shared_snapshot_hides_token: true,
      transferred_controller_fenced: aliceAfterTransfer.status === 409,
      unauthorized_stop_status: forbiddenStop.status,
      emergency_stop_run_status: stopped.run.status,
    },
    null,
    2,
  ),
);
