# Factory preflight and materialization recovery evidence

**Date:** September 4, 2026
**Issue:** [#128](https://github.com/shyamsridhar123/ecorp/issues/128)
**Environment:** local Windows server and runner with PostgreSQL 16 in WSL
**Hosted CI:** not used; GitHub Actions credits were unavailable

## Boundary implemented

The controller now builds one issue-derived mission payload and uses it for both the read-only
preflight and the later fenced materialization request. Before a new claim or an unmaterialized
reclaim, the server validates:

- task-graph shape and token/cost budgets;
- adapter, model, and reasoning selection;
- verifier checks and timeouts;
- deliverable and write-scope contracts;
- policy source, provider, tool, secret, and budget constraints;
- mission title and description normalization;
- the exact 65,536-byte materialization operation snapshot; and
- membership in an authorized destination room.

Materialization repeats these checks. A later pre-mission rejection uses a dedicated Postgres
operation that verifies the exact opaque claim token, transitions the work item to `blocked`,
releases its lease, and records claim-generation-scoped idempotency. It does not require the
attempted version or lease to remain current, while a rotated token fences stale compensation.

Migration `0031_factory_materialization_rejections.sql` adds the typed
`materialize_rejected` operation to the durable factory operation journal.

## Controller regression

Command:

```powershell
$env:DATABASE_URL='postgres://crony:crony@127.0.0.1:54329/crony'
node tools/e2e_factory_controller.mjs
```

Result: **passed** at `2026-09-04T01:11:36.108Z`.

The same suite ran dry-run and execution rejection cases for:

- a 3,000,000-token request against the 2,000,000-token maximum;
- a verifier timeout of 60,001 ms;
- a model-policy mismatch;
- a reasoning-policy mismatch;
- an unsafe `../outside/**` write scope;
- a U+0008 control character in the issue description; and
- an issue-derived materialization snapshot larger than 65,536 bytes.

Every case left:

```json
{
  "project_status": "Todo",
  "project_mutations": 0,
  "work_item_count": 0,
  "mission_count": 0,
  "task_count": 0,
  "run_count": 0
}
```

The accepted dry run returned one valid `single` task with a 20,000-token and
1,000,000-microusd budget. Execution reused that accepted preflight shape.

The process-interruption regression killed the real CLI after the durable claim and before
materialization. Shared state contained one version-1 `claimed` work item, no claim token, a `Todo`
Project item, and zero missions, tasks, or runs. A test-owned HTTP proxy held the first
materialization request without adding any behavior to the production CLI, and a `finally` path
always terminated the child and proxy. A second controller recovered the same work-item ID,
materialized it, and finished with exactly one mission and one run in `verified`.

The post-claim rejection regression proved:

- invalid materialization became `blocked` with no mission;
- the lease was released;
- widened policy and changed source revision both returned `400`;
- Bob reclaimed the exact work item;
- a second rejection using the same materialization idempotency key did not collide with the first
  claim generation;
- Alice reclaimed the same work item again; and
- test cleanup reached `cancelled` with one work item and zero missions, tasks, or runs.

Machine-readable output:

```text
output/e2e-factory-controller.json
```

## Claim and materialization regression

Command:

```powershell
$env:DATABASE_URL='postgres://crony:crony@127.0.0.1:54329/crony'
$env:CRONY_TEST_SERVER_PID_FILE=(
  Resolve-Path 'output/issue128-test-stack/stack.json'
).Path
node tools/e2e_factory_claims.mjs
```

Result: **passed** at `2026-09-04T01:12:46.349Z`.

It preserved concurrent claim, renewal, materialization, restart, fencing, exact-source,
exact-policy, and one-mission/one-run behavior. Invalid policy, tool, and secret materializations
used independent claims and each converged on a recoverable blocked pre-mission state. The same
31-second lease window also proved a mission-less `claimed` item can be taken over by a second
operator only with the exact source revision and policy; widened policy and changed source were
rejected before the recovered claim was cleaned up.

Machine-readable output:

```text
output/e2e-factory-claims.json
```

## Destination-room authorization probe

A live Postgres probe temporarily removed Bob's only room membership, called the preflight endpoint
with an otherwise valid factory request, and restored the membership in a `finally` path.

Result at `2026-09-04T00:45:44.602300Z`:

```json
{
  "status": 403,
  "body": {
    "error": "forbidden: actor is not a member of any room in this Corp"
  },
  "removed_memberships": 1,
  "factory_work_items": 0,
  "missions": 0,
  "tasks": 0,
  "runs": 0,
  "membership_restored": true
}
```

Machine-readable output:

```text
output/issue128-room-preflight.json
```

## Complete local ship gate

Command:

```powershell
node tools/check_migrations.mjs
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm build:web
pnpm lint:web
```

Result: **passed** on September 4, 2026.

- 31 append-only migration checksums passed.
- Clippy passed for every workspace target with warnings denied.
- All 114 Rust unit tests passed; all doc tests passed.
- The production web build completed successfully.
- Web lint completed with no findings.
- Two independent read-only final reviews returned `APPROVE`.

## Safety conclusions

- Preflight is mutation-free.
- GitHub Project state changes only after mission linkage.
- No claim token appears in snapshots, events, or CLI output.
- A failed materialization cannot silently leave the same claim generation stranded.
- Reclaim still rejects source-revision and policy replacement.
- Auto-merge, merge, and deployment remain disabled and separately authorized.
