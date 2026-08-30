# Approval and budget validation — August 30, 2026

## Durable action approvals

`tools/e2e_approvals.mjs`:

- suspends a live child process before a high-risk side effect;
- restarts the server while the approval is pending;
- waits for the existing runner and process to reconcile;
- approves from Bob's second actor identity;
- proves the first decision queues one effect and an identical retry queues none;
- verifies the run completes after approval; and
- verifies a rejected action cancels its run.

## Budget and loop breaker

`tools/e2e_budgets.mjs`:

- drives deterministic token usage through steer, constrain, suspend, and stop;
- proves repeated identical tool activity is bounded;
- proves requester and Corp rolling budgets are evaluated;
- proves healthy human conversation does not increment no-progress or repeated-tool counters; and
- records every transition as a typed incident.

CI uploads `output/e2e-approvals.json` and `output/e2e-budgets.json`.
