# External-provider fail-closed lifecycle — Thursday, September 3, 2026

## Verified implementation

Implementation commit:

```text
50379ba61eafbc798655000ee95f096ccd2fc4df
```

That commit is the immutable runner source used for the real-Claude release probe. The subsequent
documentation commit records the observed results without changing runner source.

The implementation provides:

- suspended Windows provider spawn before private Job Object assignment;
- retained cleanup ownership when assignment or resume setup fails after process creation;
- root-exit observation independent of inherited stdout and stderr handles;
- bounded initialization, protocol writes, permission responses, input shutdown, stream drain, and
  availability probing;
- stop, interrupt, and hard-breaker preemption during initialization and protocol writes;
- positive empty-Job verification before returning from the adapter;
- one retained availability cleanup guardian per adapter, with overlapping probes rejected;
- no accepted Claude completion without a valid `result` frame whose subtype is `success` and whose
  `is_error` field is `false`;
- Unix external CLI capability disabled until a non-escapable host ownership boundary exists; and
- runner-level suppression of terminal events and workspace cleanup while teardown is unverified.

## Exact-commit local gate

The worktree remained clean and pinned to the implementation commit throughout this gate:

```powershell
node tools/check_migrations.mjs
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p crony-runner 'adapter::external::tests::' -- --nocapture # five times
cargo test -p crony-runner teardown_fail_closed -- --nocapture
cargo test -p crony-runner process_tree -- --nocapture
pnpm build:web
pnpm lint:web
node --check scripts/fake-external-agent.mjs
git diff --check
```

Observed results:

- migration manifest: 30 immutable migrations;
- workspace tests: passed;
- external adapter: 22/22 passed in each of five consecutive runs;
- runner fail-closed: 2/2 passed;
- process tree: 6/6 passed;
- web build and lint: passed;
- Node syntax and Git whitespace checks: passed.

The negative-path coverage includes:

- post-spawn ownership setup failure with verified cleanup;
- injected root-query and empty-scope verification failures;
- availability timeout with retained single cleanup guardian;
- stalled initialization preempted by stop, interrupt, suspend breaker, and stop breaker;
- blocked stdin write preempted by stop;
- successful root exit with inherited pipe handles, both with and without a buffered terminal result;
- error and malformed Claude result frames rejected as completion;
- repeated stop idempotency;
- unrelated-process survival;
- runner-shutdown adapter drop;
- false `run.session_terminated` suppression; and
- an actual clean Git worktree whose path, branch, HEAD, bytes, registration, and cleanliness remain
  unchanged until teardown becomes verified.

## Real provider evidence

The exact runner binary SHA-256 was:

```text
1822f8de30828ed356db6fc2b9452b9cf3d63ec4d08146936418ef811ab56aa1
```

The live server-to-runner-to-Claude probe used:

- mission `b94edf34-3fb3-41d1-bc52-8056c07d203a`;
- run `21288e49-f5da-4fc5-9b46-441fee49d4b5`;
- approval `5c42eb17-82b4-429d-aa62-3de63792b7f5`;
- Claude PID `40328`;
- PowerShell PID `21704`;
- Python parent PID `36300`; and
- Python child PID `8636`.

The interrupt event was sequence 28 at `2026-09-03T23:28:35.491103Z`. Verified session termination
was sequence 29 at `2026-09-03T23:28:36.015419Z`, followed by cancellation at sequence 30. Every
original provider descendant was absent after termination, the runner survived, no manual cleanup
was required, and the source checkout stayed clean.

See `2026-09-03-external-provider-process-tree.md` for the original false-terminal reproduction,
the full lineage, and the corrected exact-commit release gate.

## Review and hosted-CI status

Two independent read-only reviews found no remaining blocking, P1, or P2 issue after result-frame
validation and bounded availability-guardian fixes.

GitHub Actions jobs were created but rejected before running any step because account billing or the
spending limit prevented job start. All claims above come from persisted local ECorp events,
process-identity checks, exact-commit tests, and the authenticated installed Claude provider.
