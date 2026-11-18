#!/usr/bin/env python3
"""SerialText.since(start) is text()[start:] at every offset, while the log grows in
uneven pieces, partial last lines included; and a poll reads only the new chunks."""
import random, sys, tempfile, unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from quil_autosave_gate import SerialText  # noqa: E402


class SerialTextTest(unittest.TestCase):
    def test_since_matches_text_slices(self):
        rng = random.Random(7)
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / 'serial.log'
            log, want = SerialText(path), ''
            self.assertEqual(log.text(), '')
            for _ in range(200):
                piece = ''.join(rng.choice('ab\né') for _ in range(rng.randrange(0, 40)))
                with path.open('ab') as f:
                    f.write(piece.encode())
                want += piece
                for start in {0, len(want), rng.randrange(0, len(want) + 1), max(0, len(want) - 3)}:
                    self.assertEqual(log.since(start), want[start:])
                self.assertEqual(log.text(), want)

    def test_missing_file_is_empty(self):
        with tempfile.TemporaryDirectory() as d:
            log = SerialText(Path(d) / 'none.log')
            self.assertEqual(log.text(), '')
            self.assertEqual(log.since(0), '')

    def test_poll_does_not_join_the_whole_log(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / 'serial.log'
            path.write_text('x' * 1000 + '\n')
            log = SerialText(path)
            start = len(log.text())
            with path.open('a') as f:
                f.write('[done] 1\n')
            self.assertEqual(log.since(start), '[done] 1\n')
            self.assertEqual(log.joined, 1)  # the poll did not rebuild the whole text


if __name__ == '__main__':
    unittest.main()
