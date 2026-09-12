import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { startOwnedTestServer, stopOwnedTestServer } from './owned_test_stack.mjs'

const root = path.resolve(import.meta.dirname, '..')

export function ciOwnedServerConfig(args, env) {
  assert.ok(args.length === 1 && ['--dry-run', '--start', '--stop'].includes(args[0]),
    'Choose exactly one CI server operation: --dry-run, --start, or --stop')
  assert.ok(env.GITHUB_ACTIONS === 'true' && env.CI === 'true' && env.RUNNER_OS === 'Linux' &&
    env.GITHUB_JOB === 'integration' && /^[1-9][0-9]*$/u.test(env.GITHUB_RUN_ID ?? '') &&
    env.ECORP_CI_OWNED_SERVER === '1', 'Only the explicit Actions integration fixture is supported')
  assert.equal(path.resolve(env.GITHUB_WORKSPACE ?? ''), root, 'Unexpected Actions workspace')
  assert.equal(env.CRONY_SERVER_HTTP, 'http://127.0.0.1:18471', 'The CI test port must be explicit')
  assert.equal(env.CRONY_TEST_SERVER_PID_FILE, path.join(root, 'output', 'server-ci.json'))
  assert.equal(env.CRONY_TEST_SERVER_BINARY, path.join(root, 'target', 'debug', 'crony-server'))
  // These are disposable Actions service credentials, never the operator DB.
  assert.ok(env.DATABASE_URL === 'postgres://crony:crony@127.0.0.1:55471/crony',
    'Expected the isolated Actions PostgreSQL service; database values are not disclosed')
  return { root, server: env.CRONY_SERVER_HTTP, binary: env.CRONY_TEST_SERVER_BINARY,
    manifestPath: env.CRONY_TEST_SERVER_PID_FILE, databaseUrl: env.DATABASE_URL,
    operation: args[0].slice(2), logPrefix: 'server-ci' }
}

export function ciOwnedServerPreview(config) {
  return { operation: config.operation, server: config.server, binary: config.binary,
    manifest: config.manifestPath, database_port: 55471,
    ownership: 'new child receipt; Linux boot ID, start ticks, executable, cwd, uid, network namespace and listener; pidfd-only stop',
    services_started: false, database_writes: false, database_url_disclosed: false }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    const config = ciOwnedServerConfig(process.argv.slice(2), process.env)
    if (config.operation === 'dry-run') {
      console.log(JSON.stringify(ciOwnedServerPreview(config), null, 2))
    } else {
      assert.equal(process.platform, 'linux', 'Only Linux Actions may execute this CI entry point')
      if (config.operation === 'stop' && !existsSync(config.manifestPath)) {
        console.log(JSON.stringify({ stopped: false, reason: 'No CI-owned server was recorded.' }))
      } else {
        const result = config.operation === 'start'
          ? await startOwnedTestServer(config)
          : await stopOwnedTestServer(config)
        console.log(JSON.stringify({ operation: config.operation, result, manifest: config.manifestPath }))
      }
    }
  } catch (error) {
    // Assertion objects can contain actual values. Do not print configuration.
    console.error(error.message.split('\n')[0])
    process.exitCode = 1
  }
}
