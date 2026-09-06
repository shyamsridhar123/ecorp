import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { basename, dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'

const script = fileURLToPath(new URL('./fake_github_cli.mjs', import.meta.url))
const now = '2026-09-06T12:00:00.000Z'
const resetAt = '2026-09-06T13:00:00.000Z'
const token = 'fake-test-credential-must-not-appear-in-output'
const rate = 'rateLimit { limit remaining cost resetAt }'
const probe = `query FixtureQuota { ${rate} }`
const resolver = `query($owner:String!,$number:Int!){
  repositoryOwner(login:$owner){
    ... on User{projectV2(number:$number){id number owner{... on User{login} ... on Organization{login}}}}
    ... on Organization{projectV2(number:$number){id number owner{... on User{login} ... on Organization{login}}}}
  } ${rate}
}`
const pageQuery = `query($id:ID!,$cursor:String){
  node(id:$id){... on ProjectV2{
    id number owner{... on User{login} ... on Organization{login}}
    items(first:100,after:$cursor,archivedStates:[NOT_ARCHIVED]){
      totalCount pageInfo{endCursor hasNextPage} nodes{
        id isArchived fieldValueByName(name:"Status"){... on ProjectV2ItemFieldSingleSelectValue{name}}
        content{__typename ... on Issue{number title body url repository{nameWithOwner}}}
      }
    }
  }} ${rate}
}`
const exactQuery = `query($id:ID!){node(id:$id){__typename ... on ProjectV2Item{
  id isArchived project{id number owner{... on User{login} ... on Organization{login}}}
  fieldValueByName(name:"Status"){... on ProjectV2ItemFieldSingleSelectValue{
    name optionId field{... on ProjectV2SingleSelectField{id name}}
  }}
  content{__typename ... on Issue{id number title body url repository{nameWithOwner}}}
}}}`
const fieldQuery = `query($id:ID!){node(id:$id){... on ProjectV2{
  field(name:"Status"){... on ProjectV2SingleSelectField{id name options{id name}}}
}}}`
const membershipsQuery = `query($owner:String!,$repo:String!,$number:Int!){
  repository(owner:$owner,name:$repo){issue(number:$number){
    projectItems(first:100){pageInfo{hasNextPage} nodes{
      id project{number owner{... on User{login} ... on Organization{login}}}
    }}
  }}
}`
const withRate = (query) => query.replace(/\}\s*$/, `${rate}}`)

function item(number) {
  return {
    id: `opaque-item:/fixture-${number}+==`,
    status: 'Todo',
    content: {
      id: `issue-${number}`,
      type: 'Issue',
      number,
      title: `  Exact title ${number} `,
      body: 'Source\r\nRésumé 🚀\n```js\nconst exact = true\n```\n',
      url: `https://github.com/Example/Repo/issues/${number}`,
      repository: 'Example/Repo',
    },
  }
}

function initialState() {
  const source = item(1)
  return {
    repository: 'Example/Repo',
    project: {
      id: 'opaque-project:/fixture+==', number: 7, owner: 'Example',
      status_field_id: 'opaque-status-field',
      status_options: [{ id: 'todo', name: 'Todo' }, { id: 'progress', name: 'In Progress' }],
    },
    items: [source],
    issues: {
      1: {
        ...source.content, state: 'OPEN', updatedAt: now,
        labels: [{ name: 'factory:ready' }, { name: 'keep' }],
      },
    },
    graphql_now: now,
    graphql_quota: { limit: 10000, remaining: 10000, cost_per_query: 2, reset_at: resetAt },
  }
}

function fixture(t, modify = () => {}) {
  const directory = mkdtempSync(join(tmpdir(), 'ecorp-fake-github-quota-'))
  const statePath = join(directory, 'state.json')
  t.after(() => {
    // Only remove this test's verified, unique directory, never a computed parent.
    assert.equal(dirname(resolve(directory)), resolve(tmpdir()))
    assert.ok(basename(directory).startsWith('ecorp-fake-github-quota-'))
    rmSync(directory, { recursive: true, force: true })
  })
  const state = initialState()
  modify(state)
  function write(value) {
    writeFileSync(statePath, `${JSON.stringify(value)}\n`)
  }
  write(state)
  function read() {
    return JSON.parse(readFileSync(statePath, 'utf8'))
  }
  function update(change) {
    const current = read()
    change(current)
    write(current)
  }
  function exec(args, { status = 0, env = {} } = {}) {
    const result = spawnSync(process.execPath, [script, ...args], {
      encoding: 'utf8', timeout: 10_000, maxBuffer: 20 * 1024 * 1024, windowsHide: true,
      env: {
        ...process.env,
        ECORP_FAKE_GITHUB_STATE: statePath,
        ECORP_FAKE_GITHUB_EXPECT_TOKEN: token,
        GH_TOKEN: token, GITHUB_TOKEN: token,
        ...env,
      },
    })
    assert.ifError(result.error)
    assert.equal(result.status, status, result.stderr)
    return result
  }
  function graphql(query, variables = {}, { include = false, status = 0, env = {} } = {}) {
    const args = include === 'before' ? ['api', '--include', 'graphql'] : ['api', 'graphql']
    if (include && include !== 'before') args.push(include === 'short' ? '-i' : '--include')
    args.push('-f', `query=${query}`)
    for (const [name, value] of Object.entries(variables)) {
      args.push(typeof value === 'string' ? '-f' : '-F', `${name}=${value}`)
    }
    const result = exec(args, { status, env })
    const boundary = result.stdout.indexOf('\r\n\r\n')
    const headers = {}
    if (boundary >= 0) {
      const lines = result.stdout.slice(0, boundary).split('\r\n')
      headers.status = Number(lines.shift().match(/^HTTP\/1\.1 (\d{3}) /)[1])
      for (const line of lines) {
        const separator = line.indexOf(':')
        headers[line.slice(0, separator).toLowerCase()] = line.slice(separator + 1).trim()
      }
    }
    return {
      ...result, headers,
      json: JSON.parse(boundary >= 0 ? result.stdout.slice(boundary + 4) : result.stdout),
    }
  }
  return { directory, read, update, exec, graphql }
}

test('User/Organization resolver verifies owner and project number without reading items', (t) => {
  for (const ownerType of ['User', 'Organization']) {
    const f = fixture(t, (state) => { state.project.owner_type = ownerType })
    const result = f.graphql(resolver, { owner: 'example', number: 7 })
    assert.equal(result.json.data.repositoryOwner.__typename, ownerType)
    const project = result.json.data.repositoryOwner.projectV2
    assert.equal(project.id, f.read().project.id)
    assert.equal(project.number, 7)
    assert.deepEqual(project.owner, { __typename: ownerType, login: 'Example' })
    assert.equal(result.json.data.rateLimit.remaining, 9998)
    assert.equal(f.graphql(resolver, { owner: 'other', number: 7 }).json.data.repositoryOwner, null)
    assert.equal(f.graphql(resolver, { owner: 'Example', number: 8 }).json.data.repositoryOwner.projectV2, null)
    assert.equal(f.read().project_item_read_calls ?? 0, 0)
    assert.equal(f.read().graphql_query_counts.project_resolve, 3)
  }
})

test('opaque paginated active-only discovery retains 1002 items and exact Issue source', (t) => {
  const f = fixture(t, (state) => {
    state.items = Array.from({ length: 1004 }, (_, index) => item(index + 1))
    state.items[998].content = { type: 'DraftIssue', title: 'not a candidate' }
    state.items[999].content = { type: 'PullRequest', number: 999, repository: 'Example/Repo' }
    state.items[1000].content = null
    state.items[1002].isArchived = true
    state.items[1003].isArchived = true
  })
  const id = f.graphql(resolver, { owner: 'Example', number: 7 }).json.data.repositoryOwner.projectV2.id
  const nodes = []
  const seen = new Set()
  let cursor = null
  let pages = 0
  do {
    const response = f.graphql(pageQuery, { id, cursor }, { include: true })
    const project = response.json.data.node
    const connection = project.items
    pages += 1
    assert.equal(project.id, id)
    assert.equal(connection.totalCount, 1002)
    assert.ok(connection.nodes.length > 0 && connection.nodes.length <= 100)
    assert.ok(connection.nodes.every((node) => node.isArchived === false))
    assert.equal(response.headers['x-ratelimit-remaining'], String(10000 - (pages + 1) * 2))
    assert.equal(response.json.data.rateLimit.remaining, Number(response.headers['x-ratelimit-remaining']))
    assert.equal(response.json.data.rateLimit.cost, 2)
    assert.equal(response.json.data.rateLimit.resetAt, resetAt)
    cursor = connection.pageInfo.endCursor
    assert.equal(typeof cursor, 'string')
    assert.ok(!/^\d+$/.test(cursor))
    assert.ok(!seen.has(cursor))
    seen.add(cursor)
    nodes.push(...connection.nodes)
    if (!connection.pageInfo.hasNextPage) break
    assert.ok(pages < 100)
  } while (true)
  assert.equal(pages, 11)
  assert.equal(nodes.length, 1002)
  assert.equal(new Set(nodes.map((node) => node.id)).size, 1002)
  const expected = item(1002).content
  assert.deepEqual(nodes.at(-1).content, {
    __typename: 'Issue', number: expected.number, title: expected.title, body: expected.body,
    url: expected.url, repository: { nameWithOwner: expected.repository },
  })
  assert.deepEqual(nodes[998].content, { __typename: 'DraftIssue' })
  assert.deepEqual(nodes[999].content, { __typename: 'PullRequest' })
  assert.equal(nodes[1000].content, null)
  const state = f.read()
  assert.equal(state.project_items_page_calls, 11)
  assert.equal(state.project_item_read_calls, 11)
  assert.equal(state.item_list_calls ?? 0, 0)
  assert.equal(state.project_field_lookup_calls ?? 0, 0)
  assert.equal(state.graphql_calls, 12)
  assert.equal(state.graphql_event_count, 12)
  assert.equal(state.graphql_query_counts.project_items, 11)
  assert.ok(state.graphql_events.every((event) => event.at === now && event.status === 200))
  const unfiltered = f.graphql(pageQuery.replace(',archivedStates:[NOT_ARCHIVED]', ''), { id, cursor: null })
  assert.equal(unfiltered.json.data.node.items.totalCount, 1004)
})

test('empty pages, invalid cursors and archived cursors never silently restart pagination', (t) => {
  const f = fixture(t)
  const id = f.read().project.id
  const first = f.graphql(pageQuery, { id }).json.data.node.items
  assert.equal(first.totalCount, 1)
  assert.equal(first.pageInfo.hasNextPage, false)
  const end = f.graphql(pageQuery, { id, cursor: first.pageInfo.endCursor }).json.data.node.items
  assert.deepEqual(end, { totalCount: 1, nodes: [], pageInfo: { endCursor: null, hasNextPage: false } })
  for (const cursor of ['0', '', 'made-up-cursor']) {
    const result = f.graphql(pageQuery, { id, cursor }, { include: true, status: 1 })
    assert.equal(result.headers.status, 400)
  }
  f.update((state) => { state.items[0].isArchived = true })
  const empty = f.graphql(pageQuery, { id, cursor: null }).json.data.node.items
  assert.deepEqual(empty, { totalCount: 0, nodes: [], pageInfo: { endCursor: null, hasNextPage: false } })
  assert.equal(f.graphql(pageQuery, { id, cursor: first.pageInfo.endCursor }, { status: 1 }).json.errors[0].type, 'BAD_USER_INPUT')
  assert.equal(f.graphql(pageQuery, { id: 'other-project' }).json.data.node, null)
})

test('quota-only probes accept include flags before/after endpoint and retain plain JSON', (t) => {
  const f = fixture(t, (state) => {
    state.graphql_quota = { limit: 10, remaining: 10, cost_per_query: 3, reset_at: resetAt }
  })
  for (const [include, remaining] of [['before', 7], ['short', 4], [false, 1]]) {
    const result = f.graphql(probe, {}, { include })
    assert.deepEqual(result.json.data.rateLimit, { limit: 10, remaining, cost: 3, resetAt })
    if (include) {
      assert.equal(result.headers.status, 200)
      assert.equal(result.headers['x-ratelimit-limit'], '10')
      assert.equal(result.headers['x-ratelimit-used'], String(10 - remaining))
      assert.equal(result.headers['x-ratelimit-reset'], String(Date.parse(resetAt) / 1000))
      assert.equal(result.headers['x-ratelimit-resource'], 'graphql')
      assert.equal(result.headers.date, new Date(now).toUTCString())
    } else {
      assert.ok(result.stdout.startsWith('{"data":'))
    }
  }
  assert.deepEqual(f.read().graphql_query_counts, { quota: 3 })
  assert.equal(f.read().project_item_read_calls ?? 0, 0)
  const exhausted = f.graphql(probe, {}, { include: true, status: 1 })
  assert.equal(exhausted.headers.status, 403)
  assert.equal(exhausted.headers['x-ratelimit-remaining'], '0')
})

test('exact items, memberships and publisher fields support metered and legacy plain callers', (t) => {
  const f = fixture(t)
  for (const metered of [false, true]) {
    const query = (value) => metered ? withRate(value) : value
    const exact = f.graphql(query(exactQuery), { id: item(1).id })
    const node = exact.json.data.node
    assert.equal(node.__typename, 'ProjectV2Item')
    assert.equal(node.content.id, 'issue-1')
    assert.equal(node.content.body, item(1).content.body)
    assert.deepEqual(node.content.repository, { nameWithOwner: 'Example/Repo' })
    assert.equal(node.project.id, f.read().project.id)
    assert.deepEqual(node.fieldValueByName, {
      name: 'Todo', __typename: 'ProjectV2ItemFieldSingleSelectValue', optionId: 'todo',
      field: { id: 'opaque-status-field', name: 'Status' },
    })
    const membership = f.graphql(query(membershipsQuery), { owner: 'example', repo: 'repo', number: 1 })
    assert.equal(membership.json.data.repository.issue.projectItems.nodes[0].id, item(1).id)
    assert.equal(membership.json.data.repository.issue.projectItems.pageInfo.hasNextPage, false)
    const field = f.graphql(query(fieldQuery), { id: f.read().project.id })
    assert.equal(field.json.data.node.field.id, 'opaque-status-field')
    assert.deepEqual(field.json.data.node.field.options, f.read().project.status_options)
    for (const result of [exact, membership, field]) {
      assert.equal(Object.hasOwn(result.json.data, 'rateLimit'), metered)
      assert.ok(!result.stdout.startsWith('HTTP/'))
      if (metered) {
        assert.equal(result.json.data.rateLimit.cost, 2)
        assert.equal(result.json.data.rateLimit.resetAt, resetAt)
      }
    }
  }
  const state = f.read()
  assert.equal(state.graphql_quota.remaining, 9988)
  assert.equal(state.project_item_lookup_calls, 2)
  assert.equal(state.project_item_read_calls, 2)
  assert.equal(state.project_field_lookup_calls, 2)
  assert.deepEqual(state.graphql_query_counts, { item_exact: 2, issue_memberships: 2, project_field: 2 })
  assert.equal(f.graphql(withRate(exactQuery), { id: 'missing-item' }).json.data.node, null)
  assert.equal(f.graphql(withRate(membershipsQuery), { owner: 'Example', repo: 'Repo', number: 999 }).json.data.repository.issue, null)
})

test('configured primary refusal exposes reset headers and recovers exactly at fixture reset', (t) => {
  const soon = '2026-09-06T12:00:10.000Z'
  const f = fixture(t, (state) => {
    state.graphql_quota = { limit: 50, remaining: 50, cost_per_query: 4, reset_at: soon }
    state.graphql_failures = [{ match: 'quota', calls: [1], kind: 'primary', retry_after: 5 }]
  })
  const failed = f.graphql(probe, {}, { include: true, status: 1 })
  assert.equal(failed.headers.status, 403)
  assert.equal(failed.headers['retry-after'], '5')
  assert.equal(failed.headers['x-ratelimit-remaining'], '0')
  assert.equal(failed.headers['x-ratelimit-reset'], String(Date.parse(soon) / 1000))
  assert.equal(failed.json.errors[0].type, 'RATE_LIMITED')
  assert.match(failed.stderr, /API rate limit exceeded/)
  f.update((state) => { state.graphql_now = '2026-09-06T12:00:09.000Z' })
  assert.equal(f.graphql(probe, {}, { include: true, status: 1 }).headers['retry-after'], '1')
  f.update((state) => { state.graphql_now = soon })
  const recovered = f.graphql(probe, {}, { include: true })
  assert.deepEqual(recovered.json.data.rateLimit, {
    limit: 50, remaining: 46, cost: 4, resetAt: '2026-09-06T13:00:10.000Z',
  })
  assert.equal(recovered.headers['x-ratelimit-remaining'], '46')
  assert.equal(f.read().graphql_resets, 1)
  assert.deepEqual(f.read().graphql_failure_counts, { primary: 2 })
  assert.deepEqual(f.read().graphql_events.map((event) => [event.call, event.status, event.cost]), [
    [1, 403, 0], [2, 403, 0], [3, 200, 4],
  ])
})

test('secondary failures are finite per-query attempts even with interleaved probes', (t) => {
  const f = fixture(t, (state) => {
    state.graphql_failures = [{ match: 'project_resolve', calls: [1, 3], kind: 'secondary', retry_after: 2 }]
  })
  const variables = { owner: 'Example', number: 7 }
  const first = f.graphql(resolver, variables, { include: true, status: 1 })
  assert.equal(first.headers.status, 403)
  assert.equal(first.headers['retry-after'], '2')
  assert.equal(first.headers['x-ratelimit-remaining'], '10000')
  assert.equal(first.headers['x-ratelimit-reset'], String(Date.parse(resetAt) / 1000))
  assert.match(first.stderr, /secondary rate limit/)
  f.graphql(probe)
  assert.equal(f.graphql(resolver, variables).json.data.rateLimit.remaining, 9996)
  assert.equal(f.graphql(resolver, variables, { include: true, status: 1 }).headers['x-ratelimit-remaining'], '9996')
  assert.equal(f.graphql(resolver, variables).json.data.rateLimit.remaining, 9994)
  const state = f.read()
  assert.equal(state.graphql_calls, 5)
  assert.deepEqual(state.graphql_query_counts, { project_resolve: 4, quota: 1 })
  assert.deepEqual(state.graphql_failure_counts, { secondary: 2 })
  assert.deepEqual(state.graphql_events.map((event) => event.kind_call), [1, 1, 2, 3, 4])
  assert.equal(state.project_item_read_calls ?? 0, 0)
})

test('503 failures expose Retry-After without spending quota or triggering source mutation', (t) => {
  const f = fixture(t, (state) => {
    state.graphql_failures = [{ match: 'project_items', calls: [1], kind: '503', retry_after: 3 }]
    state.item_list_mutation = { call: 1, issue_number: 1, patch: { body: 'changed source' } }
  })
  const variables = { id: f.read().project.id, cursor: null }
  const unavailable = f.graphql(pageQuery, variables, { include: true, status: 1 })
  assert.equal(unavailable.headers.status, 503)
  assert.equal(unavailable.headers['retry-after'], '3')
  assert.equal(unavailable.headers['x-ratelimit-remaining'], '10000')
  assert.equal(unavailable.json.errors[0].type, 'SERVICE_UNAVAILABLE')
  assert.equal(f.read().project_item_read_calls ?? 0, 0)
  assert.equal(f.read().issues[1].body, item(1).content.body)
  f.update((state) => { state.graphql_now = '2026-09-06T12:00:03.000Z' })
  const recovered = f.graphql(pageQuery, variables, { include: true })
  assert.equal(recovered.json.data.node.items.nodes[0].content.body, item(1).content.body)
  assert.equal(f.read().issues[1].body, 'changed source')
  assert.equal(f.read().item_list_mutations_applied, 1)
  assert.equal(f.read().graphql_query_counts.project_items, 2)
  assert.deepEqual(f.read().graphql_events.map((event) => event.at), [now, '2026-09-06T12:00:03.000Z'])
})

test('source-drift hook remains shared across pages, legacy lists and exact reads only', (t) => {
  const f = fixture(t, (state) => {
    state.item_list_mutation = {
      call: 3, issue_number: 1, patch: { body: 'new issue source', updatedAt: resetAt }, remove_label: 'factory:ready',
    }
  })
  const id = f.read().project.id
  f.graphql(probe)
  f.graphql(resolver, { owner: 'Example', number: 7 })
  f.graphql(fieldQuery, { id })
  f.graphql(membershipsQuery, { owner: 'Example', repo: 'Repo', number: 1 })
  assert.equal(f.read().project_item_read_calls ?? 0, 0)
  f.graphql(pageQuery, { id })
  const legacy = JSON.parse(f.exec(['project', 'item-list', '7', '--owner', 'Example', '--limit', '1000']).stdout)
  assert.equal(legacy.items[0].content.body, item(1).content.body)
  assert.equal(f.read().item_list_mutations_applied ?? 0, 0)
  const exact = f.graphql(exactQuery, { id: item(1).id })
  assert.equal(exact.json.data.node.content.body, item(1).content.body)
  const issue = JSON.parse(f.exec(['issue', 'view', '1', '--repo', 'Example/Repo']).stdout)
  assert.equal(issue.body, 'new issue source')
  assert.equal(issue.updatedAt, resetAt)
  assert.deepEqual(issue.labels, [{ name: 'keep' }])
  const state = f.read()
  assert.equal(state.project_item_read_calls, 3)
  assert.equal(state.item_list_calls, 1)
  assert.equal(state.project_item_lookup_calls, 1)
  assert.equal(state.project_items_page_calls, 1)
  assert.equal(state.item_list_mutations_applied, 1)
  assert.equal(state.item_list_mutation, null)
})

test('legacy Project edits and publisher PR mutation/recovery semantics are preserved', (t) => {
  const f = fixture(t, (state) => {
    delete state.graphql_quota
    state.branch_heads = { 'ecorp/test': 'a'.repeat(40) }
    state.canonical_repository = 'EXample/REpo'
    state.fail_pr_create_after_success = true
  })
  const initial = f.read()
  const bodyFile = join(f.directory, 'body.md')
  writeFileSync(bodyFile, 'Exact publication body\n')
  const create = ['pr', 'create', '--repo', 'Example/Repo', '--head', 'ecorp/test', '--base', 'main',
    '--title', 'Exact PR', '--body-file', bodyFile]
  f.exec(create, { status: 1 })
  assert.equal(f.read().pull_requests.length, 1)
  assert.equal(f.read().pr_create_external_success_failures, 1)
  const prs = JSON.parse(f.exec(['pr', 'list', '--repo', 'Example/Repo', '--head', 'ecorp/test']).stdout)
  assert.equal(prs[0].url, 'https://github.com/EXample/REpo/pull/1')
  assert.equal(prs[0].body, 'Exact publication body\n')
  f.update((state) => {
    state.pr_list_mutation = { call: 2, number: 1, patch: { state: 'CLOSED' } }
  })
  assert.equal(JSON.parse(f.exec(['pr', 'list', '--repo', 'Example/Repo', '--state', 'all']).stdout)[0].state, 'CLOSED')
  assert.equal(f.read().pr_list_mutations_applied, 1)
  f.exec(['project', 'item-edit', '--id', item(1).id, '--project-id', initial.project.id,
    '--field-id', initial.project.status_field_id, '--single-select-option-id', 'progress'])
  assert.equal(f.read().items[0].status, 'In Progress')
  assert.equal(f.read().effect_log.at(-1).kind, 'project_status')
  const plain = f.graphql(exactQuery, { id: item(1).id })
  assert.equal(plain.json.data.node.fieldValueByName.optionId, 'progress')
  assert.equal(Object.hasOwn(plain.json.data, 'rateLimit'), false)
  assert.equal(f.read().graphql_quota.limit, 5000)
  assert.equal(f.read().graphql_quota.remaining, 4999)
  assert.equal(f.graphql(probe).json.data.rateLimit.remaining, 4998)
})

test('errors and observation logs never echo credentials, headers, query text or variables', (t) => {
  const f = fixture(t)
  const unauthorized = f.exec(['api', 'graphql', '--include', '-f', `query=${probe}`], {
    status: 1, env: { GH_TOKEN: 'other-private-test-credential' },
  })
  assert.equal(f.read().graphql_calls ?? 0, 0)
  const invalid = f.exec(['api', 'graphql', '--include', '-H', `Authorization: Bearer ${token}`,
    '-f', `query=query { unsupported(secret:"${token}") { secret } }`, '-f', `unused=${token}`], { status: 1 })
  const unknown = f.exec(['unsupported-command', token], { status: 1 })
  for (const result of [unauthorized, invalid, unknown]) {
    assert.ok(!result.stdout.includes(token) && !result.stderr.includes(token))
    assert.ok(!result.stdout.includes('other-private-test-credential') && !result.stderr.includes('other-private-test-credential'))
  }
  const observations = JSON.stringify(f.read())
  assert.ok(!observations.includes(token))
  assert.ok(!observations.includes('Authorization'))
  assert.ok(!observations.includes('unused'))
  assert.equal(f.read().graphql_query_counts.unsupported, 1)
  f.update((state) => {
    state.fail_next_item_edit = true
    state.fail_next_item_edit_message = `existing multiline hook\n${token}\nretained detail`
  })
  const redacted = f.exec(['project', 'item-edit', '--id', item(1).id,
    '--project-id', f.read().project.id, '--field-id', 'opaque-status-field',
    '--single-select-option-id', 'progress'], { status: 1 })
  assert.ok(!redacted.stderr.includes(token))
  assert.match(redacted.stderr, /existing multiline hook\n\[redacted\]\nretained detail/)
})

test('invalid quota/failure configurations fail safely and event retention is bounded', (t) => {
  const invalidConfigs = [
    (state) => { state.graphql_quota.remaining = -1 },
    (state) => { state.graphql_quota.cost_per_query = -1 },
    (state) => { state.graphql_quota.reset_at = 'invalid' },
    (state) => { state.graphql_failures = [{ match: 'quota', calls: [0], kind: 'primary' }] },
    (state) => { state.graphql_failures = [{ match: 'quota', calls: [1], kind: '503', retry_after: 3601 }] },
    (state) => { state.graphql_failures = [{ match: 'quota', calls: Array(101).fill(1), kind: 'secondary' }] },
    (state) => { state.graphql_failures = Array(65).fill({ match: 'quota', calls: [1], kind: 'secondary' }) },
  ]
  for (const modify of invalidConfigs) {
    const f = fixture(t, modify)
    const result = f.exec(['api', 'graphql', '--include', '-f', `query=${probe}`], { status: 1 })
    assert.match(result.stderr, /invalid fake GraphQL .* configuration/)
    assert.equal(result.stdout, '')
    assert.equal(f.read().graphql_calls ?? 0, 0)
  }
  const f = fixture(t, (state) => {
    state.graphql_calls = 10000
    state.graphql_event_count = 10000
    state.graphql_query_counts = { quota: 10000 }
    state.graphql_events = Array.from({ length: 10000 }, (_, index) => ({ call: index + 1, at: now }))
  })
  f.graphql(probe)
  const state = f.read()
  assert.equal(state.graphql_calls, 10001)
  assert.equal(state.graphql_event_count, 10001)
  assert.equal(state.graphql_events.length, 10000)
  assert.equal(state.graphql_events[0].call, 2)
  assert.equal(state.graphql_events.at(-1).call, 10001)
  assert.equal(state.graphql_events.at(-1).at, now)
})
