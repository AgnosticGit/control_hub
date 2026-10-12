#!/usr/bin/env bash
set -euo pipefail
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
if [[ "$(uname -s)" == MINGW* || "$(uname -s)" == MSYS* || "$(uname -s)" == CYGWIN* ]]; then
  if command -v cygpath >/dev/null 2>&1; then script_dir="$(cygpath -w "${script_dir}")"; fi
  exec powershell.exe -NoProfile -ExecutionPolicy Bypass -File "${script_dir}\\run-benchmarks.ps1" "$@"
fi
exec pwsh -NoProfile -File "${script_dir}/run-benchmarks.ps1" "$@"
