$ErrorActionPreference='Stop'
$runner=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../work/performance-remediation-2026-10-14/run-series.ps1'))
$root=Join-Path ([IO.Path]::GetTempPath()) ('remediation-series-tests-'+[Guid]::NewGuid().ToString('N'))
$null=New-Item -ItemType Directory -Path $root
$exe=Join-Path $root 'fake.exe';'not executed' | Set-Content -LiteralPath $exe
$db=Join-Path $root 'fixture.db';'frozen fixture' | Set-Content -LiteralPath $db
$runtime=Join-Path $root 'runtime.json';@{workers=1;dbPath=$db;campaignId='fixture'} | ConvertTo-Json | Set-Content -LiteralPath $runtime
@{status='passed';executable_sha256=(Get-FileHash $exe).Hash} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'local-admission.json')
$fake=Join-Path $root 'fake-monitor.ps1'
@'
param($Executable,$ArgumentFile,$OutputDirectory,[int]$PrivateLimitMiB,[int]$TimeoutSeconds)
$ErrorActionPreference='Stop'
if($PrivateLimitMiB -ne 12288 -or $TimeoutSeconds -ne 1800){throw 'Wrong monitor limits'}
$argsList=@(Get-Content -LiteralPath $ArgumentFile -Raw | ConvertFrom-Json)
$run=$argsList[[array]::IndexOf($argsList,'--output')+1]
$weeks=[int]$argsList[[array]::IndexOf($argsList,'--run-weeks')+1]
$null=New-Item -ItemType Directory -Path $OutputDirectory
$null=New-Item -ItemType Directory -Path $run
foreach($name in @('initial','final')){$dir=Join-Path $run $name;$null=New-Item -ItemType Directory -Path $dir;'{"fixture":"same frozen input"}' | Set-Content -LiteralPath (Join-Path $dir 'manifest.json')}
$outcome=if($env:REMEDIATION_FAKE_OUTCOME){$env:REMEDIATION_FAKE_OUTCOME}else{'completed'}
@{outcome=$outcome;exit_code=0;peak_private_bytes=1048576} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $OutputDirectory 'process.json')
@{status=$(if($env:REMEDIATION_FAKE_RUN_STATUS){$env:REMEDIATION_FAKE_RUN_STATUS}else{'completed'});completed_weeks=$weeks} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $run 'run.json')
1..$weeks | ForEach-Object {@{elapsed_ms=1000;week=$_} | ConvertTo-Json -Compress} | Set-Content -LiteralPath (Join-Path $run 'weeks.jsonl')
if($outcome -ne 'completed'){exit 1}
'@ | Set-Content -LiteralPath $fake
function Case($stage,$hypothesis,$success) {
 & pwsh -NoProfile -File $runner -Stage $stage -Runtime $runtime -Executable $exe -Root $root -Hypothesis $hypothesis -Monitor $fake *> (Join-Path $root ($stage+'-'+$hypothesis+'.log'))
 if(($LASTEXITCODE -eq 0) -ne $success){throw "Wrong gate result $stage / $hypothesis"}
 Write-Output "PASS $stage / $hypothesis"
}
Case A premature $false
Case diagnostic first $true
Case diagnostic first $false
Case continuation premature $false
Case A first $true
Case B first $true
Case continuation first $true
$env:REMEDIATION_FAKE_OUTCOME='timeout'
try {Case diagnostic timeout $false;Case diagnostic timeout $false} finally {Remove-Item Env:REMEDIATION_FAKE_OUTCOME}
$env:REMEDIATION_FAKE_RUN_STATUS='cancelled'
try {Case diagnostic stop $false;Case diagnostic stop $false} finally {Remove-Item Env:REMEDIATION_FAKE_RUN_STATUS}
$ledger=Get-Content -LiteralPath (Join-Path $root 'series.json') -Raw | ConvertFrom-Json
if(@($ledger.attempts.run_directory | Select-Object -Unique).Count -ne $ledger.attempts.Count){throw 'Run directories reused'}
if(@($ledger.attempts | Where-Object {$_.hypothesis -eq 'timeout'}).Count -ne 1){throw 'Failed identical run repeated'}
$lock=[IO.File]::Open((Join-Path $root 'full-world.lock'),[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
try {Case diagnostic parallel $false} finally {$lock.Dispose()}
Write-Output "PASS limits, unique dirs, 1->10+10->52, STOP/timeout no-repeat, concurrent exclusion; artifacts $root"
exit 0
