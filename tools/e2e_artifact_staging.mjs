import assert from 'node:assert/strict'
import { execFile as execFileCallback, spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import {
  copyFile,
  mkdir,
  open,
  readFile,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises'
import path from 'node:path'
import { promisify } from 'node:util'

import {
  downloadVerifiedArtifact,
} from './artifact_client.mjs'

const execFile = promisify(execFileCallback)
const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const databaseUrl =
  process.env.DATABASE_URL ?? 'postgres://crony:crony@127.0.0.1:54329/crony'
const root = path.resolve(import.meta.dirname, '..')
const composeFile = path.join(root, 'deploy', 'compose', 'docker-compose.yml')
const artifactRoot = path.join(root, 'output', 'artifact-objects')
const pidPath =
  process.env.CRONY_TEST_SERVER_PID_FILE ??
  path.join(root, 'output', 'local-pids.json')
let psqlMode

async function request(pathname, init) {
  const response = await fetch(`${server}${pathname}`, init)
  const body = response.status === 204 ? null : await response.json()
  return { response, body }
}

async function post(pathname, body) {
  const result = await request(pathname, {
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

async function waitForSnapshot(demo, predicate, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const state = await snapshot(demo)
    const value = predicate(state)
    if (value) return { state, value }
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error('timed out waiting for artifact staging state')
}

function objectPath(key) {
  return path.join(artifactRoot, ...key.split('/'))
}

async function exists(filePath) {
  try {
    await stat(filePath)
    return true
  } catch (error) {
    if (error?.code === 'ENOENT') return false
    throw error
  }
}

async function waitForFile(filePath, expected, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if ((await exists(filePath)) === expected) return
    await new Promise((resolve) => setTimeout(resolve, 50))
  }
  throw new Error(
    `timed out waiting for ${filePath} to become ${expected ? 'present' : 'absent'}`,
  )
}

function waitForLiveEvent({
  corpId,
  actorId,
  afterSeq,
  predicate,
  trigger,
  timeoutMs = 10_000,
}) {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(
      `${server.replace(/^http/, 'ws')}/ws/corps/${corpId}` +
        `?actor_id=${actorId}&after_seq=${afterSeq}`,
    )
    const timeout = setTimeout(() => {
      socket.close()
      reject(new Error('timed out waiting for live artifact recovery event'))
    }, timeoutMs)

    function fail(error) {
      clearTimeout(timeout)
      socket.close()
      reject(error)
    }

    socket.onerror = () => fail(new Error('artifact recovery websocket failed'))
    socket.onmessage = (message) => {
      const payload = JSON.parse(message.data)
      if (payload.type === 'event' && predicate(payload.event)) {
        clearTimeout(timeout)
        socket.close()
        resolve(payload.event)
        return
      }
      if (payload.type === 'ready') {
        Promise.resolve()
          .then(trigger)
          .catch(fail)
      }
    }
  })
}

function sha256(bytes) {
  return createHash('sha256').update(bytes).digest('hex')
}

function artifactPayload(bytes, mediaType = 'text/plain') {
  return {
    sha256: sha256(bytes),
    bytes: bytes.length,
    media_type: mediaType,
    content_base64: bytes.toString('base64'),
  }
}

function sqlLiteral(value) {
  return `'${String(value).replaceAll("'", "''")}'`
}

async function psql(sql) {
  const invocation = await psqlInvocation()
  const { stdout } = await execFile(
    invocation.command,
    [
      ...invocation.args,
      '-v',
      'ON_ERROR_STOP=1',
      '-At',
      '-c',
      sql,
    ],
    {
      cwd: root,
      windowsHide: true,
      maxBuffer: 4 * 1024 * 1024,
    },
  )
  return stdout.trim()
}

async function lockRun(runId) {
  const invocation = await psqlInvocation()
  const child = spawn(
    invocation.command,
    [
      ...invocation.args,
      '-v',
      'ON_ERROR_STOP=1',
      '-q',
    ],
    {
      cwd: root,
      windowsHide: true,
      stdio: ['pipe', 'pipe', 'pipe'],
    },
  )
  let stdout = ''
  let stderr = ''
  child.stdout.setEncoding('utf8')
  child.stderr.setEncoding('utf8')
  child.stdout.on('data', (chunk) => {
    stdout += chunk
  })
  child.stderr.on('data', (chunk) => {
    stderr += chunk
  })
  child.stdin.write(
    `\\set ON_ERROR_STOP on\nBEGIN;\n` +
      `SELECT id FROM runs WHERE id = ${sqlLiteral(runId)}::uuid FOR UPDATE;\n` +
      `\\echo LOCKED\n`,
  )
  const deadline = Date.now() + 10_000
  while (!stdout.includes('LOCKED')) {
    if (child.exitCode !== null) {
      throw new Error(`run-lock psql exited early: ${stderr}`)
    }
    if (Date.now() >= deadline) {
      child.kill()
      throw new Error(`timed out acquiring run lock: ${stderr}`)
    }
    await new Promise((resolve) => setTimeout(resolve, 50))
  }
  return {
    async setStopAndRelease() {
      const exited = new Promise((resolve, reject) => {
        child.once('error', reject)
        child.once('exit', (code) => {
          if (code === 0) resolve()
          else reject(new Error(`run-lock psql exited ${code}: ${stderr}`))
        })
      })
      child.stdin.end(
        `UPDATE runs SET breaker_stage = 'stop' ` +
          `WHERE id = ${sqlLiteral(runId)}::uuid;\nCOMMIT;\n\\q\n`,
      )
      await exited
    },
  }
}

async function psqlInvocation() {
  if (!psqlMode) {
    try {
      await execFile('psql', ['--version'], {
        cwd: root,
        windowsHide: true,
      })
      psqlMode = 'direct'
    } catch {
      psqlMode = 'docker'
    }
  }
  if (psqlMode === 'direct') {
    return {
      command: 'psql',
      args: [databaseUrl],
    }
  }
  return {
    command: 'docker',
    args: [
      'compose',
      '-f',
      composeFile,
      'exec',
      '-T',
      'postgres',
      'psql',
      '-U',
      'crony',
      '-d',
      'crony',
    ],
  }
}

function connectRunner({ corpId, runnerId, credential }) {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(`${server.replace(/^http/, 'ws')}/ws/runner`)
    const connectionEpoch = crypto.randomUUID()
    const assignments = []
    const waiters = []
    const timeout = setTimeout(() => {
      socket.close()
      reject(new Error('timed out waiting for artifact test runner registration'))
    }, 10_000)

    function deliverAssignment(assignment) {
      const waiterIndex = waiters.findIndex(
        (waiter) => waiter.runId === assignment.run_id,
      )
      if (waiterIndex >= 0) {
        const [waiter] = waiters.splice(waiterIndex, 1)
        clearTimeout(waiter.timeout)
        waiter.resolve(assignment)
      } else {
        assignments.push(assignment)
      }
    }

    socket.onerror = () => {
      clearTimeout(timeout)
      reject(new Error('artifact test runner websocket failed'))
    }
    socket.onopen = () => {
      socket.send(
        JSON.stringify({
          type: 'register',
          runner_id: runnerId,
          corp_id: corpId,
          credential,
          connection_epoch: connectionEpoch,
          hostname: 'artifact-staging-e2e',
          os: process.platform,
          capabilities: [
            {
              name: 'fake-process',
              available: true,
              detail: 'controlled artifact staging runner',
              models: [],
            },
          ],
          active_runs: [],
        }),
      )
    }
    socket.onmessage = (message) => {
      const payload = JSON.parse(message.data)
      if (payload.type === 'registered') {
        clearTimeout(timeout)
        resolve({
          socket,
          connectionEpoch,
          runnerId,
          credential: payload.credential,
          waitForAssignment(runId, timeoutMs = 10_000) {
            const index = assignments.findIndex(
              (assignment) => assignment.run_id === runId,
            )
            if (index >= 0) {
              return Promise.resolve(assignments.splice(index, 1)[0])
            }
            return new Promise((assignmentResolve, assignmentReject) => {
              const assignmentTimeout = setTimeout(() => {
                const waiterIndex = waiters.findIndex(
                  (waiter) => waiter.runId === runId,
                )
                if (waiterIndex >= 0) waiters.splice(waiterIndex, 1)
                assignmentReject(
                  new Error(`timed out waiting for assignment ${runId}`),
                )
              }, timeoutMs)
              waiters.push({
                runId,
                resolve: assignmentResolve,
                timeout: assignmentTimeout,
              })
            })
          },
        })
      } else if (payload.type === 'registration_rejected') {
        clearTimeout(timeout)
        reject(new Error(`runner registration rejected: ${payload.reason}`))
      } else if (payload.type === 'start_run') {
        deliverAssignment(payload)
      }
    }
  })
}

function sendRunEvent(runner, assignment, eventType, payload, eventId) {
  const id = eventId ?? crypto.randomUUID()
  runner.socket.send(
    JSON.stringify({
      type: 'run_event',
      event_id: id,
      runner_id: runner.runnerId,
      corp_id: assignment.corp_id,
      connection_epoch: runner.connectionEpoch,
      run_id: assignment.run_id,
      agent_id: assignment.agent_id,
      assignment_token: assignment.assignment_token,
      event_type: eventType,
      payload,
    }),
  )
  return id
}

async function controlledRun(demo, runner, title) {
  const mission = await post(`/api/corps/${demo.corp_id}/missions`, {
    requested_by: demo.alice_actor_id,
    preferred_adapter: 'fake-process',
    title,
  })
  const launch = await post(
    `/api/corps/${demo.corp_id}/missions/${mission.mission_id}/launch`,
    { requested_by: demo.alice_actor_id },
  )
  const assignment = await runner.waitForAssignment(launch.run_id)
  assert.equal(assignment.run_id, launch.run_id)
  sendRunEvent(runner, assignment, 'run.started', {
    workspace: 'controlled-artifact-workspace',
    station: 'terminal',
  })
  await waitForSnapshot(
    demo,
    (state) =>
      state.snapshot.runs.find((run) => run.id === launch.run_id)?.status ===
      'running',
  )
  return { mission, launch, assignment }
}

async function uploadArtifact(
  demo,
  runner,
  controlled,
  bytes,
  eventId,
  duplicateEventId,
) {
  const payload = artifactPayload(bytes)
  const id = sendRunEvent(
    runner,
    controlled.assignment,
    'run.artifact_upload',
    payload,
    eventId,
  )
  if (duplicateEventId) {
    sendRunEvent(
      runner,
      controlled.assignment,
      'run.artifact_upload',
      payload,
      duplicateEventId,
    )
  }
  const applied = await waitForSnapshot(demo, (state) => {
    const run = state.snapshot.runs.find(
      (candidate) => candidate.id === controlled.launch.run_id,
    )
    return run?.artifact_id === id ? run : null
  })
  return {
    eventId: id,
    state: applied.state,
    run: applied.value,
    bytes: payload.bytes,
    mediaType: payload.media_type,
  }
}

async function completeVerifiedRun(demo, runner, controlled, artifact) {
  sendRunEvent(runner, controlled.assignment, 'run.verification_started', {
    check_count: 1,
  })
  sendRunEvent(runner, controlled.assignment, 'run.verification_evidence', {
    evidence_id: crypto.randomUUID(),
    check_index: 0,
    kind: 'artifact',
    status: 'passed',
    summary: 'controlled artifact exists with verified bytes',
    payload: {
      artifact_id: artifact.run.artifact_id,
      sha256: artifact.run.artifact_sha256,
      bytes: artifact.bytes,
      media_type: artifact.mediaType,
    },
  })
  sendRunEvent(runner, controlled.assignment, 'run.verification_passed', {
    summary: '1 of 1 verifier checks passed',
  })
  sendRunEvent(runner, controlled.assignment, 'run.completed', {
    summary: 'controlled artifact staging run completed',
  })
  return (
    await waitForSnapshot(demo, (state) => {
      const run = state.snapshot.runs.find(
        (candidate) => candidate.id === controlled.launch.run_id,
      )
      return run?.status === 'completed' ? run : null
    })
  ).value
}

async function acceptedArtifact(
  demo,
  runner,
  title,
  bytes,
  { duplicateDigest = false } = {},
) {
  const controlled = await controlledRun(demo, runner, title)
  const duplicateEventId = duplicateDigest ? crypto.randomUUID() : undefined
  const artifact = await uploadArtifact(
    demo,
    runner,
    controlled,
    bytes,
    crypto.randomUUID(),
    duplicateEventId,
  )
  const run = await completeVerifiedRun(demo, runner, controlled, artifact)
  return {
    controlled,
    run,
    sha256: sha256(bytes),
    finalKey: `corps/${demo.corp_id}/sha256/${sha256(bytes).slice(0, 2)}/${sha256(bytes)}`,
    bytes,
    duplicateEventId,
  }
}

async function installArtifactInsertFailure() {
  await psql(`
    DROP TRIGGER IF EXISTS crony_test_fail_artifact_insert ON artifacts;
    DROP FUNCTION IF EXISTS crony_test_fail_artifact_insert();
    DROP SEQUENCE IF EXISTS crony_test_artifact_insert_fail_seq;
    CREATE SEQUENCE crony_test_artifact_insert_fail_seq;
    CREATE FUNCTION crony_test_fail_artifact_insert()
    RETURNS trigger
    LANGUAGE plpgsql
    AS $$
    BEGIN
      PERFORM nextval('crony_test_artifact_insert_fail_seq');
      RAISE EXCEPTION 'injected artifact metadata insert failure';
    END;
    $$;
    CREATE TRIGGER crony_test_fail_artifact_insert
    BEFORE INSERT ON artifacts
    FOR EACH ROW EXECUTE FUNCTION crony_test_fail_artifact_insert();
  `)
}

async function removeArtifactInsertFailure() {
  await psql(`
    DROP TRIGGER IF EXISTS crony_test_fail_artifact_insert ON artifacts;
    DROP FUNCTION IF EXISTS crony_test_fail_artifact_insert();
    DROP SEQUENCE IF EXISTS crony_test_artifact_insert_fail_seq;
  `)
}

async function installArtifactFinalizeFailure() {
  await psql(`
    DROP TRIGGER IF EXISTS crony_test_fail_artifact_finalize ON artifacts;
    DROP FUNCTION IF EXISTS crony_test_fail_artifact_finalize();
    DROP SEQUENCE IF EXISTS crony_test_artifact_finalize_fail_seq;
    CREATE SEQUENCE crony_test_artifact_finalize_fail_seq;
    CREATE FUNCTION crony_test_fail_artifact_finalize()
    RETURNS trigger
    LANGUAGE plpgsql
    AS $$
    BEGIN
      PERFORM nextval('crony_test_artifact_finalize_fail_seq');
      RAISE EXCEPTION 'injected artifact metadata finalization failure';
    END;
    $$;
    CREATE TRIGGER crony_test_fail_artifact_finalize
    BEFORE UPDATE OF status ON artifacts
    FOR EACH ROW
    WHEN (OLD.status = 'staged' AND NEW.status = 'ready')
    EXECUTE FUNCTION crony_test_fail_artifact_finalize();
  `)
}

async function removeArtifactFinalizeFailure() {
  await psql(`
    DROP TRIGGER IF EXISTS crony_test_fail_artifact_finalize ON artifacts;
    DROP FUNCTION IF EXISTS crony_test_fail_artifact_finalize();
    DROP SEQUENCE IF EXISTS crony_test_artifact_finalize_fail_seq;
  `)
}

async function restartLocalServer() {
  const pidText = (await readFile(pidPath, 'utf8')).trim()
  const jsonPidFile = pidText.startsWith('{')
  const pidState = jsonPidFile
    ? JSON.parse(pidText)
    : { server: Number(pidText) }
  const serverPid = Number(pidState.server)
  assert.ok(Number.isSafeInteger(serverPid) && serverPid > 0)
  process.kill(serverPid)
  await new Promise((resolve) => setTimeout(resolve, 500))

  const serverUrl = new URL(server)
  const binary =
    process.env.CRONY_TEST_SERVER_BINARY ??
    path.join(
      root,
      'target',
      'debug',
      process.platform === 'win32' ? 'crony-server.exe' : 'crony-server',
    )
  const recoveryStdout = path.join(
    root,
    'output',
    'artifact-recovery-server.stdout.log',
  )
  const recoveryStderr = path.join(
    root,
    'output',
    'artifact-recovery-server.stderr.log',
  )
  await writeFile(recoveryStdout, '')
  await writeFile(recoveryStderr, '')
  const stdout = await open(recoveryStdout, 'a')
  const stderr = await open(recoveryStderr, 'a')
  const child = spawn(
    binary,
    [
      '--bind',
      `${serverUrl.hostname}:${serverUrl.port}`,
      '--database-url',
      databaseUrl,
      '--artifact-recovery-grace-secs',
      '0',
      '--artifact-recovery-interval-secs',
      '1',
    ],
    {
      cwd: root,
      detached: true,
      windowsHide: true,
      stdio: ['ignore', stdout.fd, stderr.fd],
    },
  )
  await writeFile(
    pidPath,
    jsonPidFile
      ? `${JSON.stringify({ ...pidState, server: child.pid }, null, 2)}\n`
      : `${child.pid}\n`,
  )
  child.unref()
  await stdout.close()
  await stderr.close()

  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    try {
      const health = await fetch(`${server}/health`).then((response) =>
        response.json(),
      )
      if (health.status === 'ok' && health.runners >= 1) return
    } catch {
      // Server recovery is still running.
    }
    await new Promise((resolve) => setTimeout(resolve, 200))
  }
  const recoveryError = await readFile(
    recoveryStderr,
    'utf8',
  ).catch(() => '')
  throw new Error(`server did not recover staged artifacts: ${recoveryError}`)
}

const demo = await post('/api/demo/reset', {})
const runnerId = `aaa-artifact-staging-${crypto.randomUUID()}`
const enrollment = await post(`/api/corps/${demo.corp_id}/runners/enroll`, {
  actor_id: demo.alice_actor_id,
  runner_id: runnerId,
  expires_in_seconds: 600,
})
const runner = await connectRunner({
  corpId: demo.corp_id,
  runnerId,
  credential: enrollment.enrollment_token,
})

const sharedBytes = Buffer.from(`shared digest ${crypto.randomUUID()}\n`, 'utf8')
const accepted = await acceptedArtifact(
  demo,
  runner,
  'Accept one shared-digest artifact before a competing rejection.',
  sharedBytes,
  { duplicateDigest: true },
)
const acceptedDownload = await downloadVerifiedArtifact(server, demo, accepted.run)
assert.deepEqual(acceptedDownload, sharedBytes)
assert.equal(
  await psql(
    `SELECT count(*) FROM artifacts WHERE run_id = ` +
      `${sqlLiteral(accepted.run.id)}::uuid AND sha256 = ${sqlLiteral(accepted.sha256)};`,
  ),
  '1',
)

const rejected = await controlledRun(
  demo,
  runner,
  'Reject the same digest after bytes enter staging.',
)
const rejectedEventId = crypto.randomUUID()
const rejectedStageKey = `staging/corps/${demo.corp_id}/${rejectedEventId}`
const rejectedStagePath = objectPath(rejectedStageKey)
const runLock = await lockRun(rejected.launch.run_id)
sendRunEvent(
  runner,
  rejected.assignment,
  'run.artifact_upload',
  artifactPayload(sharedBytes),
  rejectedEventId,
)
await new Promise((resolve) => setTimeout(resolve, 250))
assert.equal(await exists(rejectedStagePath), false)
await runLock.setStopAndRelease()
await waitForSnapshot(
  demo,
  (state) =>
    state.snapshot.runs.find((run) => run.id === rejected.launch.run_id)
      ?.status === 'failed',
)
await waitForFile(rejectedStagePath, false)
assert.deepEqual(
  await downloadVerifiedArtifact(server, demo, accepted.run),
  sharedBytes,
)
assert.equal(
  await psql(
    `SELECT count(*) FROM artifacts WHERE corp_id = ${sqlLiteral(demo.corp_id)}::uuid ` +
      `AND sha256 = ${sqlLiteral(accepted.sha256)};`,
  ),
  '1',
)

await post('/api/demo/reset', {})
const databaseFailureBytes = Buffer.from(
  `database failure ${crypto.randomUUID()}\n`,
  'utf8',
)
let databaseFailure
let databaseFailureEventId
const databaseFailureFinalPath = objectPath(
  `corps/${demo.corp_id}/sha256/${sha256(databaseFailureBytes).slice(0, 2)}/${sha256(databaseFailureBytes)}`,
)
let databaseFailureActualStagePath
await installArtifactInsertFailure()
try {
  databaseFailure = await controlledRun(
    demo,
    runner,
    'Reject staged bytes when artifact metadata insertion fails.',
  )
  databaseFailureEventId = crypto.randomUUID()
  databaseFailureActualStagePath = objectPath(
    `staging/corps/${demo.corp_id}/${databaseFailureEventId}`,
  )
  sendRunEvent(
    runner,
    databaseFailure.assignment,
    'run.artifact_upload',
    artifactPayload(databaseFailureBytes),
    databaseFailureEventId,
  )
  await waitForSnapshot(
    demo,
    (state) =>
      state.snapshot.runs.find(
        (run) => run.id === databaseFailure.launch.run_id,
      )?.status === 'failed',
  )
  assert.equal(
    await psql('SELECT last_value FROM crony_test_artifact_insert_fail_seq;'),
    '1',
  )
  await waitForFile(databaseFailureActualStagePath, false)
  assert.equal(await exists(databaseFailureFinalPath), false)
  assert.equal(
    await psql(
      `SELECT count(*) FROM artifacts WHERE id = ${sqlLiteral(databaseFailureEventId)}::uuid;`,
    ),
    '0',
  )
} finally {
  await removeArtifactInsertFailure()
}

await post('/api/demo/reset', {})
const recoverable = await acceptedArtifact(
  demo,
  runner,
  'Recover staged metadata and bytes after restart.',
  Buffer.from(`recoverable ${crypto.randomUUID()}\n`, 'utf8'),
)
const missing = await acceptedArtifact(
  demo,
  runner,
  'Reject staged metadata when both objects are missing.',
  Buffer.from(`missing ${crypto.randomUUID()}\n`, 'utf8'),
)
const cleanupRetry = await acceptedArtifact(
  demo,
  runner,
  'Retry cleanup for a ready artifact staging key.',
  Buffer.from(`cleanup ${crypto.randomUUID()}\n`, 'utf8'),
)
const finalizeFailureBytes = Buffer.from(
  `finalize failure ${crypto.randomUUID()}\n`,
  'utf8',
)
let finalizeFailure
let finalizeFailureEventId
await installArtifactFinalizeFailure()
try {
  finalizeFailure = await controlledRun(
    demo,
    runner,
    'Recover metadata finalization after staged and final bytes are durable.',
  )
  finalizeFailureEventId = crypto.randomUUID()
  sendRunEvent(
    runner,
    finalizeFailure.assignment,
    'run.artifact_upload',
    artifactPayload(finalizeFailureBytes),
    finalizeFailureEventId,
  )
  await waitForSnapshot(
    demo,
    (state) =>
      state.snapshot.runs.find(
        (run) => run.id === finalizeFailure.launch.run_id,
      )?.status === 'failed',
  )
  assert.equal(
    await psql('SELECT last_value FROM crony_test_artifact_finalize_fail_seq;'),
    '1',
  )
  assert.match(
    await psql(
      `SELECT status || '|' || COALESCE(staging_key, '') FROM artifacts ` +
        `WHERE id = ${sqlLiteral(finalizeFailureEventId)}::uuid;`,
    ),
    /^staged\|staging\/corps\//,
  )
} finally {
  await removeArtifactFinalizeFailure()
}

const recoverableStageKey = `staging/corps/${demo.corp_id}/${recoverable.run.artifact_id}`
const missingStageKey = `staging/corps/${demo.corp_id}/${missing.run.artifact_id}`
const cleanupStageKey = `staging/corps/${demo.corp_id}/${cleanupRetry.run.artifact_id}`
const orphanStageKey = `staging/corps/${demo.corp_id}/${crypto.randomUUID()}`
const finalizeFailureStageKey = `staging/corps/${demo.corp_id}/${finalizeFailureEventId}`
const recoverableFinalPath = objectPath(recoverable.finalKey)
const missingFinalPath = objectPath(missing.finalKey)
const cleanupFinalPath = objectPath(cleanupRetry.finalKey)
const recoverableStagePath = objectPath(recoverableStageKey)
const missingStagePath = objectPath(missingStageKey)
const cleanupStagePath = objectPath(cleanupStageKey)
const orphanStagePath = objectPath(orphanStageKey)
const finalizeFailureStagePath = objectPath(finalizeFailureStageKey)
const finalizeFailureFinalPath = objectPath(
  `corps/${demo.corp_id}/sha256/${sha256(finalizeFailureBytes).slice(0, 2)}/${sha256(finalizeFailureBytes)}`,
)
assert.equal(await exists(finalizeFailureStagePath), true)
assert.equal(await exists(finalizeFailureFinalPath), true)

await mkdir(path.dirname(recoverableStagePath), { recursive: true })
await copyFile(recoverableFinalPath, recoverableStagePath)
await rm(recoverableFinalPath)
await rm(missingFinalPath)
await copyFile(cleanupFinalPath, cleanupStagePath)
await writeFile(orphanStagePath, 'orphan staging bytes')
await rm(missingStagePath, { force: true })

await psql(`
  BEGIN;
  DELETE FROM events
  WHERE idempotency_key IN (
    ${sqlLiteral(`artifact:${recoverable.run.artifact_id}:ready`)},
    ${sqlLiteral(`artifact:${missing.run.artifact_id}:ready`)}
  );
  UPDATE runs
  SET artifact_id = NULL,
      artifact_uri = NULL,
      artifact_media_type = NULL,
      artifact_signature = NULL,
      artifact_sha256 = NULL
  WHERE id IN (
    ${sqlLiteral(recoverable.run.id)}::uuid,
    ${sqlLiteral(missing.run.id)}::uuid
  );
  UPDATE artifacts
  SET status = 'staged',
      staging_key = ${sqlLiteral(recoverableStageKey)},
      finalized_at = NULL,
      rejection_reason = NULL
  WHERE id = ${sqlLiteral(recoverable.run.artifact_id)}::uuid;
  UPDATE artifacts
  SET status = 'staged',
      staging_key = ${sqlLiteral(missingStageKey)},
      finalized_at = NULL,
      rejection_reason = NULL
  WHERE id = ${sqlLiteral(missing.run.artifact_id)}::uuid;
  UPDATE artifacts
  SET staging_key = ${sqlLiteral(cleanupStageKey)}
  WHERE id = ${sqlLiteral(cleanupRetry.run.artifact_id)}::uuid;
  COMMIT;
`)
assert.equal(await exists(recoverableStagePath), true)
assert.equal(await exists(recoverableFinalPath), false)
assert.equal(await exists(missingStagePath), false)
assert.equal(await exists(missingFinalPath), false)
assert.equal(await exists(cleanupStagePath), true)
assert.equal(await exists(cleanupFinalPath), true)
assert.equal(await exists(orphanStagePath), true)
assert.equal(await exists(finalizeFailureStagePath), true)
assert.equal(await exists(finalizeFailureFinalPath), true)

runner.socket.close()
await restartLocalServer()

assert.equal(
  await psql(
    `SELECT status || '|' || COALESCE(staging_key, '') FROM artifacts ` +
      `WHERE id = ${sqlLiteral(recoverable.run.artifact_id)}::uuid;`,
  ),
  'ready|',
)
assert.equal(await exists(recoverableFinalPath), true)
assert.equal(await exists(recoverableStagePath), false)
assert.equal(
  await psql(
    `SELECT artifact_id::text FROM runs ` +
      `WHERE id = ${sqlLiteral(recoverable.run.id)}::uuid;`,
  ),
  recoverable.run.artifact_id,
)
assert.equal(
  await psql(
    `SELECT count(*) FROM events WHERE idempotency_key = ` +
      `${sqlLiteral(`artifact:${recoverable.run.artifact_id}:ready`)} ` +
      `AND type = 'run.artifact';`,
  ),
  '1',
)

assert.equal(
  await psql(
    `SELECT count(*) FROM artifacts WHERE id = ` +
      `${sqlLiteral(missing.run.artifact_id)}::uuid;`,
  ),
  '0',
)
assert.equal(await exists(missingFinalPath), false)
assert.equal(await exists(missingStagePath), false)
assert.equal(
  await psql(
    `SELECT count(*) FROM events WHERE idempotency_key = ` +
      `${sqlLiteral(`artifact:${missing.run.artifact_id}:ready`)};`,
  ),
  '0',
)

assert.equal(
  await psql(
    `SELECT status || '|' || COALESCE(staging_key, '') FROM artifacts ` +
      `WHERE id = ${sqlLiteral(cleanupRetry.run.artifact_id)}::uuid;`,
  ),
  'ready|',
)
assert.equal(await exists(cleanupFinalPath), true)
assert.equal(await exists(cleanupStagePath), false)
assert.equal(await exists(orphanStagePath), false)
assert.equal(
  await psql(
    `SELECT status || '|' || COALESCE(staging_key, '') FROM artifacts ` +
      `WHERE id = ${sqlLiteral(finalizeFailureEventId)}::uuid;`,
  ),
  'ready|',
)
assert.equal(await exists(finalizeFailureStagePath), false)
assert.equal(await exists(finalizeFailureFinalPath), true)
assert.equal(
  await psql(
    `SELECT artifact_id::text FROM runs WHERE id = ` +
      `${sqlLiteral(finalizeFailure.launch.run_id)}::uuid;`,
  ),
  finalizeFailureEventId,
)
assert.equal(
  await psql(
    `SELECT count(*) FROM events WHERE idempotency_key = ` +
      `${sqlLiteral(`artifact:${finalizeFailureEventId}:ready`)};`,
  ),
  '1',
)

const periodicRecoveryCursor = Number(
  await psql(
    `SELECT COALESCE(max(seq), 0) FROM events ` +
      `WHERE corp_id = ${sqlLiteral(demo.corp_id)}::uuid;`,
  ),
)
assert.ok(Number.isSafeInteger(periodicRecoveryCursor))
const periodicRecoveryEvent = await waitForLiveEvent({
  corpId: demo.corp_id,
  actorId: demo.alice_actor_id,
  afterSeq: periodicRecoveryCursor,
  predicate: (event) =>
    event.type === 'run.artifact' &&
    event.payload.artifact_id === cleanupRetry.run.artifact_id,
  trigger: () =>
    psql(`
      BEGIN;
      DELETE FROM events
      WHERE idempotency_key =
        ${sqlLiteral(`artifact:${cleanupRetry.run.artifact_id}:ready`)};
      UPDATE runs
      SET artifact_id = NULL,
          artifact_uri = NULL,
          artifact_media_type = NULL,
          artifact_signature = NULL,
          artifact_sha256 = NULL
      WHERE id = ${sqlLiteral(cleanupRetry.run.id)}::uuid;
      UPDATE artifacts
      SET status = 'staged',
          staging_key = ${sqlLiteral(cleanupStageKey)},
          finalized_at = NULL,
          rejection_reason = NULL,
          created_at = now() - interval '1 second'
      WHERE id = ${sqlLiteral(cleanupRetry.run.artifact_id)}::uuid;
      COMMIT;
    `),
})
assert.equal(periodicRecoveryEvent.aggregate_id, cleanupRetry.run.id)
assert.equal(
  await psql(
    `SELECT status || '|' || COALESCE(staging_key, '') FROM artifacts ` +
      `WHERE id = ${sqlLiteral(cleanupRetry.run.artifact_id)}::uuid;`,
  ),
  'ready|',
)
assert.equal(
  await psql(
    `SELECT artifact_id::text FROM runs ` +
      `WHERE id = ${sqlLiteral(cleanupRetry.run.id)}::uuid;`,
  ),
  cleanupRetry.run.artifact_id,
)
assert.equal(await exists(cleanupFinalPath), true)

const report = {
  checked_at: new Date().toISOString(),
  accepted_shared_digest: {
    run_id: accepted.run.id,
    artifact_id: accepted.run.artifact_id,
    sha256: accepted.sha256,
    duplicate_event_id: accepted.duplicateEventId,
    distinct_duplicate_event_collapsed: true,
    remains_downloadable_after_rejected_duplicate: true,
  },
  breaker_rejection_before_staging: {
    run_id: rejected.launch.run_id,
    event_id: rejectedEventId,
    staged_bytes_never_written: true,
    final_shared_object_preserved: true,
  },
  database_reservation_failure: {
    run_id: databaseFailure.launch.run_id,
    event_id: databaseFailureEventId,
    insert_trigger_fired: true,
    staged_bytes_never_written: true,
    final_object_absent: true,
  },
  database_failure_after_object_publication: {
    run_id: finalizeFailure.launch.run_id,
    event_id: finalizeFailureEventId,
    finalization_trigger_fired: true,
    staged_metadata_recovered: true,
    final_object_preserved: true,
  },
  restart_recovery: {
    recovered_artifact_id: recoverable.run.artifact_id,
    missing_object_artifact_id: missing.run.artifact_id,
    cleanup_retry_artifact_id: cleanupRetry.run.artifact_id,
    orphan_staging_key: orphanStageKey,
    staged_metadata_finalized: true,
    missing_object_reservation_released: true,
    ready_staging_cleanup_retried: true,
    orphan_staging_removed: true,
  },
  periodic_recovery: {
    recovered_artifact_id: cleanupRetry.run.artifact_id,
    websocket_event_seq: periodicRecoveryEvent.seq,
    connected_client_notified: true,
  },
}
await writeFile(
  path.join(root, 'output', 'e2e-artifact-staging.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
