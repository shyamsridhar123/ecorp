import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { createServer } from 'node:http'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'

const script = path.join(import.meta.dirname, 'e2e_external_adapters.mjs')

// These are HTTP-contract regressions for the E2E driver, not native provider
// acceptance. The complete integration job still exercises the real runner.
async function runFixture(platform, {
  status = 409,
  reason,
  orphanRun = false,
  reportedOS = { win32: 'windows', linux: 'linux', darwin: 'macos' }[platform],
  holdLaunch = false,
  timeoutMs = 10_000,
} = {}) {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'ecorp-external-contract-'))
  const report = path.join(directory, 'report.json')
  const tasks = []
  const runs = []
  const calls = []
  const server = createServer(async (request, response) => {
    let text = ''
    for await (const chunk of request) text += chunk
    const body = text ? JSON.parse(text) : {}
    calls.push({ method: request.method, url: request.url })
    response.setHeader('content-type', 'application/json')
    if (request.url === '/api/demo/reset') {
      response.end(JSON.stringify({ corp_id: 'fixture', alice_actor_id: 'alice' }))
    } else if (request.url === '/api/corps/fixture/missions') {
      const id = body.preferred_adapter
      tasks.push({ id: `task-${id}`, mission_id: `mission-${id}`, status: 'ready' })
      response.end(JSON.stringify({ mission_id: `mission-${id}` }))
    } else if (request.url.endsWith('/launch')) {
      if (holdLaunch) return
      const task = tasks.at(-1)
      const adapter = task.id.slice('task-'.length)
      if (orphanRun) runs.push({ id: 'orphan', task_id: task.id, status: 'starting' })
      response.statusCode = status
      response.end(JSON.stringify(status === 200
        ? { run_id: 'unexpected-run' }
        : { error: reason ?? `mission dispatch incomplete (0 new runs dispatched): task ${task.id} requires adapter ${adapter}, but that adapter is unavailable` }))
    } else if (request.url.startsWith('/api/corps/fixture/snapshot?')) {
      response.end(JSON.stringify({
        snapshot: { tasks, runs, events: [] },
        runners: [{ id: 'fixture-runner', os: reportedOS, connected: true }],
      }))
    } else {
      response.statusCode = 404
      response.end(JSON.stringify({ error: 'unexpected fixture request' }))
    }
  })
  let child
  let timer
  let closed
  let childClosed = false
  async function waitForClose(timeout) {
    let deadline
    try {
      return await Promise.race([
        closed.then(() => true),
        new Promise((resolve) => { deadline = setTimeout(() => resolve(false), timeout) }),
      ])
    } finally {
      clearTimeout(deadline)
    }
  }
  try {
    await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
    child = spawn(process.execPath, [script], {
      cwd: directory,
      windowsHide: true,
      env: {
        PATH: process.env.PATH,
        ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}),
        CRONY_SERVER_HTTP: `http://127.0.0.1:${server.address().port}`,
        CRONY_TEST_RUNNER_PLATFORM: platform,
        CRONY_EXTERNAL_ADAPTER_REPORT: report,
      },
    })
    closed = new Promise((resolve) => child.once('close', (code) => {
      childClosed = true
      resolve(code)
    }))
    let stdout = ''
    let stderr = ''
    child.stdout.setEncoding('utf8').on('data', (chunk) => { stdout += chunk })
    child.stderr.setEncoding('utf8').on('data', (chunk) => { stderr += chunk })
    const code = await new Promise((resolve, reject) => {
      timer = setTimeout(() => {
        reject(new Error('owned HTTP-contract child timed out'))
      }, timeoutMs)
      child.once('error', reject)
      closed.then(resolve)
    })
    const bytes = await readFile(report, 'utf8').catch((error) => {
      if (error.code === 'ENOENT') return null
      throw error
    })
    return { code, stdout, stderr, calls, report: bytes ? JSON.parse(bytes) : null }
  } finally {
    clearTimeout(timer)
    let cleanupError
    if (child && !childClosed) {
      child.kill()
      if (!await waitForClose(2000)) {
        child.kill('SIGKILL')
        if (!await waitForClose(2000)) {
          cleanupError = new Error(`owned child did not terminate; retained ${directory}`)
        }
      }
    }
    server.closeAllConnections()
    await new Promise((resolve) => server.close(resolve))
    if (cleanupError) throw cleanupError
    assert.ok(!child || childClosed, 'child close must precede directory cleanup')
    // Only this test's unique temporary directory is eligible for cleanup.
    assert.equal(path.dirname(directory), path.resolve(os.tmpdir()))
    assert.ok(path.basename(directory).startsWith('ecorp-external-contract-'))
    await rm(directory, { recursive: true, force: true })
  }
}

for (const platform of ['linux', 'darwin']) {
  test(`${platform}: native unavailable admission is a verified refusal, not provider success`, async () => {
    const result = await runFixture(platform)
    assert.equal(result.code, 0, result.stderr)
    assert.equal(result.report.runner_platform, platform)
    assert.equal(result.report.common_sample, false)
    assert.equal(result.report.execution_supported, false)
    assert.deepEqual(result.report.providers.map((provider) => ({
      adapter: provider.adapter,
      dispatched: provider.dispatched,
      launch_status: provider.launch_status,
    })), [
      { adapter: 'claude-code', dispatched: false, launch_status: 409 },
      { adapter: 'opencode', dispatched: false, launch_status: 409 },
    ])
    assert.equal(result.calls.filter((call) => call.url.endsWith('/launch')).length, 2)
  })
}

test('Windows: an unavailable provider remains a failure', async () => {
  const result = await runFixture('win32')
  assert.notEqual(result.code, 0)
  assert.match(result.stderr, /adapter claude-code.*unavailable/)
  assert.equal(result.report, null)
})

test('Unix: accidentally accepting an unsupported provider fails the contract', async () => {
  const result = await runFixture('linux', { status: 200 })
  assert.notEqual(result.code, 0)
  assert.match(result.stderr, /must refuse Unix dispatch/)
  assert.equal(result.report, null)
})

test('Unix: an unrelated admission failure cannot stand in for containment refusal', async () => {
  const result = await runFixture('linux', { reason: 'actor is not authorized' })
  assert.notEqual(result.code, 0)
  assert.match(result.stderr, /unexpected claude-code refusal/)
  assert.equal(result.report, null)
})

test('Unix: a refusal with a persisted run is rejected', async () => {
  const result = await runFixture('linux', { orphanRun: true })
  assert.notEqual(result.code, 0)
  assert.match(result.stderr, /unsupported adapter dispatch must not persist a run/)
  assert.equal(result.report, null)
})

test('Unknown runner platforms fail before resetting a fixture', async () => {
  const result = await runFixture('unknown')
  assert.notEqual(result.code, 0)
  assert.match(result.stderr, /unsupported external-adapter test runner platform/)
  assert.equal(result.calls.length, 0)
  assert.equal(result.report, null)
})

test('A Linux expectation cannot conceal an unavailable Windows runner', async () => {
  const result = await runFixture('linux', { reportedOS: 'windows' })
  assert.notEqual(result.code, 0)
  assert.match(result.stderr, /does not match the connected runner platform/)
  assert.equal(result.calls.filter((call) => call.url.endsWith('/missions')).length, 0)
  assert.equal(result.report, null)
})

test('Unknown reported runner OS fails before creating provider work', async () => {
  const result = await runFixture('linux', { reportedOS: 'unknown' })
  assert.notEqual(result.code, 0)
  assert.match(result.stderr, /unsupported connected runner OS/)
  assert.equal(result.calls.filter((call) => call.url.endsWith('/missions')).length, 0)
})

test('An owned child stuck on HTTP is reaped before fixture cleanup', async () => {
  await assert.rejects(
    runFixture('linux', { holdLaunch: true, timeoutMs: 2000 }),
    /owned HTTP-contract child timed out/,
  )
})
