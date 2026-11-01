import sys
import unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from check_document_views import inspect


QUIL_CLIENTS = (9,16,18,19,20,21,22,23,32,33,34,35,36,37,38,39)
REGIONS = (6,7,8,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8203,8204)
BROWSER_CLIENTS = (15,30,31)


def trace():
    lines = []
    for index, client in enumerate(QUIL_CLIENTS):
        lines.extend([f'[boot.memory.region] pd=11 slot={64+index} region={REGIONS[index]} bytes=16384 rights=7 peer={client}',
                      f'[boot.memory.region] pd={client} slot=64 region={REGIONS[index]} bytes=16384 rights=3 peer=11'])
    lines.extend(['[boot.document.views] pairs=16 bytes_each=16384 total_bytes=262144 readonly=1 distinct=1 scrubbed=1 runtime_mint=0',
                  '[boot.memory.authority.sealed] runtime_mint=0'])
    return '\n'.join(lines)


class DocumentViews(unittest.TestCase):
    def test_exact_grants_and_native_selftest(self):
        result=inspect(trace());self.assertTrue(result['ok'],result)
        self.assertEqual(len(result['grants']),32)
        self.assertEqual(result['grants'][-2:],[(11,79,8204,16384,7,39),(39,64,8204,16384,3,11)])

    def test_wrong_peer_rights_backing_budget_or_missing_scrub_fails(self):
        for before, after in [('peer=18', 'peer=16'), ('rights=3', 'rights=7'),
                              ('region=8', 'region=7'), ('262144', '131072'), ('scrubbed=1', 'scrubbed=0')]:
            self.assertFalse(inspect(trace().replace(before, after))['ok'])

    def test_each_added_buffer_is_distinct_and_browser_participants_have_no_snapshot_grant(self):
        for index in range(8,16):
            row=f'[boot.memory.region] pd=11 slot={64+index} region={REGIONS[index]} bytes=16384 rights=7 peer={QUIL_CLIENTS[index]}'
            for replacement in ('',row.replace(f'region={REGIONS[index]}','region=8196'),row.replace('rights=7','rights=3')):
                with self.subTest(index=index,replacement=replacement):self.assertFalse(inspect(trace().replace(row,replacement))['ok'])
        # All19 document participants include three Browser scalar clients, but
        # only16 Quil incarnations own these permanent snapshot Region views.
        self.assertEqual(len(QUIL_CLIENTS+BROWSER_CLIENTS),19)
        for client in BROWSER_CLIENTS:
            self.assertFalse(inspect(trace()+f'\n[boot.memory.region] pd={client} slot=64 region=8204 bytes=16384 rights=3 peer=11')['ok'])

    def test_missing_duplicate_late_or_faulted_evidence_fails(self):
        lines = trace().splitlines()
        for text in ['\n'.join(lines[1:]), trace()+'\n'+lines[0], '\n'.join([lines[-1], *lines[:-1]]),
                     trace()+'\n'+lines[-2], trace()+'\nKERNEL PAGE FAULT']:
            self.assertFalse(inspect(text)['ok'])


if __name__ == '__main__':
    unittest.main()
