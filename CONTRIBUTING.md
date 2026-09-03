# Contributing to ECorp

ECorp is a governed execution system, not a collection of independent agent sessions. Contributions
must preserve the product, security, isolation, and evidence contracts described in
[`AGENTS.md`](AGENTS.md).

## Start here

Before a non-trivial change, read:

1. [`docs/PRODUCT_AND_TECHNICAL_PLAN.md`](docs/PRODUCT_AND_TECHNICAL_PLAN.md)
2. [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)
3. [`docs/SECURITY.md`](docs/SECURITY.md)
4. [`docs/EVALS.md`](docs/EVALS.md)
5. [`docs/DARK_FACTORY_CONTRIBUTOR_GUIDE.md`](docs/DARK_FACTORY_CONTRIBUTOR_GUIDE.md) for issue
   intake, factory operation, recovery, publication, and governance

## Planning source of truth

[ECorp Build GitHub Project #3](https://github.com/users/shyamsridhar123/projects/3) is the live
system for priorities, status, sequencing, ownership, dependencies, and release gates.
[`docs/BACKLOG.md`](docs/BACKLOG.md) is historical seed material only. Do not add or maintain an
active work item in `BACKLOG.md`.

Search the Project and repository issues before creating work. Add every live issue to Project #3,
record explicit dependencies, and use one independently verifiable outcome per issue.

## Work safely

- Never run write-capable work in the configured source checkout.
- Use an isolated branch and linked worktree for every contribution or factory run.
- Avoid overlapping write scopes. Coordinate shared migrations, lockfiles, schemas, and common
  documentation as serialized work.
- Preserve dirty, committed, detached, mismatched, or unverifiable worktrees. Never force-remove
  uncertain work.
- Record the exact source branch and commit for stacked work, then deliberately update and
  re-verify after the parent lands.
- Do not put long-lived secrets in prompts, logs, command arguments, or agent-readable files.

For a normal human contribution:

```powershell
git fetch origin
git worktree add ..\ecorp-issue-123 -b codex/issue-123 origin/main
Set-Location ..\ecorp-issue-123
```

Replace `123` with the issue number. If the work is intentionally stacked, replace `origin/main`
with the recorded parent branch only after coordinating the landing order.

## Run locally

Prerequisites are Git, Windows PowerShell, Rust 1.94 or newer, Node.js, pnpm 11.19.0, Docker with
Compose, and GitHub CLI for live GitHub operations.

```powershell
./tools/start_local.ps1
Invoke-RestMethod http://127.0.0.1:8791/health
Invoke-WebRequest http://127.0.0.1:5187
./tools/e2e_smoke.ps1
./tools/stop_local.ps1
```

The server owns authoritative organizational state. The outbound runner owns provider processes and
isolated worktrees. Closing a browser or desktop client must not terminate a run.

## Validate before publishing

Run targeted tests for the changed behavior. For user-visible behavior, exercise the complete
browser-to-server-to-runner path; unit tests alone are insufficient.

The repository gate is:

```powershell
node tools/check_migrations.mjs
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm build:web
pnpm lint:web
```

`pnpm check` runs the same sequence. Record exact commands, results, commit identity, and any
unrelated failure truthfully.

As of September 3, 2026, GitHub-hosted Actions credits are exhausted for the month. Do not treat an
unstarted hosted job as a completion gate or remain blocked solely for that reason. Record
reproducible local validation evidence and state clearly that hosted Actions did not run.

## Pull requests

- Link the source issue.
- State the source base, write scope, changed files, and exact local validation.
- Distinguish observed local evidence from unverified hosted or production claims.
- Keep merge, auto-merge, and deployment disabled unless a separate current authorization permits
  them. Ordinary contribution and factory publication authorize review only.
