# Stopped-session native evidence reads — September 9, 2026

## Result

Issue #211 is **not resolved**. Two separate, explicitly selected native reads
against the retained Copilot session returned `SessionNotFound`:

| Native API | Local observation (CDT, September 9) | Actual native test |
| --- | --- | --- |
| `session.eventLog.read` | 13:58 | 0 passed / 1 failed; 8.83 seconds |
| `session.getMessages` (SDK `Session::get_events`) | 14:06 | 0 passed / 1 failed; 8.24 seconds |

These are retained compatibility failures, not passing application acceptance.
Neither diagnostic created, registered, resumed, or sent a prompt to a Session.
The second read was a separately selected diagnostic, **not** a fallback.
No further native reads or automatic reattachment followed.

Both runs retained the original source-run DTO, workspace fingerprint, and HEAD.
The SDK client completed shutdown. The parent-owned process observer recorded
no lingering positively identified descendants requiring cleanup.

This establishes that these two reads did not retrieve this preserved, unloaded
session through the pinned runtime and selected profile. It is not a claim that
all Copilot versions or every session-storage configuration behave identically.

## Exact retained application

- Lab issue: `shyamsridhar123/ecorp-enterprise-lab#4`.
- Factory item: `e6182bf4-0e41-420b-ac03-23a11c5619c5`.
- Mission: `290d72c8-50e8-4c31-8947-fcf45e587317`.
- Stopped source run: `945246da-b3ac-48d5-ac28-c9039c06de6e`.
- Workspace run: `cd10992f-ce28-4f33-accf-de2b99eeb31f`.
- Native session: `3e326726-662a-4975-975e-6de1ae285729`.
- Saved connection: `a0604308-737f-4554-9e24-49493a427991`.
- Source HEAD: `e3dc3d669b1a99832e2e7af9be16f7f39842586d`.
- Preserved fingerprint:
  `cbdc96f6804697f6378ec9b6079af976548aaaa559edf03eed72ec02305a0091`.

The source remains cancelled with its recorded STOP and absent provider artifact.
There was no application edit, replacement mission, budget/attempt reset, manual
application-database mutation, outcome approval, publication, merge, or deployment.
The earlier real browser repair evidence remains separate from Factory acceptance.

## Patch boundary

The reader, adapter seam, and native probe are compiled **only with `cfg(test)`**.
No production runner command, recovery route, UI control, or artifact-adoption
path can invoke them. There is no advertised working collector capability.

The test reader reuses SDK 1.0.11 and the selected local CLI 1.0.79:

- Explicit `Transport::Stdio`; ambient SDK transport selection is not used.
- Owning saved-account profile and exact preserved workspace, not the model
  catalogue workspace. Source path representation matches the runner's native
  connection path normalization and the persisted source-run path.
- Native account status is checked before reading. Successful retrieval would
  also require a second account check; the actual missing-session failures occur
  before that second check.
- Both `CopilotRequestHandler::send_request` and `open_websocket` return errors,
  never the SDK's upstream-forwarding defaults.
- No Session object or session tool, permission, or filesystem handler is
  registered. Unknown-session requests are dropped by the SDK; this is not an
  OS-level deny-all claim.
- Event-log reads request at most 200 events per page, 20 pages, and reject
  expired/repeated cursors, duplicate identities, foreign session starts, empty
  responses, and more than 8 MiB of decoded canonical JSON.
- The separately selected timeline diagnostic makes one `session.getMessages`
  request, with bounded decoded bytes/events and no synthetic event-log cursor.
- No prompt text, tool arguments/results, raw history, credentials, or native
  profile contents are exported. A successful diagnostic would retain only
  bounded type counts, scope, and canonical-response digests, explicitly **not**
  provider completion.

The read deadline is 20 seconds after startup, with a separate 15-second shutdown
wait. The native probe additionally uses a parent-owned 90-second process bound.
SDK startup failure does not itself expose a verified process-exit receipt;
`force_stop` is only a fallback. These lifecycle limitations are another reason
the prototype is not production-enabled.

## Retained native receipts

Local evidence directory:
`C:\Users\shyamsridhar\.codex\dogfood\checkpoint-source-correction-20260909`.

Each prefix has `.parameters.json`, `.native.json`, `.result.json`,
`.stdout.log`, and `.stderr.log`. Earlier results were not overwritten.

| Prefix | SHA256 of native receipt |
| --- | --- |
| `20260909T185849910-native-history-probe` | `da17fb5957bb3402f615ad28ed9222628bc7cb3436a5015e4bacb22c122a3cbb` |
| `20260909T190603942-native-timeline-probe` | `c187e1930796df898390b53ceb3c16afb5acc0770d526861a3151769c03a53f7` |

The event-log launcher initially displayed null summary columns because PowerShell
`Select-Object` was applied directly to an ordered dictionary. The durable result
JSON contains the actual values (exit 101, no timeout, unchanged source). Display
formatting was corrected before the separately selected timeline diagnostic.
No evidence file was rewritten to repair that display.

## Local validation

All checks below were local. Hosted Actions were not used as a gate.

| Check | Result |
| --- | --- |
| Migration integrity | PASS, 41 immutable migrations |
| `cargo fmt --check` | PASS |
| Workspace/all-target Clippy with `-D warnings` | PASS |
| Focused `issue211_` tests | 11 passed / 0 failed / 1 native probe ignored |
| Established **serial** Rust workspace gate | 458 passed / 0 failed / 276 ignored |
| Web build and lint | PASS |

The eight synthetic wire cases use only owned loopback sockets and the public
SDK External transport; an explicit `current_exe` program prevents CLI
resolution/extraction. They prove fixed pagination, both separately selected
missing-session failures without fallback, malformed/expired/duplicate/oversize
response rejection, total bytes/page bounds, account recheck, pinned-runtime
rejection, and both model-traffic denial callbacks with zero downstream trap
connections. They do not prove successful retrieval from the real stopped
session or signed-object/application acceptance.

An initial missing `Path` import and one fixture-only Clippy `let_and_return`
finding were corrected; their failed outputs remain retained. No assertions or
timeouts were loosened.

The first workspace invocation used default parallel test execution and
**failed** in the runner suite: 188 passed / 7 failed / 1 ignored, 166.33 seconds.
Four failures were explicit timeouts and three expected checkpoint proofs were
absent. The failing test bodies and production lifecycle logic were unchanged
(`main.rs` gained only test-module registration), but causality was not established
by a baseline parallel control. That failure remains tracked separately in **#213**.

The established prior QA script explicitly uses `RUST_TEST_THREADS=1` and
`--test-threads=1`. Running that same serial lane with unchanged assertions
passed the complete workspace; the runner portion was 195 passed / 1 ignored in
810.90 seconds. **This is not a green parallel-suite claim.** No ignored SQLx
cases or additional native diagnostics were invoked by either workspace run.

Gate directories under the evidence directory above:

- `issue211-web-gate-20260909T191104859`.
- `issue211-rust-gate-20260909T191516984` (focused pass; initial Clippy failure).
- `issue211-rust-final-20260909T191835097` (Clippy pass; parallel workspace failure).
- `issue211-serial-workspace-20260909T192525276` (complete serial workspace pass).

Independent read-only review found no actionable issue within the test-only
patch and no release-enabled collector path. It did not independently reexecute
the native launcher or its process-cleanup observations.

## Remaining prerequisite

A supported, separately proven way to access the stopped session's native
evidence is still needed. Do not silently use `resume_session`, send a prompt,
scrape native credential/history files, substitute an empty receipt, or remove
the persisted artifact requirement.

If a future supported read or explicitly designed no-inference attachment
provides genuine evidence, the production implementation must still bind a
current checkpoint/recovery generation, source, saved connection and account
observation; revalidate actor/room/policy at dispatch and adoption; sign and
durably store evidence on the **new verifier run**; then perform native checks,
independent outcome review, and review-only publication. A historical Ready
account observation is not retrospective proof of session-account ownership.

The GitHub issue and Project remain the work queue. This report is evidence,
not a replacement backlog or a claim that #211 or the darkfactory goal is complete.
