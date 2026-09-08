import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { mkdtemp, realpath, rm } from 'node:fs/promises'
import { readFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { createInterface } from 'node:readline'
import test from 'node:test'
import { fileURLToPath } from 'node:url'
import { runInNewContext } from 'node:vm'

const script = fileURLToPath(new URL('../scripts/fake-codex-app-server.mjs', import.meta.url))

async function observe(marker, interrupt = false) {
  const temp = await realpath(os.tmpdir())
  const root = await mkdtemp(path.join(temp, 'ecorp-budget-protocol-'))
  const child = spawn(process.execPath, [script], {
    cwd: root, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'],
  })
  const lines = createInterface({ input: child.stdout })
  const usage = []
  const messages = []
  let stderr = ''
  child.stderr.on('data', chunk => {
    stderr += chunk.toString()
    assert.ok(stderr.length < 16_384, 'fixture stderr must stay bounded')
  })
  const send = message => child.stdin.write(`${JSON.stringify(message)}\n`)
  let terminal
  let timer
  const finished = new Promise((resolve, reject) => {
    timer = setTimeout(() => reject(new Error('protocol fixture deadline')), 5000)
    child.once('error', reject)
    lines.on('line', line => {
      try {
        const message = JSON.parse(line)
        messages.push(message)
        if (message.id === 1) {
          send({ id: 2, method: 'thread/start', params: { cwd: root } })
        } else if (message.id === 2) {
          send({
            id: 3, method: 'turn/start',
            params: { threadId: message.result.thread.id, input: [{ type: 'text', text: marker }] },
          })
        } else if (message.method === 'thread/tokenUsage/updated') {
          assert.equal(readFileSync(path.join(root, 'base.txt'), 'utf8'), 'base\n')
          usage.push(message.params.tokenUsage)
          if (interrupt && usage.length === 1) send({ id: 4, method: 'turn/interrupt', params: {} })
        } else if (message.method === 'turn/completed') {
          terminal = message.params.turn.status
          child.stdin.end()
        }
      } catch (error) { reject(error) }
    })
    child.once('exit', code => {
      if (code !== 0 || !terminal) reject(new Error(`fixture exited without terminal proof: ${code}`))
      else resolve({ usage, terminal, messages })
    })
  })
  try {
    send({ id: 1, method: 'initialize', params: {} })
    return await finished
  } finally {
    clearTimeout(timer)
    if (child.exitCode === null) {
      child.kill()
      await new Promise(resolve => child.once('exit', resolve))
    }
    lines.close()
    const exact = await realpath(root)
    assert.equal(path.dirname(exact).toLowerCase(), temp.toLowerCase())
    assert.ok(path.basename(exact).startsWith('ecorp-budget-protocol-'))
    await rm(exact, { recursive: true })
  }
}

test('existing budget-stream keeps its original synthetic 6000-token contract', async () => {
  const observed = await observe('[budget-stream]')
  assert.equal(observed.terminal, 'completed')
  assert.deepEqual(observed.usage.map(item => item.last.inputTokens), [3000, 3000])
  assert.equal(observed.usage.at(-1).total.totalTokens, 6000)
})

test('UI budget-stream uses the real 500K preset with explicit synthetic counters', async () => {
  const observed = await observe('[budget-stream-ui]')
  assert.equal(observed.terminal, 'completed')
  assert.deepEqual(observed.usage.map(item => item.last.inputTokens), [300000, 300000])
  assert.equal(observed.usage.at(-1).total.totalTokens, 600000)
})

test('native interrupt prevents later usage and completion in the UI fixture', async () => {
  const observed = await observe('[budget-stream-ui]', true)
  assert.equal(observed.terminal, 'interrupted')
  assert.deepEqual(observed.usage.map(item => item.last.inputTokens), [300000])
})

test('original budget-stream still interrupts before its second usage and scheduled completion', async () => {
  const observed = await observe('[budget-stream]', true)
  assert.equal(observed.terminal, 'interrupted')
  assert.deepEqual(observed.usage.map(item => item.last.inputTokens), [3000])
})

test('queued completion emits exact usage and completed before a subsequent interrupt response', async () => {
  // A protocol-only subprocess, not an ECorp server/runner race test.
  const observed = await observe('[budget-queued-completion]', true)
  assert.equal(observed.terminal, 'completed')
  assert.deepEqual(observed.usage.map(item => item.last.totalTokens), [3000, 3000])
  assert.deepEqual(observed.usage.map(item => item.total.totalTokens), [3000, 6000])
  assert.ok(observed.usage.every(item => item.last.inputTokens === 3000 && item.last.outputTokens === 0))
  const completed = observed.messages.findIndex(item => item.method === 'turn/completed')
  const acknowledged = observed.messages.findIndex(item => item.id === 4)
  assert.ok(completed >= 0 && acknowledged > completed)
  assert.equal(observed.messages.filter(item => item.method === 'turn/completed').length, 1)
})

test('actual queued fixture writes base then emits both usages and finish synchronously without timers', () => {
  // Run the actual fixture handler with in-memory stdio/files/timers. This proves
  // one JS turn, not a guessed millisecond interval or any server-side admission.
  const source = readFileSync(script, 'utf8')
  const imports = source.match(/^import .+ from 'node:[^']+'\r?$/gmu)
  assert.equal(imports?.length, 3, 'review the sandbox when fixture imports change')
  const timeline = []
  const timers = []
  let onLine
  let nextId = 0
  runInNewContext(source.replace(/^import .+ from 'node:[^']+'\r?$/gmu, ''), {
    randomUUID: () => `fixture-${++nextId}`,
    writeFileSync: (file, bytes) => timeline.push({ file, bytes }),
    createInterface: () => ({ on: (type, callback) => {
      assert.equal(type, 'line')
      onLine = callback
    } }),
    process: {
      argv: [], cwd: () => '/fixture', platform: 'fixture', stdin: {},
      stdout: { write: line => { timeline.push(JSON.parse(line)); return true } },
    },
    setTimeout: (...args) => { timers.push(args); return timers.length },
    clearTimeout: () => assert.fail('queued completion must not schedule or cancel a timer'),
  }, { timeout: 1000, filename: script })
  onLine(JSON.stringify({ id: 1, method: 'thread/start', params: { cwd: '/fixture' } }))
  timeline.length = 0
  onLine(JSON.stringify({ id: 2, method: 'turn/start',
    params: { input: [{ type: 'text', text: '[budget-queued-completion]' }] } }))
  timeline.push({ test: 'handler_returned' })

  const base = timeline.findIndex(item => item.file === '/fixture/base.txt')
  const usages = timeline.flatMap((item, index) =>
    item.method === 'thread/tokenUsage/updated' ? [{ index, usage: item.params.tokenUsage }] : [])
  const completed = timeline.findIndex(item => item.method === 'turn/completed')
  assert.equal(timeline[base].bytes, 'base\n')
  assert.equal(timeline.filter(item => item.file).length, 1)
  assert.deepEqual(usages.map(item => item.usage.last.totalTokens), [3000, 3000])
  assert.deepEqual(usages.map(item => item.usage.total.totalTokens), [3000, 6000])
  assert.ok(base < usages[0].index && usages[0].index < usages[1].index &&
    usages[1].index < completed && completed < timeline.length - 1)
  assert.equal(timeline[completed].params.turn.status, 'completed')
  assert.equal(timeline.filter(item => item.method === 'item/agentMessage/delta').length, 1)
  assert.deepEqual(timers, [])
})
