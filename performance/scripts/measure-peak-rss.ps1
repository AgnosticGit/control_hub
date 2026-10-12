param(
  [Parameter(Mandatory = $true)][string]$Executable,
  [Parameter(Mandatory = $true)][string]$Arguments,
  [string]$WorkingDirectory = (Get-Location).Path
)

$ErrorActionPreference = 'Stop'
$resolvedExecutable = (Resolve-Path -LiteralPath $Executable).Path
$startInfo = [System.Diagnostics.ProcessStartInfo]::new()
$startInfo.FileName = $resolvedExecutable
$startInfo.Arguments = $Arguments
$startInfo.WorkingDirectory = (Resolve-Path -LiteralPath $WorkingDirectory).Path
$startInfo.UseShellExecute = $false
$startInfo.CreateNoWindow = $true
$startInfo.RedirectStandardOutput = $true
$startInfo.RedirectStandardError = $true

$process = [System.Diagnostics.Process]::new()
$process.StartInfo = $startInfo
if (!$process.Start()) { throw "Could not start $resolvedExecutable" }
 $peakBytes = $null
do {
  $sampledProcess = Get-Process -Id $process.Id -ErrorAction SilentlyContinue
  if ($sampledProcess) {
    $sample = [int64]$sampledProcess.PeakWorkingSet64
    if ($null -eq $peakBytes -or $sample -gt $peakBytes) { $peakBytes = $sample }
  }
} while (!$process.WaitForExit(1))
$process.WaitForExit()
$stdout = $process.StandardOutput.ReadToEnd()
$stderr = $process.StandardError.ReadToEnd()

[ordered]@{
  executable = $resolvedExecutable
  arguments = $Arguments
  working_directory = $startInfo.WorkingDirectory
  exit_code = $process.ExitCode
  peak_working_set_bytes = $peakBytes
  stdout = $stdout.TrimEnd()
  stderr = $stderr.TrimEnd()
} | ConvertTo-Json -Depth 4

if ($process.ExitCode -ne 0) { exit $process.ExitCode }
