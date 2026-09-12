import assert from 'node:assert/strict'
import { mkdir, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { downloadVerifiedArtifact } from './artifact_client.mjs'

const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const root = path.resolve(import.meta.dirname, '..')
// An optional expectation must agree with the connected runner, not the driver
// machine or a failed launch. It cannot turn Windows execution into a Unix skip.
const expectedPlatform = process.env.CRONY_TEST_RUNNER_PLATFORM
assert.ok(
  expectedPlatform === undefined || ['win32', 'linux', 'darwin'].includes(expectedPlatform),
  `unsupported external-adapter test runner platform: ${expectedPlatform}`,
)

async function request(url, body) {
  const response = await fetch(`${server}${url}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
    signal: AbortSignal.timeout(30_000),
  })
  const payload = await response.json()
  return { response, payload }
}

async function post(url, body) {
  const { response, payload } = await request(url, body)
  if (!response.ok) throw new Error(`${response.status}: ${JSON.stringify(payload)}`)
  return payload
}

async function snapshot(demo) {
  const response = await fetch(
    `${server}/api/corps/${demo.corp_id}/snapshot?actor_id=${demo.alice_actor_id}`,
    { signal: AbortSignal.timeout(30_000) },
  )
  assert.equal(response.status, 200, 'could not read the owned fixture snapshot')
  return response.json()
}

async function assertUnavailable(demo, adapter, mission) {
  const { response, payload } = await request(
    `/api/corps/${demo.corp_id}/missions/${mission.mission_id}/launch`,
    { requested_by: demo.alice_actor_id },
  )
  assert.equal(response.status, 409, `${adapter} must refuse Unix dispatch`)
  assert.ok(
    typeof payload.error === 'string' &&
      payload.error.includes(`requires adapter ${adapter}, but that adapter is unavailable`),
    `unexpected ${adapter} refusal: ${JSON.stringify(payload)}`,
  )
  const state = await snapshot(demo)
  const tasks = state.snapshot.tasks.filter((task) => task.mission_id === mission.mission_id)
  assert.equal(tasks.length, 1, 'expected the single owned provider task')
  const taskIds = new Set(tasks.map((task) => task.id))
  assert.equal(
    state.snapshot.runs.filter((run) => taskIds.has(run.task_id)).length,
    0,
    'unsupported adapter dispatch must not persist a run',
  )
  assert.equal(
    state.snapshot.events.filter((event) =>
      event.type.startsWith('run.') &&
      (event.correlation_id === mission.mission_id || taskIds.has(event.aggregate_id)),
    ).length,
    0,
    'unsupported adapter dispatch must not journal provider execution',
  )
  return {
    adapter,
    dispatched: false,
    launch_status: response.status,
    reason: 'External CLI process containment is unsupported on this runner platform.',
  }
}

async function runProvider(demo, adapter) {
  const mission = await post(`/api/corps/${demo.corp_id}/missions`, {
    requested_by: demo.alice_actor_id,
    preferred_adapter: adapter,
    title: `Common provider parity sample for ${adapter}`,
  })
  if (!executionSupported) return assertUnavailable(demo, adapter, mission)
  const launch = await post(
    `/api/corps/${demo.corp_id}/missions/${mission.mission_id}/launch`,
    { requested_by: demo.alice_actor_id },
  )
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    const state = await snapshot(demo)
    const run = state.snapshot.runs.find((candidate) => candidate.id === launch.run_id)
    if (run?.status === 'completed') {
      const bytes = await downloadVerifiedArtifact(server, demo, run)
      const evidence = JSON.parse(bytes)
      assert.equal(evidence.provider, adapter)
      assert.equal(evidence.exit_success, true)
      assert.ok(run.provider_session_id)
      return {
        adapter,
        dispatched: true,
        run_id: run.id,
        provider_session_id: run.provider_session_id,
        artifact_sha256: run.artifact_sha256,
        input_tokens: run.input_tokens,
        output_tokens: run.output_tokens,
      }
    }
    if (run && ['failed', 'cancelled', 'lost'].includes(run.status)) {
      throw new Error(`${adapter} run ended as ${run.status}: ${run.summary}`)
    }
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error(`timed out waiting for ${adapter}`)
}

const demo = await post('/api/demo/reset', {})
const initial = await snapshot(demo)
const connected = initial.runners.filter((runner) => runner.connected)
assert.equal(connected.length, 1, 'external-adapter E2E needs exactly one connected fixture runner')
const [runner] = connected
const runnerPlatform = new Map([
  ['windows', 'win32'], ['linux', 'linux'], ['macos', 'darwin'],
]).get(runner.os)
assert.ok(runnerPlatform, `unsupported connected runner OS: ${runner.os}`)
if (expectedPlatform !== undefined) {
  assert.equal(expectedPlatform, runnerPlatform, 'test expectation does not match the connected runner platform')
}
const executionSupported = runnerPlatform === 'win32'
const results = []
for (const adapter of ['claude-code', 'opencode']) {
  results.push(await runProvider(demo, adapter))
}
if (executionSupported) {
  assert.equal(new Set(results.map((result) => result.artifact_sha256)).size, 2)
}

const report = {
  checked_at: new Date().toISOString(),
  runner_id: runner.id,
  runner_platform: runnerPlatform,
  execution_supported: executionSupported,
  common_sample: executionSupported,
  providers: results,
}
const reportPath = process.env.CRONY_EXTERNAL_ADAPTER_REPORT ??
  path.join(root, 'output', 'e2e-external-adapters.json')
await mkdir(path.dirname(reportPath), { recursive: true })
await writeFile(
  reportPath,
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
