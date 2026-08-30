# Codex adapter validation — August 29, 2026

## Scope

This record covers issue #11: start, structured streaming, live steering, interruption, emergency
stop, durable resume, usage, repository effects, and evidence.

## Environment

- Windows runner
- Codex CLI `0.150.0-alpha.8`
- Authenticated local Codex account
- Codex app-server over stdio JSON-RPC
- Workspace-write sandbox with network disabled
- User-configured MCP servers, apps, and hooks disabled by the runner

## Deterministic validation

The runner test suite uses `scripts/fake-codex-app-server.mjs` to exercise the consumed protocol
surface without credentials. The suite covers:

- adapter availability and explicit capabilities
- start and structured status/output
- cumulative usage de-duplication
- active-turn steering
- graceful interruption
- emergency stop
- resume into the same provider session and workspace
- evidence for completed, cancelled, and failed turns

`tools/e2e_codex.mjs` additionally exercises those behaviors across HTTP, Postgres, WebSocket
runner control, the adapter, repository files, and evidence hashes.

## Authenticated real-provider scenarios

### Start and live steer

- Run: `bba6ea33-3f9e-4d15-ba06-19f49ec4284c`
- Provider thread: `01a04fe8-af4d-7160-8008-85adc6263b95`
- Result: completed
- Usage: 102,154 input tokens; 1,186 output tokens
- Verified files:
  - `base.txt` contained `BASE` plus LF
  - `steered.txt` contained `STEERED` plus LF
- Evidence SHA-256:
  `d27227ae94cc3e5fd7176ef932d2bd54bb2a2694f7a2fbda05d64064e478c95f`

### Interrupt and resume

- Interrupted run: `87c706ef-6de8-4763-9f24-6f03c25244b8`
- Resumed run: `e39cd616-0e80-4fea-86f7-d1a0454fda73`
- Shared provider thread: `01a04fed-725e-7e90-8e1a-744000c4c418`
- Source result: cancelled through `turn/interrupt`
- Resume result: completed in the source workspace
- Resume usage: 105,223 input tokens; 1,467 output tokens
- Verified effects:
  - `before.txt` survived interruption
  - `resumed.txt` contained `RESUMED` plus LF
  - `should_not_exist.txt` was not created
- Resume evidence SHA-256:
  `9b7b651cf93456cde5a6d9e20cf1a30a3df79afa8350b90a9c993df1fff09498`

### Emergency stop

- Run: `ed44931d-2a15-4f0a-b5a5-b6601109a6a6`
- Provider thread: `01a04fef-4292-73d1-851d-11bd2720d41f`
- Result: cancelled through the role-gated emergency-stop path
- Verified effects:
  - `before_stop.txt` existed
  - `after_stop.txt` was not created
- Evidence SHA-256:
  `a0c0c451dd3ead7ce9e3d45bdcfd7a79c70cfc2a12882c0dff58fa17aedaf64c`

## Known boundary

Codex token notifications do not include price, so `cost_microusd` remains zero. Budget pricing and
provider-rate policy belong to backlog issue #20. A turn interrupted before its first token-usage
notification can also report zero tokens even though the resumed turn records subsequent usage.
