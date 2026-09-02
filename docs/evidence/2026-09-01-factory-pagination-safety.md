# Factory pagination-safety validation — September 1–2, 2026

## Scope

This record covers GitHub issue #67. GitHub Project `ECorp Build` #3 remained the planning and
status source of truth; `docs/BACKLOG.md` was not used for live status.

The factory controller no longer discovers existing work items through the shared Corp snapshot.
It lists current GitHub Project candidates, looks up only those Project item IDs through a
Corp-authorized server endpoint, and repeats the selected-item lookup immediately before claim.
Recovery therefore reuses the persisted source and policy snapshots even when the authoritative
work item is older than the snapshot's 500-row factory projection.

## Deterministic end-to-end evidence

`node tools/e2e_factory_controller.mjs` passed against the real Postgres store, ECorp server,
runner, scheduler, worktree manager, fake-process child, verifier, event journal, and deterministic
GitHub CLI boundary.

Final standard-port review-hardening rerun completed at `2026-09-02T01:53:48.026Z`.

Pagination regression:

- target Project item: `PVTI_FAKE_FACTORY_9020`
- target work item: `0980ed8d-9696-488e-b3d3-155856eddf45`
- target mission: `09618fab-2391-4f24-93c9-8a9fa3f3c727`
- target run: `80385961-bd64-4a70-b1b0-bbb6d33d8257`
- 501 newer historical factory work items were created after the recoverable target
- the legacy shared snapshot returned 500 factory items and omitted the target
- selected-item lookup returned exactly one authoritative match
- the same item ID existed in Project `other/8` with a conflicting policy, while lookup for
  Project `acme/7` returned only the configured Project's row
- duplicate requested IDs were deduplicated
- requests above 1,000 IDs and identifiers above 160 characters were rejected
- the guest lookup was rejected with `403` without returning source metadata
- recovery reused the exact work item, mission, and persisted policy despite changing current CLI
  lease duration from 300 to 600 seconds as well as budget and write-scope arguments
- a fresh post-replay lookup counted exactly one matching work item
- the fresh snapshot independently found every issue-derived mission by exact deterministic title
  or the persisted issue URL/revision source marker, without following the work item's mission link
- those direct mission IDs contained exactly one mission, one task, and one run, and the factory
  state was `verified`
- no misleading policy-mismatch error or duplicate mission/run occurred

The complete controller regression also retained its prior dry-run, status-sync failure, bounded
GitHub timeout, terminal failure, verifier rejection, independent review, repository-routing, and
source-revalidation coverage.

## Repository gates

`pnpm check` passed after the E2E run:

- migration schema `21` and all 21 immutable migration checksums passed
- `cargo fmt --check` passed
- `cargo clippy --workspace --all-targets -- -D warnings` passed
- `cargo test --workspace` passed 59 tests with zero failures
- `pnpm build:web` passed
- `pnpm lint:web` passed

## Runtime cleanup

After the other worktree released the shared runtime, the final rerun used the standard local
server and web ports `8791` and `5187` with isolated Postgres database
`crony_issue67_df02_20260902_003`. `tools/stop_local.ps1` stopped the seven test-owned processes,
both ports were verified closed, and the isolated database was dropped.
