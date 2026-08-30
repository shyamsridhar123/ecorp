import assert from 'node:assert/strict'
import { writeFile } from 'node:fs/promises'
import path from 'node:path'
import { downloadVerifiedArtifact } from './artifact_client.mjs'

const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const root = path.resolve(import.meta.dirname, '..')

async function request(url, init) {
  const response = await fetch(`${server}${url}`, init)
  const body = await response.json()
  return { response, body }
}

async function post(url, body) {
  const result = await request(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
  if (!result.response.ok) {
    throw new Error(`${result.response.status}: ${JSON.stringify(result.body)}`)
  }
  return result.body
}

async function snapshot(demo) {
  return (
    await request(
      `/api/corps/${demo.corp_id}/snapshot?actor_id=${demo.alice_actor_id}`,
    )
  ).body
}

async function waitForRun(demo, runId, timeoutMs = 45_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const state = await snapshot(demo)
    const run = state.snapshot.runs.find((candidate) => candidate.id === runId)
    if (
      run &&
      ['completed', 'failed', 'cancelled', 'lost'].includes(run.status) &&
      ['preserved', 'removed'].includes(run.workspace_disposition)
    ) {
      return { state, run }
    }
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error(`timed out waiting for Copilot run ${runId}`)
}

const demo = await post('/api/demo/reset', {})
const initial = await snapshot(demo)
const capability = initial.runners
  .filter((runner) => runner.connected)
  .flatMap((runner) => runner.capabilities)
  .find(
    (candidate) =>
      candidate.name === 'github-copilot' &&
      candidate.models.some((model) => model.id === 'copilot-test-reasoning'),
  )
assert.ok(capability)
assert.equal(capability.available, true)
assert.equal(capability.models.length, 3)
assert.deepEqual(
  capability.models.map((model) => model.id),
  [
    'copilot-test-fast',
    'copilot-test-reasoning',
    'copilot-test-disabled',
  ],
)
const reasoningModel = capability.models.find(
  (model) => model.id === 'copilot-test-reasoning',
)
assert.equal(reasoningModel.supports_reasoning_effort, true)
assert.deepEqual(reasoningModel.supported_reasoning_efforts, [
  'low',
  'medium',
  'high',
])

const invalid = await request(`/api/corps/${demo.corp_id}/missions`, {
  method: 'POST',
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify({
    requested_by: demo.alice_actor_id,
    preferred_adapter: 'github-copilot',
    preferred_model: 'missing-copilot-model',
    title: 'Reject an unavailable Copilot model',
  }),
})
assert.equal(invalid.response.status, 400)

const mission = await post(`/api/corps/${demo.corp_id}/missions`, {
  requested_by: demo.alice_actor_id,
  preferred_adapter: 'github-copilot',
  preferred_model: 'copilot-test-reasoning',
  reasoning_effort: 'high',
  title: 'Use the selected GitHub Copilot model and produce verified evidence.',
})
const launch = await post(
  `/api/corps/${demo.corp_id}/missions/${mission.mission_id}/launch`,
  { requested_by: demo.alice_actor_id },
)
const completed = await waitForRun(demo, launch.run_id)
assert.equal(completed.run.status, 'completed')
assert.equal(completed.run.model, 'copilot-test-reasoning')
assert.equal(completed.run.reasoning_effort, 'high')
assert.ok(completed.run.provider_session_id)
assert.equal(completed.run.input_tokens, 321)
assert.equal(completed.run.output_tokens, 123)
const evidence = JSON.parse(
  await downloadVerifiedArtifact(server, demo, completed.run),
)
assert.equal(evidence.provider, 'github-copilot')
assert.equal(evidence.model, 'copilot-test-reasoning')
assert.equal(evidence.discovered_model_count, 3)
assert.ok(evidence.changed_paths.includes('copilot-result.md'))

const resumed = await post(
  `/api/corps/${demo.corp_id}/runs/${completed.run.id}/resume`,
  {
    requested_by: demo.alice_actor_id,
    prompt: 'Resume the GitHub Copilot session and reconfirm the selected model.',
  },
)
assert.equal(resumed.provider_session_id, completed.run.provider_session_id)
const resumedRun = await waitForRun(demo, resumed.run_id)
assert.equal(resumedRun.run.status, 'completed')
assert.equal(resumedRun.run.model, 'copilot-test-reasoning')
assert.equal(
  resumedRun.run.provider_session_id,
  completed.run.provider_session_id,
)
await downloadVerifiedArtifact(server, demo, resumedRun.run)

const report = {
  checked_at: new Date().toISOString(),
  adapter_available: capability.available,
  exposed_model_count: capability.models.length,
  exposed_models: capability.models.map((model) => ({
    id: model.id,
    name: model.name,
    policy_state: model.policy_state,
    supported_reasoning_efforts: model.supported_reasoning_efforts,
  })),
  invalid_model_status: invalid.response.status,
  selected_model: completed.run.model,
  reasoning_effort: completed.run.reasoning_effort,
  provider_session_id: completed.run.provider_session_id,
  resumed_same_session: true,
  artifact_uri: completed.run.artifact_uri,
  input_tokens: completed.run.input_tokens,
  output_tokens: completed.run.output_tokens,
}
await writeFile(
  path.join(root, 'output', 'e2e-copilot.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
