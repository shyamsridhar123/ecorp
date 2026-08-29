const server = process.env.CRONY_SERVER_HTTP ?? "http://127.0.0.1:8791";
const socketBase = server.replace(/^http/, "ws");

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
  if (!result.ok) throw new Error(`${result.status}: ${JSON.stringify(result.payload)}`);
  return result.payload;
}

async function snapshot(corpId, actorId) {
  const response = await fetch(
    `${server}/api/corps/${corpId}/snapshot?actor_id=${actorId}`,
  );
  const payload = await response.json();
  if (!response.ok) throw new Error(`${response.status}: ${JSON.stringify(payload)}`);
  return payload;
}

function replay(corpId, actorId) {
  return new Promise((resolvePromise, reject) => {
    const events = [];
    const socket = new WebSocket(
      `${socketBase}/ws/corps/${corpId}?actor_id=${actorId}&after_seq=0`,
    );
    const timeout = setTimeout(() => {
      socket.close();
      reject(new Error("timed out waiting for room replay"));
    }, 10_000);
    socket.onerror = () => {
      clearTimeout(timeout);
      reject(new Error("room replay websocket failed"));
    };
    socket.onmessage = (message) => {
      const payload = JSON.parse(message.data);
      if (payload.type === "event") {
        events.push(payload.event);
      } else if (payload.type === "ready") {
        clearTimeout(timeout);
        socket.close();
        resolvePromise(events);
      }
    };
  });
}

const demo = await post("/api/demo/reset", {});
const mission = await post(`/api/corps/${demo.corp_id}/missions`, {
  requested_by: demo.alice_actor_id,
  title: "Discuss a linked room task with a threaded reply.",
});
const roomPath = `/api/corps/${demo.corp_id}/rooms/${demo.room_id}/messages`;
const root = await post(roomPath, {
  actor_id: demo.alice_actor_id,
  body: "@Bob please review the linked implementation task.",
  reply_to_id: null,
  mentions: [demo.bob_actor_id],
  link: { kind: "task", id: mission.task_id },
});
const reply = await post(roomPath, {
  actor_id: demo.bob_actor_id,
  body: "@Alice reviewed. The acceptance contract is clear.",
  reply_to_id: root.message_id,
  mentions: [demo.alice_actor_id],
  link: { kind: "mission", id: mission.mission_id },
});

const alice = await snapshot(demo.corp_id, demo.alice_actor_id);
const bob = await snapshot(demo.corp_id, demo.bob_actor_id);
for (const view of [alice, bob]) {
  if (view.snapshot.room_messages.length !== 2) {
    throw new Error("room member did not receive both durable messages");
  }
  const rootMessage = view.snapshot.room_messages.find(
    (message) => message.id === root.message_id,
  );
  const replyMessage = view.snapshot.room_messages.find(
    (message) => message.id === reply.message_id,
  );
  if (
    !rootMessage ||
    rootMessage.actor_id !== demo.alice_actor_id ||
    rootMessage.mentions[0] !== demo.bob_actor_id ||
    rootMessage.link?.kind !== "task" ||
    rootMessage.link?.id !== mission.task_id
  ) {
    throw new Error(`root message attribution or structured data failed`);
  }
  if (
    !replyMessage ||
    replyMessage.actor_id !== demo.bob_actor_id ||
    replyMessage.reply_to_id !== root.message_id ||
    replyMessage.thread_root_id !== root.message_id ||
    replyMessage.mentions[0] !== demo.alice_actor_id ||
    replyMessage.link?.kind !== "mission"
  ) {
    throw new Error(`thread reply attribution or structured data failed`);
  }
}

const eve = await snapshot(demo.corp_id, demo.eve_actor_id);
if (
  eve.snapshot.rooms.length !== 0 ||
  eve.snapshot.room_messages.length !== 0 ||
  eve.snapshot.missions.length !== 0 ||
  eve.snapshot.tasks.length !== 0 ||
  eve.snapshot.runs.length !== 0
) {
  throw new Error("non-member snapshot exposed room-scoped state");
}
const eveWrite = await request(roomPath, {
  actor_id: demo.eve_actor_id,
  body: "This write must be rejected.",
  reply_to_id: null,
  mentions: [],
  link: null,
});
if (eveWrite.status !== 403) {
  throw new Error(`non-member room write returned ${eveWrite.status}, expected 403`);
}
const eveMission = await request(`/api/corps/${demo.corp_id}/missions`, {
  requested_by: demo.eve_actor_id,
  title: "This room-scoped mission must be rejected.",
});
if (eveMission.status !== 403) {
  throw new Error(
    `non-member mission write returned ${eveMission.status}, expected 403`,
  );
}

const [bobEvents, eveEvents] = await Promise.all([
  replay(demo.corp_id, demo.bob_actor_id),
  replay(demo.corp_id, demo.eve_actor_id),
]);
const bobTypes = bobEvents.map((event) => event.type);
const eveTypes = eveEvents.map((event) => event.type);
if (bobTypes.filter((type) => type === "room.message_posted").length !== 2) {
  throw new Error("room member replay omitted message events");
}
if (eveTypes.includes("room.message_posted") || eveTypes.includes("mission.created")) {
  throw new Error("non-member replay exposed room-scoped events");
}
if (eveTypes.join(",") !== "corp.demo_bootstrapped") {
  throw new Error(`unexpected non-member event visibility: ${eveTypes.join(",")}`);
}

console.log(
  JSON.stringify(
    {
      root_message_id: root.message_id,
      reply_message_id: reply.message_id,
      member_message_count: alice.snapshot.room_messages.length,
      reply_thread_root: root.message_id,
      structured_mentions: true,
      structured_links: true,
      non_member_rooms: eve.snapshot.rooms.length,
      non_member_messages: eve.snapshot.room_messages.length,
      non_member_write_status: eveWrite.status,
      non_member_mission_status: eveMission.status,
      member_replay_messages: bobTypes.filter(
        (type) => type === "room.message_posted",
      ).length,
      non_member_replay_types: eveTypes,
    },
    null,
    2,
  ),
);
