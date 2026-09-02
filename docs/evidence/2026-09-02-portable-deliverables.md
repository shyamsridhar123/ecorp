# Portable source deliverables validation

Date: September 2, 2026

This evidence covers GitHub issue #53 at merge commit `4dc6143` plus the issue-scoped
documentation update that records these results.

## Isolated live topology

- PostgreSQL: disposable `issue53` database in container `ecorp-issue53-pg`, host port `55433`
- ECorp server: `127.0.0.1:8891`
- ECorp runner: `runner-issue53`
- Web client: `127.0.0.1:5287`
- Source checkout: this assigned Codex worktree
- Runner workspace: `output/issue53-live/runner`

The validation did not use the shared default ports or database.

## Browser-to-server-to-runner result

`node tools/e2e_portable_deliverables.mjs` passed through the real HTTP API, Postgres,
runner WebSocket, fake child process, isolated Git worktrees, staged object store, authorized
download route, and cleanup acknowledgment.

- archive run: `2073a552-ab5e-42d1-8129-362fba02efd4`
- archive artifact: `610454e9-be64-4a6d-9e1f-e1ec4164e7d3`
- archive SHA-256: `02510582064955a085cabfebf582453fa64f21c359029e934c4b18d5600d568e`
- linked verification SHA-256:
  `dafa7e10be849c30c2a4b1b065d4857d2f866f116af9fba106be456098bf876e`
- commit/branch run: `7336bb9d-2ac4-4328-b3fa-b7622ed9cce2`
- commit/branch artifact: `47a56bde-a1f5-4adc-bbab-e54b970b70de`
- isolated deliverable commit: `fee64445e0188b4de729a6bf36d6b09f893f17ec`
- clean-worktree run: `8d684b50-7727-4d52-9711-f5748643f601`
- clean-worktree artifact: `01a68243-f758-460c-8a27-10e35f0828fa`
- unauthorized download: HTTP `404`

The archive contained the tracked `README.md` modification and untracked
`portable-untracked.txt`, excluded provider evidence `result.md`, and matched its downloaded
content digest. The commit bundle linked the same verification class to the exact isolated branch
and commit. The clean worktree emitted durable `run.deliverable` before
`run.workspace_removed`, and its deliverable remained downloadable after reclamation.

`node tools/e2e_artifacts.mjs` also passed afterward:

- provider artifact run: `63ebb302-9fc2-4681-a820-52f2c82c7f7d`
- provider artifact: `08229a3a-f638-41b0-9962-26f599837a19`
- provider SHA-256: `af640e3380f5b1bc3ac9daf2b49536410e3f2aa9ae8557dd19e7fc763a9198a1`
- signed provenance:
  `378b72159b17673ec470b96d0f44215235c7fe0517f8c9f0afc653952061a2eb`
- unauthorized download: HTTP `404`
- runner-local path exposed: `false`

## Browser UI evidence

The in-app Chromium browser loaded the isolated web client and observed three completed mission
cards. The DOM contained three distinct instances of each:

- provider evidence
- verification evidence
- source deliverable
- integration state

The UI displayed the explicit boundary that pull-request publication and merge require separate
authorization. Browser logs contained zero warnings or errors. At desktop width,
`scrollWidth == clientWidth == 1265`. The responsive check requested `390x844`, produced a
`375`-pixel client width, kept all four evidence categories visible, and had
`scrollWidth == clientWidth == 375`.

## Repository gates

All required gates passed after merging immutable source routing and renumbering portable
deliverables to migration `0023`:

- `node tools/check_migrations.mjs`: 23 migrations, latest version 23, immutable checksums
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`: 68 tests passed, 0 failed
- `pnpm build:web`
- `pnpm lint:web`
- `git diff --check`

Focused portable-deliverable coverage additionally proves deterministic tracked/untracked export,
secret-like path rejection, scoped commit behavior, and preservation of unrelated staged work.

## Cleanup

Only issue-scoped processes and infrastructure were stopped. Ports `55433`, `8891`, and `5287`
were confirmed free, and container `ecorp-issue53-pg` was confirmed absent.

## Dependency integration rerun — September 2, 2026

PR #71 integrated PR #70 head `797bdea1b5a0f46c47585b5ecf0d5dc74379af0c` at merge commit
`7e26948`. Migration validation still reported 23 immutable migrations with portable deliverables
at version 23. The four focused `crony-runner` deliverable tests passed.

The complete isolated path then passed again with PostgreSQL on `55433`, server `8891`, runner
`runner-issue53-integration`, and Vite on `5287`:

- `tools/e2e_portable_deliverables.mjs`
  - archive run `4ff724c4-774a-47be-9615-19c906127ce8`
  - archive artifact `3a0bd962-9d73-46ea-abfb-98e2ca9132fe`
  - archive SHA-256 `d69592ca4cad82446f3b399ad5626cf645535168fa24bdb13c8527efcbb9d70e`
  - commit run `256807d8-7fed-424e-b71b-905970478953`
  - commit artifact `7cfa764c-707b-49df-89b2-4dacd0ad0b8e`
  - isolated commit `06f15822dab4cc7368c43be33cc95b20675f7362`
  - reclaimed run `54fb99f5-fdb8-482e-8a23-5368398df445`
  - retained-before-cleanup ordering remained true
  - unauthorized download remained HTTP `404`
- `tools/e2e_artifacts.mjs`
  - provider artifact run `50a52c39-fb9f-40c5-8104-04bb2bf65410`
  - provider artifact `273b5896-9e6b-4d8f-84ad-6fd1ee41d898`
  - runner-local path exposure remained false

Real Chromium loaded the integrated UI with zero console or page errors. Each of the three
completed mission cards rendered provider evidence, verification linkage, one source deliverable,
one integration state, and the separate publication/merge authorization boundary. Desktop width
had `scrollWidth == clientWidth == 1265`; the responsive check had
`scrollWidth == clientWidth == 390`.

Screenshots:

- `output/playwright/issue53-integration-desktop.png`
- `output/playwright/issue53-integration-mobile.png`

Cleanup stopped only the verified process tree rooted in this worktree. Ports `55433`, `8891`, and
`5287` were closed, and container `ecorp-issue53-integration-live` was absent.

The hosted CI run triggered by this dependency refresh was rejected before any step executed
because the GitHub account has a payment or Actions spending-limit block. Landing remains paused
until hosted CI can actually run; this is recorded as an external infrastructure failure rather
than a passing or failing code gate.
