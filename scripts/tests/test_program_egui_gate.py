"""A delayed Display scanout must qualify pixels without restarting the gate."""
from pathlib import Path
import json
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import program_egui_gate as gate


def screen(coloured=False,outside=False):
    width,height=16,8
    pixels=bytearray(width*height*3)
    for y in range(height):
        for x in range(width):
            inside=2<=x<12 and 2<=y<7
            if coloured and inside or outside and not inside:
                at=(y*width+x)*3
                pixels[at:at+3]=bytes((x+y*width,1,2))
    return b'P6\n16 8\n255\n'+pixels


class DelayedCaptureTests(unittest.TestCase):
    def run_capture(self,path,frames,publication=lambda:True,timeout=3):
        class Qmp:
            count=0
            def cmd(self,name,args):
                self.count+=1
                assert name=='screendump'
                Path(args['filename']).write_bytes(frames[min(self.count-1,len(frames)-1)])
        q=Qmp();now=[0.]
        def pace(seconds):now[0]+=1
        with patch.object(gate.time,'monotonic',side_effect=lambda:now[0]),patch.object(gate.time,'sleep',side_effect=pace):
            result=gate.capture_pixels(q,path,'demo',(2,2,10,5),40,publication,timeout)
        return q,result

    def test_fresh_captures_wait_for_delayed_pixels_and_preserve_failed_images(self):
        with tempfile.TemporaryDirectory()as directory:
            path=Path(directory);q,result=self.run_capture(path,[screen(),screen(True)])
            self.assertEqual(q.count,2);self.assertEqual(len(result),50)
            self.assertEqual((path/'demo-capture-001.ppm').read_bytes(),screen())
            self.assertEqual((path/'demo.ppm').read_bytes(),screen(True))
            self.assertEqual(len(json.loads((path/'demo-captures.json').read_text())['attempts']),2)

    def test_colourful_unrelated_pixels_and_missing_publication_cannot_pass_deadline(self):
        for frames,published in [([screen(outside=True)],lambda:True),([screen(True)],lambda:False)]:
            with self.subTest(published=published()),tempfile.TemporaryDirectory()as directory:
                path=Path(directory)
                with self.assertRaisesRegex(AssertionError,'before deadline'):
                    self.run_capture(path,frames,published)
                report=json.loads((path/'demo-captures.json').read_text())
                self.assertFalse(report['ok']);self.assertEqual(len(report['attempts']),3)
                self.assertFalse((path/'demo.ppm').exists())

    def test_pixels_before_current_publication_require_another_fresh_capture(self):
        calls=[0]
        def published():
            calls[0]+=1
            return calls[0]>=2
        with tempfile.TemporaryDirectory()as directory:
            path=Path(directory);q,result=self.run_capture(path,[screen(True)],published)
            self.assertEqual(q.count,2);self.assertEqual(len(result),50)
            self.assertEqual([a['published']for a in json.loads((path/'demo-captures.json').read_text())['attempts']],[False,True])

    def test_client_geometry_comes_from_exact_full_viewport(self):
        self.assertEqual(gate.client_rect((0,28,1280,772),1264,730),(8,62,1264,730))
        with self.assertRaises(AssertionError):gate.client_rect((0,28,1280,772),550,144)


if __name__=='__main__':unittest.main()
