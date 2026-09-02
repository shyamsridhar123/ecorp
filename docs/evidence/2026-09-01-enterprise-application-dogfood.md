# Enterprise application dogfood

Date: September 1, 2026

## Scope

This pass used the handoff snapshot to run three realistic application builds through the live
ECorp browser, control plane, PostgreSQL store, outbound runner, provider adapter, and isolated Git
worktree path.

This was one operator using controlled scenarios. It does **not** satisfy GitHub issue #27, which
requires three external human teams.

Repository state at the start:

- ECorp source branch at the start: `main`
- ECorp source commit: `67705f8d7a6da69171bea798b6c756423af50977`
- Scenario source repository: `output/enterprise-dogfood-20260901/source`
- Scenario source commit: `59bb8608eab986cde82619a7141c611311d92fe9`
- Web console: `http://127.0.0.1:5187`
- Control plane: `http://127.0.0.1:8791`

The scenario repository contained detailed, committed specifications for:

1. a third-party vendor onboarding and risk-review system;
2. an incident-response command center;
3. a regulated credit-policy exception workflow.

All specifications prohibited network access and package installation and required only built-in
Node.js or Python capabilities.

## Host recovery and launcher correction

Docker Desktop 4.84 initially crashed while opening stale Windows Unix-socket reparse points. The
stale `Docker\run` and `docker-secrets-engine` directories were preserved under timestamped backup
names, Docker was restarted cleanly, and PostgreSQL became available.

The documented `CRONY_SOURCE_REPOSITORY`, `CRONY_SOURCE_BASE_REF`, and
`CRONY_RUNNER_WORKSPACE` settings were not honored by `tools/start_local.ps1` because explicit
hard-coded command arguments overrode the runner environment. The launcher now resolves and passes
those settings. The corrected launcher was used repeatedly to start the scenario stack.

## Scenario 1: VendorGuard

### ECorp execution

- Provider: GitHub Copilot SDK
- Model: `gpt-5.6-sol`
- Reasoning effort: `high`
- Mission: `c793c30c-57f3-4290-b38a-cde2f98235c8`
- Task: `e8760090-2802-42c1-b189-cea45e109d11`
- Initial run: `8abe7f93-844d-40dd-ac19-f64567995eb0`
- Resume run: `5485d885-b4df-4ec1-9258-afca26b04192`
- Budget: 500,000 tokens
- Operator action approvals: 11, all approved after command/path inspection

The run created a launchable Node.js vendor-risk application with tenant isolation, role checks,
maker-checker separation, idempotency, optimistic concurrency, deterministic risk scoring, a
SHA-256-linked audit chain, browser UI, tests, documentation, and evidence.

The initial run suspended at 507,045 tokens. The explicit resume reused the provider session and
worktree, consumed 13,221 additional tokens, and immediately suspended because cumulative mission
usage reached 520,266 against the unchanged 500,000 mission limit. ECorp therefore did not accept
the otherwise working application.

### Independent validation

`node --test` passed:

- 13 tests
- 6 suites
- 13 passed
- 0 failed

The browser workflow proved:

- creation of a high-risk vendor;
- requester submission;
- independent reviewer approval;
- cross-tenant empty-state isolation;
- reviewer denial from the admin-only audit operation;
- admin audit verification with three valid linked entries.

Screenshots:

- `output/playwright/vendor-guard-plan.png`
- `output/playwright/vendor-guard-approved-by-bob.png`
- `output/playwright/vendor-guard-workflow.png`
- `output/playwright/vendor-guard-admin-audit.png`

### Finding

The advertised suspend/resume checkpoint is not recoverable after the mission-level budget has
already been crossed because there is no authorized budget revision or bounded re-scope operation.

Tracked in GitHub issue #50.

## Scenario 2: Incident Command

### ECorp execution

- Provider: OpenAI Codex app-server
- Mission: `5ff2b2d2-f113-4ea9-9bcf-d2d2b8eb5040`
- Task: `312b0cd2-a15d-4d6d-9e52-f2d9466c320b`
- Run: `4613690f-0b0a-483d-bd33-8aacef2f175d`
- Budget: 2,000,000 tokens
- Accepted artifact: `158b3174-2158-4e7c-936d-cc200eb9465a`
- Artifact SHA-256:
  `b1adc3875b5c417a3a75a666e98afaac5ce800a4423c2c6320150c0e9957a18d`

The first several minutes produced reasoning notifications but no files. A live steering message
asked the provider to prioritize a compact runnable implementation. Codex then created the
application, tests, browser UI, documentation, and evidence in the assigned worktree.

The accepted artifact is a 4,183-byte JSON provenance envelope listing 11 source files and hashes.
The application source itself remained untracked in the preserved worktree.

### Independent validation

`node --test` passed:

- 12 tests
- 12 passed
- 0 failed

The browser workflow proved:

- incident declaration;
- commander assignment and guarded state transitions;
- responder timeline update;
- transition through mitigation, resolution, and postmortem completion;
- tenant-scoped SSE connection;
- eight valid linked audit entries;
- cross-tenant empty-state isolation.

The generated browser workflow also exposed an application-quality gap: the postmortem update event
was recorded, but the reloaded contributing-factor and corrective-action fields were empty.

Screenshots:

- `output/playwright/incident-command-dispatched.png`
- `output/playwright/incident-command-app.png`
- `output/playwright/incident-command-workflow.png`

### Budget authority failure

The run persisted 3,376,985 tokens against a 2,000,000-token limit. The event journal recorded:

1. `run.usage`;
2. a `stop` breaker transition;
3. runner acknowledgment of the stop command;
4. artifact acceptance;
5. verification pass;
6. accepted run, task, and mission completion.

The final projection was contradictory: `status=completed`,
`verification_status=passed`, and `breaker_stage=stop`.

A local correction now:

- emits each unique Codex `tokenUsage.last` API-call increment immediately so an active turn can
  receive budget controls without double-counting duplicate notifications;
- blocks artifact upload, verification progress, approval requests, status revival, and completion
  after `suspend` or `stop`;
- blocks human approval and manual verification decisions from bypassing a hard breaker;
- treats a hard-breaker failure as non-retryable;
- adds a misbehaving late-completion fixture.

`node tools/e2e_budgets.mjs` passed after the correction. The late-completion mission ended after
one failed attempt, uploaded no accepted artifact, and emitted zero `run.completed` events.

An authenticated Codex run on September 2, 2026 then used 33,438 tokens against a one-token ceiling.
The journal persisted usage, `stop`, command acknowledgment, and failure in that order. Run, task,
and mission ended `failed`; no artifact, verification pass, completion event, retry, or provider
descendant remained. The desktop and mobile UI showed one coherent failed outcome. See
`docs/evidence/2026-09-02-real-provider-budget-stop.md`.

Issue #49 is complete. The same run exposed additional provider-isolation evidence retained under
issue #51.

## Scenario 3: Credit Exception

### Initial Claude behavior

- Initial run: `9350cdac-9952-4463-90c9-26dd1268efc1`

The default Claude Code adapter loaded user plugins, MCP servers, browser/shell customization, and
spawned a Telegram MCP process. Interruption killed the top-level provider process but left an
orphan `bun.exe` descendant. The clean worktree could not be removed because the descendant still
held it, while ECorp reported that no provider process remained.

The loaded `remember` hook also wrote `.remember/` into the configured source checkout rather than
the assigned worktree. Its own log recorded the run worktree as `PROJECT_DIR` and
`output/enterprise-dogfood-20260901/source/.remember` as `REMEMBER_DIR`. That directly violated the
source-checkout isolation invariant. The generated directory was archived and then removed after
its resolved path and contents were verified:

- archive:
  `output/enterprise-dogfood-20260901/provider-contamination/claude-remember-source-checkout.log`
- SHA-256:
  `7de43b67fa135d8f8900d44a46275decff5d9e991828fd050abf081667d17772`

### Local isolation correction

The Claude invocation now uses:

- `--safe-mode`
- `--no-chrome`
- `--disable-slash-commands`
- `--strict-mcp-config`
- an empty MCP server map

The isolated rerun did not load the user plugin/MCP stack and began writing the assigned
application.

### Remaining permission failure

With `acceptEdits`, Claude emitted test-command approval requests only as provider text; ECorp did
not create durable action approvals or return decisions.

An experimental retry used Claude `auto` permission mode. On resume run
`d96084f9-acfe-439d-acf2-04cbb1160fff`, the provider-side safety classifier
`claude-opus-4-8` remained unavailable. Required test commands could not execute, and no ECorp
approval path existed.

The experiment was not retained as the permission solution. The adapter remains in edit-only
permission mode until provider requests can suspend through durable ECorp approvals.

An independent operator run executed 116 generated tests:

- 30 passed
- 84 failed
- 2 errored

The dominant failure was `POST /api/requests` returning HTTP 405, which cascaded through workflow,
tenant, idempotency, concurrency, audit, and validation tests. The run was stopped without accepted
completion.

Tracked in GitHub issue #51.

## Cross-cutting product gaps

### Mission contracts and verification

The mission composer accepted a 240-character title but no durable specification body or
user-authored verifier policy. The single-task strategy generated generic acceptance tests and an
artifact-only verifier. ECorp did not run the application tests or browser flows that the operator
later executed.

Tracked in GitHub issue #52.

### Portable deliverables

The successful Incident Command result remained as untracked application files in a runner-local
worktree. The accepted artifact contained provider metadata and file hashes, not a source archive,
patch, commit, branch integration, or pull request.

Tracked in GitHub issue #53.

## Operational backlog updates

Created and added to the **ECorp Build** project:

- **In Progress:** #49 Reject accepted completion after a stop-stage budget breaker
- **In Progress:** #51 Isolate provider-local configuration and bridge external CLI permissions
  through ECorp
- **Todo:** #50 Make budget-suspended missions recoverable with an authorized budget revision
- **Todo:** #52 Let operators define rich mission contracts and verifier policies
- **Todo:** #53 Export merge-ready application deliverables from preserved mission worktrees

- **Todo:** #56 Fan out aggregate mission, requester, and Corp hard budget breakers to every active
  run in scope

Issue #27 received an internal-dogfood comment that explicitly preserves its three-real-team
acceptance requirement.

## Local code changes from this pass

- Honor external repository, base-ref, and runner-workspace settings in the local launcher.
- Start Claude Code without user plugins, MCP servers, browser integration, skills, hooks, or
  auto-memory.
- Preserve provider permission checks; the durable ECorp bridge remains tracked in #51.
- Emit unique Codex API-call usage increments while the turn is active.
- Fence progress and accepted completion after hard breaker stages.
- Prevent hard-breaker failures from automatically launching another attempt.
- Prevent action approvals and manual verification from completing a hard-stopped run.
- Recheck current budget and loop metrics under the run lock so an approval or completion cannot
  win the transaction gap before the breaker projection updates.
- Add a deterministic late-completion breaker regression scenario.
- Add a Codex app-server scenario that streams multiple usage updates before attempting completion.

## Validation completed

- Full live stack start against an external scenario repository.
- Browser mission creation, plan inspection, dispatch, second-operator approval, live steering,
  and application workflow checks.
- `cargo fmt --check`
- targeted `crony-store` hard-breaker unit test
- targeted Codex usage unit test
- targeted Claude isolation argument unit test
- targeted OpenCode plugin-isolation argument unit test
- `node --check scripts/fake-agent.mjs`
- `node --check scripts/fake-codex-app-server.mjs`
- `node --check tools/e2e_budgets.mjs`
- `node --check tools/e2e_codex.mjs`
- fresh `node tools/e2e_budgets.mjs` run through the complete stack with source-repository and
  runner-workspace paths containing spaces
- fresh `node tools/e2e_codex.mjs` run proving active-turn usage reaches a stop-stage breaker before
  accepted artifact or completion
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `pnpm build:web`
- `pnpm lint:web`
- full `pnpm check`

The first `pnpm build:web` attempt was blocked before the script by registry TLS failures during
pnpm 11's implicit dependency-status verification. Re-running with
`PNPM_CONFIG_VERIFY_DEPS_BEFORE_RUN=false` used the already installed locked dependencies and
completed both the production build and lint.
