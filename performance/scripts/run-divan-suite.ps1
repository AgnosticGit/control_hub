param(
  [string]$RunId = "$( [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ') )_divan",
  [ValidateRange(1, 10)][int]$Repetitions = 3,
  [ValidateRange(10, 1000)][int]$SampleCount = 30
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$run = Join-Path (Join-Path $repo 'performance\runs') $RunId
if (Test-Path -LiteralPath $run) { throw "Refusing to overwrite existing run: $run" }
$logs = Join-Path $run 'logs'
$null = New-Item -ItemType Directory -Path $logs -Force

$workloads = @(
  @{ package = 'karbird-api'; target = 'allocation'; id = 'divan:api/group_500_hosts.v1'; source = 'crates/karbird-api/benches/allocation.rs' },
  @{ package = 'karbird-api'; target = 'allocation'; id = 'divan:api/place_200_resources_600_addresses.v1'; source = 'crates/karbird-api/benches/allocation.rs' },
  @{ package = 'karbird-archive'; target = 'allocation'; id = 'divan:archive/encode_512_logs.v1'; source = 'crates/karbird-archive/benches/allocation.rs' },
  @{ package = 'karbird-convert'; target = 'allocation'; id = 'divan:convert/convert_256_custom_events.v1'; source = 'crates/karbird-convert/benches/allocation.rs' },
  @{ package = 'karbird-convert'; target = 'allocation'; id = 'divan:convert/normalize_1000_logs.v1'; source = 'crates/karbird-convert/benches/allocation.rs' },
  @{ package = 'karbird-engine-core'; target = 'allocation'; id = 'divan:engine/apply_batch_2000_spans.v1'; source = 'crates/karbird-engine-core/benches/allocation.rs' },
  @{ package = 'karbird-ingest'; target = 'allocation'; id = 'divan:ingest/sanitize_1000_traces.v1'; source = 'crates/karbird-ingest/benches/allocation.rs' },
  @{ package = 'karbird-protocol'; target = 'allocation'; id = 'divan:protocol/encode_256_events.v1'; source = 'crates/karbird-protocol/benches/allocation.rs' }
)

$lockPath = Join-Path $repo 'Cargo.lock'
$metadata = [ordered]@{
  schema_version = 1
  run_id = $RunId
  mode = 'allocation-baseline'
  started_utc = [DateTime]::UtcNow.ToString('o')
  linked_criterion_run = '20261012T135755240Z_workspace'
  rustc = (& rustc -Vv | Out-String).Trim()
  cargo = (& cargo -V | Out-String).Trim()
  cargo_lock_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $lockPath).Hash.ToLowerInvariant()
  criterion = '0.8.2 (see linked full-workspace run)'
  divan = '0.1.21'
  repetitions = $Repetitions
  sample_count = $SampleCount
  workloads = $workloads
  unavailable = @{
    gungraun = 'Valgrind is unavailable on the Windows host; WSL Ubuntu has Valgrind 3.18.1, below Gungraun tested versions, and installation/build prerequisites require elevated setup.'
    linux_perf = 'The benchmark host is Windows; WSL perf is not installed and hardware-counter access is restricted.'
  }
}
$metadata | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $run 'metadata.json') -Encoding utf8

$status = 'completed'
for ($repetition = 1; $repetition -le $Repetitions; $repetition++) {
  foreach ($workload in $workloads) {
    $log = Join-Path $logs "rep-$repetition-$($workload.package).log"
    $priorPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $output = & cargo bench --locked -p $workload.package --bench $workload.target -- --sample-count $SampleCount 2>&1
    $exitCode = $LASTEXITCODE
    $ErrorActionPreference = $priorPreference
    $output | ForEach-Object { $_.ToString() } | Set-Content -LiteralPath $log -Encoding utf8
    if ($exitCode -ne 0) {
      $status = 'partial'
      "FAILED: cargo bench --locked -p $($workload.package) --bench $($workload.target) -- --sample-count $SampleCount (exit $exitCode)" | Add-Content -LiteralPath $log -Encoding utf8
      break
    }
  }
  if ($status -ne 'completed') { break }
}

$metadata.status = $status
$metadata.finished_utc = [DateTime]::UtcNow.ToString('o')
$metadata | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $run 'metadata.json') -Encoding utf8

$files = Get-ChildItem -LiteralPath $run -File -Recurse | ForEach-Object {
  @{ path = $_.FullName.Substring($run.Length + 1).Replace('\', '/'); sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash.ToLowerInvariant(); bytes = $_.Length; generated_by = 'run-divan-suite.ps1' }
}
@{ schema_version = 1; run_id = $RunId; status = $status; files = $files } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $run 'manifest.json') -Encoding utf8
Write-Output "Run: performance/runs/$RunId"
Write-Output "Status: $status"
if ($status -ne 'completed') { exit 1 }
