# Claude stdio permission bridge validation

**Issue:** #110

**Source base:** `9ca4f4b108c5b513245341d78ca09149e64845b8`

## Boundary delivered

The Claude external adapter now uses Claude Code's supported bidirectional stream-JSON control
protocol. It retains safe mode, disables Chrome and slash commands, supplies a strict empty MCP
configuration, and does not enable permission bypass. The launch selects stream JSON for input and
output, `manual` permission mode, and `stdio` as the permission prompt tool. The adapter sends a
correlated `initialize` control request, validates its success response, and only then sends the
mission prompt as a typed user frame.

For each valid `can_use_tool` control request, the adapter retains the provider request ID,
tool-use ID, tool name, original structured input, blocked path, decision reason, title, display
name, and description. Requests that cannot be proved to be a recognized worktree-contained
read/write operation emit one bounded `ApprovalRequested` event and wait for the durable ECorp
decision. Bash, network, blocked-path, ambiguous, malformed, and outside-worktree requests fail
closed. Original tool input stays only in the pending in-memory request. The durable event contains
bounded field names, types, byte counts, and SHA-256 hashes, not raw Write/Edit content or
secret-shaped values.

An approval sends one correlated Claude success `control_response` with the unchanged original
input. Rejection and expiry send a correlated denial with a bounded decision note. Unknown or
duplicate decisions cannot consume another provider request. Provider cancellation, stop,
interrupt, process exit, and breaker termination deny or clear pending requests before the adapter
reports the provider session terminated. Resume passes the provider session ID as an option value,
so a dash-leading ID cannot become another CLI flag. Failure to write a control response fails and
terminates the provider run.

## Protocol-faithful coverage

`scripts/fake-external-agent.mjs` requires manual mode, completes the initialize exchange before it
accepts the typed user frame, emits Claude `control_request` frames,
and requires matching `control_response` frames before producing its final artifact. Focused runner
tests cover:

- hardened launch arguments and dash-leading resume IDs;
- preservation of provider permission context;
- durable suspension followed by one matching approval;
- rejection and expiry denials;
- contained-path auto-allow and outside-worktree suspension;
- duplicate-decision rejection and final artifact behavior;
- redacted, hashed durable input context and failed response-write handling;
- Claude/OpenCode normalized evidence compatibility.

The fake runs do not load user plugins, hooks, MCP servers, browser integration, or source-checkout
memory.

## Installed Claude Code 2.1.223 probes

The installed CLI was invoked with the adapter's exact permission boundary:

```text
claude --safe-mode --no-chrome --disable-slash-commands --strict-mcp-config --mcp-config {"mcpServers":{}} --print --verbose --input-format stream-json --output-format stream-json --permission-mode manual --permission-prompt-tool stdio
```

The probe first sent:

```json
{"type":"control_request","request_id":"probe-initialize","request":{"subtype":"initialize","hooks":null}}
```

Claude Code `2.1.223` returned a matching successful `control_response`. Only after that response,
the probe sent the typed user frame requesting a Bash marker-file write. Claude emitted a
`control_request` with subtype `can_use_tool`. The probe returned the matching denial:

```json
{"type":"control_response","response":{"subtype":"success","request_id":"<Claude request ID>","response":{"behavior":"deny","message":"ECorp probe denial"}}}
```

The process completed without creating the marker. This proves manual mode plus initialize exposes
the supported permission control boundary and that denial prevents the Bash write.

A live browser rejection then exposed one more transport requirement: Claude keeps stream input
open after its terminal `result` frame. The adapter now closes stdin only after a result arrives
with no pending permission. The protocol-faithful fake waits for that EOF before exiting, so focused
tests fail if the runner leaves a completed Claude process alive.

## Persisted verification

The candidate is gated with:

```text
cargo fmt --all -- --check
cargo test -p crony-runner adapter::external
cargo clippy -p crony-runner --all-targets -- -D warnings
git diff --check -- crates/crony-runner/src/adapter/external.rs crates/crony-runner/src/adapter/copilot.rs crates/crony-runner/src/adapter/mod.rs crates/crony-runner/src/adapter/permission.rs scripts/fake-external-agent.mjs docs/ARCHITECTURE.md docs/SECURITY.md docs/EVALS.md docs/evidence/2026-09-03-claude-permission-bridge.md
```

All four persisted gates passed on September 3, 2026. The focused test selector passed 9 tests.

## Operator live validation

The final candidate ran through a temporary Postgres database, ECorp server, and candidate runner
against the installed Claude Code `2.1.223` binary:

- mission `41a8e9f6-5775-443f-9634-419d5e391e92`;
- run `a1541138-d3f9-452f-8925-e86d059ace5e`;
- durable approval `77f3a71c-86c1-47e9-9250-7fc099d08f9f`;
- approval state `rejected`;
- run state `completed`;
- verifier state `passed`.

The approval exposed the blocked path and structured field metadata, byte counts, and SHA-256
digests. It did not persist the Bash command or its `must-not-exist` content. The runner
acknowledged the rejection, Claude completed without creating the marker, and
`run.session_terminated` preceded accepted completion. A prior browser rejection surfaced the
missing terminal-EOF behavior; after the fix, the final server-runner API flow completed cleanly.

The temporary processes and database were removed. This evidence does not claim publication,
merge, or deployment.
