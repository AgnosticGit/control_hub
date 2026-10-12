#!/usr/bin/env python3
"""Aggregate matching Criterion cases across repeated full-workspace runs."""
import json
import pathlib
import statistics
import sys


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: aggregate-repetitions.py <run-dir> <repetition-count>", file=sys.stderr)
        return 2
    run = pathlib.Path(sys.argv[1])
    count = int(sys.argv[2])
    repeats = []
    for index in range(1, count + 1):
        path = run / "results" / f"repetition-{index}" / "summary.json"
        try:
            document = json.loads(path.read_text(encoding="utf-8-sig"))
        except (OSError, json.JSONDecodeError) as exc:
            print(f"Could not load {path}: {exc}", file=sys.stderr)
            return 1
        repeats.append({metric["workload_id"]: metric for metric in document["metrics"]})
    keys = set(repeats[0])
    if not keys or any(set(repeat) != keys for repeat in repeats[1:]):
        print("Repeated runs do not contain the same non-empty workload set", file=sys.stderr)
        return 3
    metrics = []
    for workload_id in sorted(keys):
        values = [repeat[workload_id] for repeat in repeats]
        means = [item["mean_ns"] for item in values]
        stddevs = [item["std_dev_ns"] for item in values]
        average = statistics.mean(means)
        metrics.append({
            "workload_id": workload_id,
            "criterion_id": values[0]["criterion_id"],
            "source": values[0]["source"],
            "metric": values[0]["metric"],
            "unit": values[0]["unit"],
            "direction": values[0]["direction"],
            "mean_ns": average,
            "std_dev_ns": statistics.mean(stddevs),
            "between_run_std_dev_ns": statistics.stdev(means) if len(means) > 1 else 0,
            "between_run_range_pct": (max(means) - min(means)) / average * 100 if average else None,
            "repetitions": [{"mean_ns": item["mean_ns"], "std_dev_ns": item["std_dev_ns"], "sample_count": item["sample_count"]} for item in values],
        })
    output = {"schema_version": 2, "status": "completed", "repetition_count": count, "metrics": metrics}
    (run / "results" / "summary.json").write_text(json.dumps(output, indent=2) + "\n", encoding="utf-8")
    print(f"Aggregated {len(metrics)} workloads across {count} runs")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
