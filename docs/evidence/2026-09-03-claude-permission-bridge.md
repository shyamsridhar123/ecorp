# Claude stdio permission bridge validation

**Issue:** #110

**Source base:** `9ca4f4b108c5b513245341d78ca09149e64845b8`

## Boundary delivered

The Claude external adapter now uses Claude Code's supported bidirectional stream-JSON control
protocol. It retains safe mode, disables Chrome and slash commands, supplies a strict empty MCP
configuration, and does not enable permission bypass. The launch selects stream JSON for input and
output and `stdio` as the permission prompt tool. The mission prompt is sent as a typed user frame.

For each valid `can_use_tool` control request, the adapter retains the provider request ID,
tool-use ID, tool name, original structured input, blocked path, decision reason, title, display
name, and description. Requests that cannot be proved to be a recognized worktree-contained
read/write operation emit one bounded `ApprovalRequested` event and wait for the durable ECorp
decision. Bash, network, blocked-path, ambiguous, malformed, and outside-worktree requests fail
closed.

An approval sends one correlated Claude success `control_response` with the unchanged original
input. Rejection and expiry send a correlated denial with a bounded decision note. Unknown or
duplicate decisions cannot consume another provider request. Provider cancellation, stop,
interrupt, process exit, and breaker termination deny or clear pending requests before the adapter
reports the provider session terminated. Resume passes the provider session ID as an option value,
so a dash-leading ID cannot become another CLI flag.

## Protocol-faithful coverage

`scripts/fake-external-agent.mjs` reads the typed user frame, emits Claude `control_request` frames,
and requires matching `control_response` frames before producing its final artifact. Focused runner
tests cover:

- hardened launch arguments and dash-leading resume IDs;
- preservation of provider permission context;
- durable suspension followed by one matching approval;
- rejection and expiry denials;
- contained-path auto-allow and outside-worktree suspension;
- duplicate-decision rejection and final artifact behavior;
- Claude/OpenCode normalized evidence compatibility.

The fake runs do not load user plugins, hooks, MCP servers, browser integration, or source-checkout
memory.

## Persisted verification

The candidate is gated with:

```text
cargo fmt --all -- --check
cargo test -p crony-runner adapter::external
cargo clippy -p crony-runner --all-targets -- -D warnings
git diff --check -- crates/crony-runner/src/adapter/external.rs crates/crony-runner/src/adapter/copilot.rs crates/crony-runner/src/adapter/mod.rs crates/crony-runner/src/adapter/permission.rs scripts/fake-external-agent.mjs docs/ARCHITECTURE.md docs/SECURITY.md docs/EVALS.md docs/evidence/2026-09-03-claude-permission-bridge.md
```

All four persisted gates passed on September 3, 2026. The focused test selector passed 8 tests.

## Operator validation boundary

The mission operator separately owns the temporary live server-to-runner exercise against this
candidate: database-backed durable approval, fake Claude child process, browser/API snapshot, and
artifact confirmation. This local evidence does not substitute a fixture result for that operator
observation and does not claim publication, merge, or deployment.
