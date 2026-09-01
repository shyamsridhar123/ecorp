#!/usr/bin/env node

import { randomUUID } from 'node:crypto'
import { writeFileSync } from 'node:fs'
import { createInterface } from 'node:readline'

if (process.argv.includes('--version')) {
  process.stdout.write('codex-cli fake-app-server\n')
  process.exit(0)
}

const lineReader = createInterface({
  input: process.stdin,
  crlfDelay: Infinity,
})

let threadId = null
let turnId = null
let workspace = process.cwd()
let resumed = false
let completionTimer = null
let terminal = false
let finalMessage = 'Synthetic Codex turn completed.'
let usageTotal = 0
let emitUsageOnFinish = true

function send(value) {
  process.stdout.write(`${JSON.stringify(value)}\n`)
}

function respond(id, result) {
  send({ id, result })
}

function emit(method, params) {
  send({ method, params, emittedAtMs: Date.now() })
}

function textFromInput(input) {
  return (input ?? [])
    .filter((item) => item?.type === 'text')
    .map((item) => item.text)
    .join('\n')
}

function emitUsage(inputTokens = 10, outputTokens = 2) {
  const totalTokens = inputTokens + outputTokens
  usageTotal += totalTokens
  emit('thread/tokenUsage/updated', {
    threadId,
    turnId,
    tokenUsage: {
      total: {
        totalTokens: usageTotal,
        inputTokens: usageTotal - 2,
        cachedInputTokens: 0,
        cacheWriteInputTokens: 0,
        outputTokens,
        reasoningOutputTokens: 0,
      },
      last: {
        totalTokens,
        inputTokens,
        cachedInputTokens: 0,
        cacheWriteInputTokens: 0,
        outputTokens,
        reasoningOutputTokens: 0,
      },
      modelContextWindow: 1000,
    },
  })
}

function finish(status, error = null) {
  if (terminal) return
  terminal = true
  if (completionTimer) clearTimeout(completionTimer)
  if (emitUsageOnFinish) emitUsage()
  if (status === 'completed') {
    const itemId = randomUUID()
    emit('item/started', {
      threadId,
      turnId,
      item: { type: 'agentMessage', id: itemId, text: '', phase: 'final_answer' },
      startedAtMs: Date.now(),
    })
    emit('item/agentMessage/delta', {
      threadId,
      turnId,
      itemId,
      delta: finalMessage,
    })
    emit('item/completed', {
      threadId,
      turnId,
      item: {
        type: 'agentMessage',
        id: itemId,
        text: finalMessage,
        phase: 'final_answer',
      },
      completedAtMs: Date.now(),
    })
  }
  emit('turn/completed', {
    threadId,
    turn: {
      id: turnId,
      items: [],
      status,
      error: error ? { message: error } : null,
    },
  })
}

function startTurn(message) {
  turnId = randomUUID()
  terminal = false
  emitUsageOnFinish = true
  const prompt = textFromInput(message.params?.input)
  respond(message.id, {
    turn: { id: turnId, items: [], status: 'inProgress', error: null },
  })
  emit('turn/started', {
    threadId,
    turn: { id: turnId, items: [], status: 'inProgress', error: null },
  })

  const commandItemId = randomUUID()
  emit('item/started', {
    threadId,
    turnId,
    item: {
      type: 'commandExecution',
      id: commandItemId,
      command: 'write deterministic fixture',
      cwd: workspace,
      status: 'inProgress',
      commandActions: [],
    },
    startedAtMs: Date.now(),
  })
  const output = resumed ? 'resumed fixture\n' : 'base fixture\n'
  emit('item/commandExecution/outputDelta', {
    threadId,
    turnId,
    itemId: commandItemId,
    delta: output,
  })
  if (resumed) {
    writeFileSync(`${workspace}/resumed.txt`, 'resumed\n')
    finalMessage = 'Synthetic Codex session resumed and completed.'
  } else {
    writeFileSync(`${workspace}/base.txt`, 'base\n')
    finalMessage = 'Synthetic Codex session started and completed.'
  }
  emit('item/completed', {
    threadId,
    turnId,
    item: {
      type: 'commandExecution',
      id: commandItemId,
      command: 'write deterministic fixture',
      cwd: workspace,
      status: 'completed',
      commandActions: [],
      aggregatedOutput: output,
      exitCode: 0,
    },
    completedAtMs: Date.now(),
  })

  if (prompt.includes('[budget-stream]')) {
    emitUsageOnFinish = false
    setTimeout(() => {
      if (!terminal) emitUsage(3_000, 0)
    }, 40)
    setTimeout(() => {
      if (!terminal) emitUsage(3_000, 0)
    }, 120)
    completionTimer = setTimeout(() => finish('completed'), 1_000)
  } else if (prompt.includes('[fail]')) {
    completionTimer = setTimeout(() => finish('failed', 'synthetic failure'), 80)
  } else {
    completionTimer = setTimeout(
      () => finish('completed'),
      prompt.includes('[slow]')
        ? 700
        : prompt.includes('[graph-slow]')
          ? 1_500
          : 80,
    )
  }
}

lineReader.on('line', (line) => {
  let message
  try {
    message = JSON.parse(line)
  } catch {
    return
  }

  switch (message.method) {
    case 'initialize':
      respond(message.id, {
        userAgent: 'fake-codex-app-server',
        codexHome: process.cwd(),
        platformFamily: process.platform,
        platformOs: process.platform,
      })
      break
    case 'initialized':
      break
    case 'thread/start':
      threadId = randomUUID()
      workspace = message.params?.cwd ?? process.cwd()
      resumed = false
      respond(message.id, { thread: { id: threadId } })
      emit('thread/started', { thread: { id: threadId } })
      break
    case 'thread/resume':
      threadId = message.params?.threadId
      workspace = message.params?.cwd ?? process.cwd()
      resumed = true
      respond(message.id, { thread: { id: threadId } })
      emit('thread/started', { thread: { id: threadId } })
      break
    case 'turn/start':
      startTurn(message)
      break
    case 'turn/steer': {
      const text = textFromInput(message.params?.input).replace(
        /^Live direction from human actor [^:]+:\s*/,
        '',
      )
      writeFileSync(`${workspace}/steered.txt`, `${text}\n`)
      finalMessage = 'Synthetic Codex turn applied live steering and completed.'
      respond(message.id, { turnId })
      const itemId = randomUUID()
      emit('item/completed', {
        threadId,
        turnId,
        item: {
          type: 'fileChange',
          id: itemId,
          status: 'completed',
          changes: [{ path: `${workspace}/steered.txt`, kind: 'add' }],
        },
        completedAtMs: Date.now(),
      })
      break
    }
    case 'turn/interrupt':
      respond(message.id, {})
      finish('interrupted')
      break
    default:
      if (message.id !== undefined) {
        send({
          id: message.id,
          error: { code: -32601, message: `unsupported method ${message.method}` },
        })
      }
      break
  }
})
