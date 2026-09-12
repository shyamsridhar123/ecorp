# Issue #50 recovery baseline - September 11, 2026

## Status and scope

Follow-up: [isolated Factory budget-recovery evidence](2026-09-11-issue-50-factory-budget-recovery.md)
records the subsequently approved synthetic QA run. The baseline below preserves
what had and had not been performed at preparation time.

This is preparation and baseline evidence, not a fix or an accepted reproduction of the reopened Factory recovery failure. No production source was changed, no retained application database was modified, and no existing mission was resumed.

- Issue: https://github.com/All-The-Vibes/ecorp/issues/50
- Assigned operator: Bakar404.
- Investigation branch: `codex/issue-50-factory-recovery`.
- Investigation base: `b28fd4d26309794f38c0455bbf42d22aedf7cfd1` (reviewed upstream main).
- Running operator checkout: unchanged at `971445e1cbf9388c51803e2adf28b11bd98b1ffa`.
- This is a separate contributor worktree, not a replacement Factory mission or a modification of #235's runner-owned worktree.

## Native Codex execution-tool smoke test

The complete installed Codex 0.153.4 bundle was used for a bounded ephemeral CLI probe in an empty, operator-owned diagnostic directory. The sandbox was read-only. The prompt requested exactly one PowerShell `(Get-Location).Path` command, with no retry after a tool failure, no repository/file inspection, and no writes.

Observed results:

- Process exit: 0; elapsed time: 21.6 seconds.
- Exactly one command-execution item, exit code 0, returning the expected diagnostic directory.
- No files created in that directory.
- No missing-code-mode-host failure.
- No ECorp mission, task, or run was created or resumed by this diagnostic.

Reported turn usage (not a billing statement):

| Counter | Reported value |
| --- | ---: |
| Input tokens | 176026 |
| Cached input tokens | 87937 |
| Cache-write input tokens | 88083 |
| Output tokens | 91 |
| Reasoning output tokens | 0 |

These are provider-reported categories; do not sum cache categories as additional input tokens. This diagnostic usage is separate from #235's persisted 209664-token total, which was not reset or changed. The probe emitted a skills-context truncation warning and malformed-frontmatter warnings for three local skill entries. Those warnings establish that substantial context was loaded, not an exact attribution of tokens to individual skills. No personal skills, plugins, model selection, or account configuration was changed.

This verifies native CLI tool execution against the repaired bundle. It is not a new browser/server/runner Factory acceptance run and is not proof of successful #235 recovery.

## Deterministic baseline tests

Executed from the isolated investigation worktree, with its own Cargo target directory and no DATABASE_URL supplied to Cargo:

```powershell
cargo test --locked -p crony-cli issue206_
node --test tools/fake_codex_budget_stream.test.mjs
```

Results:

- CLI checkpoint/cancellation tests: 7 passed, 0 failed (89 unrelated tests filtered out).
- Fake Codex budget-stream tests: 6 passed, 0 failed.
- Rust/Cargo 1.98.1; Node.js 24.19.0; pnpm 11.19.0 were available.
- No real model is used by these deterministic tests. The Node tests create and remove only their verified temporary protocol-fixture directories.

These passing tests cover existing #206 checkpoint protections and synthetic usage/interrupt behavior. They do not establish that the full approved-budget/provider-continuation/Factory-state sequence in #50 works.

## Reproduction boundary

PR #77 already delivered the original budget-revision ledger/API/UI. Upstream main also includes #206's server-validated checkpoint catch-up. The remaining investigation must distinguish checkpoint-only verification from authorized continuation of incomplete provider work.

The existing `tools/e2e_budget_revision.mjs` directly creates missions and resumes runs. It contains demo resets and defaults to the operator's normal ports/database. Its direct psql path also places the connection URL in arguments. Do not run it against retained operator resources or with private credentials in arguments.

The minimum full-stack reproduction should use:

1. An explicitly owned disposable PostgreSQL data directory and test database, independent of retained PostgreSQL on port 54329.
2. A separate loopback test server and runner, with exact process, port, binary, and data-path receipts; do not start an unattended Factory watcher.
3. The existing `scripts/fake-codex-app-server.mjs` protocol fixture through the native Codex adapter's supported command/prefix-argument settings, and the existing fake GitHub CLI for all controller interactions. No real provider inference or GitHub publication is required.
4. A single synthetic Factory issue: measured suspend, preserved source, owner-approved budget revision, explicit same-session/workspace provider continuation, verification, and exact mission/task/run/Factory state checks.
5. Negative cases for missing approval, stale revision/source, genuine stop/quarantine, and duplicate/lost responses; preserve original spend and lineage in every case.

Proposed isolated resource locations (not created or started by this baseline):

- Operator QA root: `.ecorp/qa/issue-50-factory-recovery`.
- PostgreSQL loopback port: 55450.
- API loopback port: 18450.

Both ports were observed unused and the QA root absent during preparation; recheck immediately before creation. Before the full-stack test, bind the harness to verified owned resources and remove credential-bearing command arguments. Stop only test-owned processes afterward and preserve reports/data for inspection. Do not alter #235, its failed checkpoint, the existing runner credential, or the retained operator database.

## Not performed

- No #50 implementation fix, full repository gate, or full-stack recovery reproduction yet.
- No new QA PostgreSQL/server/runner service yet.
- No budget change or resume for #235.
- No GitHub pull request, publication, merge, or deployment.
- Auditor policy #236 remains an implementation target, not an active runtime policy.
