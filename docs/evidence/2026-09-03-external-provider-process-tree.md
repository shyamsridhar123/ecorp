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
4. a tree whose grandchild already exited still terminates cleanly;
5. an injected verification failure retains ownership for a successful retry; and
6. an injected root-query failure retains ownership for verified cleanup.

The external-adapter filter includes real child-process fixtures for both interrupt and stop, in
addition to the Claude stream-JSON permission suite.

## Exact-commit real-Claude release gate

The original failed probe above remains preserved as the regression. The corrected implementation
was committed and locally verified at
`50379ba61eafbc798655000ee95f096ccd2fc4df`. The runner binary built from that exact commit had
SHA-256 `1822f8de30828ed356db6fc2b9452b9cf3d63ec4d08146936418ef811ab56aa1`.

ECorp mission `b94edf34-3fb3-41d1-bc52-8056c07d203a`, run
`21288e49-f5da-4fc5-9b46-441fee49d4b5`, used installed Claude Code `2.1.223`.
Bob independently approved the one bounded Bash request. The observed process lineage was:

```text
crony-runner 18780
└─ claude 40328
   └─ bash 29564
      └─ bash 43600
         └─ bash 14940
            └─ powershell 21704
               └─ python parent 36300
                  └─ python child 8636
```

The Python child was configured to sleep for 300 seconds. ECorp persisted
`run.interrupt_requested` at `2026-09-03T23:28:35.491103Z`, then persisted
`run.session_terminated` at `2026-09-03T23:28:36.015419Z`, 524.316 milliseconds later. The terminal
payload reported `provider_process_alive=false`, followed by `run.cancelled` at
`2026-09-03T23:28:36.025392Z`.

After the terminal event, the original Claude process, all three Bash wrappers, PowerShell, the
Python parent, and the Python child were absent. The runner remained connected, no manual process
cleanup was required, and the configured source checkout was clean before and after the probe. The
isolated run worktree was correctly preserved because it contained the PID evidence file.

The exact implementation commit also passed the complete local repository gate, five consecutive
22-test external-adapter suites, two `teardown_fail_closed` regressions, and the six-test
process-tree filter. Hosted GitHub Actions were unavailable because the account jobs were rejected
before step one for billing/spending-limit reasons; they are not acceptance evidence.

This boundary owns provider descendants. It does not replace the still-open network sandbox,
isolated provider-home, or inherited-environment allowlist work in #51.
