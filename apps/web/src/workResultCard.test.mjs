import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import { isValidElement } from 'react'
import * as jsxRuntime from 'react/jsx-runtime'
import { renderToStaticMarkup } from 'react-dom/server'
import ts from 'typescript'
import * as reader from './missionResultContext.ts'
import * as originReader from './missionOriginContext.ts'
import * as evidenceSelection from './evidenceSelection.ts'
import * as workflow from './workflowContext.ts'
import * as checkpointRecovery from './factoryCheckpointRecovery.ts'

// Actual components, hook and reader; only hook scheduling, transport and timers
// are controlled. SSR/callback tests are not browser, download or runtime proof.
async function compile(fileName) {
  const source = await readFile(new URL(`./${fileName}`, import.meta.url), 'utf8')
  const compiled = ts.transpileModule(source, {
    fileName, reportDiagnostics: true,
    compilerOptions: {
      target: ts.ScriptTarget.ES2023, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX,
    },
  })
  assert.deepEqual(compiled.diagnostics, [], fileName)
  return compiled.outputText
}

function evaluate(source, imports) {
  const exports = {}
  new Function('require', 'exports', source)((name) => {
    assert.ok(Object.hasOwn(imports, name), `Unexpected test import: ${name}`)
    return imports[name]
  }, exports)
  return exports
}

const { WorkResultCard } = evaluate(await compile('WorkResultCard.tsx'), {
  'react/jsx-runtime': jsxRuntime, './WorkResultCard.css': {},
})
const { PublishedResultCard } = evaluate(await compile('PublishedResultCard.tsx'), {
  'react/jsx-runtime': jsxRuntime, './WorkResultCard': { WorkResultCard },
})
const hookSource = await compile('useMissionResultContext.ts')
const ids = {
  corpId: 'corp-a', actorId: 'alice', roomId: 'room-a', missionId: 'mission-a',
  workItemId: 'item-a', sourceRepository: 'owner/repo',
}
const scope = reader.missionResultScope(ids)
const digest = 'a'.repeat(64), verification = 'b'.repeat(64)
const baseCommit = 'c'.repeat(40), headCommit = 'd'.repeat(40)
const tick = () => new Promise((resolve) => setImmediate(resolve))

// One synthetic valid public DTO, not a second implementation of reader checks.
// The separate reader suite owns malformed/provenance JSON matrices.
function contextFor(selected = scope, {
  phase = 'published', failure = null, retention = '2099-01-01T00:00:00Z',
} = {}) {
  const [owner, name] = selected.sourceRepository.split('/')
  const issueUrl = `https://github.com/${owner}/${name}/issues/71`
  const work_item = {
    id: selected.workItemId, corp_id: selected.corpId, mission_id: selected.missionId,
    version: 9, source_repository_owner: owner, source_repository_name: name,
    source_issue_number: 71, source_issue_url: issueUrl,
  }
  if (phase === null) return { work_item, publication: null, source_deliverables: [] }
  const runId = `run-${selected.workItemId}`, taskId = `task-${selected.workItemId}`
  const artifactId = `artifact-${selected.workItemId}`, deliverableId = `deliverable-${selected.workItemId}`
  const pr = ['pull_request_created', 'published'].includes(phase) ? {
    number: 17, url: `https://github.com/${owner}/${name}/pull/17`,
    state: 'OPEN', draft: false, base_ref: 'main', head_sha: headCommit,
    head_repository_owner: owner, is_cross_repository: false,
  } : null
  const deliverable = {
    id: deliverableId, corp_id: selected.corpId, task_id: taskId, run_id: runId,
    artifact_id: artifactId, form: 'commit_branch', file_name: 'ecorp-commit-branch.json',
    uri: `/api/corps/${encodeURIComponent(selected.corpId)}/artifacts/${artifactId}`,
    sha256: digest, media_type: 'application/vnd.ecorp.deliverable+json', bytes: 1234,
    provenance_signature: 'e'.repeat(64), verification_sha256: verification,
    base_commit: baseCommit, head_commit: headCommit, branch: 'crony/source',
    integration_state: phase === 'published' ? 'published' : 'ready_for_review',
    retention_until: retention,
  }
  return {
    work_item,
    publication: {
      id: `publication-${selected.workItemId}`, corp_id: selected.corpId,
      mission_id: selected.missionId, factory_work_item_id: selected.workItemId,
      task_id: taskId, run_id: runId, artifact_id: artifactId, source_deliverable_id: deliverableId,
      source_issue_number: 71, source_issue_url: issueUrl,
      state: phase, version: 4, target_repository: selected.sourceRepository,
      base_ref: 'HEAD', branch: 'ecorp/result', commit_sha: headCommit, failure_detail: failure,
      pull_request_number: pr?.number ?? null, pull_request_url: pr?.url ?? null,
      pull_request_state: pr?.state ?? null, pull_request_draft: pr?.draft ?? null,
      pull_request_base_ref: pr?.base_ref ?? null, pull_request_head_sha: pr?.head_sha ?? null,
      pull_request_head_repository_owner: pr?.head_repository_owner ?? null,
      pull_request_is_cross_repository: pr?.is_cross_repository ?? null,
      provenance: {
        schema_version: 2, factory_work_item_id: selected.workItemId, mission_id: selected.missionId,
        task_ids: [taskId], run_ids: [runId], verification_sha256: verification,
        deliverable: {
          id: deliverableId, artifact_id: artifactId, sha256: digest,
          base_commit: baseCommit, head_commit: headCommit, source_branch: deliverable.branch,
        },
        target: {
          repository: selected.sourceRepository, base_ref: 'HEAD', branch: 'ecorp/result', commit: headCommit,
        },
        pull_request: pr,
      },
    },
    source_deliverables: [deliverable],
  }
}

function deferred() {
  let resolve, reject
  const promise = new Promise((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}

function fakeClock() {
  let nextId = 0
  const timers = new Map()
  return {
    timers,
    setTimeout(callback, delay) {
      const id = ++nextId
      timers.set(id, { callback, delay })
      return id
    },
    clearTimeout(id) { timers.delete(id) },
    expire() {
      assert.equal(timers.size, 1)
      const [id, timer] = [...timers][0]
      assert.equal(timer.delay, 15_000)
      timers.delete(id)
      timer.callback()
    },
  }
}

async function readLoad(value, selected = scope) {
  const loads = [], clock = fakeClock()
  const stop = reader.startMissionResultRead(selected,
    () => typeof value === 'function' ? value() : Promise.resolve(value),
    (load) => loads.push(load), clock)
  try {
    await tick()
    assert.equal(clock.timers.size, 0)
    return loads.at(-1)
  } finally {
    stop()
  }
}

async function presentation(options, selectedRun) {
  const load = await readLoad(contextFor(scope, options))
  assert.equal(load.status, 'ready', 'card fixtures must pass the production reader')
  return reader.missionResultPresentation(scope, load, selectedRun)
}

// Inspect semantic elements and invoke their actual supplied callbacks. Avoid
// serialized-markup snapshots, CSS class ordering and incidental wrapper shape.
function hosts(node) {
  if (Array.isArray(node)) return node.flatMap(hosts)
  if (!isValidElement(node)) return []
  if (typeof node.type === 'function') return hosts(node.type(node.props))
  const children = hosts(node.props.children)
  return typeof node.type === 'string' ? [node, ...children] : children
}

function text(node) {
  if (Array.isArray(node)) return node.map(text).join(' ')
  if (!isValidElement(node)) return typeof node === 'string' || typeof node === 'number' ? String(node) : ''
  return text(typeof node.type === 'function' ? node.type(node.props) : node.props.children)
}

function elements(tree, type) {
  return hosts(tree).filter((node) => node.type === type)
}

function button(tree, name) {
  const matches = elements(tree, 'button').filter((node) => name.test(text(node)))
  assert.equal(matches.length, 1, `Expected one button matching ${name}`)
  assert.equal(matches[0].props.type, 'button')
  return matches[0]
}

function card(result, overrides = {}) {
  const calls = { refresh: 0, select: 0, downloads: [] }
  const tree = jsxRuntime.jsx(PublishedResultCard, {
    result,
    onRefresh: () => { calls.refresh++ },
    onSelectDelivered: () => { calls.select++ },
    onDownload: (deliverable) => { calls.downloads.push(deliverable) },
    ...overrides,
  })
  const html = renderToStaticMarkup(tree)
  return { tree, html, calls }
}

function assertNoAutomaticActions(calls) {
  assert.deepEqual(calls, { refresh: 0, select: 0, downloads: [] })
}

function controlledHook(overrides = {}) {
  const slots = [], effects = [], calls = [], scopes = [], clock = fakeClock()
  let cursor = 0, dirty = false, writes = 0, output, result, tree
  const different = (previous, next) =>
    !previous || previous.length !== next.length || next.some((value, index) => !Object.is(value, previous[index]))
  const react = {
    useMemo(create, deps) {
      const index = cursor++
      if (different(slots[index]?.deps, deps)) slots[index] = { deps, value: create() }
      return slots[index].value
    },
    useCallback(callback, deps) { return react.useMemo(() => callback, deps) },
    useState(initial) {
      const index = cursor++
      if (!slots[index]) {
        const state = { value: typeof initial === 'function' ? initial() : initial }
        state.set = (value) => {
          state.value = typeof value === 'function' ? value(state.value) : value
          dirty = true
          writes++
        }
        slots[index] = state
      }
      return [slots[index].value, slots[index].set]
    },
    useEffect(create, deps) {
      const index = cursor++
      if (different(slots[index]?.deps, deps)) {
        const effect = { deps, create, cleanup: slots[index]?.cleanup }
        slots[index] = effect
        effects.push(effect)
      }
    },
  }
  const api = (path, init) => {
    assert.equal(init.method, 'GET', 'result discovery must not execute an action')
    const response = deferred()
    calls.push({ path, init, response })
    return response.promise
  }
  const actualReader = {
    ...reader,
    startMissionResultRead(selected, get, publish) {
      scopes.push(selected)
      return reader.startMissionResultRead(selected, get, publish, clock)
    },
  }
  const { useMissionResultContext } = evaluate(hookSource, {
    react, './missionResultContext': actualReader, './missionResultContext.ts': actualReader,
  })
  let props = { ...ids, actorRole: 'member', revision: 'item-a:9', api, ...overrides }
  const render = function ResultHookHarness(next = props) {
    props = next
    cursor = 0
    dirty = false
    output = useMissionResultContext(props)
    result = reader.missionResultPresentation(output.scope, output.current)
    tree = jsxRuntime.jsx(PublishedResultCard, { result, onRefresh: output.refresh })
    return renderToStaticMarkup(tree)
  }
  const commit = () => {
    for (let pass = 0; ; pass++) {
      assert.ok(pass < 10, 'hook must settle without a render/request loop')
      for (const effect of effects.splice(0)) {
        effect.cleanup?.()
        effect.cleanup = effect.create()
      }
      if (!dirty) return
      render()
    }
  }
  const update = (next = props) => { render(next); commit() }
  return {
    calls, scopes, clock, render, commit, update,
    get props() { return props },
    get output() { return output },
    get result() { return result },
    get tree() { return tree },
    get html() { return renderToStaticMarkup(tree) },
    get writes() { return writes },
    async flush() { await tick(); update() },
    unmount() {
      slots.forEach((slot) => slot.cleanup?.())
      effects.length = 0
      assert.equal(clock.timers.size, 0)
    },
  }
}

test('WorkResultCard exposes its status and supplied content with metadata collapsed by default', () => {
  let invoked = 0
  const tree = jsxRuntime.jsx(WorkResultCard, {
    heading: 'Verified result', status: 'Checking', pending: true,
    description: 'The exact result is being checked.',
    facts: [{ label: 'Repository', value: 'owner/repo' }],
    actions: jsxRuntime.jsx('button', { type: 'button', onClick: () => { invoked++ }, children: 'Inspect result' }),
    children: jsxRuntime.jsx('p', { children: 'Existing review remains selected.' }),
    details: jsxRuntime.jsx('code', { children: 'bounded-metadata' }),
  })
  const html = renderToStaticMarkup(tree)
  const section = elements(tree, 'section').find((node) => node.props['aria-label'] === 'Result and next step')
  assert.ok(section)
  assert.equal(section.props['aria-busy'], true)
  assert.ok(elements(tree, 'p').some((node) => node.props['aria-live'] === 'polite'))
  assert.match(text(tree), /Verified result.*Checking/)
  assert.match(text(tree), /Repository.*owner\/repo/)
  assert.match(text(tree), /Existing review remains selected/)
  const details = elements(tree, 'details')
  assert.equal(details.length, 1)
  assert.equal(Boolean(details[0].props.open), false)
  assert.match(text(details[0]), /Delivery details.*bounded-metadata/)
  assert.doesNotMatch(html, /<details\b[^>]*\sopen(?:[=>\s])/)
  assert.equal(invoked, 0)
  button(tree, /^Inspect result$/).props.onClick()
  assert.equal(invoked, 1)
  assert.equal(elements(jsxRuntime.jsx(WorkResultCard, {
    heading: 'Result', status: 'Waiting', description: 'No metadata yet.',
  }), 'details').length, 0)
})

test('ready PR retains its exact URL and only an explicit download invokes the matching callback', async () => {
  const selected = reader.missionResultScope({ ...ids, sourceRepository: 'Owner/Repo' })
  const wire = contextFor(selected)
  const load = await readLoad(wire, selected)
  assert.equal(load.status, 'ready')
  const result = reader.missionResultPresentation(selected, load)
  const view = card(result)
  const links = elements(view.tree, 'a')
  assert.equal(links.length, 1)
  assert.equal(links[0].props.href, wire.publication.pull_request_url)
  assert.equal(links[0].props.target, '_blank')
  assert.deepEqual(new Set(links[0].props.rel.split(/\s+/)), new Set(['noopener', 'noreferrer']))
  assert.match(text(links[0]), /pull request #\s*17\b/i)
  assertNoAutomaticActions(view.calls)
  button(view.tree, /^Download source bundle$/).props.onClick()
  assert.equal(view.calls.downloads.length, 1)
  assert.equal(view.calls.downloads[0], result.deliverable)
  assert.equal(view.calls.downloads[0].uri, wire.source_deliverables[0].uri)
  assert.equal(view.calls.refresh, 0)
  assert.equal(view.calls.select, 0)
  const busy = card(result, { busy: true })
  assert.equal(button(busy.tree, /^Download source bundle$/).props.disabled, true)
  assert.equal(elements(busy.tree, 'a')[0].props.href, wire.publication.pull_request_url)
  assertNoAutomaticActions(busy.calls)
})

test('loading, unavailable and no-publication states do not fabricate a PR or perform actions', async () => {
  const cases = [
    ['loading', reader.missionResultPresentation(scope, null)],
    ['unavailable', reader.missionResultPresentation(scope, await readLoad(() => Promise.reject(new Error('offline'))))],
    ['none', await presentation({ phase: null })],
  ]
  for (const [state, result] of cases) {
    assert.equal(result.state, state)
    const view = card(result)
    assert.equal(elements(view.tree, 'a').length, 0, state)
    assert.equal(elements(view.tree, 'button').some((node) => /Download|Go to delivered/.test(text(node))), false)
    assertNoAutomaticActions(view.calls)
    if (state === 'loading') {
      assert.equal(elements(view.tree, 'section')[0].props['aria-busy'], true)
      assert.equal(elements(view.tree, 'button').length, 0)
    } else {
      button(view.tree, /^Refresh result$/).props.onClick()
      assert.deepEqual(view.calls, { refresh: 1, select: 0, downloads: [] })
    }
  }
})

test('a historical review mismatch changes selection only through explicit Go to delivered result', async () => {
  const selectedRun = Object.freeze({ id: 'older-review-run', task_id: 'older-review-task' })
  const result = await presentation({}, selectedRun)
  assert.equal(result.state, 'mismatch')
  assert.equal(result.resultRunId, 'run-item-a')
  const view = card(result)
  assert.equal(elements(view.tree, 'a').length, 0)
  assertNoAutomaticActions(view.calls)
  assert.equal(elements(view.tree, 'button').length, 1)
  button(view.tree, /^Go to delivered result$/).props.onClick()
  assert.deepEqual(view.calls, { refresh: 0, select: 1, downloads: [] })
  assert.deepEqual(selectedRun, { id: 'older-review-run', task_id: 'older-review-task' })
})

test('an unavailable delivered-run target preserves the historical selection and only offers refresh', async () => {
  const result = await presentation({}, { id: 'older-review-run' })
  const view = card(result, { onSelectDelivered: undefined })
  assert.equal(elements(view.tree, 'a').length, 0)
  assert.equal(elements(view.tree, 'button').length, 1)
  assert.match(text(view.tree), /No other run has been selected/)
  assertNoAutomaticActions(view.calls)
  button(view.tree, /^Refresh result$/).props.onClick()
  assert.deepEqual(view.calls, { refresh: 1, select: 0, downloads: [] })
})

test('a failed publication with a recorded PR remains inspectable without being labeled Published', async () => {
  const failed = await presentation({ phase: 'pull_request_created', failure: 'Project update failed' })
  const published = await presentation()
  assert.equal(failed.state, 'failed')
  assert.equal(published.state, 'published')
  const failedView = card(failed), publishedView = card(published)
  assert.equal(elements(failedView.tree, 'a')[0].props.href, published.pullRequestUrl)
  assert.match(text(failedView.tree), /Needs attention/)
  assert.match(text(failedView.tree), /publication has not finished/i)
  assert.doesNotMatch(text(failedView.tree), /\bPublished\b/)
  assert.match(text(publishedView.tree), /\bPublished\b/)
  assertNoAutomaticActions(failedView.calls)
  assertNoAutomaticActions(publishedView.calls)
})

test('Published is not merged or deployed and its collapsed details never invent a running preview', async () => {
  const result = await presentation()
  const view = card(result)
  assert.match(text(view.tree), /Publication does not merge or deploy/)
  assert.match(text(view.tree), /not a running preview/)
  assert.equal(elements(view.tree, 'a').length, 1)
  assert.equal(elements(view.tree, 'a')[0].props.href, result.pullRequestUrl)
  assert.equal(elements(view.tree, 'iframe').length, 0)
  assert.equal(elements(view.tree, 'button').some((node) =>
    /^(Open app|Preview|Launch|Deploy|Merge)\b/i.test(text(node))), false)
  const details = elements(view.tree, 'details')
  assert.equal(details.length, 1)
  assert.equal(Boolean(details[0].props.open), false)
  for (const value of [result.publication.branch, result.publication.run_id, result.publication.id]) {
    assert.ok(text(details[0]).includes(value))
  }
  assertNoAutomaticActions(view.calls)
})

test('expired artifact retention does not hide an existing PR or replace server download validation', async () => {
  const result = await presentation({ retention: '2000-01-01T00:00:00Z' })
  assert.equal(result.state, 'published')
  const view = card(result)
  assert.equal(elements(view.tree, 'a')[0].props.href, result.pullRequestUrl)
  assert.match(text(view.tree), /server checks source-download access, integrity and retention/i)
  assertNoAutomaticActions(view.calls)
  button(view.tree, /^Download source bundle$/).props.onClick()
  assert.equal(view.calls.downloads.length, 1)
  assert.equal(view.calls.downloads[0], result.deliverable)
  assert.equal(view.calls.downloads[0].retention_until, '2000-01-01T00:00:00Z')
})

test('actual hook hides prior result before cleanup and aborts on each exact scope change', async () => {
  for (const change of [
    { actorId: 'bob' }, { roomId: 'room-b' }, { missionId: 'mission-b' },
    { workItemId: 'item-b' }, { corpId: 'corp-b' }, { sourceRepository: 'owner/other' },
  ]) {
    const view = controlledHook()
    try {
      view.update()
      await view.flush()
      view.calls[0].response.resolve(contextFor(view.scopes[0]))
      await view.flush()
      assert.equal(view.result.state, 'published')
      const oldScope = view.output.scope
      view.render({ ...view.props, ...change })
      assert.equal(view.calls[0].init.signal.aborted, false, 'cleanup has not run yet')
      assert.notEqual(view.output.scope, oldScope)
      assert.equal(view.output.current, null, JSON.stringify(change))
      assert.equal(elements(view.tree, 'a').length, 0)
      view.commit()
      await view.flush()
      assert.equal(view.calls[0].init.signal.aborted, true)
      assert.equal(view.calls.length, 2)
      view.calls[1].response.resolve(contextFor(view.scopes[1]))
      await view.flush()
      assert.equal(view.result.state, 'published')
      assert.equal(view.output.current.context.work_item_id, view.props.workItemId)
    } finally {
      view.unmount()
    }
  }
})

test('actual hook fences late success and failure before cleanup and after replacement', async () => {
  for (const outcome of ['resolve', 'reject']) {
    for (const timing of ['before-cleanup', 'after-cleanup']) {
      const view = controlledHook()
      try {
        view.update()
        await view.flush()
        const oldScope = view.scopes[0]
        view.render({ ...view.props, actorId: 'bob', roomId: 'room-b' })
        assert.equal(view.output.current, null)
        if (timing === 'after-cleanup') {
          view.commit()
          await view.flush()
          view.calls[1].response.resolve(contextFor(view.scopes[1], { phase: null }))
          await view.flush()
        }
        const writes = view.writes
        view.calls[0].response[outcome](outcome === 'resolve' ? contextFor(oldScope) : new Error('old failure'))
        await tick()
        view.render()
        assert.equal(elements(view.tree, 'a').length, 0)
        if (timing === 'before-cleanup') {
          assert.equal(view.output.current, null, 'a completion racing cleanup must remain hidden')
          view.commit()
          await view.flush()
          view.calls[1].response.resolve(contextFor(view.scopes[1], { phase: null }))
          await view.flush()
        } else {
          assert.equal(view.writes, writes, 'disposed reads cannot write hook state')
        }
        assert.equal(view.calls[0].init.signal.aborted, true)
        assert.equal(view.calls.length, 2)
        assert.equal(view.result.state, 'none')
      } finally {
        view.unmount()
      }
    }
  }
})

test('actual hook close and reopen with identical IDs cannot revive old ready or pending data', async () => {
  for (const previous of ['ready', 'pending-success', 'pending-failure']) {
    const view = controlledHook()
    try {
      view.update()
      await view.flush()
      if (previous === 'ready') {
        view.calls[0].response.resolve(contextFor(view.scopes[0]))
        await view.flush()
      }
      const original = view.props, oldScope = view.output.scope
      view.render({ ...original, missionId: null })
      assert.equal(view.output.scope, null)
      assert.equal(view.output.current, null)
      assert.equal(elements(view.tree, 'a').length, 0)
      view.commit()
      await view.flush()
      assert.equal(view.calls[0].init.signal.aborted, true)
      assert.equal(view.calls.length, 1)
      view.render(original)
      assert.equal(view.output.scope.key, oldScope.key)
      assert.notEqual(view.output.scope, oldScope)
      assert.equal(view.output.current, null)
      assert.equal(elements(view.tree, 'a').length, 0)
      view.commit()
      await view.flush()
      assert.equal(view.calls.length, 2)
      view.calls[1].response.reject(new Error('current denial'))
      await view.flush()
      const writes = view.writes
      if (previous === 'pending-success') view.calls[0].response.resolve(contextFor(oldScope))
      if (previous === 'pending-failure') view.calls[0].response.reject(new Error('late old failure'))
      await view.flush()
      assert.equal(view.writes, writes)
      assert.equal(view.result.state, 'unavailable')
      assert.equal(elements(view.tree, 'a').length, 0)
    } finally {
      view.unmount()
    }
  }
})

test('actual hook treats positive revision as one refresh hint and never retries failure on render', async () => {
  for (const failure of ['denial', 'timeout']) {
    const view = controlledHook()
    try {
      view.update()
      await view.flush()
      view.calls[0].response.resolve(contextFor(view.scopes[0]))
      await view.flush()
      const readyScope = view.output.scope, refresh = view.output.refresh
      for (let index = 0; index < 3; index++) {
        view.update({ ...view.props })
        await view.flush()
      }
      assert.equal(view.calls.length, 1)
      assert.equal(view.output.scope, readyScope)
      assert.equal(view.output.refresh, refresh)
      view.render({ ...view.props, revision: 'item-a:10:publication-a:5' })
      assert.equal(view.output.current, null)
      assert.equal(view.result.state, 'loading', 'snapshot hint cannot invent a result or absence')
      view.commit()
      await view.flush()
      assert.equal(view.calls.length, 2)
      if (failure === 'timeout') view.clock.expire()
      else view.calls[1].response.reject(new Error('current denied read'))
      await view.flush()
      assert.equal(view.result.state, 'unavailable', 'failure is not authoritative no-PR data')
      for (let index = 0; index < 3; index++) {
        view.update({ ...view.props })
        await view.flush()
      }
      assert.equal(view.calls.length, 2, 'same positive hint cannot trigger an automatic retry loop')
      assert.equal(view.clock.timers.size, 0)
      button(view.tree, /^Refresh result$/).props.onClick()
      view.update()
      await view.flush()
      assert.equal(view.calls.length, 3, 'only explicit refresh adds this request')
      assert.equal(view.calls[1].init.signal.aborted, true)
      view.calls[2].response.resolve(contextFor(view.scopes[2], { phase: null }))
      await view.flush()
      assert.equal(view.result.state, 'none', 'absence requires the actual authorized reader response')
      assert.equal(view.calls.length, 3)
    } finally {
      view.unmount()
    }
  }
})

// Keep App's actual MissionCard selection, hooks, dependent declarations and
// result JSX. Omit unrelated dossier sections, not result predicates or actions.
// This seam avoids mounting the office, networking globals or browser services.
async function compileAppResultSlice() {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8')
  const file = ts.createSourceFile('App.tsx', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
  assert.deepEqual(file.parseDiagnostics, [])
  const functions = new Map(file.statements
    .filter((node) => ts.isFunctionDeclaration(node) && node.name)
    .map((node) => [node.name.text, node]))
  const mission = functions.get('MissionCard')
  assert.ok(mission?.body, 'exercise the actual App MissionCard')
  const returned = mission.body.statements.find(ts.isReturnStatement)
  let root = returned?.expression
  while (root && ts.isParenthesizedExpression(root)) root = root.expression
  assert.ok(root && ts.isJsxElement(root))
  const surfaces = []
  const findSurface = (node) => {
    if (ts.isJsxElement(node) && node.openingElement.attributes.properties.some((attribute) =>
      ts.isJsxAttribute(attribute) && attribute.name.getText(file) === 'id' &&
      attribute.initializer && ts.isJsxExpression(attribute.initializer) &&
      attribute.initializer.expression && ts.isTemplateExpression(attribute.initializer.expression) &&
      attribute.initializer.expression.head.text === 'mission-result-')) {
      surfaces.push(node)
    }
    ts.forEachChild(node, findSurface)
  }
  findSurface(root)
  assert.equal(surfaces.length, 1, 'select the actual result navigation target, not copied JSX')
  const identifiers = (node) => {
    const names = new Set()
    const visit = (child) => {
      if (ts.isIdentifier(child)) names.add(child.text)
      ts.forEachChild(child, visit)
    }
    visit(node)
    return names
  }
  const boundNames = (name) => ts.isIdentifier(name) ? [name.text]
    : name.elements.flatMap((element) => ts.isBindingElement(element) ? boundNames(element.name) : [])
  const declarations = new Map()
  for (const statement of mission.body.statements) {
    if (!ts.isVariableStatement(statement)) continue
    for (const declaration of statement.declarationList.declarations) {
      for (const name of boundNames(declaration.name)) declarations.set(name, statement)
    }
  }
  const selectedStatements = new Set()
  const selectedFunctions = new Set()
  const required = new Set([...identifiers(surfaces[0]), ...identifiers(root.openingElement)])
  // Retain App's real guarded initial pin effect as well as the explicit selector.
  for (const statement of mission.body.statements) {
    if (ts.isExpressionStatement(statement) && ts.isCallExpression(statement.expression) &&
      ts.isIdentifier(statement.expression.expression) && statement.expression.expression.text === 'useEffect' &&
      identifiers(statement).has('setSelectedEvidenceRunId')) {
      selectedStatements.add(statement)
      for (const name of identifiers(statement)) required.add(name)
    }
  }
  for (const name of required) {
    const statement = declarations.get(name)
    const helper = functions.get(name)
    if (statement && !selectedStatements.has(statement)) {
      selectedStatements.add(statement)
      for (const dependency of identifiers(statement)) required.add(dependency)
    } else if (helper && helper !== mission && name !== 'api' && !selectedFunctions.has(helper)) {
      selectedFunctions.add(helper)
      for (const dependency of identifiers(helper.body)) required.add(dependency)
    }
  }
  for (const name of ['originRead', 'resultRead', 'evidenceRun', 'selectedEvidenceRunId']) {
    assert.ok(selectedStatements.has(declarations.get(name)), `actual ${name} wiring must remain in the seam`)
  }
  const narrowedRoot = ts.factory.updateJsxElement(root, root.openingElement, surfaces, root.closingElement)
  const narrowedMission = ts.factory.updateFunctionDeclaration(
    mission, mission.modifiers, mission.asteriskToken, mission.name, mission.typeParameters,
    mission.parameters, mission.type,
    ts.factory.updateBlock(mission.body, [
      ...mission.body.statements.filter((statement) => selectedStatements.has(statement)),
      ts.factory.updateReturnStatement(returned, narrowedRoot),
    ]),
  )
  const importedModules = new Set([
    'react', './workflowContext', './evidenceSelection', './useMissionOriginContext',
    './useMissionResultContext', './missionResultContext', './WorkResultCard', './PublishedResultCard',
    './factoryCheckpointRecovery',
  ])
  const imports = file.statements.filter((node) =>
    ts.isImportDeclaration(node) && importedModules.has(node.moduleSpecifier.text))
  const printer = ts.createPrinter()
  const extracted = [
    ...imports, ...selectedFunctions, narrowedMission,
  ].map((node) => printer.printNode(ts.EmitHint.Unspecified, node, file)).join('\n')
  const compiled = ts.transpileModule(
    `const { API_URL, api, window, document } = require('app-test-environment');\n${extracted}\nexports.AppMissionResult = MissionCard;`,
    {
      fileName: 'AppResultSlice.tsx', reportDiagnostics: true,
      compilerOptions: {
        target: ts.ScriptTarget.ES2023, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX,
      },
    },
  )
  assert.deepEqual(compiled.diagnostics, [])
  return compiled.outputText
}

const appResultSource = await compileAppResultSlice()
const originHookSource = await compile('useMissionOriginContext.ts')
const appApiUrl = 'http://ecorp-fixture.invalid'
const deliveredRunId = '00000000-0000-4000-8000-000000000145'
const olderRunId = '00000000-0000-4000-8000-000000000144'
const newerRunId = '00000000-0000-4000-8000-000000000146'

function appContext() {
  const value = contextFor()
  value.publication.run_id = deliveredRunId
  value.publication.provenance.run_ids = [deliveredRunId]
  value.source_deliverables[0].run_id = deliveredRunId
  return value
}

function appOrigin(kind = 'factory') {
  return {
    corp_id: ids.corpId, actor_id: ids.actorId, mission_id: ids.missionId, room_id: ids.roomId,
    origin: kind === 'direct' ? { kind } : {
      kind, work_item_id: ids.workItemId, source_repository: ids.sourceRepository,
      source_issue_number: 71, source_issue_url: 'https://github.com/owner/repo/issues/71',
    },
  }
}

function appResultFixture({ pinnedRunId = deliveredRunId, ...overrides } = {}) {
  const slots = [], effects = [], calls = [], storageWrites = [], actions = [], clock = fakeClock()
  const animationFrames = [], focusCalls = []
  let cursor = 0, dirty = false, tree
  const different = (before, after) =>
    !before || before.length !== after.length || after.some((value, index) => !Object.is(value, before[index]))
  const react = {
    useMemo(create, deps) {
      const index = cursor++
      if (different(slots[index]?.deps, deps)) slots[index] = { deps, value: create() }
      return slots[index].value
    },
    useCallback(callback, deps) { return react.useMemo(() => callback, deps) },
    useState(initial) {
      const index = cursor++
      if (!slots[index]) {
        const slot = { value: typeof initial === 'function' ? initial() : initial }
        slot.set = (next) => {
          slot.value = typeof next === 'function' ? next(slot.value) : next
          dirty = true
        }
        slots[index] = slot
      }
      return [slots[index].value, slots[index].set]
    },
    useEffect(create, deps) {
      const index = cursor++
      if (different(slots[index]?.deps, deps)) {
        const effect = { deps, create, cleanup: slots[index]?.cleanup }
        slots[index] = effect
        effects.push(effect)
      }
    },
  }
  const api = (path, init) => {
    assert.equal(init.method, 'GET', 'the App result slice may only read through the controlled API')
    const response = deferred()
    calls.push({ path, init, response })
    return response.promise
  }
  const origin = evaluate(originHookSource, {
    react,
    './missionOriginContext': {
      ...originReader,
      startMissionOriginRead(selected, get, publish) {
        return originReader.startMissionOriginRead(selected, get, publish, clock)
      },
    },
  })
  const result = evaluate(hookSource, {
    react,
    './missionResultContext': {
      ...reader,
      startMissionResultRead(selected, get, publish) {
        return reader.startMissionResultRead(selected, get, publish, clock)
      },
    },
  })
  const storageKey = evidenceSelection.evidenceSelectionKey({
    server: appApiUrl, corpId: ids.corpId, actorId: ids.actorId, missionId: ids.missionId,
  })
  const storage = new Map([[storageKey, pinnedRunId]])
  const { AppMissionResult } = evaluate(appResultSource, {
    react, 'react/jsx-runtime': jsxRuntime,
    './workflowContext': workflow, './evidenceSelection': evidenceSelection,
    './factoryCheckpointRecovery': checkpointRecovery,
    './useMissionOriginContext': origin, './useMissionResultContext': result,
    './missionResultContext': reader, './WorkResultCard': { WorkResultCard },
    './PublishedResultCard': { PublishedResultCard },
    'app-test-environment': {
      API_URL: appApiUrl, api,
      window: {
        sessionStorage: {
          getItem: (key) => storage.get(key) ?? null,
          setItem(key, value) { storageWrites.push({ key, value }); storage.set(key, value) },
        },
        requestAnimationFrame(callback) { animationFrames.push(callback); return animationFrames.length },
      },
      document: {
        getElementById(id) {
          const target = elements(tree, 'div').find((node) => node.props.id === id)
          assert.ok(target, 'focus must target a currently rendered App result surface')
          assert.equal(target.props.tabIndex, -1)
          return {
            focus(options) { focusCalls.push({ operation: 'focus', id, options }) },
            scrollIntoView(options) { focusCalls.push({ operation: 'scroll', id, options }) },
          }
        },
      },
    },
  })
  const run = (id, status = 'completed') => ({
    id, task_id: 'task-item-a', status, execution_mode: 'provider',
    input_tokens: 0, output_tokens: 0, cost_microusd: 0,
    verification_sha256: verification, deliverable_sha256: digest,
  })
  let props = {
    corpId: ids.corpId, actorId: ids.actorId, actorRole: 'member', busy: false,
    mission: {
      id: ids.missionId, room_id: ids.roomId, title: 'Retained application', status: 'completed',
      budget_tokens: 1000, budget_cost_microusd: 1000,
    },
    tasks: [{ id: 'task-item-a', mission_id: ids.missionId, plan_key: 'deliver', depth: 0, status: 'completed' }],
    runs: [run(newerRunId, 'failed'), run(deliveredRunId), run(olderRunId)],
    agents: [], actors: [], evidence: [], revisions: [], contractRevisions: [],
    verificationRequests: [], actionApprovals: [], factoryRecoveries: [], events: [],
    deliverables: appContext().source_deliverables,
    factoryItem: { ...appContext().work_item, state: 'published' },
    origin: { kind: 'unknown', label: 'Mission origin unavailable', detail: 'Context must be read.' },
    onLaunch: () => { actions.push('launch') },
    onDownloadDeliverable: () => { actions.push('download') },
    onVerificationDecision: () => { actions.push('decide') },
    onViewAgents: () => { actions.push('agents') },
    onDiscuss: () => { actions.push('discuss') },
    ...overrides,
  }
  const render = (next = props) => {
    props = next
    cursor = 0
    dirty = false
    tree = AppMissionResult(props)
    return renderToStaticMarkup(tree)
  }
  const commit = () => {
    for (let pass = 0; ; pass++) {
      assert.ok(pass < 10, 'actual App hook dependencies must settle')
      for (const effect of effects.splice(0)) {
        effect.cleanup?.()
        effect.cleanup = effect.create()
      }
      if (!dirty) return
      render()
    }
  }
  const update = (next = props) => { render(next); commit() }
  return {
    calls, actions, storageWrites, animationFrames, focusCalls, render, update,
    get props() { return props },
    get tree() { return tree },
    get pinned() { return storage.get(storageKey) },
    async flush() { await tick(); update() },
    flushAnimationFrames() { animationFrames.splice(0).forEach((callback) => callback()) },
    unmount() {
      slots.forEach((slot) => slot.cleanup?.())
      effects.length = 0
      animationFrames.length = 0
      assert.equal(clock.timers.size, 0)
    },
  }
}

function assertPendingOriginCard(tree) {
  assert.equal(elements(tree, 'section').filter((node) => node.props['aria-busy'] === true).length, 1)
  assert.match(text(tree), /\bLoading\b/)
  assert.doesNotMatch(text(tree), /The mission is complete|No pull request is recorded|No decision recorded/)
  assert.equal(elements(tree, 'a').length, 0)
  assert.equal(elements(tree, 'button').length, 0)
}

function assertResultRefocus(view) {
  assert.equal(view.animationFrames.length, 1, 'manual refresh queues exactly one post-render focus')
  assert.deepEqual(view.focusCalls, [], 'the callback must not focus a removed button synchronously')
  view.flushAnimationFrames()
  assert.deepEqual(view.focusCalls, [
    { operation: 'focus', id: `mission-result-${ids.missionId}`, options: { preventScroll: true } },
    { operation: 'scroll', id: `mission-result-${ids.missionId}`, options: { block: 'start', behavior: 'auto' } },
  ])
  assert.equal(view.animationFrames.length, 0)
}

test('actual App origin-unavailable branch offers manual Refresh work context with no PR CTA', async () => {
  const view = appResultFixture()
  try {
    view.update()
    await view.flush()
    assert.equal(view.calls.length, 1)
    assertPendingOriginCard(view.tree)
    assert.deepEqual(view.focusCalls, [])
    assert.equal(view.animationFrames.length, 0)
    const originPath = view.calls[0].path
    assert.match(originPath, /\/missions\/mission-a\/context\?actor_id=alice$/)
    view.calls[0].response.reject(new Error('origin unavailable'))
    await view.flush()
    assert.match(text(view.tree), /Work context is unavailable/)
    assert.equal(elements(view.tree, 'a').length, 0)
    assert.equal(elements(view.tree, 'button').length, 1)
    assert.equal(view.tree.props['data-run-id'], deliveredRunId)
    assert.equal(view.pinned, deliveredRunId)
    assert.deepEqual(view.actions, [])
    assert.deepEqual(view.storageWrites, [])
    assert.equal(view.calls.length, 1, 'first-hop failure must not retry itself')
    assert.equal(view.animationFrames.length, 0, 'failure alone must not move keyboard focus')
    button(view.tree, /^Refresh work context$/).props.onClick()
    await view.flush()
    assert.equal(view.calls.length, 2)
    assert.equal(view.calls[1].path, originPath, 'App must refresh origin, not the disabled second-hop reader')
    assert.equal(view.calls[0].init.signal.aborted, true)
    assertPendingOriginCard(view.tree)
    assertResultRefocus(view)
    assert.equal(view.tree.props['data-run-id'], deliveredRunId)
    assert.deepEqual(view.actions, [])
    assert.deepEqual(view.storageWrites, [])
  } finally {
    view.unmount()
  }
})

test('actual App first-hop retry exposes the authorized result without moving pinned evidence', async () => {
  for (const pinnedRunId of [deliveredRunId, olderRunId]) {
    const view = appResultFixture({ pinnedRunId })
    try {
      view.update()
      await view.flush()
      view.calls[0].response.reject(new Error('temporary origin failure'))
      await view.flush()
      button(view.tree, /^Refresh work context$/).props.onClick()
      await view.flush()
      assertPendingOriginCard(view.tree)
      assertResultRefocus(view)
      view.calls[1].response.resolve(appOrigin())
      await view.flush()
      assert.equal(view.calls.length, 3)
      assert.match(view.calls[2].path, /\/factory\/work-items\/item-a\/publication-context\?actor_id=alice$/)
      view.calls[2].response.resolve(appContext())
      await view.flush()
      assert.equal(view.tree.props['data-run-id'], pinnedRunId)
      assert.equal(view.pinned, pinnedRunId)
      assert.deepEqual(view.storageWrites, [])
      assert.deepEqual(view.actions, [])
      if (pinnedRunId === deliveredRunId) {
        assert.equal(elements(view.tree, 'a').length, 1)
        assert.equal(elements(view.tree, 'a')[0].props.href, appContext().publication.pull_request_url)
        assert.match(text(view.tree), /\bPublished\b/)
      } else {
        assert.equal(elements(view.tree, 'a').length, 0, 'a retry is not permission to show another review target')
        button(view.tree, /^Go to delivered result$/)
        assert.match(text(view.tree), /Historical evidence/)
      }
      await view.flush()
      assert.equal(view.calls.length, 3, 'successful discovery must not create a retry loop')
      assert.equal(view.animationFrames.length, 0, 'result arrival must not schedule another focus or selection')
      assert.deepEqual(view.storageWrites, [])
    } finally {
      view.unmount()
    }
  }
})

test('actual App fallback reports unavailable review details for an unresolved run or request', async () => {
  for (const missing of ['run', 'request']) {
    const view = appResultFixture()
    try {
      const runs = missing === 'run'
        ? view.props.runs.filter((run) => run.id !== deliveredRunId)
        : view.props.runs.map((run) => run.id === deliveredRunId ? { ...run, status: 'waiting_for_approval' } : run)
      view.update({
        ...view.props, runs, mission: { ...view.props.mission, status: 'running' },
        verificationRequests: [{ run_id: newerRunId, task_id: 'task-item-a', status: 'approved' }],
      })
      await view.flush()
      view.calls[0].response.resolve(appOrigin('direct'))
      await view.flush()
      assert.equal(view.calls.length, 1, 'direct origin must not request an unrelated Factory result')
      assert.ok(elements(view.tree, 'dt').some((node) => text(node) === 'Outcome review'))
      assert.ok(elements(view.tree, 'dd').some((node) => text(node) === 'Review details unavailable'))
      assert.doesNotMatch(text(view.tree), /No decision recorded|No review required|No approval required/)
      assert.equal(view.tree.props['data-run-id'], missing === 'run' ? undefined : deliveredRunId)
      assert.equal(view.pinned, deliveredRunId)
      assert.equal(elements(view.tree, 'a').length, 0)
      assert.deepEqual(view.actions, [])
      assert.deepEqual(view.storageWrites, [])
    } finally {
      view.unmount()
    }
  }
})

async function actualMissionQueue() {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8')
  const file = ts.createSourceFile('App.tsx', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
  const declarations = []
  const pickers = []
  const visit = (node) => {
    if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.name.text === 'missionChoices') {
      declarations.push(node)
    }
    if (ts.isJsxElement(node) && node.openingElement.attributes.properties.some((attribute) =>
      ts.isJsxAttribute(attribute) && attribute.initializer && ts.isStringLiteral(attribute.initializer) &&
      ((attribute.name.getText(file) === 'id' && attribute.initializer.text === 'mission-work-switch') ||
        (attribute.name.getText(file) === 'className' && attribute.initializer.text === 'mission-selector')))) {
      pickers.push(node)
    }
    ts.forEachChild(node, visit)
  }
  visit(file)
  assert.equal(declarations.length, 1)
  assert.equal(pickers.length, 2, 'compact and desktop pickers must both use the bounded choices')
  for (const picker of pickers) {
    assert.match(picker.getText(file), /missionChoices\.map\(/)
    assert.doesNotMatch(picker.getText(file), /latestMissions\.map\(|rememberEvidenceRun|setSelectedEvidenceRunId/)
  }
  const compiled = ts.transpileModule(
    `exports.choices = (latestMissions, selectedMission) => (${declarations[0].initializer.getText(file)});`,
    { compilerOptions: { target: ts.ScriptTarget.ES2023, module: ts.ModuleKind.CommonJS } },
  )
  return evaluate(compiled.outputText, {}).choices
}

test('actual work pickers retain a deep-linked historical mission outside the recent eight', async () => {
  const choices = await actualMissionQueue()
  const latest = Object.freeze(Array.from({ length: 8 }, (_, index) =>
    Object.freeze({ id: `recent-${index}`, title: `Recent ${index}` })))
  const selected = Object.freeze({ id: 'historical-deep-link', title: 'Older selected work' })
  const result = choices(latest, selected)
  assert.equal(result.length, 9)
  assert.equal(result[0], selected)
  assert.deepEqual(result.slice(1), latest)
  assert.equal(result.filter((mission) => mission.id === selected.id).length, 1)
  assert.equal(latest.length, 8, 'visible-choice construction must not mutate the recent queue')
})

test('actual work pickers do not duplicate a recent selection or invent an absent mission', async () => {
  const choices = await actualMissionQueue()
  const latest = Object.freeze([{ id: 'recent-a' }, { id: 'recent-b' }].map(Object.freeze))
  assert.equal(choices(latest, latest[1]), latest)
  assert.equal(choices(latest, { id: 'recent-a' }), latest, 'identity matches by stored mission ID')
  assert.equal(choices(latest, undefined), latest)
  const empty = Object.freeze([])
  assert.equal(choices(empty, undefined), empty)
})
