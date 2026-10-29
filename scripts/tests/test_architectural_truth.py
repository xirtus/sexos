"""Negative regressions for the architectural obligation/evidence inventory."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest import mock

REPO = Path(__file__).resolve().parents[2]
IMPORT = importlib.util.spec_from_file_location("architectural_truth", REPO / "scripts/check_architectural_truth.py")
truth = importlib.util.module_from_spec(IMPORT)
IMPORT.loader.exec_module(truth)


class ArchitecturalTruthTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="sexos-truth-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "repo"
        (self.root / "spec").mkdir(parents=True)
        paths = {"ARCHITECTURE.md", "scripts/check_architectural_truth.py"}
        for spec in (REPO / "spec").glob("*.sexspec"):
            shutil.copyfile(spec, self.root / "spec" / spec.name)
            doc = json.loads(spec.read_text())
            paths.update(source["path"] for item in doc["invariants"] for source in item["sources"])
            paths.update(check["argv"][1] for check in doc.get("checks", []))
        for name in paths:
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(REPO / name, target)

    def edit(self, name, change):
        path = self.root / "spec" / f"{name}.sexspec"
        data = json.loads(path.read_text())
        change(data)
        path.write_text(json.dumps(data))

    def invalid(self, message):
        with self.assertRaisesRegex(truth.InvalidSpec, message):
            truth.load_specs(self.root)

    def test_real_inventory_is_valid_but_does_not_claim_proof(self):
        docs, checks, invariants, _ = truth.load_specs(self.root)
        self.assertEqual(set(docs), truth.SUBSYSTEMS)
        self.assertGreaterEqual(len(invariants), 10)
        self.assertEqual({item["status"] for item in invariants.values()}, {"open", "partial"})
        self.assertIn("runtime", {check["kind"] for check in checks.values()})

    def test_constitution_drift_requires_review(self):
        with (self.root / "ARCHITECTURE.md").open("a") as stream:
            stream.write("\nChanged constitution.\n")
        self.invalid("constitution changed")

    def test_every_constitutional_obligation_is_required(self):
        path = self.root / "spec/architecture.sexspec"
        baseline = path.read_text()
        for identity in truth.CORE_INVARIANTS:
            with self.subTest(identity=identity):
                path.write_text(baseline)
                self.edit("architecture", lambda d: d.update(invariants=[i for i in d["invariants"] if i["id"] != identity]))
                self.invalid("missing constitutional obligations")

    def test_unsupported_proof_claim_is_rejected(self):
        self.edit("architecture", lambda d: d["invariants"][0].update(status="proven"))
        self.invalid("unsupported proof claim")

    def test_gaps_cannot_be_hidden(self):
        self.edit("pdx", lambda d: d["invariants"][0].update(gaps=[]))
        self.invalid("gaps: empty")

    def test_status_must_match_scoped_check_coverage(self):
        self.edit("pdx", lambda d: d["invariants"][0].update(status="open"))
        self.invalid("coverage mismatch")

    def test_stale_anchor_fails(self):
        self.edit("scheduler", lambda d: d["invariants"][0]["sources"][0].update(anchor="MISSING_PRODUCTION_SYMBOL"))
        self.invalid("missing source anchor")

    def test_missing_source_fails(self):
        (self.root / "kernel/src/scheduler.rs").unlink()
        self.invalid("missing file")

    def test_path_escape_fails(self):
        self.edit("scheduler", lambda d: d["invariants"][0]["sources"][0].update(path="../outside.rs"))
        self.invalid("repository relative")

    def test_symlink_escape_fails(self):
        outside = Path(self.temp.name) / "outside.rs"
        outside.write_text("pub fn tick")
        target = self.root / "kernel/src/scheduler.rs"
        target.unlink()
        target.symlink_to(outside)
        self.invalid("escapes repository")

    def test_missing_subsystem_fails(self):
        (self.root / "spec/uiir.sexspec").unlink()
        self.invalid("exactly six")

    def test_unknown_subsystem_fails(self):
        (self.root / "spec/unknown.sexspec").write_text("{}")
        self.invalid("exactly six")

    def test_unknown_check_fails(self):
        self.edit("pdx", lambda d: d["invariants"][0]["checks"].append("invented_check"))
        self.invalid("unknown check")

    def test_unused_check_fails(self):
        def add(doc):
            check = dict(doc["checks"][0])
            check["id"] = "unbound_check"
            doc["checks"].append(check)
        self.edit("architecture", add)
        self.invalid("orphaned check")

    def test_shell_expression_is_not_a_gate_argv(self):
        self.edit("architecture", lambda d: d["checks"][0].update(argv=["bash", "-c", "exit 0"]))
        self.invalid("literal bash script")

    def test_check_ids_cannot_escape_log_directory(self):
        path = self.root / "spec/architecture.sexspec"
        baseline = path.read_text()
        for identity in ("../escaped", "/tmp/absolute", "nested/file", "", "bad-id", "white space"):
            with self.subTest(identity=identity):
                path.write_text(baseline)
                self.edit("architecture", lambda d: d["checks"][0].update(id=identity))
                self.invalid("check ID")

    def test_dependency_must_exist(self):
        self.edit("pdx", lambda d: d["invariants"][0]["depends_on"].append("UNKNOWN_OBLIGATION"))
        self.invalid("invalid dependency")

    def test_dependency_cycle_fails(self):
        self.edit("architecture", lambda d: d["invariants"][1]["depends_on"].append("PDX_SLOT_IDENTITY"))
        self.invalid("dependency cycle")

    def test_duplicate_invariant_fails(self):
        self.edit("pdx", lambda d: d["invariants"].append(d["invariants"][0]))
        self.invalid("duplicate invariant")

    def test_duplicate_json_field_fails(self):
        path = self.root / "spec/pdx.sexspec"
        path.write_text(path.read_text().replace('"schema_version": 1', '"schema_version": 1, "schema_version": 1'))
        self.invalid("duplicate JSON field")

    def test_unknown_fields_fail(self):
        self.edit("pdx", lambda d: d.update(silently_ignored=True))
        self.invalid("wrong fields")

    def test_transition_requires_forbidden_outcomes(self):
        self.edit("pdx", lambda d: d["transitions"][0].update(forbidden=[]))
        self.invalid("forbidden: empty")

    def test_epoch_phase_order_is_pinned(self):
        self.edit("architecture", lambda d: d["state_model"].update(boundary=["commit", "swap", "transform", "snapshot"]))
        self.invalid("boundary order changed")

    def test_host_runner_reports_failure_and_never_runs_runtime(self):
        _, checks, _, _ = truth.load_specs(self.root)
        for check in checks.values():
            # Deliberately failing runtime gates must not run with --run-host.
            code = 19 if check["kind"] == "runtime" else 0
            (self.root / check["argv"][1]).write_text(f"exit {code}\n")
        (self.root / checks["scheduler_queue"]["argv"][1]).write_text("exit 7\n")
        evidence = Path(self.temp.name) / "evidence"
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            status = truth.main(["--root", str(self.root), "--run-host", "--evidence-dir", str(evidence)])
        self.assertEqual(status, 1)
        result = json.loads((evidence / "report.json").read_text())
        self.assertFalse(result["system_proven"])
        self.assertFalse(result["runtime_checks_executed"])
        self.assertTrue(all(row["kind"] != "runtime" for row in result["checks"]))
        self.assertEqual(next(row for row in result["checks"] if row["id"] == "scheduler_queue")["exit_code"], 7)
        self.assertIn("kernel/src/scheduler.rs", result["source_sha256"])

    def test_interrupted_run_cannot_reuse_prior_success_report(self):
        evidence = Path(self.temp.name) / "evidence"
        evidence.mkdir()
        (evidence / "report.json").write_text('{"run_status":"complete","checks_passed":true}')
        with mock.patch.object(truth.subprocess, "run", side_effect=KeyboardInterrupt):
            with self.assertRaises(KeyboardInterrupt):
                truth.main(["--root", str(self.root), "--run-host", "--evidence-dir", str(evidence)])
        result = json.loads((evidence / "report.json").read_text())
        self.assertEqual(result["run_status"], "running")
        self.assertNotIn("checks_passed", result)
        self.assertEqual(result["checks"], [])

    def test_check_launch_failure_is_retained_and_fails_run(self):
        evidence = Path(self.temp.name) / "evidence"
        with mock.patch.object(truth.subprocess, "run", side_effect=OSError("test launch failure")):
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                status = truth.main(["--root", str(self.root), "--run-host", "--evidence-dir", str(evidence)])
        self.assertEqual(status, 1)
        result = json.loads((evidence / "report.json").read_text())
        self.assertFalse(result["checks_passed"])
        self.assertTrue(all(row["exit_code"] == 127 for row in result["checks"]))


if __name__ == "__main__":
    unittest.main()
