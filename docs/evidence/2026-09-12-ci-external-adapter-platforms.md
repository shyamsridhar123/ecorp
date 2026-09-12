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

## Task-graph fixture follow-up

Hosted run `34679761001` on `942720191574dc84c32ed582c5dc494f582a167d`
passed quality, all three runner-platform jobs, Windows desktop, and the Windows
external-adapter job. Its Unix external-adapter refusal step also passed.
Integration then failed in the unchanged bounded task-graph fixture: the demo
roster selected Codex and Claude on Linux, where Claude is intentionally unavailable.
Changing only the preferred adapter cannot fix that roster: the planner swaps
the preferred worker into slot zero and retains Claude in slot one.

The approved follow-up changes only CI fixtures and this evidence record:

- Linux prepares the two Windows-only demo agents as offline **before creating
  any mission**. The helper requires the explicit Actions `integration` job,
  Linux runner, fixed synthetic demo Corp, no mission/task/run history, and an
  exact running `postgres:17-alpine` service-container identity. A bounded
  serializable transaction rechecks history and idle/offline, non-retired,
  run-free agents, scoped to the exact Corp and agent ID/adapter pairs.
  Repeated preparation is idempotent; dry-run performs no database operation.
- This is initial test-data preparation, not a production planner capability
  fix or permission to rewrite a real factory roster. No production planner,
  adapter, security guard, or migration is changed.
- Linux retains the original three-node graph, two distinct adapters (Codex and
  fake-process), two concurrent roots, separate worktrees, dependency ordering,
  verified specialist artifacts, and bounded retry assertions. Planned adapters
  must also be advertised available before launch.
- Windows additionally executes the original Codex/Claude graph on the unaltered
  demo roster, using the existing deterministic Codex app-server fixture. Its
  supervisor explicitly asserts the two root adapter names. The provider paths
  remain synthetic, with real GitHub/provider access disabled.
- Both graph entry points now require an explicit owned endpoint and test opt-in.
  Manual-stack ports are rejected outside Actions. The fixture supports a pure
  `--dry-run`; the report records the roster preparation and actual root adapters.

Validation on parent `9427201` plus this follow-up:

- `node --test tools/e2e_external_adapters.test.mjs tools/task_graph_fixture.test.mjs`:
  29 passed, zero skipped. Cases include ownership/container/platform guards,
  history and busy-agent refusal, exact SQL scope, no-effect previews, subprocess
  failures, unexpected SQL results, and both CI graph lanes.
- Node syntax checks, Windows supervisor AST parse, and `git diff --check` passed.
- Migration check: 40 immutable migrations. Rust format and offline Clippy passed.
- `cargo test --offline --workspace --quiet`: 452 passed, 200 opt-in database
  tests ignored; ambient `DATABASE_URL` was removed for this command.
- `pnpm build:web` and `pnpm lint:web` passed.
- The isolated Windows runtime dry-run and execution passed in retained fixture
  `ecorp-external-adapters-ci-20260912g`, using API `18437` and PostgreSQL `55437`.
  It ran from `2026-09-12T07:51:42.9221189Z` to
  `2026-09-12T07:52:20.2858857Z`; synthetic source commit:
  `9c14d8d5f3b78be8539930a12d582492f4713e90`.
- Graph mission `cee1f61a-e478-4738-ba3c-6b14202f2e9b` completed all three tasks
  with root adapters `claude-code` and `codex`, two observed concurrent runs,
  synthesis after both roots, and verified specialist output consumed.
- Retry mission `2f07f1ae-ff58-4bf9-8295-5b32b6e820f8` reached its expected failed
  state after exactly two allowed attempts. Both external-adapter lifecycles
  passed in the same fixture before the graph test.
- Source remained unchanged and all three owned QA processes were stopped.
  No real provider call or GitHub mutation was performed by the fixture.
- Retained `e2e-task-graph.json` SHA-256:
  `0e1c1c1b4053b1a83f8fac6e65d94a84413607216fddca1ad1c6e907d4430b88`.
- Retained `fixture-report.json` SHA-256:
  `246659026beadeeb9e98b97419ca1ae87473c7a6572def6d923501f86320694d`.

## Controlled artifact-runner readiness follow-up

Hosted run `34682044685` on `a7855a22f44369a28775c3927b0325676207f5f4`
passed both graph lanes, including the real Linux roster SQL, and all six other
jobs. Integration passed verification, replay, control fencing, rooms, runner
reconnection and idempotency, then failed in `e2e_artifact_staging.mjs`:
`timed out waiting for assignment 3e7b6884-d709-42fc-9ee0-44982ecbe740`.

The existing controlled runner resolved its connection promise on `Registered`
and immediately created/launched a mission. Native server code intentionally
keeps that connection non-dispatchable until current-epoch reconciliation and its
one-second finalization delay finish. Another ready runner can therefore receive
the unbound mission; the fixture did not check the returned runner ID.

The CI-only correction waits for the existing source-bound, read-only mission
preview to admit the controlled runner, using a unique per-connection synthetic
source marker. That marker is explicitly **preview-only**, never persisted as an
execution source or presented as evidence of an actual Git checkout. The helper
checks the exact connected runner, Corp, marker and available fake-process
capability, with zero mission/task/run history. It retries only narrowly
classified readiness refusals, with a 30-second bound. Every effectful launch now
asserts its returned runner ID before waiting for the assignment. No production
registration, dispatch, reconciliation, authorization or completion guard changes.

Linux retains the complete original SQL-fault, staged-byte, duplicate-digest,
breaker-refusal, restart and periodic-recovery assertions. Full execution now
requires the explicit existing Actions integration workspace, endpoint, PID file
and database configuration. Windows adds a separately labeled readiness smoke
that exits after one accepted, verified artifact; it cannot claim SQL fault or
restart coverage. Manual-stack ports are refused by that smoke mode.

Local validation on parent `a7855a2` plus this follow-up:

- All three Node regression files: 37 passed, zero skipped. Node syntax,
  PowerShell AST, migration check, Rust format, offline Clippy, web build and lint
  passed. Offline Rust tests again passed 452 cases with 200 explicitly ignored
  database cases and no ambient `DATABASE_URL`.
- Pure readiness and supervisor previews made no changes.
- Retained fixture `ecorp-external-adapters-ci-20260912h` passed from
  `2026-09-12T08:10:04.9054492Z` to `2026-09-12T08:10:46.3739470Z`, using only
  QA API `18437` and PostgreSQL `55437`. Its synthetic source commit remained
  `c29732e2abd4b5bb2bd72b566fdb74cb5060457e`.
- The smoke made eight native readiness previews. Controlled runner
  `aaa-artifact-staging-8f970f79-8918-4fdd-873c-197d7410b229` received and completed
  run `922ac2c7-56fa-4eab-ae68-e16d412f8b87`; artifact SHA-256 was
  `a47e8ab1dea707ba3a06aab4db7c40a45a0a5c0afea4d69dda554b88e0ec2172`.
- Both external-adapter lifecycles and the Codex/Claude graph passed again.
  The report confirms unchanged synthetic source, zero real provider/GitHub
  effects, and verified shutdown of the owned runner, server and PostgreSQL.
- `e2e-controlled-runner-readiness.json` SHA-256:
  `ac4b737c73d7bef73219e7a854268be68114994a1efa192d64208e3d24257e8c`.
- `fixture-report.json` SHA-256:
  `aaa9355649d4eeb3adcc4461963a778a21f6ecbae8104c1bcf4f94d06b908f11`.

## ATV immutable-source fixture alignment

Hosted run `34682748305` on `4b939c57c6216d5ccfb27b58a3722a0a8f1ee00a`
passed the complete Linux artifact-staging/restart suite and all six other jobs,
including the Windows controlled-runner readiness smoke. The next integration
step, factory claims, correctly refused to dispatch a mission bound to the stale
`shyamsridhar123/ecorp` identity: the checkout belongs to `All-The-Vibes/ecorp`.
The failing task required old-repository commit
`47d5d6472f054b27c36f870bf35f19f6a4eefeff` (the CI merge checkout), which no runner
advertised under that old repository name. HTTP 409 allocated zero runs.

This repository-scoped follow-up retargets only the positive source identities in
`e2e_factory_claims.mjs`, `e2e_factory_controller.mjs` and
`e2e_factory_publication.mjs`. Their policies, issue/PR fixture URLs, fake GitHub
state, normalized effect keys and identity assertions now use ATV consistently.
Lowercase and mixed/uppercase forms are retained for case-normalization coverage.
Deliberately unrelated `acme/widget` repositories, synthetic Project identities,
fake GitHub CLI transport, and local bare publication remotes remain unchanged.
No live Project, repository remote, database configuration or runner is migrated.

The claim fixture also reuses the tested native read-only preview barrier before
claim creation and immediately after its existing CI restart. A pure selector
first verifies exactly one connected runner in the expected Corp with the exact
repository, `HEAD` ref and locally resolved immutable commit. It never adopts an
arbitrary advertised checkout to make a test pass. The preview precedes all
mission/task/run history; the source-fencing and authorization assertions remain.

Validation on parent `4b939c5` plus this follow-up:

- Four Node regression files: 45 passed, zero skipped, including wrong-repository,
  wrong-ref/commit, foreign/ambiguous runner, case normalization, fake transport,
  immutable-source assertions and preservation of all three full factory lanes.
- All three changed E2E files passed Node syntax checking; `git diff --check`
  passed. No Rust, UI or migration source file changed.
- Migration check: 40 immutable migrations. Rust format, offline Clippy and
  offline Rust tests passed (452 passed, 200 explicitly ignored database tests,
  ambient `DATABASE_URL` removed). Web build and lint passed.
- The four pending terminal-recovery files remained byte-identical and unstaged.
  No local service, factory scenario, SQL fault or publication fixture was run for
  this metadata-only alignment. The previous isolated fixture remains stopped.

The updated factory-claim, controller and publication runtime results remain a
hosted-CI gate at this pre-publication checkpoint; static regressions are not a
claim that those full runtime suites have already passed.

## Remaining boundary

This is Windows deterministic full-stack evidence and unit-tested Unix admission
logic. It is not real vendor inference or browser acceptance. Hosted runs have
now proved both external-adapter lanes, Linux roster SQL, both graph variants,
and the full Linux artifact-staging/restart suite. The next run must validate the
ATV-aligned factory fixtures and remaining integration steps. The local readiness
smoke does not claim SQL fault injection or restart recovery on Windows.

The live factory, original runner identity, paused work, budgets and pending #50
terminal-recovery cases remain outside this correction. PR #237 remains review
work; no merge, auto-merge, deployment or issue closure is authorized by these tests.
