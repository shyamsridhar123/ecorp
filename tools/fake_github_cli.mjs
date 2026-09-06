import { readFile, writeFile } from 'node:fs/promises'

// Local GraphQL fixture API (no network):
// graphql_quota: {limit:5000, remaining:5000, cost_per_query:1, reset_at:<ISO time>}.
// Missing values get those defaults and reset_at = now + 1 hour. Expiry restores
// limit and starts a fresh hour. graphql_now optionally pins now to an ISO time.
// Every successful query, including a quota-only probe, spends cost_per_query.
// graphql_failures: [{match:'project_items', calls:[1,2], kind:'secondary', retry_after:2}].
// match is a query kind below; calls are one-based *per-kind* attempts, including
// failures. First matching rule wins; at most 64 rules / 100 calls per rule.
// kind: primary | secondary | 503. retry_after: optional integer seconds, 0..3600.
// Primary failures exhaust remaining until reset; other failures spend nothing.
// Counters: graphql_calls, graphql_query_counts, graphql_event_count, graphql_resets,
// graphql_failure_counts. graphql_events retains the latest 10,000 timestamped
// attempts, never raw queries, variables, command arguments or credentials.
const graphqlKinds = [
  'quota', 'project_resolve', 'project_items', 'item_exact',
  'issue_memberships', 'project_field', 'unsupported',
]

const statePath = process.env.ECORP_FAKE_GITHUB_STATE
if (!statePath) {
  console.error('ECORP_FAKE_GITHUB_STATE is required')
  process.exit(2)
}

const state = JSON.parse(await readFile(statePath, 'utf8'))
const args = process.argv.slice(2)

function option(name) {
  const index = args.indexOf(name)
  return index >= 0 ? args[index + 1] : undefined
}

function formValue(name) {
  for (let index = 0; index < args.length - 1; index += 1) {
    if (
      ['-F', '-f', '--field', '--raw-field'].includes(args[index]) &&
      args[index + 1].startsWith(`${name}=`)
    ) {
      const value = args[index + 1].slice(name.length + 1)
      return ['-F', '--field'].includes(args[index]) && value === 'null' ? null : value
    }
  }
  return undefined
}

function fail(message) {
  let safe = String(message)
  for (const secret of [
    process.env.GH_TOKEN, process.env.GITHUB_TOKEN, process.env.ECORP_FAKE_GITHUB_EXPECT_TOKEN,
  ]) {
    if (secret) safe = safe.split(secret).join('[redacted]')
  }
  console.error(safe)
  process.exit(1)
}

function assertProject() {
  const number = Number(args[2])
  const owner = option('--owner')
  if (number !== state.project.number || owner !== state.project.owner) {
    fail(`unknown project ${owner}/${number}`)
  }
}

function assertPublisherCredential() {
  const expected = process.env.ECORP_FAKE_GITHUB_EXPECT_TOKEN
  if (expected && process.env.GH_TOKEN !== expected) {
    fail('trusted publisher credential was not brokered to the fake GitHub boundary')
  }
}

function observeProjectItemRead() {
  state.project_item_read_calls = (state.project_item_read_calls ?? 0) + 1
  // Legacy fixture name: this hook schedules a source change at an item-read
  // boundary, whether the controller uses a full list or an exact node.
  const mutation = state.item_list_mutation
  if (mutation && mutation.call === state.project_item_read_calls) {
    const issue = state.issues[String(mutation.issue_number)]
    if (!issue) fail(`scheduled mutation references unknown issue ${mutation.issue_number}`)
    Object.assign(issue, mutation.patch ?? {})
    if (mutation.remove_label) issue.labels = issue.labels.filter((label) => label.name !== mutation.remove_label)
    state.item_list_mutations_applied = (state.item_list_mutations_applied ?? 0) + 1
    state.item_list_mutation = null
  }
}

function projectIdentity() {
  return {
    __typename: 'ProjectV2',
    id: state.project.id,
    number: state.project.number,
    owner: {
      __typename: state.project.owner_type ?? 'Organization',
      login: state.project.owner,
    },
  }
}

function projectItemNode(item, exact = false) {
  const status = state.project.status_options?.find((candidate) => candidate.name === item.status)
  const kind = item.content?.type ?? item.content?.__typename
  let content = null
  if (item.content && kind) {
    content = exact
      ? { ...item.content, __typename: kind, repository: { nameWithOwner: item.content.repository } }
      : kind === 'Issue'
        ? {
            __typename: kind,
            number: item.content.number,
            title: item.content.title,
            body: item.content.body,
            url: item.content.url,
            repository: { nameWithOwner: item.content.repository },
          }
        : { __typename: kind }
  }
  return {
    ...(exact ? { __typename: 'ProjectV2Item', project: projectIdentity() } : {}),
    id: item.id,
    isArchived: item.isArchived ?? false,
    content,
    fieldValueByName: status
      ? {
          name: status.name,
          ...(exact ? {
            __typename: 'ProjectV2ItemFieldSingleSelectValue',
            optionId: status.id,
            field: { id: state.project.status_field_id, name: 'Status' },
          } : {}),
        }
      : null,
  }
}

function fixtureCursor(item) {
  // Returned opaque fixture identity, not a numeric offset accepted from callers.
  return Buffer.from(JSON.stringify(['fake-project-cursor', state.project.id, item.id])).toString('base64url')
}

async function graphql() {
  assertPublisherCredential()
  const query = formValue('query') ?? ''
  const compact = query.replace(/\s+/g, '')
  const itemId = formValue('id')
  const kind = compact.includes('repositoryOwner(') ? 'project_resolve'
    : compact.includes('projectItems(') ? 'issue_memberships'
      : compact.includes('node(') && compact.includes('items(') ? 'project_items'
        : compact.includes('node(') && itemId === state.project.id ? 'project_field'
          : compact.includes('node(') ? 'item_exact'
            : /^(?:query(?:[A-Za-z_]\w*)?(?:\([^)]*\))?)?\{rateLimit\{(?:limit|remaining|cost|resetAt)+\}\}$/.test(compact)
              ? 'quota' : 'unsupported'
  const now = state.graphql_now === undefined ? Date.now() : Date.parse(state.graphql_now)
  if (!Number.isFinite(now)) fail('invalid fake GraphQL clock configuration')
  const quota = state.graphql_quota ?? {}
  if (typeof quota !== 'object' || Array.isArray(quota)) fail('invalid fake GraphQL quota configuration')
  quota.limit ??= 5000
  quota.remaining ??= quota.limit
  quota.cost_per_query ??= 1
  quota.reset_at ??= new Date(now + 3_600_000).toISOString()
  let reset = Date.parse(quota.reset_at)
  if (
    ![quota.limit, quota.remaining, quota.cost_per_query].every((value) => Number.isSafeInteger(value) && value >= 0) ||
    quota.remaining > quota.limit || !Number.isFinite(reset)
  ) fail('invalid fake GraphQL quota configuration')
  if (now >= reset) {
    quota.remaining = quota.limit
    reset = now + 3_600_000
    state.graphql_resets = (state.graphql_resets ?? 0) + 1
  }
  quota.reset_at = new Date(reset).toISOString()
  state.graphql_quota = quota
  const rules = state.graphql_failures ?? []
  if (!Array.isArray(rules) || rules.length > 64 || rules.some((rule) =>
    !rule || !graphqlKinds.includes(rule.match) ||
    !['primary', 'secondary', '503'].includes(rule.kind) ||
    !Array.isArray(rule.calls) || rule.calls.length === 0 || rule.calls.length > 100 ||
    rule.calls.some((call) => !Number.isSafeInteger(call) || call < 1) ||
    (rule.retry_after !== undefined &&
      (!Number.isInteger(rule.retry_after) || rule.retry_after < 0 || rule.retry_after > 3600))
  )) fail('invalid fake GraphQL failure configuration')

  state.graphql_calls = (state.graphql_calls ?? 0) + 1
  state.graphql_query_counts ??= {}
  const kindCall = state.graphql_query_counts[kind] = (state.graphql_query_counts[kind] ?? 0) + 1
  const rule = rules.find((candidate) => candidate.match === kind && candidate.calls.includes(kindCall))

  async function respond(data, failure, retryAfter) {
    const status = failure === '503' ? 503
      : ['primary', 'secondary'].includes(failure) ? 403 : failure ? 400 : 200
    const message = failure === 'primary' ? 'API rate limit exceeded'
      : failure === 'secondary' ? 'You have exceeded a secondary rate limit'
        : failure === '503' ? 'Service Unavailable' : 'Invalid fake GraphQL query or cursor'
    if (failure === 'primary') quota.remaining = 0
    const cost = failure ? 0 : quota.cost_per_query
    quota.remaining -= cost
    if (failure) {
      state.graphql_failure_counts ??= {}
      state.graphql_failure_counts[failure] = (state.graphql_failure_counts[failure] ?? 0) + 1
      retryAfter ??= failure === 'primary' ? Math.max(1, Math.ceil((reset - now) / 1000)) : 1
    }
    state.graphql_event_count = (state.graphql_event_count ?? 0) + 1
    state.graphql_events = [...(state.graphql_events ?? []), {
      call: state.graphql_calls, kind, kind_call: kindCall, at: new Date(now).toISOString(),
      status, cost, remaining: quota.remaining, reset_at: quota.reset_at,
      ...(failure ? { failure } : {}),
    }].slice(-10_000)
    await writeFile(statePath, `${JSON.stringify(state, null, 2)}\n`)
    if (args.includes('--include') || args.includes('-i')) {
      const reason = status === 200 ? 'OK' : status === 403 ? 'Forbidden'
        : status === 503 ? 'Service Unavailable' : 'Bad Request'
      const headers = [
        `HTTP/1.1 ${status} ${reason}`,
        'Content-Type: application/json',
        `Date: ${new Date(now).toUTCString()}`,
        `X-RateLimit-Limit: ${quota.limit}`,
        `X-RateLimit-Remaining: ${quota.remaining}`,
        `X-RateLimit-Used: ${quota.limit - quota.remaining}`,
        `X-RateLimit-Reset: ${Math.floor(reset / 1000)}`,
        'X-RateLimit-Resource: graphql',
        ...(['primary', 'secondary', '503'].includes(failure) ? [`Retry-After: ${retryAfter}`] : []),
      ]
      process.stdout.write(`${headers.join('\r\n')}\r\n\r\n`)
    }
    if (failure) {
      console.log(JSON.stringify({ errors: [{
        type: failure === 'primary' ? 'RATE_LIMITED' : failure === 'secondary' ? 'FORBIDDEN'
          : failure === '503' ? 'SERVICE_UNAVAILABLE' : 'BAD_USER_INPUT',
        message,
      }] }))
      fail(`gh: ${message} (HTTP ${status})`)
    }
    if (/\brateLimit\s*\{/.test(query)) {
      data.rateLimit = {
        limit: quota.limit, remaining: quota.remaining, cost, resetAt: quota.reset_at,
      }
    }
    console.log(JSON.stringify({ data }))
  }

  if (rule || quota.remaining < quota.cost_per_query) {
    return respond(null, rule?.kind ?? 'primary', rule?.retry_after)
  }
  if (kind === 'quota') return respond({})
  if (kind === 'project_resolve') {
    const ownerMatches = typeof formValue('owner') === 'string' &&
      formValue('owner').toLowerCase() === state.project.owner.toLowerCase()
    return respond({ repositoryOwner: ownerMatches ? {
      __typename: state.project.owner_type ?? 'Organization',
      login: state.project.owner,
      projectV2: Number(formValue('number')) === state.project.number ? projectIdentity() : null,
    } : null })
  }
  if (kind === 'issue_memberships') {
    const repository = `${formValue('owner')}/${formValue('repo')}`
    const number = Number(formValue('number'))
    const issue = state.issues[String(number)]
    const items = state.items.filter((item) => item.content?.number === number &&
      item.content.repository?.toLowerCase() === repository.toLowerCase())
    return respond({ repository: { issue: issue ? {
      projectItems: { pageInfo: { hasNextPage: false }, nodes: items.map((item) => ({
        id: item.id, project: projectIdentity(),
      })) },
    } : null } })
  }
  if (kind === 'unsupported' || !itemId) return respond(null, 'invalid')
  if (kind === 'project_items') {
    if (itemId !== state.project.id) return respond({ node: null })
    const first = Number(formValue('first') ?? compact.match(/\bfirst:(\d+)/)?.[1] ?? 100)
    if (!Number.isInteger(first) || first < 1 || first > 100) return respond(null, 'invalid')
    const activeOnly = compact.includes('archivedStates:[NOT_ARCHIVED]')
    const items = state.items.filter((item) => !activeOnly || !item.isArchived)
    const after = formValue('cursor') ?? formValue('after')
    let offset = 0
    if (after !== undefined && after !== null) {
      const matches = items.flatMap((item, index) => fixtureCursor(item) === after ? [index] : [])
      if (matches.length !== 1) return respond(null, 'invalid')
      offset = matches[0] + 1
    }
    observeProjectItemRead()
    state.project_items_page_calls = (state.project_items_page_calls ?? 0) + 1
    const nodes = items.slice(offset, offset + first)
    return respond({ node: {
      ...projectIdentity(),
      items: {
        totalCount: items.length,
        pageInfo: {
          endCursor: nodes.length ? fixtureCursor(nodes.at(-1)) : null,
          hasNextPage: offset + nodes.length < items.length,
        },
        nodes: nodes.map((item) => projectItemNode(item)),
      },
    } })
  }
  if (kind === 'project_field') {
    state.project_field_lookup_calls =
      (state.project_field_lookup_calls ?? 0) + 1
    return respond({ node: {
      ...projectIdentity(),
      field: {
        __typename: 'ProjectV2SingleSelectField',
        id: state.project.status_field_id,
        name: 'Status',
        options: state.project.status_options,
      },
    } })
  }
  observeProjectItemRead()
  const item = state.items.find((candidate) => candidate.id === itemId)
  state.project_item_lookup_calls = (state.project_item_lookup_calls ?? 0) + 1
  return respond({ node: item ? projectItemNode(item, true) : null })
}

if (args[0] === 'api' && args.slice(1).includes('graphql')) {
  await graphql()
} else if (args[0] === 'project' && args[1] === 'item-list') {
  assertProject()
  state.item_list_calls = (state.item_list_calls ?? 0) + 1
  observeProjectItemRead()
  await writeFile(statePath, `${JSON.stringify(state, null, 2)}\n`)
  const requestedLimit = Number(option('--limit') ?? state.items.length)
  const activeItems = state.items.filter((item) => !item.isArchived)
  console.log(
    JSON.stringify({
      items: activeItems.slice(0, requestedLimit),
      totalCount: activeItems.length,
    }),
  )
} else if (args[0] === 'project' && args[1] === 'view') {
  assertProject()
  console.log(JSON.stringify(state.project))
} else if (args[0] === 'project' && args[1] === 'field-list') {
  assertProject()
  state.field_list_calls = (state.field_list_calls ?? 0) + 1
  const fields = state.project_fields ?? [
    {
      id: state.project.status_field_id,
      name: 'Status',
      type: 'ProjectV2SingleSelectField',
      options: state.project.status_options,
    },
  ]
  const requestedLimit = Number(option('--limit') ?? 30)
  await writeFile(statePath, `${JSON.stringify(state, null, 2)}\n`)
  console.log(
    JSON.stringify({
      fields: fields.slice(0, requestedLimit),
      totalCount: fields.length,
    }),
  )
} else if (args[0] === 'project' && args[1] === 'item-edit') {
  const itemId = option('--id')
  const projectId = option('--project-id')
  const fieldId = option('--field-id')
  const optionId = option('--single-select-option-id')
  if (
    projectId !== state.project.id ||
    fieldId !== state.project.status_field_id
  ) {
    fail('project item edit used the wrong project or status field')
  }
  const item = state.items.find((candidate) => candidate.id === itemId)
  const status = state.project.status_options.find(
    (candidate) => candidate.id === optionId,
  )
  if (!item || !status) {
    fail('project item edit referenced an unknown item or option')
  }
  if (state.item_edit_delay_ms) {
    await new Promise((resolve) => setTimeout(resolve, state.item_edit_delay_ms))
  }
  if (state.fail_next_item_edit) {
    state.fail_next_item_edit = false
    state.item_edit_failures = (state.item_edit_failures ?? 0) + 1
    await writeFile(statePath, `${JSON.stringify(state, null, 2)}\n`)
    fail(
      state.fail_next_item_edit_message ??
        'injected GitHub Project status update failure',
    )
  }
  item.status = status.name
  state.item_edits = (state.item_edits ?? 0) + 1
  state.effect_log = [
    ...(state.effect_log ?? []),
    {
      kind: 'project_status',
      item_id: item.id,
      status: status.name,
      pull_request_count: (state.pull_requests ?? []).length,
      target_pull_request_count: (state.pull_requests ?? []).filter(
        (pullRequest) => pullRequest.isCrossRepository === false,
      ).length,
    },
  ]
  await writeFile(statePath, `${JSON.stringify(state, null, 2)}\n`)
} else if (args[0] === 'issue' && args[1] === 'view') {
  const number = Number(args[2])
  const repository = option('--repo')
  if (repository !== state.repository) {
    fail(`unknown repository ${repository}`)
  }
  const issue = state.issues[String(number)]
  if (!issue) {
    fail(`unknown issue ${number}`)
  }
  console.log(JSON.stringify(issue))
} else if (args[0] === 'pr' && args[1] === 'list') {
  assertPublisherCredential()
  const repository = option('--repo')
  if (repository !== state.repository) {
    fail(`unknown repository ${repository}`)
  }
  const head = option('--head')
  const base = option('--base')
  const requestedState = option('--state') ?? 'open'
  state.pr_list_calls = (state.pr_list_calls ?? 0) + 1
  const mutation = state.pr_list_mutation
  if (mutation && mutation.call === state.pr_list_calls) {
    const pullRequest = (state.pull_requests ?? []).find(
      (candidate) => candidate.number === mutation.number,
    )
    if (!pullRequest) {
      fail(`scheduled pr-list mutation references unknown PR ${mutation.number}`)
    }
    Object.assign(pullRequest, mutation.patch ?? {})
    state.pr_list_mutations_applied =
      (state.pr_list_mutations_applied ?? 0) + 1
    state.pr_list_mutation = null
  }
  const pullRequests = (state.pull_requests ?? []).filter(
    (pullRequest) =>
      (!head || pullRequest.headRefName === head) &&
      (!base || pullRequest.baseRefName === base) &&
      (requestedState === 'all' ||
        (requestedState === 'open' && pullRequest.state === 'OPEN') ||
        (requestedState === 'closed' && pullRequest.state !== 'OPEN')),
  )
  await writeFile(statePath, `${JSON.stringify(state, null, 2)}\n`)
  console.log(JSON.stringify(pullRequests))
} else if (args[0] === 'pr' && args[1] === 'create') {
  assertPublisherCredential()
  const repository = option('--repo')
  if (repository !== state.repository) {
    fail(`unknown repository ${repository}`)
  }
  const head = option('--head')
  const base = option('--base')
  const title = option('--title')
  const bodyFile = option('--body-file')
  if (!head || !base || !title || !bodyFile) {
    fail('pull request creation omitted required fields')
  }
  const existing = (state.pull_requests ?? []).find(
    (pullRequest) =>
      pullRequest.headRefName === head &&
      pullRequest.baseRefName === base &&
      pullRequest.isCrossRepository === false &&
      pullRequest.headRepositoryOwner?.login === repository.split('/')[0] &&
      pullRequest.state === 'OPEN',
  )
  if (existing) {
    fail(`a pull request already exists for ${head} -> ${base}`)
  }
  const body = await readFile(bodyFile, 'utf8')
  if (state.pr_create_delay_ms) {
    await new Promise((resolve) => setTimeout(resolve, state.pr_create_delay_ms))
  }
  const number = state.next_pr_number ?? 1
  const canonicalRepository = state.canonical_repository ?? repository
  const pullRequest = {
    number,
    id: `PR_FAKE_${number}`,
    url: `https://github.com/${canonicalRepository}/pull/${number}`,
    state: 'OPEN',
    isDraft: false,
    headRefName: head,
    baseRefName: base,
    headRefOid: state.branch_heads?.[head],
    headRepositoryOwner: { login: repository.split('/')[0] },
    isCrossRepository: false,
    autoMergeRequest: null,
    title,
    body,
  }
  if (!pullRequest.headRefOid) {
    fail(`fake GitHub has no target-repository head oid for ${head}`)
  }
  state.next_pr_number = number + 1
  state.pr_create_calls = (state.pr_create_calls ?? 0) + 1
  state.pull_requests = [...(state.pull_requests ?? []), pullRequest]
  state.effect_log = [
    ...(state.effect_log ?? []),
    {
      kind: 'pull_request_created',
      number,
      head,
      base,
    },
  ]
  await writeFile(statePath, `${JSON.stringify(state, null, 2)}\n`)
  if (state.fail_pr_create_after_success) {
    state.fail_pr_create_after_success = false
    state.pr_create_external_success_failures =
      (state.pr_create_external_success_failures ?? 0) + 1
    await writeFile(statePath, `${JSON.stringify(state, null, 2)}\n`)
    fail('injected local failure after remote pull request creation')
  }
  console.log(pullRequest.url)
} else {
  fail('unsupported fake gh command')
}
