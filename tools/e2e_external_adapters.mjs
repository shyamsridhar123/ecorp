import assert from 'node:assert/strict'
import { writeFile } from 'node:fs/promises'
import path from 'node:path'
import { downloadVerifiedArtifact } from './artifact_client.mjs'

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
  return response.json()
}

async function runProvider(demo, adapter) {
  const mission = await post(`/api/corps/${demo.corp_id}/missions`, {
    requested_by: demo.alice_actor_id,
    preferred_adapter: adapter,
    title: `Common provider parity sample for ${adapter}`,
  })
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
const results = []
for (const adapter of ['claude-code', 'opencode']) {
  results.push(await runProvider(demo, adapter))
}
assert.equal(new Set(results.map((result) => result.artifact_sha256)).size, 2)

const report = {
  checked_at: new Date().toISOString(),
  common_sample: true,
  providers: results,
}
await writeFile(
  path.join(root, 'output', 'e2e-external-adapters.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
