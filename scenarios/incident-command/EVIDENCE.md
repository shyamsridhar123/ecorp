# Final Incident Command recovery evidence

Date: 2026-09-03

## Provenance

- Final recovery issue: GitHub `#90`, Finalize Incident Command from the completed reference
- Completed implementation issue: GitHub `#88`
- Audited hard-stop proof issue: GitHub `#75`
- Immutable source base: `5999a4c808867796671dbafe02ccd8af69a74445`
- Authorized completed local reference: `e498b4520f61a6a1abe9165ab93f15c2cc3173b7`
- Reconstruction restored the full `scenarios/incident-command/**` tree from the authorized local Git object; unrelated history was not imported.
- The implementation uses Node.js built-in modules only. No external network access or package installation was used.
- Hosted GitHub Actions evidence is unavailable by contract; the persisted ECorp verifier and producing-worktree results are authoritative.

## Final broad-scope recovery

Issue `#88` completed the implementation and its verifier workflow, but source export subsequently failed when the preserved deliverable scope was invalidated. The completed tree was retained as the authorized reference above rather than reconstructed from unrelated history or reimplemented.

Issue `#90` performs the bounded recovery under the broad `scenarios/incident-command/**` deliverable scope. It restores the complete reference tree and refreshes only this evidence to identify the final recovery and the earlier post-completion export failure.

## Current automated evidence

The persisted Node verifier result is:

```text
node --test scenarios/incident-command/test/incident-command.test.mjs
12 tests, 12 passed, 0 failed, duration 5448.7076 ms
```

The browser harness passed the complete local browser-to-server-to-persistence workflow and regenerated:

- `evidence/browser-result.json`
- `evidence/browser.png` at 1280 by 900
- `evidence/browser-mobile.png` at 390 by 844

The JSON result reports `status: passed`, healthy durable persistence, no unexpected browser console or page errors, no horizontal overflow at either viewport, and all workflow checks as true. Its top-level `sse` object records the live connection, tenant filtering, and the streamed refresh that preserved an unsaved postmortem draft. After the postmortem was saved, a full browser reload restored both contributing factors and corrective actions from durable state. The workflow also verified eight SHA-256-linked audit entries and deterministic Markdown postmortem content.

The 12 Node checks plus the browser workflow are the 13 automated verifier checks. Coverage includes tenant RBAC and isolation, guarded workflow transitions, idempotency replay and conflict rejection, integer `If-Match` concurrency, server-derived SLOs, SHA-256 audit verification, tenant-filtered SSE, deterministic postmortems, postmortem reload persistence, restart durability, desktop/mobile layout, and traversal rejection.

## Audited hard-stop and recovery lineage

The original `#75` run `f5a6b5a4-1fe3-4e72-a854-67ed17337fd0` used the real Codex app-server adapter. ECorp recorded an immediate audited live-steering message and non-zero streamed Codex usage. The run crossed the prior 4,000,000-token rolling actor limit, escalated monotonically through `constrain -> suspend -> stop`, rejected accepted completion, and preserved its worktree. That hard-stop lineage is intentionally non-resumable.

For `#88`, the explicit rolling policy is 20,000,000 actor tokens and 100,000,000 Corp tokens per 24 hours. No rolling actor/Corp breaker occurred under that policy. ECorp recorded two durable mission-budget recoveries while preserving prior spend and the isolated workspace. Resume retained the same Codex provider session and the same preserved worktree rather than creating a replacement implementation lineage. Streaming usage remained non-zero and live steering remained audited.

## Governance boundary

This file reports source and verifier evidence only. Factory work-item linkage, mission/task/run acceptance, budget records, and reviewer/publication decisions remain authoritative in ECorp state. It does not claim independent review, source export, branch or pull-request publication, duplicate-publication recovery, Project status movement, merge, auto-merge, issue closure, or deployment before those effects occur. No Git commit was created by the producing agent; ECorp owns post-verification commit creation.