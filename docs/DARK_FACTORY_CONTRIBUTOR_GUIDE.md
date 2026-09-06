# ECorp dark-factory contributor guide

This guide explains how to curate work, run ECorp locally, dispatch bounded missions, preserve
conflict-free execution, recover from failures, and publish verified results for review.

ECorp's dark factory is inspired by the general tracker-driven, isolated-workspace orchestration
model described by [OpenAI Symphony](https://github.com/openai/symphony). This guide was
independently written and cites Symphony only as conceptual inspiration. ECorp uses its own
terminology, commands, state machines, governance, and architecture. Symphony and ECorp are both
Apache-2.0-licensed projects, but ECorp remains an independent implementation.

## Contents

- [Operating model](#operating-model)
- [Local setup](#local-setup)
- [Curate the live backlog](#curate-the-live-backlog)
- [Write a dispatchable issue](#write-a-dispatchable-issue)
- [Run the factory controller](#run-the-factory-controller)
- [Conflict-free execution](#conflict-free-execution)
- [Verification and evidence](#verification-and-evidence)
- [Recovery](#recovery)
- [Publish a verified result](#publish-a-verified-result)
- [Security rules](#security-rules)
- [Issue resolution and closure](#issue-resolution-and-closure)
- [Current operational constraints](#current-operational-constraints)

## Operating model

The durable path is:

```text
GitHub Project issue
        |
        v
curated contract + factory:ready
        |
        v
fenced ECorp work item + immutable source commit
        |
        v
mission + bounded task graph
        |
        v
runner-owned isolated worktree
        |
        v
persisted automated evidence
        |
        v
human approval or independent review
        |
        v
signed source deliverable
        |
        v
review pull request
```

The following boundaries are non-negotiable:

- **GitHub Project #3 is planning authority.** The Project and linked issues hold current priority,
  status, sequencing, ownership, dependencies, and release gates.
- **`docs/BACKLOG.md` is historical.** It seeded early milestones. It is not an active parallel
  backlog and must not receive new work items or status updates.
- **The server is authoritative.** A browser, desktop client, controller process, or provider
  process may disappear without erasing durable task, approval, budget, or event state.
- **The runner owns execution.** The server never executes untrusted agent shell commands.
- **A message is not a task.** Every executable task has an explicit contract, state machine,
  budget, retry policy, and verifier policy.
- **Provider completion is not accepted completion.** Persisted checks and required manual gates
  must pass.
- **Publication is not integration.** Creating or recovering a pull request does not authorize
  auto-merge, merge, deployment, or another irreversible effect.

## Local setup

### Prerequisites

- Git
- Windows PowerShell
- Rust 1.94 or newer
- Node.js; the repository does not declare a minimum version
- pnpm 11.19.0
- Docker with Compose
- GitHub CLI authenticated with repository and Project access for live factory operations
- Free local ports:
  - PostgreSQL: `54329`
  - server: `8791`
  - web: `5187`

The deterministic `fake-process` adapter works without provider credentials. Real adapters require
their own authenticated runtime or account.

Verify GitHub CLI access before Project or publication work:

```powershell
gh auth status
```

### Start the complete stack

From an isolated ECorp contribution worktree:

```powershell
./tools/start_local.ps1
```

The script:

1. stops only processes recorded for this worktree;
2. starts PostgreSQL through `deploy/compose/docker-compose.yml`;
3. installs JavaScript dependencies unless `-SkipInstall` is supplied;
4. builds `crony-server`, `crony-runner`, and `crony-cli` unless `-SkipBuild` is supplied;
5. starts the Rust server;
6. bootstraps the development Corp and creates a one-time runner enrollment;
7. starts an outbound runner; and
8. starts the Vite web client.

Reuse existing dependencies and binaries only when they match the current source:

```powershell
./tools/start_local.ps1 -SkipInstall -SkipBuild
```

### Configure the source repository

Set the source repository before starting the stack:

```powershell
$env:CRONY_SOURCE_REPOSITORY = 'C:\path\to\repository'
$env:CRONY_SOURCE_BASE_REF = 'HEAD'
$env:CRONY_RUNNER_WORKSPACE = 'C:\path\to\isolated-runner-workspace'
./tools/start_local.ps1
```

`CRONY_SOURCE_REPOSITORY` is a read-only source checkout from the perspective of write-capable
runs. The runner must create linked worktrees below `CRONY_RUNNER_WORKSPACE`; it must never fall
back to editing the configured checkout.

The Missions **Run setup** screen lists the structured repository, ref, and immutable commit
advertised by every connected runner. Select and confirm the intended target before choosing a
runtime. Runtime and model choices are filtered to runners serving that exact source. Local
repositories without a GitHub remote use a stable `local/<name>-<digest>` identity.

CLI callers can pin the same tuple explicitly:

```powershell
crony mission $corpId $actorId `
  --adapter github-copilot `
  --source-repository local/example-0123456789ab `
  --source-base-ref HEAD `
  --source-base-commit 0123456789abcdef0123456789abcdef01234567 `
  'Build and verify the requested application.'
```

For factory work, the configured Git remote, symbolic source ref, and resolved immutable commit must
match the persisted factory policy and the runner's advertised capability.

### Verify the stack

```powershell
Invoke-RestMethod http://127.0.0.1:8791/health
Invoke-WebRequest http://127.0.0.1:5187
./tools/e2e_smoke.ps1
```

Open `http://127.0.0.1:5187`. The smoke test exercises the real server, runner, child process,
control lease, artifact upload/download, hash verification, event journal, and worktree
disposition.

Stop only this worktree's recorded processes:

```powershell
./tools/stop_local.ps1
```

`stop_local.ps1` does not stop PostgreSQL. The current Compose project name is shared by worktrees,
so do not run `docker compose ... down` from one worktree unless every user of that shared database
has coordinated the shutdown.

Do not run repository E2E scripts against shared development data unless the script explicitly
supports it. Factory E2Es may restart the server, create temporary repositories and credentials, or
modify test database state.

## Curate the live backlog

Inspect the live Project and issues before creating work:

```powershell
gh project view 3 --owner shyamsridhar123 --format json
gh project item-list 3 --owner shyamsridhar123 --limit 200 --format json
gh issue list --state all --limit 200
gh issue list --state all --search 'dark factory contributor'
```

For a new issue:

1. Search for an existing issue with the same outcome or failure.
2. Prefer one issue per independently verifiable outcome.
3. Add the issue to Project #3; do not create a `BACKLOG.md` row.
4. Set the Project status to `Todo` while it is curated and not executing.
5. Record explicit dependencies.
6. Apply `factory:ready` only when the issue is genuinely dispatchable.

Add an existing issue to the Project:

```powershell
$IssueUrl = 'https://github.com/shyamsridhar123/ecorp/issues/123'
gh project item-add 3 --owner shyamsridhar123 --url $IssueUrl
```

Use Project statuses consistently:

| Status | Meaning |
| --- | --- |
| `Todo` | Curated work that is not executing. A new factory item must start here. |
| `In Progress` | Durably claimed, active, blocked while recovering, or otherwise owned work. |
| `In Review` | The exact verified pull request exists and is ready for human review. |
| `Done` | The issue's acceptance and intended integration or closure condition are satisfied. |

Never advance status because work exists only on one machine, an agent says it is done, a provider
produces an artifact, or a PR is merely planned.

### Deduplicate and capture discoveries

- Link duplicates and close the redundant issue with a durable reason.
- Use parent/sub-issues for a large outcome and its bounded implementation slices.
- When execution exposes a new product gap, security concern, or missing feature, create a separate
  linked issue instead of silently widening the active mission.
- Create a recovery issue only when the existing lineage cannot safely continue. Include the
  original issue, work item, mission, run, source base, checkpoint branch/commit, failed gate, and
  remaining bounded work.
- Put temporary operational timelines in issue comments or dated evidence files. Keep timeless
  product documentation focused on invariants and supported behavior.

## Write a dispatchable issue

The controller parses acceptance criteria and dependencies from exact Markdown sections. Use this
shape:

```markdown
## Outcome

State the observable user or system result.

## Acceptance criteria

- [ ] First verifiable result.
- [ ] Exact tests or evidence required.
- [ ] Required approval or independent review.

## Dependencies

Blocked by #120 and #121.
Aligned with #63.

## Governed delivery contract

- Source repository: `owner/repository`.
- Source base: `main` at exact commit `...`.
- Provider and strategy: ...
- Write scope: ...
- Token and cost budgets: ...
- Allowed tools and prohibited actions: ...
- Persisted verifier policy: ...
- Publication: one review PR; no merge, auto-merge, or deployment.
```

Only issue references following `Blocked by` inside the `## Dependencies` section are execution
blockers. General references such as `Aligned with` do not block dispatch. The controller reads
checklist items only from `## Acceptance criteria`. The rest of the issue records contributor
intent and review context; it does not replace enforced controller arguments and the persisted
factory policy. Write scope, adapter/model selection, budgets, and verifier policy must be supplied
through the controller inputs described below. Other task-contract fields are materialized by the
selected planning strategy and must be reviewed in the dry-run output.

Before applying `factory:ready`, verify:

- the issue is open and present in Project #3 as `Todo`;
- the outcome and acceptance checklist are complete;
- all explicit blockers are closed;
- the repository and symbolic source base are correct;
- the intended source commit can be resolved and is acceptable;
- the write scope is minimal and non-overlapping;
- provider, strategy, model, and reasoning constraints are explicit where needed;
- token and cost budgets are bounded;
- allowed tools, prohibited actions, references, deadline, and escalation path are sufficient;
- the persisted verifier policy checks the actual deliverable;
- human approval or independent review is included when required;
- publication, merge, auto-merge, and deployment authority are explicitly separated.

Apply the label only after curation:

```powershell
gh issue edit 123 --add-label factory:ready
```

A label and Project status are eligibility signals, not an execution lock. ECorp's durable claim,
lease, fencing token, policy snapshot, and version provide execution authority.

## Run the factory controller

### Preview without mutation

Run a dry run first:

```powershell
$CorpId = '00000000-0000-4000-8000-000000000001'
$ActorId = '00000000-0000-4000-8000-000000000011'
$IssueNumber = 123

cargo run -p crony-cli -- factory `
  $CorpId `
  $ActorId `
  --owner shyamsridhar123 `
  --project-number 3 `
  --repository shyamsridhar123/ecorp `
  --source-repository-path . `
  --source-base-ref HEAD `
  --publication-base-ref main `
  --adapter fake-process `
  --strategy single `
  --budget-tokens 500000 `
  --budget-cost-microusd 1000000 `
  --write-scope 'docs/**' `
  --write-scope 'CONTRIBUTING.md' `
  --issue $IssueNumber `
  --dry-run
```

Use the real required adapter instead of `fake-process` for provider work. The connected runner
must advertise the selected adapter and the same repository, source ref, and immutable commit.

Review the dry-run output for:

- selected issue and Project item;
- eligibility and dependency results;
- source repository, symbolic ref, and immutable commit;
- adapter, strategy, model, and reasoning selection;
- write scope;
- token and cost authority;
- verification policy;
- publication base; and
- an empty mutation list.

Remove `--dry-run` only after the preview matches the issue contract. The controller then claims the
item, persists the policy, atomically materializes one mission, updates Project status after durable
state exists, and dispatches only to a matching runner.

Replaying the same request after a lost response or controller restart should recover the durable
work item and mission. Do not create a replacement issue or second mission merely because the
controller's response was lost.

### Supply an explicit verifier policy

A verification policy is authority-bearing. Store it outside agent-readable secret locations and
pass it explicitly:

```json
{
  "checks": [
    { "type": "artifact", "min_bytes": 1 },
    { "type": "file", "path": "CONTRIBUTING.md", "min_bytes": 1000 }
  ],
  "manual_gate": {
    "type": "independent_review",
    "roles": ["owner", "admin", "manager", "member"],
    "exclude_requester": true
  }
}
```

```powershell
cargo run -p crony-cli -- factory `
  $CorpId `
  $ActorId `
  --owner shyamsridhar123 `
  --project-number 3 `
  --repository shyamsridhar123/ecorp `
  --source-repository-path . `
  --source-base-ref HEAD `
  --publication-base-ref main `
  --adapter codex `
  --budget-tokens 500000 `
  --budget-cost-microusd 1000000 `
  --write-scope 'docs/**' `
  --write-scope 'CONTRIBUTING.md' `
  --verification-policy-file .\verification-policy.json `
  --issue $IssueNumber `
  --dry-run
```

Do not choose arbitrary byte floors or surrogate checks that can reject a correct result or pass an
incorrect one. Verify behavior at the scope of the claim.

## Conflict-free execution

### Isolated worktrees

Write-capable runs never execute in the configured source checkout. Initial runs receive a
deterministic branch and linked worktree. Resume may reuse only the exact preserved provider
session, branch, worktree, and source identity.

Provisioning fails closed when a path is occupied, detached, mismatched, outside the workspace
root, or not a valid worktree. It must not fall back to the source checkout.

Cleanup also fails safe. Preserve the worktree when it is:

- dirty;
- committed but not integrated or tree-equivalent to the base;
- carrying relevant ignored files;
- detached;
- mismatched; or
- otherwise unverifiable.

Never force-remove uncertain work. Inspect its source identity, branch, commits, changed files,
provider session, run lineage, and evidence before deciding the next action.

### Human contributor worktrees

Create an isolated branch and worktree from the intended source:

```powershell
git fetch origin
git worktree add ..\ecorp-issue-123 -b codex/issue-123 origin/main
Set-Location ..\ecorp-issue-123
```

Audit before and after work:

```powershell
git status --short --branch
git diff --check
git diff --name-only origin/main...HEAD
```

Do not mutate another contributor's configured checkout, reuse a factory worktree for unrelated
work, or delete a preserved worktree to make a retry convenient.

### Overlapping scopes and stacked pull requests

- Avoid concurrent missions that write the same paths.
- Serialize database migrations, lockfiles, shared schemas, central routing, and common governance
  documents unless the scopes are demonstrably independent.
- Record the parent PR/branch and exact base commit for stacked work.
- Document landing order in the issues and PRs.
- Start from the actual parent branch, not an assumed future `main`.
- After a parent changes or lands, deliberately update the child and rerun the applicable verifier
  and local gates on the integrated head.
- Do not force-push a conflicting publication branch. Recover only an exact matching remote effect;
  otherwise fail closed and choose a separately authorized path.

Rebase only your own coordinated branch when its consumers understand the history change. Never
use history rewriting or worktree deletion as a substitute for provenance and reconciliation.

## Verification and evidence

Completion requires evidence at the same scope as the claim.

### Test layers

1. **Targeted checks:** tests, formatters, link checks, or focused E2Es for the touched behavior.
2. **Repository gates:** required when shared contracts, Rust code, web code, migrations, runtime
   behavior, or publication boundaries may be affected.
3. **Complete runtime evidence:** required for user-visible behavior. Start the complete stack and
   prove browser to server to runner to child process to artifact to persisted state and back to the
   browser.
4. **Manual gates:** durable human approval or independent review when policy requires it.

The repository gate from `AGENTS.md` is:

```powershell
node tools/check_migrations.mjs
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm build:web
pnpm lint:web
```

The aggregate command is:

```powershell
pnpm check
```

For factory-specific changes, select focused scripts from:

```powershell
node tools/e2e_factory_claims.mjs
node tools/e2e_factory_controller.mjs
node tools/e2e_factory_commit_routing.mjs
node tools/e2e_factory_publication.mjs
```

Run these against a dedicated local or test database. Some restart the server, create temporary
repositories and credentials, or manipulate migration-shaped test state.

`tools/e2e_factory_legacy_source_upgrade.mjs` is more invasive: its SQL path targets the default
Compose `crony` database directly. Run it only in a test-owned Compose stack with no shared
development state.

### Evidence record

Record:

- exact commit SHA and source base;
- exact commands;
- pass/fail counts and relevant output;
- browser, server, runner, provider, artifact, and database observations where applicable;
- failed checks and whether they are related to the change;
- work item, mission, task, and run IDs;
- artifact or source-deliverable digest;
- approving and reviewing actors;
- branch and pull request;
- whether hosted CI ran; and
- any remaining authorization, review, or integration step.

Do not report:

- a started command as a passed command;
- provider output as verifier evidence;
- a local test as hosted or production evidence;
- a PR as merged when it is merely open;
- an interrupted or cancelled run as complete; or
- a clean subset as a clean full suite.

## Recovery

Always inspect persisted state before acting. Do not infer state from a controller terminal, browser
tab, or provider transcript.

| Condition | Required response |
| --- | --- |
| Lost controller response or restart | Replay the same idempotent request. Recover the existing work item and mission; do not create a duplicate. |
| GitHub synchronization failure | Persist `blocked`, preserve the lease/work item/mission, repair the cause, revalidate the source issue and dependencies, then recover. |
| Runner disconnect | Respect the grace and reconciliation state. Do not kill or replace a process that may still be owned by the runner. |
| Interrupted run | Inspect process, provider session, run state, worktree, branch, and evidence before resuming or replacing it. |
| Execution failure | Preserve logs, events, attempts, source identity, and worktree disposition. Retry only when the persisted policy and remaining budget permit it. |
| Budget exhaustion at `suspend` | Preserve original authority and consumed usage. Continue only through an authorized budget revision or narrower finish scope followed by a separate resume operation. |
| Hard breaker `stop` | Treat the lineage as terminal. Do not resume an ancestor to bypass the stop. |
| Verifier failure | Preserve the exact commit, worktree, prior evidence, and failed check. Distinguish correct source from failed acceptance metadata. |
| Cancelled mission | Verify whether cancellation is terminal. Never repeatedly resume a cancelled lineage without explicit supported authority. |
| Exhausted attempts | Require a new governance decision. Never silently reset attempt counters. |
| Orphaned run or workspace | Reconcile runner ownership, persisted assignment, process state, source commit, and branch before cleanup or replacement. Preserve anything uncertain. |
| Staged or orphaned artifact | Use server reconciliation. Do not manually delete bytes that may be reserved or content-addressed. |
| Dirty, committed, or unverifiable worktree | Preserve it and create durable recovery provenance before any new execution. |

### Current lessons from issues #50 and #113

[Issue #50](https://github.com/shyamsridhar123/ecorp/issues/50) and
[issue #113](https://github.com/shyamsridhar123/ecorp/issues/113) are current product-gap records,
not timeless product guarantees.

- **Budget recovery:** the presence of a budget-revision API does not prove a suspended mission is
  operationally recoverable. After approval and resume, inspect the resulting mission, task, run,
  and factory states. If the lineage still becomes terminal, preserve the remote checkpoint,
  record the defect, and do not claim recovery succeeded.
- **Verifier-only recovery:** a failed threshold or metadata check does not make a correct source
  commit disposable. Preserve the commit and evidence. Until same-lineage governed retry works for
  the specific failure, a linked recovery issue is a transparent workaround, not proof that the
  original recovery model is complete.

These lessons require contributors to report the actual persisted result and to keep correct work
recoverable even when the control path has a defect.

## Publish a verified result

Publication is allowed only after:

- the factory item is authoritatively `verified`;
- all automated checks pass;
- required approval or independent review is complete;
- the exact signed commit/branch deliverable exists;
- current role, room membership, budget, breaker, source, and publication policy still authorize
  the effect; and
- an owner or admin has created a short-lived publisher credential for the trusted publisher.

### Create a short-lived publisher credential

For a strong boundary, use a dedicated trusted publisher host or OS identity that does not run
provider children. The following development-mode example is reduced assurance because the server,
runner, provider, and publisher may share one Windows user. Stop active provider processes before
creating the credential, keep the credential outside the repository and runner workspace, and
delete it immediately after the one publication attempt.

An owner/admin can enroll a credential through the server API:

```powershell
$PublisherId = "crony-cli:$env:COMPUTERNAME"
$PublisherDirectory = Join-Path $env:USERPROFILE '.ecorp-publisher'
$PublisherCredentialPath = Join-Path $PublisherDirectory 'publisher.credential'
New-Item -ItemType Directory -Path $PublisherDirectory -Force | Out-Null

$Publisher = Invoke-RestMethod `
  -Method Post `
  -Uri "http://127.0.0.1:8791/api/corps/$CorpId/factory/publication-publishers/credentials" `
  -ContentType application/json `
  -Body (@{
    actor_id = $ActorId
    publisher_id = $PublisherId
    expires_in_seconds = 600
  } | ConvertTo-Json)

Set-Content `
  -LiteralPath $PublisherCredentialPath `
  -Value $Publisher.credential `
  -NoNewline
```

The plaintext is returned once. Production requests also require the configured human
authentication token.

### Preview and publish

```powershell
$WorkItemId = '<factory-work-item-id>'

cargo run -p crony-cli -- factory-publish `
  $CorpId `
  $ActorId `
  $WorkItemId `
  --publisher-id $PublisherId `
  --publisher-credential-file $PublisherCredentialPath `
  --authorization-reason 'Publish the verified result for human review.' `
  --dry-run
```

Review the exact deliverable, repository, base, branch, commit, PR title/body, source issue,
authorization, and Project effect. Then remove `--dry-run`.

The publisher:

- verifies the signed Git bundle and exact commit;
- refuses a conflicting branch and never force-pushes it;
- creates or adopts only an exact matching same-repository pull request;
- records the PR before moving the Project item to `In Review`;
- recovers matching effects after restart or a lost response; and
- leaves auto-merge, merge, and deployment disabled.

### Revoke and delete the credential

Immediately after publication or abandonment:

```powershell
$CredentialId = $Publisher.credential_id

Invoke-RestMethod `
  -Method Post `
  -Uri "http://127.0.0.1:8791/api/corps/$CorpId/factory/publication-publishers/credentials/$CredentialId/revoke" `
  -ContentType application/json `
  -Body (@{
    actor_id = $ActorId
    reason = 'Publication completed; revoke the one-use credential.'
  } | ConvertTo-Json)

Remove-Item -LiteralPath $PublisherCredentialPath -Force
Remove-Variable Publisher -ErrorAction SilentlyContinue
```

Verify revocation succeeded and the plaintext file no longer exists. Do not reuse a publication
credential across unrelated issues or leave it for a later run.

## Security rules

- Use least privilege for every actor, runner, provider, secret, tool, repository, path, and
  publication effect.
- Never put long-lived secrets in prompts, logs, command arguments, issue bodies, artifacts,
  evidence, or agent-readable files.
- Use typed secret references. Broker short-lived values against actor, task, run, runner, tool,
  resource, and expiry scope.
- Treat environment-only secret delivery as reduced assurance.
- Keep GitHub credentials in the trusted publisher's keyring or process environment. Never provide
  them to the runner or producing agent.
- Use short-lived, scoped, preferably one-use publisher credentials. Revoke them and delete
  plaintext files after use.
- Require durable approval and idempotency for irreversible or risky effects.
- Revalidate current authority immediately before external mutations.
- Never infer authorization from an old issue comment, stale role, previous approval, label,
  Project status, or possession of a worktree.
- Do not publish exploit details in a public issue while private vulnerability reporting is
  unavailable; contact the repository owner.

## Issue resolution and closure

Keep issue and Project state current throughout work:

1. Comment when work is claimed, blocked, checkpointed, recovered, verified, or published.
2. Include durable identifiers and exact source provenance.
3. Record newly discovered gaps as linked issues.
4. Update explicit dependencies when they change.
5. Do not close the parent outcome merely because one implementation slice completed.

Close an issue only when:

- every applicable acceptance item has evidence;
- dependencies are closed, superseded, or explicitly removed with rationale;
- the exact source base, branch, commit, and output provenance are recorded;
- persisted automated verification passes;
- required approval and independent review are complete;
- user-visible behavior has complete runtime evidence when applicable;
- the review PR contains the exact verified commit;
- Project status reflects the actual integration/closure state;
- remaining failures and deferred work are captured separately; and
- no merge, auto-merge, deployment, or credential authority is implied.

Link the issue and pull request in both directions. A PR body should include the closing or related
issue, changed files, source base, local validation, hosted-CI status, and explicit statement that
auto-merge and merge remain disabled.

## Current operational constraints

As of **September 3, 2026**:

- GitHub-hosted Actions credits are exhausted for the month. Jobs may be rejected before any step
  runs.
- Do not rely on Actions as the completion gate and do not remain blocked solely because hosted
  jobs cannot start.
- Record reproducible local checks, full-stack evidence when applicable, exact commit identity, and
  the fact that hosted Actions were unavailable.
- Issues #50 and #113 remain open recovery gaps; use the rules in [Recovery](#recovery) and do not
  overstate current support.

This dated section should be updated when the external constraint or product gaps change. The
invariants elsewhere in this guide remain the contributor contract.
