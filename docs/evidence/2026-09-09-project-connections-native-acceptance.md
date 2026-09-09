# Saved project connections: native application acceptance

Date: September 9, 2026 (UTC). Tracking: #198, under #145 and #63.

## Result

A real GitHub Copilot worker built Pantry Board through ECorp against a newly
initialized, independent local Git repository. The operator supplied the
specification, not application code. The persisted mission, task and run reached
`completed`, with verification `passed`, after the development Bob principal
accepted the evidence in the browser.

This is one ordinary saved-connection application run. It does not close the
broader Factory cockpit, production identity or real-GitHub publication work.

## Native connections

The browser used **Connect and test**, **Test** and **Use** against the actual
server and runner. Executable selection and account selection were explicit.

| Coding agent | Installation/account choice | Observed result |
| --- | --- | --- |
| GitHub Copilot | Pinned SDK 1.0.11 / CLI 1.0.79; existing approved native account | Ready; native account/catalog checks, followed by real inference |
| Codex | Installed real CLI; new private profile | Sign-in needed; no sign-in or inference claimed |
| Claude Code | Installed real CLI; new private profile | Sign-in needed; no sign-in or inference claimed |

The Copilot path did not require a token file. No account credential was copied
into the browser, application source or evidence report. The retained legacy QA
fake adapters were not substituted for the selected connection.

All three create requests were replayed with their original request keys and
retained the same connection/operation identities. Each saved connection's Test
operation completed. Bob could not read Alice's private setup operation:
the exact read returned HTTP 404 and the operation was absent from Bob's list.

The Copilot selection and immutable source survived browser reload and an owned
runner disconnect/reconnect without another connection or replacement source.

### Live-presence corrections

Acceptance exposed two UI freshness defects:

1. A cached offline connection could remain labeled offline after the runner
   reconnected with unchanged capabilities.
2. An idle runner could disconnect without a run event, leaving an open console
   and connection panel falsely online.

The UI now invalidates connection reads on relevant runner lifecycle changes and
briefly re-reads the connection endpoint while registration presence differs from
live dispatch readiness. It never promotes cached metadata to Ready.

An idle-presence fallback reuses the existing coalesced snapshot reader every five
seconds while the document is visible, and on return to visibility. Disposal
cancels the timer, listener and pending read scope. This is read-only UI
revalidation, not a new runner, controller, execution loop or permission engine.

The open panel subsequently showed **Machine offline** with Test disabled and
then returned to **Ready** with Test enabled after reconnection, without clicking
Test, reopening the panel or creating another connection. The mission picker
also uses the saved project name instead of leading with an opaque local ID.

## Application and provenance

| Object | Recorded identity |
| --- | --- |
| Mission | `2b7495e3-a1ba-448e-bde7-2d2f28ba7142` |
| Task | `4c1fdcaa-7f9f-49b8-8e4f-943d61a50d39` |
| Run | `d33fe7e7-bc6f-4a7f-b625-b27faf95083e` |
| Saved connection | `174c9124-89d6-4dfc-b341-c36f214aecab` |
| Source base | `fe614356526bb2d8c7440da53c8b95572779e09c` |
| Source artifact | `672a2961-b226-4b20-8a66-658af47c0a6b` |
| Source artifact SHA-256 | `5b6e0512bc4a39cb865ff9596ee06b1dd568eecba74d4a228be5bbe3d352a9d0` |
| Verification digest | `7a625c310eee19078d2ac9e77cdaab2250e78e9f67f16b1baa845a1d68cd2ba5` |

The initial source contained only README.md and .gitignore, with no remote.
After execution it remained clean at the same commit. Changes were retained in
the assigned worktree, not written into that checkout or ECorp.

Before dispatch, the actual browser saved a held plan with zero runs. Inspection
confirmed the selected connection, source tuple, assigned Copilot agent, enabled
model and persisted completion policy. The operator then used Dispatch mission
once. A native provider session and nonzero provider usage were observed.

Four persisted checks passed:

- Provider artifact: at least one byte.
- `index.html`: at least one byte.
- `evidence/result.md`: at least one byte.
- `node --test tests/inventory.test.mjs`, with a 60-second timeout.

The final independent-review gate excluded the requester. Alice saw the explicit
different-reviewer explanation; Bob's enabled Accept evidence button completed
the same run. These are development-principal role checks, not evidence of two
separately authenticated production humans.

The 91,345-byte source artifact was retrieved through its authorized artifact
endpoint. Its byte count and SHA-256 matched the authoritative metadata. All nine
regular-file entries were decoded into a separate export directory only after
validating paths, modes, byte counts and per-file digests. No application code was
added or repaired by the acceptance operator.

The provider's test and loopback-server/HTTP-check commands received scoped
approvals. A broad process-name-based stop request was rejected; the mission
continued and unrelated baseline processes remained alive. This run does not
claim zero shell approvals.

## Delivered application checks

The exported app, not the configured source checkout, ran with its documented
Node server. Its six inventory tests passed again on the decoded source.

Real browser checks covered:

- Honest empty state.
- Blank-name, negative-quantity and fractional-quantity errors without losing
  the draft.
- Adding items in Pantry, Fridge and Freezer; correct record and low-stock counts.
- Editing a name and quantity.
- Case-insensitive search combined with low-stock filtering and a no-match state.
- Persistence of inventory and edits across a hard reload.
- Keyboard submission with visible focus.
- Export button activation.
- A 390-pixel layout without horizontal overflow, 16-pixel input text and a
  measured Add button above 44 pixels high.
- No warnings/errors in the inspected browser console.

Nine separate Node VM/minimal-DOM tests executed the actual exported app handlers
and inventory module. They covered exact-item deletion, edit reset/cancellation,
corrupt-storage recovery and parseable versioned JSON export. Those tests use
in-memory DOM/storage/object-URL fixtures; they are not real-browser deletion,
storage-policy or filesystem-download evidence.

The in-app driver did not expose a completed browser download event, although
ECorp displayed "Source deliverable download started." The independently
retrieved source bytes matched the same artifact. Raw CDP storage methods and
resource timing were unavailable in the chosen browser tooling; no browser
corrupt-storage or complete network-capture claim is made.

## Local validation

- Migration check: 40 migrations; immutable checksums verified.
- Cargo formatting and workspace/all-target Clippy: passed.
- Rust workspace: 428 passed, 0 failed, 159 ignored.
- Opt-in actual-migration `issue198_` SQLx cases: 17 passed, 0 failed.
- Final focused connection/mission/snapshot UI tests: 52 passed.
- Web production build and lint: passed.
- Windows launcher source checks: 8 passed, without creating processes.
- Exported inventory tests: 6 passed.
- Separate exported-app DOM unit tests: 9 passed.
- Connection-panel static design detector: no findings.

The full local gate preceded the last web-only idle-presence addition. Web build,
lint and all 52 focused tests were rerun after that addition. Rust source and
runtime binaries did not change during these final UI corrections; existing
store/runtime results are not represented as newly executed tests.

No hosted GitHub Actions result is inferred. Existing application history,
source worktrees, native profiles and the baseline manual stack were retained.
No new container was needed for these connections.

## Remaining scope

- #145 / #63 remain open for the complete issue-to-Factory-to-reviewable-PR lane.
- This run did not exercise real GitHub clone/sign-in/publication, real Codex or
  Claude inference, multi-agent Studio execution, or production human identity.
- The source archive remains ECorp's JSON format. This acceptance decoded and
  served it separately; it does not prove a one-click in-product result preview
  or a conventional ZIP download.
- Application publication, merge and deployment were not requested by this
  independent local-source scenario. Auto-merge was not enabled.
