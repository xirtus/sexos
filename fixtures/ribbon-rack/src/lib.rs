// SPDX-License-Identifier: GPL-3.0-only
#![no_std]
//! Developer-only exact-address carrier and parent marker integrity checks.
//! A PCM address is test data, never production authority or a pointer API.
#[repr(C,align(4096))]
pub struct Marker(pub [u8;4096]);
impl Marker {
 pub fn new()->Self { Self(core::array::from_fn(|i| (i as u8).wrapping_mul(29).wrapping_add(0x37))) }
 pub fn intact(&self)->bool { self.0.iter().enumerate().all(|(i,v)| *v==(i as u8).wrapping_mul(29).wrapping_add(0x37)) }
}
pub fn encode_address(address:u64)->Option<[[f32;2];256]> {
 if address==0||address&4095!=0||address>=1u64<<47{return None}
 let mut pcm=[[0.125,-0.25];256];
 for i in 0..16 {pcm[i][0]=((address>>(4*i))&15)as f32;}
 Some(pcm)
}
pub fn decode_address(pcm:&[[f32;2]])->Option<u64>{
 if pcm.len()<16{return None}let mut address=0u64;
 for i in 0..16 {let v=pcm[i][0];if !(0.0..=15.0).contains(&v)||v!=v as u8 as f32{return None}address|=(v as u64)<<(4*i);}
 (address!=0&&address&4095==0&&address<1u64<<47).then_some(address)
}
#[cfg(test)] mod tests {
 use super::*;
 #[test]fn actual_aligned_marker_carrier_integrity_and_invalid_floats(){
 let mut marker=Marker::new();assert!(marker.intact());let address=0x4000_1234_5000;
 let pcm=encode_address(address).unwrap();assert_eq!(decode_address(&pcm),Some(address));assert_eq!(core::mem::align_of::<Marker>(),4096);
 for value in [f32::NAN,f32::INFINITY,-1.0,0.5,16.0]{let mut broken=pcm;broken[7][0]=value;assert_eq!(decode_address(&broken),None)}
 assert!(encode_address(address+1).is_none());assert!(encode_address(1<<47).is_none());assert!(encode_address(0).is_none());assert!(decode_address(&pcm[..15]).is_none());
 marker.0[0]=0xa5;assert!(!marker.intact());
 }
}
