import { execFile as execFileCallback, spawn } from 'node:child_process'
import {
  closeSync, lstatSync, openSync, readFileSync, readdirSync, readlinkSync,
  realpathSync, statSync, writeFileSync,
} from 'node:fs'
import path from 'node:path'
import { promisify } from 'node:util'

const execFile = promisify(execFileCallback)
const manualPorts = new Set(['8791', '8793', '5187', '5291', '15191', '15193'])
const bootIdPattern = /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/u
const unsigned = /^(?:0|[1-9][0-9]*)$/u
const ownershipError = () =>
  new Error('QA process ownership changed or is unverifiable; refusing server restart.')

function contextFor({ root, server, binary, platform = process.platform }) {
  if (!['win32', 'linux'].includes(platform)) throw ownershipError()
  const paths = platform === 'win32' ? path.win32 : path.posix
  if (![root, server, binary].every((value) => typeof value === 'string' && value.trim()) ||
      !paths.isAbsolute(root) || !paths.isAbsolute(binary)) throw ownershipError()
  let endpoint
  try { endpoint = new URL(server) } catch { throw ownershipError() }
  if (!['http:', 'https:'].includes(endpoint.protocol) ||
      !['127.0.0.1', 'localhost', '[::1]'].includes(endpoint.hostname) ||
      !endpoint.port || Number(endpoint.port) <= 0 || manualPorts.has(endpoint.port) ||
      endpoint.username || endpoint.password || endpoint.pathname !== '/' ||
      endpoint.search || endpoint.hash) throw ownershipError()
  const normalize = (value) => {
    if (typeof value !== 'string' || !paths.isAbsolute(value)) throw ownershipError()
    const resolved = paths.resolve(value)
    return platform === 'win32' ? resolved.toLowerCase() : resolved
  }
  return { root, server, binary, platform, endpoint, normalize }
}

function validTicks(value) {
  return typeof value === 'string' && unsigned.test(value) &&
    value.length <= 20 && BigInt(value) <= 18446744073709551615n
}

function assertManifest(manifest, context) {
  const { root, server, binary, platform, normalize } = context
  if (!manifest || Array.isArray(manifest) || manifest.test_owned !== true ||
      manifest.server_url !== server || normalize(manifest.workspace) !== normalize(root) ||
      !Number.isSafeInteger(manifest.server) || manifest.server <= 0 ||
      (manifest.platform !== undefined && manifest.platform !== platform)) throw ownershipError()
  if (platform === 'win32') {
    if (typeof manifest.server_creation !== 'string' ||
        !Number.isFinite(Date.parse(manifest.server_creation))) throw ownershipError()
  } else if (manifest.platform !== 'linux' || typeof manifest.server_boot_id !== 'string' ||
      !bootIdPattern.test(manifest.server_boot_id ?? '') ||
      !validTicks(manifest.server_start_ticks) ||
      normalize(manifest.server_executable) !== normalize(binary)) throw ownershipError()
}

function assertProcessIdentity(manifest, identity, context) {
  assertManifest(manifest, context)
  const { root, binary, platform, normalize } = context
  if (!identity || identity.platform !== platform || identity.pid !== manifest.server ||
      normalize(identity.executable) !== normalize(binary)) throw ownershipError()
  if (platform === 'win32') {
    if (typeof identity.creation !== 'string' || !Number.isFinite(Date.parse(identity.creation)) ||
        Math.abs(Date.parse(identity.creation) - Date.parse(manifest.server_creation)) > 20) {
      throw ownershipError()
    }
  } else if (identity.boot_id !== manifest.server_boot_id ||
      identity.start_ticks !== manifest.server_start_ticks ||
      normalize(identity.cwd) !== normalize(root) ||
      identity.cwd_matches !== true || identity.root_matches !== true ||
      identity.executable_matches !== true) throw ownershipError()
}

// platform is useful for pure guard tests; effectful entry points always use the host OS.
export function assertOwnedRestart(manifest, identity, options) {
  const context = contextFor(options)
  if (context.platform === 'win32' && identity && typeof identity === 'object') {
    // Existing Windows callers supply only executable/creation/port_owned.
    // Explicit new fields still win and must match; Linux never gets defaults.
    identity = { platform: 'win32', pid: manifest?.server, port: Number(context.endpoint.port), ...identity }
  }
  assertProcessIdentity(manifest, identity, context)
  if (identity.port !== Number(context.endpoint.port) || identity.port_owned !== true ||
      (context.platform === 'linux' &&
       (!Array.isArray(identity.socket_inodes) || identity.socket_inodes.length === 0 ||
        !identity.socket_inodes.every((inode) => validTicks(inode) && inode !== '0')))) {
    throw ownershipError()
  }
}

export function parseOwnedTestServerManifest(text, options) {
  try {
    if (typeof text !== 'string' || text.length > 16_384) throw ownershipError()
    const manifest = JSON.parse(text)
    assertManifest(manifest, contextFor(options))
    return manifest
  } catch { throw ownershipError() }
}

// comm may contain spaces, newlines, and parentheses. Only the last ')' closes it.
export function parseLinuxProcStat(text, expectedPid) {
  if (typeof text !== 'string' || text.length > 4096) throw ownershipError()
  const prefix = /^([1-9][0-9]*) \(/u.exec(text)
  const end = text.lastIndexOf(')')
  const fields = text.slice(end + 1).trim().split(/\s+/u)
  if (!prefix || end < prefix[0].length || text[end + 1] !== ' ' ||
      Number(prefix[1]) !== expectedPid ||
      !Number.isSafeInteger(expectedPid) || expectedPid <= 0 ||
      fields.length < 20 || !/^[RSDZTtWXxKPI]$/u.test(fields[0]) ||
      !fields.slice(1).every((field) => /^-?[0-9]+$/u.test(field)) ||
      !validTicks(fields[19])) throw ownershipError()
  return { pid: expectedPid, state: fields[0], start_ticks: fields[19] }
}

export function linuxListeningSocketInodes({ tcp, tcp6, socketLinks, port }) {
  if (!Number.isSafeInteger(port) || port < 1 || port > 65535 ||
      !Array.isArray(socketLinks)) throw ownershipError()
  const owned = new Set(socketLinks.flatMap((link) => {
    if (typeof link !== 'string') throw ownershipError()
    const match = /^socket:\[([1-9][0-9]*)\]$/u.exec(link)
    if (link.startsWith('socket:') && (!match || !validTicks(match[1]))) throw ownershipError()
    return match ? [match[1]] : []
  }))
  const found = new Set()
  for (const [table, width] of [[tcp, 8], [tcp6, 32]]) {
    if (typeof table !== 'string' || table.length > 2 * 1024 * 1024) throw ownershipError()
    const [header, ...rows] = table.trim().split('\n')
    if (!/\blocal_address\b/u.test(header) || !/\bst\b/u.test(header) ||
        !/\binode\b/u.test(header)) throw ownershipError()
    const address = new RegExp(`^[0-9a-f]{${width}}:([0-9a-f]{4})$`, 'iu')
    for (const row of rows) {
      if (!row.trim()) continue
      const fields = row.trim().split(/\s+/u)
      const local = address.exec(fields[1] ?? '')
      if (fields.length < 10 || !/^[0-9]+:$/u.test(fields[0]) ||
          !local || !address.test(fields[2]) || !/^[0-9a-f]{2}$/iu.test(fields[3]) ||
          !validTicks(fields[9])) throw ownershipError()
      if (fields[3].toUpperCase() === '0A' && parseInt(local[1], 16) === port &&
          owned.has(fields[9])) found.add(fields[9])
    }
  }
  return [...found].sort()
}

async function windowsIdentity(pid, port) {
  const script = [
    "$ErrorActionPreference = 'Stop'",
    '$processId = [int]$env:ECORP_QA_PROCESS_ID',
    '$port = [int]$env:ECORP_QA_PROCESS_PORT',
    '$process = Get-CimInstance Win32_Process -Filter "ProcessId = $processId" -Property ProcessId,ExecutablePath,CreationDate',
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
  return { ...JSON.parse(stdout), platform: 'win32', pid, port: Number(port) }
}

function linuxIdentity(pid, port, binary, root) {
  const proc = `/proc/${pid}`
  const readStat = () => parseLinuxProcStat(readFileSync(`${proc}/stat`, 'utf8'), pid)
  const before = readStat()
  const boot = readFileSync('/proc/sys/kernel/random/boot_id', 'utf8').trim()
  const executable = realpathSync(`${proc}/exe`)
  const cwd = realpathSync(`${proc}/cwd`)
  const sameFile = (left, right) => left.dev === right.dev && left.ino === right.ino
  const executableStat = statSync(`${proc}/exe`, { bigint: true })
  const expectedExecutable = statSync(binary, { bigint: true })
  const processRoot = statSync(`${proc}/root`, { bigint: true })
  const hostRoot = statSync('/', { bigint: true })
  const cwdMatches = sameFile(statSync(`${proc}/cwd`, { bigint: true }), statSync(root, { bigint: true }))
  const descriptors = readdirSync(`${proc}/fd`)
  if (descriptors.length > 4096) throw ownershipError()
  const socketLinks = []
  for (const fd of descriptors) {
    if (!/^[0-9]+$/u.test(fd)) throw ownershipError()
    try {
      const link = readlinkSync(`${proc}/fd/${fd}`)
      if (link.startsWith('socket:[')) socketLinks.push(link)
    } catch (error) {
      // A concurrently closed descriptor supplies no ownership evidence.
      if (error.code !== 'ENOENT') throw error
    }
  }
  const socketInodes = linuxListeningSocketInodes({
    tcp: readFileSync(`${proc}/net/tcp`, 'utf8'),
    tcp6: readFileSync(`${proc}/net/tcp6`, 'utf8'),
    socketLinks,
    port: Number(port),
  })
  const after = readStat()
  if (!bootIdPattern.test(boot) || before.start_ticks !== after.start_ticks ||
      /[ZXx]/u.test(after.state) ||
      boot !== readFileSync('/proc/sys/kernel/random/boot_id', 'utf8').trim() ||
      executable !== realpathSync(`${proc}/exe`) || cwd !== realpathSync(`${proc}/cwd`) ||
      !sameFile(executableStat, statSync(`${proc}/exe`, { bigint: true }))) throw ownershipError()
  // Recheck the descriptor links after the TCP-table read, not only the PID stamp.
  const stillOwned = new Set()
  for (const fd of descriptors) {
    try { stillOwned.add(readlinkSync(`${proc}/fd/${fd}`)) } catch (error) {
      if (error.code !== 'ENOENT') throw error
    }
  }
  const retainedInodes = socketInodes.filter((inode) => stillOwned.has(`socket:[${inode}]`))
  const final = readStat()
  if (final.start_ticks !== before.start_ticks || /[ZXx]/u.test(final.state)) throw ownershipError()
  return {
    platform: 'linux', pid, port: Number(port), executable, cwd,
    boot_id: boot, start_ticks: after.start_ticks,
    executable_matches: expectedExecutable.isFile() && sameFile(executableStat, expectedExecutable),
    cwd_matches: cwdMatches,
    root_matches: sameFile(processRoot, hostRoot),
    socket_inodes: retainedInodes, port_owned: retainedInodes.length > 0,
  }
}

async function serverIdentity(pid, port, context) {
  if (!Number.isSafeInteger(pid) || pid <= 0) throw ownershipError()
  if (context.platform === 'win32') return windowsIdentity(pid, port)
  if (context.platform === 'linux') return linuxIdentity(pid, port, context.binary, context.root)
  throw ownershipError()
}

function effectContext({ root, server, binary }) {
  let context = contextFor({ root, server, binary })
  if (context.platform === 'linux') {
    context = contextFor({ root: realpathSync(root), server, binary: realpathSync(binary) })
  }
  return context
}

function manifestFor(identity, context, previous = {}) {
  return {
    ...previous,
    test_owned: true, workspace: context.root, server_url: context.server,
    platform: context.platform, server: identity.pid, server_executable: identity.executable,
    ...(context.platform === 'win32'
      ? { server_creation: identity.creation }
      : { server_boot_id: identity.boot_id, server_start_ticks: identity.start_ticks }),
  }
}

function writeManifest(pidPath, manifest) {
  try {
    if (!lstatSync(pidPath).isFile()) throw ownershipError()
  } catch (error) {
    if (error.code !== 'ENOENT') throw error
  }
  writeFileSync(pidPath, `${JSON.stringify(manifest, null, 2)}\n`, { mode: 0o600 })
}

// Capture only after the caller's initial health gate; never starts or stops a process.
export async function captureOwnedTestServerManifest({ root, server, binary, pidPath, pid }) {
  const context = effectContext({ root, server, binary })
  if (typeof pidPath !== 'string' || !path.isAbsolute(pidPath)) throw ownershipError()
  const identity = await serverIdentity(pid, context.endpoint.port, context)
  const manifest = manifestFor(identity, context)
  assertOwnedRestart(manifest, identity, context)
  writeManifest(pidPath, manifest)
  return manifest
}

export function ownedServerEnvironment(environment = {}, databaseUrl, inherited = process.env) {
  const bounds = {
    CRONY_ARTIFACT_RECOVERY_GRACE_SECS: [0, 3600],
    CRONY_ARTIFACT_RECOVERY_INTERVAL_SECS: [1, 3600],
  }
  if (!environment || Array.isArray(environment) || typeof environment !== 'object') throw ownershipError()
  for (const [key, value] of Object.entries(environment)) {
    if (!Object.hasOwn(bounds, key) || typeof value !== 'string' || !unsigned.test(value) ||
        Number(value) < bounds[key][0] || Number(value) > bounds[key][1]) throw ownershipError()
  }
  return { ...inherited, ...environment, DATABASE_URL: databaseUrl }
}

function processGone(pid) {
  try { process.kill(pid, 0); return false } catch (error) {
    if (error.code === 'ESRCH') return true
    throw ownershipError()
  }
}

async function waitForExit(manifest, previous, context) {
  const deadline = Date.now() + 10_000
  while (Date.now() < deadline) {
    if (processGone(manifest.server)) return
    try {
      if (context.platform === 'linux') {
        const current = parseLinuxProcStat(readFileSync(`/proc/${manifest.server}/stat`, 'utf8'), manifest.server)
        if (current.start_ticks !== previous.start_ticks ||
            readFileSync('/proc/sys/kernel/random/boot_id', 'utf8').trim() !== previous.boot_id) throw ownershipError()
        if (/[ZXx]/u.test(current.state)) return
      } else {
        assertProcessIdentity(manifest, await serverIdentity(manifest.server, context.endpoint.port, context), context)
      }
    } catch (error) {
      if (processGone(manifest.server)) return
      throw error
    }
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error('Owned server did not exit; preserve the manifest. No force-kill fallback.')
}

export async function restartOwnedTestServer({
  root, server, databaseUrl, binary = process.env.CRONY_TEST_SERVER_BINARY,
  logPrefix = 'owned-server-restart', environment = {},
}) {
  const pidPath = process.env.CRONY_TEST_SERVER_PID_FILE
  if (!pidPath || !binary || !databaseUrl || !/^[a-z0-9-]+$/u.test(logPrefix)) {
    throw new Error('Owned restart requires explicit binary, database, and JSON process manifest.')
  }
  const context = effectContext({ root, server, binary })
  const childEnvironment = ownedServerEnvironment(environment, databaseUrl)
  let manifest
  try {
    const file = lstatSync(pidPath)
    if (!file.isFile() || file.size > 16_384) throw ownershipError()
    manifest = parseOwnedTestServerManifest(readFileSync(pidPath, 'utf8'), context)
  } catch { throw ownershipError() }
  const { endpoint } = context
  const previous = await serverIdentity(manifest.server, endpoint.port, context)
  assertOwnedRestart(manifest, previous, context)
  process.kill(manifest.server)
  await waitForExit(manifest, previous, context)

  const logRoot = path.dirname(path.resolve(pidPath))
  const stdout = openSync(path.join(logRoot, `${logPrefix}.stdout.log`), 'a')
  const stderr = openSync(path.join(logRoot, `${logPrefix}.stderr.log`), 'a')
  const child = spawn(context.binary, ['--bind', `${endpoint.hostname}:${endpoint.port}`], {
    cwd: context.root,
    env: childEnvironment,
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
  const checkpoint = {
    ...manifest,
    test_owned: false,
    previous_server_pid: manifest.server,
    previous_server_creation: previous.creation,
    previous_server_boot_id: previous.boot_id,
    previous_server_start_ticks: previous.start_ticks,
    server: child.pid,
  }
  // An interrupted capture retains the new PID but grants no restart authority.
  writeManifest(pidPath, checkpoint)
  const started = await serverIdentity(child.pid, endpoint.port, context)
  const next = manifestFor(started, context, checkpoint)
  assertProcessIdentity(next, started, context)
  writeManifest(pidPath, next)
  const deadline = Date.now() + 60_000
  while (Date.now() < deadline) {
    let ready = false
    try {
      const response = await fetch(new URL('/health', server), { signal: AbortSignal.timeout(3000) })
      const health = await response.json()
      ready = response.ok && health.status === 'ok' && health.runners >= 1
    } catch {
      // Reconnect is bounded; no unverified process is killed on timeout.
    }
    if (ready) {
      const current = await serverIdentity(child.pid, endpoint.port, context)
      assertOwnedRestart(next, current, context)
      return child.pid
    }
    await new Promise((resolve) => setTimeout(resolve, 200))
  }
  throw new Error('Owned server/runner did not reconnect; preserve the process manifest and logs.')
}
