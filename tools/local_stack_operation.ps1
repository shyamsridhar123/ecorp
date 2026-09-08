#requires -Version 7.4

function Invoke-LocalStackOperation {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$Workspace,
        [Parameter(Mandatory)][scriptblock]$Action,
        [ValidateRange(0,600)][int]$WaitSeconds = 300
    )
    if (!$IsWindows) { throw 'The local Windows launcher requires Windows.' }
    $root = (Resolve-Path -LiteralPath $Workspace -ErrorAction Stop).Path.TrimEnd('\', '/')
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    try { $sid = $identity.User.Value } finally { $identity.Dispose() }
    $key = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData(
        [Text.Encoding]::UTF8.GetBytes($sid + '|' + $root.ToUpperInvariant())))
    # Native, process-independent ownership: no stale lock file after reboot.
    # The same-thread reentrancy also covers start -Restart -> stop.
    $mutex = [Threading.Mutex]::new($false, "Local\ECorp.LocalStack.$key")
    $acquired = $false
    try {
        try { $acquired = $mutex.WaitOne([TimeSpan]::FromSeconds($WaitSeconds)) }
        catch [Threading.AbandonedMutexException] {
            $acquired = $true
            # The prior command exited. Normal ownership/state checks still run;
            # abandonment is not permission to reset or adopt a process.
        }
        if (!$acquired) {
            throw 'Another ECorp start/stop command is still running for this checkout. Existing services were left unchanged.'
        }
        & $Action
    } finally {
        if ($acquired) { $mutex.ReleaseMutex() }
        $mutex.Dispose()
    }
}
