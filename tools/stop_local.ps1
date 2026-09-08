#requires -Version 7.4
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'local_stack.psm1') -Force
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
. (Join-Path $PSScriptRoot 'local_stack_operation.ps1')
Invoke-LocalStackOperation -Workspace $root -Action {
    $stateFile = Join-Path $root 'output\local-pids.json'
    $state = Read-LocalStackState -Path $stateFile -Workspace $root
    if (!$state) {
        Write-Host 'No local ECorp ownership record exists. Nothing was stopped.'
        return
    }
    if ($state.schema_version -ne 2) {
        throw 'The old PID-only record is preserved, but cannot authorize process control. No process or descendant was stopped.'
    }

    $stopped = 0
    $unverified = 0
    foreach ($role in @('factoryController', 'runner', 'server', 'web')) {
        if (!$state.processes.ContainsKey($role) -or !$state.processes[$role]) { continue }
        $record = $state.processes[$role]
        if (Stop-LocalOwnedProcess -Record $record -Workspace $root) {
            $record.stopped_at = [DateTime]::UtcNow.ToString('o')
            $record.stop_outcome = 'verified_root_stopped'
            $state[$role] = $null
            $stopped++
        } else {
            $current = Get-LocalProcessIdentity -ProcessId ([int]$record.pid)
            if ($current) {
                $record.stop_outcome = 'identity_not_verified_preserved'
                $unverified++
            } else {
                $record.stop_outcome = 'already_absent'
                $state[$role] = $null
            }
        }
        Save-LocalStackState -Path $stateFile -State $state -Workspace $root
    }
    $state.last_stop_at = [DateTime]::UtcNow.ToString('o')
    Save-LocalStackState -Path $stateFile -State $state -Workspace $root
    Write-Host "Stopped $stopped verified local ECorp process(es). $unverified unverified/reused PID(s) were left untouched."
    Write-Host 'The database, credentials, provider homes, worktrees, logs and ownership history are retained.'
}

