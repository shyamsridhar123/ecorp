#requires -Version 7.4
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$FixtureRoot,
    [string]$PgBin = $env:PGBIN,
    [ValidateRange(1024, 65535)][int]$ServerPort = 18437,
    [ValidateRange(1024, 65535)][int]$PostgresPort = 55437,
    [switch]$DryRun,
    [switch]$Execute
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (!$IsWindows) { throw 'The positive external-adapter fixture requires Windows Job Objects.' }
if ($DryRun -and $Execute) { throw 'Select preview or execution, not both.' }
$repo = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
Import-Module (Join-Path $PSScriptRoot 'local_stack.psm1') -Force

if (![IO.Path]::IsPathFullyQualified($FixtureRoot) -or !$PgBin -or
    ![IO.Path]::IsPathFullyQualified($PgBin)) { throw 'Explicit absolute fixture and PostgreSQL binary paths are required.' }
$fixture = Get-LocalFullPath $FixtureRoot
if ([System.Management.Automation.WildcardPattern]::ContainsWildcardCharacters($fixture) -or
    [System.Management.Automation.WildcardPattern]::ContainsWildcardCharacters($PgBin)) {
    throw 'Use literal CI fixture and PostgreSQL paths without wildcard metacharacters.'
}
if (!(Split-Path -Leaf $fixture).StartsWith('ecorp-external-adapters-') -or
    (Test-LocalPathEqual $fixture $repo) -or
    $fixture.StartsWith($repo + '\', [StringComparison]::OrdinalIgnoreCase) -or
    (Test-Path -LiteralPath $fixture)) {
    throw 'Use a new ecorp-external-adapters-* directory outside the source worktree; existing fixtures are preserved.'
}
$ancestor = (Resolve-Path -LiteralPath (Split-Path -Parent $fixture)).Path
while ($ancestor) {
    if ((Get-Item -LiteralPath $ancestor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw 'Fixture ancestors must not redirect outside the declared scope.'
    }
    $ancestor = Split-Path -Parent $ancestor
}
$reservedPorts = @(54329, 8791, 8793, 5187, 5291, 15191, 15193)
if ($ServerPort -eq $PostgresPort -or $ServerPort -in $reservedPorts -or $PostgresPort -in $reservedPorts) {
    throw 'Refusing shared/manual stack ports.'
}
$listeners = @(Get-NetTCPConnection -State Listen -ErrorAction SilentlyContinue |
    Where-Object LocalPort -in @($ServerPort, $PostgresPort))
if ($listeners.Count) { throw 'A fixture port is occupied; no existing process will be stopped.' }
$pg = @{}
foreach ($name in @('initdb', 'postgres', 'psql', 'pg_ctl')) {
    $pg[$name] = (Resolve-Path -LiteralPath (Join-Path $PgBin "$name.exe")).Path
}
$serverBinary = (Resolve-Path -LiteralPath (Join-Path $repo 'target/debug/crony-server.exe')).Path
$runnerBinary = (Resolve-Path -LiteralPath (Join-Path $repo 'target/debug/crony-runner.exe')).Path
$node = (Get-Command node -CommandType Application | Select-Object -First 1).Source
$git = (Get-Command git -CommandType Application | Select-Object -First 1).Source
$api = "http://127.0.0.1:$ServerPort"
$evidence = Join-Path $fixture 'evidence'
$source = Join-Path $fixture 'source'
$data = Join-Path $fixture 'pg-data'
$private = Join-Path $fixture 'private'
$preview = [ordered]@{
    suite = 'external-adapters-windows-ci-v1'
    fixture_root = $fixture
    source_worktree = $repo
    server = $api
    postgres_port = $PostgresPort
    node = $node
    evidence = $evidence
    proposed = @(
        'Initialize one fresh loopback-only PostgreSQL cluster with synthetic trust-authenticated data.',
        'Start receipt-owned native server and runner using the existing local_stack.psm1 helpers.',
        'Execute Claude Code and OpenCode through deterministic protocol fixtures, never real accounts.',
        'Verify signed artifact download, session, usage and provider termination before completion.',
        'Stop only owned fixture processes and preserve all data, worktrees and evidence.'
    )
    services_started = $false
    database_writes = $false
    real_provider_calls = 0
    real_github_mutations = 0
}
if (!$Execute) {
    $preview | ConvertTo-Json -Depth 6
    return
}

New-Item -ItemType Directory -Path $fixture | Out-Null
foreach ($directory in @($evidence, $source, $private, (Join-Path $private 'home'))) {
    New-Item -ItemType Directory -Path $directory | Out-Null
}
# Stop dotenv discovery and native Git/provider configuration at this synthetic boundary.
[IO.File]::WriteAllText((Join-Path $fixture '.env'), ('# Synthetic CI fixture only' + [Environment]::NewLine))
[IO.File]::WriteAllText((Join-Path $source 'README.md'), ('# Synthetic external-adapter source' + [Environment]::NewLine))
[IO.File]::WriteAllText((Join-Path $private 'empty.gitconfig'), '')
$childEnv = @{
    GIT_CONFIG_GLOBAL = (Join-Path $private 'empty.gitconfig'); GIT_CONFIG_NOSYSTEM = '1'
    HOME = (Join-Path $private 'home'); USERPROFILE = (Join-Path $private 'home')
    APPDATA = (Join-Path $private 'home'); LOCALAPPDATA = (Join-Path $private 'home')
    CRONY_COPILOT_USE_LOGGED_IN_USER = 'false'
}
$state = @{ schema_version = 2; workspace = $fixture; processes = @{}; test_owned = $true }
$report = @{ suite = $preview.suite; status = 'running'; server = $api; postgres_port = $PostgresPort
    source_worktree = $repo; fixture_root = $fixture; real_provider_calls = 0; real_github_mutations = 0
    started_at = [DateTime]::UtcNow.ToString('o'); cleanup = @() }

function Save-FixtureReceipts {
    Save-LocalStackState -Path (Join-Path $evidence 'processes.json') -State $state -Workspace $fixture
}

function Invoke-FixtureCommand {
    param([string]$Role, [string]$Program, [string[]]$Arguments,
        [hashtable]$Environment = $childEnv, [int]$TimeoutSeconds = 60)
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $Program
    $info.WorkingDirectory = $fixture
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    foreach ($argument in $Arguments) { $info.ArgumentList.Add($argument) }
    $info.Environment.Clear()
    $safeEnvironment = New-LocalProcessEnvironment -Environment $Environment
    foreach ($key in $safeEnvironment.Keys) {
        if ($null -ne $safeEnvironment[$key]) { $info.Environment[$key] = [string]$safeEnvironment[$key] }
    }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $info
    try {
        if (!$process.Start()) { throw "$Role did not start." }
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (!$process.WaitForExit($TimeoutSeconds * 1000)) {
            # Exact retained handle, never a broad PID/name or descendant sweep.
            $process.Kill()
            [void]$process.WaitForExit(15000)
            throw "$Role exceeded its bounded deadline."
        }
        $out = $stdout.GetAwaiter().GetResult()
        $err = $stderr.GetAwaiter().GetResult()
        [IO.File]::WriteAllText((Join-Path $evidence "$Role.stdout.log"), $out)
        [IO.File]::WriteAllText((Join-Path $evidence "$Role.stderr.log"), $err)
        if ($process.ExitCode -ne 0) { throw "$Role failed (exit $($process.ExitCode)); inspect its retained fixture logs." }
        return $out.Trim()
    } finally { $process.Dispose() }
}

function Start-FixtureProcess {
    param([string]$Role, [string]$Program, [string[]]$Arguments, [hashtable]$Environment = $childEnv)
    $parameters = @{
        Role = $Role; Workspace = $fixture; FilePath = $Program; ArgumentList = $Arguments
        WorkingDirectory = $fixture; LogDirectory = $evidence; Environment = $Environment
    }
    $state.processes[$Role] = Start-LocalOwnedProcess @parameters
    Save-FixtureReceipts
}

function Start-FixturePostgres {
    # PostgreSQL 17 pg_ctl uses CreateRestrictedProcess on Windows. Hosted runners
    # are elevated; directly spawning postgres.exe is intentionally rejected.
    $startedAfter = [DateTimeOffset]::UtcNow
    $log = Join-Path $evidence 'postgres.log'
    $startError = $null
    $report.postgres_start_unverified = $true
    $pgControl = $null
    try {
        # The background postmaster can inherit a captured pipe even with -l.
        # File-backed stdio avoids waiting forever for that pipe's EOF.
        $arguments = @('-D', $data, '-l', $log, '-o', "-p $PostgresPort -h 127.0.0.1", '-w', '-t', '30', 'start')
        $parameters = @{
            FilePath = $pg.pg_ctl; WorkingDirectory = $fixture; WindowStyle = 'Hidden'; PassThru = $true
            ArgumentList = @($arguments | ForEach-Object { ConvertTo-LocalProcessArgument $_ })
            Environment = (New-LocalProcessEnvironment -Environment $childEnv)
            RedirectStandardOutput = (Join-Path $evidence 'pg-start.stdout.log')
            RedirectStandardError = (Join-Path $evidence 'pg-start.stderr.log')
        }
        $pgControl = Start-Process @parameters
        [void]$pgControl.Handle
        if (!$pgControl.WaitForExit(45000)) {
            $pgControl.Kill()
            [void]$pgControl.WaitForExit(15000)
            throw 'pg_ctl exceeded its bounded startup deadline; inspect its retained logs.'
        }
        if ($pgControl.ExitCode -ne 0) { throw 'pg_ctl startup failed; inspect its retained logs.' }
    } catch { $startError = $_ }
    finally { if ($pgControl) { $pgControl.Dispose() } }

    # pg_ctl's Windows process handle is the launcher shell, not the postmaster.
    # Combine its fresh private PID/data receipt with executable, creation-time
    # for cleanup, followed by listener/SQL identity checks before test admission.
    $pidFile = Join-Path $data 'postmaster.pid'
    if (Test-Path -LiteralPath $pidFile) {
        $receipt = @(Get-Content -LiteralPath $pidFile -First 2)
        $postgresId = 0
        if ($receipt.Count -ne 2 -or ![int]::TryParse($receipt[0], [ref]$postgresId) -or
            $postgresId -le 0 -or !(Test-LocalPathEqual $receipt[1] $data)) {
            throw 'The new PostgreSQL startup receipt is invalid; no PID was adopted.'
        }
        $identity = Get-LocalProcessIdentity -ProcessId $postgresId
        if (!$identity -or !(Test-LocalPathEqual $identity.executable $pg.postgres) -or
            [DateTimeOffset]$identity.started_utc -lt $startedAfter -or
            [DateTimeOffset]$identity.started_utc -gt [DateTimeOffset]::UtcNow) {
            throw 'The PostgreSQL executable or startup time is unverifiable; retain the fixture.'
        }
        $state.processes.postgres = @{
            role = 'postgres'; workspace = $fixture; pid = $postgresId
            executable = $identity.executable; started_utc = $identity.started_utc
            stdout = $log; stderr = $log; launcher = 'pg_ctl restricted process'
        }
        Save-FixtureReceipts
        $report.postgres_start_unverified = $false
    }
    if ($startError) { throw $startError }
    if ($report.postgres_start_unverified) { throw 'pg_ctl returned without a verifiable postmaster receipt.' }
}

function Wait-FixtureReady {
    param([string]$Role, [scriptblock]$Check)
    $deadline = [DateTime]::UtcNow.AddSeconds(60)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (!(Test-LocalOwnedProcess -Record $state.processes[$Role] -Workspace $fixture)) {
            throw "$Role exited or lost its verified process identity; inspect retained fixture logs."
        }
        if (& $Check) { return }
        Start-Sleep -Milliseconds 200
    }
    throw "$Role readiness exceeded 60 seconds."
}

function Invoke-FixturePost {
    param([string]$Route, [hashtable]$Body)
    Invoke-RestMethod -Uri ($api + $Route) -Method Post -ContentType 'application/json' -Body (
        $Body | ConvertTo-Json -Compress) -TimeoutSec 10 -MaximumRedirection 0
}

try {
    Save-FixtureReceipts
    Invoke-FixtureCommand 'git-init' $git @('init', '--initial-branch=main', $source) | Out-Null
    Invoke-FixtureCommand 'git-add' $git @('-C', $source, 'add', 'README.md') | Out-Null
    Invoke-FixtureCommand 'git-commit' $git @('-C', $source, '-c', 'user.name=ECorp CI', '-c',
        'user.email=ci@example.invalid', 'commit', '-m', 'Synthetic external-adapter fixture') | Out-Null
    $report.fixture_source_commit = Invoke-FixtureCommand 'source-before' $git @('-C', $source, 'rev-parse', 'HEAD')
    Invoke-FixtureCommand 'initdb' $pg.initdb @('-D', $data, '-U', 'ecorp_external_ci', '-A', 'trust', '--encoding=UTF8', '--locale=C') | Out-Null
    Start-FixturePostgres
    Wait-FixtureReady 'postgres' {
        @(Get-NetTCPConnection -State Listen -LocalPort $PostgresPort -ErrorAction SilentlyContinue |
            Where-Object OwningProcess -eq $state.processes.postgres.pid).Count -eq 1
    }
    $pgEnv = $childEnv.Clone()
    $pgEnv.PGHOST = '127.0.0.1'; $pgEnv.PGPORT = [string]$PostgresPort
    $pgEnv.PGUSER = 'ecorp_external_ci'; $pgEnv.PGDATABASE = 'postgres'
    $observed = (Invoke-FixtureCommand 'pg-identity' $pg.psql @('-X', '-At', '-v', 'ON_ERROR_STOP=1', '-c',
        "SELECT current_setting('data_directory') || '|' || current_setting('port') || '|' || current_user") $pgEnv).Split('|')
    if (!(Test-LocalPathEqual $observed[0] $data) -or $observed[1] -ne [string]$PostgresPort -or
        $observed[2] -ne 'ecorp_external_ci') { throw 'Synthetic PostgreSQL identity mismatch.' }
    Invoke-FixtureCommand 'create-database' $pg.psql @('-X', '-v', 'ON_ERROR_STOP=1', '-c',
        'CREATE DATABASE ecorp_external_ci') $pgEnv | Out-Null
    $serverEnv = $childEnv.Clone()
    $serverEnv.DATABASE_URL = "postgres://ecorp_external_ci@127.0.0.1:$PostgresPort/ecorp_external_ci"
    Start-FixtureProcess 'server' $serverBinary @('--bind', "127.0.0.1:$ServerPort", '--mode', 'development',
        '--object-store-local-root', (Join-Path $fixture 'artifact-objects')) $serverEnv
    Wait-FixtureReady 'server' {
        try { (Invoke-RestMethod "$api/health" -TimeoutSec 2 -MaximumRedirection 0).status -eq 'ok' }
        catch { $false }
    }
    $demo = Invoke-FixturePost '/api/demo/bootstrap' @{}
    $runnerId = 'runner-external-ci'
    $enrollment = Invoke-FixturePost "/api/corps/$($demo.corp_id)/runners/enroll" @{
        actor_id = $demo.alice_actor_id; runner_id = $runnerId; expires_in_seconds = 600
    }
    $tokenFile = Join-Path $private 'enrollment.token'
    [IO.File]::WriteAllText($tokenFile, $enrollment.enrollment_token)
    $runnerArgs = @(
        '--server-ws', "ws://127.0.0.1:$ServerPort/ws/runner", '--runner-id', $runnerId, '--corp-id', $demo.corp_id,
        '--credential-file', (Join-Path $private 'credential.json'), '--enrollment-token-file', $tokenFile,
        '--workspace', (Join-Path $fixture 'runner-workspaces'), '--source-repository', $source, '--source-base-ref', 'HEAD',
        '--fake-agent-script', (Join-Path $repo 'scripts/fake-agent.mjs'),
        '--claude-command', $node, '--claude-command-arg', (Join-Path $repo 'scripts/fake-external-agent.mjs'),
        '--opencode-command', $node, '--opencode-command-arg', (Join-Path $repo 'scripts/fake-external-agent.mjs'),
        '--codex-command', (Join-Path $private 'disabled-codex.exe'),
        '--copilot-cli-path', (Join-Path $private 'disabled-copilot.exe'), '--copilot-home', (Join-Path $private 'copilot-home'),
        '--connections-directory', (Join-Path $private 'connections'), '--github-command', (Join-Path $private 'disabled-gh.exe')
    )
    Start-FixtureProcess 'runner' $runnerBinary $runnerArgs
    Wait-FixtureReady 'runner' {
        $snapshot = Invoke-RestMethod "$api/api/corps/$($demo.corp_id)/snapshot?actor_id=$($demo.alice_actor_id)" -TimeoutSec 3 -MaximumRedirection 0
        $registered = @($snapshot.runners | Where-Object { $_.id -eq $runnerId -and $_.connected })
        if ($registered.Count -eq 1) {
            [IO.File]::WriteAllText((Join-Path $evidence 'runner-capabilities.json'), ($registered[0] | ConvertTo-Json -Depth 8))
            if (@($registered[0].capabilities | Where-Object { $_.name -in @('claude-code', 'opencode') -and $_.available }).Count -ne 2) {
                throw 'The Windows fixture registered unavailable adapters; inspect runner-capabilities.json.'
            }
            return $true
        }
        return $false
    }
    $testEnv = $childEnv.Clone()
    $testEnv.CRONY_EXTERNAL_ADAPTER_TEST = '1'; $testEnv.CRONY_SERVER_HTTP = $api
    $testEnv.CRONY_EXTERNAL_ADAPTER_OUTPUT = Join-Path $evidence 'e2e-external-adapters.json'
    Invoke-FixtureCommand 'contract' $node @((Join-Path $repo 'tools/e2e_external_adapters.mjs'), '--expect-windows') $testEnv 180 | Out-Null
    $after = Invoke-FixtureCommand 'source-after' $git @('-C', $source, 'rev-parse', 'HEAD')
    $status = Invoke-FixtureCommand 'source-status' $git @('-C', $source, 'status', '--porcelain')
    if ($after -ne $report.fixture_source_commit -or $status) { throw 'The fixture source checkout changed.' }
    $report.source_checkout_unchanged = $true
    $report.status = 'passed'
} catch {
    $report.status = 'failed'
    $report.failure = $_.Exception.Message
    throw
} finally {
    $cleanupFailed = $report.ContainsKey('postgres_start_unverified') -and $report.postgres_start_unverified
    foreach ($role in @('runner', 'server', 'postgres')) {
        if (!$state.processes.ContainsKey($role)) { continue }
        $record = $state.processes[$role]
        try {
            if ($role -eq 'postgres' -and (Test-LocalOwnedProcess -Record $record -Workspace $fixture)) {
                if ([int](Get-Content -LiteralPath (Join-Path $data 'postmaster.pid') -First 1) -ne [int]$record.pid) {
                    throw 'PostgreSQL PID file does not match the owned process.'
                }
                Invoke-FixtureCommand 'pg-stop' $pg.pg_ctl @('-D', $data, '-m', 'fast', '-w', '-t', '30', 'stop') | Out-Null
            } else {
                [void](Stop-LocalOwnedProcess -Record $record -Workspace $fixture)
            }
            if (Get-Process -Id ([int]$record.pid) -ErrorAction SilentlyContinue) {
                throw 'Process exit could not be verified; its receipt and data are retained.'
            }
            $report.cleanup += @{ role = $role; stopped = $true }
        } catch {
            $cleanupFailed = $true
            $report.cleanup += @{ role = $role; stopped = $false; detail = $_.Exception.Message }
        }
    }
    $report.finished_at = [DateTime]::UtcNow.ToString('o')
    if ($cleanupFailed) { $report.status = 'failed'; $report.cleanup_unverified = $true }
    Save-FixtureReceipts
    [IO.File]::WriteAllText((Join-Path $evidence 'fixture-report.json'), ($report | ConvertTo-Json -Depth 8))
    $report | ConvertTo-Json -Depth 8
    if ($cleanupFailed) { throw 'Fixture cleanup is unverified; inspect its retained receipts. No unrelated process was stopped.' }
}
