"""Production SDK counter codec and reset semantics against literal wire records."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]

class ReadIoTests(unittest.TestCase):
    def test_wire_and_reset(self):
        with tempfile.TemporaryDirectory(prefix='sexos-readio-') as out:
            binary = str(Path(out) / 'tests')
            subprocess.run(['rustc', '--edition=2021', '--test', str(ROOT/'tests/fs_read_io.rs'), '-o', binary], check=True, cwd=ROOT)
            subprocess.run([binary], check=True)
