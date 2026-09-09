# Source correction after failed checkpoint verification

Date: September 9, 2026. Issue: #210. Parent acceptance: #206 / #145 / #164 / #63.

## Scope and status

The correction extends the existing native recovery path so that a coding agent
can finish its saved application after a checkpoint verifier exposes missing work.
It does not invent an artifact, replace a mission or bypass a stop. Source-only
verification still starts no provider; explicit source-correction consumes normal
remaining provider authority.

Implementation, fixture evidence, actual application behavior, review and
publication are separate claims. **Integrated local checks have passed; final
application/runtime acceptance remains pending until observed below.**

## Reproduced native failure

The retained application is lab issue `shyamsridhar123/ecorp-enterprise-lab#4`,
Factory `e6182bf4-0e41-420b-ac03-23a11c5619c5`, mission
`290d72c8-50e8-4c31-8947-fcf45e587317`. The original integration is
`cd10992f-ce28-4f33-accf-de2b99eeb31f`; its checkpoint verifier is
`0c131a28-6027-44c3-bfb7-51ff82eba8f3`.

The explicit native source-correction preview selected the original issue and
returned no mutations. Execution persisted contract revision
`3b049aca-8645-4ddb-a096-be765be051e7` (version 2), then returned HTTP 400:

> factory verification recovery cannot bypass a suspended, stopped, or quarantined lineage

No provider run was created. The Factory recorded the failure as blocked,
version 74. Original source/history/usage remained. The original mission had
176,294 tokens and one provider attempt remaining; its native source origin
was suspended, not explicitly stopped.

## Narrow implementation

- Migration 0041 adds separate optional `source_correction_authority`; the
  checkpoint-mode constraint in migration 0039 is unchanged.
- Server-derived provenance binds the failed checkpoint recovery, exact native
  suspension incident, original checkpoint and termination evidence, source,
  historical and replacement contract/policy digests, immutable Factory policy,
  saved execution connection and original provider session.
- The existing source-correction operation remains explicit and revision-bound.
  Only its one proven historical suspension is admitted. Current attempts,
  cumulative mission/requester/Corp allocation, stops, loops and quarantine
  retain their normal enforcement.
- Dispatch uses the existing connection-admission logic read-only, repeats
  authority before/after secret resolution, and re-reads command/Factory state
  after row waits while holding native mutation gates.
- Retryable authorization database errors keep the existing durable command
  pending instead of consuming a final attempt as an execution failure.
- Replay and ordinary publication revalidate correction provenance. Missing
  provenance returns a normal denial, never accepted publication.
- Recovery context keeps historical checkpoint proof separate from a valid
  revised policy and already-admitted correction. The browser exposes only
  server-proven modes and uses the exact source run for its contract editor.

## Retained red test

The baseline `issue210_initial_correction_resumes_after_failed_checkpoint` ran
before the product hook changed, at base
`77c10fbbec31fa7954a2ddcdc65dc8e8eb3670c7`. It reached native suspension,
checkpoint verification failure and a public current contract revision, then
failed at the intended source-correction admission: **0 passed / 1 failed**.
The harness finished in 2.68 seconds; the loader including build took 93.34 seconds.

The synthetic fixture declares its initial mission/run limits and attempt count
before any usage: mission 10,000, run 5,000, usage 5,300, attempt 1 of 2. It does
not lower/reset original budgets, spend or attempts after suspension. It uses a
non-null saved connection and rejects a false artifact-pass claim.

Receipts are in
`C:\Users\shyamsridhar\.codex\dogfood\checkpoint-source-correction-20260909`,
with prefix
`20260909T164646514-issue210_initial_correction_resumes_after_failed_checkpoint`.
The earlier real CLI failure remains in the `issue206-native-checkpoint-20260909`
directory and was not replaced by this fixture.

## Actual browser baseline

The original agent-authored `server.mjs` started on loopback port 18576 using
separate host-owned synthetic demo data outside the preserved worktree. No
application source was edited and no Docker container was added.

Real browser interactions verified login, invalid form handling, draft creation,
submission and a reviewer decision. They also reproduced all three review findings:

1. The approval form disappears while its actual decision request is pending.
2. Accepted unbroken input produces a document width of **8,994 pixels** at a
   **390-pixel** viewport.
3. An actual delayed authenticated Alpha list response renders Alpha content
   after logout and Beta sign-in. This is a browser stale-response disclosure,
   not an API tenant-authorization bypass.

The browser produced no script errors during that reproduction. The receipt is
`browser-before-1788973435812.json`; screenshots use the
`application-decision-before`, `application-mobile-before` and
`application-session-race-before` names. The preview process was then stopped
through its exact ownership receipt. Synthetic data was retained for restart
checking; the ECorp QA stack and original integration record were unchanged.

These observations establish the defects, not their repair.

## Final checks and retained application acceptance

### Actual-store regression results

The full 75-case SQLx run produced **72 passed / 3 failed** in 384.03 seconds.
All three failures were fixture construction errors against the real schema:

- An attempt count above `max_attempts` is itself forbidden by the database.
  The corrected test now proves that rejection leaves accounting unchanged and
  does not invalidate the already-authorized last attempt.
- The unrelated rolling-spend task needed its required `plan_key`.
- `applied` is not a native runner-command status. The corrected test uses the
  actual `acknowledge_runner_command` operation.

Only those three corrected fixtures were rerun, serially, through the same
approved loader. Each passed (5.59, 5.68 and 5.18 seconds in the harness).
**All 75 final cases therefore have passing evidence across the full run and
focused reruns**, not one claimed 75/75 invocation. Product code did not change
for those fixture corrections. The earlier compile failure in the last added
test (`Uuid` versus `Option<Uuid>`) and all prior failed logs remain retained.

Coverage includes the positive preserved-session continuation, ordinary healthy
source-correction compatibility, current role/room/connection/source/policy,
stops/loops/quarantine, positive provider allocation and consumption, replay,
rollback, missing/damaged provenance, native Factory-block serialization during
a connection wait, and context before/after a valid narrowed-policy correction.
The direct publication-provenance guard case is not a full publication E2E.

Independent source review found and verified fixes for stale state after row
waits, retryable database errors being terminalized, missing-publication-proof
panic, and revised/active correction context. These are scoped findings and
tests, not a claim of global deadlock freedom.

### Remaining runtime acceptance

The reviewed full workspace gate completed successfully at
`C:\Users\shyamsridhar\.codex\dogfood\checkpoint-source-correction-20260909\full-gate-20260909T174427544`.

| Check | Observed result |
| --- | --- |
| Migration integrity | 41 append-only migrations, passed |
| Formatting | Passed |
| Workspace/all-target Clippy, warnings denied | Passed |
| Rust workspace suite | 447 passed, 0 failed, 275 explicitly ignored |
| Integrated checkpoint-recovery client file | 23 passed, 0 failed |
| Web build and lint | Passed |
| Whitespace and source-hash stability | Passed |
| Server, runner and CLI binaries | Built successfully |

The client worker also recorded 45 passing focused tests across its selected
client checks. The integrated gate's 23-case file is reported separately rather
than presented as the same invocation. Hosted Actions was not the acceptance
dependency.

### Real native continuation and browser correction

The existing QA stack was upgraded to the gated binaries, preserving its
original records and event prefix. No new container or application mission was
created. The actual recovery context exposed source-correction availability.

The native CLI reused the already-persisted version-2 contract revision and
created recovery `5519152c-a1c9-4b0d-903a-d85df3ce1ac9`, run
`945246da-b3ac-48d5-ac28-c9039c06de6e`, in the original mission/task. The real runner
acknowledged the command and emitted a genuine Copilot session/start:

- Provider session remains `3e326726-662a-4975-975e-6de1ae285729`.
- Workspace run remains `cd10992f-ce28-4f33-accf-de2b99eeb31f`.
- Connection remains `a0604308-737f-4554-9e24-49493a427991`.
- Source commit remains `e3dc3d669b1a99832e2e7af9be16f7f39842586d`.

Copilot read the existing files and applied the three UI fixes with its native
filesystem tool. No parent or review worker edited the application source.
During the final source check, its normal allocation was exhausted. The three
recorded usage increments were 65,720 / 80,284 / 82,744 input tokens and
520 / 2,245 / 639 output tokens. The run stopped at 232,152 against its
176,294 allocation; those recorded values were not discounted or reset.

The new run is cancelled with a recorded budget `stop`, native provider
termination and preserved checkpoint fingerprint
`cbdc96f6804697f6378ec9b6079af976548aaaa559edf03eed72ec02305a0091`.
Factory is `verification_failed`, version 82. Original history remains.

The corrected application then started from those exact retained files with the
same external synthetic-data directory. The real browser flow passed:

- Requester login, invalid-form handling, creation and submission.
- Reviewer login and a real approval with its form retained while pending.
- The same maximum-length input now fits a **390-pixel document in a 390-pixel
  viewport**, compared with 8,994 pixels before.
- The delayed Alpha response is discarded after Beta login.
- No browser script errors were observed.

Receipt: `browser-after-1788977264017.json`. The before/after browser evidence
proves these fixes, not accepted ECorp completion. The native run and usage
records are retained in `native-correction-stopped.json`.

### Remaining native evidence gap

The required provider artifact is still absent. Read-only adapter tracing
confirms that cancellation returns before the normal JSON-receipt writer, and
artifact callbacks are dropped after a hard boundary. The existing checkpoint
path preserves source but cannot collect/adopt missing provider evidence.

The pinned SDK has read-only session history/event-log methods, but they are not
wired into an ECorp collection operation here. Any follow-up must retrieve and
attest genuine native evidence without sending another prompt, continuing
pending provider work, fabricating completion, removing the required check or
altering the stopped history. No usage-overcount explanation is claimed for this
run: the SDK defines these notifications per API call.

Independent ECorp outcome acceptance and trusted review-only application
publication remain pending #211, which tracks native stopped-session evidence
collection without another inference run. No merge or auto-merge was performed.
