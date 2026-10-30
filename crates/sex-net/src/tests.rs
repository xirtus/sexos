//! Actual smoltcp-backed production engine, independent raw packet peer.
extern crate std;
use super::*;
use std::{boxed::Box,vec,vec::Vec};
const BASELINE:sex_net_config::Config=sex_net_config::Config::CONTROLLED;
const OUR:[u8;6]=[0x52,0x54,0,0x12,0x34,0x56];const PEER:[u8;6]=[2,0,0,0,0,1];const PEER_IP:[u8;4]=[10,0,2,2];
fn sum(bytes:&[u8])->u16{let mut s=0u32;for c in bytes.chunks(2){s+=u16::from_be_bytes([c[0],*c.get(1).unwrap_or(&0)])as u32;}while s>>16!=0{s=(s&0xffff)+(s>>16);}!(s as u16)}
fn engine()->Engine<'static>{engine_with(BASELINE)}
fn engine_with(config:sex_net_config::Config)->Engine<'static>{Engine::new(config,OUR,0,0x7365786f735f3736,Box::leak(Box::new([0;FLOW_BYTES])),Box::leak(Box::new([0;FLOW_BYTES])),Box::leak(Box::new([0;FLOW_BYTES])),Box::leak(Box::new([0;FLOW_BYTES])),Box::leak(Box::new([DnsMetadata::EMPTY])),Box::leak(Box::new([0;DNS_BYTES])),Box::leak(Box::new([DnsMetadata::EMPTY])),Box::leak(Box::new([0;DNS_BYTES])),Box::leak(Box::new([Storage::EMPTY,Storage::EMPTY,Storage::EMPTY]))).unwrap()}
fn arp()->Vec<u8>{arp_for(PEER_IP,LOCAL)}
fn arp_for(peer:[u8;4],local:[u8;4])->Vec<u8>{let mut v=vec![0u8;60];v[..6].fill(255);v[6..12].copy_from_slice(&PEER);v[12..22].copy_from_slice(&[8,6,0,1,8,0,6,4,0,1]);v[22..28].copy_from_slice(&PEER);v[28..32].copy_from_slice(&peer);v[38..42].copy_from_slice(&local);v}
fn tcp(seq:u32,ack:u32,flags:u8,body:&[u8])->Vec<u8>{
 tcp_for(PEER_IP,LOCAL,62000,PORT,seq,ack,flags,body)
}

// Raw remote endpoints share only a local gateway. Different link identities
// ensure an ingress peer cannot accidentally stand in for next-hop discovery.
const ROUTED:sex_net_config::Config=sex_net_config::Config{dns:[198,51,100,53],http:[203,0,113,9],..BASELINE};
const GATEWAY_MAC:[u8;6]=[2,0,0,0,0,2];
fn gateway_reply(c:sex_net_config::Config,mac:[u8;6])->Vec<u8>{
 let mut p=arp_for(c.gateway,c.local);p[..6].copy_from_slice(&OUR);p[6..12].copy_from_slice(&mac);
 p[20..22].copy_from_slice(&[0,2]);p[22..28].copy_from_slice(&mac);p[32..38].copy_from_slice(&OUR);p
}
fn gateway_query(frame:&Frame,c:sex_net_config::Config){
 let p=&frame.bytes[..frame.length];assert_eq!(&p[..6],&[255;6]);assert_eq!(&p[12..14],&[8,6]);
 assert_eq!(&p[20..22],&[0,1]);assert_eq!(&p[28..32],&c.local);assert_eq!(&p[38..42],&c.gateway);
}
fn via_gateway(frame:&Frame,peer:[u8;4],mac:[u8;6]){
 assert_eq!(&frame.bytes[..6],&mac);assert_eq!(&frame.bytes[6..12],&OUR);
 assert_eq!(&frame.bytes[12..14],&[8,0]);assert_eq!(&frame.bytes[26..30],&ROUTED.local);assert_eq!(&frame.bytes[30..34],&peer);
}
#[test]fn exactly_one_immutable_default_route_uses_checked_onlink_gateway(){
 let mut e=engine_with(ROUTED);let r=e.iface.routes().get_default_ipv4_route().unwrap();
 assert_eq!(r.via_router,IpAddress::v4(10,0,2,2));assert!(r.expires_at.is_none());
 e.iface.routes_mut().update(|routes|assert_eq!(routes.len(),1));
 assert_eq!(NEIGHBOR_CAPACITY,8);assert_eq!(ROUTE_CAPACITY,2);
}
#[test]fn offlink_tcp_discovers_gateway_and_retains_exact_peer_tuple_and_ticket(){
 let c=ROUTED;let mut e=engine_with(c);let mut now=0;e.begin_client(7).unwrap();
 gateway_query(&tick(&mut e,&mut now).unwrap(),c);
 // Off-link ARP and the peer's different ingress MAC cannot supply a gateway.
 e.submit_frame(&arp_for(c.http,c.local)).unwrap();assert!(tick(&mut e,&mut now).is_none());
 e.submit_frame(&gateway_reply(c,GATEWAY_MAC)).unwrap();let syn=tick(&mut e,&mut now).unwrap();
 via_gateway(&syn,c.http,GATEWAY_MAC);assert_eq!(syn.client_ticket(&c),Some(7));let s=parse(&syn).unwrap();assert_eq!(s.flags&2,2);
 let server=s.seq.wrapping_add(1);e.submit_frame(&tcp_for(c.http,c.local,c.http_port,c.source_base+7,900,server,0x12,&[])).unwrap();
 let ack=tick(&mut e,&mut now).unwrap();via_gateway(&ack,c.http,GATEWAY_MAC);assert_eq!(e.client_state(),State::Established);
 assert_eq!(e.client_send(b"configured remote"),Ok(17));let packet=tick(&mut e,&mut now).unwrap();via_gateway(&packet,c.http,GATEWAY_MAC);assert_eq!(parse(&packet).unwrap().body,b"configured remote");
 e.submit_frame(&tcp_for(c.http,c.local,c.http_port,c.source_base+7,901,server+17,0x18,b"real answer")).unwrap();tick(&mut e,&mut now);
 let mut out=[0;32];assert_eq!(e.client_receive(&mut out),Ok(11));assert_eq!(&out[..11],b"real answer");
}
#[test]fn offlink_dns_uses_gateway_without_weakening_source_response_binding(){
 let c=ROUTED;let mut e=engine_with(c);let mut now=0;e.begin_dns(9).unwrap();gateway_query(&tick(&mut e,&mut now).unwrap(),c);
 e.submit_frame(&gateway_reply(c,GATEWAY_MAC)).unwrap();let query=tick(&mut e,&mut now).unwrap();via_gateway(&query,c.dns,GATEWAY_MAC);
 assert_eq!(query.dns_ticket(&c),Some(9));let(a,b,p)=parse_udp(&query).unwrap();assert_eq!((a,b),(48009,c.dns_port));assert_eq!(&p[12..],sex_net_config::DNS_QUESTION);
 let mut out=[0xA5;DNS_BYTES];e.submit_frame(&udp(c.http,c.local,c.dns_port,48009,b"foreign")).unwrap();tick(&mut e,&mut now);assert_eq!(e.dns_receive(&mut out),Ok(None));assert_eq!(out,[0xA5;DNS_BYTES]);
 e.submit_frame(&udp(c.dns,c.local,c.dns_port,48009,b"answer")).unwrap();tick(&mut e,&mut now);assert_eq!(e.dns_receive(&mut out),Ok(Some(6)));assert_eq!(&out[..6],b"answer");
}
#[test]fn missing_gateway_has_rate_limited_discovery_and_no_broadcast_ip_fallback(){
 let c=ROUTED;let mut e=engine_with(c);let mut now=0;e.begin_dns(1).unwrap();gateway_query(&tick(&mut e,&mut now).unwrap(),c);
 let mut queries=1;for _ in 0..3500{if let Some(frame)=tick(&mut e,&mut now){gateway_query(&frame,c);queries+=1;}}
 assert!((2..=4).contains(&queries));assert_eq!(e.dns_receive(&mut[0;DNS_BYTES]),Ok(None));
 e.dns_abort();for _ in 0..2500{assert!(tick(&mut e,&mut now).is_none());}
}
#[test]fn cancel_before_gateway_answer_keeps_old_reset_tagged_and_new_jobs_distinct(){
 let c=ROUTED;
 for dns in [false,true]{let mut e=engine_with(c);let mut now=0;
  if dns{e.begin_dns(1).unwrap();}else{e.begin_client(1).unwrap();}gateway_query(&tick(&mut e,&mut now).unwrap(),c);
  if dns{e.dns_abort();}else{e.client_abort();}
  e.submit_frame(&gateway_reply(c,GATEWAY_MAC)).unwrap();
  for _ in 0..8{if let Some(frame)=tick(&mut e,&mut now){
   // TCP abort may produce a private RST. It MUST retain the old ticket so
   // the parent's live-job fence discards it before DMA admission. DNS close
   // cannot replay a query; neither protocol may resurrect payload or SYN.
   assert!(!dns);assert_eq!(frame.client_ticket(&c),Some(1));assert_eq!(frame.dns_ticket(&c),None);
   let reset=parse(&frame).unwrap();assert_eq!(reset.flags&4,4);assert_eq!(reset.flags&2,0);assert!(reset.body.is_empty());
  }}
  if dns{e.begin_dns(2).unwrap();}else{e.begin_client(2).unwrap();}
  let frame=tick(&mut e,&mut now).unwrap();via_gateway(&frame,if dns{c.dns}else{c.http},GATEWAY_MAC);
  assert_eq!(if dns{frame.dns_ticket(&c)}else{frame.client_ticket(&c)},Some(2));
 }
}
#[test]fn gateway_neighbor_expiry_requires_fresh_discovery_then_accepts_new_mac(){
 let c=ROUTED;let mut e=engine_with(c);let mut now=0;e.submit_frame(&gateway_reply(c,GATEWAY_MAC)).unwrap();assert!(tick(&mut e,&mut now).is_none());
 e.begin_dns(1).unwrap();let first=tick(&mut e,&mut now).unwrap();via_gateway(&first,c.dns,GATEWAY_MAC);e.dns_abort();
 // Time is captured input. No clock getter or wall-clock assumption is used.
 now=60_002;e.advance(now);assert!(!e.has_frame());e.begin_dns(2).unwrap();gateway_query(&tick(&mut e,&mut now).unwrap(),c);
 let new_mac=[2,0,0,0,0,3];e.submit_frame(&gateway_reply(c,new_mac)).unwrap();let fresh=tick(&mut e,&mut now).unwrap();via_gateway(&fresh,c.dns,new_mac);assert_eq!(fresh.dns_ticket(&c),Some(2));
}
#[test]fn finite_neighbor_pressure_evicts_gateway_then_rediscovers_without_peer_redirection(){
 let c=ROUTED;let mut e=engine_with(c);let mut now=0;e.submit_frame(&gateway_reply(c,GATEWAY_MAC)).unwrap();tick(&mut e,&mut now);
 for i in 32..48{e.submit_frame(&arp_for([10,0,2,i],c.local)).unwrap();let reply=tick(&mut e,&mut now).unwrap();assert_eq!(&reply.bytes[38..42],&[10,0,2,i]);}
 e.begin_dns(4).unwrap();gateway_query(&tick(&mut e,&mut now).unwrap(),c);
 e.submit_frame(&gateway_reply(c,GATEWAY_MAC)).unwrap();let query=tick(&mut e,&mut now).unwrap();via_gateway(&query,c.dns,GATEWAY_MAC);assert_eq!(query.dns_ticket(&c),Some(4));
}
#[test]fn stopped_transport_discards_neighbor_pending_data_and_late_gateway_reply(){
 let c=ROUTED;let mut e=engine_with(c);let mut now=0;e.begin_dns(1).unwrap();gateway_query(&tick(&mut e,&mut now).unwrap(),c);
 e.stop(Error::Stopped);assert_eq!(e.submit_frame(&gateway_reply(c,GATEWAY_MAC)),Err(Error::Stopped));
 for _ in 0..8{assert!(tick(&mut e,&mut now).is_none());}assert_eq!(e.begin_client(2),Err(Error::Stopped));assert_eq!(e.begin_dns(2),Err(Error::Stopped));
}
fn tcp_for(source:[u8;4],destination:[u8;4],source_port:u16,destination_port:u16,seq:u32,ack:u32,flags:u8,body:&[u8])->Vec<u8>{
 let mut v=vec![0;54+body.len()];v[..6].copy_from_slice(&OUR);v[6..12].copy_from_slice(&PEER);v[12..14].copy_from_slice(&[8,0]);
 let total=(40+body.len())as u16;v[14]=0x45;v[16..18].copy_from_slice(&total.to_be_bytes());v[20]=0x40;v[22]=64;v[23]=6;v[26..30].copy_from_slice(&source);v[30..34].copy_from_slice(&destination);let check=sum(&v[14..34]);v[24..26].copy_from_slice(&check.to_be_bytes());
 v[34..36].copy_from_slice(&source_port.to_be_bytes());v[36..38].copy_from_slice(&destination_port.to_be_bytes());v[38..42].copy_from_slice(&seq.to_be_bytes());v[42..46].copy_from_slice(&ack.to_be_bytes());v[46]=5<<4;v[47]=flags;v[48..50].copy_from_slice(&4096u16.to_be_bytes());v[54..].copy_from_slice(body);
 let mut pseudo=Vec::from(&v[26..34]);pseudo.extend_from_slice(&[0,6]);pseudo.extend_from_slice(&((20+body.len())as u16).to_be_bytes());pseudo.extend_from_slice(&v[34..]);let check=sum(&pseudo);v[50..52].copy_from_slice(&check.to_be_bytes());v
}
struct Segment{seq:u32,ack:u32,flags:u8,body:Vec<u8>}
fn parse(frame:&Frame)->Option<Segment>{let v=&frame.bytes[..frame.length];if v.get(12..14)!=Some(&[8,0]){return None;}let ih=(v[14]&15)as usize*4;let total=u16::from_be_bytes([v[16],v[17]])as usize;assert_eq!(sum(&v[14..14+ih]),0);assert!(total+14<=v.len());if v[23]!=6{return None;}let start=14+ih;let mut pseudo=Vec::from(&v[26..34]);pseudo.extend_from_slice(&[0,6]);pseudo.extend_from_slice(&((total-ih)as u16).to_be_bytes());pseudo.extend_from_slice(&v[start..14+total]);assert_eq!(sum(&pseudo),0);let th=(v[start+12]>>4)as usize*4;Some(Segment{seq:u32::from_be_bytes(v[start+4..start+8].try_into().unwrap()),ack:u32::from_be_bytes(v[start+8..start+12].try_into().unwrap()),flags:v[start+13],body:v[start+th..14+total].to_vec()})}
fn tick(e:&mut Engine<'_>,now:&mut u64)->Option<Frame>{*now+=1;let old=e.statistics();e.advance(*now);let new=e.statistics();assert!(new.received-old.received<=TURN_BYTES as u64);assert!(new.queued-old.queued<=TURN_BYTES as u64);e.take_frame()}
fn established()->(Engine<'static>,u64,u32,u32){let mut e=engine();let mut now=0;e.submit_frame(&arp()).unwrap();let reply=tick(&mut e,&mut now).unwrap();assert_eq!(&reply.bytes[20..22],&[0,2]);
 let seq=0xfffffff0;e.submit_frame(&tcp(seq,0,2,&[])).unwrap();let synack=parse(&tick(&mut e,&mut now).unwrap()).unwrap();assert_eq!(synack.flags&0x12,0x12);assert_eq!(synack.ack,seq+1);let client=seq+1;let server=synack.seq.wrapping_add(1);
 e.submit_frame(&tcp(client,server,0x10,&[])).unwrap();tick(&mut e,&mut now);assert_eq!(e.state(),State::Established);assert_eq!(e.statistics().flow,1);(e,now,client,server)}
fn deliver(e:&mut Engine<'_>,now:&mut u64,client:u32,server:&mut u32,expected:usize)->Vec<u8>{let mut out=Vec::new();for _ in 0..1000{if let Some(f)=tick(e,now){if let Some(s)=parse(&f){if !s.body.is_empty(){assert_eq!(s.seq,*server);*server=server.wrapping_add(s.body.len()as u32);out.extend_from_slice(&s.body);e.submit_frame(&tcp(client,*server,0x10,&[])).unwrap();}}}if out.len()==expected&&e.can_receive(){break;}}assert_eq!(out.len(),expected);out}
#[test]fn actual_tcp_handshake_and_more_than_buffer_capacity_echo_wrap_with_bounded_turns(){let(mut e,mut now,mut client,mut server)=established();let payload:Vec<u8>=(0..12289).map(|n|(n*17)as u8).collect();let mut echoed=Vec::new();for bytes in payload.chunks(512){e.submit_frame(&tcp(client,server,0x18,bytes)).unwrap();client=client.wrapping_add(bytes.len()as u32);echoed.extend_from_slice(&deliver(&mut e,&mut now,client,&mut server,bytes.len()));}assert_eq!(echoed,payload);let s=e.statistics();assert_eq!(s.flow_received,12289);assert_eq!(s.flow_queued,12289);assert_eq!(s.flow_receive_hash,hash(BASIS,&payload));assert_eq!(s.flow_queue_hash,s.flow_receive_hash);}
#[test]fn invalid_checksum_or_ipv4_fragment_cannot_reach_application_custody(){let(mut e,mut now,client,server)=established();for which in 0..3{let mut p=tcp(client,server,0x18,b"not accepted");match which{0=>p[24]^=1,1=>p[50]^=1,_=>{p[20]=0x20;p[24]=0;p[25]=0;let c=sum(&p[14..34]);p[24..26].copy_from_slice(&c.to_be_bytes());}}e.submit_frame(&p).unwrap();for _ in 0..4{tick(&mut e,&mut now);}assert_eq!(e.statistics().received,0);assert_eq!(e.statistics().queued,0);}}
#[test]fn receive_and_transmit_backpressure_never_overwrite_a_pending_private_frame(){let mut e=engine();let p=arp();e.submit_frame(&p).unwrap();assert_eq!(e.submit_frame(&p),Err(Error::Busy));e.advance(1);assert!(e.has_frame());e.submit_frame(&p).unwrap();let packets=e.statistics().packets;for now in 2..10{e.advance(now);assert_eq!(e.statistics().packets,packets);}let first=e.take_frame().unwrap();e.advance(10);let second=e.take_frame().unwrap();assert_eq!(&first.bytes[..first.length],&second.bytes[..second.length]);}
#[test]fn clock_reversal_stops_protocol_without_emitting_or_accepting_more_packets(){let(mut e,mut now,client,server)=established();e.submit_frame(&tcp(client,server,0x18,b"private")).unwrap();tick(&mut e,&mut now);e.advance(now-1);assert_eq!(e.failure(),Some(Error::Clock));assert!(!e.has_frame());assert_eq!(e.submit_frame(&arp()),Err(Error::Stopped));let stats=e.statistics();for n in now..now+100{e.advance(n);}assert_eq!(e.statistics(),stats);}
#[test]fn lost_segment_is_retransmitted_with_same_sequence_without_duplicate_application_consumption(){let(mut e,mut now,client,server)=established();let body=b"real transport retransmission";e.submit_frame(&tcp(client,server,0x18,body)).unwrap();let mut first=None;for _ in 0..20{if let Some(f)=tick(&mut e,&mut now){if let Some(s)=parse(&f){if !s.body.is_empty(){first=Some(s);break;}}}}let first=first.unwrap();assert_eq!(first.seq,server);let stats=e.statistics();now+=1500;let duplicate=parse(&tick(&mut e,&mut now).unwrap()).unwrap();assert_eq!(duplicate.seq,first.seq);assert_eq!(duplicate.body,first.body);assert_eq!(e.statistics().received,stats.received);assert_eq!(e.statistics().queued,stats.queued);}
#[test]fn malformed_lengths_and_station_address_are_rejected_before_processing(){let mut e=engine();for n in [0,13,1515]{assert_eq!(e.submit_frame(&vec![0;n]),Err(Error::Length));}assert_eq!(e.statistics().packets,0);}

#[test]fn outgoing_client_frames_keep_unique_job_identity_through_abort_and_reconnect(){
 let mut e=engine();let mut now=0;e.submit_frame(&arp()).unwrap();tick(&mut e,&mut now);
 e.begin_client(1).unwrap();let first=tick(&mut e,&mut now).unwrap();let s=parse(&first).unwrap();assert_eq!(s.flags&2,2);assert_eq!(first.client_ticket(&BASELINE),Some(1));assert_eq!(u16::from_be_bytes(first.bytes[36..38].try_into().unwrap()),CLIENT_REMOTE_PORT);
 e.client_abort();e.begin_client(2).unwrap();let second=tick(&mut e,&mut now).unwrap();assert_eq!(second.client_ticket(&BASELINE),Some(2));assert_eq!(first.client_ticket(&BASELINE),Some(1));assert_ne!(&first.bytes[34..36],&second.bytes[34..36]);
 e.client_abort();assert_eq!(e.client_state(),State::Closed);assert!(!e.has_frame());assert_eq!(e.state(),State::Listen);
}
#[test]fn client_bounds_and_stop_cannot_create_another_wire_flow(){
 let mut e=engine();for ticket in[0,17536,u32::MAX]{assert_eq!(e.begin_client(ticket),Err(Error::Length));assert_eq!(e.client_state(),State::Closed);}
 assert_eq!(e.client_send(&[0;TURN_BYTES+1]),Err(Error::Length));assert_eq!(e.client_receive(&mut[0;TURN_BYTES+1]),Err(Error::Length));e.stop(Error::Stopped);assert_eq!(e.begin_client(1),Err(Error::Stopped));assert_eq!(e.client_send(b"x"),Err(Error::Stopped));assert!(!e.has_frame());
}

// This peer independently builds Ethernet/IPv4/UDP, including the pseudo-header
// checksum. It does not use the production codec or smoltcp packet emit helpers.
fn udp(source:[u8;4],destination:[u8;4],source_port:u16,destination_port:u16,body:&[u8])->Vec<u8>{
 let mut v=vec![0;42+body.len()];v[..6].copy_from_slice(&OUR);v[6..12].copy_from_slice(&PEER);v[12..14].copy_from_slice(&[8,0]);
 v[14]=0x45;v[16..18].copy_from_slice(&((28+body.len())as u16).to_be_bytes());v[20]=0x40;v[22]=64;v[23]=17;
 v[26..30].copy_from_slice(&source);v[30..34].copy_from_slice(&destination);let check=sum(&v[14..34]);v[24..26].copy_from_slice(&check.to_be_bytes());
 v[34..36].copy_from_slice(&source_port.to_be_bytes());v[36..38].copy_from_slice(&destination_port.to_be_bytes());
 v[38..40].copy_from_slice(&((8+body.len())as u16).to_be_bytes());v[42..].copy_from_slice(body);
 let mut pseudo=Vec::from(&v[26..34]);pseudo.extend_from_slice(&[0,17]);pseudo.extend_from_slice(&((8+body.len())as u16).to_be_bytes());pseudo.extend_from_slice(&v[34..]);
 let check=match sum(&pseudo){0=>0xFFFF,n=>n};v[40..42].copy_from_slice(&check.to_be_bytes());v
}
fn parse_udp(frame:&Frame)->Option<(u16,u16,Vec<u8>)>{
 let v=&frame.bytes[..frame.length];if v.get(12..14)!=Some(&[8,0])||v[23]!=17{return None;}
 let ih=(v[14]&15)as usize*4;let total=u16::from_be_bytes([v[16],v[17]])as usize;let offset=14+ih;
 assert_eq!(sum(&v[14..offset]),0);assert!(14+total<=v.len());assert!(total>=ih+8);
 assert_eq!(u16::from_be_bytes([v[offset+4],v[offset+5]])as usize,total-ih);
 let mut pseudo=Vec::from(&v[26..34]);pseudo.extend_from_slice(&[0,17]);pseudo.extend_from_slice(&((total-ih)as u16).to_be_bytes());pseudo.extend_from_slice(&v[offset..14+total]);assert_eq!(sum(&pseudo),0);
 Some((u16::from_be_bytes([v[offset],v[offset+1]]),u16::from_be_bytes([v[offset+2],v[offset+3]]),v[offset+8..14+total].to_vec()))
}
fn dns_ready()->(Engine<'static>,u64){let mut e=engine();let mut now=0;e.submit_frame(&arp()).unwrap();tick(&mut e,&mut now);(e,now)}
#[test]fn actual_dns_query_has_independent_checked_udp_headers_and_exact_wire_bytes(){
 let(mut e,mut now)=dns_ready();e.begin_dns(1).unwrap();let frame=tick(&mut e,&mut now).unwrap();
 assert_eq!(&frame.bytes[26..30],&LOCAL);assert_eq!(&frame.bytes[30..34],&PEER_IP);
 let(src,dst,body)=parse_udp(&frame).unwrap();assert_eq!((src,dst),(48001,DNS_REMOTE_PORT));
 assert_eq!(body,b"\x00\x01\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\x05probe\x05sexos\x04test\x00\x00\x01\x00\x01");
 assert_eq!(frame.dns_ticket(&BASELINE),Some(1));assert_eq!(frame.client_ticket(&BASELINE),None);
}
#[test]fn dns_receives_one_full_datagram_and_never_replays_it(){
 let(mut e,mut now)=dns_ready();e.begin_dns(1).unwrap();tick(&mut e,&mut now);
 let body:Vec<u8>=(0..DNS_BYTES).map(|n|(n*7)as u8).collect();e.submit_frame(&udp(PEER_IP,LOCAL,DNS_REMOTE_PORT,48001,&body)).unwrap();tick(&mut e,&mut now);
 let mut out=[0;DNS_BYTES];assert_eq!(e.dns_receive(&mut out),Ok(Some(DNS_BYTES)));assert_eq!(&out[..],&body);
 out.fill(0xA5);assert_eq!(e.dns_receive(&mut out),Ok(None));assert_eq!(out,[0xA5;DNS_BYTES]);
}
#[test]fn wrong_dns_source_or_local_tuple_is_discarded_without_exposing_bytes(){
 let(mut e,mut now)=dns_ready();e.begin_dns(1).unwrap();tick(&mut e,&mut now);
 for(source,destination,source_port,destination_port)in[
  ([10,0,2,3],LOCAL,DNS_REMOTE_PORT,48001),(PEER_IP,LOCAL,DNS_REMOTE_PORT+1,48001),
  (PEER_IP,[10,0,2,16],DNS_REMOTE_PORT,48001),(PEER_IP,LOCAL,DNS_REMOTE_PORT,48002)]{
  e.submit_frame(&udp(source,destination,source_port,destination_port,b"foreign datagram")).unwrap();tick(&mut e,&mut now);
  let mut out=[0xA5;DNS_BYTES];assert_eq!(e.dns_receive(&mut out),Ok(None));assert_eq!(out,[0xA5;DNS_BYTES]);
 }
 e.submit_frame(&udp(PEER_IP,LOCAL,DNS_REMOTE_PORT,48001,b"accepted")).unwrap();tick(&mut e,&mut now);
 let mut out=[0;DNS_BYTES];assert_eq!(e.dns_receive(&mut out),Ok(Some(8)));assert_eq!(&out[..8],b"accepted");
}
#[test]fn dns_abort_rebind_has_fresh_id_and_port_and_old_frame_keeps_identity(){
 let(mut e,mut now)=dns_ready();e.begin_dns(1).unwrap();let first=tick(&mut e,&mut now).unwrap();
 e.dns_abort();e.begin_dns(2).unwrap();let second=tick(&mut e,&mut now).unwrap();
 let(a,_,first_body)=parse_udp(&first).unwrap();let(b,_,second_body)=parse_udp(&second).unwrap();
 assert_eq!((a,b),(48001,48002));assert_eq!(&first_body[..2],&[0,1]);assert_eq!(&second_body[..2],&[0,2]);
 assert_eq!(first.dns_ticket(&BASELINE),Some(1));assert_eq!(second.dns_ticket(&BASELINE),Some(2));
 e.submit_frame(&udp(PEER_IP,LOCAL,DNS_REMOTE_PORT,48001,b"old response")).unwrap();tick(&mut e,&mut now);
 let mut out=[0xA5;DNS_BYTES];assert_eq!(e.dns_receive(&mut out),Ok(None));assert_eq!(out,[0xA5;DNS_BYTES]);
 e.begin_dns(3).unwrap();e.advance(now+1);assert!(e.has_frame());e.dns_abort();assert!(!e.has_frame());
 assert!(!e.flows.get::<udp::Socket>(e.dns_handle).is_open());
}
#[test]fn invalid_dns_tickets_cannot_abort_or_mutate_valid_pending_query(){
 let(mut e,mut now)=dns_ready();e.begin_dns(1).unwrap();
 for ticket in[0,17536,u32::MAX]{assert_eq!(e.begin_dns(ticket),Err(Error::Length));}
 let frame=tick(&mut e,&mut now).unwrap();assert_eq!(frame.dns_ticket(&BASELINE),Some(1));assert_eq!(parse_udp(&frame).unwrap().2[..2],[0,1]);
 e.begin_dns(17535).unwrap();let frame=tick(&mut e,&mut now).unwrap();assert_eq!(parse_udp(&frame).unwrap().0,u16::MAX);
 assert_eq!(frame.dns_ticket(&BASELINE),Some(17535));
 e.stop(Error::Stopped);assert_eq!(e.begin_dns(1),Err(Error::Stopped));
 assert_eq!(e.dns_receive(&mut[0;DNS_BYTES]),Err(Error::Stopped));assert!(!e.has_frame());assert!(!e.flows.get::<udp::Socket>(e.dns_handle).is_open());
}
#[test]fn udp_query_is_not_automatically_replayed_on_timeout_or_later_ticks(){
 let(mut e,mut now)=dns_ready();e.begin_dns(1).unwrap();assert!(parse_udp(&tick(&mut e,&mut now).unwrap()).is_some());
 for _ in 0..20{now+=1000;if let Some(frame)=tick(&mut e,&mut now){assert!(parse_udp(&frame).is_none());}}
 assert_eq!(e.flows.get::<udp::Socket>(e.dns_handle).send_queue(),0);
}
#[test]fn udp_metadata_capacity_and_checksums_bound_response_custody(){
 let(mut e,mut now)=dns_ready();e.begin_dns(1).unwrap();tick(&mut e,&mut now);
 e.submit_frame(&udp(PEER_IP,LOCAL,DNS_REMOTE_PORT,48001,b"first")).unwrap();tick(&mut e,&mut now);
 e.submit_frame(&udp(PEER_IP,LOCAL,DNS_REMOTE_PORT,48001,b"second")).unwrap();tick(&mut e,&mut now);
 let mut out=[0;DNS_BYTES];assert_eq!(e.dns_receive(&mut out),Ok(Some(5)));assert_eq!(&out[..5],b"first");assert_eq!(e.dns_receive(&mut out),Ok(None));
 for which in 0..4{
  let mut packet=udp(PEER_IP,LOCAL,DNS_REMOTE_PORT,48001,if which==3{&[0;DNS_BYTES+1]}else{b"bad"});
  match which{0=>packet[24]^=1,1=>packet[40]^=1,2=>packet[38..40].copy_from_slice(&u16::MAX.to_be_bytes()),_=>{}}
  e.submit_frame(&packet).unwrap();tick(&mut e,&mut now);assert_eq!(e.dns_receive(&mut out),Ok(None));
 }
}
#[test]fn aborting_tcp_and_dns_preserves_other_protocols_collected_frame(){
 let(mut e,mut now)=dns_ready();e.begin_client(1).unwrap();now+=1;e.advance(now);assert!(e.has_frame());
 e.begin_dns(1).unwrap();e.dns_abort();let tcp=e.take_frame().unwrap();assert_eq!(tcp.client_ticket(&BASELINE),Some(1));assert_eq!(tcp.dns_ticket(&BASELINE),None);
 let(mut e,mut now)=dns_ready();e.begin_dns(2).unwrap();now+=1;e.advance(now);assert!(e.has_frame());e.client_abort();
 let dns=e.take_frame().unwrap();assert_eq!(dns.dns_ticket(&BASELINE),Some(2));assert_eq!(dns.client_ticket(&BASELINE),None);
}

const ALTERNATE:sex_net_config::Config=sex_net_config::Config{
 local:[192,168,44,17],prefix:24,gateway:[192,168,44,1],dns:[192,168,44,1],
 http:[192,168,44,2],listen_port:46086,http_port:46088,dns_port:46089,..BASELINE
};
fn configured_ready(config:sex_net_config::Config)->(Engine<'static>,u64){
 let mut e=engine_with(config);let mut now=0;
 for peer in [config.dns,config.http]{e.submit_frame(&arp_for(peer,config.local)).unwrap();let reply=tick(&mut e,&mut now).unwrap();
  assert_eq!(&reply.bytes[20..22],&[0,2]);assert_eq!(&reply.bytes[28..32],&config.local);assert_eq!(&reply.bytes[38..42],&peer);
 }
 assert_eq!(e.iface.routes().get_default_ipv4_route().unwrap().via_router,IpAddress::v4(config.gateway[0],config.gateway[1],config.gateway[2],config.gateway[3]));(e,now)
}
#[test]fn alternate_arp_listener_echo_and_rearm_use_configured_local_address_and_port(){
 let c=ALTERNATE;let(mut e,mut now)=configured_ready(c);let client=101;
 e.submit_frame(&tcp_for(c.http,c.local,62000,c.listen_port,client,0,2,&[])).unwrap();
 let frame=tick(&mut e,&mut now).unwrap();let synack=parse(&frame).unwrap();assert_eq!(synack.flags&0x12,0x12);
 assert_eq!(&frame.bytes[26..30],&c.local);assert_eq!(&frame.bytes[30..34],&c.http);
 assert_eq!(u16::from_be_bytes(frame.bytes[34..36].try_into().unwrap()),c.listen_port);
 assert_eq!(frame.client_ticket(&c),None);let server=synack.seq.wrapping_add(1);
 e.submit_frame(&tcp_for(c.http,c.local,62000,c.listen_port,client+1,server,0x10,&[])).unwrap();tick(&mut e,&mut now);assert_eq!(e.state(),State::Established);
 e.submit_frame(&tcp_for(c.http,c.local,62000,c.listen_port,client+1,server,0x18,b"alternate echo")).unwrap();
 let mut echo=Vec::new();for _ in 0..10{if let Some(frame)=tick(&mut e,&mut now){if let Some(segment)=parse(&frame){echo.extend_from_slice(&segment.body);}}if echo.len()==14{break;}}
 assert_eq!(echo,b"alternate echo");assert_eq!(e.statistics().flow_received,14);
 e.submit_frame(&tcp_for(c.http,c.local,62000,c.listen_port,client+15,server+14,0x14,&[])).unwrap();tick(&mut e,&mut now);assert_eq!(e.state(),State::Listen);
 e.submit_frame(&tcp_for(c.http,c.local,62001,c.listen_port,200,0,2,&[])).unwrap();
 let frame=tick(&mut e,&mut now).unwrap();assert_eq!(parse(&frame).unwrap().flags&0x12,0x12);assert_eq!(u16::from_be_bytes(frame.bytes[34..36].try_into().unwrap()),c.listen_port);
}
#[test]fn alternate_outbound_tcp_uses_configured_http_tuple_and_rejects_foreign_tuples(){
 let c=ALTERNATE;let(mut e,mut now)=configured_ready(c);e.begin_client(7).unwrap();let first=tick(&mut e,&mut now).unwrap();let syn=parse(&first).unwrap();
 assert_eq!(syn.flags&2,2);assert_eq!(&first.bytes[26..30],&c.local);assert_eq!(&first.bytes[30..34],&c.http);
 assert_eq!(u16::from_be_bytes(first.bytes[34..36].try_into().unwrap()),c.source_base+7);
 assert_eq!(u16::from_be_bytes(first.bytes[36..38].try_into().unwrap()),c.http_port);
 assert_eq!(first.client_ticket(&c),Some(7));assert_eq!(first.client_ticket(&BASELINE),None);assert_eq!(first.dns_ticket(&c),None);
 let server=syn.seq.wrapping_add(1);let peer=901;
 e.submit_frame(&tcp_for(c.http,c.local,c.http_port,c.source_base+7,peer,server,0x12,&[])).unwrap();tick(&mut e,&mut now);assert_eq!(e.client_state(),State::Established);
 for(source,destination,source_port,destination_port)in[
  (c.dns,c.local,c.http_port,c.source_base+7),(c.http,c.local,c.http_port+1,c.source_base+7),
  (c.http,[192,168,44,18],c.http_port,c.source_base+7),(c.http,c.local,c.http_port,c.source_base+8)]{
  e.submit_frame(&tcp_for(source,destination,source_port,destination_port,peer+1,server,0x18,b"foreign")).unwrap();tick(&mut e,&mut now);
  let mut out=[0xA5;16];assert_eq!(e.client_receive(&mut out),Ok(0));assert_eq!(out,[0xA5;16]);assert_eq!(e.client_state(),State::Established);
 }
 e.submit_frame(&tcp_for(c.http,c.local,c.http_port,c.source_base+7,peer+1,server,0x18,b"accepted")).unwrap();tick(&mut e,&mut now);
 let mut out=[0;16];assert_eq!(e.client_receive(&mut out),Ok(8));assert_eq!(&out[..8],b"accepted");
 e.client_abort();e.begin_client(8).unwrap();let second=tick(&mut e,&mut now).unwrap();assert_eq!(second.client_ticket(&c),Some(8));assert_eq!(first.client_ticket(&c),Some(7));
}
#[test]fn alternate_dns_query_and_response_binding_use_configured_addresses_and_ports(){
 let c=ALTERNATE;let(mut e,mut now)=configured_ready(c);e.begin_dns(9).unwrap();let first=tick(&mut e,&mut now).unwrap();
 assert_eq!(&first.bytes[26..30],&c.local);assert_eq!(&first.bytes[30..34],&c.dns);
 let(source,destination,query)=parse_udp(&first).unwrap();assert_eq!((source,destination),(c.source_base+9,c.dns_port));
 assert_eq!(&query[..12],b"\x00\x09\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00");assert_eq!(&query[12..],sex_net_config::DNS_QUESTION);
 assert_eq!(first.dns_ticket(&c),Some(9));assert_eq!(first.dns_ticket(&BASELINE),None);assert_eq!(first.client_ticket(&c),None);
 for(source,destination,source_port,destination_port)in[
  (c.http,c.local,c.dns_port,c.source_base+9),(c.dns,c.local,c.dns_port+1,c.source_base+9),
  (c.dns,[192,168,44,18],c.dns_port,c.source_base+9),(c.dns,c.local,c.dns_port,c.source_base+10)]{
  e.submit_frame(&udp(source,destination,source_port,destination_port,b"foreign")).unwrap();tick(&mut e,&mut now);
  let mut out=[0xA5;DNS_BYTES];assert_eq!(e.dns_receive(&mut out),Ok(None));assert_eq!(out,[0xA5;DNS_BYTES]);
 }
 e.submit_frame(&udp(c.dns,c.local,c.dns_port,c.source_base+9,b"accepted")).unwrap();tick(&mut e,&mut now);
 let mut out=[0;DNS_BYTES];assert_eq!(e.dns_receive(&mut out),Ok(Some(8)));assert_eq!(&out[..8],b"accepted");
 e.dns_abort();e.begin_dns(10).unwrap();let second=tick(&mut e,&mut now).unwrap();assert_eq!(second.dns_ticket(&c),Some(10));assert_eq!(first.dns_ticket(&c),Some(9));
}
#[test]fn configured_prefix_prefers_onlink_peers_across_slash24_over_default_route(){
 let c=sex_net_config::Config{prefix:23,dns:[192,168,45,1],http:[192,168,45,2],..ALTERNATE};
 let(mut e,mut now)=configured_ready(c);e.begin_client(1).unwrap();let frame=tick(&mut e,&mut now).unwrap();assert!(parse(&frame).is_some());assert_eq!(&frame.bytes[30..34],&c.http);
 let(mut e,mut now)=configured_ready(c);e.begin_dns(1).unwrap();let frame=tick(&mut e,&mut now).unwrap();assert!(parse_udp(&frame).is_some());assert_eq!(&frame.bytes[30..34],&c.dns);
 assert_eq!(e.iface.ip_addrs(),&[IpCidr::new(IpAddress::v4(192,168,44,17),23)]);assert_eq!(e.iface.routes().get_default_ipv4_route().unwrap().via_router,IpAddress::v4(c.gateway[0],c.gateway[1],c.gateway[2],c.gateway[3]));
}
#[test]fn invalid_config_is_rejected_before_borrowed_buffer_effects_and_classification(){
 for config in[
  sex_net_config::Config{prefix:0,..ALTERNATE},sex_net_config::Config{profile:2,..ALTERNATE},
  sex_net_config::Config{http:[224,0,0,2],..ALTERNATE},sex_net_config::Config{dns_port:0,..ALTERNATE},
  sex_net_config::Config{source_base:47999,..ALTERNATE},sex_net_config::Config{listen_port:48000,..ALTERNATE}]{
  let(mut rx,mut tx,mut client_rx,mut client_tx)=([0xA1;FLOW_BYTES],[0xA2;FLOW_BYTES],[0xA3;FLOW_BYTES],[0xA4;FLOW_BYTES]);
  let(mut dns_rx,mut dns_tx)=([0xA5;DNS_BYTES],[0xA6;DNS_BYTES]);let mut rxmeta=[DnsMetadata::EMPTY];let mut txmeta=[DnsMetadata::EMPTY];let mut storage=[Storage::EMPTY,Storage::EMPTY,Storage::EMPTY];
  assert_eq!(Engine::new(config,[0;6],u64::MAX,0,&mut rx,&mut tx,&mut client_rx,&mut client_tx,&mut rxmeta,&mut dns_rx,&mut txmeta,&mut dns_tx,&mut storage).err(),Some(Error::Config));
  assert_eq!(rx,[0xA1;FLOW_BYTES]);assert_eq!(tx,[0xA2;FLOW_BYTES]);assert_eq!(client_rx,[0xA3;FLOW_BYTES]);assert_eq!(client_tx,[0xA4;FLOW_BYTES]);assert_eq!(dns_rx,[0xA5;DNS_BYTES]);assert_eq!(dns_tx,[0xA6;DNS_BYTES]);
  let(mut e,mut now)=configured_ready(ALTERNATE);e.begin_client(1).unwrap();let tcp=tick(&mut e,&mut now).unwrap();assert_eq!(tcp.client_ticket(&config),None);
  e.client_abort();e.begin_dns(1).unwrap();let dns=tick(&mut e,&mut now).unwrap();assert_eq!(dns.dns_ticket(&config),None);
 }
}
