# Native checkpoint recovery accepted on the original application lineage

**Subsequent acceptance:** [native checkpoint-bound publication](2026-09-08-checkpoint-publication.md)
now covers this same bundle through real local Git, the controlled GitHub fixture and
an actual API/runner/web restart. It is not a real GitHub application PR.

September 8, 2026. Scope: #148 / PR #195, following
[the retained failed application probe](2026-09-08-checkpoint-recovery-review-followup.md).

**The same native case now reaches `verified_and_downloaded`.** Its original failure
remains in the report and a byte-for-byte archived report. This is a deterministic
Codex protocol fixture, not real-vendor inference or the completed darkfactory.

## Implemented correction

The existing checkpoint mechanism recognizes a checkpoint verifier's exported HEAD
only through its exact ready artifact, producer, source, policy and verification-digest
bindings. It preserves the original admission HEAD/fingerprint and ordinary verifier
HEAD equality. A missing or mismatched commit artifact cannot fall back to the old HEAD.

A finished checkpoint verifier awaiting review can use the existing `CheckpointWorkspace`
command to confirm a missing retention record. Current authority and source are checked
at admission and dispatch. No task, run, model session or budget allocation is created.
Evidence-complete review survives an unclaimed runner reconnect: waiting for a person
does not require a live provider.

Bounded review also found that denied re-attestation commands could obstruct later
commands. The final denial-path fix retires only that rejected command; database
failures remain retryable and the run/review is not terminalized. This last branch has
focused server-unit coverage; the successful live case used the preceding server build.

## Observed native sequence

1. Retained the failed case, application bytes and original source checkpoint.
2. Upgraded only QA server PID `36944` to `38280`. The database, source, runner PID
   `14532`, web PID `47032`, task and two runs were preserved.
3. Observed the same verifier still waiting for review after reconnect.
4. Reclaimed the same Factory item and requested its native same-run re-attestation.
5. Command `1f49dfe1-8ce5-4641-8b2a-9a5e3ec11e86` checked the physical HEAD and
   fingerprint. Journal sequence `268`, event `d009b5ae-6028-43de-a305-6bcf75279409`,
   recorded `run.workspace_preserved`.
6. In the visible Codex browser side pane, Bob clicked **Accept evidence** for verifier
   `5245249d-df3d-45e3-984e-011a09c305e4`.
7. The browser showed **Completed**, **1/1 tasks**, **0 decisions** and the preserved
   worktree. Reload retained the selection and result. API readback confirmed the
   completed mission, verified Factory item and Bob's approved decision.
8. Read-only driver continuation used an explicit server-upgrade receipt, retained the
   original binding/failure, checked the same lineage/journal, and downloaded identical bytes.

No invented event, application-database edit, replacement issue/mission, provider restart,
budget reset or policy weakening was used. The normal app and protected #172 bridge were
not operated.

## Exact result

| Object | Identity |
| --- | --- |
| Factory item | `38b06eef-d451-46e3-8f4f-af42a5c84820` |
| Mission | `b54001b1-713f-4950-8603-bdd37bffd5c3` |
| Task | `f5e7b75e-29bd-4c30-a523-cfb670cb15df` |
| Original provider | `b8b21950-1351-4fa8-9e72-9e4e1bdb2006` |
| Same verifier | `5245249d-df3d-45e3-984e-011a09c305e4` |
| Recovery | `7f596c68-52bc-4bbd-85f4-121b511bc22d` |
| Artifact | `bbe0dbf2-17c8-4a3f-ad6b-2ed08c031efe` |

- Original base: `a8894b5f02d56f10e2da38df47a450ff71e92fbe`
- Verified head: `dd3d27526e8d03fc14f44282d9d14e8c08189543`
- Fingerprint: `ee9f18226879d6542374bc1c5eaa4d2f3dbe5be10ca272421c1ba29f2d03257f`
- Artifact SHA-256: `da2e2093bf2ad5ec657ba5f522552efb89e2d7203fbdddd58f3963c0ae2170a8`
- Bundle SHA-256: `d06b753e79ee9f97efb90a2a28269fd794124600b9d41ce46fcc52dc9533e66f`
- Verification SHA-256: `8365727ca6e570ae48abb79fcd3f931b18c30c7e77749ec6a3ca13e5a47c7c9b`

The verifier has zero input tokens, output tokens and model cost. Original usage remains
6,000 synthetic protocol tokens against the original 5,000-token authority. There is one
mission, one task, one original provider run, one verifier and one ready source deliverable.
The exact downloaded incident app passed browser checks before review.

## UI and validation

Completed missions now show collapsed **Recorded spend**, not a budget-recovery demand.
The actual exhausted figures remain visible on expansion, with explicit completed-result
guidance. This was checked in the browser and after reload; it is not a budget reset.

- The actual-migration SQLx family passed **23/23**, 54.57 seconds.
- Six new cases cover HEAD binding, missing/invalid artifacts, re-attestation/replay,
  current role/room/claim, stop/quarantine/active-lineage denial and unclaimed-review retention.
- The first six-case execution failed five cases. Reversed SQL parameters were corrected;
  the staged-artifact negative fixture was repaired without weakening its constraint.
  Original failures and diagnostic output remain retained.
- **181 Node/application/workflow checks passed.** The upgrade validator now accepts a
  root-level `Cargo.lock`; its initial rejection remains a recorded test-driver error.
- Final repository validation passed **381 ordinary Rust tests**, with 129 opt-in
  SQLx cases ignored in that ordinary run. All 39 migration checksums, formatting,
  workspace/all-target Clippy, web build/lint and whitespace checks passed. This
  includes the final denied-command/transient-error regression; it does not imply
  another native application execution.
- The final acceptance report is `passed=true` / `verified_and_downloaded` and retains
  the prior native failure.

Receipts are under
`C:\Users\shyamsridhar\.codex\dogfood\issue195-checkpoint-runtime-20260908`:
`server-upgrade.json`, `native-retention-repaired.json`, `retention-accepted-state.json`,
`evidence/checkpoint-verification-report.json`, and the reviewed/downloaded artifact.
The preserved failed-report digest is
`9fadc842a23ba6d192dfe570f886ae13980c58f47bc33bace7adc41bc68a295a`.

SQLx receipt:
`C:\Users\shyamsridhar\.codex\dogfood\issue148-checkpoint-admission-20260908\20260908T180817228-issue148_checkpoint_.result.json`.

Final gate:
`C:\Users\shyamsridhar\.codex\dogfood\remaining-work-20260908\issue195-retention-final-20260908T184255732`.

## Remaining scope

Publication/checkpoint provenance, a first-class browser action to initiate checkpoint
recovery, real-vendor acceptance, #194's queued-upload race, and #145's repository/harness
setup experience remain unfinished. The fake GitHub boundary published no application PR.
There is no merge, deployment, production-identity, OS-containment or global deadlock claim.
