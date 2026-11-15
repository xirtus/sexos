import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import cufflinks_idle_gate as g

BOOT = """[kernel.spawn.cufflinks] id=52 key=4 organ=senses
[boot.bt.hci] usb=5 cufflinks=52 files=11 slot=1003 bytes=131072 runtime_mint=0
[sexusb.bt.route] region=1
[cufflinks.start]
[cufflinks.transport] ok=1 slot=1003
[cufflinks.idle] controller=none
"""
QUIET = '[sched.active] tick=2000 second_distinct=4 window_peak=2 switches=40 domains=0x183e\n'
BUSY = f'[sched.active] tick=3000 second_distinct=5 window_peak=2 switches=90 domains={hex(0x183e | 1 << 52)}\n'


class CufflinksIdleGate(unittest.TestCase):
    def test_idle_boot_passes(self):
        fails, info = g.check(BOOT + QUIET * 5 + BUSY)
        self.assertEqual(fails, [])
        self.assertEqual((info['idle_seconds'], info['cufflinks_active_seconds']), (6, 1))

    def test_busy_cufflinks_fails(self):
        fails, _ = g.check(BOOT + BUSY * 3)
        self.assertTrue(any('sleep' in f for f in fails), fails)

    def test_missing_spawn_and_panic(self):
        fails, _ = g.check(BOOT.replace('[kernel.spawn.cufflinks]', '[x]') + '[cufflinks.panic] boom\n')
        self.assertIn('BOOT did not spawn Cufflinks', fails)
        self.assertIn('Cufflinks panicked', fails)

    def test_controller_present_needs_no_idle_line(self):
        text = BOOT.replace('[cufflinks.idle] controller=none\n', '') + '[sexusb.bt] vendor=0x8087 product=0x0032\n'
        self.assertEqual(g.check(text)[0], [])


if __name__ == '__main__':
    unittest.main()
