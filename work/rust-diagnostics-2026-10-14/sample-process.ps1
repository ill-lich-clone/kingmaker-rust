$ErrorActionPreference='Stop'
$diagRoot=$PSScriptRoot
$profilePath=Join-Path $diagRoot 'run/profile.jsonl'
$samplePath=Join-Path $diagRoot 'process-samples.jsonl'
$deadline=[DateTime]::UtcNow.AddSeconds(1900)
while (-not (Test-Path -LiteralPath $profilePath)) {
 if ([DateTime]::UtcNow -gt $deadline) { throw 'Profile did not appear' }
 Start-Sleep -Milliseconds 500
}
$header=$null
while (-not $header) {
 try { $header=Get-Content -LiteralPath $profilePath -TotalCount 1 | ConvertFrom-Json } catch {}
 if (-not $header) { Start-Sleep -Milliseconds 100 }
}
$targetProcessId=[int]$header.pid
$writer=[IO.StreamWriter]::new($samplePath,$false,[Text.UTF8Encoding]::new($false))
try {
 while ([DateTime]::UtcNow -lt $deadline) {
  $sampleProcess=Get-Process -Id $targetProcessId -ErrorAction SilentlyContinue
  if (-not $sampleProcess) { break }
  $sampleProcess.Refresh()
  [ordered]@{utc=[DateTime]::UtcNow.ToString('o');pid=$targetProcessId;elapsed_seconds=([DateTime]::UtcNow-$sampleProcess.StartTime.ToUniversalTime()).TotalSeconds;cpu_seconds=$sampleProcess.TotalProcessorTime.TotalSeconds;user_seconds=$sampleProcess.UserProcessorTime.TotalSeconds;kernel_seconds=$sampleProcess.PrivilegedProcessorTime.TotalSeconds;private_bytes=$sampleProcess.PrivateMemorySize64;working_set_bytes=$sampleProcess.WorkingSet64;threads=$sampleProcess.Threads.Count;priority=[string]$sampleProcess.PriorityClass} | ConvertTo-Json -Compress | ForEach-Object {$writer.WriteLine($_)}
  $writer.Flush()
  Start-Sleep -Seconds 1
 }
} finally { $writer.Dispose() }
Write-Output 'Process sampling finished'