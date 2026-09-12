# Scheduler standalone acceptance

September 12, 2026. PR #179 / issue #172.

The scheduler rotates the representative runner after each actual Corp visit,
including a failed command drain or stale candidate. A healthy peer therefore
receives a later turn while each tick retains the one-attempt-per-Corp and
100-Corp bounds. The reviewed singleton-churn correction and epoch fences are
unchanged.

This candidate merges main `b28fd4d26309794f38c0455bbf42d22aedf7cfd1` and
the already-reviewed shared CI fixture dependency from #234,
`bb3a38f4174c15c592f073dd12f799eca0a4310c`. The latter changes test support,
not the scheduler or other Rust product code.

| Standalone gate | Result |
| --- | --- |
| Rust workspace, serial | 459 passed; 0 failed; 200 intentionally ignored |
| Scheduler/readiness cases within that workspace | 7 issue172 + 9 issue171 passed |
| Workspace/all-target Clippy and formatting | Passed |
| Recursive frontend suite, all 13 files | 194 passed; 0 failed |
| Web build and lint | Passed |
| Immutable migrations | All 40 passed |
| Shared CI fixture regressions | 65 passed; 0 failed |

The [standalone receipt](2026-09-12-scheduler-standalone-validation.json)
binds these results to the tested source objects and retained log hashes.
The earlier 177-test frontend invocation omitted the nested office suite; the
194-test recursive invocation above includes it. Mirrored dependency installation
preserved the repository lockfile; the local pnpm metadata warning remains in
the build and lint logs.

Fresh live acceptance ran the unchanged scheduler patch together with the
recovery stack, on the separately built and hash-bound combined server. Two
native QA runners reconnected to the same retained database and source tuple.
The existing four-run recovery history, accounting, review, source hashes and
signed download remained unchanged; replay after sequence 94 reached 99 with
unique ordered events. The temporary peer and its credential were revoked and
retired after the check. See the [combined runtime report](2026-09-12-pr-stack-acceptance.md)
and its [machine receipt](2026-09-12-pr-stack-validation.json).

That combined runtime lane is separate from this standalone source gate. It
uses native protocol fixtures and development principals, and proves reconnect
and retention compatibility. It does not inject persistent failure into a live
vendor runner or claim a fresh vendor inference session. The September 7
transport-denied run remains a failed historical attempt; this new case does
not relabel its old mission or downloads as passing.

Hosted CI is tracked against each actual PR head. Local acceptance neither
changes repository protections nor asserts that a PR was merged or deployed.
