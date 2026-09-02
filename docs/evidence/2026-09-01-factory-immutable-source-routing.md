# Factory immutable source routing validation — September 1, 2026

## Scope

This record covers GitHub issue #66. GitHub Project `ECorp Build` #3 remained the operational
status source of truth; issue #66 was moved from `Todo` to `In Progress` before implementation.
No markdown backlog was used for live status.

The delivered source identity is an all-or-nothing tuple:

```text
normalized GitHub owner/repository
  + human-readable base ref
  + resolved full immutable Git commit
```

The trusted factory controller verifies the local origin before resolving the commit. The policy
snapshot, materialized task contract, persisted run, server assignment, runner capability, and
workspace evidence retain that tuple. Commit values accept only full 40- or 64-character
hexadecimal Git object IDs.

## Deterministic multi-runner evidence

`node tools/e2e_factory_commit_routing.mjs` passed against the real Postgres, server, runner, child
process, verifier, and artifact path.

Both connected runners advertised:

- repository `shyamsridhar123/ecorp`
- symbolic base ref `HEAD`

They intentionally resolved that ref differently:

- authorized runner `runner-local`:
  `e2f3115d3dd5edffbc421929182fdb185f42b00f`
- wrong runner `aaa-wrong-commit`:
  `d968a5c3d4f5244284530045943299cc78dd7918`

The wrong runner sorted before `runner-local`, so ref-only routing would have selected it. Commit
matching instead selected `runner-local`. The factory mission completed and verified with:

- task source commit:
  `e2f3115d3dd5edffbc421929182fdb185f42b00f`
- persisted run source commit:
  `e2f3115d3dd5edffbc421929182fdb185f42b00f`
- runner-emitted workspace base commit:
  `e2f3115d3dd5edffbc421929182fdb185f42b00f`
- wrong-runner run count: `0`
- wrong-runner managed worktree file count: `0`

The fixture runner was stopped and its disposable repository and workspace were removed after the
assertions.

## Factory regression evidence

`node tools/e2e_factory_controller.mjs` passed its complete deterministic controller suite.
The successful issue produced one work item, one mission, and one run. The task and persisted run
both retained the immutable source commit. Existing repository mismatch, source revision change,
dependency reopening, external-effect timeout, terminal mission failure, verification failure,
independent review, and replay checks continued to pass.

`node tools/e2e_factory_claims.mjs` also passed after restarting the server between claim and
renewal. Concurrent claims, renewals, and materializations still collapsed to one effect; expired
reclaim preserved the source and policy snapshot; the linked mission and run completed; and the
factory item reached `verified`.

Targeted Rust tests passed:

- `crony-cli`: 6 passed
- `crony-runner`: 29 passed
- `crony-server`: 16 passed
- `crony-store`: 7 passed

The runner coverage proves a matching repository and ref with a different commit is rejected, as
is a partial source tuple. Both `StartRun` and `ResumeRun` carry the source tuple and execute the
same fail-closed check before creating or reusing a worktree. The server independently revalidates
the tuple before resume dispatch.

## Browser evidence

The complete local server, runner, and Vite client were exercised with headless Chrome at
`http://127.0.0.1:5187`.

- HTTP server health was `ok` with one connected runner.
- The web client returned HTTP `200`.
- The factory panel rendered issue `#9066` as `Verified`.
- The rendered task contract displayed
  `shyamsridhar123/ecorp @ HEAD (e2f3115d3dd5)`.
- The DOM contained the complete acceptance checks and successful run evidence.
- Screenshot:
  `output/playwright/factory-immutable-commit-routing.png`
- Rendered DOM capture:
  `output/playwright/factory-immutable-commit-routing.html`

After browser validation, `tools/stop_local.ps1` stopped seven owned processes. Ports `8791` and
`5187` had zero listeners, and no server, runner, or Node process referencing this worktree
remained.

## Repository gates

All required repository gates passed after the final implementation and routing E2E:

```powershell
node tools/check_migrations.mjs
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm build:web
pnpm lint:web
```

Migration validation reported 22 immutable migrations with version 22 latest. Clippy completed with
warnings denied, the full Rust workspace tests passed, the production web bundle built, and web
lint completed without findings.
