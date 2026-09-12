import { execFile as execFileCallback, spawn } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import {
  closeSync, existsSync, fstatSync, lstatSync, openSync, readFileSync, realpathSync,
  renameSync, unlinkSync, writeFileSync,
} from 'node:fs'
import net from 'node:net'
import path from 'node:path'
import { promisify } from 'node:util'

const execFile = promisify(execFileCallback)
const refusal = 'QA process ownership changed or is unverifiable; refusing server restart.'
const manualPorts = new Set(['5432', '54329', '8791', '8793', '5187', '5291', '15191', '15193'])
const pause = ms => new Promise(resolve => setTimeout(resolve, ms))

export function assertTestEndpoint(server) {
  const endpoint = new URL(server)
  if (endpoint.protocol !== 'http:' ||
    !['127.0.0.1', 'localhost', '[::1]'].includes(endpoint.hostname) ||
    !endpoint.port || manualPorts.has(endpoint.port) ||
    endpoint.pathname !== '/' || endpoint.search || endpoint.hash || endpoint.username || endpoint.password) {
    throw new Error(refusal)
  }
  return endpoint
}

export function assertOwnedRestart(manifest, identity, { root, server, binary, platform = process.platform }) {
  assertTestEndpoint(server)
  const paths = platform === 'win32' ? path.win32 : path.posix
  const samePath = (left, right) => typeof left === 'string' && typeof right === 'string' &&
    (platform === 'win32'
      ? paths.resolve(left).toLowerCase() === paths.resolve(right).toLowerCase()
      : paths.resolve(left) === paths.resolve(right))
  if (!manifest || !identity || !['win32', 'linux'].includes(platform) ||
    manifest.test_owned !== true || manifest.server_url !== server ||
    !samePath(manifest.workspace, root) ||
    !Number.isSafeInteger(manifest.server) || manifest.server <= 1 ||
    identity.pid !== manifest.server || identity.platform !== platform ||
    !Number.isFinite(Date.parse(manifest.server_creation)) ||
    !Number.isFinite(Date.parse(identity.creation)) ||
    !samePath(identity.executable, binary) ||
    Math.abs(Date.parse(identity.creation) - Date.parse(manifest.server_creation)) > 20 ||
    identity.port_owned !== true ||
    (identity.port !== undefined && identity.port !== Number(new URL(server).port)) ||
    (manifest.server_state !== undefined && manifest.server_state !== 'running')) {
    throw new Error(refusal)
  }
  if (platform === 'linux') {
    // Wall-clock timestamps alone are not process identity. Match boot ID and
    // kernel start ticks exactly, and never case-fold a Linux executable path.
    const recorded = manifest.server_identity
    const fields = ['platform', 'pid', 'executable', 'cwd', 'uid', 'creation',
      'boot_id', 'start_ticks', 'network_namespace']
    if (!recorded || fields.some(field => recorded[field] !== identity[field]) ||
      !samePath(identity.cwd, root) || !Number.isSafeInteger(identity.uid) || identity.uid < 0 ||
      !/^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$/u.test(identity.boot_id ?? '') ||
      !/^[0-9]+$/u.test(identity.start_ticks ?? '') || !/^net:\[[0-9]+\]$/u.test(identity.network_namespace ?? '')) {
      throw new Error(refusal)
    }
  }
}

async function linuxProcess(request) {
  // CPython's stdlib exposes pidfd_open/pidfd_send_signal; Node 22 does not.
  // No shell, environment dump, credential argument, or numeric kill fallback.
  const operation = execFile('python3', ['-B', path.join(import.meta.dirname, 'owned_test_process_linux.py')], {
    timeout: 40_000, maxBuffer: 64 * 1024, windowsHide: true,
  })
  operation.child.stdin.end(JSON.stringify(request))
  const { stdout } = await operation
  return JSON.parse(stdout)
}

async function serverIdentity(pid, context) {
  if (!Number.isSafeInteger(pid) || pid <= 1) throw new Error(refusal)
  const endpoint = assertTestEndpoint(context.server)
  if (process.platform === 'linux') {
    return linuxProcess({ action: 'inspect', pid, root: context.root, server: context.server, binary: context.binary })
  }
  if (process.platform !== 'win32') throw new Error('Owned test process receipts are supported only on Windows and Linux.')
  const script = [
    "$ErrorActionPreference = 'Stop'",
    '$processId = [int]$env:ECORP_QA_PROCESS_ID',
    '$port = [int]$env:ECORP_QA_PROCESS_PORT',
    '$process = Get-CimInstance Win32_Process -Filter "ProcessId = $processId" -Property ProcessId,ExecutablePath,CreationDate',
    "if (!$process) { throw 'Recorded QA server is absent' }",
    '$listeners = @(Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue)',
    "@{ platform='win32'; pid=$processId; executable=$process.ExecutablePath; creation=$process.CreationDate.ToUniversalTime().ToString('o');",
    'port_owned=[bool]($listeners | Where-Object OwningProcess -eq $processId) } | ConvertTo-Json -Compress',
  ].join('\n')
  const { stdout } = await execFile('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], {
    windowsHide: true, timeout: 20_000,
    env: { ...process.env, ECORP_QA_PROCESS_ID: String(pid), ECORP_QA_PROCESS_PORT: endpoint.port },
  })
  return JSON.parse(stdout)
}

export function ownedServerEnvironment(environment = {}, databaseUrl, inherited = process.env) {
  const bounds = { CRONY_ARTIFACT_RECOVERY_GRACE_SECS: [0, 3600], CRONY_ARTIFACT_RECOVERY_INTERVAL_SECS: [1, 3600] }
  if (!environment || Array.isArray(environment) || typeof environment !== 'object') throw new Error(refusal)
  for (const [key, value] of Object.entries(environment)) {
    if (!Object.hasOwn(bounds, key) || typeof value !== 'string' || !/^(?:0|[1-9][0-9]*)$/u.test(value) ||
      Number(value) < bounds[key][0] || Number(value) > bounds[key][1]) throw new Error(refusal)
  }
  return { ...inherited, ...environment, DATABASE_URL: databaseUrl }
}

function options({ root, server, databaseUrl, binary = process.env.CRONY_TEST_SERVER_BINARY,
  manifestPath = process.env.CRONY_TEST_SERVER_PID_FILE, logPrefix = 'owned-server-restart',
  args = [], environment = {}, minimumRunners = 1 }) {
  if (!manifestPath || !binary || !databaseUrl || !root ||
    !path.isAbsolute(manifestPath) || !path.isAbsolute(binary) || !path.isAbsolute(root) ||
    !/^[a-z0-9-]+$/u.test(logPrefix) || !Array.isArray(args) || args.some(arg => typeof arg !== 'string') ||
    !Number.isSafeInteger(minimumRunners) || minimumRunners < 0) {
    throw new Error('Owned restart requires explicit binary, database, and JSON process manifest.')
  }
  assertTestEndpoint(server)
  ownedServerEnvironment(environment, databaseUrl, {})
  if (!['win32', 'linux'].includes(process.platform)) throw new Error('Unsupported owned test platform.')
  let database
  try { database = new URL(databaseUrl) } catch { throw new Error('Invalid PostgreSQL test database URL; its value was not disclosed.') }
  if (!['postgres:', 'postgresql:'].includes(database.protocol)) throw new Error('Expected an explicit PostgreSQL test database.')
  // Passwords and URL options are deliberately not part of persisted receipts.
  const databaseTarget = { host: database.hostname, port: database.port || '5432',
    database: database.pathname, user: database.username }
  return { root: realpathSync(root), server, binary: realpathSync(binary), databaseUrl,
    databaseTarget, manifestPath: path.resolve(manifestPath), logPrefix, args, environment, minimumRunners }
}

function readManifest(file) {
  const metadata = lstatSync(file)
  if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.size > 64 * 1024) throw new Error(refusal)
  const raw = readFileSync(file, 'utf8')
  const manifest = JSON.parse(raw)
  if (!manifest || typeof manifest !== 'object' || Array.isArray(manifest) || manifest.test_owned !== true ||
    !Number.isSafeInteger(manifest.server) || manifest.server <= 1) throw new Error(refusal)
  return { raw, manifest }
}

function unchanged(context, raw) {
  if (readManifest(context.manifestPath).raw !== raw) throw new Error('Owned process manifest changed; preserve it for inspection.')
}

async function locked(context, action) {
  const lock = `${context.manifestPath}.lock`
  const nonce = JSON.stringify({ owner_pid: process.pid, nonce: randomUUID() })
  const fd = openSync(lock, 'wx', 0o600) // Never adopt or remove a pre-existing lock.
  const owned = fstatSync(fd)
  try {
    writeFileSync(fd, nonce)
    return await action()
  } finally {
    closeSync(fd)
    if (existsSync(lock) && !lstatSync(lock).isSymbolicLink() && lstatSync(lock).ino === owned.ino &&
      readFileSync(lock, 'utf8') === nonce) unlinkSync(lock)
  }
}

function publish(context, manifest, previousRaw) {
  const raw = `${JSON.stringify(manifest, null, 2)}\n`
  if (previousRaw === undefined) {
    writeFileSync(context.manifestPath, raw, { flag: 'wx', mode: 0o600 })
  } else {
    unchanged(context, previousRaw)
    const pending = `${context.manifestPath}.next-${randomUUID()}`
    writeFileSync(pending, raw, { flag: 'wx', mode: 0o600 })
    unchanged(context, previousRaw)
    renameSync(pending, context.manifestPath)
  }
  return raw
}

async function assertPortAvailable(server) {
  const endpoint = assertTestEndpoint(server)
  const probe = net.createServer()
  await new Promise((resolve, reject) => {
    probe.once('error', reject)
    probe.listen({ host: endpoint.hostname.replace(/^\[|\]$/gu, ''), port: Number(endpoint.port), exclusive: true }, resolve)
  })
  await new Promise((resolve, reject) => probe.close(error => error ? reject(error) : resolve()))
}

async function stopVerified(context, manifest, identity) {
  assertOwnedRestart(manifest, identity, context)
  if (process.platform === 'linux') {
    // The helper revalidates the receipt while holding the kernel process handle.
    await linuxProcess({ action: 'stop', pid: manifest.server, expected: identity,
      root: context.root, server: context.server, binary: context.binary })
    return
  }
  const script = [
    "$ErrorActionPreference = 'Stop'",
    '$processId = [int]$env:ECORP_QA_PROCESS_ID',
    '$process = Get-Process -Id $processId -ErrorAction Stop',
    '[void]$process.Handle',
    "if ($process.HasExited) { throw 'Owned server already exited during verification' }",
    '$currentPath = [IO.Path]::GetFullPath($process.Path)',
    '$expectedPath = [IO.Path]::GetFullPath($env:ECORP_QA_PROCESS_EXE)',
    '$currentTicks = $process.StartTime.ToUniversalTime().Ticks',
    '$expectedTicks = ([DateTimeOffset]$env:ECORP_QA_PROCESS_CREATION).UtcTicks',
    "if (!([string]::Equals($currentPath, $expectedPath, [StringComparison]::OrdinalIgnoreCase)) -or $currentTicks -ne $expectedTicks) { throw 'QA process ownership changed or is unverifiable; refusing server restart.' }",
    '$process.Kill()',
    "if (!$process.WaitForExit(30000)) { throw 'Owned server did not exit; no replacement or force-stop was attempted.' }",
  ].join('\n')
  await execFile('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], {
    windowsHide: true, timeout: 40_000,
    env: { ...process.env, ECORP_QA_PROCESS_ID: String(manifest.server),
      ECORP_QA_PROCESS_EXE: identity.executable, ECORP_QA_PROCESS_CREATION: identity.creation },
  })
}

function sameDatabase(context, manifest) {
  if (manifest.database_target && JSON.stringify(manifest.database_target) !== JSON.stringify(context.databaseTarget)) {
    throw new Error('Owned test database target changed; refusing restart.')
  }
  if (process.platform === 'linux' && !manifest.database_target) throw new Error(refusal)
}

async function launch(context, previous, previousRaw) {
  // Windows can report process exit before its listening socket is released.
  // Wait only after our verified stop; never stop or adopt a different listener.
  const releaseDeadline = Date.now() + (previous ? 10_000 : 0)
  for (;;) {
    try { await assertPortAvailable(context.server); break } catch (error) {
      if (error.code !== 'EADDRINUSE' || Date.now() >= releaseDeadline) throw error
      await pause(100)
    }
  }
  if (process.platform === 'linux') await linuxProcess({ action: 'capabilities' })
  const endpoint = assertTestEndpoint(context.server)
  const logRoot = path.dirname(context.manifestPath)
  const stdout = openSync(path.join(logRoot, `${context.logPrefix}.stdout.log`), 'a', 0o600)
  const stderr = openSync(path.join(logRoot, `${context.logPrefix}.stderr.log`), 'a', 0o600)
  const child = spawn(context.binary, [...context.args, '--bind', `${endpoint.hostname}:${endpoint.port}`], {
    cwd: context.root, env: ownedServerEnvironment(context.environment, context.databaseUrl),
    detached: true, windowsHide: true, stdio: ['ignore', stdout, stderr],
  })
  try {
    await new Promise((resolve, reject) => { child.once('spawn', resolve); child.once('error', reject) })
  } finally {
    closeSync(stdout)
    closeSync(stderr)
  }
  child.unref()
  const alive = () => { if (child.exitCode !== null || child.signalCode !== null) throw new Error('Owned test child exited; preserve its logs.') }
  const started = await serverIdentity(child.pid, context)
  alive() // Do not record a new process that reused an already reaped child PID.
  const next = { ...previous, test_owned: true, workspace: context.root, server_url: context.server,
    server: child.pid, server_binary: context.binary, server_creation: started.creation,
    server_identity: started, server_state: 'starting', database_target: context.databaseTarget }
  if (previous) {
    next.previous_server_pid = previous.server
    next.previous_server_creation = previous.server_creation
  }
  let raw = publish(context, next, previousRaw)
  const deadline = Date.now() + 60_000
  while (Date.now() < deadline) {
    alive()
    let healthy = false
    try {
      const response = await fetch(`${context.server}/health`, { redirect: 'error', signal: AbortSignal.timeout(3000) })
      const health = await response.json()
      healthy = response.ok && health.status === 'ok' && Number.isSafeInteger(health.runners) && health.runners >= context.minimumRunners
    } catch { /* Only readiness is retried; ownership errors below are terminal. */ }
    if (healthy) {
      const current = await serverIdentity(child.pid, context)
      alive()
      const running = { ...next, server_state: 'running' }
      assertOwnedRestart(running, current, context)
      running.server_identity = current
      raw = publish(context, running, raw)
      return child.pid
    }
    await pause(200)
  }
  throw new Error('Owned server/runner did not reconnect; preserve the process manifest and logs.')
}

export async function startOwnedTestServer(input) {
  const context = options({ ...input, minimumRunners: input.minimumRunners ?? 0 })
  return locked(context, async () => {
    if (existsSync(context.manifestPath)) throw new Error('A process manifest already exists; refusing adoption or replacement.')
    return launch(context)
  })
}

export async function restartOwnedTestServer(input) {
  const context = options(input)
  return locked(context, async () => {
    const { manifest, raw } = readManifest(context.manifestPath)
    sameDatabase(context, manifest)
    const identity = await serverIdentity(manifest.server, context)
    assertOwnedRestart(manifest, identity, context)
    unchanged(context, raw)
    await stopVerified(context, manifest, identity)
    const stopped = { ...manifest, server_state: 'stopped' }
    const stoppedRaw = publish(context, stopped, raw)
    return launch(context, stopped, stoppedRaw)
  })
}

export async function stopOwnedTestServer(input) {
  const context = options(input)
  return locked(context, async () => {
    const { manifest, raw } = readManifest(context.manifestPath)
    sameDatabase(context, manifest)
    if (manifest.server_state === 'stopped') return false
    const identity = await serverIdentity(manifest.server, context)
    assertOwnedRestart(manifest, identity, context)
    unchanged(context, raw)
    await stopVerified(context, manifest, identity)
    publish(context, { ...manifest, server_state: 'stopped' }, raw)
    return true
  })
}
