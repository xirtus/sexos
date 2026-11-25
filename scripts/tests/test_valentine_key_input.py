import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from valentine_key_input import send_text


class FakeQemu:
    def __init__(self):
        self.events = []

    def key(self, key):
        self.events.append(('key', key))

    def cmd(self, command, payload):
        self.events.append(('cmd', command, payload))


class ValentineKeyInputTests(unittest.TestCase):
    def test_url_colons_use_shift_semicolon(self):
        q = FakeQemu()
        send_text(q, 'http://10.0.2.2:41567/inbox.eml', delay=0)
        shifted = [event[2] for event in q.events if event[0] == 'cmd']
        self.assertEqual(len(shifted), 2)
        for payload in shifted:
            self.assertEqual(payload['keys'], [
                {'type': 'qcode', 'data': 'shift'},
                {'type': 'qcode', 'data': 'semicolon'},
            ])
        self.assertNotIn(('key', 'semicolon'), q.events)

    def test_mail_address_at_sign_uses_shift_two(self):
        q = FakeQemu()
        send_text(q, 'bob@example.org', delay=0)
        self.assertIn(('cmd', 'send-key', {'keys': [
            {'type': 'qcode', 'data': 'shift'},
            {'type': 'qcode', 'data': '2'},
        ], 'hold-time': 80}), q.events)


if __name__ == '__main__':
    unittest.main()
