// SPDX-License-Identifier: GPL-3.0-only
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/desktop_recording.rs"]pub mod desktop_recording;
include!(env!("RECORDING_FILES_PATH_SOURCE"));
fn confined(binding:&Binding,path:&[u8])->Option<Vec<u8>>{
 let mut out=[0;w::PATH_BYTES];confine(binding,path,&mut out).map(|n|out[..n].to_vec())
}
#[test]
fn recorder_constructor_uses_explicit_root_without_generic_loader_or_picker_state(){
 assert!(recording_binding(0).is_none());let binding=recording_binding(107).unwrap();
 assert_eq!(binding.task,107);assert_eq!(&binding.root[..binding.len],b"/recordings");
 assert_eq!(binding.args_len,0);assert_eq!(binding.picked_len,0);
 assert_eq!(confined(&binding,b"my desktop.mkv"),Some(b"/recordings/my desktop.mkv".to_vec()));
 for path in [b"../private.mkv".as_slice(),b"../../bin/record",b"@/private",b"@/recordings/existing.mkv"]{assert_eq!(confined(&binding,path),None);}
}
#[test]
fn ordinary_basename_record_or_recordings_cannot_inherit_the_recorder_root(){
 for basename in [b"record".as_slice(),b"recordings"]{
  let mut binding=recording_binding(108).unwrap();let mut root=b"/data/".to_vec();root.extend_from_slice(basename);
  binding.root[..root.len()].copy_from_slice(&root);binding.len=root.len();
  for attempt in [b"/recordings/private.mkv".as_slice(),b"recordings/private.mkv"]{
   let path=confined(&binding,attempt).unwrap();assert!(path.starts_with(&root));assert!(!path.starts_with(b"/recordings/"));
  }
  for attempt in [b"../../recordings/private.mkv".as_slice(),b"@/recordings/private.mkv"]{assert!(confined(&binding,attempt).is_none());}
 }
}
#[test]
fn a_picked_root_is_available_only_when_the_explicit_grant_was_installed(){
 let mut binding=recording_binding(108).unwrap();binding.root[..12].copy_from_slice(b"/data/record");binding.len=12;
 let path=b"@/recordings/private.mkv";assert!(confined(&binding,path).is_none());
 let grant=b"@/recordings";binding.picked[..grant.len()].copy_from_slice(grant);binding.picked_len=grant.len();
 assert_eq!(confined(&binding,path),Some(b"/recordings/private.mkv".to_vec()));
 assert!(confined(&binding,b"@/recordings2/private.mkv").is_none());
 assert!(confined(&binding,b"@/recordings/../private.mkv").is_none());
}
