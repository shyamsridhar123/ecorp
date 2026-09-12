// Requires a caller-owned isolated stack. Never use the manual/demo service ports.
// Install Playwright separately or point CRONY_PLAYWRIGHT_MODULE at an installed package.
import assert from 'node:assert/strict'
import { createHash, randomUUID } from 'node:crypto'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { createRequire } from 'node:module'
import path from 'node:path'

assert.equal(process.env.CRONY_ADMISSION_TEST, '1')
assert.deepEqual(process.argv.slice(2, 3), ['--phase'])
assert.equal(process.argv.length, 4)
const phase = process.argv[3]
assert.ok(['prepare', 'resume-prepare', 'release'].includes(phase))
assert.ok(process.env.CRONY_ADMISSION_OUTPUT)
function ownedOrigin(value, forbidden) {
  assert.ok(value, 'Explicit QA URL required')
  const url = new URL(value)
  assert.equal(url.protocol, 'http:')
  assert.ok(['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname))
  assert.ok(url.port && !forbidden.includes(url.port))
  assert.ok(!url.username && !url.password && !url.search && !url.hash)
  assert.equal(url.pathname, '/')
  return url.origin
}
const server = ownedOrigin(process.env.CRONY_SERVER_HTTP, ['8791', '8991'])
const web = ownedOrigin(process.env.CRONY_ADMISSION_WEB, ['5291', '15481', '15491'])
const output = path.resolve(process.env.CRONY_ADMISSION_OUTPUT)
const checkpointPath = path.join(output, 'browser-launch-admission.json')
const require = createRequire(import.meta.url)
const { chromium } = require(process.env.CRONY_PLAYWRIGHT_MODULE || 'playwright')
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const active = new Set(['provisioning', 'starting', 'running', 'waiting_for_input', 'waiting_for_approval', 'verifying'])
const errors = []
let checkpoint

async function request(route, body) {
  const response = await fetch(`${server}${route}`, {
    method: body === undefined ? 'GET' : 'POST',
    headers: { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
    redirect: 'error',
    signal: AbortSignal.timeout(15_000),
  })
  assert.equal(response.status, 200, `${route}: HTTP ${response.status}`)
  return response.json()
}
const demo = await request('/api/demo/bootstrap', {})
const api = (suffix) => `/api/corps/${demo.corp_id}${suffix}`
const snapshot = () => request(api(`/snapshot?actor_id=${demo.alice_actor_id}`))
function view(state, id) {
  const mission = state.snapshot.missions.find((item) => item.id === id)
  assert.ok(mission, `Mission ${id} is missing; do not reset the QA database`)
  const tasks = state.snapshot.tasks.filter((item) => item.mission_id === id).sort((a, b) => a.id.localeCompare(b.id))
  const ids = new Set(tasks.map((item) => item.id))
  return { mission, tasks, runs: state.snapshot.runs.filter((item) => ids.has(item.task_id)) }
}
function authority(current) {
  const value = {
    description: current.mission.description,
    version: current.mission.specification_version,
    strategy: current.mission.strategy,
    budget_tokens: current.mission.budget_tokens,
    budget_cost_microusd: current.mission.budget_cost_microusd,
    tasks: current.tasks.map((task) => ({ id: task.id, contract: task.contract, policy: task.verification_policy })),
  }
  const text = JSON.stringify(value, (_, item) => item && typeof item === 'object' && !Array.isArray(item)
    ? Object.fromEntries(Object.entries(item).sort(([a], [b]) => a.localeCompare(b))) : item)
  return createHash('sha256').update(text).digest('hex')
}
async function assertHeld() {
  const current = view(await snapshot(), checkpoint.mission_id)
  assert.equal(current.mission.status, 'ready')
  assert.equal(current.runs.length, 0)
  assert.equal(authority(current), checkpoint.authority)
}
async function completed(id) {
  const deadline = Date.now() + 90_000
  while (Date.now() < deadline) {
    const current = view(await snapshot(), id)
    assert.ok(!['failed', 'cancelled'].includes(current.mission.status), `Mission ${id} failed`)
    if (current.mission.status === 'completed' && current.runs.every((run) =>
      !active.has(run.status) && ['removed', 'preserved'].includes(run.workspace_disposition))) return current
    await delay(100)
  }
  throw new Error(`Mission ${id} did not complete within its bounded wait`)
}
await mkdir(output, { recursive: true })
const save = () => writeFile(checkpointPath, `${JSON.stringify(checkpoint, null, 2)}\n`)
if (phase === 'prepare') {
  await assert.rejects(readFile(checkpointPath), { code: 'ENOENT' }, 'Preserve the existing checkpoint')
  checkpoint = {
    phase: 'starting', server, web, corp_id: demo.corp_id,
    title: `[slow] Browser briefing hold issue155 ${randomUUID()}`,
    prepared_at: new Date().toISOString(),
  }
} else {
  checkpoint = JSON.parse(await readFile(checkpointPath, 'utf8'))
  assert.equal(checkpoint.phase, phase === 'resume-prepare' ? 'created' : 'prepared',
    'Continue only the exact existing mission at its recorded phase')
  assert.equal(checkpoint.server, server)
  assert.equal(checkpoint.web, web)
  assert.equal(checkpoint.corp_id, demo.corp_id)
  assert.match(checkpoint.mission_id, /^[0-9a-f-]{36}$/)
  await assertHeld()
}
const browser = await chromium.launch({ channel: process.env.CRONY_BROWSER_CHANNEL || 'chrome', headless: true })
try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1050 }, reducedMotion: 'reduce' })
  page.on('pageerror', (error) => errors.push(error.message))
  await page.goto(`${web}/#missions`, { waitUntil: 'networkidle', timeout: 30_000 })
  await page.locator('.live-indicator.live-live').waitFor({ timeout: 30_000 })
  if (phase === 'prepare') {
    await page.getByRole('button', { name: 'New mission', exact: true }).click()
    await page.locator('#mission-title').fill(checkpoint.title)
    await page.locator('details.mission-advanced-options')
      .filter({ has: page.locator('#mission-description') }).locator('summary').click()
    await page.locator('#mission-description').fill('Isolated deterministic fixture: save this plan without running it; verify explicit dispatch after restart. No real AI inference.')
    const wanted = process.env.CRONY_ADMISSION_SOURCE_REPOSITORY || 'ecorp-fixture/launch-admission'
    const choices = await page.locator('#mission-repository option').evaluateAll((items) =>
      items.filter((item) => item.value).map((item) => ({ value: item.value, source: JSON.parse(item.value) })))
    const targets = choices.filter((item) => item.source[0] === wanted)
    assert.equal(targets.length, 1, 'Select exactly the owned fixture source')
    checkpoint.source = { repository: targets[0].source[0], base_ref: targets[0].source[1], base_commit: targets[0].source[2] }
    await page.locator('#mission-repository').selectOption(targets[0].value)
    await page.getByRole('checkbox', { name: /Confirm this target/ }).check()
    await page.locator('details.mission-advanced-options')
      .filter({ has: page.locator('#mission-deliverable') }).locator('summary').click()
    await page.getByRole('checkbox', { name: /Developer fixtures/ }).check()
    await page.locator('#mission-adapter').selectOption('fake-process')
    await page.locator('#mission-strategy').selectOption('single')
    await page.locator('#mission-deliverable').selectOption('review_only_report')
    await page.getByRole('checkbox', { name: /Commit verified work/ }).uncheck()
    await page.getByRole('checkbox', { name: /Save without starting/ }).check()
    await page.getByRole('button', { name: 'Review and build', exact: true }).click()
    const responsePromise = page.waitForResponse((response) =>
      response.url() === `${server}${api('/missions')}` && response.request().method() === 'POST')
    await page.getByRole('button', { name: 'Save plan', exact: true }).click()
    const response = await responsePromise
    assert.equal(response.status(), 200)
    checkpoint.mission_id = (await response.json()).mission_id
    checkpoint.phase = 'created'
    checkpoint.authority = authority(view(await snapshot(), checkpoint.mission_id))
    await save()
  } else if (phase === 'resume-prepare') {
    await page.getByRole('button').filter({ hasText: checkpoint.title }).click()
  }
  if (phase !== 'release') {
    await page.locator(`[data-mission-id="${checkpoint.mission_id}"]`)
      .locator('.status-chip-ready').filter({ hasText: 'Awaiting dispatch' }).waitFor()
    await assertHeld()
    await page.screenshot({ path: path.join(output, 'browser-held-before-restart.png'), fullPage: true })
    // Close the client before triggering unrelated work. The server must enforce the hold.
    await page.close()
    const unrelated = await request(api('/missions'), {
      requested_by: demo.alice_actor_id, preferred_adapter: 'fake-process', strategy: 'single',
      source: checkpoint.source, title: `Unrelated browser admission check ${randomUUID()}`, budget_tokens: 80_000,
    })
    await request(api(`/missions/${unrelated.mission_id}/launch`), { requested_by: demo.alice_actor_id })
    await completed(unrelated.mission_id)
    for (let index = 0; index < 12; index += 1) { await assertHeld(); await delay(100) }
    checkpoint.unrelated_mission_id = unrelated.mission_id
    checkpoint.phase = 'prepared'
  } else {
    await page.getByRole('button').filter({ hasText: checkpoint.title }).click()
    const card = page.locator(`[data-mission-id="${checkpoint.mission_id}"]`)
    await card.locator('.status-chip-ready').filter({ hasText: 'Awaiting dispatch' }).waitFor()
    await page.screenshot({ path: path.join(output, 'browser-held-after-restart.png'), fullPage: true })
    await page.setViewportSize({ width: 390, height: 844 })
    const size = await page.evaluate(() => ({ width: innerWidth, content: document.documentElement.scrollWidth }))
    assert.ok(size.content <= size.width, 'Mobile viewport must not overflow')
    checkpoint.mobile = size
    await page.screenshot({ path: path.join(output, 'browser-held-mobile.png'), fullPage: true })
    const responsePromise = page.waitForResponse((response) =>
      response.url() === `${server}${api(`/missions/${checkpoint.mission_id}/launch`)}` && response.request().method() === 'POST')
    await card.getByTestId('work-result-card').getByRole('button', { name: 'Start mission', exact: true }).click()
    const response = await responsePromise
    assert.equal(response.status(), 200)
    checkpoint.launch = await response.json()
    assert.equal(checkpoint.launch.replayed, false)
    const finished = await completed(checkpoint.mission_id)
    assert.equal(finished.runs.length, 1)
    assert.equal(finished.tasks[0].attempt_count, 1)
    assert.equal(authority(finished), checkpoint.authority)
    const replay = await request(api(`/missions/${checkpoint.mission_id}/launch`), { requested_by: demo.alice_actor_id })
    assert.equal(replay.replayed, true)
    assert.equal(replay.run_id, checkpoint.launch.run_id)
    assert.equal(view(await snapshot(), checkpoint.mission_id).runs.length, 1)
    checkpoint.phase = 'released'
    checkpoint.released_at = new Date().toISOString()
    checkpoint.completed_run_id = finished.runs[0].id
    checkpoint.replayed_without_new_attempt = true
    await page.screenshot({ path: path.join(output, 'browser-dispatched-completed.png'), fullPage: true })
  }
  assert.deepEqual(errors, [])
  checkpoint.page_errors = errors
  checkpoint.fixture = 'Actual browser/server/runner path using explicit deterministic fake-process; no real-provider claim.'
  await save()
  console.log(JSON.stringify({ phase: checkpoint.phase, mission_id: checkpoint.mission_id, checkpoint: checkpointPath }, null, 2))
} finally {
  await browser.close()
}
