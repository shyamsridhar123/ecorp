# Regulated Credit-Policy Exception Workflow

A launchable, dependency-free reference implementation of a regulated
credit-policy exception process: multi-tenant intake, deterministic policy
validation, maker-checker separation, explicit risk and compliance review
states, expiring approvals, optimistic concurrency, and a tamper-evident
audit chain.

Scenario 3 for ECorp issue [#62]; implements issue
[#76](https://github.com/shyamsridhar123/ecorp/issues/76).

## Requirements

- **Python 3.11+** — the application runtime uses the standard library only.
  No packages to install, no network access at runtime.
- **Playwright + a local Chromium** — *only* for `browser_smoke.py`. The
  application itself never imports it. `browser_smoke.py` reuses whatever
  Chromium build is already present on the host and never downloads one.

## Start and stop

From `scenarios/credit-exception`:

```bash
# start (foreground, prints its PID)
python server.py --port 9313

# start with request logging
python server.py --port 9313 --verbose

# stop
#   foreground: Ctrl-C
#   background: send SIGTERM to the PID printed at startup
```

Then open <http://127.0.0.1:9313/>.

The server is a `ThreadingHTTPServer` that handles SIGINT and SIGTERM,
drains in-flight requests, and closes its listening socket on the way out —
so a stop leaves no orphaned port binding.

## Using the console

The UI has no login; pick an identity from the **Acting as** selector in
the header. Each fixture actor carries a tenant and a role set, and the
console only offers actions that actor could actually complete. The server
re-checks every one of them regardless.

A typical pass:

1. As `nw-requester-1`, fill in the request form and **Create draft**.
2. **Submit for review** (only the originator may submit).
3. As `nw-risk-1`, **Run eligibility analysis**, then **Risk: approve**.
4. As `nw-compliance-1`, **Compliance: approve**.
5. As `nw-authority-1`, **Authorize exception**. This actor must be
   independent of the requester and both reviewers.
6. As `nw-auditor-1`, **Verify integrity** to re-walk the audit chain.

## Domain model

### Lifecycle

```
draft → submitted → risk_review → compliance_review → pending_decision → approved
                          │              │                    │
                          └──────────────┴────────────────────┴──→ rejected
  (any pre-terminal state) ──→ withdrawn
  (expiry horizon passes)  ──→ expired
```

`compliance_review` is skipped for rules that do not require it (for
example `CP-201`).

### Deterministic policy

Validation is a pure function of the payload and the current time.

| Rule     | Ceiling  | Compliance review | Notes                          |
|----------|----------|-------------------|--------------------------------|
| `CP-101` | 2000 bps | required          | Debt-to-income                 |
| `CP-102` | 1500 bps | required          | Credit-score band              |
| `CP-201` | 1000 bps | not required      | Collateral coverage            |
| `CP-202` |  800 bps | required          | Sector concentration           |
| `CP-900` | —        | —                 | **Prohibited**: sanctions      |
| `CP-901` | —        | —                 | **Prohibited**: affordability  |

Additional deterministic gates:

- Applicant is a pseudonymous `APP-XXXXXX` key. Anything resembling a real
  identifier is rejected outright.
- Justification ≥ 40 characters; 1–6 compensating controls, each ≥ 15
  characters, no duplicates.
- Expiry must be in the future and within 180 days.
- The tenant ceiling applies on top of the rule ceiling, whichever binds
  first. Cascadia caps at 1500 bps, Northwind at 2500 bps.

### Maker-checker

Three independent constraints, all enforced server-side:

1. The requester may never review their own exception.
2. No actor may occupy two review seats on the same exception.
3. The final authority must be independent of the requester **and** every
   reviewer.

The `nw-dual-1` fixture deliberately holds both requester and authority
roles, so the tests prove separation binds on identity rather than on the
role grant alone.

### Approvals and expiry

Each review approval carries its own 72-hour TTL, clamped so it can never
outlive the exception's own expiry. An expired approval cannot be reused to
authorize — the file must be re-reviewed. A stale file can still be
*declined*; only the approval path is fenced.

### Concurrency

Every mutating call requires `expected_version`. Version is checked before
state, so a caller racing on a stale read gets an actionable
`version_conflict` rather than a confusing `invalid_state`.

### Idempotency

Mutating requests accept an `X-Command-Id` header. Replaying the same id
returns the stored response verbatim with `X-Idempotent-Replay: true`.
Reusing an id with a *different* payload is a client bug and returns
`409 idempotency_mismatch`. Command ids are tenant-scoped.

### Audit chain

One global append-only chain. Each entry commits to its predecessor via
SHA-256 over a canonical JSON encoding, so mutation, reordering, insertion,
and deletion all break verification at the first affected index. A global
(rather than per-tenant) chain makes cross-tenant deletion detectable too;
reads remain tenant-scoped.

## API

All `/api/*` routes except `/api/health`, `/api/policy-rules`, and
`/api/actors` require an `X-Actor` header.

| Method | Path                                       | Capability                     |
|--------|--------------------------------------------|--------------------------------|
| GET    | `/api/health`                              | open                           |
| GET    | `/api/policy-rules`                        | open                           |
| GET    | `/api/actors`                              | open (fixture directory)       |
| GET    | `/api/session`                             | any authenticated              |
| GET    | `/api/exceptions[?state=…]`                | `exception:read`               |
| POST   | `/api/exceptions`                          | `exception:create`             |
| GET    | `/api/exceptions/{id}`                     | `exception:read`               |
| GET    | `/api/exceptions/{id}/audit`               | `exception:read`               |
| POST   | `/api/exceptions/{id}/submit`              | `exception:submit` + owner     |
| POST   | `/api/exceptions/{id}/analyze`             | `exception:read`               |
| POST   | `/api/exceptions/{id}/withdraw`            | `exception:withdraw` + owner   |
| POST   | `/api/exceptions/{id}/risk-review`         | `exception:review_risk`        |
| POST   | `/api/exceptions/{id}/compliance-review`   | `exception:review_compliance`  |
| POST   | `/api/exceptions/{id}/decide`              | `exception:decide`             |
| GET    | `/api/audit/verify`                        | `exception:read`               |

### Error envelope

```json
{ "error": { "code": "version_conflict", "message": "…", "details": { … } } }
```

Codes: `validation_failed` (422), `policy_violation` (422),
`unauthenticated` (401), `forbidden` (403), `tenant_mismatch` (403),
`maker_checker_violation` (403), `not_found` (404),
`method_not_allowed` (405), `version_conflict` (409), `invalid_state` (409),
`approval_expired` (409), `idempotency_mismatch` (409).

A record belonging to another tenant returns `404`, never `403` —
cross-tenant existence must not leak.

## Fixtures

| Actor              | Tenant             | Roles                          |
|--------------------|--------------------|--------------------------------|
| `nw-requester-1`   | `tenant-northwind` | requester                      |
| `nw-requester-2`   | `tenant-northwind` | requester                      |
| `nw-risk-1`        | `tenant-northwind` | risk reviewer                  |
| `nw-compliance-1`  | `tenant-northwind` | compliance reviewer            |
| `nw-authority-1`   | `tenant-northwind` | credit authority               |
| `nw-auditor-1`     | `tenant-northwind` | auditor                        |
| `nw-dual-1`        | `tenant-northwind` | requester + credit authority   |
| `cs-requester-1`   | `tenant-cascadia`  | requester                      |
| `cs-risk-1`        | `tenant-cascadia`  | risk reviewer                  |
| `cs-compliance-1`  | `tenant-cascadia`  | compliance reviewer            |
| `cs-authority-1`   | `tenant-cascadia`  | credit authority               |

The token *is* the actor id. This is a scenario harness, not a credential
system, and it does not pretend otherwise.

## Data minimization

No real or synthetic customer data exists anywhere in this scenario. The
only applicant reference is an opaque `APP-XXXXXX` pseudonym, and the
validator rejects anything else by shape. There are no names, addresses,
account numbers, or government identifiers in the code, fixtures, or
evidence.

## Verification

```bash
python -m unittest discover -s tests -t . -v   # 91 tests
python browser_smoke.py                        # real Chromium, port 9313
python verify_evidence.py                      # artifact validation
```

See [EVIDENCE.md](EVIDENCE.md) for exact commands and recorded results.

## Layout

```
scenarios/credit-exception/
├── server.py                     # launcher, signal handling
├── browser_smoke.py              # Playwright workflow + overflow guards
├── verify_evidence.py            # artifact validator
├── creditexc/
│   ├── api.py                    # routing, JSON envelopes, idempotency
│   ├── service.py                # governed workflow transitions
│   ├── domain.py                 # aggregate, states, approvals
│   ├── policy.py                 # rule catalog + deterministic validation
│   ├── rbac.py                   # tenants, actors, capability matrix
│   ├── audit.py                  # hash-linked append-only chain
│   └── errors.py                 # structured error taxonomy
├── static/                       # index.html, app.js, styles.css
├── tests/test_credit_exception.py
└── evidence/
```

## Deliberate limitations

- **Storage is in-process.** Restarting the server clears all state. The
  scenario forbids external dependencies, and a locked dict demonstrates
  isolation, concurrency, and idempotency faithfully without one.
- **No real authentication.** Identity is a header. Swapping in a real
  IdP would not change the authorization logic, which is what this scenario
  is actually about.
- **The audit chain is not externally anchored.** It detects tampering by
  anyone without write access to the chain's own storage; it does not
  defend against an attacker who can rewrite the whole chain and recompute
  every hash. Production use would anchor the head hash externally.
