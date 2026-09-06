# Mission launch admission — September 6, 2026

Work: #155; parent #63; related #145.

Base: `1762ab46f712dea0709847a267029af8c5623ee9`, including the current office and Copilot work.

**Local validation passed. This record is not a merge or full dark-factory completion claim.**

## Product change

The persisted mission lifecycle is the admission boundary: `ready` is held, while `running`
permits automatic continuation. Corp scheduling excludes held plans; per-mission scheduling
admits them only for an explicit authenticated operator; run creation repeats the check while
holding the mission/task/agent locks.

The operator's current role and room membership are retained through the admission transaction.
The first `run.requested` event identifies the actor and `mission_launch: true`. Dependencies and
bounded retries remain part of the admitted mission, without another approval layer or migration.

Newly dispatched run IDs remain the launch response. Historical fallback requires a running or
completed mission, positive persisted `run.started` evidence, and no `dispatch_not_started`
marker. Failed/cancelled missions and merely allocated runs are not successful replays.

The UI renders saved plans as **Awaiting dispatch** and explains that briefing holds are kept
by the server rather than the browser.

## Local test boundary

The isolated QA service used its own database and independent Git fixture source:

```text
ecorp-fixture/launch-admission
HEAD b67af32c59ba6ee1e7bd4700b5bf25c0e60d1f3a
```

That GitHub-shaped identity is synthetic; no repository, issue, Project change, or publication
was performed against it. Execution uses `fake-process` and protocol-faithful external fixtures.
It is not real Copilot inference, a three-agent game, or external-team evidence.

The existing manual UI, server, game, branding work, and unfinished recovery branches were not
replaced or reset. Hosted GitHub Actions was not used.

## Verified hold and restart

- Ordinary held mission: `fbe8889c-f92f-4156-b233-a33ec5813fb8`.
- Materialized factory plan: `ed57c27d-d121-4c1d-a867-b1d2a7e3f0d5`.
- Browser-created held mission: `01b5fd91-0578-4859-ba68-faa4853712bb`.

Unrelated same-Corp work completed and failed while the held plans remained `ready` with zero
runs. The browser flow used the actual repository confirmation, fixture runtime, briefing
checkbox, and plan-submission controls, then closed the client before unrelated work completed.

The supervisor verified old server PID `43424` and runner PID `60080` exited, then verified
replacement server PID `16556` and runner PID `52664`, their executable paths/start times,
service health, and reconnection. Database and source were retained. All three plans remained
held after restart; the fixture source HEAD and working tree were unchanged.

Browser dispatch subsequently completed run `3e6ef26a-d8d9-47c8-9c63-d626fcf12ce4` with exactly
one attempt and unchanged authority. Replay returned that run without another attempt.
The 390-pixel browser had 390-pixel document width and no page errors.

The ordinary plan completed run `c3cdadf0-42ed-45b5-9d19-550366b95116` with one attempt.
Concurrent/repeated requests returned `409, 200, 409, 409, 409, 200`; successful responses
identified that same run. Early conflicts did not create duplicate work. Guest dispatch was
rejected and contract/source/budget/verifier authority was retained.

## Retained negative parallelism result

The initial graph case `b423d172-2760-452d-823d-1b14ec1338e6` completed but did **not** prove
physical root overlap: the Claude fixture completed at event sequence `133`, before the
fake-process root started at `134`. This execution is not counted as parallel proof.

The fixture's Claude stream-input path ignored the `[slow]` marker carried by the mission,
although its older argument-based path honored it. The fixture was corrected to apply the same
bounded delay in stream mode. The overlap assertion is unchanged; a distinct fresh graph case
was required after that correction. The original runs and failed assertion are retained, with
their original fixture hash; they were not reset or relabeled as passing.

## Completed regression cases

The corrected fixture passed with a distinct graph, `213173f1-1c78-4bcf-89a7-9fdb13a6503b`.
Live journal inspection verified both roots started at sequences `168` and `170`, before either
completed at `183` and `189`. The join was not requested until `190`. All three tasks completed
once in distinct worktrees, and the join artifact contained the actual verified dependency bytes,
run IDs and hashes. This proves two overlapping deterministic roots plus their dependency join,
not three simultaneous Copilot agents.

| Case | Fresh evidence |
| --- | --- |
| Failed first dispatch | Mission `4db0b77b-f96c-43cc-bcd2-5694aa4777d3`, run `d0322af6-5dbd-4a60-b900-0e73fc447284`: missing-secret denial returned 409; three repeated launches also returned 409; one failed attempt, `dispatch_not_started`, zero `run.started` events. |
| Fresh continuation IDs | Mission `8f2f56ce-f54a-404b-a095-2a005db26d0c`: after releasing only a test-owned blocked worker, explicit dispatch returned new run `290e4883-546f-4070-a64b-77f4083071f9`, not historical root `b478cff0-9164-49db-96e5-7c203f7677aa`. The three-node graph completed with verified dependency bytes. |
| Retry bound | Mission `630888d2-f4fe-4954-a46b-4ae97dc585d9` failed after exactly two allowed attempts; unrelated held work did not start. |
| Factory plan release | The original synthetic factory plan remained held through the other cases, then explicit launch completed run `df80ebcb-67b8-4027-ab6a-348e4fbf6acd` once with unchanged source, contract, budget and verifier. |

The releasing checkpoint resumed its already-verified ordinary mission without another launch
or replacement mission. One explicitly authorized overlap recheck required retained negative
evidence and a changed fixture hash. The test cannot silently repeat an unchanged failed fixture
or discard an ambiguously started check.

This materialized-plan case is not live GitHub intake, a controller-service recovery run,
publication, or merge evidence. Those broader factory requirements remain separate.

## Quality gates

- 34 immutable migrations verified; no migration was added or renumbered.
- `cargo fmt --check` passed.
- Strict offline workspace/all-targets Clippy passed.
- Final `cargo test --offline --workspace` after the fixture correction: **153 passed, 0 failed**.
- Web build and lint passed.
- Both executable admission regressions passed, including the actual browser flow.
- Compiled server/store/protocol/source-lockfile hashes matched the tested source.
- Fixture source HEAD remained `b67af32c59ba6ee1e7bd4700b5bf25c0e60d1f3a`, with a clean working tree.

Raw local checkpoints, process evidence, logs, fixture hashes and screenshots are under
`C:\Users\shyamsridhar\.codex\dogfood\issue155-launch-admission-20260906`.
