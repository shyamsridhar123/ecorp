# Factory controller context

Date: September 9, 2026. Tracking: #205, under #145 / #63.

## Observed defect and correction

The real #204 acceptance selected a Project #3 work item while the controller
strip showed the unrelated old Project #174 controller and its controls.
`controllers[0]` was not a valid controller-to-work-item relationship.

The client now resolves the controller using the selected item's Corp, Project
owner/number and repository owner/name. GitHub names normalize independently;
Corp and Project identities remain exact. It prefers a live matching controller
and the exact active-item relationship, with deterministic freshness/ID ties.
It never borrows a controller from another scope for an unavailable selection.

Displayed state and all existing pause/resume/reconcile callbacks receive the
same original resolved record, including its ID/version. No new controller,
polling mechanism, backend permission or visual redesign was added.

## Validation

- **32 focused Node tests passed**, including 15 new selection/integration cases.
- Tests cover wrong-first ordering, every scope dimension, identity case,
  active-item/freshness behavior, unavailable selections and stable ID/version.
- Integration tests exercise the production action handler with mocked HTTP and
  inspect the actual component wiring. These are not live action-authorization
  or backend idempotency proofs.
- Production web build and lint passed.

In the actual existing QA browser, selecting lab issue #4 now showed its
Project #3 / enterprise-lab controller, including the paused state deliberately
requested by the operator while #206 recovery is addressed. Selecting an older
published fixture with no matching controller showed **Not Configured**, with no
borrowed Resume/Reconcile controls. Returning to issue #4 restored the Project #3
controller. These checks did not mutate another controller or reset history.

The remaining retained-source recovery failure belongs to #206; correcting the
controller strip does not make the stopped application verified or complete.
No hosted Actions, merge, auto-merge or deployment is claimed.
