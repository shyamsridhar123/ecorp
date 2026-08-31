import { randomUUID } from "node:crypto";
import { createInterface } from "node:readline";
import { mkdir, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";

const args = new Map();
for (let index = 2; index < process.argv.length; index += 2) {
  args.set(process.argv[index], process.argv[index + 1]);
}

const runId = args.get("--run-id");
const workdir = args.get("--workdir");
const mission = args.get("--mission");

if (!runId || !workdir || !mission) {
  console.error("missing --run-id, --workdir, or --mission");
  process.exit(2);
}

const emit = (event) => {
  process.stdout.write(`${JSON.stringify(event)}\n`);
};

const slowRun = mission.includes("[slow]");
const graphSlowRun = mission.includes("[graph-slow]");
const failRun = mission.includes("[always-fail]");
const cleanWorktree = mission.includes("[clean-worktree]");
const ignoredWorktree = mission.includes("[ignored-worktree]");
const verificationMatrix = mission.includes("[verification-matrix]");
const secretProbe = mission.includes("[secret-probe]");
const approvalExpiry = mission.includes("[approval-expiry]");
const approvalAction = mission.includes("[approval-action]") || approvalExpiry;
const budgetLoop = mission.includes("[budget-loop]");
const healthyConversation = mission.includes("[healthy-conversation]");
const externalEvidence = cleanWorktree || ignoredWorktree;
const briefingDelay = slowRun ? 4_000 : graphSlowRun ? 1_200 : 700;
const workDelay = slowRun ? 5_000 : graphSlowRun ? 1_200 : 900;

const wait = (milliseconds) =>
  new Promise((resolvePromise) => setTimeout(resolvePromise, milliseconds));

const controls = [];
let approvalResolver;
let breakerStopped = false;
let breakerStopResolver;
const breakerStopPromise = new Promise((resolvePromise) => {
  breakerStopResolver = resolvePromise;
});
const input = createInterface({ input: process.stdin });
input.on("line", (line) => {
  try {
    const message = JSON.parse(line);
    if (message.type === "approval_decision") {
      approvalResolver?.(message);
      return;
    }
    if (message.type === "circuit_breaker") {
      if (message.stage === "stop") {
        breakerStopped = true;
        breakerStopResolver();
      }
      emit({
        type: "output",
        stream: "control",
        text: `Circuit breaker entered ${message.stage}.`,
      });
      return;
    }
    controls.push(message);
    emit({
      type: "output",
      stream: "control",
      text: `Received live direction from ${message.actor_id}: ${message.text}`,
    });
  } catch {
    emit({ type: "output", stream: "control", text: line });
  }
});

await mkdir(workdir, { recursive: true });
emit({
  type: "status",
  status: "working",
  station: "briefing",
  message: "Reading the mission contract",
});
await wait(briefingDelay);

if (approvalAction) {
  const approvalId = randomUUID();
  emit({
    type: "approval_requested",
    approval_id: approvalId,
    action_key: "publish-release",
    action: "Publish the prepared release artifact",
    risk: "high",
    rationale: "Publishing is an externally visible side effect.",
    required_roles: ["owner", "admin", "member"],
    expires_in_seconds: approvalExpiry ? 1 : 300,
  });
  const decision = await new Promise((resolvePromise) => {
    approvalResolver = resolvePromise;
  });
  approvalResolver = null;
  if (!decision.approved) {
    emit({ type: "cancelled", reason: "Risky action was rejected." });
    input.close();
    process.exit(0);
  }
  emit({
    type: "status",
    status: "working",
    station: "terminal",
    message: "Authorized action resumed exactly once",
  });
}

if (healthyConversation) {
  for (let index = 0; index < 12; index += 1) {
    emit({
      type: "tool_activity",
      signature: "human-conversation",
      progressed: false,
      human_conversation: true,
    });
  }
}

if (budgetLoop) {
  for (let index = 0; index < 12; index += 1) {
    emit({
      type: "usage",
      input_tokens: 5_000,
      output_tokens: 5_000,
      cost_microusd: 100_000,
    });
    emit({
      type: "tool_activity",
      signature: "search:unchanged-query",
      progressed: false,
      human_conversation: false,
    });
    await wait(700);
    if (breakerStopped) {
      emit({ type: "cancelled", reason: "Circuit breaker stopped the run." });
      input.close();
      process.exit(0);
    }
  }
  await Promise.race([breakerStopPromise, wait(15_000)]);
  if (breakerStopped) {
    emit({ type: "cancelled", reason: "Circuit breaker stopped the run." });
    input.close();
    process.exit(0);
  }
}

emit({
  type: "output",
  stream: "stdout",
  text: `Mission accepted: ${mission}`,
});
emit({
  type: "status",
  status: "working",
  station: "research",
  message: "Collecting evidence and constraints",
});
await wait(workDelay);

emit({
  type: "status",
  status: "working",
  station: "terminal",
  message: "Producing the artifact in an isolated workspace",
});
await wait(workDelay);

if (failRun) {
  emit({ type: "failed", error: "Synthetic bounded task failure." });
  input.close();
  process.exit(0);
}

if (verificationMatrix) {
  await writeFile(resolve(workdir, "verify.txt"), "VERIFIED\n", "utf8");
  await writeFile(
    resolve(workdir, "schema.json"),
    `${JSON.stringify({ status: "ok", count: 1 })}\n`,
    "utf8",
  );
  await writeFile(
    resolve(workdir, "screenshot.png"),
    Buffer.concat([Buffer.from("89504e470d0a1a0a", "hex"), Buffer.from("CRONY_SCREENSHOT")]),
  );
}

if (secretProbe && !process.env.CRONY_TEST_SECRET) {
  emit({ type: "failed", error: "Task-scoped secret was not delivered." });
  input.close();
  process.exit(0);
}

const artifact = [
  "# Verified mission artifact",
  "",
  `Run: \`${runId}\``,
  "",
  "## Mission",
  "",
  mission,
  "",
  "## Evidence",
  "",
  "- Executed as a real child process owned by the ECorp runner.",
  "- Wrote this artifact inside the run-specific workspace.",
  "- Runner computes and reports the SHA-256 digest.",
  `- Live control messages observed: ${controls.length}.`,
  `- Task-scoped secret available: ${secretProbe ? "yes" : "not requested"}.`,
  "",
  "## Result",
  "",
  "The first multiplayer vertical slice completed its execution-plane contract.",
  "",
].join("\n");

if (ignoredWorktree) {
  await writeFile(resolve(workdir, "valuable.log"), "ignored but valuable\n", "utf8");
}

const artifactPath = externalEvidence
  ? resolve(workdir, "..", "..", "..", "evidence", `fake-clean-${runId}.md`)
  : resolve(workdir, "result.md");
await mkdir(dirname(artifactPath), { recursive: true });
await writeFile(artifactPath, artifact, "utf8");
emit({
  type: "artifact",
  path: externalEvidence ? artifactPath : "result.md",
  media_type: "text/markdown",
});
await wait(500);

emit({
  type: "completed",
  summary: externalEvidence
    ? "Created runner evidence outside the task worktree."
    : "Created and verified result.md through the runner-owned child process.",
});
input.close();
