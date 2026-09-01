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

- work item `3ee7fa4e-a1cc-48af-b978-fde73da4874a`
- mission `fea261e9-26ad-46af-8a35-2a41587ee00b`
- task `0095a564-6b72-4df9-89c1-5da1e52038b0`
- run `30f94cc8-2272-453b-b92d-317885aa7559`
- concurrent claim, renewal, and materialization requests collapsed to one effect
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
- work item `d7e8cf79-595d-4c32-9623-a84850c83ba6`
- mission `0c7ed378-e0c6-4fa0-ad5b-51b9f7925b55`
- run `f977b064-ed00-4334-b8b4-aafd7057817c`
- dry run changed neither ECorp nor GitHub state
- Project status changed from `Todo` to `In Progress` only after mission linkage
- replay recovered the same work item, mission, and run
- controller lease was renewed before the GitHub status effect and revalidated again before launch
- the task persisted `shyamsridhar123/ecorp @ HEAD`, matching the runner's advertised checkout
- final factory state was `verified`
- the next unqualified controller pass selected Todo issue `9003` instead of repeatedly selecting
  the already-verified item

Injected GitHub Project failure:

- Project item `PVTI_FAKE_FACTORY_9002`
- work item `1f5e33c7-279a-436b-b268-1fc0120b3957`
- mission `c01730b7-3c56-4c97-b3dc-b6766ab5e8c8`
- run `7af916cb-4181-4589-8729-b69df6a024fb`
- the failed status update persisted a `blocked` factory state and failure detail
- no run launched before the external status effect succeeded
- retry reused the existing mission, moved the Project item to `In Progress`, launched one run,
  and reached `verified`

Terminal mission failure:

- work item `f3c3c4c9-4333-4a53-b783-36ac3c31ecde`
- mission `160efe81-3839-4350-a509-a888f038d04c`
- the mission exhausted its bounded retries and ended `failed`
- the next controller pass persisted factory state `failed` and returned an error rather than
  reporting a healthy running factory item

Repository routing rejection:

- work item `ef22d2d8-9ee8-43c0-afb4-9ac90723e48e`
- mission `c82fcc3b-adf4-41ec-bb7c-4bb379344771`
- required checkout `acme/widget @ HEAD`
- the connected ECorp runner was rejected before a run was created
- the factory work item durably entered `blocked`

Source changed before Project mutation:

- work item `41b53d57-d602-4110-88a3-b375b9622e40`
- the issue revision changed and `factory:ready` was removed after mission materialization
- the controller durably entered `blocked`
- the Project item remained `Todo`
- no GitHub Project mutation and no run occurred

Dependency reopened before launch:

- work item `ca66e3bc-62fe-4d5a-8e90-a657860eba83`
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
