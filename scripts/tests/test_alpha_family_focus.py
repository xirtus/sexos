"""Late-reserve captures require current family status and every focus-dot pixel."""
import sys
import unittest
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import alpha_evidence as gate
from app_switch_gate import indicator_colors


class FamilyFocus(unittest.TestCase):
    def ppm(self,color):
        pixels=bytearray(320*100*3)
        for y in range(22,26):
            for x in range(155,159):pixels[(y*320+x)*3:(y*320+x)*3+3]=color
        return b'P6\n320 100\n255\n'+pixels

    def check(self,prefix,pixels):
        c=dict(font=None,bar=None,reserve_focus={23:312},indicators=indicator_colors(Path(__file__).resolve().parents[2]))
        with patch.object(gate,'quil_pixels',return_value=[]),patch.object(gate,'quil_view',return_value={}),patch.object(gate,'bar_pixels',return_value=dict(ok=True,errors=[])):
            return gate.verify_frame(prefix,pixels,c,(320,100),'quil','t#',23)

    def test_current_family_status_and_full_dot_join(self):
        prefix='[shell.focus.set] id=312\n[shell.silkbar.status.send] focus=312 app=Quil tint=0\n'
        result=self.check(prefix,self.ppm(b'\xcb\xa6\xf7'))
        self.assertTrue(result['ok']);self.assertEqual(result['native_family'],dict(focus=312,app='Quil',indicator='0xcba6f7'))
        for bad in ['',prefix.replace('app=Quil','app=App'),prefix+'[shell.focus.set] id=300\n',prefix+'[shell.silkbar.status.send] focus=300 app=Browser tint=0\n']:
            with self.subTest(bad=bad):self.assertFalse(self.check(bad,self.ppm(b'\xcb\xa6\xf7'))['ok'])
        self.assertFalse(self.check(prefix,self.ppm(b'\x44\x44\x44'))['ok'])
        bad=bytearray(self.ppm(b'\xcb\xa6\xf7'));bad[len(b'P6\n320 100\n255\n')+(25*320+158)*3]^=1
        self.assertFalse(self.check(prefix,bad)['ok'])


if __name__=='__main__':unittest.main()
