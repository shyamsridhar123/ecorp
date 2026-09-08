# Late provider-termination accounting

- **Issue:** #193; dependency of #148.
- **Source base:** `3524cf4956392ebc929032a823f6febb641855ec`.
- **Observed:** September 8, 2026, Windows.
- **Status:** store and fixture checks passed; native race acceptance pending.

## Boundary

An upload already allowed before the runner receives hard control can be rejected
by the server's authoritative budget check. That logical failure must not erase
a later factual report that the original provider process has stopped.

The existing runner-event path now admits `run.session_terminated` alongside
workspace cleanup on terminal runs. Its exact Corp/run/agent/runner/assignment
fence remains in place. The server's registered/current connection-epoch check
also remains unchanged; this patch does not claim a new atomic epoch guarantee.

The store validates provider execution mode, false `provider_process_alive`, one
of the native outcome values, bounded optional message text and exact payload
keys. Adapter identity comes from the task's pinned adapter or unambiguous native
start evidence, never a newly edited current-agent adapter. Conflicting or
unproven assignment metadata is rejected.

Acceptance appends only the original room-scoped journal event. It does not
change run/task/mission status, verification, spend, attempts, source checkpoint,
Factory state, queued commands or current agent ownership. Provider outcome
`completed` remains distinct from accepted ECorp completion.

## Actual-store verification

The new `issue193_` SQLx family applied real migrations in disposable test
databases through the already owned maintenance fixture:

- **10 passed, 0 failed**, 29.67 seconds.
- Native failure followed by late termination and exact duplicate replay.
- Completed/cancelled/lost states and quarantine remain unchanged.
- Another current assignment, in all six active states, remains untouched.
- Wrong Corp/run/agent/runner/token and malformed payloads do not append.
- Legacy start evidence binds the old adapter despite current agent edits;
  missing, conflicting and ambiguous adapter authority is rejected.
- Terminal progress remains rejected; active termination is also validated.
- Journal insertion failure rolls the entire operation back.

The first test setup failed because its review-state snapshot ordered the
`verification_requests` table by nonexistent `id` instead of `run_id`; that
fixture error is retained separately. The corrected test then reproduced the
real admission defect against the old predicate: **0 passed, 1 failed** with
`runner event does not match an active run`. Restoring the scoped termination
admission produced the ten-case pass above.

No older ignored SQLx family was executed. The loader supplies the exact
maintenance connection only to its Cargo child, not through ambient
`DATABASE_URL`, command arguments or evidence.

## Native race probe

The additive protocol marker `[budget-queued-completion]` writes the existing
fixture source, then emits two usage updates and completion synchronously.
Existing budget-stream modes are unchanged. This is a race candidate, not proof
by itself.

`tools/e2e_late_termination.mjs` reuses the existing scoped HTTP client, bounded
Ready-watermarked journal replay and ownership receipt. It preserves mutation
intent and IDs before requests, never retries create/launch, and supports only
intentional continuation of the same case. It has no reset, policy change,
enrollment, provider-resume, SQL or process-control path.

A runtime pass requires actual journal order: usage, hard stop, artifact-rejection
failure, then valid provider termination. The real command acknowledgment order
is reported, not manufactured. A missed race or missing telemetry fails closed
and retains the case. No transport bytes are delayed, dropped or retagged.

The helper/protocol test set, including existing #190 helpers, passed **165 pure
Node tests**. This is not native runtime acceptance, real-vendor persistence,
global deadlock freedom or full #148 recovery.

Original commands and SQLx logs are retained under
`C:\Users\shyamsridhar\.codex\dogfood\issue193-terminal-telemetry-20260908`.
Native runtime results and the final repository gate will be recorded separately.

## Pre-runtime repository gate

The unchanged source passed all required repository checks: **370 Rust tests**
(106 opt-in SQLx cases ignored in that ordinary run), 38 immutable migrations,
format, workspace/all-target Clippy with warnings denied, web build/lint and
whitespace. The ten new SQLx cases above ran separately; older ignored families
were not selected. Independent scoped review reported no concrete finding.

The hash-bound receipt and original output are retained at
`C:\Users\shyamsridhar\.codex\dogfood\remaining-work-20260908\issue193-20260908T134006563\result.json`.
This gate does not substitute for the queued-upload runtime probe.
