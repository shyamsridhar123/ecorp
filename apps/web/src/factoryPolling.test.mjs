import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import { factoryControllerState, presentFactoryPolling } from './factoryPolling.ts'

const now = Date.parse('2026-09-06T15:00:00Z')
const nextRetry = '2026-09-06T15:05:07Z'
const reset = '2026-09-06T16:00:00+00:00'
const observed = '2026-09-06T14:59:50Z'
const controller = (polling = {}, health = {}) => ({
  status: 'backing_off',
  desired_state: 'running',
  polling: {
    next_retry_at: nextRetry,
    retry_reason: 'graphql_quota',
    consecutive_failures: 1,
    graphql: { limit: 5000, remaining: 12, cost: 2, reset_at: reset, observed_at: observed },
    ...polling,
  },
  ...health,
})
const field = (view, label) => view.fields.find((entry) => entry.label === label)

for (const [reason, expected] of [
  ['primary_rate_limit', /primary API rate limit/],
  ['secondary_rate_limit', /slowing requests \(secondary rate limit\)/],
  ['graphql_quota', /GraphQL quota is low/],
  ['github_unavailable', /GitHub is temporarily unavailable/],
]) {
  test(`${reason} has a reader-facing reason and an automatic future retry`, () => {
    const input = controller({ retry_reason: reason })
    const before = structuredClone(input)
    const view = presentFactoryPolling(input, now)
    assert.equal(factoryControllerState(input), 'backing_off')
    assert.equal(view.heading, 'GitHub intake waiting')
    assert.match(view.reason, expected)
    assert.equal(view.retryState, 'scheduled')
    assert.equal(view.refreshAt, Date.parse(nextRetry))
    assert.match(view.retryMessage, /resume automatically at or after the scheduled time/)
    assert.match(view.continuity, /Local runs and reviews continue/)
    assert.match(view.backoffRule, /Queue changes and Reconcile now cannot bypass GitHub backoff/)
    assert.deepEqual(input, before)
  })
}

test('quota, cost and timestamps come from the snapshot, not derived or default budgets', () => {
  const view = presentFactoryPolling(controller(), now)
  assert.equal(field(view, 'GraphQL remaining / limit').value, '12 / 5000')
  assert.equal(field(view, 'Last query cost').value, '2')
  for (const [label, timestamp] of [
    ['Next retry (local)', nextRetry],
    ['GraphQL reset (local)', reset],
    ['Last observed (local)', observed],
  ]) {
    const actual = field(view, label)
    assert.equal(actual.dateTime, timestamp)
    assert.equal(actual.value, new Date(timestamp).toLocaleString(undefined, {
      year: 'numeric', month: 'short', day: 'numeric',
      hour: 'numeric', minute: '2-digit', second: '2-digit', timeZoneName: 'short',
    }))
    assert.match(actual.value, /2026/)
  }
})

test('equivalent offset timestamps have the same local retry time and boundary', () => {
  const utc = presentFactoryPolling(controller(), now)
  const offset = presentFactoryPolling(controller({
    next_retry_at: '2026-09-06T10:05:07-05:00',
  }), now)
  assert.equal(field(offset, 'Next retry (local)').value, field(utc, 'Next retry (local)').value)
  assert.equal(offset.refreshAt, utc.refreshAt)
})

test('at or after the retry deadline, waiting is not presented as a successful retry', () => {
  for (const current of [Date.parse(nextRetry), Date.parse(nextRetry) + 60_000]) {
    const input = controller()
    const view = presentFactoryPolling(input, current)
    assert.equal(view.retryState, 'elapsed')
    assert.equal(view.refreshAt, null)
    assert.equal(view.heading, 'GitHub intake waiting')
    assert.match(view.retryMessage, /time has passed; waiting for the controller/)
    assert.doesNotMatch(view.retryMessage, /retrying now|resumed|succeeded/i)
    assert.equal(field(view, 'Next retry (local)').dateTime, nextRetry)
    assert.equal(factoryControllerState(input), 'backing_off')
  }
})

test('zero remaining, limit, query cost and failures remain real zeros', () => {
  const view = presentFactoryPolling(controller({
    consecutive_failures: 0,
    graphql: { limit: 0, remaining: 0, cost: 0, reset_at: reset, observed_at: observed },
  }), now)
  assert.equal(field(view, 'GraphQL remaining / limit').value, '0 / 0')
  assert.equal(field(view, 'Last query cost').value, '0')
  assert.equal(field(view, 'Consecutive intake failures').value, '0')
  assert.match(view.reason, /quota is exhausted/)
})

test('paused desired state wins without rewriting controller health or promising a retry', () => {
  for (const current of [now, Date.parse(nextRetry) + 1]) {
    const input = controller({}, { desired_state: 'paused' })
    const view = presentFactoryPolling(input, current)
    assert.equal(factoryControllerState(input), 'paused')
    assert.equal(input.status, 'backing_off')
    assert.equal(view.heading, 'Intake paused')
    assert.match(view.retryMessage, /wait until intake is resumed/)
    assert.equal(view.refreshAt, null)
    assert.equal(field(view, 'Recorded retry (local)').dateTime, nextRetry)
    assert.equal(field(view, 'GraphQL remaining / limit').value, '12 / 5000')
  }
})

test('offline health wins over polling metadata, including an elapsed retry', () => {
  for (const current of [now, Date.parse(nextRetry) + 1]) {
    const input = controller({}, { status: 'offline' })
    const view = presentFactoryPolling(input, current)
    assert.equal(factoryControllerState(input), 'offline')
    assert.equal(input.desired_state, 'running')
    assert.equal(view.heading, 'Controller offline')
    assert.match(view.retryMessage, /retry timing is not live/)
    assert.match(view.retryMessage, /after reconnection/)
    assert.equal(view.refreshAt, null)
    assert.equal(field(view, 'Recorded retry (local)').dateTime, nextRetry)
  }
})

test('paused and offline together retain both conditions and existing paused precedence', () => {
  const input = controller({}, { desired_state: 'paused', status: 'offline' })
  const view = presentFactoryPolling(input, now)
  assert.equal(factoryControllerState(input), 'paused')
  assert.match(view.retryMessage, /paused and the controller is offline/)
  assert.match(view.retryMessage, /resume and reconnection/)
})

test('absent legacy polling leaves the old strip intact and never fabricates a wait', () => {
  assert.equal(factoryControllerState(undefined), 'not_configured')
  for (const input of [undefined, null, false, [], 'legacy']) {
    assert.equal(presentFactoryPolling(input, now), null)
  }
  for (const polling of [undefined, null]) {
    const input = { status: 'watching', desired_state: 'running', polling }
    assert.equal(presentFactoryPolling(input, now), null)
    assert.equal(factoryControllerState(input), 'watching')
  }
  assert.equal(factoryControllerState({ status: 'unknown' }), 'unavailable')
})

test('missing or malformed backoff data is unavailable, never zero or an invented deadline', () => {
  for (const polling of [undefined, null, {}, [], false, 0, 'invalid']) {
    const view = presentFactoryPolling(controller({}, { polling }), now)
    assert.equal(view.reason, 'Unavailable')
    assert.equal(view.retryState, 'unavailable')
    assert.equal(view.refreshAt, null)
    assert.match(view.retryMessage, /next retry time is unavailable/)
    assert.equal(field(view, 'GraphQL remaining / limit').value, 'Unavailable / Unavailable')
    assert.equal(field(view, 'Last query cost').value, 'Unavailable')
    assert.equal(field(view, 'Consecutive intake failures').value, 'Unavailable')
    for (const entry of view.fields.filter(({ label }) => label.endsWith('(local)'))) {
      assert.equal(entry.value, 'Unavailable')
      assert.equal(entry.dateTime, undefined)
    }
  }
})

test('invalid values are not coerced to numbers or dates and unknown reasons are not rendered raw', () => {
  for (const invalid of [undefined, null, '', '0', -1, 1.5, NaN, Infinity, false, {}, []]) {
    const view = presentFactoryPolling(controller({
      consecutive_failures: invalid,
      retry_reason: 'toString',
      graphql: { limit: invalid, remaining: invalid, cost: invalid },
    }), now)
    assert.equal(view.reason, 'Unavailable')
    assert.equal(field(view, 'GraphQL remaining / limit').value, 'Unavailable / Unavailable')
    assert.equal(field(view, 'Last query cost').value, 'Unavailable')
    assert.equal(field(view, 'Consecutive intake failures').value, 'Unavailable')
  }
  for (const invalid of ['', '0', 'not-a-date', '2026-09-06', '2026-09-06T15:00:00',
    '2026-02-30T15:00:00Z', '2026-13-01T15:00:00Z', '2026-09-06T24:00:00Z', 0, null]) {
    const view = presentFactoryPolling(controller({
      next_retry_at: invalid,
      graphql: { reset_at: invalid, observed_at: invalid },
    }), now)
    assert.equal(view.refreshAt, null)
    assert.equal(view.retryState, 'unavailable')
    assert.equal(field(view, 'Next retry (local)').value, 'Unavailable')
    assert.equal(field(view, 'GraphQL reset (local)').value, 'Unavailable')
    assert.equal(field(view, 'Last observed (local)').value, 'Unavailable')
  }
})

test('partial quota data preserves observed zeros while missing fields remain unavailable', () => {
  const view = presentFactoryPolling(controller({
    graphql: { remaining: 0, cost: 0 },
  }), now)
  assert.equal(field(view, 'GraphQL remaining / limit').value, '0 / Unavailable')
  assert.equal(field(view, 'Last query cost').value, '0')
  assert.equal(field(view, 'GraphQL reset (local)').value, 'Unavailable')
})

test('healthy telemetry does not infer a backoff from a zero quota or failure count', () => {
  const input = controller({
    next_retry_at: null, retry_reason: null, consecutive_failures: 0,
    graphql: { limit: 5000, remaining: 0, cost: 0, reset_at: reset, observed_at: observed },
  }, { status: 'working' })
  const view = presentFactoryPolling(input, now)
  assert.equal(factoryControllerState(input), 'working')
  assert.equal(view.heading, 'GitHub polling')
  assert.equal(view.refreshAt, null)
  assert.equal(field(view, 'GraphQL remaining / limit').value, '0 / 5000')
  assert.match(view.retryMessage, /No intake backoff is currently reported/)
})

test('UI integration remains read-only, accessible, and uses existing controller controls', async () => {
  const notice = await readFile(new URL('./FactoryPollingNotice.tsx', import.meta.url), 'utf8')
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8')
  const strip = app.slice(app.indexOf('aria-label="Factory controller status"'),
    app.indexOf('<div className={`factory-console'))
  assert.match(notice, /aria-label="GitHub intake polling"/)
  assert.match(notice, /role="status"/)
  assert.match(notice, /<time dateTime=\{field\.dateTime\}/)
  assert.match(notice, /window\.clearTimeout\(timeout\)/)
  assert.doesNotMatch(notice, /fetch\(|\bapi[<(]|\bonClick\b|<button|\bonSubmit\b/)
  assert.match(strip, /<FactoryPollingNotice controller=\{controller\}/)
  assert.equal((strip.match(/disabled=\{busy \|\| !canControlFactory\}/g) ?? []).length, 2)
  assert.match(strip, /controller\.desired_state === 'paused' \? 'resume' : 'pause'/)
  assert.match(strip, /onControllerControl\(controller, 'reconcile'\)/)
  assert.match(app, /import '\.\/Accessible\.css'/)
})
