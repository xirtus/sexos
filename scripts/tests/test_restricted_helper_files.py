"""Actual Files scrub predicates/volatile erasure and production intake ordering."""
from pathlib import Path
import subprocess
import tempfile
import unittest
ROOT = Path(__file__).resolve().parents[2]


def function(source, name):
    start = source.index(name)
    body = source.index('{', start)
    depth = 1
    end = body + 1
    while depth:
        depth += (source[end] == '{') - (source[end] == '}')
        end += 1
    return source[start:end]


class FilesPolicy(unittest.TestCase):
    def test_actual_full_grant_scrub_and_adjacent_canaries(self):
        source = (ROOT / 'servers/sexfiles/src/restricted_runtime.rs').read_text()
        rust = '''extern crate self as sex_pdx;
#[path="''' + str(ROOT / 'crates/sex-pdx/src/memory_region.rs') + '''"] pub mod memory_region;
#[path="''' + str(ROOT / 'crates/sex-pdx/src/restricted_helper.rs') + '''"] mod helper;
use helper::Subject;
mod f {pub const REGION_BYTES:usize=256*1024;pub const REGION_ID_BASE:u32=0x5000;}
''' + function(source, 'fn scrub_shape(') + '\n' + function(source, 'unsafe fn erase_words(') + r'''
#[test]fn full_padding_scrub_keeps_neighbors_and_shape_refuses_foreign_grants(){
 use memory_region::*;
 unsafe{
 let layout=std::alloc::Layout::from_size_align(f::REGION_BYTES+8192,4096).unwrap();
 let allocation=std::alloc::alloc_zeroed(layout);assert!(!allocation.is_null());
 let base=allocation.add(4096) as *mut u64;
 base.sub(1).write(0x11223344);base.add(f::REGION_BYTES/8).write(0xaabbccdd);
 for n in 0..f::REGION_BYTES/8{base.add(n).write(0x0123456789abcdef);}
 let subject=Subject{pd:43,task:55};let index=3;
 let info=MemoryRegionInfo{region_id:f::REGION_ID_BASE+index as u32,length:f::REGION_BYTES as u64,rights:QUERY_VIEW|READ|WRITE,peer_pd:Some(43)};
 let view=RegionView{region_id:info.region_id,virtual_address:base as u64,length:info.length,dma_address:None};
 assert!(scrub_shape(subject,index,&info,&view));erase_words(base,view.length);
 assert!((0..f::REGION_BYTES/8).all(|n|base.add(n).read()==0));
 assert_eq!(base.sub(1).read(),0x11223344);assert_eq!(base.add(f::REGION_BYTES/8).read(),0xaabbccdd);
 for bad in [MemoryRegionInfo{peer_pd:Some(42),..info},MemoryRegionInfo{rights:QUERY_VIEW|READ,..info},MemoryRegionInfo{length:info.length-8,..info},MemoryRegionInfo{region_id:info.region_id+1,..info}]{assert!(!scrub_shape(subject,index,&bad,&view));}
 for bad in [RegionView{length:view.length+4096,..view},RegionView{length:view.length-8,..view},RegionView{virtual_address:view.virtual_address+8,..view},RegionView{virtual_address:0,..view},RegionView{region_id:view.region_id+1,..view}]{assert!(!scrub_shape(subject,index,&info,&bad));}
 std::alloc::dealloc(allocation,layout);
 }
}
'''
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / 'scrub.rs'
            path.write_text(rust)
            binary = Path(d) / 'tests'
            for command in (['rustc', '--edition=2021', '--test', str(path), '-o', str(binary)], [str(binary)]):
                result = subprocess.run(command, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_production_startup_and_dispatch_fences(self):
        source = (ROOT / 'servers/sexfiles/src/trampoline.rs').read_text()
        self.assertLess(source.index('restricted_runtime::initialize()'), source.index('recording_runtime::initialize()'))
        self.assertLess(source.index('restricted_runtime::poll()'), source.index('history.poll(pdx_reply)'))
        self.assertLess(source.index('restricted_runtime::message('), source.index('if hot_log() { serial_println!', source.index('let caller = msg.caller_pd;')))
        self.assertLess(source.index('restricted_runtime::denied(caller)'), source.index('crate::fs_service::handle(caller'))
        service = (ROOT / 'servers/sexfiles/src/fs_service.rs').read_text()
        bind = function(service, 'pub fn bind(')
        self.assertLess(bind.index('restricted_runtime::denied(target)'), bind.index('program_index(target)'))
        clean = function(service, 'pub fn quarantine_program(')
        for item in ('BINDINGS', 'PICKS', 'OFFERS', 'REPLIES', 'PREPARED', 'retire_app_subject'):
            self.assertIn(item, clean)


if __name__ == '__main__':
    unittest.main()
