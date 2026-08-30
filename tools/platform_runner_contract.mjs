import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { spawn } from 'node:child_process'

const root = path.resolve(import.meta.dirname, '..')
const temp = await mkdtemp(path.join(os.tmpdir(), 'crony runner ü space '))
const runId = crypto.randomUUID()
const script = path.join(root, 'scripts', 'fake-agent.mjs')

try {
  const child = spawn(process.execPath, [
    script,
    '--run-id',
    runId,
    '--workdir',
    temp,
    '--mission',
    'cross-platform artifact and stdin/stdout contract',
  ])
  let stdout = ''
  let stderr = ''
  child.stdout.setEncoding('utf8')
  child.stderr.setEncoding('utf8')
  child.stdout.on('data', (chunk) => {
    stdout += chunk
  })
  child.stderr.on('data', (chunk) => {
    stderr += chunk
  })
  const exitCode = await new Promise((resolve, reject) => {
    child.on('error', reject)
    child.on('exit', resolve)
  })
  assert.equal(exitCode, 0, stderr)
  const artifact = await readFile(path.join(temp, 'result.md'))
  const report = {
    checked_at: new Date().toISOString(),
    os: process.platform,
    arch: process.arch,
    node: process.version,
    path_with_spaces_and_unicode: temp,
    structured_stdio: stdout.includes('"type":"completed"'),
    artifact_bytes: artifact.length,
    artifact_sha256: createHash('sha256').update(artifact).digest('hex'),
    pty_requirement:
      'not required by current structured-stdio adapters; interactive PTY adapters remain future work',
  }
  const output = path.join(root, 'output', 'platform')
  await mkdir(output, { recursive: true })
  const file = path.join(output, `${process.platform}-${process.arch}.json`)
  await writeFile(file, `${JSON.stringify(report, null, 2)}\n`)
  await writeFile(
    `${file}.sha256`,
    `${createHash('sha256').update(JSON.stringify(report)).digest('hex')}  ${path.basename(file)}\n`,
  )
  console.log(JSON.stringify(report, null, 2))
} finally {
  await rm(temp, { recursive: true, force: true })
}
