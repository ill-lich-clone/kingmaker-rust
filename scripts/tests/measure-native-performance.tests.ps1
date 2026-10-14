$ErrorActionPreference = 'Stop'
$monitor = Join-Path $PSScriptRoot '../measure-native-performance.ps1'
if (-not (Test-Path -LiteralPath $monitor)) { throw 'Missing process monitor: cannot enforce native memory budget' }
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('native-performance-tests-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $testRoot | Out-Null
$exe = Join-Path $testRoot 'performance-child.exe'
& rustc (Join-Path $PSScriptRoot 'performance-child.rs') -o $exe
if ($LASTEXITCODE -ne 0) { throw 'Could not build test helper' }
$foreign = Start-Process -FilePath $exe -ArgumentList 'sleep' -PassThru -WindowStyle Hidden
function Check-Run([string]$name, [string[]]$childArgs, [int]$seconds, [string]$want) {
    $argFile = Join-Path $testRoot "$name.args.json"
    ConvertTo-Json -InputObject $childArgs -Compress | Set-Content -LiteralPath $argFile
    $out = Join-Path $testRoot $name
    & pwsh -NoProfile -File $monitor -Executable $exe -ArgumentFile $argFile -OutputDirectory $out -PrivateLimitMiB 64 -TimeoutSeconds $seconds
    $exit = $LASTEXITCODE
    $result = Get-Content -LiteralPath (Join-Path $out 'process.json') -Raw | ConvertFrom-Json
    if ($result.outcome -ne $want) { throw "$name outcome=$($result.outcome), expected=$want" }
    if (($want -eq 'completed') -ne ($exit -eq 0)) { throw "$name wrong monitor exit=$exit" }
    if ($result.peak_private_bytes -le 0 -or $result.peak_working_set_bytes -le 0) { throw "$name did not measure process memory" }
    if ($result.sampling_interval_ms -ne 100) { throw "$name sampling contract broken" }
    $foreign.Refresh()
    if ($foreign.HasExited) { throw 'Monitor terminated unrelated same-name process' }
    if ($want -eq 'process_error' -and $result.exit_code -ne 7) { throw 'Child exit code lost' }
    if ($want -eq 'memory_limit' -and -not $result.memory_limit_notified) { throw 'Memory limit outcome lacks OS evidence' }
    if (Get-Process -Id $result.pid -ErrorAction SilentlyContinue) { throw 'Monitored process left alive' }
    if ($name -eq 'arguments') {
        $actual = @(Get-Content -LiteralPath (Join-Path $out 'stdout.log'))
        if (@(Compare-Object -ReferenceObject $childArgs -DifferenceObject $actual -SyncWindow 0).Count) { throw 'Argument round trip failed' }
    }
    Write-Host "PASS $name"
}
try {
    Check-Run 'arguments' @('hello world', 'кириллица', 'quote"inside', 'trailing slash\') 10 'completed'
    Check-Run 'exit' @('fail') 10 'process_error'
    Check-Run 'timeout' @('sleep') 1 'timeout'
    Check-Run 'memory' @('allocate') 15 'memory_limit'
    Write-Host "All process-monitor tests passed. Artifacts: $testRoot"
} finally {
    $foreign.Refresh()
    if (-not $foreign.HasExited) { $foreign.Kill(); $foreign.WaitForExit() }
    $foreign.Dispose()
}
