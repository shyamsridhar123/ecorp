import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { mkdtemp, mkdir, writeFile, readFile, access, realpath, lstat, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { setTimeout as delay } from 'node:timers/promises'
import test from 'node:test'

const directory = path.dirname(fileURLToPath(import.meta.url))
const helper = path.join(directory, 'local_stack_operation.ps1')
const environment = {}
for (const name of [
  'SystemRoot', 'WINDIR', 'ComSpec', 'PATH', 'PATHEXT', 'TEMP', 'TMP',
  'PSModulePath', 'ProgramFiles', 'ProgramFiles(x86)', 'ProgramW6432',
]) {
  if (process.env[name] !== undefined) environment[name] = process.env[name]
}

const fixtureSource = `
param([string]$Helper, [string]$Workspace, [string]$Mode, [string]$Signal, [string]$Release)
$ErrorActionPreference = 'Stop'
. $Helper
try {
    if ($Mode -eq 'hold') {
        Invoke-LocalStackOperation -Workspace $Workspace -WaitSeconds 0 -Action {
            [IO.File]::WriteAllText($Signal, 'entered')
            $deadline = [DateTime]::UtcNow.AddSeconds(20)
            while (![IO.File]::Exists($Release)) {
                if ([DateTime]::UtcNow -gt $deadline) { throw 'Synthetic fixture lease expired.' }
                Start-Sleep -Milliseconds 20
            }
        }
        Write-Output 'released'
    } elseif ($Mode -eq 'nested') {
        Invoke-LocalStackOperation -Workspace $Workspace -WaitSeconds 0 -Action {
            Invoke-LocalStackOperation -Workspace $Workspace -WaitSeconds 0 -Action {
                Write-Output 'nested'
            }
        }
    } elseif ($Mode -eq 'failure') {
        try {
            Invoke-LocalStackOperation -Workspace $Workspace -WaitSeconds 0 -Action {
                throw 'synthetic operation failure'
            }
        } catch {
            if ($_.Exception.Message -ne 'synthetic operation failure') { throw }
        }
        Invoke-LocalStackOperation -Workspace $Workspace -WaitSeconds 0 -Action {
            Write-Output 'after-failure'
        }
    } else {
        Invoke-LocalStackOperation -Workspace $Workspace -WaitSeconds 0 -Action {
            Write-Output 'acquired'
        }
    }
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 2
}
`

async function fixture(t) {
  const root = await mkdtemp(path.join(tmpdir(), 'ecorp-stack-lock-'))
  const workspace = path.join(root, 'workspace [literal] with spaces')
  const other = path.join(root, 'another workspace')
  await mkdir(workspace)
  await mkdir(other)
  const script = path.join(root, 'synthetic operation.ps1')
  await writeFile(script, fixtureSource, 'utf8')
  const signal = path.join(root, 'entered')
  const release = path.join(root, 'release')
  const children = []
  t.after(async () => {
    await writeFile(release, 'release', 'utf8')
    for (const owned of children) {
      if (owned.child.exitCode === null && owned.child.signalCode === null) {
        owned.child.kill() // Only the exact synthetic child handle created below.
      }
      await owned.closed
    }
    const resolved = await realpath(root)
    const temporary = await realpath(tmpdir())
    const relative = path.relative(temporary, resolved)
    assert(relative && !relative.startsWith('..') && !path.isAbsolute(relative))
    assert(path.basename(resolved).startsWith('ecorp-stack-lock-'))
    assert.equal((await lstat(root)).isSymbolicLink(), false)
    await rm(root, { recursive: true })
  })
  function run(mode, target = workspace) {
    const child = spawn('pwsh.exe', [
      '-NoLogo', '-NoProfile', '-NonInteractive', '-File', script,
      '-Helper', helper, '-Workspace', target, '-Mode', mode,
      '-Signal', signal, '-Release', release,
    ], { env: environment, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => { stdout = (stdout + chunk).slice(-8192) })
    child.stderr.on('data', (chunk) => { stderr = (stderr + chunk).slice(-8192) })
    const closed = new Promise((resolve, reject) => {
      child.once('error', reject)
      child.once('close', (code, signal) => resolve({ code, signal, stdout, stderr }))
    })
    const owned = { child, closed }
    children.push(owned)
    return owned
  }
  async function entered() {
    const deadline = Date.now() + 10_000
    while (true) {
      try { await access(signal); return } catch {}
      assert(Date.now() < deadline, 'Synthetic lock holder must signal entry')
      await delay(25)
    }
  }
  return { root, workspace, other, release, run, entered }
}

const windowsOnly = {
  skip: process.platform !== 'win32' ? 'Windows local launcher' : false,
  timeout: 60_000,
}

test('one checkout serializes commands while another checkout remains usable', windowsOnly, async (t) => {
  const f = await fixture(t)
  const holder = f.run('hold')
  await f.entered()
  const blocked = await f.run('quick').closed
  assert.equal(blocked.code, 2)
  assert.match(blocked.stderr, /Another ECorp start\/stop command is still running/)
  assert.doesNotMatch(blocked.stdout, /acquired/)
  const separate = await f.run('quick', f.other).closed
  assert.equal(separate.code, 0, separate.stderr)
  assert.match(separate.stdout, /acquired/)
  await writeFile(f.release, 'release', 'utf8')
  assert.equal((await holder.closed).code, 0)
  const after = await f.run('quick').closed
  assert.equal(after.code, 0, after.stderr)
})

test('explicit restart nesting and failed operations release their native lock', windowsOnly, async (t) => {
  const f = await fixture(t)
  const nested = await f.run('nested').closed
  assert.equal(nested.code, 0, nested.stderr)
  assert.match(nested.stdout, /nested/)
  const failed = await f.run('failure').closed
  assert.equal(failed.code, 0, failed.stderr)
  assert.match(failed.stdout, /after-failure/)
  assert.equal((await f.run('quick').closed).code, 0)
})

test('an interrupted command does not leave a stale lock requiring manual cleanup', windowsOnly, async (t) => {
  const f = await fixture(t)
  const holder = f.run('hold')
  await f.entered()
  holder.child.kill()
  await holder.closed
  const recovered = await f.run('quick').closed
  assert.equal(recovered.code, 0, recovered.stderr)
  assert.match(recovered.stdout, /acquired/)
})

test('both supported entrypoints use the same operation boundary', async () => {
  for (const name of ['start_local.ps1', 'stop_local.ps1']) {
    const source = await readFile(path.join(directory, name), 'utf8')
    assert.match(source, /local_stack_operation\.ps1/)
    assert.match(source, /Invoke-LocalStackOperation -Workspace \$root -Action\s*\{/)
  }
})
