# Source-correction recovery after runner loss

- **Issue:** #183; original review on #162.
- **Observed:** September 8, 2026, America/Chicago.
- **Source base:** `a2d2b532ffd506878f7f47fc2f7d6fb1f31340a3`.
- **Upstream integration:** `3202f38fddd372f7b77aae75945f48acdd4da124`, containing #182.
- **Product source SHA-256:** `544D7635B605F7254901F98EC8FB663CA32C35D10BC6BD37104D6EFA2E57A1A6`
  for `crates/crony-store/src/lib.rs`.
- **Test source SHA-256:** `C44592E86709274BA7A446585C4242BC1EA74B5A5134BF2EF3CE0F180E114DF0`
  for `crates/crony-store/src/factory_recovery_loss_tests.rs`.

## Corrected behavior

Native loss reconciliation previously recognized only verifier-only recovery. A lost
source-correction run could leave its recovery and Factory item reporting running.

The existing terminalization path now also handles an exactly bound source-correction
replacement. It records loss and verification failure atomically, releases the active recovery
slot, and retains original history, attempt counts, budgets, source and provider-session
identity. Existing recovery/publication advisory gates are taken before run/task/mission rows.

A later native cleanup report may make the lost source-correction checkpoint usable again.
It must supply a valid fingerprint through the existing assignment-fenced event path. Missing
assignment metadata, when `run.started` never arrived, is recovered only from the exact
persisted parent assignment. The callback's path is ignored. Existing metadata and fingerprints
are not overwritten, and the run never returns to running merely because cleanup arrived.
Ordinary provider loss, unconfirmed preservation, stopped/quarantined lineage and unrelated
recovery bindings do not gain recovery authority.

## Actual-store red and green

All cases use SQLx-created disposable databases and the **38 actual migrations**, via the
existing explicitly owned QA PostgreSQL maintenance environment. No manual application
database, provider process, or signed object was used to manufacture these results.

| Invocation | Result | Runtime |
| --- | --- | --- |
| Original ACK/loss regression, unchanged product behavior | 0 passed, 1 failed: recovery stayed `running` | 2.44 s |
| First scoped loss/cleanup matrix | 11 passed, 0 failed | 31.07 s |
| Additional next-native-recovery admission and replay | 1 passed, 0 failed | 4.13 s |
| Missing-start-report cleanup regression before its fix | 0 passed, 1 failed: no confirmed workspace checkpoint | 2.28 s |
| Final complete `issue183_` family, hashes above | **13 passed, 0 failed, 0 ignored** | **38.05 s** |

The final family covers command acknowledgment before the start report, grace expiration,
current connection epoch and accepted-claim fencing, retained pending commands, verifier-only
seal enforcement, ordinary provider-loss exclusion, rollback, protected Factory outcomes,
foreign/replaced recovery-context rejection, idempotent cleanup and loss, and both native
Factory gates preceding lifecycle row locks.

The last case calls **both** `create_mission_contract_revision` and
`create_factory_verification_recovery` after cleanup of a lost run whose start report was
missing. It verifies a new run and exactly one durable command retain that lost source run,
the newly confirmed fingerprint, original workspace root, provider-session ID and source
tuple. Native idempotent replay creates no second run/command. Prior source/run rows remain
unchanged; attempts increase to three, while original budgets and spend are retained.
This proves native store admission and command persistence, not that a provider executed
the new command or that a vendor session was independently restored.

## Final source gates

The complete source-gate invocation at the hashes above passed with the source unchanged:

| Gate | Result |
| --- | --- |
| Immutable migration check | 38 migrations |
| Workspace formatting | Passed |
| Workspace/all-target Clippy, warnings denied | Passed |
| `RUST_TEST_THREADS=1 cargo test --workspace` | 335 passed; 23 opt-in database tests ignored |
| Frontend tests on this foundation layer | 64 passed |
| Web build and lint | Passed |
| Startup/lifecycle operation regressions | 34 passed; 14 owned synthetic processes reaped |
| Server, runner and CLI binaries | Built |
| Whitespace/scope check | Passed |

The thirteen #183 SQLx tests are included in the ordinary run's ignored count, then exercised
explicitly by the separate invocation above. The foundation gate does not include the later
stack layers or claim their browser changes were tested by these 64 frontend cases.
The structured receipt is in `validation-20260908T063851491/result.json` beneath the operator
evidence directory below.

## Retained limitations

- The first twelve-case result and the first full source-validation pass are earlier evidence,
  not a substitute for the subsequently fixed missing-start-report case.
- An initial test compile referenced an unavailable direct Tokio dependency. The fixture was
  corrected to wait for its one-second native PostgreSQL grace period without adding a dependency.
- Scoped independent source reviews found no blocking delta defect. A nonblocking suggested
  extension is a dedicated missing-start-report negative matrix that changes receipt/source
  binding before cleanup and separately exercises quarantine. Existing context negatives and
  the guarded implementation are not represented as that additional matrix.
- No global deadlock-freedom, production identity, signed-object bytes, genuine vendor-session
  persistence, hosted Actions, merge, auto-merge or deployment result is inferred from SQLx.
- The earlier explicitly bounded #169 SQLx families were **not rerun**.

Full local logs and immutable source receipts are retained in the operator evidence directory
`C:\Users\shyamsridhar\.codex\dogfood\issue183-recovery-loss-20260908`.
Source gates and browser/runtime acceptance are recorded separately from this store proof.
