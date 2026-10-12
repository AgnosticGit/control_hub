param(
  [Parameter(Mandatory = $true)][string]$RunId,
  [Parameter(Mandatory = $true)][string]$Executable,
  [string]$CompareExecutable,
  [string]$CriterionRun = '20261012T135755240Z_workspace',
  [string]$DivanRun = '20261012T142120657Z_divan',
  [ValidateRange(5, 100)][int]$HyperfineRuns = 20,
  [ValidateRange(1, 20)][int]$RssRepetitions = 5
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$run = Join-Path (Join-Path $repo 'performance\runs') $RunId
if (Test-Path -LiteralPath $run) { throw "Refusing to overwrite existing run: $run" }
$results = Join-Path $run 'results'
$logs = Join-Path $run 'logs'
$binaries = Join-Path $run 'binaries'
$null = New-Item -ItemType Directory -Path $results,$logs,$binaries -Force

$measureSource = (Resolve-Path -LiteralPath $Executable).Path
$compareSource = if ($CompareExecutable) { (Resolve-Path -LiteralPath $CompareExecutable).Path } else { $null }
$measureName = if ($compareSource) { 'candidate' } else { 'baseline' }
$referenceName = if ($compareSource) { 'baseline' } else { $null }
$measureExe = Join-Path $binaries "$measureName.exe"
Copy-Item -LiteralPath $measureSource -Destination $measureExe
$compareExe = if ($compareSource) { Join-Path $binaries 'baseline.exe' } else { $null }
if ($compareSource) { Copy-Item -LiteralPath $compareSource -Destination $compareExe }
$workloads = @(
  @{ id = 'hyperfine:karbird-cli/discover-repository.v1'; args = 'discover --path .' },
  @{ id = 'hyperfine:karbird-cli/plan-repository.v1'; args = 'plan --path .' }
)

$metadata = [ordered]@{
  schema_version = 1
  run_id = $RunId
  mode = $(if ($compareExe) { 'candidate-paired' } else { 'baseline' })
  started_utc = [DateTime]::UtcNow.ToString('o')
  linked_criterion_run = $CriterionRun
  linked_divan_run = $DivanRun
  host_os = [System.Runtime.InteropServices.RuntimeInformation]::OSDescription
  architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
  rustc = (& rustc -Vv | Out-String).Trim()
  cargo = (& cargo -V | Out-String).Trim()
  hyperfine = (& hyperfine --version | Out-String).Trim()
  executable = $measureExe
  executable_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $measureExe).Hash.ToLowerInvariant()
  compare_executable = $compareExe
  compare_executable_sha256 = $(if ($compareExe) { (Get-FileHash -Algorithm SHA256 -LiteralPath $compareExe).Hash.ToLowerInvariant() } else { $null })
  hyperfine_runs = $HyperfineRuns
  warmup_runs = 3
  rss_repetitions = $RssRepetitions
  rss_metric = 'Windows PROCESS_MEMORY_COUNTERS.PeakWorkingSetSize via Get-Process PeakWorkingSet64; bytes'
  workloads = $workloads
}
$metadata | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $run 'metadata.json') -Encoding utf8

$status = 'completed'
foreach ($workload in $workloads) {
  $safeId = $workload.id.Split('/')[-1].Replace('.v1', '')
  $hyperfineJson = Join-Path $results "$safeId-hyperfine.json"
  $log = Join-Path $logs "$safeId-hyperfine.log"
  $priorPreference = $ErrorActionPreference
  $ErrorActionPreference = 'Continue'
  if ($compareExe) {
    $output = & hyperfine --shell=default --warmup 3 --runs $HyperfineRuns --export-json $hyperfineJson `
      --command-name "baseline $safeId" "$compareExe $($workload.args)" `
      --command-name "candidate $safeId" "$measureExe $($workload.args)" 2>&1
  } else {
    $output = & hyperfine --shell=default --warmup 3 --runs $HyperfineRuns --export-json $hyperfineJson `
      --command-name "$safeId" "$measureExe $($workload.args)" 2>&1
  }
  $hyperfineExit = $LASTEXITCODE
  $ErrorActionPreference = $priorPreference
  $output | ForEach-Object { $_.ToString() } | Set-Content -LiteralPath $log -Encoding utf8
  if ($hyperfineExit -ne 0) { $status = 'partial'; break }

  $rssTargets = @(@{ name = $measureName; exe = $measureExe })
  if ($compareExe) { $rssTargets = @(@{ name = $referenceName; exe = $compareExe }) + $rssTargets }
  foreach ($target in $rssTargets) {
    for ($repetition = 1; $repetition -le $RssRepetitions; $repetition++) {
      $rssFile = Join-Path $results "$safeId-$($target.name)-rss-rep-$repetition.json"
      $arguments = $workload.args
      & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'measure-peak-rss.ps1') `
        -Executable $target.exe -Arguments $arguments -WorkingDirectory $repo `
        | Set-Content -LiteralPath $rssFile -Encoding utf8
      if ($LASTEXITCODE -ne 0) { $status = 'partial'; break }
    }
    if ($status -ne 'completed') { break }
  }
  if ($status -ne 'completed') { break }
}

$metadata.status = $status
$metadata.finished_utc = [DateTime]::UtcNow.ToString('o')
$metadata | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $run 'metadata.json') -Encoding utf8

$files = Get-ChildItem -LiteralPath $run -File -Recurse | ForEach-Object {
  @{ path = $_.FullName.Substring($run.Length + 1).Replace('\', '/'); sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash.ToLowerInvariant(); bytes = $_.Length; generated_by = 'run-app-workloads.ps1' }
}
@{ schema_version = 1; run_id = $RunId; status = $status; files = $files } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $run 'manifest.json') -Encoding utf8
Write-Output "Run: performance/runs/$RunId"
Write-Output "Status: $status"
if ($status -ne 'completed') { exit 1 }
