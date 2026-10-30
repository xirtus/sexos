import sys
import unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from check_document_snapshots import inspect
from test_document_views import trace as boot


def trace():
    return boot()+'''\n[files.document.snapshot.clear] caller=9 bytes=16384 scrubbed=1
[files.document.snapshot] caller=9 object=5 identity=393217 sequence=2 bytes=12287 published=1
[quil.document.snapshot] caller=9 object=5 identity=393217 sequence=2 bytes=12287 validated=1
[files.document.snapshot.clear] caller=9 bytes=16384 scrubbed=1
[files.app.cleanup] target=9 task=9 handles=0 caps=0 saved_data=retained selection=0
'''


class Snapshots(unittest.TestCase):
    def test_exact_join_and_terminal_scrub(self):
        self.assertTrue(inspect(trace())['ok'])

    def test_wrong_identity_peer_sequence_or_missing_export_fails(self):
        for before, after in [('quil.document.snapshot] caller=9', 'quil.document.snapshot] caller=16'),
                              ('bytes=12287 validated', 'bytes=12288 validated'),
                              ('sequence=2 bytes=12287 validated', 'sequence=3 bytes=12287 validated'),
                              ('identity=393217 sequence=2 bytes=12287 validated', 'identity=393218 sequence=2 bytes=12287 validated'),
                              ('files.document.snapshot]', 'missing]')]:
            self.assertFalse(inspect(trace().replace(before, after))['ok'])

    def test_duplicate_and_late_or_missing_scrub_fail(self):
        lines=trace().splitlines()
        for text in [trace()+'\n'+lines[-3], '\n'.join(lines[:-2]+[lines[-1]]),
                     '\n'.join(lines[:-2]+[lines[-1], lines[-2]])]:
            self.assertFalse(inspect(text)['ok'])

    def test_no_export_pending_load_close_still_requires_terminal_scrub(self):
        text=boot()+'\n[files.document.snapshot.clear] caller=9 bytes=16384 scrubbed=1\n[files.app.cleanup] target=9 task=9 handles=0 '
        self.assertTrue(inspect(text, (), 0)['ok'])
        self.assertFalse(inspect(text)['ok'])

    def test_reboot_copy_can_explicitly_omit_terminal_cleanup(self):
        text='\n'.join(trace().splitlines()[:-1])
        self.assertFalse(inspect(text)['ok'])
        self.assertTrue(inspect(text, require_cleanup=False)['ok'])


if __name__ == '__main__':
    unittest.main()
