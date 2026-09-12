import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import test from 'node:test'
import { ciOwnedServerConfig, ciOwnedServerPreview } from './ci_owned_test_server.mjs'

const root = path.resolve(import.meta.dirname, '..')
const env = {
  GITHUB_ACTIONS: 'true', CI: 'true', RUNNER_OS: 'Linux', GITHUB_JOB: 'integration',
  GITHUB_RUN_ID: '12345', GITHUB_WORKSPACE: root, ECORP_CI_OWNED_SERVER: '1',
  CRONY_SERVER_HTTP: 'http://127.0.0.1:18471',
  CRONY_TEST_SERVER_BINARY: path.join(root, 'target', 'debug', 'crony-server'),
  CRONY_TEST_SERVER_PID_FILE: path.join(root, 'output', 'server-ci.json'),
  DATABASE_URL: 'postgres://crony:crony@127.0.0.1:55471/crony',
}

test('CI server preview is explicit, isolated, read-only and credential-free', () => {
  const plan = ciOwnedServerPreview(ciOwnedServerConfig(['--dry-run'], env))
  assert.equal(plan.services_started, false)
  assert.equal(plan.database_writes, false)
  assert.equal(plan.database_url_disclosed, false)
  assert.equal(plan.server, env.CRONY_SERVER_HTTP)
  assert.equal(plan.database_port, 55471)
  assert.equal(plan.manifest, env.CRONY_TEST_SERVER_PID_FILE)
  assert.equal(JSON.stringify(plan).includes(env.DATABASE_URL), false)
  for (const mode of ['--start', '--stop']) assert.equal(ciOwnedServerConfig([mode], env).operation, mode.slice(2))
})

test('CI server refuses missing scope, legacy/manual targets and unexpected database configuration', () => {
  for (const args of [[], ['--execute'], ['--start', '--stop'], ['--dry-run', '--dry-run']]) {
    assert.throws(() => ciOwnedServerConfig(args, env))
  }
  for (const [key, value] of [
    ['GITHUB_ACTIONS', undefined], ['CI', undefined], ['RUNNER_OS', 'Windows'], ['GITHUB_JOB', 'quality'],
    ['GITHUB_RUN_ID', '0'], ['GITHUB_RUN_ID', '123x'], ['GITHUB_WORKSPACE', path.dirname(root)],
    ['ECORP_CI_OWNED_SERVER', undefined], ['CRONY_SERVER_HTTP', 'http://127.0.0.1:8791'],
    ['CRONY_SERVER_HTTP', 'http://localhost:18471'], ['CRONY_SERVER_HTTP', 'https://example.com:18471'],
    ['CRONY_TEST_SERVER_PID_FILE', path.join(root, 'output', 'server-ci.pid')],
    ['CRONY_TEST_SERVER_PID_FILE', path.join(root, 'output', 'local-pids.json')],
    ['CRONY_TEST_SERVER_BINARY', undefined], ['CRONY_TEST_SERVER_BINARY', '/unrelated/server'],
    ['DATABASE_URL', undefined], ['DATABASE_URL', 'postgres://crony:secret@127.0.0.1:54329/crony'],
    ['DATABASE_URL', 'postgres://crony:crony@remote:55471/crony'],
  ]) assert.throws(() => ciOwnedServerConfig(['--dry-run'], { ...env, [key]: value }))
})

test('all CI restart fixtures consume the same explicit ownership manifest without numeric-PID fallbacks', () => {
  const workflow = readFileSync(path.join(root, '.github', 'workflows', 'ci.yml'), 'utf8')
  const integration = workflow.split('  integration:')[1].split('  external-adapters-windows:')[0]
  assert.match(integration, /CRONY_SERVER_HTTP: http:\/\/127\.0\.0\.1:18471/u)
  assert.match(integration, /CRONY_TEST_SERVER_BINARY: .*target\/debug\/crony-server/u)
  assert.match(integration, /CRONY_TEST_SERVER_PID_FILE: .*output\/server-ci\.json/u)
  assert.match(integration, /55471:5432/u)
  assert.match(integration, /e2e_smoke\.ps1 -Server \$env:CRONY_SERVER_HTTP/u)
  assert.ok(integration.indexOf('ci_owned_test_server.mjs --dry-run') < integration.indexOf('ci_owned_test_server.mjs --start'))
  assert.match(integration, /Stop the ownership-verified CI server\n\s+if: always\(\)\n\s+run: node tools\/ci_owned_test_server\.mjs --stop/u)
  assert.doesNotMatch(integration, /8791|54329|server-ci\.pid|--database-url|target\/debug\/crony-server \\/u)
  for (const file of ['e2e_artifact_staging.mjs', 'e2e_factory_claims.mjs', 'e2e_factory_publication.mjs', 'e2e_approvals.mjs']) {
    const code = readFileSync(path.join(root, 'tools', file), 'utf8')
    assert.match(code, /restartOwnedTestServer/u)
    assert.doesNotMatch(code, /process\.kill\((serverPid|pid|manifest\.server)/u)
    assert.doesNotMatch(code, /jsonPidFile|local-pids\.json/u)
  }
  const artifact = readFileSync(path.join(root, 'tools', 'e2e_artifact_staging.mjs'), 'utf8')
  assert.match(artifact, /CRONY_ARTIFACT_RECOVERY_GRACE_SECS: '0'/u)
  assert.match(artifact, /CRONY_ARTIFACT_RECOVERY_INTERVAL_SECS: '1'/u)
})
