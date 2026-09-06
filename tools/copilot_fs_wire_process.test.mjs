import assert from 'node:assert/strict'
import test from 'node:test'
import { spawn } from 'node:child_process'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'

test('optional wire observation forwards exact bytes and drains the child', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'ecorp-fs-wire-'))
  const log = path.join(root, 'observations.jsonl')
  const receipt = path.join(root, 'received.bin')
  const fixture = path.join(root, 'fixture.cjs')
  const body = Buffer.from(JSON.stringify({ id: 7, method: 'sessionFs.stat', params: { path: 'private-seed' } }))
  const request = Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`), body])
  const responseBody = Buffer.from(JSON.stringify({
    id: 7, result: { isFile: true, isDirectory: false, privateData: 'never-log-this-данные' },
  }))
  const response = Buffer.concat([Buffer.from(`Content-Length: ${responseBody.length}\r\n\r\n`), responseBody])
  await writeFile(fixture, `const fs=require('fs');process.stdout.write(Buffer.from(${JSON.stringify(request.toString('base64'))},'base64'));let chunks=[];process.stdin.on('data',c=>{chunks.push(c);let b=Buffer.concat(chunks);if(b.length>=${response.length}){fs.writeFileSync(${JSON.stringify(receipt)},b);process.stdin.pause();process.exitCode=0;}});`)
  const env = { ...process.env }
  delete env.GITHUB_TOKEN
  env.ECORP_COPILOT_ENV_OBSERVATIONS = log
  env.ECORP_COPILOT_PROBE_ID = 'wire-test'
  env.ECORP_COPILOT_FS_WIRE = '1'
  const child = spawn(process.execPath, [
    path.join(import.meta.dirname, 'copilot_probe_process.mjs'),
    'copilot', process.execPath, fixture, '--no-auto-update',
  ], { env, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] })
  const output = []
  const errors = []
  let requestReady
  const ready = new Promise((resolve) => { requestReady = resolve })
  const closed = new Promise((resolve, reject) => {
    child.on('error', reject)
    child.on('close', (code) => resolve(code))
  })
  child.stdout.on('data', (chunk) => {
    output.push(chunk)
    if (Buffer.concat(output).length >= request.length) requestReady()
  })
  child.stderr.on('data', (chunk) => errors.push(chunk))
  const timer = setTimeout(() => { child.kill(); }, 10_000)
  try {
    await Promise.race([ready, closed.then(() => { throw new Error('fixture closed before request') })])
    child.stdin.write(response.subarray(0, 19))
    child.stdin.end(response.subarray(19))
    assert.equal(await closed, 0, Buffer.concat(errors).toString())
    assert.deepEqual(Buffer.concat(output), request)
    assert.deepEqual(await readFile(receipt), response)
    const observed = await readFile(log, 'utf8')
    assert.ok(!observed.includes('private-seed'))
    assert.ok(!observed.includes('never-log-this'))
    assert.ok(!observed.includes('данные'))
    assert.ok(observed.split('\n').filter(Boolean).map((line) => JSON.parse(line))
      .some((row) => row.kind === 'copilot_fs_wire' && row.phase === 'response' && row.is_file_camel === true))
    assert.ok(observed.split('\n').filter(Boolean).map((line) => JSON.parse(line))
      .some((row) => row.kind === 'copilot_environment' && row.auto_update_disabled_by_flag === true))
  } finally {
    clearTimeout(timer)
    if (child.exitCode === null) child.kill()
    await closed
    assert.ok(root.startsWith(path.join(os.tmpdir(), 'ecorp-fs-wire-')))
    await rm(root, { recursive: true, force: true })
  }
})
