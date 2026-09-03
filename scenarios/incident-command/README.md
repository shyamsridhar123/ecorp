# Incident Command

Incident Command is a launchable, dependency-free Node.js control room for
local incident-response dogfooding. It supports tenant-scoped incident
declaration, triage, responder updates, guarded workflow transitions, live SSE
updates, durable JSON persistence, and deterministic Markdown postmortems.

> **Local dogfood only:** this is not a production pager, identity provider, or
> substitute for an enterprise incident-management platform.

## Quick start

Requirements: Node.js 22 or newer. No package installation is needed.

```powershell
node server.mjs --port 9312
```

Open `http://127.0.0.1:9312/` and select a local tenant, actor, and role in the
identity bar.

Optional launch settings:

```powershell
node server.mjs --port 9312 --host 127.0.0.1 --data-dir .\runtime
```

Equivalent environment variables are `PORT`, `HOST`, and
`INCIDENT_COMMAND_DATA_DIR`.

## Roles and workflow

Every API and SSE request requires:

- `x-tenant-id`
- `x-actor-id`
- `x-role`: `reporter`, `responder`, `commander`, or `auditor`

Least privilege is explicit:

| Role | Allowed operations |
| --- | --- |
| Reporter | Create incidents and read tenant data |
| Responder | Append technical timeline updates and read tenant data |
| Commander | Assign owners, change severity, edit postmortem fields, advance workflow state, and read |
| Auditor | Read-only access within the tenant |

The guarded workflow is:

```text
open -> investigating -> mitigating -> resolved -> postmortem_complete
```

Skipping, reversing, or repeating a state transition returns
`INVALID_TRANSITION`.

## Server-owned SLO policy

Clients provide severity, but never deadlines or breach status. The server
derives response and mitigation deadlines from the incident creation timestamp:

| Severity | Response target | Mitigation target |
| --- | ---: | ---: |
| SEV1 | 15 minutes | 60 minutes |
| SEV2 | 30 minutes | 240 minutes |
| SEV3 | 60 minutes | 480 minutes |
| SEV4 | 240 minutes | 1,440 minutes |

Transitioning to `investigating` records the response milestone; transitioning
to `mitigating` records the mitigation milestone. API responses expose
server-calculated `pending`, `met`, or `breached` status.

## API

All paths below except `/health` and browser assets require the three identity
headers.

| Method | Path | Purpose | Role |
| --- | --- | --- | --- |
| `GET` | `/health` | Process and persistence readiness | Public |
| `GET` | `/api/incidents` | List tenant incidents; optional `severity` and `state` filters | Any |
| `POST` | `/api/incidents` | Declare an incident | Reporter |
| `GET` | `/api/incidents/:id` | Read one tenant incident | Any |
| `PATCH` | `/api/incidents/:id/assignment` | Assign owner | Commander |
| `PATCH` | `/api/incidents/:id/severity` | Change severity and recalculate deadlines | Commander |
| `POST` | `/api/incidents/:id/timeline` | Append a technical update | Responder |
| `POST` | `/api/incidents/:id/transitions` | Advance one guarded workflow step | Commander |
| `PATCH` | `/api/incidents/:id/postmortem` | Save contributing factors/actions | Commander |
| `GET` | `/api/incidents/:id/postmortem` | Preview deterministic Markdown | Any |
| `GET` | `/api/incidents/:id/postmortem?download=1` | Download generated Markdown | Any |
| `GET` | `/api/incidents/:id/audit` | Verify the linked timeline | Any |
| `GET` | `/api/events` | Tenant-filtered SSE stream | Any |

### Mutation controls

Every `POST` or `PATCH` requires an `Idempotency-Key`. Records are persisted per
tenant. Repeating the same canonical request replays the original status, body,
ETag, and Location; reusing the key with a different payload, route, actor,
role, or expected version returns `IDEMPOTENCY_CONFLICT`.

Assignment, severity, postmortem, and transition operations require `If-Match`
with the current integer version:

```http
If-Match: "3"
```

A stale value returns `VERSION_CONFLICT` with expected and actual versions.

### Example declaration

```powershell
$headers = @{
  'x-tenant-id'    = 'acme-ops'
  'x-actor-id'     = 'alice'
  'x-role'         = 'reporter'
  'Idempotency-Key' = [guid]::NewGuid().ToString()
}

$body = @{
  title = 'Checkout requests failing'
  severity = 'sev2'
  affectedService = 'checkout-api'
  customerImpact = 'Customers cannot complete purchases.'
} | ConvertTo-Json

Invoke-RestMethod `
  -Uri 'http://127.0.0.1:9312/api/incidents' `
  -Method Post `
  -Headers $headers `
  -ContentType 'application/json' `
  -Body $body
```

Errors are always structured JSON:

```json
{
  "error": {
    "code": "VERSION_CONFLICT",
    "message": "Incident version does not match If-Match.",
    "correlationId": "a UUID",
    "details": {
      "expected": 2,
      "actual": 3
    }
  }
}
```

The same correlation ID is returned in `X-Correlation-Id`.

## Architecture

- `server.mjs` — compact HTTP router, identity/RBAC checks, JSON API, static
  asset serving, SSE broker, correlation IDs, and CLI lifecycle.
- `lib/domain.mjs` — workflow rules, validation, SLO derivation, integer
  versions, SHA-256 timeline chain, and deterministic postmortem rendering.
- `lib/store.mjs` — single-process serialized mutations and durable,
  fsync-before-rename JSON snapshots.
- `public/` — responsive browser command center. It uses `fetch()` streaming so
  the required identity headers are present on SSE requests.
- `test/` — deterministic HTTP integration tests using only `node:test` and
  other built-in modules.

State is stored at `runtime/state.json` by default. Each mutation is applied to
a cloned state snapshot, written to a temporary file, flushed, atomically
renamed, and only then made visible in memory. Runtime data is ignored by Git.

## Audit and postmortem guarantees

Every incident starts with a creation event. Each later change appends a
timeline entry containing the prior hash and a SHA-256 hash over canonical event
data. Reads expose `auditChain.valid`, entry count, head hash, and verification
issues.

Postmortems are generated directly from the scoped in-memory incident record;
the API accepts no filesystem path and has no arbitrary-file read endpoint.
Filenames are derived only from validated server-generated UUIDs. Markdown
contains summary, impact, full timeline, contributing factors, corrective
actions, and audit-chain status, with no generation timestamp, so repeated
exports are byte-for-byte deterministic.

## Security choices

- Tenant lookup always occurs before incident lookup; a cross-tenant ID is
  indistinguishable from a missing incident.
- Reporters, responders, commanders, and auditors have separate mutation
  allowlists.
- Unsupported fields and client-supplied deadline fields are rejected.
- Request bodies are JSON-only and capped at 1 MiB.
- Mutation replay is durable and payload conflicts fail closed.
- Optimistic concurrency protects command decisions.
- Static assets use an exact route map rather than user-controlled paths.
- Browser responses include CSP, frame denial, no-referrer, and MIME-sniffing
  protections.
- Corrupt persistence fails startup rather than silently discarding state.
- SSE writes are tenant-filtered and disconnect/error cleanup is idempotent.

The header identity model is intentionally a local simulation. In production,
put the service behind authenticated transport and derive tenant, actor, and
role from verified claims rather than accepting caller-authored headers.

## Test

```powershell
node --test
```

The suite covers tenant isolation, RBAC, invalid transitions, durable
idempotency replay/conflict, optimistic concurrency, deadline and breach
calculations, timeline tamper detection, deterministic postmortem content, SSE
tenant filtering/disconnects, restart persistence, launch health, and
path-traversal resistance.

## Known limitations

- Persistence is designed for one server process. There is no cross-process
  file lock or distributed consensus.
- The local header identity selector is not authentication.
- SSE events are live-only; clients reconnect and refresh current state rather
  than replaying an event log by `Last-Event-ID`.
- Runtime JSON has no retention/compaction policy for incidents or idempotency
  records.
- Notifications, paging, chat integrations, and multi-region failover are
  intentionally out of scope for this local dogfood build.
