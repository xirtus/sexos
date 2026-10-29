//! Actual passive dispatch. Caller arguments never grant pointer access.
#![allow(dead_code,unused_variables,static_mut_refs)]
macro_rules! serial_println{($($t:tt)*)=>{}}
include!(env!("SEXNET_SERVICE_SOURCE"));
fn reset(){unsafe{PROOF_LEN=0;PROOF_BUF=[0;32];BODY_LEN=0;BODY_BUF=[0;64];DNS_A_CACHE_VALID=[0;4];DNS_A_CACHE_TTL=[0;4];DNS_A_CACHE_IP=[[0;4];4];}}
const CONTROLS:[u64;5]=[SEXNET_SCAN_WIFI,SEXNET_CONNECT,SEXNET_DISCONNECT,SEXNET_VPN_UP,SEXNET_VPN_DOWN];
#[test]fn unsupported_controls_never_accept_raw_addresses_or_fabricate_connectivity(){
 reset();for opcode in CONTROLS{for address in [0,1,0x400000,0xffff800000000000,u64::MAX]{for length in [0,1,32,33,u64::MAX]{
  assert_eq!(handle_call(opcode,address,length),u64::MAX);assert_eq!(handle_call(SEXNET_GET_STATUS,0,0),0);assert_eq!(handle_call(SEXNET_GET_IP,0,0),0);
 }}}
}
#[test]fn scan_cannot_write_even_to_an_accessible_caller_supplied_buffer(){
 reset();let mut bytes=[0xa5u8;128];let ptr=bytes.as_mut_ptr()as u64;
 for opcode in CONTROLS{assert_eq!(handle_call(opcode,ptr,3),u64::MAX);assert_eq!(bytes,[0xa5;128]);}
}
#[test]fn cached_body_remains_exact_bounded_data_without_implying_a_live_link(){
 reset();let n=handle_call(SEXNET_GET_STATUS,SEXNET_HTTP_BODY_LEN,0)as usize;assert_eq!(n,BODY_TEXT.len());let mut bytes=vec![];
 for i in 0..(n+7)/8{bytes.extend_from_slice(&handle_call(SEXNET_GET_STATUS,SEXNET_HTTP_BODY_CHUNK,i as u64).to_le_bytes());}
 assert_eq!(&bytes[..n],BODY_TEXT);assert!(bytes[n..].iter().all(|&b|b==0));assert_eq!(handle_call(SEXNET_GET_STATUS,0,0),0);
 unsafe{BODY_BUF[..5].copy_from_slice(b"cache");BODY_LEN=5;}
 assert_eq!(handle_call(SEXNET_GET_STATUS,SEXNET_HTTP_BODY_LEN,0),5);assert_eq!(handle_call(SEXNET_GET_STATUS,SEXNET_HTTP_BODY_CHUNK,0),u64::from_le_bytes(*b"cache\0\0\0"));assert_eq!(handle_call(SEXNET_GET_IP,0,0),0);
}
#[test]fn proof_and_body_chunk_indices_saturate_before_access_and_tail_bytes_stay_zero(){
 reset();unsafe{PROOF_BUF[..3].copy_from_slice(b"abc");PROOF_LEN=3;BODY_LEN=64;BODY_BUF.fill(0x71);}
 assert_eq!(handle_call(SEXNET_GET_STATUS,SEXNET_HTTP_PROOF_CHUNK,0),u64::from_le_bytes(*b"abc\0\0\0\0\0"));
 for index in [8,usize::MAX as u64/8+1,u64::MAX]{
  assert_eq!(handle_call(SEXNET_GET_STATUS,SEXNET_HTTP_PROOF_CHUNK,index),u64::MAX);assert_eq!(handle_call(SEXNET_GET_STATUS,SEXNET_HTTP_BODY_CHUNK,index),u64::MAX);
 }
 assert_eq!(handle_call(0xffff,u64::MAX,u64::MAX),u64::MAX);
}
#[test]fn dns_cache_data_does_not_set_connection_state_or_create_a_live_query(){
 reset();assert_eq!(handle_call(SEXNET_DNS_RESOLVE,1,0),0);
 unsafe{DNS_A_CACHE_IP[0]=[10,0,2,2];DNS_A_CACHE_VALID[0]=1;DNS_A_CACHE_TTL[0]=1;}
 assert_eq!(handle_call(SEXNET_DNS_RESOLVE,1,0),u32::from_be_bytes([10,0,2,2])as u64);assert_eq!(handle_call(SEXNET_DNS_RESOLVE,2,0),0);
 assert_eq!(handle_call(SEXNET_GET_STATUS,0,0),0);assert_eq!(handle_call(SEXNET_GET_IP,0,0),0);
 unsafe{DNS_A_CACHE_TTL[0]=0;}assert_eq!(handle_call(SEXNET_DNS_RESOLVE,1,0),0);
}
