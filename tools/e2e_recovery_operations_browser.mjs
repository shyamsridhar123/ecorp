// Parent-owned, pre-seeded QA only. This script never creates a fixture or starts services.
// Required: CRONY_RECOVERY_TEST=1, CRONY_SERVER_HTTP, CRONY_RECOVERY_WEB,
// CRONY_RECOVERY_OUTPUT (absolute directory), ECORP_UI_RECOVERY_FIXTURE (absolute JSON file).
// Fixture keys: corp_id, alice_actor_id, bob_actor_id, mission_id, run_id, factory_work_item_id.
// Corp/Alice/Bob must be the already-seeded development identities used by the actual App.
// Optional: CRONY_PLAYWRIGHT_MODULE, pointing to an already-installed Playwright entry/package.
// Run with node; Chrome must already be installed. Never point this at the manual UI.
//
// Only real requests: GET preflight/readback, normal App bootstrap with seed_crew=false,
// and exactly one Bob UI rejection. Routes only forward/abort; no fulfilment, mocks,
// init scripts, storage seeding, synthetic snapshots, or page-state injection.
// The current App supplies "<Bob's name> rejected the recorded verification evidence."
// It has no editable review-reason field. We verify that real handler-generated note.
// Verifier-editor changes stay in an UNSAVED draft; no mission form is submitted.
// Pure, offline regression checks only: node tools/e2e_recovery_operations_browser.mjs --pure-test
// This separate mode reads source and uses in-memory test records, never the persisted UI fixture.

import assert from 'node:assert/strict'
import { createHash, randomUUID } from 'node:crypto'
import { mkdir, readFile, stat, writeFile } from 'node:fs/promises'
import { createRequire } from 'node:module'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const fixtureKeys = [
  'corp_id', 'alice_actor_id', 'bob_actor_id', 'mission_id', 'run_id', 'factory_work_item_id',
]
const uuid = /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i
// These match the normal App bootstrap contract, not manufactured fixture state.
const seededDemo = {
  corp_id: '00000000-0000-4000-8000-000000000001',
  alice_actor_id: '00000000-0000-4000-8000-000000000011',
  bob_actor_id: '00000000-0000-4000-8000-000000000012',
  eve_actor_id: '00000000-0000-4000-8000-000000000013',
  room_id: '00000000-0000-4000-8000-000000000041',
}
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const digest = (value) => createHash('sha256').update(value).digest('hex')
const normalizedText = (value) => value.replace(/\s+/g, ' ').trim()
let server, web, fixture, output, report, page, browser, baseline
let closing = false
let rejectionArmed = false
let expectedNote = ''
const responseTasks = new Set()

function ownedOrigin(value, forbiddenPorts, name) {
  assert.ok(value?.trim(), `${name} is required`)
  const url = new URL(value)
  assert.equal(url.protocol, 'http:', `${name}: development HTTP only`)
  assert.ok(['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname), `${name}: loopback only`)
  assert.ok(url.port && !forbiddenPorts.includes(url.port), `${name}: choose a fresh QA port`)
  assert.ok(!url.username && !url.password && !url.search && !url.hash, `${name}: bare origin only`)
  assert.equal(url.pathname, '/', `${name}: root origin required`)
  return url.origin
}

function safeUrl(raw) {
  try {
    const url = new URL(raw)
    return `${url.origin}${url.pathname}` // No query values, tickets, or userinfo.
  } catch { return '[invalid URL]' }
}

function safeError(error) {
  return String(error?.message ?? error)
    .replace(/\b(?:https?|wss?):\/\/[^\s"'<>]+/gi, safeUrl)
    .replace(/\b(?:postgres(?:ql)?|mysql|redis):\/\/\S+/gi, '[connection string withheld]')
    .replace(/\bBearer\s+\S+/gi, 'Bearer [withheld]')
    .replace(/\b(token|password|secret|authorization)\s*[:=]\s*\S+/gi, '$1=[withheld]')
    .slice(0, 1600)
}

function cleanBrowser() {
  assert.equal(report.blocked_requests.length, 0, 'A forbidden request was attempted; inspect evidence')
  assert.equal(report.errors.length, 0, 'Browser/network errors occurred; inspect evidence')
}

async function until(label, read, ready, timeout = 30_000) {
  const deadline = Date.now() + timeout
  while (Date.now() < deadline) {
    cleanBrowser()
    const value = await read()
    if (ready(value)) return value
    await delay(150)
  }
  throw new Error(`Timed out waiting for ${label}`)
}

async function getJson(suffix) {
  assert.ok(suffix === '/health' || suffix.startsWith(`/api/corps/${fixture.corp_id}/snapshot?`))
  const response = await fetch(`${server}${suffix}`, {
    method: 'GET', redirect: 'error', signal: AbortSignal.timeout(15_000),
    headers: { accept: 'application/json' },
  })
  assert.equal(response.status, 200, `Read-only ${suffix.split('?')[0]}: HTTP ${response.status}`)
  const text = await response.text()
  assert.ok(Buffer.byteLength(text) <= 16 * 1024 * 1024, 'Snapshot exceeds the bounded read size')
  return JSON.parse(text)
}

async function readView(actorId) {
  assert.ok([fixture.alice_actor_id, fixture.bob_actor_id].includes(actorId))
  const { snapshot: s } = await getJson(
    `/api/corps/${fixture.corp_id}/snapshot?actor_id=${encodeURIComponent(actorId)}`,
  )
  assert.equal(s?.corp?.id, fixture.corp_id, 'Wrong persisted Corp')
  const one = (rows, id, label) => {
    const matches = rows.filter((row) => row.id === id)
    assert.equal(matches.length, 1, `Exact ${label} is absent/ambiguous; do not recreate or reset it`)
    return matches[0]
  }
  const mission = one(s.missions, fixture.mission_id, 'mission')
  const run = one(s.runs, fixture.run_id, 'run')
  const task = one(s.tasks, run.task_id, 'task')
  const item = one(s.factory_work_items, fixture.factory_work_item_id, 'factory item')
  const alice = one(s.actors, fixture.alice_actor_id, 'Alice actor')
  const bob = one(s.actors, fixture.bob_actor_id, 'Bob actor')
  const eve = one(s.actors, seededDemo.eve_actor_id, 'already-seeded Eve actor')
  one(s.rooms, seededDemo.room_id, 'already-seeded demo room')
  for (const [actor, name, role] of [[alice, 'Alice', 'owner'], [bob, 'Bob', 'member'], [eve, 'Eve', 'guest']]) {
    assert.equal(actor.name, name, 'Normal App bootstrap must not rename fixture actors')
    assert.equal(actor.kind, 'human')
    assert.equal(actor.role, role, 'Normal App bootstrap must not change fixture authority')
  }
  assert.equal(task.mission_id, mission.id)
  assert.equal(item.mission_id, mission.id)
  const tasks = s.tasks.filter((row) => row.mission_id === mission.id)
  const taskIds = new Set(tasks.map((row) => row.id))
  const runs = s.runs.filter((row) => taskIds.has(row.task_id))
  assert.equal(runs[0]?.id, run.id, 'Fixture run must be the latest mission run displayed by App')
  const reviews = s.verification_requests.filter((row) => row.run_id === run.id)
  assert.equal(reviews.length, 1, 'Require one persisted review for the exact run')
  assert.equal(reviews[0].task_id, task.id)
  assert.equal(reviews[0].gate_type, 'independent_review')
  assert.equal(reviews[0].gate.type, 'independent_review')
  const evidence = s.verification_evidence
    .filter((row) => row.run_id === run.id).sort((a, b) => a.check_index - b.check_index)
  assert.equal(task.verification_policy.checks.length, 11, 'Parent must supply eleven real checks')
  assert.equal(task.verification_policy.manual_gate?.type, 'independent_review')
  assert.equal(evidence.length, 11, 'Require eleven persisted evidence records')
  assert.deepEqual(evidence.map((row) => row.check_index), Array.from({ length: 11 }, (_, i) => i))
  assert.ok(evidence.every((row) => row.task_id === task.id && row.status === 'passed'))
  const runTotals = [
    ...s.events.filter((event) => event.type === 'run.verification_started'
      && event.aggregate_type === 'run' && event.aggregate_id === run.id)
      .map((event) => event.payload.check_count),
    ...s.factory_verification_recoveries.filter((recovery) =>
      recovery.replacement_run_id === run.id && recovery.task_id === task.id)
      .map((recovery) => recovery.replacement_verification_policy?.checks?.length),
  ]
  assert.ok(runTotals.length > 0 && runTotals.every((count) => count === 11),
    'This 11/11 acceptance requires an exact run-bound count receipt; missing/conflicting history must remain unknown in the UI')
  return { mission, task, item, run, review: reviews[0], alice, bob, evidence, tasks, runs, agents: s.agents }
}

function stableAuthority(view) {
  return digest(JSON.stringify({
    task_ids: view.tasks.map((row) => row.id).sort(),
    run_ids: view.runs.map((row) => row.id).sort(),
    tasks: view.tasks.map((row) => ({
      id: row.id, contract: row.contract, policy: row.verification_policy,
    })).sort((a, b) => a.id.localeCompare(b.id)),
    evidence: view.evidence,
    item_id: view.item.id, mission_id: view.mission.id,
    budgets: [view.mission.budget_tokens, view.mission.budget_cost_microusd],
    source: [view.run.source_base_commit, view.run.workspace_run_id, view.run.workspace_branch],
    actors: [view.alice, view.bob].map(({ id, name, kind, role }) => ({ id, name, kind, role })),
  }))
}

function publicView(view) {
  return {
    observed_at: new Date().toISOString(), run_id: view.run.id, task_id: view.task.id,
    mission_status: view.mission.status, task_status: view.task.status,
    run_status: view.run.status, verification_status: view.run.verification_status,
    factory_state: view.item.state, workspace_disposition: view.run.workspace_disposition,
    review: {
      status: view.review.status, gate_type: view.review.gate_type,
      decided_by: view.review.decided_by, decision_note: view.review.decision_note,
    },
    automated_passed: view.evidence.filter((row) => row.status === 'passed').length,
    automated_total: view.evidence.length, unchanged_authority_sha256: stableAuthority(view),
  }
}

function rejected(view) {
  return view.review.status === 'rejected'
    && view.review.decided_by === fixture.bob_actor_id
    && view.review.decision_note === expectedNote
    && view.run.status === 'failed' && view.run.verification_status === 'failed'
    && view.task.status === 'verification_failed' && view.mission.status === 'failed'
    && view.item.state === 'verification_failed' && view.run.workspace_disposition === 'preserved'
}

async function persistedRejection(actorId) {
  const view = await until('persisted rejected review and preserved failed workspace',
    async () => {
      const observed = await readView(actorId)
      report.last_persisted_fixture = publicView(observed)
      return observed
    }, rejected)
  assert.equal(stableAuthority(view), stableAuthority(baseline),
    'Review/draft browsing must not create runs or alter source, policy, evidence, or budgets')
  return view
}

async function screenshot(name) {
  const filename = `${name}.png`
  await page.screenshot({ path: path.join(output, filename), fullPage: true, timeout: 15_000 })
  report.screenshots.push(filename)
  return filename
}

async function openMission() {
  const bootstrapBeforeNavigation = report.bootstrap_validated
  if (page.url().startsWith(`${web}/`)) {
    // A hash-only goto preserves the unsaved composer from the previous viewport.
    // Use a real browser reload to test the documented fresh-load disclosure state.
    await page.reload({ waitUntil: 'domcontentloaded', timeout: 30_000 })
  }
  await page.goto(`${web}/#missions`, { waitUntil: 'domcontentloaded', timeout: 30_000 })
  await page.locator('.live-indicator.live-live').waitFor({ state: 'visible' })
  await until('verified normal App bootstrap', () => report.bootstrap_validated,
    (count) => count > bootstrapBeforeNavigation)
  const picker = page.locator('#operator-actor')
  await picker.waitFor({ state: 'visible' })
  assert.equal(await picker.inputValue(), fixture.alice_actor_id, 'Fresh App must select seeded Alice')
  const title = new RegExp(`^${baseline.mission.title.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`)
  const button = page.locator('#missions .mission-selector button').filter({
    has: page.locator('strong', { hasText: title }),
  })
  assert.equal(await button.count(), 1, 'Require an unambiguous real mission selector')
  assert.match(await button.innerText(), new RegExp(fixture.mission_id.slice(0, 8)))
  await button.click()
  const card = page.getByTestId(`mission-${fixture.mission_id}`)
  await card.waitFor({ state: 'visible' })
  assert.equal(await card.getAttribute('data-run-id'), fixture.run_id)
  return card
}

async function chooseActor(actorId) {
  await page.locator('#operator-actor').selectOption(actorId)
  await until('actor selection', () => page.locator('#operator-actor').inputValue(),
    (value) => value === actorId)
  await page.locator('.live-indicator.live-live').waitFor({ state: 'visible' })
}

// Read-only DOM/computed-style measurements. No assignments to page/app state.
async function appearance(locator) {
  return locator.evaluate((element) => {
    const rgba = (value) => {
      const match = /^rgba?\(([^)]+)\)$/.exec(value)
      if (!match) throw new Error(`Unsupported computed color format: ${value}`)
      const parts = match[1].split(/[, /]+/).filter(Boolean).map(Number)
      return [...parts.slice(0, 3), parts.length > 3 ? parts[3] : 1]
    }
    const blend = (top, bottom) => top.slice(0, 3).map((v, i) => v * top[3] + bottom[i] * (1 - top[3]))
    const luminance = (rgb) => rgb.map((v) => {
      const n = v / 255
      return n <= 0.04045 ? n / 12.92 : ((n + 0.055) / 1.055) ** 2.4
    }).reduce((sum, v, i) => sum + v * [0.2126, 0.7152, 0.0722][i], 0)
    const ancestors = []
    for (let current = element; current; current = current.parentElement) ancestors.unshift(current)
    let background = [255, 255, 255]
    for (const ancestor of ancestors) {
      background = blend(rgba(getComputedStyle(ancestor).backgroundColor), background)
    }
    const style = getComputedStyle(element)
    const foreground = blend(rgba(style.color), background)
    const light = luminance(background), ink = luminance(foreground)
    const rect = element.getBoundingClientRect()
    return {
      text: element.textContent.trim(), color: style.color, background: style.backgroundColor,
      effective_background: background, background_luminance: light,
      contrast: (Math.max(light, ink) + 0.05) / (Math.min(light, ink) + 0.05),
      border: style.borderColor, box_shadow: style.boxShadow,
      outline_style: style.outlineStyle, outline_width: parseFloat(style.outlineWidth),
      focused: document.activeElement === element, width: rect.width, height: rect.height,
    }
  })
}

async function noOverflow(width) {
  const measured = await page.evaluate(() => {
    const selectors = ['#missions', '.mission-list', '.operations-verification',
      '.operations-review-decision', '.operations-identity', '.verification-policy-editor']
    return {
      viewport: innerWidth, document: document.documentElement.scrollWidth,
      body: document.body.scrollWidth,
      regions: selectors.flatMap((selector) => [...document.querySelectorAll(selector)]
        .filter((el) => el.getClientRects().length > 0)
        .map((el) => ({ selector, client: el.clientWidth, scroll: el.scrollWidth }))),
    }
  })
  assert.equal(measured.viewport, width)
  assert.ok(measured.document <= width + 1 && measured.body <= width + 1, 'Horizontal page overflow')
  assert.ok(measured.regions.every((row) => row.scroll <= row.client + 1), 'Horizontal content overflow')
  return measured
}

async function identityHelp() {
  const picker = page.locator('#operator-actor')
  const help = page.locator('#operator-identity-help')
  assert.equal(await picker.isEnabled(), true, 'This is a seeded development identity selector')
  assert.ok((await picker.getAttribute('aria-describedby'))?.split(/\s+/).includes('operator-identity-help'))
  assert.equal(await page.locator('label[for="operator-actor"]').count(), 1)
  assert.equal(await help.isVisible(), true)
  assert.match(await help.innerText(), /Alice, Bob and Eve.*seeded local demo users/)
  assert.match(await help.innerText(), /permissions, not your GitHub sign-in/)
  await page.keyboard.press('Tab') // Establish real keyboard modality for :focus-visible.
  await picker.focus()
  const focus = await appearance(picker)
  assert.ok(focus.focused && focus.outline_style !== 'none' && focus.outline_width >= 2)
  assert.ok(focus.height >= 44, 'Identity selector is below the touch-target floor')
  const copy = await appearance(help)
  assert.ok(copy.contrast >= 4.5, 'Identity help contrast is below 4.5:1')
  const row = await appearance(page.locator('.operations-identity'))
  const consoleRow = await appearance(page.locator('.operator-console'))
  if (page.viewportSize().width <= 720) {
    const availableWidth = await page.locator('.operator-console').evaluate((el) => {
      const style = getComputedStyle(el)
      return el.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight)
    })
    assert.ok(Math.abs(row.width - availableWidth) <= 2, 'Mobile identity/help must span the full operator row')
    const columns = await page.locator('.operations-identity').evaluate((el) => {
      const style = getComputedStyle(el)
      return [style.gridColumnStart, style.gridColumnEnd]
    })
    assert.deepEqual(columns, ['1', '-1'], 'The wrapper, not just its nested label, must span both columns')
  }
  return { focus, help: copy, row, operator_console: consoleRow }
}

// Parse the copied PowerShell single-quote grammar; never execute the command.
function commandWords(command) {
  assert.ok(command.length <= 8192 && !/[\r\n]/.test(command), 'Unexpected command size/line break')
  const words = []
  let i = 0
  while (i < command.length) {
    if (/\s/.test(command[i])) { i++; continue }
    let word = ''
    if (command[i] === "'") {
      i++
      let closed = false
      while (i < command.length) {
        if (command[i] === "'") {
          if (command[i + 1] === "'") { word += "'"; i += 2; continue }
          i++; closed = true; break
        }
        word += command[i++]
      }
      assert.ok(closed && (i === command.length || /\s/.test(command[i])), 'Invalid quoted argument')
    } else {
      while (i < command.length && !/\s/.test(command[i])) word += command[i++]
      assert.match(word, /^[a-zA-Z0-9-]+$/, 'Unexpected unquoted command token')
    }
    words.push(word)
  }
  return words
}

function expectedCommand(view, mode) {
  const { task, item, mission } = view
  const agent = view.agents.find((row) => row.id === task.assigned_agent_id)
  const adapter = task.required_adapter ?? agent?.adapter
  assert.ok(adapter && item.source_project_owner && item.source_repository_owner && item.source_repository_name)
  assert.equal(view.tasks.find((row) => row.status === 'verification_failed')?.id, task.id,
    'The displayed recovery controls must select this fixture task, not another failed task')
  for (const value of [item.source_project_number, item.source_issue_number]) {
    assert.ok(Number.isSafeInteger(value) && value > 0, 'Complete persisted recovery authority is required')
  }
  for (const value of [mission.budget_tokens, mission.budget_cost_microusd]) {
    assert.ok(Number.isSafeInteger(value) && value >= 0, 'Preserve exact persisted budget values, including zero')
  }
  const words = [
    'crony', 'factory', fixture.corp_id, fixture.alice_actor_id,
    '--owner', item.source_project_owner, '--project-number', String(item.source_project_number),
    '--repository', `${item.source_repository_owner}/${item.source_repository_name}`,
    '--source-base-ref', task.contract.source_base_ref ?? 'HEAD', '--adapter', adapter,
    '--budget-tokens', String(mission.budget_tokens),
    '--budget-cost-microusd', String(mission.budget_cost_microusd),
    '--issue', String(item.source_issue_number), '--verification-recovery', mode,
    '--verification-recovery-reason', 'Explain why this bounded recovery is authorized.',
  ]
  if (task.contract.model) words.push('--model', task.contract.model)
  if (task.contract.reasoning_effort) words.push('--reasoning-effort', task.contract.reasoning_effort)
  return words
}

async function copiedCommands(card, view) {
  const recovery = card.getByTestId('factory-verification-recovery')
  await recovery.waitFor({ state: 'visible' })
  await until('loaded exact recovery context',
    () => recovery.getAttribute('data-recovery-context-status'), (value) => value === 'ready')
  const context = await until('observed App recovery-context GET',
    () => report.recovery_contexts.findLast((row) => row.actor_id === fixture.alice_actor_id
      && row.work_item_id === fixture.factory_work_item_id && row.version === view.item.version),
    Boolean)
  assert.equal(await recovery.getAttribute('data-recovery-run-id'), context.source_run_id,
    'Recovery controls must use the source selected by the exact endpoint')
  assert.equal(await recovery.getAttribute('data-recovery-task-id'), context.task_id)
  assert.equal(await recovery.getAttribute('data-recovery-item-version'), String(context.version))
  assert.equal(context.source_run_id, fixture.run_id, 'This acceptance fixture must remain the selected source')
  assert.equal(await recovery.getAttribute('data-recovery-state'), context.has_checkpoint ? 'ready' : 'checkpoint_required')
  assert.match(normalizedText(await recovery.innerText()), /Copying is not granting and executes nothing/)
  assert.equal(await card.getByRole('button', { name: 'Resume agent session', exact: true }).count(), 0,
    'Factory verification failure must use governed recovery, not generic ancestor resume')
  const metadata = recovery.locator('.factory-recovery-details')
  assert.equal(await metadata.evaluate((element) => element.open), false, 'Technical recovery details must default closed')
  await metadata.locator('summary').focus()
  await page.keyboard.press('Enter')
  assert.equal(await metadata.evaluate((element) => element.open), true)
  assert.equal(await metadata.locator(`[title="${context.source_run_id}"]`).isVisible(), true)
  await metadata.locator('summary').focus()
  await page.keyboard.press('Space')
  assert.equal(await metadata.evaluate((element) => element.open), false)
  const results = []
  for (const [mode, label, feedback] of [
    ['verifier-only', 'Copy verifier-only command', 'Verifier command copied'],
    ['source-correction', 'Copy source-correction command', 'Correction command copied'],
  ]) {
    const button = recovery.getByRole('button', { name: label, exact: true })
    assert.equal(await button.isEnabled(), true)
    const target = await appearance(button)
    assert.ok(target.height >= 44 && target.width >= 44, 'Recovery copy touch target is too small')
    await button.click()
    await recovery.getByRole('button', { name: feedback, exact: true }).waitFor({ state: 'visible' })
    // Reads the real clipboard only after the App reports a successful UI copy.
    const command = await page.evaluate(() => navigator.clipboard.readText())
    assert.deepEqual(commandWords(command), expectedCommand(view, mode), 'Incomplete or incorrectly scoped recovery CLI')
    results.push({ mode, command, sha256: digest(command), executed: false, reason_is_operator_template: true, recovery_details_keyboard_operable: true })
  }
  return results
}

async function verifierTabs(name, width) {
  report.stage = `${name}: unsaved verifier editor`
  const missions = page.locator('#missions')
  const newMission = missions.getByRole('button', { name: 'New mission', exact: true })
  if (await newMission.isVisible()) await newMission.click()
  const stages = missions.locator('.mission-stage-nav')
  await stages.getByRole('button', { name: 'Run setup', exact: true }).click()
  const strategy = missions.getByLabel('Execution strategy', { exact: true })
  const originalStrategy = await strategy.inputValue()
  const approvalExpectations = []
  for (const [value, expected] of [
    ['single', /Solo run can still need decisions for risky actions/],
    ['parallel-specialists', /Two specialists and synthesis can request different scoped actions/],
  ]) {
    await strategy.selectOption(value)
    const text = normalizedText(await missions.locator('#mission-strategy-policy').innerText())
    assert.match(text, expected)
    assert.match(text, /not duplicate grants for the same action/)
    approvalExpectations.push({ strategy: value, text })
  }
  await strategy.selectOption(originalStrategy)
  await stages.getByRole('button', { name: 'Verification', exact: true }).click()
  const custom = missions.getByRole('checkbox', { name: /Custom verification/ })
  assert.equal(await custom.isEnabled(), true, 'Use the normal composer, not a deterministic cartridge')
  await custom.check()
  const editor = page.getByTestId('mission-verification-editor')
  await editor.waitFor({ state: 'visible' })
  const tabs = editor.locator('.verification-check-tabs button')
  if (await tabs.count() < 2) await editor.getByRole('button', { name: 'Add check', exact: true }).click()
  assert.ok(await tabs.count() >= 2)
  const measurements = []
  for (const index of [1, 0]) {
    await tabs.nth(index).focus()
    await page.keyboard.press('Enter')
    const input = editor.getByLabel(`Verifier check ${index + 1} type`, { exact: true })
    await input.waitFor({ state: 'visible' })
    await input.focus()
    assert.equal(await tabs.nth(index).getAttribute('aria-pressed'), 'true')
    assert.match(await tabs.nth(index).getAttribute('class'), /check-tab-active/)
    assert.equal(await tabs.nth(1 - index).getAttribute('aria-pressed'), 'false')
    const active = await appearance(tabs.nth(index))
    const inactive = await appearance(tabs.nth(1 - index))
    const editorFocus = await appearance(input)
    assert.ok(editorFocus.focused && editorFocus.outline_style !== 'none' && editorFocus.outline_width >= 2)
    assert.ok(active.contrast >= 4.5 && inactive.contrast >= 4.5, 'Verifier-tab text contrast is below 4.5:1')
    assert.ok(active.background !== inactive.background || active.border !== inactive.border
      || active.box_shadow !== inactive.box_shadow, 'Selected tab loses visual distinction after focus moves')
    measurements.push({ selected_index: index, active, inactive, editor_focus: editorFocus })
  }
  const layout = await noOverflow(width)
  const image = await screenshot(`${name}-verifier-tabs-unsaved`)
  return { measurements, approval_expectations: approvalExpectations, layout, screenshot: image, submitted: false }
}

async function viewportCase(name, width, height) {
  report.stage = `${name}: rejected-review acceptance`
  await page.setViewportSize({ width, height })
  const card = await openMission() // A real reload also proves default disclosure state/persistence.
  const view = await persistedRejection(fixture.alice_actor_id)
  const missions = page.locator('#missions')
  const toolbar = missions.locator(':scope > .operations-mission-toolbar')
  assert.equal(await toolbar.count(), 1, 'Mission actions must share one compact toolbar')
  assert.equal(await toolbar.getByRole('heading', { name: 'Mission queue', exact: true }).count(), 1)
  assert.equal(await missions.locator('.arcade-new-mission-bar').count(), 0, 'Do not restore a second mission header')
  const newMission = toolbar.getByRole('button', { name: 'New mission', exact: true })
  assert.equal(await newMission.isEnabled(), true)
  const toolbarStyle = await appearance(toolbar)
  const newMissionStyle = await appearance(newMission)
  assert.ok(toolbarStyle.height <= (width <= 480 ? 180 : 112), 'Mission toolbar is too tall')
  assert.ok(newMissionStyle.height >= 44 && newMissionStyle.width >= 44, 'New mission touch target is too small')
  const review = card.getByTestId('review-decision')
  await review.waitFor({ state: 'visible' })
  const automated = card.getByTestId('verification-evidence')
  assert.equal(await automated.getAttribute('data-check-total'), '11',
    'The visible denominator must come from the exact run-bound receipt validated in preflight')
  assert.equal(await automated.evaluate((el) => el.open), false, 'Automated evidence must default closed after rejection')
  assert.match(await automated.locator('summary').innerText(), /Automated verification/)
  assert.equal(normalizedText(await automated.locator('.operations-verification-score').innerText()), '11/11 passed')
  assert.match(await automated.locator('summary').innerText(), /All recorded checks passed/)
  assert.equal(await automated.locator('.evidence-check').first().isVisible(), false)
  assert.equal(await review.getAttribute('role'), 'alert')
  const reviewText = normalizedText(await review.innerText())
  assert.ok(reviewText.includes('Independent review') && reviewText.includes('Changes requested'))
  assert.ok(reviewText.includes(`Reviewed by ${baseline.bob.name}`) && reviewText.includes(expectedNote))
  assert.match(reviewText, /Next step:.*governed recovery controls/)
  const styles = {
    automated: await appearance(automated.locator('summary strong')),
    score: await appearance(automated.locator('.operations-verification-score')),
    review: await appearance(review.locator(':scope > strong')),
    reason: await appearance(review.locator(':scope > p').first()),
  }
  assert.ok(styles.automated.background_luminance >= 0.6 && styles.review.background_luminance >= 0.6,
    'Verification/review surfaces must remain light')
  assert.ok(Object.values(styles).every((value) => value.contrast >= 4.5), 'Text contrast below 4.5:1')
  const identity = await identityHelp()
  const summary = automated.locator('summary')
  await page.keyboard.press('Tab')
  await summary.focus()
  const focus = await appearance(summary)
  assert.ok(focus.focused && focus.outline_style !== 'none' && focus.outline_width >= 2)
  assert.ok(focus.height >= 44)
  const collapsedImage = await screenshot(`${name}-rejected-collapsed`)
  await page.keyboard.press('Enter')
  await until('keyboard evidence expansion', () => automated.evaluate((el) => el.open), Boolean)
  const rows = automated.locator('.evidence-check')
  assert.equal(await rows.count(), 11)
  const rowStyles = []
  for (let index = 0; index < 11; index++) {
    assert.equal(await rows.nth(index).isVisible(), true)
    assert.equal(normalizedText(await rows.nth(index).locator(':scope > span').innerText()).toLowerCase(), 'passed')
    const measured = await appearance(rows.nth(index).locator(':scope > strong'))
    assert.ok(measured.background_luminance >= 0.6 && measured.contrast >= 4.5, 'Expanded check is not a readable light row')
    rowStyles.push(measured)
  }
  const layout = await noOverflow(width)
  const expandedImage = await screenshot(`${name}-rejected-expanded`)
  await summary.focus()
  await page.keyboard.press('Space')
  await until('keyboard evidence collapse', () => automated.evaluate((el) => el.open), (open) => !open)
  const commands = await copiedCommands(card, view)
  const metadata = card.locator('.operations-run-metadata')
  assert.equal(await metadata.isVisible(), true, 'Compact run metadata is missing')
  const editor = await verifierTabs(name, width)
  cleanBrowser()
  return {
    name, width, height, passed: true, persisted: publicView(view), styles, row_styles: rowStyles,
    mission_toolbar: { single_header: true, toolbar: toolbarStyle, new_mission: newMissionStyle },
    identity, keyboard_focus: focus, default_collapsed: true, keyboard_expand_collapse: true,
    review_reason_source: 'Actual App rejection handler; no editable reason input exists',
    long_reason_disclosure: 'Not exercised: the actual UI-generated reason is short; no injection used',
    layout, copied_commands: commands, verifier_editor: editor,
    screenshots: [collapsedImage, expandedImage],
  }
}

async function main() {
  const alreadyRejected = process.env.ECORP_UI_RECOVERY_ALREADY_REJECTED === '1'
  assert.equal(process.env.CRONY_RECOVERY_TEST, '1', 'Explicit CRONY_RECOVERY_TEST=1 is required')
  assert.equal(process.argv.length, 2, 'Configure through the documented environment variables only')
  server = ownedOrigin(process.env.CRONY_SERVER_HTTP, ['8791', '8991', '18941', '18962'], 'CRONY_SERVER_HTTP')
  web = ownedOrigin(process.env.CRONY_RECOVERY_WEB, ['5291', '15481', '15491'], 'CRONY_RECOVERY_WEB')
  const fixturePath = process.env.ECORP_UI_RECOVERY_FIXTURE
  const outputRoot = process.env.CRONY_RECOVERY_OUTPUT
  assert.ok(fixturePath && path.isAbsolute(fixturePath), 'Absolute ECORP_UI_RECOVERY_FIXTURE is required')
  assert.ok(outputRoot && path.isAbsolute(outputRoot), 'Absolute CRONY_RECOVERY_OUTPUT is required')
  const metadata = await stat(fixturePath)
  assert.ok(metadata.isFile() && metadata.size <= 64 * 1024, 'Fixture must be a small existing JSON file')
  const supplied = JSON.parse(await readFile(fixturePath, 'utf8'))
  fixture = Object.fromEntries(fixtureKeys.map((key) => {
    assert.ok(typeof supplied[key] === 'string' && uuid.test(supplied[key]), `Invalid fixture ${key}`)
    return [key, supplied[key].toLowerCase()]
  }))
  for (const key of ['corp_id', 'alice_actor_id', 'bob_actor_id']) {
    assert.equal(fixture[key], seededDemo[key],
      `The normal App uses seeded ${key}; do not bootstrap a different fixture`)
  }
  assert.notEqual(fixture.alice_actor_id, fixture.bob_actor_id)
  await mkdir(outputRoot, { recursive: true })
  output = path.join(outputRoot, `operations-browser-${new Date().toISOString().replace(/[:.]/g, '-')}-${randomUUID().slice(0, 8)}`)
  await mkdir(output) // Unique directory; preserve every prior receipt/screenshot.
  report = {
    schema_version: 1, passed: false, started_at: new Date().toISOString(), server, web, fixture,
    stage: 'read-only preflight', browser_executed: false, browser_closed: false,
    scope: 'Existing fixture only; one Bob rejection through App. No direct API POST, fixture creation, CLI execution, or services.',
    network_guard: 'Forward/abort only; normal pre-seeded App bootstrap and one exact UI rejection',
    emulated_reduced_motion: 'reduce',
    bootstrap_forwarded: 0, bootstrap_validated: 0, rejection_forwarded: 0,
    cases: [], screenshots: [], errors: [], warnings: [], blocked_requests: [],
    recovery_contexts: [], recovery_context_aborts: [],
  }
  try {
    const health = await getJson('/health')
    assert.equal(health.status, 'ok')
    assert.equal(health.mode, 'development', 'Seeded demo-identity acceptance must not target production')
    baseline = await readView(fixture.alice_actor_id)
    const bobView = await readView(fixture.bob_actor_id)
    assert.equal(stableAuthority(bobView), stableAuthority(baseline))
    assert.equal(baseline.review.status, alreadyRejected ? 'rejected' : 'pending')
    assert.equal(baseline.run.status, alreadyRejected ? 'failed' : 'waiting_for_approval')
    assert.equal(baseline.run.verification_status, alreadyRejected ? 'failed' : 'waiting_for_approval')
    assert.equal(baseline.item.state, alreadyRejected ? 'verification_failed' : 'awaiting_approval')
    if (alreadyRejected) {
      assert.equal(baseline.review.decided_by, fixture.bob_actor_id)
      report.scope = 'Read-only continuation of the existing Bob-rejected fixture; no new decision or fixture.'
      report.continued_existing_rejection = true
    }
    assert.ok(['owner', 'admin', 'manager'].includes(baseline.alice.role), 'Alice must be eligible to copy governed recovery commands')
    assert.ok(baseline.review.gate.roles.includes(baseline.bob.role), 'Bob must be an eligible independent reviewer')
    assert.ok(!baseline.review.gate.exclude_requester || baseline.mission.requested_by !== fixture.bob_actor_id)
    expectedNote = `${baseline.bob.name} rejected the recorded verification evidence.`
    report.before = publicView(baseline)
    const require = createRequire(import.meta.url)
    const override = process.env.CRONY_PLAYWRIGHT_MODULE
    const module = override
      ? await import(pathToFileURL(require.resolve(override.startsWith('file:') ? fileURLToPath(override) : override)).href)
      : await import('playwright')
    const { chromium } = module.default ?? module
    browser = await chromium.launch({ channel: 'chrome', headless: true })
    report.browser_executed = true
    report.browser_version = browser.version()
    report.browser_channel = 'chrome'
    const context = await browser.newContext({
      viewport: { width: 1440, height: 900 }, reducedMotion: 'reduce', serviceWorkers: 'block',
    })
    await context.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: web })
    const decisionPath = `/api/corps/${fixture.corp_id}/runs/${fixture.run_id}/verification-decision`
    await context.route('**/*', async (route) => {
      const request = route.request()
      const url = new URL(request.url())
      const owned = [server, web].includes(url.origin) && !url.username && !url.password
      if (owned && ['GET', 'HEAD', 'OPTIONS'].includes(request.method())) return route.continue()
      if (url.origin === server && url.pathname === '/api/demo/bootstrap'
        && url.search === '?seed_crew=false' && request.method() === 'POST'
        && request.postData() === '{}' && report.bootstrap_forwarded < 12) {
        report.bootstrap_forwarded++
        return route.continue() // The actual App handshake; fixture/actors already exist.
      }
      if (url.origin === server && url.pathname === decisionPath && !url.search
        && request.method() === 'POST' && rejectionArmed && report.rejection_forwarded === 0) {
        let body
        try { body = request.postDataJSON() } catch { /* Rejected below without recording payload. */ }
        if (body?.actor_id === fixture.bob_actor_id && body.approved === false && body.note === expectedNote
          && Object.keys(body).sort().join(',') === 'actor_id,approved,note') {
          rejectionArmed = false
          report.rejection_forwarded++
          return route.continue()
        }
      }
      report.blocked_requests.push({ method: request.method(), url: safeUrl(request.url()) })
      return route.abort('blockedbyclient')
    })
    assert.equal(typeof context.routeWebSocket, 'function', 'Installed Playwright must support guarded WebSockets')
    await context.routeWebSocket('**/*', (route) => {
      const url = new URL(route.url())
      const httpOrigin = url.origin.replace(/^ws:/, 'http:')
      const actor = url.searchParams.get('actor_id')
      const appSocket = httpOrigin === server && url.pathname === `/ws/corps/${fixture.corp_id}`
        && [fixture.alice_actor_id, fixture.bob_actor_id].includes(actor)
      const viteSocket = httpOrigin === web && url.pathname === '/'
      if (!url.username && !url.password && (appSocket || viteSocket)) {
        route.connectToServer()
      } else {
        report.blocked_requests.push({ method: 'WEBSOCKET', url: safeUrl(route.url()) })
        route.close({ code: 1008, reason: 'Only owned QA sockets are permitted' })
      }
    })
    page = await context.newPage()
    page.setDefaultTimeout(20_000)
    page.on('pageerror', (error) => report.errors.push(`page: ${safeError(error)}`))
    page.on('console', (message) => {
      if (message.type() === 'error') report.errors.push(`console: ${safeError(message.text())}`)
      if (message.type() === 'warning' && report.warnings.length < 30) report.warnings.push(safeError(message.text()))
    })
    page.on('requestfailed', (request) => {
      const contextPath = `/api/corps/${fixture.corp_id}/factory/work-items/${fixture.factory_work_item_id}/verification-recoveries`
      if (request.method() === 'GET' && new URL(request.url()).pathname === contextPath
        && request.failure()?.errorText === 'net::ERR_ABORTED') {
        report.recovery_context_aborts.push({ url: safeUrl(request.url()), reason: 'Scope change/navigation cancelled this read-only lookup' })
        return
      }
      if (!closing) report.errors.push(`request: ${safeUrl(request.url())}: ${safeError(request.failure()?.errorText)}`)
    })
    page.on('response', (response) => {
      if (response.status() >= 400) report.errors.push(`HTTP ${response.status()}: ${safeUrl(response.url())}`)
      if (response.request().method() === 'POST' && new URL(response.url()).pathname === '/api/demo/bootstrap') {
        const task = (async () => {
          const body = await response.json()
          for (const key of ['corp_id', 'alice_actor_id', 'bob_actor_id']) assert.equal(body[key], fixture[key], `App bootstrap ${key} differs from pre-seeded fixture`)
          report.bootstrap_validated++
        })().catch((error) => report.errors.push(`bootstrap identity: ${safeError(error)}`))
        responseTasks.add(task)
        void task.finally(() => responseTasks.delete(task))
      }
      const responseUrl = new URL(response.url())
      if (response.request().method() === 'GET' && responseUrl.origin === server &&
        responseUrl.pathname === `/api/corps/${fixture.corp_id}/factory/work-items/${fixture.factory_work_item_id}/verification-recoveries`) {
        const task = (async () => {
          if (!response.ok()) return // Already recorded as an HTTP error above.
          const body = await response.json()
          const actorId = responseUrl.searchParams.get('actor_id')
          assert.ok([fixture.alice_actor_id, fixture.bob_actor_id].includes(actorId))
          assert.equal(body.work_item.id, fixture.factory_work_item_id)
          assert.equal(body.work_item.corp_id, fixture.corp_id)
          assert.equal(body.mission_id, fixture.mission_id)
          assert.equal(typeof body.task_id, 'string')
          assert.equal(typeof body.source_run_id, 'string')
          // Only source IDs/version and checkpoint presence enter evidence; no private policy or credentials.
          report.recovery_contexts.push({
            actor_id: actorId, work_item_id: body.work_item.id, version: body.work_item.version,
            task_id: body.task_id, source_run_id: body.source_run_id, has_checkpoint: Boolean(body.workspace_fingerprint),
          })
        })().catch((error) => {
          if (!closing && response.request().failure()?.errorText !== 'net::ERR_ABORTED') {
            report.errors.push(`recovery context: ${safeError(error)}`)
          }
        })
        responseTasks.add(task)
        void task.finally(() => responseTasks.delete(task))
      }
    })
    page.on('dialog', (dialog) => {
      report.errors.push(`Unexpected ${dialog.type()} dialog: current review handler has no reason prompt`)
      void dialog.dismiss().catch(() => {})
    })
    if (!alreadyRejected) {
      report.stage = 'Bob UI rejection'
      let card = await openMission()
      await chooseActor(fixture.bob_actor_id)
      card = page.getByTestId(`mission-${fixture.mission_id}`)
      await card.waitFor({ state: 'visible' })
      assert.equal(await card.getAttribute('data-run-id'), fixture.run_id)
      const rejectButton = card.getByRole('button', { name: 'Reject evidence', exact: true })
      await until('eligible Bob review button', () => rejectButton.isEnabled(), Boolean)
      await screenshot('desktop-before-rejection')
      rejectionArmed = true
      const [decisionResponse] = await Promise.all([
        page.waitForResponse((response) => response.request().method() === 'POST'
          && response.url() === `${server}${decisionPath}`, { timeout: 30_000 }),
        rejectButton.click(),
      ])
      rejectionArmed = false
      assert.ok(decisionResponse.ok(), 'The actual UI rejection must succeed')
      report.rejection_http_status = decisionResponse.status()
      report.after_ui_rejection = publicView(await persistedRejection(fixture.bob_actor_id))
    } else {
      report.after_ui_rejection = publicView(await persistedRejection(fixture.bob_actor_id))
    }
    for (const dimensions of [['desktop', 1440, 900], ['mobile-390', 390, 844]]) {
      report.cases.push(await viewportCase(...dimensions))
    }
    report.after_reload_and_unsaved_editor = publicView(await persistedRejection(fixture.alice_actor_id))
    await Promise.all([...responseTasks])
    assert.equal(report.rejection_forwarded, alreadyRejected ? 0 : 1)
    assert.equal(report.bootstrap_validated, report.bootstrap_forwarded)
    assert.equal(report.cases.length, 2)
    cleanBrowser()
    report.passed = true
  } catch (error) {
    report.failure = safeError(error)
    if (page && !page.isClosed()) {
      try { await screenshot('failure') } catch (captureError) { report.errors.push(`capture: ${safeError(captureError)}`) }
    }
  } finally {
    rejectionArmed = false
    closing = true
    try {
      if (browser) { await browser.close(); report.browser_closed = true }
    } catch (error) { report.errors.push(`browser cleanup: ${safeError(error)}`) }
    await Promise.all([...responseTasks])
    if (report.errors.length || report.blocked_requests.length || !report.browser_closed) report.passed = false
    report.finished_at = new Date().toISOString()
    const reportPath = path.join(output, 'report.json')
    await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`, { encoding: 'utf8', flag: 'wx' })
    console.log(JSON.stringify({ passed: report.passed, report: reportPath, failure: report.failure ?? null }, null, 2))
    if (!report.passed) process.exitCode = 1
  }
}

async function pureTests() {
  const { test } = await import('node:test')
  const { runInNewContext } = await import('node:vm')
  const appPath = new URL('../apps/web/src/App.tsx', import.meta.url)
  const appSource = await readFile(appPath, 'utf8')
  const css = await readFile(new URL('../apps/web/src/OperationsUx.css', import.meta.url), 'utf8')
  const require = createRequire(new URL('../apps/web/package.json', import.meta.url))
  const ts = require('typescript')
  const parsed = ts.createSourceFile('App.tsx', appSource, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
  const names = [
    'automatedVerificationPresentation', 'factoryRecoveryScopeKey', 'currentFactoryRecoveryLoad',
    'requestFactoryRecoveryContext', 'factoryRecoveryPresentation',
  ]
  const declarations = names.map((name) => {
    const declaration = parsed.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === name)
    assert.ok(declaration, `Missing actual App helper: ${name}`)
    return declaration.getText(parsed)
  })
  // Compile the actual helpers in memory, never mount App or initialize Playwright.
  // Async tests supply local deferred API/timer stubs; no fetch/service/fixture access.
  const compiled = ts.transpileModule(declarations.join('\n'), {
    compilerOptions: {
      target: ts.ScriptTarget.ES2023, module: ts.ModuleKind.ESNext,
      moduleResolution: ts.ModuleResolutionKind.Bundler,
    },
    reportDiagnostics: true,
  })
  assert.deepEqual(compiled.diagnostics?.filter((item) => item.category === ts.DiagnosticCategory.Error)
    .map((item) => ts.flattenDiagnosticMessageText(item.messageText, '\n')), [])
  let apiHandler = () => Promise.reject(new Error('No network is allowed in pure tests'))
  let timerId = 0
  const timers = new Map()
  const helpers = runInNewContext(`${compiled.outputText}\n({${names.join(',')}})`, {
    api: (url, init) => apiHandler(url, init),
    AbortController, Error,
    setTimeout: (callback) => { const id = ++timerId; timers.set(id, callback); return id },
    clearTimeout: (id) => timers.delete(id),
  }, { timeout: 1000 })
  const automated = (...args) => JSON.parse(JSON.stringify(helpers.automatedVerificationPresentation(...args)))
  const recovery = (...args) => JSON.parse(JSON.stringify(helpers.factoryRecoveryPresentation(...args)))
  const policy = (count) => ({ checks: Array.from({ length: count }, () => ({ type: 'file', path: 'proof.txt', min_bytes: 1 })), manual_gate: null })
  const run = {
    id: 'run-current', task_id: 'task-a', workspace_run_id: 'workspace-a',
    status: 'failed', verification_status: 'failed', breaker_stage: null,
    workspace_disposition: 'preserved', workspace_fingerprint: 'a'.repeat(64),
  }
  const rows = (count, status = 'passed') => Array.from({ length: count }, (_, index) => ({
    id: `evidence-${index}`, run_id: run.id, task_id: run.task_id, check_index: index, status,
  }))
  const event = (count, runId = run.id) => ({
    type: 'run.verification_started', aggregate_type: 'run', aggregate_id: runId, payload: { check_count: count },
  })
  const mission = { id: 'mission-a', status: 'failed' }
  const task = { id: run.task_id, mission_id: mission.id, status: 'verification_failed', max_attempts: 4, attempt_count: 1, verification_policy: policy(12) }
  const item = { id: 'factory-a', corp_id: 'corp-a', version: 7, mission_id: mission.id, state: 'verification_failed' }
  const history = {
    id: 'recovery-a', factory_work_item_id: item.id, mission_id: mission.id, task_id: task.id,
    source_run_id: 'run-ancestor', replacement_run_id: run.id, status: 'failed',
    previous_verification_policy: policy(9), replacement_verification_policy: policy(11),
  }
  const context = {
    work_item: item, recoveries: [], mission_id: mission.id, task_id: task.id,
    source_run_id: run.id, remaining_attempts: 3, remaining_mission_tokens: 500,
    remaining_mission_cost_microusd: 1000, workspace_fingerprint: 'b'.repeat(64),
    expected_head_commit: null,
  }
  const scope = { corpId: item.corp_id, actorId: 'alice', missionId: mission.id, itemId: item.id, version: item.version, reload: 0 }
  const clone = (value) => JSON.parse(JSON.stringify(value))
  const flush = () => new Promise((resolve) => setImmediate(resolve))
  const deferred = () => {
    let resolve, reject
    const promise = new Promise((yes, no) => { resolve = yes; reject = no })
    return { promise, resolve, reject }
  }
  const start = (selectedScope, loads) => helpers.requestFactoryRecoveryContext(selectedScope,
    (load) => loads.push(clone(load)))

  await test('historical 11/11 remains independent of a revised 12-check task and rejected review', () => {
    const result = automated(run, rows(11), [event(11)], [])
    assert.equal(task.verification_policy.checks.length, 12)
    assert.deepEqual([result.total, result.passed, result.status, result.score], [11, 11, 'passed', '11/11 passed'])
    assert.doesNotMatch(declarations[0], /taskById|task\.verification_policy|previous_verification_policy/)
    assert.match(appSource, /automatedVerificationPresentation\(latestRun, evidence, events, factoryRecoveries\)/)
  })
  await test('exact replacement-run policy supplies a total after its journal receipt is absent', () => {
    const result = automated(run, rows(11), [], [history])
    assert.equal(result.total, 11)
    assert.equal(result.status, 'passed')
  })
  await test('source-run or later revision policy is not replacement-run authority', () => {
    const unrelated = { ...history, source_run_id: run.id, replacement_run_id: 'later-run', replacement_verification_policy: policy(12) }
    const result = automated(run, rows(11), [], [unrelated])
    assert.equal(result.total, null)
    assert.equal(result.status, 'pending')
  })
  await test('eleven passing records alone never establish a verified total', () => {
    const result = automated(run, rows(11), [], [])
    assert.equal(result.total, null)
    assert.equal(result.status, 'pending')
    assert.equal(result.score, '11 passed · total unknown')
  })
  await test('partial and absent evidence keep the exact total without claiming completeness', () => {
    for (const count of [0, 3]) {
      const result = automated(run, rows(count), [event(11)], [])
      assert.deepEqual([result.total, result.passed, result.missing, result.status], [11, count, 11 - count, 'pending'])
    }
  })
  await test('failed automated checks stay failed independently of the run-level decision', () => {
    const evidence = rows(11)
    evidence[10].status = 'failed'
    const result = automated(run, evidence, [event(11)], [])
    assert.deepEqual([result.passed, result.failed, result.status], [10, 1, 'failed'])
  })
  await test('conflicting or malformed exact receipts fail closed to unknown', () => {
    for (const extra of [event(12), event('11'), event(0)]) {
      const result = automated(run, rows(11), [event(11), extra], [])
      assert.equal(result.total, null)
      assert.equal(result.status, 'pending')
    }
    assert.equal(automated(run, rows(11), [event(11)], [{ ...history, replacement_verification_policy: policy(12) }]).total, null)
  })
  await test('other run/task receipts and evidence cannot substitute a denominator or result', () => {
    const result = automated(run, [...rows(3), { ...rows(1)[0], task_id: 'other-task' }],
      [event(11), event(12, 'other-run')], [{ ...history, task_id: 'other-task', replacement_verification_policy: policy(12) }])
    assert.deepEqual([result.total, result.records.length, result.missing], [11, 3, 8])
  })
  await test('duplicate, unknown-status or out-of-range records never produce a passing aggregate', () => {
    for (const evidence of [
      [...rows(10), rows(1)[0]],
      [...rows(10), { ...rows(1)[0], check_index: 11 }],
      [...rows(10), { ...rows(11)[10], status: 'unknown' }],
    ]) {
      const result = automated(run, evidence, [event(11)], [])
      assert.equal(result.status, 'pending')
      assert.match(result.score, /completeness unknown/)
    }
  })
  await test('exact context selects source and checkpoint instead of the newest snapshot candidate', () => {
    const newest = { ...run, id: 'newer-pre-start-failure', workspace_fingerprint: 'c'.repeat(64) }
    const result = recovery(context, [newest, run])
    assert.equal(result.run.id, context.source_run_id)
    assert.equal(result.checkpoint, context.workspace_fingerprint)
    assert.notEqual(result.checkpoint, run.workspace_fingerprint)
    assert.match(result.detail, /controller rechecks authorization before running/)
  })
  await test('legacy missing fingerprint retains the native owning-runner checkpoint path', () => {
    const result = recovery({ ...context, workspace_fingerprint: null }, [run])
    assert.equal(result.state, 'checkpoint_required')
    assert.equal(result.checkpoint, null)
    assert.match(result.detail, /controller can ask the owning runner to seal/)
    assert.equal('canRequest' in result, false)
  })
  await test('server-selected lost or cancelled sources are not rejected by frontend status rules', () => {
    for (const status of ['lost', 'cancelled']) {
      const result = recovery(context, [{ ...run, status }])
      assert.equal(result.run.status, status)
      assert.equal(result.state, 'ready')
      assert.equal(result.checkpoint, context.workspace_fingerprint)
    }
  })
  await test('remaining attempts, budgets and breaker state are not a second frontend eligibility engine', () => {
    const result = recovery({ ...context, remaining_attempts: 0, remaining_mission_tokens: 0, remaining_mission_cost_microusd: 0 },
      [{ ...run, breaker_stage: 'stop' }])
    assert.equal(result.state, 'ready')
    assert.equal('canRequest' in result, false)
    assert.doesNotMatch(declarations.find((text) => text.startsWith('function factoryRecoveryPresentation')),
      /max_attempts|attempt_count|remaining_mission|breaker_stage|failedTasks/)
  })
  await test('known quarantine warns and hides hashes without manufacturing an authorization decision', () => {
    const quarantined = { ...run, id: 'quarantined-descendant', workspace_disposition: 'quarantined' }
    for (const runs of [[run, quarantined], [quarantined]]) {
      const result = recovery(context, runs)
      assert.equal(result.state, 'quarantined')
      assert.equal(result.checkpoint, null)
      assert.equal('canRequest' in result, false)
    }
  })
  await test('exact context stays usable when its source row is outside the bounded snapshot', () => {
    const result = recovery(context, [])
    assert.equal(result.run, undefined)
    assert.equal(result.state, 'ready')
    assert.equal(result.checkpoint, context.workspace_fingerprint)
    assert.ok(appSource.includes('The exact endpoint returns task/source IDs, not the task contract or adapter.'))
  })
  await test('active recovery uses exact context history, with no snapshot-count fence or duplicate grant', () => {
    const result = recovery({ ...context, recoveries: [{ ...history, status: 'running' }] }, [run])
    assert.equal(result.state, 'active')
    assert.equal(result.checkpoint, null)
    assert.match(result.detail, /do not create a duplicate grant/)
  })
  await test('context loader uses only the exact actor-scoped GET through the existing API helper', async () => {
    const loads = [], calls = []
    apiHandler = (url, init) => { calls.push({ url, init }); return Promise.resolve(context) }
    const cleanup = start(scope, loads)
    assert.equal(loads[0].status, 'loading')
    assert.equal(loads[0].data, null)
    await flush()
    assert.equal(calls.length, 1)
    assert.equal(calls[0].url, `/api/corps/${scope.corpId}/factory/work-items/${scope.itemId}/verification-recoveries?actor_id=${scope.actorId}`)
    assert.equal(calls[0].init.method, 'GET')
    assert.ok(calls[0].init.signal instanceof AbortSignal)
    assert.equal(loads.at(-1).data.source_run_id, context.source_run_id)
    assert.equal(loads.at(-1).status, 'ready')
    cleanup()
    assert.equal(timers.size, 0)
  })
  await test('render-time scope fencing hides prior actor, item, version, mission and reload data/errors', () => {
    for (const change of [
      { actorId: 'bob' }, { itemId: 'other-item' }, { version: 8 },
      { corpId: 'other-corp' }, { missionId: 'other-mission' }, { reload: 1 },
    ]) {
      for (const status of ['ready', 'error', 'loading']) {
        const old = { scopeKey: helpers.factoryRecoveryScopeKey(scope), status, data: context, error: 'old error' }
        assert.equal(helpers.currentFactoryRecoveryLoad({ ...scope, ...change }, old), null)
      }
    }
    assert.equal(helpers.currentFactoryRecoveryLoad(null, { scopeKey: helpers.factoryRecoveryScopeKey(scope), data: context }), null)
  })
  await test('actor switch aborts and ignores an old success even when transport completes late', async () => {
    const old = deferred(), next = deferred(), loads = [], signals = []
    apiHandler = (url, init) => { signals.push(init.signal); return url.endsWith('alice') ? old.promise : next.promise }
    const stopOld = start(scope, loads)
    stopOld()
    const bobScope = { ...scope, actorId: 'bob' }
    const stopNext = start(bobScope, loads)
    next.resolve(context)
    await flush()
    const length = loads.length
    old.resolve({ ...context, workspace_fingerprint: 'stale-fingerprint' })
    await flush()
    assert.equal(signals[0].aborted, true)
    assert.equal(loads.length, length)
    assert.equal(loads.at(-1).scopeKey, helpers.factoryRecoveryScopeKey(bobScope))
    assert.equal(loads.at(-1).data.workspace_fingerprint, context.workspace_fingerprint)
    stopNext()
    assert.equal(timers.size, 0)
  })
  await test('cancelled lookup cannot publish a late error after version/item change or unmount', async () => {
    const pending = deferred(), loads = []
    apiHandler = () => pending.promise
    const cleanup = start(scope, loads)
    cleanup()
    pending.reject(new Error('late obsolete failure'))
    await flush()
    assert.deepEqual(loads.map((load) => load.status), ['loading'])
    assert.equal(timers.size, 0)
  })
  await test('server errors remain explicit and never fall back to an ancestor snapshot', async () => {
    const loads = []
    apiHandler = () => Promise.reject(new Error('No authoritative source context'))
    const cleanup = start(scope, loads)
    await flush()
    assert.equal(loads.at(-1).status, 'error')
    assert.equal(loads.at(-1).data, null)
    assert.match(loads.at(-1).error, /No authoritative source context/)
    cleanup()
  })
  await test('mismatched context scope or version is rejected before displaying a source hash', async () => {
    for (const body of [
      { ...context, work_item: { ...item, version: 8 } },
      { ...context, work_item: { ...item, id: 'other-item' } },
      { ...context, work_item: { ...item, corp_id: 'other-corp' } },
      { ...context, mission_id: 'other-mission' },
    ]) {
      const loads = []
      apiHandler = () => Promise.resolve(body)
      const cleanup = start(scope, loads)
      await flush()
      assert.equal(loads.at(-1).status, 'error')
      assert.equal(loads.at(-1).data, null)
      cleanup()
    }
  })
  await test('missing response fields are reported as a gap, not inferred from snapshot data', async () => {
    const loads = []
    apiHandler = () => Promise.resolve({ ...context, workspace_fingerprint: undefined })
    const cleanup = start(scope, loads)
    await flush()
    assert.equal(loads.at(-1).status, 'error')
    assert.match(loads.at(-1).error, /missing required source, checkpoint/)
    cleanup()
  })
  await test('bounded lookup timeout reports an error and ignores a late response', async () => {
    const pending = deferred(), loads = []
    let signal
    apiHandler = (_url, init) => { signal = init.signal; return pending.promise }
    const cleanup = start(scope, loads)
    assert.equal(timers.size, 1)
    ;[...timers.values()][0]()
    assert.equal(signal.aborted, true)
    assert.equal(loads.at(-1).status, 'error')
    assert.match(loads.at(-1).error, /timed out/)
    const count = loads.length
    pending.resolve(context)
    await flush()
    assert.equal(loads.length, count)
    cleanup()
    assert.equal(timers.size, 0)
  })
  await test('actual markup uses context IDs/counters and gates copy on fresh metadata, not eligibility', () => {
    assert.equal(recovery(null, [run]), null)
    assert.match(appSource, /recoveryItemState === 'verification_failed'/)
    assert.match(appSource, /currentFactoryRecoveryLoad\(recoveryScope, recoveryContextLoad\)/)
    assert.match(appSource, /requestFactoryRecoveryContext\(recoveryScope, setRecoveryContextLoad\)/)
    assert.match(appSource, /recoveryCommandAvailable && canAuthorizeRecovery/)
    assert.match(appSource, /data-recovery-run-id=\{recoveryContext\?\.source_run_id\}/)
    assert.match(appSource, /recoveryContext\.remaining_attempts/)
    assert.match(appSource, /recoveryCopyKey\('verifier-only'\)/)
    assert.doesNotMatch(appSource, /recovery\??\.canRequest/)
    assert.match(appSource, /resumableRun && activeRuns === 0 && factoryItem\?\.state !== 'verification_failed'/)
  })
  await test('mobile identity wrapper spans the same 720px breakpoint as the operator grid', () => {
    assert.match(css, /@media \(max-width: 720px\)\s*\{\s*\.app-shell \.operator-console \.operations-identity\s*\{[^}]*grid-column: 1 \/ -1;[^}]*width: 100%/s)
  })
  await test('approval copy keeps exact scope, Solo/parallel expectations and three-worker studio semantics', () => {
    for (const copy of [
      'only the exact action and scope shown', 'does not need a duplicate grant',
      'Solo run can still need decisions for risky actions',
      'Two specialists and synthesis can request different scoped actions',
      'does not create a fourth concurrent worker or a grant per artifact',
      'ECorp provisions 3 distinct mission workers on GitHub Copilot.',
    ]) assert.ok(appSource.includes(copy), `Missing scoped approval/studio copy: ${copy}`)
  })
  await test('same-backlog guidance requires same server, Corp and claim namespace', async () => {
    for (const file of ['../CONTRIBUTING.md', '../docs/DARK_FACTORY_CONTRIBUTOR_GUIDE.md']) {
      const text = normalizedText((await readFile(new URL(file, import.meta.url), 'utf8')).replaceAll('**', ''))
      assert.match(text, /same authenticated server\/control plane, the same Corp, and the same claim namespace/)
    }
  })
}

if (process.argv.includes('--pure-test')) {
  assert.deepEqual(process.argv.slice(2), ['--pure-test'], 'Pure tests accept no runtime arguments')
  await pureTests()
} else {
  await main().catch((error) => {
    console.error(safeError(error))
    process.exitCode = 1
  })
}
