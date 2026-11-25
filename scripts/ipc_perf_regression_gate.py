#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""CI performance regression gate for SexOS IPC and app-launch bench logs.

Parses serial output emitted by `apps/bench`:
  [bench] name=<metric> ops=<n> cycles=<per_op> per=<unit>
and optional app-launch hop markers:
  [app.launch.hops] hops=<n>

Fails with exit code 1 if any observed metric exceeds its configured budget
or if any required metric is missing.
"""

from __future__ import annotations

import argparse
import json
import re
import statistics
import sys
from pathlib import Path
from typing import Dict, List, Optional, Sequence, Tuple

DEFAULT_BUDGETS: Dict[str, float] = {
    "null_syscall": 500.0,
    "ipc_roundtrip": 4500.0,
    "ipc2_roundtrip": 3500.0,
    "app_launch_hops": 1.0,
}

BENCH_LINE_RE = re.compile(
    r"\[bench\]\s+name=(?P<name>[A-Za-z0-9_.-]+)\s+ops=(?P<ops>\d+)\s+cycles=(?P<cycles>\d+)(?:\s+per=(?P<per>[A-Za-z0-9_./-]+))?"
)
LAUNCH_HOPS_RE = re.compile(r"\[app\.launch\.hops\]\s+hops=(?P<hops>\d+)")


def parse_bench_log(text: str) -> Dict[str, List[float]]:
    """Extract per-metric sample lists from serial log text."""
    samples: Dict[str, List[float]] = {}
    for line in text.splitlines():
        m = BENCH_LINE_RE.search(line)
        if m:
            name = m.group("name")
            cycles = float(int(m.group("cycles")))
            samples.setdefault(name, []).append(cycles)
            continue
        h = LAUNCH_HOPS_RE.search(line)
        if h:
            hops = float(int(h.group("hops")))
            samples.setdefault("app_launch_hops", []).append(hops)
    return samples


def evaluate_budgets(
    samples: Dict[str, List[float]],
    budgets: Dict[str, float],
    required: Sequence[str] = (),
) -> Tuple[bool, List[Dict[str, object]]]:
    """Compare median observed values against budgets. Returns (passed, rows)."""
    passed = True
    rows: List[Dict[str, object]] = []
    seen_keys = set(samples.keys()) | set(required)

    for metric in sorted(seen_keys):
        vals = samples.get(metric)
        budget = budgets.get(metric)
        if not vals:
            passed = False
            rows.append(
                {
                    "metric": metric,
                    "median": None,
                    "budget": budget,
                    "samples": 0,
                    "status": "MISSING",
                }
            )
            continue

        med = float(statistics.median(vals))
        if budget is not None and med > budget:
            passed = False
            status = "FAIL"
        else:
            status = "PASS"
        rows.append(
            {
                "metric": metric,
                "median": med,
                "budget": budget,
                "samples": len(vals),
                "status": status,
            }
        )

    return passed, rows


def parse_budget_overrides(items: Sequence[str]) -> Dict[str, float]:
    out: Dict[str, float] = {}
    for item in items:
        if "=" not in item:
            raise ValueError(f"invalid --budget '{item}', expected metric=max_value")
        k, v = item.split("=", 1)
        k = k.strip()
        if not k:
            raise ValueError(f"empty metric name in --budget '{item}'")
        out[k] = float(v.strip())
    return out


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="SexOS IPC performance regression CI gate")
    parser.add_argument("log", type=Path, help="Path to serial log containing [bench] lines")
    parser.add_argument(
        "--budget",
        action="append",
        default=[],
        help="Override or add budget as metric=max_value (repeatable)",
    )
    parser.add_argument(
        "--require",
        action="append",
        default=[],
        help="Require metric to be present in the log (repeatable)",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        dest="as_json",
        help="Emit machine-readable JSON summary",
    )
    args = parser.parse_args(argv)

    budgets = dict(DEFAULT_BUDGETS)
    try:
        budgets.update(parse_budget_overrides(args.budget))
    except ValueError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2

    if not args.log.is_file():
        print(f"error: log file not found: {args.log}", file=sys.stderr)
        return 2

    text = args.log.read_text(encoding="utf-8", errors="replace")
    samples = parse_bench_log(text)
    passed, rows = evaluate_budgets(samples, budgets, required=args.require)

    if args.as_json:
        print(json.dumps({"passed": passed, "results": rows}, indent=2))
    else:
        for row in rows:
            med = "missing" if row["median"] is None else f"{row['median']:.1f}"
            bud = "none" if row["budget"] is None else f"{row['budget']:.1f}"
            print(
                f"[{row['status']}] {row['metric']}: median={med} budget={bud} samples={row['samples']}"
            )
        print("[ipc_perf_regression_gate] " + ("PASSED" if passed else "FAILED"))

    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
