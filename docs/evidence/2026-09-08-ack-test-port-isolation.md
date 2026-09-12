# ACK transport tests without shared-port collisions

- **Issue:** #181.
- **Source base:** `41fc0d6c6f847ebe0dcaa1cef864c56aa90e211e`.
- **Observed:** September 8, 2026.
- **Scope:** synthetic loopback transport tests, not real-provider or #172 acceptance.

## Reproduced failure and correction

The original complete helper command produced **23 passed / 1 failed** because its
synthetic upstream tried to bind the existing QA server's `127.0.0.1:18961`.
No existing listener was stopped, reused or connected to as a synthetic peer.

Both synthetic servers now bind port zero and retain their live server objects
through teardown. The test consumes their actual bound addresses rather than
probing a free port, releasing it and racing to reopen it.

The relay has one import-only test seam: an already-listening Node HTTP `Server`
object. Its exact IPv4 loopback address must match the configuration. JSON/CLI
configuration cannot enable this seam. Normal CLI endpoints and all original
opt-in, source, issue, Corp, runner, manual-port and redaction checks remain intact.

## Verification

```text
node --test tools/e2e_evidence_selection.test.mjs tools/runner_ack_fault_relay.test.mjs
34 tests; 34 passed; 0 failed; 0 cancelled; 0 skipped; 0 todo
```

Two complete transport fixtures ran concurrently, using four distinct live
endpoints and separate output directories. They retained the original forwarding,
ordering, ACK withholding, reconnection and redaction assertions, and checked
cross-fixture metadata isolation. Teardown verified both server pairs were closed.
Existing missing/delayed/late-ACK gate tests remain unchanged.

The protected listeners remained:

| Port | Existing PID |
| --- | ---: |
| 18961 | 32148 |
| 18963 | 34092 |
| 18962 | 48228 |
| 15491 | 51216 |

The parent independently reran the complete 34-test command and compared those
same four port/PID pairs before and after. Syntax and whitespace checks passed.
The worker also recorded zero temporary-directory delta after cleanup.

## Boundaries

No existing service, provider, database, container or browser was restarted or
modified by this test. No dependency was installed. The prior #172 QA-bridge
restart denial was not retried or bypassed. Its existing relay still reports
HTTP 503 / failed, and the original QA runner remains offline; these synthetic
tests do not turn that separate draft acceptance into a pass.

The pre-fix failure remains in the worker transcript. Parent logs and protected
listener receipts are retained under
`C:\Users\shyamsridhar\.codex\dogfood\issue181-port-isolation-20260908`.
Full repository gates are recorded separately.
