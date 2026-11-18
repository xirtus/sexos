# SPDX-License-Identifier: GPL-3.0-only
"""PKU gate stub (crates/sex-gate): the checked-in template matches stub.S, and the
disassembly obeys the gate rules (study section 4.2)."""
import re, shutil, subprocess, sys, tempfile, unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
import gen_gate_stub  # noqa: E402

TOOLS = all(shutil.which(t) for t in ('as', 'nm', 'objcopy', 'objdump'))


@unittest.skipUnless(TOOLS, 'needs GNU as, nm, objcopy and objdump')
class GateStubTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with tempfile.TemporaryDirectory() as tmp:
            obj = Path(tmp) / 'stub.o'
            args = ['as', '--64', '-o', str(obj)]
            for name, (_, base, _) in gen_gate_stub.FIELDS.items():
                args += ['--defsym', f'{name}={base:#x}']
            subprocess.run(args + [str(gen_gate_stub.SRC)], check=True)
            out = subprocess.run(['objdump', '-d', '--no-show-raw-insn', '-M', 'intel', str(obj)],
                                 capture_output=True, text=True, check=True).stdout
        cls.insns = [l.split('\t', 1)[1].strip() for l in out.splitlines() if re.match(r'^\s+[0-9a-f]+:\t', l)]

    def test_template_is_current(self):
        self.assertEqual(gen_gate_stub.OUT.read_text(), gen_gate_stub.render(),
                         'crates/sex-gate/src/template.rs is stale; run scripts/gen_gate_stub.py')

    def test_no_indirect_branches(self):
        for i in self.insns:
            op, _, rest = i.partition(' ')
            if op.startswith(('call', 'jmp')) or (op.startswith('j') and op != 'jmp'):
                self.assertRegex(rest.strip(), r'^[0-9a-f]+ <', f'indirect or unexpected branch: {i}')

    def test_every_wrpkru_writes_an_immediate_and_is_checked(self):
        idx = [n for n, i in enumerate(self.insns) if i.startswith('wrpkru')]
        self.assertEqual(len(idx), 4)
        for n in idx:
            load = next(i for i in reversed(self.insns[:n]) if i.startswith('mov') and 'eax' in i.split(',')[0])
            imm = load.split(',')[1].strip()
            self.assertRegex(imm, r'^0x[0-9a-f]+$', f'WRPKRU value is not an immediate: {load}')
            self.assertEqual(self.insns[n + 1].replace(' ', ''), f'cmpeax,{imm}', 'check after WRPKRU')
            self.assertTrue(self.insns[n + 2].startswith('jne'), 'jne after the check')

    def test_no_other_key_register_writers(self):
        for i in self.insns:
            self.assertFalse(i.startswith(('xrstor', 'xsetbv', 'wrmsr', 'syscall', 'sysenter', 'int ')), i)


if __name__ == '__main__':
    unittest.main()
