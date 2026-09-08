# Native stopped-source checkpoint emission

- **Issue:** #190, first implementation layer of #148.
- **Source base:** `d1d9dcc113345486a1fb7abbe4365707bd514521`.
- **Observed:** September 8, 2026, Windows.
- **Status:** native-runner coverage passed; full-stack budget recovery remains unproven.

## Implemented boundary

The runner reuses native `CircuitBreaker` delivery, workspace verification and
fingerprinting, and the existing `run.workspace_preserved` event. No second
execution loop, approval API, checkpoint database, or provider session is added.

A hard directive received before ordinary verification wins over a late provider
`Completed` result. The provider stops first. The runner then reports retained
source before its native terminal event rather than entering ordinary verification.
The source-checkpoint proof binds Corp, mission, task, run, workspace, agent and
runner identities; the original repository/ref/base; current branch and HEAD;
the workspace fingerprint; and hashes of the verifier, write-scope and deliverable
policies. The proof contains no assignment credential or absolute host path.

The existing workspace predicate verifies the managed root, branch and original
base ancestry before and after reading bytes. HEAD must remain unchanged during
capture. The checkpoint-only read has a 4 GiB ceiling and a cooperative read
deadline; an overall ten-second capture deadline releases terminal reporting
through the retain-without-proof fallback. Ordinary workspace fingerprints keep
their existing semantics. This is not a claim that the operating system can
forcibly cancel an indefinitely stalled filesystem read.

The fingerprint covers the supported workspace tree, including ignored entries,
and excludes root Git control data. It is not a portable source export. Existing
deliverable filtering and write-scope enforcement still apply to future export.
Unpinned source, uncertain teardown, failed integrity checks, and quarantine do
not produce a trusted proof; retained work is not deleted to make the check pass.

## Late directives and cleanup

Ordinary verification now uses the existing cancellable check execution and
native verifier-process teardown. Its upload acknowledgment wait observes the
same cancellation signal. A hard directive arriving after verification has
started cancels that work and retains the workspace **without** pretending that
post-verifier bytes are a pre-verification checkpoint.

One atomic phase chooses hard-boundary retention or ordinary finalization before
cleanup awaits. A directive that loses to committed finalization is rejected,
not acknowledged as retaining a worktree that cleanup can remove. Only applied
circuit-breaker commands enter the native command-ID cache; a negative
acknowledgment remains negative when the same command is retried.

## Observed verification

The final full repository gate passed:

| Check | Result |
| --- | --- |
| Immutable migration manifest | 38 migrations passed |
| `cargo fmt --check` | Passed |
| Workspace/all-target Clippy, warnings denied | Passed |
| Serial workspace Rust tests | 367 passed, 0 failed, 96 opt-in SQLx cases ignored |
| Runner tests within that gate | 138 passed, including all 16 `issue190_` cases |
| Web build and lint | Passed |
| Whitespace and exact source-hash check | Passed |

The new cases run the actual assignment executor with deterministic in-process
adapter fixtures and real isolated Git worktrees. Late-stop cases hold a real
Node verifier or a native deliverable-upload acknowledgment. Cleanup coverage
holds the real workspace-operation mutex and retries one command ID twice.
Other cases cover suspend/stop, completed/cancelled/failed exits, runtime
uncertainty, unpinned source, policy/content binding, branch/base mismatch,
quarantine, byte limits, and blocked capture deadlines.

Independent source review exposed the late-verification, capture-duration and
finalization-window gaps. They were corrected and the regressions expanded.
The earlier 11-, 15-, and 16-case focused outputs remain in the task transcript;
the final full gate additionally covers the command-ID replay assertion.

Exact commands, original output, and hashes for all four product files are in
`C:\Users\shyamsridhar\.codex\dogfood\remaining-work-20260908\issue190-20260908T111437742\result.json`
and its adjacent logs. The full gate completed at `2026-09-08T11:22:19Z`.

## Not established by this layer

This does not prove server receipt/persistence of the new proof, browser
acceptance, a real vendor run, or full #148 recovery. The existing store,
source-correction, generic resume, budget/attempt, review and publication gates
are unchanged. In particular, budget-stopped sources are not newly admitted to
verifier-only recovery by this patch.

Next layers must validate the persisted checkpoint in the existing recovery
aggregate, preserve original spend and lineage, run the unchanged verifier policy
without a provider, create the required non-empty verification bundle, and prove
authorized review/publication and restart/tamper negatives. A late stop after
ordinary verification begins still lacks an original pre-verification checkpoint.

No manual service, runner identity, provider home, application database, retained
mission, or protected #172 QA relay was changed. No container, real-provider run,
hosted Actions, auto-merge, or deployment was used for these checks.
