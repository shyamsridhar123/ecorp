# Factory cockpit restart and multiplayer evidence

**Date:** September 4, 2026  
**Issue:** #145  
**Pull request:** #152  
**Hosted CI:** Not used; monthly GitHub Actions credits were exhausted.

## Scope

This validation exercised the selected Factory work-item cockpit across two human actors, a live
runner, a server restart, browser event-stream reconnects, lease-token recovery, durable steering,
contextual comments, and an inline action decision.

The source repository was an isolated local fixture outside the ECorp checkout:

```text
C:\Users\shyamsridhar\.codex\dogfood\issue145-cockpit-20260904-151206
```

Its Git remote identity was `acme/darkfactory-fixture`. Runner worktrees were also outside the
ECorp repository.

## Repeatable automated drill

Command:

```powershell
node tools/e2e_factory_cockpit_reconnect.mjs
```

The script uses two independently attributed browser-protocol clients, Alice and Bob, then:

1. creates one factory controller and one claimed work item;
2. exactly replays claim and materialization requests;
3. launches one slow, steerable run that suspends for an action decision;
4. records an Alice comment and a Bob steer;
5. restarts the test-owned server while leaving the runner alive;
6. reconnects both clients from their last event sequence;
7. replays the same comment and steer operation keys without adding another row, event, or provider
   effect;
8. rotates Bob's lease token after a browser reload and proves the old token returns HTTP 409 for a
   new command;
9. records a second Bob steer and comment;
10. approves from Bob and exactly replays the decision without another runner effect;
11. verifies the one work item on the one mission, one task, and one run.

Fresh final result:

```json
{
  "checked_at": "2026-09-04T20:53:04.046Z",
  "alice_replay_events": 23,
  "bob_replay_events": 23,
  "comment_retry_idempotent": true,
  "stale_lease_token_rejected": true,
  "steer_retry_idempotent": true,
  "durable_steer_acks": 2,
  "decision_attribution_preserved": true,
  "duplicate_decision_effects": 0,
  "duplicate_work_items": 0,
  "duplicate_missions": 0,
  "duplicate_runs": 0,
  "controller_status": "watching",
  "final_state": "verified"
}
```

The complete machine-readable report is written to:

```text
output/e2e-factory-cockpit-reconnect.json
```

## Real two-browser UI drill

Two separate browser tabs opened the same Factory item, one operating as Alice and one as Bob.

Observed through the rendered UI:

- both tabs received the same work item and pending decision without reload;
- Bob posted a contextual comment and sent live direction;
- Alice received Bob's comment, replied, and retained the same selected work item;
- the server restarted while the approval remained pending;
- both tabs returned to `live` and displayed `Run.Reconciled`;
- the two comments, Bob's lease holder, and the pending decision remained;
- Bob approved in the cockpit after restart;
- both tabs observed the decision and run completion with Bob's attribution.

This drill found a real UI defect: a reloaded browser lost its private fencing token while the
durable lease still named the same actor, leaving that actor unable to steer. The cockpit now shows
**Reclaim control** for that state. Reclaiming rotates the token and preserves the single-holder
lease; a stale token cannot issue a new command.

## Reliability changes proved by the drill

- Room comments carry a client operation UUID. Exact retry returns the original message; reuse
  with different content fails closed.
- Steering carries a client operation UUID and becomes a persisted `control_message` runner
  command.
- The server can redispatch a pending steering command after reconnect.
- The runner deduplicates command IDs, applies the steer at most once for a continuous runner
  process, and acknowledges the durable command.
- The server records the acknowledgment and marks the control message delivered.
- Lease tokens remain private and are not persisted in runner-command payloads.
- Controller pause, resume, and reconcile clicks retain their operation UUID and original expected
  version in browser session storage until a response succeeds.

## Boundary

This fixture intentionally stopped at verified work and did not create a real GitHub pull request.
Remote publication idempotency remains covered by `tools/e2e_factory_publication.mjs`; this document
does not claim that a same-lineage real PR restart drill was rerun here.
