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

if (args[0] === 'project' && args[1] === 'item-list') {
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
  console.log(
    JSON.stringify({
      items: state.items,
      totalCount: state.items.length,
    }),
  )
} else if (args[0] === 'project' && args[1] === 'view') {
  assertProject()
  console.log(JSON.stringify(state.project))
} else if (args[0] === 'project' && args[1] === 'field-list') {
  assertProject()
  console.log(
    JSON.stringify({
      fields: [
        {
          id: state.project.status_field_id,
          name: 'Status',
          type: 'ProjectV2SingleSelectField',
          options: state.project.status_options,
        },
      ],
      totalCount: 1,
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
} else {
  fail(`unsupported fake gh command: ${args.join(' ')}`)
}
