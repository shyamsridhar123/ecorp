# Prospective task-attempt policy

September 10, 2026. Implementation and local validation for #224, on top of
the #221 correction-retry candidate. The public drill now reproduces a native
compatibility defect and the focused fix passes store tests.
**Patched runtime and complete public recovery acceptance remain pending.**

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
| Initial planned-policy SQLx, real migrations | 3 passed, 0 failed; 6.77 seconds test time |
| Native-seal guard unit cases | 2 passed in the final workspace gate |
| Native-seal SQLx regressions, real migrations | 3 passed, 0 failed; 17.86 seconds test time |
| Final serial Rust workspace | 540 passed, 0 failed, 323 intentionally ignored |
| Web | 233 tests passed; build and lint passed |
| Formatting and workspace/all-target Clippy | Passed |
| Immutable migrations | 41 checked; unchanged |
| Native binaries | Six compiler-reported executables built and hash-recorded |
| Corrected public driver syntax and bounded source review | Passed; not patched runtime acceptance |

The prospective policy has 35 distinct focused unit cases; the protocol rerun
overlaps three earlier cases. The seal follow-up adds two separate guard cases.
The earlier 538-passed/320-ignored workspace receipt is retained; the final gate
took 529.58 seconds. Ignored tests are not counted as passes.
The initial SQLx cases cover prospective persistence/replay, invalid/mismatched
preflight and claimed-work rejection without ledger changes, and legacy
omission without retrospective override. They do not prove public runtime
reachability or provider behavior.

The later three SQLx regressions reproduce the native verifier event shape
without a redundant HEAD, then prove current context, required new revision,
admission, exact replay/dispatch and retained accounting. They reject an
explicit conflicting HEAD, missing/changed fingerprint and damaged verifier
authority. Event-shape setup and corruption are confined to SQLx-owned fixtures,
not the retained application history. The two pure guard cases additionally
distinguish omission from null/malformed fields and exclude required-head
provider sources. These are not replacement runtime or cryptographic tests.

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
- `legacy-verifier-seal-red-20260910T221220171/result.json`
- `legacy-verifier-seal-green-20260910T221510660/result.json`
- `seal-compat-format-20260910T221838559/result.json`
- `seal-compat-migrations-20260910T221736250/result.json`
- `seal-compat-native-build-20260910T221843118/result.json`
- `seal-compat-workspace-tests-20260910T222159515/result.json`
- `seal-compat-clippy-20260910T223102075/result.json`
- `web-20260910T221734995/result.json`

## Native runtime progress and remaining acceptance

The guarded Windows driver is `tools/e2e_planned_attempts.mjs`. It requires
an explicitly owned local server at `http://127.0.0.1:19084`, a matching public
context and an existing evidence directory outside the checkout. It does not
start services, inspect a database, reset fixtures, change counters, or read
provider homes/worktrees.

Its required single-mission sequence is:

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

The original isolated startup was denied before process creation. After the
user explicitly approved it, the same owned QA stack started successfully on
API19084/UI16084 with the already-prepared separate database and no new container.
The existing manual ECorp instance on API18962/UI15491 was not repurposed.

Three diagnostic driver invocations are retained, without resets or counter repairs:

1. `runtime/planned-attempts-20260910T215446977Z-138d33c3-bd52-4e84-a462-e6990f3073a4/0078-result.json`
   found that the driver omitted the native controller's post-launch
   `mission_created -> running` Factory transition. The driver now calls that
   existing actor/token/version-bound API; no recovery guard was weakened.
2. `runtime/planned-attempts-20260910T215756227Z-d2216c16-4187-4636-aa55-104814ad5d28/0112-result.json`
   reached the actual failed provider correction, but current correction
   availability incorrectly returned false.
3. `runtime/planned-attempts-20260910T215922811Z-32b23434-676d-41e8-81fb-9a9f97a37e25/0122-result.json`
   reached the next source-bound revision and exposed
   `historical request no longer matches its native source seal`.

The last two failures have the same cause: historical validation demanded
`head_commit` on an earlier verifier's preservation event, while the native
verifier emits its authorized fingerprint after checking both fingerprint and
HEAD but omits that redundant field. The authorized HEAD was still retained.
The pre-revision context caught the history error as unavailable correction;
the revised context propagated it. An intermediate availability hypothesis was
corrected, and the original positive driver assertion was restored.

The focused compatibility fix accepts only raw field absence for a
verification-only source with its valid exact authorized fingerprint. It keeps
the authorized HEAD, rejects explicit invalid/conflicting values and required-head
provider omissions, and retains the independent native-authority reconstruction.
It rewrites no historical event, request, grant or source. The actual-store RED
ran one test against unchanged product code and failed at the intended availability
assertion (3.99 seconds test time); all three new cases passed after the fix.

The retained third mission is `de811a71-fcb0-4c16-b016-04b8e3f899d4`, with
original run `50fedd24-a2b3-4377-bc81-14ef7a56743f`, verifier
`4be311c2-4ebf-47c3-ac02-f603db677232`, and failed correction
`1360e70e-15b9-48bc-b266-7467035974aa`. The separate QA browser observation
selected that failed correction and displayed its real 2/4 passing checks;
`native-seal-before.png` retains the pre-fix state.

The patched native binaries built successfully, but the subsequent owned QA-only
Stop/Start request was rejected by host policy before PowerShell creation.
`qa-patched-restart-denial-20260910T222157984.json` confirms the original QA stack
and manual application remained healthy; no stop, alternate launcher or upgrade
ran. The patched full public lifecycle, final Bob decision and Factory verified
outcome are therefore still unproven. #224 and #221 remain open and the PR remains
draft. Offline/store success is not substituted for this missing runtime result.

## Retained failures and review correction

The first `--locked` compile rejected the missing local dependency lock entry.
Offline Cargo metadata then added exactly one local dependency line; the
subsequent locked compile passed. The failed receipt remains retained.

Review identified that top-level request strictness did not reject
`contract.max_task_attempts`. The targeted nested decoder and valid-payload,
3/null rejection and unrelated-annotation compatibility controls address that
finding. The focused protocol rerun and final full workspace gate passed on
the corrected code. The retained public diagnostic drills also exercised the
nested-setting rejection and no-mutation checks. The later complete recovery
sequence against the patched server remains pending as described above.
