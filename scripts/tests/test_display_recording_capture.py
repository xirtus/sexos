"""Production Display state machine with explicit synthetic syscall fixtures."""
import os
import pathlib
import subprocess
import tempfile
import unittest
ROOT = pathlib.Path(__file__).resolve().parents[2]
class DisplayRecording(unittest.TestCase):
    def test_actual_server(self):
        with tempfile.TemporaryDirectory(prefix="sexos-display-recording-") as tmp:
            binary = pathlib.Path(tmp) / "capture-tests"
            env = dict(os.environ, SEXOS_RECORDER="1")
            subprocess.run(["rustc", "--edition=2021", "--test", str(ROOT / "tests/display_recording_capture.rs"), "-o", str(binary)], check=True, env=env)
            subprocess.run([str(binary), "--test-threads=1"], check=True)
if __name__ == "__main__":
    unittest.main()
