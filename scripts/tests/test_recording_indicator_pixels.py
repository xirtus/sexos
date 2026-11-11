"""Actual marker + extracted production cursor/save-under pixels; fixture scene.

This is source-pixel evidence, not native scanout/BOOT or controller authority.
"""
import pathlib
import re
import subprocess
import tempfile
import unittest
ROOT = pathlib.Path(__file__).resolve().parents[2]
def function(source, name):
    start = source.index('fn ' + name + '(')
    end = source.index('{', start) + 1
    depth = 1
    while depth:
        depth += (source[end] == '{') - (source[end] == '}')
        end += 1
    return source[start:end]
class RecordingIndicator(unittest.TestCase):
    def test_actual_cursor_and_marker(self):
        source = (ROOT/'servers/sexdisplay/src/main.rs').read_text()
        batch = function(source, 'raster_batch')
        assert batch.index('draw_cursor_z_top(') < batch.index('recording_indicator::paint(')
        assert 'if scene.recording.is_some() {' in batch
        prepared = function(source, 'render_prepared')
        assert prepared.index('preparation.commit(') < prepared.index('RECORDING_CAPTURE.published(')
        assert 'if plan.clock_dirty(){RECORDING_CAPTURE.published(' in prepared
        a = source.index('const CURSOR_ARROW_W:')
        b = source.index('static mut CURSOR_UNDER_VALID:', a)
        cursor = source[a:source.index(';', b)+1]
        code = f'''#![allow(dead_code,static_mut_refs)]
extern crate self as sex_pdx;
#[path="{ROOT}/crates/sex-pdx/src/app_catalog.rs"]mod app_catalog;
#[path="{ROOT}/crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
#[path="{ROOT}/servers/sexdisplay/src/recording_indicator.rs"]mod recording_indicator;
''' + cursor + r'''
const CURSOR_SURFACE_ID:u64=0x90;
#[derive(Clone,Copy)]struct PreparedCursor{x:usize,y:usize,seq:u32,shell_tick:u32}
static mut PREPARED_CURSOR:Option<PreparedCursor>=None;
static mut INPUT_TRACE_LAST_SEQ:u32=0;
static mut INPUT_TRACE_LAST_SHELL_TICK:u32=0;
struct Surface {active:bool,surface_id:u64,x:i32,y:i32}
mod scene {
 pub struct SceneSnapshot {pub surfaces:Vec<super::Surface>,pub metrics:super::desktop_style::chrome::Metrics}
}
''' + function(source,'restore_cursor_under') + '\n' + function(source,'draw_cursor_z_top') + r'''
#[test]fn exact_marker_survives_actual_cursor_and_save_under_at_all_scales(){unsafe{
 for scale in 1..=3 {
  let(w,h)=(640*scale as usize,360*scale as usize);
  let metrics=desktop_style::chrome::Metrics::new(scale);
  let mut bar=silkbar_model::DEFAULT_SILK_BAR;
  assert!(silkbar_model::reflow_chrome(&mut bar,w,metrics.bar()as usize,metrics.px(24)as usize));
  let r=recording_indicator::rect(&bar,w,h,scale).unwrap();
  let mut pixels=vec![0x102030;w*h];let(fg,bg)=(0xff4040,0x203040);
  assert!(recording_indicator::paint(pixels.as_mut_ptr(),w,h,&bar,scale,fg,bg));
  let snapshot=scene::SceneSnapshot{surfaces:vec![Surface{active:true,surface_id:0x90,x:r.x as i32,y:r.y as i32}],metrics};
  draw_cursor_z_top(pixels.as_mut_ptr(),w,h,pixels.len(),true,&snapshot);
  assert!(recording_indicator::matches(&pixels,w,h,&bar,scale,fg,bg));
  restore_cursor_under(pixels.as_mut_ptr(),w,h,pixels.len(),metrics);
  assert!(recording_indicator::matches(&pixels,w,h,&bar,scale,fg,bg));
  // Network visibility/kind do not participate in protected marker placement.
  bar.chips[0].visible=false;bar.chips[0].kind=silkbar_model::ChipKind::Battery;
  pixels[r.y*w+r.x]=0xabcdef;
  assert!(!recording_indicator::matches(&pixels,w,h,&bar,scale,fg,bg));
  assert!(recording_indicator::paint(pixels.as_mut_ptr(),w,h,&bar,scale,fg,bg));
  assert!(recording_indicator::matches(&pixels,w,h,&bar,scale,fg,bg));
  bar.layout[silkbar_model::ModuleSlot::Chip0 as usize].w=0;
  assert!(recording_indicator::rect(&bar,w,h,scale).is_none());
  assert!(!recording_indicator::paint(pixels.as_mut_ptr(),w,h,&bar,scale,fg,bg));
 }
}}
'''
        with tempfile.TemporaryDirectory(prefix='sexos-record-marker-') as tmp:
            tmp = pathlib.Path(tmp)
            lib = tmp/'libsilkbar_model.rlib'
            subprocess.run(['rustc','--edition=2021','--crate-type=rlib','--crate-name','silkbar_model',str(ROOT/'crates/silkbar-model/src/lib.rs'),'-o',str(lib)],check=True)
            src = tmp/'indicator.rs'; src.write_text(code)
            binary = tmp/'indicator-tests'
            subprocess.run(['rustc','--edition=2021','--test',str(src),'--extern',f'silkbar_model={lib}','-o',str(binary)],check=True)
            subprocess.run([str(binary),'--test-threads=1'],check=True)
if __name__ == '__main__':
    unittest.main()
