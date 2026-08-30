#!/usr/bin/env node

import { randomUUID } from 'node:crypto'

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
const sessionFlag = provider === 'claude' ? '--resume' : '--session'
const sessionIndex = process.argv.indexOf(sessionFlag)
const sessionId =
  sessionIndex >= 0 ? process.argv[sessionIndex + 1] : `${provider}-${randomUUID()}`

process.stdout.write(`${JSON.stringify({ session_id: sessionId })}\n`)
process.stdout.write(
  `${JSON.stringify({ text: `${provider} normalized provider output` })}\n`,
)
process.stdout.write(
  `${JSON.stringify({
    usage: {
      input_tokens: 100,
      output_tokens: 40,
      cost_microusd: 200,
    },
  })}\n`,
)
if (slow) {
  await new Promise((resolve) => setTimeout(resolve, 10_000))
}
process.stdout.write(`${JSON.stringify({ type: 'result', text: 'completed' })}\n`)
