# Checkpoint-derived correction retry — September 10, 2026

> September 12 follow-up: the patched public recovery sequence and publication
> acceptance now pass. See [current stack acceptance](2026-09-12-pr-stack-acceptance.md).
> The dated failures below remain historical evidence.

## Scope and remaining acceptance

This repairs #221 for a valid stored task policy with remaining attempts. It
reuses the existing correction, session, workspace and verification mechanisms.
There is no new execution mode, migration, retry engine, budget exemption,
default-attempt change or post-failure counter reset.

**Complete public two-correction acceptance is not established.** Every current
public manager creates tasks with at most two attempts. The domain validator
permits three, but no public planning setting or graph import establishes that
allowance. The original provider consumes attempt one, checkpoint verification
does not consume another, and two provider corrections require attempts two and
three. #224 records the missing pre-execution planning surface.

The SQLx fixture sets `max_attempts=3` at construction. It is valid stored-policy
evidence, not proof that a user can configure three attempts in today's UI/API.

## Implementation

- Reconstruct bounded native correction history through exact recovery, run,
  source, connection, session, request, command, revision and authorization-event
  bindings. Retain schema-1 authority shape and all original rows.
- Verify historical revisions privately, including native revision bridges.
  A historical revision is not current execution authority: admission, dispatch
  and replay still enforce the new current revision and actor/room authority.
- Reuse the same reconstruction for retained receipts and publication. Missing,
  NULL or damaged required grants cannot become ordinary correction.
- Preserve stop, quarantine, unrelated-suspension, current authority, remaining
  allocation, attempt and rollback guards. Publication's exact typed checkpoint
  exception remains publication-only.
- Context retains the latest failed provider's current fingerprint and nullable
  exported head. A checkpoint-family flag is not checkpoint-verification
  availability. Only explicitly available source correction is offered.
- CLI mode checks, dry runs and active-recovery replay retain current native
  authority and request equality. A false new-work flag cannot create more work,
  but it does not suppress an otherwise congruent active replay.
- Frontend null-head handling permits only explicit source-correction permission.
  It does not grant checkpoint verification, ordinary Resume or cancelled intent.

## Local validation

The isolated checkout combined the existing #218 tip `0113080a...` with the
validated foundation commit `578002a...`; unfinished #219 files were not used.
Source manifests and complete logs are retained in the owner's
`issue221-correction-retry-20260910` dogfood evidence directory.

| Check | Observed result |
| --- | --- |
| Actual-store RED | One test failed at the intended old checkpoint-only admission gate |
| Same baseline after fix | 1 passed; prior history and current dispatch assertions retained |
| Final new SQLx cases | 11 passed, 0 failed |
| Four exact SQLx compatibility controls | 4 passed: initial correction, retained receipt collection, publication/replay, erased-grant denial |
| CLI focused tests | 11 passed |
| Frontend tests | 233 passed |
| Full serial Rust workspace | 503 passed, 0 failed, 317 intentionally ignored |
| Migrations / formatting / workspace Clippy | Passed; 41 immutable migrations, warnings denied |
| Web build / lint | Passed |
| Native executable build | Passed, with source-bound binaries |

The final SQLx run covered the exact source after the style correction. It
includes current/replayed admission, native ordinary and pre-dispatch failures,
missing/damaged grants, source/request/command/revision bindings, protected
history, authority revocation, exhausted spend and transactional rollback.
The four compatibility controls ran individually, not as broad old suites.

## Runtime and browser regression

A separate disposable database reused the already-owned PostgreSQL container.
Existing application databases and real provider profiles were untouched.

- All four native Codex **protocol-fixture** lifecycle scenarios passed. These
  cover ordinary start/steer/resume, interruption, stop and hard-stop retention;
  they do not establish genuine vendor session persistence or the three-attempt
  correction sequence.
- Browser → server → runner saved a held plan, ran unrelated work, closed its
  client, survived an observed server/runner/web restart, then explicitly started
  the same mission and completed it. Launch replay created no second run or
  attempt. The 390px viewport had no overflow and recorded no page errors.
- The browser fixture initially failed because the result cockpit introduced a
  second “Awaiting dispatch” label. Its selector now targets the status chip and
  its launch action targets “Start mission.” Explicit `resume-prepare` continued
  the same recorded held mission; no replacement mission or history reset hid
  that failure.

## Retained limitations

The initial compile lacked a `PgRow` import, and the first full Clippy check
found a collapsible conditional. Both failed receipts remain retained. The
compatibility loader also rejected its own parameter reassignment before any
test executed; a separate effective filter corrected that fixture issue.

No hosted Actions success, real-provider inference, global deadlock-freedom,
production identity, deployment or complete enterprise-readiness claim follows.
The pre-existing steering lock inversion remains #223. The first PR batch's
protected-branch merge decision remains separate; no owner override is implied.
