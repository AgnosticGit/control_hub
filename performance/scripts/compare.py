#!/usr/bin/env python3
"""Compare compatible normalized performance summaries."""
import argparse, datetime, json, pathlib, sys

ROOT = pathlib.Path(__file__).resolve().parents[1]

def load_run(run_id: str) -> tuple[dict, dict]:
    directory = ROOT / "runs" / run_id
    return json.loads((directory / "metadata.json").read_text(encoding="utf-8-sig")), json.loads((directory / "results/summary.json").read_text(encoding="utf-8-sig"))

def pct(value: object) -> str:
    return f"{value:.2f}%" if isinstance(value, (int, float)) else "n/a"

def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("before", help="run ID or baseline:initial")
    parser.add_argument("after", help="run ID")
    args = parser.parse_args()
    try:
        if args.before.startswith("baseline:"):
            baseline_name = args.before.split(":", 1)[1]
            base = ROOT / "baseline" / baseline_name
            left_meta = json.loads((base / "metadata.json").read_text(encoding="utf-8-sig"))
            left_summary = json.loads((base / "results/summary.json").read_text(encoding="utf-8-sig"))
        else: left_meta, left_summary = load_run(args.before)
        right_meta, right_summary = load_run(args.after)
    except (OSError, json.JSONDecodeError) as exc:
        print(f"Cannot load comparison input: {exc}", file=sys.stderr); return 2
    if left_meta.get("compatibility_key") != right_meta.get("compatibility_key"):
        print("REFUSED: compatibility keys differ", file=sys.stderr); return 3
    if left_meta.get("workloads") != right_meta.get("workloads"):
        print("REFUSED: workload IDs or versions differ", file=sys.stderr); return 3
    if left_meta.get("workload_source_sha256") != right_meta.get("workload_source_sha256"):
        print("REFUSED: workload source/fixture hashes differ", file=sys.stderr); return 3
    left = {m["workload_id"]: m for m in left_summary["metrics"]}
    right = {m["workload_id"]: m for m in right_summary["metrics"]}
    common = sorted(left.keys() & right.keys())
    if not common or left.keys() != right.keys():
        print("REFUSED: workload sets differ or contain no comparable workloads", file=sys.stderr); return 3
    rows = []
    for workload in common:
        a, b = left[workload], right[workload]
        if (a.get("metric"), a.get("unit"), a.get("direction")) != (b.get("metric"), b.get("unit"), b.get("direction")):
            print(f"REFUSED: metric schema differs for {workload}", file=sys.stderr); return 3
        before, after = a["mean_ns"], b["mean_ns"]
        rows.append((workload, before, after, (before-after)/before*100 if before else None, a.get("std_dev_ns"), b.get("std_dev_ns")))
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    name = f"comparison_{stamp}_{args.before.replace(':','-')}_{args.after}.md"
    report = ROOT / "reports/comparisons" / name
    text = [f"# Performance comparison: {args.before} → {args.after}", "", f"Compatibility key: `{right_meta.get('compatibility_key')}`", "", "| Workload | Before mean (ns) | After mean (ns) | Change | Before within-run SD | After within-run SD | Before run range | After run range |", "|---|---:|---:|---:|---:|---:|---:|---:|"]
    for workload, before, after, change, std_a, std_b in rows:
        a, b = left[workload], right[workload]
        range_a = a.get("between_run_range_pct")
        range_b = b.get("between_run_range_pct")
        change_text = f"{change:+.2f}%" if change is not None else "n/a"
        text.append(f"| {workload} | {before:.3f} | {after:.3f} | {change_text} | {std_a:.3f} | {std_b:.3f} | {pct(range_a)} | {pct(range_b)} |")
    text += ["", "Generated from stored Criterion summaries. Variation is represented by Criterion's estimated standard deviation; no significance claim is made by this report.", ""]
    report.write_text("\n".join(text), encoding="utf-8")
    (ROOT / "reports/latest.md").write_text("\n".join(text), encoding="utf-8")
    print(report.relative_to(ROOT).as_posix()); return 0

if __name__ == "__main__": sys.exit(main())
