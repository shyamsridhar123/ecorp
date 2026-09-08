# Native Copilot source-acknowledgment acceptance

**Date:** September 7, 2026

**Tracking:** #169, with separate concurrent/reload acceptance in #168

**Status:** End-to-end single-runner acceptance passed after the two recorded
native lifecycle corrections. Multi-runner fairness remains tracked in #172.

**GitHub Actions:** Not used.

## Scope and first candidate

The first runtime used fresh standalone server, runner and CLI executables built from
`1201c754254ab2654f17030924b56e3460c7c66e`. Its source was a separate clone of
`shyamsridhar123/ecorp-enterprise-lab` pinned to
`e3dc3d669b1a99832e2e7af9be16f7f39842586d`, with `HEAD` advertised by both the
owned runner and the native one-shot Factory invocation.

This was genuine GitHub Copilot: Rust SDK 1.0.11, CLI 1.0.79, product no-auto-update
control, fixture mode false, an enabled `gpt-5.6-sol` model, and retained native
session-store files. Development Alice/Bob remain test principals, not proof of
production GitHub human identity.

The explicitly scoped GitHub intake was enterprise-lab issue #2 in Project #3.
The existing manual watcher was inspected and remained restricted to enterprise-lab
issue #1. No watcher was started for this QA Corp. No application scenario was
written in ECorp's product checkout.

The fixture reused the approved PostgreSQL container on loopback 54441 with a
new owned database, `crony_issue169_ack_20260907`. Its API/UI/transport ports were
18961/15496/18963, separate from the manual API/UI at 18962/15491. No new
container, manual database mutation, application/game preview, publication, merge
or auto-merge was used.

## Transport-only fault fixture

`tools/runner_ack_fault_relay.mjs` forwards original WebSocket bytes. It binds
native start/resume run and task IDs against the actual persisted Factory issue,
Corp, source tuple and Studio plan keys. It never generates lifecycle events or
changes attempts, budgets, artifact bytes, provider behavior or product timeouts.

This opt-in diagnostic requires an installed `ws` package in Node's resolution
path; this host used Node 25.9.0 and `ws` 8.21.3. It does not install dependencies
or start a product stack automatically. The original and fixed-candidate
directories are explicitly paired with their own runner IDs and enterprise-lab
issues #2 and #3; mixing those identities is rejected.

The missing lane withholds every exact source-deliverable acknowledgment for each
of the two native visual-direction automatic attempts. The delayed lane holds all
matching quality-verification acknowledgments until eight seconds after its first
one. Gameplay and integration acknowledgments pass normally. A later explicit
fixture control can release one original old-run acknowledgment without retagging
it; that step has **not** been exercised in the first candidate.

Pre-runtime review caught and corrected native owner/name schema matching,
connection-local FIFO ordering, stale-connection ownership, exact issue binding
and metadata logging. Thirteen focused tests passed, including actual loopback
WebSocket forwarding with synthetic peers. Those are transport fixture tests,
not server/store/provider acceptance.

## Observed first-run result: FAIL, preserved

| Identity | Value |
| --- | --- |
| Mission | `6af8daed-07b8-42ac-b59d-3062def7ed29` |
| Factory item | `0d70b1db-8155-43fa-bef5-81603dc984d3` |
| Visual run | `e5b5d2fa-ce57-4f29-893c-74bb279159d7` |
| Quality run | `e39a3b23-92a5-4f77-8f87-41085e0fb48f` |
| Systems run | `bb0e5c04-1332-4098-8615-4bee3e631a0a` |

- Native Copilot lifecycle records show three distinct sessions overlapping for
  approximately 23.7 seconds. Two real ECorp browser clients were connected.
- Gameplay completed with its normal source acknowledgment.
- Quality completed after two duplicate acknowledgments were withheld for
  **8,221 ms**, crossing a native five-second retry window.
- Visual emitted its own acknowledgment timeout after **30,047 ms** and six
  withheld acknowledgments. The fixture did not synthesize that failure.
- All three visual file/artifact/UTF-8 checks were persisted as passed, but no
  accepted verification-passed/completed event was emitted for that failed run.
- Three real source deliverables were downloaded through the authenticated
  server route. Each local byte count and SHA-256 matched its stored metadata;
  each source-role and provenance-signature response header matched the record.
  This is server-authorized signature checking plus local digest verification,
  not an independently recomputed HMAC proof.
- The visual worktree and fingerprint remained preserved. Native Copilot
  `events.jsonl` and session-store database/WAL files remain in its owned state
  directory, with the recorded session-start ID matching the ECorp run.
- There were zero action approvals and zero manual verification requests.
  The integration task never started.

The failure did **not** exhaust two native task attempts. The task correctly became
`ready`, attempt 1 of 2, while mission and Factory stayed `running`. However, the
visual agent stayed `reviewing` with a null `current_run_id`. A checkpoint taken
over 530 seconds after the terminal update still showed only three total runs.
This is a real stalled retry, not a passing two-attempt exhaustion test.

The actual ECorp mission UI showed the failed visual run, all three recorded
checks, preserved worktree, signed source deliverable, 2/4 tasks completed and
the native Resume action. The screenshot remains in the browser tool transcript.
No Resume or manual re-launch was used to disguise the missing automatic retry.

## Additional regression isolated

Native artifact finalization sets the agent to `reviewing` and clears its
`current_run_id`. Verification start does not restore that pointer. The #169
failure cleanup fence in `b2eec640` only clears an agent whose current pointer
still equals the failed run, so it updates no row after genuine artifact
finalization. Both native scheduler queries require an idle agent.

The server does invoke automatic scheduling on `run.failed`; the reviewing
projection simply makes the retry ineligible. The original store fixture seeded
reviewing **with** a run pointer and did not exercise artifact finalization or
assert scheduler eligibility. The correction and new focused store regressions
were implemented in a narrow follow-up; the original 23-case result is not
relabeled as coverage of this gap.

The follow-up replaces only the previous failure-cleanup UPDATE at its original
lock position. It locks the same-Corp agent row, preserves exact attached-run
cleanup, and releases a detached reviewing projection only after a fresh
post-lock query proves the failed run is uniquely latest and no other same-agent,
same-Corp run is active. Equal assignment timestamps fail closed. Other pointers,
later assignments and unrelated reviews remain untouched. Task, mission, retry,
Factory, policy, budgets and artifact-finalization behavior are unchanged.

Only the new `issue169_detached_review_` SQLx subset was run through the approved
isolated maintenance loader:

- Red, unchanged cleanup: **10 passed, 1 failed**, 24.60 seconds, exec 73629.
  Real prepare/finalize plus verification left reviewing/null and both scheduler
  queries returned no work.
- Green, corrected cleanup: **11 passed, 0 failed**, 55.31 seconds, exec 1475.
  The positive case now selects the original mission/task through both native
  scheduler queries; artifact, source, fingerprint, usage and evidence fields
  remain intact.
- Negatives cover another pointer, every one of the six native active statuses,
  a later terminal assignment and tied timestamps. Exact attached-run cleanup
  also remains covered.
- Focused format, compilation and all-target store Clippy passed. The default
  store unit run passed **43 tests**, with **44 opt-in cases ignored**.
  The old 23-case SQLx set was not repeated.

These checks prove store eligibility, not actual runtime dispatch or global
deadlock freedom. A separately scoped post-correction candidate uses
enterprise-lab issue #3; the original issue #2 diagnostic is not reset or reused
as a purported passing automatic-retry run.

## Acceptance status

The second candidate passed native retry, failure reconciliation, late-ACK
fencing, saved-session recovery, dependency-gated integration, independent final
review and unchanged signed-byte checks. The fresh three-provider browser
reload observation also passed. The first stalled diagnostic and the later
#171 staffing obstruction remain recorded below rather than relabeled.

Ordinary resume must not be described as verifier-only sealed-checkpoint recovery:
it carries the persisted source/workspace/session policy, but does not supply the
verifier-only expected-fingerprint/HEAD parameters. A resumed worktree fingerprint
may change after real authorized edits; the original failed-run record must not.

## Second candidate: native recovery and the #171 obstruction

Fresh standalone executables were built from the clean, pushed
`10068b60f130ce89259f87944d099015c15f3946` candidate. The runner/source tuple,
Copilot SDK/runtime pair, immutable enterprise-lab source commit and native
verifier policies were unchanged. A new owned database and source clone were
used for enterprise-lab issue #3; the first diagnostic was not reset.

| Identity | Value |
| --- | --- |
| Mission | `d561cc2b-8d5e-44c9-b9da-ae6060af78b3` |
| Factory | `c860fb2b-9b67-46a5-ba46-3db0360c7fd2` |
| First visual attempt | `38ff0719-0208-434e-bd5c-1baa1d16b1f9` |
| Second, exhausted visual attempt | `58333c7a-2360-499b-b580-9697410b7d77` |
| Native resumed run | `70660159-a8ea-4f62-99d8-c65c308f94e1` |
| Preserved Copilot session | `83761e9f-32bb-4ce5-8353-ae6700e1e2c4` |
| Pending integration task | `94d99940-a24d-484d-8d24-d809224222f6` |

The first ordinary timeout now caused an actual second automatic assignment.
The mission and Factory stayed running between attempts. After all six native
ACK windows expired in each attempt, the mission failed and Factory immediately
became blocked v8, with the exact bounded timeout detail. Twelve original ACK
frames were withheld in total. No external Factory watcher, manual launch,
counter change or synthetic lifecycle event was used.

Both failed runs retained three passed check rows, their original workspace
fingerprints, source identity and usage. Neither emitted accepted verification
or completion. Four signed source objects were downloaded and byte-verified
before recovery; integration had never started.

The fixture released one original second-attempt ACK without retagging it.
Mission/Factory/version and every recorded original run field remained unchanged.
Fault injection was then disabled.

Alice selected the exact exhausted run in the real mission UI and used the
product's native **Resume agent session** action. No alternate recovery endpoint
or custom prompt/approval mechanism was added. The new run preserved its task,
agent, provider session, workspace-run ID, path, branch and source tuple.
Factory changed to running v9 with no failure detail.

The actual retained Copilot journal records `session.resume` at
`2026-09-07T20:13:11.874Z`, followed by real tool/model activity and native
shutdown. The resumed run completed with 77,586 new input tokens and 997 output
tokens. Original run history/usage remained unchanged. Its new fingerprint
`189b5db753ca5ab8fabb380fe5cafa0a0bbab4d78623da8db985b4476eddfda8`
legitimately differs from the unchanged original
`6f777fb7a7d2ea5f303724864add71a1e0d573f32bc13f5b8214a40da6ce02ec`.

All three roots completed, but integration remained ready at attempt 0. Its
original Gameplay worker `107b8cbd-b6a6-487c-b1d6-c970bc90813a` had been
automatically retired when the mission failed. Native resume reactivated only
the visual worker. This is #171, not a missing ACK or failed vendor-session
recovery. No successful root was rerun and no runtime database row was manually
changed to make the graph advance.

### Fresh browser reload observation for #168

Two CUA-operated ECorp clients were connected. The floor showed all three
genuine Copilot workers Working. Native session-start/shutdown records establish
20,216 ms of three-session overlap.

Bob reloaded between `2026-09-07T20:06:18.823Z` and
`2026-09-07T20:06:19.517Z`; the measured reload-to-ready interval was **692 ms**.
His UI still showed the same mission, three live runs and Bob's development
role. All three native sessions remained active throughout the reload; Alice's
client stayed connected. Neither client had application console errors at
inspection.

Live in-flight snapshot request counts were not directly instrumented:
Performance is unavailable in CUA's read-only DOM scope. The 99-test frontend
run covers the coalescer's bounded burst/trailing behavior, cancellation and
errors separately. The real observation proves functional concurrency and
reload survival, not a fully isolated causal or throughput benchmark.

The second fixture and its phase-specific proof receipts are retained at:

```text
C:\Users\shyamsridhar\.codex\dogfood\issue169-ack-acceptance-20260907-fixed
```

Its original service keys are retained only in a restricted Windows
current-user encrypted envelope, permitting an explicit same-state server
upgrade without invalidating original signed objects. A #171 fix must continue
this same mission through the normal scheduler; that continuation subsequently
passed as described next.

### Same-state continuation and accepted outcome

The reviewed #171 server was built from the exact recorded staffing/server
source digests and substituted for only the owned QA server, PID 33328 → 14008.
The same database, original encrypted signing/master keys, source checkout,
mission, worktrees, runner identity and five existing run records were retained.
No manual runtime database mutation, fresh mission, successful-root rerun or
extra launch request was used.

The QA-only ACK bridge treated the planned server downtime as a fatal transport
failure. Its latched diagnostic state was preserved, and that bridge alone was
restarted after the replacement server was healthy. Faults stayed disabled,
the already-executed one-shot late-ACK command was removed from its control
file, and the original trace remained append-only. This is not a claim of an
uninterrupted test-bridge connection.

The native reconciler reactivated exactly the original Gameplay worker, leaving
the completed Quality worker retired. The existing runner then reconnected
through native enrollment/reconciliation, and normal scheduling created only
the pending integration run:

```text
Integration run: b6f9e47e-4b49-42d5-9467-4772381456b0
Native session:  129fa7a8-bf81-4ba5-aaa9-7e54c28f2f18
Gameplay worker: 107b8cbd-b6a6-487c-b1d6-c970bc90813a
```

Copilot produced the actual release-receipt module, tests and README under
`qa/issue169/` in its isolated worktree. Its persisted runner evidence records
**12 Node tests passed** and **all five verifier checks passed**. Independent
read-only source review found no blocker for this bounded synthetic utility.
It is not a production compliance certification or a complete enterprise app.

Bob's real UI retained an older failed run selection. **Review next pending
run** correctly switched to integration; **Accept evidence** applied only to
that selected run. There was exactly one independent final review, zero routine
action approvals, and no source-artifact-by-artifact approval workflow.

The actual server snapshot and both UI projections then reported:

- Mission completed, all **4/4 tasks completed**, **6 total runs**.
- Factory **verified v11**, no remaining review decision.
- Original two failed attempts still failed, with unchanged source, fingerprints,
  usage, budgets and source bytes.
- The resumed run still uses the exact failed run's native session/workspace
  lineage, with its new usage retained.
- **Six authenticated source downloads** matched their metadata and provenance
  headers; all four original signed objects remained byte-identical.
- The immutable source checkout remained clean at the same `e3dc3d6…` commit.
- No pull-request publication, merge, auto-merge, deployment or game preview.

`complete-proof.json`, phase-specific snapshots, native session records,
actual test evidence, browser observations and service receipts are preserved
under the fixed fixture root. The separate #172 source-review finding limits
claims about fairness across multiple runners; this acceptance used one runner
with multiple real Copilot sessions.

## Retained local evidence

The first fixture, native logs, immutable source clone, worktrees, session state,
signed artifact downloads and metadata receipts remain under:

```text
C:\Users\shyamsridhar\.codex\dogfood\issue169-ack-acceptance-20260907
```

Its four owned services and two QA browser tabs were retired after capture.
Process identity was checked against the original executable and .NET start
timestamp. Temporary enrollment/workload/provider credential files were removed;
the database, objects, source, native session stores and worktrees were retained.
The manual ECorp API/UI and both existing application previews stayed running.
The passive observer subsequently exited on the expected closed QA endpoint;
that exit is not recorded as a passing acceptance test.

The initial invalid preflight allocation was also retained: a 20,000,000-microUSD
mission allocation put the 55% integration share above the native per-task
10,000,000 limit. Preflight rejected it before any mission, run or Factory claim.
The fresh, unlaunched configuration was corrected to 10,000,000 microUSD; no
persisted attempt, original budget or spend was reset.
