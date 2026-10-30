import sys
import unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from check_document_views import inspect


def trace():
    lines = []
    for index, client in enumerate([9, 16, 18]):
        lines.extend([f'[boot.memory.region] pd=11 slot={64+index} region={6+index} bytes=16384 rights=7 peer={client}',
                      f'[boot.memory.region] pd={client} slot=64 region={6+index} bytes=16384 rights=3 peer=11'])
    lines.extend(['[boot.document.views] pairs=3 bytes_each=16384 total_bytes=49152 readonly=1 distinct=1 scrubbed=1 runtime_mint=0',
                  '[boot.memory.authority.sealed] runtime_mint=0'])
    return '\n'.join(lines)


class DocumentViews(unittest.TestCase):
    def test_exact_grants_and_native_selftest(self):
        self.assertTrue(inspect(trace())['ok'])

    def test_wrong_peer_rights_backing_budget_or_missing_scrub_fails(self):
        for before, after in [('peer=18', 'peer=16'), ('rights=3', 'rights=7'),
                              ('region=8', 'region=7'), ('49152', '32768'), ('scrubbed=1', 'scrubbed=0')]:
            self.assertFalse(inspect(trace().replace(before, after))['ok'])

    def test_missing_duplicate_late_or_faulted_evidence_fails(self):
        lines = trace().splitlines()
        for text in ['\n'.join(lines[1:]), trace()+'\n'+lines[0], '\n'.join([lines[-1], *lines[:-1]]),
                     trace()+'\n'+lines[-2], trace()+'\nKERNEL PAGE FAULT']:
            self.assertFalse(inspect(text)['ok'])


if __name__ == '__main__':
    unittest.main()
