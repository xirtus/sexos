"""Profile orchestration/restoration tests; child build/runtime tools are doubles."""
from pathlib import Path
import os
import shutil
import subprocess
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[2]


class CleanupProfilesGateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='sexos-profile-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root/'scripts').mkdir()
        (self.root/'iso_root').mkdir()
        shutil.copyfile(REPO/'scripts/paging_cleanup_profiles_gate.sh', self.root/'scripts/paging_cleanup_profiles_gate.sh')
        self.original = '''[[stage]]
id = "build_sexbell"
rustflags = ""
[[stage]]
id = "build_kaleidoscope"
rustflags = ""
[[stage]]
id = "tail"
'''
        (self.root/'sexos_build_spec.toml').write_text(self.original)
        (self.root/'sexos-v1.0.0.iso').write_text('original normal image')
        (self.root/'scripts/master_runtime_gate.sh').write_text('''#!/usr/bin/env bash
set -euo pipefail
mkdir -p "$GATE_DIR"
cp sexos_build_spec.toml "$GATE_DIR/observed-spec.toml"
if [[ "${FAIL_PROFILE:-}" != "" && "$GATE_DIR" == */"$FAIL_PROFILE" ]]; then exit 9; fi
printf 'stub orchestration transcript\n' > "$GATE_DIR/serial.log"
cp sexos_build_spec.toml sexos-v1.0.0.iso
cp sexos_build_spec.toml iso_root/sexos-kernel
''')
        for name in ('task_exit_cleanup_runtime_gate.sh', 'paging_recovery_runtime_gate.sh'):
            (self.root/'scripts'/name).write_text('#!/usr/bin/env bash\nset -euo pipefail\ntest -f "$PAGING_LOG"\n')

    def run_gate(self, **extra):
        env = os.environ.copy(); env.update(GATE_DIR=str(self.root/'evidence'), **extra)
        return subprocess.run(['bash','scripts/paging_cleanup_profiles_gate.sh'],cwd=self.root,env=env,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)

    def test_success_restores_spec_and_builds_normal_last(self):
        r=self.run_gate(); self.assertEqual(r.returncode,0,r.stdout)
        self.assertEqual((self.root/'sexos_build_spec.toml').read_text(),self.original)
        self.assertEqual((self.root/'sexos-v1.0.0.iso').read_text(),self.original)
        proof=self.root/'evidence'
        self.assertIn('--cfg paging_exit_cleanup_proof',(proof/'self/observed-spec.toml').read_text())
        self.assertIn('--cfg paging_quota_proof',(proof/'quota/observed-spec.toml').read_text())
        self.assertEqual((proof/'normal/observed-spec.toml').read_text(),self.original)

    def test_failed_build_restores_original_and_never_claims_pass(self):
        r=self.run_gate(FAIL_PROFILE='quota'); self.assertEqual(r.returncode,9,r.stdout)
        self.assertEqual((self.root/'sexos_build_spec.toml').read_text(),self.original)
        self.assertNotIn('[paging.cleanup.profiles] PASS',r.stdout)
        self.assertEqual((self.root/'sexos-v1.0.0.iso').read_text(),'original normal image')
        self.assertFalse((self.root/'evidence/normal').exists())

    def test_existing_custom_flags_are_not_overwritten(self):
        custom=self.original.replace('rustflags = ""','rustflags = "--cfg custom"',1)
        (self.root/'sexos_build_spec.toml').write_text(custom)
        r=self.run_gate(); self.assertNotEqual(r.returncode,0)
        self.assertEqual((self.root/'sexos_build_spec.toml').read_text(),custom)
        self.assertFalse((self.root/'evidence/self').exists())


if __name__=='__main__': unittest.main()
