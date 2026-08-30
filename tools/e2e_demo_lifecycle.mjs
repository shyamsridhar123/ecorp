import assert from 'node:assert/strict'
import { writeFile } from 'node:fs/promises'
import path from 'node:path'

const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const root = path.resolve(import.meta.dirname, '..')

async function post(endpoint) {
  const response = await fetch(`${server}${endpoint}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: '{}',
  })
  const body = await response.json()
  if (!response.ok) {
    throw new Error(`${endpoint} failed: ${response.status} ${JSON.stringify(body)}`)
  }
  return body
}

const calls = Array.from({ length: 12 }, (_, index) =>
  post(index % 2 === 0 ? '/api/demo/bootstrap' : '/api/demo/reset'),
)
const results = await Promise.all(calls)
const finalState = await post('/api/demo/reset')
assert.ok(results.every((result) => result.corp_id === finalState.corp_id))

const report = {
  checked_at: new Date().toISOString(),
  concurrent_calls: results.length,
  status: 'ok',
  corp_id: finalState.corp_id,
}
await writeFile(
  path.join(root, 'output', 'e2e-demo-lifecycle.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
