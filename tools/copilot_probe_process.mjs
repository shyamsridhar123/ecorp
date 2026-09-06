// Test-only process boundary observer. Never forward a real token into this
// probe: the inherited credential below is a public, synthetic test marker.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { appendFileSync } from 'node:fs'

const canary = 'ECORP_CREDENTIAL_CANARY_MUST_NOT_REACH_COPILOT'
const [role, binary, ...args] = process.argv.slice(2)
assert.ok(['runner', 'copilot'].includes(role))
assert.ok(binary)
assert.ok(process.env.ECORP_COPILOT_ENV_OBSERVATIONS)
assert.ok(process.env.ECORP_COPILOT_PROBE_ID)
const canaryPresent = Object.values(process.env).some(
  (value) => value?.includes(canary),
)
const expected = role === 'runner'
appendFileSync(
  process.env.ECORP_COPILOT_ENV_OBSERVATIONS,
  `${JSON.stringify({
    kind: `${role}_environment`,
    probe_id: process.env.ECORP_COPILOT_PROBE_ID,
    pid: process.pid,
    parent_pid: process.ppid,
    canary_present: canaryPresent,
    github_token_present: Object.hasOwn(process.env, 'GITHUB_TOKEN'),
  })}\n`,
)
assert.equal(canaryPresent, expected, `${role} canary precondition failed`)
assert.ok(
  expected ? process.env.GITHUB_TOKEN === canary : !Object.hasOwn(process.env, 'GITHUB_TOKEN'),
  `${role} inherited credential boundary failed`,
)
const child = spawn(binary, args, { stdio: 'inherit', windowsHide: true })
appendFileSync(
  process.env.ECORP_COPILOT_ENV_OBSERVATIONS,
  `${JSON.stringify({
    kind: `${role}_spawn`,
    probe_id: process.env.ECORP_COPILOT_PROBE_ID,
    pid: process.pid,
    child_pid: child.pid,
    binary,
  })}\n`,
)
child.on('error', (error) => {
  console.error(error.message)
  process.exitCode = 1
})
child.on('exit', (code) => {
  appendFileSync(
    process.env.ECORP_COPILOT_ENV_OBSERVATIONS,
    `${JSON.stringify({
      kind: `${role}_exit`,
      probe_id: process.env.ECORP_COPILOT_PROBE_ID,
      pid: process.pid,
      child_pid: child.pid,
      code,
    })}\n`,
  )
  process.exitCode = code ?? 1
  if (process.connected) process.disconnect()
})
process.on('message', (message) => {
  if (message?.type === 'stop' && child.exitCode === null) child.kill()
})
for (const signal of ['SIGTERM', 'SIGINT']) {
  process.on(signal, () => { child.kill(signal) })
}
