"""Exercise the real Bash boot-wait helpers; no QEMU or fabricated guest proof."""
from pathlib import Path
import json
import socket
import subprocess
import tempfile
import threading
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
SOURCE = (ROOT / "scripts/usb_path_gate.sh").read_text()
HELPERS = SOURCE[SOURCE.index("has() {"):SOURCE.index("boot_lane() {")]
SENDKEY = SOURCE[SOURCE.index("qmp_sendkey() {"):SOURCE.index("# Hard faults")]
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


class QmpKeyReader(unittest.TestCase):
    def exchange(self, mode, budget="2"):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "qmp.sock"
            server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            server.bind(str(path))
            server.listen(1)
            server.settimeout(3)
            commands, errors = [], []

            def serve():
                try:
                    with server.accept()[0] as conn:
                        conn.settimeout(3)
                        io = conn.makefile("rwb", buffering=0)
                        io.write(b'{"QMP":{"version":{}}}\n')
                        cap = json.loads(io.readline())
                        commands.append(cap)
                        if mode == "close":
                            return
                        # Deliberately coalesce events/replies; the next command
                        # must not discard a buffered event or accept it as ACK.
                        io.write((json.dumps({"event": "STOP"}) + "\n" +
                                  json.dumps({"return": {}, "id": cap["id"]}) + "\n" +
                                  json.dumps({"event": "RESUME"}) + "\n").encode())
                        key = json.loads(io.readline())
                        commands.append(key)
                        if mode == "silent":
                            threading.Event().wait(.6)
                            return
                        reply = {"error": {"class": "GenericError"}} if mode == "error" else {"return": {}}
                        reply["id"] = key["id"]
                        io.write((json.dumps(reply) + "\n").encode())
                except Exception as exc:
                    errors.append(str(exc))

            thread = threading.Thread(target=serve, daemon=True)
            thread.start()
            start = time.monotonic()
            try:
                result = subprocess.run(["/bin/bash", "-euc", SENDKEY + '\nqmp_sendkey "$1" ret "$2"\n',
                                         "qmp-key-test", str(path), budget], capture_output=True,
                                        text=True, timeout=3)
                elapsed = time.monotonic() - start
            finally:
                thread.join(timeout=3)
                server.close()
            self.assertFalse(thread.is_alive(), "fixture server did not stop")
            self.assertEqual(errors, [])
            return result, commands, elapsed

    def test_eof_fails_instead_of_spinning(self):
        result, commands, elapsed = self.exchange("close")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("QMP closed", result.stderr)
        self.assertEqual(len(commands), 1)
        self.assertLess(elapsed, 2)

    def test_coalesced_events_require_matching_command_ack(self):
        result, commands, _ = self.exchange("ok")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([c["execute"] for c in commands], ["qmp_capabilities", "send-key"])
        self.assertEqual(commands[1]["arguments"]["keys"], [{"type": "qcode", "data": "ret"}])
        self.assertNotEqual(commands[0]["id"], commands[1]["id"])

    def test_command_error_is_not_key_delivery(self):
        result, _, _ = self.exchange("error")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("QMP key command rejected", result.stderr)

    def test_missing_ack_obeys_shared_deadline(self):
        result, _, elapsed = self.exchange("silent", ".15")
        self.assertNotEqual(result.returncode, 0)
        self.assertLess(elapsed, 2)


if __name__ == "__main__":
    unittest.main()
