import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'

export async function fetchArtifact(server, corpId, actorId, artifactUri) {
  assert.ok(artifactUri, 'run omitted artifact URI')
  const separator = artifactUri.includes('?') ? '&' : '?'
  return fetch(`${server}${artifactUri}${separator}actor_id=${actorId}`, {
    headers: { accept: 'application/octet-stream' },
  })
}

export async function downloadVerifiedArtifact(server, demo, run) {
  assert.equal(
    Object.hasOwn(run, 'artifact_path'),
    false,
    'shared run state exposed a runner-local artifact path',
  )
  assert.ok(run.artifact_id, 'run omitted artifact id')
  assert.ok(run.artifact_uri, 'run omitted artifact URI')
  assert.ok(run.artifact_sha256, 'run omitted artifact digest')
  assert.ok(run.artifact_media_type, 'run omitted artifact media type')
  assert.ok(run.artifact_signature, 'run omitted artifact provenance signature')

  const response = await fetchArtifact(
    server,
    demo.corp_id,
    demo.alice_actor_id,
    run.artifact_uri,
  )
  if (response.status !== 200) {
    throw new Error(
      `artifact download returned ${response.status}: ${await response.text()}`,
    )
  }
  assert.equal(
    response.headers.get('content-type'),
    run.artifact_media_type,
  )
  assert.equal(
    response.headers.get('x-content-type-options'),
    'nosniff',
  )
  assert.equal(
    response.headers.get('x-crony-artifact-signature'),
    run.artifact_signature,
  )
  const bytes = Buffer.from(await response.arrayBuffer())
  assert.equal(
    createHash('sha256').update(bytes).digest('hex'),
    run.artifact_sha256,
  )
  return bytes
}
