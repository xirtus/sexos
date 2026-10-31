"""Exact complete-reserve ancestry and real journal decoding, no guest doubles."""
import copy
import sys
import unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import quil_reserve_cold_gate as gate
from test_pending_io_cold_gate import disk_image


class ReserveCold(unittest.TestCase):
    def fixture(self):
        payload = b'first noteqqtttttt'
        seed = disk_image(payload)
        rows = gate.quil_catalog(Path(__file__).resolve().parents[2])
        saved = dict(object=5, content=payload.decode(), bytes=len(payload), hash=hex(gate.cold.fnv(payload)))
        third = dict(catalog=copy.deepcopy(rows), final_target=23, append_count=6,
                     chain=[dict(target=target, saved=saved) for target in [18, 19, 20, 21, 22, 23]])
        return seed, dict(options=dict(third_quil=True), final_disk_sha256=gate.cold.digest(seed)), dict(ok=True, soak=dict(third_quil=third)), rows

    def test_exact_last_save_is_joined_to_real_terminal_journal(self):
        args = self.fixture()
        seed, payload, chain = gate.terminal_document(*args, '/tmp/sealed-parent')
        self.assertEqual(seed, args[0])
        self.assertEqual(chain['targets'], [9, 16, 18, 19, 20, 21, 22, 23])
        self.assertEqual(chain['final_target'], 23)
        self.assertEqual(chain['recovered']['identity'], 6 << 16 | 3)
        self.assertEqual(gate.POLICY.expected(payload), payload[-8:].decode())

    def test_skipped_forged_or_incomplete_chain_and_payload_rejected(self):
        for change in ['skipped', 'target', 'catalog', 'count', 'last', 'content', 'hash', 'seed', 'not_complete']:
            with self.subTest(change=change):
                seed, manifest, result, rows = copy.deepcopy(self.fixture())
                third = result['soak']['third_quil']
                if change == 'skipped': third['chain'].pop(2)
                if change == 'target': third['chain'][-1]['target'] = 22
                if change == 'catalog': third['catalog'][-1]['target'] = 24
                if change == 'count': third['append_count'] = 5
                if change == 'last': third['final_target'] = 22
                if change == 'content': third['chain'][-1]['saved']['content'] += 'z'
                if change == 'hash': third['chain'][-1]['saved']['hash'] = '0x0'
                if change == 'seed': manifest['final_disk_sha256'] = '0' * 64
                if change == 'not_complete': manifest['options']['third_quil'] = False
                with self.assertRaises(ValueError): gate.terminal_document(seed, manifest, result, rows, '/tmp/parent')


if __name__ == '__main__':
    unittest.main()
