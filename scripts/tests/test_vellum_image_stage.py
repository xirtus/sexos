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

    def test_shared_image_is_fenced_through_kernel_copy(self):
        files = (ROOT / 'servers/sexfiles/src/program_store.rs').read_text()
        lease = (ROOT / 'servers/sexfiles/src/restricted_image.rs').read_text()
        shell = (ROOT / 'servers/silk-shell/src/main.rs').read_text()
        dispatch = (ROOT / 'servers/sexfiles/src/trampoline.rs').read_text()
        # Recorder's queued load must hit the same writer fence as a signed run.
        self.assertIn('writer.seal_ordinary(shell_epoch, files_epoch)', files)
        self.assertIn('self.ordinary.is_some()', lease)
        self.assertIn('ordinary_consumed(msg.arg0 as u32,msg.arg1)', dispatch)
        self.assertIn('let copied_epoch=program_files_epoch();', shell)
        self.assertIn('program_image_consumed(generation,image_consume_epoch(requested_epoch,copied_epoch));', shell)
        self.assertIn('image_approval_after_copy(approved,generation,requested_epoch,copied_epoch)', shell)
        self.assertIn('ordinary_recover(peer.epoch, peer.life == PeerLife::Dead)', lease)

if __name__ == '__main__': unittest.main()
