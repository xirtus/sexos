#![allow(dead_code)]
extern crate self as sex_pdx;
pub const SPINDLE_HISTORY_BASE_LBA:u64=1808;pub const SPINDLE_HISTORY_SECTORS:u64=18;
#[path="../crates/sex-pdx/src/spindle_history.rs"]pub mod spindle_history;
#[path="../crates/sex-pdx/src/history_recovery.rs"]pub mod history_recovery;
#[path="../servers/sexfiles/src/history_journal.rs"]mod journal;
use journal::{Journal,SectorIo,Store,STORAGE};use history_recovery::Grant;use spindle_history as w;
#[derive(Clone)]struct Disk{stable:Vec<[u8;512]>,cache:Vec<[u8;512]>,ops:usize,cut:Option<usize>,torn:bool}
impl Disk{fn new()->Self{Self{stable:vec![[0;512];18],cache:vec![[0;512];18],ops:0,cut:None,torn:false}}fn tick(&mut self)->Result<(),u32>{self.ops+=1;if self.cut==Some(self.ops){Err(STORAGE)}else{Ok(())}}fn crash(&self)->Self{Self{stable:self.stable.clone(),cache:self.stable.clone(),..Self::new()}}}
impl SectorIo for Disk{
 fn read(&mut self,lba:u64,out:&mut[u8;512])->Result<(),u32>{self.tick()?;*out=self.cache[(lba-1808)as usize];Ok(())}
 fn write(&mut self,lba:u64,data:&[u8;512])->Result<(),u32>{if self.tick().is_err(){if self.torn{let i=(lba-1808)as usize;self.stable[i][..37].copy_from_slice(&data[..37]);self.cache[i]=self.stable[i];}return Err(STORAGE)}self.cache[(lba-1808)as usize]=*data;Ok(())}
 fn sync(&mut self)->Result<(),u32>{self.tick()?;self.stable=self.cache.clone();Ok(())}
}
fn grant()->Grant{Grant{holder:11,logical:12,original:12,original_task:12,successor:17,successor_task:17,base:1808,sectors:18}}
fn blob(owner:u32,text:&[u8])->Vec<u8>{let mut b=vec![0;24+2+text.len()];b[24..26].copy_from_slice(&(text.len()as u16).to_le_bytes());b[26..].copy_from_slice(text);let len=b.len();assert!(w::seal(&mut b,owner,1,len));b}
fn maximum(owner:u32)->Vec<u8>{let mut bytes=vec![0;4096];let mut pos=24;let mut count=0;while pos<4096{let len=(4096-pos-2).min(256);bytes[pos..pos+2].copy_from_slice(&(len as u16).to_le_bytes());bytes[pos+2..pos+2+len].fill(b'x');pos+=2+len;count+=1;}assert!(w::seal(&mut bytes,owner,count,4096));bytes}
fn generation(d:&mut Disk,bank:usize,value:u64){let h=&mut d.stable[bank*9];h[16..24].copy_from_slice(&value.to_le_bytes());h[48..56].fill(0);let hash=w::hash(h);h[48..56].copy_from_slice(&hash.to_le_bytes());d.cache=d.stable.clone();}
#[test]fn second_boot_recovers_data_and_rebinds_only_current_header(){let mut j=Journal::new(Disk::new(),Some(grant()));let old=blob(17,b"echo survived reboot");j.save(17,&old).unwrap();let mut reboot=Journal::new(j.io.crash(),Some(grant()));let mut out=[0;4096];let n=reboot.recover(12,&mut out).unwrap();assert_eq!(&out[24..n],&old[24..]);assert_eq!(w::validate(&out[..n],12),Ok(1));assert!(w::validate(&out[..n],17).is_err());assert_eq!(reboot.last.unwrap().writer,17);}
#[test]fn every_save_crash_point_including_reused_bank_preserves_complete_old_or_new(){
 for previous_saves in [1,2]{for torn in[false,true]{
  let mut first=Journal::new(Disk::new(),Some(grant()));let old=blob(12,b"old stable");
  for _ in 0..previous_saves{first.save(12,&old).unwrap();}
  let original=first.io.crash();let new=maximum(17);let mut measure=Journal::new(original.clone(),Some(grant()));measure.save(17,&new).unwrap();
  for point in 1..=measure.io.ops{let mut disk=original.clone();disk.cut=Some(point);disk.torn=torn;let mut j=Journal::new(disk,Some(grant()));let _=j.save(17,&new);let mut reboot=Journal::new(j.io.crash(),Some(grant()));let mut out=[0;4096];let n=reboot.recover(12,&mut out).unwrap();assert!(&out[24..n]==&old[24..]||&out[24..n]==&new[24..],"mixed snapshot cut {point} torn {torn} old generations {previous_saves}");}
 }}
}
#[test]fn every_first_save_crash_is_missing_or_complete_never_partial(){let new=blob(12,b"first save");let mut measure=Journal::new(Disk::new(),Some(grant()));measure.save(12,&new).unwrap();for point in 1..=measure.io.ops{let mut d=Disk::new();d.cut=Some(point);let mut j=Journal::new(d,Some(grant()));let _=j.save(12,&new);let mut reboot=Journal::new(j.io.crash(),Some(grant()));let mut out=[0;4096];match reboot.recover(12,&mut out){Ok(n)=>assert_eq!(&out[..n],&new),Err(e)=>assert_eq!(e,w::MISSING)}}}
#[test]fn missing_misbound_and_cross_subject_grants_never_touch_disk(){for g in[None,Some(Grant{holder:7,..grant()}),Some(Grant{logical:9,..grant()}),Some(Grant{base:2047,..grant()}),Some(Grant{sectors:19,..grant()}),Some(Grant{original_task:99,..grant()})]{let mut j=Journal::new(Disk::new(),g);assert_eq!(j.save(12,&blob(12,b"deny")),Err(w::DENIED));assert_eq!(j.io.ops,0);}let mut j=Journal::new(Disk::new(),Some(grant()));assert_eq!(j.recover(9,&mut[0;4096]),Err(w::DENIED));assert_eq!(j.io.ops,0);}
#[test]fn latest_corrupt_payload_falls_back_to_previous_complete_bank(){let mut j=Journal::new(Disk::new(),Some(grant()));let old=blob(12,b"keep old");j.save(12,&old).unwrap();j.save(17,&blob(17,b"new damaged")).unwrap();let mut d=j.io.crash();d.stable[10][25]^=1;d.cache=d.stable.clone();let mut reboot=Journal::new(d,Some(grant()));let mut out=[0;4096];let n=reboot.recover(12,&mut out).unwrap();assert_eq!(&out[24..n],&old[24..]);assert_eq!(reboot.last.unwrap().generation,1);}
#[test]fn unknown_or_both_corrupt_headers_fail_closed_without_overwrite(){let mut d=Disk::new();d.stable[0][0]=1;d.cache=d.stable.clone();let mut j=Journal::new(d,Some(grant()));assert_eq!(j.save(12,&blob(12,b"no repair grant")),Err(w::CORRUPT));assert_eq!(j.io.stable[0][0],1);}
#[test]fn shorter_and_maximum_snapshots_do_not_publish_trailing_bytes(){let mut j=Journal::new(Disk::new(),Some(grant()));let large=maximum(12);j.save(12,&large).unwrap();let small=blob(17,b"x");j.save(17,&small).unwrap();let mut out=[0xAA;4096];let n=j.recover(17,&mut out).unwrap();assert_eq!(&out[..n],&small);assert!(out[n..].iter().all(|b|*b==0));}
#[test]fn source_identity_in_header_never_substitutes_for_recovery_grant(){let mut j=Journal::new(Disk::new(),Some(grant()));j.save(17,&blob(17,b"restricted writer")).unwrap();let mut restricted=grant();restricted.successor_task=99;let mut reboot=Journal::new(j.io.crash(),Some(restricted));assert_eq!(reboot.recover(12,&mut[0;4096]),Err(w::CORRUPT));}
#[test]fn failed_flush_is_not_a_successful_durable_commit(){let mut d=Disk::new();d.cut=Some(4);let mut j=Journal::new(d,Some(grant()));assert!(j.save(12,&blob(12,b"no false Saved")).is_err());assert_eq!(j.last,None);}

#[test]fn exhausted_generation_never_wraps_or_overwrites_committed_bank(){let mut j=Journal::new(Disk::new(),Some(grant()));j.save(12,&blob(12,b"keep" )).unwrap();generation(&mut j.io,0,u64::MAX);let before=j.io.stable.clone();assert_eq!(j.save(17,&blob(17,b"deny wrapping")),Err(w::STALE));assert_eq!(j.io.stable,before);}
#[test]fn equal_generation_with_different_complete_sources_is_ambiguous(){let mut j=Journal::new(Disk::new(),Some(grant()));j.save(12,&blob(12,b"first" )).unwrap();j.save(17,&blob(17,b"second")).unwrap();generation(&mut j.io,1,1);let before=j.io.stable.clone();assert_eq!(j.recover(12,&mut[0;4096]),Err(w::CORRUPT));assert_eq!(j.save(12,&blob(12,b"no repair authority")),Err(w::CORRUPT));assert_eq!(j.io.stable,before);}
#[test]fn first_save_torn_sector_never_recovers_a_partial_snapshot(){let new=maximum(12);let mut measure=Journal::new(Disk::new(),Some(grant()));measure.save(12,&new).unwrap();for point in 1..=measure.io.ops{let mut d=Disk::new();d.cut=Some(point);d.torn=true;let mut j=Journal::new(d,Some(grant()));let _=j.save(12,&new);let mut reboot=Journal::new(j.io.crash(),Some(grant()));let mut out=[0;4096];match reboot.recover(12,&mut out){Ok(n)=>assert_eq!(&out[..n],&new),Err(e)=>assert!(e==w::MISSING||e==w::CORRUPT)}}}
