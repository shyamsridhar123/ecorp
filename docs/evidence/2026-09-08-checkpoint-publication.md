# Checkpoint-bound publication of the retained application

September 8, 2026. Parent: #148 / #63; code layer above PR #195.

**Native publication passed on the same retained case.** Git operations were real,
against an explicitly owned local bare remote. GitHub issue/PR/Project operations used
the existing fake-GitHub boundary. **No application PR was created on real GitHub.**

## Implementation

- Publication reconstructs the exact native checkpoint authority and matches it to
  the completed provider-free recovery, retained fingerprint and ready verified export.
- Only this bound, separately authorized zero-provider publication can proceed past
  its original measured model-budget stop and retrospective shared model counters.
  Original usage is unchanged. Unrelated enforced stops/suspends, explicit stops,
  current loop limits, quarantine, source/policy checks and actor/room/workload authority remain.
- Provenance records the original run/workspace, native checkpoint and termination
  event IDs, authority digest, fingerprint, original HEAD and verified publication HEAD.
- Renewals and new publication checkpoints revalidate that provenance and current
  authority. A checkpoint lease alone cannot authorize progress after revocation or
  source/artifact changes. Failure recording and terminal idempotent replay retain
  their existing semantics.
- Ordinary/legacy publications receive no checkpoint-budget exemption. Existing
  provenance remains compatible; validation refreshes a legacy-upgraded record before
  a subsequent checkpoint writes its provenance.

## Native evidence

The original item `38b06eef-d451-46e3-8f4f-af42a5c84820`, mission
`b54001b1-713f-4950-8603-bdd37bffd5c3`, task
`f5e7b75e-29bd-4c30-a523-cfb670cb15df` and verifier
`5245249d-df3d-45e3-984e-011a09c305e4` were reused.

The native publisher first returned HTTP 400 because of the original stop. The bare
remote still contained only `main` at the original base; no fake PR or review-status
mutation had occurred. That failure and its intent/output remain retained.

After the candidate server upgrade, the same native publication invocation succeeded:

| Field | Recorded value |
| --- | --- |
| Publication | `45136278-f810-4e4e-8c25-aa52ea405936` |
| State / attempt | `published` / `1` |
| Target branch | `ecorp/checkpoint-publication-195` |
| Original `main` | `a8894b5f02d56f10e2da38df47a450ff71e92fbe` |
| Verified branch HEAD | `dd3d27526e8d03fc14f44282d9d14e8c08189543` |
| Original provider | `b8b21950-1351-4fa8-9e72-9e4e1bdb2006` |
| Checkpoint event | `4517d739-a0c1-4e7f-8c57-e9dff0127c33` |
| Termination event | `1a819de6-3336-4c31-8df0-57efe056925f` |
| Fingerprint | `ee9f18226879d6542374bc1c5eaa4d2f3dbe5be10ca272421c1ba29f2d03257f` |
| Authority digest | `b9ec293b234d8f1740e6a7edbcb2955c7d6c9c94ac38d804c28ec28511e39ef0` |
| Artifact digest | `da2e2093bf2ad5ec657ba5f522552efb89e2d7203fbdddd58f3963c0ae2170a8` |

The controlled GitHub boundary contains exactly one open PR record, pointing from that
branch to `main` at the exact verified SHA. Its original Project item advanced from
`In Progress` to `In Review`. Auto-merge, merge and deployment remained false.
The visible ECorp browser showed **Integration · Published** on the same completed run.
The fake PR link was not opened as a real GitHub result.

Re-running the native publisher returned `recovered`, with the same publication and
one attempt. The fake-GitHub state file and actual Git refs were byte-for-byte unchanged.

The owned API, runner and web were then actually restarted without resetting their
database, enrollment, artifacts or source:

- API: `19740` → `48016`
- Runner: `14532` → `53648`
- Web: `47032` → `41072`

Old process identities were gone and new identities matched the native ownership records.
A further native publisher retry again recovered the same result with no Git/PR/Project
changes. The same two runs remained: original provider plus completed zero-provider verifier.

## Validation and limits

The actual-migration SQLx family passed **29/29**, 71.57 seconds. Six new cases cover
publication/replay/provenance, healthy historical tasks under exhausted model counters,
native origin proof, source/policy/quarantine, artifact binding/retention, and current
actor/room/publisher credentials.

Final repository validation passed **382 ordinary Rust tests**, with 135 opt-in SQLx
cases ignored in that ordinary run. The 29 cases above ran separately. All 39 migration
checksums, formatting, workspace/all-target Clippy, web build/lint and whitespace gates
passed on unchanged code inputs. Full gate:
`C:\Users\shyamsridhar\.codex\dogfood\remaining-work-20260908\issue148-checkpoint-publication-final-20260908T195530453`.

The initial six-case run passed two and failed four. Three failures exposed missing
checkpoint-time revalidation; one was a staged-artifact fixture constraint error.
Both the original failures and corrected six-case pass are retained. Metadata SQLx
tests are not fresh cryptographic verification; the native publisher separately
downloaded/validated the signed bundle and exercised real Git.

Receipts:
`C:\Users\shyamsridhar\.codex\dogfood\issue148-checkpoint-publication-20260908`.
Key files include `red-publication-readback.json`, `native-publication-replay.json`,
`native-publication-after-stack-restart.json`, publisher stdout/stderr and restart records.
The same original fake-GitHub state and artifact store were reused; no replacement
mission, issue, source base, provider run or budget reset was used.

This is deterministic provider-protocol and fake-GitHub acceptance, not real vendor
inference, production identity, OS-containment or a real external deployment.
First-class browser recovery initiation, repository/harness onboarding and the specific
#194 queued-upload race remain unresolved.
