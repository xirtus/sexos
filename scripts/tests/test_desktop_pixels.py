"""Synthetic parser/pixel fixtures only; real guest proof is a separate run."""
from pathlib import Path
import sys
import unittest
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import check_desktop_pixels as checker

ROOT=Path(__file__).resolve().parents[2]
FONT=checker.font_from_source((ROOT/'servers/sexdisplay/src/main.rs').read_text())

class PrintableFont(unittest.TestCase):
    def test_complete_printable_coverage_and_explicit_unsupported_characters(self):
        self.assertEqual(len(FONT['ascii']),95)
        for code in range(32,127):self.assertEqual(checker.glyph(FONT,chr(code)),FONT['ascii'][code-32])
        for char in ['','aa','\n']:
            with self.assertRaises(ValueError):checker.glyph(FONT,char)
    def test_all_lowercase_shapes_are_distinct_from_uppercase(self):
        for code in range(ord('a'),ord('z')+1):
            self.assertNotEqual(checker.glyph(FONT,chr(code)),checker.glyph(FONT,chr(code-32)))
        self.assertEqual(checker.glyph(FONT,'a'),(0,0,14,1,15,17,15))
        self.assertEqual(checker.glyph(FONT,'g'),(0,0,15,17,15,1,14))
    def test_incomplete_or_out_of_cell_rom_is_rejected(self):
        source=(ROOT/'servers/sexdisplay/src/main.rs').read_text()
        for changed in [source.replace('[0, 0, 0, 0, 0, 0, 0], // ASCII 32','[0, 0, 0, 0, 0, 0, 32], // ASCII 32'),source.replace('[0, 0, 14, 1, 15, 17, 15], // ASCII 97','')]:
            with self.assertRaises(ValueError):checker.font_from_source(changed)
    def test_old_uppercase_substitution_cannot_pass_lowercase_name_pixels(self):
        upper=LOG.replace('alpha-note','ALPHA-NOTE').replace('beta-file','BETA-FILE')
        self.assertTrue(checker.verify(LOG,image(upper),FONT))
        self.assertEqual(checker.verify(LOG,image(),FONT),[])
LOG='''[linen.list.text] x=640 y=50 w=640 h=670 scale=2 text_color=0xf0f8ff rows=2 selected=2 row_y=72 row_h=36 first=0 committed_view=1
[linen.list.label] id=1 row=0 text=alpha-note
[linen.list.label] id=2 row=1 text=beta-file
'''

def image(log=LOG):
    view,labels=checker.final_view(log);width,height=1280,800
    pixels=bytearray(width*height*3)
    for identity,row,name in labels:
        value=('> 'if identity==view['selected']else'  ')+name
        for col,char in enumerate(value):
            for gy,bits in enumerate(checker.glyph(FONT,char)):
                for gx in range(5):
                    if not bits&(1<<(4-gx)):continue
                    for dy in range(2):
                        for dx in range(2):
                            x=view['x']+8+(col*6+gx)*2+dx;y=view['y']+view['row_y']+row*36+gy*2+dy
                            pixels[(y*width+x)*3:(y*width+x)*3+3]=b'\xf0\xf8\xff'
    return b'P6\n1280 800\n255\n'+pixels

class PixelChecks(unittest.TestCase):
    def test_explicit_compact_geometry_checks_real_glyphs_and_exact_framebuffer(self):
        log=LOG.replace('x=640','x=0');raw=image(log).split(b'\n',3)[3]
        pixels=b''.join(raw[y*1280*3:(y*1280+1024)*3]for y in range(768))
        ppm=b'P6\n1024 768\n255\n'+pixels
        self.assertEqual(checker.verify(log,ppm,FONT,resolution=(1024,768)),[])
        self.assertTrue(checker.verify(log,ppm,FONT))
        self.assertTrue(checker.verify(log,ppm,FONT,resolution=(1280,800)))
    def test_hide_requires_actual_text_ink_to_disappear(self):
        blank=b'P6\n1280 800\n255\n'+bytes(1280*800*3)
        self.assertEqual(checker.verify_hidden(LOG,blank),[])
        self.assertTrue(checker.verify_hidden(LOG,image()))
        self.assertTrue(checker.verify_hidden(LOG,blank[:-1]))
    def test_hidden_check_does_not_accept_invalid_view_or_missing_intent(self):
        self.assertTrue(checker.verify_hidden('',image()))
        self.assertTrue(checker.verify_hidden(LOG.replace('x=640','x=2000'),image()))
    def test_actual_glyph_shapes_and_selected_marker(self):self.assertEqual(checker.verify(LOG,image(),FONT),[])
    def test_log_without_pixels_cannot_pass(self):self.assertTrue(checker.verify(LOG,b'P6\n1280 800\n255\n'+bytes(1280*800*3),FONT))
    def test_solid_white_blocks_cannot_pass(self):self.assertTrue(checker.verify(LOG,b'P6\n1280 800\n255\n'+b'\xf0\xf8\xff'*(1280*800),FONT))
    def test_other_names_not_accepted(self):self.assertTrue(checker.verify(LOG.replace('beta-file','beta-fake'),image(),FONT))
    def test_longer_stale_filename_cannot_pass_as_expected_prefix(self):
        for old,new in [('alpha-note','alpha-notebook'),('beta-file','beta-files'),('alpha-note','alpha-note  X')]:
            with self.subTest(suffix=new):
                self.assertTrue(checker.verify(LOG,image(LOG.replace(old,new)),FONT))
    def test_trailing_ink_at_last_permitted_column_is_rejected(self):
        longer='alpha-note'+' '*(51-2-len('alpha-note'))+'X'
        self.assertTrue(checker.verify(LOG,image(LOG.replace('alpha-note',longer)),FONT))
    def test_selection_intent_must_match_marker_pixels(self):self.assertTrue(checker.verify(LOG.replace('selected=2','selected=1'),image(),FONT))
    def test_incomplete_name_intents_fail(self):self.assertTrue(checker.verify(LOG.split('[linen.list.label] id=2')[0],image(),FONT))
    def test_outside_frame_and_invalid_scale_fail(self):
        for a,b in [('x=640','x=2000'),('scale=2','scale=3'),('rows=2','rows=6'),('committed_view=1','committed_view=0')]:self.assertTrue(checker.verify(LOG.replace(a,b),image(),FONT))
    def test_only_last_committed_view_is_checked(self):self.assertEqual(checker.verify(LOG.replace('alpha-note','old-note')+LOG,image(),FONT),[])
    def test_truncated_ppm_fails(self):self.assertTrue(checker.verify(LOG,image()[:-1],FONT))
    def test_atomic_intent_after_foreign_prefix_is_read(self):self.assertEqual(checker.verify('[foreign.partial] tail='+LOG,image(),FONT),[])

KALEIDO_LOG='[shell.tile.apply] sid=300 x=640 y=385 w=640 h=335\n'
def kaleido_image(x=640,y=385,w=640,lines=('Kaleidoscope','Cached / sample network text'),color=b'\xcd\xd6\xf4',status_offset=None):
    width,height=1280,800
    pixels=bytearray(width*height*3);scale=2 if w>=256 else 1
    for row,line in enumerate(lines):
        y0=y+36+(row*9*scale if status_offset is None or row==0 else status_offset)
        for col,char in enumerate(line):
            for gy,bits in enumerate(checker.glyph(FONT,char)):
                for gx in range(5):
                    if not bits&(1<<(4-gx)):continue
                    for dy in range(scale):
                        for dx in range(scale):
                            px=x+8+(col*6+gx)*scale+dx;py=y0+gy*scale+dy
                            pixels[(py*width+px)*3:(py*width+px)*3+3]=color
    return b'P6\n1280 800\n255\n'+pixels

class KaleidoPixelChecks(unittest.TestCase):
    def test_tiled_title_and_status_at_actual_scaled_line_positions(self):
        self.assertEqual(checker.verify_kaleido_layout(KALEIDO_LOG,kaleido_image(),FONT),[])
    def test_narrow_surface_uses_scale_one(self):
        log='[shell.tile.apply] sid=300 x=40 y=60 w=240 h=160\n'
        self.assertEqual(checker.verify_kaleido_layout(log,kaleido_image(40,60,240),FONT),[])
    def test_startup_position_cannot_satisfy_final_tiled_geometry(self):
        self.assertTrue(checker.verify_kaleido_layout(KALEIDO_LOG,kaleido_image(50,100,520),FONT))
    def test_latest_complete_target_tile_wins_over_older_or_foreign_intents(self):
        old='[shell.tile.apply] sid=300 x=50 y=100 w=520 h=120\n'
        foreign='[shell.tile.apply] sid=200 x=50 y=100 w=520 h=120\n'
        incomplete='[shell.tile.apply] sid=300 x=10 y=60 w=520\n'
        self.assertEqual(checker.verify_kaleido_layout(old+KALEIDO_LOG+foreign+incomplete,kaleido_image(),FONT),[])
        self.assertTrue(checker.verify_kaleido_layout(KALEIDO_LOG+old,kaleido_image(),FONT))
    def test_wrong_glyphs_spacing_color_and_trailing_text_fail(self):
        for ppm in [kaleido_image(lines=('Kaleidofcope','Cached / sample network text')),
                    kaleido_image(lines=('Kaleidoscope','HTTP 200 from 10.0.2.2')),
                    kaleido_image(lines=('KaleidoscopeX','Cached / sample network text')),
                    kaleido_image(status_offset=36),kaleido_image(color=b'\xff\xff\xff')]:
            self.assertTrue(checker.verify_kaleido_layout(KALEIDO_LOG,ppm,FONT))
    def test_blank_missing_malformed_small_and_offscreen_fail(self):
        blank=b'P6\n1280 800\n255\n'+bytes(1280*800*3)
        self.assertTrue(checker.verify_kaleido_layout(KALEIDO_LOG,blank,FONT))
        self.assertTrue(checker.verify_kaleido_layout(KALEIDO_LOG,kaleido_image()[:-1],FONT))
        for log in ['',KALEIDO_LOG.replace('sid=300','sid=200'),KALEIDO_LOG.replace('x=640','x=-1'),KALEIDO_LOG.replace('x=640','x=1000'),KALEIDO_LOG.replace('y=385','y=10'),KALEIDO_LOG.replace('w=640','w=100'),KALEIDO_LOG.replace('h=335','h=50'),KALEIDO_LOG.replace('x=640','x=bad')]:
            self.assertTrue(checker.verify_kaleido_layout(log,kaleido_image(),FONT))


class ScalarFontChecks(unittest.TestCase):
    def test_finite_repertoire_and_unknown_box_are_source_bound_and_json_stable(self):
        import json
        frozen=json.loads(json.dumps(FONT))
        for char in 'éÉäÄöÖüÜß€…←↑→↓':
            self.assertTrue(any(checker.glyph(FONT,char)))
            self.assertEqual(tuple(checker.glyph(frozen,char)),tuple(checker.glyph(FONT,char)))
        self.assertEqual(checker.glyph(FONT,'\t'),(0,)*7)
        self.assertEqual(checker.glyph(FONT,'\u00a0'),(0,)*7)
        self.assertEqual(tuple(checker.glyph(FONT,'😀')),(31,17,21,21,21,17,31))
        self.assertNotEqual(checker.glyph(FONT,'😀'),checker.glyph(FONT,'?'))
    def test_incomplete_unbound_and_out_of_cell_scalar_rom_are_rejected(self):
        source=(ROOT/'servers/sexdisplay/src/main.rs').read_text()
        for changed in [source.replace('const FONT_UNICODE_5X7:','const UNUSED_UNICODE_5X7:'),
                        source.replace('[7, 8, 30, 8, 30, 8, 7]','[32, 8, 30, 8, 30, 8, 7]'),
                        source.replace('map_or(FONT_UNSUPPORTED_5X7','map_or(OTHER_ROM'),
                        source.replace('[31, 17, 21, 21, 21, 17, 31]','[0, 0, 0, 0, 0, 0, 0]')]:
            with self.assertRaises(ValueError):checker.font_from_source(changed)
    def test_independent_cells_retain_exact_scalar_byte_spans_and_replace_invalid_bytes_individually(self):
        raw='Aé€😀\t\n'.encode()
        self.assertEqual(checker.text_cells(raw),[('A',0,1),('é',1,3),('€',3,6),('😀',6,10),('\t',10,11),('\n',11,12)])
        self.assertEqual(checker.text_cells(b'\xe2\x82X\xff'),[('\ufffd',0,1),('\ufffd',1,2),('X',2,3),('\ufffd',3,4)])
        self.assertEqual(checker.text_cells(b'\xed\xa0\x80'),[('\ufffd',0,1),('\ufffd',1,2),('\ufffd',2,3)])
    def test_complete_unicode_filename_cells_fail_ascii_question_mark_substitution(self):
        log=LOG.replace('alpha-note','é€😀').replace('beta-file','Äöß')
        self.assertEqual(checker.verify(log,image(log),FONT),[])
        self.assertTrue(checker.verify(log,image(log.replace('é€😀','???')),FONT))

if __name__=='__main__':unittest.main()
