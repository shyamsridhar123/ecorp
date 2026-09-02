# Rich mission contracts and verifier-policy validation

Date: September 2, 2026

This record covers GitHub issue #52 on branch
`codex/issue-52-rich-mission-contracts`, based on remote `main` commit
`bea5f6be2ecd674e9b8cef16a598fe7efae57890`.

No hosted GitHub Actions result was used because the account's monthly Actions credits were
exhausted. All evidence below came from the local Windows stack.

## Implemented authority

- Missions persist a normalized long-form description and specification version.
- Tasks persist a contract version.
- Mission creation accepts an operator-authored contract overlay containing objective, expected
  output, acceptance criteria, allowed tools, prohibited actions, references, and write scope.
- Mission creation and the factory controller accept typed verifier policies for artifact, file,
  command, test, JSON-schema, screenshot, human-approval, and independent-review gates.
- The mission description is delivered to each planned task alongside its task-specific objective.
- The browser contains a structured contract editor, typed verifier editor, human-readable exact
  completion-plan preview, version display, revision form, and revision history.
- `redispatch` revisions are limited to ready missions before their first run.
- `resume` revisions require the latest terminal preserved non-stop provider/worktree checkpoint
  and cannot widen source, secret, provider, budget, deliverable, tool, write, or prohibition
  authority.
- Revisions are actor-attributed, versioned, exactly idempotent, room-authorized, and never start
  execution implicitly.
- Factory issue bodies become durable mission descriptions, and
  `--verification-policy-file` is preserved through claim, materialization, and recovery.

## Local quality gate

The repository gate passed:

```text
node tools/check_migrations.mjs
  migration_count: 29
  latest_version: 29
  immutable_checksums: true

cargo fmt --check
  passed

cargo clippy --workspace --all-targets -- -D warnings
  passed

cargo test --workspace
  97 passed, 0 failed

pnpm build:web
  passed

pnpm lint:web
  passed

git diff --check
  passed
```

## API, runner, worktree, and verifier evidence

`node tools/e2e_mission_contracts.mjs` ran against PostgreSQL, the real Axum server, the outbound
runner, the deterministic process adapter, the protocol-faithful Codex fixture, isolated Git
worktrees, and the normal snapshot/decision APIs.

It proved:

1. A durable specification, repository file reference, issue URL, approved context source, custom
   contract, and six-check verifier policy survived creation and planning.
2. Artifact, file, command, test, JSON-schema, and screenshot checks all passed on the runner.
3. Alice, the requester, received HTTP 403 for an independent-review decision; Bob completed it.
4. A separate non-zero application test produced `verification_failed`, no accepted
   `run.completed` event, and a failed mission despite valid provider evidence.
5. A separately authored owner/admin human-approval gate rejected Bob's member role and completed
   only after Alice's owner decision.
6. A pre-run revision reached contract/specification version 2, replayed the same idempotency key,
   rejected Bob's member role, rejected replay after Alice's room membership was removed, rejected
   a stale version, and completed only after a separate dispatch.
7. A preserved Codex failure rejected a widened tool set, accepted a bounded resume revision,
   replayed exactly once, and completed only after a separate resume while retaining the same
   provider session and worktree. A verifier read the fixture's captured resume prompt and proved
   that both the revised mission description and task objective reached the provider.

The generated report was:

```text
rich creation: completed; 6/6 verifier kinds; requester decision 403
failing test: failed; completion events 0
human approval: member decision 403; owner decision completed
redispatch revision: version 2; member 403; removed-room replay 403; stale version 400
resume revision: version 2; widening 400; same provider session true; same worktree true; file and prompt tests passed
```

## Governed factory evidence

`node tools/e2e_factory_controller.mjs` passed its complete controller regression after adding the
explicit policy lane. The first governed issue:

- preserved the complete issue body as the mission description;
- persisted the explicit artifact and `result.md` verifier policy;
- materialized exactly one mission, task, run, and commit/branch deliverable;
- recovered the same mission on replay; and
- reached factory state `verified` without enabling auto-merge.

The broader controller regression also retained pagination-safe recovery beyond the 500-item
snapshot boundary, lease renewal, source/repository fencing, dependency revalidation, timeout
handling, verification failure, independent review, and terminal failure behavior.

## Browser evidence

Bundled Chromium exercised the real Vite client against the live server and runner at desktop
`1440x1000` and mobile `390x844`.

The browser:

- used a dark CRT arcade shell, animated sprite office, sharp pixel geometry, and one cyan primary
  action color;
- exposed mission authoring as three focused screens: Mission, Loadout, and Win conditions;
- collapsed the composer to one **New mission** control after creation so the mission log remained
  primary;
- authored a title, durable specification, objective, expected output, acceptance criteria,
  references, allowed tools, prohibitions, and write scope;
- added an explicit test check and independent-review gate;
- created a paused mission plan;
- expanded the generated task and rendered the exact completion plan;
- saved a pre-dispatch revision;
- rendered specification version 2, contract version 2, and revision history;
- emitted no console or page errors; and
- had no mobile horizontal overflow in either the collapsed mission log or expanded Win conditions
  screen (`scrollWidth = innerWidth = 390`).

Screenshots and machine-readable reports were written below the ignored
`output/contract-tests/` directory.
