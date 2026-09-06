// Run only after the parent confirms persisted backoff on owned QA web15498/API18967.
// Required: CRONY_POLLING_TEST=1, CRONY_POLLING_WEB, CRONY_POLLING_OUTPUT (directory).
// Optional: CRONY_POLLING_REASON, CRONY_PLAYWRIGHT_MODULE (installed package path).
// Otherwise read-only navigation: allow only the normal browser bootstrap handshake
// on owned API18967 with seed_crew=false and literal body {}. Parent pre-seeds QA.
// No other mutations, setup, controls, or response/page/fixture injection.
import assert from 'node:assert/strict'
import { mkdir, writeFile } from 'node:fs/promises'
import { createRequire } from 'node:module'
import { homedir } from 'node:os'
import path from 'node:path'

assert.equal(process.env.CRONY_POLLING_TEST, '1', 'Explicit CRONY_POLLING_TEST=1 is required')
assert.ok(process.env.CRONY_POLLING_WEB, 'Explicit CRONY_POLLING_WEB is required')
assert.ok(process.env.CRONY_POLLING_OUTPUT?.trim(), 'Explicit CRONY_POLLING_OUTPUT is required')
const web = new URL(process.env.CRONY_POLLING_WEB)
const loopback = new Set(['127.0.0.1', 'localhost', '[::1]'])
assert.equal(web.protocol, 'http:')
assert.ok(loopback.has(web.hostname), 'Only an owned loopback QA stack is allowed')
assert.equal(web.port, '15498', 'Use owned QA web15498, never manual web15491/API18962')
assert.equal(web.pathname, '/')
assert.ok(!web.username && !web.password && !web.search && !web.hash)
const reasons = {
  primary_rate_limit: /GitHub primary API rate limit reached\./,
  secondary_rate_limit: /GitHub is temporarily slowing requests \(secondary rate limit\)\./,
  graphql_quota: /GitHub GraphQL quota is (?:low|exhausted)\./,
  github_unavailable: /GitHub is temporarily unavailable\./,
}
const expectedReason = process.env.CRONY_POLLING_REASON || null
assert.ok(expectedReason === null || Object.hasOwn(reasons, expectedReason),
  'CRONY_POLLING_REASON must be a supported server retry_reason')
const output = path.resolve(process.env.CRONY_POLLING_OUTPUT)
const report = {
  passed: false, web: web.origin, api_port: 18967, expected_reason: expectedReason,
  started_at: new Date().toISOString(), browser_closed: false,
  scope: 'Read-only navigation except POST on owned loopback API18967 /api/demo/bootstrap?seed_crew=false with literal body {}.',
  allowed_bootstrap_handshake_count: 0,
  work_creation_audit: 'Not performed; pre-seeded QA Corp/actors are caller-confirmed, not verified by an ID comparison.',
  cases: [], errors: [], blocked_requests: [],
}
const saveText = (name, text) => writeFile(path.join(output, name), `${text}\n`, 'utf8')
function permitted(url) {
  const target = new URL(url)
  return ['http:', 'ws:'].includes(target.protocol) && loopback.has(target.hostname)
    && ['15498', '18967'].includes(target.port) && !target.username && !target.password
}
function permittedBootstrap(request) {
  const target = new URL(request.url())
  return permitted(request.url()) && target.protocol === 'http:' && target.port === '18967'
    && target.pathname === '/api/demo/bootstrap' && target.search === '?seed_crew=false'
    && !target.hash && request.method() === 'POST' && request.postData() === '{}'
}
function safeUrl(value) {
  const url = new URL(value)
  return `${url.origin}${url.pathname}` // Do not record tokens or actor query parameters.
}

await mkdir(output, { recursive: true })
let browser
let page
let closing = false
try {
  const require = createRequire(import.meta.url)
  const bundled = path.join(homedir(), '.cache', 'codex-runtimes',
    'codex-primary-runtime', 'dependencies', 'node')
  const modulePath = process.env.CRONY_PLAYWRIGHT_MODULE
    || require.resolve('playwright', { paths: [process.cwd(), bundled] })
  const { chromium } = require(modulePath)
  browser = await chromium.launch({ channel: 'chrome', headless: true })
  const context = await browser.newContext({
    viewport: { width: 1440, height: 1050 }, reducedMotion: 'reduce', serviceWorkers: 'block',
  })
  await context.route('**/*', (route) => {
    const request = route.request()
    if (permitted(request.url()) && ['GET', 'HEAD', 'OPTIONS'].includes(request.method())) {
      return route.continue()
    }
    if (permittedBootstrap(request)) {
      report.allowed_bootstrap_handshake_count += 1
      return route.continue() // Forward the actual browser request; never synthesize a response.
    }
    report.blocked_requests.push({ method: request.method(), url: safeUrl(request.url()) })
    return route.abort('blockedbyclient')
  })
  await context.routeWebSocket('**/*', (route) => {
    if (permitted(route.url())) return void route.connectToServer()
    report.blocked_requests.push({ method: 'WEBSOCKET', url: safeUrl(route.url()) })
    return route.close({ code: 1008, reason: 'Only owned QA origins are allowed' })
  })
  page = await context.newPage()
  page.setDefaultTimeout(20_000)
  page.on('pageerror', (error) => report.errors.push(`page: ${error.message}`))
  page.on('console', (message) => {
    if (message.type() === 'error') report.errors.push(`console: ${message.text()}`)
  })
  page.on('requestfailed', (request) => {
    if (!closing) report.errors.push(`request: ${safeUrl(request.url())}: ${request.failure()?.errorText}`)
  })
  page.on('response', (response) => {
    if (response.status() >= 400) {
      report.errors.push(`HTTP ${response.status()}: ${safeUrl(response.url())}`)
    }
  })

  await page.goto(web.origin, { waitUntil: 'domcontentloaded', timeout: 30_000 })
  await page.locator('body').waitFor({ state: 'visible' })
  // Observe the real page before navigating or asserting against specific UI.
  await saveText('initial-dom.txt', await page.locator('body').innerText())
  await page.goto(`${web.origin}/#factory`, { waitUntil: 'domcontentloaded', timeout: 30_000 })
  const notice = page.locator('#factory .factory-polling-notice[aria-label="GitHub intake polling"]')
  await notice.waitFor({ state: 'visible' })
  assert.equal(await notice.count(), 1, 'Require exactly one real Factory polling notice')

  for (const [name, width, height] of [['desktop', 1440, 1050], ['mobile-390', 390, 844]]) {
    await page.setViewportSize({ width, height })
    await notice.scrollIntoViewIfNeeded()
    const observed = await notice.evaluate((element) => {
      const factory = document.getElementById('factory')
      return {
        text: element.innerText,
        controller_text: factory.querySelector('[aria-label="Factory controller status"]').innerText,
        backing_off: !!factory.querySelector('.factory-controller-backing_off'),
        fields: Object.fromEntries([...element.querySelectorAll('dl > div')].map((row) => [
          row.querySelector('dt').textContent.trim(),
          {
            text: row.querySelector('dd').textContent.trim(),
            date_time: row.querySelector('time')?.getAttribute('datetime') ?? null,
          },
        ])),
        local_timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
        layout: {
          viewport: innerWidth,
          document: document.documentElement.scrollWidth,
          body: document.body.scrollWidth,
          factory_client: factory.clientWidth, factory_scroll: factory.scrollWidth,
          notice_client: element.clientWidth, notice_scroll: element.scrollWidth,
        },
      }
    })
    const evidence = { name, width, height, captured_at: new Date().toISOString(), passed: false, ...observed }
    report.cases.push(evidence)
    await saveText(`${name}-dom.txt`, await page.locator('body').innerText())
    await page.screenshot({ path: path.join(output, `${name}.png`), fullPage: true })
    assert.ok(observed.backing_off, `${name}: controller must currently be backing_off, not paused/offline`)
    assert.match(observed.text, /GitHub intake waiting\./)
    assert.match(observed.text, /Intake retries resume automatically at or after the scheduled time\./)
    assert.match(observed.text, /Local runs and reviews continue while intake waits\./)
    assert.match(observed.text, /Queue changes and Reconcile now cannot bypass GitHub backoff\./)
    const actualReason = Object.keys(reasons).find((reason) => reasons[reason].test(observed.text))
    assert.ok(actualReason, `${name}: real retry reason is absent or unavailable`)
    if (expectedReason) assert.equal(actualReason, expectedReason, `${name}: wrong backoff fixture`)
    evidence.retry_reason = actualReason

    const retry = observed.fields['Next retry (local)']
    assert.ok(retry?.date_time, `${name}: exact next retry timestamp is missing`)
    assert.ok(Date.parse(retry.date_time) > Date.now(), `${name}: retry is elapsed or invalid, not future`)
    evidence.next_retry_at = retry.date_time
    for (const label of ['Next retry (local)', 'GraphQL reset (local)', 'Last observed (local)']) {
      const field = observed.fields[label]
      assert.ok(field?.date_time && Number.isFinite(Date.parse(field.date_time)), `${name}: missing ${label}`)
      const local = await page.evaluate((value) => new Date(value).toLocaleString(undefined, {
        year: 'numeric', month: 'short', day: 'numeric',
        hour: 'numeric', minute: '2-digit', second: '2-digit', timeZoneName: 'short',
      }), field.date_time)
      assert.equal(field.text, local, `${name}: ${label} must show the exact browser-local time`)
    }
    const budget = /^(\d+) \/ (\d+)$/.exec(observed.fields['GraphQL remaining / limit']?.text ?? '')
    const cost = observed.fields['Last query cost']?.text ?? ''
    assert.ok(budget && /^\d+$/.test(cost), `${name}: quota/cost must be real numbers, not Unavailable`)
    evidence.graphql = { remaining: Number(budget[1]), limit: Number(budget[2]), cost: Number(cost) }
    assert.ok(Object.values(evidence.graphql).every(Number.isSafeInteger), `${name}: invalid quota numbers`)
    evidence.zero_fields = Object.keys(evidence.graphql).filter((key) => evidence.graphql[key] === 0)
    if (actualReason === 'graphql_quota') {
      assert.equal(evidence.graphql.remaining, 0, `${name}: low-quota fixture must exercise zero rendering`)
    }
    const layout = observed.layout
    assert.equal(layout.viewport, width)
    assert.ok(layout.document <= width + 1 && layout.body <= width + 1
      && layout.factory_scroll <= layout.factory_client + 1
      && layout.notice_scroll <= layout.notice_client + 1, `${name}: horizontal overflow`)
    assert.deepEqual(report.blocked_requests, [], 'Forbidden network/mutation request attempted; see report')
    assert.deepEqual(report.errors, [], `${name}: browser/network errors`)
    evidence.passed = true
  }
  assert.equal(report.cases.length, 2)
  assert.equal(report.cases[0].retry_reason, report.cases[1].retry_reason, 'Fixture changed between captures')
  assert.equal(report.cases[0].next_retry_at, report.cases[1].next_retry_at, 'Retry changed between captures')
  report.passed = true
} catch (error) {
  report.failure = error.message
  if (page && !page.isClosed()) {
    try {
      await saveText('failure-dom.txt', await page.locator('body').innerText())
      await page.screenshot({ path: path.join(output, 'failure.png'), fullPage: true, timeout: 10_000 })
    } catch (captureError) {
      report.errors.push(`failure capture: ${captureError.message}`)
    }
  }
} finally {
  closing = true
  try {
    if (browser) {
      await browser.close()
      report.browser_closed = true
    }
  } catch (error) {
    report.errors.push(`browser close: ${error.message}`)
  }
  if (report.errors.length || report.blocked_requests.length || !report.browser_closed) report.passed = false
  report.finished_at = new Date().toISOString()
  await saveText('report.json', JSON.stringify(report, null, 2))
  console.log(JSON.stringify({ passed: report.passed, report: path.join(output, 'report.json'),
    failure: report.failure ?? null, scope: report.scope,
    allowed_bootstrap_handshake_count: report.allowed_bootstrap_handshake_count,
    work_creation_audit: report.work_creation_audit,
    blocked_requests: report.blocked_requests }, null, 2))
  if (!report.passed) process.exitCode = 1
}
