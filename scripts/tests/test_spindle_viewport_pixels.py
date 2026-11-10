import sys,unittest,tempfile,shutil,hashlib
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from check_spindle_viewport_pixels import verify,layout,_scale_threshold,SCALE_SOURCE
from history_boot_fixture import boot
from check_desktop_pixels import font_from_source,glyph,draw_cell
FONT=font_from_source((Path(__file__).resolve().parents[2]/'servers/sexdisplay/src/main.rs').read_text())
ROOT=Path(__file__).resolve().parents[2]
def fixture_layout(w,h,threshold=128,ui=None):
    # Independent fixture policy, never reads the oracle or production scale.
    factor=1 if ui is None else ui
    inset=8*factor;top=36 if ui is None else 34*factor
    body_w=max(0,w-2*inset);body_h=max(0,h-top-inset);scale=(2 if body_w>=threshold else 1)*factor
    cols=min(110,max(0,max(0,body_w-2*inset)//(6*scale)-1))
    rows=min(48,1+(body_h-12*factor-7*scale)//(9*scale))if body_h>=12*factor+7*scale and cols else 0
    empty=(inset,top,body_w,0)
    return [(inset,top,body_w,body_h),empty,empty,empty],cols,[rows,0]
def fixture(w=1280,h=134,target=12,task=None,caption=None,threshold=128,glyph_threshold=None,ui=None):
    task=target if task is None else task
    width,height=1280,800;data=bytearray(width*height*3);x,y=0,50 if ui is None else 28*ui;factor=1 if ui is None else ui;panes,cols,rows=fixture_layout(w,h,threshold,ui);sids={12:[154,160,161,162],17:[302,303,304,305],24:[313,315,316,317],25:[318,320,321,322],26:[323,325,326,327],27:[328,330,331,332],28:[333,335,336,337],29:[338,340,341,342],40:[353,355,356,357]}[target]
    text=boot({target:task})+f'[spindle.view.accepted] target={target} task={task} sequence=2 w={w} h={h} cols={cols} top_rows={rows[0]} bottom_rows={rows[1]} sender=3\n'
    if ui is not None:text=f'[display.chrome.metrics] scale={ui} bar={28*ui} title={26*ui} inset={8*ui} body_top={34*ui} target={24*ui} width=1280 height=800 publication=1\n'+text
    prompt=b'> echo spindle17'.ljust(cols)
    for sid,(px,py,pw,ph)in zip(sids,panes):text+=f'[shell.app.viewport] target={target} sid={sid} x={x+px} y={y+py} w={pw} h={ph}\n'
    for i,(sid,(px,py,pw,ph))in enumerate(zip(sids[:1],panes)):
        r=rows[0];lines=[b' '*cols for _ in range(r)]
        if r and caption is not None:lines[0]=caption.encode().ljust(cols)
        if r:lines[-1]=prompt
        payload=b''.join(line+b'\n'for line in lines);c=cols if r else 0
        text+=f'[spindle.view.text] target={target} sid={sid} sequence=2 paint={i+1} cols={c} rows={r} bytes={len(payload)}\n'
        for off in range(0,len(payload),48):text+=f'[spindle.view.bytes] target={target} sid={sid} seq=2 paint={i+1} off={off} hex={payload[off:off+48].hex()}\n'
        scale=(2 if pw>=(threshold if glyph_threshold is None else glyph_threshold)else 1)*factor
        for row,line in enumerate(lines):
            for col,char in enumerate(line):
                draw_cell(data,width,x+px+8*factor+col*6*scale,y+py+12*factor+row*9*scale,chr(char),scale,b'\xe8\xff\xff',FONT)
    return text,b'P6\n1280 800\n255\n'+data
class Check(unittest.TestCase):
    def source_root(self,scale_source):
        temporary=tempfile.TemporaryDirectory();self.addCleanup(temporary.cleanup)
        root=Path(temporary.name);sdk=root/'crates/sex-pdx/src';sdk.mkdir(parents=True)
        shutil.copyfile(ROOT/'crates/sex-pdx/src/app_catalog.rs',sdk/'app_catalog.rs')
        if scale_source is not None:(root/SCALE_SOURCE).write_text(scale_source)
        return root
    def test_current_shared_geometry_and_actual_font_scale_at_all_three_levels(self):
        for ui,w,h,expected in [(1,512,134,((8,34,496,92),39,[4,0])),(2,1280,700,((16,68,1248,616),49,[16,0])),(3,1280,700,((24,102,1232,574),31,[10,0]))]:
            m=dict(inset=8*ui,body_top=34*ui,scale=ui)
            panes,cols,rows=layout(w,h,metrics=m);self.assertEqual((panes[0],cols,rows),expected)
            text,ppm=fixture(w,h,ui=ui)
            self.assertEqual(verify(text,ppm,FONT,12,'> echo spindle17',True,False),[])
            self.assertTrue(verify(text.replace(f'body_top={34*ui} ',f'body_top={36*ui} '),ppm,FONT,12,'> echo spindle17',True,False))

    def test_independent_128_threshold_layout_goldens(self):
        self.assertEqual(_scale_threshold(),128)
        self.assertEqual(layout(512,134),([(8,36,496,90),(8,36,496,0),(8,36,496,0),(8,36,496,0)],39,[4,0]))
        self.assertEqual(layout(278,134)[1:],(19,[4,0]))
        self.assertEqual(layout(1280,750)[1:],(103,[38,0]))
        for height in range(90,104):
            panes,cols,rows=layout(512,height)
            self.assertEqual((panes[0][3],panes[1][3],cols,rows),(height-44,0,39,[2,0]))
            text,ppm=fixture(512,height)
            self.assertEqual(verify(text,ppm,FONT,12,'> echo spindle17',True,False),[])
        text,ppm=fixture(278,134)
        self.assertEqual(verify(text,ppm,FONT,12,'> echo spindle17',True,False),[])
    def test_sdk_scale_source_is_sealed_in_alpha_contract(self):
        from alpha_evidence import contract
        self.assertEqual(contract(ROOT)['source_sha256'][SCALE_SOURCE],hashlib.sha256((ROOT/SCALE_SOURCE).read_bytes()).hexdigest())
    def test_bound_scale_controls_both_layout_and_actual_ink(self):
        source=(ROOT/SCALE_SOURCE).read_text()
        old=self.source_root(source.replace('width>=128','width>=256'))
        self.assertEqual(_scale_threshold(old),256)
        self.assertEqual(layout(260,134,old)[1:],(37,[8,0]))
        text,ppm=fixture(260,134,threshold=256)
        self.assertEqual(verify(text,ppm,FONT,12,'> echo spindle17',True,False,root=old),[])
        self.assertTrue(verify(text,ppm,FONT,12,'> echo spindle17',True,False))
        text,ppm=fixture(260,134)
        self.assertEqual(verify(text,ppm,FONT,12,'> echo spindle17',True,False),[])
        self.assertTrue(verify(text,ppm,FONT,12,'> echo spindle17',True,False,root=old))
        # Correct 128-layout metadata cannot bless old 1x ink at pane width244.
        text,ppm=fixture(260,134,glyph_threshold=256)
        self.assertTrue(verify(text,ppm,FONT,12,'> echo spindle17',True,False))
    def test_scale_source_missing_ambiguous_or_unsupported_fails_closed(self):
        source=(ROOT/SCALE_SOURCE).read_text()
        for index,changed in enumerate([None,'',source.replace('width>=128','width>128'),source.replace('width>=128','width>=129'),
                        source.replace('{2}else{1}','{3}else{1}'),
                        source+'\npub const fn scale(width:u32)->u32{if width>=128{2}else{1}}',
                        source.replace('if width>=128{2}else{1}','let threshold=128;if width>=threshold{2}else{1}')]):
            root=self.source_root(changed)
            with self.subTest(case=index),self.assertRaises(ValueError):layout(512,134,root)
            text,ppm=fixture(512,134)
            self.assertTrue(verify(text,ppm,FONT,12,None,False,False,root=root))
    def test_compact_resolution_is_explicit_and_exact(self):
        text,ppm=fixture(1024,134)
        rgb=ppm.split(b'\n',3)[3]
        compact=b'P6\n1024 768\n255\n'+b''.join(rgb[y*1280*3:y*1280*3+1024*3]for y in range(768))
        self.assertEqual(verify(text,compact,FONT,12,'> echo spindle17',False,False,resolution=(1024,768)),[])
        self.assertTrue(verify(text,compact,FONT,12,'> echo spindle17',False,False))
        self.assertTrue(verify(text,compact,FONT,12,'> echo spindle17',False,False,resolution=(1280,800)))
    def test_small_tile_and_wide_resize_keep_real_prompt_ink(self):
        for w,h in [(1280,134),(640,134),(1280,670),(640,80)]:
            text,ppm=fixture(w,h);self.assertEqual(verify(text,ppm,FONT,12,'> echo spindle17',True,False),[])
    def test_blank_frame_is_not_readable_prompt_evidence(self):
        text,ppm=fixture();blank=ppm[:ppm.index(b'255\n')+4]+bytes(1280*800*3);self.assertTrue(verify(text,blank,FONT,12,'> echo spindle17',True,False))
    def test_stale_layout_wrong_owner_or_geometry_cannot_pass(self):
        text,ppm=fixture()
        for old,new in [('sid=154 sequence=2','sid=154 sequence=1'),('task=12 sequence=2','task=17 sequence=2'),('bottom_rows=0','bottom_rows=6'),('h=90','h=91')]:self.assertTrue(verify(text.replace(old,new),ppm,FONT,12,'> echo spindle17',True,False),(old,new))
    def test_empty_or_partial_hex_payload_is_not_a_paint(self):
        text,ppm=fixture();self.assertTrue(verify(text.replace('bytes=416','bytes=415'),ppm,FONT,12,None,False,False));self.assertTrue(verify(text.replace('off=0 hex=2020','off=0 hex=0'),ppm,FONT,12,None,False,False))
    def test_all_nine_catalog_grids_and_unequal_held_task_have_exact_current_pixels(self):
        for target,task in [(12,12),(17,17),(24,24),(24,240),(25,250),(26,260),(27,270),(28,280),(29,290),(40,400)]:
            text,ppm=fixture(512,134,target,task,caption='NET / verified')
            self.assertEqual(verify(text,ppm,FONT,target,'> echo spindle17',False,False,require_text='NET / verified'),[])
        text,ppm=fixture(640,134,24,240,caption='NET / verified across panes')
        self.assertEqual(verify(text,ppm,FONT,24,None,False,False,require_text='NET / verified across panes'),[])

    def test_third_grid_wrong_task_layout_sequence_unknown_and_one_pixel_reject(self):
        text,ppm=fixture(640,134,24,240,caption='NET / verified')
        for changed in [text.replace('task=240 sequence=2','task=24 sequence=2'),
                        text.replace('sid=313','sid=302'),text.replace('sid=313 sequence=2','sid=313 sequence=1'),
                        text.replace('task=240 sequence=2','task=240 sequence=0')]:
            self.assertTrue(verify(changed,ppm,FONT,24,None,False,False))
        self.assertTrue(verify(text,ppm,FONT,30,None,False,False))
        header,rgb=ppm.split(b'255\n',1);changed=bytearray(rgb);at=changed.index(b'\xe8\xff\xff');changed[at]=0xe7
        self.assertTrue(verify(text,header+b'255\n'+changed,FONT,24,None,False,False))
        # An unused physical text column may not retain old foreground ink.
        changed=bytearray(rgb);offset=((50+36+12)*1280+(8+8+49*12))*3;changed[offset:offset+3]=b'\xe8\xff\xff'
        self.assertTrue(verify(text,header+b'255\n'+changed,FONT,24,None,False,False))

    def test_required_caption_comes_only_from_latest_current_pane_payload(self):
        old,_=fixture(640,134,24,240,caption='NET / verified')
        current,ppm=fixture(640,134,24,240,caption='NET / unavailable')
        old_paints=old[old.index('[spindle.view.accepted]'):]
        # Same-layout repaint with newer paint IDs replaces every pane.
        current_paints=current[current.index('[spindle.view.accepted]'):]
        for i in range(4,0,-1):current_paints=current_paints.replace(f'paint={i}',f'paint={i+4}')
        prefix=current[:current.index('[spindle.view.accepted]')]+old_paints+current_paints+'[old.event] NET / verified\n'
        self.assertTrue(verify(prefix,ppm,FONT,24,None,False,False,require_text='NET / verified'))
        self.assertEqual(verify(prefix,ppm,FONT,24,None,False,False,require_text='NET / unavailable'),[])
        for caption in ['', 'not visible']:
            self.assertTrue(verify(current,ppm,FONT,24,None,False,False,require_text=caption))

    def test_boot_provenance_and_literal_unique_grid_are_required(self):
        text,ppm=fixture(640,134,24,240,caption='NET / verified')
        self.assertTrue(verify(text[text.index('[spindle.view.accepted]'):],ppm,FONT,24,None,False,False))
        self.assertTrue(verify(text.replace('index=2 target=24 task=240','index=2 target=24 task=24'),ppm,FONT,24,None,False,False))
        root=Path(__file__).resolve().parents[2];source=(root/'crates/sex-pdx/src/app_catalog.rs').read_text()
        for children in ['315, 316, 316','315, 316']:
            with tempfile.TemporaryDirectory() as temporary:
                custom=Path(temporary);path=custom/'crates/sex-pdx/src/app_catalog.rs';path.parent.mkdir(parents=True)
                path.write_text(source.replace('children: &[315, 316, 317]','children: &['+children+']'))
                shutil.copyfile(ROOT/SCALE_SOURCE,custom/SCALE_SOURCE)
                self.assertTrue(verify(text,ppm,FONT,24,None,False,False,root=custom))

if __name__=='__main__':unittest.main()
