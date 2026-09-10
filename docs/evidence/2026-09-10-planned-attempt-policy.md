# Prospective task-attempt policy

September 10, 2026. Implementation and offline validation for #224, on top of
the #221 correction-retry candidate. **Public runtime acceptance is pending.**

## Product behavior

The existing planner already permits at most three attempts, but its public
entry points previously exposed only the strategy defaults. The new optional
`max_task_attempts` uses that existing field and validation, not another retry
engine.

- Direct creation and preview accept integers 1-3 before execution. The native
  CLI exposes `--max-task-attempts`; MCP, ACP and A2A propagate the same option.
- An explicit choice applies to every planned task. Normal strategies retain
  default 2 and deterministic verification/review strategies retain default 1
  when it is omitted.
- Factory policy and preflight/materialization must carry the same choice.
  Actual allowances appear in the constrained preflight task projection.
- Existing claims cannot acquire or change this policy through continuation.
  CLI omission reuses the recorded choice. Legacy policy/operation JSON does
  not gain a null field merely because the client or server was upgraded.
- The allowance includes the initial execution. It does not increase token or
  cost authority, change provider retries, add approvals, reset accounting, or
  relax source, workspace, room, actor or verification requirements.
- Post-creation commands are not attempt setters. A focused revision-contract
  decoder rejects a nested `max_task_attempts`, including null, without making
  the general `TaskContract` decoder strict.

The existing scheduler, task counter and verifier-only accounting are reused.
No database migration or provider-adapter behavior changed. The only lockfile
change adds the already-present local `crony-domain` dependency to gateways;
no registry version was upgraded.

## Observed local validation

All checks used the isolated #224 worktree and recorded source/log hashes.
Hosted Actions is not the acceptance dependency.

| Check | Result |
| --- | --- |
| Full workspace no-run compilation | Passed |
| Focused initial workspace cases | 34 passed, 3 SQLx cases intentionally ignored |
| Focused protocol rerun after nested-field review fix | 4 passed, including one new case |
| Actual-store SQLx, real migrations | 3 passed, 0 failed; 6.77 seconds test time |
| Full serial Rust workspace | 538 passed, 0 failed, 320 intentionally ignored |
| Web | 233 tests passed; build and lint passed |
| Formatting and workspace/all-target Clippy | Passed |
| Immutable migrations | 41 checked; unchanged |
| Native binaries | Six compiler-reported executables built and hash-recorded |
| Public driver syntax and bounded source review | Passed; not execution |

The focused unit totals represent 35 distinct new cases; the protocol rerun
overlaps three earlier cases. Ignored tests are not counted as passes.
The SQLx cases cover prospective persistence/replay, invalid/mismatched
preflight and claimed-work rejection without ledger changes, and legacy
omission without retrospective override. They do not prove public runtime
reachability or provider behavior.

Retained receipt root: `issue224-planned-attempt-policy-20260910`.
Principal receipts are:

- `compile-lock-aligned-20260910T203532835/result.json`
- `focused-unit-20260910T204124609/result.json`
- `focused-sqlx-20260910T204202197/result.json`
- `protocol-nested-guard-20260910T204754976/result.json`
- `clippy-workspace-20260910T204829423/result.json`
- `native-build-20260910T205654611/result.json`
- `full-workspace-tests-20260910T210600440/result.json`
- `web-20260910T201755586/result.json`

## Remaining public acceptance

The guarded Windows driver is `tools/e2e_planned_attempts.mjs`. It requires
an explicitly owned local server at `http://127.0.0.1:19084`, a matching public
context and an existing evidence directory outside the checkout. It does not
start services, inspect a database, reset fixtures, change counters, or read
provider homes/worktrees.

Its intended single-mission sequence is:

1. Preflight, claim and materialize with allowance 3 before any run.
2. Use a native pre-run revision to narrow the task budget to 5,700 within a
   10,000-token mission; 6,000 fixture usage then suspends rather than hard-stops.
3. Run the original provider-mode attempt and its provider-free checkpoint
   verifier. Missing application checks remain failures.
4. Authorize a first source correction that genuinely fails the unchanged
   verification policy after provider completion.
5. Select that latest failed source in a new current revision, authorize the
   final ordinary attempt, and let the existing application fixture satisfy
   the same checks.
6. Require Bob's independent evidence decision, Factory verified state,
   remaining attempts 0 and unchanged original history/source bindings.

This is an intended native **Codex protocol-fixture** lane, not real vendor
inference or session-persistence proof. Its first correction is a
`verification_failed` outcome, not an ordinary transport `run.failed` test.
Client-side independent cryptographic verification and a browser journey are
not claimed by the driver.

The isolated startup request was denied by the host execution policy before
PowerShell/process creation. Readback found no QA listeners or started service
records. No alternate launch route was attempted. The existing manual ECorp
instance remained healthy and was not used as a substitute.

Consequently #224 and #221 remain open, and this change must remain a draft
until the actual public lifecycle is executed and inspected. A compiled
driver, admitted store fixture or passing workspace suite is not that proof.

## Retained failures and review correction

The first `--locked` compile rejected the missing local dependency lock entry.
Offline Cargo metadata then added exactly one local dependency line; the
subsequent locked compile passed. The failed receipt remains retained.

Review identified that top-level request strictness did not reject
`contract.max_task_attempts`. The targeted nested decoder and valid-payload,
3/null rejection and unrelated-annotation compatibility controls address that
finding. The focused protocol rerun and final full workspace gate passed on
the corrected code. The public no-mutation check remains part of the
unexecuted driver.
