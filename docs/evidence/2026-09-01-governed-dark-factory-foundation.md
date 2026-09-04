# Governed dark-factory foundation validation — September 1, 2026

## Scope

This record covers GitHub issues #59 and #60: durable, fenced GitHub Project issue claims plus the
first trusted Project-to-mission controller. GitHub Project `ECorp Build` #3 remains the
operational source of truth. No markdown backlog was used for status or sequencing.

The implemented path is:

```text
Project issue eligibility
  -> dry run
  -> durable factory claim
  -> atomic mission/task materialization
  -> Project status update
  -> runner dispatch
  -> evidence-gated completion
  -> verified factory state
```

Pull-request publication is tracked separately in #61. This validation did not create or merge a
pull request and did not deploy anything.

## Durable claim and mission evidence

`node tools/e2e_factory_claims.mjs` passed against the complete local Postgres, server, runner, and
real child-process stack.

Latest result:

- work item `7a9964a9-6fc8-4e83-996e-f4a3c6d09d60`
- mission `6e18f720-2a5c-4c18-afdc-f3f638a4fafa`
- task `e6285cd2-3dd1-44eb-8daf-eb4cda3a1a47`
- run `cc803b41-7cae-4a1d-9d4a-1be98055efee`
- concurrent claim, renewal, and materialization requests collapsed to one effect
- mixed-case GitHub owner and repository identities resolved to the same canonical work item
- the server restarted between claim and renewal
- guest and cross-Corp claims were rejected
- stale versions, stale tokens, and an invalid verified-to-running regression were rejected
- a wider write scope and budget than the persisted factory policy were rejected
- unit coverage proves a task cannot omit a policy-pinned model or reasoning effort and silently
  fall back to provider defaults
- undeclared tools and secret references were rejected
- `verified` was rejected while the linked mission was still running
- `published` was rejected because no dedicated publication operation has run
- an expired pre-materialization `blocked` claim was reclaimed by a second operator, returned to
  `claimed`, and materialized without creating a second work item
- attempted expired reclaims with a wider policy or changed source revision were rejected, while
  the original source and policy snapshots remained unchanged
- the issue contract and write scope reached the persisted task contract
- the one linked mission and run completed
- the factory work item reached `verified`
- claim tokens were absent from snapshots and events
- guests and spectators received neither factory work items nor GitHub source metadata in events

The factory event sequence was:

1. `factory.work_item_claimed`
2. `factory.claim_renewed`
3. `factory.mission_linked`
4. `factory.state_changed` to `running`
5. `factory.state_changed` to `verified`

## Controller evidence

`node tools/e2e_factory_controller.mjs` passed with a deterministic GitHub CLI boundary while
using the real ECorp server, Postgres store, scheduler, runner, child process, verifier, and event
journal.

Successful issue:

- Project item `PVTI_FAKE_FACTORY_9001`
- work item `28d7c9d3-23e0-43c3-94b5-d28fc9969968`
- mission `52f23778-eb73-4c5f-bb3b-3f1f6ce4fec2`
- run `d3642bb7-21f8-4048-abd6-ceb80f338700`
- dry run changed neither ECorp nor GitHub state
- Project status changed from `Todo` to `In Progress` only after mission linkage
- replay recovered the same work item, mission, and run
- controller lease was renewed before the GitHub status effect and revalidated again before launch
- the task persisted `shyamsridhar123/ecorp @ HEAD`, matching the runner's advertised checkout
- mixed-case repository input was canonicalized through claim, materialization, and runner routing
- final factory state was `verified`
- the next unqualified controller pass selected Todo issue `9003` instead of repeatedly selecting
  the already-verified item

Injected GitHub Project failure:

- Project item `PVTI_FAKE_FACTORY_9002`
- work item `34902dcc-6191-4c25-b02e-3b8a198fdc87`
- mission `99eaf716-d701-4e38-9702-f0745ea57b7f`
- run `eb933d0d-7bfb-441e-bfcd-da35f17dd4fc`
- the failed status update persisted a `blocked` factory state and failure detail
- multi-line GitHub CLI stderr was normalized to bounded single-line failure text
- no run launched before the external status effect succeeded
- retry reused the existing mission, moved the Project item to `In Progress`, launched one run,
  and reached `verified`

Stalled GitHub Project mutation:

- work item `7d1a6f87-87a7-4f6e-bc09-93f2388f5c06`
- mission `d2381ef5-feb2-476a-81bb-39ee4bcf93e8`
- the injected Project edit exceeded its 1.5-second test deadline and was killed
- the Project item remained `Todo`, the factory item entered `blocked`, and no run was created

Terminal mission failure:

- work item `185ef368-7f0e-4c54-8c46-ce86669a4d9b`
- mission `1bd79ae0-9af2-4225-ae7b-58330f8e6a0c`
- the mission exhausted its bounded retries and ended `failed`
- the next controller pass persisted factory state `failed` and returned an error rather than
reporting a healthy running factory item

Verification failure:

- work item `22b44e3a-076c-43a0-85ec-9060b26200ca`
- mission `1129b617-1581-40bc-8930-37c150f09a64`
- run `1d53e927-ea5d-4651-a4ef-ec2192897865`
- the mission ended `failed` because its verifier rejected evidence
- the task and factory work item remained explicitly `verification_failed`

Independent verification:

- work item `079ac334-88f9-45c9-8fae-130294df5ab2`
- mission `01d6b619-979b-4ff8-b320-1aac30558694`
- run `b2d054d5-066f-4112-b629-ef13807e91b7`
- the factory projection reached `awaiting_approval`
- the requester received `403` when attempting to self-review
- Bob approved the evidence and the next controller pass reached `verified`

Repository routing rejection:

- work item `a54b7423-b741-4d5f-b0b2-5fcceed60336`
- mission `0e1a9394-fd01-4460-829d-0ee8bb46aef9`
- required checkout `acme/widget @ HEAD`
- the connected ECorp runner was rejected before a run was created
- the factory work item durably entered `blocked`

Source changed before Project mutation:

- work item `65340633-84b4-40f0-8bbe-dd2e4f998c02`
- the issue revision changed and `factory:ready` was removed after mission materialization
- the controller durably entered `blocked`
- the Project item remained `Todo`
- no GitHub Project mutation and no run occurred

Dependency reopened before launch:

- work item `2dba7708-4984-44d0-8f65-02d922411d18`
- dependency issue `#9009` reopened after the Project item reached `In Progress`
- the controller re-evaluated dependency eligibility, durably entered `blocked`, and created no run

## Live GitHub Project canary

GitHub issue #64 was created as a bounded live canary with `factory:ready`, added to Project #3 as
`Todo`, and run through the controller.

- Project item `PVTI_lAHOBwBdFs4Bh3Clzg468GE`
- work item `d93a2990-49cc-4fe3-b3ee-5c7444cd3579`
- mission `c20fd131-db31-4f94-b423-5f50e83b41b8`
- run `e1c81ef0-58b4-4213-95aa-37f2ddced9a3`
- Project status moved to `In Progress` after durable mission linkage
- the deterministic child process completed with accepted verification
- a repeated controller invocation recovered the same work item, mission, and run
- the canary was then marked `Done` and closed with the evidence attached

The first replay attempt found a real defect: rebuilding policy from the changed Project status
violated the original idempotency request. Recovery now reuses the persisted policy snapshot, and
the repeated live invocation passed.

The pre-landing adversarial review then found and reproduced broader-policy materialization,
forged `verified` state, blocked pre-materialization recovery, verified-item queue starvation,
terminal mission state drift, and expired-controller failover gaps. Each reproduced case now has a
deterministic regression check.

GitHub's review of PR #65 found two additional P1 gaps. The controller now renews its lease before
the Project mutation and revalidates it before launch. Factory materialization now persists the
claimed repository and base ref into each task, runners advertise their normalized GitHub checkout,
and scheduling rejects a mismatched checkout before creating a run.

A second review of the updated head found two more P1 gaps. Factory materialization now rejects
omitted policy-pinned model or reasoning values. The controller now re-fetches source revision,
state, labels, Project content, and blockers immediately before both the Project mutation and
mission launch; a change is persisted as `blocked` before any later effect.

A third review found that an expired pre-materialization reclaim could replace the original source
and policy snapshots. Reclaims now require exact snapshot equality and update only ownership,
fencing, lease, state, and failure detail.

A fourth review found that case variants could bypass source uniqueness and that verifier failures
were collapsed into execution failures. GitHub identities are now canonicalized before locking and
lookup, and the controller derives `verification_failed` from authoritative task verification
state.

A fifth review found that artifact-only provider results could verify without an evidence decision
and that a stalled GitHub mutation could outlive its lease. Provider-backed factory tasks now carry
a manual verification gate, factory state exposes `awaiting_approval`, GitHub subprocesses are
bounded, and the controller renews again immediately before each effect.

## September 4, 2026 verification-recovery review hardening

This narrow update records the regression coverage added during PR #143 review. It does not claim a
new full repository gate or a completed recovery-to-publication remote-effect run.

- Runner unit regression
  `verifier_snapshot_is_physical_isolated_and_excludes_git_control` creates a physical snapshot,
  verifies that `.git` is excluded, copies untracked and ignored source files, mutates the snapshot
  without changing the preserved worktree, and verifies cleanup when the snapshot is dropped.
- `tools/e2e_factory_verification_recovery.mjs` adds a direct-command verifier side effect. The
  original run writes the source file once; verifier-only recovery runs the same command in the
  ephemeral snapshot, and the assertion confirms the preserved source still contains the original
  value. The runner fingerprints that preserved source after verification.
- Store unit regression `publication_source_revision_links_completed_recovery` selects the reviewed
  source revision and recovery ID for a completed recovery, and falls back to the original claimed
  revision with a null recovery ID when publication does not follow recovery.
- Publication provenance retains both `source_issue.claimed_revision` and the effective
  `source_issue.revision`, plus `source_issue.recovery_id` when applicable. Revalidation compares
  those fields with the currently selected verified recovery before allowing later effects.

## Browser evidence

The live web application was exercised at `http://127.0.0.1:5187` in Chromium.

- the `DARK FACTORY / 02` panel rendered the verified issue, Project, mission, controller, lease,
  source revision, and aggregate version
- the page contained no `claim_token`
- Eve received zero factory work items and no GitHub source metadata in visible factory events
- no browser console or page errors were observed
- desktop screenshot: `output/playwright/factory-panel-desktop.png`
- 390-pixel mobile screenshot: `output/playwright/factory-panel-mobile.png`
- routed-contract screenshot: `output/playwright/factory-repository-routing-desktop.png`
- the expanded task contract displayed `acme/widget @ HEAD` for the rejected mismatch scenario
- mobile `scrollWidth` equaled `innerWidth` at 390 pixels

## Remaining factory scope

- #61 owns verifier-gated, idempotent pull-request publication.
- #62 owns multiple enterprise scenarios plus publication and restart/failure evidence across the
  complete issue-to-PR path.
- #48, #49, #50, #51, #52, #53, #56, and #58 remain explicit product dependencies rather than
  hidden assumptions.
