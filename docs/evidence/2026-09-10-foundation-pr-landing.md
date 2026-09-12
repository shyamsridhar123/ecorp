# Foundation PR landing validation — September 10, 2026

## Scope

The candidate extends `77c10fbbec31fa7954a2ddcdc65dc8e8eb3670c7` (PR #209).
It contains the lower native stack #203: #200, #201, #202, #207, #208 and #209.
It does not include #212, #214, #215, #217, #218 or unfinished #219 changes.

The correction is carried forward into #209 so the six PRs can land atomically
without rewriting branches or dissolving/recreating the existing GitHub stack.
The native stack extension's add operation only appends; it cannot insert a
new review-fix layer in the middle of an existing stack.

## Review corrections

- Canonicalize GitHub's case-preserving repository identity, while retaining
  the exact repository node ID and case-sensitive branch.
- Do not advertise native connection setup on an unsupported process-ownership
  platform. Stale setup commands receive an unavailable result without execution.
- At the setup receipt bound, retire only an expired, terminal, acknowledged,
  unreferenced receipt outside the active-operation set. Live commands, pending
  acknowledgments and all retained source/snapshot proofs stay protected.
  Expired evicted commands cannot execute again; no archive subsystem was added.
- Include ready bound runtimes in the unscoped status projection while retaining
  exact binding constraints for target-specific dispatch selection.
- Poll presence only while a connection-dependent view is open, sharing the
  existing refresh coalescer without reconnecting its socket.
- Keep ordinary native resume available when an optional Factory recovery lookup
  has no eligible source. Historical checks are scoped to the selected task and
  workspace; unrelated task history does not block it. Current governed recovery,
  cancellation, stop and server-side authority checks remain.

## Shared hosted-check failures

The failed jobs on #209/#212/#214/#217/#218 were inspected. They executed and
failed; these were not billing or provisioning errors:

- Unix Clippy: three Windows-only imports in connection tests were unconditional.
- macOS: an owned temporary fixture needed canonicalization; a filesystem rejected
  creation of a non-UTF-8 filename before fingerprint verification could run.
  Product containment was not relaxed. Linux keeps the physical fingerprint test;
  macOS separately checks its creation rejection.
- Codex integration: a hard stop now intentionally retains source and cancels the
  run, instead of attempting an artifact upload that produces `run.failed`.
  The regression requires exactly `cancelled`, coherent task/mission state,
  retained source, stop/acknowledgment/termination ordering, no verification or
  completion, and one run.

No hosted run was retriggered and no hosted success is claimed. Unix/macOS
execution of the corrected fixtures was not performed on this Windows host.

## Local evidence

Checks used the isolated foundation checkout, not the dirty #219 worktree.
The local receipt root is `pr-drain-20260910` in the owner's dogfood evidence.
Receipts retain source hashes, actual executable/arguments, exit code, timing,
and complete stdout/stderr hashes.

- 40 immutable migrations; formatting; workspace/all-target Clippy: passed.
- Rust workspace: **452 passed, 0 failed, 200 intentionally ignored**.
  The established serial lane was used; this is not a parallel-suite claim.
- Frontend: **177 passed**; native web build and lint: passed.
- Six runtime executables were freshly built from the foundation manifest paths.
- Native Codex protocol-fixture E2E: the bounded repeat passed all four scenarios,
  including the strengthened hard-stop assertions. This is deterministic protocol
  evidence, not genuine vendor session-persistence or model-inference evidence.
- Actual browser/server/runner: save without dispatch, unrelated work, client
  closure, an observed server/runner/web restart, explicit dispatch, accepted
  completion and identical launch replay passed. The held mission retained its
  authority and had one eventual run/attempt. The 390px viewport had no overflow
  and the browser recorded no page errors.
- Actual browser network observation: over separate 6.5-second windows, the
  closed floor made zero snapshot reads, opening the guide made one, and hiding
  the guide returned to zero. No page errors occurred. The first supplementary
  probe looked for a nonexistent Close label; it was corrected to the observed
  Hide guide control without a product change.

The QA lane reused the existing PostgreSQL container with a separate disposable
database, source repository, credentials, worktrees and object store. The retained
schema-41 application database and its original records were not reset or downgraded.

## Retained failures and remaining work

The first frontend run was **176/177**: a backported test helper lacked its optional
AST scope argument. That fixture-only correction preceded the final 177/177 run.
The initial local supervisor selected two `node.exe` paths; only its owned
processes were stopped before selecting one executable and reusing enrollment.

The first Codex E2E failed during steering with an agent/run foreign-key lock
inversion. Its failed receipt and original run snapshot were retained before the
one bounded repeat. The complete relevant store functions and message handler
match pre-existing main `971445e1cbf9388c51803e2adf28b11bd98b1ffa`.
**#223 tracks this pre-existing race; the repeat does not fix or disprove it.**

The repeated checkpoint-derived correction admission defect is tracked in #221.
PR #212 and its descendants remain a separate landing batch. #179's scheduling
fairness runtime acceptance remains independent. No global deadlock-freedom,
production identity, real-provider, deployment, or complete enterprise-readiness
claim follows from this report.
