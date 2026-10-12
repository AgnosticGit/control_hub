param([ValidateSet('baseline','optimize')][string]$Mode='baseline',[switch]$Help)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$perf = Join-Path $repo 'performance'
$catalogPath = Join-Path $perf 'benchmarks\workloads\workspace-criterion-v1.json'
$catalog = Get-Content -Raw -LiteralPath $catalogPath | ConvertFrom-Json
$config = Get-Content -Raw -LiteralPath (Join-Path $perf 'config.toml')
$activeMatch = [regex]::Match($config, '(?m)^active_baseline\s*=\s*"([^"]+)"')
$activeBaseline = if ($activeMatch.Success) { $activeMatch.Groups[1].Value } else { '' }
if ($Help) {
  Write-Output 'Run the complete Criterion suite from every Rust workspace bench target, 3 repetitions, 30 samples each.'
  Write-Output 'PowerShell: powershell -NoProfile -ExecutionPolicy Bypass -File performance/scripts/run-benchmarks.ps1'
  Write-Output 'After the clean baseline is activated, pass -Mode optimize to compare a new full-suite run with it.'
  exit 0
}
if ($Mode -eq 'optimize' -and (!$activeBaseline -or $activeBaseline -like 'pending-*' -or !(Test-Path (Join-Path $perf "baseline\$activeBaseline\manifest.json")))) {
  throw 'Optimize mode requires a verified active baseline configured in performance/config.toml.'
}

$runId = "$( [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ') )_workspace"
$run = Join-Path (Join-Path $perf 'runs') $runId
if (Test-Path -LiteralPath $run) { throw "Refusing to overwrite existing run: $run" }
$null = New-Item -ItemType Directory -Path $run
$results = Join-Path $run 'results'; $logs = Join-Path $run 'logs'
$null = New-Item -ItemType Directory -Path $results,$logs
$lock = Join-Path $repo 'Cargo.lock'
$lockHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $lock).Hash.ToLowerInvariant()
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1 -ExpandProperty Name
$logical = (Get-CimInstance Win32_ComputerSystem).NumberOfLogicalProcessors
$memory = [int64](Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory
$rustVersion = (& rustc -Vv 2>&1 | Out-String).Trim()
$rustShort = (& rustc -V 2>&1 | Out-String).Trim()
$cargoVersion = (& cargo -V 2>&1 | Out-String).Trim()

# Git calls here are read-only metadata inspection, permitted by AGENTS.md.
$gitSha = 'unavailable'; $gitDirty = 'unknown'; $gitMetadataReason = 'Git inspection failed.'
$productionDirty = 'unknown'; $priorPreference = $ErrorActionPreference
try {
  $ErrorActionPreference = 'Continue'
  $shaText = (& git -C $repo rev-parse --verify HEAD 2>$null | Out-String).Trim(); $shaExit = $LASTEXITCODE
  $statusText = (& git -C $repo status --porcelain --untracked-files=all 2>$null | Out-String).Trim(); $statusExit = $LASTEXITCODE
  $sourceStatus = (& git -C $repo status --porcelain -- crates 2>$null | Out-String).Trim(); $sourceStatusExit = $LASTEXITCODE
  if ($shaExit -eq 0 -and $statusExit -eq 0 -and $shaText) {
    $gitSha = $shaText; $gitDirty = [bool]$statusText; $gitMetadataReason = ''
    if ($sourceStatusExit -eq 0) { $productionDirty = [bool]$sourceStatus }
  }
} catch {
  $gitMetadataReason = 'Git executable or repository metadata is unavailable.'
} finally { $ErrorActionPreference = $priorPreference }

$sourceHashes = @{}
foreach ($source in ($catalog.workloads.source | Sort-Object -Unique)) {
  $sourcePath = Join-Path $repo ($source.Replace('/', '\'))
  $sourceHashes[$source] = (Get-FileHash -Algorithm SHA256 -LiteralPath $sourcePath).Hash.ToLowerInvariant()
}
$workloadIds = @($catalog.workloads | ForEach-Object workload_id)
$rustTarget = (& rustc -vV | Select-String '^host:' | ForEach-Object { $_.Line.Substring(6) })
$metadata = [ordered]@{
  schema_version = 2; run_id = $runId; mode = $Mode; active_baseline = $activeBaseline; started_utc = [DateTime]::UtcNow.ToString('o')
  git_sha = $gitSha; git_dirty = $gitDirty; production_sources_dirty = $productionDirty; git_metadata_reason = $gitMetadataReason
  os = [System.Runtime.InteropServices.RuntimeInformation]::OSDescription; architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
  cpu_model = $cpu; logical_cores = $logical; ram_bytes = $memory; rustc = $rustVersion; cargo = $cargoVersion
  cargo_lock_sha256 = $lockHash; profile = 'bench (Cargo optimized benchmark profile)'; rustflags = $(if ($env:RUSTFLAGS) { $env:RUSTFLAGS } else { '' })
  target = $rustTarget; features = 'workspace defaults'; allocator = 'system default'
  tools = @{ criterion = '0.8'; nextest = 'unavailable'; cpu_profiler = 'unavailable on this host'; rss_profiler = 'unavailable on this host' }
  catalog_id = $catalog.catalog_id; workloads = $workloadIds; workload_source_sha256 = $sourceHashes
  repetition = @{ repetitions = 3; criterion_sample_size = 30; warm_up_seconds = 3; measurement_seconds = 5 }
  command = 'powershell -NoProfile -ExecutionPolicy Bypass -File performance/scripts/run-benchmarks.ps1'
  compatibility_key = "workspace-criterion-v1|$rustTarget|$rustShort|bench|defaults|lock-$lockHash"
}
$metadata | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $run 'metadata.json') -Encoding utf8

$status = 'completed'; $exitCode = 0
for ($rep = 1; $rep -le 3; $rep++) {
  $repName = "repetition-$rep"
  $repDir = Join-Path $results $repName
  $criterionOut = Join-Path $repDir 'criterion'
  $null = New-Item -ItemType Directory -Path $criterionOut -Force
  $log = Join-Path $logs "workspace-$repName.log"
  Push-Location $repo
  try {
    $priorPreference = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    $benchExitCode = 0
    $benchTargets = @($catalog.workloads | ForEach-Object { "$($_.package)|$($_.bench_target)" } | Sort-Object -Unique)
    foreach ($target in $benchTargets) {
      $parts = $target.Split('|', 2)
      "Running Criterion target $($parts[0])/$($parts[1])" | Add-Content -LiteralPath $log
      & cargo bench -p $parts[0] --bench $parts[1] -- --sample-size 30 --warm-up-time 3 --measurement-time 5 --noplot *>> $log
      if ($LASTEXITCODE -ne 0) { $benchExitCode = $LASTEXITCODE; break }
    }
    $ErrorActionPreference = $priorPreference
  } catch {
    $benchExitCode = 1; $_ | Out-String | Add-Content -LiteralPath $log
  } finally { $ErrorActionPreference = $priorPreference; Pop-Location }
  if ($benchExitCode -ne 0) {
    $exitCode = $benchExitCode; $status = 'partial'
    "One or more targeted cargo bench commands failed in $repName (exit $benchExitCode); no result data from target/criterion was archived as fresh output." | Add-Content -LiteralPath $log
    break
  }

  $criterionRoot = Join-Path $repo 'target\criterion'
  foreach ($item in $catalog.workloads) {
    $folder = $item.criterion_id.Replace('/', '_')
    $sourceDir = Join-Path $criterionRoot $folder
    $destDir = Join-Path $criterionOut $folder
    $newDir = Join-Path $sourceDir 'new'
    if (!(Test-Path -LiteralPath (Join-Path $newDir 'estimates.json'))) {
      $status = 'partial'
      "Missing fresh estimates for $($item.criterion_id) in $repName" | Add-Content -LiteralPath $log
      continue
    }
    $null = New-Item -ItemType Directory -Path (Join-Path $destDir 'new') -Force
    Copy-Item -Path (Join-Path $newDir '*') -Destination (Join-Path $destDir 'new') -Recurse -Force
  }
  $summaryPath = Join-Path $repDir 'summary.json'
  $priorPreference = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
  & py -3 (Join-Path $PSScriptRoot 'collect-criterion.py') $criterionOut $summaryPath --catalog $catalogPath *> (Join-Path $logs "collect-$repName.log")
  $collectExit = $LASTEXITCODE; $ErrorActionPreference = $priorPreference
  if ($collectExit -ne 0) { $status = 'partial'; if ($exitCode -eq 0) { $exitCode = $collectExit } }
}

if ($status -eq 'completed') {
  $priorPreference = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
  & py -3 (Join-Path $PSScriptRoot 'aggregate-repetitions.py') $run 3 *> (Join-Path $logs 'aggregate.log')
  $aggregateExit = $LASTEXITCODE; $ErrorActionPreference = $priorPreference
  if ($aggregateExit -ne 0) { $status = 'partial'; $exitCode = $aggregateExit }
} else {
  @{ schema_version = 2; status = 'partial'; repetition_count = 3; metrics = @() } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $results 'summary.json') -Encoding utf8
}

$metadata.mode = $Mode; $metadata.status = $status; $metadata.finished_utc = [DateTime]::UtcNow.ToString('o')
$metadata | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $run 'metadata.json') -Encoding utf8
$manifestFiles = Get-ChildItem -LiteralPath $run -File -Recurse | Where-Object Name -ne 'manifest.json' | ForEach-Object {
  $relative = $_.FullName.Substring($run.Length+1).Replace('\','/')
  @{ path=$relative; sha256=(Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash.ToLowerInvariant(); bytes=$_.Length; generated_by='run-benchmarks.ps1' }
}
@{ schema_version=1; run_id=$runId; status=$status; files=$manifestFiles } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $run 'manifest.json') -Encoding utf8
Write-Output "Run: performance/runs/$runId"
Write-Output "Status: $status"
if ($Mode -eq 'optimize' -and $status -eq 'completed') {
  $verifyLog = Join-Path $logs 'verify-layout.log'
  $compareLog = Join-Path $logs 'compare.log'
  $priorPreference = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
  & py -3 (Join-Path $PSScriptRoot 'verify-layout.py') *> $verifyLog
  $verifyExit = $LASTEXITCODE
  if ($verifyExit -eq 0) { & py -3 (Join-Path $PSScriptRoot 'compare.py') "baseline:$activeBaseline" $runId *> $compareLog; $compareExit = $LASTEXITCODE } else { $compareExit = $verifyExit }
  $ErrorActionPreference = $priorPreference
  if ($compareExit -ne 0) { Write-Error "Optimize run comparison failed; inspect $compareLog"; exit $compareExit }
}
if ($exitCode -ne 0) { exit $exitCode }
