# VendorGuard

VendorGuard is a dependency-free Node.js demonstration of multi-tenant third-party
vendor intake and risk review. It provides a browser workspace, JSON HTTP API,
file-backed persistence, deterministic risk scoring, optimistic concurrency,
idempotent submission, maker-checker review, and a per-tenant SHA-256-linked audit
chain.

This application demonstrates control behavior; it does not claim production
compliance certification.

## Start and stop

Requirements: Node.js 22 or newer. No package installation is required.

From the repository root in PowerShell:

```powershell
$env:PORT = "3000"
$env:VENDOR_GUARD_DATA_FILE = "$PWD\scenarios\vendor-guard\data\vendor-guard.json"
node scenarios/vendor-guard/server.mjs
```

Open <http://127.0.0.1:3000>. The process prints its bound URL after the data file
has loaded. Press `Ctrl+C` in the same terminal to stop it cleanly. To stop a
process started by a script, retain its process ID and use:

```powershell
Stop-Process -Id <process-id>
```

If `VENDOR_GUARD_DATA_FILE` is omitted, data is stored at
`scenarios/vendor-guard/data/vendor-guard.json`. The `data/` directory is ignored
by Git.

## Roles and tenant boundary

Every non-health API request requires all three explicit headers:

| Header | Purpose |
| --- | --- |
| `X-Tenant-Id` | Selects the tenant partition |
| `X-Actor-Id` | Identifies the durable event actor |
| `X-Role` | Must be `requester`, `reviewer`, or `admin` |

Requesters create, edit, and submit assessments. Reviewers approve, reject, or
request changes and must provide durable reasoning. For high-risk vendors, the
reviewer actor must differ from the submitter. Administrators can read and verify
only their tenant's audit chain.

Records are filtered and resolved by tenant before they are returned. A
cross-tenant identifier receives the same `404 NOT_FOUND` response as an unknown
identifier. Roles come from headers in this local scenario; a production boundary
must replace these demonstration headers with authenticated, server-issued
identity and tenant claims.

## Risk model and lifecycle

Risk is recalculated from the complete assessment on every write:

| Factor | Points |
| --- | --- |
| Data: public / internal / confidential / restricted | 0 / 10 / 22 / 35 |
| Internet exposure: no / yes | 0 / 25 |
| Criticality: low / medium / high | 0 / 12 / 25 |
| Spend: below $100k / $100k-$999,999 / at least $1m | 0 / 8 / 15 |
| Security review: complete / missing | 0 / 15 |

Scores are capped at 100. Thresholds are low `0-39`, medium `40-69`, and high
`70-100`. Responses include every factor, its points, explanation, thresholds,
and whether maker-checker separation is required.

The lifecycle is `draft -> pending_review -> approved|rejected|changes_requested`.
Changes-requested records may be edited and resubmitted. Approved and rejected
records are terminal. Every mutation requires `expectedVersion`; stale changes
return `409 VERSION_CONFLICT`. Submission also requires an `Idempotency-Key`
header of 8-128 safe characters. An exact replay returns the original result;
reuse for different input returns `409 IDEMPOTENCY_CONFLICT`.

## API

All request and response bodies are JSON.

| Method and path | Role | Behavior |
| --- | --- | --- |
| `GET /api/health` | Public | Process health |
| `GET /api/vendors` | Any authenticated role | Tenant vendor list |
| `GET /api/vendors/:id` | Any authenticated role | Tenant vendor detail |
| `POST /api/vendors` | Requester | Create a draft |
| `PATCH /api/vendors/:id` | Requester | Edit a draft or changes-requested record |
| `POST /api/vendors/:id/submit` | Requester | Idempotently submit |
| `POST /api/vendors/:id/reviews` | Reviewer | Record decision and reasoning |
| `GET /api/audit` | Admin | Read tenant audit events |
| `GET /api/audit/verify` | Admin | Recompute the tenant event chain |

Create and update bodies contain `name`, `service`, and:

```json
{
  "assessment": {
    "dataClassification": "restricted",
    "internetExposure": true,
    "criticality": "high",
    "annualSpend": 2000000,
    "securityReview": false
  }
}
```

Update, submit, and review bodies include a positive integer `expectedVersion`.
A review also contains `decision` (`approve`, `reject`, or `request_changes`) and
a `reason` of 10-1000 characters.

Errors have one stable shape and a request correlation identifier:

```json
{
  "error": {
    "code": "VERSION_CONFLICT",
    "message": "The vendor was changed by another actor",
    "requestId": "uuid",
    "details": {
      "expectedVersion": 1,
      "currentVersion": 2
    }
  }
}
```

## Persistence and security boundaries

Writes are serialized in-process, written to a same-directory temporary file,
and atomically renamed. State survives restart. Each tenant has an append-only
logical event chain: every event records its predecessor hash, then hashes the
canonical event with SHA-256. Admin verification recomputes sequence, link, and
content integrity. File-system write access remains outside this application's
trust boundary; verification detects modification but does not prevent an
operator from replacing the entire file. The server is intentionally bound to
`127.0.0.1`, has a 64 KiB body limit, serves only allow-listed static assets, and
sets restrictive browser security headers.

The JSON file is suitable for this isolated scenario, not multi-process
production use. A production deployment would require durable transactional
storage, an external identity provider, authorization claims, key management,
backups, retention rules, rate limiting, TLS, monitoring, and independently
anchored audit heads.

## Verification

Run the exact governed verifier commands from the repository root:

```powershell
node --test scenarios/vendor-guard/test/server.test.mjs
node scenarios/vendor-guard/test/browser.test.mjs
git diff --check -- scenarios/vendor-guard
```

The browser verifier requires `ECORP_PLAYWRIGHT_MODULE` to point to the
repository-provided or runner-bundled Playwright module. It does not install
packages or access the network. It launches the real server on a free loopback
port, uses isolated temporary persistence, closes the child process, and writes
desktop and 390-by-844 mobile captures to `evidence/browser.png` and
`evidence/browser-mobile.png`, plus machine-readable results to
`evidence/browser-result.json`.
