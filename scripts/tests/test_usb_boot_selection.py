"""Exercise the real Bash boot-wait helpers; no QEMU or fabricated guest proof."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SOURCE = (ROOT / "scripts/usb_path_gate.sh").read_text()
HELPERS = SOURCE[SOURCE.index("has() {"):SOURCE.index("boot_lane() {")]
MENU = "Limine 7.13.3 (x86-64, BIOS)\nBooting automatically in 1, press any key to stop the countdown...\n"


class UsbBootSelection(unittest.TestCase):
    def shell(self, program, *args):
        return subprocess.run(["/bin/bash", "-euc", HELPERS + program, "boot-wait-test", *args],
                              capture_output=True, text=True, timeout=12)

    def test_only_unbooted_limine_menu_qualifies(self):
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "serial.log"
            for text, expected in [(MENU, 0), ("", 1), ("Booting automatically in 1", 1),
                                   (MENU + "limine: Loading kernel `boot:///sexos-kernel`...\n", 1),
                                   (MENU + "[boot.kernel_entry] tsc=123\n", 1)]:
                with self.subTest(text=text):
                    log.write_text(text)
                    result = self.shell('\nboot_menu_waiting "$1"\n', str(log))
                    self.assertEqual(result.returncode, expected, result.stderr)

    def test_ready_guest_does_not_receive_boot_key(self):
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "serial.log"
            log.write_text(MENU + "[boot.kernel_entry] tsc=123\nREADY\n")
            result = self.shell('\nqmp_sendkey() { exit 99; }; wait_marker READY "$1" 1 socket\n', str(log))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(Path(str(log) + ".boot-actions.log").exists())

    def test_menu_selection_once_then_requires_actual_ready_marker(self):
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "serial.log"
            calls = Path(tmp) / "calls"
            log.write_text(MENU)
            result = self.shell('''
log="$1"; calls="$2"
qmp_sendkey() {
  printf '%s %s\n' "$1" "$2" >> "$calls"
  printf 'limine: Loading kernel\n[boot.kernel_entry] tsc=123\nREADY\n' >> "$log"
}
wait_marker READY "$log" 8 socket
''', str(log), str(calls))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(calls.read_text(), "socket ret\n")
            self.assertEqual(Path(str(log) + ".boot-actions.log").read_text().count("select_default"), 1)

    def test_loading_guest_and_missing_menu_still_time_out(self):
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "serial.log"
            for text, timeout in [("", 1), (MENU + "limine: Loading kernel\n", 5),
                                  (MENU + "[boot.kernel_entry] tsc=123\n", 5)]:
                with self.subTest(text=text):
                    log.write_text(text)
                    result = self.shell('\nqmp_sendkey() { exit 99; }; wait_marker READY "$1" "$2" socket\n', str(log), str(timeout))
                    self.assertEqual(result.returncode, 1, result.stderr)

    def test_key_delivery_failure_is_not_readiness(self):
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "serial.log"
            log.write_text(MENU)
            result = self.shell('\nqmp_sendkey() { return 1; }; wait_marker READY "$1" 8 socket\n', str(log))
            self.assertEqual(result.returncode, 1, result.stderr)

    def test_successful_key_reply_without_guest_readiness_still_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            log, calls = Path(tmp) / "serial.log", Path(tmp) / "calls"
            log.write_text(MENU)
            result = self.shell('''
calls="$2"
qmp_sendkey() { printf 'attempt\n' >> "$calls"; }
wait_marker READY "$1" 6 socket
''', str(log), str(calls))
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertEqual(calls.read_text(), "attempt\n")


if __name__ == "__main__":
    unittest.main()
