import assert from 'node:assert/strict'
import { writeFile } from 'node:fs/promises'
import path from 'node:path'
import {
  downloadVerifiedArtifact,
  fetchArtifact,
} from './artifact_client.mjs'

const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const root = path.resolve(import.meta.dirname, '..')

async function request(url, init) {
  const response = await fetch(`${server}${url}`, init)
  const body =
    response.status === 204
      ? null
      : await response
          .clone()
          .json()
          .catch(() => null)
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
  const result = await request(
    `/api/corps/${demo.corp_id}/snapshot?actor_id=${demo.alice_actor_id}`,
  )
  assert.equal(result.response.status, 200)
  return result.body
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
  throw new Error(`timed out waiting for artifact run ${runId}`)
}

const demo = await post('/api/demo/reset', {})
const mission = await post(`/api/corps/${demo.corp_id}/missions`, {
  requested_by: demo.alice_actor_id,
  preferred_adapter: 'fake-process',
  title: 'Create a durable content-addressed artifact with signed provenance.',
})
const launch = await post(
  `/api/corps/${demo.corp_id}/missions/${mission.mission_id}/launch`,
  { requested_by: demo.alice_actor_id },
)
const completed = await waitForRun(demo, launch.run_id)
assert.equal(completed.run.status, 'completed', completed.run.summary)
const bytes = await downloadVerifiedArtifact(server, demo, completed.run)
assert.ok(bytes.length > 0)

const artifactEvent = completed.state.snapshot.events.find(
  (event) =>
    event.type === 'run.artifact' &&
    event.aggregate_id === completed.run.id,
)
assert.ok(artifactEvent, 'durable artifact event was not recorded')
assert.equal(artifactEvent.payload.artifact_id, completed.run.artifact_id)
assert.equal(artifactEvent.payload.uri, completed.run.artifact_uri)
assert.equal(artifactEvent.payload.sha256, completed.run.artifact_sha256)
assert.equal(
  artifactEvent.payload.provenance_signature,
  completed.run.artifact_signature,
)
assert.equal(artifactEvent.payload.producer_agent_id, completed.run.agent_id)
assert.equal(artifactEvent.payload.producer_runner_id, completed.run.runner_id)
assert.equal(artifactEvent.payload.verifier, 'crony-server:artifact-ingest-v1')
assert.ok(artifactEvent.payload.retention_until)
assert.equal(Object.hasOwn(artifactEvent.payload, 'content_base64'), false)
assert.equal(Object.hasOwn(artifactEvent.payload, 'path'), false)
assert.equal(Object.hasOwn(artifactEvent.payload, 'object_key'), false)

const verificationEvidence = completed.state.snapshot.verification_evidence.find(
  (evidence) =>
    evidence.run_id === completed.run.id && evidence.kind === 'artifact',
)
assert.ok(verificationEvidence, 'artifact verification evidence was not recorded')
assert.equal(Object.hasOwn(verificationEvidence.payload, 'path'), false)
assert.equal(
  verificationEvidence.payload.artifact_id,
  completed.run.artifact_id,
)
assert.equal(
  verificationEvidence.summary.includes('output'),
  false,
  'verification summary exposed a runner-local path',
)

const unauthorized = await fetchArtifact(
  server,
  demo.corp_id,
  demo.eve_actor_id,
  completed.run.artifact_uri,
)
assert.equal(unauthorized.status, 404)

const report = {
  checked_at: new Date().toISOString(),
  run_id: completed.run.id,
  artifact_id: completed.run.artifact_id,
  artifact_uri: completed.run.artifact_uri,
  artifact_sha256: completed.run.artifact_sha256,
  artifact_media_type: completed.run.artifact_media_type,
  provenance_signature: completed.run.artifact_signature,
  verifier: artifactEvent.payload.verifier,
  retention_until: artifactEvent.payload.retention_until,
  downloaded_bytes: bytes.length,
  unauthorized_download_status: unauthorized.status,
  local_path_exposed: Object.hasOwn(completed.run, 'artifact_path'),
}
await writeFile(
  path.join(root, 'output', 'e2e-artifacts.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
