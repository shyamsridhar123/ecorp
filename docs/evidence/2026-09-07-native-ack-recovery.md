# Native Copilot source-acknowledgment acceptance

**Date:** September 7, 2026

**Tracking:** #169, with separate concurrent/reload acceptance in #168

**Status:** In progress; the first real run exposed an additional retry regression.

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

## Pending acceptance

- Fresh post-correction real run reaching both native automatic attempts.
- Immediate mission-failed / scoped Factory-blocked reconciliation without a watcher.
- Late original acknowledgment fencing.
- Exact same-mission/task/agent/worktree/native Copilot-session resume and new usage.
- Preserved failed history, original fingerprints, signed bytes, attempts and spend.
- Dependency-gated integration and an authorized independent final review.
- Separate completion of #168's fresh multi-provider load/reload regression.

Ordinary resume must not be described as verifier-only sealed-checkpoint recovery:
it carries the persisted source/workspace/session policy, but does not supply the
verifier-only expected-fingerprint/HEAD parameters. A resumed worktree fingerprint
may change after real authorized edits; the original failed-run record must not.

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
