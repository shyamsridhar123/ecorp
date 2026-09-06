// Finish a preserved deterministic polling test; never create replacement work.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { readFile, writeFile, mkdir, open } from 'node:fs/promises'
import path from 'node:path'

assert.equal(process.env.CRONY_POLLING_TEST, '1')
const previous = process.env.CRONY_POLLING_RECOVERY_CHECKPOINT
const output = process.env.CRONY_POLLING_OUTPUT
const server = process.env.CRONY_SERVER_HTTP
assert.ok(previous && output && server && process.env.CRONY_CLI_BINARY && process.env.ECORP_TEST_SOURCE_REPOSITORY)
assert.equal(new URL(server).hostname, '127.0.0.1')
assert.equal(new URL(server).port, '18967')
const prior = JSON.parse(await readFile(previous, 'utf8'))
assert.equal(prior.phase, 'failed')
assert.ok(prior.mission_id && prior.run_id && prior.factory_item_id && prior.checks.secondary_wait)
for (const record of prior.processes) {
  assert.ok(record.ended_at)
  let missing = false
  try { process.kill(record.pid, 0) } catch (error) { if (error.code === 'ESRCH') missing = true; else throw error }
  assert.ok(missing, 'Prior controller PID remains present; inspect before any new start')
}
await mkdir(output, { recursive: true })
const reportPath = path.join(output, 'recovery-evidence.json')
const report = { scope: 'Supplemental deterministic recovery of preserved original lineage',
  predecessor: previous, mission_id: prior.mission_id, run_id: prior.run_id,
  factory_item_id: prior.factory_item_id, started_at: new Date().toISOString(), passed: false }
await writeFile(reportPath, JSON.stringify(report, null, 2), { flag: 'wx' })
const corp = '00000000-0000-4000-8000-000000000001'
const actor = '00000000-0000-4000-8000-000000000011'
async function snapshot() {
  const response = await fetch(`${server}/api/corps/${corp}/snapshot?actor_id=${actor}`)
  assert.ok(response.ok)
  return (await response.json()).snapshot
}
const before = await snapshot()
assert.equal(before.missions.length, 1)
assert.equal(before.runs.length, 1)
assert.equal(before.factory_work_items.length, 1)
assert.equal(before.missions[0].id, prior.mission_id)
assert.equal(before.runs[0].id, prior.run_id)
assert.equal(before.missions[0].status, 'completed')
assert.equal(before.runs[0].verification_status, 'passed')
assert.equal(before.factory_work_items[0].state, 'blocked')
const completion = before.events.find((event) => event.type === 'run.completed' && event.aggregate_id === prior.run_id)
assert.ok(completion)
assert.ok(Date.parse(completion.created_at) > Date.parse(prior.checks.failure_event.at))
assert.ok(Date.parse(completion.created_at) < Date.parse(prior.checks.secondary_wait.next_retry_at))
report.local_completion_during_outage = completion.created_at
const fixturePath = path.join(path.dirname(previous), 'github-state.json')
const originalFixture = JSON.parse(await readFile(fixturePath, 'utf8'))
assert.equal(originalFixture.repository, 'ecorp-fixtures/quota-intake')
assert.equal(originalFixture.graphql_query_counts.item_exact, 7)
assert.ok(originalFixture.graphql_query_counts.project_items >= 11)
assert.equal(originalFixture.item_list_calls ?? 0, 0)
const stdout = await open(path.join(output, 'controller.stdout.log'), 'a')
const stderr = await open(path.join(output, 'controller.stderr.log'), 'a')
const root = path.resolve(import.meta.dirname, '..')
const child = spawn(process.env.CRONY_CLI_BINARY, ['--server', server, 'factory-watch', corp, actor,
  '--controller-id', prior.controller_id, '--owner', 'ecorp-fixtures', '--project-number', '9',
  '--repository', 'ecorp-fixtures/quota-intake', '--source-repository-path', process.env.ECORP_TEST_SOURCE_REPOSITORY,
  '--source-base-ref', 'HEAD', '--publication-base-ref', 'main',
  '--adapter', 'fake-process', '--strategy', 'single', '--budget-tokens', '50000',
  '--budget-cost-microusd', '1000000', '--interval-seconds', '5', '--heartbeat-seconds', '5',
  '--github-cli', process.execPath,
], { cwd: root, windowsHide: true, stdio: ['ignore', stdout.fd, stderr.fd],
  env: { ...process.env, ECORP_FAKE_GITHUB_STATE: fixturePath,
    ECORP_GITHUB_CLI_PREFIX_ARGS_JSON: JSON.stringify([path.join(root, 'tools/fake_github_cli.mjs')]),
    GH_TOKEN: '', GITHUB_TOKEN: '', CRONY_ACCESS_TOKEN: '' } })
report.controller_pid = child.pid
await writeFile(reportPath, JSON.stringify(report, null, 2))
try {
  const deadline = Date.now() + 90_000
  let after
  while (Date.now() < deadline) {
    assert.ok(child.exitCode === null && child.signalCode === null, 'Owned recovery controller exited')
    after = await snapshot()
    if (after.factory_work_items[0]?.state === 'verified') break
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  assert.equal(after.factory_work_items.length, 1)
  assert.equal(after.factory_work_items[0].id, prior.factory_item_id)
  assert.equal(after.factory_work_items[0].state, 'verified')
  assert.equal(after.missions.length, 1)
  assert.equal(after.missions[0].id, prior.mission_id)
  assert.equal(after.runs.length, 1)
  assert.equal(after.runs[0].id, prior.run_id)
  assert.equal(after.tasks.length, 1)
  assert.equal(after.tasks[0].attempt_count, 1)
  const github = JSON.parse(await readFile(fixturePath, 'utf8'))
  assert.ok(github.graphql_events.filter((event) => event.call > prior.checks.failure_event.call)
    .every((event) => Date.parse(event.at) >= Date.parse(prior.checks.secondary_wait.next_retry_at)))
  report.no_requests_before_retry = true
  report.no_replacement_work = true
  report.factory_state = after.factory_work_items[0].state
  report.passed = true
} catch (error) {
  report.error = error.message
  throw error
} finally {
  if (child.exitCode === null && child.signalCode === null) {
    const exited = once(child, 'exit')
    child.kill()
    await exited
  }
  report.controller_ended_at = new Date().toISOString()
  await stdout.close(); await stderr.close()
  await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`)
}
console.log(JSON.stringify(report, null, 2))
