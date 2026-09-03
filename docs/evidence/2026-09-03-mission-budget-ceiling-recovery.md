# Mission budget ceiling recovery

**Date:** September 3, 2026  
**Issue:** GitHub #80  
**Source base:** `7b2e601b5881c0f9403df64a56e0817e4a999634`

## Schema alignment

Forward-only migration `0030_mission_budget_ceiling.sql` replaces
`missions_graph_limits_check`. It preserves `max_nodes BETWEEN 1 AND 32` and
`max_depth BETWEEN 0 AND 8`, while changing only the mutable
`missions.budget_tokens` upper bound from 2,000,000 to 20,000,000.

No historical migration was changed. The SHA-384 recorded for migration 0030 is:

```text
36994a8052952a036df2ae49baac5f41094015e3deb498fef8a7e356adc193ac8812bbbb5e88d2a4690354e8e12f276d
```

The distinct admission boundaries remain:

- initial mission task graphs: 2,000,000 tokens in
  `crony-server::planning::MAX_GRAPH_BUDGET_TOKENS`;
- initial and replacement task contracts: 2,000,000 tokens in
  `crony-server::planning::MAX_TASK_BUDGET_TOKENS` and contract normalization;
- budget-revision finish scopes: 2,000,000 tokens in
  `crony-store::budget_revision::MAX_TASK_BUDGET_TOKENS`; and
- revised current mission ceilings: 20,000,000 tokens in
  `crony-store::budget_revision::MAX_MISSION_BUDGET_TOKENS` and migration 0030.

## Local verification

`node tools/check_migrations.mjs` reported:

```json
{
  "schema_version": 1,
  "migration_count": 30,
  "latest_version": 30,
  "immutable_checksums": true
}
```

The focused Rust tests passed locally:

```text
cargo test -p crony-store budget_revision::tests --quiet
  4 passed; 0 failed

cargo test -p crony-server planning::tests::rejects_cycles_depth_budget_and_retry_violations --quiet
  1 passed; 0 failed
```

The budget-revision regressions accept the exact 20,000,000-token mission
ceiling, reject 20,000,001, accept the exact 2,000,000-token finish-scope
ceiling, and reject 2,000,001. The existing planning regression continues to
reject a task contract above 2,000,000, while the graph admission guard remains
bounded by `MAX_GRAPH_BUDGET_TOKENS = 2_000_000`.

Migration 0030 was also applied to an isolated local PostgreSQL 16 database
whose `missions` table started with the migration-0008 constraint and a row at
its 2,000,000-token maximum. Updates to 2,750,000 and 20,000,000 succeeded.
Updates to 20,000,001 tokens, 33 nodes, and depth 9 each raised a check
violation. PostgreSQL reported the resulting constraint as:

```text
CHECK ((((max_nodes >= 1) AND (max_nodes <= 32))
  AND ((max_depth >= 0) AND (max_depth <= 8))
  AND ((budget_tokens >= 1) AND (budget_tokens <= 20000000))))
```

Formatting and patch whitespace checks passed locally. No hosted GitHub Actions
evidence was used.

## Preserved VendorGuard recovery

The active VendorGuard #74 database was not modified, and
`tools/e2e_budget_revision.mjs` was not run against it. After this change is
merged, restarting `crony-server` against that database will let the normal
SQLx startup migrator apply version 0030 before requests are served. The
migration changes no mission, task, run, provider-session, or worktree data, so
the pending 2,750,000-token owner decision can be retried without replacing the
preserved provider session or workspace lineage. Approval and the subsequent
resume must be captured as post-merge operational evidence against #74.
