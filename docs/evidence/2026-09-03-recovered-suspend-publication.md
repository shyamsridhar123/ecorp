# Recovered suspend publication validation

**Issue:** #98

**Source base:** `5999a4c808867796671dbafe02ccd8af69a74445`

## Failure reproduced

Incident Command correction issue #96 completed through the governed factory:

- original run `54a40fe7-25d0-4464-ae7e-c2f6ae187dc6` reached `suspend` at 524,308
  tokens and preserved its provider session and worktree;
- an owner approved a bounded mission budget revision from 500,000 to 800,000 tokens;
- two descendants resumed provider session `599f85d6-0ee7-40c1-8b72-1688e179ba4b`
  in the same worktree;
- final run `f947e09f-b985-4be5-bbeb-00a5087b63b2` passed 16/16 verifier checks;
- Bob, not the requester, approved the independent-review gate;
- ECorp created runner commit `ada87bd85dafdc62f354c4641c7e9340be5b1ece`
  and signed deliverable `98e0da3c-1fa4-4eba-b442-d9f1291a3f13`.

The factory reached `verified`, but publication failed before a durable start:

```text
pull-request publication is blocked by circuit breaker stage suspend
```

No remote branch, pull request, or publication row existed after the failure.

## Root cause

Publication correctly performed a full current-authority check on the selected deliverable run.
It then repeated that same hard-breaker check for every historical mission run. This treated a
recovered `suspend` ancestor as terminal even though the explicit resume chain ended in a completed,
verified, independently reviewed run.

## Correction

Publication now builds the selected run's explicit `resumed_from_run_id` ancestry from the
mission's locked run rows.

- The selected run still receives the complete current budget and breaker check.
- `stop` remains terminal in every position.
- A historical `suspend` bypasses the repeated historical-run check only when it is an actual
  ancestor of the selected run.
- An unrelated suspended run still fails.
- A selected suspended run still fails through the selected-run check.
- Missing, duplicate, or cyclic lineage fails closed.
- Every mission run ID remains in publication provenance.

## Local validation

```text
cargo fmt --all -- --check
cargo clippy -p crony-store --all-targets -- -D warnings
cargo test -p crony-store
cargo check -p crony-server
```

All 19 `crony-store` tests passed. New focused coverage proves:

- recovered suspend ancestors are recognized;
- unrelated suspends are not recognized as recovered;
- stop ancestors are never treated as recovered;
- the selected run cannot bypass its own hard-breaker check;
- missing and cyclic resume lineage fail closed.

## Live recovery proof

The patched server restarted against the same Postgres database and object store. The existing
runner reconnected through its outbound session.

The exact issue #96 publication then succeeded:

- publication: `999a8353-d14a-4386-86dc-5a11b582bdd5`
- pull request: #99
- branch: `ecorp/incident-command-review-fixes-96`
- exact head: `ada87bd85dafdc62f354c4641c7e9340be5b1ece`
- Project transition: `In Progress -> In Review`
- attempts: 1
- auto-merge: false
- merge: false
- deployment: false

Replaying the identical publication command returned `mode: recovered`, the same publication, branch,
PR, and head, and retained one attempt. No duplicate remote effect occurred.

Hosted GitHub Actions could not start because of account billing limits and were not used as the
validation gate.
