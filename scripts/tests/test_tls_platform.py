"""Dependency and effective-graph failures tested without mutating the repo."""
import copy
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch
import tomllib

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_tls_platform as gate

ROOT = Path(__file__).resolve().parents[2]
TLS = tomllib.loads((ROOT / 'crates/sex-tls/Cargo.toml').read_text())
WORKSPACE = tomllib.loads((ROOT / 'Cargo.toml').read_text())
APP = tomllib.loads((ROOT / 'apps/kaleidoscope/Cargo.toml').read_text())
SPEC = tomllib.loads((ROOT / 'sexos_build_spec.toml').read_text())
TREE = '\n'.join(['sex-tls v0.1.0 (/private/work/crates/sex-tls) '] +
                 [f'{name} v{version} ' + ('force-soft' if name == 'sha2' else '')
                  for name, version in gate.PINS.items()] +
                 ['aes v0.8.4 ', 'polyval v0.6.2 ', 'ppv-lite86 v0.2.21 simd',
                  'zerocopy v0.8.63 simd', 'rand_core v0.6.4  (*)'])


class TLSPlatform(unittest.TestCase):
    def flags(self, flags):
        spec = copy.deepcopy(SPEC)
        next(s for s in spec['stage'] if s['id'] == 'build_kaleidoscope')['rustflags'] = flags
        return gate.check_flags(spec)

    def test_actual_manifest_policy_and_scalar_stage(self):
        self.assertEqual(gate.check_manifests(TLS, WORKSPACE, APP), [])
        self.assertEqual(gate.check_flags(SPEC), [])

    def test_each_pinned_dependency_rejects_version_defaults_features_and_source_override(self):
        for name in gate.PINS:
            for key, value in [('version', '*'), ('default-features', True),
                               ('features', ['std']), ('path', '/tmp/different')]:
                tls = copy.deepcopy(TLS)
                tls['dependencies'][name][key] = value
                self.assertTrue(gate.check_manifests(tls, WORKSPACE, APP), (name, key))

    def test_extra_or_target_dependencies_and_workspace_replacements_rejected(self):
        tls = copy.deepcopy(TLS)
        tls['dependencies']['getrandom'] = '0.3'
        self.assertTrue(gate.check_manifests(tls, WORKSPACE, APP))
        tls = copy.deepcopy(TLS)
        tls['target'] = {'cfg(unix)': {'dependencies': {'libc': '0.2'}}}
        self.assertTrue(gate.check_manifests(tls, WORKSPACE, APP))
        workspace = copy.deepcopy(WORKSPACE)
        workspace['patch'] = {'crates-io': {'embedded-tls': {'path': '/tmp/replacement'}}}
        self.assertTrue(gate.check_manifests(TLS, workspace, APP))

    def test_workspace_or_app_cannot_bypass_reviewed_crate(self):
        workspace = copy.deepcopy(WORKSPACE)
        workspace['workspace']['members'].remove('crates/sex-tls')
        self.assertTrue(gate.check_manifests(TLS, workspace, APP))
        app = copy.deepcopy(APP)
        app['dependencies']['sex-tls'] = {'version': '0.1'}
        self.assertTrue(gate.check_manifests(TLS, WORKSPACE, app))

    def test_both_cfg_spellings_accepted_and_missing_duplicate_malformed_rejected(self):
        self.assertEqual(self.flags('--cfg=aes_force_soft --cfg=polyval_force_soft'), [])
        for flags in ['', '--cfg aes_force_soft', '--cfg polyval_force_soft',
                      '--cfg aes_force_soft --cfg polyval_force_soft --cfg aes_force_soft',
                      '--cfg aes_force_soft --cfg polyval_force_soft --cfg', '"']:
            self.assertTrue(self.flags(flags), flags)

    def test_codegen_cannot_enable_unpreserved_instructions(self):
        base = '--cfg aes_force_soft --cfg polyval_force_soft '
        for suffix in ['-Ctarget-feature=+sse2', '-C target-feature=+avx2',
                       '--codegen=target-feature=+aes', '--codegen target-feature=+pclmulqdq',
                       '-Ctarget-feature=-sse2,+sha', '-Ctarget-cpu=native']:
            self.assertTrue(self.flags(base + suffix), suffix)
        self.assertEqual(self.flags(base + '-Ctarget-feature=-sse2,-avx -Ctarget-cpu=generic'), [])

    def test_missing_duplicate_or_wrong_build_stage_rejected(self):
        spec = copy.deepcopy(SPEC)
        stage = next(s for s in spec['stage'] if s['id'] == 'build_kaleidoscope')
        stage['manifest'] = 'another/Cargo.toml'
        self.assertTrue(gate.check_flags(spec))
        spec['stage'].append(copy.deepcopy(stage))
        self.assertTrue(gate.check_flags(spec))
        spec['stage'] = []
        self.assertTrue(gate.check_flags(spec))

    def test_graph_parses_duplicate_rows_features_and_permitted_scalar_portable_metadata(self):
        errors, packages = gate.check_tree(TREE)
        self.assertEqual(errors, [])
        self.assertEqual(packages['rand_core']['versions'], ['0.6.4'])
        self.assertEqual(packages['sha2']['features'], ['force-soft'])
        self.assertEqual(packages['ppv-lite86']['features'], ['simd'])

    def test_each_effective_os_or_provider_package_rejected(self):
        for name in sorted(gate.BLOCKED | {'windows-sys', 'aws-lc-rs', 'openssl-src'}):
            self.assertTrue(gate.check_tree(TREE + f'\n{name} v1.0.0 ')[0], name)

    def test_effective_std_entropy_or_provider_features_rejected(self):
        for feature in ['std', 'getrandom', 'os_rng', 'system-verification', 'native-certs']:
            self.assertTrue(gate.check_tree(TREE + '\nnew-provider v1.0.0 ' + feature)[0], feature)
        for line in ['embedded-tls v0.19.0 alloc', 'rand_core v0.6.4 default',
                     'sha2 v0.10.9 asm', 'rand_chacha v0.3.1 std']:
            self.assertTrue(gate.check_tree(TREE + '\n' + line)[0], line)

    def test_empty_malformed_missing_or_multiple_versions_fail_closed(self):
        for tree in ['', TREE + '\nnot a cargo line', TREE.replace('embedded-tls v0.19.0', 'embedded-tls v0.18.0'),
                     TREE + '\nsha2 v0.11.0 force-soft', TREE.replace('sha2 v0.10.9 force-soft', 'sha2 v0.10.9')]:
            self.assertTrue(gate.check_tree(tree)[0], tree)

    def test_dynamic_cargo_command_uses_locked_baremetal_graph_and_repo_pin(self):
        with patch.dict(gate.os.environ, {'RUSTUP_TOOLCHAIN': 'wrong'}):
            with patch.object(gate.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, TREE)) as run:
                result = gate.inspect(ROOT)
        self.assertTrue(result['ok'], result)
        self.assertEqual(run.call_args.args[0], ['cargo', 'tree', '--locked', '-p', 'sex-tls', '--target',
                         str(ROOT / 'x86_64-sex.json'), '-e', 'normal,build', '--prefix', 'none', '--format', '{p} {f}'])
        self.assertEqual(run.call_args.kwargs['cwd'], ROOT)
        self.assertNotIn('RUSTUP_TOOLCHAIN', run.call_args.kwargs['env'])
        self.assertTrue(run.call_args.kwargs['check'])
        self.assertEqual(len(result['graph_sha256']), 64)

    def test_failed_actual_cargo_command_cannot_pass(self):
        with patch.object(gate.subprocess, 'run', side_effect=subprocess.CalledProcessError(1, ['cargo'])):
            self.assertFalse(gate.inspect(ROOT)['ok'])


if __name__ == '__main__':
    unittest.main()
