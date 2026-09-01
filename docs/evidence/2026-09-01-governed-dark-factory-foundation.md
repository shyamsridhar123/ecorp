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

- work item `f3c86a7b-9891-4f5a-bf65-d0bf4b869535`
- mission `ba31e540-3ce8-4bcf-b19a-78cc6cb18d90`
- task `a75aac5b-e872-4964-934e-0a5c6d7db7a5`
- run `2c9afb29-4a3f-49d5-9e94-6acfdbacd648`
- concurrent claim, renewal, and materialization requests collapsed to one effect
- the server restarted between claim and renewal
- guest and cross-Corp claims were rejected
- stale versions, stale tokens, and an invalid verified-to-running regression were rejected
- a wider write scope and budget than the persisted factory policy were rejected
- undeclared tools and secret references were rejected
- `verified` was rejected while the linked mission was still running
- `published` was rejected because no dedicated publication operation has run
- an expired pre-materialization `blocked` claim was reclaimed by a second operator, returned to
  `claimed`, and materialized without creating a second work item
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
- work item `5c185cf6-dad8-469f-a31d-b498cbf42620`
- mission `4b3bffc7-4cbf-479f-994e-c4acc2d74c22`
- run `4370149e-9339-4239-b6bc-c4f72cafa6cc`
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
- work item `7c5ee5d2-9798-416f-8342-23638ac78ba5`
- mission `632502ac-cfb6-4d6e-9b5c-a9f7439cbe6a`
- run `432ae234-3dd0-4da5-8949-f7265294e010`
- the failed status update persisted a `blocked` factory state and failure detail
- no run launched before the external status effect succeeded
- retry reused the existing mission, moved the Project item to `In Progress`, launched one run,
  and reached `verified`

Terminal mission failure:

- work item `cc48e57a-9052-46ea-a736-aa525b7eea49`
- mission `faa9415f-fcba-4038-9941-fb7b449b497c`
- the mission exhausted its bounded retries and ended `failed`
- the next controller pass persisted factory state `failed` and returned an error rather than
  reporting a healthy running factory item

Repository routing rejection:

- work item `be7f9cd6-2871-4c57-a2d2-dea4d0c98473`
- mission `1613c2ef-7f95-43f0-94ae-6bc39bbd128d`
- required checkout `acme/widget @ HEAD`
- the connected ECorp runner was rejected before a run was created
- the factory work item durably entered `blocked`

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
