# Factory quota-aware intake — September 6, 2026

Work: #157; parent outcome #63. Hosted Actions were not used.

## Implemented boundary

- Minimal complete GraphQL pagination, with non-archived items, opaque cursors,
  count/identity checks and explicit 10,000-item/100-page/16-MiB bounds.
- Bounded server lookup batches, and exact live source/Project revalidation
  before effects. A cached issue number is only a discovery hint.
- Actual GraphQL cost/remaining/reset observation, a request reserve, fresh
  observations and primary/secondary/temporary-upstream retry classification.
- Retry-After/reset-aware waits with local escalation; a healthy primary window
  does not unnecessarily delay a secondary limit until its hourly reset.
- Epoch-fenced durable polling state. Restart/Resume/Reconcile cannot shorten
  an outstanding wait. Heartbeats and local work continue independently.
- Same-lineage catch-up from blocked to verified, retaining the existing server
  gate requiring the original completed mission and passed task verification.
- Visible retry reason/time and quota in the Factory cockpit.

## Local checks

- 36 immutable migrations.
- Strict workspace/all-target Clippy and formatting.
- 248 Rust tests passed in the serial workspace run.
- 17 new polling presenter tests plus 29 existing office/runtime tests.
- 12 subprocess tests for the fake GitHub quota/pagination boundary.
- Web build/lint.

## Controlled live-stack observations

This lane uses an isolated development server and real runner process with an
explicit deterministic fake worker/GitHub boundary. It is not Copilot inference,
production identity or an external GitHub publication claim.

The complete fixture Project contains 1,002 non-archived items, including a target
after the first 1,000 and unrelated/non-Issue entries. Each complete pass uses
11 minimal pages; the legacy `gh project item-list` path is never used.

A quota-only response consumed the fixture's last point. The controller exposed
remaining zero and persisted its wait. An actual controller restart retained
the deadline; an operator's Reconcile request did not cause an early GitHub
request. The primary-quota notice was rendered at desktop/390px with no
overflow or browser errors.

The subsequent original factory mission was:

| Object | Identity |
|---|---|
| Controller | `06f56b6d-29e8-4866-8f99-e305fb696608` |
| Factory item | `3e7bb485-6f92-4486-9bc3-fda2d1b54fcb` |
| Mission | `1dd541aa-9820-49ea-b2e9-65ecc4d2c46b` |
| Run | `64900af1-7a44-44fc-8073-f9ac044d5292` |
| Source commit | `79e5c432ea38b0b288597d9d87594face0212cfc` |

A controlled secondary-limit failure at the pre-effect boundary moved the
existing item to blocked while the worker continued. Its permitted retry was
`2026-09-06T21:31:40.648323400Z`. The authoritative `run.completed` event is
`2026-09-06T21:30:59.083412Z`: local work and verification finished during
the external wait.

The resumed controller reconciled that same item to verified after the retry
window, retaining exactly one mission, one task, one run and one task attempt.
The timestamped fake-GitHub log contains no post-failure request before the
required retry. No manual database correction or replacement work was used.

A separate controlled 503 showed the correct future retry, healthy primary
quota and increasing failure count at desktop/390px. It created no new work.

A separate after-claim source-drift case changed the fixture issue revision/body
at the exact pre-effect read. The existing authority checks blocked that new
test item before dispatch or a Project status mutation. Its planned mission
remained ready, and the server's run set did not change. This negative case is
distinct from the successfully recovered original mission above.

## Read-only real GitHub check

The candidate's broad, unselected-issue `factory --dry-run` read the real ECorp
Project while restricted to the already-reviewed private Arcade Lab repository.
It returned no eligible issue and `mutations: []`. Its last GraphQL response
reported cost 1 and 4,961 points remaining. This is last-query telemetry, not a
claim that the entire discovery pass costs one point.

## Retained failures and evidence limits

The initial monolithic test did **not** pass. Browser capture consumed the
window intended for a restart assertion. A continuation then incorrectly read
stale controller metadata; another phase exposed an omitted repository field
in the test fixture. No mission or run existed in those attempts. They were
preserved, and corrected phases reused the authoritative test stack/controller
rather than resetting its history.

The first secondary-limit browser helper also incorrectly required a zero
quota even though secondary throttling can occur with a healthy primary quota.
Its failed report remains preserved. The reason-specific assertion was
corrected; primary zero rendering and the later 503 desktop/390px notice were
separately observed. Do not describe all old harness reports as green.

Fresh QA provisioning was blocked by tool policy. That setup was not retried
through another route. The already-owned stack remained available and was
reused after its prior controller process was verified stopped.

Evidence root:

```text
C:\Users\shyamsridhar\.codex\dogfood\issue157-quota-20260906
```

Notable records:

- `checks/e2e-fresh-observation/polling-evidence.json`: low-quota, restart and
  forced-reconcile evidence; later fixture error retained.
- `checks/e2e-corrected/polling-evidence.json`: original mission/run and
  secondary wait; browser-helper failure retained.
- `checks/e2e-recovery/recovery-evidence.json`: passed exact-lineage catch-up.
- `checks/ui-unavailable/notice-evidence.json` and `report.json`: passed 503
  notice and unchanged work IDs.
- `checks/live-discovery-dry-run.json`: read-only real GitHub result.
- `checks/source-drift/source-drift-evidence.json`: passed after-claim drift
  denial with zero added runs.

The existing manual UI, real Copilot game proof, unrelated main-checkout work
and recovery worktrees were not changed by this validation lane.
