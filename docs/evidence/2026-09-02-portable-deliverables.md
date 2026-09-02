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
