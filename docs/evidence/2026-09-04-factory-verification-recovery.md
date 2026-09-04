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
- Project status remained `In Progress`; publication, merge, auto-merge, and deployment were not
  requested.

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
$env:DATABASE_URL='postgres://crony:crony@127.0.0.1:54329/crony_issue113_recovery'
$env:CRONY_CLI_BINARY='C:\Users\shyamsridhar\.codex\targets\ecorp-issue113\debug\crony-cli.exe'
$env:CRONY_TEST_SERVER_PID_FILE='output\issue113-live\pids.json'
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

Its final report at `2026-09-04T08:19:03.407Z` records one completed mission, one completed run,
a preserved workspace, and one `verified` factory item. The controller now waits for terminal
workspace finalization before moving a completed item to `verified`, including launch-conflict
replay.

### Verifier-only recovery

- Factory work item: `c5d3193d-6362-4ba6-b900-4e543a3cb0f2`
- Mission: `e24c26d0-35bc-4237-88cc-86ce7d804ab8`
- Source provider run: `a738625a-3db9-4111-b2c1-11e274e12b80`
- Recovery run: `76639fad-e272-426c-8229-342027d02566`
- Keyed manual-decision replay: passed
- Duplicate controller replay returned the same recovery and run: passed
- Legacy source fingerprint removed before recovery: passed
- Runner-owned legacy workspace checkpoint with no provider execution: passed
- Legacy provider-artifact relative metadata removed and safely recovered: passed
- Same workspace lineage: passed
- Same committed head: passed
- Provider session/output/artifact events on the recovery run: `0`
- Final factory state: `verified`
- Total runs: `2`

### Source-correction recovery

- Factory work item: `b8fa4dd2-4642-4892-aef1-e9a7ff933d2c`
- Mission: `3d4ecfb1-9679-42e5-b3c2-15a8f7fccaac`
- Source Codex fixture run: `f14b37d8-fce2-46c7-89a3-974526330d46`
- Recovery run: `bfbcf9f0-6415-469c-b631-6dc50e45cb6d`
- Contract revision: `df13d575-7a48-467f-8cda-18cd86c34e34`
- Changed issue revision without explicit recovery: rejected
- Weakened policy that removed the independent-review gate: rejected before revision
- Test-owned server restart before recovery: passed
- Same provider session: passed
- Same worktree and workspace lineage: passed
- Contract version: `2`
- Attempt count: `2`
- Final factory state: `verified`
- Total runs: `2`

### Attempt exhaustion

- Factory work item: `15f35728-a067-438f-a3a6-0b644b305d43`
- Mission: `ac308260-8c09-4b75-9762-09fd0adafaec`
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
  138 passed; 0 failed

pnpm build:web
  passed

pnpm lint:web
  passed

git diff --check
  passed
```

Earlier parallel Windows runs exposed that two verifier tests used five-second command deadlines
while the complete runner suite was saturating process startup. The focused checks passed, the
test-only deadlines were raised to 30 seconds, and the subsequent complete
`cargo test --workspace` run passed all 138 tests.

GitHub Actions were not used as acceptance evidence because the account's monthly hosted-runner
credits are exhausted.

## Isolation and cleanup

- The configured checkout at `C:\Users\shyamsridhar\code\crony-corp` was not modified.
- Implementation occurred only in the isolated worktree
  `C:\Users\shyamsridhar\.codex\worktrees\ecorp-issue113-recovery-v2`.
- Test-only ports `18791` and `15193` were closed.
- Operator services on `8791` and `5187` remained running.
- No pull request, merge, auto-merge, or deployment was performed by this evidence run.
