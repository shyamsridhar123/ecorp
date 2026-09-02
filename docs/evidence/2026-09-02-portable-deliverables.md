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

## Export-boundary review hardening — September 2, 2026

Commit `cd016427685508b2dfdb5d1d37d6db2e4c4acce2` closes the three later review
findings against the portable export boundary:

- each start and resume assignment now carries the persisted task `write_scope` to the runner;
- export rejects any selected changed path outside that scope, including the default
  `deliverable.paths: []` case;
- the temporary export index starts from the verified base commit rather than the current task
  `HEAD`, so an existing agent commit cannot silently add unselected paths;
- commit/branch exports create a bounded commit directly on the verified base, then atomically move
  only the isolated task branch while preserving unselected work in the worktree index; and
- sensitive runner and credential directories such as nested `.azure`, `.ssh`, and
  `.config/gcloud` paths are rejected at every path depth.

The focused runner suite passed `38/38`, including regressions for narrowed write scope, nested
credential directories, unselected committed changes, selected-commit ancestry, and preservation
of unrelated staged work.

A fresh isolated stack used PostgreSQL `pr71review` on `55434`, server `8892`, runner
`runner-pr71-review`, and Vite `5288`. `tools/e2e_portable_deliverables.mjs` passed twice and
`tools/e2e_artifacts.mjs` passed between those runs. The final portable run recorded:

- archive run `f651113a-0dbe-494b-a972-70170bfa1421`
- archive artifact `4b35d49c-638f-4206-9914-2ebac5ae1af5`
- archive SHA-256 `5fcc011dda886c4709f36e8a08a97d6c390711278290736e2935980b4ca8ac9e`
- commit run `117a9cf7-30c7-4adf-a1c3-f8f90bf22c03`
- commit artifact `096a9a35-e16e-477d-ab98-c4ff436c63d1`
- bounded isolated commit `ef549f2b292b45a0c33996c8ad69c1746067f4bb`
- reclaimed run `2e88ae33-8df2-4abc-b577-75ae10edd952`
- retained-before-cleanup ordering `true`
- unauthorized download HTTP `404`

Headless Chromium loaded the resulting source-deliverable cards with zero console or page errors.
Desktop had `scrollWidth == clientWidth == 1440`; mobile had
`scrollWidth == clientWidth == 390`. Both displayed the source deliverable, download action, and
`INTEGRATION · READY FOR REVIEW`.

Screenshots:

- `output/playwright/pr71-deliverable-desktop.png`
- `output/playwright/pr71-deliverable-mobile.png`

The exact code head `cd016427685508b2dfdb5d1d37d6db2e4c4acce2` passed all 23 immutable
migration checks, formatting, warning-free workspace clippy, all 73 Rust tests, the production web
build, web lint, and `git diff --check`. The test-owned process tree and container were removed;
ports `55434`, `8892`, and `5288` were closed.

GitHub Actions is unavailable because the account has exhausted its hosted-runner credits for the
month. Those zero-step billing failures are not treated as product validation or as a landing
blocker. Landing uses the complete local gate, focused E2Es, browser evidence, and clean review
threads; auto-merge remains disabled.
