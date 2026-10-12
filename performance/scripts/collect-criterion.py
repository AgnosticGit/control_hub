#!/usr/bin/env python3
"""Normalize Criterion estimate files without discarding their raw artifacts."""
import argparse
import json
import pathlib
import sys


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("criterion_dir", type=pathlib.Path)
    parser.add_argument("output", type=pathlib.Path)
    parser.add_argument("--all-result-sets", action="store_true", help="include historical Criterion labels such as base/before-opt")
    parser.add_argument("--catalog", type=pathlib.Path, help="versioned workload catalog; requires every catalog case to have a fresh new estimate")
    args = parser.parse_args()
    root = args.criterion_dir.resolve()
    catalog_by_id = {}
    if args.catalog:
        catalog = json.loads(args.catalog.read_text(encoding="utf-8-sig"))
        catalog_by_id = {item["criterion_id"]: item for item in catalog["workloads"]}
    rows = []
    for estimate_path in sorted(root.rglob("estimates.json")):
        label = estimate_path.parent.name
        if not args.all_result_sets and label != "new":
            continue
        benchmark_json = estimate_path.parent / "benchmark.json"
        if not benchmark_json.exists():
            benchmark_json = next(iter(estimate_path.parent.parent.glob("*/benchmark.json")), benchmark_json)
        sample_json = estimate_path.parent / "sample.json"
        try:
            estimates = json.loads(estimate_path.read_text(encoding="utf-8-sig"))
            benchmark = json.loads(benchmark_json.read_text(encoding="utf-8-sig")) if benchmark_json.exists() else {}
            sample = json.loads(sample_json.read_text(encoding="utf-8-sig")) if sample_json.exists() else {}
        except (OSError, json.JSONDecodeError) as exc:
            print(f"Invalid Criterion artifact {estimate_path}: {exc}", file=sys.stderr)
            return 2
        mean = estimates.get("mean", {})
        std_dev = estimates.get("std_dev", {})
        criterion_id = benchmark.get("full_id", estimate_path.parent.parent.name)
        if catalog_by_id and criterion_id not in catalog_by_id:
            continue
        item = catalog_by_id.get(criterion_id, {})
        rows.append({
            "workload_id": item.get("workload_id"),
            "source": item.get("source"),
            "criterion_id": criterion_id,
            "result_set": label,
            "metric": "latency",
            "unit": "nanoseconds per iteration",
            "direction": "lower_is_better",
            "mean_ns": mean.get("point_estimate"),
            "mean_ci95_ns": mean.get("confidence_interval"),
            "std_dev_ns": std_dev.get("point_estimate"),
            "sample_count": len(sample.get("iters", [])) or None,
            "raw_estimates": estimate_path.relative_to(root).as_posix(),
        })
    if catalog_by_id:
        found = {row["criterion_id"] for row in rows}
        missing = sorted(set(catalog_by_id) - found)
        if missing:
            print("Missing fresh Criterion estimates: " + ", ".join(missing), file=sys.stderr)
            return 3
    document = {"schema_version": 1, "status": "completed" if rows else "empty", "metrics": rows}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
    print(f"Collected {len(rows)} Criterion estimate sets")
    return 0 if rows else 1


if __name__ == "__main__":
    raise SystemExit(main())
