# Idempotent pull-request publication validation

**Date:** September 2, 2026
**Initial implementation head:** `988434ef348f21e7d68400418beace9faa4cfb0d`
**Review-hardening head:** `e9eaf24d23d8a10c0c80d076d96e91068b61a113`
**Final local validation head:** `ae2df7c50d778f0de56fcb694711044d7cfc9bec`
**Resolved-base fix head:** `59638db`
**Retry and base-branch guard head:** `aa55790`
**Preflight and actor-handoff head:** `6f4c57c`
**Completed-retry head:** `173e20e`
**PR-content and deliverable-pin head:** `86472e2`
**Review-blocker closure head:** `5afe09f6abdd4ffa3d25f2365539463d0d51cd23`
**Room-membership closure head:** `b631bd9e2f0ad8321631c046b38a549da64fec6e`
**Title-normalization closure head:** `ba20a8347a67c1e1ab6e4b188ad3602b131654c1`
**Context and base-ref closure head:** `7e8f13f24e5a3f4e923695b2ac00d369a99e15f7`
**Metadata lookup closure head:** `75bfc0de71233d1c79363c2d2c642304bda0b960`
**URL identity closure head:** `70b287478d060fc08bafee4f33f28c393fb7c5dd`
**Final ordering/default head:** `a49705e9550dd758e855a7407f35c68407742608`
**Integrated gate head:** `926fa7705f43f512c0aba75bd2f0042065fc03d4`
**Stacked base:** `4ad38c5f0f6c99bad9807cc487859af0cebd3c3b`

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

The Rust workspace ran 87 non-documentation unit tests with no failures.

## Deterministic publication E2E

`tools/e2e_factory_publication.mjs` ran against an isolated database, server port, runner workspace,
object store, test-owned PID file, bare Git remote, and fake GitHub state.

The final exact-head run recorded:

```json
{
  "base_branch_collision_rejected": true,
  "invalid_git_branches_rejected_before_start": true,
  "custom_title_normalized": true,
  "mixed_case_repository_normalized": true,
  "mixed_case_pull_request_url_accepted": true,
  "implicit_authorization_retry_stable": true,
  "cross_publisher_default_start_recovery": true,
  "body_file_crlf_normalized": true,
  "actor_handoff_authorization_distinct": true,
  "published_retry_after_base_move": true,
  "exact_publication_context_lookup": true,
  "cross_room_publication_context_denial": 404,
  "cross_room_publication_start_rejection": 403,
  "cross_room_publication_recovery_rejection": 403,
  "cross_room_publication_status_denial": 404,
  "pre_branch_room_membership_renewal_rejection": 403,
  "pre_pull_request_room_membership_renewal_rejection": 403,
  "pre_project_room_membership_renewal_rejection": 403,
  "pr_revalidated_after_project_renewal": true,
  "bounded_snapshot_work_item_and_deliverable_absent": true,
  "published_retry_outside_bounded_snapshot": true,
  "exact_project_item_lookup": true,
  "exact_project_field_lookup": true,
  "project_item_count_during_publication": 1003,
  "project_field_count_during_publication": 32,
  "publication_attempts": 10,
  "pull_request_number": 41,
  "pull_request_head_sha": "ab6117ba0c702053641ab34ca34fa5e74aa88ccb",
  "pull_request_head_repository_owner": "shyamsridhar123",
  "fork_pull_request_rejected": true,
  "unauthorized_pr_content_rejected": true,
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

A separate verified factory item explicitly allowed publication branch `main`. The read-only
preflight rejected it before creating any publication row and left the factory item verified, so a
corrected branch remained usable. A later CLI invocation on that corrected branch omitted
`--authorization-id`, crashed immediately after durable start, restarted the server, and recovered
through a duplicate invocation using a CRLF body file with a trailing newline. The remote `main`
object did not change, no pull request was created, the body canonicalized identically on client and
server, and the generated authorization identity remained stable across crash, restart, and retry.

The main publication then expired an Alice-owned attempt, promoted Bob to manager for recovery, and
proved Bob's attempt received a different authorization ID whose snapshot named Bob. Bob was
returned to member afterward and Alice completed the remaining recovery path.

After publication completed, the fake remote `main` branch advanced to another commit. A duplicate
publisher invocation returned the same persisted publication without remote preflight, PR creation,
or Project mutation.

The review-blocker closure run inserted 501 newer factory work items and 501 newer source
deliverables after the selected verified result. The bounded shared snapshot no longer contained
either requested object, while the exact publication-context endpoint returned the work item and
deliverable and publication completed normally. After completion, the harness inserted 501 newer
publication aggregates, proved the target publication was also outside the shared snapshot, and
replayed the persisted published result through the same exact context.

The first publisher host crashed after durable start. Recovery used a different publisher ID,
authorization reason, and lease duration without an idempotency-key reuse conflict because the
default key fingerprints the complete normalized request. Separate pre-start regressions passed
`ecorp/foo//bar` and `ecorp/foo.lock`; both failed `git check-ref-format --branch`, created no
publication row, and left the verified work item correctable.

Before Project movement, the fake Project was expanded to 1,003 items with the authorized item after
1,001 fillers. The fake CLI enforced the `item-list --limit` bound. Publication made no additional
item-list call, loaded the exact stored node ID through GraphQL, verified its Project and Status
field identity, and completed the one `In Progress -> In Review` transition.

The Project also exposed 32 fields with Status ordered after 31 fillers. The fake CLI enforced the
30-field default for `field-list`; publication made no additional field-list call, queried Status
directly by name from the exact Project node, and used its complete single-select option set.

A same-Corp manager fixture was created in a separate room with no membership in the publication
mission room. A direct start and an expired-lease recovery both returned `403`. While an authorized
publisher attempt was active, the fixture requested exact publication status by the known work-item
UUID; the endpoint returned `404`, and its response omitted the pull-request body, authorization
reason and ID, publisher ID, and a persisted failure-detail canary.

The same actor's publication-context read also returned `404`. The response omitted the exact work
item ID, source issue URL/title, claim owner, policy JSON, and a persisted work-item failure canary,
proving the initial context lookup is room-scoped rather than only filtering its nested publication
and deliverables.

The authorized publisher was then removed from the mission room after attempt start at each external
effect boundary. The pre-branch, pre-pull-request, and pre-Project renewals all returned `403`.
The remote branch remained absent in the first case, no target-repository pull request existed in the
second, and the Project item stayed `In Progress` in the third. Membership was restored only after
each denial so the rest of the recovery harness could continue.

The collision-recovery fixture supplied a pull-request title with leading and trailing whitespace.
The CLI normalized it before deriving the start idempotency key and before sending the durable
request, then reached a test crash immediately after exact plan validation. Recovery retained the
trimmed title and created no remote pull request or Project mutation for that fixture. The same
invocation supplied `ShyamSridhar123/ECorp`; the plan and durable publication both retained the
canonical `shyamsridhar123/ecorp`.

The fake GitHub API returned the canonical pull-request URL
`https://github.com/ShyamSridhar123/ECorp/pull/41`. URL validation required the exact GitHub
scheme/host, pull path, and number while comparing owner/repository components
case-insensitively. The URL was accepted, persisted, and recovered.

The controller was also invoked without `--publication-base-ref`. Its persisted policy base was
`HEAD`, matching the source revision selection instead of assuming `main`.

Before authorized PR creation, the harness injected a same-repository, same-branch, exact-SHA pull
request whose title and body differed from the persisted plan. The publisher ignored it, the fake
GitHub create operation refused the duplicate head, and ECorp retained `branch_pushed` without a PR
or Project transition. Removing the unauthorized PR allowed one exact authorized PR to be created.
Retry selection is pinned to the publication's persisted deliverable ID.

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

The review-blocker closure changed only CLI, API, persistence, and deterministic test behavior; no
web UI source changed, so the existing browser rendering remains representative.

## Review-blocker closure validation

On September 2, 2026, commit `5afe09f6abdd4ffa3d25f2365539463d0d51cd23`
passed:

```text
node tools/check_migrations.mjs
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm build:web
pnpm lint:web
git diff --check
tools/e2e_factory_controller.mjs
tools/e2e_factory_publication.mjs
```

Both focused E2Es used a fresh isolated PostgreSQL database and test-owned process/artifact/worktree
directories. Their servers, runners, web process, ports, and database were removed afterward.
All four review threads were replied to and resolved.

The later room-membership implementation commit
`b631bd9e2f0ad8321631c046b38a549da64fec6e` passed the same repository gates plus fresh isolated
controller and publication E2Es. No web UI source changed.

The title-normalization implementation commit
`ba20a8347a67c1e1ab6e4b188ad3602b131654c1` passed the same repository gates and another fresh
exact-head run of both focused E2Es.

The context/base-ref implementation commit
`7e8f13f24e5a3f4e923695b2ac00d369a99e15f7` passed the same repository gates and fresh isolated
runs of both focused E2Es. The controller report recorded
`non_branch_publication_base_rejected_before_claim: true`; the invalid `refs/tags/v1` invocation
created no factory work item and made no Project mutation.

The metadata implementation commit `75bfc0de71233d1c79363c2d2c642304bda0b960`
added exact Status-field lookup, target repository canonicalization, and control-character ref
rejection. The controller report recorded
`control_character_publication_base_policy_rejected: 400`.

After normally merging stacked base `7d151e1914d522a2c691e320f6b909918b3ad731`,
clippy exposed the new deliverable helper's eighth argument. Commit
`41b6fc568a3112bda8115fc22c37b5dd63b91f99` grouped its temporary paths without changing
behavior. Full gates and fresh exact-head controller/publication E2Es then passed.

The final dependency head `95c8ff94cf3227448353e6dfb0b72d13a2b1a677` was merged normally.
The conflict resolution retained both its shared path/write-scope safety grammar and publication's
portable Git bundle path grouping. URL identity commit
`70b287478d060fc08bafee4f33f28c393fb7c5dd` then passed all repository gates and fresh isolated
controller/publication E2Es on the merged tree.

The final base `4ad38c5f0f6c99bad9807cc487859af0cebd3c3b` was then merged normally.
For final ordering coverage, the fake GitHub API closed PR #41 on the second PR lookup: the first
lookup occurred before Project-stage renewal and the second immediately afterward. Publication
rejected the changed durable PR, made no Project edit, restored the fixture, and then completed
normally. Commit `a49705e9550dd758e855a7407f35c68407742608` and merged head
`926fa7705f43f512c0aba75bd2f0042065fc03d4` passed all gates and both focused E2Es.

GitHub Actions run `33618313806` could not start any of its six jobs. Every job had zero steps,
runner ID `0`, and the account payment/spending-limit annotation. This is an external CI block, not
a repository test failure.

All test-owned processes, ports, and containers were stopped after validation. Failed-run worktrees
were preserved according to the repository safety contract.
