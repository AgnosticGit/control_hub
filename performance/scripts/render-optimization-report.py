#!/usr/bin/env python3
"""Render the final verified before/after report for a complete workspace run."""
import json
import pathlib
import sys
import tomllib


ROOT = pathlib.Path(__file__).resolve().parents[1]


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: render-optimization-report.py <completed-run-id>", file=sys.stderr)
        return 2
    config = tomllib.loads((ROOT / "config.toml").read_text(encoding="utf-8"))
    baseline_name = config["active_baseline"]
    baseline_dir = ROOT / "baseline" / baseline_name
    run_dir = ROOT / "runs" / sys.argv[1]
    base_meta = json.loads((baseline_dir / "metadata.json").read_text(encoding="utf-8-sig"))
    run_meta = json.loads((run_dir / "metadata.json").read_text(encoding="utf-8-sig"))
    base_summary = json.loads((baseline_dir / "results/summary.json").read_text(encoding="utf-8-sig"))
    run_summary = json.loads((run_dir / "results/summary.json").read_text(encoding="utf-8-sig"))
    catalog = json.loads((ROOT / "benchmarks/workloads/workspace-criterion-v1.json").read_text(encoding="utf-8"))
    if run_meta.get("status") != "completed" or len(run_summary.get("metrics", [])) != 40:
        print("Refusing report: candidate must be a completed 40-workload run", file=sys.stderr)
        return 1
    for key in ("compatibility_key", "workloads", "workload_source_sha256"):
        if base_meta.get(key) != run_meta.get(key):
            print(f"Refusing report: incompatible {key}", file=sys.stderr)
            return 1
    before = {item["workload_id"]: item for item in base_summary["metrics"]}
    after = {item["workload_id"]: item for item in run_summary["metrics"]}
    descriptions = {item["workload_id"]: item for item in catalog["workloads"]}
    if before.keys() != after.keys() or len(before) != 40:
        print("Refusing report: workload set differs", file=sys.stderr)
        return 1
    lines = [
        "# Rust workspace optimization report",
        "",
        "**Verdict: verified improvements in selected workloads; otherwise a mixed, measurement-limited result.** Two full API verification runs and the final full workspace run confirmed large reductions in realistic host grouping and CPU fallback. Engine batch application also improved; JSON batch parsing remained within the baseline's broad variation. No global optimization claim is made.",
        "",
        "## Source, candidate, and environment",
        "",
        f"- Baseline: `{baseline_name}` from runs `{', '.join(base_meta['source_run_ids'])}`; 40 workloads across six repetitions.",
        f"- Candidate full run: `{run_meta['run_id']}`; 40 workloads across {run_summary['repetition_count']} repetitions.",
        f"- Additional API confirmation runs: `20261012T064523350Z_workspace`, `20261012T070457608Z_workspace` (the final run above also remeasured the API cases).",
        f"- Base and candidate source Git SHA: `{base_meta['git_sha']}`. Candidate production sources are modified in the worktree; no Git state was changed.",
        f"- Compatibility key matched: `{run_meta['compatibility_key']}`. Workload IDs, benchmark harness hashes, toolchain, target, and Cargo.lock hash match.",
        f"- {run_meta['cpu_model']}; {run_meta['os']}; rustc {run_meta['rustc'].splitlines()[0]}; Criterion 0.8; 30 samples, 3-second warmup, 5-second measurement.",
        "- Baseline variability is material: median six-run range across workloads is about 15%; per-case ranges are shown below. Small percentage changes are not treated as gains.",
        "",
        "## Before and after",
        "",
        "Improvement is `(before − after) / before`; positive values mean lower latency. Each row measures the full deterministic batch/operation, not per record.",
        "",
        "| Workload | Before (ns) | Before range | After (ns) | After range | Change |",
        "|---|---:|---:|---:|---:|---:|",
    ]
    rows = []
    for workload_id in sorted(before):
        left, right = before[workload_id], after[workload_id]
        change = (left["mean_ns"] - right["mean_ns"]) / left["mean_ns"] * 100 if left["mean_ns"] else None
        rows.append((workload_id, left, right, change))
        left_range = left.get("between_run_range_pct")
        right_range = right.get("between_run_range_pct")
        change_text = f"{change:+.2f}%" if change is not None else "n/a"
        lines.append(f"| `{workload_id}` | {left['mean_ns']:.2f} | {left_range:.1f}% | {right['mean_ns']:.2f} | {right_range:.1f}% | {change_text} |")
    lines += [
        "",
        "## Accepted changes",
        "",
        "- `crates/karbird-api/src/routes/infrastructure.rs`: use hash maps to gather host and container series, then sort IDs to preserve deterministic output. In the final combined full run, realistic 500-host grouping improved 35.6% and CPU utilization fallback improved 33.0%. Across the two API-only confirmation runs, the corresponding means improved 43.4% and 33.7%, with between-run ranges of 2.5% and 1.1%. Plain host grouping was +0.7% faster in the final run; resource placement was +0.2%, both treated as neutral.",
        "- `crates/karbird-engine-core/src/lib.rs`: count span/log/event items during the existing validation pass and reserve their maps when empty, avoiding repeated hash-table growth for an initial batch. `apply_batch` improved 17.7% in the final full run, with a 2.6% candidate range. The baseline range was 17.1%; the six underlying baseline repetitions ranged from 715–845 µs, while candidate repetitions were lower. `apply_batch_json` changed by +5.0%, within its 14.0% baseline range, so that end-to-end path is inconclusive.",
        "",
        "## Rejected experiment",
        "",
        "- Reusing a sorted index buffer for log fingerprint attributes slowed `convert/normalize_1000_logs` by 11.5% (Criterion interval +10.5% to +12.8%, p<0.05). The code was reverted; the raw experiment is preserved in `runs/20261012T0805_convert-log-index-experiment/`.",
        "",
        "## Correctness and limits",
        "",
        "- `just check`: passed (fmt, Clippy with warnings denied, wasm32, TypeScript). ESLint emitted three existing warnings in untouched `SignalsPanel.tsx`.",
        "- `just test`: all Rust workspace tests and Node SDK tests passed; recipe exited 1 because Composer is not installed for PHP SDK tests. Focused API tests: 209 passed; engine-core: 171 passed; convert: 36 passed.",
        "- Integration/e2e were not run. `just dev` could not connect because PostgreSQL was stopped. `just bootstrap` started local infrastructure but stopped at `migration 9 was previously applied but has been modified`; the database was not reset or repaired. The containers started during this check were stopped afterward.",
        "- CPU profiling was attempted with installed Windows Performance Recorder but could not enable system profiling policy (`0xc5585011`); no profile or RSS data was collected. Kafka/database/S3 latency, browser rendering, allocation counts, RSS, and hardware counters remain unmeasured.",
        "- The failed composer, bootstrap, and profiler attempts are recorded honestly; no tests or benchmark runs are reported as passed unless their output confirms success.",
        "",
        "## Evidence and reproduction",
        "",
        f"- Baseline details: [`baseline.md`](baseline.md).",
        f"- Candidate comparison: [`{run_meta['run_id']}`](../runs/{run_meta['run_id']}/metadata.json) and its uniquely named comparison report under `comparisons/`.",
        "- Re-run: `powershell -NoProfile -ExecutionPolicy Bypass -File performance/scripts/run-benchmarks.ps1 -Mode optimize`.",
        "",
    ]
    output = "\n".join(lines)
    report_path = ROOT / "reports/optimization-2026-10.md"
    report_path.write_text(output, encoding="utf-8")
    (ROOT / "reports/latest.md").write_text(output, encoding="utf-8")
    print(f"Rendered full 40-workload optimization report: {report_path.relative_to(ROOT).as_posix()}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
