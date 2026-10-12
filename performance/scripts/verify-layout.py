#!/usr/bin/env python3
"""Validate required performance artifact layout and SHA-256 manifests."""
import hashlib, json, pathlib, sys

ROOT = pathlib.Path(__file__).resolve().parents[1]

def check_manifest(directory: pathlib.Path) -> list[str]:
    problems = []
    manifest_path = directory / "manifest.json"
    if not manifest_path.is_file():
        return [f"missing {manifest_path.relative_to(ROOT)}"]
    data = json.loads(manifest_path.read_text(encoding="utf-8-sig"))
    for item in data.get("files", []):
        path = directory / item["path"]
        if not path.is_file():
            problems.append(f"missing artifact {path.relative_to(ROOT)}")
            continue
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        if digest != item["sha256"]:
            problems.append(f"hash mismatch {path.relative_to(ROOT)}")
    return problems

def main() -> int:
    required = ["README.md", "config.toml", "baseline/initial/metadata.json", "baseline/initial/manifest.json", "baseline/initial/results", "benchmarks/workloads/workspace-criterion-v1.json", "benchmarks/datasets", "runs", "reports/baseline.md", "reports/latest.md", "reports/optimization-2026-10.md", "reports/comparisons", "scripts/run-benchmarks.sh", "scripts/compare.py", "scripts/collect-criterion.py", "scripts/aggregate-repetitions.py", "scripts/snapshot-baseline.py", "scripts/merge-baseline-runs.py", "scripts/render-baseline-report.py", "scripts/render-optimization-report.py"]
    problems = [f"missing {item}" for item in required if not (ROOT / item).exists()]
    for baseline in (ROOT / "baseline").iterdir() if (ROOT / "baseline").exists() else []:
        if baseline.is_dir() and (baseline / "manifest.json").exists(): problems.extend(check_manifest(baseline))
    for run in (ROOT / "runs").iterdir() if (ROOT / "runs").exists() else []:
        if run.is_dir(): problems.extend(check_manifest(run))
    if problems:
        print("INVALID\n" + "\n".join(problems)); return 1
    print("VALID: required layout and all present manifests verified")
    return 0

if __name__ == "__main__": sys.exit(main())
