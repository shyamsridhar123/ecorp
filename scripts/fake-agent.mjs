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
const externalEvidence = cleanWorktree || ignoredWorktree;
const briefingDelay = slowRun ? 4_000 : graphSlowRun ? 1_200 : 700;
const workDelay = slowRun ? 5_000 : graphSlowRun ? 1_200 : 900;

const wait = (milliseconds) =>
  new Promise((resolvePromise) => setTimeout(resolvePromise, milliseconds));

const controls = [];
const input = createInterface({ input: process.stdin });
input.on("line", (line) => {
  try {
    const message = JSON.parse(line);
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
  "- Executed as a real child process owned by the Crony runner.",
  "- Wrote this artifact inside the run-specific workspace.",
  "- Runner computes and reports the SHA-256 digest.",
  `- Live control messages observed: ${controls.length}.`,
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
