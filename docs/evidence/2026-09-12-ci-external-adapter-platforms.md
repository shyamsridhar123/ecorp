# CI external-adapter platform contract

Observed: 2026-09-12 UTC. Scope: CI and deterministic test fixtures only.
Related review: https://github.com/All-The-Vibes/ecorp/pull/237

## Failure and source

Hosted CI run `34676356598`, integration job `103506808050`, failed on published
head `f3bb566df718f21257de038edb92d119d10f015a`. The Claude/OpenCode sample received
HTTP 409: `requires adapter claude-code, but that adapter is unavailable`.
The other five jobs passed; the subsequent integration steps were skipped.

The integration job uses Ubuntu. `ExternalCliAdapter` deliberately enables these
providers only on Windows: Unix process groups cannot establish the required
non-escapable descendant-termination boundary. The workflow, adapter restriction
and original sample were unchanged from base
`b28fd4d26309794f38c0455bbf42d22aedf7cfd1`.

## Approved correction

- Ubuntu runs the common sample as an explicit unsupported-admission contract.
  Both capabilities must advertise unavailable execution and disabled feature flags. Both launch
  requests must return the exact unavailable-adapter conflict, allocate zero
  runs, retain held missions and leave task state unchanged. Unrelated HTTP
  failures or fallback execution fail the test.
- A separate Windows job runs both successful synthetic provider lifecycles
  through the real server, runner, managed worktrees and signed artifact download.
  Windows unavailability remains a failure.
- The test checks the connected runner's OS, not the HTTP client's OS.
  The capability DTO carries the native summary flags (such as `spawn=no`), not
  the internal unsupported-reason string; the test verifies the actual wire contract.
- Positive execution waits through the existing source-bound, read-only mission
  preview while native reconnect reconciliation is incomplete. It does not retry
  launches. Preview failures are narrowly classified and bounded to 60 seconds.
- The Windows supervisor uses the existing `local_stack.psm1` ownership and
  environment helpers, native PostgreSQL binaries and the existing protocol
  fixtures. The missing harness behavior was a disposable Windows full-stack CI
  fixture; no alternative provider executor, process owner or production approval
  mechanism was introduced.
- The hosted Windows image supplies PostgreSQL through `PGBIN`. The test initializes
  its own loopback-only cluster instead of starting the image's installed database
  service. Only `evidence/` is uploaded, excluding credentials, database and worktrees.
  Native `pg_ctl` starts its restricted Windows process; the fixture validates the
  fresh private PID/data receipt, executable, creation time, listener and SQL identity.
  PostgreSQL's own administrative-user restriction remains intact.

No production Rust, UI, migration, lockfile, runtime platform guard, live factory
configuration, merge policy or account setting is changed.

## Local validation

Source worktree: `codex/issue-50-factory-recovery`, parent `f3bb566` plus this CI-only
patch. The four pending terminal-recovery files were byte-checked and left unchanged.
The configured source checkout remained clean at
`971445e1cbf9388c51803e2adf28b11bd98b1ffa`.

Passed:

- `node --test tools/e2e_external_adapters.test.mjs`: 14 tests, no skips.
- PowerShell AST parse of `tools/ci_external_adapters_windows.ps1`.
- Both harness dry-runs: no services started or database writes.
- `cargo build --offline -p crony-server -p crony-runner`.
- `node tools/check_migrations.mjs`: 40 immutable migrations.
- `cargo fmt --check`.
- `cargo clippy --offline --workspace --all-targets -- -D warnings`.
- `cargo test --offline --workspace --quiet`: 452 passed, 200 explicitly ignored.
  No ambient `DATABASE_URL` was provided. The ignored cases require an explicitly
  owned SQLx maintenance database and were not claimed as executed.
- `pnpm build:web` and `pnpm lint:web`.

The corrected Windows runtime invocation used:

```powershell
pwsh -NoProfile -File tools/ci_external_adapters_windows.ps1 -FixtureRoot '<absolute QA parent>\ecorp-external-adapters-ci-20260912d' -PgBin '<existing PostgreSQL 17 bin>' -DryRun
pwsh -NoProfile -File tools/ci_external_adapters_windows.ps1 -FixtureRoot '<absolute QA parent>\ecorp-external-adapters-ci-20260912d' -PgBin '<existing PostgreSQL 17 bin>' -Execute
```

The run completed from `2026-09-12T06:35:40.1079512Z` to
`2026-09-12T06:35:59.8728061Z`, using API port `18437`, database port `55437`,
runner `runner-external-ci`, and synthetic source commit
`9db3e7dcae2654afaa2d671fba0f644a6c0cca43`.

- Claude Code run: `0b949297-9eea-458d-a567-df71dccdb16a`.
  Artifact SHA-256: `343351179ba5493a82563951d73fe7b741b3bfdce6eabf518a57189d4a60eb94`.
  Termination event 15 preceded completion event 19.
- OpenCode run: `149c9cba-89c3-4f9b-aa3e-01e203ea7966`.
  Artifact SHA-256: `785d91223c6269b4d039d54b2520f437852c9664daf4b8527cee7f79120b2fe6`.
  Termination event 32 preceded completion event 36.

Each provider reported 100 input and 40 output **synthetic** tokens. Both retained
a provider session ID, verified artifact bytes and `provider_process_alive=false`.
The fixture source HEAD and working tree remained unchanged. The supervisor
verified shutdown of its runner, server and PostgreSQL process; a separate check
found neither QA port listening afterward.

Retained evidence under that fixture's `evidence/` directory:

- `e2e-external-adapters.json` SHA-256:
  `07722bf9b3974234250227f2947a130916ea193fc17068062e5cf2405a8bc1af`.
- `fixture-report.json` SHA-256:
  `fe25acb7cde4464f17a64eeb8e6725d5f13123cc98f828bca33e794a1fa2e4b5`.

Earlier failed fixture preparations were retained: attempt `a` exposed Windows
Git's rejection of `NUL` as a config file; `b` exposed multiple Node executable
paths being combined; `c` exposed the native reconciliation/readiness race.
The final harness uses an actual empty private Git config, selects one Node
executable and uses native read-only readiness previews. All started QA processes
were stopped; no historical worktree or database was deleted.

## Hosted follow-up and native-launcher verification

The first hosted follow-up (`8d4356b`, run `34678881288`) passed quality, all three
runner-platform jobs and Windows desktop. It exposed two fixture defects:

- The Unix assertion expected the internal unsupported-reason text, which the
  capability DTO does not transmit. The corrected test requires the actual
  unavailable flag and every native execution feature to be disabled.
- The hosted Windows account is elevated, so direct `postgres.exe` startup was
  correctly rejected. PostgreSQL 17's native `pg_ctl` uses `CreateRestrictedProcess`
  on Windows; the test now uses that launcher with a fresh owned data directory.

Reference checked: https://github.com/postgres/postgres/blob/REL_17_STABLE/src/bin/pg_ctl/pg_ctl.c

The native launcher also needs file-backed stdout/stderr: its background process
can retain a captured pipe even when PostgreSQL's own log file is configured.
Local attempt `e` exposed that wait. Its exact PostgreSQL PID, executable, creation
time, listener and SQL data-directory identity were verified before a graceful
stop; `verified-pipe-cleanup.json` preserves that independent cleanup receipt.
No live-factory process or data was used.

The final local repetition, `ecorp-external-adapters-ci-20260912f`, ran the updated
code on parent `8d4356b` with the CI-only follow-up. It passed from
`2026-09-12T06:59:39.1420696Z` to `2026-09-12T06:59:59.9095769Z`.
Its synthetic source commit was `8c71cb43c843dcce0a41abe9a1255b87270ad808`.

- Claude Code run `e45303a4-fd15-45f2-b35a-16768a693fe3`, artifact SHA-256
  `b9869291d4623110cca3fa5274eecf7f5f819987b82e6e841dc511c5e6ed6b02`.
- OpenCode run `edda6691-12cb-4d1e-82d7-69f8eda443f6`, artifact SHA-256
  `c3d75798ac6b053395f91dcfa0a0d74a30ff3aa9dd3a7d9b3b00f1033fb36421`.
- `e2e-external-adapters.json` SHA-256:
  `a8d48c08a6bc31a08d3a7976d035213be943a615bc57d4adf031394cd1a53dc6`.
- `fixture-report.json` SHA-256:
  `ba1f6ead4c4972e6625d9e4efda7943c3750f091ebb60f004e5e1313208494eb`.

Both providers completed with verified artifacts and prior termination events.
The report confirms unchanged source and verified shutdown of all three owned
processes; neither test port remained listening. All 14 contract regressions
passed again. This does not itself prove the elevated hosted-image case; the
replacement hosted run remains the next gate.

## Remaining boundary

This is Windows deterministic full-stack evidence and unit-tested Unix admission
logic. It is not real vendor inference, browser acceptance, or proof that the
updated hosted Linux integration suite passed. A new hosted run must execute the
corrected Unix step and all previously skipped integration steps. The new Windows
job must also pass on the hosted image.

The live factory, original runner identity, paused work, budgets and pending #50
terminal-recovery cases remain outside this correction. PR #237 remains review
work; no merge, auto-merge, deployment or issue closure is authorized by these tests.
