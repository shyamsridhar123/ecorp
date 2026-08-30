import assert from 'node:assert/strict'
import { readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'

const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const root = path.resolve(import.meta.dirname, '..')

async function post(url, body) {
  const response = await fetch(`${server}${url}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
  const payload = await response.json()
  if (!response.ok) throw new Error(`${response.status}: ${JSON.stringify(payload)}`)
  return payload
}

async function snapshot(demo) {
  const response = await fetch(
    `${server}/api/corps/${demo.corp_id}/snapshot?actor_id=${demo.alice_actor_id}`,
  )
  const payload = await response.json()
  if (!response.ok) throw new Error(`${response.status}: ${JSON.stringify(payload)}`)
  return payload
}

async function waitForRun(demo, runId, timeoutMs = 360_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const state = await snapshot(demo)
    const run = state.snapshot.runs.find((candidate) => candidate.id === runId)
    const pendingApproval = state.snapshot.action_approvals.find(
      (candidate) => candidate.run_id === runId && candidate.status === 'pending',
    )
    if (pendingApproval) {
      throw new Error(
        `live Copilot requested unexpected durable approval: ${pendingApproval.action} (${pendingApproval.rationale})`,
      )
    }
    if (
      run &&
      ['completed', 'failed', 'cancelled', 'lost'].includes(run.status) &&
      ['preserved', 'removed'].includes(run.workspace_disposition)
    ) {
      return { state, run }
    }
    await new Promise((resolve) => setTimeout(resolve, 500))
  }
  throw new Error(`timed out waiting for live Copilot run ${runId}`)
}

const demo = await post('/api/demo/reset', {})
const initial = await snapshot(demo)
const capability = initial.runners
  .filter((runner) => runner.connected)
  .flatMap((runner) => runner.capabilities)
  .find((candidate) => candidate.name === 'github-copilot')
assert.ok(capability, 'runner omitted the GitHub Copilot capability')
assert.equal(capability.available, true, capability.detail)
assert.ok(capability.models.length > 0, 'Copilot returned no models')
const model =
  capability.models.find((candidate) => candidate.id === 'gpt-5-mini') ??
  capability.models.find(
    (candidate) =>
      candidate.policy_state !== 'disabled' &&
      candidate.id !== 'auto' &&
      !candidate.name.toLowerCase().includes('internal only'),
  ) ??
  capability.models.find((candidate) => candidate.policy_state !== 'disabled')
assert.ok(model, 'Copilot returned no enabled model')

const mission = await post(`/api/corps/${demo.corp_id}/missions`, {
  requested_by: demo.alice_actor_id,
  preferred_adapter: 'github-copilot',
  preferred_model: model.id,
  budget_tokens: 200_000,
  title:
    'Create copilot-live-proof.txt containing exactly: GitHub Copilot SDK live adapter verified. Do not modify any other file.',
})
const launch = await post(
  `/api/corps/${demo.corp_id}/missions/${mission.mission_id}/launch`,
  { requested_by: demo.alice_actor_id },
)
const completed = await waitForRun(demo, launch.run_id)
assert.equal(completed.run.status, 'completed', completed.run.summary)
assert.equal(completed.run.model, model.id)
assert.ok(completed.run.workspace_path)
const proof = await readFile(
  path.join(completed.run.workspace_path, 'copilot-live-proof.txt'),
  'utf8',
)
assert.equal(proof.trim(), 'GitHub Copilot SDK live adapter verified.')
assert.equal(
  completed.state.snapshot.action_approvals.length,
  0,
  'worktree and isolated Copilot state operations should not require durable approval',
)

const report = {
  checked_at: new Date().toISOString(),
  sdk_version: '1.0.11',
  model_count: capability.models.length,
  models: capability.models,
  selected_model: model.id,
  run_id: completed.run.id,
  provider_session_id: completed.run.provider_session_id,
  run_status: completed.run.status,
  input_tokens: completed.run.input_tokens,
  output_tokens: completed.run.output_tokens,
  durable_approval_count: completed.state.snapshot.action_approvals.length,
  proof_file: 'copilot-live-proof.txt',
}
await writeFile(
  path.join(root, 'output', 'e2e-copilot-live.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(
  JSON.stringify(
    {
      ...report,
      models: report.models.map((candidate) => candidate.id),
    },
    null,
    2,
  ),
)
