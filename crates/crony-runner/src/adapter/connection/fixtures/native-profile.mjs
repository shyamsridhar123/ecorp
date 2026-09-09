// Deterministic native-protocol fixture. No network, credentials or model turns.
import fs from "node:fs";
import path from "node:path";
import readline from "node:readline";

const [provider, auditDirectory, ...args] = process.argv.slice(2);
const home = process.env[
  { codex: "CODEX_HOME", claude: "CLAUDE_CONFIG_DIR", copilot: "COPILOT_HOME" }[provider]
];
if (!home || !path.isAbsolute(home)) process.exit(80);
const audit = (method, extra = {}) =>
  fs.appendFileSync(path.join(auditDirectory, "calls.jsonl"), JSON.stringify({ method, home, ...extra }) + "\n");
const marker = path.join(home, "fixture-signed-in");
const signedIn = () => fs.existsSync(marker);
const signIn = () => fs.writeFileSync(marker, "fixture-only");
const out = value => process.stdout.write(JSON.stringify(value) + "\n");

if (args.includes("--version")) {
  audit("forbidden-constructor-version-probe");
  process.exit(81);
}
for (const key of ["OPENAI_API_KEY", "ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"]) {
  if (key in process.env) process.exit(82);
}
if (provider === "copilot" && process.env.COPILOT_DISABLE_KEYTAR !== "1") process.exit(83);

if (provider === "claude" && args[0] === "auth" && args[1] === "status") {
  audit("claude.auth.status");
  out({ loggedIn: signedIn(), email: "fixture@example.test" });
  process.exit(signedIn() ? 0 : 1);
}
if (provider === "claude" && args[0] === "auth" && args[1] === "login") {
  audit("claude.auth.login");
  const url = "https://claude.ai/oauth/authorize?code=true&client_id=fixture&response_type=code&redirect_uri=https%3A%2F%2Fconsole.anthropic.com%2Foauth%2Fcode%2Fcallback&scope=user%3Aprofile&state=fixture&code_challenge=fixture&code_challenge_method=S256";
  process.stdout.write(`Open this URL:\n${url}\nPaste code here if prompted > `);
  const input = readline.createInterface({ input: process.stdin });
  input.once("line", code => {
    // Deliberately never retain or echo the submitted value.
    audit("claude.authorization-code-received");
    if (code !== "fixture-authorization-code") process.exit(84);
    signIn();
    input.close();
    process.exit(0);
  });
} else if (provider === "codex") {
  const input = readline.createInterface({ input: process.stdin });
  input.on("line", line => {
    const message = JSON.parse(line);
    audit(message.method);
    let result;
    switch (message.method) {
      case "initialize": result = { userAgent: "fixture" }; break;
      case "initialized": return;
      case "account/read":
        result = { account: signedIn() ? { type: "chatgpt", email: "fixture@example.test" } : null, requiresOpenaiAuth: true };
        break;
      case "model/list":
        result = { data: [{ id: "fixture-codex", model: "fixture-codex", displayName: "Fixture Codex", supportedReasoningEfforts: [{ reasoningEffort: "low" }], defaultReasoningEffort: "low", inputModalities: ["text"] }], nextCursor: null };
        break;
      case "account/login/start":
        if (message.params.type !== "chatgptDeviceCode") process.exit(85);
        result = { type: "chatgptDeviceCode", loginId: "fixture-login", verificationUrl: "https://auth.openai.com/codex/device", userCode: "TEST-1234" };
        out({ id: message.id, result });
        signIn();
        out({ method: "account/login/completed", params: { loginId: "fixture-login", success: true } });
        return;
      case "account/login/cancel": result = { status: "canceled" }; break;
      default: process.exit(86); // No thread/start, turn/start, or model input.
    }
    out({ id: message.id, result });
  });
  input.on("close", () => process.exit(0));
} else if (provider === "claude" && args.includes("--input-format")) {
  const input = readline.createInterface({ input: process.stdin });
  input.on("line", line => {
    const message = JSON.parse(line);
    if (message.type !== "control_request" || message.request.subtype !== "initialize") process.exit(87);
    audit("claude.initialize");
    out({ type: "control_response", response: { subtype: "success", request_id: message.request_id, response: { models: [{ value: "fixture-claude", displayName: "Fixture Claude", supportsEffort: true, supportedEffortLevels: ["low", "high"] }], account: { email: "fixture@example.test" } } } });
  });
  input.on("close", () => process.exit(0));
} else if (provider === "copilot" && args.includes("--server")) {
  let buffer = Buffer.alloc(0);
  const reply = value => {
    const body = Buffer.from(JSON.stringify(value));
    process.stdout.write(`Content-Length: ${body.length}\r\n\r\n`);
    process.stdout.write(body);
  };
  process.stdin.on("data", chunk => {
    buffer = Buffer.concat([buffer, chunk]);
    while (true) {
      const split = buffer.indexOf("\r\n\r\n");
      if (split < 0) return;
      const length = Number(/Content-Length:\s*(\d+)/i.exec(buffer.subarray(0, split).toString())?.[1]);
      if (!Number.isSafeInteger(length) || length > 65536) process.exit(88);
      if (buffer.length < split + 4 + length) return;
      const message = JSON.parse(buffer.subarray(split + 4, split + 4 + length));
      buffer = buffer.subarray(split + 4 + length);
      audit(message.method);
      let result;
      switch (message.method) {
        case "connect":
          result = {
            ok: true,
            protocolVersion: 3,
            version: fs.existsSync(path.join(auditDirectory, "wrong-version"))
              ? "1.0.83" : "1.0.79"
          };
          break;
        case "ping": result = { protocolVersion: 3 }; break;
        case "status.get": result = { version: fs.existsSync(path.join(auditDirectory, "wrong-version")) ? "1.0.83" : "1.0.79", protocolVersion: 3 }; break;
        case "auth.getStatus": result = { isAuthenticated: signedIn(), authType: "user", login: "fixture-user" }; break;
        case "models.list": result = { models: [{ id: "fixture-copilot", name: "Fixture Copilot", capabilities: {} }] }; break;
        default: process.exit(89); // No session.create, session.send or tools.
      }
      reply({ jsonrpc: "2.0", id: message.id, result });
    }
  });
  process.stdin.on("end", () => process.exit(0));
} else if (provider === "copilot" && args.includes("login")) {
  audit("copilot.login");
  process.stdout.write("First copy your one-time code: TEST-1234\nOpen https://github.com/login/device\n");
  signIn();
  process.exit(0);
} else {
  process.exit(90);
}
