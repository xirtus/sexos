//! Real VFS extent loop and DiskFS entry I/O; mock only extent metadata and block transport.
#![allow(dead_code)]
extern crate self as sex_pdx;
pub mod document {pub const IO:u32=7;pub const STALE:u32=8;pub const INVALID:u32=9;}
include!("fixtures/diskfs_transport.rs");
mod document_journal {
 pub trait Io {fn read(&mut self,offset:u64,out:&mut[u8])->Result<(),u32>;fn write(&mut self,offset:u64,bytes:&[u8])->Result<(),u32>;fn sync(&mut self)->Result<(),u32>;}
}
struct DocumentIo{id:u8}
struct Entry{size_bytes:u32}
const EXTENTS:[DiskManifestEntryV1;3]=[
 DiskManifestEntryV1{start_lba:400,len_bytes:1536},
 DiskManifestEntryV1{start_lba:420,len_bytes:2560},
 DiskManifestEntryV1{start_lba:440,len_bytes:8192},
];
fn diskfs_bridge_get_buf_va()->u64{VA.load(std::sync::atomic::Ordering::Relaxed)}
fn v4_ensure(_:u64)->Result<(),u64>{Ok(())}
fn v4_get(id:u64)->Result<Entry,u64>{if id==5{Ok(Entry{size_bytes:12288})}else{Err(1)}}
fn v4_resolve_offset(id:usize,offset:u64,_va:u64)->Result<(DiskManifestEntryV1,u64),u64>{
 if id!=5{return Err(1)}let mut base=0;
 for e in EXTENTS {if offset>=base&&offset-base<e.len_bytes as u64{return Ok((e,offset-base))}base+=e.len_bytes as u64;}
 Err(1)
}
impl DiskFs{fn diskfs_block_sync()->u64{0}}
include!(env!("DOCUMENT_EXTENT_SOURCE"));
#[test]fn fragmented_full_document_uses_only_exact_extent_transfers_and_recovers_bytes(){
 let _g=TEST_LOCK.lock().unwrap();let mut buf=[0;4096];setup(&mut buf);
 let bytes:Vec<u8>=(0..12288).map(|i|((i*29+7)%251)as u8).collect();
 let mut io=DocumentIo{id:5};use document_journal::Io;
 assert_eq!(io.write(0,&bytes),Ok(()));
 let expected_calls=[(true,204800,1536),(true,215040,2560),(true,225280,4096),(true,229376,4096)];
 assert_eq!(STATE.lock().unwrap().as_ref().unwrap().calls,expected_calls);
 let mut out=vec![0;12288];assert_eq!(io.read(0,&mut out),Ok(()));assert_eq!(out,bytes);
 let s=STATE.lock().unwrap();let s=s.as_ref().unwrap();
 assert_eq!(&s.calls[4..],[(false,204800,1536),(false,215040,2560),(false,225280,4096),(false,229376,4096)]);
 let mut expected=vec![0xa5;MODEL_BYTES];let mut at=0;
 for e in EXTENTS{let n=e.len_bytes as usize;expected[e.start_lba as usize*512..e.start_lba as usize*512+n].copy_from_slice(&bytes[at..at+n]);at+=n;}
 assert_eq!(s.bytes,expected);
}
#[test]fn unaligned_cross_extent_writes_preserve_every_neighbor_and_reads_match(){
 let _g=TEST_LOCK.lock().unwrap();let mut buf=[0;4096];
 for (offset,len) in [(1,8000),(1500,7000),(1535,2),(4095,4097),(12000,288)] {
  setup(&mut buf);let bytes:Vec<u8>=(0..len).map(|i|(i%251)as u8).collect();let mut io=DocumentIo{id:5};use document_journal::Io;
  assert_eq!(io.write(offset,&bytes),Ok(()));let mut out=vec![0;len];assert_eq!(io.read(offset,&mut out),Ok(()));assert_eq!(out,bytes);
  let mut expected=vec![0xa5;MODEL_BYTES];let mut logical=0;
  for e in EXTENTS {for i in 0..e.len_bytes as usize{let at=logical+i as u64;if at>=offset && at-offset<len as u64{expected[e.start_lba as usize*512+i]=bytes[(at-offset)as usize];}}logical+=e.len_bytes as u64;}
  assert_eq!(STATE.lock().unwrap().as_ref().unwrap().bytes,expected);
 }
}
#[test]fn document_failures_stop_without_replay_and_invalid_ranges_have_no_effect(){
 let _g=TEST_LOCK.lock().unwrap();let mut buf=[0;4096];setup(&mut buf);let mut io=DocumentIo{id:5};use document_journal::Io;
 STATE.lock().unwrap().as_mut().unwrap().fail_write_at=2;
 assert_eq!(io.write(0,&[0x42;12288]),Err(document::IO));assert_eq!(STATE.lock().unwrap().as_ref().unwrap().writes,2);
 setup(&mut buf);STATE.lock().unwrap().as_mut().unwrap().fail_read_at=2;
 assert_eq!(io.read(0,&mut[0;12288]),Err(document::IO));assert_eq!(STATE.lock().unwrap().as_ref().unwrap().reads,2);
 setup(&mut buf);assert_eq!(io.write(12288,&[]),Ok(()));assert_eq!(io.read(12288,&mut[]),Ok(()));
 assert_eq!(io.write(12289,&[]),Err(document::INVALID));assert_eq!(io.read(12289,&mut[]),Err(document::IO));
 assert_eq!(io.write(12287,&[1;2]),Err(document::INVALID));assert_eq!(io.read(12287,&mut[0;2]),Err(document::IO));
 assert_eq!(io.write(u64::MAX,&[1;2]),Err(document::INVALID));assert_eq!(io.read(u64::MAX,&mut[0;2]),Err(document::IO));
 assert_eq!(document_piece_len(EXTENTS[0],1536,512),Err(document::IO));assert_eq!(document_piece_len(EXTENTS[0],u64::MAX,512),Err(document::IO));
 assert!(STATE.lock().unwrap().as_ref().unwrap().calls.is_empty());
}
