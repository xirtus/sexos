#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Unit tests for scripts/ipc_perf_regression_gate.py."""

from __future__ import annotations

import io
import json
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

SCRIPTS_DIR = Path(__file__).resolve().parent.parent
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

import ipc_perf_regression_gate as gate  # noqa: E402


SAMPLE_PASS_LOG = """
[boot] ready
[bench] name=null_syscall ops=20000 cycles=120 per=call
[bench] name=ipc_roundtrip ops=2000 cycles=1850 per=trip
[bench] name=ipc2_roundtrip ops=2000 cycles=1400 per=trip
[app.launch.hops] hops=1
"""


class TestIpcPerfRegressionGate(unittest.TestCase):
    def test_parse_bench_log_extracts_metrics_and_hops(self) -> None:
        samples = gate.parse_bench_log(SAMPLE_PASS_LOG)
        self.assertEqual(samples["null_syscall"], [120.0])
        self.assertEqual(samples["ipc_roundtrip"], [1850.0])
        self.assertEqual(samples["ipc2_roundtrip"], [1400.0])
        self.assertEqual(samples["app_launch_hops"], [1.0])

    def test_evaluate_budgets_passes_within_limits(self) -> None:
        samples = gate.parse_bench_log(SAMPLE_PASS_LOG)
        passed, rows = gate.evaluate_budgets(
            samples,
            gate.DEFAULT_BUDGETS,
            required=("null_syscall", "ipc_roundtrip", "ipc2_roundtrip", "app_launch_hops"),
        )
        self.assertTrue(passed)
        self.assertTrue(all(r["status"] == "PASS" for r in rows))

    def test_evaluate_budgets_fails_on_ipc_regression(self) -> None:
        log = SAMPLE_PASS_LOG + "\n[bench] name=ipc_roundtrip ops=2000 cycles=9999 per=trip\n[bench] name=ipc_roundtrip ops=2000 cycles=8888 per=trip\n"
        samples = gate.parse_bench_log(log)
        passed, rows = gate.evaluate_budgets(samples, gate.DEFAULT_BUDGETS)
        self.assertFalse(passed)
        by_name = {r["metric"]: r for r in rows}
        self.assertEqual(by_name["ipc_roundtrip"]["status"], "FAIL")
        self.assertEqual(by_name["ipc_roundtrip"]["median"], 8888.0)

    def test_evaluate_budgets_fails_on_hop_regression(self) -> None:
        log = "[app.launch.hops] hops=4\n"
        samples = gate.parse_bench_log(log)
        passed, rows = gate.evaluate_budgets(samples, gate.DEFAULT_BUDGETS)
        self.assertFalse(passed)
        self.assertEqual(rows[0]["metric"], "app_launch_hops")
        self.assertEqual(rows[0]["status"], "FAIL")

    def test_evaluate_budgets_fails_on_missing_required_metric(self) -> None:
        samples = gate.parse_bench_log("[bench] name=null_syscall ops=20000 cycles=110 per=call\n")
        passed, rows = gate.evaluate_budgets(
            samples,
            gate.DEFAULT_BUDGETS,
            required=("null_syscall", "ipc_roundtrip"),
        )
        self.assertFalse(passed)
        by_name = {r["metric"]: r for r in rows}
        self.assertEqual(by_name["ipc_roundtrip"]["status"], "MISSING")

    def test_cli_json_and_budget_override(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            log_path = Path(td) / "bench.log"
            log_path.write_text(SAMPLE_PASS_LOG, encoding="utf-8")

            buf = io.StringIO()
            with redirect_stdout(buf):
                rc = gate.main([str(log_path), "--require", "ipc_roundtrip", "--json"])
            self.assertEqual(rc, 0)
            payload = json.loads(buf.getvalue())
            self.assertTrue(payload["passed"])

            buf2 = io.StringIO()
            with redirect_stdout(buf2):
                rc2 = gate.main([str(log_path), "--budget", "ipc_roundtrip=1000", "--json"])
            self.assertEqual(rc2, 1)
            payload2 = json.loads(buf2.getvalue())
            self.assertFalse(payload2["passed"])


if __name__ == "__main__":
    unittest.main()
