// Account/catalog-only app-server fixture. No auth files, network, threads, or turns.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import readline from "node:readline";

const [directory, ...args] = process.argv.slice(2);
// Compare native canonical paths, not a mixture of Windows 8.3 and long names.
const root = fs.realpathSync.native(directory);
const audit = (method, extra = {}) => fs.appendFileSync(
  path.join(root, "calls.jsonl"),
  JSON.stringify({ tool: "codex", method, ...extra }) + "\n",
);

if (args.length === 1 && args[0] === "--version") {
  audit("version");
  fs.writeSync(1, "codex-cli connection-fixture\n");
} else {
  assert.deepEqual(args.slice(0, -1), [
    "app-server", "--listen", "stdio://",
    "-c", "mcp_servers={}", "-c", "features.apps=false", "-c", "hooks={}",
    "-c",
  ]);
  // A Windows operator .cmd wrapper may consume quotes around this literal.
  assert.equal(args.at(-1).replaceAll('"', ""), "cli_auth_credentials_store=file");
  const home = fs.realpathSync.native(process.env.CODEX_HOME);
  const relative = path.relative(
    fs.realpathSync.native(path.join(root, "connections", "accounts")), home,
  ).split(path.sep);
  assert.equal(relative.length, 4);
  const [owner, agents, connection, native] = relative;
  assert(/^[a-f0-9]{32}$/.test(owner) && /^[a-f0-9]{32}$/.test(connection));
  assert.equal(agents, "agents");
  assert.equal(native, "native");
  assert.equal(
    fs.realpathSync.native(process.cwd()),
    fs.realpathSync.native(path.join(root, "connections", "catalog", owner, connection)),
  );
  const input = readline.createInterface({ input: process.stdin });
  const seen = [];
  input.on("line", line => {
    try {
      const frame = JSON.parse(line);
      const method = frame.method;
      audit(method, { home, owner, connection });
      assert.equal(method, ["initialize", "initialized", "account/read", "model/list"][seen.length]);
      seen.push(method);
      let result;
      if (method === "initialize") {
        result = { userAgent: "ECorp connection fixture" };
      } else if (method === "initialized") {
        assert.equal(frame.id, undefined);
        return;
      } else if (method === "account/read") {
        assert.deepEqual(frame.params, { refreshToken: false });
        const signedOut = fs.existsSync(path.join(root, `codex-signed-out-${connection}`));
        result = {
          account: signedOut ? null : { type: "chatgpt", email: "fixture@example.invalid" },
          requiresOpenaiAuth: true,
        };
      } else if (method === "model/list") {
        assert.deepEqual(frame.params, { limit: 100, cursor: null, includeHidden: false });
        result = {
          data: [{ id: "fixture-codex", model: "fixture-codex", displayName: "Fixture Codex" }],
          nextCursor: null,
        };
      } else {
        throw new Error("provider execution is forbidden");
      }
      fs.writeSync(1, JSON.stringify({ id: frame.id, result }) + "\n");
    } catch {
      audit("forbidden", { home, owner, connection });
      process.exit(91);
    }
  });
  // The production adapter owns shutdown; the fixture starts no descendants.
}
