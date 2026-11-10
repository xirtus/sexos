"""Independent approved chrome goldens and complete-record serial controls."""
import sys
import unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
from browser_gate_viewport import chrome_metrics, content_rect

class ChromeMetrics(unittest.TestCase):
    def record(self, scale=1):
        return ('[display.chrome.metrics] scale=%d bar=%d title=%d inset=%d '
                'body_top=%d target=%d width=1280 height=800 publication=1' %
                (scale,28*scale,26*scale,8*scale,34*scale,24*scale))

    def test_requires_newline_complete_record(self):
        for ending in ('', '\r'):
            with self.assertRaises(AssertionError):
                chrome_metrics(self.record()+ending)
        for ending in ('\n', '\r\n'):
            self.assertEqual(chrome_metrics(self.record()+ending)['bar'],28)
        self.assertEqual(chrome_metrics(self.record()+'\n'+self.record(3))['scale'],1)
        for cut in range(len(self.record())):
            with self.assertRaises(AssertionError):
                chrome_metrics(self.record()[:cut])

    def test_independent_scaled_contract_and_body(self):
        for scale in (1,2,3):
            text=self.record(scale)+'\n'
            self.assertEqual(content_rect(text,(10,100,1000,600)),
                             (10+8*scale,100+34*scale,1000-16*scale,600-42*scale))
            for key,logical in (('bar',28),('title',26),('inset',8),('body_top',34),('target',24)):
                wrong=text.replace(f'{key}={logical*scale} ',f'{key}={logical*scale+1} ')
                with self.assertRaises(AssertionError):chrome_metrics(wrong)
        with self.assertRaises(AssertionError):chrome_metrics(self.record().replace('publication=1','publication=0')+'\n')

if __name__ == '__main__': unittest.main()
