#requires -Version 7.4
Set-StrictMode -Version Latest

function Get-LocalFullPath {
    param([Parameter(Mandatory)][string]$Path)
    [IO.Path]::GetFullPath($Path).TrimEnd([IO.Path]::DirectorySeparatorChar)
}

function Test-LocalPathEqual {
    param([string]$Left, [string]$Right)
    if (!$Left -or !$Right) { return $false }
    try {
        return [string]::Equals((Get-LocalFullPath $Left), (Get-LocalFullPath $Right),
            [StringComparison]::OrdinalIgnoreCase)
    } catch { return $false }
}

function ConvertTo-LocalProcessArgument {
    param([AllowEmptyString()][Parameter(Mandatory)][string]$Value)
    # Start-Process joins ArgumentList into a Windows command line. Preserve one
    # literal argument, including spaces, quotes and trailing backslashes.
    '"' + [regex]::Replace(
        [regex]::Replace($Value, '(\\*)"', '$1$1\"'), '(\\+)$', '$1$1') + '"'
}

function Get-LocalProcessIdentity {
    param([Parameter(Mandatory)][int]$ProcessId)
    if ($ProcessId -le 0) { return $null }
    $process = $null
    try {
        $process = Get-Process -Id $ProcessId -ErrorAction Stop
        [void]$process.Handle
        if ($process.HasExited) { return $null }
        @{
            pid = $process.Id
            executable = $process.Path
            started_utc = $process.StartTime.ToUniversalTime().ToString('o')
        }
    } catch { return $null }
    finally { if ($process) { $process.Dispose() } }
}

function Test-LocalRecordShape {
    param([hashtable]$Record, [string]$Workspace)
    if (!$Record -or !$Record.ContainsKey('pid') -or !$Record.ContainsKey('executable') -or
        !$Record.ContainsKey('started_utc') -or !$Record.ContainsKey('workspace') -or
        !(Test-LocalPathEqual $Record.workspace $Workspace)) { return $false }
    $number = 0
    $time = [DateTimeOffset]::MinValue
    [int]::TryParse([string]$Record.pid, [ref]$number) -and $number -gt 0 -and
        [IO.Path]::IsPathFullyQualified([string]$Record.executable) -and
        [DateTimeOffset]::TryParse([string]$Record.started_utc, [ref]$time)
}

function Test-LocalOwnedProcess {
    param([hashtable]$Record, [Parameter(Mandatory)][string]$Workspace)
    if (!(Test-LocalRecordShape $Record $Workspace)) { return $false }
    $current = Get-LocalProcessIdentity -ProcessId ([int]$Record.pid)
    if (!$current) { return $false }
    (Test-LocalPathEqual $current.executable $Record.executable) -and
        ([DateTimeOffset]$current.started_utc).UtcTicks -eq
        ([DateTimeOffset]$Record.started_utc).UtcTicks
}

function Stop-LocalOwnedProcess {
    param([hashtable]$Record, [Parameter(Mandatory)][string]$Workspace)
    if (!(Test-LocalRecordShape $Record $Workspace)) { return $false }
    $process = $null
    try {
        $process = Get-Process -Id ([int]$Record.pid) -ErrorAction Stop
        # Keep the process handle open across verification and termination. Do
        # not look up a PID again, or infer ownership of its current descendants.
        [void]$process.Handle
        if ($process.HasExited -or !(Test-LocalPathEqual $process.Path $Record.executable) -or
            $process.StartTime.ToUniversalTime().Ticks -ne
            ([DateTimeOffset]$Record.started_utc).UtcTicks) { return $false }
        $process.Kill()
        if (!$process.WaitForExit(15000)) {
            throw 'The verified process has not exited; its ownership record is retained.'
        }
        return $true
    } catch [Microsoft.PowerShell.Commands.ProcessCommandException] {
        return $false
    } catch [System.ArgumentException] {
        return $false
    } finally {
        if ($process) { $process.Dispose() }
    }
}

function New-LocalProcessEnvironment {
    param([hashtable]$Environment = @{})
    # Native Start-Process -Environment removes keys with null values. Do not
    # inherit unrelated cloud/provider secrets into the runner or web client.
    $result = @{}
    foreach ($name in [Environment]::GetEnvironmentVariables('Process').Keys) {
        $result[[string]$name] = $null
    }
    $allow = @('PATH', 'PATHEXT', 'SystemRoot', 'windir', 'ComSpec', 'TEMP', 'TMP',
        'USERPROFILE', 'HOMEDRIVE', 'HOMEPATH', 'HOME', 'APPDATA', 'LOCALAPPDATA',
        'PROGRAMDATA', 'PROGRAMFILES', 'PROGRAMFILES(X86)', 'PROGRAMW6432',
        'SYSTEMDRIVE', 'USERNAME', 'USERDOMAIN', 'COMPUTERNAME', 'PSModulePath',
        'NUMBER_OF_PROCESSORS', 'PROCESSOR_ARCHITECTURE', 'OS')
    foreach ($name in $allow) {
        $value = [Environment]::GetEnvironmentVariable($name, 'Process')
        if ($null -ne $value) { $result[$name] = $value }
    }
    foreach ($name in $Environment.Keys) { $result[[string]$name] = $Environment[$name] }
    $result
}

function Initialize-LocalLiteralLauncher {
    if ('ECorp.LocalLiteralLauncher' -as [type]) { return }
    # Start-Process resolves stdout/stderr twice when both are redirected. Its
    # second wildcard pass cannot preserve a resolved literal bracket path.
    # This narrow Windows fallback inherits file handles (not supervisor-owned
    # pipes), so children and their logs survive the launching PowerShell.
    Add-Type -TypeDefinition @'
using System;
using System.Collections;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;

namespace ECorp {
    public static class LocalLiteralLauncher {
        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        private struct StartupInfo {
            public int cb;
            public string reserved, desktop, title;
            public int x, y, xSize, ySize, xChars, yChars, fillAttribute, flags;
            public short showWindow, reservedCount;
            public IntPtr reservedBytes, stdin, stdout, stderr;
        }
        [StructLayout(LayoutKind.Sequential)]
        private struct StartupInfoEx {
            public StartupInfo startup;
            public IntPtr attributes;
        }
        [StructLayout(LayoutKind.Sequential)]
        private struct ProcessInfo {
            public IntPtr process, thread;
            public int processId, threadId;
        }
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool CreateProcessW(string application, StringBuilder command,
            IntPtr processAttributes, IntPtr threadAttributes, bool inheritHandles, uint flags,
            IntPtr environment, string directory, ref StartupInfoEx startup, out ProcessInfo process);
        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool InitializeProcThreadAttributeList(IntPtr list, int count,
            int flags, ref IntPtr size);
        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool UpdateProcThreadAttribute(IntPtr list, uint flags,
            IntPtr attribute, IntPtr value, IntPtr size, IntPtr previous, IntPtr returned);
        [DllImport("kernel32.dll")]
        private static extern void DeleteProcThreadAttributeList(IntPtr list);
        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern uint ResumeThread(IntPtr thread);
        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool TerminateProcess(IntPtr process, uint exitCode);
        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);
        [DllImport("kernel32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool CloseHandle(IntPtr handle);

        private static bool RecordRollbackResult(Exception failure, bool terminated,
            int terminationError, uint waitResult, int waitError) {
            bool verified = terminated && waitResult == 0; // WAIT_OBJECT_0
            failure.Data["LocalStackRollbackVerified"] = verified;
            failure.Data["LocalStackRollbackTerminateSucceeded"] = terminated;
            failure.Data["LocalStackRollbackTerminationError"] = terminationError;
            failure.Data["LocalStackRollbackWaitResult"] = waitResult;
            failure.Data["LocalStackRollbackWaitError"] = waitError;
            return verified;
        }

        public static Process Start(string executable, string command, string directory,
            string stdout, string stderr, IDictionary environment) {
            if (!OperatingSystem.IsWindows()) throw new PlatformNotSupportedException();
            var values = new SortedDictionary<string, string>(StringComparer.OrdinalIgnoreCase);
            foreach (DictionaryEntry entry in environment) {
                if (entry.Value == null) continue;
                var name = Convert.ToString(entry.Key, CultureInfo.InvariantCulture);
                var value = Convert.ToString(entry.Value, CultureInfo.InvariantCulture);
                if (String.IsNullOrEmpty(name) || name.Contains('=') || name.Contains('\0') ||
                    value.Contains('\0')) throw new ArgumentException("Invalid child environment entry.");
                values[name] = value;
            }
            var block = new StringBuilder();
            foreach (var entry in values) block.Append(entry.Key).Append('=').Append(entry.Value).Append('\0');
            block.Append('\0');
            if (values.Count == 0) block.Append('\0');
            // Only these three file handles may be inherited by the new root.
            using var input = File.OpenHandle("NUL", FileMode.Open, FileAccess.Read,
                FileShare.ReadWrite | FileShare.Inheritable);
            using var output = File.OpenHandle(stdout, FileMode.Open, FileAccess.Write,
                FileShare.ReadWrite | FileShare.Inheritable);
            using var error = File.OpenHandle(stderr, FileMode.Open, FileAccess.Write,
                FileShare.ReadWrite | FileShare.Inheritable);
            IntPtr attributes = IntPtr.Zero, handles = IntPtr.Zero, env = IntPtr.Zero;
            bool initialized = false, resumed = false;
            ProcessInfo native = default;
            Process owned = null;
            Exception startupFailure = null;
            try {
                IntPtr size = IntPtr.Zero;
                InitializeProcThreadAttributeList(IntPtr.Zero, 1, 0, ref size);
                attributes = Marshal.AllocHGlobal(size);
                if (!InitializeProcThreadAttributeList(attributes, 1, 0, ref size))
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                initialized = true;
                handles = Marshal.AllocHGlobal(IntPtr.Size * 3);
                Marshal.WriteIntPtr(handles, 0, input.DangerousGetHandle());
                Marshal.WriteIntPtr(handles, IntPtr.Size, output.DangerousGetHandle());
                Marshal.WriteIntPtr(handles, IntPtr.Size * 2, error.DangerousGetHandle());
                if (!UpdateProcThreadAttribute(attributes, 0, new IntPtr(0x20002), handles,
                    new IntPtr(IntPtr.Size * 3), IntPtr.Zero, IntPtr.Zero))
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                var startup = new StartupInfoEx {
                    startup = new StartupInfo {
                        cb = Marshal.SizeOf<StartupInfoEx>(), flags = 0x101, showWindow = 0,
                        stdin = input.DangerousGetHandle(), stdout = output.DangerousGetHandle(),
                        stderr = error.DangerousGetHandle()
                    },
                    attributes = attributes
                };
                env = Marshal.StringToHGlobalUni(block.ToString());
                // Suspended until the exact process handle and creation time
                // are retained; no user code can run during ownership setup.
                const uint creationFlags = 0x08000000 | 0x00080000 | 0x00000400 | 0x00000004;
                if (!CreateProcessW(executable, new StringBuilder(command), IntPtr.Zero,
                    IntPtr.Zero, true, creationFlags, env, directory, ref startup, out native))
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                owned = Process.GetProcessById(native.processId);
                _ = owned.Handle;
                _ = owned.StartTime;
                if (ResumeThread(native.thread) == UInt32.MaxValue)
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                resumed = true;
                return owned;
            } catch (Exception failure) {
                startupFailure = failure;
                throw; // Preserve the original error, inner cause and stack.
            } finally {
                if (!resumed && native.process != IntPtr.Zero) {
                    // Roll back only this still-suspended root, never a PID lookup.
                    bool terminated = TerminateProcess(native.process, 1);
                    int terminationError = terminated ? 0 : Marshal.GetLastWin32Error();
                    uint waitResult = WaitForSingleObject(native.process, 15000);
                    int waitError = waitResult == UInt32.MaxValue ? Marshal.GetLastWin32Error() : 0;
                    if (!RecordRollbackResult(startupFailure, terminated, terminationError, waitResult, waitError)) {
                        // The caller retains the exact handle with the original
                        // exception for explicit resolution. Do not discard the
                        // last handle or claim cleanup after an unverified rollback.
                        startupFailure.Data["LocalStackRollbackProcessId"] = native.processId;
                        startupFailure.Data["LocalStackRollbackProcessHandle"] =
                            new Microsoft.Win32.SafeHandles.SafeProcessHandle(native.process, true);
                        native.process = IntPtr.Zero;
                    }
                    owned?.Dispose();
                }
                if (native.thread != IntPtr.Zero) CloseHandle(native.thread);
                if (native.process != IntPtr.Zero) CloseHandle(native.process);
                if (initialized) DeleteProcThreadAttributeList(attributes);
                if (attributes != IntPtr.Zero) Marshal.FreeHGlobal(attributes);
                if (handles != IntPtr.Zero) Marshal.FreeHGlobal(handles);
                if (env != IntPtr.Zero) Marshal.FreeHGlobal(env);
            }
        }
    }
}
'@
}

function Start-LocalOwnedProcess {
    param(
        [Parameter(Mandatory)][ValidatePattern('^[a-zA-Z][a-zA-Z0-9-]*$')][string]$Role,
        [Parameter(Mandatory)][string]$Workspace,
        [Parameter(Mandatory)][string]$FilePath,
        [string[]]$ArgumentList = @(),
        [Parameter(Mandatory)][string]$WorkingDirectory,
        [Parameter(Mandatory)][string]$LogDirectory,
        [hashtable]$Environment = @{}
    )
    $exe = (Resolve-Path -LiteralPath $FilePath -ErrorAction Stop).Path
    $cwd = (Resolve-Path -LiteralPath $WorkingDirectory -ErrorAction Stop).Path
    $scope = (Resolve-Path -LiteralPath $Workspace -ErrorAction Stop).Path
    $logRoot = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($LogDirectory)
    [IO.Directory]::CreateDirectory($logRoot) | Out-Null
    $stamp = [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfff') + '-' + [guid]::NewGuid().ToString('N')
    $stdout = Join-Path $logRoot "$Role-$stamp.stdout.log"
    $stderr = Join-Path $logRoot "$Role-$stamp.stderr.log"
    foreach ($log in @($stdout, $stderr)) {
        # Reserve unique literal files without truncating earlier evidence.
        $stream = [IO.File]::Open($log, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write,
            [IO.FileShare]::ReadWrite)
        $stream.Dispose()
    }
    $parameters = @{
        FilePath = $exe; WorkingDirectory = $cwd; PassThru = $true; WindowStyle = 'Hidden'
        RedirectStandardOutput = $stdout; RedirectStandardError = $stderr
        Environment = (New-LocalProcessEnvironment -Environment $Environment)
    }
    if ($ArgumentList.Count) {
        $parameters.ArgumentList = @($ArgumentList | ForEach-Object { ConvertTo-LocalProcessArgument $_ })
    }
    $literalOnly = @($exe, $cwd, $stdout, $stderr) | Where-Object {
        $_.Contains('[') -or $_.Contains(']') -or $_.Contains('`')
    }
    if ($literalOnly) {
        Initialize-LocalLiteralLauncher
        $command = ConvertTo-LocalProcessArgument $exe
        if ($ArgumentList.Count) { $command += ' ' + ($parameters.ArgumentList -join ' ') }
        try {
            $process = [ECorp.LocalLiteralLauncher]::Start(
                $exe, $command, $cwd, $stdout, $stderr, $parameters.Environment)
        } catch {
            $failure = $_.Exception
            while ($failure -and !$failure.Data.Contains('LocalStackRollbackVerified')) {
                $failure = $failure.InnerException
            }
            if ($failure -and !$failure.Data['LocalStackRollbackVerified']) {
                $message = 'Failed-start rollback is unverified for owned process {0}. ' +
                    'The original exception retains native results and LocalStackRollbackProcessHandle; explicit resolution is required.'
                Write-Warning -WarningAction Continue ($message -f $failure.Data['LocalStackRollbackProcessId'])
            }
            throw
        }
    } else {
        $process = Start-Process @parameters
    }
    try {
        @{
            role = $Role; workspace = $scope; pid = $process.Id; executable = $exe
            started_utc = $process.StartTime.ToUniversalTime().ToString('o')
            stdout = $stdout; stderr = $stderr
        }
    } finally { $process.Dispose() }
}

function Read-LocalStackState {
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][string]$Workspace)
    if (!(Test-Path -LiteralPath $Path -PathType Leaf)) { return $null }
    try { $state = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json -AsHashtable -ErrorAction Stop }
    catch { throw 'The local ownership record cannot be parsed. It was not removed or used for process control.' }
    if ($state -isnot [hashtable]) { throw 'Invalid local ownership record.' }
    if (!$state.ContainsKey('schema_version')) {
        # Old numeric-only PIDs are historical data, never stop authority.
        $state.schema_version = 1
        return $state
    }
    if ($state.schema_version -ne 2 -or !$state.ContainsKey('workspace') -or
        !(Test-LocalPathEqual $state.workspace $Workspace) -or
        !$state.ContainsKey('processes') -or $state.processes -isnot [hashtable]) {
        throw 'Local ownership record scope/version mismatch. No process was controlled.'
    }
    $state
}

function Save-LocalStackState {
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][hashtable]$State,
        [Parameter(Mandatory)][string]$Workspace)
    $root = (Resolve-Path -LiteralPath $Workspace -ErrorAction Stop).Path.TrimEnd('\', '/')
    $target = [IO.Path]::GetFullPath($Path)
    if (!$target.StartsWith($root + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'The ownership record must stay inside its explicit workspace.'
    }
    if ($State.ContainsKey('workspace') -and !(Test-LocalPathEqual $State.workspace $root)) {
        throw 'Refusing to overwrite a differently scoped ownership record.'
    }
    # Reading a legacy file is diagnostic only. Neither a returned v1 record
    # nor raw PID-only fields may be relabeled as v2 stop authority.
    if (($State.ContainsKey('schema_version') -and $State.schema_version -ne 2) -or
        !$State.ContainsKey('processes') -or $State.processes -isnot [hashtable]) {
        throw 'Only v2 process-record state may be saved. Legacy PID-only state is read-only.'
    }
    foreach ($record in $State.processes.Values) {
        if ($null -ne $record -and $record -isnot [hashtable]) {
            throw 'Numeric PIDs cannot be promoted to v2 process ownership records.'
        }
    }
    $parent = Split-Path -Parent $target
    $cursor = $parent
    while ($cursor -and $cursor.Length -ge $root.Length) {
        if (Test-Path -LiteralPath $cursor) {
            if ((Get-Item -LiteralPath $cursor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) {
                throw 'Ownership-record directories must not redirect outside the workspace.'
            }
        }
        if (Test-LocalPathEqual $cursor $root) { break }
        $cursor = Split-Path -Parent $cursor
    }
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
    $State.schema_version = 2
    $State.workspace = $root
    $State.updated_at = [DateTime]::UtcNow.ToString('o')
    $temporary = $target + '.' + [guid]::NewGuid().ToString('N') + '.tmp'
    try {
        [IO.File]::WriteAllText($temporary, ($State | ConvertTo-Json -Depth 12), [Text.UTF8Encoding]::new($false))
        [IO.File]::Move($temporary, $target, $true)
    } finally {
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
    }
}

Export-ModuleMember -Function Get-LocalFullPath, Test-LocalPathEqual,
    ConvertTo-LocalProcessArgument, Get-LocalProcessIdentity, Test-LocalOwnedProcess,
    Stop-LocalOwnedProcess, New-LocalProcessEnvironment, Start-LocalOwnedProcess,
    Read-LocalStackState, Save-LocalStackState
