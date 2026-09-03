# External-provider process-tree ownership — Thursday, September 3, 2026

## Why this boundary was required

The regulated Credit Exception release gate ran a controlled real-Claude interrupt probe against
the PR #115 permission-bridge stack. Claude launched a workspace-local Python parent, which launched
a Python child sleeping for 300 seconds. ECorp issued `run.interrupt_requested` at
`2026-09-03T14:17:09.046465Z` and emitted `run.session_terminated` at
`2026-09-03T14:17:09.160096Z` with `provider_process_alive=false`.

That terminal claim was wrong. The verified probe parent PID `36016` and child PID `57268` remained
alive after Claude PID `52988` exited. The operator terminated only those two known probe processes.
The source checkout remained clean. The reproduction is recorded on #51, #117, and #118.

## Implemented ownership model

`OwnedProcessTree` replaces single-root `child.kill()` handling for the external CLI adapter.

- **Windows:** create a private Job Object, enable
  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, start the provider with `CREATE_SUSPENDED`, assign its process
  handle to the Job Object, and resume the primary thread only after assignment succeeds. A setup
  failure after process creation retains the suspended root until cleanup is verified.
- **Unix:** advertise external CLI adapters as unsupported and refuse to spawn. A session/process
  group alone cannot prove that descendants did not escape.
- **All terminal paths:** interrupt, stop, circuit breaker, malformed or oversized protocol input,
  failed control-response delivery, initialization failure, and adapter drop converge on the same
  owned-scope termination path.
- **Verification:** terminate the scope, reap the provider root, and repeatedly verify that the
  Windows Job Object is empty before returning from the adapter.

If spawn ownership, termination, or empty-scope verification fails, the adapter fails closed. It
does not treat killing the provider root PID as proof that grandchildren are gone.
Availability timeout returns do not abandon the Job Object: one shared cleanup guardian remains
active, and duplicate probes are rejected until its empty-scope verification finishes.

## Deterministic evidence

Run from the repository root:

```powershell
cargo fmt --check
cargo test -p crony-runner process_tree -- --nocapture
cargo test -p crony-runner adapter::external::tests -- --nocapture
cargo clippy -p crony-runner --all-targets -- -D warnings
```

The focused process-tree filter covers:

1. a stubborn parent and grandchild are removed;
2. repeated termination is idempotent;
3. an unrelated process remains alive;
4. a tree whose grandchild already exited still terminates cleanly; and
5. an injected verification failure retains ownership for a successful retry; and
6. an injected root-query failure retains ownership for verified cleanup.

The external-adapter filter includes real child-process fixtures for both interrupt and stop, in
addition to the Claude stream-JSON permission suite.

## Release gate

The original real-Claude probe failed and is intentionally preserved as evidence. After ECorp
creates a verified candidate commit, the operator must build a runner from that exact commit and
repeat the parent/grandchild interrupt probe. Publication is allowed only if:

- the Claude root, tool shell, Python parent, and Python child are all absent;
- `run.session_terminated` follows verified subtree cleanup;
- no manual process cleanup is required;
- the configured source checkout remains uncontaminated.

This boundary owns provider descendants. It does not replace the still-open network sandbox,
isolated provider-home, or inherited-environment allowlist work in #51.
