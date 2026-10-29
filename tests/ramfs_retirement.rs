#![allow(dead_code)]
extern crate alloc;extern crate self as spin;extern crate self as sex_pdx;
include!(concat!(env!("APP_RESOURCE_TEST_DIR"),"/diskfs_constant.rs"));
pub struct RwLock<T>(std::sync::RwLock<T>);
impl<T> RwLock<T>{pub const fn new(v:T)->Self{Self(std::sync::RwLock::new(v))}pub fn read(&self)->std::sync::RwLockReadGuard<'_,T>{self.0.read().unwrap()}pub fn write(&self)->std::sync::RwLockWriteGuard<'_,T>{self.0.write().unwrap()}}
mod backends{include!(concat!(env!("APP_RESOURCE_TEST_DIR"),"/backend_trait.rs"));}
#[path="../servers/sexfiles/src/messages.rs"]mod messages;
#[path="../servers/sexfiles/src/backends/ramfs.rs"]mod ramfs;
use backends::FsBackend;use ramfs::*;
#[test]fn sole_owned_handle_closes_but_saved_name_bytes_reopen_intact(){
 let fs=RamFs::new();let old=fs.open(b"saved",messages::RAMFS_O_CREATE,0,15).unwrap();fs.write(old,0,b"persistent bytes",15).unwrap();
 assert_eq!(fs.retire_subject(15),(1,0));assert_eq!(fs.stat(old,0),Err(messages::ERR_INVALID_HANDLE));
 let reopened=fs.open(b"saved",0,0,0).unwrap();assert_ne!(reopened,old);let mut bytes=[0;32];let n=fs.read(reopened,0,&mut bytes,0).unwrap();assert_eq!(&bytes[..n as usize],b"persistent bytes");
 assert_eq!(fs.retire_subject(99),(0,0));
}
#[test]fn valid_live_shared_cap_preserves_handle_and_saved_bytes(){
 let fs=RamFs::new();let h=fs.open(b"shared",messages::RAMFS_O_CREATE,0,15).unwrap();fs.write(h,0,b"shared payload",15).unwrap();
 fs.proof_grant_caps_by_name(15,b"shared",9,CAP_RIGHT_READ|CAP_RIGHT_LIST).unwrap();
 assert_eq!(fs.retire_subject(15),(0,0));assert_eq!(fs.retire_subject(15),(0,0));let mut bytes=[0;32];let n=fs.read(h,0,&mut bytes,9).unwrap();assert_eq!(&bytes[..n as usize],b"shared payload");assert_eq!(fs.stat(h,9).unwrap().0,14);
}
#[test]fn dead_subject_cap_revoked_without_revoking_other_live_subject(){
 let fs=RamFs::new();let h=fs.open(b"live-owner",messages::RAMFS_O_CREATE,0,9).unwrap();fs.write(h,0,b"keep",9).unwrap();
 for subject in [15,12]{fs.proof_grant_caps_by_name(9,b"live-owner",subject,CAP_RIGHT_READ).unwrap();}
 assert_eq!(fs.retire_subject(15),(0,1));assert_eq!(fs.retire_subject(15),(0,0));
 assert_eq!(fs.read(h,0,&mut[0;4],15),Err(messages::ERR_PERM_DENIED));assert_eq!(fs.read(h,0,&mut[0;4],12),Ok(4));assert_eq!(fs.read(h,0,&mut[0;4],9),Ok(4));
}
#[test]fn stale_generation_or_list_only_caps_do_not_pin_owner_handle(){
 for stale in [false,true]{let fs=RamFs::new();let h=fs.open(b"private",messages::RAMFS_O_CREATE,0,15).unwrap();fs.write(h,0,b"save",15).unwrap();
 if stale{fs.proof_inject_stale_generation_cap(b"private",9,CAP_RIGHT_READ,0).unwrap();}else{fs.proof_grant_caps_by_name(15,b"private",9,CAP_RIGHT_LIST).unwrap();}
 assert_eq!(fs.retire_subject(15),(1,0));assert_eq!(fs.stat(h,0),Err(messages::ERR_INVALID_HANDLE));let h=fs.open(b"private",0,0,0).unwrap();assert_eq!(fs.read(h,0,&mut[0;4],0),Ok(4));}
}
