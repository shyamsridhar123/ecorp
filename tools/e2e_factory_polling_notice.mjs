// Supplemental live notice check against a controlled 503, with existing work preserved.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { mkdir, readFile, writeFile, open } from 'node:fs/promises'
import path from 'node:path'
assert.equal(process.env.CRONY_POLLING_TEST, '1')
const server = process.env.CRONY_SERVER_HTTP
assert.equal(new URL(server).origin, 'http://127.0.0.1:18967')
const output = process.env.CRONY_POLLING_OUTPUT
assert.ok(output && process.env.CRONY_POLLING_FIXTURE && process.env.CRONY_CLI_BINARY && process.env.ECORP_TEST_SOURCE_REPOSITORY)
await mkdir(output, { recursive: true })
const old = JSON.parse(await readFile(process.env.CRONY_POLLING_FIXTURE, 'utf8'))
const fixture = path.join(output, 'github-state.json')
old.graphql_failures.push({ match: 'project_resolve',
  calls: [(old.graphql_query_counts.project_resolve ?? 0) + 1], kind: '503', retry_after: 90 })
await writeFile(fixture, JSON.stringify(old, null, 2), { flag: 'wx' })
const corp = '00000000-0000-4000-8000-000000000001'
const actor = '00000000-0000-4000-8000-000000000011'
const snapshot = async () => (await fetch(`${server}/api/corps/${corp}/snapshot?actor_id=${actor}`).then((r) => r.json())).snapshot
const before = await snapshot()
assert.equal(before.factory_controllers.length, 1)
assert.equal(before.factory_controllers[0].status, 'offline')
const id = before.factory_controllers[0].id
const root = path.resolve(import.meta.dirname, '..')
const stdout = await open(path.join(output, 'controller.stdout.log'), 'a')
const stderr = await open(path.join(output, 'controller.stderr.log'), 'a')
const child = spawn(process.env.CRONY_CLI_BINARY, ['--server', server, 'factory-watch', corp, actor,
  '--controller-id', id, '--owner', 'ecorp-fixtures', '--project-number', '9',
  '--repository', 'ecorp-fixtures/quota-intake', '--source-repository-path', process.env.ECORP_TEST_SOURCE_REPOSITORY,
  '--source-base-ref', 'HEAD', '--publication-base-ref', 'main',
  '--adapter', 'fake-process', '--strategy', 'single', '--budget-tokens', '50000',
  '--budget-cost-microusd', '1000000', '--interval-seconds', '5', '--heartbeat-seconds', '5',
  '--github-cli', process.execPath,
], { cwd: root, windowsHide: true, stdio: ['ignore', stdout.fd, stderr.fd],
  env: { ...process.env, ECORP_FAKE_GITHUB_STATE: fixture,
    ECORP_GITHUB_CLI_PREFIX_ARGS_JSON: JSON.stringify([path.join(root, 'tools/fake_github_cli.mjs')]),
    GH_TOKEN: '', GITHUB_TOKEN: '', CRONY_ACCESS_TOKEN: '' } })
const result = { passed: false, controller_pid: child.pid, started_at: new Date().toISOString() }
try {
  const deadline = Date.now() + 60_000
  let state
  while (Date.now() < deadline) {
    assert.equal(child.exitCode, null)
    state = await snapshot()
    if (state.factory_controllers[0].polling.retry_reason === 'github_unavailable' &&
        Date.parse(state.factory_controllers[0].polling.next_retry_at) > Date.now()) break
    await new Promise((resolve) => setTimeout(resolve, 200))
  }
  assert.equal(state.factory_controllers[0].polling.retry_reason, 'github_unavailable')
  const browser = spawn(process.execPath, [path.join(root, 'tools/e2e_factory_polling_browser.mjs')], {
    cwd: root, windowsHide: true, stdio: ['ignore', stdout.fd, stderr.fd],
    env: { ...process.env, CRONY_POLLING_REASON: 'github_unavailable' },
  })
  const [code] = await once(browser, 'exit')
  assert.equal(code, 0)
  const after = await snapshot()
  for (const key of ['missions', 'tasks', 'runs', 'factory_work_items']) {
    assert.deepEqual(after[key].map((item) => item.id).sort(), before[key].map((item) => item.id).sort())
  }
  result.polling = state.factory_controllers[0].polling
  result.no_new_work = true
  result.passed = true
} catch (error) {
  result.error = error.message
  throw error
} finally {
  if (child.exitCode === null && child.signalCode === null) {
    const exited = once(child, 'exit'); child.kill(); await exited
  }
  result.controller_ended_at = new Date().toISOString()
  await stdout.close(); await stderr.close()
  await writeFile(path.join(output, 'notice-evidence.json'), JSON.stringify(result, null, 2))
}
console.log(JSON.stringify(result, null, 2))
