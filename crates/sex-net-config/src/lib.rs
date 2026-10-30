#![no_std]
// SPDX-License-Identifier: GPL-3.0-only
//! Bounded immutable BOOT network configuration data, never a capability.
//! Reading or copying these words grants no route, device, memory or endpoint
//! authority. sex-pdx must attest the held BOOT grant independently. A validated
//! on-link gateway can carry configured unicast endpoints outside the local link.

pub const SYS_CONFIG:u64=71;
pub const HOST_NAME:&[u8]=b"probe.sexos.test";
pub const DNS_NAME:&[u8]=b"\x05probe\x05sexos\x04test\0";
pub const DNS_QUESTION:&[u8]=b"\x05probe\x05sexos\x04test\0\0\x01\0\x01";
pub const MAX_TICKET:u32=17_535;
const VERSION:u8=1;
const SOURCE_BASE:u16=48_000;

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Config {
 pub local:[u8;4],pub prefix:u8,pub gateway:[u8;4],pub dns:[u8;4],
 pub dns_port:u16,pub http:[u8;4],pub http_port:u16,pub listen_port:u16,
 pub source_base:u16,pub profile:u8,
}

fn mask(prefix:u8)->Option<u32> {
 if (1..=30).contains(&prefix) {Some(u32::MAX<<(32-prefix))}else{None}
}
fn usable(address:[u8;4],mask:u32)->bool {
 if address[0]==0||address[0]==127||address[0]>=224 {return false;}
 let host=u32::from_be_bytes(address)&!mask;
 host!=0&&host!=!mask
}
fn unicast(address:[u8;4])->bool {address[0]!=0&&address[0]!=127&&address[0]<224}

impl Config {
 pub const CONTROLLED:Self=Self {
  local:[10,0,2,15],prefix:24,gateway:[10,0,2,2],dns:[10,0,2,2],dns_port:46079,
  http:[10,0,2,2],http_port:46078,listen_port:46076,source_base:SOURCE_BASE,profile:1,
 };

 /// Membership is restricted to usable unicast hosts on the configured local
 /// subnet. Invalid prefixes/local hosts fail closed even for constructed data.
 pub fn contains(&self,address:[u8;4])->bool {
  let Some(mask)=mask(self.prefix)else{return false;};
  usable(self.local,mask)&&usable(address,mask)
   &&u32::from_be_bytes(self.local)&mask==u32::from_be_bytes(address)&mask
 }

 /// Validation is required even when public fields were constructed directly.
 /// Endpoints may be off-link; the gateway must remain a usable local neighbor.
 /// Endpoint selection is immutable BOOT data, never newly granted authority.
 pub fn valid(&self)->bool {
  self.profile==1&&self.contains(self.local)&&self.contains(self.gateway)
   &&self.endpoint(self.dns)&&self.endpoint(self.http)&&self.gateway!=self.local
   &&self.dns_port!=0&&self.http_port!=0&&self.listen_port!=0
   &&self.listen_port<self.source_base&&self.source_base==SOURCE_BASE
   &&u32::from(self.source_base)+MAX_TICKET==u32::from(u16::MAX)
 }

 fn endpoint(&self,address:[u8;4])->bool {
  let Some(mask)=mask(self.prefix)else{return false;};
  unicast(address)&&address!=self.local
   &&(u32::from_be_bytes(address)&mask!=u32::from_be_bytes(self.local)&mask||self.contains(address))
 }

 /// Known protocol profile data only; no arbitrary name, pointer or ownership.
 pub fn hostname(&self)->Option<&'static[u8]> {
  if self.profile==1 {Some(HOST_NAME)}else{None}
 }

 /// Canonical scalar representation. This encoder does not sanction its input;
 /// consumers still validate constructed data or use the checked decoder.
 pub fn to_words(&self)->[u64;4] {
  [u64::from(u32::from_be_bytes(self.local))|(u64::from(self.prefix)<<32)
    |(u64::from(self.profile)<<40)|(u64::from(VERSION)<<56),
   u64::from(u32::from_be_bytes(self.gateway))|(u64::from(u32::from_be_bytes(self.dns))<<32),
   u64::from(self.dns_port)|(u64::from(self.http_port)<<16)
    |(u64::from(self.listen_port)<<32)|(u64::from(self.source_base)<<48),
   u64::from(u32::from_be_bytes(self.http))]
 }

 pub fn from_words(words:[u64;4])->Option<Self> {
  if words[0]>>56!=u64::from(VERSION)||(words[0]>>48)&255!=0||words[3]>>32!=0 {return None;}
  let value=Self {
   local:(words[0]as u32).to_be_bytes(),prefix:(words[0]>>32)as u8,
   profile:(words[0]>>40)as u8,gateway:(words[1]as u32).to_be_bytes(),
   dns:((words[1]>>32)as u32).to_be_bytes(),dns_port:words[2]as u16,
   http_port:(words[2]>>16)as u16,listen_port:(words[2]>>32)as u16,
   source_base:(words[2]>>48)as u16,http:(words[3]as u32).to_be_bytes(),
  };
  if value.valid() {Some(value)}else{None}
 }
}

#[cfg(test)]mod tests {
 use super::*;
 fn rejected(config:Config) {assert!(!config.valid(),"{config:?}");assert_eq!(Config::from_words(config.to_words()),None);}

 #[test]fn canonical_baseline_words_and_exact_roundtrip() {
  let config=Config::CONTROLLED;assert!(config.valid());
  assert_eq!(config.to_words(),[0x010001180a00020f,0x0a0002020a000202,0xbb80b3fcb3feb3ff,0x0a000202]);
  assert_eq!(Config::from_words(config.to_words()),Some(config));
 }
 #[test]fn each_reserved_bit_and_unknown_version_is_rejected() {
  for bit in 48..56 {let mut words=Config::CONTROLLED.to_words();words[0]|=1<<bit;assert_eq!(Config::from_words(words),None);}
  for bit in 32..64 {let mut words=Config::CONTROLLED.to_words();words[3]|=1<<bit;assert_eq!(Config::from_words(words),None);}
  for version in [0,2,127,255] {let mut words=Config::CONTROLLED.to_words();words[0]=(words[0]&0x00ffffffffffffff)|((version as u64)<<56);assert_eq!(Config::from_words(words),None);}
 }
 #[test]fn known_profile_has_exact_bounded_protocol_name() {
  assert_eq!(Config::CONTROLLED.hostname(),Some(b"probe.sexos.test" as &[u8]));
  assert_eq!(DNS_QUESTION,b"\x05probe\x05sexos\x04test\0\0\x01\0\x01");assert_eq!(DNS_QUESTION.len(),22);
  for profile in [0,2,127,255] {let config=Config {profile,..Config::CONTROLLED};assert_eq!(config.hostname(),None);rejected(config);}
 }
 #[test]fn alternate_onlink_private_configuration_and_protocol_port_overlap() {
  let config=Config {local:[192,168,7,9],prefix:27,gateway:[192,168,7,1],dns:[192,168,7,2],
   dns_port:53,http:[192,168,7,3],http_port:53,listen_port:1000,..Config::CONTROLLED};
  assert!(config.valid());assert_eq!(Config::from_words(config.to_words()),Some(config));
  assert!(config.contains([192,168,7,30]));assert!(!config.contains([192,168,7,33]));
 }
 #[test]fn prefixes_are_bounded_and_contains_never_shifts_invalid_values() {
  for prefix in [0,31,32,33,255] {let config=Config {prefix,..Config::CONTROLLED};assert!(!config.contains(config.local));rejected(config);}
  for prefix in [1,8,24,30] {let config=Config {local:[10,0,2,1],gateway:[10,0,2,2],dns:[10,0,2,2],http:[10,0,2,2],prefix,..Config::CONTROLLED};assert!(config.valid());assert_eq!(Config::from_words(config.to_words()),Some(config));}
 }
 #[test]fn unusable_local_ipv4_ranges_and_subnet_hosts_are_rejected() {
  for local in [[0,0,0,0],[0,12,2,1],[127,0,0,1],[224,0,0,1],[239,255,2,1],[240,0,2,1],[255,255,255,255],[10,0,2,0],[10,0,2,255]] {rejected(Config {local,..Config::CONTROLLED});}
 }
 #[test]fn gateway_requires_onlink_host_and_endpoints_reject_local_reserved_and_broadcast() {
  for address in [[0,0,0,1],[127,0,0,1],[224,0,0,1],[240,0,0,1],[10,0,2,0],[10,0,2,255]] {
   assert!(!Config::CONTROLLED.contains(address));
   rejected(Config {gateway:address,..Config::CONTROLLED});rejected(Config {dns:address,..Config::CONTROLLED});rejected(Config {http:address,..Config::CONTROLLED});
  }
  rejected(Config {gateway:[10,0,3,2],..Config::CONTROLLED});
  rejected(Config {gateway:Config::CONTROLLED.local,..Config::CONTROLLED});
  rejected(Config {dns:Config::CONTROLLED.local,..Config::CONTROLLED});
  rejected(Config {http:Config::CONTROLLED.local,..Config::CONTROLLED});
 }
 #[test]fn offlink_configured_endpoints_keep_one_onlink_gateway_and_canonical_words() {
  let config=Config {dns:[198,51,100,53],http:[203,0,113,9],..Config::CONTROLLED};
  assert!(config.valid());assert!(!config.contains(config.dns));assert!(!config.contains(config.http));
  assert!(config.contains(config.gateway));assert_eq!(Config::from_words(config.to_words()),Some(config));
 }
 #[test]fn subnet_edges_use_configured_prefix_not_a_fixed_slash24() {
  let config=Config {local:[172,16,1,9],prefix:23,gateway:[172,16,0,1],dns:[172,16,1,254],http:[172,16,0,255],..Config::CONTROLLED};
  assert!(config.valid());assert!(config.contains([172,16,0,255]));assert!(config.contains([172,16,1,0]));
  assert!(!config.contains([172,16,0,0]));assert!(!config.contains([172,16,1,255]));assert!(!config.contains([172,16,2,1]));
 }
 #[test]fn zero_ports_and_listener_overlap_are_rejected() {
  rejected(Config {dns_port:0,..Config::CONTROLLED});rejected(Config {http_port:0,..Config::CONTROLLED});rejected(Config {listen_port:0,..Config::CONTROLLED});
  for listen_port in [48000,48001,u16::MAX] {rejected(Config {listen_port,..Config::CONTROLLED});}
  let config=Config {listen_port:47999,dns_port:u16::MAX,http_port:u16::MAX,..Config::CONTROLLED};assert!(config.valid());
 }
 #[test]fn source_port_ticket_namespace_is_finite_and_never_wraps() {
  for source_base in [0,1,47999,48001,u16::MAX] {rejected(Config {source_base,..Config::CONTROLLED});}
  let config=Config::CONTROLLED;
  assert_eq!(config.source_base.checked_add(1),Some(48001));assert_eq!(config.source_base.checked_add(MAX_TICKET as u16),Some(u16::MAX));assert_eq!(config.source_base.checked_add((MAX_TICKET+1)as u16),None);
 }
}

#[cfg(test)]mod profile_consistency{
 use super::*;
 #[test]fn canonical_name_question_and_http_host_share_one_profile(){
  assert_eq!(&DNS_QUESTION[..DNS_NAME.len()],DNS_NAME);assert_eq!(&DNS_QUESTION[DNS_NAME.len()..],&[0,1,0,1]);
  let mut at=0;let mut out=[0u8;63];let mut len=0;
  while DNS_NAME[at]!=0{let n=DNS_NAME[at]as usize;at+=1;assert!(n>0&&n<=63&&at+n<DNS_NAME.len());if len>0{out[len]=b'.';len+=1;}out[len..len+n].copy_from_slice(&DNS_NAME[at..at+n]);len+=n;at+=n;}
  assert_eq!(at+1,DNS_NAME.len());assert_eq!(&out[..len],HOST_NAME);assert_eq!(Config::CONTROLLED.hostname(),Some(HOST_NAME));
 }
}
