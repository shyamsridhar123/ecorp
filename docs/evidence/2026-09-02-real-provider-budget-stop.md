# Authenticated Codex budget-stop validation

Date: September 2, 2026

This evidence closes the remaining real-provider acceptance gap in GitHub issue #49. The
implementation under test was already merged by PR #57; the validation ran on the later integrated
stack at `e990852`.

## Isolated topology

- PostgreSQL container: `ecorp-issue49-real-provider-live`
- PostgreSQL port/database: `55449` / `issue49`
- ECorp server: `127.0.0.1:8949`
- ECorp runner: `runner-issue49-real`
- Web client: `127.0.0.1:5249`
- Codex: `codex-cli 0.151.0-alpha.7.2`

## Real-provider run

- mission: `ea1c4acc-3a9f-415e-8146-6c8da9817b2a`
- task: `fab63265-a61a-499d-befc-853b65bfa194`
- run: `f58a16f6-1fd6-47b6-b750-b2f2b3cb2b7a`
- provider session: `01a0614e-0808-7892-98d7-4f3c911552fb`
- token ceiling: `1`
- measured usage: `32,884` input + `554` output = `33,438`

The immutable event order was:

1. sequence `44`: `run.usage`
2. sequence `45`: `run.breaker_transition` to `stop`
3. sequence `46`: `runner.command_acknowledged`
4. sequence `50`: `run.failed`
5. sequence `51`: `run.workspace_removed`

The late provider artifact attempt was rejected after the breaker. Final state was coherent:

- run, task, and mission: `failed`
- breaker: `stop`
- verification: `pending`, never `passed`
- accepted artifacts: `0`
- `run.completed` events: `0`
- retries: `0`; exactly one run existed
- generated `result.md`: absent
- clean worktree: removed, with its branch deleted
- live provider descendants after terminal cleanup: `0`

The current deterministic budget suite separately covers duplicate usage notifications,
hard-breaker non-retryability, approval fencing, late artifact rejection, and late completion
rejection.

## Browser evidence

Real Chromium rendered the mission as `FAILED`, with `0/1` tasks complete, one attempt, no artifact,
and no completed state. The audit panel showed one breaker event and the usage, breaker,
acknowledgment, failure ordering.

- desktop: `scrollWidth == clientWidth == 1265`
- mobile: `scrollWidth == clientWidth == 390`
- console/page errors: `0`
- screenshots:
  - `output/playwright/issue49-real-provider-desktop.png`
  - `output/playwright/issue49-real-provider-mobile.png`

## Separate lifecycle finding

The Codex process tree was empty after the failed run, but no durable `run.session_terminated`
event was recorded. The supervised Codex process also read user-global memory and attempted an MCP
authentication handshake despite the configured empty MCP map. These do not weaken the budget
breaker result; they are additional evidence for open security issue #51.

## Cleanup

Only the verified test-owned process tree and container were stopped. Ports `55449`, `8949`, and
`5249` were closed, and the PostgreSQL container was absent.
