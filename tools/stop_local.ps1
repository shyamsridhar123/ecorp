[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$pidFile = Join-Path $root 'output\local-pids.json'

if (-not (Test-Path -LiteralPath $pidFile)) {
    Write-Host 'No Crony local PID file exists.'
    exit 0
}

$roots = Get-Content -LiteralPath $pidFile -Raw | ConvertFrom-Json
$rootIds = @($roots.server, $roots.runner, $roots.web) |
    Where-Object { $_ -is [int] -or $_ -is [long] } |
    ForEach-Object { [int]$_ }

$all = Get-CimInstance Win32_Process
$targets = New-Object 'System.Collections.Generic.HashSet[int]'

function Add-ProcessTree {
    param([int]$ProcessId)
    if (-not $targets.Add($ProcessId)) {
        return
    }
    foreach ($child in $all | Where-Object ParentProcessId -eq $ProcessId) {
        Add-ProcessTree -ProcessId ([int]$child.ProcessId)
    }
}

foreach ($rootId in $rootIds) {
    Add-ProcessTree -ProcessId $rootId
}

foreach ($processId in ($targets | Sort-Object -Descending)) {
    Stop-Process -Id $processId -Force -ErrorAction SilentlyContinue
}

Remove-Item -LiteralPath $pidFile -Force -ErrorAction SilentlyContinue
Write-Host "Stopped $($targets.Count) Crony process(es)."

