# Synthetic fault injection only: hold one QA file unreadable, without changing its bytes.
#requires -Version 7.4
param(
    [Parameter(Mandatory)][string]$QaRoot,
    [Parameter(Mandatory)][string]$Workspace,
    [Parameter(Mandatory)][string]$ReleaseSignal
)
$ErrorActionPreference = 'Stop'
$qaPath = (Resolve-Path -LiteralPath $QaRoot).Path
$workspacePath = (Resolve-Path -LiteralPath $Workspace).Path
if ([IO.Path]::GetFileName($qaPath) -cne 'issue-50-factory-recovery' -or
    [IO.Path]::GetFileName([IO.Path]::GetDirectoryName($qaPath)) -cne 'qa') {
    throw 'Not the explicitly owned QA root.'
}
$prefix = $qaPath.TrimEnd('\') + '\attempts\'
if (!$workspacePath.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase) -or
    !$workspacePath.Contains('\runner-workspaces\worktrees\') -or
    !([IO.Path]::GetFullPath($ReleaseSignal)).StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Lock and release signal must stay within the QA attempt.'
}
$walkPath = $workspacePath
while ($walkPath.Length -ge $qaPath.Length) {
    $item = Get-Item -LiteralPath $walkPath -Force
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Refuse redirected QA path.' }
    if ($walkPath.Equals($qaPath, [StringComparison]::OrdinalIgnoreCase)) { break }
    $walkPath = [IO.Path]::GetDirectoryName($walkPath)
}
# An unchanged tracked file is outside the Codex adapter's changed-file evidence
# scan but remains inside the native full-workspace checkpoint fingerprint.
$file = Join-Path $workspacePath 'README.md'
$item = Get-Item -LiteralPath $file -Force
if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Refuse redirected QA file.' }
$handle = [IO.File]::Open($file, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::None)
try {
    [IO.File]::WriteAllText((Join-Path $workspacePath '.qa-checkpoint-lock-ready'), 'Synthetic QA lock ready')
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while (!(Test-Path -LiteralPath $ReleaseSignal) -and [DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 100
    }
} finally {
    $handle.Dispose()
}
