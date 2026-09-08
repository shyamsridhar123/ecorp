#requires -Version 7.4
[CmdletBinding()]
param(
    [switch]$SkipInstall,
    [switch]$SkipBuild,
    [switch]$SkipFactoryController,
    [switch]$Restart,
    [ValidateRange(0,65535)][int]$ServerPort = 0,
    [ValidateRange(0,65535)][int]$WebPort = 0
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
. (Join-Path $PSScriptRoot 'local_stack_operation.ps1')
Invoke-LocalStackOperation -Workspace $root -Action {
    if ($Restart) {
        & (Join-Path $PSScriptRoot 'stop_local.ps1')
    }
    & (Join-Path $PSScriptRoot 'local_stack_start.ps1') -Workspace $root `
        -SkipInstall:$SkipInstall -SkipBuild:$SkipBuild -SkipFactoryController:$SkipFactoryController `
        -ServerPort $ServerPort -WebPort $WebPort
}
