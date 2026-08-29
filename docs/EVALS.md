# Evaluation and real-world testing

## Evidence rule

An implementation claim needs evidence at the same scope:

- code compiles
- targeted tests pass
- the real service starts
- the browser reaches the server
- the server reaches the runner
- the runner starts a child process
- the process creates an artifact
- Postgres records state and events
- the browser receives the final state

## First vertical-slice scenario

1. Start Postgres, server, runner, and web client.
2. Bootstrap the demo Corp.
3. Open Alice and Bob in separate browser tabs.
4. File a mission.
5. Dispatch it.
6. Confirm the runner launches a real child process.
7. Confirm live status events reach both tabs.
8. Confirm one operator can hold the agent control lease.
9. Confirm a second operator cannot replace an unexpired lease.
10. Send live direction to the active run.
11. Confirm the child process acknowledges it.
12. Confirm `result.md` exists.
13. Recompute and compare its SHA-256.
14. Confirm mission, task, and run reach `completed`.
15. Restart a browser and confirm state remains.

## Required chaos cases

- duplicate runner event
- server restart during an active run
- runner disconnect and reconnect
- expired lease
- two simultaneous lease claims
- duplicate mission launch
- child exits without terminal event
- child emits invalid JSON
- artifact missing after artifact event
- path containing spaces
- Postgres unavailable
- browser reconnect

## Quality gates

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm build:web
pnpm lint:web
```

The repository also contains `tools/e2e_smoke.ps1`, which exercises the actual running stack.
`tools/e2e_replay.mjs` verifies ordered reconnect replay and proves that an up-to-date cursor
receives no duplicate events.
`tools/e2e_leases.mjs` races two controllers, verifies token rotation and stale-command rejection,
exercises explicit release and transfer, checks role-gated emergency stop, and confirms the runner
actually cancels the child process.
`tools/e2e_rooms.mjs` verifies persisted top-level messages and replies, author attribution,
structured mentions and entity links, two-member visibility, non-member write rejection, and
room-filtered WebSocket replay.
