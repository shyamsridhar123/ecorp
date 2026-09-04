# Factory verification recovery evidence

Date: September 4, 2026
Repository: `shyamsridhar123/ecorp`
Source base: `main` at `4da7089cc833d7da482d7ed7f9691f9239559a0f`
Implementation branch: `codex/issue-113-verification-recovery-v2`

## Outcome

ECorp can recover a verification-failed GitHub Project item without creating another source issue,
mission, task, branch, or workspace lineage.

- Automated verifier failure and manual review rejection update the linked factory item inside the
  same transaction as run, task, and mission failure.
- Keyed verification decisions replay without another event or state change.
- `verifier_only` creates one provider-free run against the exact preserved workspace fingerprint.
  Every automated check receives a fresh bounded physical snapshot, so one command cannot alter the
  source or manufacture evidence for a later check. Escaping links fail closed, and snapshot cleanup
  must finish before verification can be accepted.
- A pre-0032 preserved run with no fingerprint is checkpointed by its owning runner after exact
  managed-worktree and head verification; no provider starts during checkpointing.
- Legacy provider artifacts without relative-path metadata are resolved inside the preserved
  worktree by exact file name, byte count, and SHA-256.
- `source_correction` creates one versioned contract revision and resumes the exact provider
  session, worktree, branch, and workspace lineage.
- The generic factory transition endpoint cannot move `verification_failed -> running`.
- Recovery requires owner, admin, or manager authority, an explicit issue, mode, and reason.
- Reviewed GitHub issue revisions, prior/replacement verifier policies, source and replacement runs,
  and recovery actor/reason are persisted.
- Attempts and budgets remain monotonic. A third run was rejected after the task reached `2/2`.
- Exact work-item recovery context keeps active replay available even after 101 newer recovery rows
  displace it from the bounded Corp snapshot.
- Interrupted recovery terminalizes the recovery, returns the task and factory item to
  `verification_failed`, releases the active slot, and permits a separately authorized retry.
- Recovery-aware publication provenance records the claimed revision, effective reviewed revision,
  and recovery ID through the selected run's resume lineage. New records use schema version 2 while
  legacy schema-version-1 publication attempts remain resumable.
- Project status remained `In Progress`; no real GitHub publication, merge, auto-merge, or
  deployment was requested.

## Live pre-migration compatibility probe

A read-only probe of the operator database and preserved Credit Exception #117 worktree confirmed
the legacy case this implementation must recover:

- migration 0032 is not applied there yet, so the run has no fingerprint column;
- the durable provider artifact has no stored local path and empty legacy metadata;
- its persisted file name is `claude-code-evidence.json`, size is `224` bytes, and SHA-256 is
  `816f238c8dbc3e16fb8dd7ca59da3cb87b284825438a19bd3d704a0b51352dc0`;
- the file still exists inside the preserved worktree with exactly that size and digest;
- the preserved worktree head and signed deliverable head both equal
  `8d35d3034f973c0872ccbb1b9eea11870ec084d1`.

The operator database, server, runner, and worktree were not modified by this probe.

## Executable recovery scenario

Command:

```powershell
$env:CRONY_SERVER_HTTP='http://127.0.0.1:18791'
$env:DATABASE_URL='postgres://crony:crony@127.0.0.1:54329/crony_issue113_recovery_v3'
$env:CRONY_CLI_BINARY='C:\Users\shyamsridhar\.codex\targets\ecorp-issue113\debug\crony-cli.exe'
$env:CRONY_TEST_SERVER_PID_FILE='output\issue113-live-v3\pids.json'
$env:CRONY_TEST_SERVER_BINARY='C:\Users\shyamsridhar\.codex\targets\ecorp-issue113\debug\crony-server.exe'
$env:ECORP_TEST_PYTHON_PSQL='1' # local fallback because psql.exe is not on PATH
node tools/e2e_factory_verification_recovery.mjs
```

Result: passed. The durable report is
`output/e2e-factory-verification-recovery.json`.

The existing controller regression also passed after the final workspace-finalization fix:

```powershell
$env:CRONY_SERVER_HTTP='http://127.0.0.1:18791'
$env:CRONY_CLI_BINARY='C:\Users\shyamsridhar\.codex\targets\ecorp-issue113\debug\crony-cli.exe'
node tools/e2e_factory_controller.mjs
```

Its final report at `2026-09-04T11:20:51.782Z` records one completed mission, one completed run,
a preserved workspace, and one `verified` factory item. The controller now waits for terminal
workspace finalization before moving a completed item to `verified`, including launch-conflict
replay.

### Verifier-only recovery

- Factory work item: `06ee7e28-4b1d-4172-923a-2dc3e2e48057`
- Mission: `6221cea3-ba33-42a3-84aa-0a3864766df6`
- Source provider run: `61cf0880-1cd7-450d-bcdb-36bfa84cb6e1`
- Recovery run: `9295408e-a1b3-4401-9a1c-73a56e5b0651`
- Keyed manual-decision replay: passed
- Duplicate controller replay returned the same recovery and run: passed
- Legacy source fingerprint removed before recovery: passed
- Runner-owned legacy workspace checkpoint with no provider execution: passed
- Legacy provider-artifact relative metadata removed and safely recovered: passed
- Active recovery displaced from the 100-row Corp snapshot by 101 newer rows: passed
- Exact work-item recovery context returned and replayed the same recovery/run: passed
- A command corrupted `result.md` only in its own snapshot; the later file check used a fresh
  snapshot and the preserved source remained valid: passed
- Bounded explicit snapshot cleanup before accepted verification: passed
- Same workspace lineage: passed
- Same committed head: passed
- Provider session/output/artifact events on the recovery run: `0`
- Final factory state: `verified`
- Total runs: `2`

### Source-correction recovery

- Factory work item: `db6e567c-be98-422e-9749-63a8d7830a68`
- Mission: `8d91d62a-ba57-4439-b7da-1b1ea05e85a7`
- Source Codex fixture run: `5dbbb643-6ef1-4f6f-b9da-e0cc3d57d605`
- Failed pre-dispatch recovery run: `4701c4fd-6c2b-409e-903a-01bc09b7e576`
- Failed pre-dispatch recovery: `b3de8455-dc0c-45bc-af1c-b598a513ea40`
- Successful recovery run: `a83bc563-736c-467b-823d-7724b8f062e3`
- Contract revision: `cf47d6d4-669c-4ca9-a30a-3a8cb04b2d13`
- Changed issue revision without explicit recovery: rejected
- Weakened policy that removed the independent-review gate: rejected before revision
- Test-owned server restart before recovery: passed
- Revoked scoped secret before runner dispatch: terminalized without an active-recovery wedge
- New authorization after secret-policy repair: passed
- Same provider session: passed
- Same worktree and workspace lineage: passed
- Contract version: `3`
- Attempt count: `3`
- Final factory state: `verified`
- Total runs: `3`

### Interrupted recovery

- Factory work item: `d4380a20-1686-41f4-8f51-f01ad1416bd7`
- Mission: `cb17ba78-7f6d-47ea-80ea-8aefab9df287`
- First cancelled recovery/run:
  `845d541c-e157-4d04-8e1d-ead1e92ebe0b` /
  `7130fe2d-493e-4ba0-b816-9f1b8f6de9b3`
- Recovery status after interrupt: `failed`
- Run status after interrupt: `cancelled`
- Task/factory state after interrupt: `verification_failed`
- Active recovery slot released: passed
- Exact context selected the cancelled preserved run as the next source: passed
- Fresh second recovery authorized:
  `177aae87-4ced-419c-b9a3-c5ab33e6d6ea`
- Second test cancellation cleaned up:
  `252176f8-5bca-4561-afb1-e5c6f9faec57`

### Attempt exhaustion

- Factory work item: `9b04fb6e-5128-4154-916f-2ddd04e5a5a0`
- Mission: `15642743-b2ee-4d6b-bbc2-f54d7b95e9e9`
- Attempt count / maximum: `2/2`
- Third recovery run creation: rejected
- Final run count: `2`

## Browser evidence

A real system Chrome run exercised the recovery card at `1440x900` and `390x844`.

- recovery callout visible;
- failure reason, attempts remaining, preserved workspace, and checkpoint fingerprint visible;
- verifier-only and source-correction controller actions visible;
- no horizontal page overflow;
- no console or page errors.

A second real Chrome pass loaded the replay-heavy two-run state at `1440x900` and `390x844`.
It proved reused provider artifacts produce one structured-link option per durable artifact ID, with
no duplicate React keys, horizontal overflow, console errors, or page errors.

A third desktop/mobile Chrome pass copied the source-correction command from the recovery callout
and verified that it contains the persisted Project owner/number, repository, source ref, adapter,
mission token and cost limits, issue number, recovery mode, and explicit reason. The generated
command now satisfies every mandatory `crony factory` argument.

The final branch browser sweep used the isolated server on `18791` and Vite on `15193`. Control
floor, Factory, Missions, Comms, and Audit had no horizontal overflow at the system desktop viewport
or `390x844`, and the browser recorded no console warnings or errors.

## Independent review remediation

A local read-only Codex review and the live PR review found additional actionable defects, all
corrected before the final gate:

- secret resolution after durable recovery creation could leave an active recovery wedged;
- pre-start runner rejection could retain the command ID and later acknowledge an unapplied command;
- browser recovery actions omitted mandatory CLI arguments;
- verifier commands could mutate the preserved worktree or create evidence for a later check;
- escaping symbolic links or reparse points could bypass snapshot isolation;
- snapshot cleanup could occur after accepted completion or fail silently;
- cancelled recoveries and active recoveries displaced from the bounded snapshot could wedge retry;
- recovery publication provenance was not resume-lineage aware; and
- strict new provenance fields could strand schema-version-1 in-flight publication attempts.

The recovery E2E, runner/store unit regressions, complete publication E2E, and desktop/mobile browser
sweeps are the regression evidence for those fixes.

Screenshots and the browser result were written outside the repository to:

```text
C:\Users\SHYAMS~1\AppData\Local\Temp\ecorp-issue113-ui\
```

## Repository gate

Final authoritative results:

```text
node tools/check_migrations.mjs
  32 migrations; manifest checksums immutable

cargo fmt --check
  passed

cargo clippy --workspace --all-targets -- -D warnings
  passed

cargo test --workspace
  142 passed; 0 failed

pnpm build:web
  passed

pnpm lint:web
  passed

git diff --check
  passed
```

The final complete `cargo test --workspace` run passed all 142 tests. The complete deterministic
publication E2E also passed at `2026-09-04T11:30:22.823Z`, including restart recovery and exact
publication-context lookup. Its GitHub boundary was a local fixture; it created no real pull request.

GitHub Actions were not used as acceptance evidence because the account's monthly hosted-runner
credits are exhausted.

## Isolation and cleanup

- The configured checkout at `C:\Users\shyamsridhar\code\crony-corp` was not modified.
- Implementation occurred only in the isolated worktree
  `C:\Users\shyamsridhar\.codex\worktrees\ecorp-issue113-recovery-v2`.
- Test-only ports `18791` and `15193` were closed.
- The test database `crony_issue113_recovery_v3` was removed.
- `%TEMP%\ecorp-verification-snapshots` contained zero residual snapshot directories after the final
  recovery E2E.
- Operator services on `8791` and `5187` remained running.
- The deterministic publication fixture created no real pull request. No merge, auto-merge, or
  deployment was performed by this evidence run.
