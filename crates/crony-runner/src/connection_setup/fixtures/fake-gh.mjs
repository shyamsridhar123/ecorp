// Native CLI fixture only: no network, keychain, token reads, or real authentication.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

const [directory, ...args] = process.argv.slice(2);
// Expand Windows 8.3 aliases just as the runner's native canonicalization does.
const root = fs.realpathSync.native(directory);
const settings = JSON.parse(fs.readFileSync(path.join(root, "fixture.json"), "utf8"));
const stateRoot = fs.realpathSync.native(path.join(root, "connections"));
const cwd = fs.realpathSync.native(process.cwd());
const relative = path.relative(stateRoot, cwd).split(path.sep);
const machine = relative.length === 1 && relative[0] === "github-machine";
assert(machine || (relative.length === 3 && relative[0] === "accounts"
  && /^[a-f0-9]{32}$/.test(relative[1]) && relative[2] === "github"));
const owner = machine ? "machine" : relative[1];
// Never inspect the operator's ambient GH_CONFIG_DIR or credentials for machine mode.
const home = machine ? null : fs.realpathSync.native(process.env.GH_CONFIG_DIR);
if (home !== null) assert.equal(home, fs.realpathSync.native(path.join(cwd, "native")));
const marker = home === null ? null : path.join(home, "fixture-login-marker");
const hosts = home === null ? null : path.join(home, "hosts.yml");
const loggedIn = () => machine || (fs.existsSync(marker) && fs.existsSync(hosts));
const login = machine ? "fixture-machine" : `fixture-${owner.slice(0, 24)}`;
const audit = (method, extra = {}) => fs.appendFileSync(
  path.join(root, "calls.jsonl"),
  JSON.stringify({ tool: "gh", method, owner, home, ...extra }) + "\n",
);
const reply = value => fs.writeSync(1, JSON.stringify(value) + "\n");
const repository = {
  full_name: "team/repo",
  node_id: "R_fixture_repo",
  default_branch: "main",
  private: true,
  permissions: { push: true },
};

function git(argv) {
  const env = {
    GIT_CONFIG_NOSYSTEM: "1",
    GIT_CONFIG_GLOBAL: path.join(root, "empty.gitconfig"),
    GIT_TERMINAL_PROMPT: "0",
    GIT_ALLOW_PROTOCOL: "file",
    HOME: path.join(root, "empty-home"),
    USERPROFILE: path.join(root, "empty-home"),
  };
  for (const key of ["PATH", "SystemRoot", "SYSTEMROOT", "WINDIR", "COMSPEC", "TEMP", "TMP"]) {
    if (process.env[key] !== undefined) env[key] = process.env[key];
  }
  const result = spawnSync(settings.git, ["-c", "core.hooksPath=", ...argv], {
    cwd: root, env, encoding: "utf8", timeout: 30_000, windowsHide: true,
  });
  assert.equal(result.status, 0, "owned fixture Git command failed");
}

try {
  assert(!args.includes("--insecure-storage"), "never force insecure credential storage");
  if (args[0] === "auth" && args[1] === "status") {
    assert.deepEqual(args, ["auth", "status", "--active", "--hostname", "github.com"]);
    audit("auth.status");
    if (loggedIn()) {
      const storage = fs.existsSync(path.join(root, `file-storage-${owner}`)) ? "hosts.yml" : "keyring";
      fs.writeSync(2, `Logged in to github.com account ${login} (${storage})\n`);
    }
    process.exitCode = loggedIn() ? 0 : 1;
  } else if (args[0] === "auth" && args[1] === "login") {
    assert(!machine, "machine login must never be mutated");
    assert.deepEqual(args, [
      "auth", "login", "--hostname", "github.com", "--web", "--git-protocol", "https",
    ]);
    audit("auth.login", { insecure_storage: args.includes("--insecure-storage") });
    fs.writeFileSync(marker, "synthetic fixture account; not a credential\n", { flag: "wx" });
    // The manager checks only existence/type. This contains no credential values.
    fs.writeFileSync(hosts, "# synthetic native account metadata only\n", { flag: "wx" });
    fs.writeSync(2, "! First copy your one-time code: TEST-1234\n");
  } else if (args[0] === "api") {
    assert.deepEqual(args.slice(0, 5), ["api", "--hostname", "github.com", "--method", "GET"]);
    assert.equal(args.length, 6);
    assert(loggedIn(), "API requires the selected fixture account");
    const endpoint = args[5];
    audit("api", { endpoint });
    if (endpoint === "user") {
      const override = path.join(root, `api-login-${owner}`);
      reply({ login: fs.existsSync(override) ? fs.readFileSync(override, "utf8").trim() : login });
    } else if (endpoint === "user/repos?per_page=100&sort=updated&affiliation=owner,collaborator,organization_member") {
      reply([repository]);
    } else if (endpoint === "repos/team/repo") {
      reply(repository);
    } else if (endpoint === "repos/team/repo/commits/main") {
      const sha = fs.readFileSync(path.join(root, "selected-head"), "utf8").trim();
      assert(/^(?:[a-f0-9]{40}|[a-f0-9]{64})$/.test(sha));
      reply({ sha });
    } else {
      throw new Error("unexpected API endpoint");
    }
  } else if (args[0] === "repo" && args[1] === "clone") {
    assert(loggedIn());
    assert.equal(args[2], "https://github.com/team/repo.git");
    assert.deepEqual(args.slice(4), [
      "--", "--no-checkout", "--no-hardlinks", "--no-tags", "--config=core.longpaths=true",
    ]);
    assert(path.isAbsolute(args[3]));
    assert(/^[a-f0-9]{32}$/.test(path.basename(args[3])));
    const parent = fs.realpathSync.native(path.dirname(args[3]));
    assert.equal(
      parent,
      fs.realpathSync.native(path.join(stateRoot, "r")),
    );
    const target = path.join(parent, path.basename(args[3]));
    assert.equal(fs.realpathSync.native(settings.source), fs.realpathSync.native(path.join(root, "source")));
    audit("repo.clone", { target });
    // Honor the production clone's native, repository-local Windows path option.
    git([
      "clone", "--no-checkout", "--no-hardlinks", "--no-tags", "--no-local",
      "--config=core.longpaths=true", settings.source, target,
    ]);
    git(["-C", target, "remote", "set-url", "origin", args[2]]);
    // Both selected commits are already local. Any unexpected remote access fails offline.
    git(["-C", target, "config", "--local", "protocol.allow", "never"]);
    git(["-C", target, "config", "--local", "core.hooksPath", path.join(root, "empty-hooks")]);
    git(["-C", target, "config", "--local", "core.fsmonitor", "false"]);
  } else {
    throw new Error("unexpected native command; credential extraction is forbidden");
  }
} catch {
  audit("forbidden");
  process.exitCode = 90;
}
