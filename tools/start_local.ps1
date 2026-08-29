[CmdletBinding()]
param(
    [switch]$SkipInstall,
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$output = Join-Path $root 'output'
$serverPort = 8791
$webPort = 5187

New-Item -ItemType Directory -Path $output -Force | Out-Null

& (Join-Path $PSScriptRoot 'stop_local.ps1')

foreach ($port in @($serverPort, $webPort)) {
    $listener = Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue
    if ($listener) {
        $owners = $listener.OwningProcess | Sort-Object -Unique
        throw "Port $port is already in use by process id(s): $($owners -join ', ')"
    }
}

Push-Location $root
try {
    docker compose -f deploy/compose/docker-compose.yml up -d

    $deadline = (Get-Date).AddMinutes(2)
    do {
        docker compose -f deploy/compose/docker-compose.yml exec -T postgres `
            pg_isready -U crony -d crony *> $null
        if ($LASTEXITCODE -eq 0) {
            $healthy = $true
            break
        }
        Start-Sleep -Seconds 1
    } while ((Get-Date) -lt $deadline)
    if (-not $healthy) {
        throw 'Postgres did not become healthy.'
    }

    if (-not $SkipInstall) {
        pnpm install --frozen-lockfile
    }
    if (-not $SkipBuild) {
        cargo build -p crony-server -p crony-runner
    }

    $logs = @(
        'server.stdout.log',
        'server.stderr.log',
        'runner.stdout.log',
        'runner.stderr.log',
        'web.stdout.log',
        'web.stderr.log'
    )
    foreach ($log in $logs) {
        Remove-Item -LiteralPath (Join-Path $output $log) -Force -ErrorAction SilentlyContinue
    }

    $server = Start-Process `
        -FilePath (Join-Path $root 'target\debug\crony-server.exe') `
        -ArgumentList @(
            '--bind', "127.0.0.1:$serverPort",
            '--database-url', 'postgres://crony:crony@127.0.0.1:54329/crony'
        ) `
        -WorkingDirectory $root `
        -RedirectStandardOutput (Join-Path $output 'server.stdout.log') `
        -RedirectStandardError (Join-Path $output 'server.stderr.log') `
        -PassThru `
        -WindowStyle Hidden

    $runner = Start-Process `
        -FilePath (Join-Path $root 'target\debug\crony-runner.exe') `
        -ArgumentList @(
            '--server-ws', "ws://127.0.0.1:$serverPort/ws/runner",
            '--workspace', (Join-Path $root 'output\runner'),
            '--fake-agent-script', (Join-Path $root 'scripts\fake-agent.mjs')
        ) `
        -WorkingDirectory $root `
        -RedirectStandardOutput (Join-Path $output 'runner.stdout.log') `
        -RedirectStandardError (Join-Path $output 'runner.stderr.log') `
        -PassThru `
        -WindowStyle Hidden

    $web = Start-Process `
        -FilePath 'pnpm.cmd' `
        -ArgumentList @(
            '--dir', 'apps/web', 'dev',
            '--host', '127.0.0.1',
            '--port', $webPort,
            '--strictPort'
        ) `
        -WorkingDirectory $root `
        -RedirectStandardOutput (Join-Path $output 'web.stdout.log') `
        -RedirectStandardError (Join-Path $output 'web.stderr.log') `
        -PassThru `
        -WindowStyle Hidden

    @{
        server = $server.Id
        runner = $runner.Id
        web = $web.Id
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $output 'local-pids.json')

    $deadline = (Get-Date).AddMinutes(2)
    do {
        try {
            $health = Invoke-RestMethod -Uri "http://127.0.0.1:$serverPort/health" -TimeoutSec 3
            $page = Invoke-WebRequest -Uri "http://127.0.0.1:$webPort" -TimeoutSec 3
            if ($health.status -eq 'ok' -and $health.runners -ge 1 -and $page.StatusCode -eq 200) {
                break
            }
        } catch {
            Start-Sleep -Seconds 1
        }
    } while ((Get-Date) -lt $deadline)

    if ($health.status -ne 'ok' -or $health.runners -lt 1 -or $page.StatusCode -ne 200) {
        throw 'Crony local stack did not become ready.'
    }

    Write-Host "Crony server: http://127.0.0.1:$serverPort"
    Write-Host "Crony web:    http://127.0.0.1:$webPort"
    Write-Host "Runner count: $($health.runners)"
} finally {
    Pop-Location
}
