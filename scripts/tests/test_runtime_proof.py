#!/usr/bin/env python3
"""Host-only runtime-proof regressions; no image build or real QEMU required."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import check_bootgraph_log as checker
import check_disk_persistence as disk_checker


def good_log():
    lines = ["[bootgraph.phase25.begin]"]
    lines += [f"[bootgraph.cap.grant {marker}]" for marker in checker.REQUIRED_A2_GRANTS_OK1]
    lines += ["[bootgraph.phase25.complete]"]
    for pd, token in checker.REQUIRED_PD_MARKERS:
        lines += [f"[{token}.init.start]", f"[{token}.ready]"]
    lines += ["[silk-shell.ready.notify] to=sexinput", "[sexinput.shell.ready.accept] caller=3"]
    for sender, receiver in sorted(checker.REQUIRED_STARTUP_EDGES):
        lines += [f"[bootgraph.edge.send from={sender} to={receiver} slot=5 op=UPDATE first=1]"]
    lines += ["[silkbar.clock.send]", "[silkbar.clock.send]", "[sexdisplay.clock.recv]",
              "[sexdisplay.clock.redraw]", "[sexdisplay.render.live.ok]"]
    for i, pd in [(1, "sexdisplay"), (2, "sexdrive"), (3, "silk-shell"),
                  (4, "sexinput"), (6, "silkbar"), (7, "linen"), (11, "sexfiles"), (14, "sext")]:
        lines += [f"Spawned PD {pd}", f"task.running pd_id={i}"]
    return lines


# Exact prefixes emitted by kernel/src/interrupts.rs, main.rs, lib.rs, and
# the AArch64 panic path. Synthetic addresses keep these host-only fixtures.
ACTUAL_FAULT_LINES = [
    "EXCEPTION: PAGE FAULT at 0x1234 (RIP: 0x5678, RSP: 0x9000, ERR: 0x2)",
    "KERNEL PAGE FAULT HALT: addr=0x1234 rip=0x5678 rsp=0x9000 err=0x2 pd=3",
    "EXCEPTION: GP FAULT ENTER pd=3 err=0x0 rip=0x5678 rsp=0x9000",
    "EXCEPTION: DOUBLE FAULT",
    "KERNEL PANIC: DOUBLE FAULT",
    "EXCEPTION: UD ENTER pd=3 rip=0x5678 rsp=0x9000",
    "EXCEPTION: INVALID OPCODE",
    "PKU SECURITY VIOLATION (WRITE at 0x1234)",
    "FATAL: Limine memmap missing",
    "TEST FAILED: unexpected state",
    "[sex.arm.panic]",
]


class BootGraphTests(unittest.TestCase):
    def order(self, lines):
        _, _, ready = checker.bootgraph_gate(lines)
        return checker.order_gate(lines, ready)

    def test_complete_idle_boot(self):
        lines = good_log()
        self.assertTrue(checker.bootgraph_gate(lines)[0].passed)
        self.assertTrue(checker.cap_grant_gate(lines).passed)
        gate, rows = self.order(lines)
        self.assertTrue(gate.passed, gate.reason)
        self.assertTrue(any(row.result == "UNOBSERVED" for row in rows))

    def test_missing_startup_send_is_not_pass(self):
        lines = [line for line in good_log() if "from=silkbar" not in line]
        self.assertFalse(self.order(lines)[0].passed)

    def test_early_send_rejected(self):
        lines = good_log()
        marker = next(line for line in lines if "from=silkbar" in line)
        lines.remove(marker)
        lines.insert(1, marker)
        self.assertFalse(self.order(lines)[0].passed)

    def test_shell_receiver_alias_checked(self):
        lines = good_log()
        lines.insert(0, "[bootgraph.edge.send from=sexinput to=silk-shell slot=6 op=HID first=1]")
        gate, rows = self.order(lines)
        self.assertFalse(gate.passed)
        row = next(row for row in rows if row.edge == "sexinput->silk-shell")
        self.assertIsNotNone(row.receiver_ready_line)
        self.assertEqual(row.result, "FAIL")

    def test_missing_duplicate_or_forged_shell_acceptance_fails(self):
        marker = "[sexinput.shell.ready.accept] caller=3"
        missing = [line for line in good_log() if line != marker]
        for lines in (missing, good_log() + [marker], missing + [marker.replace("caller=3", "caller=9")]):
            self.assertFalse(self.order(lines)[0].passed)

    def test_missing_or_early_shell_notification_fails(self):
        marker = "[silk-shell.ready.notify] to=sexinput"
        missing = [line for line in good_log() if line != marker]
        self.assertFalse(self.order(missing)[0].passed)
        self.assertFalse(self.order([marker] + missing)[0].passed)

    def test_input_send_requires_prior_authenticated_acceptance(self):
        lines = good_log()
        index = lines.index("[sexinput.shell.ready.accept] caller=3")
        lines.insert(index, "[bootgraph.edge.send from=sexinput to=silk-shell slot=6 op=HID first=1]")
        self.assertFalse(self.order(lines)[0].passed)
        lines[index], lines[index + 1] = lines[index + 1], lines[index]
        self.assertTrue(self.order(lines)[0].passed)

    def test_missing_receiver_ready_rejected(self):
        lines = [line for line in good_log() if line != "[sexdisplay.ready]"]
        self.assertFalse(self.order(lines)[0].passed)

    def test_duplicate_first_send_rejected(self):
        lines = good_log()
        lines += [next(line for line in lines if "from=silkbar" in line)]
        self.assertFalse(self.order(lines)[0].passed)

    def test_malformed_first_send_rejected(self):
        lines = good_log() + ["[bootgraph.edge.send from=sexusb to=sexinput first=1]"]
        self.assertFalse(self.order(lines)[0].passed)

    def test_duplicate_ready_rejected(self):
        self.assertFalse(checker.bootgraph_gate(good_log() + ["[sexdisplay.ready]"])[0].passed)

    def test_missing_cap_grants_rejected(self):
        lines = [line for line in good_log() if "[bootgraph.cap.grant" not in line]
        self.assertFalse(checker.cap_grant_gate(lines).passed)

    def test_grant_outside_phase_rejected(self):
        lines = good_log()
        marker = lines.pop(1)
        lines.append(marker)
        self.assertFalse(checker.cap_grant_gate(lines).passed)

    def test_missing_phase_rejected(self):
        lines = [line for line in good_log() if "phase25.complete" not in line]
        self.assertFalse(self.order(lines)[0].passed)

    def test_actual_kernel_fault_signatures_case_insensitive(self):
        for signature in ACTUAL_FAULT_LINES:
            for line in [signature, signature.lower(), signature.upper()]:
                with self.subTest(signature=line):
                    result = checker.fault_gate(good_log() + [line], False)
                    self.assertFalse(result.passed, line)
                    self.assertTrue(checker.fault_gate([line], True).passed)

    def test_fault_and_clock_failures(self):
        self.assertFalse(checker.fault_gate(good_log() + ["#PF"], False).passed)
        lines = [line for line in good_log() if "sexdisplay.clock.recv" not in line]
        self.assertFalse(checker.clock_gate(lines, False)[0].passed)


class DiskPersistenceProofTests(unittest.TestCase):
    SAVE = "[quil.persist.save.ok] bytes=242 hash=0xabcdef doc=2\n"
    LOAD = "[quil.persist.load.ok] bytes=242 hash=0xabcdef doc=2\n"

    def test_both_loads_match_saved_document_and_fingerprint(self):
        rows = disk_checker.persistence_rows(self.SAVE, self.LOAD * 2)
        self.assertTrue(all(row.passed for row in rows))

    def test_same_length_corruption_on_either_load_fails(self):
        bad = self.LOAD.replace("0xabcdef", "0xabcdee")
        for log in [bad + self.LOAD, self.LOAD + bad]:
            rows = disk_checker.persistence_rows(self.SAVE, log)
            self.assertTrue(rows[0].passed)
            self.assertFalse(rows[1].passed)

    def test_missing_malformed_or_duplicate_metadata_fails(self):
        for field in ["hash=0xabcdef", "bytes=242", "doc=2"]:
            for load in [self.LOAD.replace(field, ""),
                         self.LOAD.replace(field, field + " " + field)]:
                with self.subTest(load=load):
                    self.assertFalse(all(row.passed for row in disk_checker.persistence_rows(self.SAVE, load * 2)))
        for invalid_hash in ["", "0x", "0xnope", "0x10000000000000000"]:
            save = self.SAVE.replace("0xabcdef", invalid_hash)
            self.assertFalse(all(row.passed for row in disk_checker.persistence_rows(save, self.LOAD * 2)))

    def test_other_document_or_single_reload_cannot_satisfy_proof(self):
        for log in [self.LOAD, self.LOAD.replace("doc=2", "doc=3") * 2,
                    self.LOAD + self.LOAD.replace("doc=2", "doc=3"), ""]:
            self.assertFalse(all(row.passed for row in disk_checker.persistence_rows(self.SAVE, log)))

    def test_changed_length_on_earlier_load_fails(self):
        rows = disk_checker.persistence_rows(self.SAVE, self.LOAD.replace("242", "241") + self.LOAD)
        self.assertFalse(any(row.passed for row in rows))

    def ppm(self, count=51, width=1272, height=360):
        pixels = bytearray(width * height * 3)
        start = (56 * width + 1072) * 3
        pixels[start:start + count * 3] = b"\xe0\xf0\xff" * count
        return f"P6\n{width} {height}\n255\n".encode() + pixels

    def test_valid_screenshot_requires_enough_text_pixels(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "screen.ppm"
            for count, expected in [(51, True), (50, False)]:
                path.write_bytes(self.ppm(count))
                self.assertEqual(disk_checker.pixel_row(path).passed, expected)

    def test_missing_malformed_truncated_or_out_of_bounds_screenshot_fails(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "screen.ppm"
            self.assertFalse(disk_checker.pixel_row(path).passed)
            valid = self.ppm()
            for data in [b"", b"P6\n1272", b"P6\nbad dimensions\n255\n",
                         valid.replace(b"P6", b"P3", 1), valid[:-1], valid + b"\x00",
                         valid.replace(b"255\n", b"65535\n", 1),
                         self.ppm(width=1271), self.ppm(height=359)]:
                with self.subTest(header=data[:30]):
                    path.write_bytes(data)
                    self.assertFalse(disk_checker.pixel_row(path).passed)

    def test_ppm_comments_and_raster_whitespace_preserved(self):
        data = self.ppm().replace(b"P6\n", b"P6\n# QEMU screenshot\n", 1)
        width, height, pixels = disk_checker.ppm_rgb(data)
        self.assertEqual((width, height), (1272, 360))
        header = b"P6\n1272 360\n255\n"
        _, _, raster = disk_checker.ppm_rgb(header + b"\n" + pixels[1:])
        self.assertEqual(raster[0], 10)

    def test_native_viewport_selects_current_quil_content(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "screen.ppm"
            path.write_bytes(self.ppm())
            visible = "[shell.app.viewport] target=9 sid=156 x=1072 y=56 w=200 h=304"
            self.assertTrue(disk_checker.pixel_row(path, visible).passed)
            moved = "[shell.app.viewport] target=9 sid=156 x=8 y=56 w=200 h=304"
            self.assertFalse(disk_checker.pixel_row(path, visible + moved).passed)
            self.assertTrue(disk_checker.pixel_row(path, moved + visible).passed)

    def test_pixel_stripe_claims_only_color_occupancy(self):
        with tempfile.TemporaryDirectory() as temp:
            path=Path(temp)/"screen.ppm"
            path.write_bytes(self.ppm())
            row=disk_checker.pixel_row(path,"[shell.app.viewport] target=9 sid=156 x=1072 y=56 w=200 h=304")
            self.assertTrue(row.passed)
            self.assertEqual(row.name,"pixel_quil_text_color_occupancy")
            self.assertIn("glyph_identity=unverified",row.detail)
            self.assertIn("document_identity=transport_fingerprint",row.detail)

    def test_native_viewport_missing_foreign_empty_or_offscreen_fails(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "screen.ppm"
            path.write_bytes(self.ppm())
            for view in ["", "[shell.app.viewport] target=12 sid=156 x=1072 y=56 w=200 h=304",
                         "[shell.app.viewport] target=9 sid=154 x=1072 y=56 w=200 h=304",
                         "[shell.app.viewport] target=9 sid=156 x=1072 y=56 w=0 h=304",
                         "[shell.app.viewport] target=9 sid=156 x=5000 y=56 w=200 h=304"]:
                with self.subTest(view=view):
                    self.assertFalse(disk_checker.pixel_row(path, view).passed)


class UsbGatePortabilityTests(unittest.TestCase):
    def usb_helpers(self):
        source = (ROOT / "scripts/usb_path_gate.sh").read_text()
        # Exercise real helpers without starting a build or QEMU lane.
        return "FAULT_RE=" + source.split("FAULT_RE=", 1)[1].split("# ── BUILD", 1)[0]

    def test_bash3_named_rows_preserve_literal_values_and_reject_bad_keys(self):
        program = self.usb_helpers() + r'''
for key in build usb_discovery usb_enum usb_pointer_producer touch_contact lane1_fault_free usb_discovery_negative usb_kbd_lane usb_boot_mouse; do
  row_set "$key" "$1"
  [[ "$(row_get "$key")" == "$1" ]] || exit 3
done
if row_set 'bad[$(exit 99)]' PASS; then exit 4; fi
if row_get 'bad[$(exit 99)]'; then exit 5; fi
'''
        literal = '$(touch should-never-execute); $HOME `exit 99`'
        result = subprocess.run(["/bin/bash", "-euc", program, "usb-row-test", literal],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_usb_fault_scan_matches_actual_kernel_diagnostics(self):
        with tempfile.TemporaryDirectory() as temp:
            log = Path(temp) / "serial.log"
            program = self.usb_helpers() + '\nhas_fault "$1"\n'
            for signature in ACTUAL_FAULT_LINES:
                for line in [signature, signature.lower()]:
                    with self.subTest(signature=line):
                        log.write_text(line + "\n")
                        result = subprocess.run(["/bin/bash", "-euc", program, "usb-fault-test", str(log)],
                                                capture_output=True, text=True)
                        self.assertEqual(result.returncode, 0, result.stderr)


class HarnessTests(unittest.TestCase):
    def run_fake(self, mode, extra_lines=(), persistence=False):
        with tempfile.TemporaryDirectory() as temp:
            work = Path(temp)
            fixture = work / "fixture.log"
            fixture.write_text("\n".join(good_log() + list(extra_lines)) + "\n")
            image = work / "test.iso"
            image.write_bytes(b"test image; not bootable")
            fake = work / "fake-qemu"
            fake.write_text("""#!/usr/bin/env python3
import os, pathlib, signal, sys, time
args = sys.argv[1:]
log = args[args.index('-serial') + 1].removeprefix('file:')
pathlib.Path(log).write_text(pathlib.Path(os.environ['FIXTURE']).read_text())
print('CURRENT_QEMU_DIAGNOSTIC', flush=True)
if os.environ['FAKE_MODE'] == 'exit_7':
    sys.exit(7)
if os.environ['FAKE_MODE'] == 'exit_0':
    sys.exit(0)
signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
while True:
    time.sleep(0.1)
""")
            fake.chmod(0o755)
            gate = work / "gate"
            gate.mkdir()
            for name in ["qemu-command.txt", "qemu.log", "serial.boot_a.log", "serial.boot_b.log",
                         "report.md", "provenance.json"]:
                (gate / name).write_text("STALE_PREVIOUS_RUN\n")
            env = dict(os.environ, GATE_DIR=str(gate), QEMU_BIN=str(fake),
                       ISO=str(image), FIXTURE=str(fixture), FAKE_MODE=mode,
                       SEXOS_PERSISTENCE_REBOOT_PROOF="1" if persistence else "0")
            result = subprocess.run(["bash", str(ROOT / "scripts/master_runtime_gate.sh"),
                                     "--skip-build", "--no-nvme", "--probe", "1", "--keep-log"],
                                    cwd=ROOT, env=env, capture_output=True, text=True, timeout=15)
            self.assertTrue((work / "gate/report.md").exists(), result.stdout + result.stderr)
            report = (work / "gate/report.md").read_text()
            provenance = (work / "gate/provenance.json").read_text()
            commands = (gate / "qemu-command.txt").read_text()
            diagnostics = (gate / "qemu.log").read_text()
            for content in [report, provenance, commands, diagnostics]:
                self.assertNotIn("STALE_PREVIOUS_RUN", content)
            boot_count = 2 if persistence and mode == "running" else 1
            self.assertEqual(len(commands.splitlines()), boot_count)
            self.assertEqual(diagnostics.count("CURRENT_QEMU_DIAGNOSTIC"), boot_count)
            if persistence and mode == "running":
                for name in ["serial.boot_a.log", "serial.boot_b.log"]:
                    self.assertIn("[sexdisplay.ready]", (gate / name).read_text())
                    self.assertNotIn("STALE_PREVIOUS_RUN", (gate / name).read_text())
            else:
                self.assertFalse((gate / "serial.boot_a.log").exists())
                self.assertFalse((gate / "serial.boot_b.log").exists())
            return result, report, provenance

    def test_full_probe_completes_with_provenance(self):
        result, report, provenance = self.run_fake("running")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("GREEN_MASTER", report)
        self.assertIn("unverified_existing_image", provenance)
        self.assertIn("iso_sha256", provenance)
        self.assertIn("git_status", provenance)

    def test_qemu_nonzero_exit_fails_even_with_complete_markers(self):
        result, report, _ = self.run_fake("exit_7")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("early_exit:7", report)
        self.assertIn("RED_MASTER", report)

    def test_qemu_early_clean_exit_is_incomplete_observation(self):
        result, report, _ = self.run_fake("exit_0")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("early_exit:0", report)

    def test_actual_fault_signatures_fail_master_gate(self):
        for signature in [ACTUAL_FAULT_LINES[2], ACTUAL_FAULT_LINES[3], ACTUAL_FAULT_LINES[4]]:
            with self.subTest(signature=signature):
                result, report, _ = self.run_fake("running", [signature])
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("| FAULT_GATE | FAIL |", report)
                self.assertIn("Fault/panic hits: 1", report)
                self.assertIn("RED_MASTER", report)

    def test_custom_iso_requires_skip_build_before_build_starts(self):
        with tempfile.TemporaryDirectory() as temp:
            gate = Path(temp) / "gate"
            gate.mkdir()
            for name in ["qemu-command.txt", "qemu.log", "report.md", "provenance.json"]:
                (gate / name).write_text("STALE_PREVIOUS_RUN\n")
            env = dict(os.environ, ISO=str(Path(temp) / "unrelated.iso"), GATE_DIR=str(gate))
            result = subprocess.run(["bash", str(ROOT / "scripts/master_runtime_gate.sh"),
                                     "--no-nvme", "--probe", "1"],
                                    env=env, capture_output=True, text=True, timeout=3)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("custom ISO requires --skip-build", result.stderr)
            self.assertNotIn("BUILD phase", result.stdout)
            self.assertFalse((gate / "report.md").exists())
            self.assertFalse((gate / "provenance.json").exists())
            self.assertEqual((gate / "qemu-command.txt").read_text(), "")
            self.assertEqual((gate / "qemu.log").read_text(), "")

    def test_reused_gate_preserves_current_persistence_boots_only(self):
        result, _, _ = self.run_fake("running", persistence=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_probe_requires_positive_integer(self):
        for value in ["0", "-1", "1.5", "oops"]:
            result = subprocess.run(["bash", str(ROOT / "scripts/master_runtime_gate.sh"),
                                     "--probe", value], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("positive integer", result.stderr)


if __name__ == "__main__":
    unittest.main()
