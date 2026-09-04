[CmdletBinding()]
param(
    [switch]$SkipInstall,
    [switch]$SkipBuild,
    [switch]$SkipFactoryController
)

$ErrorActionPreference = 'Stop'

function ConvertTo-ProcessArgument {
    param([Parameter(Mandatory)][string]$Value)

    if ($Value.Contains('"')) {
        throw 'Process arguments cannot contain a double quote.'
    }
    $trailingBackslashes = $Value.Length - $Value.TrimEnd('\').Length
    $escaped = $Value + ('\' * $trailingBackslashes)
    "`"$escaped`""
}

$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$output = Join-Path $root 'output'
$serverPort = 8791
$webPort = 5187
$databaseUrl = if ($env:DATABASE_URL) {
    $env:DATABASE_URL
} else {
    'postgres://crony:crony@127.0.0.1:54329/crony'
}
$sourceRepository = if ($env:CRONY_SOURCE_REPOSITORY) {
    (Resolve-Path -LiteralPath $env:CRONY_SOURCE_REPOSITORY).Path
} else {
    $root
}
$sourceBaseRef = if ($env:CRONY_SOURCE_BASE_REF) {
    $env:CRONY_SOURCE_BASE_REF
} else {
    'HEAD'
}
$runnerWorkspace = if ($env:CRONY_RUNNER_WORKSPACE) {
    $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath(
        $env:CRONY_RUNNER_WORKSPACE
    )
} else {
    Join-Path $root 'output\runner'
}
$factoryWatchEnabled = -not $SkipFactoryController -and $env:ECORP_FACTORY_WATCH -eq '1'
$factoryAdapter = if ($env:ECORP_FACTORY_ADAPTER) {
    $env:ECORP_FACTORY_ADAPTER
} else {
    'github-copilot'
}
$factoryBudgetTokens = if ($env:ECORP_FACTORY_BUDGET_TOKENS) {
    [int64]$env:ECORP_FACTORY_BUDGET_TOKENS
} else {
    1000000
}
$factoryBudgetCost = if ($env:ECORP_FACTORY_BUDGET_COST_MICROUSD) {
    [int64]$env:ECORP_FACTORY_BUDGET_COST_MICROUSD
} else {
    1000000
}

New-Item -ItemType Directory -Path $output -Force | Out-Null
New-Item -ItemType Directory -Path $runnerWorkspace -Force | Out-Null

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
        cargo build -p crony-server -p crony-runner -p crony-cli
    }

    $logs = @(
        'server.stdout.log',
        'server.stderr.log',
        'runner.stdout.log',
        'runner.stderr.log',
        'factory-controller.stdout.log',
        'factory-controller.stderr.log',
        'web.stdout.log',
        'web.stderr.log'
    )
    foreach ($log in $logs) {
        Remove-Item -LiteralPath (Join-Path $output $log) -Force -ErrorAction SilentlyContinue
    }

    $credentialFile = Join-Path $output 'runner\credential.json'
    $enrollmentFile = Join-Path $output 'runner\enrollment.token'
    New-Item -ItemType Directory -Path (Split-Path $credentialFile -Parent) -Force | Out-Null
    Remove-Item -LiteralPath $credentialFile -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $enrollmentFile -Force -ErrorAction SilentlyContinue

    $server = Start-Process `
        -FilePath (Join-Path $root 'target\debug\crony-server.exe') `
        -ArgumentList @(
            '--bind', "127.0.0.1:$serverPort",
            '--database-url', $databaseUrl
        ) `
        -WorkingDirectory $root `
        -RedirectStandardOutput (Join-Path $output 'server.stdout.log') `
        -RedirectStandardError (Join-Path $output 'server.stderr.log') `
        -PassThru `
        -WindowStyle Hidden
    @{
        server = $server.Id
        runner = $null
        factoryController = $null
        web = $null
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $output 'local-pids.json')

    $deadline = (Get-Date).AddMinutes(2)
    do {
        try {
            $health = Invoke-RestMethod -Uri "http://127.0.0.1:$serverPort/health" -TimeoutSec 3
            if ($health.status -eq 'ok') {
                break
            }
        } catch {
            Start-Sleep -Seconds 1
        }
    } while ((Get-Date) -lt $deadline)
    if ($health.status -ne 'ok') {
        throw 'ECorp server did not become ready.'
    }

    $demo = Invoke-RestMethod `
        -Method Post `
        -Uri "http://127.0.0.1:$serverPort/api/demo/bootstrap" `
        -ContentType 'application/json' `
        -Body '{}'
    $enrollment = Invoke-RestMethod `
        -Method Post `
        -Uri "http://127.0.0.1:$serverPort/api/corps/$($demo.corp_id)/runners/enroll" `
        -ContentType 'application/json' `
        -Body (@{
            actor_id = $demo.alice_actor_id
            runner_id = 'runner-local'
            expires_in_seconds = 600
        } | ConvertTo-Json)
    Set-Content -LiteralPath $enrollmentFile -Value $enrollment.enrollment_token -NoNewline
    & icacls.exe $enrollmentFile /inheritance:r /grant:r "$env:USERNAME`:F" *> $null
    if ($LASTEXITCODE -ne 0) {
        Write-Warning "Could not narrow the enrollment token ACL; delete $enrollmentFile after startup."
    }

    $runner = Start-Process `
        -FilePath (Join-Path $root 'target\debug\crony-runner.exe') `
        -ArgumentList @(
            '--server-ws', "ws://127.0.0.1:$serverPort/ws/runner",
            '--corp-id', $demo.corp_id,
            '--credential-file', (ConvertTo-ProcessArgument $credentialFile),
            '--enrollment-token-file', (ConvertTo-ProcessArgument $enrollmentFile),
            '--workspace', (ConvertTo-ProcessArgument $runnerWorkspace),
            '--source-repository', (ConvertTo-ProcessArgument $sourceRepository),
            '--source-base-ref', $sourceBaseRef,
            '--fake-agent-script', (ConvertTo-ProcessArgument (Join-Path $root 'scripts\fake-agent.mjs'))
        ) `
        -WorkingDirectory $root `
        -RedirectStandardOutput (Join-Path $output 'runner.stdout.log') `
        -RedirectStandardError (Join-Path $output 'runner.stderr.log') `
        -PassThru `
        -WindowStyle Hidden

    $factoryController = $null
    if ($factoryWatchEnabled) {
        $factoryController = Start-Process `
            -FilePath (Join-Path $root 'target\debug\crony-cli.exe') `
            -ArgumentList @(
                '--server', "http://127.0.0.1:$serverPort",
                'factory-watch',
                $demo.corp_id,
                $demo.alice_actor_id,
                '--adapter', $factoryAdapter,
                '--budget-tokens', $factoryBudgetTokens,
                '--budget-cost-microusd', $factoryBudgetCost,
                '--source-repository-path', (ConvertTo-ProcessArgument $sourceRepository),
                '--source-base-ref', $sourceBaseRef
            ) `
            -WorkingDirectory $root `
            -RedirectStandardOutput (Join-Path $output 'factory-controller.stdout.log') `
            -RedirectStandardError (Join-Path $output 'factory-controller.stderr.log') `
            -PassThru `
            -WindowStyle Hidden
    }

    $web = Start-Process `
        -FilePath (Join-Path $root 'apps\web\node_modules\.bin\vite.cmd') `
        -ArgumentList @(
            '--host', '127.0.0.1',
            '--port', $webPort,
            '--strictPort'
        ) `
        -WorkingDirectory (Join-Path $root 'apps\web') `
        -RedirectStandardOutput (Join-Path $output 'web.stdout.log') `
        -RedirectStandardError (Join-Path $output 'web.stderr.log') `
        -PassThru `
        -WindowStyle Hidden

    @{
        server = $server.Id
        runner = $runner.Id
        factoryController = if ($factoryController) { $factoryController.Id } else { $null }
        web = $web.Id
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $output 'local-pids.json')

    $deadline = (Get-Date).AddMinutes(2)
    do {
        try {
            $health = Invoke-RestMethod -Uri "http://127.0.0.1:$serverPort/health" -TimeoutSec 3
            $page = Invoke-WebRequest -Uri "http://127.0.0.1:$webPort" -TimeoutSec 3
            $factoryReady = -not $factoryWatchEnabled
            if ($factoryWatchEnabled) {
                $snapshot = Invoke-RestMethod `
                    -Uri "http://127.0.0.1:$serverPort/api/corps/$($demo.corp_id)/snapshot?actor_id=$($demo.alice_actor_id)" `
                    -TimeoutSec 3
                $factoryReady = @($snapshot.snapshot.factory_controllers |
                    Where-Object { $_.status -ne 'offline' }).Count -ge 1
            }
            if ($health.status -eq 'ok' -and $health.runners -ge 1 -and
                $page.StatusCode -eq 200 -and $factoryReady) {
                break
            }
        } catch {
            Start-Sleep -Seconds 1
        }
    } while ((Get-Date) -lt $deadline)

    if ($health.status -ne 'ok' -or $health.runners -lt 1 -or
        $page.StatusCode -ne 200 -or -not $factoryReady) {
        throw 'ECorp local stack did not become ready.'
    }

    Write-Host "ECorp server: http://127.0.0.1:$serverPort"
    Write-Host "ECorp web:    http://127.0.0.1:$webPort"
    Write-Host "Runner count: $($health.runners)"
    Write-Host "Source repo:  $sourceRepository ($sourceBaseRef)"
    Write-Host "Worktrees:    $runnerWorkspace"
    Write-Host "Factory:      $(if ($factoryWatchEnabled) { 'watching' } else { 'not configured' })"
} finally {
    Pop-Location
}
