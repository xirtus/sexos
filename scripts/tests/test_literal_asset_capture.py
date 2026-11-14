"""Binary inputs to literal Rust includes participate in sealed source binding."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
from freeze_native_evidence import freeze


def capture_code():
    script = (ROOT / 'scripts/master_runtime_gate.sh').read_text()
    return script.split("<<'PYSOURCES'\n", 1)[1].split('\nPYSOURCES', 1)[0]


def capture(root, output):
    return subprocess.run([sys.executable, '-c', capture_code(), str(output)],
                          cwd=root, capture_output=True, text=True, timeout=30)


def fixture(root, source):
    (root / 'kernel/src').mkdir(parents=True)
    (root / 'kernel/src/main.rs').write_text(source)
    (root / 'Cargo.toml').write_text('[workspace]\nmembers=["kernel"]\nresolver="2"\n')
    (root / 'kernel/Cargo.toml').write_text('[package]\nname="asset-fixture"\nversion="0.0.0"\nedition="2021"\n')
    (root / 'Cargo.lock').write_text('version=4\n[[package]]\nname="asset-fixture"\nversion="0.0.0"\n')
    (root / 'assets').mkdir()
    (root / 'assets/pixels.bin').write_bytes(b'\x00\x01\x80\xff')


class LiteralAssetCaptureTests(unittest.TestCase):
    def test_quoted_and_raw_literals_are_bound_and_generated_forms_are_explicitly_outside_scope(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture(root, 'const A:&[u8]=include_bytes!("../../assets/pixels.bin");\n'
                          'const B:&str=include_str ! {r#"../../assets/label.txt"#};\n'
                          'const C:&[u8]=include_bytes!["../../assets/pixels.bin",];\n'
                          'const G:&[u8]=include_bytes!(concat!(env!("OUT_DIR"),"/generated.bin"));\n')
            (root / 'assets/label.txt').write_text('real included text')
            result = capture(root, root / 'manifest.json')
            self.assertEqual(result.returncode, 0, result.stderr)
            data = json.loads((root / 'manifest.json').read_text())
            for name in ('assets/pixels.bin', 'assets/label.txt'):
                self.assertEqual(data[name], hashlib.sha256((root / name).read_bytes()).hexdigest())
            self.assertFalse(any('generated.bin' in name for name in data))

    def test_missing_absolute_external_symlink_and_escaped_literals_fail_closed(self):
        with tempfile.TemporaryDirectory() as directory, tempfile.TemporaryDirectory() as outside:
            root = Path(directory)
            fixture(root, '')
            external = Path(outside) / 'private.bin'
            external.write_bytes(b'must never be captured')
            (root / 'assets/link.bin').symlink_to(external)
            # This lexical filename exists, but Rust's escaped literal denotes
            # pixels.bin instead. Never hash the wrong apparent path.
            (root / 'assets' / r'\x70ixels.bin').write_bytes(b'wrong lexical target')
            for argument in ('"../../assets/missing.bin"', json.dumps(str(external)),
                             '"../../assets/link.bin"', '"../../assets/\\x70ixels.bin"'):
                with self.subTest(argument=argument):
                    (root / 'kernel/src/main.rs').write_text(f'const A:&[u8]=include_bytes!({argument});')
                    result = capture(root, root / 'manifest.json')
                    self.assertNotEqual(result.returncode, 0)
                    self.assertFalse((root / 'manifest.json').exists())

    def test_internal_symlink_alias_is_frozen_under_the_original_include_path(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture(root, 'const A:&[u8]=include_bytes!("../../assets/./alias.bin");'
                          'fn main(){assert_eq!(A,&[0u8,1,128,255]);}')
            (root / 'assets/alias.bin').symlink_to('pixels.bin')
            result = capture(root, root / 'capture.json')
            self.assertEqual(result.returncode, 0, result.stderr)
            captured = json.loads((root / 'capture.json').read_text())
            self.assertIn('assets/alias.bin', captured)
            self.assertNotIn('assets/pixels.bin', captured)
            self.assertFalse(any('/./' in name or '/../' in name for name in captured))
            build = root / 'build'
            build.mkdir()
            image, kernel = build / 'image.iso', build / 'kernel'
            image.write_bytes(b'host fixture, not bootable')
            kernel.write_bytes(b'host kernel fixture')
            (build / 'provenance.json').write_text(json.dumps(dict(
                build_gate='PASS', source_image_binding='sealed_build_this_run',
                sources_unchanged_during_build=True, production_source_sha256=captured,
                iso_sha256=hashlib.sha256(image.read_bytes()).hexdigest(),
                kernel_sha256=hashlib.sha256(kernel.read_bytes()).hexdigest())))
            (build / 'report.md').write_text('| **FINAL_SCORE** | **GREEN_MASTER** |')
            frozen = root / 'frozen'
            freeze(root, build, frozen, image, kernel)
            alias = frozen / 'source/assets/alias.bin'
            self.assertTrue(alias.is_file())
            self.assertFalse(alias.is_symlink())
            self.assertFalse((frozen / 'source/assets/pixels.bin').exists())
            executable = root / 'frozen-probe'
            subprocess.run(['rustc', '--edition=2021', str(frozen / 'source/kernel/src/main.rs'),
                            '-o', str(executable)], check=True, capture_output=True, timeout=60)
            subprocess.run([str(executable)], check=True, capture_output=True, timeout=10)

    def test_single_asset_size_is_checked_before_hashing(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture(root, 'const A:&[u8]=include_bytes!("../../assets/pixels.bin");')
            with (root / 'assets/pixels.bin').open('wb') as stream:
                stream.truncate(64 * 1024 * 1024 + 1)
            result = capture(root, root / 'manifest.json')
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('literal Rust asset closure exceeds bound', result.stderr)
            self.assertFalse((root / 'manifest.json').exists())

    def test_binary_asset_mutation_during_build_prevents_boot_and_binding(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture(root, 'const A:&[u8]=include_bytes!("../../assets/pixels.bin");')
            scripts = root / 'scripts'
            scripts.mkdir()
            shutil.copyfile(ROOT / 'scripts/master_runtime_gate.sh', scripts / 'master_runtime_gate.sh')
            entry = scripts / 'entrypoint_build.sh'
            entry.write_text('#!/usr/bin/env python3\nfrom pathlib import Path\n'
                             'Path("assets/pixels.bin").write_bytes(b"changed during build")\n'
                             'Path("sexos-v1.0.0.iso").write_bytes(b"untrusted fixture image")\n')
            entry.chmod(0o755)
            qemu = root / 'fake-qemu'
            qemu.write_text('#!/bin/sh\ntouch should-not-boot\n')
            qemu.chmod(0o755)
            gate = root / 'gate'
            result = subprocess.run(['bash', str(scripts / 'master_runtime_gate.sh'), '--probe', '1'],
                                    cwd=root, env=dict(os.environ, GATE_DIR=str(gate), QEMU_BIN=str(qemu),
                                                       ISO='sexos-v1.0.0.iso'),
                                    capture_output=True, text=True, timeout=15)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((root / 'should-not-boot').exists())
            before = json.loads((gate / 'build-sources.before.json').read_text())
            after = json.loads((gate / 'build-sources.after.json').read_text())
            self.assertNotEqual(before['assets/pixels.bin'], after['assets/pixels.bin'])
            provenance = json.loads((gate / 'provenance.json').read_text())
            self.assertEqual(provenance['build_gate'], 'FAIL')
            self.assertEqual(provenance['probe_gate'], 'NOT_RUN')
            self.assertIsNone(provenance['iso_sha256'])
            with self.assertRaises(ValueError):
                freeze(root, gate, root / 'forbidden-frozen', root / 'sexos-v1.0.0.iso', qemu)

    def test_actual_fonts_are_captured_and_preserved_in_frozen_resources(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = Path(directory)
            result = capture(ROOT, temporary / 'actual-capture.json')
            self.assertEqual(result.returncode, 0, result.stderr)
            captured = json.loads((temporary / 'actual-capture.json').read_text())
            fonts = {name: digest for name, digest in captured.items() if name.startswith('assets/fonts/')}
            for name in ('assets/fonts/geist/GeistMono-Regular.ttf', 'assets/fonts/geist/Geist-Regular.ttf'):
                self.assertIn(name, fonts)
                self.assertEqual(fonts[name], hashlib.sha256((ROOT / name).read_bytes()).hexdigest())
            self.assertGreater(len(fonts), 2)  # Retain fallback font inputs too.
            # A simulated build fixture isolates freeze's copying of these exact
            # real resources. This is a host test, not guest-build provenance.
            root = temporary / 'fixture'
            for name in fonts:
                target = root / name
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / name, target)
            build = root / 'build'
            build.mkdir()
            image, kernel = build / 'image.iso', build / 'kernel'
            image.write_bytes(b'host fixture; not bootable')
            kernel.write_bytes(b'host kernel fixture')
            (build / 'provenance.json').write_text(json.dumps(dict(
                build_gate='PASS', source_image_binding='sealed_build_this_run',
                sources_unchanged_during_build=True, production_source_sha256=fonts,
                iso_sha256=hashlib.sha256(image.read_bytes()).hexdigest(),
                kernel_sha256=hashlib.sha256(kernel.read_bytes()).hexdigest())))
            (build / 'report.md').write_text('| **FINAL_SCORE** | **GREEN_MASTER** |')
            frozen = root / 'frozen'
            freeze(root, build, frozen, image, kernel)
            for name, digest in fonts.items():
                self.assertEqual(hashlib.sha256((frozen / 'source' / name).read_bytes()).hexdigest(), digest)


if __name__ == '__main__':
    unittest.main()
