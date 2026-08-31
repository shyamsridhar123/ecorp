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
    $snapshot = Invoke-RestMethod -Uri "$Server/api/corps/$($demo.corp_id)/snapshot?actor_id=$($demo.alice_actor_id)"
    $run = $snapshot.snapshot.runs | Where-Object id -eq $launch.run_id
    if (
        $run.status -in @('completed', 'failed', 'cancelled') -and
        $run.workspace_disposition -in @('preserved', 'removed')
    ) {
        break
    }
    Start-Sleep -Milliseconds 500
} while ((Get-Date) -lt $deadline)

if ($run.status -ne 'completed') {
    throw "Run did not complete successfully: $($run | ConvertTo-Json -Depth 10)"
}
if (-not $run.workspace_path -or -not $run.workspace_branch) {
    throw 'Run did not persist isolated worktree metadata.'
}
if ($run.workspace_disposition -ne 'preserved') {
    throw "Expected dirty worktree preservation, got $($run.workspace_disposition)."
}

if ($run.PSObject.Properties.Name -contains 'artifact_path') {
    throw 'Shared run state exposed a runner-local artifact path.'
}
if (
    -not $run.artifact_id -or
    -not $run.artifact_uri -or
    -not $run.artifact_media_type -or
    -not $run.artifact_signature
) {
    throw 'Run omitted durable artifact metadata.'
}
$downloadPath = Join-Path $root "output\artifact-$($run.artifact_id).bin"
$artifactResponse = Invoke-WebRequest `
    -Uri "$Server$($run.artifact_uri)?actor_id=$($demo.alice_actor_id)" `
    -OutFile $downloadPath `
    -PassThru
if ($artifactResponse.StatusCode -ne 200) {
    throw "Artifact download failed with HTTP $($artifactResponse.StatusCode)."
}
if ($artifactResponse.Headers.'Content-Type' -ne $run.artifact_media_type) {
    throw 'Artifact download media type did not match the signed record.'
}
if ($artifactResponse.Headers.'X-Crony-Artifact-Signature' -ne $run.artifact_signature) {
    throw 'Artifact download provenance signature did not match the run.'
}
$actualSha = (Get-FileHash -LiteralPath $downloadPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualSha -ne $run.artifact_sha256) {
    throw "Artifact hash mismatch: expected $($run.artifact_sha256), got $actualSha"
}
Remove-Item -LiteralPath $downloadPath -Force

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

$adapterCapability = $snapshot.runners |
    Where-Object connected |
    ForEach-Object capabilities |
    Where-Object name -eq 'fake-process' |
    Select-Object -First 1
if (
    -not $adapterCapability -or
    -not $adapterCapability.available -or
    $adapterCapability.detail -notmatch 'spawn=yes' -or
    $adapterCapability.detail -notmatch 'resume=no'
) {
    throw 'Runner did not report the explicit fake-process adapter capability contract.'
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
    artifact_id = $run.artifact_id
    artifact_uri = $run.artifact_uri
    artifact_media_type = $run.artifact_media_type
    artifact_signature = $run.artifact_signature
    artifact_sha256 = $actualSha
    event_count = $snapshot.snapshot.events.Count
    control_acknowledged = [bool]$controlOutput
    adapter_capability = $adapterCapability.detail
    workspace_path = $run.workspace_path
    workspace_branch = $run.workspace_branch
    workspace_disposition = $run.workspace_disposition
}

$report | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $reportPath
$report | ConvertTo-Json -Depth 10
