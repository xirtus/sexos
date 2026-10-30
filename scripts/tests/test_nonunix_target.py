"""Source and compiler policy failures tested independently; no repo mutations."""
import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_nonunix_target as gate

ROOT = Path(__file__).resolve().parents[2]
SOURCE = json.loads((ROOT / 'x86_64-sex.json').read_text())
CFG = 'target_os="none"\ntarget_env=""\ntarget_arch="x86_64"\ntarget_pointer_width="64"\ntarget_endian="little"\npanic="abort"\ntarget_feature="x87"\n'


class NonUnixTarget(unittest.TestCase):
    def temporary(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name).resolve()
        (root / 'x86_64-sex.json').write_text(json.dumps(SOURCE))
        (root / 'rust-toolchain.toml').write_text('[toolchain]\nchannel="nightly-2026-07-22"\n')
        return root

    def test_valid_source_and_independent_compiler_cfg(self):
        root = self.temporary()
        result = gate.inspect(root, cfg_output=CFG, compiler_output='rustc pinned-test')
        self.assertTrue(result['ok'], result)
        self.assertEqual(result['cfg']['features'], ['x87'])
        self.assertEqual(len(result['target_sha256']), 64)

    def test_wrong_source_labels_never_reach_compiler(self):
        for key, value in [('os', 'redox'), ('env', 'relibc'), ('target-family', ['unix']),
                           ('target-family', 'unix'), ('llvm-target', 'x86_64-unknown-linux-gnu')]:
            with self.subTest(key=key, value=value):
                root = self.temporary()
                source = copy.deepcopy(SOURCE)
                source[key] = value
                (root / 'x86_64-sex.json').write_text(json.dumps(source))
                with patch.object(gate.subprocess, 'run', side_effect=AssertionError('compiler reached')):
                    self.assertFalse(gate.inspect(root)['ok'])

    def test_each_scalar_baremetal_source_requirement(self):
        for key, value in [('target-pointer-width', '64'), ('target-pointer-width', 32),
                           ('target-c-int-width', 64), ('rustc-abi', ''), ('disable-redzone', False),
                           ('panic-strategy', 'unwind'), ('has-rpath', True), ('arch', 'aarch64'),
                           ('linker', 'cc'), ('code-model', 'small'), ('data-layout', '')]:
            source = copy.deepcopy(SOURCE)
            source[key] = value
            self.assertTrue(gate.check_source(source), key)

    def test_source_features_missing_enabled_or_conflicting_fail(self):
        for features in ['', '-mmx,-sse,+soft-float', SOURCE['features'] + ',+sse2',
                         SOURCE['features'] + ',+avx2', SOURCE['features'] + ',-sse2', None]:
            source = copy.deepcopy(SOURCE)
            source['features'] = features
            self.assertTrue(gate.check_source(source), features)

    def test_cfg_os_env_arch_and_width_fail_independently_of_good_json(self):
        root = self.temporary()
        for old, new in [('target_os="none"', 'target_os="redox"'), ('target_env=""', 'target_env="relibc"'),
                         ('target_arch="x86_64"', 'target_arch="aarch64"'), ('target_pointer_width="64"', 'target_pointer_width="32"'),
                         ('panic="abort"', 'panic="unwind"')]:
            result = gate.inspect(root, cfg_output=CFG.replace(old, new), compiler_output='rustc test')
            self.assertFalse(result['ok'], new)

    def test_cfg_unix_family_and_vector_features_fail(self):
        for suffix in ['unix\n', 'target_family="unix"\n', 'target_family="windows"\n',
                       'target_feature="sse2"\n', 'target_feature="avx"\n', 'target_feature="avx512f"\n']:
            self.assertTrue(gate.check_cfg(CFG + suffix)[0], suffix)

    def test_cfg_missing_duplicate_malformed_or_empty_fail(self):
        for cfg in ['', CFG.replace('target_env=""\n', ''), CFG + 'target_os="none"\n',
                    CFG + 'target_os=none\n', CFG + 'target_feature="bad\\q"\n']:
            self.assertTrue(gate.check_cfg(cfg)[0], cfg)

    def test_duplicate_or_invalid_json_and_missing_toolchain_fail_closed(self):
        root = self.temporary()
        for raw in ['{} trailing', '[]', '{"os":"none","os":"redox"}']:
            (root / 'x86_64-sex.json').write_text(raw)
            self.assertFalse(gate.inspect(root, cfg_output=CFG, compiler_output='rustc test')['ok'])
        (root / 'x86_64-sex.json').write_text(json.dumps(SOURCE))
        (root / 'rust-toolchain.toml').unlink()
        self.assertFalse(gate.inspect(root, cfg_output=CFG, compiler_output='rustc test')['ok'])

    def test_actual_invocations_are_argument_arrays_and_repo_pin_cwd(self):
        root = self.temporary()
        with patch.dict(gate.os.environ, {'RUSTUP_TOOLCHAIN': 'wrong-override'}):
            with patch.object(gate.subprocess, 'run', side_effect=[subprocess.CompletedProcess([], 0, CFG),
                              subprocess.CompletedProcess([], 0, 'rustc fixture')]) as run:
                result = gate.inspect(root)
        self.assertTrue(result['ok'], result)
        self.assertEqual(run.call_args_list[0].args[0], ['rustc', '-Z', 'unstable-options', '--print', 'cfg', '--target', str(root / 'x86_64-sex.json')])
        for call in run.call_args_list:
            self.assertEqual(call.kwargs['cwd'], root)
            self.assertNotIn('RUSTUP_TOOLCHAIN', call.kwargs['env'])
            self.assertTrue(call.kwargs['check'])

    def test_failed_compiler_or_absent_identity_cannot_pass(self):
        root = self.temporary()
        with patch.object(gate.subprocess, 'run', side_effect=subprocess.CalledProcessError(1, ['rustc'])):
            self.assertFalse(gate.inspect(root)['ok'])
        self.assertFalse(gate.inspect(root, cfg_output=CFG, compiler_output='')['ok'])


if __name__ == '__main__':
    unittest.main()
