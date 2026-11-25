"""Keep the long memory proof opt-in for manual app CI runs."""

import pathlib
import re
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
WORKFLOW = (ROOT / ".github/workflows/ci.yml").read_text()


class MemoryScheduleTests(unittest.TestCase):
    def test_manual_extended_memory_is_explicit_and_off_by_default(self):
        dispatch = WORKFLOW.split("  workflow_dispatch:\n", 1)[1].split("\nconcurrency:", 1)[0]
        self.assertRegex(dispatch, r"(?m)^      extended_memory:\n")
        self.assertRegex(dispatch, r"(?m)^        type: boolean$")
        self.assertRegex(dispatch, r"(?m)^        default: false$")

    def test_ordinary_manual_run_keeps_short_memory_proof(self):
        job = WORKFLOW.split("  denim-double:\n", 1)[1].split("\n  programs:", 1)[0]
        self.assertIn("- name: Program heap, lazy stacks and the stack guard page", job)
        self.assertIn("- name: Heap over twice the RAM at 1 GiB\n      run:", job)
        predicate = re.search(r"(?m)^    - name: Heap over twice the RAM at 12 GiB\n      if: (.+)$", job)
        self.assertIsNotNone(predicate)
        expected = "github.event_name == 'schedule' || (github.event_name == 'workflow_dispatch' && inputs.extended_memory == true)"
        self.assertEqual(predicate.group(1), expected)
        self.assertIn(f"timeout-minutes: ${{{{ ({expected}) && 355 || 60 }}}}", job)


if __name__ == "__main__":
    unittest.main()
