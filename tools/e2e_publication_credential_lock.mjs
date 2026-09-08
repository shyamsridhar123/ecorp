// Native PostgreSQL regression for the exact credential SELECT/UPDATE used by
// publication.rs. The parent starts an existing owned QA database; this script
// creates only a uniquely named synthetic schema and removes that schema.
import assert from 'node:assert/strict'
import { execFile, spawn } from 'node:child_process'
import { createHash, randomUUID } from 'node:crypto'
import { readFile } from 'node:fs/promises'
import { promisify } from 'node:util'

const execute = promisify(execFile)
assert.equal(process.env.ECORP_PUBLICATION_LOCK_TEST, '1', 'Explicit owned-fixture opt-in required')
assert.equal(process.argv.length, 2, 'This regression accepts no arguments')
const container = process.env.ECORP_TEST_POSTGRES_CONTAINER ?? ''
assert.match(container, /^ecorp-r143-(?:publication-final|acceptance-v2)-\d{8}$/)
const ownership = JSON.parse((await execute('docker', ['inspect', '--format',
  '{"id":{{json .Id}},"running":{{json .State.Running}},"labels":{{json .Config.Labels}},"ports":{{json .HostConfig.PortBindings}}}',
  container], { windowsHide: true, timeout: 10_000 })).stdout)
assert.equal(ownership.running, true, 'The parent must start its existing QA database')
assert.equal(ownership.labels?.['ecorp.fixture'], 'issue143-current')
assert.ok(ownership.ports['5432/tcp'].every((port) =>
  port.HostIp === '127.0.0.1' && !['5432', '54441'].includes(port.HostPort)),
'Never target the manual/shared database')

const source = await readFile(new URL('../crates/crony-store/src/publication.rs', import.meta.url), 'utf8')
const constant = 'PUBLICATION_PUBLISHER_CREDENTIAL_LOCK_SQL'
const select = source.match(new RegExp(`const ${constant}: &str = r#"([\\s\\S]*?)"#;`))?.[1]
assert.ok(select, 'The exact production credential-lock SQL must be available')
const functionBody = source.slice(source.indexOf('async fn revalidate_publication_publisher_credential_tx('),
  source.indexOf('\nfn replayable_publication_token('))
assert.ok(functionBody.includes(`sqlx::query_scalar::<_, Uuid>(${constant})`),
  'Production revalidation must use the selected SQL')
const update = functionBody.match(/sqlx::query\("(UPDATE publication_publisher_credentials[^"]+)"\)/)?.[1]
assert.ok(update, 'Use the exact production last-used update')
const hash = (value) => createHash('sha256').update(value).digest('hex')
const tag = randomUUID().replaceAll('-', '')
const schema = `ecorp_pub_lock_${tag}`
assert.match(schema, /^ecorp_pub_lock_[a-f0-9]{32}$/)
const credentialId = randomUUID()
const corpId = randomUUID()
const publisher = `fixture-${tag}`
const credentialHash = hash(`synthetic-not-a-secret-${tag}`)
const applicationA = `ecorp-lock-${tag}-a`
const applicationB = `ecorp-lock-${tag}-b`
const args = ['exec', '-i', ownership.id, 'psql', '-X', '-q', '-U', 'crony', '-d', 'crony',
  '-At', '-v', 'ON_ERROR_STOP=1']
const report = {
  suite: 'publication-credential-lock',
  status: 'in_progress',
  started_at: new Date().toISOString(),
  source_sha256: hash(source),
  select_sha256: hash(select),
  update_sha256: hash(update),
  lock_clause: select.match(/FOR (?:NO KEY )?(?:UPDATE|SHARE)/)?.[0],
  container_id: ownership.id,
  synthetic_schema: schema,
  provider_started: false,
  cleanup_complete: false,
}
let created = false
const sessions = []

async function sql(text) {
  return new Promise((resolve, reject) => {
    const child = spawn('docker', args, { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] })
    let stdout = '', stderr = ''
    const timer = setTimeout(() => { child.kill(); reject(new Error('Bounded SQL command timed out')) }, 20_000)
    child.stdout.on('data', (value) => { stdout += value })
    child.stderr.on('data', (value) => { stderr += value })
    child.once('error', (error) => { clearTimeout(timer); reject(error) })
    child.once('close', (code) => {
      clearTimeout(timer)
      if (code !== 0) reject(new Error(`SQL command failed: ${stderr.slice(0, 600)}`))
      else resolve(stdout.trim())
    })
    child.stdin.end(`SET statement_timeout = '15s';\n${text}\n`)
  })
}

function session(applicationName) {
  const child = spawn('docker', args, { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] })
  const state = { child, stdout: '', stderr: '', code: null }
  state.done = new Promise((resolve, reject) => {
    child.once('error', reject)
    child.once('close', (code) => { state.code = code; resolve(state) })
  })
  child.stdout.on('data', (value) => { state.stdout += value })
  child.stderr.on('data', (value) => { state.stderr += value })
  child.stdin.on('error', () => {}) // A deadlock victim can close its pipe before queued COMMIT.
  child.stdin.write(`
    SET application_name = '${applicationName}';
    SET statement_timeout = '15s';
    BEGIN;
    SET LOCAL search_path = "${schema}", pg_catalog;
    PREPARE credential_lock(uuid,text,text) AS ${select};
    PREPARE credential_touch(uuid) AS ${update};
  `)
  sessions.push(state)
  return state
}

async function waitFor(predicate, label) {
  const end = Date.now() + 12_000
  while (Date.now() < end) {
    const value = await predicate()
    if (value) return value
    await new Promise((resolve) => setTimeout(resolve, 50))
  }
  throw new Error(`Timed out waiting for ${label}`)
}

const publicFingerprint = () => sql(`
  SELECT count(*) || ':' || md5(coalesce(string_agg(id::text || ':' ||
    coalesce(last_used_at::text, '') || ':' || coalesce(revoked_at::text, ''),
    ',' ORDER BY id), '')) FROM public.publication_publisher_credentials;
`)
try {
  report.public_before = await publicFingerprint()
  await sql(`
    BEGIN;
    CREATE SCHEMA "${schema}";
    CREATE TABLE "${schema}".publication_publisher_credentials (
      id uuid PRIMARY KEY, corp_id uuid NOT NULL, publisher_id text NOT NULL,
      credential_hash text NOT NULL, revoked_at timestamptz, expires_at timestamptz NOT NULL,
      last_used_at timestamptz
    );
    INSERT INTO "${schema}".publication_publisher_credentials
      VALUES ('${credentialId}', '${corpId}', '${publisher}', '${credentialHash}',
              NULL, now() + interval '10 minutes', NULL);
    COMMIT;
  `)
  created = true
  const a = session(applicationA)
  a.child.stdin.write(`EXECUTE credential_lock('${corpId}', '${publisher}', '${credentialHash}');\n\\echo A_LOCKED\n`)
  await waitFor(() => a.stdout.includes('A_LOCKED'), 'A to hold the production credential lock')
  assert.ok(a.stdout.includes(credentialId), 'A must select the synthetic credential')
  const b = session(applicationB)
  b.child.stdin.end(`EXECUTE credential_lock('${corpId}', '${publisher}', '${credentialHash}');\nEXECUTE credential_touch('${credentialId}');\nCOMMIT;\n\\echo B_DONE\n`)
  const blocked = await waitFor(async () => {
    const value = await sql(`SELECT json_build_object('wait_type',wait_event_type,'query',query)
      FROM pg_stat_activity WHERE application_name='${applicationB}';`)
    const row = value ? JSON.parse(value) : null
    return row?.wait_type === 'Lock' ? row : null
  }, 'B to be observably blocked on the same credential')
  report.second_session_blocked_at = blocked.query.trimStart().startsWith('EXECUTE credential_lock')
    ? 'credential SELECT' : 'last-used UPDATE'
  a.child.stdin.end(`EXECUTE credential_touch('${credentialId}');\nCOMMIT;\n\\echo A_DONE\n`)
  let terminationTimer
  const results = await Promise.race([
    Promise.all([a.done, b.done]),
    new Promise((_, reject) => {
      terminationTimer = setTimeout(() => reject(new Error('Credential sessions did not terminate')), 20_000)
    }),
  ]).finally(() => clearTimeout(terminationTimer))
  report.sessions = results.map((result, index) => ({
    name: index === 0 ? 'A' : 'B',
    exit_code: result.code,
    completed: result.stdout.includes(index === 0 ? 'A_DONE' : 'B_DONE'),
    error: result.stderr.replaceAll(credentialHash, '[synthetic hash]').slice(0, 700),
  }))
  assert.ok(report.sessions.every((result) => result.exit_code === 0 && result.completed),
    'Concurrent credential revalidation must complete without a row-lock upgrade deadlock')
  assert.equal(report.second_session_blocked_at, 'credential SELECT',
    'The second transaction must wait before a shared-lock upgrade is possible')
  report.public_after = await publicFingerprint()
  assert.equal(report.public_after, report.public_before, 'Real publisher credentials changed')
  report.status = 'passed'
} catch (error) {
  report.status = 'failed'
  report.error = error.message
  process.exitCode = 1
} finally {
  await sql(`SELECT pg_terminate_backend(pid) FROM pg_stat_activity
    WHERE application_name IN ('${applicationA}', '${applicationB}')
      AND pid <> pg_backend_pid();`).catch((error) => { report.cleanup_error = error.message })
  for (const state of sessions) if (state.code === null) state.child.kill()
  if (created) {
    // Only the literal synthetic namespace created by this invocation is removed.
    await sql(`DROP SCHEMA "${schema}" CASCADE;`).catch((error) => { report.cleanup_error = error.message })
  }
  report.public_after = await publicFingerprint().catch((error) => {
    report.cleanup_error = error.message
    return null
  })
  if (report.public_before && report.public_after !== report.public_before) {
    report.cleanup_error = 'Real publisher credential fingerprint changed'
  }
  report.cleanup_complete = !report.cleanup_error
  if (!report.cleanup_complete) process.exitCode = 1
  report.finished_at = new Date().toISOString()
  console.log(JSON.stringify(report, null, 2))
}
