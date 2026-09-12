# Owned-work review checkpoint: #50, #235 and #236

This is a dated evidence/hand-off snapshot, not a second backlog. All-The-Vibes
Project #5 and its linked issues remain the planning authority. No GitHub state
was changed while preparing this note.

## Verified ownership and scope

The GitHub open-issue search for `assignee:Bakar404` returned exactly these three
items. Project #5 had 148 items; the three relevant statuses were read directly.
No open PR authored by Bakar404 was found in this repository.

| Item | Board status | Actual work and boundary |
| --- | --- | --- |
| [#50](https://github.com/All-The-Vibes/ecorp/issues/50) budget recovery | Todo | PR #77 shipped the original feature. The reopened Factory regression is under investigation; our contribution is test tooling and evidence, not a replacement budget implementation. |
| [#235](https://github.com/All-The-Vibes/ecorp/issues/235) MAF contributor proof | In Progress | Its existing live lineage is stopped and preserved. The scenario was not implemented by that run. A new budget/recovery decision is needed before another provider attempt. |
| [#236](https://github.com/All-The-Vibes/ecorp/issues/236) audited extensions | Todo | Separate, unimplemented feature, explicitly blocked by #50. Its 1M initial allowance, 70% audit and bounded extensions are policy targets, not active settings. |

#235 does not automatically acquire a dependency on #236 merely because its
original allowance was small. A bounded human-authorized continuation using
existing controls and an unattended auditor feature are separate decisions.

## Local contribution inventory

- Branch: `codex/issue-50-factory-recovery`.
- Base: `b28fd4d26309794f38c0455bbf42d22aedf7cfd1`.
- A live GitHub comparison reported that base identical to `main`, with zero
  commits ahead/behind at the time of this review.
- Running checkout: unchanged, clean `971445e1cbf9388c51803e2adf28b11bd98b1ffa`.
- Authored changes are confined to synthetic provider/test helpers and dated
  evidence under `scripts/`, `tools/`, and `docs/evidence/`.
- No production Rust, web application, migration, root lockfile or BACKLOG change.
- No commit, push, PR, issue closure, Project update, merge or deployment performed.

## Harness review fixes

Before considering publication, the locally authored harness was hardened:

1. `--dry-run` and `--execute` together now fail closed. Default mode remains a
   dry-run; unknown and incompatible scenario flags are rejected.
2. A manual server is no longer an implicit prerequisite. Optional retained-state
   comparison uses `ECORP_ISSUE50_REFERENCE_SNAPSHOT_URL`, with only an exact
   loopback Corp snapshot and actor UUID allowed. Credentials, other query fields,
   remote hosts, redirects and the QA port are rejected. Its implementation has
   only a GET path; QA mutation requests are separate.
3. Access-denied filesystem checks are not interpreted as absence. Existing QA
   root/parent redirection is checked before creating an attempt directory.
4. QA HTTP redirects are rejected rather than following an unexpected endpoint.

Four configuration tests cover these boundaries in addition to the eleven
synthetic provider-protocol tests. The standalone dry-run succeeded without a
reference server and reported `retained_comparison: not_requested`, not a false
claim of retained-state verification.

## Required local gate results

Executed in the contribution worktree:

| Check | Result |
| --- | --- |
| `node tools/check_migrations.mjs` | Passed: 40 immutable migrations |
| `cargo fmt --check` | Passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed, no warnings |
| `cargo test --workspace` | Passed: 452 executed, 200 explicitly ignored |
| `pnpm build:web` | Passed |
| `pnpm lint:web` | Passed |
| `node --test tools/factory_budget_fixture_config.test.mjs tools/fake_codex_budget_stream.test.mjs` | Passed: 15 tests |
| JavaScript / PowerShell helper syntax; `git diff --check` | Passed |

The test command was run with no ambient `DATABASE_URL`. The 200 ignored tests
require an explicitly owned SQLx maintenance database (4 server and 196 store
tests); their coverage is not claimed. The executed Rust counts are CLI 96,
domain 8, gateways 2, protocol 6, runner 190, server 95 and store 55. Test listings
were used to verify totals after verbose output truncation.

Web dependencies were installed from the existing frozen lockfile with lifecycle
scripts disabled: `pnpm install --frozen-lockfile --ignore-scripts`. No root
dependency lockfile changed. No hosted CI or new browser acceptance is claimed.

## Final native QA revalidation

The same previously approved isolated missing-checkpoint scenario was rerun with
the hardened harness and an explicitly supplied read-only reference snapshot:

```powershell
$env:ECORP_ISSUE50_QA_ROOT = 'C:\Users\aabdelsalam\.ecorp\qa\issue-50-factory-recovery'
$env:ECORP_ISSUE50_PG_BIN = 'C:\Users\aabdelsalam\.ecorp\pg\pgsql\bin'
$env:ECORP_ISSUE50_REFERENCE_SNAPSHOT_URL = 'http://127.0.0.1:8791/api/corps/00000000-0000-4000-8000-000000000001/snapshot?actor_id=00000000-0000-4000-8000-000000000011'
node tools/e2e_factory_budget_recovery.mjs --dry-run --missing-checkpoint
# Previously approved QA-only scope; no live-provider or publication authority.
node tools/e2e_factory_budget_recovery.mjs --execute --missing-checkpoint
```

Report: `C:\Users\aabdelsalam\.ecorp\qa\issue-50-factory-recovery\attempts\20260912052704787\report.json`.

Result: `recovery_passed`. The native checkpoint reader failed on the locked
unchanged synthetic README; no original proof was fabricated. Approved budget
revision plus explicit same-session/workspace resume completed verification and
independent review. Retained reference state was unchanged, the helper exited,
all three QA services were stopped by verified receipts, and QA ports were empty.
The earlier captured-checkpoint and hard-stop reports remain historical evidence
for their recorded harness revisions; those two scenarios were not repeated here.

## #50 acceptance assessment

Already implemented by merged PR #77: owner/admin revisions, original/current
budget ledger, consumed/remaining accounting, narrower finish scope, pre-start
budget checks, durable decisions, UI controls, and hard-stop lineage fencing.
Do not reimplement these or keep #50 open solely because #236 is not implemented.

New local evidence establishes Factory continuation when the item is still
`running`, both with and without a successfully captured original checkpoint.
It also establishes duplicate/role/self-review rejection and hard-stop protection
in the documented synthetic cases.

Not established by these tests:

- Provider continuation after the Factory item has already become terminal
  `cancelled`, as in the September 3 reopening report.
- Recovery after the distinct adapter-evidence-read error seen in the first file-lock
  attempt, where reconciliation changed Factory `blocked` to `failed`.
- Lost-response/reconnect behavior across the whole Factory recovery sequence,
  current browser acceptance, or independent review of our contribution.

These are specific closure/review questions. The tests are not a basis for claiming
all reopened cases fixed or for changing terminal-state guards speculatively.

## Proposed next delivery actions

1. Present this non-closing test-and-evidence contribution for review under #50.
   Proposed PR title: `test(factory): cover budget recovery and missing checkpoint proof`.
   Use `Related to #50`, not `Closes #50`. Commit/push/PR creation await their review
   and approval; merge and deployment remain disabled.
2. Reconcile the exact September 3 terminal-Factory case with current #206 behavior.
   Add a bounded regression only for the remaining distinction, then make a
   demonstrated scoped fix if necessary. Do not turn the new auditor feature into
   an additional #50 acceptance criterion.
3. Keep #235's original session/worktree/usage intact. Its board entry may remain
   In Progress, but its stopped state needs an explanatory update before anyone
   mistakes it for live work. Any live continuation needs an explicit budget and
   scope preview; increasing only mission authority leaves the 200K task cap.
4. Curate #236's separate branch, shared-file allocation, verifier and implementation
   budget after its #50 dependency is resolved or explicitly narrowed by a reviewed
   decision. No automatic dependency removal or `factory:ready` change is implied.

The local UI remained HTTP 200 at `http://127.0.0.1:5187`, with one connected
runner, zero active runs, zero controllers and zero publication attempts.
