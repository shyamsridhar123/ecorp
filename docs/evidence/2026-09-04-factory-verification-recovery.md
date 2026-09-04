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

Its final report at `2026-09-04T09:39:33.798Z` records one completed mission, one completed run,
a preserved workspace, and one `verified` factory item. The controller now waits for terminal
workspace finalization before moving a completed item to `verified`, including launch-conflict
replay.

### Verifier-only recovery

- Factory work item: `9a6d61b4-e836-4b16-84fe-1f6fe4934cf9`
- Mission: `69e0cef9-fd56-424a-af72-9ce50d78cc6e`
- Source provider run: `714e2581-f847-46da-93b1-4c30f290e3cf`
- Recovery run: `f1a86f08-a170-4bab-b0db-a0c748da7251`
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

- Factory work item: `898345e8-c2b4-40d9-9d1e-b27d0f1710d1`
- Mission: `62151b0d-4795-4cf4-9884-a5572cf41ff4`
- Source Codex fixture run: `439a011a-8533-4cc3-b663-8ddd8b375e7d`
- Failed pre-dispatch recovery run: `d5df4684-758f-461e-9cc4-68268428748c`
- Failed pre-dispatch recovery: `f7896eba-5388-4f05-8ed6-efa4f8b24deb`
- Successful recovery run: `67742d76-c54d-4813-a076-74d042c30956`
- Contract revision: `d9139dff-c80c-4d85-b347-6bd718b3d8a4`
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

### Attempt exhaustion

- Factory work item: `db3687f4-e4a3-468a-a892-db0b4e62be4c`
- Mission: `5db2400e-cbc1-4ded-b108-ff29041c29e2`
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

## Independent review remediation

A local read-only Codex review found three actionable defects, all corrected before the final gate:

- secret resolution after durable recovery creation could leave an active recovery wedged;
- pre-start runner rejection could retain the command ID and later acknowledge an unapplied command;
- browser recovery actions omitted mandatory CLI arguments.

The dispatch-failure E2E and desktop/mobile command-copy browser checks are the regression evidence
for those fixes.

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
