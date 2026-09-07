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

## Reuse the harness before building

ECorp coordinates existing agent harnesses; it should not rebuild their execution loops, tools,
session persistence, or permission systems. For any proposed mechanism, record in the issue or PR:

1. The selected harness and pinned version, plus the native capability you checked.
2. The concrete requirement that capability does not satisfy.
3. The smallest adapter or ECorp-level change needed, with verification evidence.

Prefer the existing native session/resume and permission interfaces. Do not add another approval
for the same currently authorized action and exact scope, or turn every artifact into a decision.
ECorp still owns shared tenant authority, cross-agent coordination, budgets, durable audit, and
outcome verification. A harness permission callback or configured sandbox is not, by itself, proof
of those boundaries.

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

## Run your own contributor dark factory

A contributor clone contains the ECorp dark-factory implementation. OpenAI Symphony inspired parts
of the operating model, but Symphony is not an ECorp dependency and does not need to be installed.
Contributors supply their own provider identity and private runner workspace. A local server is
appropriate for solo testing or a disjoint backlog. Factories consuming the same backlog must use
the **same authenticated server/control plane, the same Corp, and the same claim namespace**
(the same canonical GitHub Project owner, Project number, and Project item identity).
Database co-location, separate Corps on one server, or a shared GitHub Project alone do not unify
claim authority.

Prerequisites are Git, Windows PowerShell, Rust 1.94 or newer, Node.js, pnpm 11.19.0, Docker with
Compose, GitHub CLI authenticated for the repository and Project #3, and any provider entitlement
required for real-agent work.

Clone ECorp and give the runner an execution root that is separate from the configured source
checkout:

```powershell
git clone https://github.com/shyamsridhar123/ecorp.git
Set-Location ecorp

$env:CRONY_SOURCE_REPOSITORY = (Get-Location).Path
$env:CRONY_SOURCE_BASE_REF = 'HEAD'
$env:CRONY_RUNNER_WORKSPACE = Join-Path $env:USERPROFILE '.ecorp\runner-workspaces'

./tools/start_local.ps1
Invoke-RestMethod http://127.0.0.1:8791/health
Invoke-WebRequest http://127.0.0.1:5187
./tools/e2e_smoke.ps1
./tools/stop_local.ps1
```

The server owns authoritative organizational state. The outbound runner owns provider processes and
isolated worktrees. Closing a browser or desktop client must not terminate a run. Do not share the
runner workspace, credential directory, or provider state directory with another contributor.
Shared deployments expose one authenticated ECorp authority, not shared database credentials.
Contributors running local tests on the same machine must also coordinate the stack's ports and
shared development Compose database.

### Use GitHub Copilot

ECorp's `github-copilot` adapter uses the official GitHub Copilot SDK. By default, the runner uses
the contributor's logged-in GitHub identity:

```powershell
gh auth status
$env:CRONY_COPILOT_USE_LOGGED_IN_USER = 'true'
./tools/start_local.ps1
```

The account and organization policy must allow GitHub Copilot. The runner discovers the models and
reasoning levels actually available to that account; do not hard-code a globally advertised model
that the runner does not expose.

A trusted host may instead provide a token through a file outside the repository and runner
workspace:

```powershell
$env:CRONY_COPILOT_GITHUB_TOKEN_FILE = 'C:\secure\ecorp\copilot.token'
./tools/start_local.ps1
```

Never place the token value in a prompt, command argument, log, issue, worktree, or committed file.
Do not share one contributor's Copilot identity with another contributor or with a producing agent.

GitHub Copilot is the recommended real-provider path for contributor factory work. External CLI
providers can have different isolation and process-lifecycle assurance; check current Project #3
issues before treating them as equivalent.

### Coordinate independent factories

Personal factory hosts do not create personal backlogs. Every contributor must use the same Project
#3 issue, status, revision, dependency, and `factory:ready` contract:

- run only open `Todo` issues that are explicitly labeled `factory:ready`;
- let the controller claim and revalidate the Project item before dispatch;
- use a unique, bounded write scope and avoid another active issue's paths;
- stop if the Project status, issue revision, dependency state, source commit, or policy changes;
- require persisted verifier evidence and independent review before publication; and
- never use `docs/BACKLOG.md` as a second queue.

Separate local servers can both observe an eligible issue before either changes its Project status.
GitHub status and labels are not an atomic execution lock. Do not run unattended consumers of the
same backlog against independent ECorp databases. Use the same server/control plane, same Corp,
and same claim namespace with separately enrolled runners, or explicitly partition the eligible
issue sets. Merely placing databases on one host is not coordination. Automatic enforcement and multi-host
acceptance remain tracked in [#161](https://github.com/shyamsridhar123/ecorp/issues/161); this guidance
does not claim that the gap is fixed.

For a shared remote ECorp deployment instead of independent local factories, configure production
OIDC, Corp membership, runner enrollment, private artifact storage, and the other production
boundaries in the architecture and security guides.

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

GitHub API throttling is a different limit. The factory observes GraphQL quota
from GraphQL responses and persists the next permitted retry. Leave a throttled
controller running: its heartbeat and local runner work continue, and intake
resumes after the displayed wait. Restart, Resume and Reconcile do not bypass
that deadline. Do not rotate accounts, reset the database, recreate the mission
or move its Project status to work around throttling. See the contributor guide's
quota/recovery section for the supported behavior.

## Pull requests

- Link the source issue.
- State the source base, write scope, changed files, and exact local validation.
- For orchestration or runtime mechanisms, include the native-capability check and the specific
  ECorp gap rather than proposing a parallel harness.
- Distinguish observed local evidence from unverified hosted or production claims.
- Keep merge, auto-merge, and deployment disabled unless a separate current authorization permits
  them. Ordinary contribution and factory publication authorize review only.
