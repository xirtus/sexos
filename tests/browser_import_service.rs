//! Actual SexFiles document service and ABI. Storage is an isolated host model;
//! this is create-only policy/receipt evidence, not native disk durability.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[macro_export]macro_rules! serial_record{($($a:tt)*)=>{{let _=format_args!($($a)*);}};}
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
#[path="../crates/sex-pdx/src/document.rs"]pub mod document;
#[path="../servers/sexfiles/src/document.rs"]mod service;
use document as w;
use service::{Backend,Service};
use std::collections::BTreeMap;
struct Object{generation:u32,name:[u8;16],bytes:Vec<u8>}
struct Store{objects:BTreeMap<u8,Object>,legacy_creates:usize,fresh_creates:usize,identities:Vec<u8>,loads:usize,names:usize,saves:Vec<(u32,u8,Vec<u8>)>,hold:bool,cancelled:Vec<u32>}
fn name(text:&[u8])->[u8;16]{let mut n=[0;16];n[..text.len()].copy_from_slice(text);n}
impl Store{
 fn new()->Self{let mut objects=BTreeMap::new();objects.insert(2,Object{generation:0x30001,name:name(b"existing"),bytes:b"DO NOT OVERWRITE".to_vec()});Self{objects,legacy_creates:0,fresh_creates:0,identities:vec![],loads:0,names:0,saves:vec![],hold:false,cancelled:vec![]}}
}
impl Backend for Store{
 fn identity(&mut self,id:u8)->Result<u32,u32>{self.identities.push(id);self.objects.get(&id).map(|o|o.generation).ok_or(w::STALE)}
 fn name(&mut self,id:u8)->Result<[u8;16],u32>{self.names+=1;self.objects.get(&id).map(|o|o.name).ok_or(w::STALE)}
 fn load(&mut self,_:u8,_:&mut[u8;w::MAX])->Result<usize,u32>{self.loads+=1;Err(w::IO)}
 // Deliberately unsafe legacy fake: Browser must never invoke this old API.
 fn create(&mut self,_:[u8;16])->Result<u8,u32>{self.legacy_creates+=1;Ok(2)}
 fn create_new(&mut self,n:[u8;16])->Result<(u8,u32),u32>{
  self.fresh_creates+=1;if self.objects.values().any(|o|o.name==n){return Err(w::EXISTS)}
  let id=(3..15).find(|id|!self.objects.contains_key(id)).ok_or(w::IO)?;let generation=((id as u32+1)<<16)|1;
  self.objects.insert(id,Object{generation,name:n,bytes:vec![]});Ok((id,generation))
 }
 fn save_step(&mut self,caller:u32,id:u8,bytes:&[u8])->Result<bool,u32>{if self.hold{return Ok(false)}self.objects.get_mut(&id).ok_or(w::STALE)?.bytes=bytes.to_vec();self.saves.push((caller,id,bytes.to_vec()));Ok(true)}
 fn cancel_save(&mut self,caller:u32){self.cancelled.push(caller);}
}
fn req(seq:u32,kind:u8,object:u8,a:u64,b:u64)->w::Request{w::Request{seq,kind,object,offset:0,a,b}}
fn create(seq:u32,text:&[u8])->w::Request{let n=name(text);req(seq,w::CREATE,0,u64::from_le_bytes(n[..8].try_into().unwrap()),u64::from_le_bytes(n[8..].try_into().unwrap()))}
fn rpc(s:&mut Service,b:&mut Store,pd:u32,r:w::Request)->Result<u32,u32>{
 if let Some(v)=s.handle(pd,true,r,b){return w::decode_reply(v,r.seq).unwrap()}
 s.progress(b);let mut answer=None;s.poll(|caller,v|{if caller==pd{answer=Some(v);0}else{1}});w::decode_reply(answer.expect("actual retained receipt"),r.seq).unwrap()
}
fn begin(s:&mut Service,b:&mut Store,seq:&mut u32,text:&[u8],bytes:&[u8])->(u8,u32){
 let id=rpc(s,b,15,create(*seq,text)).unwrap()as u8;*seq+=1;let generation=rpc(s,b,15,req(*seq,w::INFO,id,0,0)).unwrap();*seq+=1;
 assert_eq!(rpc(s,b,15,req(*seq,w::BEGIN_SAVE,id,generation as u64,bytes.len()as u64)),Ok(0));*seq+=1;
 for(offset,chunk)in bytes.chunks(16).enumerate(){let mut raw=[0;16];raw[..chunk.len()].copy_from_slice(chunk);let mut r=req(*seq,w::WRITE,id,u64::from_le_bytes(raw[..8].try_into().unwrap()),u64::from_le_bytes(raw[8..].try_into().unwrap()));r.offset=(offset*16)as u32;assert_eq!(rpc(s,b,15,r),Ok(chunk.len()as u32));*seq+=1;}
 (id,generation)
}
#[test]fn full_import_is_exact_create_only_and_binding_ends_after_positive_end(){
 let(mut s,mut b)=(Service::new(),Store::new());let mut seq=1;let body:Vec<u8>=(0..w::MAX).map(|i|((i*37+11)%251)as u8).collect();
 let(id,_)=begin(&mut s,&mut b,&mut seq,b"web-import",&body);assert_eq!(rpc(&mut s,&mut b,15,req(seq,w::COMMIT,id,w::hash(&body),0)),Ok(w::MAX as u32));seq+=1;
 assert_eq!(b.saves,vec![(15,id,body)]);assert_eq!(b.legacy_creates,0);assert_eq!(b.objects[&2].bytes,b"DO NOT OVERWRITE");
 assert_eq!(rpc(&mut s,&mut b,15,req(seq,w::END,id,0,0)),Ok(0));assert!(s.browser_import_scrubbed());seq+=1;
 assert_eq!(rpc(&mut s,&mut b,15,req(seq,w::INFO,id,0,0)),Err(w::DENIED));seq+=1;
 assert_eq!(rpc(&mut s,&mut b,15,create(seq,b"next-import")),Ok(id as u32+1));
}
#[test]fn browser_cannot_probe_load_read_name_or_overwrite_any_uncreated_object(){
 let(mut s,mut b)=(Service::new(),Store::new());let mut seq=1;
 for kind in [w::INFO,w::LOAD,w::READ,w::NAME,w::BEGIN_SAVE,w::WRITE,w::COMMIT,w::END]{assert_eq!(rpc(&mut s,&mut b,15,req(seq,kind,2,0,0)),Err(w::DENIED));seq+=1;}
 assert!(b.identities.is_empty());assert_eq!((b.loads,b.names,b.legacy_creates,b.fresh_creates),(0,0,0,0));
 let id=rpc(&mut s,&mut b,15,create(seq,b"scoped")).unwrap()as u8;seq+=1;
 for kind in [w::LOAD,w::READ,w::NAME]{assert_eq!(rpc(&mut s,&mut b,15,req(seq,kind,id,0,0)),Err(w::DENIED));seq+=1;}
 let before=b.identities.len();for kind in [w::INFO,w::BEGIN_SAVE,w::WRITE,w::COMMIT,w::END]{assert_eq!(rpc(&mut s,&mut b,15,req(seq,kind,2,0,0)),Err(w::DENIED));seq+=1;}assert_eq!(b.identities.len(),before);assert!(b.saves.is_empty());
}
#[test]fn collisions_retain_existing_bytes_and_second_create_is_busy_until_end_or_abort(){
 let(mut s,mut b)=(Service::new(),Store::new());assert_eq!(rpc(&mut s,&mut b,15,create(1,b"existing")),Err(w::EXISTS));assert_eq!(b.objects.len(),1);assert_eq!(b.objects[&2].bytes,b"DO NOT OVERWRITE");
 let id=rpc(&mut s,&mut b,15,create(2,b"fresh")).unwrap()as u8;let effects=b.fresh_creates;
 assert_eq!(rpc(&mut s,&mut b,15,create(3,b"another")),Err(w::BUSY));assert_eq!(b.fresh_creates,effects);
 assert_eq!(rpc(&mut s,&mut b,15,req(4,w::END,id,0,0)),Ok(0));assert_eq!(b.objects.len(),2); // No implicit deletion.
 assert_eq!(rpc(&mut s,&mut b,15,create(5,b"fresh")),Err(w::EXISTS));assert_eq!(rpc(&mut s,&mut b,15,create(6,b"another")),Ok(4));
 assert_eq!(rpc(&mut s,&mut b,15,req(7,w::ABORT,0,0,0)),Ok(0));assert!(s.browser_import_scrubbed());assert_eq!(rpc(&mut s,&mut b,15,req(8,w::INFO,4,0,0)),Err(w::DENIED));assert_eq!(b.objects.len(),3);
}
#[test]fn retained_create_and_commit_receipts_do_not_replay_under_full_or_duplicates(){
 let(mut s,mut b)=(Service::new(),Store::new());let r=create(1,b"retained");assert_eq!(s.handle(15,true,r,&mut b),None);for _ in 0..8{assert_eq!(s.handle(15,true,r,&mut b),None);s.poll(|_,_|1);}assert_eq!(b.fresh_creates,1);
 let mut words=vec![];s.poll(|caller,v|{assert_eq!(caller,15);words.push(v);0});assert_eq!(words.len(),1);assert_eq!(w::decode_reply(words[0],1),Some(Ok(3)));
 assert_eq!(rpc(&mut s,&mut b,15,r),Ok(3));assert_eq!(b.fresh_creates,1);
 let generation=rpc(&mut s,&mut b,15,req(2,w::INFO,3,0,0)).unwrap();assert_eq!(rpc(&mut s,&mut b,15,req(3,w::BEGIN_SAVE,3,generation as u64,0)),Ok(0));
 b.hold=true;let commit=req(4,w::COMMIT,3,w::hash(&[]),0);assert_eq!(s.handle(15,true,commit,&mut b),None);for _ in 0..6{s.progress(&mut b);assert_eq!(s.handle(15,true,commit,&mut b),None);s.poll(|_,_|panic!("unfinished COMMIT became visible"));}assert!(b.saves.is_empty());
 b.hold=false;s.progress(&mut b);for _ in 0..5{s.poll(|_,_|1);assert_eq!(s.handle(15,true,commit,&mut b),None);s.progress(&mut b);}assert_eq!(b.saves.len(),1);
 let mut positive=None;s.poll(|_,v|{positive=Some(v);0});assert_eq!(w::decode_reply(positive.unwrap(),4),Some(Ok(0)));assert_eq!(rpc(&mut s,&mut b,15,commit),Ok(0));assert_eq!(b.saves.len(),1);
}
#[test]fn saturated_receipt_window_rejects_before_fresh_create_effect(){
 let(mut s,mut b)=(Service::new(),Store::new());for seq in 1..=w::WINDOW as u32{assert_eq!(s.handle(15,true,req(seq,w::INFO,2,0,0),&mut b),None);}
 let r=create(5,b"after-full");let v=s.handle(15,true,r,&mut b).unwrap();assert_eq!(w::decode_reply(v,5),Some(Err(w::BUSY)));assert_eq!(b.fresh_creates,0);
 s.poll(|_,_|0);assert_eq!(rpc(&mut s,&mut b,15,r),Ok(3));assert_eq!(b.fresh_creates,1);
}
#[test]fn generation_replacement_and_terminal_retirement_never_restore_import_binding(){
 let(mut s,mut b)=(Service::new(),Store::new());let id=rpc(&mut s,&mut b,15,create(1,b"stale")).unwrap()as u8;let generation=b.objects[&id].generation;b.objects.get_mut(&id).unwrap().generation+=1;
 assert_eq!(rpc(&mut s,&mut b,15,req(2,w::INFO,id,0,0)),Err(w::STALE));assert_eq!(rpc(&mut s,&mut b,15,req(3,w::BEGIN_SAVE,id,generation as u64,1)),Err(w::STALE));assert!(b.saves.is_empty());
 assert!(s.retire(15,&mut b));assert!(s.browser_import_scrubbed());s.progress(&mut b);s.poll(|_,_|panic!("retired Browser receipt"));assert!(b.objects.contains_key(&id));
 for r in [create(4,b"again"),req(5,w::ABORT,0,0,0),req(6,w::INFO,id,0,0)]{let v=s.handle(15,true,r,&mut b).unwrap();assert_eq!(w::decode_reply(v,r.seq),Some(Err(w::DENIED)));}assert_eq!(b.fresh_creates,1);
}
#[test]fn close_cancels_pending_import_without_commit_and_does_not_touch_quil_session(){
 let(mut s,mut b)=(Service::new(),Store::new());let mut seq=1;let(id,_)=begin(&mut s,&mut b,&mut seq,b"cancel",b"PRIVATE IMPORT");b.hold=true;let commit=req(seq,w::COMMIT,id,w::hash(b"PRIVATE IMPORT"),0);assert_eq!(s.handle(15,true,commit,&mut b),None);s.progress(&mut b);
 assert!(s.retire(15,&mut b));assert!(s.browser_import_scrubbed());b.hold=false;s.progress(&mut b);s.poll(|_,_|panic!("retired commit receipt"));assert!(b.saves.is_empty());assert!(b.cancelled.contains(&15));assert!(b.objects[&id].bytes.is_empty());
 // Existing Quil policy is unchanged and owns an independent service session.
 assert_eq!(rpc(&mut s,&mut b,9,req(1,w::INFO,2,0,0)),Ok(0x30001));assert_eq!(rpc(&mut s,&mut b,16,req(1,w::INFO,2,0,0)),Ok(0x30001));assert_eq!(rpc(&mut s,&mut b,18,req(1,w::INFO,2,0,0)),Ok(0x30001));
}
#[test]fn dead_or_unknown_subject_and_malformed_name_cannot_allocate(){
 let(mut s,mut b)=(Service::new(),Store::new());for pd in [0,3,12,17,24,u32::MAX]{let r=create(1,b"deny");assert_eq!(w::decode_reply(s.handle(pd,true,r,&mut b).unwrap(),1),Some(Err(w::DENIED)));}
 let r=create(1,b"deny");assert_eq!(w::decode_reply(s.handle(15,false,r,&mut b).unwrap(),1),Some(Err(w::DENIED)));
 for(seq,n)in [(1,b"bad/name".as_slice()),(2,b"".as_slice()),(3,b"bad\\name".as_slice())]{assert_eq!(rpc(&mut s,&mut b,15,create(seq,n)),Err(w::INVALID));}
 assert_eq!(b.fresh_creates,0);assert_eq!(b.legacy_creates,0);
}
#[test]fn accepted_commit_abort_scrubs_binding_and_old_mutation_cannot_replay(){
 let(mut s,mut b)=(Service::new(),Store::new());let mut seq=1;let(id,_)=begin(&mut s,&mut b,&mut seq,b"abort",b"CANCEL THIS IMPORT");b.hold=true;let commit=req(seq,w::COMMIT,id,w::hash(b"CANCEL THIS IMPORT"),0);
 assert_eq!(s.handle(15,true,commit,&mut b),None);s.progress(&mut b);assert_eq!(rpc(&mut s,&mut b,15,req(seq+1,w::ABORT,0,0,0)),Ok(0));assert!(s.browser_import_scrubbed());assert!(b.objects[&id].bytes.is_empty());
 b.hold=false;assert_eq!(rpc(&mut s,&mut b,15,commit),Err(w::STALE));s.progress(&mut b);assert!(b.saves.is_empty());assert_eq!(rpc(&mut s,&mut b,15,req(seq+2,w::INFO,id,0,0)),Err(w::DENIED));assert_eq!(rpc(&mut s,&mut b,15,create(seq+3,b"retry-new")),Ok(4));
}
