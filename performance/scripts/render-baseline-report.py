#!/usr/bin/env python3
"""Render the active baseline's complete workload table and measurement caveats."""
import datetime
import json
import pathlib
import tomllib


ROOT = pathlib.Path(__file__).resolve().parents[1]


def main() -> int:
    config = tomllib.loads((ROOT / "config.toml").read_text(encoding="utf-8"))
    name = config["active_baseline"]
    baseline = ROOT / "baseline" / name
    metadata = json.loads((baseline / "metadata.json").read_text(encoding="utf-8-sig"))
    summary = json.loads((baseline / "results/summary.json").read_text(encoding="utf-8-sig"))
    catalog = json.loads((ROOT / "benchmarks/workloads/workspace-criterion-v1.json").read_text(encoding="utf-8"))
    descriptions = {item["workload_id"]: item for item in catalog["workloads"]}
    metrics = summary["metrics"]
    ranges = sorted(item.get("between_run_range_pct", 0) or 0 for item in metrics)
    median_range = ranges[len(ranges) // 2]
    p90_range = ranges[int(0.9 * (len(ranges) - 1))]
    lines = [
        "# Clean Rust workspace performance baseline",
        "",
        f"**Status: COMPLETE for the existing Rust Criterion suite.** Baseline `{name}` covers {len(metrics)} workloads across {summary['repetition_count']} repetitions from two independent full-suite runs.",
        "",
        "## Source and environment",
        "",
        f"- Source Git SHA: `{metadata['git_sha']}`; production Rust sources were clean (`production_sources_dirty={str(metadata['production_sources_dirty']).lower()}).",
        f"- Source runs: {', '.join(f'`{item}`' for item in metadata['source_run_ids'])}.",
        f"- Cargo.lock SHA-256: `{metadata['cargo_lock_sha256']}`; compatibility key: `{metadata['compatibility_key']}`.",
        f"- {metadata['os']}, {metadata['architecture']}; {metadata['cpu_model']}; {metadata['logical_cores']} logical processors; {metadata['ram_bytes']:,} bytes RAM.",
        f"- {metadata['rustc'].splitlines()[0]}; {metadata['cargo']}; optimized Cargo bench profile; workspace default features; system allocator.",
        "- Criterion 0.8; 30 samples, 3-second warmup, 5-second measurement per case; deterministic fixed benchmark fixtures.",
        f"- Across these six repetitions, per-workload between-run range has a median of {median_range:.1f}% and a 90th percentile of {p90_range:.1f}%. Use this spread when judging small changes.",
        "",
        "## All measured workloads",
        "",
        "Each time is the mean cost of the entire benchmark operation, not per item. Between-run range is `(max repetition mean − min repetition mean) / aggregate mean`.",
        "",
        "| Workload | Operation | Mean (ns) | Criterion SD (ns) | Six-run range |",
        "|---|---|---:|---:|---:|",
    ]
    for metric in sorted(metrics, key=lambda item: item["workload_id"]):
        workload = descriptions[metric["workload_id"]]
        operation = workload["scenario"].replace("|", "\\|")
        value = metric["mean_ns"]
        sd = metric.get("std_dev_ns")
        spread = metric.get("between_run_range_pct")
        lines.append(f"| `{metric['workload_id']}` | {operation} | {value:.2f} | {sd:.2f} | {spread:.1f}% |")
    lines += [
        "",
        "## Historical measurements and limits",
        "",
        "- `baseline/initial/` and `reports/baseline-initial-partial.md` preserve the original three-case baseline unchanged.",
        "- `runs/historic-target-criterion_20261012T033203378Z/` preserves 134 existing Criterion estimate sets and the 40-case prior `new` set. They were collected under different conditions and are supplementary historical context, not mixed into this clean baseline.",
        "- The full Rust CPU microbenchmark suite covers 15 existing benchmark targets across domain, API, archive, archiver, CLI, convert, engine, executor, ingest, processor, protocol, runtime, storage, and web-engine crates.",
        "- This suite does not measure live Kafka/PostgreSQL/ClickHouse/S3 latency, full service throughput, browser rendering, peak RSS, allocation counts, hardware counters, or CPU profiles. Do not infer those values from Criterion CPU timings.",
        "- The first full run showed substantially higher variation than the later idle runs; it is preserved as calibration history and not included in the active baseline.",
        "",
        f"Generated {datetime.datetime.now(datetime.timezone.utc).isoformat()} from the immutable baseline artifacts.",
        "",
    ]
    (ROOT / "reports/baseline.md").write_text("\n".join(lines), encoding="utf-8")
    print(f"Rendered {len(metrics)} workloads from baseline:{name}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
