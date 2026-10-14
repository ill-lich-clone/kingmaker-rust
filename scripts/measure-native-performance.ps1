[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Executable,

    [Parameter(Mandatory = $true)]
    [string]$ArgumentFile,

    [Parameter(Mandatory = $true)]
    [string]$OutputDirectory,

    [int64]$PrivateLimitMiB = 4096,

    [int]$TimeoutSeconds = 600
)

$ErrorActionPreference = 'Stop'
if ($PrivateLimitMiB -le 0 -or $PrivateLimitMiB -gt 1048576 -or $TimeoutSeconds -le 0) {
    throw 'Memory limit and timeout must be positive; memory limit must not exceed 1 TiB'
}
if (Test-Path -LiteralPath $OutputDirectory) { throw 'OutputDirectory must be a new directory' }

if (-not (Test-Path -LiteralPath $Executable -PathType Leaf)) {
    Write-Error "Executable not found: $Executable"
    exit 1
}
$resolvedExecutable = (Resolve-Path -LiteralPath $Executable).Path

if (-not (Test-Path -LiteralPath $ArgumentFile -PathType Leaf)) {
    Write-Error "ArgumentFile not found: $ArgumentFile"
    exit 1
}

try {
    $rawJson = Get-Content -LiteralPath $ArgumentFile -Raw
    $parsedArgs = $rawJson | ConvertFrom-Json -NoEnumerate
} catch {
    Write-Error "Failed to parse JSON ArgumentFile: $_"
    exit 1
}

if ($parsedArgs -isnot [System.Collections.IList] -and $parsedArgs -isnot [Array]) {
    Write-Error "ArgumentFile content must be a JSON array."
    exit 1
}

foreach ($a in $parsedArgs) {
    if ($a -isnot [string] -or $a.Contains([char]0)) { throw 'Arguments must be strings without NUL' }
}
$argList = [string[]]$parsedArgs

if (-not (Test-Path -LiteralPath $OutputDirectory)) {
    try {
        $null = New-Item -ItemType Directory -Path $OutputDirectory -Force
    } catch {
        Write-Error "Failed to create OutputDirectory: $_"
        exit 1
    }
}
$resolvedOutputDir = (Resolve-Path -LiteralPath $OutputDirectory).Path
$stdoutFile = [System.IO.Path]::Combine($resolvedOutputDir, "stdout.log")
$stderrFile = [System.IO.Path]::Combine($resolvedOutputDir, "stderr.log")
$processJsonFile = [System.IO.Path]::Combine($resolvedOutputDir, "process.json")

if (-not ('NativeProcessRunner' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;

public static class NativeProcessRunner
{
    private const uint CREATE_SUSPENDED = 0x00000004;
    private const uint CREATE_NO_WINDOW = 0x08000000;
    private const uint STARTF_USESTDHANDLES = 0x00000100;

    private const uint GENERIC_WRITE = 0x40000000;
    private const uint FILE_SHARE_READ = 0x00000001;
    private const uint FILE_SHARE_WRITE = 0x00000002;
    private const uint CREATE_ALWAYS = 2;
    private const uint FILE_ATTRIBUTE_NORMAL = 0x00000080;

    private const int JobObjectAssociateCompletionPortInformation = 7;
    private const int JobObjectExtendedLimitInformation = 9;

    private const uint JOB_OBJECT_LIMIT_PROCESS_MEMORY = 0x00000100;
    private const uint JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x00002000;
    private const uint JOB_OBJECT_MSG_PROCESS_MEMORY_LIMIT = 9;

    private const uint WAIT_OBJECT_0 = 0x00000000;
    private static readonly IntPtr INVALID_HANDLE_VALUE = new IntPtr(-1);

    [StructLayout(LayoutKind.Sequential)]
    public struct IO_COUNTERS
    {
        public ulong ReadOperationCount;
        public ulong WriteOperationCount;
        public ulong OtherOperationCount;
        public ulong ReadTransferCount;
        public ulong WriteTransferCount;
        public ulong OtherTransferCount;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct JOBOBJECT_BASIC_LIMIT_INFORMATION
    {
        public long PerProcessUserTimeLimit;
        public long PerJobUserTimeLimit;
        public uint LimitFlags;
        public UIntPtr MinimumWorkingSetSize;
        public UIntPtr MaximumWorkingSetSize;
        public uint ActiveProcessLimit;
        public UIntPtr Affinity;
        public uint PriorityClass;
        public uint SchedulingClass;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION
    {
        public JOBOBJECT_BASIC_LIMIT_INFORMATION BasicLimitInformation;
        public IO_COUNTERS IoInfo;
        public UIntPtr ProcessMemoryLimit;
        public UIntPtr JobMemoryLimit;
        public UIntPtr PeakProcessMemoryUsed;
        public UIntPtr PeakJobMemoryUsed;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct JOBOBJECT_ASSOCIATE_COMPLETION_PORT
    {
        public IntPtr CompletionKey;
        public IntPtr CompletionPort;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct SECURITY_ATTRIBUTES
    {
        public int nLength;
        public IntPtr lpSecurityDescriptor;
        [MarshalAs(UnmanagedType.Bool)]
        public bool bInheritHandle;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct STARTUPINFO
    {
        public int cb;
        public string lpReserved;
        public string lpDesktop;
        public string lpTitle;
        public int dwX;
        public int dwY;
        public int dwXSize;
        public int dwYSize;
        public int dwXCountChars;
        public int dwYCountChars;
        public int dwFillAttribute;
        public uint dwFlags;
        public short wShowWindow;
        public short cbReserved2;
        public IntPtr lpReserved2;
        public IntPtr hStdInput;
        public IntPtr hStdOutput;
        public IntPtr hStdError;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct PROCESS_INFORMATION
    {
        public IntPtr hProcess;
        public IntPtr hThread;
        public int dwProcessId;
        public int dwThreadId;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct PROCESS_MEMORY_COUNTERS_EX
    {
        public uint cb;
        public uint PageFaultCount;
        public UIntPtr PeakWorkingSetSize;
        public UIntPtr WorkingSetSize;
        public UIntPtr QuotaPeakPagedPoolUsage;
        public UIntPtr QuotaPagedPoolUsage;
        public UIntPtr QuotaPeakNonPagedPoolUsage;
        public UIntPtr QuotaNonPagedPoolUsage;
        public UIntPtr PagefileUsage;
        public UIntPtr PeakPagefileUsage;
        public UIntPtr PrivateUsage;
    }

    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    private static extern IntPtr CreateJobObjectW(IntPtr lpJobAttributes, string lpName);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetInformationJobObject(IntPtr hJob, int JobObjectInformationClass, ref JOBOBJECT_EXTENDED_LIMIT_INFORMATION lpJobObjectInformation, int cbJobObjectInformationLength);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetInformationJobObject(IntPtr hJob, int JobObjectInformationClass, ref JOBOBJECT_ASSOCIATE_COMPLETION_PORT lpJobObjectInformation, int cbJobObjectInformationLength);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool QueryInformationJobObject(IntPtr hJob, int JobObjectInformationClass, ref JOBOBJECT_EXTENDED_LIMIT_INFORMATION lpJobObjectInformation, int cbJobObjectInformationLength, IntPtr lpReturnLength);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern IntPtr CreateIoCompletionPort(IntPtr FileHandle, IntPtr ExistingCompletionPort, UIntPtr CompletionKey, uint NumberOfConcurrentThreads);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetQueuedCompletionStatus(IntPtr CompletionPort, out uint lpNumberOfBytesTransferred, out UIntPtr lpCompletionKey, out IntPtr lpOverlapped, uint dwMilliseconds);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool AssignProcessToJobObject(IntPtr hJob, IntPtr hProcess);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool TerminateJobObject(IntPtr hJob, uint uExitCode);

    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    private static extern IntPtr CreateFileW(string lpFileName, uint dwDesiredAccess, uint dwShareMode, ref SECURITY_ATTRIBUTES lpSecurityAttributes, uint dwCreationDisposition, uint dwFlagsAndAttributes, IntPtr hTemplateFile);

    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CreateProcessW(string lpApplicationName, StringBuilder lpCommandLine, IntPtr lpProcessAttributes, IntPtr lpThreadAttributes, [MarshalAs(UnmanagedType.Bool)] bool bInheritHandles, uint dwCreationFlags, IntPtr lpEnvironment, string lpCurrentDirectory, ref STARTUPINFO lpStartupInfo, out PROCESS_INFORMATION lpProcessInformation);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern uint ResumeThread(IntPtr hThread);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool TerminateProcess(IntPtr hProcess, uint uExitCode);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern uint WaitForSingleObject(IntPtr hHandle, uint dwMilliseconds);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetExitCodeProcess(IntPtr hProcess, out uint lpExitCode);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CloseHandle(IntPtr hObject);

    [DllImport("psapi.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetProcessMemoryInfo(IntPtr hProcess, out PROCESS_MEMORY_COUNTERS_EX counters, uint size);

    public class RunResult
    {
        public string Outcome;
        public int? ExitCode;
        public int Pid;
        public long PeakPrivateBytes;
        public long PeakWorkingSetBytes;
        public int SamplingIntervalMs;
        public bool MemoryLimitNotified;
    }

    private static string EscapeArg(string arg)
    {
        if (string.IsNullOrEmpty(arg)) return "\"\"";
        if (arg.IndexOfAny(new char[] { ' ', '\t', '\n', '\v', '\"' }) == -1) return arg;

        var sb = new StringBuilder();
        sb.Append('"');
        for (int i = 0; i < arg.Length; i++)
        {
            int backslashes = 0;
            while (i < arg.Length && arg[i] == '\\')
            {
                backslashes++;
                i++;
            }
            if (i == arg.Length)
            {
                sb.Append('\\', backslashes * 2);
                break;
            }
            if (arg[i] == '"')
            {
                sb.Append('\\', backslashes * 2 + 1);
                sb.Append('"');
            }
            else
            {
                sb.Append('\\', backslashes);
                sb.Append(arg[i]);
            }
        }
        sb.Append('"');
        return sb.ToString();
    }

    public static RunResult Run(string exePath, string[] args, string stdoutPath, string stderrPath, long privateLimitMiB, int timeoutSeconds)
    {
        IntPtr hJob = IntPtr.Zero;
        IntPtr hPort = IntPtr.Zero;
        IntPtr hStdOut = INVALID_HANDLE_VALUE;
        IntPtr hStdErr = INVALID_HANDLE_VALUE;
        PROCESS_INFORMATION pi = default;

        try
        {
            hJob = CreateJobObjectW(IntPtr.Zero, null);
            if (hJob == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error(), "CreateJobObjectW failed");

            hPort = CreateIoCompletionPort(INVALID_HANDLE_VALUE, IntPtr.Zero, UIntPtr.Zero, 1);
            if (hPort == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error(), "CreateIoCompletionPort failed");

            var assoc = new JOBOBJECT_ASSOCIATE_COMPLETION_PORT
            {
                CompletionKey = new IntPtr(1),
                CompletionPort = hPort
            };
            if (!SetInformationJobObject(hJob, JobObjectAssociateCompletionPortInformation, ref assoc, Marshal.SizeOf(assoc)))
                throw new Win32Exception(Marshal.GetLastWin32Error(), "Set JobObjectAssociateCompletionPortInformation failed");

            var extLimits = new JOBOBJECT_EXTENDED_LIMIT_INFORMATION();
            extLimits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_PROCESS_MEMORY | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            ulong limitBytes = (ulong)privateLimitMiB * 1024UL * 1024UL;
            extLimits.ProcessMemoryLimit = new UIntPtr(limitBytes);

            if (!SetInformationJobObject(hJob, JobObjectExtendedLimitInformation, ref extLimits, Marshal.SizeOf(extLimits)))
                throw new Win32Exception(Marshal.GetLastWin32Error(), "Set JobObjectExtendedLimitInformation failed");

            var sa = new SECURITY_ATTRIBUTES();
            sa.nLength = Marshal.SizeOf(sa);
            sa.bInheritHandle = true;

            hStdOut = CreateFileW(stdoutPath, GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, ref sa, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, IntPtr.Zero);
            if (hStdOut == INVALID_HANDLE_VALUE) throw new Win32Exception(Marshal.GetLastWin32Error(), "CreateFileW for stdout failed");

            hStdErr = CreateFileW(stderrPath, GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, ref sa, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, IntPtr.Zero);
            if (hStdErr == INVALID_HANDLE_VALUE) throw new Win32Exception(Marshal.GetLastWin32Error(), "CreateFileW for stderr failed");

            var cmdLine = new StringBuilder();
            cmdLine.Append(EscapeArg(exePath));
            if (args != null)
            {
                foreach (var arg in args)
                {
                    cmdLine.Append(' ');
                    cmdLine.Append(EscapeArg(arg));
                }
            }

            var si = new STARTUPINFO();
            si.cb = Marshal.SizeOf(si);
            si.dwFlags = STARTF_USESTDHANDLES;
            si.hStdOutput = hStdOut;
            si.hStdError = hStdErr;
            si.hStdInput = IntPtr.Zero;

            uint createFlags = CREATE_SUSPENDED | CREATE_NO_WINDOW;
            bool ok = CreateProcessW(exePath, cmdLine, IntPtr.Zero, IntPtr.Zero, true, createFlags, IntPtr.Zero, null, ref si, out pi);
            if (!ok) throw new Win32Exception(Marshal.GetLastWin32Error(), "CreateProcessW failed");

            CloseHandle(hStdOut);
            hStdOut = INVALID_HANDLE_VALUE;
            CloseHandle(hStdErr);
            hStdErr = INVALID_HANDLE_VALUE;

            if (!AssignProcessToJobObject(hJob, pi.hProcess))
            {
                TerminateProcess(pi.hProcess, 1);
                throw new Win32Exception(Marshal.GetLastWin32Error(), "AssignProcessToJobObject failed");
            }

            if (ResumeThread(pi.hThread) == unchecked((uint)-1))
            {
                TerminateProcess(pi.hProcess, 1);
                throw new Win32Exception(Marshal.GetLastWin32Error(), "ResumeThread failed");
            }

            string outcome = "completed";
            long peakPrivateBytes = 0;
            long peakWorkingSetBytes = 0;
            var sw = Stopwatch.StartNew();
            long maxDurationMs = (long)timeoutSeconds * 1000L;

            while (true)
            {
                while (GetQueuedCompletionStatus(hPort, out uint msgId, out UIntPtr compKey, out IntPtr lpOverlapped, 0))
                {
                    if (msgId == JOB_OBJECT_MSG_PROCESS_MEMORY_LIMIT)
                    {
                        outcome = "memory_limit";
                        TerminateJobObject(hJob, 999);
                        break;
                    }
                }
                if (outcome == "memory_limit") break;

                var memCounters = new PROCESS_MEMORY_COUNTERS_EX();
                if (GetProcessMemoryInfo(pi.hProcess, out memCounters, (uint)Marshal.SizeOf(typeof(PROCESS_MEMORY_COUNTERS_EX))))
                {
                    long curPriv = (long)memCounters.PrivateUsage.ToUInt64();
                    long curWs = (long)memCounters.WorkingSetSize.ToUInt64();
                    long peakPriv = (long)memCounters.PeakPagefileUsage.ToUInt64();
                    long peakWs = (long)memCounters.PeakWorkingSetSize.ToUInt64();

                    if (curPriv > peakPrivateBytes) peakPrivateBytes = curPriv;
                    if (peakPriv > peakPrivateBytes) peakPrivateBytes = peakPriv;
                    if (curWs > peakWorkingSetBytes) peakWorkingSetBytes = curWs;
                    if (peakWs > peakWorkingSetBytes) peakWorkingSetBytes = peakWs;
                }

                var jobInfo = new JOBOBJECT_EXTENDED_LIMIT_INFORMATION();
                if (QueryInformationJobObject(hJob, JobObjectExtendedLimitInformation, ref jobInfo, Marshal.SizeOf(jobInfo), IntPtr.Zero))
                {
                    long jobPeak = (long)jobInfo.PeakProcessMemoryUsed.ToUInt64();
                    if (jobPeak > peakPrivateBytes) peakPrivateBytes = jobPeak;
                }

                if (WaitForSingleObject(pi.hProcess, 0) == WAIT_OBJECT_0) break;

                if (sw.ElapsedMilliseconds >= maxDurationMs)
                {
                    outcome = "timeout";
                    TerminateJobObject(hJob, 998);
                    break;
                }

                // Always sample and drain the completion port once more after exit.
                WaitForSingleObject(pi.hProcess, 100);
            }

            WaitForSingleObject(pi.hProcess, 5000);
            while (GetQueuedCompletionStatus(hPort, out uint finalMessage, out UIntPtr finalKey, out IntPtr finalProcess, 0)) {
                if (finalMessage == JOB_OBJECT_MSG_PROCESS_MEMORY_LIMIT) outcome = "memory_limit";
            }
            if (GetProcessMemoryInfo(pi.hProcess, out PROCESS_MEMORY_COUNTERS_EX finalCounters, (uint)Marshal.SizeOf(typeof(PROCESS_MEMORY_COUNTERS_EX)))) {
                peakWorkingSetBytes = Math.Max(peakWorkingSetBytes, (long)finalCounters.PeakWorkingSetSize.ToUInt64());
                peakPrivateBytes = Math.Max(peakPrivateBytes, (long)finalCounters.PeakPagefileUsage.ToUInt64());
            }

            var finalJobInfo = new JOBOBJECT_EXTENDED_LIMIT_INFORMATION();
            if (QueryInformationJobObject(hJob, JobObjectExtendedLimitInformation, ref finalJobInfo, Marshal.SizeOf(finalJobInfo), IntPtr.Zero))
            {
                long jobPeak = (long)finalJobInfo.PeakProcessMemoryUsed.ToUInt64();
                if (jobPeak > peakPrivateBytes) peakPrivateBytes = jobPeak;
            }

            int? finalExitCode = null;
            if (GetExitCodeProcess(pi.hProcess, out uint rawExitCode))
            {
                finalExitCode = unchecked((int)rawExitCode);
            }

            if (outcome == "completed" && finalExitCode.HasValue && finalExitCode.Value != 0)
            {
                outcome = "process_error";
            }

            return new RunResult
            {
                Outcome = outcome,
                ExitCode = finalExitCode,
                Pid = pi.dwProcessId,
                PeakPrivateBytes = peakPrivateBytes,
                PeakWorkingSetBytes = peakWorkingSetBytes,
                SamplingIntervalMs = 100,
                MemoryLimitNotified = outcome == "memory_limit"
            };
        }
        finally
        {
            if (pi.hThread != IntPtr.Zero) CloseHandle(pi.hThread);
            if (pi.hProcess != IntPtr.Zero) CloseHandle(pi.hProcess);
            if (hStdOut != INVALID_HANDLE_VALUE && hStdOut != IntPtr.Zero) CloseHandle(hStdOut);
            if (hStdErr != INVALID_HANDLE_VALUE && hStdErr != IntPtr.Zero) CloseHandle(hStdErr);
            if (hPort != IntPtr.Zero) CloseHandle(hPort);
            if (hJob != IntPtr.Zero) CloseHandle(hJob);
        }
    }
}
'@
}

try {
    $result = [NativeProcessRunner]::Run(
        $resolvedExecutable,
        $argList,
        $stdoutFile,
        $stderrFile,
        $PrivateLimitMiB,
        $TimeoutSeconds
    )
} catch {
    Write-Error "Execution setup failed: $_"
    exit 1
}

$payload = [ordered]@{
    outcome               = $result.Outcome
    exit_code             = $result.ExitCode
    pid                   = $result.Pid
    peak_private_bytes    = $result.PeakPrivateBytes
    peak_working_set_bytes = $result.PeakWorkingSetBytes
    sampling_interval_ms  = $result.SamplingIntervalMs
    memory_limit_notified = $result.MemoryLimitNotified
}

$jsonOutput = $payload | ConvertTo-Json -Depth 4
Set-Content -LiteralPath $processJsonFile -Value $jsonOutput -Encoding utf8

if ($result.Outcome -ne 'completed' -or ($null -ne $result.ExitCode -and $result.ExitCode -ne 0)) {
    exit 1
}

exit 0

