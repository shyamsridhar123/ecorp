import { readFile, writeFile } from 'node:fs/promises'

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
      (args[index] === '-F' || args[index] === '-f') &&
      args[index + 1].startsWith(`${name}=`)
    ) {
      return args[index + 1].slice(name.length + 1)
    }
  }
  return undefined
}

function fail(message) {
  console.error(message)
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

if (args[0] === 'api' && args[1] === 'graphql') {
  assertPublisherCredential()
  const itemId = formValue('id')
  if (!itemId) {
    fail('exact Project item lookup omitted its node id')
  }
  if (itemId === state.project.id) {
    state.project_field_lookup_calls =
      (state.project_field_lookup_calls ?? 0) + 1
    await writeFile(statePath, `${JSON.stringify(state, null, 2)}\n`)
    console.log(
      JSON.stringify({
        data: {
          node: {
            __typename: 'ProjectV2',
            field: {
              __typename: 'ProjectV2SingleSelectField',
              id: state.project.status_field_id,
              name: 'Status',
              options: state.project.status_options,
            },
          },
        },
      }),
    )
    process.exit(0)
  }
  const item = state.items.find((candidate) => candidate.id === itemId)
  const status = item
    ? state.project.status_options.find(
        (candidate) => candidate.name === item.status,
      )
    : undefined
  state.project_item_lookup_calls = (state.project_item_lookup_calls ?? 0) + 1
  await writeFile(statePath, `${JSON.stringify(state, null, 2)}\n`)
  console.log(
    JSON.stringify({
      data: {
        node: item
          ? {
              __typename: 'ProjectV2Item',
              id: item.id,
              project: {
                id: state.project.id,
                number: state.project.number,
                owner: {
                  __typename: 'Organization',
                  login: state.project.owner,
                },
              },
              fieldValueByName: status
                ? {
                    __typename: 'ProjectV2ItemFieldSingleSelectValue',
                    name: status.name,
                    optionId: status.id,
                    field: {
                      id: state.project.status_field_id,
                      name: 'Status',
                    },
                  }
                : null,
            }
          : null,
      },
    }),
  )
} else if (args[0] === 'project' && args[1] === 'item-list') {
  assertProject()
  state.item_list_calls = (state.item_list_calls ?? 0) + 1
  const mutation = state.item_list_mutation
  if (mutation && mutation.call === state.item_list_calls) {
    const issue = state.issues[String(mutation.issue_number)]
    if (!issue) {
      fail(`scheduled item-list mutation references unknown issue ${mutation.issue_number}`)
    }
    Object.assign(issue, mutation.patch ?? {})
    if (mutation.remove_label) {
      issue.labels = issue.labels.filter((label) => label.name !== mutation.remove_label)
    }
    state.item_list_mutations_applied = (state.item_list_mutations_applied ?? 0) + 1
    state.item_list_mutation = null
  }
  await writeFile(statePath, `${JSON.stringify(state, null, 2)}\n`)
  const requestedLimit = Number(option('--limit') ?? state.items.length)
  console.log(
    JSON.stringify({
      items: state.items.slice(0, requestedLimit),
      totalCount: state.items.length,
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
  fail(`unsupported fake gh command: ${args.join(' ')}`)
}
