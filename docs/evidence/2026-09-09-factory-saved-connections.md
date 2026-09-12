# Factory saved-connection routing

Date: September 9, 2026. Tracking: #204, under #145 / #63.

## Result and remaining acceptance

The original routing defect was reproduced and fixed against the actual ECorp
API. A real GitHub repository connection made through the browser could plan a
direct mission, but Factory rejected the same checked source because it discarded
the saved connection. After the change, both previews returned HTTP 200 without
creating a mission.

The native trusted controller then consumed a real GitHub Project issue and
dispatched three real Copilot specialists through that same saved connection.
All three roots completed their native checks, and integration began only after
their verified handoffs were available. No application code was authored by the
acceptance operator.

**The complete application outcome is not accepted yet.** Integration produced
retained application source and tests, but its task token allocation stopped the
provider. Existing controller/recovery behavior then classified the work as
cancelled and rejected checkpoint-verification discovery. This separate,
observed recovery gap is tracked in #206. Do not close #204's full outcome
acceptance, #145 or #63 on the routing evidence alone.

## Implementation

- Optional `workspace_connection_id` is immutable Factory policy. Absent legacy
  bindings remain absent rather than borrowing a new connection.
- Preflight and materialization reuse the existing connection-aware planner and
  current room/source/runner admission.
- Claim transactions require current connection/Corp/room authority. Every task
  must retain the exact claimed binding.
- Both materialization replay paths require current human/mission-room access.
  Historical replay does not require the provider or runner to remain online.
- A connection readiness/source conflict after claim releases only that fenced
  claim generation. Actual stale-token/version conflicts do not cancel a newer
  claim.
- The controller's scoped connection read returns only the shared DTO with
  `Cache-Control: no-store`, not native account configuration or setup reports.
- New bound intake reads the runner-checked source through the control plane.
  Native Git branch syntax validation does not require a controller checkout.
- The supported launcher retains the connection and an independent Factory ref;
  both old-record and current-record guards compare Git refs exactly.

## Local evidence

- Actual-migration store regressions: **14 passed**, 41.15 seconds.
- Actual-handler/real-migration regressions: **4 passed**, 11.25 seconds.
- Full local gate: **439 Rust tests passed, 189 ignored**; migration integrity,
  format, workspace/all-target Clippy, production web build, lint and whitespace
  passed. Ignored tests are not counted as passing.
- Launcher source suite: **11 passed**, including execution of the production
  ref comparator for case-only and Unicode-distinct refs.
- The full gate preceded the final CLI-only branch-syntax correction. The entire
  CLI suite (**89 passed**), all-target CLI Clippy, formatting and the actual CLI
  binary build were rerun after that correction. Server/runner Rust did not change.
- Independent review identified claim/replay, post-claim compensation and ref
  comparison gaps; their corrections have scoped regression coverage.

Earlier probe-fixture errors remain retained: invalid requested cost allocations
in the read-only comparison and an empty prohibited-actions list in the handler
fixture. A real CLI dry run also exposed the branch-syntax checkout dependency,
which was fixed rather than worked around with a dummy repository. No old opt-in
#198 / #169 SQLx suite was executed.

## Real connection, API and restart

The browser's actual Connect and test operation used:

- repository `shyamsridhar123/ecorp-enterprise-lab`;
- native GitHub identity `shyamsridhar123`;
- real Copilot readiness with the retained SDK 1.0.11 / CLI 1.0.79;
- connection `a0604308-737f-4554-9e24-49493a427991`;
- source `main` at `e3dc3d669b1a99832e2e7af9be16f7f39842586d`.

Alice and Bob's exact connection reads returned HTTP 200 with no private
configuration/report fields and `no-store`. Eve was denied with HTTP 403.
An unknown connection was denied with HTTP 400 under the existing shared
store-error convention, without source metadata; an initial probe incorrectly
expected 404 and was recorded as a probe assumption, not a product fix.

Only the owned QA stack was restarted. Original mission/task/run identities and
statuses were retained, including the completed Pantry Board app, which continued
to return HTTP 200. No new container or baseline-stack replacement was needed.
The Stop action completed normally; an outer wrapper initially misread a stale
native exit code. Process receipts and sockets confirmed the completed stop
before continuing only the pending Start.

## Real Factory provenance

The native CLI dry run selected lab issue #4 from Project #3, resolved the checked
source without a controller checkout, and validated a four-task Studio plan with
an empty mutation list.

| Object | Identity |
| --- | --- |
| GitHub issue | `shyamsridhar123/ecorp-enterprise-lab#4` |
| Controller | `00000000-0000-4000-8000-000000000204` |
| Factory item | `e6182bf4-0e41-420b-ac03-23a11c5619c5` |
| Mission | `290d72c8-50e8-4c31-8947-fcf45e587317` |
| Visual root run | `1b1ac83c-5e47-465c-926c-70f40f3d39a1` |
| Systems root run | `1f4fa6b3-a322-4cda-847f-7da09c7a58a3` |
| Quality root run | `d156d399-0faa-4ec2-88e5-e15a7a4097b8` |
| Integration run | `cd10992f-ce28-4f33-accf-de2b99eeb31f` |

All four runs retain the same saved connection and immutable source. The three
roots had distinct agent/provider-session identities, nonzero native usage and
passed verification. No routine action approval was pending in the observed
snapshots. This is not a claim that every future tool operation is approval-free.

Integration was stopped at 588,189 recorded tokens against its 550,000 allocation;
the whole mission had used 823,706 of 1,000,000. The original accounting was not
reset or revised. Its preserved checkpoint contains server/client code, tests,
README and evidence notes under
`scenarios/vendor-desk/saved-connection-demo/`. File existence is not proof the
application passes its tests or browser acceptance.

The exact controller was explicitly paused while this same source is recovered.
The native checkpoint preview selected no item because Factory state was
`cancelled`; it made no mutation. No replacement provider, mission or source issue
was created to hide that failure. No application PR, merge or deployment is
claimed. #206 owns completing this retained-source acceptance.

These are development-principal operations, not production two-human identity or
OS-level isolation evidence. No hosted GitHub Actions result is inferred.
