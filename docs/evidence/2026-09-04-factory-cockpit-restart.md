# Factory cockpit restart and multiplayer evidence

**Date:** September 4, 2026  
**Issue:** #145  
**Pull request:** #152  
**Hosted CI:** Not used; monthly GitHub Actions credits were exhausted.

## Scope

This validation exercised the selected Factory work-item cockpit across two human actors, a live
runner, server restarts, browser event-stream reconnects, lease-token recovery, durable steering,
contextual comments, an inline action decision, a verified commit/branch deliverable, and trusted
pull-request publication.

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
  "checked_at": "2026-09-04T21:26:07.034Z",
  "alice_replay_events": 23,
  "bob_replay_events": 23,
  "comment_retry_idempotent": true,
  "stale_lease_token_rejected": true,
  "stale_pending_steer_cancelled": true,
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
- A lease version fences a pending command if control is renewed, released, transferred, or expires
  before dispatch.
- The runner deduplicates command IDs, applies the steer at most once for a continuous runner
  process, and acknowledges the durable command.
- The server records the acknowledgment and marks the control message delivered.
- A negative runner acknowledgment terminalizes the command and cancels the associated control
  message instead of leaving it to starve the pending queue.
- Lease tokens remain private and are not persisted in runner-command payloads.
- New and previous runner versions retain a compatible control-message wire shape; new runners
  advertise durable acknowledgment support while older runners retain bounded legacy delivery.
- Controller pause, resume, and reconcile clicks retain their operation UUID and original expected
  version in browser session storage until a response succeeds.

## Focused publication restart drill

Command:

```powershell
$env:CRONY_SERVER_HTTP = 'http://127.0.0.1:8891'
$env:DATABASE_URL = 'postgres://crony:crony@127.0.0.1:54329/crony_issue145pub_43ba9414dd'
$env:ECORP_TEST_SOURCE_REPOSITORY = 'C:\Users\shyamsridhar\.codex\dogfood\issue145-publication-source-20260904-163621'
$env:ECORP_TEST_ARTIFACT_ROOT = 'C:\Users\shyamsridhar\.codex\dogfood\issue145-publication-artifacts-20260904-163621\publication-test-20260904-1836'
$env:CRONY_TEST_SERVER_PID_FILE = 'C:\Users\shyamsridhar\.codex\dogfood\issue145-publication-artifacts-20260904-163621\live\pids.json'
node tools/e2e_factory_cockpit_publication.mjs
```

The test used the real ECorp server, runner, Factory CLI, verifier, source-deliverable path, and
trusted publisher. GitHub was represented by the deterministic fake CLI and a local bare remote, so
the drill created no pull request in the production ECorp repository.

The publisher intentionally exited after the fake GitHub boundary created the pull request but
before the server recorded that remote checkpoint. The test then restarted the server, resumed
Alice and Bob from their independent event cursors, recovered the publication, and issued two
concurrent duplicate publication requests.

Fresh result:

```json
{
  "checked_at": "2026-09-04T23:33:10.715Z",
  "source_base_commit": "62a5b7377b0e1e544d7fab1b4eb5f92b26aa1930",
  "pull_request_number": 51,
  "pull_request_create_calls": 1,
  "project_status": "In Review",
  "project_after_pull_request": true,
  "duplicate_work_items": 0,
  "duplicate_missions": 0,
  "duplicate_runs": 0,
  "duplicate_publications": 0,
  "auto_merge_enabled": false,
  "merge_authorized": false,
  "deployment_authorized": false,
  "alice_replay_events": 9,
  "bob_replay_events": 9,
  "final_state": "published"
}
```

The complete machine-readable report is written to:

```text
output/e2e-factory-cockpit-publication.json
```

The focused drill proves:

- the same Factory lineage reaches a verified commit/branch deliverable and publication;
- crash recovery does not create another pull request or publication;
- concurrent retries converge on the persisted publication;
- the remote base remains unchanged while the review branch points to the verified commit;
- the Project item moves to `In Review` after, not before, pull-request creation;
- auto-merge remains off and neither merge nor deployment is authorized.

## Boundary

The publication restart drill used a deterministic GitHub boundary and local bare remote. It proves
the same-lineage ECorp orchestration and recovery behavior without mutating the production ECorp
repository. It does not claim provider-native GitHub approval handling, a real GitHub pull request,
merge, auto-merge, deployment, or hosted GitHub Actions evidence.
