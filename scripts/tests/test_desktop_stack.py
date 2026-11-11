"""Actual Shell input adapters and shared user-space stack policy; no guest proof."""
from pathlib import Path
import os
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


def function(source, name):
    match = re.search(r"(?m)^(?:unsafe )?fn " + re.escape(name) + r"\(", source)
    if not match:
        raise AssertionError("missing actual function " + name)
    start = match.start()
    end = source.index("{", match.end()) + 1
    depth = 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end]


def wiring(source):
    for name in ("hit_test_at", "clear_focus_if_dead"):
        body = function(source, name)
        assert "sex_pdx::desktop_stack::ORDER" in body, name
        assert "z_order" not in body, name
    hit = function(source, "hit_test_at")
    assert "input_focus_surface(FOCUSED_SURFACE_ID)" in hit
    assert "for entry in APP_CUSTODY" not in hit
    assert "point_in_surface(x, y, sid)" in hit
    point = function(source, "point_in_surface")
    assert point.index("input_surface_eligible(sid)") < point.index("get_surface_bounds(sid)")
    assert "input_surface_eligible(input_focus_surface(focused))" in function(source, "clear_focus_if_dead")


class SharedStackTests(unittest.TestCase):
    def setUp(self):
        self.source = (ROOT / "servers/silk-shell/src/main.rs").read_text()
        self.actual = "\n".join(function(self.source, name) for name in
                                ("input_surface_eligible", "input_focus_surface"))

    def compile_run(self, directory, name, actual, exact=None):
        module = Path(directory) / (name + ".rs")
        module.write_text(actual)
        binary = Path(directory) / name
        env = dict(os.environ, SHELL_STACK_SOURCE=str(module))
        subprocess.run(["rustc", "--edition=2021", "--test",
                        str(ROOT / "tests/desktop_stack.rs"), "-o", str(binary)],
                       env=env, check=True, capture_output=True, timeout=60)
        args = [str(binary), "--test-threads=1"]
        if exact:
            args += ["--exact", exact]
        return subprocess.run(args, capture_output=True, text=True, timeout=30)

    def test_actual_adapters_rank_and_live_input_facts(self):
        wiring(self.source)
        with tempfile.TemporaryDirectory(prefix="sexos-stack-") as directory:
            result = self.compile_run(directory, "actual", self.actual)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_initial_input_wiring_rejects_private_fallback_orders(self):
        for name in ("hit_test_at", "clear_focus_if_dead"):
            body = function(self.source, name)
            changed = body.replace("sex_pdx::desktop_stack::ORDER", "PRIVATE_Z_ORDER")
            self.assertNotEqual(changed, body)
            with self.assertRaises(AssertionError):
                wiring(self.source.replace(body, changed, 1))

    def test_actual_adapters_reject_four_compiled_counterexamples(self):
        cases = [
            ("placeholder-owned", "standalone_owned:is_shell_surface(sid)",
             "standalone_owned:true",
             "actual_shell_wrapper_refuses_uncreated_and_inactive_tabs_without_erasing_owned_path"),
            ("inactive-tab", "active_tab:active_surface_for_frame(frame)==Some(sid)",
             "active_tab:true",
             "actual_shell_wrapper_refuses_uncreated_and_inactive_tabs_without_erasing_owned_path"),
            ("dead-alias", "!entry.dead && !entry.close_pending &&", "",
             "actual_focus_alias_needs_exact_live_custody_and_real_bound_parent"),
            ("unbound-parent", "frame_for_surface(parent).is_some()", "true",
             "actual_focus_alias_needs_exact_live_custody_and_real_bound_parent"),
        ]
        with tempfile.TemporaryDirectory(prefix="sexos-stack-mutants-") as directory:
            for name, old, new, exact in cases:
                self.assertEqual(self.actual.count(old), 1, name)
                result = self.compile_run(directory, name, self.actual.replace(old, new), exact)
                self.assertNotEqual(result.returncode, 0, name)
                self.assertIn("FAILED", result.stdout, name)
