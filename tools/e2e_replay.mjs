const server = process.env.CRONY_SERVER_HTTP ?? "http://127.0.0.1:8791";
const socketBase = server.replace(/^http/, "ws");

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

function collectReplay(corpId, afterSeq) {
  return new Promise((resolvePromise, reject) => {
    const events = [];
    const socket = new WebSocket(
      `${socketBase}/ws/corps/${corpId}?after_seq=${afterSeq}`,
    );
    const timeout = setTimeout(() => {
      socket.close();
      reject(new Error("timed out waiting for replay ready message"));
    }, 10_000);

    socket.onerror = () => {
      clearTimeout(timeout);
      reject(new Error("replay websocket failed"));
    };
    socket.onmessage = (message) => {
      const payload = JSON.parse(message.data);
      if (payload.type === "event") {
        events.push(payload.event);
        return;
      }
      if (payload.type === "ready") {
        clearTimeout(timeout);
        socket.close();
        resolvePromise({
          events,
          replayedThrough: payload.replayed_through,
        });
      }
    };
  });
}

const demo = await post("/api/demo/reset", {});
const first = await collectReplay(demo.corp_id, 0);
if (first.events.length !== 1 || first.events[0].type !== "corp.demo_bootstrapped") {
  throw new Error(`unexpected initial replay: ${JSON.stringify(first)}`);
}

await post(`/api/corps/${demo.corp_id}/missions`, {
  requested_by: demo.alice_actor_id,
  title: "Verify event-stream replay after a disconnected client.",
});

const second = await collectReplay(demo.corp_id, first.replayedThrough);
const types = second.events.map((event) => event.type);
if (types.join(",") !== "mission.created,task.created") {
  throw new Error(`unexpected reconnect replay: ${JSON.stringify(second)}`);
}
if (
  second.events[0].seq <= first.replayedThrough ||
  second.events[1].seq <= second.events[0].seq ||
  second.replayedThrough !== second.events[1].seq
) {
  throw new Error(`replay sequence is not strictly ordered: ${JSON.stringify(second)}`);
}

const empty = await collectReplay(demo.corp_id, second.replayedThrough);
if (empty.events.length !== 0 || empty.replayedThrough !== second.replayedThrough) {
  throw new Error(`duplicate replay was delivered: ${JSON.stringify(empty)}`);
}

console.log(
  JSON.stringify(
    {
      corp_id: demo.corp_id,
      initial_replay_count: first.events.length,
      reconnect_replay_count: second.events.length,
      duplicate_replay_count: empty.events.length,
      replayed_through: second.replayedThrough,
      event_types: types,
    },
    null,
    2,
  ),
);

