import { execFile as execFileCallback, spawn } from 'node:child_process'
import { closeSync, openSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { promisify } from 'node:util'

const execFile = promisify(execFileCallback)
const manualPorts = new Set(['8791', '8793', '5187', '5291', '15191', '15193'])

export function assertOwnedRestart(manifest, identity, { root, server, binary }) {
  const endpoint = new URL(server)
  if (
    !['127.0.0.1', 'localhost', '[::1]'].includes(endpoint.hostname) ||
    !endpoint.port || manualPorts.has(endpoint.port) ||
    endpoint.username || endpoint.password ||
    manifest.test_owned !== true ||
    manifest.server_url !== server ||
    path.resolve(manifest.workspace ?? '') !== path.resolve(root) ||
    !Number.isSafeInteger(manifest.server) || manifest.server <= 0 ||
    !Number.isFinite(Date.parse(manifest.server_creation)) ||
    !Number.isFinite(Date.parse(identity.creation)) ||
    path.resolve(identity.executable).toLowerCase() !== path.resolve(binary).toLowerCase() ||
    Math.abs(Date.parse(identity.creation) - Date.parse(manifest.server_creation)) > 20 ||
    !identity.port_owned
  ) {
    throw new Error('QA process ownership changed or is unverifiable; refusing server restart.')
  }
}

async function serverIdentity(pid, port) {
  if (process.platform !== 'win32') {
    throw new Error('The owned restart drill requires Windows process/listener receipts.')
  }
  const script = [
    "$ErrorActionPreference = 'Stop'",
    '$processId = [int]$env:ECORP_QA_PROCESS_ID',
    '$port = [int]$env:ECORP_QA_PROCESS_PORT',
    '$process = Get-CimInstance Win32_Process -Filter "ProcessId = $processId"',
    "if (!$process) { throw 'Recorded QA server is absent' }",
    '$listeners = @(Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue)',
    "@{ executable=$process.ExecutablePath; creation=$process.CreationDate.ToUniversalTime().ToString('o');",
    'port_owned=[bool]($listeners | Where-Object OwningProcess -eq $processId) } | ConvertTo-Json -Compress',
  ].join('\n')
  const { stdout } = await execFile(
    'powershell.exe',
    ['-NoProfile', '-NonInteractive', '-Command', script],
    {
      windowsHide: true,
      env: {
        ...process.env,
        ECORP_QA_PROCESS_ID: String(pid),
        ECORP_QA_PROCESS_PORT: String(port),
      },
    },
  )
  return JSON.parse(stdout)
}

export async function restartOwnedTestServer({
  root, server, databaseUrl, binary = process.env.CRONY_TEST_SERVER_BINARY,
  logPrefix = 'owned-server-restart',
}) {
  const pidPath = process.env.CRONY_TEST_SERVER_PID_FILE
  if (!pidPath || !binary || !databaseUrl || !/^[a-z0-9-]+$/u.test(logPrefix)) {
    throw new Error('Owned restart requires explicit binary, database, and JSON process manifest.')
  }
  const manifest = JSON.parse(readFileSync(pidPath, 'utf8'))
  const endpoint = new URL(server)
  const previous = await serverIdentity(manifest.server, endpoint.port)
  assertOwnedRestart(manifest, previous, { root, server, binary })
  process.kill(manifest.server)
  await new Promise((resolve) => setTimeout(resolve, 500))

  const logRoot = path.dirname(path.resolve(pidPath))
  const stdout = openSync(path.join(logRoot, `${logPrefix}.stdout.log`), 'a')
  const stderr = openSync(path.join(logRoot, `${logPrefix}.stderr.log`), 'a')
  const child = spawn(binary, ['--bind', `${endpoint.hostname}:${endpoint.port}`], {
    cwd: root,
    env: { ...process.env, DATABASE_URL: databaseUrl },
    detached: true,
    windowsHide: true,
    stdio: ['ignore', stdout, stderr],
  })
  try {
    await new Promise((resolve, reject) => {
      child.once('spawn', resolve)
      child.once('error', reject)
    })
  } finally {
    closeSync(stdout)
    closeSync(stderr)
  }
  child.unref()
  const started = await serverIdentity(child.pid, endpoint.port)
  const next = {
    ...manifest,
    previous_server_pid: manifest.server,
    previous_server_creation: previous.creation,
    server: child.pid,
    server_creation: started.creation,
  }
  writeFileSync(pidPath, `${JSON.stringify(next, null, 2)}\n`)
  const deadline = Date.now() + 60_000
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${server}/health`, { signal: AbortSignal.timeout(3000) })
      const health = await response.json()
      if (response.ok && health.status === 'ok' && health.runners >= 1) {
        const current = await serverIdentity(child.pid, endpoint.port)
        assertOwnedRestart(next, current, { root, server, binary })
        return child.pid
      }
    } catch {
      // Reconnect is bounded; no unverified process is killed on timeout.
    }
    await new Promise((resolve) => setTimeout(resolve, 200))
  }
  throw new Error('Owned server/runner did not reconnect; preserve the process manifest and logs.')
}
