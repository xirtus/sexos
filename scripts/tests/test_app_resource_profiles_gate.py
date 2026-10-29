"""Exercise the real profile driver with stub builds/checkers/GUI transport.

These tests prove orchestration/restoration, not guest or rendering behavior.
"""
from pathlib import Path
import os
import shutil
import subprocess
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[2]


class AppResourceProfilesTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='sexos-app-profiles-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root/'scripts').mkdir()
        (self.root/'iso_root').mkdir()
        shutil.copyfile(REPO/'scripts/app_resource_profiles_gate.sh', self.root/'scripts/app_resource_profiles_gate.sh')
        self.original = 'abi_hash = "preserve-this-hash"\n' + ''.join(
            f'[[stage]]\nid = "{stage}"\nrustflags = ""\n' for stage in
            ('build_kernel', 'build_sexinput', 'build_sexbell', 'build_kaleidoscope'))
        self.original += '[[stage]]\nid = "build_quil"\nrustflags = "--cfg sexfiles_quil_persistence_proof"\n'
        (self.root/'sexos_build_spec.toml').write_text(self.original)
        (self.root/'sexos-v1.0.0.iso').write_text('original image')
        (self.root/'iso_root/sexos-kernel').write_text('original kernel')
        (self.root/'scripts/master_runtime_gate.sh').write_text('''set -euo pipefail
[[ "$*" == "--probe 50 --keep-log" ]]
[[ "$ISO" == "sexos-v1.0.0.iso" ]]
profile="${GATE_DIR##*/}"
echo "build:$profile" >> events
mkdir -p "$GATE_DIR"
cp sexos_build_spec.toml "$GATE_DIR/observed-spec.toml"
cp sexos_build_spec.toml sexos-v1.0.0.iso
cp sexos_build_spec.toml iso_root/sexos-kernel
if [[ "${SIGNAL_AT:-}" == "build:$profile" ]]; then kill -TERM "$PPID"; exit 0; fi
[[ "${FAIL_AT:-}" != "build:$profile" ]] || exit 19
printf '%s' "$profile" > "$GATE_DIR/serial.log"
''')
        (self.root/'scripts/app_resource_runtime_gate.sh').write_text('''set -euo pipefail
[[ "$(cat "$APP_RESOURCE_LOG")" == resource ]]
echo resource:check >> events
[[ "${FAIL_AT:-}" != resource:check ]] || exit 20
''')
        (self.root/'scripts/paging_recovery_runtime_gate.sh').write_text('''set -euo pipefail
[[ "$(cat "$PAGING_LOG")" == normal ]]
echo normal:check >> events
[[ "${FAIL_AT:-}" != normal:check ]] || exit 21
''')
        (self.root/'scripts/desktop_close_runtime_gate.sh').write_text('''set -euo pipefail
cmp "$ISO" evidence/normal/tested.iso
cmp sexos_build_spec.toml evidence/normal/tested-spec.toml
echo desktop:close3 >> events
mkdir -p "$GATE_DIR"
cp "$ISO" "$GATE_DIR/observed.iso"
[[ "${FAIL_AT:-}" != desktop:close3 ]] || exit 22
if [[ "${CORRUPT_CLOSE:-}" == 1 ]]; then printf corrupt > "$ISO"; fi
''')

    def run_gate(self, **extra):
        env = os.environ.copy()
        env.update(GATE_DIR=str(self.root/'evidence'), **extra)
        return subprocess.run(['bash','scripts/app_resource_profiles_gate.sh'], cwd=self.root,
                              env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)

    def assert_restored(self, spec=None):
        self.assertEqual((self.root/'sexos_build_spec.toml').read_text(), spec or self.original)
        self.assertEqual((self.root/'sexos-v1.0.0.iso').read_text(), 'original image')
        self.assertEqual((self.root/'iso_root/sexos-kernel').read_text(), 'original kernel')

    def test_success_exact_two_builds_then_three_app_close(self):
        result = self.run_gate()
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual((self.root/'events').read_text().splitlines(),
                         ['build:resource','resource:check','build:normal','normal:check','desktop:close3'])
        expected = self.original.replace('id = "build_kaleidoscope"\nrustflags = ""',
            'id = "build_kaleidoscope"\nrustflags = "--cfg app_resource_cleanup_proof"')
        for artifact in ('observed-spec.toml','tested-spec.toml','tested.iso','tested.kernel'):
            self.assertEqual((self.root/'evidence/resource'/artifact).read_text(), expected)
            self.assertEqual((self.root/'evidence/normal'/artifact).read_text(), self.original)
        self.assertEqual((self.root/'sexos_build_spec.toml').read_text(), self.original)
        self.assertEqual((self.root/'sexos-v1.0.0.iso').read_text(), self.original)
        self.assertEqual((self.root/'iso_root/sexos-kernel').read_text(), self.original)
        self.assertIn('[app.resource.profiles] PASS', result.stdout)

    def test_each_build_or_checker_failure_restores_spec_iso_and_kernel(self):
        for step, code in [('build:resource',19),('resource:check',20),('build:normal',19),('normal:check',21),('desktop:close3',22)]:
            with self.subTest(step=step):
                shutil.rmtree(self.root/'evidence', ignore_errors=True)
                (self.root/'events').unlink(missing_ok=True)
                result = self.run_gate(FAIL_AT=step)
                self.assertEqual(result.returncode,code,result.stdout)
                self.assert_restored()
                self.assertNotIn('[app.resource.profiles] PASS',result.stdout)
                self.assertEqual((self.root/'events').read_text().splitlines()[-1],step)

    def test_all_mutated_normal_stages_rejected_before_any_build(self):
        for stage in ('build_kernel','build_sexinput','build_sexbell','build_kaleidoscope'):
            with self.subTest(stage=stage):
                custom=self.original.replace(f'id = "{stage}"\nrustflags = ""',f'id = "{stage}"\nrustflags = "--cfg custom"')
                (self.root/'sexos_build_spec.toml').write_text(custom)
                result=self.run_gate()
                self.assertNotEqual(result.returncode,0)
                self.assert_restored(custom)
                self.assertFalse((self.root/'events').exists())

    def test_missing_or_duplicate_stage_is_rejected(self):
        for custom in [self.original.replace('id = "build_kaleidoscope"','id = "unexpected"'),self.original+'[[stage]]\nid = "build_kernel"\nrustflags = ""\n']:
            (self.root/'sexos_build_spec.toml').write_text(custom)
            self.assertNotEqual(self.run_gate().returncode,0)
            self.assert_restored(custom)
            self.assertFalse((self.root/'events').exists())

    def test_nonempty_evidence_is_never_reused(self):
        (self.root/'evidence').mkdir()
        (self.root/'evidence/stale').write_text('old proof')
        self.assertNotEqual(self.run_gate().returncode,0)
        self.assert_restored()
        self.assertFalse((self.root/'events').exists())

    def test_failure_without_original_image_removes_generated_artifacts(self):
        (self.root/'sexos-v1.0.0.iso').unlink()
        (self.root/'iso_root/sexos-kernel').unlink()
        result=self.run_gate(FAIL_AT='normal:check')
        self.assertEqual(result.returncode,21,result.stdout)
        self.assertEqual((self.root/'sexos_build_spec.toml').read_text(),self.original)
        self.assertFalse((self.root/'sexos-v1.0.0.iso').exists())
        self.assertFalse((self.root/'iso_root/sexos-kernel').exists())

    def test_term_during_build_restores_original_artifacts(self):
        result=self.run_gate(SIGNAL_AT="build:normal")
        self.assertEqual(result.returncode,143,result.stdout)
        self.assert_restored()
        self.assertNotIn("[app.resource.profiles] PASS",result.stdout)

    def test_close_cannot_replace_verified_normal_image(self):
        result=self.run_gate(CORRUPT_CLOSE='1')
        self.assertNotEqual(result.returncode,0)
        self.assert_restored()
        self.assertNotIn('[app.resource.profiles] PASS',result.stdout)


if __name__ == '__main__': unittest.main()
