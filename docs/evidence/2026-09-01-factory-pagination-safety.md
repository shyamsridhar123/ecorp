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

Review-hardening rerun completed at `2026-09-02T00:58:57.407Z`.

Pagination regression:

- target Project item: `PVTI_FAKE_FACTORY_9020`
- target work item: `6b71fa30-7988-4624-b073-d4ea646e2eed`
- target mission: `15a05e65-5dd4-489b-a603-b820d052ce2c`
- target run: `c5469812-9179-4241-a81a-a90d6b7b58d4`
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
- a fresh post-replay lookup and snapshot counted exactly one matching work item, mission, task,
  and run, then confirmed the factory state was `verified`
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
had no listeners afterward, and no test-owned ECorp process remained. The rerun used and then
removed the isolated Postgres database `crony_issue67_df02_20260902_001`; the shared database was
not modified after it reported a migration from another worktree.
