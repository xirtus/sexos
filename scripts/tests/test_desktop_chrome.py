"""Compile the production shared geometry against independent pixel goldens."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class SharedChromeTests(unittest.TestCase):
    def test_production_metrics_bounds_and_scale_steps(self):
        with tempfile.TemporaryDirectory(prefix="sexos-chrome-") as directory:
            binary = str(Path(directory) / "tests")
            subprocess.run(["rustc", "--edition=2021", "--test",
                            str(ROOT / "tests/desktop_chrome.rs"), "-o", binary],
                           check=True, capture_output=True, timeout=60)
            subprocess.run([binary], check=True, capture_output=True, timeout=30)
