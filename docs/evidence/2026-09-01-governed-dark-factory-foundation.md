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

- work item `6fdab92e-7d8b-4c87-995f-e0839fd19621`
- mission `c8722026-5b1d-4052-aa61-134b29e43838`
- task `3ffe55e8-fba6-4975-87b6-2a09ebda47e7`
- run `02253fce-6824-467e-b673-281b3c1a6544`
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
- work item `744c3caa-d9a2-432a-8140-50f97417f2fa`
- mission `f2581b14-2248-4657-a93a-78e06d6ada31`
- run `a5adc4bf-30be-46c6-90f9-25540dedfe0b`
- dry run changed neither ECorp nor GitHub state
- Project status changed from `Todo` to `In Progress` only after mission linkage
- replay recovered the same work item, mission, and run
- final factory state was `verified`
- the next unqualified controller pass selected Todo issue `9003` instead of repeatedly selecting
  the already-verified item

Injected GitHub Project failure:

- Project item `PVTI_FAKE_FACTORY_9002`
- work item `6b586150-aacd-49b1-b317-cf5604e3d708`
- mission `c0c190c4-b60a-4e64-9982-3d68c68ec3eb`
- run `777b6a6b-e54a-41c0-8caa-dd7b8bc42496`
- the failed status update persisted a `blocked` factory state and failure detail
- no run launched before the external status effect succeeded
- retry reused the existing mission, moved the Project item to `In Progress`, launched one run,
  and reached `verified`

Terminal mission failure:

- work item `6d178c48-9c90-4d5c-a1d4-d060a25c20d1`
- mission `165b3003-3c18-45c3-a818-43f951cc2b6b`
- the mission exhausted its bounded retries and ended `failed`
- the next controller pass persisted factory state `failed` and returned an error rather than
  reporting a healthy running factory item

Expired controller failover:

- work item `b6a8979e-b331-479e-b3a9-c52749dddda9`
- mission `47a1c02a-4e22-41cb-8235-d041f6af0e7e`
- run `d27c6560-fb8f-44b6-92eb-60e7740e61ff`
- Bob reclaimed Alice's expired controller lease
- the replacement controller reused the same work item, mission, and run and advanced it to
  `verified`

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

## Browser evidence

The live web application was exercised at `http://127.0.0.1:5187` in Chromium.

- the `DARK FACTORY / 02` panel rendered the verified issue, Project, mission, controller, lease,
  source revision, and aggregate version
- the page contained no `claim_token`
- Eve received zero factory work items and no GitHub source metadata in visible factory events
- no browser console or page errors were observed
- desktop screenshot: `output/playwright/factory-panel-desktop.png`
- 390-pixel mobile screenshot: `output/playwright/factory-panel-mobile.png`
- mobile `scrollWidth` equaled `innerWidth` at 390 pixels

## Remaining factory scope

- #61 owns verifier-gated, idempotent pull-request publication.
- #62 owns multiple enterprise scenarios plus publication and restart/failure evidence across the
  complete issue-to-PR path.
- #48, #49, #50, #51, #52, #53, #56, and #58 remain explicit product dependencies rather than
  hidden assumptions.
