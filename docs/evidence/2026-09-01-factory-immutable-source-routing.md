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
  `a159184f3d9f76209c4bc48b7291b8fe27e7ea2d`
- wrong runner `aaa-wrong-commit`:
  `e2f3115d3dd5edffbc421929182fdb185f42b00f`

The wrong runner sorted before `runner-local`, so ref-only routing would have selected it. Commit
matching instead selected `runner-local`. The factory mission completed and verified with:

- task source commit:
  `a159184f3d9f76209c4bc48b7291b8fe27e7ea2d`
- persisted run source commit:
  `a159184f3d9f76209c4bc48b7291b8fe27e7ea2d`
- runner-emitted workspace base commit:
  `a159184f3d9f76209c4bc48b7291b8fe27e7ea2d`
- wrong-runner run count: `0`
- wrong-runner managed worktree file count: `0`

The fixture runner was stopped and its disposable repository and workspace were removed after the
assertions.

## Review remediation evidence

The live-ref regression
`workspace::tests::unpinned_worktrees_follow_the_live_ref_while_pinned_and_resumed_work_stays_fixed`
passed. It kept one `WorkspaceManager` alive, advanced the configured `HEAD`, and proved:

- a later unpinned worktree started from the advanced commit
- a pinned worktree still started from its assigned immutable commit
- pinned and unpinned resumes reused the original persisted workspace base identity

`node tools/e2e_factory_legacy_source_upgrade.mjs` passed with source commit
`a159184f3d9f76209c4bc48b7291b8fe27e7ea2d`.

Derivable legacy run:

- work item `3585bb07-31a3-4d9d-a997-5350cddc35f5`
- mission `5a5af86e-e4d1-4bb0-b09b-0a20875197e5`
- run `550a28f6-78a2-44bd-ae19-ddde81ae1660`
- policy, task contract, and run source tuple were backfilled from the one authoritative
  `workspace_base_commit`

Unmaterialized legacy claim:

- work item `2e7b916b-aaaf-4e19-ae29-c8318e6adeee`
- migration marked `source_commit_upgrade_required=true`
- the controller used the active claim fence to persist one fresh resolved commit
- exactly one `factory.source_commit_pinned` audit event was emitted
- mission `dde4a382-eb71-4889-af45-e618536f8468`
- run `ddd0dbbf-d6bd-4a4d-ad7d-a2fbf3bbefe3`
- final factory state `verified`

### September 2 review hardening

The current-head rerun at `bddba44dfabc756b887fa9d4f0a6d04e2c581e8c` closed two P1 recovery
gaps found during pre-landing review:

- a new claim with `source_base_ref` but no immutable commit was rejected, and no factory work item
  was created
- only records carrying the migration-owned `source_commit_upgrade_required=true` marker could
  enter legacy recovery
- recovery with requested ref `main` against persisted ref `HEAD` failed before the policy or audit
  journal changed
- recovery with the matching persisted ref emitted exactly one `factory.source_commit_pinned`
  event and completed one verified mission

The rerun produced derivable work item `b87a0a24-9c99-4f92-8d88-b56c067d104a` and migrated
unmaterialized work item `181f3d00-204e-4c61-9621-49ebb1103691`. The latter completed as mission
`eb28c5df-38d7-4910-b26f-d5536b9fa329` and run
`af5e5569-5139-4e2e-a759-0823ba1fdf8b`.

The complete claims, immutable-routing, and controller E2Es then passed on the same head. The
test-owned stack stopped cleanly, and ports `8791` and `5187` were closed.

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
