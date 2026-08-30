import assert from 'node:assert/strict'
import { readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'

const root = path.resolve(import.meta.dirname, '..')
const load = async (name) =>
  JSON.parse(await readFile(path.join(root, 'output', name), 'utf8'))

const [approval, reconnect, idempotency, replay] = await Promise.all([
  load('e2e-approvals.json'),
  load('e2e-runner-reconnect.json'),
  load('e2e-idempotency.json'),
  load('e2e-replay.json'),
])

assert.equal(approval.server_restarted_while_suspended, true)
assert.equal(approval.duplicate_effect_queued, false)
assert.equal(reconnect.reconciliation_event, true)
assert.equal(reconnect.lost_run_status, 'lost')
assert.equal(idempotency.persisted_run_started_events, 1)
assert.ok(replay.reconnect_replay_count > 0)

const report = {
  checked_at: new Date().toISOString(),
  server_restart_during_run: true,
  runner_disconnect_and_recovery: true,
  duplicate_event_and_launch_defense: true,
  browser_reconnect_and_sequence_replay: true,
  evidence_files: [
    'e2e-approvals.json',
    'e2e-runner-reconnect.json',
    'e2e-idempotency.json',
    'e2e-replay.json',
  ],
}
await writeFile(
  path.join(root, 'output', 'e2e-chaos.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
