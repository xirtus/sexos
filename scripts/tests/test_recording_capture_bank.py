"""Host atomic-bank behavior with actual SDK sources and fixture-only grants.

This does not establish BOOT authority, PKU denial, or native restart safety.
"""
import pathlib
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class RecordingBank(unittest.TestCase):
    def test_actual_bank(self):
        modules = "\n".join(
            f'#[path = "{ROOT}/crates/sex-pdx/src/{name}.rs"] mod {name};'
            for name in ("desktop_recording", "memory_region", "desktop_look", "desktop_recording_capture")
        )
        code = modules + r'''
pub use memory_region::MemoryRegionInfo;
use desktop_recording_capture::*;
use desktop_recording::{RoleSequence,SessionSequence};
use memory_region::{RegionView,QUERY_VIEW,READ,WRITE};
use std::alloc::{alloc_zeroed,dealloc,Layout};
struct Backing(*mut u8);
impl Backing {
 fn new()->Self {let p=unsafe{alloc_zeroed(Layout::from_size_align(BYTES,4096).unwrap())};assert!(!p.is_null());Self(p)}
 fn grant(&self,writer:bool)->(MemoryRegionInfo,RegionView){
  (MemoryRegionInfo {region_id:REGION_ID,length:BYTES as u64,rights:QUERY_VIEW|READ|if writer{WRITE}else{0},peer_pd:Some(if writer{42}else{1})},
   RegionView {region_id:REGION_ID,virtual_address:self.0 as u64,length:BYTES as u64,dma_address:None})
 }
 fn bank(&self,writer:bool)->Bank {let(i,v)=self.grant(writer);unsafe{Bank::from_resolved(i,v,writer).unwrap()}}
}
impl Drop for Backing {fn drop(&mut self){unsafe{dealloc(self.0,Layout::from_size_align(BYTES,4096).unwrap())}}}
fn header()->Header {
 let scope=Scope {session:SessionSequence::new(1,1).unwrap(),shell_epoch:1,display_epoch:1,record_task:1073741830,spindle_task:12};
 Header::new(scope,RoleSequence::new(1,1).unwrap(),1,900,WIDTH,HEIGHT,0).unwrap()
}
#[test] fn readonly_grant_cannot_write(){
 let b=Backing::new();let mut w=b.bank(true);let mut r=b.bank(false);assert!(w.install_epoch(1));
 assert!(!r.install_epoch(2));assert!(!r.scrub());assert!(!r.publish(header(),&vec![0;PIXELS]));assert_eq!(w.service_epoch(),1);
}
#[test] fn exact_copy_then_revocation_and_no_aba(){
 let b=Backing::new();let mut w=b.bank(true);let r=b.bank(false);assert!(w.install_epoch(1));
 let source:Vec<_>=(0..PIXELS).map(|i|0xff000000|(i as u32)).collect();assert!(w.publish(header(),&source));
 let old=r.snapshot().unwrap();assert!(!w.publish(header(),&source));let mut private=vec![u32::MAX;PIXELS];let mut off=0;
 while off<PIXELS {let n=r.copy_chunk(old,&mut private,off).unwrap();assert!(n>0&&n<=CHUNK_PIXELS);off+=n;}
 assert!(private.iter().enumerate().all(|(i,p)|*p==i as u32));
 assert!(w.scrub());assert!(r.snapshot().is_none());assert!(r.copy_chunk(old,&mut private,0).is_none());
 assert!(w.publish(header(),&source));assert_ne!(r.snapshot().unwrap().token,old.token);assert!(r.copy_chunk(old,&mut private,0).is_none());
}
#[test] fn scrub_zeroes_all_sensitive_cells(){
 let b=Backing::new();let mut w=b.bank(true);assert!(w.install_epoch(1));assert!(w.publish(header(),&vec![0xffffff;PIXELS]));assert!(w.scrub());
 for i in 0..BYTES/8 {if i!=30&&i!=31 {assert_eq!(unsafe{(&*(b.0 as *const std::sync::atomic::AtomicU64).add(i)).load(std::sync::atomic::Ordering::SeqCst)},0);}}
 assert!(!w.install_epoch(1));assert!(w.install_epoch(2));assert!(!w.publish(header(),&vec![0;PIXELS]));
}
#[test] fn invalid_input_has_no_publication(){
 let b=Backing::new();let mut w=b.bank(true);let r=b.bank(false);assert!(w.install_epoch(1));
 assert!(!w.publish(header(),&[0;4]));let mut h=header();h.scope.record_task=0;assert!(!w.publish(h,&vec![0;PIXELS]));assert!(r.snapshot().is_none());
 assert!(w.publish(header(),&vec![0x123456;PIXELS]));let snap=r.snapshot().unwrap();let mut dst=vec![0;PIXELS];
 assert!(r.copy_chunk(snap,&mut dst,1).is_none());assert!(r.copy_chunk(snap,&mut dst,PIXELS+2).is_none());assert!(r.copy_chunk(snap,&mut dst[..100],0).is_none());
}
#[test] fn grant_mismatch_rejected(){
 let b=Backing::new();let(i,v)=b.grant(false);
 let mut bad=i;bad.rights|=WRITE;assert!(unsafe{Bank::from_resolved(bad,v,false)}.is_none());
 bad=i;bad.peer_pd=Some(3);assert!(unsafe{Bank::from_resolved(bad,v,false)}.is_none());
 let mut wrong=v;wrong.virtual_address+=8;assert!(unsafe{Bank::from_resolved(i,wrong,false)}.is_none());
 wrong=v;wrong.dma_address=Some(0x1000);assert!(unsafe{Bank::from_resolved(i,wrong,false)}.is_none());
 bad=i;bad.length-=4096;assert!(unsafe{Bank::from_resolved(bad,v,false)}.is_none());
}
'''
        with tempfile.TemporaryDirectory(prefix="sexos-record-bank-") as tmp:
            src = pathlib.Path(tmp) / "bank.rs"
            src.write_text(code)
            binary = pathlib.Path(tmp) / "bank-tests"
            subprocess.run(["rustc", "--edition=2021", "--test", str(src), "-o", str(binary)], check=True)
            subprocess.run([str(binary), "--test-threads=1"], check=True)


if __name__ == "__main__":
    unittest.main()
