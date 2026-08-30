# Alpha evaluation, chaos, and platform validation — August 30, 2026

## 100-scenario suite

`tests/scenarios/v1.jsonl` contains 100 immutable, versioned scenarios across task decomposition,
capability matching, delegation, recovery, approvals, budgets, secrets, artifact verification,
multiplayer control, and protocol boundaries.

`node tools/run_evals.mjs` keeps deterministic and real-provider lanes separate and emits success,
verified completion, safety, rework, cost, latency, and intervention metrics. Deterministic
regression thresholds run in CI; credentialed real-provider cases remain separately opt-in.

## Chaos evidence

`tools/e2e_chaos_report.mjs` consolidates independently executed evidence for server restart during
a live run, runner reconnect and loss, duplicate delivery, and browser replay.

## Platform evidence

The `runner-platforms` matrix executes the Rust runner suite and
`tools/platform_runner_contract.mjs` on GitHub-hosted Windows, macOS, and Linux runners. Each lane
checks process behavior, Git worktrees, paths with spaces and Unicode, structured stdio, artifact
creation, and SHA-256 integrity.

The supported adapters use structured stdio or JSON-RPC rather than an interactive PTY. PTY
behavior is documented as not applicable to the current adapter contract; a future terminal
adapter must add its own PTY conformance lane.
