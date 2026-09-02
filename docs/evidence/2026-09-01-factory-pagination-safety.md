# Factory pagination-safety validation — September 1, 2026

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

Run completed at `2026-09-01T23:46:02.372Z`.

Pagination regression:

- target Project item: `PVTI_FAKE_FACTORY_9020`
- target work item: `8aec968d-6dec-4c9f-8752-3e515efac35d`
- target mission: `2d106a1a-621d-447c-96fd-58fc1733c048`
- target run: `a9839219-2f3b-4411-b04a-bb32aac09686`
- 501 newer historical factory work items were created after the recoverable target
- the legacy shared snapshot returned 500 factory items and omitted the target
- selected-item lookup returned exactly one authoritative match
- duplicate requested IDs were deduplicated
- requests above 1,000 IDs and identifiers above 240 characters were rejected
- the guest lookup was rejected with `403` without returning source metadata
- recovery reused the exact work item, mission, and persisted policy despite different current CLI
  budget and write-scope arguments
- recovery produced one mission and one run, then reached `verified`
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

The test-owned server and runner were stopped with `tools/stop_local.ps1`. Ports `8791` and `5187`
had no listeners afterward, and no test-owned ECorp process remained.
