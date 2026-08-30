import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { writeFile } from 'node:fs/promises'
import path from 'node:path'
import readline from 'node:readline'

const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const root = path.resolve(import.meta.dirname, '..')
const exe = (name) =>
  path.join(root, 'target', 'debug', process.platform === 'win32' ? `${name}.exe` : name)

async function post(url, body) {
  const response = await fetch(`${server}${url}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
  const payload = await response.json()
  if (!response.ok) throw new Error(`${response.status}: ${JSON.stringify(payload)}`)
  return payload
}

function stdioRpc(binary, args = []) {
  const child = spawn(binary, args, { cwd: root })
  const lines = readline.createInterface({ input: child.stdout })
  const pending = []
  lines.on('line', (line) => pending.shift()?.resolve(JSON.parse(line)))
  child.stderr.on('data', (chunk) => {
    const text = chunk.toString()
    if (text.trim()) process.stderr.write(text)
  })
  return {
    call(request) {
      return new Promise((resolve, reject) => {
        pending.push({ resolve, reject })
        child.stdin.write(`${JSON.stringify(request)}\n`)
      })
    },
    close() {
      child.kill()
    },
  }
}

const demo = await post('/api/demo/reset', {})
const commonArgs = [
  '--server',
  server,
  '--corp-id',
  demo.corp_id,
  '--actor-id',
  demo.alice_actor_id,
]

const mcp = stdioRpc(exe('crony-mcp'), commonArgs)
const mcpInit = await mcp.call({
  jsonrpc: '2.0',
  id: 1,
  method: 'initialize',
  params: { protocolVersion: '2025-06-18' },
})
assert.equal(mcpInit.result.protocolVersion, '2025-06-18')
const mcpTools = await mcp.call({
  jsonrpc: '2.0',
  id: 2,
  method: 'tools/list',
  params: {},
})
assert.equal(mcpTools.result.tools.length, 3)
const mcpSnapshot = await mcp.call({
  jsonrpc: '2.0',
  id: 3,
  method: 'tools/call',
  params: { name: 'crony_snapshot', arguments: {} },
})
assert.equal(mcpSnapshot.result.structuredContent.snapshot.corp.id, demo.corp_id)
mcp.close()

const acp = stdioRpc(exe('crony-acp'), commonArgs)
const acpInit = await acp.call({
  jsonrpc: '2.0',
  id: 1,
  method: 'initialize',
  params: { protocolVersion: 1 },
})
assert.equal(acpInit.result.protocolVersion, 1)
const acpSession = await acp.call({
  jsonrpc: '2.0',
  id: 2,
  method: 'session/new',
  params: {},
})
assert.ok(acpSession.result.sessionId)
const acpPrompt = await acp.call({
  jsonrpc: '2.0',
  id: 3,
  method: 'session/prompt',
  params: {
    sessionId: acpSession.result.sessionId,
    prompt: 'ACP common mission sample',
    adapter: 'fake-process',
  },
})
assert.ok(acpPrompt.result.runId)
acp.close()

const a2aPort = 8794
const a2a = spawn(
  exe('crony-a2a'),
  [
    ...commonArgs,
    '--bind',
    `127.0.0.1:${a2aPort}`,
    '--public-url',
    `http://127.0.0.1:${a2aPort}`,
  ],
  { cwd: root, stdio: ['ignore', 'ignore', 'pipe'] },
)
try {
  const deadline = Date.now() + 10_000
  let card
  while (Date.now() < deadline) {
    try {
      card = await fetch(
        `http://127.0.0.1:${a2aPort}/.well-known/agent-card.json`,
      ).then((response) => response.json())
      break
    } catch {
      await new Promise((resolve) => setTimeout(resolve, 100))
    }
  }
  assert.equal(card.protocolVersion, '1.0')
  assert.equal(card.capabilities.streaming, true)
  const sent = await fetch(`http://127.0.0.1:${a2aPort}/a2a`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: 1,
      method: 'message/send',
      params: { protocolVersion: '1.0', message: { parts: [{ text: 'A2A mission' }] } },
    }),
  }).then((response) => response.json())
  assert.ok(sent.result.mission_id)
  const streamed = await fetch(`http://127.0.0.1:${a2aPort}/a2a/stream`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: 2,
      method: 'tasks/get',
      params: { protocolVersion: '1.0' },
    }),
  }).then((response) => response.text())
  assert.ok(streamed.includes('status-update'))
  assert.ok(streamed.includes('artifact-update'))

  const report = {
    checked_at: new Date().toISOString(),
    mcp_protocol_version: mcpInit.result.protocolVersion,
    mcp_tool_count: mcpTools.result.tools.length,
    acp_protocol_version: acpInit.result.protocolVersion,
    acp_session_created: true,
    a2a_protocol_version: card.protocolVersion,
    a2a_streaming: card.capabilities.streaming,
  }
  await writeFile(
    path.join(root, 'output', 'e2e-gateways.json'),
    `${JSON.stringify(report, null, 2)}\n`,
  )
  console.log(JSON.stringify(report, null, 2))
} finally {
  a2a.kill()
}
