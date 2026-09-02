# Idempotent pull-request publication validation

**Date:** September 2, 2026  
**Initial implementation head:** `988434ef348f21e7d68400418beace9faa4cfb0d`
**Review-hardening head:** `e9eaf24d23d8a10c0c80d076d96e91068b61a113`
**Final local validation head:** `ae2df7c50d778f0de56fcb694711044d7cfc9bec`
**Resolved-base fix head:** `59638db`
**Stacked base:** `be0560f7e8f39dc70973c762890b88edbe5c2210`

## Scope

This validation covers GitHub issue #61's trusted issue-to-branch-to-pull-request publication
boundary. GitHub behavior is deterministic and fake; server, PostgreSQL, runner, worktree, Git
commit, portable bundle, bare remote, CLI, API, React UI, restart, and browser behavior are real.
It does not merge, enable auto-merge, deploy, or claim a live GitHub pull request was created.

## Fresh-database and repository gates

A new PostgreSQL 16 database applied all 26 migrations. Migration 0024 created:

- `pull_request_publications`
- `pull_request_publication_attempts`
- `pull_request_publication_operations`

Authorization JSON is stored only in the explicit `authorization_snapshot` columns.
Migration 0025 adds the exact pull-request head SHA, head repository owner, and cross-repository
identity needed to reject same-named fork pull requests.
Migration 0026 stores the actual GitHub PR base separately from the authorized symbolic base.

The exact implementation head passed:

```text
node tools/check_migrations.mjs
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm build:web
pnpm lint:web
git diff --check
```

The Rust workspace ran 73 non-documentation unit tests with no failures.

## Deterministic publication E2E

`tools/e2e_factory_publication.mjs` ran against an isolated database, server port, runner workspace,
object store, test-owned PID file, bare Git remote, and fake GitHub state.

The final exact-head run recorded:

```json
{
  "publication_attempts": 8,
  "pull_request_number": 41,
  "pull_request_head_sha": "da8b2c6144334af7f4f5f44be746bb2f5c5b2370",
  "pull_request_head_repository_owner": "shyamsridhar123",
  "fork_pull_request_rejected": true,
  "pull_request_create_calls": 1,
  "publication_base_ref": "HEAD",
  "resolved_pull_request_base_ref": "main",
  "remote_branch_count": 1,
  "project_status": "In Review",
  "project_after_pull_request": true,
  "policy_rejection": 400,
  "role_rejection": 403,
  "corp_rejection": 403,
  "budget_rejection": 400,
  "breaker_rejection": 400,
  "post_start_role_renewal_rejection": 400,
  "post_start_breaker_renewal_rejection": 400,
  "pre_pull_request_budget_renewal_rejection": 400,
  "pre_project_corp_budget_renewal_rejection": 400,
  "credential_non_disclosure": true,
  "auto_merge": false,
  "merge_authorized": false,
  "deployment_authorized": false
}
```

The attempt sequence covered:

1. branch push followed by publisher crash before the local checkpoint;
2. server restart and adoption of the existing exact branch;
3. pull-request creation that succeeded remotely while the fake CLI returned a local failure;
4. recovery and durable pull-request identity;
5. Project transition followed by another publisher crash;
6. concurrent duplicate recovery calls converging on the same publication and pull request.

The hardened run also seeded an open same-named fork PR with a different owner and head SHA. The
publisher ignored it, created one same-repository PR, and persisted the exact verified head. It used
the accepted `HEAD` policy base, verified both the symbolic target and object ID, and used/persisted
the resolved `main` branch for GitHub PR operations. After publication
start, changing the owner to another still-publish-capable role, raising a hard breaker, exhausting
the selected run budget, and exhausting the Corp aggregate budget each caused the next lease
renewal to fail before branch, PR, or Project effects.

The fake GitHub effect log proves the `In Review` transition occurred after a pull request existed.
The persisted factory item and source deliverable ended as `published`. The credential canary was
absent from snapshots, events, publications, attempts, provenance, fake GitHub state, and CLI
output.

## Windows Git-bundle portability

The first Windows run exposed this Git-for-Windows failure for long managed branch refs:

```text
fatal: failed to stat 'refs/heads/crony/task-.../run-...': Filename too long
```

Creating the bundle from `HEAD ^<verified-base>` instead produced a valid 647-byte bundle. The
runner still verifies that the long persisted task branch points at the exact head before creating
the bundle, and the publisher separately verifies the document's source branch provenance.

## Browser verification

The React console loaded against the final isolated stack in the Codex in-app browser with no
console errors. The visible publication card showed:

- target `shyamsridhar123/ecorp` and verified symbolic base `HEAD`;
- exact branch and commit;
- owner authorization and eight retained attempts;
- pull request #41 open for review;
- verified head owner `shyamsridhar123`, exact head SHA, and same-repository identity;
- Project `In Progress -> In Review`; and
- `auto-merge off` plus merge/deploy unauthorized.

Screenshot:

`output/playwright/factory-publication-clean-review.png`

All test-owned processes, ports, and containers were stopped after validation. Failed-run worktrees
were preserved according to the repository safety contract.
