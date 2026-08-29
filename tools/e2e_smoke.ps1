[CmdletBinding()]
param(
    [string]$Server = 'http://127.0.0.1:8791'
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$reportPath = Join-Path $root 'output\e2e-smoke.json'

function Post-Json {
    param(
        [Parameter(Mandatory)][string]$Uri,
        [Parameter(Mandatory)][object]$Body
    )
    Invoke-RestMethod `
        -Method Post `
        -Uri $Uri `
        -ContentType 'application/json' `
        -Body ($Body | ConvertTo-Json -Depth 20)
}

$health = Invoke-RestMethod -Uri "$Server/health"
if ($health.status -ne 'ok' -or $health.runners -lt 1) {
    throw "Stack is not ready: $($health | ConvertTo-Json -Compress)"
}

$demo = Post-Json -Uri "$Server/api/demo/reset" -Body @{}

$aliceLease = Post-Json `
    -Uri "$Server/api/corps/$($demo.corp_id)/agents/$($demo.worker_agent_id)/lease" `
    -Body @{ actor_id = $demo.alice_actor_id }
if (-not $aliceLease.acquired) {
    throw 'Alice failed to acquire the control lease.'
}

$bobLease = Post-Json `
    -Uri "$Server/api/corps/$($demo.corp_id)/agents/$($demo.worker_agent_id)/lease" `
    -Body @{ actor_id = $demo.bob_actor_id }
if ($bobLease.acquired -or $bobLease.holder_actor_id -ne $demo.alice_actor_id) {
    throw 'Bob replaced an unexpired Alice lease.'
}

$mission = Post-Json `
    -Uri "$Server/api/corps/$($demo.corp_id)/missions" `
    -Body @{
        requested_by = $demo.alice_actor_id
        title = 'Create a verified multiplayer operations artifact.'
    }

$launch = Post-Json `
    -Uri "$Server/api/corps/$($demo.corp_id)/missions/$($mission.mission_id)/launch" `
    -Body @{ requested_by = $demo.alice_actor_id }

$message = Post-Json `
    -Uri "$Server/api/corps/$($demo.corp_id)/agents/$($demo.worker_agent_id)/messages" `
    -Body @{
        actor_id = $demo.alice_actor_id
        lease_token = $aliceLease.token
        text = 'Include the live-control acknowledgement in the run evidence.'
    }
if ($message.delivery -ne 'immediate') {
    throw "Expected immediate message delivery, got $($message.delivery)."
}

$deadline = (Get-Date).AddMinutes(2)
do {
    $snapshot = Invoke-RestMethod -Uri "$Server/api/corps/$($demo.corp_id)/snapshot"
    $run = $snapshot.snapshot.runs | Where-Object id -eq $launch.run_id
    if ($run.status -in @('completed', 'failed', 'cancelled')) {
        break
    }
    Start-Sleep -Milliseconds 500
} while ((Get-Date) -lt $deadline)

if ($run.status -ne 'completed') {
    throw "Run did not complete successfully: $($run | ConvertTo-Json -Depth 10)"
}

$artifactPath = [System.IO.Path]::GetFullPath(
    [System.IO.Path]::Combine($root, $run.artifact_path)
)
$runnerRoot = [System.IO.Path]::GetFullPath((Join-Path $root 'output\runner'))
if (-not $artifactPath.StartsWith($runnerRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Artifact escaped the runner output root: $artifactPath"
}
if (-not (Test-Path -LiteralPath $artifactPath)) {
    throw "Artifact does not exist: $artifactPath"
}

$actualSha = (Get-FileHash -LiteralPath $artifactPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualSha -ne $run.artifact_sha256) {
    throw "Artifact hash mismatch: expected $($run.artifact_sha256), got $actualSha"
}

$eventTypes = @($snapshot.snapshot.events | ForEach-Object type)
foreach ($required in @(
    'control.lease_acquired',
    'control.message_accepted',
    'mission.created',
    'task.created',
    'run.requested',
    'run.started',
    'run.artifact',
    'run.completed'
)) {
    if ($required -notin $eventTypes) {
        throw "Missing required event: $required"
    }
}

$controlOutput = $snapshot.snapshot.events |
    Where-Object {
        $_.type -eq 'run.output' -and
        $_.payload.stream -eq 'control' -and
        $_.payload.text -match 'live-control acknowledgement'
    } |
    Select-Object -First 1
if (-not $controlOutput) {
    throw 'The child process did not acknowledge the live control message.'
}

$report = [ordered]@{
    checked_at = (Get-Date).ToUniversalTime().ToString('o')
    corp_id = $demo.corp_id
    mission_id = $mission.mission_id
    run_id = $launch.run_id
    runner_id = $launch.runner_id
    alice_lease_acquired = $aliceLease.acquired
    bob_conflict_enforced = -not $bobLease.acquired
    live_message_delivery = $message.delivery
    run_status = $run.status
    artifact_path = $artifactPath
    artifact_sha256 = $actualSha
    event_count = $snapshot.snapshot.events.Count
    control_acknowledged = [bool]$controlOutput
}

$report | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $reportPath
$report | ConvertTo-Json -Depth 10
