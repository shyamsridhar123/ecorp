# Checkpoint cancellation reconciliation

Date: September 9, 2026. Issue: #206.

## Outcome and boundary

ECorp now has a narrow native path to recover a saved checkpoint that an older
Factory controller incorrectly projected as terminal cancellation. The path does
not reopen operator cancellations or start a coding agent.

This report distinguishes reconciliation, verification, outcome review and
publication. Passing the first step does not establish the remaining steps.

## Implementation

- The existing exact recovery-context read exposes an optional
  `checkpoint_cancellation_event_id` only for the currently proven cancellation.
- `POST /api/corps/{corp}/factory/work-items/{item}/checkpoint-reconciliation`
  requires current recovery authority, exact source/run/checkpoint/revision and
  Factory event/version bindings, a reason and an idempotency key.
- Native source authority verifies measured budget suspension, provider
  termination and preserved lineage. Actor authorization remains held through
  commit; a quarantined predecessor is not ignored.
- The controller cancellation must match the original native operation and
  preceding run cancellation. Key formatting is provenance recognition, not
  authentication. Unknown or newer events cannot substitute for that proof.
- The transaction changes only the Factory projection, increments its version
  and appends `factory.checkpoint_cancellation_reconciled`. It returns no claim
  token and does not rewrite original provider history, budgets or attempts.
- Only explicit `checkpoint-verification` invokes reconciliation. It refreshes
  context before using existing claim/recovery operations. Dry run is
  mutation-free; generic polling and other recovery modes never reopen cancelled
  items.
- Future controller catch-up recognizes native checkpoint recovery before
  mirroring cancellation. The UI exposes the existing copied checkpoint command
  rather than an inappropriate provider-resume action; it does not execute that
  command itself.

## Local checks

Canonical source was the reviewed working tree on
`codex/issue206-checkpoint-cancellation`, based on
`f61d1f60adf088878a08ef3681e6df284fcd1995`.

| Check | Observed result |
| --- | --- |
| Migration integrity | Passed |
| `cargo fmt --check` | Passed |
| Workspace/all-target Clippy with warnings denied | Passed |
| Rust workspace tests | 446 passed, 0 failed, 200 explicitly ignored |
| Web production build and lint | Passed |
| Scoped client/recovery/controller Node tests | 42 passed, 0 failed |
| Focused actual-migration `issue206_` store tests | 11 passed, 0 failed; 40.43 seconds in the test harness |
| Whitespace and reviewed source hashes | Passed |
| Server, runner and CLI binary build | Passed |

The full source gate and binary build completed in retained execution session
71561. Gate receipts are under
`C:\Users\shyamsridhar\.codex\dogfood\remaining-work-20260908\issue206-native-checkpoint-reviewed-20260909T154658334`.
Focused SQLx and client receipts are under
`C:\Users\shyamsridhar\.codex\dogfood\issue206-native-checkpoint-20260909`.

The eleven store cases include positive reconciliation followed by native
verifier-only recovery, explicit operator cancellation/stop, policy cancellation,
missing native termination/budget proof, exact version/event/revision/checkpoint
binding, current authority and replay, deterministic actor-lock serialization,
quarantined predecessor rejection and late-event rollback. They use SQLx-owned
disposable databases under the approved maintenance loader, not the application
database. Older ignored acceptance families were not rerun.

An earlier nine-case pass remains retained. Independent review then found an
actor-demotion race and a quarantined-predecessor gap; both were fixed before the
eleven-case run. The earlier full gate stopped on two test-only Clippy findings;
its failure log remains retained rather than being replaced with the passing run.

## Retained application

The existing real-Copilot VendorDesk Intake lane is still the acceptance target:

- Repository/issue: `shyamsridhar123/ecorp-enterprise-lab#4`.
- Factory item: `e6182bf4-0e41-420b-ac03-23a11c5619c5`.
- Mission: `290d72c8-50e8-4c31-8947-fcf45e587317`.
- Original integration run: `cd10992f-ce28-4f33-accf-de2b99eeb31f`.
- Saved execution connection: `a0604308-737f-4554-9e24-49493a427991`.
- Source commit: `e3dc3d669b1a99832e2e7af9be16f7f39842586d`.
- Original GitHub source revision: `2026-09-09T13:20:11Z`.

Before upgrade, the actual API still reported Factory `cancelled`, version 60,
and `checkpoint_verification: true`. The existing browser showed the cancelled
item and a deliberately paused intake controller. The Codex goal itself was
active, not blocked.

The original integration has no linked provider artifact. Its persisted
artifact check must therefore be observed, not bypassed or reported as passing
based on source-file existence. The three earlier specialist completions do not
establish final application acceptance.

## Actual native runtime result

The reviewed server, runner and web client replaced only the existing owned QA
processes on ports 18574/15574. The native restart helper verified the original
mission/task/run records and original event prefix were preserved. It started no
additional container or application mission and retained native account profiles.

The explicit CLI dry run selected the original issue, reported
`checkpoint_reconciliation_needed: true`, and returned `mutations: []`.
Execution of that same request then:

1. Reconciled the exact recorded cancellation from Factory version 60 to 61;
   journal sequence 1775 is `factory.checkpoint_cancellation_reconciled`.
2. Created recovery `138b71dd-4c05-4c04-a449-3240bf1b971c` and verifier-only run
   `0c131a28-6027-44c3-bfb7-51ff82eba8f3` in the original mission/task.
3. Reused the original workspace run, preserved fingerprint, source commit and
   saved connection. The original integration record remained byte-equivalent
   after JSON serialization of the before/after API reads.
4. Retained 14 missions and added exactly one run (18 to 19). The child has no
   provider session and zero input/output tokens and model cost. Its observed
   journal is verification/cleanup, not a new provider session.
5. Ran all four persisted checks against the retained source. Both file checks
   passed. The actual HTTP application test command passed all four test groups,
   with no skipped or cancelled tests.
6. Correctly failed the required artifact check with `stored: false`. The final
   Factory state is `verification_failed`, version 68, not cancelled or verified.
   The source was preserved with the same fingerprint.

The real browser updated to **Verification Failed — 1 of 4 verifier checks
failed**, with no pending approval or published pull request. This proves the
dead-end reconciliation and runner verification path, **not an accepted
application or successful outcome review**. Intake remains deliberately paused
while that same application's acceptance is completed.

Receipts include `native-context-before-reconciliation.json`,
`native-checkpoint-dry-run.log`, `native-checkpoint-execute.log`, and
`runtime-checkpoint-observation-20260909T160943781.json` in the focused evidence
directory above. Earlier failed previews and the original cancellation evidence
remain retained. The original source issue was not edited or commented on.

## Remaining application acceptance

Independent read-only source review found three UI issues in the retained
application; these are not claimed as browser reproductions:

- Late list/detail responses can render after logout and a different tenant's
  login because rendering is not bound to the current session/navigation.
- The approval submit handler treats its form as the button and replaces the
  form's contents while the decision request is pending.
- Maximum-length unbroken input can overflow detail/list layouts at 390 pixels.

The tests exercise real HTTP handlers and temporary persisted data but do not
cover direct startup or these browser paths. Those checks, the missing required
provider artifact, independent outcome review and review-only publication remain
open. The next correction must use the existing scoped recovery contract on this
same application; no handwritten repair or fabricated artifact is substituted.
Issue #206 remains open while these end-to-end acceptance requirements remain.

No replacement issue or mission, manual application-data repair, blanket policy
exemption, merge, auto-merge or deployment was performed.
