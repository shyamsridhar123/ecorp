import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import {
  execFile as execFileCallback,
  execFileSync,
  spawn,
} from 'node:child_process'
import {
  existsSync,
  openSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { readFile, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { promisify } from 'node:util'

const execFile = promisify(execFileCallback)
const root = path.resolve(import.meta.dirname, '..')
const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const databaseUrl =
  process.env.DATABASE_URL ?? 'postgres://crony:crony@127.0.0.1:54329/crony'
const binary =
  process.env.CRONY_CLI_BINARY ??
  path.join(
    root,
    'target',
    'debug',
    process.platform === 'win32' ? 'crony-cli.exe' : 'crony-cli',
  )
const fakeGithub = path.join(root, 'tools', 'fake_github_cli.mjs')
const nonce = crypto.randomUUID()
const statePath = path.join(root, 'output', `fake-github-publication-${nonce}.json`)
const remotePath = path.join(root, 'output', `fake-publication-remote-${nonce}.git`)
const reportPath = path.join(root, 'output', 'e2e-factory-publication.json')
const publisherToken = `publisher-secret-${crypto.randomUUID()}`
const authorizationId = crypto.randomUUID()
const publisherCredentials = new Map()
const publisherCredentialFiles = new Map()
const publisherCredentialIds = new Map()
const publisherCredentialPaths = []
const publisherCredentialSecrets = []
process.on('exit', () => {
  for (const credentialPath of publisherCredentialPaths) {
    rmSync(credentialPath, { force: true })
  }
})
const sourceBaseCommit = execFileSync('git', ['rev-parse', 'HEAD'], {
  cwd: root,
  encoding: 'utf8',
  windowsHide: true,
}).trim()
let psqlMode

async function request(url, init) {
  const response = await fetch(`${server}${url}`, init)
  const body = await response
    .clone()
    .json()
    .catch(() => null)
  return { response, body }
}

async function requestOk(url, init) {
  const result = await request(url, init)
  if (!result.response.ok) {
    throw new Error(
      `${init?.method ?? 'GET'} ${url}: ${result.response.status} ${JSON.stringify(result.body)}`,
    )
  }
  return result.body
}

function post(url, body) {
  return request(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
}

function postOk(url, body) {
  return requestOk(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
}

function publisherCredentialHeaders(publisherId) {
  const credential = publisherCredentials.get(publisherId)
  assert.ok(credential, `missing enrolled credential for ${publisherId}`)
  return publisherHeadersForCredential(credential)
}

function publisherHeadersForCredential(credential) {
  return {
    'content-type': 'application/json',
    'x-crony-publication-publisher-credential': credential,
  }
}

function postWithPublisherCredential(url, body, credential) {
  return request(url, {
    method: 'POST',
    headers: publisherHeadersForCredential(credential),
    body: JSON.stringify(body),
  })
}

function postPublisher(url, body, publisherId) {
  return request(url, {
    method: 'POST',
    headers: publisherCredentialHeaders(publisherId),
    body: JSON.stringify(body),
  })
}

function postPublisherOk(url, body, publisherId) {
  return requestOk(url, {
    method: 'POST',
    headers: publisherCredentialHeaders(publisherId),
    body: JSON.stringify(body),
  })
}

function postPublicationStart(url, body) {
  return postPublisher(url, body, body.publisher_id)
}

function postPublicationStartOk(url, body) {
  return postPublisherOk(url, body, body.publisher_id)
}

function snapshot(demo, actorId = demo.alice_actor_id) {
  return requestOk(
    `/api/corps/${demo.corp_id}/snapshot?actor_id=${actorId}`,
  )
}

function publicationContext(demo, workItemId, actorId = demo.alice_actor_id) {
  return requestOk(
    `/api/corps/${demo.corp_id}/factory/work-items/${workItemId}/publication-context?actor_id=${actorId}`,
  )
}

async function waitForMission(demo, missionId, timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const state = await snapshot(demo)
    const mission = state.snapshot.missions.find((item) => item.id === missionId)
    if (mission && ['completed', 'failed', 'cancelled'].includes(mission.status)) {
      return { state, mission }
    }
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error(`timed out waiting for publication mission ${missionId}`)
}

async function runController(demo, issueNumber) {
  const { stdout } = await execFile(
    binary,
    [
      'factory',
      demo.corp_id,
      demo.alice_actor_id,
      '--owner',
      'acme',
      '--project-number',
      '7',
      '--repository',
      'shyamsridhar123/ecorp',
      '--source-repository-path',
      root,
      '--source-base-ref',
      'HEAD',
      '--publication-base-ref',
      'HEAD',
      '--adapter',
      'fake-process',
      '--strategy',
      'single',
      '--budget-tokens',
      '20000',
      '--budget-cost-microusd',
      '1000000',
      '--lease-seconds',
      '300',
      '--issue',
      String(issueNumber),
      '--github-cli',
      process.execPath,
    ],
    {
      cwd: root,
      env: {
        ...process.env,
        ECORP_GITHUB_CLI_PREFIX_ARGS_JSON: JSON.stringify([fakeGithub]),
        ECORP_FAKE_GITHUB_STATE: statePath,
      },
      maxBuffer: 4 * 1024 * 1024,
      windowsHide: true,
    },
  )
  return JSON.parse(stdout)
}

async function runPublisher(
  demo,
  workItemId,
  {
    crashAfter,
    idempotencyKey,
    expectCrash = false,
    expectFailure,
    branch,
    bodyFile,
    omitAuthorizationId = false,
    actorId = demo.alice_actor_id,
    sourceDeliverableId,
    title,
    repository,
    publisherId = 'trusted-publication-e2e',
    publisherCredentialFile,
    authorizationReason =
      'Publication E2E authorizes review-only branch and pull request creation.',
    leaseSeconds = 5,
  } = {},
) {
  const args = [
    'factory-publish',
    demo.corp_id,
    actorId,
    workItemId,
    '--authorization-reason',
    authorizationReason,
    '--publisher-id',
    publisherId,
    '--lease-seconds',
    String(leaseSeconds),
    '--wait-seconds',
    '60',
    '--github-cli',
    process.execPath,
  ]
  const credentialFile =
    publisherCredentialFile ?? publisherCredentialFiles.get(publisherId)
  if (credentialFile) {
    args.push('--publisher-credential-file', credentialFile)
  }
  if (!omitAuthorizationId) {
    args.push('--authorization-id', authorizationId)
  }
  if (sourceDeliverableId) {
    args.push('--source-deliverable-id', sourceDeliverableId)
  }
  if (title) args.push('--title', title)
  if (repository) args.push('--repository', repository)
  if (idempotencyKey) args.push('--idempotency-key', idempotencyKey)
  if (branch) args.push('--branch', branch)
  if (bodyFile) args.push('--body-file', bodyFile)
  try {
    const { stdout, stderr } = await execFile(binary, args, {
      cwd: root,
      env: {
        ...process.env,
        GH_TOKEN: publisherToken,
        ECORP_FAKE_GITHUB_EXPECT_TOKEN: publisherToken,
        ECORP_GITHUB_CLI_PREFIX_ARGS_JSON: JSON.stringify([fakeGithub]),
        ECORP_FAKE_GITHUB_STATE: statePath,
        ECORP_PUBLICATION_TEST_REMOTE_URL: remotePath,
        ECORP_PUBLICATION_EFFECT_LEASE_SECONDS: '30',
        ECORP_GITHUB_COMMAND_TIMEOUT_MS: '1000',
        ECORP_SOURCE_GIT_COMMAND_TIMEOUT_MS: '5000',
        ...(crashAfter
          ? { ECORP_PUBLICATION_TEST_CRASH_AFTER: crashAfter }
          : {}),
      },
      maxBuffer: 8 * 1024 * 1024,
      windowsHide: true,
    })
    if (expectCrash) {
      throw new Error(`publisher did not crash at ${crashAfter}`)
    }
    assert.equal(stdout.includes(publisherToken), false)
    assert.equal(stderr.includes(publisherToken), false)
    for (const credential of publisherCredentialSecrets) {
      assert.equal(stdout.includes(credential), false)
      assert.equal(stderr.includes(credential), false)
    }
    return JSON.parse(stdout)
  } catch (error) {
    assert.equal(String(error.stdout ?? '').includes(publisherToken), false)
    assert.equal(String(error.stderr ?? '').includes(publisherToken), false)
    for (const credential of publisherCredentialSecrets) {
      assert.equal(String(error.stdout ?? '').includes(credential), false)
      assert.equal(String(error.stderr ?? '').includes(credential), false)
    }
    if (expectCrash && error.code === 86) {
      return { crashed: true, stage: crashAfter }
    }
    if (expectFailure) {
      assert.match(String(error.stderr ?? ''), expectFailure)
      return { failed: true, detail: String(error.stderr ?? '').trim() }
    }
    throw error
  }
}

async function restartLocalServer() {
  const pidPath =
    process.env.CRONY_TEST_SERVER_PID_FILE ??
    path.join(root, 'output', 'local-pids.json')
  if (!existsSync(pidPath)) {
    throw new Error(`test-owned server PID file does not exist: ${pidPath}`)
  }
  const jsonPidFile = pidPath.endsWith('.json')
  const pidState = jsonPidFile
    ? JSON.parse(readFileSync(pidPath, 'utf8'))
    : { server: Number(readFileSync(pidPath, 'utf8').trim()) }
  const serverPid = Number(pidState.server)
  if (!Number.isSafeInteger(serverPid) || serverPid <= 0) {
    throw new Error(`test-owned server PID is invalid: ${serverPid}`)
  }
  process.kill(serverPid, 0)
  process.kill(serverPid)
  await new Promise((resolve) => setTimeout(resolve, 500))

  const serverUrl = new URL(server)
  const serverBinary =
    process.env.CRONY_TEST_SERVER_BINARY ??
    path.join(
      root,
      'target',
      'debug',
      process.platform === 'win32' ? 'crony-server.exe' : 'crony-server',
    )
  const logDir =
    process.env.CRONY_TEST_SERVER_LOG_DIR ?? path.dirname(path.resolve(pidPath))
  const stdout = openSync(
    path.join(logDir, 'publication-server-restart.stdout.log'),
    'a',
  )
  const stderr = openSync(
    path.join(logDir, 'publication-server-restart.stderr.log'),
    'a',
  )
  const child = spawn(
    serverBinary,
    [
      '--bind',
      `${serverUrl.hostname}:${serverUrl.port}`,
      '--database-url',
      databaseUrl,
    ],
    {
      cwd: root,
      detached: true,
      windowsHide: true,
      stdio: ['ignore', stdout, stderr],
    },
  )
  if (jsonPidFile) {
    writeFileSync(
      pidPath,
      `${JSON.stringify({ ...pidState, server: child.pid }, null, 2)}\n`,
    )
  } else {
    writeFileSync(pidPath, `${child.pid}\n`)
  }
  child.unref()

  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    try {
      const health = await fetch(`${server}/health`).then((response) =>
        response.json(),
      )
      if (health.status === 'ok' && health.runners >= 1) return child.pid
    } catch {
      // The test-owned server is restarting and the runner is reconnecting.
    }
    await new Promise((resolve) => setTimeout(resolve, 200))
  }
  throw new Error('server or runner did not recover after publication restart')
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
      maxBuffer: 8 * 1024 * 1024,
    },
  )
  return stdout.trim()
}

async function psqlInvocation() {
  if (!psqlMode) {
    try {
      await execFile('psql', ['--version'], { cwd: root, windowsHide: true })
      psqlMode = 'direct'
    } catch {
      psqlMode = 'docker'
    }
  }
  if (psqlMode === 'direct') {
    return { command: 'psql', args: [databaseUrl] }
  }
  const container = process.env.ECORP_TEST_POSTGRES_CONTAINER
  if (!container) {
    throw new Error(
      'psql is unavailable and ECORP_TEST_POSTGRES_CONTAINER was not provided',
    )
  }
  const parsedDatabaseUrl = new URL(databaseUrl)
  const databaseName = decodeURIComponent(
    parsedDatabaseUrl.pathname.replace(/^\/+/, ''),
  )
  const databaseUser = decodeURIComponent(parsedDatabaseUrl.username || 'crony')
  if (!databaseName) {
    throw new Error('DATABASE_URL omitted its PostgreSQL database name')
  }
  return {
    command: 'docker',
    args: [
      'exec',
      '-i',
      container,
      'psql',
      '-U',
      databaseUser,
      '-d',
      databaseName,
    ],
  }
}

function sqlLiteral(value) {
  return `'${String(value).replaceAll("'", "''")}'`
}

async function revokeMissionRoomMembership(missionId, actorId) {
  await psql(`
    DELETE FROM room_memberships
    WHERE room_id = (
      SELECT room_id FROM missions WHERE id = ${sqlLiteral(missionId)}::uuid
    )
      AND actor_id = ${sqlLiteral(actorId)}::uuid;
  `)
}

async function restoreMissionRoomMembership(missionId, actorId) {
  await psql(`
    INSERT INTO room_memberships (room_id, actor_id, role)
    SELECT room_id, ${sqlLiteral(actorId)}::uuid, 'member'
    FROM missions
    WHERE id = ${sqlLiteral(missionId)}::uuid
    ON CONFLICT (room_id, actor_id) DO NOTHING;
  `)
}

async function seedNewerPublicationContextRows(
  workItem,
  source,
  count = 501,
) {
  const seed = `publication-context-${nonce}`
  const missionPrefix = `${seed}:mission:`
  const taskPrefix = `${seed}:task:`
  const runPrefix = `${seed}:run:`
  const assignmentPrefix = `${seed}:assignment:`
  const artifactPrefix = `${seed}:artifact:`
  const deliverablePrefix = `${seed}:deliverable:`
  const workItemPrefix = `${seed}:work-item:`
  const claimPrefix = `${seed}:claim:`
  await psql(`
    BEGIN;

    WITH template AS (
      SELECT * FROM missions WHERE id = ${sqlLiteral(workItem.mission_id)}::uuid
    )
    INSERT INTO missions
    SELECT (
      jsonb_populate_record(
        NULL::missions,
        to_jsonb(template) || jsonb_build_object(
          'id', md5(${sqlLiteral(missionPrefix)} || generated.sequence::text)::uuid,
          'title', 'Publication context history ' || generated.sequence::text,
          'status', 'completed',
          'created_at', clock_timestamp() + generated.sequence * interval '1 millisecond',
          'updated_at', clock_timestamp() + generated.sequence * interval '1 millisecond'
        )
      )
    ).*
    FROM template
    CROSS JOIN generate_series(1, ${count}) AS generated(sequence);

    WITH template AS (
      SELECT * FROM tasks WHERE id = ${sqlLiteral(source.task_id)}::uuid
    )
    INSERT INTO tasks
    SELECT (
      jsonb_populate_record(
        NULL::tasks,
        to_jsonb(template) || jsonb_build_object(
          'id', md5(${sqlLiteral(taskPrefix)} || generated.sequence::text)::uuid,
          'mission_id', md5(${sqlLiteral(missionPrefix)} || generated.sequence::text)::uuid,
          'plan_key', 'publication-context-' || generated.sequence::text,
          'title', 'Publication context task ' || generated.sequence::text,
          'status', 'completed',
          'verification_status', 'passed',
          'created_at', clock_timestamp() + generated.sequence * interval '1 millisecond',
          'updated_at', clock_timestamp() + generated.sequence * interval '1 millisecond'
        )
      )
    ).*
    FROM template
    CROSS JOIN generate_series(1, ${count}) AS generated(sequence);

    WITH template AS (
      SELECT * FROM runs WHERE id = ${sqlLiteral(source.run_id)}::uuid
    )
    INSERT INTO runs
    SELECT (
      jsonb_populate_record(
        NULL::runs,
        to_jsonb(template) || jsonb_build_object(
          'id', md5(${sqlLiteral(runPrefix)} || generated.sequence::text)::uuid,
          'task_id', md5(${sqlLiteral(taskPrefix)} || generated.sequence::text)::uuid,
          'assignment_token', md5(${sqlLiteral(assignmentPrefix)} || generated.sequence::text)::uuid,
          'workspace_run_id', md5(${sqlLiteral(runPrefix)} || generated.sequence::text)::uuid,
          'resumed_from_run_id', NULL,
          'provider_session_id', NULL,
          'artifact_id', NULL,
          'artifact_path', NULL,
          'artifact_uri', NULL,
          'artifact_media_type', NULL,
          'artifact_signature', NULL,
          'input_tokens', 0,
          'output_tokens', 0,
          'cost_microusd', 0,
          'status', 'completed',
          'verification_status', 'passed',
          'breaker_stage', 'healthy',
          'created_at', clock_timestamp() + generated.sequence * interval '1 millisecond',
          'updated_at', clock_timestamp() + generated.sequence * interval '1 millisecond'
        )
      )
    ).*
    FROM template
    CROSS JOIN generate_series(1, ${count}) AS generated(sequence);

    WITH template AS (
      SELECT * FROM artifacts WHERE id = ${sqlLiteral(source.artifact_id)}::uuid
    )
    INSERT INTO artifacts
    SELECT (
      jsonb_populate_record(
        NULL::artifacts,
        to_jsonb(template) || jsonb_build_object(
          'id', md5(${sqlLiteral(artifactPrefix)} || generated.sequence::text)::uuid,
          'task_id', md5(${sqlLiteral(taskPrefix)} || generated.sequence::text)::uuid,
          'run_id', md5(${sqlLiteral(runPrefix)} || generated.sequence::text)::uuid,
          'object_key', ${sqlLiteral(`${seed}/artifact/`)} || generated.sequence::text,
          'uri', ${sqlLiteral(`memory://${seed}/artifact/`)} || generated.sequence::text,
          'status', 'ready',
          'staging_key', NULL,
          'rejection_reason', NULL,
          'created_at', clock_timestamp() + generated.sequence * interval '1 millisecond',
          'finalized_at', clock_timestamp() + generated.sequence * interval '1 millisecond',
          'retention_until', clock_timestamp() + interval '30 days'
        )
      )
    ).*
    FROM template
    CROSS JOIN generate_series(1, ${count}) AS generated(sequence);

    UPDATE runs
    SET artifact_id = md5(${sqlLiteral(artifactPrefix)} || generated.sequence::text)::uuid
    FROM generate_series(1, ${count}) AS generated(sequence)
    WHERE runs.id = md5(${sqlLiteral(runPrefix)} || generated.sequence::text)::uuid;

    WITH template AS (
      SELECT * FROM source_deliverables WHERE id = ${sqlLiteral(source.id)}::uuid
    )
    INSERT INTO source_deliverables
    SELECT (
      jsonb_populate_record(
        NULL::source_deliverables,
        to_jsonb(template) || jsonb_build_object(
          'id', md5(${sqlLiteral(deliverablePrefix)} || generated.sequence::text)::uuid,
          'task_id', md5(${sqlLiteral(taskPrefix)} || generated.sequence::text)::uuid,
          'run_id', md5(${sqlLiteral(runPrefix)} || generated.sequence::text)::uuid,
          'artifact_id', md5(${sqlLiteral(artifactPrefix)} || generated.sequence::text)::uuid,
          'branch', 'ecorp/publication-context-' || generated.sequence::text,
          'integration_state', 'ready_for_review',
          'created_at', clock_timestamp() + generated.sequence * interval '1 millisecond'
        )
      )
    ).*
    FROM template
    CROSS JOIN generate_series(1, ${count}) AS generated(sequence);

    WITH template AS (
      SELECT * FROM factory_work_items WHERE id = ${sqlLiteral(workItem.id)}::uuid
    )
    INSERT INTO factory_work_items
    SELECT (
      jsonb_populate_record(
        NULL::factory_work_items,
        to_jsonb(template) || jsonb_build_object(
          'id', md5(${sqlLiteral(workItemPrefix)} || generated.sequence::text)::uuid,
          'source_project_item_id', 'PVTI_PUBLICATION_CONTEXT_' || generated.sequence::text || '_${nonce}',
          'source_issue_number', 50000 + generated.sequence,
          'source_issue_node_id', 'I_PUBLICATION_CONTEXT_' || generated.sequence::text || '_${nonce}',
          'source_issue_url', 'https://github.com/shyamsridhar123/ecorp/issues/' || (50000 + generated.sequence)::text,
          'source_title', 'Publication context history ' || generated.sequence::text,
          'source_revision', (clock_timestamp() + generated.sequence * interval '1 millisecond')::text,
          'state', 'verified',
          'version', 1,
          'claim_token', md5(${sqlLiteral(claimPrefix)} || generated.sequence::text)::uuid,
          'lease_expires_at', clock_timestamp() + interval '1 day',
          'mission_id', md5(${sqlLiteral(missionPrefix)} || generated.sequence::text)::uuid,
          'failure_detail', NULL,
          'created_at', clock_timestamp() + generated.sequence * interval '1 millisecond',
          'updated_at', clock_timestamp() + generated.sequence * interval '1 millisecond'
        )
      )
    ).*
    FROM template
    CROSS JOIN generate_series(1, ${count}) AS generated(sequence);

    COMMIT;
  `)
  return { count, seed }
}

async function seedNewerPublications(publication, seed, count = 501) {
  const missionPrefix = `${seed}:mission:`
  const taskPrefix = `${seed}:task:`
  const runPrefix = `${seed}:run:`
  const artifactPrefix = `${seed}:artifact:`
  const deliverablePrefix = `${seed}:deliverable:`
  const workItemPrefix = `${seed}:work-item:`
  const publicationPrefix = `${seed}:publication:`
  const authorizationPrefix = `${seed}:authorization:`
  await psql(`
    BEGIN;

    WITH template AS (
      SELECT * FROM pull_request_publications
      WHERE id = ${sqlLiteral(publication.id)}::uuid
    )
    INSERT INTO pull_request_publications
    SELECT (
      jsonb_populate_record(
        NULL::pull_request_publications,
        to_jsonb(template) || jsonb_build_object(
          'id', md5(${sqlLiteral(publicationPrefix)} || generated.sequence::text)::uuid,
          'factory_work_item_id', md5(${sqlLiteral(workItemPrefix)} || generated.sequence::text)::uuid,
          'mission_id', md5(${sqlLiteral(missionPrefix)} || generated.sequence::text)::uuid,
          'source_deliverable_id', md5(${sqlLiteral(deliverablePrefix)} || generated.sequence::text)::uuid,
          'artifact_id', md5(${sqlLiteral(artifactPrefix)} || generated.sequence::text)::uuid,
          'task_id', md5(${sqlLiteral(taskPrefix)} || generated.sequence::text)::uuid,
          'run_id', md5(${sqlLiteral(runPrefix)} || generated.sequence::text)::uuid,
          'source_issue_number', 50000 + generated.sequence,
          'source_issue_url', 'https://github.com/shyamsridhar123/ecorp/issues/' || (50000 + generated.sequence)::text,
          'branch', 'ecorp/publication-history-' || generated.sequence::text || '-${nonce}',
          'authorization_id', md5(${sqlLiteral(authorizationPrefix)} || generated.sequence::text)::uuid,
          'effect_key', 'publication-context-effect-' || generated.sequence::text || '-${nonce}',
          'idempotency_key', 'publication-context-start-' || generated.sequence::text || '-${nonce}',
          'state', 'published',
          'version', 5,
          'attempt_count', 1,
          'publisher_id', NULL,
          'publisher_token', NULL,
          'publisher_lease_expires_at', NULL,
          'failure_detail', NULL,
          'branch_pushed_at', clock_timestamp(),
          'pull_request_number', 50000 + generated.sequence,
          'pull_request_node_id', 'PR_PUBLICATION_CONTEXT_' || generated.sequence::text || '_${nonce}',
          'pull_request_url', 'https://github.com/shyamsridhar123/ecorp/pull/' || (50000 + generated.sequence)::text,
          'pull_request_state', 'OPEN',
          'pull_request_draft', false,
          'project_item_id', 'PVTI_PUBLICATION_CONTEXT_' || generated.sequence::text || '_${nonce}',
          'project_status_after', 'In Review',
          'project_status_updated_at', clock_timestamp(),
          'auto_merge_enabled', false,
          'merge_authorized', false,
          'deployment_authorized', false,
          'created_at', clock_timestamp() + generated.sequence * interval '1 millisecond',
          'updated_at', clock_timestamp() + generated.sequence * interval '1 millisecond',
          'provenance', template.provenance || jsonb_build_object(
            'factory_work_item_id', md5(${sqlLiteral(workItemPrefix)} || generated.sequence::text)::uuid,
            'mission_id', md5(${sqlLiteral(missionPrefix)} || generated.sequence::text)::uuid
          )
        )
      )
    ).*
    FROM template
    CROSS JOIN generate_series(1, ${count}) AS generated(sequence);

    UPDATE factory_work_items
    SET state = 'published', updated_at = clock_timestamp()
    FROM generate_series(1, ${count}) AS generated(sequence)
    WHERE factory_work_items.id =
      md5(${sqlLiteral(workItemPrefix)} || generated.sequence::text)::uuid;

    UPDATE source_deliverables
    SET integration_state = 'published'
    FROM generate_series(1, ${count}) AS generated(sequence)
    WHERE source_deliverables.id =
      md5(${sqlLiteral(deliverablePrefix)} || generated.sequence::text)::uuid;

    COMMIT;
  `)
}

async function setFakeState(patch) {
  const state = JSON.parse(await readFile(statePath, 'utf8'))
  Object.assign(state, patch)
  await writeFile(statePath, `${JSON.stringify(state, null, 2)}\n`)
}

async function enrollPublicationPublisher(
  demo,
  publisherId,
  { register = true } = {},
) {
  const enrollment = await postOk(
    `/api/corps/${demo.corp_id}/factory/publication-publishers/credentials`,
    {
      actor_id: demo.alice_actor_id,
      publisher_id: publisherId,
      expires_in_seconds: 3600,
    },
  )
  assert.equal(enrollment.publisher_id, publisherId)
  assert.ok(enrollment.credential)
  publisherCredentialSecrets.push(enrollment.credential)
  const credentialPath = path.join(
    root,
    'output',
    `publication-publisher-${publisherId.replaceAll(/[^A-Za-z0-9_.-]/g, '_')}-${enrollment.credential_id}.credential`,
  )
  await writeFile(credentialPath, enrollment.credential)
  if (register) {
    publisherCredentials.set(publisherId, enrollment.credential)
    publisherCredentialFiles.set(publisherId, credentialPath)
    publisherCredentialIds.set(publisherId, enrollment.credential_id)
  }
  publisherCredentialPaths.push(credentialPath)
  return { ...enrollment, credentialPath }
}

async function publicationState(demo, workItemId) {
  const state = await snapshot(demo)
  return {
    state,
    publication: state.snapshot.pull_request_publications.find(
      (item) => item.factory_work_item_id === workItemId,
    ),
  }
}

function publicationRenewPath(demo, publicationId) {
  return `/api/corps/${demo.corp_id}/factory/publications/${publicationId}/renew`
}

function publicationCheckpointPath(demo, publicationId) {
  return `/api/corps/${demo.corp_id}/factory/publications/${publicationId}/checkpoint`
}

async function renewPublicationAttempt(
  demo,
  publication,
  publisherToken,
  idempotencyKey,
) {
  return postPublisher(
    publicationRenewPath(demo, publication.id),
    {
      actor_id: demo.alice_actor_id,
      publisher_token: publisherToken,
      expected_version: publication.version,
      idempotency_key: idempotencyKey,
      lease_seconds: 10,
    },
    publication.publisher_id,
  )
}

async function failPublicationAttempt(
  demo,
  publication,
  publisherToken,
  idempotencyKey,
  detail,
) {
  return postPublisherOk(
    publicationCheckpointPath(demo, publication.id),
    {
      actor_id: demo.alice_actor_id,
      publisher_token: publisherToken,
      expected_version: publication.version,
      idempotency_key: idempotencyKey,
      checkpoint: {
        kind: 'failed',
        failure_detail: detail,
      },
    },
    publication.publisher_id,
  )
}

async function remoteBranchExists(branch) {
  try {
    await execFile(
      'git',
      ['--git-dir', remotePath, 'show-ref', '--verify', `refs/heads/${branch}`],
      { cwd: root, windowsHide: true },
    )
    return true
  } catch {
    return false
  }
}

async function waitForPublicationLeaseExpiry(demo, workItemId) {
  const current = await publicationState(demo, workItemId)
  const expiry = Date.parse(
    current.publication?.publisher_lease_expires_at ?? new Date().toISOString(),
  )
  const delay = Math.max(0, expiry - Date.now()) + 500
  await new Promise((resolve) => setTimeout(resolve, delay))
}

await rm(statePath, { force: true })
await rm(remotePath, { recursive: true, force: true })
await execFile('git', ['clone', '--quiet', '--bare', root, remotePath], {
  cwd: root,
  windowsHide: true,
})
await execFile(
  'git',
  ['--git-dir', remotePath, 'update-ref', 'refs/heads/main', sourceBaseCommit],
  { cwd: root, windowsHide: true },
)
await execFile(
  'git',
  ['--git-dir', remotePath, 'symbolic-ref', 'HEAD', 'refs/heads/main'],
  { cwd: root, windowsHide: true },
)
const resolvedPublicationBase = (
  await execFile(
    'git',
    ['--git-dir', remotePath, 'symbolic-ref', '--short', 'HEAD'],
    { cwd: root, windowsHide: true },
  )
).stdout.trim()
assert.equal(resolvedPublicationBase, 'main')

const issueNumber = 9101
const collisionIssueNumber = 9100
const issue = {
  id: `I_PUBLICATION_${nonce}`,
  number: issueNumber,
  title: 'Publish a verified factory result exactly once',
  body: `## Outcome

Produce one verified commit/branch deliverable and publish it for review.

## Acceptance criteria

- [ ] verified commit exists
- [ ] pull request exists exactly once
- [ ] Project enters review only after the pull request exists

## Dependencies

No blockers.
`,
  url: `https://github.com/shyamsridhar123/ecorp/issues/${issueNumber}`,
  state: 'OPEN',
  createdAt: '2026-09-02T02:00:00Z',
  updatedAt: '2026-09-02T02:00:00Z',
  labels: [{ name: 'factory:ready' }],
}
const collisionIssue = {
  id: `I_PUBLICATION_COLLISION_${nonce}`,
  number: collisionIssueNumber,
  title: 'Reject publication onto the resolved base branch',
  body: `## Outcome

Prove the trusted publisher refuses to push directly to the resolved base branch.

## Acceptance criteria

- [ ] the base branch remains unchanged
- [ ] retry without an explicit authorization id reaches the same guarded failure

## Dependencies

No blockers.
`,
  url: `https://github.com/shyamsridhar123/ecorp/issues/${collisionIssueNumber}`,
  state: 'OPEN',
  createdAt: '2026-09-02T01:59:00Z',
  updatedAt: '2026-09-02T01:59:00Z',
  labels: [{ name: 'factory:ready' }],
}
await writeFile(
  statePath,
  `${JSON.stringify(
    {
      repository: 'shyamsridhar123/ecorp',
      canonical_repository: 'ShyamSridhar123/ECorp',
      project: {
        id: 'PVT_PUBLICATION',
        number: 7,
        owner: 'acme',
        title: 'Factory Publication Test',
        status_field_id: 'PVTSSF_PUBLICATION_STATUS',
        status_options: [
          { id: 'todo', name: 'Todo' },
          { id: 'in-progress', name: 'In Progress' },
          { id: 'in-review', name: 'In Review' },
          { id: 'done', name: 'Done' },
        ],
      },
      items: [
        {
          id: `PVTI_PUBLICATION_COLLISION_${nonce}`,
          status: 'Todo',
          content: {
            body: collisionIssue.body,
            number: collisionIssue.number,
            repository: 'shyamsridhar123/ecorp',
            title: collisionIssue.title,
            type: 'Issue',
            url: collisionIssue.url,
          },
        },
        {
          id: `PVTI_PUBLICATION_${nonce}`,
          status: 'Todo',
          content: {
            body: issue.body,
            number: issue.number,
            repository: 'shyamsridhar123/ecorp',
            title: issue.title,
            type: 'Issue',
            url: issue.url,
          },
        },
      ],
      issues: {
        [String(collisionIssueNumber)]: collisionIssue,
        [String(issueNumber)]: issue,
      },
      pull_requests: [],
      next_pr_number: 41,
      item_edits: 0,
      pr_create_calls: 0,
      pr_create_delay_ms: 300,
      effect_log: [],
    },
    null,
    2,
  )}\n`,
)

const demo = await postOk('/api/demo/reset', {})
const memberPublisherEnrollment = await post(
  `/api/corps/${demo.corp_id}/factory/publication-publishers/credentials`,
  {
    actor_id: demo.bob_actor_id,
    publisher_id: 'member-must-not-enroll',
    expires_in_seconds: 3600,
  },
)
assert.equal(memberPublisherEnrollment.response.status, 403)
for (const publisherId of [
  'trusted-publication-e2e',
  'trusted-publication-host-a',
  'trusted-publication-host-b',
  'trusted-publication-host-c',
]) {
  await enrollPublicationPublisher(demo, publisherId)
}
const revokedPublisherCredential = await enrollPublicationPublisher(
  demo,
  'trusted-publication-e2e',
  { register: false },
)
const revokedPublisher = await postOk(
  `/api/corps/${demo.corp_id}/factory/publication-publishers/credentials/${revokedPublisherCredential.credential_id}/revoke`,
  {
    actor_id: demo.alice_actor_id,
    reason: 'Publication E2E revocation test.',
  },
)
assert.equal(revokedPublisher.revoked, true)
const expiredPublisherCredential = await enrollPublicationPublisher(
  demo,
  'trusted-publication-e2e',
  { register: false },
)
await psql(
  `UPDATE publication_publisher_credentials SET created_at = now() - interval '2 hours', expires_at = now() - interval '1 second' WHERE id = ${sqlLiteral(expiredPublisherCredential.credential_id)}::uuid;`,
)
const otherCorpId = crypto.randomUUID()
const otherCorpOwnerId = crypto.randomUUID()
await psql(`
  INSERT INTO corps (id, slug, name)
  VALUES (
    ${sqlLiteral(otherCorpId)}::uuid,
    ${sqlLiteral(`publisher-other-${nonce}`)},
    'Other publisher Corp'
  );
  INSERT INTO actors (id, corp_id, name, kind, role)
  VALUES (
    ${sqlLiteral(otherCorpOwnerId)}::uuid,
    ${sqlLiteral(otherCorpId)}::uuid,
    'Other publisher owner',
    'human',
    'owner'
  );
`)
const crossCorpPublisherCredential = await postOk(
  `/api/corps/${otherCorpId}/factory/publication-publishers/credentials`,
  {
    actor_id: otherCorpOwnerId,
    publisher_id: 'trusted-publication-e2e',
    expires_in_seconds: 3600,
  },
)
publisherCredentialSecrets.push(crossCorpPublisherCredential.credential)
const crossRoomActorId = crypto.randomUUID()
const crossRoomId = crypto.randomUUID()
await psql(`
  INSERT INTO actors (id, corp_id, name, kind, role)
  VALUES (
    ${sqlLiteral(crossRoomActorId)}::uuid,
    ${sqlLiteral(demo.corp_id)}::uuid,
    'Cross-room publication manager',
    'human',
    'manager'
  );
  INSERT INTO rooms (id, corp_id, name, purpose)
  VALUES (
    ${sqlLiteral(crossRoomId)}::uuid,
    ${sqlLiteral(demo.corp_id)}::uuid,
    ${sqlLiteral(`Cross-room publication ${nonce}`)},
    'Prove publication data and effects remain mission-room scoped.'
  );
  INSERT INTO room_memberships (room_id, actor_id, role)
  VALUES (
    ${sqlLiteral(crossRoomId)}::uuid,
    ${sqlLiteral(crossRoomActorId)}::uuid,
    'member'
  );
`)
const collisionFirstController = await runController(demo, collisionIssueNumber)
assert.equal(collisionFirstController.factory_state, 'running')
const collisionCompleted = await waitForMission(
  demo,
  collisionFirstController.mission_id,
)
assert.equal(collisionCompleted.mission.status, 'completed')
const collisionVerifiedController = await runController(
  demo,
  collisionIssueNumber,
)
assert.equal(collisionVerifiedController.factory_state, 'verified')
const collisionSnapshot = await snapshot(demo)
const collisionWorkItem = collisionSnapshot.snapshot.factory_work_items.find(
  (item) => item.id === collisionFirstController.factory_work_item_id,
)
assert.ok(collisionWorkItem)
const collisionTaskIds = new Set(
  collisionSnapshot.snapshot.tasks
    .filter((task) => task.mission_id === collisionFirstController.mission_id)
    .map((task) => task.id),
)
const collisionSource = collisionSnapshot.snapshot.source_deliverables.find(
  (deliverable) =>
    collisionTaskIds.has(deliverable.task_id) &&
    deliverable.form === 'commit_branch' &&
    deliverable.integration_state === 'ready_for_review',
)
assert.ok(collisionSource)
await runPublisher(demo, collisionWorkItem.id, {
  branch: `ecorp/missing-credential-${nonce.slice(0, 8)}`,
  sourceDeliverableId: collisionSource.id,
  publisherId: 'missing-publication-publisher',
  expectFailure: /credential file is required/,
})
assert.equal(
  (await publicationContext(demo, collisionWorkItem.id)).publication,
  null,
)
const invalidPublisherCredential = `invalid-publisher-${crypto.randomUUID()}`
publisherCredentialSecrets.push(invalidPublisherCredential)
const invalidPublisherCredentialPath = path.join(
  root,
  'output',
  `publication-publisher-invalid-${nonce}.credential`,
)
await writeFile(invalidPublisherCredentialPath, invalidPublisherCredential)
publisherCredentialPaths.push(invalidPublisherCredentialPath)
await runPublisher(demo, collisionWorkItem.id, {
  branch: `ecorp/invalid-credential-${nonce.slice(0, 8)}`,
  sourceDeliverableId: collisionSource.id,
  publisherId: 'trusted-publication-host-a',
  publisherCredentialFile: invalidPublisherCredentialPath,
  expectFailure: /403|credential was rejected/,
})
assert.equal(
  (await publicationContext(demo, collisionWorkItem.id)).publication,
  null,
)
for (const invalidBranch of ['ecorp/foo//bar', 'ecorp/foo.lock']) {
  await runPublisher(demo, collisionWorkItem.id, {
    branch: invalidBranch,
    sourceDeliverableId: collisionSource.id,
    expectFailure: /not a valid Git branch name/,
  })
}
assert.equal(
  (await publicationContext(demo, collisionWorkItem.id)).publication,
  null,
)
await psql(
  `UPDATE factory_work_items SET policy = jsonb_set(policy, '{publication,branch_prefix}', '\"main\"'::jsonb) WHERE id = ${sqlLiteral(collisionWorkItem.id)}::uuid;`,
)
const collisionBodyPath = path.join(
  root,
  'output',
  `publication-body-${nonce}.md`,
)
await writeFile(
  collisionBodyPath,
  `Implements ${collisionIssue.url}\r\n\r\nBase branch collision guard.\r\n`,
)
const remoteMainBefore = (
  await execFile(
    'git',
    ['--git-dir', remotePath, 'rev-parse', 'refs/heads/main'],
    { cwd: root, windowsHide: true },
  )
).stdout.trim()
const collisionFailurePattern =
  /publication branch main must differ from resolved pull request base main/
await runPublisher(demo, collisionWorkItem.id, {
  branch: 'main',
  bodyFile: collisionBodyPath,
  omitAuthorizationId: true,
  expectFailure: collisionFailurePattern,
})
const collisionPreflightState = await snapshot(demo)
assert.equal(
  collisionPreflightState.snapshot.pull_request_publications.some(
    (publication) =>
      publication.factory_work_item_id === collisionWorkItem.id,
  ),
  false,
)
assert.equal(
  collisionPreflightState.snapshot.factory_work_items.find(
    (item) => item.id === collisionWorkItem.id,
  ).state,
  'verified',
)
const collisionRecoveryBranch = `main-safe-${collisionSource.head_commit.slice(0, 12)}`
const collisionCustomTitle = `Collision recovery ${nonce}`
await runPublisher(demo, collisionWorkItem.id, {
  branch: collisionRecoveryBranch,
  bodyFile: collisionBodyPath,
  omitAuthorizationId: true,
  title: `  ${collisionCustomTitle}  `,
  repository: 'ShyamSridhar123/ECorp',
  publisherId: 'trusted-publication-host-a',
  authorizationReason: 'Host A authorizes the first recoverable publication attempt.',
  leaseSeconds: 5,
  crashAfter: 'after_plan_validation',
  expectCrash: true,
})
const collisionRestartedServerPid = await restartLocalServer()
await waitForPublicationLeaseExpiry(demo, collisionWorkItem.id)
await runPublisher(demo, collisionWorkItem.id, {
  branch: collisionRecoveryBranch,
  bodyFile: collisionBodyPath,
  omitAuthorizationId: true,
  publisherId: 'trusted-publication-host-b',
  authorizationReason: 'Host B authorizes recovery after the first publisher stopped.',
  leaseSeconds: 7,
  crashAfter: 'after_start',
  expectCrash: true,
})
const remoteMainAfter = (
  await execFile(
    'git',
    ['--git-dir', remotePath, 'rev-parse', 'refs/heads/main'],
    { cwd: root, windowsHide: true },
  )
).stdout.trim()
assert.equal(remoteMainAfter, remoteMainBefore)
assert.equal(remoteMainAfter, sourceBaseCommit)
let collisionFakeState = JSON.parse(await readFile(statePath, 'utf8'))
assert.equal(collisionFakeState.pr_create_calls, 0)
const collisionPublication = (await snapshot(demo)).snapshot.pull_request_publications.find(
  (publication) => publication.factory_work_item_id === collisionWorkItem.id,
)
assert.equal(collisionPublication.branch, collisionRecoveryBranch)
assert.equal(collisionPublication.title, collisionCustomTitle)
assert.equal(collisionPublication.target_repository, 'shyamsridhar123/ecorp')
assert.equal(collisionPublication.publisher_id, 'trusted-publication-host-b')
assert.equal(collisionPublication.attempt_count, 2)
assert.equal(
  collisionFakeState.items.find(
    (item) => item.id === collisionWorkItem.source_project_item_id,
  ).status,
  'In Progress',
)
await rm(collisionBodyPath, { force: true })

const firstController = await runController(demo, issueNumber)
assert.equal(firstController.factory_state, 'running')
const completed = await waitForMission(demo, firstController.mission_id)
assert.equal(completed.mission.status, 'completed')
const verifiedController = await runController(demo, issueNumber)
assert.equal(verifiedController.factory_state, 'verified')
assert.equal(
  verifiedController.factory_work_item_id,
  firstController.factory_work_item_id,
)

const verifiedSnapshot = await snapshot(demo)
const workItem = verifiedSnapshot.snapshot.factory_work_items.find(
  (item) => item.id === firstController.factory_work_item_id,
)
assert.ok(workItem)
const taskIds = new Set(
  verifiedSnapshot.snapshot.tasks
    .filter((task) => task.mission_id === firstController.mission_id)
    .map((task) => task.id),
)
const source = verifiedSnapshot.snapshot.source_deliverables.find(
  (deliverable) =>
    taskIds.has(deliverable.task_id) &&
    deliverable.form === 'commit_branch' &&
    deliverable.integration_state === 'ready_for_review',
)
assert.ok(source)
assert.ok(source.head_commit)
const contextSeed = await seedNewerPublicationContextRows(
  workItem,
  source,
)
const boundedPublicationSnapshot = await snapshot(demo)
assert.equal(
  boundedPublicationSnapshot.snapshot.factory_work_items.some(
    (item) => item.id === workItem.id,
  ),
  false,
)
assert.equal(
  boundedPublicationSnapshot.snapshot.source_deliverables.some(
    (item) => item.id === source.id,
  ),
  false,
)
const exactPublicationContext = await publicationContext(demo, workItem.id)
assert.equal(exactPublicationContext.work_item.id, workItem.id)
assert.equal(exactPublicationContext.publication, null)
assert.equal(
  exactPublicationContext.source_deliverables.some(
    (deliverable) => deliverable.id === source.id,
  ),
  true,
)
const workItemFailureCanary = `cross-room-work-item-${nonce}`
await psql(
  `UPDATE factory_work_items SET failure_detail = ${sqlLiteral(workItemFailureCanary)} WHERE id = ${sqlLiteral(workItem.id)}::uuid;`,
)
const crossRoomPublicationContext = await fetch(
  `${server}/api/corps/${demo.corp_id}/factory/work-items/${workItem.id}/publication-context?actor_id=${crossRoomActorId}`,
)
const crossRoomPublicationContextBody =
  await crossRoomPublicationContext.text()
assert.equal(crossRoomPublicationContext.status, 404)
for (const secret of [
  workItem.id,
  workItem.source_issue_url,
  workItem.source_title,
  workItem.claim_owner_id,
  workItemFailureCanary,
  JSON.stringify(workItem.policy),
]) {
  assert.equal(
    crossRoomPublicationContextBody.includes(secret),
    false,
    `cross-room publication context leaked ${secret}`,
  )
}
await psql(
  `UPDATE factory_work_items SET failure_detail = NULL WHERE id = ${sqlLiteral(workItem.id)}::uuid;`,
)
const guestPublicationContext = await fetch(
  `${server}/api/corps/${demo.corp_id}/factory/work-items/${workItem.id}/publication-context?actor_id=${demo.eve_actor_id}`,
)
const guestPublicationContextBody = await guestPublicationContext.text()
assert.equal(guestPublicationContext.status, 403)
assert.equal(guestPublicationContextBody.includes(workItem.id), false)
assert.equal(guestPublicationContextBody.includes(source.id), false)

const prePublicationFakeState = JSON.parse(await readFile(statePath, 'utf8'))
const projectItemListCallsBeforePublication =
  prePublicationFakeState.item_list_calls ?? 0
const projectFieldListCallsBeforePublication =
  prePublicationFakeState.field_list_calls ?? 0
const projectItems = prePublicationFakeState.items
const projectFillers = Array.from({ length: 1001 }, (_, index) => ({
  id: `PVTI_PUBLICATION_FILLER_${String(index).padStart(4, '0')}_${nonce}`,
  status: 'Done',
  content: {
    body: '',
    number: 60000 + index,
    repository: 'shyamsridhar123/ecorp',
    title: `Publication filler ${index}`,
    type: 'Issue',
    url: `https://github.com/shyamsridhar123/ecorp/issues/${60000 + index}`,
  },
}))
const expandedProjectItems = [...projectFillers, ...projectItems]
const expandedProjectFields = [
  ...Array.from({ length: 31 }, (_, index) => ({
    id: `PVTF_PUBLICATION_FILLER_${String(index).padStart(2, '0')}_${nonce}`,
    name: `Filler ${index}`,
    type: 'ProjectV2Field',
  })),
  {
    id: prePublicationFakeState.project.status_field_id,
    name: 'Status',
    type: 'ProjectV2SingleSelectField',
    options: prePublicationFakeState.project.status_options,
  },
]
assert.ok(
  expandedProjectItems.findIndex(
    (item) => item.id === workItem.source_project_item_id,
  ) >= 1000,
)
await setFakeState({
  items: expandedProjectItems,
  project_fields: expandedProjectFields,
})
const branch = `ecorp/issue-${issueNumber}-${source.head_commit.slice(0, 12)}`
const body = `## ECorp verified factory deliverable

Implements ${issue.url}

- Factory work item: \`${workItem.id}\`
- Mission: \`${firstController.mission_id}\`
- Verified commit: \`${source.head_commit}\`
- Deliverable digest: \`${source.sha256}\`

Closes #${issueNumber}

Auto-merge, merge, and deployment are not authorized by this publication.`
const effectKey = `github-pr:${workItem.id}:${source.id}:shyamsridhar123/ecorp:${branch}`
const forkPullRequest = {
  number: 7,
  id: 'PR_FAKE_FORK_7',
  url: 'https://github.com/shyamsridhar123/ecorp/pull/7',
  state: 'OPEN',
  isDraft: false,
  headRefName: branch,
  baseRefName: resolvedPublicationBase,
  headRefOid: 'f'.repeat(40),
  headRepositoryOwner: { login: 'untrusted-fork-owner' },
  isCrossRepository: true,
  autoMergeRequest: null,
  title: 'Untrusted same-name fork pull request',
  body: 'This pull request must never be adopted.',
}
await setFakeState({
  branch_heads: { [branch]: source.head_commit },
  pull_requests: [forkPullRequest],
})
const publicationRequest = {
  actor_id: demo.alice_actor_id,
  source_deliverable_id: source.id,
  target_repository: 'shyamsridhar123/ecorp',
  base_ref: 'HEAD',
  branch,
  title: issue.title,
  body,
  authorization_id: authorizationId,
  authorization_reason:
    'Publication E2E authorizes review-only branch and pull request creation.',
  effect_key: effectKey,
  idempotency_key: `${effectKey}:negative`,
  publisher_id: 'trusted-publication-e2e',
  lease_seconds: 5,
}
const publicationPath =
  `/api/corps/${demo.corp_id}/factory/work-items/${workItem.id}/publication`

const humanOnlyStartRejected = await post(publicationPath, {
  ...publicationRequest,
  idempotency_key: `${effectKey}:human-only-start-rejected`,
})
assert.equal(humanOnlyStartRejected.response.status, 403)
assert.equal((await publicationContext(demo, workItem.id)).publication, null)

const crossRoomStartRejected = await postPublicationStart(publicationPath, {
  ...publicationRequest,
  actor_id: crossRoomActorId,
  authorization_id: crypto.randomUUID(),
  idempotency_key: `${effectKey}:cross-room-start-rejected`,
})
assert.equal(crossRoomStartRejected.response.status, 403)
assert.match(crossRoomStartRejected.body.error, /not a member of this room/)

const roleRejected = await postPublicationStart(publicationPath, {
  ...publicationRequest,
  actor_id: demo.bob_actor_id,
  idempotency_key: `${effectKey}:member-rejected`,
})
assert.equal(roleRejected.response.status, 403)
const corpRejected = await postPublicationStart(
  `/api/corps/${crypto.randomUUID()}/factory/work-items/${workItem.id}/publication`,
  {
    ...publicationRequest,
    idempotency_key: `${effectKey}:corp-rejected`,
  },
)
assert.equal(corpRejected.response.status, 403)

await psql(
  `UPDATE factory_work_items SET policy = jsonb_set(policy, '{publication,allowed}', 'false'::jsonb) WHERE id = ${sqlLiteral(workItem.id)}::uuid;`,
)
const policyRejected = await postPublicationStart(publicationPath, {
  ...publicationRequest,
  idempotency_key: `${effectKey}:policy-rejected`,
})
assert.equal(policyRejected.response.status, 400)
assert.match(policyRejected.body.error, /does not authorize/)
await psql(
  `UPDATE factory_work_items SET policy = jsonb_set(policy, '{publication,allowed}', 'true'::jsonb) WHERE id = ${sqlLiteral(workItem.id)}::uuid;`,
)

const run = verifiedSnapshot.snapshot.runs.find((item) =>
  taskIds.has(item.task_id),
)
assert.ok(run)
const breakerBefore = await psql(
  `SELECT breaker_stage FROM runs WHERE id = ${sqlLiteral(run.id)}::uuid;`,
)
await psql(
  `UPDATE runs SET breaker_stage = 'suspend' WHERE id = ${sqlLiteral(run.id)}::uuid;`,
)
const breakerRejected = await postPublicationStart(publicationPath, {
  ...publicationRequest,
  idempotency_key: `${effectKey}:breaker-rejected`,
})
assert.equal(breakerRejected.response.status, 400)
assert.match(breakerRejected.body.error, /circuit breaker/)
await psql(
  `UPDATE runs SET breaker_stage = ${sqlLiteral(breakerBefore)} WHERE id = ${sqlLiteral(run.id)}::uuid;`,
)

const usageBefore = (
  await psql(
    `SELECT input_tokens || ',' || output_tokens || ',' || budget_tokens_limit FROM runs WHERE id = ${sqlLiteral(run.id)}::uuid;`,
  )
)
  .split(',')
  .map(Number)
await psql(
  `UPDATE runs SET input_tokens = budget_tokens_limit, output_tokens = 0 WHERE id = ${sqlLiteral(run.id)}::uuid;`,
)
const budgetRejected = await postPublicationStart(publicationPath, {
  ...publicationRequest,
  idempotency_key: `${effectKey}:budget-rejected`,
})
assert.equal(budgetRejected.response.status, 400)
assert.match(budgetRejected.body.error, /budget|hard breaker/)
await psql(
  `UPDATE runs SET input_tokens = ${usageBefore[0]}, output_tokens = ${usageBefore[1]} WHERE id = ${sqlLiteral(run.id)}::uuid;`,
)

const initialAttempt = await postPublicationStartOk(publicationPath, {
  ...publicationRequest,
  idempotency_key: `${effectKey}:initial-authority-attempt`,
  lease_seconds: 30,
})
assert.ok(initialAttempt.publisher_token)
const humanOnlyRenewRejected = await post(
  publicationRenewPath(demo, initialAttempt.publication.id),
  {
    actor_id: demo.alice_actor_id,
    publisher_token: initialAttempt.publisher_token,
    expected_version: initialAttempt.publication.version,
    idempotency_key: `${effectKey}:human-only-renew-rejected`,
    lease_seconds: 10,
  },
)
assert.equal(humanOnlyRenewRejected.response.status, 403)
const humanOnlyBranchCheckpointRejected = await post(
  publicationCheckpointPath(demo, initialAttempt.publication.id),
  {
    actor_id: demo.alice_actor_id,
    publisher_token: initialAttempt.publisher_token,
    expected_version: initialAttempt.publication.version,
    idempotency_key: `${effectKey}:human-only-branch-rejected`,
    checkpoint: {
      kind: 'branch_pushed',
      commit_sha: source.head_commit,
    },
  },
)
assert.equal(humanOnlyBranchCheckpointRejected.response.status, 403)
const humanOnlyFailureRejected = await post(
  publicationCheckpointPath(demo, initialAttempt.publication.id),
  {
    actor_id: demo.alice_actor_id,
    publisher_token: initialAttempt.publisher_token,
    expected_version: initialAttempt.publication.version,
    idempotency_key: `${effectKey}:human-only-failure-rejected`,
    checkpoint: {
      kind: 'failed',
      failure_detail: 'A human credential alone must not fail publication.',
    },
  },
)
assert.equal(humanOnlyFailureRejected.response.status, 403)
const humanOnlyCompletionRejected = await post(
  publicationCheckpointPath(demo, initialAttempt.publication.id),
  {
    actor_id: demo.alice_actor_id,
    publisher_token: initialAttempt.publisher_token,
    expected_version: initialAttempt.publication.version,
    idempotency_key: `${effectKey}:human-only-completion-rejected`,
    checkpoint: {
      kind: 'published',
      project_status: 'In Review',
      project_field_id: 'PVTSSF_PUBLICATION_STATUS',
      project_option_id: 'in-review',
    },
  },
)
assert.equal(humanOnlyCompletionRejected.response.status, 403)
const revokedCredentialRenewRejected = await postWithPublisherCredential(
  publicationRenewPath(demo, initialAttempt.publication.id),
  {
    actor_id: demo.alice_actor_id,
    publisher_token: initialAttempt.publisher_token,
    expected_version: initialAttempt.publication.version,
    idempotency_key: `${effectKey}:revoked-credential-renew-rejected`,
    lease_seconds: 10,
  },
  revokedPublisherCredential.credential,
)
assert.equal(revokedCredentialRenewRejected.response.status, 403)
const expiredCredentialRenewRejected = await postWithPublisherCredential(
  publicationRenewPath(demo, initialAttempt.publication.id),
  {
    actor_id: demo.alice_actor_id,
    publisher_token: initialAttempt.publisher_token,
    expected_version: initialAttempt.publication.version,
    idempotency_key: `${effectKey}:expired-credential-renew-rejected`,
    lease_seconds: 10,
  },
  expiredPublisherCredential.credential,
)
assert.equal(expiredCredentialRenewRejected.response.status, 403)
const crossCorpCredentialRenewRejected = await postWithPublisherCredential(
  publicationRenewPath(demo, initialAttempt.publication.id),
  {
    actor_id: demo.alice_actor_id,
    publisher_token: initialAttempt.publisher_token,
    expected_version: initialAttempt.publication.version,
    idempotency_key: `${effectKey}:cross-corp-credential-renew-rejected`,
    lease_seconds: 10,
  },
  crossCorpPublisherCredential.credential,
)
assert.equal(crossCorpCredentialRenewRejected.response.status, 403)
const mismatchedPublisherRenewRejected = await postPublisher(
  publicationRenewPath(demo, initialAttempt.publication.id),
  {
    actor_id: demo.alice_actor_id,
    publisher_token: initialAttempt.publisher_token,
    expected_version: initialAttempt.publication.version,
    idempotency_key: `${effectKey}:wrong-publisher-renew-rejected`,
    lease_seconds: 10,
  },
  'trusted-publication-host-a',
)
assert.equal(mismatchedPublisherRenewRejected.response.status, 409)
assert.match(
  mismatchedPublisherRenewRejected.body.error,
  /another trusted publisher/,
)
const publicationFailureCanary = `cross-room-failure-${nonce}`
await psql(
  `UPDATE pull_request_publications SET failure_detail = ${sqlLiteral(publicationFailureCanary)} WHERE id = ${sqlLiteral(initialAttempt.publication.id)}::uuid;`,
)
const crossRoomStatus = await request(
  `${publicationPath}?actor_id=${crossRoomActorId}`,
)
assert.equal(crossRoomStatus.response.status, 404)
const crossRoomStatusText = JSON.stringify(crossRoomStatus.body)
for (const secret of [
  body,
  publicationRequest.authorization_reason,
  publicationRequest.publisher_id,
  publicationRequest.authorization_id,
  publicationFailureCanary,
]) {
  assert.equal(
    crossRoomStatusText.includes(secret),
    false,
    `cross-room publication status leaked ${secret}`,
  )
}
await psql(
  `UPDATE pull_request_publications SET failure_detail = NULL WHERE id = ${sqlLiteral(initialAttempt.publication.id)}::uuid;`,
)

await revokeMissionRoomMembership(workItem.mission_id, demo.alice_actor_id)
const branchMembershipRenewRejected = await renewPublicationAttempt(
  demo,
  initialAttempt.publication,
  initialAttempt.publisher_token,
  `${effectKey}:renew-branch-membership-rejected`,
)
assert.equal(branchMembershipRenewRejected.response.status, 403)
assert.match(
  branchMembershipRenewRejected.body.error,
  /not a member of this room/,
)
assert.equal(await remoteBranchExists(branch), false)
await restoreMissionRoomMembership(workItem.mission_id, demo.alice_actor_id)

await psql(
  `UPDATE actors SET role = 'admin' WHERE id = ${sqlLiteral(demo.alice_actor_id)}::uuid;`,
)
const roleRenewRejected = await renewPublicationAttempt(
  demo,
  initialAttempt.publication,
  initialAttempt.publisher_token,
  `${effectKey}:renew-role-rejected`,
)
assert.equal(roleRenewRejected.response.status, 400)
assert.match(roleRenewRejected.body.error, /authorization role changed/)
assert.equal(await remoteBranchExists(branch), false)
await psql(
  `UPDATE actors SET role = 'owner' WHERE id = ${sqlLiteral(demo.alice_actor_id)}::uuid;`,
)
await psql(
  `UPDATE runs SET breaker_stage = 'suspend' WHERE id = ${sqlLiteral(run.id)}::uuid;`,
)
const postStartBreakerRejected = await renewPublicationAttempt(
  demo,
  initialAttempt.publication,
  initialAttempt.publisher_token,
  `${effectKey}:renew-breaker-rejected`,
)
assert.equal(postStartBreakerRejected.response.status, 400)
assert.match(postStartBreakerRejected.body.error, /circuit breaker/)
assert.equal(await remoteBranchExists(branch), false)
await psql(
  `UPDATE runs SET breaker_stage = ${sqlLiteral(breakerBefore)} WHERE id = ${sqlLiteral(run.id)}::uuid;`,
)
await failPublicationAttempt(
  demo,
  initialAttempt.publication,
  initialAttempt.publisher_token,
  `${effectKey}:release-initial-authority-test`,
  'Release the authority-revocation test attempt.',
)
const crossRoomRecoveryRejected = await postPublicationStart(publicationPath, {
  ...publicationRequest,
  actor_id: crossRoomActorId,
  authorization_id: crypto.randomUUID(),
  idempotency_key: `${effectKey}:cross-room-recovery-rejected`,
  lease_seconds: 30,
})
assert.equal(crossRoomRecoveryRejected.response.status, 403)
assert.match(crossRoomRecoveryRejected.body.error, /not a member of this room/)

await runPublisher(demo, workItem.id, {
  crashAfter: 'after_branch_remote',
  expectCrash: true,
})
const remoteBranchAfterCrash = (
  await execFile(
    'git',
    ['--git-dir', remotePath, 'rev-parse', `refs/heads/${branch}`],
    { cwd: root, windowsHide: true },
  )
).stdout.trim()
assert.equal(remoteBranchAfterCrash, source.head_commit)
let publicationSnapshot = await publicationState(demo, workItem.id)
assert.equal(publicationSnapshot.publication.state, 'publishing')
assert.equal(publicationSnapshot.publication.branch_pushed_at, null)

const restartedServerPid = await restartLocalServer()
await waitForPublicationLeaseExpiry(demo, workItem.id)
await psql(
  `UPDATE actors SET role = 'manager' WHERE id = ${sqlLiteral(demo.bob_actor_id)}::uuid;`,
)
await runPublisher(demo, workItem.id, {
  actorId: demo.bob_actor_id,
  omitAuthorizationId: true,
  crashAfter: 'after_branch_checkpoint',
  expectCrash: true,
})
publicationSnapshot = await publicationState(demo, workItem.id)
assert.equal(publicationSnapshot.publication.state, 'branch_pushed')
const bobPublicationAttempt =
  publicationSnapshot.state.snapshot.pull_request_publication_attempts
    .filter(
      (attempt) =>
        attempt.publication_id === publicationSnapshot.publication.id &&
        attempt.actor_id === demo.bob_actor_id,
    )
    .sort((left, right) => right.attempt - left.attempt)[0]
assert.ok(bobPublicationAttempt)
assert.equal(
  bobPublicationAttempt.authorization_snapshot.actor_id,
  demo.bob_actor_id,
)
assert.notEqual(
  bobPublicationAttempt.authorization_id,
  publicationSnapshot.publication.authorization_id,
)
await psql(
  `UPDATE actors SET role = 'member' WHERE id = ${sqlLiteral(demo.bob_actor_id)}::uuid;`,
)
let fakeState = JSON.parse(await readFile(statePath, 'utf8'))
assert.equal(
  fakeState.pull_requests.filter(
    (pullRequest) => pullRequest.isCrossRepository === false,
  ).length,
  0,
)

await waitForPublicationLeaseExpiry(demo, workItem.id)
const pullRequestAuthorityAttempt = await postPublicationStartOk(publicationPath, {
  ...publicationRequest,
  idempotency_key: `${effectKey}:pull-request-authority-attempt`,
  lease_seconds: 30,
})
assert.ok(pullRequestAuthorityAttempt.publisher_token)
await revokeMissionRoomMembership(workItem.mission_id, demo.alice_actor_id)
const pullRequestMembershipRenewRejected = await renewPublicationAttempt(
  demo,
  pullRequestAuthorityAttempt.publication,
  pullRequestAuthorityAttempt.publisher_token,
  `${effectKey}:pull-request-membership-rejected`,
)
assert.equal(pullRequestMembershipRenewRejected.response.status, 403)
assert.match(
  pullRequestMembershipRenewRejected.body.error,
  /not a member of this room/,
)
fakeState = JSON.parse(await readFile(statePath, 'utf8'))
assert.equal(
  fakeState.pull_requests.filter(
    (pullRequest) => pullRequest.isCrossRepository === false,
  ).length,
  0,
)
await restoreMissionRoomMembership(workItem.mission_id, demo.alice_actor_id)
await psql(
  `UPDATE runs SET input_tokens = budget_tokens_limit, output_tokens = 0 WHERE id = ${sqlLiteral(run.id)}::uuid;`,
)
const pullRequestBudgetRejected = await renewPublicationAttempt(
  demo,
  pullRequestAuthorityAttempt.publication,
  pullRequestAuthorityAttempt.publisher_token,
  `${effectKey}:pull-request-budget-rejected`,
)
assert.equal(pullRequestBudgetRejected.response.status, 400)
assert.match(pullRequestBudgetRejected.body.error, /budget|hard breaker/)
fakeState = JSON.parse(await readFile(statePath, 'utf8'))
assert.equal(
  fakeState.pull_requests.filter(
    (pullRequest) => pullRequest.isCrossRepository === false,
  ).length,
  0,
)
await psql(
  `UPDATE runs SET input_tokens = ${usageBefore[0]}, output_tokens = ${usageBefore[1]} WHERE id = ${sqlLiteral(run.id)}::uuid;`,
)
await failPublicationAttempt(
  demo,
  pullRequestAuthorityAttempt.publication,
  pullRequestAuthorityAttempt.publisher_token,
  `${effectKey}:release-pull-request-authority-test`,
  'Release the pull-request authority-revocation test attempt.',
)

const unauthorizedContentPullRequest = {
  number: 8,
  id: 'PR_FAKE_UNAUTHORIZED_CONTENT_8',
  url: 'https://github.com/shyamsridhar123/ecorp/pull/8',
  state: 'OPEN',
  isDraft: false,
  headRefName: branch,
  baseRefName: resolvedPublicationBase,
  headRefOid: source.head_commit,
  headRepositoryOwner: { login: 'shyamsridhar123' },
  isCrossRepository: false,
  autoMergeRequest: null,
  title: 'Unauthorized replacement title',
  body: 'Unauthorized replacement body.',
}
await setFakeState({
  pull_requests: [forkPullRequest, unauthorizedContentPullRequest],
})
await runPublisher(demo, workItem.id, {
  expectFailure: /create GitHub pull request|already exists/,
})
publicationSnapshot = await publicationState(demo, workItem.id)
assert.equal(publicationSnapshot.publication.state, 'branch_pushed')
assert.equal(publicationSnapshot.publication.pull_request_number, null)
fakeState = JSON.parse(await readFile(statePath, 'utf8'))
assert.equal(
  fakeState.items.find((item) => item.id === workItem.source_project_item_id)
    .status,
  'In Progress',
)
await setFakeState({ pull_requests: [forkPullRequest] })

await setFakeState({ fail_pr_create_after_success: true })
await runPublisher(demo, workItem.id, {
  crashAfter: 'after_pull_request_checkpoint',
  expectCrash: true,
})
publicationSnapshot = await publicationState(demo, workItem.id)
assert.equal(publicationSnapshot.publication.state, 'pull_request_created')
assert.equal(publicationSnapshot.publication.pull_request_number, 41)
assert.equal(
  publicationSnapshot.publication.pull_request_base_ref,
  resolvedPublicationBase,
)
assert.equal(publicationSnapshot.publication.pull_request_head_sha, source.head_commit)
assert.equal(
  publicationSnapshot.publication.pull_request_head_repository_owner,
  'shyamsridhar123',
)
assert.equal(
  publicationSnapshot.publication.pull_request_is_cross_repository,
  false,
)
fakeState = JSON.parse(await readFile(statePath, 'utf8'))
assert.equal(fakeState.pull_requests.length, 2)
assert.equal(fakeState.pr_create_calls, 1)
assert.equal(fakeState.pr_create_external_success_failures, 1)
const authorizedPullRequest = fakeState.pull_requests.find(
  (pullRequest) => pullRequest.isCrossRepository === false,
)
assert.equal(authorizedPullRequest.number, 41)
assert.equal(authorizedPullRequest.headRefOid, source.head_commit)
assert.equal(authorizedPullRequest.headRepositoryOwner.login, 'shyamsridhar123')
assert.equal(
  authorizedPullRequest.url,
  'https://github.com/ShyamSridhar123/ECorp/pull/41',
)
assert.notEqual(publicationSnapshot.publication.pull_request_number, forkPullRequest.number)
assert.equal(
  fakeState.items.find((item) => item.id === workItem.source_project_item_id)
    .status,
  'In Progress',
)

await waitForPublicationLeaseExpiry(demo, workItem.id)
const projectAuthorityAttempt = await postPublicationStartOk(publicationPath, {
  ...publicationRequest,
  idempotency_key: `${effectKey}:project-authority-attempt`,
  lease_seconds: 30,
})
assert.ok(projectAuthorityAttempt.publisher_token)
await revokeMissionRoomMembership(workItem.mission_id, demo.alice_actor_id)
const projectMembershipRenewRejected = await renewPublicationAttempt(
  demo,
  projectAuthorityAttempt.publication,
  projectAuthorityAttempt.publisher_token,
  `${effectKey}:project-membership-rejected`,
)
assert.equal(projectMembershipRenewRejected.response.status, 403)
assert.match(
  projectMembershipRenewRejected.body.error,
  /not a member of this room/,
)
fakeState = JSON.parse(await readFile(statePath, 'utf8'))
assert.equal(
  fakeState.items.find((item) => item.id === workItem.source_project_item_id)
    .status,
  'In Progress',
)
await restoreMissionRoomMembership(workItem.mission_id, demo.alice_actor_id)
assert.equal(
  Number(
    await psql(
      `SELECT COUNT(*) FROM corp_budget_policies WHERE corp_id = ${sqlLiteral(demo.corp_id)}::uuid;`,
    ),
  ),
  0,
)
await psql(`
  INSERT INTO corp_budget_policies
    (corp_id, actor_tokens_per_24h, actor_cost_microusd_per_24h,
     corp_tokens_per_24h, corp_cost_microusd_per_24h,
     no_progress_event_limit, repeated_tool_limit)
  VALUES
    (${sqlLiteral(demo.corp_id)}::uuid, 500000, 10000000, 1, 100000000, 8, 5);
  UPDATE runs
  SET input_tokens = 1, output_tokens = 0
  WHERE id = ${sqlLiteral(run.id)}::uuid;
`)
const projectCorpBudgetRejected = await renewPublicationAttempt(
  demo,
  projectAuthorityAttempt.publication,
  projectAuthorityAttempt.publisher_token,
  `${effectKey}:project-corp-budget-rejected`,
)
assert.equal(projectCorpBudgetRejected.response.status, 400)
assert.match(projectCorpBudgetRejected.body.error, /budget|hard breaker/)
fakeState = JSON.parse(await readFile(statePath, 'utf8'))
assert.equal(
  fakeState.items.find((item) => item.id === workItem.source_project_item_id)
    .status,
  'In Progress',
)
await psql(`
  UPDATE runs
  SET input_tokens = ${usageBefore[0]}, output_tokens = ${usageBefore[1]}
  WHERE id = ${sqlLiteral(run.id)}::uuid;
  DELETE FROM corp_budget_policies
  WHERE corp_id = ${sqlLiteral(demo.corp_id)}::uuid;
`)
await failPublicationAttempt(
  demo,
  projectAuthorityAttempt.publication,
  projectAuthorityAttempt.publisher_token,
  `${effectKey}:release-project-authority-test`,
  'Release the Project authority-revocation test attempt.',
)

fakeState = JSON.parse(await readFile(statePath, 'utf8'))
const projectEditsBeforePrMutation = fakeState.item_edits
await setFakeState({
  pr_list_mutation: {
    call: (fakeState.pr_list_calls ?? 0) + 2,
    number: 41,
    patch: { state: 'CLOSED' },
  },
})
await runPublisher(demo, workItem.id, {
  expectFailure:
    /persisted publication pull request no longer matches|authorized non-merging publication/,
})
fakeState = JSON.parse(await readFile(statePath, 'utf8'))
assert.equal(fakeState.item_edits, projectEditsBeforePrMutation)
assert.equal(
  fakeState.items.find((item) => item.id === workItem.source_project_item_id)
    .status,
  'In Progress',
)
assert.equal(fakeState.pr_list_mutations_applied, 1)
await setFakeState({
  pull_requests: fakeState.pull_requests.map((pullRequest) =>
    pullRequest.number === 41
      ? { ...pullRequest, state: 'OPEN' }
      : pullRequest,
  ),
  pr_list_mutation: null,
})

await runPublisher(demo, workItem.id, {
  crashAfter: 'after_project_remote',
  expectCrash: true,
})
publicationSnapshot = await publicationState(demo, workItem.id)
assert.equal(publicationSnapshot.publication.state, 'pull_request_created')
fakeState = JSON.parse(await readFile(statePath, 'utf8'))
assert.equal(
  fakeState.items.find((item) => item.id === workItem.source_project_item_id)
    .status,
  'In Review',
)

await waitForPublicationLeaseExpiry(demo, workItem.id)
const concurrent = await Promise.all([
  runPublisher(demo, workItem.id, {
    idempotencyKey: `${effectKey}:concurrent-a`,
  }),
  runPublisher(demo, workItem.id, {
    idempotencyKey: `${effectKey}:concurrent-b`,
  }),
])
assert.equal(concurrent[0].publication.pull_request_number, 41)
assert.equal(concurrent[1].publication.pull_request_number, 41)
assert.equal(concurrent[0].publication.id, concurrent[1].publication.id)

const finalSnapshot = await snapshot(demo)
const finalPublicationContext = await publicationContext(demo, workItem.id)
const publication = finalPublicationContext.publication
assert.equal(publication.state, 'published')
assert.equal(publication.pull_request_number, 41)
assert.equal(publication.pull_request_url, authorizedPullRequest.url)
assert.equal(publication.pull_request_head_sha, source.head_commit)
assert.equal(publication.pull_request_head_repository_owner, 'shyamsridhar123')
assert.equal(publication.pull_request_is_cross_repository, false)
assert.equal(publication.project_status_after, 'In Review')
assert.equal(publication.auto_merge_enabled, false)
assert.equal(publication.merge_authorized, false)
assert.equal(publication.deployment_authorized, false)
assert.equal(finalPublicationContext.work_item.state, 'published')
assert.equal(
  finalPublicationContext.source_deliverables.find(
    (item) => item.id === source.id,
  ).integration_state,
  'published',
)
const attempts = finalSnapshot.snapshot.pull_request_publication_attempts
  .filter((attempt) => attempt.publication_id === publication.id)
  .sort((left, right) => left.attempt - right.attempt)
assert.ok(attempts.length >= 7)
assert.equal(attempts.at(-1).state, 'published')
assert.ok(attempts.some((attempt) => attempt.state === 'abandoned'))
assert.equal(
  finalSnapshot.snapshot.pull_request_publications.filter(
    (item) => item.factory_work_item_id === workItem.id,
  ).length,
  1,
)

fakeState = JSON.parse(await readFile(statePath, 'utf8'))
assert.equal(fakeState.pull_requests.length, 2)
assert.equal(
  fakeState.pull_requests.filter(
    (pullRequest) => pullRequest.isCrossRepository === false,
  ).length,
  1,
)
assert.equal(fakeState.pr_create_calls, 1)
assert.equal(
  fakeState.item_list_calls ?? 0,
  projectItemListCallsBeforePublication,
)
assert.ok((fakeState.project_item_lookup_calls ?? 0) > 0)
assert.equal(
  fakeState.field_list_calls ?? 0,
  projectFieldListCallsBeforePublication,
)
assert.ok((fakeState.project_field_lookup_calls ?? 0) > 0)
const reviewEffectIndex = fakeState.effect_log.findIndex(
  (effect) => effect.kind === 'project_status' && effect.status === 'In Review',
)
const pullRequestEffectIndex = fakeState.effect_log.findIndex(
  (effect) => effect.kind === 'pull_request_created',
)
assert.ok(pullRequestEffectIndex >= 0)
assert.ok(reviewEffectIndex > pullRequestEffectIndex)
assert.ok(
  fakeState.effect_log[reviewEffectIndex].target_pull_request_count >= 1,
  'Project entered review before the verified target-repository pull request existed',
)

const remoteBranches = (
  await execFile(
    'git',
    [
      '--git-dir',
      remotePath,
      'for-each-ref',
      '--format=%(refname):%(objectname)',
      `refs/heads/${branch}`,
    ],
    { cwd: root, windowsHide: true },
  )
).stdout
  .trim()
  .split(/\r?\n/)
  .filter(Boolean)
assert.deepEqual(remoteBranches, [
  `refs/heads/${branch}:${source.head_commit}`,
])

const pullRequestCreatesBeforePublishedRetry = fakeState.pr_create_calls
const itemEditsBeforePublishedRetry = fakeState.item_edits
await seedNewerPublications(
  publication,
  contextSeed.seed,
  contextSeed.count,
)
const boundedPublishedSnapshot = await snapshot(demo)
assert.equal(
  boundedPublishedSnapshot.snapshot.pull_request_publications.some(
    (item) => item.id === publication.id,
  ),
  false,
)
const exactPublishedContext = await publicationContext(demo, workItem.id)
assert.equal(exactPublishedContext.publication.id, publication.id)
assert.equal(exactPublishedContext.publication.state, 'published')
await execFile(
  'git',
  [
    '--git-dir',
    remotePath,
    'update-ref',
    'refs/heads/main',
    source.head_commit,
  ],
  { cwd: root, windowsHide: true },
)
const publishedRetry = await runPublisher(demo, workItem.id, {
  publisherId: 'trusted-publication-host-c',
  authorizationReason:
    'Host C verifies published recovery after context pagination and base movement.',
  leaseSeconds: 9,
})
assert.equal(publishedRetry.publication.id, publication.id)
assert.equal(publishedRetry.publication.state, 'published')
fakeState = JSON.parse(await readFile(statePath, 'utf8'))
assert.equal(fakeState.pr_create_calls, pullRequestCreatesBeforePublishedRetry)
assert.equal(fakeState.item_edits, itemEditsBeforePublishedRetry)

const durableText = [
  JSON.stringify(finalSnapshot),
  JSON.stringify(fakeState),
  await psql(
    `SELECT jsonb_build_object(
      'publications', COALESCE(jsonb_agg(to_jsonb(publication)), '[]'::jsonb)
    )::text
    FROM pull_request_publications publication
    WHERE publication.corp_id = ${sqlLiteral(demo.corp_id)}::uuid;`,
  ),
  await psql(
    `SELECT COALESCE(jsonb_agg(to_jsonb(credential)), '[]'::jsonb)::text
     FROM publication_publisher_credentials credential
     WHERE credential.corp_id = ${sqlLiteral(demo.corp_id)}::uuid;`,
  ),
].join('\n')
assert.equal(
  durableText.includes(publisherToken),
  false,
  'trusted publisher credential leaked into durable or shared state',
)
for (const credential of publisherCredentialSecrets) {
  assert.equal(
    durableText.includes(credential),
    false,
    'publication publisher workload credential leaked into durable or shared state',
  )
}
assert.equal(
  finalSnapshot.snapshot.events.some((event) =>
    JSON.stringify(event).includes(publisherToken),
  ),
  false,
)
assert.ok(
  finalSnapshot.snapshot.events.some(
    (event) =>
      event.type === 'factory.publication_completed' &&
      event.aggregate_id === publication.id,
  ),
)

const report = {
  checked_at: new Date().toISOString(),
  base_branch_collision_rejected: remoteMainAfter === remoteMainBefore,
  invalid_git_branches_rejected_before_start: true,
  custom_title_normalized: collisionPublication.title === collisionCustomTitle,
  mixed_case_repository_normalized:
    collisionPublication.target_repository === 'shyamsridhar123/ecorp',
  mixed_case_pull_request_url_accepted:
    publication.pull_request_url ===
    'https://github.com/ShyamSridhar123/ECorp/pull/41',
  implicit_authorization_retry_stable: true,
  cross_publisher_default_start_recovery:
    collisionPublication.publisher_id === 'trusted-publication-host-b' &&
    collisionPublication.attempt_count === 2,
  body_file_crlf_normalized: true,
  implicit_authorization_restart_pid: collisionRestartedServerPid,
  actor_handoff_authorization_distinct: true,
  published_retry_after_base_move: true,
  exact_publication_context_lookup: true,
  publisher_enrollment_manage_only:
    memberPublisherEnrollment.response.status,
  human_only_start_rejection: humanOnlyStartRejected.response.status,
  missing_publisher_credential_no_start: true,
  invalid_publisher_credential_no_start: true,
  cross_room_publication_context_denial:
    crossRoomPublicationContext.status,
  cross_room_publication_start_rejection:
    crossRoomStartRejected.response.status,
  cross_room_publication_recovery_rejection:
    crossRoomRecoveryRejected.response.status,
  cross_room_publication_status_denial: crossRoomStatus.response.status,
  pre_branch_room_membership_renewal_rejection:
    branchMembershipRenewRejected.response.status,
  pre_pull_request_room_membership_renewal_rejection:
    pullRequestMembershipRenewRejected.response.status,
  pre_project_room_membership_renewal_rejection:
    projectMembershipRenewRejected.response.status,
  pr_revalidated_after_project_renewal:
    fakeState.pr_list_mutations_applied === 1,
  bounded_snapshot_work_item_and_deliverable_absent: true,
  published_retry_outside_bounded_snapshot: true,
  exact_project_item_lookup:
    (fakeState.project_item_lookup_calls ?? 0) > 0 &&
    (fakeState.item_list_calls ?? 0) === projectItemListCallsBeforePublication,
  exact_project_field_lookup:
    (fakeState.project_field_lookup_calls ?? 0) > 0 &&
    (fakeState.field_list_calls ?? 0) ===
      projectFieldListCallsBeforePublication,
  project_item_count_during_publication: expandedProjectItems.length,
  project_field_count_during_publication: expandedProjectFields.length,
  unauthorized_pr_content_rejected: true,
  factory_work_item_id: workItem.id,
  mission_id: firstController.mission_id,
  source_deliverable_id: source.id,
  source_sha256: source.sha256,
  verification_sha256: source.verification_sha256,
  commit_sha: source.head_commit,
  branch,
  publication_id: publication.id,
  publication_attempts: attempts.length,
  pull_request_number: publication.pull_request_number,
  pull_request_url: publication.pull_request_url,
  pull_request_head_sha: publication.pull_request_head_sha,
  pull_request_head_repository_owner:
    publication.pull_request_head_repository_owner,
  fork_pull_request_rejected:
    publication.pull_request_number !== forkPullRequest.number,
  pull_request_create_calls: fakeState.pr_create_calls,
  publication_base_ref: publication.base_ref,
  resolved_pull_request_base_ref: publication.pull_request_base_ref,
  remote_branch_count: remoteBranches.length,
  project_status: publication.project_status_after,
  project_after_pull_request: reviewEffectIndex > pullRequestEffectIndex,
  server_restart_pid: restartedServerPid,
  policy_rejection: policyRejected.response.status,
  role_rejection: roleRejected.response.status,
  corp_rejection: corpRejected.response.status,
  budget_rejection: budgetRejected.response.status,
  breaker_rejection: breakerRejected.response.status,
  post_start_role_renewal_rejection: roleRenewRejected.response.status,
  post_start_breaker_renewal_rejection:
    postStartBreakerRejected.response.status,
  pre_pull_request_budget_renewal_rejection:
    pullRequestBudgetRejected.response.status,
  pre_project_corp_budget_renewal_rejection:
    projectCorpBudgetRejected.response.status,
  human_only_renew_rejection: humanOnlyRenewRejected.response.status,
  human_only_branch_checkpoint_rejection:
    humanOnlyBranchCheckpointRejected.response.status,
  human_only_failure_checkpoint_rejection:
    humanOnlyFailureRejected.response.status,
  human_only_completion_checkpoint_rejection:
    humanOnlyCompletionRejected.response.status,
  mismatched_publisher_identity_rejection:
    mismatchedPublisherRenewRejected.response.status,
  revoked_publisher_credential_rejection:
    revokedCredentialRenewRejected.response.status,
  expired_publisher_credential_rejection:
    expiredCredentialRenewRejected.response.status,
  cross_corp_publisher_credential_rejection:
    crossCorpCredentialRenewRejected.response.status,
  publisher_workload_auth_survived_restart: true,
  credential_non_disclosure:
    !durableText.includes(publisherToken) &&
    publisherCredentialSecrets.every(
      (credential) => !durableText.includes(credential),
    ),
  auto_merge: publication.auto_merge_enabled,
  merge_authorized: publication.merge_authorized,
  deployment_authorized: publication.deployment_authorized,
}
await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`)
console.log(JSON.stringify(report, null, 2))
