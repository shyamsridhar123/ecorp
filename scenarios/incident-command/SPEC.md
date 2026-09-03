# Incident Command: enterprise incident response control room

Build a launchable Node.js 22+ application using only built-in modules. It must include a browser UI, JSON REST API, Server-Sent Events, durable local persistence, tests, and operator documentation.

## Business workflow

Operations teams create production incidents, assign responders, record timeline events, track mitigation, resolve incidents, and export a postmortem. Valid states are `open -> investigating -> mitigating -> resolved -> postmortem_complete`, with explicit guarded transitions.

## Required controls

- Scope all data by `x-tenant-id`, `x-actor-id`, and `x-role`. Roles: `reporter`, `responder`, `commander`, `auditor`.
- Enforce least privilege: reporters create incidents; responders append technical updates; commanders assign, change severity, transition state, and resolve; auditors are read-only.
- Mutating operations require `Idempotency-Key`, with deterministic replay and payload-conflict rejection.
- Use integer versions and `If-Match` for assignments, severity changes, and state transitions.
- Record severity, affected service, customer impact, owner, timestamps, and target response/mitigation deadlines. Derive and expose SLO breach state without trusting client-calculated values.
- Keep an append-only SHA-256-linked timeline and provide verification status.
- Stream tenant-scoped incident/timeline changes through an SSE endpoint; disconnects must not crash the server.
- Export a deterministic Markdown postmortem containing summary, impact, timeline, contributing factors, corrective actions, and audit-chain verification. Prevent path traversal and arbitrary file reads.
- Return structured JSON errors with stable codes and correlation IDs.

## Browser experience

Create a responsive command center that can:

- create and triage incidents;
- filter by severity and state;
- show SLO countdown/breach status;
- append timeline entries and transition workflow state according to role;
- show live SSE updates and reconnection state;
- preview/download the postmortem;
- clearly label this as a local dogfood application, not a production pager replacement.

## Delivery contract

- Include `server.mjs`, browser assets, `README.md`, and `EVIDENCE.md`.
- `node --test` must cover tenant isolation, RBAC, invalid transitions, idempotency replay/conflict, optimistic concurrency, deadline calculations, audit-chain verification, postmortem content, and SSE tenant filtering.
- `node server.mjs --port 9312` must start the app, and `GET /health` must return JSON status.
- Keep generated runtime data out of Git with `.gitignore`.
