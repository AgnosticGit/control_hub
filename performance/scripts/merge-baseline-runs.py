#!/usr/bin/env python3
"""Merge repeated full-workspace runs into a new immutable active baseline."""
import hashlib
import json
import pathlib
import shutil
import subprocess
import sys


def main() -> int:
    if len(sys.argv) != 5:
        print("usage: merge-baseline-runs.py <baseline-name> <run-id-1> <run-id-2> <previous-active-baseline>", file=sys.stderr)
        return 2
    root = pathlib.Path(__file__).resolve().parents[1]
    name, *args = sys.argv[1:]
    run_ids, previous_name = args[:2], args[2]
    destination = root / "baseline" / name
    if destination.exists():
        print(f"Refusing to overwrite baseline: {destination}", file=sys.stderr)
        return 1
    runs = [root / "runs" / run_id for run_id in run_ids]
    metadata = [json.loads((run / "metadata.json").read_text(encoding="utf-8-sig")) for run in runs]
    summaries = [json.loads((run / "results/summary.json").read_text(encoding="utf-8-sig")) for run in runs]
    if any(item.get("status") != "completed" or len(summary.get("metrics", [])) != 40 or summary.get("repetition_count") != 3 for item, summary in zip(metadata, summaries)):
        print("Refusing merge: each source must be a completed 40-workload run with three repetitions", file=sys.stderr)
        return 1
    reference = metadata[0]
    for current in metadata[1:]:
        for key in ("compatibility_key", "workloads", "workload_source_sha256"):
            if current.get(key) != reference.get(key):
                print(f"Refusing merge: source runs differ in {key}", file=sys.stderr)
                return 1
    destination.mkdir(parents=True)
    results = destination / "results"
    results.mkdir()
    for source_index, run in enumerate(runs):
        for repetition in range(1, 4):
            target_index = source_index * 3 + repetition
            shutil.copytree(run / "results" / f"repetition-{repetition}", results / f"repetition-{target_index}")
    subprocess.run([sys.executable, str(root / "scripts/aggregate-repetitions.py"), str(destination), "6"], check=True)
    reference.update({
        "baseline_name": name,
        "baseline_run_id": None,
        "run_id": name,
        "status": "completed",
        "source_run_ids": run_ids,
        "historical_measurement_archive": "runs/historic-target-criterion_20261012T033203378Z",
        "historical_note": "Older Criterion estimate sets and the original partial baseline are preserved as supplementary history; only the six current full-suite repetitions form this comparable baseline.",
    })
    reference["repetition"]["repetitions"] = 6
    (destination / "metadata.json").write_text(json.dumps(reference, indent=2) + "\n", encoding="utf-8")
    manifest_files = []
    for path in sorted(destination.rglob("*")):
        if path.is_file() and path.name != "manifest.json":
            manifest_files.append({"path": path.relative_to(destination).as_posix(), "sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "bytes": path.stat().st_size, "generated_by": "merge-baseline-runs.py"})
    manifest = {"schema_version": 1, "baseline_name": name, "source_run_ids": run_ids, "previous_active_baseline": previous_name, "status": "completed", "files": manifest_files}
    (destination / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    config_path = root / "config.toml"
    config = config_path.read_text(encoding="utf-8")
    old = f'active_baseline = "{previous_name}"'
    if old not in config:
        print("Refusing to change config: previous active baseline marker is missing", file=sys.stderr)
        return 1
    config_path.write_text(config.replace(old, f'active_baseline = "{name}"', 1), encoding="utf-8")
    print(f"Created immutable baseline {name}: 40 workloads across six repetitions from {', '.join(run_ids)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
