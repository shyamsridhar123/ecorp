# ADR 0024: Versioned mission specifications and verifier policies

## Status

Accepted — September 2, 2026

## Context

A short mission title and provider-produced evidence envelope are insufficient authority for
enterprise application work. Operators need to preserve the complete specification, references,
task boundaries, and exact evidence that will gate completion before an agent starts. Failed or
cancelled work may also need a corrected contract without silently mutating the authority under
which prior work ran.

## Decision

Every mission stores a durable description and positive specification version. Every task stores a
positive contract version alongside its typed `TaskContract` and `VerificationPolicy`.

Mission creation may supply:

- a long-form description/specification;
- an operator contract overlay for objective, expected output, acceptance criteria, allowed tools,
  prohibited actions, references, and write scope; and
- a typed verifier policy containing artifact, file, command, test, JSON-schema, and screenshot
  checks plus an optional human-approval or independent-review gate.

The description is delivered to every planned task in addition to remaining task-specific
instructions. User-authored lists are preserved or deliberately overlaid rather than compressed
back into the title. The generated task graph exposes the exact persisted checks and manual gate
before dispatch.

Contract changes create an immutable `mission_contract_revisions` row and
`mission.contract_revised` event. Each revision records the prior and replacement description,
contract, verifier policy, actor, reason, source run, action, version, normalized request, and
idempotency key.

Two explicit revision actions exist:

- `redispatch` is allowed only while the mission is ready and before its first run;
- `resume` is allowed only from the latest terminal, preserved, non-stop provider/worktree lineage.

A revision never dispatches or resumes work automatically. The operator must perform that separate
action after reviewing the new version. Resume reconstructs the provider prompt from the current
versioned contract before appending the operator's resume instruction.

Revision requests require current mission-room membership and either the original requester or an
owner, admin, or manager role. They reject active runs, stale contract versions, cross-task source
runs, completed missions, invalid contracts, and mission-budget overflow. Resume revisions cannot
change source identity, secrets, provider model or reasoning, budget, or deliverable authority.
They cannot widen allowed tools or write scope or remove an existing prohibition.

Verifier execution remains on the authenticated runner inside the assigned worktree. The server
still rejects accepted completion until every persisted automated check passes and any manual gate
is durably decided.

## Consequences

- Mission intent and acceptance evidence survive clients, provider sessions, and retries.
- A provider saying “done” cannot substitute for the operator-authored verifier policy.
- Revisions remain attributable and replayable without rewriting prior run history.
- Correcting a resumable task is intentionally constrained; widening authority requires a new
  mission or a pre-dispatch revision.
- Rich contracts increase UI density, so the default composer keeps an explicit advanced section
  and renders a human-readable completion-plan preview.
