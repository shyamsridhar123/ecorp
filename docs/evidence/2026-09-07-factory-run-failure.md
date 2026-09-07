# ECorp ordinary Factory failure and native resume coherence

**Date:** September 7, 2026

**Issue:** #169

**Delivery:** `codex/ui-enterprise-journey-audit`, draft PR #170, stacked on #162.

**Status:** Implemented, independently reviewed and actual-store verified.
**Not yet proven:** A new browser/server/runner missing-or-delayed-ack fault drill.
Do not close the issue on SQLx coverage alone.

## Verified gap and implementation

An ordinary post-dispatch `run.failed` could exhaust task retries and fail its
mission without updating the owning Factory item. The existing one-shot
reproduction left Factory running after source-deliverable acknowledgement
timeouts. Another repository's watcher must not be required to repair that state.

The store now reconciles an exhausted ordinary execution failure transactionally:

- Persist a bounded, normalized diagnostic and an explicit `ordinary_run_failure`
  origin in `factory.blocked`. This is not `verification_failed` and does not
  make the generic terminal `factory.failed` state resumable.
- Preserve retryable work, evidence, source identity, budgets, usage and history.
- Reject stale, superseded, foreign or duplicate failures without downgrading
  accepted work or changing another run's agent state.
- Keep unrelated source/policy blocks and verified/publication outcomes protected.
- Reuse the native same-mission resume path. It may clear only the exact current
  failure origin, with matching run, mission, diagnostic and authority.

This is ECorp-level persistence and coordination, not a replacement for the
underlying harness's tools, execution loop, permissions or session resume.

## Review-driven corrections

Independent static review found and the worker corrected two bounded edge cases.

### Lease-only versions must not invalidate a valid failure origin

A legitimate native claim renewal advances the Factory version without replacing
its failure block. Recovery now proves a contiguous chain from the latest
non-renew state event to the current item. Each allowed `factory.claim_renewed`
version must be backed by a matching native renew operation for the same Corp and
item, with the expected actor, fencing token and prior version.

Changed source/policy authority, replacement blocks, gaps, unknown events and
unproven revisions fail closed. Resume records the origin and resumed Factory
versions. This does not accept any historically similar diagnostic as authority.

### Reject an active descendant before waiting on its lineage lock

The existing active-task/agent rejection was moved before descendant lineage
`FOR UPDATE`, after the existing authorization/source checks. The focused
concurrency regression holds an active descendant's run row, confirms that an
ineligible parent resume rejects without waiting for it, then releases the row
and verifies that the descendant's actual output event proceeds.

This mitigates that particular interleaving. It is **not** a claim of global
deadlock freedom or an audit of every terminal-lineage/cleanup path.

The five low-frequency Factory-affecting runner event types, native resume and
verification decisions share the existing item/publication gate order before
run/task/mission rows. Streaming output and usage do not take Factory gates.

## Actual-store validation

The worker ran the approved credential-safe loader once for the final snapshot:

```powershell
& 'C:\Users\shyamsridhar\.codex\dogfood\issue163-acceptance-20260907\run-store-sqlx-169.ps1' -TestFilter 'issue169_'
```

It invoked:

```powershell
cargo test -p crony-store issue169_ -- --ignored --test-threads=1
```

**Final result:** exit 0; **23 passed, 0 failed, 0 ignored, 53 filtered out**;
52.47 seconds. Full case output is retained in the backend worker's tool
transcript, exec session `40066`. No additional SQLx invocation followed that
final run.

Coverage includes:

- exhausted, retryable, hard-breaker, empty-diagnostic and non-Factory failures;
- tenant/assignment fencing, superseded callbacks, idempotency and transaction rollback;
- preservation of accepted outcomes, unrelated blocks and stored evidence;
- exact native resume through verified completion without rewriting history;
- native renew/replay → resume → verified, and replacement-policy/unproven-chain negatives;
- real `run.usage` accounting exhaustion without reducing original budget or spend;
- manual-review positives, requester exclusion, role and room membership denials,
  decision replay and current-room rechecks;
- both named advisory gates for all five lifecycle events and review paths;
- active-descendant early rejection and ungated output/usage.

Earlier failing fixture runs remain in the transcript, including the 14/15 pass
whose setup violated `missions_original_budget_check`. The fixture was corrected
to exhaust usage through `apply_runner_event(run.usage)`. The original-budget
constraint, spend and production assertions were not weakened or reset.

Focused checks on the same source also passed:

- Scoped rustfmt and diff whitespace checks.
- `cargo test -p crony-store issue169_ --no-run`.
- `cargo clippy -p crony-store --all-targets -- -D warnings`.
- `cargo test -p crony-store --lib`: **43 passed, 33 opt-in tests ignored**.
  The 23 new database tests are included in those opt-in tests and were run
  separately by the approved loader, not claimed as covered by the default suite.

The parent subsequently ran all required workspace gates on the integrated source:
38 immutable migration checks, workspace rustfmt, all-target Clippy with warnings
denied, **342 passing Rust tests** with 33 opt-in database tests ignored, **99
passing frontend tests**, 14 helper tests, and web build/lint. No additional
ignored SQLx run was made. See the
[source-hashed validation receipt](2026-09-07-issue163-169-local-validation.json).

## Source and ownership

The worker and independent reviewer verified the same byte hashes; the parent
rechecked both before integration:

| File | SHA-256 |
| --- | --- |
| `crates/crony-store/src/lib.rs` | `1ed53ca42108ad9a1aaa0968251bc4bcb4220cc5b0f632a87d63423b076a7930` |
| `crates/crony-store/src/factory_run_failure.rs` | `937678e417dc1c7b5124b98a08f65df6486f9bff4ffd1b6584aa46097a8a0a27` |

The source base was `f2ecc8c1c3ec4b3966b36b41806fe397813ac4cf`.
The worker changed only those two store files and made no commit or push.
The parent separately owned the #163 frontend/fixture work.

SQLx used the owned maintenance database `crony_issue169_20260907` and its
invocation-owned test databases in the already-approved PostgreSQL container.
Credentials were loaded into process environment, not prompts, arguments or
logs. Ambient `DATABASE_URL` was not used for a connection. No manual app
database, provider, browser, service or container lifecycle operation was part
of the worker's validation.

Contributors must provision their own isolated maintenance fixture; this local
loader path is an evidence receipt, not a shared credential or database contract.

## Remaining acceptance

The real missing/delayed source-acknowledgement path must still be exercised
through a newly built server, runner persistence and the UI, including one-shot
intake and authorized recovery. SQLx event application does not prove transport
failure handling or signed object bytes. The existing failed real-provider
history and subsequently accepted application remain preserved.

No hosted Actions result, runtime rollout, merge, auto-merge or deployment is
asserted by this report.
