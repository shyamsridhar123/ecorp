# Exact mission-to-Factory context

Date: September 9, 2026. Tracking: #199, under #145 / #63.

## Outcome

The selected mission can show its actual source context without treating a
filtered or truncated Factory snapshot as an origin index. A direct mission is
identified by an authorized complete lookup, while a Factory mission receives
its exact issue link. Guests retain the neutral behavior established in #185.

This is a foundation for the unified work-item cockpit, not a claim that the
entire Factory-to-PR workflow or production human identity is complete.

## Implementation and trust boundary

`GET /api/corps/{corp_id}/missions/{mission_id}/context?actor_id=…`:

- Uses existing `Operate` authentication.
- Checks the current human operator role, Corp and mission-room membership in
  the same SQL statement as the exact Factory lookup.
- Uses the existing unique `factory_work_items.mission_id` relationship, with
  no recent-item limit, new table or migration.
- Rejects an anomalous foreign-Corp link instead of filtering it into Direct.
- Returns only scoped identity and issue/link fields, with `Cache-Control:
  no-store`; claim tokens, policies and native account diagnostics are excluded.
- Reads no GitHub state and launches no controller or provider.

Factory materialization commits the mission and unique link together. There is
no supported later adoption/unlink operation; development reset removes both
aggregates together. Complete absence under these application invariants
establishes direct/non-Factory linkage. It does not distinguish browser creation
from another direct client, or promise correctness after arbitrary database
tampering.

The client binds response identity to the exact Corp/operator/mission/room and
view instance. Role or context changes, cancellation, timeout and same-ID
reopening discard previous responses. Old or denied APIs remain inconclusive.
Source links accept only the matching raw HTTPS GitHub issue URL, including
legitimate owner/repository display-case differences.

## Actual-store evidence

Twelve opt-in `issue199_` SQLx tests passed against the real migrations:

- Native direct creation and native Factory claim/materialization.
- All four supported human operator roles.
- Exact Factory attribution after 501 newer rows displace the target from the
  actual 500-item snapshot.
- Guest/spectator, nonhuman/unknown actor, wrong Corp and wrong room denial.
- Corrupt cross-Corp linkage rejection.
- Current membership and role removal.
- Private-field exclusion and absent mission handling.

Each read compared persisted state before and after. Target creation/linkage
used native store methods; additional identities, revocations, historical
fillers and the deliberate corrupt link were fixture SQL in SQLx-owned
disposable databases. No application database or provider was used by this suite.

Result: **12 passed, 0 failed**, 24.26 seconds. The earlier #198 / #169 ignored
suites were not rerun.

## Real API and browser evidence

The existing owned QA stack was upgraded with the validated binaries. Existing
missions, runs, source worktrees, native profiles and history were preserved.

- Alice and Bob received HTTP 200 for direct mission
  `2b7495e3-a1ba-448e-bde7-2d2f28ba7142`; each response echoed its own viewer.
- Both received the exact Factory work-item linkage for existing mission
  `b54001b1-713f-4950-8603-bdd37bffd5c3`.
- Guest context requests returned HTTP 403 without source fields; an absent
  mission returned HTTP 404.
- The existing guest-visible mission
  `13dcdd66-c613-4378-8490-3a5e916036b2` remained visible to Eve while the Factory
  snapshot stayed empty. Its exact context read was denied, and the browser
  retained **Mission origin unavailable**, without a hidden issue link.
- The direct mission changed from the old server's neutral/unavailable state
  to **Direct mission · This mission is not linked to Factory intake**.
- The authorized Factory mission showed its matching GitHub issue hyperlink.
- At a 390-pixel viewport the document measured 375 pixels wide, with no
  horizontal overflow; the source link received visible keyboard focus.

These browser operators are development principals. The existing Factory
records are retained QA fixtures, not new live GitHub intake or publication.
No new mission or provider run was created to obtain this evidence. The
previously completed application remained completed.

## Local checks

- Full required local gate passed: migration checks, formatting,
  workspace/all-target Clippy, workspace tests, web build, web lint and
  whitespace checks.
- Rust workspace: **429 passed, 0 failed, 171 ignored**.
- Focused client context, room projection and review-context tests:
  **64 passed** (30 new context tests plus existing regressions).
- Current server/runner/CLI binaries built successfully.
- Independent read-only review of the domain/store/handler change reported no
  actionable findings.

Hosted Actions, remote push, PR creation, merge, auto-merge and deployment are
not inferred from these local results.
