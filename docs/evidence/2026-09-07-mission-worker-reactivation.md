# Mission-owned worker recovery

**Date:** September 7, 2026

**Tracking:** #171; related #48, #169, #168

## Reproduced gap

The retained real Copilot mission
`d561cc2b-8d5e-44c9-b9da-ae6060af78b3` completed its three roots after native
saved-session recovery, but integration remained ready at attempt 0.
The Gameplay worker `107b8cbd-b6a6-487c-b1d6-c970bc90813a` had been
automatically retired when that same mission previously failed.

Native resume restored only the resumed visual worker. Both scheduler selectors
and run admission correctly excluded the retired Gameplay worker. The missing
operation was scoped reactivation of required mission-owned staffing—not a need
to rerun successful roots, alter attempts, create replacement agents, or add
another approval/provider loop.

## Narrow control-plane correction

The original terminal-retirement implementation remains unchanged. The inverse
operation uses the existing three-second server lifecycle reconciliation:

- Only unpinned, idle, detached workers of their exact running owning mission
  with unfinished assigned work and attempts remaining are candidates.
- Current native automatic retirement must be proven from its exact scoped
  system event and transaction timestamp. Unknown/replaced retirement is not
  authority.
- Existing active-run, lease, queued-input, approval/command and unresolved
  teardown boundaries remain. Locked candidates are skipped and rechecked.
- Only retirement metadata changes; a retirement-identity-keyed
  `agent.reactivated` event preserves the audit trail.
- Normal scheduling is awakened after committed reactivation and successful
  current-epoch runner reconciliation. Bounded periodic retries do not depend
  on another new lifecycle event, so a transient command/scheduling failure
  cannot permanently lose the wakeup in the single-runner case.

This does not claim global deadlock freedom or general multi-runner fairness.

## Focused checks

The approved isolated SQLx loader ran only the new `issue171_` family:

- **18 passed, 0 failed**, 120.73 seconds; final exec 54012.
- **9 server ordering/retry tests passed**, 0.01 seconds.
- Store/server all-target Clippy with warnings denied passed, 9.75 seconds.
- Scoped rustfmt and whitespace checks passed.
- Earlier 1/17 and 17/18 fixture failures remain in the transcript; both fixture
  defects were corrected without weakening production constraints.
- Neither previous #169 SQLx family was repeated.

Final worker source digests:

```text
staffing.rs
0999795885751D52E925309A4408DE4F21FD02D807942743595ABF68A2CF076D

server main.rs
54277EE42EA72FC6565DC1F28D4E5F7E246C4865908CCFB082AA61A3B77F80AE
```

Popper's scoped review cleared the post-commit lost-wakeup P1. The separately
reported P2 is tracked as **#172**: a repeatedly failing first runner in a
multi-runner Corp can monopolize its representative selection. That finding
does not block this retained single-runner fixture and is not silently counted
as fixed.

## Runtime acceptance

**Passed for the retained single-runner mission.**

The reviewed replacement server was built and substituted for only the owned
QA server, PID 33328 → 14008. The database, original service-key envelope,
source checkout, worktrees, mission, runner identity and all five original
run records were retained. The QA transport bridge latched the deliberate
server downtime as a failure; that diagnostic state was saved and only the
bridge was restarted once the server was healthy. No fault or old ACK command
was replayed.

The native lifecycle operation restored the same Gameplay identity. The
completed Quality worker stayed retired. Current-epoch runner reconciliation
and the normal scheduler then dispatched integration without a manual launch:

```text
Task: 94d99940-a24d-484d-8d24-d809224222f6
Run:  b6f9e47e-4b49-42d5-9467-4772381456b0
```

That real Copilot run produced the requested module/test/doc deliverable and
passed all five persisted checks, including 12 actual Node tests. Bob selected
its one pending independent review in the real UI and accepted it. Mission
completion and Factory verified v11 followed.

The final proof re-downloaded six signed source objects, compared the original
four objects byte-for-byte through their recorded digests, checked original
run history and preserved session/workspace lineage, and verified the configured
source checkout was unchanged. No application database rows, task attempts,
budgets, artifacts or successful roots were manually rewritten.

See [the complete ACK/recovery report](2026-09-07-native-ack-recovery.md) for
the phase chronology and explicit limits. #172 remains open; this is not a
general multi-runner fairness or global deadlock-freedom claim.
