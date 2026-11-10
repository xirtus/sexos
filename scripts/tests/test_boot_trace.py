"""Compile the production boot tracing macro with each supported build setting."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class BootTraceTests(unittest.TestCase):
    def test_opt_in_does_not_evaluate_quiet_arguments(self):
        serial = (ROOT / "kernel/src/serial.rs").read_text()
        macro = serial[serial.index("#[macro_export]\nmacro_rules! boot_trace"):]
        source = r'''
#[macro_export]
macro_rules! serial_println { ($($arg:tt)*) => { println!($($arg)*); }; }
''' + macro + r'''
fn main() {
    let mut calls = 0;
    boot_trace!("detail={}", { calls += 1; calls });
    // Faults and service tags remain on the ordinary logging path.
    serial_println!("[wifi.failed] step=init");
    serial_println!("[sexnet.ready]");
    println!("calls={calls}");
}
'''
        with tempfile.TemporaryDirectory(prefix="sexos-boot-trace-") as directory:
            path = Path(directory)
            (path / "probe.rs").write_text(source)
            for setting in (None, "0", "1"):
                with self.subTest(setting=setting):
                    env = dict(os.environ)
                    env.pop("SEXOS_VERBOSE_BOOT", None)
                    if setting is not None:
                        env["SEXOS_VERBOSE_BOOT"] = setting
                    subprocess.run(["rustc", "--edition=2021", "-A", "unused_mut",
                                    str(path / "probe.rs"), "-o", str(path / "probe")],
                                   env=env, check=True, capture_output=True, timeout=60)
                    output = subprocess.check_output([str(path / "probe")], text=True)
                    prefix = "detail=1\n" if setting == "1" else ""
                    self.assertEqual(output, prefix + "[wifi.failed] step=init\n"
                                     + "[sexnet.ready]\ncalls="
                                     + ("1\n" if setting == "1" else "0\n"))
