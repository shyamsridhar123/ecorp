# Recovery and publication runtime fixes

**Scope:** issues #165 and #166, carried by PR #162. Both previously failing complete
local suites now pass. This is deterministic systems evidence, not a new game build,
production OIDC validation, real GitHub publication, merge or deployment.

## Publication deadlock

The original PostgreSQL log showed two publisher transactions holding shared locks on
one credential and both attempting its `last_used_at` update. A native two-session
regression reproduced that deadlock using the exact production SELECT/UPDATE.

Credential revalidation now takes the required update lock at the first read. Corp,
publisher, hash, revocation and expiry predicates remain unchanged. No retry, command
classifier or additional approval was introduced.

The [red receipt](assets/runtime-recovery-publication/credential-lock-red.json) records
the old failure. The [green receipt](assets/runtime-recovery-publication/credential-lock-green.json)
records both transactions completing after the second waits at the initial SELECT.
The actual publisher-credential fingerprint is identical before and after.

## Recovered-parent handoff

The original upstream recovery had completed; synthesis failed because artifact
selection required the original producer to be the verifier-only run.

Dependency selection now resolves the exact retained provider artifact through bounded,
acyclic governed recovery lineage. Producer and verification run identities remain
separate. Current parent verification, exact source/workspace identity, artifact status,
metadata, signatures, retention and byte checks are not bypassed. A newer unverified
run cannot fall back to an older completed one.

A [read-only replay](assets/runtime-recovery-publication/preserved-graph-readback.json)
of the production query against the original failed database now returns both parent
artifacts. Its existing mission and rows were not changed. Native SQLx regressions also
passed for inherited artifacts, multiple recovery generations, stale/cyclic paths and
missing/cross-boundary/mismatched metadata.

## Complete local acceptance

The [publication report](assets/runtime-recovery-publication/publication.json) completed
the server/runner/CLI/Git drill, including concurrent calls, restart and remote-effect
crash recovery, exact context outside bounded snapshots, source/role/room/budget/credential
denials, and replay after base movement. There is one target branch, one target pull
request creation and one durable publication after 11 attempts. Its GitHub boundary is
local and deterministic; the PR-shaped URL in the fixture is not a real publication.

The [recovery report](assets/runtime-recovery-publication/recovery.json) completed all
eight cases:

- rejected review to verifier-only recovery;
- failed verification through a verifier bridge and native-session correction;
- cancellation and reauthorization;
- exhausted attempts;
- upstream then later-task recovery;
- gate-free recovery;
- acknowledged verifier loss;
- lost verifier recovery with a reviewed revision.

The repaired graph retained three tasks and five runs, consumed both verified parent
handoffs before synthesis, and completed two recoveries within the original mission.
Signed original provider artifacts were retained rather than recreated.

The [validation receipt](assets/runtime-recovery-publication/validation.json) binds the
tested code hashes and commands: 38 immutable migrations, formatting, workspace Clippy,
334 ordinary Rust tests, binary build, web build/lint, 104 frontend/runtime/office/ownership
tests, script syntax and diff checks. Two database-dependent Rust tests are ignored by
the ordinary suite and were separately executed successfully against an owned PostgreSQL
maintenance database.

## Preservation and remaining work

Both full suites used new, isolated synthetic databases inside one **existing** QA
container, one suite at a time. The original failure databases were never reset.
All suite processes and that container were stopped afterward. The manual ECorp
database and unrelated Open Notebook containers remain available.

Independent read-only review found no concrete new regression in these fixes. It
confirmed a pre-existing generic pre-dispatch failure/state-coherence gap, tracked
separately as #167. PR #162 remains draft while that release work is addressed.

#148 budget-boundary checkpoints, #164 real-game completion and the broader dark-factory
objective remain incomplete. No manual mission, hard stop, budget or provider session
was reset or silently rerun.
