"""The release Vellum ELF must exist before Files pins it."""
import tomllib
import unittest
import hashlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

class VellumImageStage(unittest.TestCase):
    def test_release_artifact_is_a_sealed_files_input(self):
        spec = tomllib.loads((ROOT / 'sexos_build_spec.toml').read_text())
        stages = spec['stage']
        pdf = [(i, s) for i, s in enumerate(stages) if s['id'] == 'build_pdf']
        files = [(i, s) for i, s in enumerate(stages) if s['id'] == 'build_sexfiles']
        shell = [(i, s) for i, s in enumerate(stages) if s['id'] == 'build_silk_shell']
        self.assertEqual((len(pdf), len(files), len(shell)), (1, 1, 1))
        index, built = pdf[0]
        self.assertLess(index, files[0][0])
        self.assertLess(index, shell[0][0])
        self.assertEqual(built['manifest'], 'apps/pdf/Cargo.toml')
        self.assertEqual(built['dest_artifact'], 'iso_root/programs/vellum')
        self.assertIn(built['dest_artifact'], files[0][1]['consumed_inputs'])

    def test_replaced_or_renamed_payload_cannot_reuse_release_digest(self):
        release = b'\x7fELF\x02\x01\x01' + bytes(range(57))
        approved = hashlib.sha256(release).digest()
        replacement = bytearray(release)
        replacement[-1] ^= 1
        self.assertNotEqual(hashlib.sha256(replacement).digest(), approved)
        self.assertEqual(hashlib.sha256(release).digest(), approved)
        # Keep the production check tied to image bytes, not the requested path.
        files = (ROOT / 'servers/sexfiles/src/program_store.rs').read_text()
        broker = (ROOT / 'servers/silk-shell/src/pdf_sign.rs').read_text()
        self.assertIn('approved_image(&image[..len as usize])', files)
        self.assertIn('approved_image(&image[..entry.len as usize])', files)
        self.assertIn('op == p::BEGIN && !approved(caller)', broker)

if __name__ == '__main__': unittest.main()
