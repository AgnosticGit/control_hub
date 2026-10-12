#!/usr/bin/env python3
"""Create an immutable named baseline snapshot from a completed full run."""
import hashlib
import json
import pathlib
import shutil
import sys


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: snapshot-baseline.py <run-id> <baseline-name>", file=sys.stderr)
        return 2
    root = pathlib.Path(__file__).resolve().parents[1]
    run = root / "runs" / sys.argv[1]
    name = sys.argv[2]
    destination = root / "baseline" / name
    metadata = json.loads((run / "metadata.json").read_text(encoding="utf-8-sig"))
    summary = json.loads((run / "results/summary.json").read_text(encoding="utf-8-sig"))
    if metadata.get("status") != "completed" or len(summary.get("metrics", [])) != 40:
        print("Refusing snapshot: source run must be completed with all 40 workspace workloads", file=sys.stderr)
        return 1
    if destination.exists():
        print(f"Refusing to overwrite baseline: {destination}", file=sys.stderr)
        return 1
    destination.mkdir(parents=True)
    shutil.copytree(run / "results", destination / "results")
    metadata.update({
        "baseline_name": name,
        "baseline_run_id": metadata["run_id"],
        "historical_measurement_archive": "runs/historic-target-criterion_20261012T033203378Z",
        "historical_note": "Older Criterion estimates are preserved in the referenced archive; they used incompatible run conditions and are supplementary context, not merged into this clean baseline.",
    })
    (destination / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    files = []
    for path in sorted(destination.rglob("*")):
        if path.is_file() and path.name != "manifest.json":
            files.append({
                "path": path.relative_to(destination).as_posix(),
                "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                "bytes": path.stat().st_size,
                "generated_by": "snapshot-baseline.py",
            })
    manifest = {"schema_version": 1, "baseline_name": name, "source_run_id": metadata["run_id"], "status": "completed", "files": files}
    (destination / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    config_path = root / "config.toml"
    config = config_path.read_text(encoding="utf-8")
    old = 'active_baseline = "pending-clean-workspace"'
    if old not in config:
        print("Refusing to change config: expected pending baseline marker is missing", file=sys.stderr)
        return 1
    config_path.write_text(config.replace(old, f'active_baseline = "{name}"', 1), encoding="utf-8")
    print(f"Created immutable baseline {name}: {len(summary['metrics'])} workloads from {metadata['run_id']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
