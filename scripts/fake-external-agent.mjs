#!/usr/bin/env node

import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { writeFile } from 'node:fs/promises'
import path from 'node:path'
import readline from 'node:readline'

const treeParentIndex = process.argv.indexOf('--fixture-tree-parent')
const treeGrandchildIndex = process.argv.indexOf('--fixture-tree-grandchild')
if (treeGrandchildIndex >= 0) {
  if (process.argv.includes('--stubborn')) {
    process.on('SIGTERM', () => {})
  }
  if (process.argv.includes('--exit-soon')) {
    setTimeout(() => process.exit(0), 50)
  }
  await new Promise(() => {})
} else if (treeParentIndex >= 0) {
  const pidFile = process.argv[treeParentIndex + 1]
  const fixtureFlags = [
    ...(process.argv.includes('--stubborn') ? ['--stubborn'] : []),
    ...(process.argv.includes('--exit-soon') ? ['--exit-soon'] : []),
  ]
  if (fixtureFlags.includes('--stubborn')) {
    process.on('SIGTERM', () => {})
  }
  const grandchild = spawn(
    process.execPath,
    [process.argv[1], '--fixture-tree-grandchild', ...fixtureFlags],
    { stdio: 'ignore' },
  )
  await writeFile(
    pidFile,
    JSON.stringify({ parent: process.pid, grandchild: grandchild.pid }),
  )
  await new Promise(() => {})
}

const providerIndex = process.argv.indexOf('--provider')
const provider =
  providerIndex >= 0
    ? process.argv[providerIndex + 1]
    : process.argv.includes('--print')
      ? 'claude-code'
      : process.argv.includes('run')
        ? 'opencode'
        : 'external'

if (process.argv.includes('--version')) {
  process.stdout.write(`${provider} fake 1.0.0\n`)
  process.exit(0)
}

const slow = process.argv.some((value) => value.includes('[slow]'))
const sessionArgument = process.argv.find((value) =>
  value.startsWith(provider === 'claude-code' ? '--resume=' : '--session='),
)
const legacySessionFlag = provider === 'claude-code' ? '--resume' : '--session'
const legacySessionIndex = process.argv.indexOf(legacySessionFlag)
const sessionId =
  sessionArgument?.slice(sessionArgument.indexOf('=') + 1) ??
  (legacySessionIndex >= 0
    ? process.argv[legacySessionIndex + 1]
    : `${provider}-${randomUUID()}`)

function output(value) {
  process.stdout.write(`${JSON.stringify(value)}\n`)
}

function finish() {
  output({ text: `${provider} normalized provider output` })
  output({
    usage: {
      input_tokens: 100,
      output_tokens: 40,
      cost_microusd: 200,
    },
  })
  output({ type: 'result', text: 'completed' })
}

function startTreeFixture(mission) {
  if (!mission.includes('[process-tree')) {
    return
  }
  const flags = [
    ...(mission.includes(':stubborn') ? ['--stubborn'] : []),
    ...(mission.includes(':exited-descendant') ? ['--exit-soon'] : []),
  ]
  const parent = spawn(
    process.execPath,
    [
      process.argv[1],
      '--fixture-tree-parent',
      path.join(process.cwd(), 'external-tree-pids.json'),
      ...flags,
    ],
    { stdio: 'ignore' },
  )
  assert.ok(parent.pid, 'fixture parent did not start')
}

async function finishClaude(input, inputInterface) {
  finish()
  const end = await input.next()
  assert.equal(end.done, true, 'Claude stream input remained open after result')
  inputInterface.close()
}

async function runClaude() {
  for (const required of [
    '--safe-mode',
    '--no-chrome',
    '--disable-slash-commands',
    '--strict-mcp-config',
    '--input-format',
    '--output-format',
    '--permission-mode',
    '--permission-prompt-tool',
  ]) {
    assert.ok(process.argv.includes(required), `missing Claude argument ${required}`)
  }
  assert.ok(!process.argv.includes('--dangerously-skip-permissions'))
  assert.ok(!process.argv.includes('bypassPermissions'))
  const argumentValue = (name) => process.argv[process.argv.indexOf(name) + 1]
  assert.equal(argumentValue('--input-format'), 'stream-json')
  assert.equal(argumentValue('--output-format'), 'stream-json')
  assert.equal(argumentValue('--permission-mode'), 'manual')
  assert.equal(argumentValue('--permission-prompt-tool'), 'stdio')

  const inputInterface = readline.createInterface({
    input: process.stdin,
    crlfDelay: Infinity,
  })
  const input = inputInterface[Symbol.asyncIterator]()
  const first = await input.next()
  assert.equal(first.done, false, 'Claude initialize frame missing')
  const initializeFrame = JSON.parse(first.value)
  assert.equal(initializeFrame.type, 'control_request')
  assert.equal(initializeFrame.request.subtype, 'initialize')
  assert.equal(initializeFrame.request.hooks, null)
  assert.equal(typeof initializeFrame.request_id, 'string')
  output({
    type: 'control_response',
    response: {
      subtype: 'success',
      request_id: initializeFrame.request_id,
      response: {},
    },
  })

  const missionLine = await input.next()
  assert.equal(missionLine.done, false, 'Claude mission frame missing')
  const missionFrame = JSON.parse(missionLine.value)
  assert.equal(missionFrame.type, 'user')
  assert.equal(missionFrame.message.role, 'user')
  assert.equal(typeof missionFrame.message.content, 'string')
  const mission = missionFrame.message.content

  output({ type: 'system', subtype: 'init', session_id: sessionId })
  startTreeFixture(mission)
  if (mission.includes('[process-tree')) {
    await new Promise((resolve) => setTimeout(resolve, 10_000))
  }

  if (!mission.includes('[permission:')) {
    await finishClaude(input, inputInterface)
    return
  }

  const requestId = 'claude-request-001'
  const toolUseId = 'claude-tool-use-001'
  const artifactPath = path.join(process.cwd(), 'claude-permission-artifact.txt')
  let toolName = 'Bash'
  let toolInput = {
    command:
      'node -e "process.stdout.write(process.env.ECORP_SECRET_TOKEN)"',
    env: { ECORP_SECRET_TOKEN: 'sk-secret-must-not-be-durable' },
  }
  let blockedPath = null

  if (mission.includes('[permission:safe]')) {
    toolName = 'Write'
    toolInput = {
      file_path: artifactPath,
      content: 'safe worktree write\n',
    }
  } else if (mission.includes('[permission:outside]')) {
    toolName = 'Write'
    toolInput = {
      file_path: path.join(process.cwd(), '..', 'outside.txt'),
      content: 'must not be written\n',
    }
  } else if (mission.includes('[permission:blocked]')) {
    toolName = 'Write'
    toolInput = {
      file_path: artifactPath,
      content: 'must require approval\n',
    }
    blockedPath = artifactPath
  }

  const request = {
    subtype: 'can_use_tool',
    tool_name: toolName,
    input: toolInput,
    tool_use_id: toolUseId,
    blocked_path: blockedPath,
    decision_reason: 'Fake provider requires a governed decision',
    title: 'Protocol-faithful fake request',
    display_name: `Claude ${toolName}`,
    description: 'Exercises ECorp durable permission translation',
  }
  if (mission.includes('[permission:malformed]')) {
    delete request.input
  }
  output({ type: 'control_request', request_id: requestId, request })

  if (mission.includes('[permission:response-write-failure]')) {
    inputInterface.close()
    process.stdin.destroy()
    return
  }
  if (mission.includes('[permission:cancelled]')) {
    output({ type: 'control_cancel_request', request_id: requestId })
    await finishClaude(input, inputInterface)
    return
  }
  const responseLine = await input.next()
  assert.equal(responseLine.done, false, 'Claude control response missing')
  const response = JSON.parse(responseLine.value)
  assert.equal(response.type, 'control_response')
  assert.equal(response.response.subtype, 'success')
  assert.equal(response.response.request_id, requestId)
  const decision = response.response.response
  if (decision.behavior === 'allow') {
    assert.deepEqual(decision.updatedInput, toolInput)
    await writeFile(
      artifactPath,
      mission.includes('[permission:safe]')
        ? toolInput.content
        : 'durably approved exactly once\n',
    )
  } else {
    assert.equal(decision.behavior, 'deny')
    assert.equal(typeof decision.message, 'string')
    assert.ok(decision.message.length <= 501)
  }
  await finishClaude(input, inputInterface)
}

try {
  if (provider === 'claude-code') {
    await runClaude()
  } else {
    output({ session_id: sessionId })
    finish()
  }
  if (slow) {
    await new Promise((resolve) => setTimeout(resolve, 10_000))
  }
} catch (error) {
  process.stderr.write(`${error.stack ?? error}\n`)
  process.exitCode = 2
}
