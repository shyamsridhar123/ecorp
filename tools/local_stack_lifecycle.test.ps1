#requires -Version 7.4
[CmdletBinding()]
param(
    [ValidateSet('Module', 'Source')][string]$Suite = 'Module',
    [string]$NodePath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
# This script is a child test scope. Delete without first reading or retaining
# the caller's value. Every DATABASE_URL subsequently used is a synthetic canary.
Remove-Item -LiteralPath Env:DATABASE_URL -ErrorAction SilentlyContinue
if (!$IsWindows) { throw 'These fixtures require Windows and PowerShell 7.4+.' }

$script:Cases = [Collections.Generic.List[object]]::new()
$script:Processes = [Collections.Generic.List[hashtable]]::new()
$script:Sentinels = [Collections.Generic.List[string]]::new()
$script:Junctions = [Collections.Generic.List[string]]::new()
$script:FixtureRoot = $null
$script:Lease = $null
$script:Cleanup = @{ created_processes = 0; remaining_processes = 0; temp_removed = $true }

function Assert-True {
    param($Actual, [string]$Message)
    if ($Actual -isnot [bool] -or !$Actual) { throw $Message }
}

function Assert-Equal {
    param($Actual, $Expected, [string]$Message)
    if ($Actual -cne $Expected) { throw $Message }
}

function Assert-False {
    param($Actual, [string]$Message)
    if ($Actual -isnot [bool] -or $Actual) { throw $Message }
}

function Assert-Throws {
    param([scriptblock]$Action, [string]$Message)
    $rejected = $false
    try { & $Action | Out-Null } catch { $rejected = $true }
    Assert-True $rejected $Message
}

function Invoke-Case {
    param([string]$Name, [scriptblock]$Action)
    try {
        & $Action | Out-Null
        $script:Cases.Add(@{ name = $Name; passed = $true; error = $null })
    } catch {
        $message = $_.Exception.Message + "`n" + $_.FullyQualifiedErrorId + "`n" + $_.ScriptStackTrace
        foreach ($sentinel in $script:Sentinels) { $message = $message.Replace($sentinel, '[synthetic-redacted]') }
        $script:Cases.Add(@{ name = $Name; passed = $false; error = $message })
    }
}

function Assert-InFixture {
    param([string]$Path)
    $full = [IO.Path]::GetFullPath($Path).TrimEnd('\', '/')
    $root = [IO.Path]::GetFullPath($script:FixtureRoot).TrimEnd('\', '/')
    if ($full -ne $root -and !$full.StartsWith($root + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Refusing a filesystem operation outside the task-created temporary tree.'
    }
    $full
}

function Write-FixtureFile {
    param([string]$Path, [string]$Text)
    $full = Assert-InFixture $Path
    [IO.File]::WriteAllText($full, $Text, [Text.UTF8Encoding]::new($false))
}

function Wait-FixtureJson {
    param([string]$Path)
    $full = Assert-InFixture $Path
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        if ([IO.File]::Exists($full)) {
            try { return ([IO.File]::ReadAllText($full) | ConvertFrom-Json -AsHashtable) }
            catch { } # The fixture may be in its one small readiness-file write.
        }
        Start-Sleep -Milliseconds 25
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'A synthetic fixture did not publish readiness within 10 seconds.'
}

function New-FixtureSpec {
    param([string]$Mode = 'worker', [string[]]$Extra = @())
    $nonce = [guid]::NewGuid().ToString('N')
    @{
        nonce = $nonce
        ready = Join-Path $script:FixtureRoot "$nonce ready [literal].json"
        before = [DateTime]::UtcNow
        arguments = @($script:FixtureScript, $Mode, $script:Lease,
            (Join-Path $script:FixtureRoot "$nonce ready [literal].json"), $nonce) + $Extra
    }
}

function Register-FixtureHandle {
    param([Diagnostics.Process]$Process, [hashtable]$Spec)
    # Pin this exact native process before checking identity. Cleanup never
    # reacquires a PID, delegates to the module under test, or kills a tree.
    [void]$Process.Handle
    $creation = $Process.StartTime.ToUniversalTime()
    $owned = @{
        process = $Process; process_id = $Process.Id
        executable = $script:NodeExecutable; creation = $creation; verified = $false
    }
    # Retain the exact handle even if readiness or image inspection subsequently
    # fails. An unverified fixture can exit on its lease, but is never killed.
    $script:Processes.Add($owned)
    if ($Process.HasExited -or $creation -lt $Spec.before.AddSeconds(-1)) {
        throw 'Could not independently establish synthetic process ownership.'
    }
    $owned
}

function Assert-FixtureReady {
    param([hashtable]$Ready, [hashtable]$Owned, [hashtable]$Spec)
    Assert-Equal $Ready.nonce $Spec.nonce 'Fixture readiness nonce must match this launch.'
    Assert-Equal ([int]$Ready.process_id) $Owned.process_id 'Fixture readiness must identify the held process.'
    Assert-Equal $Ready.executable $script:NodeExecutable 'Fixture must execute the explicitly selected Node binary.'
    $Owned.process.Refresh()
    $image = $Owned.process.MainModule
    Assert-True ($null -ne $image -and
        [string]::Equals($image.FileName, $script:NodeExecutable, [StringComparison]::OrdinalIgnoreCase)) 'The ready process image must match the held synthetic handle.'
    Assert-True ($Owned.creation -le [IO.File]::GetLastWriteTimeUtc($Spec.ready)) 'A reused PID created after readiness is not this fixture.'
    $Owned.verified = $true
}

function Start-Fixture {
    param([hashtable]$Spec)
    $info = [Diagnostics.ProcessStartInfo]::new($script:NodeExecutable)
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.WorkingDirectory = $script:Workspace
    $info.Environment.Clear()
    foreach ($name in @('SystemRoot', 'WINDIR', 'PATH', 'PATHEXT', 'TEMP', 'TMP')) {
        $value = [Environment]::GetEnvironmentVariable($name, 'Process')
        if ($null -ne $value) { $info.Environment[$name] = $value }
    }
    foreach ($argument in $Spec.arguments) { $info.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $info
    if (!$process.Start()) { throw 'Synthetic process creation failed.' }
    $owned = Register-FixtureHandle $process $Spec
    $ready = Wait-FixtureJson $Spec.ready
    Assert-FixtureReady $ready $owned $Spec
    @{ owned = $owned; ready = $ready; spec = $Spec }
}

function Get-FixtureRecord {
    param([hashtable]$Fixture)
    @{
        workspace = $script:Workspace
        pid = $Fixture.owned.process_id
        executable = $Fixture.owned.executable
        started_utc = $Fixture.owned.creation.ToString('o')
    }
}

function Assert-FixtureAlive {
    param([hashtable]$Fixture)
    Assert-False $Fixture.owned.process.HasExited 'A synthetic process that must survive was terminated.'
}

function Stop-FixtureHandle {
    param([hashtable]$Owned)
    $process = $Owned.process
    if (!$process.HasExited) {
        if (!$Owned.verified) {
            Assert-True ($process.WaitForExit(5000)) 'An unverified fixture did not cooperate with lease cleanup; it was not killed.'
            return
        }
        if ($process.Id -ne $Owned.process_id -or
            $process.StartTime.ToUniversalTime().Ticks -ne $Owned.creation.Ticks -or
            ![string]::Equals($process.MainModule.FileName, $Owned.executable, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Refusing cleanup: the held synthetic process identity no longer matches.'
        }
        $process.Kill() # Root only, through the handle retained at fixture creation.
    }
    Assert-True ($process.WaitForExit(5000)) 'The exact synthetic process did not exit during cleanup.'
}

function Start-ModuleFixture {
    param(
        [hashtable]$Spec,
        [string]$Role = 'literal-fixture',
        [hashtable]$Environment = @{},
        [string]$Workspace = $script:Workspace,
        [string]$WorkingDirectory = $Workspace,
        [string]$LogDirectory = $script:LogDirectory
    )
    $output = @(Start-LocalOwnedProcess -Role $Role -Workspace $Workspace `
        -FilePath $script:NodeExecutable -ArgumentList $Spec.arguments `
        -WorkingDirectory $WorkingDirectory -LogDirectory $LogDirectory `
        -Environment $Environment *>&1)
    $records = @($output | Where-Object { $_ -is [hashtable] })
    Assert-Equal $records.Count 1 'Start must return exactly one process-record hashtable.'
    $record = $records[0]
    $ready = Wait-FixtureJson $Spec.ready
    Assert-Equal $ready.nonce $Spec.nonce 'Module-started child must publish its unique fixture nonce.'
    Assert-Equal ([int]$ready.process_id) ([int]$record.pid) 'Module record must name the actual fixture child.'
    $process = [Diagnostics.Process]::GetProcessById([int]$ready.process_id)
    $owned = Register-FixtureHandle $process $Spec
    Assert-FixtureReady $ready $owned $Spec
    Assert-Equal ([DateTimeOffset]$record.started_utc).UtcTicks $owned.creation.Ticks 'Start must record exact OS creation time.'
    Assert-Equal $record.executable $owned.executable 'Start must record the actual executable.'
    Assert-Equal $record.workspace $Workspace 'Start must bind the explicit workspace.'
    @{ owned = $owned; ready = $ready; spec = $Spec; record = $record; output = $output }
}

function Wait-FixtureLog {
    param([string]$Path, [string]$Marker)
    $full = Assert-InFixture $Path
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        if ([IO.File]::Exists($full)) {
            $stream = $null
            $reader = $null
            try {
                # Reading a live redirected log must also share the writer's
                # existing write handle; File.ReadAllText shares reads only.
                $stream = [IO.File]::Open($full, [IO.FileMode]::Open, [IO.FileAccess]::Read,
                    [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete)
                $reader = [IO.StreamReader]::new($stream)
                $text = $reader.ReadToEnd()
                if ($text.Contains($Marker)) { return $text }
            } catch [IO.IOException] {
                # The launcher may still be opening/flushing this new log.
            } finally {
                if ($reader) { $reader.Dispose() }
                elseif ($stream) { $stream.Dispose() }
            }
        }
        Start-Sleep -Milliseconds 25
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'Expected output was not retained in the fixture log.'
}

function Assert-ReadRefused {
    param([string]$Path, [string]$Workspace)
    # Missing files may return null; corrupt or foreign state may throw. Neither
    # is stop authority, and neither may remove/repair the evidence file.
    $value = $null
    try { $value = Read-LocalStackState -Path $Path -Workspace $Workspace } catch { return }
    Assert-True ($null -eq $value) 'Invalid state must not return usable ownership authority.'
}

function Invoke-ModuleCases {
    $module = Join-Path $PSScriptRoot 'local_stack.psm1'
    Import-Module -Name $module -Force -DisableNameChecking
    $script:NodeExecutable = (Resolve-Path -LiteralPath $NodePath).Path
    $temporaryParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\', '/')
    $script:FixtureRoot = Join-Path $temporaryParent ("ecorp local lifecycle " + [guid]::NewGuid().ToString('N'))
    if ([IO.Directory]::Exists($script:FixtureRoot)) { throw 'Refusing to reuse a temporary test directory.' }
    [IO.Directory]::CreateDirectory($script:FixtureRoot) | Out-Null
    $script:Cleanup.temp_removed = $false
    $script:Workspace = Join-Path $script:FixtureRoot 'workspace with spaces [literal]'
    $script:LogDirectory = Join-Path $script:Workspace 'logs with spaces [literal]'
    [IO.Directory]::CreateDirectory($script:LogDirectory) | Out-Null
    $script:Lease = Join-Path $script:FixtureRoot 'fixture lease'
    Write-FixtureFile $script:Lease 'synthetic process lease'
    $script:FixtureScript = Join-Path $script:FixtureRoot 'synthetic child [literal] with spaces.mjs'
    # No sockets, database/client code, shell, real service, or credential reads.
    # All children exit when our lease disappears, with a 45-second hard TTL as
    # a backstop if the test supervisor is interrupted before its finally block.
    Write-FixtureFile $script:FixtureScript @'
import fs from 'node:fs'
import { createHash } from 'node:crypto'
import { spawn } from 'node:child_process'
const [mode, lease, readyPath, nonce, ...extra] = process.argv.slice(2)
const write = (file, value) => fs.writeFileSync(file, JSON.stringify(value))
const digest = (name) => process.env[name] === undefined ? null
  : createHash('sha256').update(process.env[name]).digest('hex')
const alive = () => fs.existsSync(lease)
const expires = Date.now() + 45_000
const ready = {
  nonce, process_id: process.pid, parent_id: process.ppid,
  executable: process.execPath, cwd: process.cwd(), arguments: extra,
  sentinel_digest: digest('ECORP_LOCAL_STACK_TEST_SENTINEL'),
  database_digest: digest('DATABASE_URL'),
  ambient_present: Object.hasOwn(process.env, 'ECORP_LOCAL_STACK_TEST_AMBIENT'),
  removed_present: Object.hasOwn(process.env, 'ECORP_LOCAL_STACK_TEST_REMOVED'),
}
if (mode === 'parent') {
  const child = spawn(process.execPath,
    [process.argv[1], 'worker', lease, extra[0], extra[1]],
    { windowsHide: true, stdio: 'ignore', detached: true })
  child.on('error', () => process.exit(2))
  child.unref()
}
if (mode === 'reader') {
  const [statePath, resultPath, readerLease] = extra
  let reads = 0, failures = 0
  const generations = new Set()
  const inspect = () => {
    try {
      const state = JSON.parse(fs.readFileSync(statePath, 'utf8'))
      const generation = state.generation
      if (state.schema_version !== 2 || !Number.isInteger(generation) ||
          state.payload !== String(generation % 10).repeat(65536) ||
          state.proof !== `generation-${generation}`) throw new Error('torn state')
      reads++
      generations.add(generation)
    } catch { failures++ }
  }
  inspect()
  write(readyPath, ready)
  const timer = setInterval(() => {
    if (!alive() || !fs.existsSync(readerLease) || Date.now() >= expires) {
      clearInterval(timer)
      write(resultPath, { reads, failures, distinct_generations: generations.size })
      process.exit(0)
    }
    inspect()
  }, 1)
} else {
  write(readyPath, ready)
  console.log(`fixture stdout ${nonce}`)
  console.error(`fixture stderr ${nonce}`)
  const timer = setInterval(() => {
    if (!alive() || Date.now() >= expires) { clearInterval(timer); process.exit(0) }
  }, 50)
}
'@

    $guard = Start-Fixture (New-FixtureSpec)
    $guardRecord = Get-FixtureRecord $guard
    Invoke-Case 'identity comes from the live process and binds only the explicit workspace' {
        $identity = Get-LocalProcessIdentity -ProcessId $guard.owned.process_id
        Assert-True ($identity -is [hashtable]) 'Identity must be a hashtable.'
        Assert-Equal ([int]$identity.pid) $guard.owned.process_id 'Identity PID differs from the held fixture.'
        Assert-Equal $identity.executable $guard.owned.executable 'Identity executable differs from the held fixture.'
        Assert-Equal ([DateTimeOffset]$identity.started_utc).UtcTicks $guard.owned.creation.Ticks 'Identity creation timestamp differs.'
        Assert-False (Test-LocalOwnedProcess -Record $identity -Workspace $script:Workspace) 'An identity without a workspace cannot authorize stopping.'
        $identity.workspace = $script:Workspace
        Assert-True (Test-LocalOwnedProcess -Record $identity -Workspace $script:Workspace) 'An exact, workspace-bound record must match.'
    }

    Invoke-Case 'wrong timestamps, executable, workspace and malformed records fail closed' {
        $variants = [Collections.Generic.List[object]]::new()
        $variants.Add($null)
        $variants.Add(@{})
        foreach ($key in @('pid', 'workspace', 'executable', 'started_utc')) {
            $copy = $guardRecord.Clone()
            $copy.Remove($key)
            $variants.Add($copy)
        }
        foreach ($change in @(
            @{ started_utc = $guard.owned.creation.AddSeconds(-1).ToString('o') },
            @{ started_utc = $guard.owned.creation.AddSeconds(1).ToString('o') },
            @{ started_utc = 'not-a-timestamp' }, @{ started_utc = $null },
            @{ executable = (Join-Path $script:Workspace 'not-the-fixture.exe') },
            @{ executable = 'node.exe' }, @{ executable = '' },
            @{ workspace = ($script:Workspace + '-neighbor') }, @{ workspace = $null },
            @{ pid = 'not-a-process-id' }, @{ pid = 0 }, @{ pid = -1 }
        )) {
            $copy = $guardRecord.Clone()
            foreach ($key in $change.Keys) { $copy[$key] = $change[$key] }
            $variants.Add($copy)
        }
        foreach ($record in $variants) {
            Assert-FixtureAlive $guard
            Assert-False (Test-LocalOwnedProcess -Record $record -Workspace $script:Workspace) 'A mismatched/malformed record passed ownership validation.'
            Assert-False (Stop-LocalOwnedProcess -Record $record -Workspace $script:Workspace) 'A mismatched/malformed record authorized stopping.'
            Assert-FixtureAlive $guard
        }
        Assert-False (Stop-LocalOwnedProcess -Record $guardRecord -Workspace ($script:Workspace + '-neighbor')) 'The caller workspace must also match.'
        Assert-FixtureAlive $guard
    }

    Invoke-Case 'a stale creation record aimed at a live synthetic PID cannot stop it' {
        $old = Start-Fixture (New-FixtureSpec)
        $stale = Get-FixtureRecord $old
        Stop-FixtureHandle $old.owned
        $replacement = Start-Fixture (New-FixtureSpec)
        Assert-True ($old.owned.creation.Ticks -ne $replacement.owned.creation.Ticks) 'PID-reuse simulation requires distinct observed creation times.'
        # Deliberately simulate PID reuse; do not churn system PIDs to force it.
        $stale.pid = $replacement.owned.process_id
        Assert-False (Test-LocalOwnedProcess -Record $stale -Workspace $script:Workspace) 'A PID alone must not adopt a new process.'
        Assert-False (Stop-LocalOwnedProcess -Record $stale -Workspace $script:Workspace) 'A stale creation record must not stop the replacement.'
        Assert-FixtureAlive $replacement
        Assert-FixtureAlive $guard
    }

    Invoke-Case 'stopping the verified root leaves its descendant and an unrelated child alive' {
        $descendantSpec = New-FixtureSpec
        $parent = Start-Fixture (New-FixtureSpec -Mode 'parent' -Extra @($descendantSpec.ready, $descendantSpec.nonce))
        $ready = Wait-FixtureJson $descendantSpec.ready
        Assert-Equal ([int]$ready.parent_id) $parent.owned.process_id 'The descendant fixture must actually belong to this root.'
        $held = Register-FixtureHandle ([Diagnostics.Process]::GetProcessById([int]$ready.process_id)) $descendantSpec
        Assert-FixtureReady $ready $held $descendantSpec
        $descendant = @{ owned = $held }
        $record = Get-FixtureRecord $parent
        Assert-True (Stop-LocalOwnedProcess -Record $record -Workspace $script:Workspace) 'The exact owned root should stop.'
        Assert-True ($parent.owned.process.WaitForExit(5000)) 'The verified root did not exit.'
        Start-Sleep -Milliseconds 150
        Assert-FixtureAlive $descendant
        Assert-FixtureAlive $guard
        Assert-True ($null -eq (Get-LocalProcessIdentity -ProcessId $record.pid)) 'An exited fixture must have no live identity.'
        Assert-False (Test-LocalOwnedProcess -Record $record -Workspace $script:Workspace) 'The exited record must not match.'
        Assert-False (Stop-LocalOwnedProcess -Record $record -Workspace $script:Workspace) 'Repeated stopping must not act on another process.'
    }

    foreach ($pathCase in @(
        @{ name = 'Start accepts working and log directories containing spaces'; working = 'plain working directory'; logs = 'plain log directory' },
        @{ name = 'Start treats bracketed log directories as literal paths'; working = 'plain working directory'; logs = 'bracketed [literal] log directory' },
        @{ name = 'Start treats bracketed working directories as literal paths'; working = 'bracketed [literal] working directory'; logs = 'plain log directory' }
    )) {
        Invoke-Case $pathCase.name {
            $working = Join-Path $script:FixtureRoot $pathCase.working
            $logs = Join-Path $script:FixtureRoot $pathCase.logs
            [IO.Directory]::CreateDirectory((Assert-InFixture $working)) | Out-Null
            [IO.Directory]::CreateDirectory((Assert-InFixture $logs)) | Out-Null
            $started = Start-ModuleFixture (New-FixtureSpec) -Workspace $script:FixtureRoot `
                -WorkingDirectory $working -LogDirectory $logs
            Assert-Equal $started.ready.cwd $working 'The child did not enter the exact requested literal working directory.'
            $null = Wait-FixtureLog $started.record.stdout "fixture stdout $($started.spec.nonce)"
            $null = Wait-FixtureLog $started.record.stderr "fixture stderr $($started.spec.nonce)"
            Assert-True (Stop-LocalOwnedProcess -Record $started.record -Workspace $script:FixtureRoot) 'The literal-path fixture must stop through its exact ownership record.'
            Assert-True ($started.owned.process.WaitForExit(5000)) 'The literal-path fixture did not exit.'
        }
    }

    Invoke-Case 'failed-start rollback results preserve the original error and require both success signals' {
        # Pure result-handling cases, not injected failures in Windows kernel
        # calls. No process handles or actual termination failures are fabricated.
        $launcher = 'ECorp.LocalLiteralLauncher' -as [type]
        Assert-True ($null -ne $launcher) 'The actual literal launcher must have loaded.'
        $method = $launcher.GetMethod('RecordRollbackResult', [Reflection.BindingFlags]'NonPublic,Static')
        Assert-True ($null -ne $method) 'The native rollback result handler must exist.'
        foreach ($case in @(
            @{ terminated = $true; wait = [uint32]0; verified = $true },
            @{ terminated = $false; wait = [uint32]0; verified = $false },
            @{ terminated = $true; wait = [uint32]258; verified = $false },
            @{ terminated = $false; wait = [uint32]258; verified = $false },
            @{ terminated = $true; wait = [uint32]::MaxValue; verified = $false },
            @{ terminated = $false; wait = [uint32]::MaxValue; verified = $false }
        )) {
            $cause = [InvalidOperationException]::new('synthetic inner cause')
            $failure = [InvalidOperationException]::new('synthetic original startup failure', $cause)
            $terminationError = if ($case.terminated) { 0 } else { 5 }
            $waitError = if ($case.wait -eq [uint32]::MaxValue) { 6 } else { 0 }
            $verified = $method.Invoke($null, [object[]]@(
                $failure, [bool]$case.terminated, [int]$terminationError, [uint32]$case.wait, [int]$waitError))
            Assert-Equal $verified $case.verified 'Rollback must not be verified if termination or waiting failed.'
            Assert-Equal $failure.Data['LocalStackRollbackVerified'] $case.verified 'The original exception must disclose rollback verification status.'
            Assert-Equal $failure.Data['LocalStackRollbackTerminateSucceeded'] $case.terminated 'The native termination result must remain observable.'
            Assert-Equal $failure.Data['LocalStackRollbackTerminationError'] $terminationError 'The native termination error must be preserved.'
            Assert-Equal $failure.Data['LocalStackRollbackWaitResult'] $case.wait 'The native wait result must be preserved.'
            Assert-Equal $failure.Data['LocalStackRollbackWaitError'] $waitError 'The native wait error must be preserved.'
            Assert-Equal $failure.Message 'synthetic original startup failure' 'Rollback reporting must not replace the startup error.'
            Assert-True ([object]::ReferenceEquals($failure.InnerException, $cause)) 'Rollback reporting must preserve the original inner exception.'
        }
    }

    Invoke-Case 'Start preserves literal arguments, spaces, child-only environment and redacted evidence' {
        $sentinel = 'synthetic-not-a-database-' + [guid]::NewGuid().ToString('N')
        $script:Sentinels.Add($sentinel)
        [Environment]::SetEnvironmentVariable('ECORP_LOCAL_STACK_TEST_AMBIENT', $sentinel, 'Process')
        [Environment]::SetEnvironmentVariable('ECORP_LOCAL_STACK_TEST_REMOVED', $sentinel, 'Process')
        try {
            $arguments = @('one argument with spaces', '[literal]*?;&$HOME', '"quoted value"', 'C:\trailing space path\', '')
            $started = Start-ModuleFixture (New-FixtureSpec -Extra $arguments) -Environment @{
                ECORP_LOCAL_STACK_TEST_SENTINEL = $sentinel
                DATABASE_URL = $sentinel
                ECORP_LOCAL_STACK_TEST_REMOVED = $null
            }
            Assert-Equal $started.ready.cwd $script:Workspace 'WorkingDirectory must be a literal path containing spaces and brackets.'
            Assert-Equal ($started.ready.arguments | ConvertTo-Json -Compress) ($arguments | ConvertTo-Json -Compress) 'Argument boundaries, quotes and empty arguments must survive.'
            $digest = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($sentinel))).ToLowerInvariant()
            Assert-Equal $started.ready.sentinel_digest $digest 'The explicitly supplied environment canary did not reach the child.'
            Assert-Equal $started.ready.database_digest $digest 'The synthetic database canary must travel through environment only.'
            Assert-False $started.ready.ambient_present 'An unapproved ambient canary leaked into the child.'
            Assert-False $started.ready.removed_present 'A null environment override must remove the inherited name.'
            Assert-True ($null -eq [Environment]::GetEnvironmentVariable('DATABASE_URL', 'Process')) 'Start must not mutate the supervisor database environment.'
            Assert-True ($null -eq [Environment]::GetEnvironmentVariable('ECORP_LOCAL_STACK_TEST_SENTINEL', 'Process')) 'Start must not mutate the supervisor canary environment.'
            $stdout = Wait-FixtureLog $started.record.stdout "fixture stdout $($started.spec.nonce)"
            $stderr = Wait-FixtureLog $started.record.stderr "fixture stderr $($started.spec.nonce)"
            $statePath = Join-Path $script:Workspace 'redacted state.json'
            Save-LocalStackState -Path $statePath -Workspace $script:Workspace -State @{ processes = @{ fixture = $started.record } }
            foreach ($text in @($stdout, $stderr, ($started.output | Out-String),
                ($started.record | ConvertTo-Json -Depth 12), [IO.File]::ReadAllText($statePath),
                [IO.File]::ReadAllText($started.spec.ready))) {
                Assert-False ($text.Contains($sentinel)) 'A synthetic environment canary leaked into arguments, logs, state, record or diagnostics.'
            }
            Assert-True (Test-LocalOwnedProcess -Record $started.record -Workspace $script:Workspace) 'The Start record must be usable as exact ownership evidence.'
            Assert-True (Stop-LocalOwnedProcess -Record $started.record -Workspace $script:Workspace) 'The module-started child must stop through its exact record.'
            Assert-True ($started.owned.process.WaitForExit(5000)) 'The module-started child did not exit.'
        } finally {
            Remove-Item -LiteralPath Env:ECORP_LOCAL_STACK_TEST_AMBIENT -ErrorAction SilentlyContinue
            Remove-Item -LiteralPath Env:ECORP_LOCAL_STACK_TEST_REMOVED -ErrorAction SilentlyContinue
        }
    }

    Invoke-Case 'repeated role launches retain earlier logs and allocate unique stdout/stderr files' {
        $historical = Join-Path $script:LogDirectory 'literal-fixture.stdout.log'
        Write-FixtureFile $historical 'historical log must survive'
        $first = Start-ModuleFixture (New-FixtureSpec)
        $firstOut = Wait-FixtureLog $first.record.stdout "fixture stdout $($first.spec.nonce)"
        $firstErr = Wait-FixtureLog $first.record.stderr "fixture stderr $($first.spec.nonce)"
        Assert-True (Stop-LocalOwnedProcess -Record $first.record -Workspace $script:Workspace) 'First exact record should stop.'
        $second = Start-ModuleFixture (New-FixtureSpec)
        $null = Wait-FixtureLog $second.record.stdout "fixture stdout $($second.spec.nonce)"
        $null = Wait-FixtureLog $second.record.stderr "fixture stderr $($second.spec.nonce)"
        $paths = @($first.record.stdout, $first.record.stderr, $second.record.stdout, $second.record.stderr)
        Assert-Equal (@($paths | Sort-Object -Unique).Count) 4 'Each launch and stream needs a distinct log file.'
        Assert-Equal ([IO.File]::ReadAllText($first.record.stdout)) $firstOut 'A later launch changed the earlier stdout log.'
        Assert-Equal ([IO.File]::ReadAllText($first.record.stderr)) $firstErr 'A later launch changed the earlier stderr log.'
        Assert-Equal ([IO.File]::ReadAllText($historical)) 'historical log must survive' 'A launch deleted a historical log.'
        Assert-True (Stop-LocalOwnedProcess -Record $second.record -Workspace $script:Workspace) 'Second exact record should stop.'
    }

    foreach ($rollbackMode in @('verified', 'unverified')) {
        Invoke-Case "extracted Launch preserves ownership on save failure ($rollbackMode rollback)" {
            $tokens = $null
            $errors = $null
            $starter = [Management.Automation.Language.Parser]::ParseFile(
                (Join-Path $PSScriptRoot 'local_stack_start.ps1'), [ref]$tokens, [ref]$errors)
            Assert-Equal $errors.Count 0 'The starter must parse before extracting Launch.'
            $definitions = @($starter.FindAll({ param($node)
                $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Launch'
            }, $true))
            Assert-Equal $definitions.Count 1 'Exactly one production Launch function must be exercised.'
            $definition = $definitions[0]
            # Never dot-source/run the starter. Refuse any expansion of this
            # extracted function into service, network, or credential operations.
            foreach ($command in $definition.FindAll({ param($node)
                $node -is [Management.Automation.Language.CommandAst]
            }, $true)) {
                Assert-True ($command.GetCommandName() -in @(
                    'Role-Live', 'Select-Object', 'Start-LocalOwnedProcess',
                    'Save-State', 'Stop-LocalOwnedProcess', 'Write-Warning'
                )) 'The extracted Launch function contains an operation outside this synthetic fixture.'
            }
            . ([scriptblock]::Create($definition.Extent.Text))

            $root = $script:Workspace
            $logs = $script:LogDirectory
            $role = 'save-failure-fixture'
            $statePath = Join-Path $root "launch $rollbackMode save failure [literal].json"
            $state = @{
                processes = @{ unrelated = $guardRecord }
                previous_processes = @()
            }
            Save-LocalStackState -Path $statePath -Workspace $root -State $state
            $before = [IO.File]::ReadAllText($statePath)
            $context = @{
                role = $role; spec = (New-FixtureSpec); fixture = $null; saveFailure = $null
                stopRecord = $null; stopWorkspace = $null
                stopFailure = [InvalidOperationException]::new('synthetic exact-root rollback failure')
            }
            function Role-Live([string]$Role) {
                $record = if ($state.processes.ContainsKey($Role)) { $state.processes[$Role] } else { $null }
                Test-LocalOwnedProcess -Record $record -Workspace $root
            }
            function Save-State {
                $record = $state.processes[$context.role]
                $ready = Wait-FixtureJson $context.spec.ready
                Assert-Equal ([int]$ready.process_id) ([int]$record.pid) 'The Launch record must identify this synthetic child.'
                $owned = Register-FixtureHandle ([Diagnostics.Process]::GetProcessById([int]$record.pid)) $context.spec
                Assert-FixtureReady $ready $owned $context.spec
                $context.fixture = @{ owned = $owned; ready = $ready; spec = $context.spec }
                $null = Wait-FixtureLog $record.stdout "fixture stdout $($context.spec.nonce)"
                $null = Wait-FixtureLog $record.stderr "fixture stderr $($context.spec.nonce)"
                try {
                    # Actual writer + Windows sharing denial, not a mocked save.
                    Save-LocalStackState -Path $statePath -Workspace $root -State $state
                } catch {
                    $context.saveFailure = $_
                    throw
                }
            }
            if ($rollbackMode -eq 'unverified') {
                # Simulate only the rollback error. The child is real, remains
                # on a held verified handle, and is reaped by fixture cleanup.
                function Stop-LocalOwnedProcess([hashtable]$Record, [string]$Workspace) {
                    $context.stopRecord = $Record
                    $context.stopWorkspace = $Workspace
                    throw $context.stopFailure
                }
            }

            $lock = [IO.File]::Open($statePath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
            $failure = $null
            try {
                try {
                    Launch $role $script:NodeExecutable $context.spec.arguments $root @{} *>&1 | Out-Null
                } catch { $failure = $_ }
            } finally { $lock.Dispose() }
            Assert-True ($null -ne $context.saveFailure) 'The real state writer must hit the deliberately locked destination.'
            Assert-True ($null -ne $failure) 'Launch must rethrow the persistence failure, not report startup success.'
            Assert-True ([object]::ReferenceEquals(
                $failure.Exception.GetBaseException(), $context.saveFailure.Exception.GetBaseException()
            )) 'Rollback must preserve the original persistence error/cause.'
            $diagnostic = $failure.Exception
            while ($diagnostic -and !$diagnostic.Data.Contains('LocalStackNewProcessRecord')) {
                $diagnostic = $diagnostic.InnerException
            }
            Assert-True ($null -ne $diagnostic) 'The original exception must retain the exact unsaved ownership record.'
            $record = $diagnostic.Data['LocalStackNewProcessRecord']
            Assert-True ($record -is [hashtable]) 'Retained cleanup authority must be a process-record hashtable.'
            Assert-Equal ([int]$record.pid) $context.fixture.owned.process_id 'The retained record must name only the newly created child.'
            Assert-Equal $record.workspace $root 'The retained record must preserve its workspace.'
            Assert-Equal ([DateTimeOffset]$record.started_utc).UtcTicks $context.fixture.owned.creation.Ticks 'The retained record must preserve exact process creation time.'
            Assert-Equal $record.executable $context.fixture.owned.executable 'The retained record must preserve its executable.'
            Assert-Equal ([IO.File]::ReadAllText($statePath)) $before 'A failed launch save must not change existing state bytes.'
            Assert-FixtureAlive $guard
            Assert-True ([IO.File]::Exists($record.stdout) -and [IO.File]::Exists($record.stderr)) 'Rollback must retain both newly created log files.'
            Assert-Equal (@(Get-ChildItem -LiteralPath $root -Filter '*.tmp').Count) 0 'Failed persistence must clean only its temporary state file.'
            if ($rollbackMode -eq 'verified') {
                Assert-True $diagnostic.Data['LocalStackPersistenceRollbackVerified'] 'A successful exact-root rollback must be explicitly verified.'
                Assert-True ($context.fixture.owned.process.WaitForExit(5000)) 'The actual newly launched child must be reaped on persistence failure.'
                Assert-Equal $record.stop_outcome 'startup_persistence_rollback' 'The retained record must explain why this child was stopped.'
                Assert-True ($null -eq $state[$role]) 'The failed role must not remain advertised as running.'
            } else {
                Assert-False $diagnostic.Data['LocalStackPersistenceRollbackVerified'] 'A failed rollback must never claim verified cleanup.'
                Assert-True ([object]::ReferenceEquals($context.stopRecord, $record)) 'Rollback must target only the exact newly created record.'
                Assert-Equal $context.stopWorkspace $root 'Rollback must use the original workspace.'
                Assert-Equal $diagnostic.Data['LocalStackPersistenceRollbackError'].Message $context.stopFailure.Message 'The secondary rollback error must remain observable without replacing the save error.'
                Assert-FixtureAlive $context.fixture
                Stop-FixtureHandle $context.fixture.owned
            }
        }
    }

    Invoke-Case 'v2 state round-trips nested records and read preserves exact bytes' {
        $statePath = Join-Path $script:Workspace 'round trip [literal].json'
        Assert-True ($null -eq (Read-LocalStackState -Path $statePath -Workspace $script:Workspace)) 'A missing state file must return null.'
        Save-LocalStackState -Path $statePath -Workspace $script:Workspace -State @{
            processes = @{ fixture = $guardRecord }; metadata = @{ marker = 'generation one' }
        }
        $before = [IO.File]::ReadAllText($statePath)
        $state = Read-LocalStackState -Path $statePath -Workspace $script:Workspace
        Assert-True ($state -is [hashtable]) 'Read must return a hashtable.'
        Assert-Equal $state.schema_version 2 'The state schema must be version 2.'
        Assert-Equal $state.workspace $script:Workspace 'State must bind its explicit workspace.'
        Assert-Equal $state.metadata.marker 'generation one' 'Nested state did not round-trip.'
        Assert-True (Test-LocalOwnedProcess -Record $state.processes.fixture -Workspace $script:Workspace) 'Serialization must retain exact live process identity.'
        Assert-Equal ([IO.File]::ReadAllText($statePath)) $before 'Reading state must not rewrite it.'
        Assert-ReadRefused $statePath ($script:Workspace + '-neighbor')
        Assert-Equal ([IO.File]::ReadAllText($statePath)) $before 'Foreign reads must not change the state.'
        $state.metadata.marker = 'generation two'
        Save-LocalStackState -Path $statePath -Workspace $script:Workspace -State $state
        $updated = Read-LocalStackState -Path $statePath -Workspace $script:Workspace
        Assert-Equal $updated.metadata.marker 'generation two' 'Replacing existing state did not persist the complete update.'
        Assert-Equal (@(Get-ChildItem -LiteralPath $script:Workspace -Filter '*.tmp').Count) 0 'Successful replacement left a temporary state file.'
        Assert-FixtureAlive $guard
    }

    Invoke-Case 'corrupt, unsupported and differently scoped state is refused without deleting evidence' {
        $statePath = Join-Path $script:Workspace 'invalid state.json'
        foreach ($text in @(
            '{broken json', 'null', '42', '[]',
            '{"schema_version":3,"workspace":"not-this-workspace","processes":{}}',
            (@{ schema_version = 2; workspace = $script:Workspace; processes = 'invalid' } | ConvertTo-Json),
            (@{ schema_version = 2; processes = @{} } | ConvertTo-Json)
        )) {
            Write-FixtureFile $statePath $text
            Assert-ReadRefused $statePath $script:Workspace
            Assert-Equal ([IO.File]::ReadAllText($statePath)) $text 'Rejected state was removed or silently repaired.'
        }
        Assert-FixtureAlive $guard
    }

    Invoke-Case 'state writes reject traversal, sibling-prefix paths and foreign workspace claims' {
        $sibling = Join-Path $script:FixtureRoot 'workspace with spaces [literal]-neighbor'
        [IO.Directory]::CreateDirectory($sibling) | Out-Null
        foreach ($outside in @(
            (Join-Path $script:FixtureRoot 'outside state.json'),
            (Join-Path $script:Workspace '..\traversal state.json'),
            (Join-Path $sibling 'prefix collision.json')
        )) {
            Write-FixtureFile $outside 'outside evidence'
            Assert-Throws { Save-LocalStackState -Path $outside -State @{ processes = @{} } -Workspace $script:Workspace } 'A state write escaped the explicit workspace.'
            Assert-Equal ([IO.File]::ReadAllText($outside)) 'outside evidence' 'A rejected write changed a file outside its workspace.'
        }
        $inside = Join-Path $script:Workspace 'foreign scoped state.json'
        Write-FixtureFile $inside 'inside evidence'
        Assert-Throws { Save-LocalStackState -Path $inside -State @{ workspace = $sibling; processes = @{} } -Workspace $script:Workspace } 'A foreign state workspace must be rejected.'
        Assert-Equal ([IO.File]::ReadAllText($inside)) 'inside evidence' 'Rejected foreign state overwrote prior evidence.'
    }

    Invoke-Case 'state writes reject reparse-point parent directories' {
        $outside = Join-Path $script:FixtureRoot 'junction destination'
        $junction = Join-Path $script:Workspace 'redirect [literal]'
        [IO.Directory]::CreateDirectory($outside) | Out-Null
        $null = Assert-InFixture $junction
        New-Item -ItemType Junction -Path $junction -Target $outside | Out-Null
        $script:Junctions.Add($junction)
        $evidence = Join-Path $outside 'state.json'
        Write-FixtureFile $evidence 'junction target evidence'
        Assert-Throws { Save-LocalStackState -Path (Join-Path $junction 'state.json') -Workspace $script:Workspace -State @{ processes = @{} } } 'A reparse-point parent must not redirect the writer.'
        Assert-Equal ([IO.File]::ReadAllText($evidence)) 'junction target evidence' 'The junction destination was modified.'
    }

    Invoke-Case 'a failed atomic replacement preserves old bytes and removes its temporary file' {
        $statePath = Join-Path $script:Workspace 'locked state.json'
        Save-LocalStackState -Path $statePath -Workspace $script:Workspace -State @{ processes = @{}; marker = 'before' }
        $before = [IO.File]::ReadAllText($statePath)
        $lock = [IO.File]::Open($statePath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        try {
            Assert-Throws { Save-LocalStackState -Path $statePath -Workspace $script:Workspace -State @{ processes = @{}; marker = 'after' } } 'A replacement must fail when the target denies delete sharing.'
        } finally { $lock.Dispose() }
        Assert-Equal ([IO.File]::ReadAllText($statePath)) $before 'A failed replacement damaged the previous state.'
        Assert-Equal (@(Get-ChildItem -LiteralPath $script:Workspace -Filter '*.tmp').Count) 0 'A failed replacement left temporary files.'
    }

    Invoke-Case 'concurrent synthetic reader observes only complete atomic state generations' {
        $statePath = Join-Path $script:Workspace 'concurrent state.json'
        $readerLease = Join-Path $script:FixtureRoot 'reader lease'
        $resultPath = Join-Path $script:FixtureRoot 'reader result.json'
        Write-FixtureFile $readerLease 'reader lease'
        $makeState = { param([int]$Generation)
            @{ processes = @{}; generation = $Generation; proof = "generation-$Generation"
                payload = ([string]($Generation % 10)) * 65536 }
        }
        Save-LocalStackState -Path $statePath -Workspace $script:Workspace -State (& $makeState 0)
        $reader = Start-Fixture (New-FixtureSpec -Mode 'reader' -Extra @($statePath, $resultPath, $readerLease))
        try {
            for ($generation = 1; $generation -le 24; $generation++) {
                # Windows may reject an atomic rename while a reader has the
                # destination open. Retry that safe failure; never accept a
                # missing/torn snapshot or require destructive lock breaking.
                $saved = $false
                for ($attempt = 0; $attempt -lt 30 -and !$saved; $attempt++) {
                    try {
                        Save-LocalStackState -Path $statePath -Workspace $script:Workspace -State (& $makeState $generation)
                        $saved = $true
                    } catch [IO.IOException] {
                        Start-Sleep -Milliseconds 10
                    } catch [UnauthorizedAccessException] {
                        Start-Sleep -Milliseconds 10
                    }
                }
                Assert-True $saved 'Atomic replacement never completed within the bounded reader-contention retries.'
                Start-Sleep -Milliseconds 5
            }
        } finally {
            $null = Assert-InFixture $readerLease
            Remove-Item -LiteralPath $readerLease -Force
        }
        $result = Wait-FixtureJson $resultPath
        Assert-True ($reader.owned.process.WaitForExit(5000)) 'The bounded reader fixture did not exit.'
        Assert-Equal $reader.owned.process.ExitCode 0 'The reader fixture failed.'
        Assert-True ($result.reads -gt 1 -and $result.distinct_generations -gt 1) 'The reader must overlap multiple actual state generations.'
        Assert-Equal $result.failures 0 'A concurrent reader observed missing, malformed or torn state.'
    }

    Invoke-Case 'legacy numeric state stays read-only and never authorizes stopping' {
        $statePath = Join-Path $script:Workspace 'legacy numeric state.json'
        $original = @{ server = $guard.owned.process_id; runner = $null; factoryController = $null; web = $null } | ConvertTo-Json
        Write-FixtureFile $statePath $original
        $legacy = Read-LocalStackState -Path $statePath -Workspace $script:Workspace
        Assert-True ($legacy -is [hashtable]) 'Legacy numeric state must remain readable for diagnostics.'
        Assert-Equal ([int]$legacy.server) $guard.owned.process_id 'Legacy diagnostics lost the historical PID.'
        Assert-Equal ([IO.File]::ReadAllText($statePath)) $original 'Reading legacy data must not upgrade or delete it.'
        $insufficient = @{ workspace = $script:Workspace; pid = $legacy.server }
        Assert-False (Test-LocalOwnedProcess -Record $insufficient -Workspace $script:Workspace) 'Legacy PID-only state is not ownership evidence.'
        Assert-False (Stop-LocalOwnedProcess -Record $insufficient -Workspace $script:Workspace) 'Legacy PID-only state must not stop the live fixture.'
        Assert-FixtureAlive $guard
        $rawLegacy = $original | ConvertFrom-Json -AsHashtable
        foreach ($candidate in @(
            $legacy, $rawLegacy,
            @{ schema_version = 1; workspace = $script:Workspace; processes = @{ fixture = $guardRecord } },
            @{ schema_version = 2; workspace = $script:Workspace; processes = $rawLegacy }
        )) {
            $before = $candidate | ConvertTo-Json -Depth 12 -Compress
            Assert-Throws { Save-LocalStackState -Path $statePath -Workspace $script:Workspace -State $candidate } 'Legacy numeric state must not be silently rewritten as v2 authority.'
            Assert-Equal ($candidate | ConvertTo-Json -Depth 12 -Compress) $before 'Rejecting legacy state must not mutate the caller record.'
            Assert-Equal ([IO.File]::ReadAllText($statePath)) $original 'The legacy numeric evidence must remain read-only.'
            Assert-FixtureAlive $guard
        }
    }
}

function Invoke-SourceCases {
    $tokens = $null
    $errors = $null
    $start = [Management.Automation.Language.Parser]::ParseFile(
        (Join-Path $PSScriptRoot 'local_stack_start.ps1'), [ref]$tokens, [ref]$errors)
    if ($errors.Count) { throw 'The starter must parse before source guards are meaningful.' }
    $stop = [Management.Automation.Language.Parser]::ParseFile(
        (Join-Path $PSScriptRoot 'stop_local.ps1'), [ref]$tokens, [ref]$errors)
    if ($errors.Count) { throw 'The stopper must parse before source guards are meaningful.' }
    $commands = $start.FindAll({ param($node) $node -is [Management.Automation.Language.CommandAst] }, $true)
    function Get-ConditionalAncestors {
        param($Node)
        for ($parent = $Node.Parent; $null -ne $parent; $parent = $parent.Parent) {
            if ($parent -is [Management.Automation.Language.IfStatementAst] -or
                $parent -is [Management.Automation.Language.CatchClauseAst]) { $parent }
        }
    }
    function Get-SourceExpression {
        param($Node)
        while ($null -ne $Node) {
            if ($Node -is [Management.Automation.Language.PipelineAst] -and $Node.PipelineElements.Count -eq 1) {
                $Node = $Node.PipelineElements[0]
            } elseif ($Node -is [Management.Automation.Language.CommandExpressionAst]) {
                $Node = $Node.Expression
            } elseif ($Node -is [Management.Automation.Language.ParenExpressionAst]) {
                $Node = $Node.Pipeline
            } elseif ($Node -is [Management.Automation.Language.StatementBlockAst] -and $Node.Statements.Count -eq 1) {
                $Node = $Node.Statements[0]
            } else { return $Node }
        }
    }
    function Test-SourceMember {
        param($Node, [string]$Variable, [string]$Member)
        $expression = Get-SourceExpression $Node
        return $expression -is [Management.Automation.Language.MemberExpressionAst] -and
            $expression.Expression -is [Management.Automation.Language.VariableExpressionAst] -and
            $expression.Expression.VariablePath.UserPath -eq $Variable -and
            $expression.Member -is [Management.Automation.Language.StringConstantExpressionAst] -and
            $expression.Member.Value -eq $Member
    }
    function Test-ManagedCondition {
        param($Node)
        $expression = Get-SourceExpression $Node
        if ($expression -isnot [Management.Automation.Language.BinaryExpressionAst]) { return $false }
        if ($expression.Operator -eq 'And') {
            return (Test-ManagedCondition $expression.Left) -or (Test-ManagedCondition $expression.Right)
        }
        if ($expression.Operator -eq 'Or') {
            return (Test-ManagedCondition $expression.Left) -and (Test-ManagedCondition $expression.Right)
        }
        if ($expression.Operator -notin @('Ieq', 'Ceq')) { return $false }
        $left = Get-SourceExpression $expression.Left
        $right = Get-SourceExpression $expression.Right
        return $left -is [Management.Automation.Language.VariableExpressionAst] -and
            $left.VariablePath.UserPath -eq 'dbMode' -and
            $right -is [Management.Automation.Language.StringConstantExpressionAst] -and
            $right.Value -ceq 'managed'
    }
    function Test-ComposeGuard {
        param($Command)
        foreach ($ancestor in @(Get-ConditionalAncestors $Command)) {
            if ($ancestor -isnot [Management.Automation.Language.IfStatementAst]) { continue }
            foreach ($clause in $ancestor.Clauses) {
                # An if's condition does not guard its else branch.
                if ($Command.Extent.StartOffset -ge $clause.Item2.Extent.StartOffset -and
                    $Command.Extent.EndOffset -le $clause.Item2.Extent.EndOffset -and
                    (Test-ManagedCondition $clause.Item1)) { return $true }
            }
        }
        return $false
    }
    Invoke-Case 'starter has no unconditional stop invocation' {
        foreach ($command in $commands | Where-Object { $_.Extent.Text -match 'stop_local\.ps1|^\s*(Stop-LocalOwnedProcess|Stop-Process)\b' }) {
            Assert-True (@(Get-ConditionalAncestors $command).Count -gt 0) 'Stop must be an explicit conditional action or failed-start cleanup, not unconditional startup.'
        }
    }
    Invoke-Case 'starter never passes a database URL as a command argument' {
        $arguments = $start.FindAll({ param($node)
            ($node -is [Management.Automation.Language.StringConstantExpressionAst] -or
                $node -is [Management.Automation.Language.ExpandableStringExpressionAst]) -and
                $node.Value -match '(^|\s)--database-url(\s|$)'
        }, $true)
        Assert-Equal @($arguments).Count 0 'DATABASE_URL must not be exposed through --database-url.'
    }
    Invoke-Case 'starter forwards the native runner startup-recovery opt-out unchanged' {
        $launches = @($commands | Where-Object {
            $_.GetCommandName() -eq 'Launch' -and $_.CommandElements.Count -gt 1 -and
            $_.CommandElements[1] -is [Management.Automation.Language.StringConstantExpressionAst] -and
            $_.CommandElements[1].Value -eq 'server'
        })
        Assert-Equal $launches.Count 1 'The source guard must find the actual server launch.'
        $launch = $launches[0]
        $setups = @($start.FindAll({ param($node)
            $node -is [Management.Automation.Language.AssignmentStatementAst] -and
                $node.Left -is [Management.Automation.Language.VariableExpressionAst] -and
                $node.Left.VariablePath.UserPath -eq 'serverEnvironment'
        }, $true) | Where-Object { $_.Extent.EndOffset -lt $launch.Extent.StartOffset })
        Assert-Equal $setups.Count 1 'The server environment must come from one explicit pre-launch selection.'
        $selection = $setups[0].Right
        Assert-True ($selection -is [Management.Automation.Language.PipelineAst] -and
            $selection.PipelineElements.Count -eq 1) 'The selected server environment must not be replaced by a different pipeline.'
        $call = $selection.PipelineElements[0]
        Assert-True ($call -is [Management.Automation.Language.CommandAst] -and
            $call.GetCommandName() -eq 'Explicit-Environment' -and $call.CommandElements.Count -eq 2) 'The server must use the explicit environment selector.'
        $names = @($call.CommandElements[1].SafeGetValue()) # Literal names only; no environment values are read.
        Assert-True ($names -contains 'CRONY_RUNNER_STARTUP_RECOVERY') 'The caller native recovery opt-out must not be stripped.'
        $argument = $launch.CommandElements[-1]
        Assert-True ($argument -is [Management.Automation.Language.VariableExpressionAst] -and
            $argument.VariablePath.UserPath -eq 'serverEnvironment') 'The selected recovery flag must reach the actual server launch.'
        $overrides = @($start.FindAll({ param($node)
            $node -is [Management.Automation.Language.AssignmentStatementAst]
        }, $true) | Where-Object {
            (Test-SourceMember $_.Left 'serverEnvironment' 'CRONY_RUNNER_STARTUP_RECOVERY') -and
                $_.Extent.StartOffset -gt $setups[0].Extent.EndOffset -and
                $_.Extent.EndOffset -lt $launch.Extent.StartOffset
        })
        Assert-Equal $overrides.Count 0 'The caller recovery flag must not be overwritten after selection.'
        Assert-Equal (@($launch.FindAll({ param($node)
            $node -is [Management.Automation.Language.StringConstantExpressionAst] -and
                $node.Value -eq '--runner-startup-recovery'
        }, $true)).Count) 0 'A hard-coded command argument must not override the caller recovery flag.'
    }
    Invoke-Case 'persisted runner IDs block automatic reenrollment even when offline' {
        $lookups = @($start.FindAll({ param($node)
            $node -is [Management.Automation.Language.AssignmentStatementAst] -and
                $node.Left -is [Management.Automation.Language.VariableExpressionAst] -and
                $node.Left.VariablePath.UserPath -eq 'existingRunner'
        }, $true))
        Assert-Equal $lookups.Count 1 'Initial enrollment must inspect persisted runner identity, not only a local initialized flag.'
        $lookup = Get-SourceExpression $lookups[0].Right
        Assert-True ($lookup -is [Management.Automation.Language.ArrayExpressionAst] -and
            $lookup.SubExpression.Statements.Count -eq 1) 'The persisted-runner lookup must have a single explicit filter.'
        $pipeline = $lookup.SubExpression.Statements[0]
        Assert-True ($pipeline -is [Management.Automation.Language.PipelineAst] -and
            $pipeline.PipelineElements.Count -eq 2) 'The persisted-runner lookup must not discard offline history through extra filters.'
        # The native endpoint returns runner_records(corp_id) at top-level
        # response.runners, including connected, grace and offline identities.
        Assert-True (Test-SourceMember $pipeline.PipelineElements[0] 'snapshot' 'runners') 'The lookup must consume the native persisted runner summaries.'
        $filter = $pipeline.PipelineElements[1]
        Assert-True ($filter -is [Management.Automation.Language.CommandAst] -and
            $filter.GetCommandName() -eq 'Where-Object' -and $filter.CommandElements.Count -eq 4) 'Identity matching must not be restricted to connected/status-active runners.'
        Assert-Equal $filter.CommandElements[1].Value 'id' 'The persisted lookup must match runner ID.'
        Assert-Equal $filter.CommandElements[2].ParameterName 'eq' 'The persisted lookup must require exact ID equality.'
        Assert-True ($filter.CommandElements[3] -is [Management.Automation.Language.VariableExpressionAst] -and
            $filter.CommandElements[3].VariablePath.UserPath -eq 'runnerId') 'The lookup must match the requested retained runner ID.'
        $guards = @($start.FindAll({ param($node)
            $node -is [Management.Automation.Language.IfStatementAst]
        }, $true) | Where-Object {
            $condition = Get-SourceExpression $_.Clauses[0].Item1
            $condition -is [Management.Automation.Language.BinaryExpressionAst] -and
                $condition.Operator -eq 'Or' -and
                (Test-SourceMember $condition.Left 'state' 'identity_initialized') -and
                (Test-SourceMember $condition.Right 'existingRunner' 'Count')
        })
        Assert-Equal $guards.Count 1 'Either established local identity or a persisted runner ID must reject automatic enrollment.'
        $guard = $guards[0]
        Assert-True ($guard.Clauses[0].Item2.Statements.Count -eq 1 -and
            $guard.Clauses[0].Item2.Statements[0] -is [Management.Automation.Language.ThrowStatementAst]) 'Existing identity must fail closed, not fall through into enrollment.'
        Assert-True ([object]::ReferenceEquals($guard.Parent, $lookups[0].Parent) -and
            $lookups[0].Extent.EndOffset -lt $guard.Extent.StartOffset) 'The identity lookup and rejection must run together in order.'
        $enrollments = @($commands | Where-Object {
            $_.GetCommandName() -eq 'Invoke-RestMethod' -and $_.Extent.Text -match '/runners/enroll'
        })
        Assert-Equal $enrollments.Count 1 'The guard must cover the actual enrollment call.'
        $inside = $false
        for ($parent = $enrollments[0].Parent; $null -ne $parent; $parent = $parent.Parent) {
            if ([object]::ReferenceEquals($parent, $guard.Parent)) { $inside = $true; break }
        }
        Assert-True ($inside -and $guard.Extent.EndOffset -lt $enrollments[0].Extent.StartOffset) 'Existing identity must be rejected before the enrollment effect.'
    }
    Invoke-Case 'Factory policy paths are canonicalized against the checkout before persistence and launch' {
        $pathAssignments = @($start.FindAll({ param($node)
            $node -is [Management.Automation.Language.AssignmentStatementAst] -and
                $node.Left -is [Management.Automation.Language.VariableExpressionAst] -and
                $node.Left.VariablePath.UserPath -eq 'policyPath'
        }, $true))
        Assert-Equal $pathAssignments.Count 1 'A policy filename must be resolved before changing the child working directory.'
        $resolve = Get-SourceExpression $pathAssignments[0].Right
        Assert-True ($resolve -is [Management.Automation.Language.InvokeMemberExpressionAst] -and
            $resolve.Static -and $resolve.Expression -is [Management.Automation.Language.TypeExpressionAst] -and
            $resolve.Expression.TypeName.FullName -in @('IO.Path', 'System.IO.Path') -and
            $resolve.Member.Value -eq 'GetFullPath' -and $resolve.Arguments.Count -eq 2) 'Policy resolution must use an explicit base path, not the process working directory.'
        Assert-True (Test-SourceMember $resolve.Arguments[0] 'factory' 'verification_policy_file') 'Resolve the selected persisted policy filename.'
        Assert-True ($resolve.Arguments[1] -is [Management.Automation.Language.VariableExpressionAst] -and
            $resolve.Arguments[1].VariablePath.UserPath -eq 'root') 'The policy base must be the checkout root, not the dotenv guard directory.'
        $canonical = @($start.FindAll({ param($node)
            $node -is [Management.Automation.Language.AssignmentStatementAst]
        }, $true) | Where-Object { Test-SourceMember $_.Left 'factory' 'verification_policy_file' })
        Assert-Equal $canonical.Count 1 'The canonical filename must replace the selected value exactly once.'
        $value = Get-SourceExpression $canonical[0].Right
        Assert-True ($value -is [Management.Automation.Language.MemberExpressionAst] -and
            $value.Member.Value -eq 'Path') 'Persist the resolved filesystem path, not a provider object or the original relative string.'
        $literalResolutions = @($canonical[0].Right.FindAll({ param($node)
            $node -is [Management.Automation.Language.CommandAst] -and $node.GetCommandName() -eq 'Resolve-Path'
        }, $true))
        Assert-Equal $literalResolutions.Count 1 'The canonical policy must be resolved literally.'
        Assert-True ($literalResolutions[0].CommandElements.Count -eq 3 -and
            $literalResolutions[0].CommandElements[1].ParameterName -eq 'LiteralPath' -and
            $literalResolutions[0].CommandElements[2].VariablePath.UserPath -eq 'policyPath') 'Bracket characters in policy paths must not expand as wildcards.'
        $configurations = @($start.FindAll({ param($node)
            $node -is [Management.Automation.Language.AssignmentStatementAst] -and
                $node.Left -is [Management.Automation.Language.VariableExpressionAst] -and
                $node.Left.VariablePath.UserPath -eq 'configuration'
        }, $true))
        Assert-Equal $configurations.Count 1 'The source guard must identify the persisted configuration construction.'
        Assert-True ($canonical[0].Extent.EndOffset -lt $configurations[0].Extent.StartOffset) 'Canonicalization must happen before the configuration is persisted.'
        $options = @($start.FindAll({ param($node)
            $node -is [Management.Automation.Language.StringConstantExpressionAst] -and
                $node.Value -eq '--verification-policy-file'
        }, $true))
        Assert-Equal $options.Count 1 'The guard must cover the actual native policy argument.'
        $arguments = $options[0].Parent
        Assert-True ($arguments -is [Management.Automation.Language.ArrayLiteralAst] -and
            $arguments.Elements.Count -eq 2 -and
            (Test-SourceMember $arguments.Elements[1] 'factory' 'verification_policy_file')) 'The controller must receive the canonicalized policy field.'
    }
    Invoke-Case 'starter never deletes enrollment or credentials without an explicit reset/rotation guard' {
        foreach ($command in $commands | Where-Object {
            $_.GetCommandName() -eq 'Remove-Item' -and $_.Extent.Text -match 'credential|enrollment'
        }) {
            $guards = @(Get-ConditionalAncestors $command | Where-Object {
                $_ -is [Management.Automation.Language.IfStatementAst] -and
                ($_.Clauses | ForEach-Object { $_.Item1.Extent.Text }) -match 'Rotate|Reset|Re.?enroll'
            })
            Assert-True ($guards.Count -gt 0) 'Credential/enrollment deletion requires an explicit reset/rotation condition.'
        }
    }
    Invoke-Case 'Compose operations are guarded by external-database selection' {
        $assignments = @($start.FindAll({ param($node)
            $node -is [Management.Automation.Language.AssignmentStatementAst] -and
                $node.Left -is [Management.Automation.Language.VariableExpressionAst]
        }, $true))
        $modeAssignments = @($assignments | Where-Object { $_.Left.VariablePath.UserPath -eq 'dbMode' })
        Assert-Equal $modeAssignments.Count 1 'Database mode must be selected once, without a later override.'
        $selection = $modeAssignments[0].Right
        Assert-True ($selection -is [Management.Automation.Language.IfStatementAst]) 'Database mode must explicitly prioritize an external URL.'
        $condition = Get-SourceExpression $selection.Clauses[0].Item1
        $external = Get-SourceExpression $selection.Clauses[0].Item2
        Assert-True ($condition -is [Management.Automation.Language.VariableExpressionAst] -and
            $condition.VariablePath.UserPath -eq 'databaseUrl') 'A supplied database URL must control the first mode-selection branch.'
        Assert-True ($external -is [Management.Automation.Language.StringConstantExpressionAst] -and
            $external.Value -ceq 'external') 'A supplied URL must choose external, ahead of saved or default database mode.'
        $urlReads = @($assignments | Where-Object {
            $_.Left.VariablePath.UserPath -eq 'databaseUrl' -and
                $_.Extent.EndOffset -lt $modeAssignments[0].Extent.StartOffset
        } | Sort-Object { $_.Extent.StartOffset })
        Assert-True ($urlReads.Count -gt 0) 'Mode selection must receive the explicit DATABASE_URL.'
        $urlRead = Get-SourceExpression $urlReads[-1].Right
        # Inspect the call as syntax only; never invoke it or read the real URL.
        Assert-True ($urlRead -is [Management.Automation.Language.InvokeMemberExpressionAst] -and
            $urlRead.Static -and $urlRead.Expression -is [Management.Automation.Language.TypeExpressionAst] -and
            $urlRead.Expression.TypeName.FullName -in @('Environment', 'System.Environment') -and
            $urlRead.Member.Value -eq 'GetEnvironmentVariable' -and $urlRead.Arguments.Count -eq 2 -and
            $urlRead.Arguments[0] -is [Management.Automation.Language.StringConstantExpressionAst] -and
            $urlRead.Arguments[0].Value -ceq 'DATABASE_URL' -and
            $urlRead.Arguments[1] -is [Management.Automation.Language.StringConstantExpressionAst] -and
            $urlRead.Arguments[1].Value -ceq 'Process') 'The selected URL must come directly from the explicit process DATABASE_URL.'
        $composeCommands = @($commands | Where-Object {
            $_.GetCommandName() -match '^docker(\.exe)?$' -and $_.Extent.Text -match '\bcompose\b'
        })
        Assert-True ($composeCommands.Count -gt 0) 'The source matcher must find the actual managed-database Compose operations.'
        foreach ($command in $composeCommands) {
            Assert-True (Test-ComposeGuard $command) 'Every Compose operation must require dbMode=managed; an external URL must bypass it.'
        }
        # Negative parser fixtures prevent a variable-name-only check from
        # accidentally accepting an OR bypass or the opposite branch.
        foreach ($unsafeSource in @(
            'docker compose up -d',
            'if ($needsServer -or $dbMode -eq ''managed'') { docker compose up -d }',
            'if ($dbMode -eq ''managed'') { } else { docker compose up -d }',
            'if ($dbMode -eq ''external'') { docker compose up -d }'
        )) {
            $fixtureTokens = $null
            $fixtureErrors = $null
            $fixture = [Management.Automation.Language.Parser]::ParseInput($unsafeSource, [ref]$fixtureTokens, [ref]$fixtureErrors)
            Assert-Equal $fixtureErrors.Count 0 'The negative source fixture must parse.'
            $command = $fixture.Find({ param($node)
                $node -is [Management.Automation.Language.CommandAst] -and $node.GetCommandName() -eq 'docker'
            }, $true)
            Assert-False (Test-ComposeGuard $command) 'The source guard accepted an external-database Compose bypass.'
        }
    }
    Invoke-Case 'stopper does not rediscover or terminate a descendant tree from saved PIDs' {
        Assert-False ($stop.Extent.Text -match 'Get-CimInstance|ParentProcessId|Add-ProcessTree|taskkill|Stop-Process\s+-Id') 'Stop must use verified root ownership, not PID-based descendant enumeration.'
        Assert-True ($stop.Extent.Text -match 'Stop-LocalOwnedProcess') 'Stop must delegate to the identity-verifying lifecycle boundary.'
    }
}

try {
    if ($Suite -eq 'Module') { Invoke-ModuleCases } else { Invoke-SourceCases }
} catch {
    $script:Cases.Add(@{ name = 'suite setup and execution'; passed = $false; error = $_.Exception.Message })
} finally {
    if ($script:FixtureRoot) {
        Invoke-Case 'cleanup reaps only held synthetic handles and removes only its temporary tree' {
            if ($script:Lease -and [IO.File]::Exists($script:Lease)) {
                $null = Assert-InFixture $script:Lease
                Remove-Item -LiteralPath $script:Lease -Force
            }
            $script:Cleanup.created_processes = $script:Processes.Count
            $cleanupFailures = 0
            foreach ($owned in $script:Processes) {
                try { Stop-FixtureHandle $owned }
                catch { $cleanupFailures++ }
                if (!$owned.process.HasExited) { $script:Cleanup.remaining_processes++ }
                $owned.process.Dispose()
            }
            Assert-Equal $cleanupFailures 0 'Synthetic process cleanup could not be verified; temporary evidence was preserved.'
            Assert-Equal $script:Cleanup.remaining_processes 0 'A synthetic process survived cleanup.'
            foreach ($junction in $script:Junctions) {
                $full = Assert-InFixture $junction
                Assert-True ([bool]((Get-Item -LiteralPath $full -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) 'Expected test-created junction changed before cleanup.'
                Remove-Item -LiteralPath $full -Force # Remove the link, never recurse through it.
            }
            $full = Assert-InFixture $script:FixtureRoot
            Assert-True ([IO.Path]::GetFileName($full).StartsWith('ecorp local lifecycle ')) 'Temporary root ownership marker is missing.'
            Assert-False ([bool]((Get-Item -LiteralPath $full -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) 'Refusing recursive deletion of a redirected temporary root.'
            Remove-Item -LiteralPath $full -Recurse -Force
            $script:Cleanup.temp_removed = ![IO.Directory]::Exists($full)
            Assert-True $script:Cleanup.temp_removed 'The verified task-created temporary tree was not removed.'
        }
    }
}

$report = @{
    suite = $Suite
    scope = 'synthetic-only; not native startup acceptance'
    powershell = $PSVersionTable.PSVersion.ToString()
    cases = @($script:Cases.ToArray())
    cleanup = $script:Cleanup
}
Write-Output ('ECORP_LOCAL_STACK_TEST_RESULT=' + ($report | ConvertTo-Json -Depth 12 -Compress))
if (@($script:Cases | Where-Object { !$_.passed }).Count) { exit 1 }
