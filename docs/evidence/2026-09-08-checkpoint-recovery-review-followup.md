# Checkpoint recovery review fixes and native application probe

Date: September 8, 2026. Parent: #148 / PR #195, above PR #194.
Candidate base: `e291a6e19c0b5079ce7260a2fe05c4eb451165c4`.

**Status: review fixes tested; full native recovery acceptance is not passed.**
The native probe produced a working, downloadable application without a second provider,
but exposed a remaining server-side retention defect. The original case remains intact.

## Implemented review fixes

- Separate the checkpoint's admission HEAD from ordinary verifier commit preservation.
  Explicit `checkpoint_verification` may create the first runner-owned verification commit;
  the original HEAD and fingerprint still fence admission. Other verifier recoveries retain
  the existing commit rule. A dedicated runner capability prevents sending the new behavior
  to an older runner.
- Recheck the exact pending command, recovery, source, current authorizer role and room
  membership before optional artifact hydration and immediately before `VerifyRun`.
  An absent provider artifact no longer skips that authorization.
- Reject explicit `run.stop_requested` provenance across the checkpoint lineage, including
  dispatch, retry and the zero-provider budget exception. A budget incident is not an
  override of an operator's stop.
- After a successful export, retain only the exporter-returned HEAD as the failure/cancellation
  cleanup guard. An ACK failure must not classify the runner's own commit as tampering.
  A different HEAD or changed source bytes still fail integrity checks.

These changes do not authorize another model, relax required artifact evidence, rewrite
original spending, or grant publication, merge or deployment.

## Local evidence

- The approved serial `issue148_checkpoint_` SQLx family: **17 passed, 0 failed**, 61.54
  seconds of test execution, with all real migrations. This includes current-room/role
  revocation, exact command/source/policy bindings, explicit-stop fencing, rollback,
  missing provider artifacts, unchanged original authority and independent review.
- Five focused native-runner checkpoint tests passed, including a non-empty Git bundle,
  ACK-channel failure, cancellation, and changed-HEAD rejection. The ordinary-verifier
  uncommitted-tree negative also passed.
- The full repository gate: **380 ordinary Rust tests passed**; 123 opt-in SQLx tests were
  ignored in that ordinary invocation. The 17 scoped SQLx cases above ran separately.
  All 39 migration checksums, formatting, workspace/all-target Clippy, web build/lint
  and tracked whitespace checks passed.
- The supplemental application/driver Node tests passed **158/158** after correcting a
  test-driver assumption about Windows executable hardlinks. These are test-driver tests,
  not 158 native application runs.

Retained gate:
`C:\Users\shyamsridhar\.codex\dogfood\remaining-work-20260908\issue195-review-fixes-20260908T162935425`.

Retained SQLx receipt:
`C:\Users\shyamsridhar\.codex\dogfood\issue148-checkpoint-admission-20260908\20260908T161815169-issue148_checkpoint_.result.json`.

The Rust/product files were unchanged after that gate. Subsequent changes were confined
to the standalone Node driver's executable reader and its test; the 158-case Node run
covers those changes. The original failed preflight and browser-selector observation
remain retained rather than being represented as passes.

## Native case: actual server, runner, isolated Git worktree and artifact storage

The existing owned QA stack at API `18574` / web `15574` was reused without a database,
credential, source-base or history reset. No new container was created. The normal manual
app and protected #172 transport were not changed.

This is a **deterministic Codex protocol fixture**, not real vendor inference.
The native factory CLI used a local fake-GitHub boundary, not real issue/Project mutations.
Application files were produced in the separate lab runner worktree, not the ECorp source checkout.

| Object | Exact identity |
| --- | --- |
| Factory item | `38b06eef-d451-46e3-8f4f-af42a5c84820` |
| Mission | `b54001b1-713f-4950-8603-bdd37bffd5c3` |
| Task | `f5e7b75e-29bd-4c30-a523-cfb670cb15df` |
| Original provider run | `b8b21950-1351-4fa8-9e72-9e4e1bdb2006` |
| Verifier-only run | `5245249d-df3d-45e3-984e-011a09c305e4` |
| Recovery | `7f596c68-52bc-4bbd-85f4-121b511bc22d` |
| Source artifact | `bbe0dbf2-17c8-4a3f-ad6b-2ed08c031efe` |

The protocol fixture wrote an incident tracker and its four Node tests, then reported
6,000 synthetic tokens against the original 5,000-token limit. Native hard-stop handling
terminated the provider and retained its checkpoint. The same item/mission/task then
received the explicit verifier-only recovery.

That replacement has no provider session, model allocation or model usage. Its four
persisted checks passed, and it exported a non-empty commit/branch deliverable:

- Original base: `a8894b5f02d56f10e2da38df47a450ff71e92fbe`
- Runner-owned head: `dd3d27526e8d03fc14f44282d9d14e8c08189543`
- Source fingerprint: `ee9f18226879d6542374bc1c5eaa4d2f3dbe5be10ca272421c1ba29f2d03257f`
- Artifact SHA-256: `da2e2093bf2ad5ec657ba5f522552efb89e2d7203fbdddd58f3963c0ae2170a8`
- Git-bundle SHA-256: `d06b753e79ee9f97efb90a2a28269fd794124600b9d41ce46fcc52dc9533e66f`
- Verification SHA-256: `8365727ca6e570ae48abb79fcd3f931b18c30c7e77749ec6a3ca13e5a47c7c9b`

Bob's authorized artifact download was byte-checked against this run's digest. The exact
downloaded application passed browser checks for creation, search, resolution, status
filtering, reload persistence, reopening, empty-title validation, and 390-pixel layout.
No external requests or page errors were observed. Screenshots and per-file hashes are
retained in the QA evidence directory.

## Remaining defect — do not call this verified Factory completion

The server rejected the genuine `run.workspace_preserved` event:

```text
verifier source head conflicts with its authorized checkpoint
```

`source_workspace_checkpoint_tx` still applies ordinary verifier HEAD equality: the new
runner-owned deliverable HEAD differs from the original admission HEAD. Consequently this
verifier's workspace projection remains `active` with a null fingerprint, despite its
completed checks and ready bundle. The driver correctly retained `passed=false` /
`not_exact_provider_free_workspace_lineage`.

Independent outcome approval has **not** been granted. The same verifier is waiting for
review; no replacement issue, mission, source base, provider attempt or fabricated cleanup
event was used.

The next change must recognize only this exact checkpoint recovery's ready, source-bound,
verification-linked artifact HEAD for retention, while keeping ordinary verifier equality
and the original checkpoint immutable. Native `waiting_for_approval` must remain supported.

A server restart alone cannot recover the rejected receipt: the runner retains messages
only while writer enqueue is unavailable, not after a successful enqueue rejected by the
server. A truthful, authorized same-run `CheckpointWorkspace` re-attestation needs its own
bounded admission support; do not manufacture the event or reset the lineage.

Durable post-commit retry authority, recovery UI actions, publication, real-vendor evidence,
and #194's missed queued-upload race remain separate unresolved acceptance work.
There is no merge, auto-merge, deployment, hosted Actions or completed-darkfactory claim.

Native receipts, downloaded bytes and browser evidence:
`C:\Users\shyamsridhar\.codex\dogfood\issue195-checkpoint-runtime-20260908`.
