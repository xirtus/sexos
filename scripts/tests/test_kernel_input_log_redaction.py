"""Compile the real PS/2 diagnostic statements in normal and proof modes."""
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
EVENTS = ("ps2.port60.read", "ps2.input_ring.enqueue", "ps2.input_ring.drop")


def diagnostic_statements(source):
    """Keep the source cfg attributes; an unguarded duplicate must fail too."""
    statements = []
    for event in EVENTS:
        pattern = r'serial_println!\("\[' + re.escape(event) + r'\][^;]*;'
        matches = list(re.finditer(pattern, source))
        if len(matches) != 2:
            raise AssertionError(f"{event}: expected exactly two cfg-gated statements")
        attributes = []
        for match in matches:
            attribute = re.search(r'#\[cfg\(([^\n]+)\)\]\s*$', source[:match.start()])
            if attribute is None:
                raise AssertionError(f"{event}: unguarded input diagnostic")
            attributes.append(attribute.group(1))
            statements.append(source[attribute.start():match.end()])
        if attributes != ["sexos_input_payload_proof", "not(sexos_input_payload_proof)"]:
            raise AssertionError(f"{event}: expected explicit proof and default guards")
    return "\n".join(statements)


class KernelInputLogRedactionTests(unittest.TestCase):
    def test_actual_sites_preserve_metadata_and_only_opt_in_payload(self):
        statements = diagnostic_statements((ROOT / "kernel/src/interrupts.rs").read_text())
        program = r'''
macro_rules! serial_println { ($($args:tt)*) => { println!($($args)*); }; }
fn main() {
    let read_n = 7;
    let enq_n = 11;
    for scancode in [0x2cu8, 0x39, 0x80] {
''' + statements + "\n    }\n}\n"
        with tempfile.TemporaryDirectory(prefix="sexos-kernel-input-log-") as directory:
            directory = Path(directory)
            source = directory / "probe.rs"
            source.write_text(program)
            for proof in (False, True):
                with self.subTest(proof=proof):
                    executable = directory / ("proof" if proof else "normal")
                    command = ["rustc", "--edition=2021", "-A", "unused_variables",
                               str(source), "-o", str(executable)]
                    if proof:
                        command += ["--cfg", "sexos_input_payload_proof"]
                    subprocess.run(command, check=True, capture_output=True, timeout=60)
                    output = subprocess.check_output([str(executable)], text=True, timeout=10)
                    expected = []
                    for scancode in (0x2c, 0x39, 0x80):
                        suffix = f"sc={scancode:#x}" if proof else "payload=redacted"
                        expected += [f"[ps2.port60.read] n=7 {suffix}",
                                     f"[ps2.input_ring.enqueue] n=11 type=0x201 {suffix}",
                                     f"[ps2.input_ring.drop] reason=ring_full {suffix}"]
                    self.assertEqual(output.splitlines(), expected)

    def test_unguarded_or_wrongly_guarded_sites_are_rejected(self):
        source = (ROOT / "kernel/src/interrupts.rs").read_text()
        first = source.index("#[cfg(sexos_input_payload_proof)]",
                             source.index("let read_n = PS2_PORT60_LOG_COUNT"))
        for replacement in ("", "#[cfg(not(sexos_input_payload_proof))]"):
            changed = source[:first] + source[first:].replace(
                "#[cfg(sexos_input_payload_proof)]", replacement, 1)
            with self.subTest(replacement=replacement), self.assertRaises(AssertionError):
                diagnostic_statements(changed)


if __name__ == "__main__":
    unittest.main()
