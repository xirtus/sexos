"""Cold recovery rejects stale native receipts and broken live-disk ancestry."""
import contextlib
import gzip
import io
import json
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import pending_io_cold_gate as gate


def disk_image(payload):
    slot, start = 5, 2048
    identity = (slot + 1) << 16 | 3
    raw = bytearray((start + 128) * 512)
    entry = 2046 * 512 + 16 + slot * 32
    raw[entry] = 1
    struct.pack_into('<I', raw, entry + 24, 65536)
    struct.pack_into('<H', raw, entry + 30, 3)
    descriptor = (1925 - slot) * 512
    raw[descriptor:descriptor + 4] = b'SFEX'
    struct.pack_into('<H', raw, descriptor + 6, 1)
    struct.pack_into('<HH', raw, descriptor + 12, start, 128)
    header = bytearray(512)
    header[:4] = b'QJ02'
    header[4] = slot
    struct.pack_into('<I', header, 8, identity)
    struct.pack_into('<Q', header, 16, 1)
    struct.pack_into('<I', header, 24, len(payload))
    struct.pack_into('<Q', header, 32, gate.fnv(payload))
    struct.pack_into('<Q', header, 504, gate.fnv(header[:504]))
    raw[start * 512:start * 512 + 512] = header
    raw[start * 512 + 512:start * 512 + 512 + len(payload)] = payload
    return bytes(raw)


def recovered_metadata(payload=gate.BASELINE):
    return dict(identity=6 << 16 | 3, generation=1, bank=0, bytes=len(payload), hash=hex(gate.fnv(payload)))


def native(payload=gate.BASELINE, document=5):
    head = '[quil.incarnation.start] target=9 task=9 surface=156 fresh=1\n'
    span = ('[linen.open.queued] id=71 accepted=1\n'
            f'[linen.quil.disk_doc.intent] object_id=71 path_id={document} ok=1\n'
            '[linen.open.complete] status=0\n'
            f'[files.document.recovered] object={document} identity={6 << 16 | 3} generation=1 bank=0 bytes={len(payload)} hash={hex(gate.fnv(payload))}\n'
            f'[quil.persist.load.ok] bytes={len(payload)} hash={hex(gate.fnv(payload))} doc={document}\n')
    return head + span, len(head)


class ColdEvidence(unittest.TestCase):
    def test_exact_real_open_joins_both_legal_durable_outcomes(self):
        for payload in (gate.BASELINE, gate.SNAPSHOT):
            text, start = native(payload)
            result = gate.native_load(text, start, len(text), 5, payload, recovered_metadata(payload))
            self.assertEqual((result['target'], result['bytes']), (9, len(payload)))

    def test_growing_serial_reader_uses_one_snapshot_for_receipts_and_end(self):
        text, start = native()
        later = text + '[clock.source.snapshot] additional guest output\n'
        reader = Mock(side_effect=[text, later])
        # Reproduce the old two-read race without weakening native bounds.
        with self.assertRaisesRegex(ValueError, 'offsets outside serial log'):
            gate.native_load(text, start, len(later), 5, gate.BASELINE, recovered_metadata())
        observed = gate.capture_native_load(reader, start, 5, gate.BASELINE, recovered_metadata())
        reader.assert_called_once_with()
        self.assertEqual(observed['open_end'], len(text))
        self.assertEqual(observed, gate.native_load(text, start, len(text), 5, gate.BASELINE, recovered_metadata()))

    def test_stale_pre_action_load_is_not_cold_receipt(self):
        text, start = native()
        with self.assertRaises(ValueError):
            gate.native_load(text, len(text) - 1, len(text), 5, gate.BASELINE, recovered_metadata())
        duplicate = text + text[start:]
        with self.assertRaises(ValueError):
            gate.native_load(duplicate, start, len(duplicate), 5, gate.BASELINE, recovered_metadata())

    def test_wrong_object_payload_incarnation_and_missing_completion_rejected(self):
        text, start = native()
        bad = [text.replace('path_id=5', 'path_id=6'),
               text.replace('doc=5', 'doc=6'), text.replace('bytes=12286', 'bytes=12287'),
               text.replace('target=9 task=9', 'target=16 task=16'),
               text.replace('[linen.open.complete] status=0\n', '')]
        for value in bad:
            with self.subTest(value=value[-100:]), self.assertRaises(ValueError):
                gate.native_load(value, start, len(value), 5, gate.BASELINE, recovered_metadata())

    def test_load_before_open_intent_rejected(self):
        text, start = native()
        lines = text[start:].splitlines(keepends=True)
        changed = text[:start] + ''.join([lines[0], lines[4], lines[1], lines[2], lines[3]])
        with self.assertRaises(ValueError):
            gate.native_load(changed, start, len(changed), 5, gate.BASELINE, recovered_metadata())

    def test_wrong_selected_row_cannot_borrow_another_rows_load(self):
        text, start = native()
        changed = text.replace('object_id=71', 'object_id=72')
        with self.assertRaisesRegex(ValueError, 'row/path identity'):
            gate.native_load(changed, start, len(changed), 5, gate.BASELINE, recovered_metadata())

    def test_duplicate_queued_intent_complete_and_load_are_rejected(self):
        text, start = native()
        lines = text[start:].splitlines(keepends=True)
        for index in range(5):
            changed = text[:start] + ''.join(lines[:index] + [lines[index]] + lines[index:])
            with self.subTest(index=index), self.assertRaises(ValueError):
                gate.native_load(changed, start, len(changed), 5, gate.BASELINE, recovered_metadata())
        # Also reject a duplicate malformed/failed record instead of skipping it.
        changed = text + '[linen.open.complete] status=7\n'
        with self.assertRaises(ValueError):
            gate.native_load(changed, start, len(changed), 5, gate.BASELINE, recovered_metadata())

    def test_completion_before_intent_is_rejected(self):
        text, start = native()
        lines = text[start:].splitlines(keepends=True)
        changed = text[:start] + ''.join([lines[0], lines[2], lines[1], lines[3], lines[4]])
        with self.assertRaises(ValueError):
            gate.native_load(changed, start, len(changed), 5, gate.BASELINE, recovered_metadata())
        # Exact originally reported counterexample: wrong row plus early completion.
        changed = changed.replace('object_id=71', 'object_id=72')
        with self.assertRaises(ValueError):
            gate.native_load(changed, start, len(changed), 5, gate.BASELINE, recovered_metadata())

    def test_async_load_can_precede_completion_after_exact_intent(self):
        text, start = native()
        lines = text[start:].splitlines(keepends=True)
        changed = text[:start] + ''.join([lines[0], lines[1], lines[3], lines[4], lines[2]])
        self.assertEqual(gate.native_load(changed, start, len(changed), 5, gate.BASELINE, recovered_metadata())['bytes'], len(gate.BASELINE))

    def test_provider_journal_identity_generation_bank_and_receipt_uniqueness(self):
        text, start = native()
        receipt = next(line for line in text.splitlines(keepends=True) if line.startswith('[files.document.recovered]'))
        bad = [text.replace(receipt, receipt.replace('identity=393219', 'identity=393220')),
               text.replace(receipt, receipt.replace('generation=1', 'generation=2')),
               text.replace(receipt, receipt.replace('bank=0', 'bank=1')),
               text.replace(receipt, receipt.replace('object=5', 'object=6')),
               text.replace(receipt, receipt.replace('hash=0x', 'hash=bad0x')),
               text.replace(receipt, receipt + receipt), text.replace(receipt, '')]
        for changed in bad:
            with self.subTest(receipt=changed[-250:]), self.assertRaises(ValueError):
                gate.native_load(changed, start, len(changed), 5, gate.BASELINE, recovered_metadata())
        lines = text[start:].splitlines(keepends=True)
        changed = text[:start] + ''.join([lines[0], lines[1], lines[2], lines[4], lines[3]])
        with self.assertRaises(ValueError):
            gate.native_load(changed, start, len(changed), 5, gate.BASELINE, recovered_metadata())

    def test_equal_payload_different_on_disk_metadata_is_not_readonly_recovery(self):
        original = disk_image(gate.BASELINE)
        expected = recovered_metadata()
        self.assertEqual(gate.unchanged_document(original, 5, gate.BASELINE, expected), expected)
        for mutation in ('identity', 'generation', 'bank'):
            changed = bytearray(original)
            offset = 2048 * 512
            if mutation == 'generation':
                struct.pack_into('<Q', changed, offset + 16, 2)
            elif mutation == 'identity':
                struct.pack_into('<H', changed, 2046 * 512 + 16 + 5 * 32 + 30, 4)
                struct.pack_into('<I', changed, offset + 8, 6 << 16 | 4)
            struct.pack_into('<Q', changed, offset + 504, gate.fnv(changed[offset:offset + 504]))
            if mutation == 'bank':
                changed[offset + 32768:offset + 65536] = changed[offset:offset + 32768]
                changed[offset:offset + 32768] = bytes(32768)
            self.assertEqual(gate.recover(changed, 5)['payload'], gate.BASELINE)
            with self.subTest(mutation=mutation), self.assertRaisesRegex(ValueError, 'metadata/payload'):
                gate.unchanged_document(changed, 5, gate.BASELINE, expected)

    def session(self, out, payload=gate.BASELINE):
        seed = disk_image(payload)
        found = gate.recover(seed, 5)
        found.pop('payload')
        (out / 'nvme.img.gz').write_bytes(gzip.compress(seed))
        (out / 'manifest.json').write_text(json.dumps(dict(options=dict(pending_io_close=True), final_disk_sha256=gate.digest(seed))))
        (out / 'result.json').write_text(json.dumps(dict(pending_io_close=dict(document=5, terminal_document=found))))
        return seed

    def test_live_component_replayed_before_accepting_exact_terminal_seed(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            seed = self.session(out)
            with patch.object(gate.alpha, 'replay') as replay:
                observed, payload, chain = gate.accepted_session(out, out/'iso', out/'binding', out)
            replay.assert_called_once()
            self.assertEqual(observed, seed)
            self.assertEqual(payload, gate.BASELINE)
            self.assertEqual(chain['seed_sha256'], gate.digest(seed))
            with patch.object(gate.alpha, 'replay', side_effect=ValueError('live component failed')):
                with self.assertRaisesRegex(ValueError, 'live component failed'):
                    gate.accepted_session(out, out/'iso', out/'binding', out)

    def test_foreign_seed_hash_and_non_snapshot_bytes_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            self.session(out)
            m = json.loads((out/'manifest.json').read_text())
            m['final_disk_sha256'] = '0' * 64
            (out/'manifest.json').write_text(json.dumps(m))
            with patch.object(gate.alpha, 'replay'):
                with self.assertRaisesRegex(ValueError, 'seed hash'):
                    gate.accepted_session(out, out/'iso', out/'binding', out)
                self.session(out, gate.BASELINE + b'z')
                with self.assertRaisesRegex(ValueError, 'baseline or snapshot'):
                    gate.accepted_session(out, out/'iso', out/'binding', out)

    def test_no_pending_component_cannot_be_relabelled_cold(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            self.session(out)
            (out/'result.json').write_text('{}')
            with patch.object(gate.alpha, 'replay'), self.assertRaisesRegex(ValueError, 'pending-I/O component'):
                gate.accepted_session(out, out/'iso', out/'binding', out)

    def test_check_never_launches_or_writes_artifacts(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            for name in ('iso', 'binding'):
                (out/name).write_text('held')
            args = ['--iso', str(out/'iso'), '--binding', str(out/'binding'), '--session', str(out), '--out', str(out), '--check']
            before = {p.name: p.read_bytes() for p in out.iterdir()}
            with patch.object(gate, 'replay', return_value=dict(ok=True)) as replay, patch.object(gate, 'run') as run, patch.object(gate.subprocess, 'Popen') as popen, contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(gate.main(args), dict(ok=True))
            replay.assert_called_once()
            run.assert_not_called()
            popen.assert_not_called()
            self.assertEqual(before, {p.name: p.read_bytes() for p in out.iterdir()})

    def test_cold_command_has_one_unthrottled_owned_disk_no_trace(self):
        command, sock = gate.guest_command(Path('/held.iso'), Path('/owned'), '1024x768', 'e1000e')
        self.assertEqual(command.count('-qmp'), 1)
        self.assertEqual(command.count('nvme,serial=sexos01,drive=nvm'), 1)
        self.assertIn('if=none,id=nvm,file=/owned/nvme.img,format=raw', command)
        self.assertNotIn('-D', command)
        self.assertNotIn('throttle', ' '.join(command))
        self.assertIn('VGA,xres=1024,yres=768', command)

    def test_verification_file_excluded_but_other_evidence_closed(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            (out/'serial.log').write_text('raw')
            first = gate.raw_files(out)
            (out/'verification.json').write_text('untrusted display cache')
            self.assertEqual(first, gate.raw_files(out))
            (out/'serial.log').write_text('changed')
            self.assertNotEqual(first, gate.raw_files(out))


if __name__ == '__main__':
    unittest.main()
