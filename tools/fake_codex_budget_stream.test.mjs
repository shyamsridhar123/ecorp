import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { mkdtemp, realpath, rm } from 'node:fs/promises'
import { readFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { createInterface } from 'node:readline'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('../scripts/fake-codex-app-server.mjs', import.meta.url))

async function observe(marker, interrupt = false) {
  const temp = await realpath(os.tmpdir())
  const root = await mkdtemp(path.join(temp, 'ecorp-budget-protocol-'))
  const child = spawn(process.execPath, [script], {
    cwd: root, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'],
  })
  const lines = createInterface({ input: child.stdout })
  const usage = []
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
      else resolve({ usage, terminal })
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
