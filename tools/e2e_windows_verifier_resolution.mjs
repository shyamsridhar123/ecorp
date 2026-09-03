import assert from 'node:assert/strict'
import { execFile as execFileCallback } from 'node:child_process'
import { promisify } from 'node:util'
import path from 'node:path'

const execFile = promisify(execFileCallback)
const root = path.resolve(import.meta.dirname, '..')

assert.equal(
  process.platform,
  'win32',
  'Windows verifier resolution regression must run on Windows',
)

const testName =
  'verifier::tests::windows_installed_npm_shim_runs_incident_command_policy'
const { stdout, stderr } = await execFile(
  process.env.CARGO ?? 'cargo',
  [
    'test',
    '-p',
    'crony-runner',
    testName,
    '--',
    '--exact',
    '--nocapture',
  ],
  {
    cwd: root,
    windowsHide: true,
    maxBuffer: 4 * 1024 * 1024,
  },
)

assert.match(stdout + stderr, /test result: ok\./)
assert.match(stdout + stderr, /1 passed/)

process.stdout.write(
  `${JSON.stringify(
    {
      platform: process.platform,
      policy: {
        program: 'npm',
        args: ['--prefix', 'scenarios/incident-command', 'test'],
      },
      candidate_path: 'crony-runner::verifier',
      status: 'passed',
    },
    null,
    2,
  )}\n`,
)
