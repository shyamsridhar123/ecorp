// Negative conformance: drift after claim must block effects, not launch work.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { mkdir, writeFile } from 'node:fs/promises'
import path from 'node:path'
assert.equal(process.env.CRONY_POLLING_TEST, '1')
const server = process.env.CRONY_SERVER_HTTP
assert.equal(new URL(server).origin, 'http://127.0.0.1:18967')
const output = process.env.CRONY_POLLING_OUTPUT
assert.ok(output && process.env.CRONY_CLI_BINARY && process.env.ECORP_TEST_SOURCE_REPOSITORY)
await mkdir(output, { recursive: true })
const corp = '00000000-0000-4000-8000-000000000001', actor = '00000000-0000-4000-8000-000000000011'
const snapshot = async () => (await fetch(`${server}/api/corps/${corp}/snapshot?actor_id=${actor}`).then((r) => r.json())).snapshot
const before = await snapshot()
assert.ok(!before.runs.some((run) => ['starting', 'running', 'verifying', 'waiting_for_approval'].includes(run.status)))
const now = new Date().toISOString()
const issue = { id: 'I_drift_3001', number: 3001, title: 'Source drift negative fixture',
  body: 'Initial immutable issue body.', url: 'https://github.com/ecorp-fixtures/quota-intake/issues/3001',
  state: 'OPEN', createdAt: now, updatedAt: now, labels: [{ name: 'factory:ready' }] }
const fixture = path.join(output, 'github-state.json')
await writeFile(fixture, JSON.stringify({
  repository: 'ecorp-fixtures/quota-intake',
  project: { id: 'PVT_quota_fixture', owner: 'ecorp-fixtures', number: 9,
    status_field_id: 'PVTSSF_quota_status',
    status_options: [{ id: 'todo', name: 'Todo' }, { id: 'progress', name: 'In Progress' }] },
  items: [{ id: 'PVTI_drift_3001', status: 'Todo', content: { type: 'Issue', ...issue,
    repository: 'ecorp-fixtures/quota-intake' } }],
  issues: { 3001: issue },
  item_list_mutation: { call: 3, issue_number: 3001,
    patch: { body: 'Changed issue after the claim.', updatedAt: new Date(Date.now() + 1000).toISOString() } },
}, null, 2), { flag: 'wx' })
const root = path.resolve(import.meta.dirname, '..')
const child = spawn(process.env.CRONY_CLI_BINARY, ['--server', server, 'factory', corp, actor,
  '--owner', 'ecorp-fixtures', '--project-number', '9', '--repository', 'ecorp-fixtures/quota-intake',
  '--source-repository-path', process.env.ECORP_TEST_SOURCE_REPOSITORY,
  '--source-base-ref', 'HEAD', '--publication-base-ref', 'main', '--issue', '3001',
  '--adapter', 'fake-process', '--strategy', 'single', '--budget-tokens', '50000',
  '--budget-cost-microusd', '1000000', '--github-cli', process.execPath,
], { cwd: root, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
  env: { ...process.env, ECORP_FAKE_GITHUB_STATE: fixture,
    ECORP_GITHUB_CLI_PREFIX_ARGS_JSON: JSON.stringify([path.join(root, 'tools/fake_github_cli.mjs')]),
    GH_TOKEN: '', GITHUB_TOKEN: '', CRONY_ACCESS_TOKEN: '' } })
const chunks = []
child.stdout.on('data', (chunk) => chunks.push(chunk))
child.stderr.on('data', (chunk) => chunks.push(chunk))
const [code] = await once(child, 'exit')
await writeFile(path.join(output, 'controller.log'), Buffer.concat(chunks))
assert.notEqual(code, 0)
const after = await snapshot()
const item = after.factory_work_items.find((entry) => entry.source_issue_number === 3001)
assert.ok(item)
assert.equal(item.state, 'blocked')
assert.match(item.failure_detail, /source revalidation|source revision|content changed|stale/iu)
assert.ok(item.mission_id)
assert.equal(after.missions.find((mission) => mission.id === item.mission_id).status, 'ready')
assert.deepEqual(after.runs.map((run) => run.id).sort(), before.runs.map((run) => run.id).sort())
const report = { passed: true, scope: 'Deterministic after-claim source-drift denial',
  item_id: item.id, mission_id: item.mission_id, run_count_before: before.runs.length,
  run_count_after: after.runs.length, failure_detail: item.failure_detail }
await writeFile(path.join(output, 'source-drift-evidence.json'), JSON.stringify(report, null, 2))
console.log(JSON.stringify(report, null, 2))
