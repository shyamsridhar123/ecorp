# ECorp: bounded runner scheduling fairness

Date: September 7, 2026. Issue: #172. Integration: existing draft PR #170.
This is a control-plane coordination fix, not a replacement agent harness or
a claim that the whole dark factory is production-ready.
**Status: implementation locally validated; final reviewed-binary two-runner
runtime acceptance remains incomplete.**

## Cause and correction

The periodic scheduler sorted ready runner snapshots by `(corp_id, runner_id)`
and kept the first representative for each Corp. If that representative's
command handling repeatedly returned an error, a healthy peer could never
receive the periodic scheduling opportunity.

`CorpScheduleCursor` now preserves cross-Corp round-robin and advances the
runner representative on each **actual Corp visit**, including a failed or
stale candidate. Each tick still makes at most one command/scheduling attempt
per Corp and visits at most 100 Corps. The existing ready-runner snapshot/sort
remains; the change does not introduce a per-peer command retry loop or another
watcher.

The existing command-before-scheduling, current-Corp, current-epoch and
dispatch-readiness fences are unchanged. Task/run admission, attempt limits,
budgets, verifier policy, source matching and provider behavior are unchanged.

### Independent review correction

Initial singleton pruning was too eager. With 200 Corps, failing A could become
unready on an alternate tick when its Corp was outside the batch. Deleting that
unvisited Corp's cursor let returning A take the next visit again, starving
continuously ready B.

The reviewed fix prunes disappeared Corps immediately, but retains a singleton
cursor until that Corp has a current, actual visit. A stale singleton candidate
cannot reset the next peer's turn. Cursor state is bounded by Corps in the
current ready snapshot, rather than accumulating departed Corp history.

## Deterministic evidence

The original first-runner algorithm failed the new regression with:

```text
healthy runner was starved; attempted ["a-failing", "a-failing", "a-failing", "a-failing"]
```

The independent review added another genuine red case against the first fix:

```text
left:  ["001-a-failing", "001-a-failing"]
right: ["001-a-failing", "001-b-healthy"]
```

That invocation exited 101: **0 passed, 1 failed**. After the correction:

- **7/7 `issue172_` tests passed**: persistent representative failure, all-failing
  bounds/rotation, 200-Corp batching, combined batching/unvisited-singleton
  churn, epoch replacement during commands, representative removal/cursor
  cleanup, and a runner moved to another Corp.
- **9/9 existing `issue171_` scheduler/readiness tests passed**, with their
  pre-existing test bodies preserved.
- Scoped Rust formatting passed.

The test closures exercise the native server selection and command-fencing
helpers. They do not inject a persistent command failure into a live runner or
prove that a new live Copilot task was dispatched.

## Final local workspace gates

Source base: `bb98a0de0f6e5651e2aed0b96e629873376e17c3`.
Reviewed server source SHA-256:

```text
02F0C68D50A85AC2EDEFD23E5D7D76F19A1402AF5FB2107996ABFADD880ECCA1
```

| Gate | Result |
| --- | --- |
| `node tools/check_migrations.mjs` | 38 immutable migrations; pass |
| `cargo fmt --check` | Pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | Pass |
| `cargo test --workspace -- --test-threads=1` | 358 passed, 0 failed, 62 opt-in cases ignored |
| Frontend `node --test` suite | 99 passed, 0 failed |
| `pnpm build:web` | Pass |
| `pnpm lint:web` | Pass |
| `git diff --check` | Pass |
| `cargo build -p crony-server` | Pass; separate normal server executable |

The source hash matched before and after these gates. No ignored SQLx suite was
executed. The earlier pre-review 357-test workspace result, initial six-test
green result and both red failures remain retained; they are not relabeled as
the final reviewed-source run. No hosted Actions result is required or claimed.

## Reboot and runtime compatibility evidence

The user restarted Windows for updates while this work was in progress. The
prior QA process identities were confirmed absent; the already-owned PostgreSQL
container was stopped. Only that existing container and the same owned QA
services were restored, using the original encrypted service keys, database,
source checkout and workspaces. No bootstrap, reset, replacement mission,
factory-watch, application publication or database-row repair was used.

Post-reboot readback against the pre-review #172 server binary verified:

- Both native runners connected: the retained real-Copilot-capable runner and
  the temporary deterministic-process-only peer.
- Both advertised the same authorized repository/ref/commit:
  `shyamsridhar123/ecorp-enterprise-lab`, `HEAD`,
  `e3dc3d669b1a99832e2e7af9be16f7f39842586d`.
- Mission `d561cc2b-8d5e-44c9-b9da-ae6060af78b3` remained completed: four tasks,
  six original runs, including the two original failures.
- Factory `c860fb2b-9b67-46a5-ba46-3db0360c7fd2` remained verified at version 11.
- All six authorized source downloads retained the original hashes, lengths,
  verification digests and provenance signatures.
- Original task/run/session/workspace/accounting/check/review history and the
  clean source checkout were unchanged. No routine approvals were added.
- The browser showed two runners online, the completed mission with six
  selectable run histories, and the verified Factory item.

A temporary peer startup attempt rejected an empty inherited optional Copilot
token-file setting. Its failed process and log are retained. Only that confirmed
exited peer was restarted with correctly removed environment keys; no token was
printed or shared. Its native connection then succeeded using the retained
protected credential.

### Final reviewed binary: transport blocker retained, not passed

The normal reviewed executable was built and its source/binary hashes recorded.
Only the owned QA server was upgraded with the same data and service keys. Its
health check passed and the deterministic peer reconnected.

The QA-only ACK bridge latched the server downtime and returned HTTP 503 with
`status: failed`, leaving the real-Copilot-capable runner offline at the server.
The tool policy rejected the requested scoped bridge restart before executing
it. No alternate route, process-control mechanism or policy workaround was
attempted.

The **unchanged final two-runner verifier exited 1**, correctly reporting only
`a-issue172-peer` connected instead of both required runners. Its original
history/accounting/check/review assertions passed before that connection
assertion; its later signed-download checks did not execute in this final
attempt. The six signed-download passes above belong to the explicitly labeled
post-reboot/pre-review binary observation, not this incomplete final attempt.

Current receipts identify the live reviewed server, failed-closed QA bridge,
disconnected primary runner process, web process and temporary peer. The
temporary peer and its credential files have **not** yet been retired. Finish
the authorized bridge recovery, rerun the unchanged final verifier, observe the
reviewed binary in the browser, and then retire only the owned temporary peer.

See the source-hashed
[validation receipt](2026-09-07-issue172-local-validation.json). The issue stays
open/In Progress until this remaining acceptance and cleanup are complete.

## Scope and limitations

- Live connection/heartbeat/capability state is observable. The public snapshot
  does **not** expose the internal `dispatch_ready` flag; this report does not
  equate a connected badge with direct instrumentation of that flag.
- The live lane is reconnect/retention compatibility, not persistent live
  command-failure injection, a new game build, or fresh model-concurrency proof.
- The earlier real Copilot missing/delayed-ACK, automatic retry, exact native
  resume and independent review evidence remains in
  [the native acceptance report](2026-09-07-native-ack-recovery.md); this work
  does not substitute a deterministic peer for that evidence.
- Development Alice/Bob principals are not separate GitHub logins. No global
  deadlock freedom, exported-game preview or production deployment is claimed.
- Only the server scheduler source and this issue's evidence are part of the
  repository change. QA supervisors/credentials stay outside the repository.
- PR #170 remains draft. No merge or auto-merge is authorized. Neither a local
  test pass nor this partial runtime observation closes #172 or the broader goal.
