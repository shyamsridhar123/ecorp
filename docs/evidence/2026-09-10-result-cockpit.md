# Result-first work-item cockpit

September 10, 2026. This is a bounded frontend slice of [#145](https://github.com/shyamsridhar123/ecorp/issues/145), not completion of that issue or the dark-factory umbrella.

## User outcome

An operator can find the next action beside the selected work item rather than hunt through diagnostics and historical runs:

- Completed Factory work exposes its recorded pull request and exact source bundle in both Missions and Factory.
- Work awaiting a decision links to the existing scoped review or action controls. It does not create another approval.
- Ready work uses the existing launch action; running work links to its task owners.
- A narrow viewport has a compact work-item selector. A deep-linked older mission remains an explicit option even when it is outside the recent eight.
- Detailed delivery metadata and intake diagnostics remain available through disclosures. The office remains a projection, with its existing assets and behavior unchanged.

## Data and authorization

Mission origin and result discovery reuse the existing authenticated, room-scoped GET endpoints. The origin lookup is shared by the heading and result. Publication discovery does not infer absence from the bounded recent snapshot.

The client retains a small consistency-checked result: exact Corp, mission, work item, repository, publication, run, artifact and deliverable identities; recorded commit/digest echoes; and a canonical GitHub PR link. It does not retain publication credentials or an authorization snapshot, reimplement signature verification, or grant permission to publish.

Changing the viewer, room, work item, request generation or relevant revision immediately invalidates old result data. Abort and timeout handling suppress late responses. A failed first-hop lookup has an explicit **Refresh work context** action rather than requiring a reload.

The existing pinned evidence run remains authoritative for review controls. Inspecting a historical run does not silently retarget it to the published result. **Go to delivered result** performs that explicit selection and restores keyboard focus. A missing review record is described as unavailable, not as proof that no review occurred.

## Verification evidence and limits

The implementation was exercised against the retained VendorDesk mission, already verified and published through ECorp in the separate #216 acceptance:

1. Missions and Factory displayed the same recorded enterprise-lab PR #6, exact commit and source bundle.
2. Clicking **Open pull request #6** reached that actual GitHub PR in the browser.
3. Selecting the older cancelled run retained its evidence and withheld the newer result link until **Go to delivered result** was explicitly selected.
4. A temporary browser-only failure of the mission-origin GET showed an unavailable state. One manual refresh restored the real result without changing the pinned run. The test fault was removed afterward.
5. The guest view did not retain the previous collaborator's PR link; returning to the member required fresh scoped reads.
6. At 390px the document width was 375px, without horizontal overflow in the tested Missions and Factory views. The tested result text/button contrast ratios ranged from 7.4:1 to 14.74:1. Keyboard focus and collapsed delivery details were inspected. These observations are not a full WCAG certification.

Node coverage exercises the actual result components, hooks and App wiring, in addition to the reader's malformed-data and asynchronous-isolation matrices. Existing evidence-selection, origin, controller, runtime, composer and recovery tests remain in the release gate. Backend source, migrations and provider code are unchanged; their workspace checks are separately retained and hash-bound. Exact release results and interruption history are recorded in the PR, without relying on hosted Actions.

The browser checks inspect and navigate existing work. They do not represent a new application build, new provider execution, new approval, or new publication. A later machine/runtime interruption does not erase those observations or establish that a subsequently restarted stack is healthy; restart state must be verified separately.

## Remaining #145 work

This slice does **not** provide a browser-owned publisher credential, a new publisher executor, or an inferred application URL. Browser-requested publication still needs a trusted request/dispatch lifecycle around the existing native publisher. A genuine **Open app** action needs an explicitly owned preview endpoint and liveness contract; a GitHub PR or portable JSON bundle is not a deployed application.

The broader unified queue, contributor onboarding, production identity, artifact-scale acceptance, post-publication corrections and end-to-end unaided build journey remain separate tracked work. Publication still does not authorize merge or deployment.
