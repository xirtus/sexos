import contextlib
import io
from pathlib import Path
import sys
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import spindle_viewport_gate as gate


class RelativeTiles(unittest.TestCase):
    resolution=(1024,768)
    # Independent current compact geometry/capacity, not imported from layout().
    tiled=[1,512,359,18,6,6]
    expanded=[2,1024,718,20,6,6]

    def check(self,record,phase='tiled',tiled=None,after=0,resolution=None):
        return gate.validate_relative_layout(record,resolution or self.resolution,phase,tiled,after)

    def test_basic_compact_edit_zoom_and_exact_restore(self):
        tiled=self.check(self.tiled)
        self.assertEqual(tiled,(512,359,18,6,6))
        self.assertEqual(self.check(self.tiled,tiled=tiled),tiled) # input edit need not change viewport sequence
        self.assertEqual(self.check(self.expanded,'expanded',tiled,1),(1024,718,20,6,6))
        self.assertEqual(self.check([3,512,359,18,6,6],'restored',tiled,2),tiled)

    def test_wide_tiled_height375_is_modern_bounded_layout(self):
        tiled=self.check([4,640,375,20,6,6],resolution=(1280,800))
        self.assertEqual(tiled,(640,375,20,6,6))
        self.assertEqual(self.check([5,1280,750,20,6,6],'expanded',tiled,4,(1280,800)),(1280,750,20,6,6))
        self.assertEqual(self.check([6,640,375,20,6,6],'restored',tiled,5,(1280,800)),tiled)

    def test_initial_requires_real_bounded_tile_and_valid_framebuffer(self):
        for record in ([1,1024,718,20,6,6],[1,1025,359,18,6,6],[1,512,719,18,6,6],
                       [1,0,359,18,6,6],[1,512,-1,18,6,6]):
            with self.subTest(record=record),self.assertRaises(ValueError):self.check(record)
        for resolution in ((800,600),(1280,768),(1024,800)):
            with self.subTest(resolution=resolution),self.assertRaises(ValueError):self.check(self.tiled,resolution=resolution)

    def test_malformed_sequence_and_text_capacity_refused(self):
        for record in (None,[],self.tiled[:5],self.tiled+[0],[True,512,359,18,6,6],
                       [0,512,359,18,6,6],[1<<64,512,359,18,6,6],[1,'512',359,18,6,6],
                       [1,512,359,0,6,6],[1,512,359,21,6,6],[1,512,359,18,7,6],
                       [1,512,359,18,6,0],[1,512,359,18,6,7]):
            with self.subTest(record=record),self.assertRaises(ValueError):self.check(record)
        for after in (1,2,-1,True):
            with self.subTest(after=after),self.assertRaises(ValueError):self.check(self.tiled,after=after)

    def test_zoom_requires_prior_tile_fresh_sequence_and_strict_enlargement(self):
        with self.assertRaisesRegex(ValueError,'missing prior tiled'):self.check(self.expanded,'expanded',after=1)
        tiled=self.check(self.tiled)
        for record in ([1,1024,718,20,6,6],[2,512,359,18,6,6],[2,511,718,20,6,6],
                       [2,1024,358,20,6,6],[2,1024,719,20,6,6]):
            with self.subTest(record=record),self.assertRaises(ValueError):self.check(record,'expanded',tiled,1)

    def test_restore_rejects_each_stale_or_changed_accepted_shape_field(self):
        tiled=self.check(self.tiled)
        for record in ([2,512,359,18,6,6],[3,513,359,18,6,6],[3,512,360,18,6,6],
                       [3,512,359,20,6,6],[3,512,359,18,5,6],[3,512,359,18,6,5]):
            with self.subTest(record=record),self.assertRaises(ValueError):self.check(record,'restored',tiled,2)
        with self.assertRaisesRegex(ValueError,'missing prior tiled'):self.check([3,512,359,18,6,6],'restored',after=2)
        with self.assertRaises(ValueError):self.check(self.tiled,tiled=(512,359,20,6,6))
        with self.assertRaises(ValueError):self.check(self.tiled,tiled=[512,359,18,6,6])
        with self.assertRaises(ValueError):self.check(self.tiled,phase='unknown')

    def record(self,sequence=1,**changes):
        values=dict(task=12,sequence=sequence,w=512,h=359,cols=18,top=6,bottom=6,sender=3)
        values.update(changes)
        return ('[spindle.view.accepted] target=12 task={task} sequence={sequence} w={w} h={h} '
                'cols={cols} top_rows={top} bottom_rows={bottom} sender={sender}\n').format(**values)

    def test_latest_complete_canonical_record_cannot_fall_back_past_new_bad_shape(self):
        old=self.record()
        self.assertEqual(gate.accepted_layout(old+self.record(2)),[2,512,359,18,6,6])
        self.assertEqual(gate.accepted_layout(old.replace('\n','\r\n')),self.tiled)
        self.assertIsNone(gate.accepted_layout('unrelated serial\n'))
        for latest in (self.record(2,task=120),self.record(2,sender=4),self.record(2).replace('w=512','w=-512'),
                       self.record(2).replace(' sender=3',' sender=3 trailing=1'),self.record(2).rstrip('\n'),
                       '[spindle.view.accepted] target=12 task=12 sequence=2 w='):
            with self.subTest(latest=latest),self.assertRaises(ValueError):gate.accepted_layout(old+latest)
        latest=gate.accepted_layout(old+self.record(2,cols=20))
        with self.assertRaises(ValueError):self.check(latest,'restored',self.check(self.tiled),1)

    def args(self,*more):
        return gate.parse_args(['--iso','missing.iso','--source-disk','missing.img','--out','missing-output',*more])

    def test_defaults_and_legacy_advanced_flags_remain_available(self):
        args=self.args();self.assertFalse(args.relative_tiles);self.assertEqual(args.resolution,'1280x800')
        args=self.args(*['--'+name.replace('_','-')for name in gate.ADVANCED])
        self.assertTrue(all(getattr(args,name)for name in gate.ADVANCED))
        self.assertEqual(gate.vga_args('1280x800'),['-vga','none','-device','VGA,xres=1280,yres=800'])

    def test_relative_mode_is_explicit_basic_only_for_both_resolutions(self):
        for resolution in ('1024x768','1280x800'):
            args=self.args('--relative-tiles','--resolution',resolution)
            self.assertTrue(args.relative_tiles);self.assertEqual(args.resolution,resolution)
            self.assertEqual(gate.vga_args(resolution),['-vga','none','-device','VGA,xres='+resolution.replace('x',',yres=')])
        for flag in gate.ADVANCED:
            stderr=io.StringIO()
            with self.subTest(flag=flag),contextlib.redirect_stderr(stderr),self.assertRaises(SystemExit)as error:
                self.args('--relative-tiles','--'+flag.replace('_','-'))
            self.assertEqual(error.exception.code,2);self.assertIn('supports BASIC only',stderr.getvalue())
        with contextlib.redirect_stderr(io.StringIO()),self.assertRaises(SystemExit):self.args('--resolution','800x600')
        with self.assertRaises(ValueError):gate.vga_args('800x600')


if __name__=='__main__':unittest.main()
