# EVIDENCE — Issue #76, Regulated Credit-Policy Exception Workflow

Every command below was executed in the assigned worktree on
**September 3, 2026**. Results are transcribed from the actual runs. Where
a metric was not available to this agent, it is recorded as *not available*
rather than estimated.

## Environment

| Fact | Value |
|------|-------|
| Platform | Windows 11 Enterprise 10.0.26200 (win32) |
| Python | `Python 3.13.15` (contract requires 3.11+) |
| Application runtime dependencies | none — standard library only |
| Browser | Chromium headless shell, build 1234, already present on host |
| Package installations during this run | none |
| External network access during this run | none |
| Source base commit | `bbd875c22521d6c9636c8f42019f259684c6902e` |
| Write scope | `scenarios/credit-exception/**` only |

The browser binary was **discovered, not installed**. Playwright's pinned
build (`chromium_headless_shell-1200`) was absent; `browser_smoke.py`
resolves the newest locally installed build instead and reports the exact
path it used. No download occurred.

## 1. Unit and integration tests

```
$ python -m unittest discover -s tests -t . -v
```

Result — full log in [`evidence/unit-tests.txt`](evidence/unit-tests.txt):

```
Ran 91 tests in 0.792s

OK
```

**91 tests, 0 failures, 0 errors.** No third-party test framework; the
HTTP integration tests bind a real server on an ephemeral port and drive it
with `urllib`.

Coverage by required dimension:

| Required area | Test classes | Count |
|---------------|--------------|-------|
| Policy validation / determinism | `TestValidationAndPolicy` | 18 |
| Tenant isolation | `TestTenantIsolation` | 6 |
| Negative authorization | `TestAuthorization` | 10 |
| Maker-checker | `TestMakerChecker` | 5 |
| State machine | `TestStateMachine` | 8 |
| Approval expiry | `TestExpiry` | 7 |
| Optimistic concurrency | `TestConcurrency` | 6 |
| Audit integrity | `TestAuditIntegrity` | 9 |
| Idempotency | `TestIdempotencyStore` | 4 |
| HTTP end-to-end | `TestHttpApi` | 18 |

Notable cases:

- `test_requester_cannot_self_approve_as_authority` — uses the `nw-dual-1`
  fixture, which genuinely holds both roles, proving separation binds on
  identity and not merely on role grants.
- `test_concurrent_reviews_only_one_wins` — two real threads race the same
  review seat through a `threading.Barrier`; exactly one succeeds.
- `test_cross_tenant_existence_not_leaked` — asserts a foreign record and a
  nonexistent one produce byte-identical error codes and statuses.
- `test_payload_tamper_detected`, `test_deletion_detected`,
  `test_reordering_detected` — mutate the chain in place and confirm
  verification breaks at the correct index.
- `test_expired_approvals_do_not_block_rejection` — confirms the expiry
  fence applies to approval only, not to declining.

## 2. Real browser workflow

```
$ python browser_smoke.py
```

Exit code **0**. Full log in
[`evidence/browser-smoke.txt`](evidence/browser-smoke.txt); machine-readable
report in [`evidence/browser-smoke.json`](evidence/browser-smoke.json).

```
  ok  server started on port 9313
  ok  chromium launched (…\chromium_headless_shell-1234\…\chrome-headless-shell.exe)
  ok  app booted
  ok  created EXC-NORT-00001
  ok  tenant isolation: cascadia sees no northwind records
  ok  role denial: auditor has no submit action
  ok  submitted for review
  ok  version conflict returned 409 version_conflict
  ok  eligibility analysis routed to risk review
  ok  risk review approved
  ok  compliance review approved
  ok  maker-checker denial: forbidden
  ok  final authority approved the exception
  ok  audit chain verified: Chain VALID · 6 entries · tenant entries 6
  ok  audit entries rendered: 6
  ok  mobile 390x844 rendered detail and verification without overflow
  ok  server stopped (exit 1)

browser smoke: PASS (17 steps, 0 errors)
```

Contract requirements, each satisfied:

- **Owns its server lifecycle on port 9313** — starts `server.py` as a
  subprocess, polls `/api/health` until ready, and terminates it in a
  `finally` block. `exit 1` is the expected Windows return code for a
  terminated process; the stop path runs unconditionally.
- **Desktop 1440x900 and mobile 390x844** — both viewports exercised, both
  recorded in the JSON report.
- **create → submit → analyze → required approvals → audit verification** —
  the full chain, driven through real DOM clicks, not API calls.
- **Tenant denial** — switching to `cs-requester-1` yields zero visible
  Northwind records.
- **Role denial** — `nw-auditor-1` is offered no submit action.
- **Version conflict** — a deliberately stale `expected_version` returns
  `409 version_conflict`.
- **Fails on console errors, page errors, or horizontal overflow** —
  `check_no_overflow` compares `documentElement.scrollWidth` against
  `clientWidth` at five points across both viewports.

Screenshots: [`evidence/desktop-approved.png`](evidence/desktop-approved.png),
[`evidence/mobile-detail.png`](evidence/mobile-detail.png).

### Console-error filtering — disclosed

The negative-path probes intentionally provoke 403/404/409/422 responses.
Chromium logs each as a `Failed to load resource` console error even though
the application handled it correctly. `attach_guards` filters *only* that
message shape for *only* those four status codes; every other console error
and every uncaught page error still fails the run. This is a deliberate,
narrow exemption and is implemented explicitly in `browser_smoke.py` rather
than by suppressing console output wholesale.

## 3. Evidence validation

```
$ python verify_evidence.py
```

```
evidence verification: PASS (42/42 checks passed)
```

The verifier re-reads the artifacts rather than trusting any claim: it
confirms all 18 source files exist and are non-empty, asserts the runtime
package imports no third-party module, parses the unit-test log for `OK`
and a test count ≥ 80, and parses the browser report for `passed=true`,
zero errors, port 9313, both exact viewports, and each required workflow
step by name.

## 4. Scoped diff check

```
$ git diff --check -- scenarios/credit-exception
diff --check clean
```

```
$ git status --short
?? scenarios/credit-exception/
```

No whitespace errors. **Nothing outside the write scope was modified** —
the only entry is the new scenario directory.

## Defects found and fixed during this run

Two real defects were caught by the verification harness, not by
inspection. Both are recorded here because the mission asks for rework
evidence.

### 1. Version check ordered after state check (server)

`test_stale_version_rejected` and `test_version_conflict_over_http` failed
on the first full run: a caller submitting with a stale `expected_version`
received `invalid_state` instead of `version_conflict`.

Root cause: all five transition methods called `ensure_state()` before
`_check_version()`. When a concurrent actor had already advanced the record,
the state check tripped first and masked the real cause. Version staleness
is the root cause; state mismatch is its symptom — and only
`version_conflict` tells the client the actionable thing, which is to
reload and retry.

Fix: `_check_version()` now runs before `ensure_state()` in
`submit_exception`, `withdraw_exception`, `analyze_exception`,
`record_review`, and `decide_exception`.

### 2. UI offered actions the actor could not perform (client)

The browser smoke reported `role leak: auditor was offered the submit
action`. `actionButtons()` branched on workflow state alone, so an auditor
viewing a draft saw a **Submit for review** button.

The server always rejected the click, so this was never an authorization
bypass — but presenting an action that cannot succeed is a real usability
and least-privilege defect. Fix: `actionButtons()` now also checks the
current actor's capabilities and ownership. The server remains
authoritative; the client merely stops offering impossible actions.

A third, cosmetic issue — a `404` console error from the browser's
automatic `/favicon.ico` request — was fixed by answering `204` in
`_serve_static`.

## Governed-path acceptance

| Criterion | Status |
|-----------|--------|
| Claude started without user plugins, hooks, browser customization, MCP servers, or source-checkout memory writes | Satisfied for this agent's session; no such configuration was loaded or written. Independent confirmation is the harness operator's to make. |
| Provider command requests suspend through durable ECorp approvals and resume with the fenced decision | **Directly observed.** Two Bash requests in this session were suspended and returned explicit written rejections (an over-broad `find`/`wc` pipeline using command substitution, and an environment-enumerating command). Both were narrowed and re-issued successfully. |
| Interrupt/stop cleans the provider process tree without contaminating the checkout | Partially evidenced. `browser_smoke.py` starts and terminates a server subprocess and a Chromium process tree across three runs with no port-9313 leakage and no stray files; `git status --short` shows only the scoped directory. Full process-tree teardown under interrupt is umbrella issue #51 and was **not** exercised here. |
| Persisted verifier policy runs the generated tests and browser workflow; provider claims alone cannot complete the mission | Satisfied. Three independent gates (`unittest`, `browser_smoke.py`, `verify_evidence.py`) all execute real code and all must exit 0. |
| Exactly one portable, verification-linked immutable source deliverable is publishable | Pending fresh ECorp verification and independent review. The scenario is scoped to one directory and one preserved branch, but this failed run is not claimed as publishable. |
| Publication creates/reuses exactly one reviewable PR | **Not performed by this agent.** No PR was opened. |
| Project status advances only after the PR exists | **Not performed by this agent.** |
| No auto-merge, merge, or deployment occurs | Satisfied — none attempted. |
| Setup time, completion, rework, intervention, token/cost evidence, recovery, approvals | Partially available; see below. |

## Metrics

| Metric | Value |
|--------|-------|
| Rework cycles | 2 defects found and fixed (detailed above), plus 1 cosmetic |
| Interventions | 2 approval-broker rejections, both resolved by narrowing the command |
| Recovery events | 1 — Playwright's pinned browser build was absent; resolved by discovering a local build rather than installing |
| Automated tests | 91, all passing |
| Browser workflow steps | 17, zero errors |
| Evidence checks | 42/42 |
| Setup wall-clock time | *not available* — this agent has no session timer |
| Token consumption | *not available* — not exposed to this agent |
| Cost in micro-USD | *not available* — not exposed to this agent |

Token, cost, and wall-clock figures are deliberately left unfilled. The
contract forbids inventing unavailable metrics, and this agent cannot
observe them. The orchestrating harness holds that telemetry.

## Reproduction

```bash
cd scenarios/credit-exception
python -m unittest discover -s tests -t . -v   # expect: Ran 91 tests … OK
python browser_smoke.py                        # expect: PASS (17 steps, 0 errors)
python verify_evidence.py                      # expect: PASS (42/42 checks passed)
python server.py --port 9313                   # then open http://127.0.0.1:9313/
```
